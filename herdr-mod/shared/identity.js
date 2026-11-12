// Shared by capcom-agents, capcom-pane and capcom-tabs: what a session is about (ticket key, branch) and its colour.
// capcom-agents publishes `capcom.notes` (terminal -> note) and `capcom.persona` (terminal -> { label, color }).

const KEY = /\b[A-Z][A-Z0-9]+-\d+\b/;
const PALETTE = ["#e0a458", "#5aa9e6", "#c77dff", "#6bd49b", "#ef7aa0", "#4fd1bd", "#f2c14e", "#8f9bff"];
const TRUNK = /^(main|master|develop)$/;

export const keyOf = (...texts) => texts.map((t) => KEY.exec(t ?? "")?.[0]).find(Boolean) ?? null;

export function colorOf(seed) {
  let h = 0;
  for (const c of seed) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return PALETTE[h % PALETTE.length];
}

// The ticket key (from the note's task, the branch or the tab and space names) and a colour for it.
// With no ticket, a feature branch colours the session; the trunk branch does not.
export function who({ task, branch, tab, space }) {
  const key = keyOf(task, branch, tab, space);
  const seed = key ?? (branch && !TRUNK.test(branch) ? branch : null);
  return { key, branch: branch || null, color: seed ? colorOf(seed) : null };
}
