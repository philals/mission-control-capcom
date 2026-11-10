// The progress strip under a PR's repo line: a segment per check, grouped by workflow.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";

const GREEN = "#22c55e";
const RED = "#ff1f1f";
const AMBER = "#ffb000";
const AMBER_DIM = "#7a5600";

// "11/12", or "1 ✗ · 6/10" when something failed.
export function stripCount(strip) {
  const passed = strip.filter((s) => s.state === "passed").length;
  const failed = strip.filter((s) => s.state === "failed").length;
  return `${failed ? `${failed} ✗ · ` : ""}${passed}/${strip.length}`;
}

function segment(s, t, pulse) {
  if (s.state === "running" && s.done != null) {
    return h_flex()
      .flex_1()
      .h(4)
      .rounded(1)
      .bg(t.dim)
      .child(div().flex_grow(s.done).h(4).rounded(1).bg(AMBER))
      .child(div().flex_grow(1 - s.done));
  }
  const colour = { passed: GREEN, failed: RED, running: pulse ? AMBER : AMBER_DIM }[s.state] ?? t.dim;
  return div().flex_1().h(4).rounded(1).bg(colour);
}

export function strip(strip, t, pulse) {
  return h_flex()
    .gap(1)
    .h(4)
    .mt(3)
    .children(strip.map((s, i) => segment(s, t, pulse).when(i > 0 && (s.group ?? "") !== (strip[i - 1].group ?? ""), (el) => el.ml(4))));
}

export const hasRunning = (prs) => prs.some((p) => p.strip?.some((s) => s.state === "running" && s.done == null));
