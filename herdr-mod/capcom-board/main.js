// CAPCOM's story board as vertical status sections. Press a card, release on a section to move it.

import { View, div } from "gpui-kit";
import { v_flex, h_flex, InputState, Input } from "gpui-base";
import { run, theme, home, readTextFile, spaces } from "herdr";
import { capcom, openUrl, short } from "./pr-details.js";

const EVERY_MS = 5000;
const STATUSES = [
  ["todo", "TO DO"],
  ["planning", "PLANNING"],
  ["planned", "PLANNED"],
  ["implementing", "IMPLEMENTING"],
  ["done", "DONE"],
  ["dropped", "DROPPED"],
];
const COLOR = {
  todo: "#7c8493",
  planning: "#6cb6ff",
  planned: "#c792ea",
  implementing: "#f0b429",
  done: "#22c55e",
  dropped: "#5c6370",
};
const STORY_COLOR = { in_progress: "#f0b429", in_review: "#c792ea", done: "#22c55e" };
const SKILL = { planning: "story-plan", implementing: "story-implement-task" };
const STORY_DOT = { in_progress: "accent", in_review: "ansi3", done: "dim" };

const store = (k, v) => {
  try {
    if (v === undefined) return localStorage.getItem(k);
    localStorage.setItem(k, v);
  } catch (_e) {
    return null;
  }
};

export default class CapcomBoard extends View {
  init(_props, cx) {
    this.stories = [];
    this.key = store("key");
    this.board = null;
    this.error = null;
    this.showDone = store("showDone") === "1";
    this.manual = new Map();
    this.drag = null;
    this.over = null;
    this.message = null;
    this.adding = false;
    this.storyMenu = false;
    this.input = InputState.new({ placeholder: "Jira key, e.g. PROJ-123" });
    this.input.on("submit", (_e, cx) => this.addStory(cx));
    cx.timer.every(EVERY_MS, (cx) => this.load(cx));
    this.load(cx);
  }

  async root() {
    if (this.rootDir !== undefined) return this.rootDir;
    try {
      const settings = JSON.parse(await readTextFile(`${home()}/.claude/settings.json`));
      this.rootDir = settings["env"]?.CAPCOM_ROOT ?? null;
    } catch (_e) {
      this.rootDir = null;
    }
    return this.rootDir;
  }

  async capcom(args, options) {
    const root = await this.root();
    return capcom(root ? [...args, "--root", root] : args, options);
  }

  async json(args) {
    const r = await this.capcom(args, { timeout: 60000 });
    if (r.code !== 0) throw new Error((r.stderr || r.stdout).trim().split("\n").pop() || `capcom exited ${r.code}`);
    return JSON.parse(r.stdout);
  }

  load(cx) {
    cx.spawn(async (cx) => {
      try {
        this.stories = await this.json(["list"]);
        const visible = this.visibleStories();
        if (!this.key || !this.stories.some((s) => s.key === this.key)) this.key = (visible[0] ?? this.stories[0])?.key ?? null;
        this.board = this.key ? await this.json(["show", this.key]) : null;
        this.error = null;
      } catch (e) {
        this.error = String(e?.message ?? e);
      }
      cx.notify();
    });
  }

  visibleStories() {
    return this.stories.filter((s) => this.showDone || s.status !== "done" || s.key === this.key);
  }

  select(cx, key) {
    this.key = key;
    this.storyMenu = false;
    store("key", key);
    this.board = null;
    this.load(cx);
  }

  toggleCollapsed(cx, status, collapsed, empty) {
    this.manual.set(status, { collapsed: !collapsed, empty });
    cx.notify();
  }

  move(cx, task, status) {
    if (task.status === status) return;
    this.message = { text: `${task.id} → ${status}…`, ok: true };
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const key = this.key;
        const r = await this.capcom(["status", key, task.id, status, "--force"]);
        if (r.code !== 0) throw new Error((r.stderr || r.stdout).trim().split("\n").pop());
        this.message = { text: `${task.id} → ${status}`, ok: true };
        if (SKILL[status]) {
          this.launch(task, status);
          this.message = { text: `${task.id} → ${status}, agent starting in a new tab`, ok: true };
        }
      } catch (e) {
        this.message = { text: String(e?.message ?? e), ok: false };
      }
      this.load(cx);
    });
  }

  launch(task, status) {
    const skill = SKILL[status];
    if (!skill) return;
    const argv = ["claude", `/${skill} ${this.key} ${task.id}`];
    const name = `${this.key} ${task.id} ${status === "planning" ? "plan" : "impl"}`;
    const repo = task.repos?.[0]?.split("/").pop();
    const folder = task.folder || (repo ? `${home()}/code/${repo}` : null);
    if (!folder) return run("tab.new", { name, argv });
    const mine = spaces().find(
      (s) => !s.machine && s.folder && (s.folder === folder || s.folder.startsWith(`${folder}/`) || (repo && s.folder.includes(`/worktrees/${repo}/`))),
    );
    if (mine) {
      run("space.select", { id: mine.id });
      run("tab.new", { space: mine.id, name, folder, argv });
    } else {
      run("space.new", { folder, argv });
    }
  }

  async openPlan(cx) {
    const key = this.key;
    try {
      let root = await this.root();
      if (!root) {
        const r = await capcom(["root"], { timeout: 10000 });
        root = JSON.parse(r.stdout).root;
      }
      run("pane.split", { direction: "right", mod: "capcom-plan", props: { file: `${root}/${key}/story.md`, title: `${key} plan` } });
    } catch (e) {
      this.message = { text: `Cannot open the plan: ${String(e?.message ?? e)}`, ok: false };
      cx.notify();
    }
  }

  setStoryStatus(cx, status) {
    const key = this.key;
    this.storyMenu = false;
    this.message = { text: `${key} → ${status.replace("_", " ")}…`, ok: true };
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await this.capcom(["story-status", key, status]);
        if (r.code !== 0) throw new Error((r.stderr || r.stdout).trim().split("\n").pop());
        this.message = { text: `${key} is now ${status.replace("_", " ")}`, ok: true };
        if (status === "in_review") run("tab.new", { name: `${key} review`, argv: ["claude", `/story-review ${key}`] });
      } catch (e) {
        this.message = { text: String(e?.message ?? e), ok: false };
      }
      this.load(cx);
    });
  }

  toggleAdding(cx) {
    this.adding = !this.adding;
    if (this.adding) {
      this.input.set_value("");
      cx.timer.after(100, () => this.input.focus());
    }
    cx.notify();
  }

  addStory(cx) {
    const key = this.input.value().trim().toUpperCase();
    if (!/^[A-Z][A-Z0-9]*-\d+$/.test(key)) {
      this.message = { text: `"${key}" is not a Jira key like PROJ-123`, ok: false };
      cx.notify();
      return;
    }
    this.adding = false;
    if (this.stories.some((s) => s.key === key)) {
      this.message = { text: `${key} already has a board`, ok: true };
      this.select(cx, key);
      return;
    }
    this.message = { text: `Breaking down ${key} in a new tab…`, ok: true };
    run("tab.new", { name: `${key} breakdown`, argv: ["claude", `/story-plan ${key}`] });
    cx.notify();
  }

  endDrag(cx) {
    const task = this.drag;
    const status = this.over;
    this.drag = null;
    this.over = null;
    if (task && status && task.status !== status) this.move(cx, task, status);
    else cx.notify();
  }

  card(task, t, tasks) {
    const open = (task.prs ?? []).filter((p) => p.state !== "closed");
    const waiting = (task.dependsOn ?? []).filter((d) => tasks.find((x) => x.id === d)?.status !== "done");
    const dragging = this.drag?.id === task.id;
    const flags = [];
    if (task.blocked) flags.push({ text: `blocked${task.blocked.reason ? `: ${task.blocked.reason}` : ""}`, color: "#ff1f1f" });
    if (waiting.length && task.status !== "done") flags.push({ text: `waits ${waiting.join(", ")}`, color: t.ansi3 });
    if (task.type && task.type !== "pr") flags.push({ text: task.type, color: t.muted });
    const prColor = (p) => (p.state === "merged" ? t.ansi5 : p.state === "ready" ? "#22c55e" : p.state === "draft" ? "#ffb000" : t.dim);
    return v_flex()
      .id(`task-${task.id}`)
      .gap(4)
      .px(10)
      .py(7)
      .rounded(6)
      .bg(dragging ? t.selected : t.raised)
      .border(1)
      .border_color(dragging ? COLOR[task.status] : task.status === "implementing" ? COLOR.implementing : t.border)
      .opacity(dragging ? 0.6 : 1)
      .cursor_grab()
      .hover((s) => s.bg(t.hover))
      .on_mouse_down("left", (_e, cx) => {
        this.drag = task;
        cx.notify();
      })
      .child(
        h_flex().gap(6).items_start()
          .child(div().text_size(11).font_semibold().text_color(COLOR[task.status]).flex_none().child(task.id))
          .child(div().flex_1().min_w(0).text_size(12).text_color(t.foreground).child(task.title))
          .when(SKILL[this.nextStatus(task)], (el) =>
            el.child(
              div().id(`go-${task.id}`).flex_none().px(5).rounded(4).text_size(11).text_color(t.muted)
                .cursor_pointer().hover((s) => s.bg(t.selected).text_color(t.foreground))
                .tooltip(`Start ${SKILL[this.nextStatus(task)]} for ${task.id} in a new tab`)
                .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
                .on_click((_e, cx) => {
                  cx.stop_propagation();
                  this.launch(task, this.nextStatus(task));
                })
                .child("▶"),
            ),
          ),
      )
      .when(flags.length || open.length, (el) =>
        el.child(
          h_flex().gap(8).items_center().flex_wrap().text_size(11)
            .children(flags.map((f) => div().text_color(f.color).child(f.text)))
            .children(
              open.map((p, i) =>
                h_flex().id(`pr-${task.id}-${i}`).gap(4).items_center().cursor_pointer()
                  .text_color(t.muted).hover((s) => s.text_color(t.foreground))
                  .tooltip(`${p.state}: ${p.url}`)
                  .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
                  .on_click(() => openUrl(p.url))
                  .child(div().size(7).rounded(4).bg(prColor(p)))
                  .child(`${short(p.repo)}#${p.url.split("/").pop()}`),
              ),
            ),
        ),
      );
  }

  nextStatus(task) {
    return { todo: "planning", planned: "implementing" }[task.status];
  }

  section(status, label, tasks, t, all) {
    const empty = tasks.length === 0;
    const manual = this.manual.get(status);
    const collapsed = manual && manual.empty === empty ? manual.collapsed : empty;
    const target = this.drag && this.drag.status !== status;
    const hot = target && this.over === status;
    return v_flex()
      .id(`sec-${status}`)
      .rounded(8)
      .border(1)
      .border_color(hot ? COLOR[status] : target ? t.input : "#00000000")
      .bg(hot ? t.selected : "#00000000")
      .on_mouse_move(() => {
        if (this.drag && this.over !== status) this.over = status;
      })
      .on_hover((hovered) => {
        if (hovered && this.drag) this.over = status;
      })
      .child(
        h_flex().id(`head-${status}`).gap(6).px(6).py(5).items_center().cursor_pointer()
          .hover((s) => s.bg(t.hover)).rounded(6)
          .on_click((_e, cx) => this.toggleCollapsed(cx, status, collapsed, empty))
          .child(div().w(10).text_size(10).text_color(t.dim).child(collapsed ? "▸" : "▾"))
          .child(div().size(8).rounded(4).flex_none().bg(COLOR[status]))
          .child(div().text_size(11).font_semibold().text_color(COLOR[status]).child(label))
          .child(div().text_size(11).text_color(t.dim).child(String(tasks.length)))
          .when(target, (el) => el.child(div().flex_1().text_size(11).text_color(COLOR[status]).child(hot ? `  drop to move → ${label}` : ""))),
      )
      .when(!collapsed, (el) =>
        el.child(
          v_flex().gap(5).px(4).pb(6).children(
            tasks.length
              ? tasks.map((x) => this.card(x, t, all))
              : [div().px(8).py(3).text_size(11).text_color(t.dim).child(target ? "Drop here" : "empty")],
          ),
        ),
      );
  }

  render(cx) {
    const t = theme();
    const tasks = this.board?.tasks ?? [];
    const stories = this.visibleStories();
    const story = this.stories.find((s) => s.key === this.key);
    const done = tasks.filter((x) => x.status === "done").length;
    const header = v_flex().gap(6).px(10).pt(10).pb(6).flex_none()
      .child(
        h_flex().gap(6).items_center()
          .child(div().flex_1().text_size(13).font_semibold().text_color(t.foreground).child("CAPCOM Kanban"))
          .child(this.button("refresh", "↻", "Reload", () => this.load(cx), t))
          .child(this.button("show-done", this.showDone ? "☑ done" : "☐ done", "Show finished stories", () => {
            this.showDone = !this.showDone;
            store("showDone", this.showDone ? "1" : "0");
            cx.notify();
          }, t)),
      )
      .child(
        h_flex().id("stories").gap(4).flex_wrap()
          .children(
            stories.map((s) =>
              div().id(`story-${s.key}`).px(7).py(2).rounded(5).text_size(11).cursor_pointer()
                .bg(s.key === this.key ? t.selected : t.raised)
                .text_color(s.key === this.key ? t.foreground : t.muted)
                .border(1).border_color(s.key === this.key ? t.accent : t.border)
                .hover((x) => x.bg(t.hover))
                .tooltip(`${s.title} (${s.status.replace("_", " ")})`)
                .on_click((_e, cx) => this.select(cx, s.key))
                .child(s.key),
            ),
          )
          .child(
            div().id("add-story").px(7).py(2).rounded(5).text_size(11).cursor_pointer()
              .text_color(t.accent).border(1).border_color(t.border).hover((x) => x.bg(t.hover))
              .tooltip("Add a story: plan a Jira story and split it into tasks")
              .on_click((_e, cx) => this.toggleAdding(cx))
              .child(this.adding ? "× cancel" : "+ story"),
          ),
      )
      .when(Boolean(story) && this.storyMenu, (el) => {
        const labels = { in_progress: "Move to doing", in_review: "Move to review", done: "Mark done" };
        const moves = ["in_progress", "in_review", "done"].filter((x) => x !== story.status).map((x) => [x, labels[x], null]);
        return el.child(
          h_flex().gap(6).flex_wrap().items_center()
            .children(
              moves.map(([to, label, why]) =>
                div().id(`move-${to}`).px(8).py(3).rounded(6).text_size(11).font_semibold()
                  .bg(why ? t.raised : STORY_COLOR[to]).text_color(why ? t.dim : "#14161a")
                  .when(!why, (x) => x.cursor_pointer().hover((y) => y.opacity(0.85)))
                  .tooltip(why ?? (to === "in_review" ? "Opens a tab running /story-review" : label))
                  .on_click((_e, cx) => {
                    if (why) { this.message = { text: `Not yet: ${why}`, ok: false }; cx.notify(); }
                    else this.setStoryStatus(cx, to);
                  })
                  .child(label),
              ),
            ),
        );
      })
      .when(this.adding, (el) =>
        el.child(
          h_flex().gap(6).items_center()
            .child(Input.new(this.input).flex_1().h(28).px(8).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(12).text_color(t.foreground))
            .child(
              div().id("add-go").px(8).py(4).rounded(6).text_size(11).font_semibold().cursor_pointer()
                .bg(t.solid).text_color(t.solid_foreground).hover((x) => x.opacity(0.88))
                .on_click((_e, cx) => this.addStory(cx)).child("Break down"),
            ),
        ),
      )
      .when(Boolean(story), (el) =>
        el.child(
          v_flex().gap(3)
            .child(div().text_size(12).text_color(t.foreground).child(story.title))
            .child(
              h_flex().gap(8).items_center().text_size(11).text_color(t.muted)
                .child(
                  div().id("story-status").px(7).py(1).rounded(9).text_size(10).font_semibold().cursor_pointer()
                    .bg(STORY_COLOR[story.status]).text_color("#14161a").hover((x) => x.opacity(0.85))
                    .tooltip("Move this story to another status")
                    .on_click((_e, cx) => { this.storyMenu = !this.storyMenu; cx.notify(); })
                    .child(`${story.status.replace("_", " ").toUpperCase()} ▾`),
                )
                .child(
                  div().id("story-plan").px(7).py(1).rounded(9).text_size(10).font_semibold().cursor_pointer()
                    .bg(t.raised).text_color(t.accent).border(1).border_color(t.border).hover((x) => x.bg(t.hover))
                    .tooltip("Open this story's plan beside the board")
                    .on_click((_e, cx) => this.openPlan(cx))
                    .child("PLAN"),
                )
                .child(`${done}/${tasks.length} done`)
                .child(
                  div().flex_1().h(4).rounded(2).bg(t.border)
                    .child(div().h(4).rounded(2).bg("#22c55e").w(`${tasks.length ? Math.round((100 * done) / tasks.length) : 0}%`)),
                ),
            ),
        ),
      );
    const status = this.error
      ? div().px(10).py(4).text_size(11).text_color("#ff1f1f").child(this.error)
      : this.message
        ? div().px(10).py(4).text_size(11).text_color(this.message.ok ? t.muted : "#ffb000").child(this.message.text)
        : null;
    return v_flex().id("board-root").size_full().bg(t.surface)
      .on_mouse_up("left", (_e, cx) => this.endDrag(cx))
      .on_mouse_move((e, cx) => {
        if (this.drag) cx.notify();
      })
      .child(header)
      .when(Boolean(status), (el) => el.child(status))
      .child(
        v_flex().id("sections").flex_1().min_h(0).gap(2).px(6).pb(10).overflow_y_scrollbar().children(
          STATUSES.map(([s, label]) => {
            const mine = tasks.filter((x) => x.status === s);
            return s === "dropped" && !mine.length && !this.drag ? null : this.section(s, label, mine, t, tasks);
          }).filter(Boolean),
        ),
      );
  }

  button(id, label, tip, onClick, t) {
    return div().id(id).px(6).py(2).rounded(4).text_size(11).text_color(t.muted).cursor_pointer()
      .hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip(tip).on_click(onClick).child(label);
  }
}
