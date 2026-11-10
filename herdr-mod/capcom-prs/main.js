// Your open and recently merged PRs, from `capcom prs`: CI, automerge, and a click back to the agent.

import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { exec, run, theme } from "herdr";

const EVERY_MS = 30000;
const SH = ["sh", "-c", 'PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"; exec "$@"', "sh"];

function ago(iso, now) {
  const m = Math.floor((now - Date.parse(iso)) / 60000);
  if (!(m >= 0)) return "";
  if (m < 1) return "now";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h` : `${Math.floor(h / 24)}d`;
}

const short = (repo) => repo.split("/").pop();

export default class CapcomPrs extends View {
  init(_props, cx) {
    this.data = null;
    this.error = null;
    this.loading = false;
    this.now = Date.now();
    cx.timer.every(EVERY_MS, (cx) => this.refresh(cx, false));
    this.refresh(cx, false);
  }

  refresh(cx, force) {
    if (this.loading) return;
    this.loading = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await exec(SH[0], [...SH.slice(1), "capcom", "prs", ...(force ? ["--force"] : [])], { timeout: 60000 });
        if (r.code !== 0) throw new Error((r.stderr || r.stdout).trim().split("\n").pop() || `capcom exited ${r.code}`);
        this.data = JSON.parse(r.stdout);
        this.error = this.data.errors?.length ? this.data.errors.join("; ") : null;
      } catch (e) {
        this.error = String(e?.message ?? e);
      }
      this.now = Date.now();
      this.loading = false;
      cx.notify();
    });
  }

  openUrl(url) {
    exec(SH[0], [...SH.slice(1), "sh", "-c", 'xdg-open "$1" 2>/dev/null || open "$1"', "sh", url]).catch(() => {});
  }

  // The agent if it is live, else resume its session in a new tab, else the PR on GitHub.
  go(pr) {
    const a = pr.agent;
    if (a?.terminal) return run("agent.focus", { terminal: a.terminal });
    if (a?.session) {
      return run("tab.new", { name: `${short(pr.repo)}#${pr.number}`, folder: a.cwd || undefined, argv: ["claude", "--resume", a.session] });
    }
    this.openUrl(pr.url);
  }

  hint(pr) {
    if (pr.agent?.terminal) return "Go to its agent";
    if (pr.agent?.session) return "Resume its agent in a new tab";
    return "Open on GitHub";
  }

  row(pr, t, merged) {
    const dot = merged ? t.ansi5 : pr.ci === "red" ? t.ansi1 : pr.ci === "pending" ? t.ansi3 : t.ansi2;
    const live = !!pr.agent?.terminal;
    const bits = [];
    if (!merged) {
      if (pr.draft) bits.push("draft");
      if (pr.ci === "red") bits.push(pr.conflict && !pr.failing?.length ? "conflict" : `${pr.counts.failed} failing`);
      else if (pr.ci === "pending") bits.push(`${pr.counts.running + pr.counts.queued} running`);
      else if (pr.blocker) bits.push(pr.blocker);
    } else {
      bits.push(`${ago(pr.mergedAt, this.now)} ago`);
    }
    return h_flex()
      .id(`pr-${pr.url}`)
      .gap(8)
      .px(10)
      .py(6)
      .items_center()
      .cursor_pointer()
      .tooltip(this.hint(pr))
      .hover((s) => s.bg(t.hover))
      .on_click(() => this.go(pr))
      .child(div().w(8).h(8).rounded(4).flex_none().bg(dot))
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .gap(1)
          .child(div().text_size(13).text_color(t.foreground).truncate().child(pr.title))
          .child(div().text_size(11).text_color(t.muted).truncate().child([`${short(pr.repo)}#${pr.number}`, ...bits].join(" · "))),
      )
      .when(!merged && pr.automerge?.on, (el) => el.child(div().text_size(10).text_color(t.ansi2).flex_none().child("auto")))
      .child(div().text_size(12).text_color(live ? t.accent : t.dim).flex_none().child(pr.agent ? "🤖" : ""))
      .child(
        div()
          .id(`gh-${pr.url}`)
          .text_size(12)
          .text_color(t.dim)
          .flex_none()
          .px(4)
          .tooltip("Open on GitHub")
          .hover((s) => s.text_color(t.foreground))
          .on_click((_e, cx) => {
            cx.stop_propagation();
            this.openUrl(pr.url);
          })
          .child("↗"),
      );
  }

  section(label, count, t) {
    return div().px(10).pt(8).pb(2).text_size(11).font_semibold().text_color(t.dim).child(`${label} · ${count}`);
  }

  render(cx) {
    const t = theme();
    const d = this.data;
    const open = d?.open ?? [];
    const merged = d?.merged ?? [];
    const red = open.filter((p) => p.ci === "red").length;
    return v_flex()
      .size_full()
      .child(
        h_flex()
          .px(10)
          .py(6)
          .items_center()
          .justify_between()
          .child(div().text_size(12).font_semibold().text_color(red ? t.ansi1 : t.foreground).child(red ? `PRs · ${red} red` : "PRs"))
          .child(
            div()
              .id("refresh")
              .text_size(13)
              .text_color(t.dim)
              .tooltip("Ask GitHub again")
              .hover((s) => s.text_color(t.foreground))
              .on_click((_e, cx) => this.refresh(cx, true))
              .child(this.loading ? "…" : "↻"),
          ),
      )
      .when(!!this.error, (el) => el.child(div().px(10).pb(4).text_size(11).text_color(t.ansi1).child(this.error)))
      .child(
        v_flex()
          .id("pr-list")
          .flex_1()
          .overflow_y_scrollbar()
          .when(d && !open.length && !merged.length && !this.error, (el) => el.child(div().px(10).py(6).text_size(12).text_color(t.muted).child("No open or recently merged PRs")))
          .when(open.length > 0, (el) => el.child(this.section("Open", open.length, t)).children(open.map((p) => this.row(p, t, false))))
          .when(merged.length > 0, (el) => el.child(this.section("Merged", merged.length, t)).children(merged.map((p) => this.row(p, t, true)))),
      );
  }
}
