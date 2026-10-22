"use strict";
// Every piece of data from the server goes into the page as text, never as HTML.

const POLL_MS = 3000;
const $ = (id) => document.getElementById(id);

let data = null;
let failures = 0;
const openPrs = new Set();
const lastSig = {};
let showDone = false;
try {
  showDone = localStorage.getItem("capcom.showDone") === "1";
} catch (_) {}
let allPrs = false;

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined && text !== null) e.textContent = text;
  return e;
}

function add(parent, ...kids) {
  for (const k of kids) if (k) parent.append(k);
  return parent;
}

function safeUrl(u) {
  try {
    const url = new URL(u);
    return url.protocol === "https:" ? url.href : null;
  } catch (_) {
    return null;
  }
}

function link(text, url, cls) {
  const href = safeUrl(url);
  if (!href) return el("span", cls, text);
  const a = el("a", cls, text);
  a.href = href;
  a.target = "_blank";
  a.rel = "noopener noreferrer";
  return a;
}

function ago(iso) {
  const t = Date.parse(iso);
  if (!t) return "";
  const s = Math.max(0, Math.round((Date.now() - t) / 1000));
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} minute${m === 1 ? "" : "s"} ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} hour${h === 1 ? "" : "s"} ago`;
  return `${Math.floor(h / 24)} days ago`;
}

function dur(seconds) {
  const s = Math.max(0, Math.round(seconds));
  const m = Math.floor(s / 60);
  return m > 0 ? `${m}m ${String(s % 60).padStart(2, "0")}s` : `${s}s`;
}

function stamp(node, iso, kind) {
  node.dataset.ts = iso;
  node.dataset.kind = kind;
  return node;
}

function tick() {
  for (const n of document.querySelectorAll("[data-ts]")) {
    const iso = n.dataset.ts;
    if (n.dataset.kind === "ago") n.textContent = ago(iso);
    else if (n.dataset.kind === "run") n.textContent = `running ${dur((Date.now() - Date.parse(iso)) / 1000)}`;
  }
}

function copy(text, button) {
  const done = () => {
    const old = button.textContent;
    button.textContent = "✓";
    setTimeout(() => (button.textContent = old), 1200);
  };
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(done, () => fallbackCopy(text, done));
  } else {
    fallbackCopy(text, done);
  }
}

function fallbackCopy(text, done) {
  const box = el("textarea");
  box.value = text;
  box.style.position = "fixed";
  box.style.opacity = "0";
  document.body.append(box);
  box.select();
  try {
    document.execCommand("copy");
    done();
  } catch (_) {}
  box.remove();
}

function plate(parent, ...parts) {
  const p = el("div", "plate");
  for (const part of parts) p.append(part);
  parent.append(p);
}

// ---- header -------------------------------------------------------------------------------

function renderHeader() {
  const top = $("top");
  top.replaceChildren();
  const light = data.red > 0 ? el("span", "light red", `● NO-GO · ${data.red} red`) : el("span", "light green", "● GO");
  add(top, el("span", "brand", "CAPCOM"), light);
  if (data.awaitingReview > 0) add(top, el("span", "green", `◉ ${data.awaitingReview} awaiting review`));
  add(top, el("span", "spacer"));
  const status = el("span", "dim");
  if (data.prError) {
    status.textContent = `GitHub: ${data.prError}`;
    status.className = "red";
  } else if (data.loading && !data.prUpdated) {
    status.textContent = "loading pull requests…";
  } else if (data.prUpdated) {
    add(status, document.createTextNode("PRs updated "), stamp(el("span"), data.prUpdated, "ago"));
  }
  add(top, status);
}

// ---- board --------------------------------------------------------------------------------

const TASK_COLUMNS = [
  ["todo", "TO DO"],
  ["planning", "PLANNING"],
  ["planned", "PLANNED"],
  ["implementing", "IMPLEMENTING"],
  ["done", "DONE"],
  ["dropped", "DROPPED"],
];

function counts(story) {
  const c = {};
  for (const t of story.tasks) c[t.status] = (c[t.status] || 0) + 1;
  return c;
}

function storyColumn(story) {
  if (story.status === "in_review") return "review";
  if (story.status === "done") return "done";
  const c = counts(story);
  return story.tasks.length > 0 && (c.todo || 0) < story.tasks.length ? "doing" : "todo";
}

function column(title, count, ...kids) {
  const col = el("div", "column");
  plate(col, document.createTextNode(`${title} · `), el("span", "count", String(count)));
  if (kids.length === 0) add(col, el("div", "empty", "—"));
  for (const k of kids) col.append(k);
  return col;
}

function storyCard(story) {
  const c = counts(story);
  const total = story.tasks.length;
  const done = c.done || 0;
  const card = el("a", "card story");
  card.href = `#/story/${encodeURIComponent(story.key)}`;
  const head = el("div", "head");
  add(head, el("span", "key", story.key), el("span", "title", story.title));
  const bar = el("span", "bar");
  const fill = el("i");
  fill.style.width = total ? `${Math.round((100 * done) / total)}%` : "0%";
  bar.append(fill);
  const parts = ["todo", "planning", "planned", "implementing", "done"].filter((s) => c[s]).map((s) => `${c[s]} ${s}`);
  const meta = el("div", "meta");
  add(meta, el("span", done === total && total > 0 ? "green" : "", `${done}/${total} done`), bar, document.createTextNode(parts.join(" · ")));
  add(card, head, meta);
  return card;
}

function prFor(url) {
  return data.prs.find((p) => p.url.replace(/\/$/, "").toLowerCase() === url.replace(/\/$/, "").toLowerCase());
}

function prChip(task) {
  if (task.status === "done" || task.status === "dropped") return null;
  const live = task.prs.map((p) => prFor(p.url)).filter(Boolean);
  if (live.length === 0) return null;
  if (live.some((p) => p.red)) return ["✗ CI", "red"];
  if (live.some((p) => p.rail.some((s) => s.state === "wrong" && s.text.startsWith("threads")) || p.review === "changes")) return ["✎ answer", "orange"];
  if (live.some((p) => p.waiting === "reviewer")) return ["◉ review", "green"];
  if (live.some((p) => p.waiting === "merge")) return ["✔ to merge", "green"];
  if (live.some((p) => p.counts.running + p.counts.queued > 0)) return ["◔ CI", "cyan"];
  return ["✓ CI", "green"];
}

function taskStatus(task) {
  if (task.blocked) return [`✖ blocked: ${task.blocked}`, "red"];
  const waits = task.waitingOn.length ? `after ${task.waitingOn.join(", ")}` : "";
  switch (task.status) {
    case "todo":
      return [waits ? `· todo · ${waits}` : "· todo", "muted"];
    case "planning":
      return ["◌ planning", "orange"];
    case "planned":
      return waits ? [`◷ waits ${task.waitingOn.join(", ")}`, "orange"] : ["▶ ready to implement", "green"];
    case "implementing": {
      if (task.prs.length === 0) return ["● implementing", "cyan"];
      const merged = task.prs.filter((p) => p.state === "merged").length;
      return merged === task.prs.length ? ["✓ all PRs merged", "green"] : [`● implementing · PRs ${merged}/${task.prs.length}`, "cyan"];
    }
    case "done":
      return ["✓ done", "green"];
    default:
      return ["✕ dropped", "dim"];
  }
}

function taskCard(task) {
  const card = el("div", "card");
  const head = el("div", "head");
  add(head, el("span", "key", task.id), el("span", "title", task.title), el("span", "badge dim", task.type));
  const [text, tone] = taskStatus(task);
  const meta = el("div", "meta");
  add(meta, el("span", tone, text));
  const chip = prChip(task);
  if (chip) add(meta, document.createTextNode("  "), el("span", chip[1], chip[0]));
  if (task.working) add(meta, document.createTextNode("  "), el("span", "cyan", "🤖 agent"));
  add(card, head, meta);
  if (task.prs.length) {
    const chips = el("div", "chips");
    for (const p of task.prs) {
      const name = p.url.split("/").slice(-1)[0];
      add(chips, link(`${p.repo}#${name} ${p.state.toUpperCase()}`, p.url, `chip ${p.state}`));
    }
    card.append(chips);
  }
  return card;
}

function renderBoard() {
  const board = $("board");
  board.replaceChildren();
  const route = location.hash.match(/^#\/story\/(.+)$/);
  const story = route ? data.stories.find((s) => s.key === decodeURIComponent(route[1])) : null;
  if (story) {
    plate(board, document.createTextNode(`${story.key} · `), el("span", "count", `${story.tasks.length} tasks`));
    const crumb = el("div", "crumb");
    const back = el("button", "", "← stories");
    back.onclick = () => (location.hash = "");
    add(crumb, back, el("span", "bold", story.title));
    if (story.jiraUrl) add(crumb, link("Jira ↗", story.jiraUrl));
    board.append(crumb);
    const cols = el("div", "columns");
    for (const [status, label] of TASK_COLUMNS) {
      const tasks = story.tasks.filter((t) => t.status === status);
      if (status === "dropped" && tasks.length === 0) continue;
      cols.append(column(label, tasks.length, ...tasks.map(taskCard)));
    }
    board.append(cols);
    return;
  }
  plate(board, document.createTextNode("STORIES · "), el("span", "count", String(data.stories.length)));
  const tools = el("div", "tools");
  const toggle = el("button", "toggle", showDone ? "[ Hide done ]" : "[ Show done ]");
  toggle.onclick = () => {
    showDone = !showDone;
    try {
      localStorage.setItem("capcom.showDone", showDone ? "1" : "0");
    } catch (_) {}
    lastSig.board = null;
    render();
  };
  tools.append(toggle);
  board.append(tools);
  if (data.stories.length === 0) add(board, el("div", "empty-note", "No stories yet. Break one down with /story-break-down."));
  const cols = el("div", "columns");
  const groups = [
    ["todo", "TO DO"],
    ["doing", "DOING"],
    ["review", "IN REVIEW"],
  ];
  if (showDone) groups.push(["done", "DONE"]);
  for (const [key, label] of groups) {
    const stories = data.stories.filter((s) => storyColumn(s) === key);
    cols.append(column(label, stories.length, ...stories.map(storyCard)));
  }
  board.append(cols);
  for (const b of data.broken) add(board, el("div", "red", `${b.key}: ${b.error}`));
}

// ---- pull requests ------------------------------------------------------------------------

function stageIcon(state) {
  return { running: "◔", queued: "●", failed: "✗", passed: "✓", cancelled: "⊘", skipped: "–" }[state] || "·";
}

function checkRow(check) {
  const li = el("li", check.state);
  const name = el("span", "name");
  name.append(check.url ? link(check.label, check.url) : document.createTextNode(check.label));
  const st = el("span", "st");
  if (check.state === "running" && check.startedAt) stamp(st, check.startedAt, "run");
  else if (check.completedAt && check.startedAt) st.textContent = `${check.state} ${dur((Date.parse(check.completedAt) - Date.parse(check.startedAt)) / 1000)}`;
  else st.textContent = check.state;
  add(li, el("span", "icon", stageIcon(check.state)), name, st);
  return li;
}

function prCard(pr) {
  const card = el("article", "pr" + (pr.waiting === "reviewer" ? " go" : "") + (pr.red ? " red" : ""));
  const open = openPrs.has(pr.url);
  const line1 = el("div", "line1");
  add(line1, el("span", "badge " + (pr.draft ? "dim" : "green"), pr.draft ? "DRAFT" : "READY"));
  add(line1, link(`${pr.repo}#${pr.number}`, pr.url, "cyan bold"));
  if (pr.tag) add(line1, el("span", "magenta bold", `${pr.tag.story} · ${pr.tag.task}`));
  else if (pr.pasted) add(line1, el("span", "magenta bold", "pasted"));
  for (const l of pr.labels) add(line1, el("span", "dim", `[${l}]`));
  const right = el("span", "right");
  if (pr.comments) right.append(document.createTextNode(`✎ ${pr.comments}  `));
  right.append(stamp(el("span"), pr.updatedAt, "ago"));
  line1.append(right);
  card.append(line1);

  const title = link(pr.title, pr.url + "/files", "ptitle");
  card.append(title);

  if (pr.branch) {
    const branch = el("div", "branch");
    const button = el("button", "copy", "📋");
    button.title = "copy the branch name";
    button.onclick = () => copy(pr.branch, button);
    add(branch, el("span", "", "⎇"), el("code", "", pr.branch), button);
    card.append(branch);
  }

  const rail = el("div", "rail");
  if (pr.waiting === "reviewer") add(rail, el("span", "golight", "GO · ◉ AWAITING REVIEW"));
  else if (pr.waiting === "merge") add(rail, el("span", "green bold", "✔ APPROVED · READY TO MERGE"));
  for (const s of pr.rail) add(rail, el("span", "step " + s.state, s.text));
  card.append(rail);

  const openChecks = pr.checks.filter((c) => ["running", "queued", "failed"].includes(c.state));
  const shown = open ? pr.checks : openChecks.slice(0, 8);
  if (shown.length) {
    const list = el("ul", "checks");
    for (const c of shown) list.append(checkRow(c));
    card.append(list);
  }
  if (pr.checks.length > shown.length || open) {
    const more = el("button", "toggle", open ? "show fewer checks" : `show all ${pr.checks.length} checks`);
    more.onclick = () => {
      if (openPrs.has(pr.url)) openPrs.delete(pr.url);
      else openPrs.add(pr.url);
      lastSig.prs = null;
      render();
    };
    card.append(more);
  }
  if (pr.blocker && !pr.draft) card.append(el("div", "mergeblock", `not merging yet: ${pr.blocker}`));
  return card;
}

function renderPrs() {
  const panel = $("prs");
  panel.replaceChildren();
  const route = location.hash.match(/^#\/story\/(.+)$/);
  const story = route ? data.stories.find((s) => s.key === decodeURIComponent(route[1])) : null;
  let list = data.prs;
  if (story && !allPrs) list = list.filter((p) => p.tag && p.tag.story === story.key);
  const waiting = list.filter((p) => p.waiting === "reviewer").length;
  const parts = [document.createTextNode("PULL REQUESTS · "), el("span", "count", String(list.length))];
  if (waiting) parts.push(el("span", "go", ` · ${waiting} awaiting review`));
  plate(panel, ...parts);
  if (story) {
    const tools = el("div", "tools");
    const toggle = el("button", "toggle", allPrs ? "[ this story's PRs ]" : "[ all PRs ]");
    toggle.onclick = () => {
      allPrs = !allPrs;
      lastSig.prs = null;
      render();
    };
    tools.append(toggle);
    panel.append(tools);
  }
  if (list.length === 0) {
    const text = data.prError ? "Could not read GitHub." : data.loading && !data.prUpdated ? "Loading pull requests…" : "No open pull requests.";
    add(panel, el("div", "empty-note", text));
  }
  for (const pr of list) panel.append(prCard(pr));
}

// ---- loop ---------------------------------------------------------------------------------

function sig(value) {
  return JSON.stringify(value);
}

function render() {
  if (!data) return;
  const header = sig([data.red, data.awaitingReview, data.prError, data.prUpdated, data.loading]);
  if (header !== lastSig.header) {
    lastSig.header = header;
    renderHeader();
  }
  const board = sig([data.stories, data.broken, location.hash, showDone, data.prs.map((p) => [p.url, p.red, p.waiting, p.counts, p.review])]);
  if (board !== lastSig.board) {
    lastSig.board = board;
    renderBoard();
  }
  const prs = sig([data.prs, location.hash, allPrs, [...openPrs]]);
  if (prs !== lastSig.prs) {
    lastSig.prs = prs;
    renderPrs();
  }
  tick();
}

async function load() {
  if (document.hidden) return;
  try {
    const response = await fetch("/api/state", { cache: "no-store" });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    data = await response.json();
    failures = 0;
    $("banner").hidden = true;
    render();
  } catch (e) {
    failures += 1;
    if (failures >= 2) {
      $("banner").hidden = false;
      $("banner").textContent = "Cannot reach capcom web. Is it still running?";
    }
  }
}

window.addEventListener("hashchange", () => {
  lastSig.board = lastSig.prs = null;
  render();
});
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) load();
});
setInterval(load, POLL_MS);
setInterval(tick, 1000);
load();
