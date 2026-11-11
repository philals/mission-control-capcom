// The Jira tickets assigned to you, beside the CAPCOM board. Reads jira-me.json, which `capcom-jira-me` writes
// (it asks Claude's Atlassian connector). Fetches when the pane opens (if the file is over 5 minutes old) and every
// 5 minutes while it is open, so closing the pane stops it.

import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { run, theme, home, readTextFile, spaces, exec } from "herdr";
import { capcom, openUrl, SH } from "./pr-details.js";

const FETCH_MS = 5 * 60 * 1000;
const TICK_MS = 30 * 1000;
const COLOR = { progress: "#6cb6ff", review: "#c792ea", blocked: "#ff6b6b", todo: "#7c8493", other: "#9aa1ad" };
const RANK = { progress: 0, review: 1, blocked: 2, todo: 3, other: 4 };
const TASK_COLOR = { todo: "#7c8493", planning: "#6cb6ff", planned: "#c792ea", implementing: "#f0b429", done: "#22c55e", dropped: "#5c6370" };
const BOARD = "#c792ea";

const kindOf = (i) => {
  const s = i.status.toLowerCase();
  if (s.includes("block")) return "blocked";
  if (s.includes("review")) return "review";
  if (i.statusCategory === "In Progress") return "progress";
  if (i.statusCategory === "To Do") return "todo";
  return "other";
};

const branchOf = (i) => {
  const slug = i.summary.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").split("-").slice(0, 6).join("-");
  return `${i.type === "Bug" ? "fix" : "feat"}/${i.key}-${slug}`;
};

const ago = (iso) => {
  if (!iso) return "never";
  const m = Math.max(0, Math.round((Date.now() - Date.parse(iso)) / 60000));
  return m < 1 ? "just now" : m < 60 ? `${m} min ago` : `${Math.round(m / 60)} h ago`;
};

export default class CapcomJira extends View {
  init(_props, cx) {
    this.issues = [];
    this.fetchedAt = null;
    this.stories = new Map();
    this.details = new Map();
    this.open = null;
    this.loading = false;
    this.error = null;
    this.message = null;
    this.file = `${home()}/.cache/capcom/jira-me.json`;
    cx.timer.every(FETCH_MS, (cx) => this.fetch(cx));
    cx.timer.every(TICK_MS, (cx) => this.reload(cx));
    cx.spawn(async (cx) => {
      await this.readCache();
      await this.loadBoards();
      cx.notify();
      if (!this.fetchedAt || Date.now() - Date.parse(this.fetchedAt) > FETCH_MS) this.fetch(cx);
    });
  }

  async root() {
    if (this.rootDir !== undefined) return this.rootDir;
    try {
      this.rootDir = JSON.parse(await readTextFile(`${home()}/.claude/settings.json`))["env"]?.CAPCOM_ROOT ?? null;
    } catch (_e) {
      this.rootDir = null;
    }
    return this.rootDir;
  }

  async json(args) {
    const root = await this.root();
    const r = await capcom(root ? [...args, "--root", root] : args, { timeout: 60000 });
    if (r.code !== 0) throw new Error((r.stderr || r.stdout).trim().split("\n").pop() || `capcom exited ${r.code}`);
    return JSON.parse(r.stdout);
  }

  async readCache() {
    try {
      const data = JSON.parse(await readTextFile(this.file));
      this.issues = data.issues ?? [];
      this.fetchedAt = data.fetchedAt ?? null;
    } catch (_e) {
      // no file yet
    }
  }

  async loadBoards() {
    try {
      this.stories = new Map((await this.json(["list"])).map((s) => [s.key, s]));
      for (const i of this.issues) {
        if (!this.stories.has(i.key)) {
          this.details.delete(i.key);
          continue;
        }
        const [board, ready] = await Promise.all([this.json(["show", i.key]), this.json(["ready", i.key]).catch(() => [])]);
        this.details.set(i.key, { board, ready: new Set((ready.ready ?? []).map((x) => x.id ?? x)) });
      }
    } catch (_e) {
      // the board tags are optional
    }
  }

  reload(cx) {
    cx.spawn(async (cx) => {
      await this.readCache();
      await this.loadBoards();
      cx.notify();
    });
  }

  fetch(cx) {
    if (this.loading) return;
    this.loading = true;
    this.error = null;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await exec(SH[0], [...SH.slice(1), "capcom-jira-me"], { timeout: 180000 });
        if (r.code !== 0 && r.code !== 75) throw new Error((r.stderr || r.stdout).trim().split("\n").pop() || `capcom-jira-me exited ${r.code}`);
        await this.readCache();
        await this.loadBoards();
      } catch (e) {
        this.error = String(e?.message ?? e);
      }
      this.loading = false;
      cx.notify();
    });
  }

  say(cx, text, ok = true) {
    this.message = { text, ok };
    cx.notify();
    cx.timer.after(2500, (cx) => {
      if (this.message?.text === text) {
        this.message = null;
        cx.notify();
      }
    });
  }

  copy(cx, what, text) {
    cx.write_to_clipboard(text);
    this.say(cx, `Copied ${what}`);
  }

  start(cx, key, skill, task, folder) {
    const argv = ["claude", `/${skill} ${key}${task ? ` ${task.id}` : ""}`];
    const name = `${key} ${skill === "story-plan" ? "plan" : skill === "story-review" ? "review" : "impl"}`;
    const dir = task?.folder || folder;
    if (!dir) run("tab.new", { name, argv });
    else {
      const mine = spaces().find((s) => !s.machine && s.folder && (s.folder === dir || s.folder.startsWith(`${dir}/`)));
      if (mine) {
        run("space.select", { id: mine.id });
        run("tab.new", { space: mine.id, name, folder: dir, argv });
      } else run("space.new", { folder: dir, argv });
    }
    this.say(cx, `${skill} ${key} starting in a new tab`);
  }

  // The skills that make sense for this ticket now, the first being the main one.
  actions(i) {
    const d = this.details.get(i.key);
    if (!this.stories.has(i.key)) {
      return [
        { label: "Plan", full: "Plan the story", cmd: `/story-plan ${i.key}`, go: (cx) => this.start(cx, i.key, "story-plan") },
        { label: "Fix", full: "Implement directly (small bug or change)", cmd: `/story-implement-task ${i.key}`, go: (cx) => this.start(cx, i.key, "story-implement-task"), minor: true },
      ];
    }
    const out = [];
    const tasks = d?.board?.tasks ?? [];
    const finished = tasks.length > 0 && tasks.every((x) => x.status === "done" || x.status === "dropped") && tasks.some((x) => x.status === "done");
    const next = tasks.find((x) => x.status === "implementing") ?? tasks.find((x) => x.status === "planned" && d?.ready.has(x.id));
    if (finished) out.push({ label: "Review", full: "Review the story", cmd: `/story-review ${i.key}`, go: (cx) => this.start(cx, i.key, "story-review") });
    if (next) {
      const resume = next.status === "implementing";
      out.push({ label: `${resume ? "Resume" : "Do"} ${next.id}`, full: `${resume ? "Resume" : "Implement"} ${next.id}`, cmd: `/story-implement-task ${i.key} ${next.id}`, go: (cx) => this.start(cx, i.key, "story-implement-task", next) });
    }
    out.push({ label: "Plan", full: "Re-plan the story", cmd: `/story-plan ${i.key}`, go: (cx) => this.start(cx, i.key, "story-plan"), minor: true });
    return out;
  }

  chip(id, label, tip, onClick, t) {
    return div().id(id).flex_none().px(6).py(2).rounded(4).bg(t.hover).text_size(11).text_color(t.muted)
      .cursor_pointer().hover((s) => s.bg(t.selected).text_color(t.foreground)).tooltip(tip)
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .on_click((_e, cx) => {
        cx.stop_propagation();
        onClick(cx);
      })
      .child(label);
  }

  line(id, label, value, onClick, t, accent) {
    return h_flex().id(id).gap(10).px(8).py(5).rounded(5).items_center().cursor_pointer()
      .hover((s) => s.bg(t.hover)).text_color(accent ?? t.foreground).text_size(12)
      .on_click((_e, cx) => onClick(cx))
      .child(div().flex_1().min_w(0).truncate().child(label))
      .child(div().flex_none().max_w(190).truncate().text_size(11).text_color(t.dim).child(value));
  }

  heading(label, t) {
    return div().px(8).pt(8).pb(2).text_size(10).font_semibold().text_color(t.dim).child(label);
  }

  details_panel(i, t) {
    const d = this.details.get(i.key);
    const tasks = d?.board?.tasks ?? [];
    const acts = this.actions(i);
    const counts = Object.keys(TASK_COLOR).map((s) => [s, tasks.filter((x) => x.status === s).length]).filter(([, n]) => n);
    return v_flex().gap(2).pb(6).border_t(1).border_color(t.border)
      .child(
        v_flex().gap(5).px(10).py(8)
          .child(div().text_size(10).font_semibold().text_color(t.dim).child(`${i.type.toUpperCase()}${i.priority ? ` · ${i.priority.toUpperCase()}` : ""} · UPDATED ${ago(i.updated).toUpperCase()}`))
          .when(tasks.length > 0, (el) =>
            el.child(h_flex().gap(3).children(tasks.map((x) => div().flex_1().h(6).rounded(3).bg(TASK_COLOR[x.status]))))
              .child(div().text_size(11).text_color(t.muted).child(counts.map(([s, n]) => `${n} ${s}`).join(" · ")))),
      )
      .child(this.heading("COPY", t))
      .child(this.line(`cp-id-${i.key}`, "Copy ID", i.key, (cx) => this.copy(cx, i.key, i.key), t))
      .child(this.line(`cp-url-${i.key}`, "Copy URL", i.url.replace(/^https?:\/\//, ""), (cx) => this.copy(cx, "the URL", i.url), t))
      .child(this.line(`cp-title-${i.key}`, "Copy ID and title", `${i.key}: ${i.summary}`, (cx) => this.copy(cx, "ID and title", `${i.key}: ${i.summary}`), t))
      .child(this.line(`cp-branch-${i.key}`, "Copy branch name", branchOf(i), (cx) => this.copy(cx, "the branch name", branchOf(i)), t))
      .child(this.heading("OPEN", t))
      .child(this.line(`open-${i.key}`, "Open in Jira", "↗", () => openUrl(i.url), t))
      .child(this.heading("START IN A NEW TAB", t))
      .children(acts.map((a, n) => this.line(`go-${i.key}-${n}`, `▶ ${a.full}`, a.cmd, a.go, t, n === 0 && !a.minor ? "#9fc8ff" : undefined)));
  }

  card(i, color, t) {
    const open = this.open === i.key;
    const story = this.stories.get(i.key);
    const d = this.details.get(i.key);
    const tasks = d?.board?.tasks ?? [];
    const done = tasks.filter((x) => x.status === "done").length;
    const main = this.actions(i).find((a) => !a.minor);
    return v_flex().id(`jira-${i.key}`).rounded(6).bg(t.raised).border(1).border_color(open ? color : t.border)
      .hover((s) => s.bg(t.hover)).cursor_pointer()
      .on_click((_e, cx) => {
        this.open = open ? null : i.key;
        cx.notify();
      })
      .child(
        v_flex().gap(4).px(10).py(7)
          .child(
            h_flex().gap(8).items_start()
              .child(div().flex_none().pt(1).text_size(11).font_semibold().text_color(color).child(i.key))
              .child(div().flex_1().min_w(0).text_size(12).text_color(t.foreground).child(i.summary)),
          )
          .child(
            h_flex().gap(6).items_center().text_size(11).text_color(t.muted)
              .child(div().child(i.type))
              .when(Boolean(story), (el) => el.child(div().px(5).rounded(4).border(1).border_color(t.border).text_color(BOARD).child(tasks.length ? `Capcom ${done}/${tasks.length}` : "Capcom")))
              .child(div().flex_1())
              .child(this.chip(`id-${i.key}`, "ID", "Copy the ticket ID", (cx) => this.copy(cx, i.key, i.key), t))
              .child(this.chip(`url-${i.key}`, "URL", "Copy the ticket URL", (cx) => this.copy(cx, "the URL", i.url), t))
              .child(this.chip(`web-${i.key}`, "↗", "Open in Jira", () => openUrl(i.url), t))
              .when(Boolean(main), (el) => el.child(this.chip(`main-${i.key}`, `▶ ${main.label}`, `${main.full}: ${main.cmd}`, main.go, t))),
          ),
      )
      .when(open, (el) => el.child(this.details_panel(i, t)));
  }

  section(name, kind, issues, t) {
    return v_flex().id(`js-${name}`).gap(5)
      .child(
        h_flex().gap(6).px(6).py(4).items_center()
          .child(div().size(8).rounded(4).flex_none().bg(COLOR[kind]))
          .child(div().text_size(11).font_semibold().text_color(COLOR[kind]).child(name.toUpperCase()))
          .child(div().text_size(11).text_color(t.dim).child(String(issues.length))),
      )
      .child(v_flex().gap(5).px(4).pb(8).children(issues.map((i) => this.card(i, COLOR[kind], t))));
  }

  render(_cx) {
    const t = theme();
    const groups = new Map();
    for (const i of this.issues) {
      const g = groups.get(i.status) ?? { kind: kindOf(i), issues: [] };
      g.issues.push(i);
      groups.set(i.status, g);
    }
    const ordered = [...groups].sort(([a, x], [b, y]) => RANK[x.kind] - RANK[y.kind] || a.localeCompare(b));
    const status = this.error
      ? div().px(10).py(4).text_size(11).text_color("#ff1f1f").child(this.error)
      : this.message
        ? div().px(10).py(4).text_size(11).text_color(this.message.ok ? "#22c55e" : "#ffb000").child(this.message.text)
        : null;
    return v_flex().id("jira-root").size_full().bg(t.surface)
      .child(
        h_flex().gap(6).px(10).pt(10).pb(6).items_center().flex_none()
          .child(div().text_size(13).font_semibold().text_color(t.foreground).child("JIRA · ME"))
          .child(div().text_size(11).text_color(t.dim).child(`${this.issues.length} open`))
          .child(div().flex_1())
          .child(div().text_size(11).text_color(t.dim).child(this.loading ? "updating…" : ago(this.fetchedAt)))
          .child(
            div().id("jira-refresh").px(6).py(2).rounded(4).text_size(11).text_color(t.muted).cursor_pointer()
              .hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip("Ask Jira again (about 15 seconds)")
              .on_click((_e, cx) => this.fetch(cx)).child("↻"),
          ),
      )
      .when(Boolean(status), (el) => el.child(status))
      .child(
        v_flex().id("jira-sections").flex_1().min_h(0).gap(4).px(6).pb(10).overflow_y_scrollbar().children(
          ordered.length
            ? ordered.map(([name, g]) => this.section(name, g.kind, g.issues, t))
            : [div().px(10).py(8).text_size(12).text_color(t.muted).child(this.loading ? "Asking Jira…" : this.fetchedAt ? "No open tickets assigned to you" : "Nothing yet. Press ↻ to fetch your tickets.")],
        ),
      );
  }
}
