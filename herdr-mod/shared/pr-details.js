// Shared by capcom-prs (My Watch) and capcom-agents: the PR dot colour, the expanded details and `capcom pr-do`.

import { div, svg } from "gpui-kit";
import { v_flex, h_flex } from "gpui-base";
import { exec } from "herdr";

export const SH = ["sh", "-c", 'PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"; exec "$@"', "sh"];

export const capcom = (args, options) => exec(SH[0], [...SH.slice(1), "capcom", ...args], options);

export function openUrl(url) {
  exec(SH[0], [...SH.slice(1), "sh", "-c", 'xdg-open "$1" 2>/dev/null || open "$1"', "sh", url]).catch(() => {});
}

export const short = (repo) => repo.split("/").pop();

// GitHub's merged purple.
export const MERGED = "#a371f7";
// Something waiting on you: neon blue, white on the off beat of a pulse.
export const attention = (pulse = true) => (pulse ? "#00b3ff" : "#ffffff");
// A PR with a check that waits for you (a Chromatic review).
export const needsYou = (pr) => !!(pr.checks?.some((k) => k.attention) || pr.strip?.some((s) => s.attention));

// A PR icon that waits for you: neon blue and white in turn, growing and shrinking in a box of `big` so nothing around it moves.
export function attentionIcon(path, small, big, pulse) {
  return h_flex().w(big).h(big).flex_none().items_center().justify_center().child(svg(path).size(pulse ? big : small).flex_none().text_color(attention(pulse)));
}

export function dotColor(pr, merged, t, pulse = true) {
  return merged ? MERGED : needsYou(pr) ? attention(pulse) : pr.ci === "red" ? "#ff1f1f" : pr.ci === "pending" ? "#ffb000" : pr.ci === "none" ? t.dim : "#22c55e";
}

// `capcom pr-do`: change a PR or a setting; resolves to { text, ok }.
export async function prDo(action, url, on) {
  const args = ["pr-do", action, ...(url ? ["--url", url] : []), ...(on === undefined ? [] : ["--on", String(on)])];
  try {
    const out = JSON.parse((await capcom(args, { timeout: 60000 })).stdout);
    return { text: out.message, ok: out.ok };
  } catch (e) {
    return { text: String(e?.message ?? e), ok: false };
  }
}

// A tick box: ☑ or ☐ and its label.
export function pill(id, label, on, t, handler, tip) {
  return h_flex()
    .id(id)
    .gap(4)
    .items_center()
    .py(2)
    .text_size(12)
    .cursor_pointer()
    .text_color(on ? t.foreground : t.muted)
    .hover((s) => s.text_color(t.foreground))
    .tooltip(tip)
    .on_click((_e, cx) => handler(cx))
    .child(div().text_color(on ? t.ansi2 : t.dim).child(on ? "☑" : "☐"))
    .child(label);
}

// The rail, blocker, checks and toggles. `change(cx, action, url, on)` does the toggling.
export function prDetails(pr, t, change, pad = true, pulse = true) {
  const RAIL = { done: t.ansi2, going: t.ansi3, wrong: t.ansi1, pending: t.dim };
  const CHECK = { failed: t.ansi1, running: t.ansi3, queued: t.dim, cancelled: t.dim };
  const url = pr.url;
  const rows = [];
  rows.push(
    h_flex().gap(8).flex_wrap().children((pr.rail ?? []).filter((r) => r.text !== "ready" && r.text !== "merge").map((r) => div().text_size(11).text_color(RAIL[r.state] ?? t.dim).child(`${r.state === "done" ? "✓" : r.state === "wrong" ? "✗" : r.state === "going" ? "◐" : "○"} ${r.text}`))),
  );
  if (pr.blocker) rows.push(div().text_size(11).text_color(t.muted).child(`In the way: ${pr.blocker}`));
  for (const k of (pr.checks ?? []).slice(0, 6)) {
    rows.push(
      div()
        .id(`ck-${url}-${k.url ?? k.label}`)
        .text_size(11)
        .text_color(k.attention ? attention(pulse) : (CHECK[k.state] ?? t.muted))
        .when(!!k.attention, (el) => el.font_semibold())
        .truncate()
        .cursor_pointer()
        .hover((s) => s.text_color(t.foreground))
        .tooltip(`${k.attention ? "needs you" : k.state}: ${k.label}`)
        .on_click(() => k.url && openUrl(k.url))
        .child(`${k.attention ? "needs you" : k.state} · ${k.label.split(" / ").slice(-2).join(" / ")}`),
    );
  }
  if ((pr.checks ?? []).length > 6) rows.push(div().text_size(11).text_color(t.dim).child(`…and ${pr.checks.length - 6} more`));
  const meta = [pr.branch, ...(pr.labels ?? [])].filter(Boolean).join(" · ");
  if (meta) rows.push(div().text_size(11).text_color(t.dim).truncate().child(meta));
  if (pr.managed) {
    rows.push(
      h_flex()
        .gap(10)
        .flex_wrap()
        .when(pr.automerge?.available, (el) => el.child(pill(`am-${url}`, "automerge", pr.automerge.on, t, (cx) => change(cx, "automerge", url, !pr.automerge.on), "Add or remove the merge bot's automerge label")))
        .when(pr.autoupdate?.available, (el) => el.child(pill(`au-${url}`, pr.autoupdate.always ? "auto-update (always)" : "auto-update", pr.autoupdate.on || pr.autoupdate.always, t, (cx) => pr.autoupdate.always ? null : change(cx, "autoupdate", url, !pr.autoupdate.on), pr.autoupdate.always ? "The merge bot updates every PR in this repo" : "Add or remove the merge bot's autoupdate label")))
        .child(pill(`af-${url}`, "auto-fix", pr.autofix, t, (cx) => change(cx, "autofix", url, !pr.autofix), "Let capcom fix failing checks and conflicts on this PR"))
        .child(pill(`ar-${url}`, "auto-review", pr.autoreview, t, (cx) => change(cx, "autoreview", url, !pr.autoreview), "Let capcom request and address Copilot's review on this PR")),
    );
  }
  return v_flex().gap(4).when(pad, (el) => el.pl(26).pr(10).pb(8)).children(rows);
}
