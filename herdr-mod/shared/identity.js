// Shared by capcom-agents, capcom-pane and capcom-tabs: what a session is about (ticket key, branch).
// A session's colour is not worked out here: capcom-agents gives each live session its own and publishes it,
// with the persona's name, as `capcom.persona` (terminal -> { label, color, n }); `capcom.notes` holds the notes.

const KEY = /\b[A-Z][A-Z0-9]+-\d+\b/;

export const keyOf = (...texts) => texts.map((t) => KEY.exec(t ?? "")?.[0]).find(Boolean) ?? null;

export const who = ({ task, branch, tab, space }) => ({ key: keyOf(task, branch, tab, space), branch: branch || null });
