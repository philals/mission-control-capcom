// Your open and recently merged PRs, from `capcom prs`: CI, automerge, and a click back to the agent.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { exec, run, theme, shared, setShared } from "herdr";
import { strip, stripCount, hasRunning } from "./strip.js";
import { SH, capcom, openUrl, short, dotColor, prDo, pill, prDetails } from "./pr-details.js";

const EVERY_MS = 30000;

function ago(iso, now) {
  const m = Math.floor((now - Date.parse(iso)) / 60000);
  if (!(m >= 0)) return "";
  if (m < 1) return "now";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h` : `${Math.floor(h / 24)}d`;
}

export default class CapcomPrs extends View {
  init(_props, cx) {
    this.data = null;
    this.error = null;
    this.loading = false;
    this.now = Date.now();
    this.expanded = null;
    this.showSettings = false;
    this.asked = shared("capcom.refresh");
    this.message = null;
    this.pulse = true;
    cx.timer.every(EVERY_MS, (cx) => this.refresh(cx, false));
    cx.timer.every(700, (cx) => {
      if (!hasRunning(this.data?.open ?? [])) return;
      this.pulse = !this.pulse;
      cx.notify();
    });
    this.refresh(cx, false);
  }

  refresh(cx, force) {
    if (this.loading) return;
    this.loading = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await this.capcom(["prs", ...(force ? ["--force"] : [])], { timeout: 60000 });
        if (r.code !== 0) throw new Error((r.stderr || r.stdout).trim().split("\n").pop() || `capcom exited ${r.code}`);
        this.data = JSON.parse(r.stdout);
        this.error = this.data.errors?.length ? this.data.errors.join("; ") : null;
        setShared("capcom.prs", this.data);
      } catch (e) {
        this.error = String(e?.message ?? e);
      }
      this.now = Date.now();
      this.loading = false;
      cx.notify();
    });
  }

  capcom(args, options) {
    return capcom(args, options);
  }

  // `capcom pr-do`: change a PR or a setting, say what happened, then look again.
  change(cx, action, url, on) {
    this.message = { text: "…", ok: true };
    cx.notify();
    cx.spawn(async (cx) => {
      this.message = await prDo(action, url, on);
      this.loading = false;
      this.refresh(cx, true);
    });
  }

  openUrl(url) {
    openUrl(url);
  }

  copyUrl(url) {
    const cmd = 'printf %s "$1" | { wl-copy || xclip -selection clipboard || xsel -b -i || pbcopy; } 2>/dev/null';
    exec(SH[0], [...SH.slice(1), "sh", "-c", cmd, "sh", url]).catch(() => {});
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
    const dot = dotColor(pr, merged, t);
    const live = !!pr.agent?.terminal;
    const ready = !merged && !pr.draft;
    const bits = [];
    if (!merged) {
      if (pr.draft) bits.push("draft");
      if (pr.ci === "red") bits.push(pr.conflict && !pr.failing?.length ? "conflict" : `${pr.counts.failed} failing`);
      else if (pr.ci === "pending") bits.push(`${pr.counts.running + pr.counts.queued} running`);
      else if (pr.blocker) bits.push(pr.blocker);
    } else {
      bits.push(`${ago(pr.mergedAt, this.now)} ago`);
    }
    const line = h_flex()
      .id(`pr-${pr.url}`)
      .gap(8)
      .px(10)
      .py(6)
      .items_center()
      .cursor_pointer()
      .tooltip(merged ? this.hint(pr) : [pr.blocker ? `Blocked: ${pr.blocker}` : "Nothing in the way", "Click for details"].join(" · "))
      .hover((s) => s.bg(t.hover))
      .on_click((_e, cx) => {
        if (merged) return this.go(pr);
        this.expanded = this.expanded === pr.url ? null : pr.url;
        cx.notify();
      })
      .child(div().w(10).h(10).rounded(5).flex_none().bg(dot))
      .child(
        v_flex()
          .flex_1()
          .min_w_0()
          .gap(1)
          .child(
            h_flex().child(
              div()
                .text_size(13)
                .truncate()
                .rounded(3)
                .px(ready ? 4 : 0)
                .when(ready, (el) => el.bg(t.ansi2).text_color(t.background))
                .when(!ready, (el) => el.text_color(t.foreground))
                .child(pr.title),
            ),
          )
          .child(
            h_flex()
              .gap(8)
              .justify_between()
              .child(div().text_size(11).text_color(t.muted).truncate().child([`${short(pr.repo)}#${pr.number}`, ...bits].join(" · ")))
              .when(!merged && !!pr.strip?.length, (el) => el.child(div().text_size(10).flex_none().text_color(pr.ci === "red" ? t.ansi1 : t.muted).child(stripCount(pr.strip)))),
          )
          .when(!merged && !!pr.strip?.length, (el) => el.child(strip(pr.strip, t, this.pulse)))
          .when(!merged && pr.ci === "red" && !!pr.failing?.length, (el) => el.child(div().text_size(11).text_color(t.ansi1).truncate().child(`✗ ${pr.failing[0]}`))),
      )
      .when(!merged && pr.automerge?.on, (el) => el.child(div().text_size(10).text_color(t.ansi2).flex_none().child("auto")))
      .child(
        div()
          .id(`go-${pr.url}`)
          .text_size(12)
          .text_color(live ? t.accent : t.dim)
          .flex_none()
          .px(2)
          .tooltip(this.hint(pr))
          .on_click((_e, cx) => {
            cx.stop_propagation();
            this.go(pr);
          })
          .child(pr.agent ? "🤖" : ""),
      )
      .child(
        div()
          .id(`cp-${pr.url}`)
          .text_size(12)
          .text_color(t.dim)
          .flex_none()
          .px(2)
          .tooltip("Copy the PR link")
          .hover((s) => s.text_color(t.foreground))
          .on_click((_e, cx) => {
            cx.stop_propagation();
            this.copyUrl(pr.url);
          })
          .child(svg("mods/workspace-chrome/files.svg").size(13).flex_none()),
      )
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
    if (merged || this.expanded !== pr.url) return line;
    return v_flex().child(line).child(this.details(pr, t));
  }

  details(pr, t) {
    return prDetails(pr, t, (cx, action, url, on) => this.change(cx, action, url, on));
  }

  section(label, count, t) {
    return div().px(10).pt(8).pb(2).text_size(11).font_semibold().text_color(t.dim).child(`${label} · ${count}`);
  }

  render(cx) {
    const t = theme();
    const want = shared("capcom.refresh");
    if (want && want !== this.asked) {
      this.asked = want;
      this.refresh(cx, true);
    }
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
          .child(div().text_size(12).font_semibold().text_color(red ? t.ansi1 : t.foreground).child(red ? `My PRs · ${red} red` : "My PRs"))
          .child(
            h_flex()
              .gap(8)
              .child(
                div()
                  .id("gear")
                  .text_size(13)
                  .text_color(this.showSettings ? t.foreground : t.dim)
                  .tooltip("Defaults for auto-fix and auto-review")
                  .hover((s) => s.text_color(t.foreground))
                  .on_click((_e, cx) => ((this.showSettings = !this.showSettings), cx.notify()))
                  .child("⚙"),
              )
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
          ),
      )
      .when(this.showSettings && !!d, (el) =>
        el.child(
          v_flex()
            .gap(4)
            .px(10)
            .pb(6)
            .child(div().text_size(11).text_color(t.muted).child("Defaults for your PRs"))
            .child(
              h_flex()
                .gap(6)
                .child(pill("def-af", "auto-fix", d.settings?.autofix, t, (cx) => this.change(cx, "setting-autofix", "", !d.settings?.autofix), "Default for PRs with no choice of their own"))
                .child(pill("def-ar", "auto-review", d.settings?.autoreview, t, (cx) => this.change(cx, "setting-autoreview", "", !d.settings?.autoreview), "Default for PRs with no choice of their own")),
            )
            .when(d && !d.autopilot, (el2) => el2.child(div().text_size(11).text_color(t.ansi3).child("capcom web is not running, so auto-fix and auto-review do nothing yet"))),
        ),
      )
      .when(!!this.message, (el) => el.child(div().px(10).pb(4).text_size(11).text_color(this.message.ok ? t.muted : t.ansi1).child(this.message.text)))
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
