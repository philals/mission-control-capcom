// Daily, under My Watch: the start-of-day scripts (AWS sign-in, CodeArtifact token, Docker login to ECR) and the dev databases,
// as buttons. Runs `capcom-daily`, which records each step in ~/.cache/capcom/daily/<step>. Nothing runs until you press a button.

import { View, div } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { exec, run, theme, home, readTextFile } from "herdr";
import { SH } from "./pr-details.js";

const TOKEN_MS = 12 * 60 * 60 * 1000;
const TICK_MS = 30 * 1000;
const CHECK_MS = 5 * 60 * 1000;
const OPEN_PX = 150, SHUT_PX = 34;
const GREEN = "#22c55e", AMBER = "#ffb000", RED = "#ff1f1f";
const STEPS = [
  { id: "sso", label: "AWS sign-in", tip: "Sign in to AWS SSO (browser) if not already", life: 0 },
  { id: "codeartifact", label: "CodeArtifact", tip: "Write a fresh CodeArtifact token to ~/.gradle/gradle.properties", life: TOKEN_MS },
  { id: "ecr", label: "Docker · ECR", tip: "Log Docker in to ECR", life: TOKEN_MS },
];
const DBS = [
  { id: "db", label: "Informix", tip: "Restart the informix-db container (needs the ECR login)" },
  { id: "dbpg", label: "Postgres", tip: "Restart db-postgres and run its migrations (needs the ECR login)" },
];

function left(ms) {
  if (ms <= 0) return "expired";
  const m = Math.floor(ms / 60000);
  return m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m left` : `${m}m left`;
}

export default class CapcomDaily extends View {
  init(_props, cx) {
    this.stat = {};
    this.running = new Set();
    this.message = null;
    this.now = Date.now();
    this.collapsed = (() => { try { return localStorage.getItem("daily.collapsed") === "1"; } catch (_e) { return false; } })();
    this.bad = null;
    this.size = null;
    cx.timer.every(TICK_MS, (cx) => {
      this.now = Date.now();
      this.read(cx);
    });
    cx.timer.every(CHECK_MS, (cx) => this.quietCheck(cx));
    this.quietCheck(cx);
  }

  // Whether AWS is signed in, looked up without taking the buttons: it costs one small call.
  quietCheck(cx) {
    cx.spawn(async (cx) => {
      await exec(SH[0], [...SH.slice(1), "capcom-daily", "check"], { timeout: 30000 }).catch(() => null);
      this.read(cx);
    });
  }

  badCount(t) {
    return STEPS.filter((s) => this.dot(s.id, t) === RED).length;
  }

  // The panel opens by itself when something newly needs running, and keeps what you chose until then.
  settle(t) {
    const bad = this.badCount(t);
    if (bad > (this.bad ?? 0)) this.setCollapsed(false);
    this.bad = bad;
    this.resize();
  }

  setCollapsed(on) {
    this.collapsed = on;
    try { localStorage.setItem("daily.collapsed", on ? "1" : "0"); } catch (_e) {}
    this.resize();
  }

  resize() {
    const px = this.collapsed ? SHUT_PX : OPEN_PX;
    if (px === this.size) return;
    this.size = px;
    run("layout.resize", { id: "daily", px });
  }

  read(cx) {
    cx.spawn(async (cx) => {
      for (const id of [...STEPS, ...DBS].map((s) => s.id)) {
        const text = await readTextFile(`${home()}/.cache/capcom/daily/${id}`).catch(() => null);
        const m = /^(\d+) (ok|fail) ?(.*)$/m.exec(text ?? "");
        this.stat[id] = m ? { at: Number(m[1]) * 1000, ok: m[2] === "ok", msg: m[3] } : null;
      }
      this.settle(theme());
      cx.notify();
    });
  }

  run(cx, step, label) {
    if (this.running.size) return;
    this.running.add(step);
    this.message = null;
    cx.notify();
    cx.spawn(async (cx) => {
      let result = null;
      try {
        result = await exec(SH[0], [...SH.slice(1), "capcom-daily", step], { timeout: 600000 });
      } catch (e) {
        this.message = { ok: false, text: `capcom-daily would not start: ${e?.message ?? e}` };
      }
      if (result && result.code !== 0) this.message = { ok: false, text: (result.stderr.trim().split("\n").pop() || `${label} failed`).slice(0, 160) };
      this.running.delete(step);
      this.now = Date.now();
      this.read(cx);
      cx.notify();
    });
  }

  dot(id, t) {
    const s = this.stat[id];
    const running = this.running.has(id) || (this.running.has("all") && STEPS.some((x) => x.id === id));
    return running ? AMBER : !s ? t.dim : !s.ok ? RED : STEPS.find((x) => x.id === id)?.life && this.now - s.at > STEPS.find((x) => x.id === id).life ? RED : GREEN;
  }

  detail(step) {
    const s = this.stat[step.id];
    if (this.running.has(step.id) || this.running.has("all")) return "running…";
    if (!s) return "not run";
    if (!s.ok) return s.msg;
    return step.life ? left(step.life - (this.now - s.at)) : s.msg;
  }

  button(id, label, tip, on, t, strong) {
    const busy = this.running.size > 0;
    return div().id(id).px(7).py(2).rounded(4).text_size(11).flex_none().cursor_pointer()
      .bg(strong ? t.selected : t.chrome).text_color(busy ? t.dim : strong ? t.foreground : t.muted)
      .hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip(tip).on_click((_e, cx) => on(cx)).child(label);
  }

  render(_cx) {
    const t = theme();
    const bad = this.badCount(t);
    const shut = this.collapsed;
    return v_flex().id("daily-root").size_full().min_h(0).border_t(1).border_color(t.border)
      .child(
        h_flex().gap(6).px(10).py(6).items_center().flex_none()
          .child(
            h_flex().id("daily-title").gap(6).items_center().flex_1().min_w(0).cursor_pointer()
              .tooltip(shut ? "Show the daily scripts" : "Hide the daily scripts")
              .on_click((_e, cx) => { this.setCollapsed(!shut); cx.notify(); })
              .child(div().w(10).flex_none().text_size(10).text_color(t.dim).child(shut ? "▸" : "▾"))
              .child(div().text_size(12).font_semibold().text_color(bad ? t.ansi1 : t.foreground).child(bad ? `Daily · ${bad} to run` : "Daily"))
              .when(shut, (el) => el.children(STEPS.map((step) => div().size(7).flex_none().rounded(4).bg(this.dot(step.id, t))))),
          )
          .child(this.button("daily-all", this.running.has("all") ? "Running…" : "▶ Run daily", "AWS sign-in, then the CodeArtifact token and the ECR login together", (cx) => this.run(cx, "all", "Daily"), t, true))
          .child(this.button("daily-check", "↻", "Check whether AWS is signed in", (cx) => this.run(cx, "check", "Check"), t)),
      )
      .when(!shut, (el) => el.child(
        v_flex().id("daily-steps").gap(3).px(10).pb(4).flex_none()
          .children(STEPS.map((step) =>
            h_flex().gap(7).items_center().h(20)
              .child(div().size(7).flex_none().rounded(4).bg(this.dot(step.id, t)))
              .child(div().text_size(12).text_color(t.foreground).flex_none().child(step.label))
              .child(div().flex_1().min_w(0).truncate().text_size(11).text_color(t.dim).tooltip(this.stat[step.id]?.msg ?? "").child(this.detail(step)))
              .child(this.button(`daily-${step.id}`, "▶", step.tip, (cx) => this.run(cx, step.id, step.label), t)))),
      ))
      .when(!shut, (el) => el.child(
        h_flex().gap(6).px(10).pb(6).items_center().flex_none()
          .child(div().text_size(11).text_color(t.dim).child("Databases"))
          .children(DBS.map((db) => this.button(`daily-${db.id}`, this.running.has(db.id) ? `${db.label}…` : db.label, db.tip, (cx) => this.run(cx, db.id, db.label), t))),
      ))
      .when(!shut && Boolean(this.message), (el) => el.child(div().px(10).pb(6).text_size(11).text_color(this.message.ok ? GREEN : AMBER).child(this.message.text)));
  }
}
