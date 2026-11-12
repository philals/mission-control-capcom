// Agents in grouped or priority views; parents take their most urgent sub-agent's place.
// A viewed done agent stays in Done until left.

import { View, div, image, svg } from "gpui-kit";
import { v_flex, h_flex, HoverCard, Avatar, AvatarImage, AvatarFallback } from "gpui-base";
import * as fs from "fs/promises";
import { appHome, agents, terminals, panes, focused, spaces, run, exec, theme, shared, setShared, setAgentOrder, openOverlay, closeOverlay, props, window, layout } from "herdr";
import { SH, dotColor, short, openUrl, prDo, prDetails } from "./pr-details.js";
import { CHARACTERS, ringFile, dimFile } from "./characters.js";

function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

// The sidebar's rail goes by its width, not a flag: dragging its edge narrows it too.
const narrow = () => (find(layout(), "sidebar")?.size?.px ?? Infinity) < 160;

// To the rail (on macOS wide enough for the traffic lights), or back to the width it had.
function toggleSidebar() {
  const width = find(layout(), "sidebar")?.size?.px;
  if (!narrow()) setShared("sidebar.width", width);
  // Mods edited before the rail went by width still read the flag.
  setShared("sidebar.collapsed", !narrow());
  run("layout.resize", { id: "sidebar", px: narrow() ? (shared("sidebar.width") ?? 268) : window()?.platform === "macos" ? 76 : 44 });
}

const clean = (s) => (s ?? "").replace(/^[^\p{L}\p{N}]+/u, "").trim();

function leadPanes(all) {
  const leads = new Map();
  for (const p of all) {
    if (!p.terminal) continue;
    const current = leads.get(p.tab);
    // First agent, else first focused terminal, else first terminal in the tab.
    if (!current || (!current.program?.agent && (p.program?.agent || (!current.focused && p.focused)))) leads.set(p.tab, p);
  }
  return leads;
}

function title(tab, l) {
  const prog = l?.program?.name;
  const live = clean(l?.title);
  const telling = live && live !== prog && !/^[~/]|^[A-Za-z]:\\/.test(live) && !live.includes("@") && live.toLowerCase() !== l?.program?.key;
  return (tab.name === prog || !tab.named) && telling ? live : tab.name;
}
const RANK = { blocked: 4, done: 3, working: 2, idle: 1 };
// A sub-agent without a pane isn't in agents(): its raw word, error counting as
// blocked. It's never done: it reports to its parent, and the parent's done is the news.
const kidStatus = (k, listed) => {
  const s = listed.get(k.address) ?? k.status;
  return s === "error" ? "blocked" : s === "done" ? "idle" : s;
};
const SORTS = [
  { id: "priority", label: "Priority", icon: "clock" },
  { id: "grouped", label: "Grouped by space", icon: "layers" },
];
const sortOf = () => SORTS.find((s) => s.id === shared("agents.sort")) ?? SORTS[0];
const SECTION = { 4: "Needs you", 3: "Done", 2: "Working", 1: "Idle" };
const SECTION_STATUS = { 4: "blocked", 3: "done", 2: "working" };

function duration(ms) {
  const m = Math.floor(ms / 60000);
  if (m < 1) return "<1m";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h` : `${Math.floor(h / 24)}d`;
}

// The agent's icon from the app's program table: a one-color glyph or a picture.
function icon(p, size, color) {
  if (!p?.icon) return null;
  return p.tint ? svg(p.icon).size(size).flex_none().text_color(color) : image(p.icon).w(size).h(size).flex_none();
}

// Seven chevrons round the portrait, one per PR rail step that is done (six when the repo has no automerge step).
// No PR yet: a dark ring. Merged: all lit.
function ringOf(pr, merged) {
  const rail = pr?.rail ?? [];
  const total = rail.length === 6 ? 6 : 7;
  const mask = merged ? (1 << total) - 1 : rail.reduce((m, r, i) => (r.state === "done" ? m | (1 << i) : m), 0);
  return { total, mask };
}

const AV = 34;

export default class Agents extends View {
  init(_props, cx) {
    if (props()?.sortMenu) return;
    this.folded = new Set();
    // Each agent's last state and when it changed (ms), kept across restarts.
    this.changed = JSON.parse(localStorage.getItem("changed") ?? "{}");
    this.pendingPins = JSON.parse(localStorage.getItem("pins") ?? "[]");
    if (!this.pendingPins.length) localStorage.removeItem("pins");
    this.migratingPins = new Set();
    this.persona = JSON.parse(localStorage.getItem("persona") ?? "{}");
    this.photos = new Map();
    this.scanPhotos(cx);
    cx.timer.every(30000, (cx) => { this.scanPhotos(cx); cx.notify(); });
    cx.timer.every(30000, (cx) => cx.notify());
  }

  // Photos you drop in avatars/ (<id>.jpg or .png) replace the drawn portraits.
  scanPhotos(cx) {
    cx.spawn(async (cx) => {
      try {
        const files = await fs.readdir(`${appHome()}/mods/capcom-agents/avatars`);
        const found = new Map();
        for (const f of files) {
          const m = /^(.+)\.(jpe?g|png)$/i.exec(f);
          if (m) found.set(m[1].toLowerCase(), `mods/capcom-agents/avatars/${f}`);
        }
        const same = found.size === this.photos.size && [...found].every(([k, v]) => this.photos.get(k) === v);
        this.photos = found;
        if (!same) cx.notify();
      } catch {}
    });
  }

  // Each agent keeps one character for good: the first one nobody on screen has, in the cast's order.
  // With more agents than characters the cast repeats with a number.
  assign(list) {
    const live = new Set(list.map((a) => a.address));
    const taken = new Set(list.map((a) => this.persona[a.address]).filter(Boolean));
    let dirty = false;
    for (const a of list) {
      if (this.persona[a.address]) continue;
      let round = 1;
      let pick;
      while (!pick) {
        pick = CHARACTERS.map((c) => (round === 1 ? c.id : `${c.id}#${round}`)).find((id) => !taken.has(id));
        round++;
      }
      this.persona[a.address] = pick;
      taken.add(pick);
      dirty = true;
    }
    for (const k of Object.keys(this.persona)) if (!live.has(k) && !this.changed[k]) { delete this.persona[k]; dirty = true; }
    if (dirty) localStorage.setItem("persona", JSON.stringify(this.persona));
  }

  character(a) {
    const [id, n] = (this.persona[a.address] ?? "").split("#");
    const c = CHARACTERS.find((x) => x.id === id);
    return c && { ...c, label: n ? `${c.name} ${n}` : c.name };
  }

  // The agent's Claude Code session id: its terminal's foreground process is the `claude` that wrote
  // ~/.claude/sessions/<pid>.json.
  async sessionOf(terminal) {
    const cmd = 'pid=$(hc get "$1" | sed -n \'s/.*"foreground":{[^}]*"pid":\\([0-9]*\\).*/\\1/p\'); '
      + 'sed -n \'s/.*"sessionId":"\\([^"]*\\)".*/\\1/p\' "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/sessions/$pid.json"';
    try {
      return (await exec(SH[0], [...SH.slice(1), "sh", "-c", cmd, "sh", terminal], { timeout: 5000 })).stdout.trim();
    } catch {
      return "";
    }
  }

  // The portrait (your photo, else the drawn one) inside the gate ring.
  avatar(t, a, c) {
    const first = (this.prsByAgent.get(a.address) ?? []).sort((x, y) => x.merged - y.merged)[0];
    const { total, mask } = ringOf(first?.pr, first?.merged);
    const photo = this.photos.get(c.id);
    const inset = 6;
    const pic = photo ?? `mods/capcom-agents/portraits/${c.id}.svg`;
    return div()
      .relative()
      .size(AV)
      .flex_none()
      .child(svg(`mods/capcom-agents/${dimFile(total)}`).absolute().top(0).left(0).size(AV).text_color(t.border))
      .when(mask > 0, (el) => el.child(svg(`mods/capcom-agents/${ringFile(total, mask)}`).absolute().top(0).left(0).size(AV).text_color(t.foreground)))
      .child(Avatar.new().absolute().top(inset).left(inset).size(AV - inset * 2).rounded_full().overflow_hidden().bg(c.color)
        .image(AvatarImage.new(pic).size_full())
        .fallback(AvatarFallback.new().size_full().bg(c.bg)));
  }

  // Stamps agents whose state changed. Done and idle count as one: looking at a
  // finished agent turns done into idle, and looking isn't the agent doing anything.
  track(list, now) {
    let dirty = false;
    for (const a of list) {
      const s = a.status === "done" ? "idle" : a.status;
      if (this.changed[a.address]?.s === s) continue;
      this.changed[a.address] = { s, at: now };
      dirty = true;
    }
    if (!dirty) return;
    // Gone and old: the list alone is empty right after a start.
    const listed = new Set(list.map((a) => a.address));
    for (const [k, c] of Object.entries(this.changed)) if (!listed.has(k) && now - c.at > 30 * 86400000) delete this.changed[k];
    localStorage.setItem("changed", JSON.stringify(this.changed));
  }

  // Keep offline pins until their terminal returns, and keep each pending until
  // core echoes the label so a failed write doesn't erase the old pin.
  migratePins(all) {
    const listed = new Map(all.filter((t) => t.state === "live").map((t) => [t.address, t]));
    const pending = this.pendingPins.filter((address) => {
      const terminal = listed.get(address);
      if (!terminal) return true;
      if (terminal.labels?.["herdr.pinned"]) return false;
      if (!this.migratingPins.has(address)) {
        this.migratingPins.add(address);
        run("terminal.pin", { terminal: address, pinned: true });
      }
      return true;
    });
    if (pending.length === this.pendingPins.length) return;
    this.pendingPins = pending;
    if (pending.length) localStorage.setItem("pins", JSON.stringify(pending));
    else localStorage.removeItem("pins");
  }

  since(a) {
    return this.changed[a.address]?.at ?? 0;
  }

  // Looking at a done agent makes it idle; it stays with Done until you leave
  // it rather than drop away under you. Any other change moves it at once.
  hold(list) {
    const cur = list.find((a) => a.focused);
    const same = cur && cur.address === this.held?.address;
    // Arriving, or any change but done turning idle, sets what it was.
    if (!same || cur.status !== "idle") this.held = cur ? { address: cur.address, rank: RANK[cur.status] ?? 0 } : null;
  }

  rank(a) {
    const sawDone = a.address === this.held?.address && this.held.rank === RANK.done && a.status === "idle";
    return sawDone ? RANK.done : RANK[a.status] ?? 0;
  }

  sortMenu(t) {
    const on = sortOf().id;
    return v_flex()
      .w_full()
      .p(5)
      .bg(t.popover)
      .text_size(12.8)
      .child(div().px(8).pt(4).pb(6).text_size(11).text_color(t.dim).child("Sort agents"))
      .children(SORTS.map((s) => h_flex()
        .id(`sort-${s.id}`)
        .h(31)
        .px(8)
        .gap(9)
        .rounded(7)
        .items_center()
        .text_color(t.foreground)
        .hover((el) => el.bg(t.hover))
        .on_click(() => {
          setShared("agents.sort", s.id);
          closeOverlay(props().overlay);
        })
        .child(svg(`mods/workspace-chrome/${s.icon}.svg`).size(14).flex_none().text_color(t.muted))
        .child(div().flex_1().child(s.label))
        .when(s.id === on, (el) => el.child(svg("mods/workspace-chrome/check.svg").size(14).flex_none().text_color(t.foreground)))));
  }

  // `capcom pr-do` from the card, then ask My Watch to look again.
  change(cx, action, url, on) {
    cx.spawn(async (cx) => {
      await prDo(action, url, on);
      setShared("capcom.refresh", Date.now());
      cx.notify();
    });
  }

  // One icon per PR this agent sent, coloured by CI (purple once merged). Hover for My Watch' details.
  prIcons(t, a) {
    const mine = [...(this.prsByAgent.get(a.address) ?? [])].sort((x, y) => x.merged - y.merged);
    const MAX = 4;
    return h_flex()
      .gap(2)
      .flex_none()
      .items_center()
      .children(mine.slice(0, MAX).map(({ pr, merged }) => {
        const color = dotColor(pr, merged, t);
        const trigger = div()
          .id(`pri-${a.address}-${pr.url}`)
          .p(2)
          .rounded(4)
          .cursor_pointer()
          .hover((s) => s.bg(t.hover))
          .on_click((_e, cx) => {
            cx.stop_propagation();
            openUrl(pr.url);
          })
          .child(svg("mods/workspace-chrome/git-compare.svg").size(13).flex_none().text_color(color));
        const card = v_flex()
          .w(300)
          .gap(4)
          .p(10)
          .rounded(8)
          .bg(t.popover)
          .border(1)
          .border_color(t.border)
          .text_size(12)
          .child(div().font_semibold().text_color(t.foreground).child(pr.title))
          .child(div().text_size(11).text_color(t.muted).child(`${short(pr.repo)}#${pr.number}${pr.draft ? " · draft" : merged ? " · merged" : ""}`))
          .when(!merged, (el) => el.child(prDetails(pr, t, (cx, action, url, on) => this.change(cx, action, url, on), false)));
        return HoverCard.new(`prc-${a.address}-${pr.url}`).open_delay(150).trigger(trigger).content(card);
      }))
      .when(mine.length > MAX, (el) => el.child(div().text_size(10).text_color(t.dim).child(`+${mine.length - MAX}`)));
  }

  peek(addr) {
    run("terminal.peek", { terminal: addr });
  }

  // Live sub-agents of `addr`, depth first, each with the tree lines in front of it.
  tree(all, addr, lead = []) {
    const kids = all.filter((k) => k.parent === addr && k.state === "live");
    return kids.flatMap((k, i) => {
      const last = i === kids.length - 1;
      return [
        { t: k, lines: [...lead, last ? "└" : "├"] },
        ...this.tree(all, k.address, [...lead, last ? "" : "│"]),
      ];
    });
  }

  subRow(t, s, shown, compact) {
    const k = s.t;
    const st = kidStatus(k, this.listed);
    const pic = icon(k.program, 14, t.foreground);
    const on = k.address === shown;
    return h_flex()
      .id(`sub-${k.address}`)
      .pl(AV + 14)
      .pr(8)
      .rounded(8)
      .h(26)
      .flex_none()
      .gap(0)
      .items_center()
      .text_size(12.5)
      .when(on, (el) => el.bg(t.raised))
      .hover((s) => s.bg(t.hover))
      .on_click(() => (this.listed.has(k.address) ? run("pane.focus", { terminal: k.address }) : this.peek(k.address)))
      .children(s.lines.map(l => div().self_start().mt(-4).w(9).h(17).mr(4).flex_none()
        .when(Boolean(l), el => el.border_l(1.5).border_color(t.input))
        .when(l === "└" || l === "├", el => el.border_b(1.5).rounded_bl(6))))
      .when(Boolean(pic), el => el.child(div().mr(8).child(pic)))
      .child(div().flex_1().truncate().text_color(on ? t.foreground : t.muted).child(k.name || k.template || k.program?.name || k.address))
      .child(this.status(t, { status: st }, compact));
  }

  // compact: a ! for blocked where a header already says "Needs you".
  status(t, a, compact = false) {
    const color = t[a.status] ?? t.dim;
    if (a.status === "working") return svg("mods/workspace-chrome/spinner.svg").size(12).mx(1).flex_none().text_color(t.working).spin(900);
    if (a.status === "blocked" && compact) return div().size(14).flex_none().rounded(4).bg(t.blocked + "26").text_size(10.5).font_bold().text_color(t.blocked).flex().items_center().justify_center().child("!");
    if (a.status === "blocked") return div().flex_none().px(6).h(18).rounded(5).bg(t.blocked + "26").text_size(10.5).font_semibold().text_color(t.blocked).flex().items_center().child("Needs you");
    // Idle: a ring; done: a dot.
    if (a.status === "idle") return div().size(7).mx(3).flex_none().rounded(4).border(1.5).border_color(t.idle ?? t.dim);
    return div().size(7).mx(3).flex_none().rounded(4).bg(a.status === "done" ? color : t.dim + "00");
  }

  render(cx) {
    const t = theme();
    if (props()?.sortMenu) return this.sortMenu(t);
    const collapsed = narrow();
    const sort = sortOf();
    let list = agents();
    this.listed = new Map(list.map((a) => [a.address, a.status]));
    const prs = shared("capcom.prs");
    this.prsByAgent = new Map();
    for (const [merged, group] of [[false, prs?.open ?? []], [true, prs?.merged ?? []]]) {
      for (const pr of group) {
        const term = pr.agent?.terminal;
        if (term) this.prsByAgent.set(term, [...(this.prsByAgent.get(term) ?? []), { pr, merged }]);
      }
    }
    const all = terminals();
    this.migratePins(all);
    const shown = focused()?.agent;
    const now = Date.now();
    this.track(list, now);
    this.hold(list);
    this.assign(list);
    // A kept sub-agent (one with a pane) is in agents() too: where a tree shows
    // it, it gets no row of its own, unless it's pinned.
    const nested = new Set(collapsed ? [] : list.flatMap((a) => this.tree(all, a.address).map((s) => s.t.address)));
    list = list.filter((a) => a.pinned || !nested.has(a.address));
    const tabCounts = new Map();
    for (const a of list) tabCounts.set(a.tab, (tabCounts.get(a.tab) ?? 0) + 1);
    const byPriority = sort.id === "priority";
    const urgency = new Map(list.map((a) => [a.address, Math.max(this.rank(a), ...this.tree(all, a.address).map((s) => RANK[kidStatus(s.t, this.listed)] ?? 0))]));
    const rankOf = (a) => urgency.get(a.address);
    if (byPriority) list = [...list].sort((x, y) => rankOf(y) - rankOf(x) || this.since(y) - this.since(x) || y.seq - x.seq);
    const pinned = list.filter((a) => a.pinned);
    const rest = list.filter((a) => !a.pinned);
    list = [...pinned, ...rest];
    setAgentOrder(list.map((a) => a.address));

    const withKids = (a, i) => {
      const subs = collapsed ? [] : this.tree(all, a.address);
      const r = row(a, i, subs);
      if (this.folded.has(a.address)) return [r];
      return [r, ...subs.map((s) => this.subRow(t, s, shown, byPriority))];
    };

    // The fold chevron; folded, the count too, colored by the most urgent one.
    const fold = (a, subs) => {
      const sts = subs.map((s) => kidStatus(s.t, this.listed));
      const top = sts.reduce((m, s) => ((RANK[s] ?? 0) > (RANK[m] ?? 0) ? s : m), null);
      const open = !this.folded.has(a.address);
      return div()
        .id(`fold-${a.address}`)
        .h(18)
        .px(3)
        .gap(3)
        .flex()
        .items_center()
        .rounded(4)
        .text_size(11)
        .text_color(open ? t.dim : (t[top] ?? t.dim))
        .cursor_pointer()
        .hover((s) => s.bg(t.hover).text_color(t.foreground))
        .on_click((_e, cx) => {
          cx.stop_propagation();
          open ? this.folded.add(a.address) : this.folded.delete(a.address);
          cx.notify();
        })
        .child(svg(`mods/workspace-chrome/chevron-${open ? "down" : "right"}.svg`).size(11).flex_none())
        .when(!open, (el) => el.child(String(subs.length)));
    };

    // A row reads "space \u00b7 detail": the detail is a tab someone named or the
    // agent's own title, never an automatic tab number unless two rows would
    // otherwise read the same.
    // Read the pane snapshot once: every host read copies the whole list into JS.
    const leads = leadPanes(panes());
    const tabs = new Map(spaces().flatMap(s => s.tabs).map(tab => [tab.id, tab]));
    const labels = new Map(list.map(a => {
      const tab = tabs.get(a.tab);
      const many = tabCounts.get(a.tab) > 1;
      return [a.address, tab ? title(many ? { ...tab, named: false } : tab, many ? a : leads.get(a.tab)) : (a.title || a.program?.name || "")];
    }));
    const label = (a) => labels.get(a.address);
    const detail = new Map(list.map(a => [a.address, /^\d+$/.test(label(a)) ? null : label(a)]));
    const seen = {};
    for (const a of list) { const k = `${a.space_name}\u0000${detail.get(a.address) ?? ""}`; seen[k] = (seen[k] ?? 0) + 1; }
    for (const a of list) {
      if (detail.get(a.address) == null && seen[`${a.space_name}\u0000`] > 1) detail.set(a.address, label(a));
    }

    const row = (a, i, subs = []) => {
      const pic = icon(a.program, 16, t.foreground);
      const who = this.character(a);
      const more = detail.get(a.address);
      // What the agent is on leads the row; the character is only a label under it.
      const task = more ?? (/^\d+$/.test(label(a)) ? a.program?.name || label(a) : label(a));
      // An agent on another machine (`vps/t7`) says which.
      const machine = a.address.includes("/") ? a.address.split("/")[0] : null;
      if (collapsed) {
        return h_flex()
          .id(`ag-${a.address}`)
          .h(26)
          .px(8)
          .flex_none()
          .items_center()
          .gap(4)
          .hover((s) => s.bg(t.hover))
          .on_click(() => run("pane.focus", { pane: a.pane }))
          .child(div().flex_1().min_w(0).whitespace_nowrap().text_right().text_color(t.muted).child(String(i + 1)))
          .child(h_flex().w(14).flex_none().items_center().justify_center().child(this.status(t, a, true)));
      }
      const when = this.since(a);
      const idle = a.status === "idle" && !a.focused;
      const isPinned = a.pinned;
      const pin = div()
        .id(`pin-${a.address}`)
        .flex_none()
        .p(2)
        .rounded(4)
        .text_color(t.dim)
        .cursor_pointer()
        .hover((s) => s.bg(t.hover).text_color(t.foreground))
        .tooltip(isPinned ? "Unpin" : "Pin to the top")
        .on_click((_e, cx) => {
          cx.stop_propagation();
          run("terminal.pin", { terminal: a.address, pinned: !isPinned });
          // The row moves away from under the pointer, and no hover-out comes.
          this.hovered = null;
          cx.notify();
        })
        .child(svg(`mods/workspace-chrome/${isPinned ? "pin-off" : "pin"}.svg`).size(13));
      const copied = this.copied === a.address;
      const copy = div()
        .id(`copy-${a.address}`)
        .flex_none()
        .p(2)
        .rounded(4)
        .text_color(copied ? t.foreground : t.dim)
        .cursor_pointer()
        .hover((s) => s.bg(t.hover).text_color(t.foreground))
        .tooltip(copied ? "Copied" : "Copy this agent's session id")
        .on_click((_e, cx) => {
          cx.stop_propagation();
          this.sessionOf(a.address).then((id) => {
            if (!id) return;
            cx.write_to_clipboard(id);
            this.copied = a.address;
            cx.notify();
            cx.timer.after(1500, (cx) => { if (this.copied === a.address) { this.copied = null; cx.notify(); } });
          });
        })
        .child(svg(`mods/workspace-chrome/${copied ? "check" : "files"}.svg`).size(13));
      // Ends the agent and its sub-agents; a working or blocked one asks with a second click first.
      const asking = this.closing === a.address;
      const close = div()
        .id(`close-${a.address}`)
        .flex_none()
        .p(2)
        .rounded(4)
        .text_size(11)
        .text_color(asking ? t.blocked : t.dim)
        .when(asking, (el) => el.bg(t.blocked + "26").px(5).font_semibold())
        .cursor_pointer()
        .hover((s) => s.bg(t.hover).text_color(asking ? t.blocked : t.foreground))
        .tooltip(asking ? "Click again to end this agent" : "Close this agent")
        .on_click((_e, cx) => {
          cx.stop_propagation();
          if ((a.status === "working" || a.status === "blocked") && !asking) {
            this.closing = a.address;
            cx.timer.after(3000, (cx) => { if (this.closing === a.address) { this.closing = null; cx.notify(); } });
            cx.notify();
            return;
          }
          this.closing = null;
          this.hovered = null;
          run("terminal.kill", { terminal: a.address });
          cx.notify();
        })
        .child(asking ? "Close?" : svg("mods/workspace-chrome/x.svg").size(13));
      // Under a space header the space goes without saying; an automatic tab number says nothing.
      // Without a space header above (priority, pinned) the row names its space.
      const full = byPriority || isPinned;
      const name = full ? a.space_name || label(a) : more ?? (/^\d+$/.test(label(a)) ? a.program?.name || label(a) : label(a));
      return h_flex()
        .id(`ag-${a.address}`)
        .px(8)
        .h(who ? AV + 6 : 30)
        .flex_none()
        .rounded(8)
        .items_center()
        .gap(9)
        .when(a.focused, (el) => el.bg(t.selected))
        .cursor_pointer()
        .hover((s) => s.bg(t.hover))
        .on_hover((on, cx) => {
          if (on) this.hovered = a.address;
          else if (this.hovered === a.address) this.hovered = null;
          cx.notify();
        })
        .on_click(() => run("pane.focus", { pane: a.pane }))
        .when(Boolean(pic) || Boolean(who), el => el.child(div().id(`agent-fold-${a.address}`)
          .when(subs.length > 0, el => el.on_click((_e, cx) => {
            cx.stop_propagation();
            this.folded.has(a.address) ? this.folded.delete(a.address) : this.folded.add(a.address);
            cx.notify();
          })).child(who ? this.avatar(t, a, who) : pic)))
        .child(who
          ? v_flex().flex_1().min_w(0)
            .child(div().min_w(0).truncate().font_semibold().text_color(idle ? t.muted : t.foreground).child(task))
            .child(div().min_w(0).truncate().text_size(11).text_color(t.muted).child([who.label, full ? a.space_name : null].filter(Boolean).join(" \u00b7 ")))
          : h_flex().flex_1().min_w(0).gap(5).items_center()
          .child(div().min_w(0).max_w(full && more ? 120 : 200).truncate().text_color(idle ? t.muted : t.foreground).child(name))
          .when(full && Boolean(more), el => el.child(div().flex_1().min_w(0).truncate().text_color(t.dim).child(`\u00b7 ${more}`))))
        .when(this.prsByAgent.has(a.address), el => el.child(this.prIcons(t, a)))
        .when(Boolean(machine), el => el.child(div().text_size(10).text_color(t.dim).child(machine)))
        .when(subs.length > 0, el => el.child(fold(a, subs)))
        .when(this.hovered === a.address || asking, el => el.child(close))
        .when(this.hovered === a.address || copied, el => el.child(copy))
        .when(this.hovered === a.address, el => el.child(pin))
        .when(this.hovered !== a.address && Boolean(when), el => el.child(div().flex_none().text_size(11).text_color(t.dim).child(duration(now - when))))
        .child(this.status(t, a, byPriority && !isPinned));
    };

    // Groups in list order: spaces (grouped) or priority sections. Agents of a
    // space are next to each other in agents(), so neighbours make a group.
    const groups = pinned.length ? [{ id: "pinned", label: "Pinned", status: null, members: pinned }] : [];
    for (const a of rest) {
      const r = rankOf(a) || RANK.idle;
      const id = byPriority ? `sec-${r}` : `sp-${a.space}`;
      if (groups.at(-1)?.id === id) groups.at(-1).members.push(a);
      else groups.push({ id, label: byPriority ? SECTION[r] : a.space_name || "", status: byPriority ? SECTION_STATUS[r] : null, members: [a] });
    }

    // A group's header; it folds the group. Folded, it says how many and the most urgent state.
    const header = (g) => {
      const open = !this.folded.has(g.id);
      const top = g.members.reduce((m, a) => ((RANK[a.status] ?? 0) > (RANK[m] ?? 0) ? a.status : m), null);
      return h_flex()
        .id(g.id)
        .px(8)
        .pt(8)
        .h(30)
        .flex_none()
        .gap(6)
        .items_center()
        .text_size(11)
        .font_medium()
        .cursor_pointer()
        .hover((s) => s.text_color(t.foreground))
        .on_click((_e, cx) => {
          open ? this.folded.add(g.id) : this.folded.delete(g.id);
          cx.notify();
        })
        .child(div().truncate().text_color(t[g.status] ?? t.dim).child(g.label))
        .when(byPriority || !open, (el) => el.child(div().text_color(t.dim).child(String(g.members.length))))
        .child(div().flex_1())
        .when(!open, (el) => el
          .when(!byPriority && top && top !== "idle", (el) => el.child(this.status(t, { status: top }, true)))
          .child(svg("mods/workspace-chrome/chevron-right.svg").size(11).flex_none().text_color(t.dim)));
    };
    const rows = collapsed
      ? list.flatMap(withKids)
      : groups.flatMap((g) => [header(g), ...(this.folded.has(g.id) ? [] : g.members.flatMap(withKids))]);

    return v_flex()
      .size_full()
      .min_h(0)
      .bg(t.chrome)
      .border_t(1)
      .border_color(t.border)
      .text_size(13)
      .when(!collapsed, (el) =>
        el.child(
          h_flex()
            .px(16)
            .h(41)
            .pt(14)
            .pb(5)
            .flex_none()
            .items_center()
            .justify_between()
            .child(div().text_size(11.5).font_medium().text_color(t.dim).child("Agents"))
            .child(
              div()
                .id("ag-sort")
                .text_size(11)
                .text_color(t.dim)
                .hover((s) => s.text_color(t.foreground))
                .on_click(() => openOverlay({ mod: "capcom-agents", kind: "popup", width: 200, height: 140, props: { sortMenu: true } }))
                .tooltip(`Sort agents: ${sort.label}`)
                .child(svg(`mods/workspace-chrome/${sort.icon}.svg`).size(14)),
            ),
        ),
      )
      .when(list.length === 0 && !collapsed, (el) =>
        el.child(div().px(12).text_size(12).text_color(t.dim).child("No agent status yet. Agents show up here when they report their status. Older versions and agents inside tmux may not.")),
      )
      .child(v_flex().id("ag-list").flex_1().min_h(0).px(collapsed ? 0 : 8).overflow_y_scrollbar().mode("always").children(rows))
      .when(collapsed, (el) =>
        el.child(
          h_flex()
            .h(28)
            .items_center()
            .justify_center()
            .child(
              div()
                .id("collapse")
                .text_size(13)
                .text_color(t.dim)
                .hover((s) => s.text_color(t.foreground))
                .on_click(toggleSidebar)
                .child("»"),
            ),
        ),
      );
  }
}
