// Agents in grouped or priority views; parents take their most urgent sub-agent's place.
// A viewed done agent stays in Done until left.

import { View, div, image, svg } from "gpui-kit";
import { v_flex, h_flex, HoverCard, Avatar, AvatarImage, AvatarFallback } from "gpui-base";
import * as fs from "fs/promises";
import { appHome, agents, terminals, panes, focused, spaces, run, exec, theme, shared, setShared, setAgentOrder, openOverlay, closeOverlay, props, window, layout } from "herdr";
import { SH, dotColor, short, openUrl, prDo, prDetails, needsYou, attentionIcon } from "./pr-details.js";
import { BALL_FILE, BALL_URL, EXPRESSIONS, nameOf, assignable, pickPokemon, expressionFor, portraitFile, portraitUrls, typeColor, stageColor } from "./pokemon.js";
import { who as whoIs } from "./identity.js";

function find(node, id) {
  if (node?.id === id) return node;
  for (const child of node?.children ?? []) {
    const found = find(child, id);
    if (found) return found;
  }
  return null;
}

// The sidebar's rail goes by its width, not a flag: dragging its edge narrows it too.
const sidebarPx = () => find(layout(), "sidebar")?.size?.px ?? Infinity;
const TIGHT = 240;
const narrow = () => (find(layout(), "sidebar")?.size?.px ?? Infinity) < 160;

// To the rail (on macOS wide enough for the traffic lights), or back to the width it had.
function toggleSidebar() {
  const width = find(layout(), "sidebar")?.size?.px;
  if (!narrow()) setShared("sidebar.width", width);
  // Mods edited before the rail went by width still read the flag.
  setShared("sidebar.collapsed", !narrow());
  run("layout.resize", { id: "sidebar", px: narrow() ? (shared("sidebar.width") ?? 224) : window()?.platform === "macos" ? 76 : 44 });
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
// The Claude Code session name (what /rename sets; else its generated title) is the terminal's title.
const sessionNameOf = (a) => {
  const live = clean(a.title);
  return live && live !== a.program?.name && !/^[~/]|^[A-Za-z]:\\/.test(live) && !live.includes("@") && live.toLowerCase() !== a.program?.key && !/^claude( code)?$/i.test(live) ? live : "";
};
// Blocked, or done and not yet looked at (looking turns done into idle): it is waiting on you.
const wantsYou = (a) => a.status === "blocked" || a.status === "done";
const SHAKE_DX = [-3, 0, 3, 0, -3, 0, 3, 0, -3, 0, 3, 0];
const SHAKE_REST = 10;
const SHAKE_FRAMES = SHAKE_REST + SHAKE_DX.length;
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

// The agent's icon from the app's program table: a one-color glyph or a picture.
function icon(p, size, color) {
  if (!p?.icon) return null;
  return p.tint ? svg(p.icon).size(size).flex_none().text_color(color) : image(p.icon).w(size).h(size).flex_none();
}

const AV = 44;

// "claude-opus-5-5" -> { label: "Opus 5.5", color } ; date suffixes dropped.
const MODEL_COLORS = { opus: "#c58af9", sonnet: "#6cb6ff", haiku: "#57d9a3", fable: "#f2a65a" };
function modelInfo(id) {
  if (!id) return null;
  const parts = id.replace(/^claude-/, "").split("-").filter((x) => !/^\d{8}$/.test(x));
  const family = parts.find((x) => /^[a-z]+$/i.test(x));
  if (!family) return { label: id, color: null };
  const version = parts.filter((x) => /^\d+$/.test(x)).join(".");
  const name = family[0].toUpperCase() + family.slice(1);
  return { label: version ? `${name} ${version}` : name, color: MODEL_COLORS[family.toLowerCase()] ?? null };
}

// Session colours: each live session has its own, the first free one, kept until it ends.
const COLORS = ["#e8a24a", "#4aa3e8", "#b878f2", "#4fcf8e", "#ef6f9a", "#3fd0c4", "#e8d24a", "#7c8cff", "#ef7a4a", "#9bd44a", "#d65ad6", "#5ab0a0"];

export default class Agents extends View {
  init(_props, cx) {
    if (props()?.sortMenu) return;
    this.folded = new Set();
    // Each agent's last state and when it changed (ms), kept across restarts.
    this.changed = JSON.parse(localStorage.getItem("changed") ?? "{}");
    this.pendingPins = JSON.parse(localStorage.getItem("pins") ?? "[]");
    if (!this.pendingPins.length) localStorage.removeItem("pins");
    this.migratingPins = new Set();
    this.hues = JSON.parse(localStorage.getItem("hues") ?? "{}");
    let stored = {};
    try { stored = JSON.parse(localStorage.getItem("pokemon") ?? "{}") ?? {}; } catch {}
    this.pokemon = Object.fromEntries(Object.entries(stored).filter(([, n]) => assignable(n)));
    this.lastPokemon = Number(localStorage.getItem("pokemonLast")) || 0;
    this.cached = new Set();
    this.wanted = new Map();
    this.fetching = new Set();
    this.failed = new Map();
    this.scanned = false;
    this.scanPortraits(cx);
    cx.timer.every(700, (cx) => this.fetchWanted(cx));
    this.shake = 0;
    this.waiting = false;
    cx.timer.every(110, (cx) => {
      if (!this.waiting) return;
      this.shake = (this.shake + 1) % SHAKE_FRAMES;
      cx.notify();
    });
    this.pulse = true;
    cx.timer.every(700, (cx) => {
      if (![...this.prsByAgent.values()].some((list) => list.some(({ pr, merged }) => !merged && needsYou(pr)))) return;
      this.pulse = !this.pulse;
      cx.notify();
    });
    cx.timer.every(30000, (cx) => cx.notify());
    this.models = {};
    this.refreshNotes(cx);
    cx.timer.every(10000, (cx) => this.refreshNotes(cx));
  }

  // Each agent's note (`capcom note`), shared as `capcom.notes` so the pane header and tabs can show the task too.
  refreshNotes(cx) {
    cx.spawn(async () => {
      const found = {};
      const models = {};
      await Promise.all(agents().map(async (a) => {
        const [note, model] = await Promise.all([this.noteOf(a.address), this.modelOf(a.address)]);
        if (note?.task || note?.progress) found[a.address] = { task: note.task, progress: note.progress, waiting: note.waiting, needs_you: note.needs_you };
        if (model) models[a.address] = model;
      }));
      if (JSON.stringify(models) !== JSON.stringify(this.models)) {
        this.models = models;
        cx.notify();
      }
      const text = JSON.stringify(found);
      if (text !== this.notesText) {
        this.notesText = text;
        setShared("capcom.notes", found);
      }
    });
  }

  // The note file of the agent's Claude Code session (from its terminal's foreground process, the `claude` that wrote ~/.claude/sessions/<pid>.json), or null.
  async noteOf(terminal) {
    const cmd = 'pid=$(hc get "$1" | sed -n \'s/.*"foreground":{[^}]*"pid":\\([0-9]*\\).*/\\1/p\'); '
      + 's=$(sed -n \'s/.*"sessionId":"\\([^"]*\\)".*/\\1/p\' "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/sessions/$pid.json"); '
      + '[ -n "$s" ] && cat "${CAPCOM_NOTES_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/capcom/notes}/$s.json" 2>/dev/null';
    try {
      return JSON.parse((await exec(SH[0], [...SH.slice(1), "sh", "-c", cmd, "sh", terminal], { timeout: 5000 })).stdout);
    } catch {
      return null;
    }
  }

  // The model of the session's last main-thread reply, from its transcript, or "".
  async modelOf(terminal) {
    const cmd = 'pid=$(hc get "$1" | sed -n \'s/.*"foreground":{[^}]*"pid":\\([0-9]*\\).*/\\1/p\'); '
      + 's=$(sed -n \'s/.*"sessionId":"\\([^"]*\\)".*/\\1/p\' "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/sessions/$pid.json"); '
      + '[ -n "$s" ] || exit 0; '
      + 'f=$(ls "${CLAUDE_CONFIG_DIR:-$HOME/.claude}"/projects/*/"$s.jsonl" 2>/dev/null | head -1); '
      + '[ -n "$f" ] && tac "$f" | grep \'"type":"assistant"\' | grep -v \'"isSidechain":true\' | grep -o \'"model":"[^"<]*"\' | head -1 | cut -d\\" -f4';
    try {
      return (await exec(SH[0], [...SH.slice(1), "sh", "-c", cmd, "sh", terminal], { timeout: 5000 })).stdout.trim();
    } catch {
      return "";
    }
  }

  // Portraits already downloaded for Pokemon mode: "0004/Worried.png" and so on.
  scanPortraits(cx) {
    cx.spawn(async (cx) => {
      try {
        const root = `${appHome()}/data/capcom-agents/pokemon`;
        for (const dir of await fs.readdir(root)) {
          if (dir.endsWith(".png")) { this.cached.add(dir); continue; }
          try {
            for (const f of await fs.readdir(`${root}/${dir}`)) if (f.endsWith(".png")) this.cached.add(`${dir}/${f}`);
          } catch {}
        }
      } catch {}
      this.scanned = true;
      cx.notify();
    });
  }

  // Each agent keeps one Pokemon for good, chosen at random from those nobody on screen has.
  assignPokemon(list) {
    const live = new Set(list.map((a) => a.address));
    const taken = new Set(list.map((a) => this.pokemon[a.address]).filter(Boolean));
    let dirty = false;
    for (const a of list) {
      if (this.pokemon[a.address]) continue;
      const n = pickPokemon(taken, this.lastPokemon);
      this.lastPokemon = n;
      localStorage.setItem("pokemonLast", String(n));
      this.pokemon[a.address] = n;
      taken.add(n);
      dirty = true;
    }
    for (const k of Object.keys(this.pokemon)) if (!live.has(k) && !this.changed[k]) { delete this.pokemon[k]; dirty = true; }
    if (dirty) localStorage.setItem("pokemon", JSON.stringify(this.pokemon));
  }

  // The portrait's path under the app home if it's downloaded; otherwise note that it's wanted (fetchWanted downloads it)
  // and say null, so the row shows a Poke Ball meanwhile.
  wantPortrait(n, expression) {
    const file = portraitFile(n, expression);
    if (this.cached.has(file)) return `data/capcom-agents/pokemon/${file}`;
    this.wanted.set(file, { urls: portraitUrls(n, expression) });
    return null;
  }

  // Rocks side to side with a tilt, three times, then rests, so the sprite-less ball reads as a catch attempt.
  shakingBall(path, size = 40) {
    const f = this.shake % SHAKE_FRAMES;
    const i = f - SHAKE_REST;
    const dx = i >= 0 ? SHAKE_DX[i % SHAKE_DX.length] : 0;
    return div().w(size + 14).h(size + 6).flex_none().flex().items_center().justify_center()
      .child(image(path).w(size).h(size).flex_none().ml(dx * 2).mt(Math.abs(dx)));
  }

  ball() {
    if (this.cached.has(BALL_FILE)) return `data/capcom-agents/pokemon/${BALL_FILE}`;
    this.wanted.set(BALL_FILE, { urls: [BALL_URL] });
    return null;
  }

  // The agent's Pokemon: the portrait for its status, else the Normal one while that downloads
  // (every expression is asked for at once), else a Poke Ball.
  pokemonFace(a) {
    const n = this.pokemon[a.address];
    if (!n) return null;
    const want = this.wantPortrait(n, expressionFor(a.status));
    for (const e of EXPRESSIONS) this.wantPortrait(n, e);
    const path = want ?? this.wantPortrait(n, "Normal");
    return { path: path ?? this.ball(), n, name: nameOf(n) };
  }

  // Download what rows asked for, one curl per file at a time, and not again for five minutes after a failure.
  fetchWanted(cx) {
    if (!this.scanned || !this.wanted.size) return;
    const todo = [...this.wanted].filter(([file]) => !this.cached.has(file) && !this.fetching.has(file) && !(Date.now() - (this.failed.get(file) ?? 0) < 300000));
    this.wanted.clear();
    for (const [file, { urls }] of todo) {
      this.fetching.add(file);
      const dest = `${appHome()}/data/capcom-agents/pokemon/${file}`;
      cx.spawn(async (cx) => {
        try {
          // A similar expression's portrait (or Normal) is kept under the missing one's name.
          const cmd = 'mkdir -p "$(dirname "$1")" && d=$1 && shift && for u in "$@"; do curl -fsSL --max-time 15 -o "$d.part" "$u" && mv "$d.part" "$d" && exit 0; done; rm -f "$d.part"; exit 1';
          const r = await exec(SH[0], [...SH.slice(1), "sh", "-c", cmd, "sh", dest, ...urls], { timeout: 90000 });
          if (r.code !== 0) throw new Error(`curl exited ${r.code}`);
          this.cached.add(file);
          this.failed.delete(file);
        } catch {
          this.failed.set(file, Date.now());
        }
        this.fetching.delete(file);
        cx.notify();
      });
    }
  }

  // Past twelve sessions a colour repeats with a number (2nd use of colour 1 is "1#2"), so a clash shows.
  assignColors(list) {
    const live = new Set(list.map((a) => a.address));
    const taken = new Set(list.map((a) => this.hues[a.address]).filter(Boolean));
    let dirty = false;
    for (const a of list) {
      if (this.hues[a.address]) continue;
      let round = 1;
      let pick;
      while (!pick) {
        pick = COLORS.map((_, i) => (round === 1 ? String(i) : `${i}#${round}`)).find((id) => !taken.has(id));
        round++;
      }
      this.hues[a.address] = pick;
      taken.add(pick);
      dirty = true;
    }
    for (const k of Object.keys(this.hues)) if (!live.has(k) && !this.changed[k]) { delete this.hues[k]; dirty = true; }
    if (dirty) localStorage.setItem("hues", JSON.stringify(this.hues));
  }

  // The focused pane's frame takes its session's colour (the layout's `focus` border is one colour for every pane,
  // so it is rewritten when the focus moves). A shell or a non-agent pane gets the layout's own colour back.
  frameColor(color) {
    const root = layout();
    const pane = find(root, "main")?.content?.pane;
    if (!pane) return;
    const want = color ?? "accent";
    if (pane.focus?.color === want) return;
    const next = JSON.parse(JSON.stringify(root));
    find(next, "main").content.pane.focus = { width: color ? 3 : 1, color: want };
    run("layout.set", { layout: next });
  }

  hue(a) {
    const mon = stageColor(this.pokemon[a.address]);
    if (mon) return { color: mon, n: 0 };
    const [i, n] = (this.hues[a.address] ?? "").split("#");
    return COLORS[Number(i)] ? { color: COLORS[Number(i)], n: n ? Number(n) : 0 } : null;
  }

  pokemonAvatar(path, size = AV) {
    const av = Avatar.new().size(size).flex_none().rounded(8).overflow_hidden().bg("#2a2f37");
    return (path ? av.image(AvatarImage.new(path).size_full()) : av)
      .fallback(AvatarFallback.new().size_full().bg("#2a2f37"));
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
        const color = dotColor(pr, merged, t, this.pulse);
        const wants = !merged && needsYou(pr);
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
          .child(wants ? attentionIcon("mods/workspace-chrome/git-compare.svg", 11, 17, this.pulse) : svg("mods/workspace-chrome/git-compare.svg").size(13).flex_none().text_color(color));
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
    const tight = !collapsed && sidebarPx() < TIGHT;
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
    this.assignPokemon(list);
    this.assignColors(list);
    this.waiting = !collapsed && list.some((a) => wantsYou(a) && this.pokemon[a.address]);
    this.frameColor(this.hue(list.find((a) => a.focused) ?? {})?.color);
    const persona = Object.fromEntries(list.map((a) => {
      const mon = this.pokemonFace(a);
      return [a.address, { label: mon?.name ?? a.program?.name ?? "", pic: mon?.path ?? "", ...this.hue(a) }];
    }));
    const personaText = JSON.stringify(persona);
    if (personaText !== this.personaText) {
      this.personaText = personaText;
      setShared("capcom.persona", persona);
    }
    const notes = shared("capcom.notes") ?? {};
    const paneOf = new Map(panes().map((p) => [p.pane, p]));
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
      const mon = this.pokemonFace(a);
      const more = detail.get(a.address);
      const model = modelInfo(this.models[a.address]);
      const hasPrs = this.prsByAgent.has(a.address);
      // What the agent is on leads the row; its Pokemon is only a label under it.
      const note = notes[a.address];
      const sessionName = sessionNameOf(a);
      const asked = note?.task || (more ?? (/^\d+$/.test(label(a)) ? a.program?.name || label(a) : label(a)));
      const task = sessionName || asked;
      const id = whoIs({ task: note?.task, branch: paneOf.get(a.pane)?.branch, tab: a.tab_name, space: a.space_name });
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
      const idle = a.status === "idle" && !a.focused;
      // The selected agent: the row takes its Pokemon's type colour, else the theme's, and holds it.
      const alert = a.focused ? (mon && typeColor(mon.n)) || t.blocked : null;
      // Waiting on you (blocked, or done and unread): a Poke Ball that shakes like one catching a Pokemon.
      const ball = wantsYou(a) && mon ? this.ball() : null;
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
      const AVS = tight ? 26 : 32;
      const rowH = mon ? (tight ? 40 : 46) : AV + (model || hasPrs ? 14 : 6);
      const showActions = this.hovered === a.address || asking;
      return h_flex()
        .id(`ag-${a.address}`)
        .px(8)
        .h(rowH)
        .flex_none()
        .rounded(8)
        .items_center()
        .gap(9)
        .border(1.5)
        .border_color(alert ? alert : "#00000000")
        .when(a.focused, (el) => el.bg(t.selected))
        .when(Boolean(alert), (el) => el.bg(alert + "40"))
        .cursor_pointer()
        .hover((s) => s.bg(alert ? alert + "38" : t.hover))
        .on_hover((on, cx) => {
          if (on) this.hovered = a.address;
          else if (this.hovered === a.address) this.hovered = null;
          cx.notify();
        })
        .on_click(() => run("pane.focus", { pane: a.pane }))
        .child(div().w(alert ? 5 : 3).h(mon ? rowH - 14 : AV - 8).flex_none().rounded(2).bg(this.hue(a)?.color ?? "#00000000"))
        .when(Boolean(pic) || Boolean(mon), el => el.child(div().id(`agent-fold-${a.address}`)
          .when(subs.length > 0, el => el.on_click((_e, cx) => {
            cx.stop_propagation();
            this.folded.has(a.address) ? this.folded.delete(a.address) : this.folded.add(a.address);
            cx.notify();
          })).child(mon ? this.pokemonAvatar(mon.path, AVS) : pic)))
        .child(mon
          ? v_flex().flex_1().min_w(0)
            .child(div().id(`nm-${a.address}`).min_w(0).truncate().font_semibold().text_color(idle ? t.muted : t.foreground).tooltip([asked && sessionName ? `Asked: ${asked}` : "", a.space_name, id.branch, model?.label].filter(Boolean).join(" \u00b7 ") || task).child(task))
            .child(h_flex().min_w(0).gap(5).text_size(11).text_color(t.muted)
              .child(div().id(`mon-${a.address}`).flex_none().cursor_pointer().text_color(copied ? t.foreground : t.muted)
                .hover((s) => s.text_color(t.foreground)).tooltip(copied ? "Copied" : `Click to copy "${mon.name} (${a.address})"`)
                .on_click((_e, cx) => {
                  cx.stop_propagation();
                  cx.write_to_clipboard(`${mon.name} (${a.address})`);
                  this.copied = a.address;
                  cx.notify();
                  cx.timer.after(1500, (cx) => { if (this.copied === a.address) { this.copied = null; cx.notify(); } });
                }).child(copied ? "Copied" : mon.name))
              .when(Boolean(id.key), el => el.child(div().flex_none().font_semibold().text_color(t.muted).child(id.key)))
              .when(Boolean(id.branch) && !tight, el => el.child(div().flex_1().min_w(0).truncate().text_color(t.dim).child(id.branch)))
              .when(hasPrs && !tight, el => el.child(div().flex_none().ml_auto().child(this.prIcons(t, a)))))
          : h_flex().flex_1().min_w(0).gap(5).items_center()
          .child(div().min_w(0).max_w(full && more ? 120 : 200).truncate().text_color(idle ? t.muted : t.foreground).child(name))
          .when(full && Boolean(more), el => el.child(div().flex_1().min_w(0).truncate().text_color(t.dim).child(`\u00b7 ${more}`))))
        .when(Boolean(machine), el => el.child(div().text_size(10).text_color(t.dim).child(machine)))
        .when(subs.length > 0, el => el.child(fold(a, subs)))
        .when(Boolean(ball), (el) => el.child(this.shakingBall(ball, tight ? 24 : 30)))
        .when(!ball && !alert, (el) => el.child(this.status(t, a, byPriority && !isPinned)))
        .when(showActions, el => el.child(h_flex().absolute().top(2).right(3).gap(2).rounded(5).bg(t.background).child(pin).child(close)));
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
              h_flex().gap(10).items_center()
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
