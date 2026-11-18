// Claude usage, in My Watch under the PRs: the account Claude Code is signed in with and its plan limits as /usage shows them
// (session, week, a model's own weekly limit). Read with Claude Code's own `get_usage` request over `claude -p`: no prompt is
// sent, so no model runs and nothing is spent, and this mod never touches a token. Refreshes when the panel is shown with stale
// figures, every 10 minutes while it is shown, and on ↻.

import { View, div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { exec, run, theme, layout } from "herdr";
import { SH } from "./pr-details.js";

const STALE_MS = 10 * 60 * 1000;
const WATCH_MS = 5 * 1000;
const ORANGE = "#D97757";
const CYAN = "#56b6c2";
const OPEN_PX = 172, SHUT_PX = 34;
const REQUEST = JSON.stringify({ type: "control_request", request_id: "usage", request: { subtype: "get_usage", skip_behaviors: true } });

const find = (node, id) => {
  if (node?.id === id) return node;
  for (const c of node?.children ?? []) {
    const f = find(c, id);
    if (f) return f;
  }
  return null;
};

function load(key, fallback) {
  try {
    return JSON.parse(localStorage.getItem(key) ?? "null") ?? fallback;
  } catch (_e) {
    return fallback;
  }
}
const save = (key, value) => localStorage.setItem(key, JSON.stringify(value));

function ago(ms) {
  const m = Math.floor((Date.now() - ms) / 60000);
  if (m < 1) return "just now";
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h ago` : `${Math.floor(h / 24)}d ago`;
}

function until(ms) {
  const m = Math.max(0, Math.ceil(ms / 60000));
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h ${m % 60}m` : `${Math.floor(h / 24)}d ${h % 24}h`;
}

// How far through the plan's weekly window it is when its reset time is known, else Monday to Friday, 06:00 to 17:00 local time (as in the Claude mod's status line).
function workWeek(now, resetsAt) {
  if (resetsAt > now) {
    const WEEK = 7 * 24 * 3600 * 1000;
    return { kind: "work-week", label: "Week elapsed", percent: Math.max(0, Math.min(100, Math.round(((now - (resetsAt - WEEK)) / WEEK) * 100))), resetsAt, note: null, window: true };
  }
  const d = new Date(now), day = d.getDay();
  if (day === 0 || day === 6) return { kind: "work-week", label: "Working week", percent: 100, resetsAt: 0, note: "weekend" };
  const hours = Math.min(11, Math.max(0, d.getHours() + d.getMinutes() / 60 - 6));
  const end = new Date(d);
  end.setDate(d.getDate() + (5 - day));
  end.setHours(17, 0, 0, 0);
  return { kind: "work-week", label: "Working week", percent: Math.round((((day - 1) * 11 + hours) / 55) * 100), resetsAt: end.getTime(), note: null };
}

const cap = (s) => (s ? s[0].toUpperCase() + s.slice(1) : s);

function rowsOf(answer) {
  const rl = answer?.rate_limits ?? {};
  const limits = Array.isArray(rl.limits) && rl.limits.length
    ? rl.limits
    : [
        rl.five_hour && { kind: "session", percent: rl.five_hour.utilization, resets_at: rl.five_hour.resets_at },
        rl.seven_day && { kind: "weekly_all", percent: rl.seven_day.utilization, resets_at: rl.seven_day.resets_at },
      ].filter(Boolean);
  return limits
    .filter((l) => typeof l.percent === "number")
    .map((l) => {
      const scope = l.scope?.model?.display_name ?? l.scope?.surface?.display_name ?? null;
      const label = l.kind === "session" ? "Session" : l.kind === "weekly_all" ? "Week, all models" : scope ? `Week, ${scope}` : cap(String(l.kind ?? "limit").replace(/_/g, " "));
      return { kind: l.kind, label, percent: l.percent, severity: l.severity ?? "normal", resetsAt: l.resets_at ? Date.parse(l.resets_at) || 0 : 0 };
    });
}

function accountOf(stdout) {
  const text = String(stdout ?? "");
  const a = JSON.parse(text.slice(text.indexOf("{"), text.lastIndexOf("}") + 1));
  const str = (v) => (typeof v === "string" && v ? v : null);
  return { signedIn: a.loggedIn === true, email: str(a.email), org: str(a.orgName), plan: str(a.subscriptionType), endsAt: (str(a.loginExpiresAt) && Date.parse(a.loginExpiresAt)) || 0 };
}

const askUsage = () =>
  exec(SH[0], [...SH.slice(1), "sh", "-c", 'd="$HOME/.cache/capcom/claude-usage"; mkdir -p "$d" && cd "$d" && printf "%s\\n" "$1" | claude -p --input-format stream-json --output-format stream-json --verbose --no-session-persistence', "sh", REQUEST], { timeout: 60000 });

const askAccount = () => exec(SH[0], [...SH.slice(1), "sh", "-c", "exec claude auth status --json < /dev/null"], { timeout: 30000 });

export default class CapcomUsage extends View {
  init(_props, cx) {
    this.usage = load("usage", null);
    this.checkedAt = load("checked-at", 0);
    this.account = load("account", null);
    this.busy = false;
    this.problem = null;
    this.wasShown = false;
    this.collapsed = (() => { try { return localStorage.getItem("usage.collapsed") === "1"; } catch (_e) { return false; } })();
    this.size = null;
    this.resize();
    cx.timer.every(WATCH_MS, (cx) => this.watch(cx));
    this.watch(cx);
  }

  setCollapsed(on) {
    this.collapsed = on;
    try { localStorage.setItem("usage.collapsed", on ? "1" : "0"); } catch (_e) {}
    this.resize();
  }

  resize() {
    const px = this.collapsed ? SHUT_PX : OPEN_PX;
    if (px === this.size) return;
    this.size = px;
    run("layout.resize", { id: "usage", px });
  }

  shown() {
    const node = find(layout(), "watch-panel");
    return Boolean(node && !node.hidden);
  }

  watch(cx) {
    const shown = this.shown();
    if ((shown && !this.wasShown) || (shown && Date.now() - this.checkedAt > STALE_MS)) {
      if (!this.checkedAt || Date.now() - this.checkedAt > STALE_MS) this.refresh(cx);
    }
    this.wasShown = shown;
    cx.notify();
  }

  refresh(cx) {
    if (this.busy) return;
    this.busy = true;
    this.problem = null;
    cx.notify();
    cx.spawn(async (cx) => {
      const account = askAccount().then((r) => (r.code === 0 ? accountOf(r.stdout) : null)).catch(() => null);
      try {
        const r = await askUsage();
        let answer = null;
        for (const line of (r.stdout ?? "").split("\n")) {
          let m;
          try {
            m = JSON.parse(line);
          } catch (_e) {
            continue;
          }
          if (m?.type !== "control_response" || m.response?.request_id !== "usage") continue;
          if (m.response.subtype !== "success") throw new Error(m.response.error ?? "Claude Code couldn't read your usage.");
          answer = m.response.response;
        }
        if (!answer) {
          const said = `${r.stderr ?? ""}`.split("\n").map((l) => l.trim()).filter(Boolean).pop();
          throw new Error(said ? said.slice(0, 160) : `Claude Code didn't answer (exit ${r.code}).`);
        }
        this.usage = { plan: answer.subscription_type ?? null, available: answer.rate_limits_available !== false, rows: rowsOf(answer) };
        this.checkedAt = Date.now();
        save("usage", this.usage);
        save("checked-at", this.checkedAt);
      } catch (e) {
        this.problem = String(e?.message ?? e).slice(0, 160);
      }
      const who = await account;
      if (who) {
        this.account = who;
        save("account", who);
      }
      this.busy = false;
      cx.notify();
    });
  }

  color(t, row) {
    if (row.percent >= 100) return t.ansi1;
    if (row.percent >= 80 || row.severity !== "normal") return "#ffb000";
    return t.accent;
  }

  row(t, row) {
    const reset = row.resetsAt && row.resetsAt <= Date.now();
    const week = row.kind === "work-week";
    const color = week ? CYAN : reset ? t.dim : this.color(t, row);
    const pct = Math.max(0, Math.min(100, row.percent));
    return h_flex().id(`limit-${row.kind}-${row.label}`).h(22).gap(8).flex_none().items_center()
      .tooltip(week ? (row.window ? "How far through the plan's weekly window it is" : "How far through the working week it is: Monday to Friday, 06:00 to 17:00") : reset ? "It has reset since this was checked: press ↻." : `${Math.round(row.percent)}% used`)
      .child(div().w(104).flex_none().truncate().text_size(11.5).text_color(t.foreground).child(row.label))
      .child(div().flex_1().min_w(24).h(5).rounded(3).overflow_hidden().bg(t.input).child(div().h_full().w(`${reset ? 0 : pct}%`).rounded(3).bg(color)))
      .child(div().w(32).flex_none().text_right().text_size(11).font_medium().text_color(color).child(reset ? "–" : `${Math.round(row.percent)}%`))
      .child(div().w(64).flex_none().text_right().text_size(10.5).text_color(t.dim).child(week ? (row.note ?? `in ${until(row.resetsAt - Date.now())}`) : !row.resetsAt ? "" : reset ? "reset" : `in ${until(row.resetsAt - Date.now())}`));
  }

  render(_cx) {
    const t = theme();
    const rows = this.usage?.rows ?? [];
    const week = workWeek(Date.now(), rows.find((r) => r.kind === "weekly_all")?.resetsAt);
    const shut = this.collapsed;
    const session = rows.find((r) => r.kind === "session");
    const a = this.account;
    const plan = a?.plan ?? this.usage?.plan;
    return v_flex().id("usage-root").size_full().min_h(0).border_t(1).border_color(t.border)
      .child(
        h_flex().gap(6).px(10).py(6).items_center().flex_none()
          .child(
            h_flex().id("usage-title").gap(6).items_center().flex_1().min_w(0).cursor_pointer()
              .tooltip(shut ? "Show Claude usage" : "Hide Claude usage")
              .on_click((_e, cx) => { this.setCollapsed(!shut); cx.notify(); })
              .child(div().w(10).flex_none().text_size(10).text_color(t.dim).child(shut ? "▸" : "▾"))
              .child(svg("icons/claude.svg").size(13).flex_none().text_color(ORANGE))
              .child(div().text_size(12).font_semibold().text_color(t.foreground).child("Claude usage"))
              .when(Boolean(plan) && !shut, (el) => el.child(div().px(5).rounded(4).border_1().border_color(t.border).text_size(9.5).font_semibold().text_color(ORANGE).child(String(plan).toUpperCase())))
              .when(shut, (el) => el.child(div().truncate().text_size(10.5).text_color(t.dim).child(`${session ? `session ${Math.round(session.percent)}% · ` : ""}week ${week.percent}%`))),
          )
          .when(!shut, (el) => el.child(div().text_size(10.5).text_color(t.dim).child(this.busy ? "checking…" : this.checkedAt ? ago(this.checkedAt) : "")))
          .child(div().id("usage-refresh").px(6).py(1).rounded(4).text_size(12).text_color(t.dim).cursor_pointer()
            .hover((s) => s.bg(t.hover).text_color(t.foreground)).tooltip("Ask Claude Code for your usage (free: no model runs)")
            .on_click((_e, cx) => this.refresh(cx)).child("↻")),
      )
      .when(!shut, (el) => el.child(
        v_flex().id("usage-body").flex_1().min_h(0).px(10).pb(6).overflow_y_scrollbar()
          .when(Boolean(a && !a.signedIn), (el) => el.child(div().text_size(11).text_color(t.ansi1).child("Claude Code isn't signed in: run `claude auth login`.")))
          .when(Boolean(a?.signedIn && a.email), (el) => el.child(div().id("usage-account").truncate().pb(3).text_size(10.5).text_color(t.dim)
            .tooltip(`${a.org ?? ""}${a.endsAt ? ` · sign-in lasts until about ${new Date(a.endsAt).toDateString()}` : ""}`).child(a.org ? `${a.email} · ${a.org}` : a.email)))
          .when(Boolean(this.problem), (el) => el.child(div().pb(3).text_size(11).text_color("#ffb000").child(this.problem)))
          .children([...(rows.length ? rows.map((r) => this.row(t, r)) : [div().text_size(11).text_color(t.muted).child(this.busy ? "Asking Claude Code…" : "Not checked yet. Press ↻.")]), this.row(t, week)]),
      ));
  }
}
