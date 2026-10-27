// ---- what is where ------------------------------------------------------------------------

function routeStory() {
  const m = location.hash.match(/^#\/story\/(.+)$/);
  return m && data ? data.stories.find((s) => s.key === decodeURIComponent(m[1])) || null : null;
}

function prByUrl(url) {
  return data.prs.find((p) => p.url.replace(/\/$/, "").toLowerCase() === url.replace(/\/$/, "").toLowerCase());
}

function prName(pr) {
  return `${pr.repo}#${pr.number}`;
}

// A console: a heading strip (its name, and its tools at the right) that stays put, and what is
// inside scrolls by itself.
function shell(panel, plateParts, tools) {
  panel.replaceChildren();
  const head = el("div", "h");
  plate(head, ...plateParts);
  if (tools) head.append(tools);
  const body = el("div", "scroll");
  panel.append(head, body);
  return body;
}

const STATIONS = ["Booster", "Retro", "Fido", "Guidance", "Surgeon", "Control", "Telemetry", "Capcom"];

// The 1960s go/no-go poll, while the first list comes back from GitHub.
function pollNode(what) {
  const wrap = el("div", "poll");
  wrap.append(el("h5", "", `Go / no-go poll · ${what}`));
  STATIONS.forEach((name, i) => {
    const row = el("div", "row");
    row.style.setProperty("--d", `${0.22 * (i + 1)}s`);
    add(row, el("span", "lamp"), el("span", "", name), el("span", "dots"), el("span", "verdict", "go"));
    wrap.append(row);
  });
  wrap.append(el("p", "", "All stations go. Standing by for the data."));
  return wrap;
}

function lampButton(label, on, onclick, title) {
  const b = btn("", onclick, on ? "tick on" : "tick", title);
  add(b, el("span", "lamp" + (on ? " on green" : "")), document.createTextNode(label));
  return b;
}

function kv(label, value) {
  const row = el("div", "kv");
  add(row, el("span", "k", label), typeof value === "string" ? el("span", "v", value) : value);
  return row;
}

// ---- actions on a PR (buttons, menus, the palette and the keys all use these) -------------

function openDiff(pr) {
  openExternal(pr.url + "/files");
}

function copyLink(pr, button) {
  copy(pr.url, button);
}

function copyBranch(pr, button) {
  copy(pr.branch, button);
}

function toggleReady(pr) {
  if (pr.draft) {
    ask(`Mark ${prName(pr)} ready for review?`, ["Reviewers are notified when it leaves draft.", "This runs `gh pr ready`."], "Mark ready", "pr.ready", { url: pr.url });
  } else {
    ask(`Turn ${prName(pr)} back into a draft?`, ["A draft cannot be merged, so automerge stops until it is ready again."], "Turn into a draft", "pr.draft", { url: pr.url });
  }
}

function toggleTick(pr, fix) {
  if (!pr.managed) return toast("you did not write this PR and are not following it, so capcom leaves it alone");
  act(fix ? "pr.autofix" : "pr.autoreview", { url: pr.url, on: !(fix ? pr.autofix : pr.autoreview) });
}

function toggleAutomerge(pr) {
  if (!pr.managed || !pr.automerge.available) return toast("automerge needs your own or a followed PR in a repo whose merge bot is set up (a .kodiak.toml)");
  if (pr.automerge.on) {
    ask(`Turn automerge off for ${prName(pr)}?`, ["This takes the repo's automerge labels off the PR."], "Turn off", "pr.automerge", { url: pr.url, on: false });
  } else {
    ask(`Turn automerge on for ${prName(pr)}?`, [`This adds the label "${pr.automerge.labels[0]}": the merge bot then merges the PR by itself once it is approved and green.`], "Turn on", "pr.automerge", { url: pr.url, on: true });
  }
}

function goToAgent(pr) {
  if (pr.agent && pr.agent.link) openExternal(pr.agent.link, "opening the session in Claude Code desktop…");
  else toast("no Claude Code session is known to have made this PR");
}

function fixInSession(pr) {
  if (pr.fix) openExternal(pr.fix.link, "opened a new session with /pr-address typed in: press Enter to send it");
}

function unfollow(pr) {
  if (!pr.pasted) return toast("only a PR you followed by link can be taken out of the list");
  act("pr.unfollow", { url: pr.url });
}

function prMenu(pr, x, y) {
  ui.selPr = pr.url;
  openMenu(x, y, [
    ["Open the diff on GitHub", () => openDiff(pr)],
    ["Copy the link", () => copyLink(pr)],
    pr.branch ? ["Copy the branch name", () => copyBranch(pr)] : null,
    ["Details", () => prSheet(pr.url)],
    [pr.draft ? "Mark ready for review…" : "Turn back into a draft…", () => toggleReady(pr)],
    pr.managed ? [`Turn auto-fix ${pr.autofix ? "off" : "on"}`, () => toggleTick(pr, true)] : null,
    pr.managed ? [`Turn auto-review ${pr.autoreview ? "off" : "on"}`, () => toggleTick(pr, false)] : null,
    pr.managed && pr.automerge.available ? [`Turn automerge ${pr.automerge.on ? "off" : "on"}…`, () => toggleAutomerge(pr)] : null,
    pr.fix ? ["Fix it in a new Claude Code session", () => fixInSession(pr)] : null,
    pr.agent && pr.agent.link ? ["Go to the session that made it", () => goToAgent(pr)] : null,
    pr.pasted ? ["Stop following it", () => unfollow(pr)] : null,
  ]);
}

// ---- header -------------------------------------------------------------------------------

function tile(lit, text, cls, onclick, title) {
  const t = el("span", "tile" + (onclick ? " press" : "") + (lit ? " hot" : ""));
  if (title) t.title = title;
  if (onclick) t.addEventListener("click", onclick);
  add(t, el("span", "lamp" + (lit ? ` on ${cls}` : "")), text);
  return t;
}

// The working week: Monday to Friday, 06:00 to 17:00, local time.
const WORK_START = 6 * 60;
const WORK_END = 17 * 60;
const WORK_DAYS = 5;
const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

// How far through the working week it is, as whole percent, and a label for the moment.
function workWeek(now = new Date()) {
  const dow = now.getDay() || 7; // Monday is 1, Sunday is 7
  const day = WORK_END - WORK_START;
  const hhmm = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
  if (dow > WORK_DAYS) return { pct: 100, label: `${DAY_NAMES[now.getDay()]} ${hhmm}`, over: true };
  const minutes = now.getHours() * 60 + now.getMinutes();
  const today = Math.max(0, Math.min(day, minutes - WORK_START));
  return { pct: Math.floor(((dow - 1) * day + today) * 100 / (day * WORK_DAYS)), label: `${DAY_NAMES[now.getDay()]} ${hhmm}`, over: false };
}

// A gauge: a name, a row of segments lit up to `pct`, the figure, and a note.
function gauge(name, pct, tone, note, title, id) {
  const t = el("span", `tile gauge ${tone}`);
  if (title) t.title = title;
  const seg = el("span", "seg");
  if (id) seg.id = `${id}-seg`;
  const lit = pct === null ? 0 : Math.max(0, Math.min(10, Math.round(pct / 10)));
  for (let i = 0; i < 10; i++) seg.append(el("i", i < lit ? "on" : ""));
  const value = el("span", "num", pct === null ? "—" : `${Math.round(pct)}%`);
  if (id) value.id = `${id}-pct`;
  add(t, el("span", "", name), seg, value);
  if (note) {
    const n = el("span", "note-g", note);
    if (id) n.id = `${id}-note`;
    t.append(n);
  }
  return t;
}

function untilText(iso) {
  const s = Math.round((Date.parse(iso) - Date.now()) / 1000);
  if (!(s > 0)) return "";
  const m = Math.round(s / 60);
  if (m < 90) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 36) return `${h}h ${String(m % 60).padStart(2, "0")}m`;
  return `${Math.floor(h / 24)}d ${h % 24}h`;
}

// Plan usage as the status line last reported it, and the working week.
function usageGauges() {
  const out = [];
  const u = data.usage;
  const stale = u && Date.now() - Date.parse(u.at) > 20 * 60 * 1000;
  const tone = (p) => (p >= 85 ? "hot" : p >= 60 ? "warn" : "ok");
  const one = (name, w, what) => {
    if (!w) return null;
    const over = w.resetsAt && Date.parse(w.resetsAt) < Date.now();
    const note = over ? "reset" : w.resetsAt ? `resets in ${untilText(w.resetsAt)}` : "";
    const title = `${what}: ${Math.round(w.pct)}% used${w.resetsAt ? `, resets ${new Date(w.resetsAt).toLocaleString([], { weekday: "short", hour: "2-digit", minute: "2-digit" })}` : ""}. As of ${ago(u.at)}${stale ? " (it only updates while a Claude Code session is running)" : ""}.`;
    return gauge(name, over ? null : w.pct, over || stale ? "old" : tone(w.pct), note, title);
  };
  if (u) {
    const five = one("5h", u.fiveHour, "Plan usage in the 5-hour window");
    const week = one("7d", u.sevenDay, "Plan usage in the weekly window");
    if (five) out.push(five);
    if (week) out.push(week);
  }
  const w = workWeek();
  out.push(gauge("week", w.pct, "wk", w.label, `${w.pct}% of the working week (Monday to Friday, 06:00 to 17:00) has gone`, "wk"));
  return out;
}

function renderHeader() {
  renderHeaderNow();
}

function renderHeaderNow() {
  const top = $("top");
  top.replaceChildren();
  const brand = el("span", "brand");
  const logo = el("img", "logo");
  logo.src = "/favicon.svg";
  logo.alt = "";
  add(brand, logo, document.createTextNode("CAPCOM"));
  const gonogo = el("span", "gonogo");
  add(
    gonogo,
    tile(data.red === 0, "go", "green"),
    tile(data.red > 0, data.red > 0 ? `no-go · ${data.red} red` : "no-go", "red", jumpToRed, "jump to the first red PR"),
  );
  add(top, brand, gonogo);
  if (data.redWithAutofix > 0) add(top, tile(true, `auto-fix on for ${data.redWithAutofix}`, "cyan"));
  if (data.awaitingReview > 0) add(top, tile(true, `${data.awaitingReview} awaiting review`, "green"));
  if (data.agents.needsYou > 0) add(top, tile(true, `${data.agents.needsYou} agent${data.agents.needsYou === 1 ? "" : "s"} need you`, "amber pulse"));
  add(top, el("span", "spacer"));
  for (const g of usageGauges()) top.append(g);
  const status = el("span", "dim status");
  if (data.prError) {
    status.textContent = `GitHub: ${data.prError}`;
    status.className = "red status";
  } else if (data.loading && !data.prUpdated) {
    status.textContent = "loading pull requests…";
  } else if (data.prUpdated) {
    add(status, document.createTextNode("PRs updated "), stamp(el("span"), data.prUpdated, "ago"));
  }
  const clock = el("span", "clock");
  add(clock, el("small", "", "local"), el("span", "", ""));
  clock.lastChild.id = "clock";
  const events = data.events.length;
  add(
    top,
    status,
    clock,
    btn("+ follow", followDialog, "head", "follow someone else's PR or a workflow run by its link (or just paste the link)"),
    btn("+ new story", newStoryDialog, "head", "start a Jira story's break-down in a new Claude Code session"),
    btn(events ? `log ${events}` : "log", eventsDialog, "head", "what capcom did by itself"),
    btn("⚙", settingsDialog, "head", "settings"),
    btn("⌘", paletteDialog, "head", "every action (Ctrl-K)"),
    btn("?", helpDialog, "head", "keys and what things mean"),
  );
  tickClock();
}

function tickClock() {
  const c = $("clock");
  if (c) c.textContent = new Date().toLocaleTimeString([], { hour12: false });
  const pct = $("wk-pct");
  if (pct) {
    const w = workWeek();
    pct.textContent = `${w.pct}%`;
    const lit = Math.max(0, Math.min(10, Math.round(w.pct / 10)));
    [...$("wk-seg").children].forEach((n, i) => n.classList.toggle("on", i < lit));
    $("wk-note").textContent = w.label;
  }
}

function jumpToRed() {
  const red = data.prs.find((p) => p.red);
  if (!red) return toast("nothing is red");
  ui.selPr = red.url;
  ui.allPrs = true;
  lastSig.prs = null;
  setView("prs");
  scrollToSelected();
  toast(`${prName(red)} is red`);
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

function startSession(link, what) {
  openExternal(link, `opened a new Claude Code session with ${what} typed in: press Enter to send it`);
}

function markStoryDone(story) {
  ask(`Mark ${story.key} done?`, [story.status === "in_review" ? "This accepts the review." : "This puts the story into DONE and skips the review.", "Every task must be done or dropped."], "Mark done", "story.done", { key: story.key });
}

function reopenStory(story) {
  ask(`Reopen ${story.key}?`, ["It goes back to DOING."], "Reopen", "story.reopen", { key: story.key });
}

function syncStory(story) {
  ask(`Sync ${story.key} with GitHub?`, ["Every recorded PR's state is written to the board, and a task whose PRs are all merged is finished."], "Sync", "story.sync", { key: story.key });
}

function finishTask(story, task) {
  ask(`Finish ${task.id}?`, [task.allMerged ? "Every recorded PR is merged." : "Its recorded PRs are looked up on GitHub first: it only finishes when they are all merged.", `${task.id} — ${task.title}`], "Finish", "task.finish", { key: story.key, id: task.id }, (out) => {
    if (out && out.cleanup) {
      openModal("Clean up worktrees?", () => ({
        body: [el("p", "", `${task.id}'s progress notes mention a git worktree. Open a session to clean them up?`)],
        foot: [btn("No", closeModal), btn("Open a session", () => { closeModal(); startSession(out.cleanup.link, "the clean-up request"); }, "primary")],
      }));
    }
  });
}

function storyMenu(story, x, y) {
  openMenu(x, y, [
    ["Open", () => (location.hash = `#/story/${encodeURIComponent(story.key)}`)],
    story.tasks.length === 0 ? ["Break it down in a new session ↗", () => startSession(story.breakdown, "/story-break-down")] : null,
    story.allFinished && story.status === "in_progress" ? ["Review it in a new session ↗", () => startSession(story.review, "/story-review")] : null,
    story.status === "in_review" ? ["Run the review again in a new session ↗", () => startSession(story.review, "/story-review")] : null,
    story.status === "in_progress" && story.allFinished ? ["Mark done…", () => markStoryDone(story)] : null,
    story.status === "in_review" ? ["Accept the review (mark done)…", () => markStoryDone(story)] : null,
    story.status === "in_review" || story.status === "done" ? ["Reopen…", () => reopenStory(story)] : null,
    story.jiraUrl ? ["Open in Jira", () => openExternal(story.jiraUrl)] : null,
    ["Sync PR states with GitHub…", () => syncStory(story)],
  ]);
}

function storyRow(story, col) {
  const c = counts(story);
  const total = story.tasks.length;
  const done = c.done || 0;
  const row = el("a", "srow " + col);
  row.href = `#/story/${encodeURIComponent(story.key)}`;
  row.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    storyMenu(story, ev.clientX, ev.clientY);
  });
  const parts = ["todo", "planning", "planned", "implementing", "done"].filter((s) => c[s]).map((s) => `${c[s]} ${s}`);
  const mid = el("span");
  add(mid, el("div", "ttl", story.title), el("div", "meta", total ? `${done}/${total} done · ${parts.join(" · ")}` : "not broken down yet"));
  const bar = el("span");
  const track = el("div", "bar");
  const fill = el("i");
  fill.style.width = total ? `${Math.round((100 * done) / total)}%` : "0%";
  track.append(fill);
  bar.append(track);
  const more = el("button", "more", "⋯");
  more.type = "button";
  more.title = "actions for this story (or right-click)";
  more.addEventListener("click", (ev) => {
    ev.preventDefault();
    ev.stopPropagation();
    const r = more.getBoundingClientRect();
    storyMenu(story, r.left, r.bottom);
  });
  add(row, el("span", "key", story.key), mid, bar, more);
  return row;
}

function prChip(task) {
  if (task.status === "done" || task.status === "dropped") return null;
  const live = task.prs.map((p) => prByUrl(p.url)).filter(Boolean);
  if (live.length === 0) return null;
  if (live.some((p) => p.red)) return ["✗ CI", "red"];
  if (live.some((p) => p.copilotOpen > 0 || p.review === "changes")) return ["✎ answer", "orange"];
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
      if (task.allMerged) return ["✓ all PRs merged · finish it", "green"];
      const merged = task.prs.filter((p) => p.state === "merged").length;
      return [`● implementing · PRs ${merged}/${task.prs.length}`, "cyan"];
    }
    case "done":
      return ["✓ done", "green"];
    default:
      return ["✕ dropped", "dim"];
  }
}

function taskCard(story, task) {
  const card = el("div", "card task s-" + task.status);
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
  const actions = el("div", "task-actions");
  add(actions, btn("details", () => taskSheet(story.key, task.id), "small", "its plan or description, and its file"));
  if (task.status === "todo") add(actions, btn("plan ↗", () => startSession(task.plan, `/story-plan-task ${story.key} ${task.id}`), "small", "open a new Claude Code session with the planning skill typed in"));
  if ((task.status === "todo" || task.status === "planned") && task.waitingOn.length === 0 && !task.blocked) add(actions, btn("implement ↗", () => startSession(task.implement, `/story-implement-task ${story.key} ${task.id}`), "small", "open a new Claude Code session with the implement skill typed in"));
  if (task.status === "implementing") add(actions, btn(task.allMerged ? "finish ✓" : "finish…", () => finishTask(story, task), task.allMerged ? "small primary" : "small", "check its PRs on GitHub and finish it when they are all merged"));
  card.append(actions);
  return card;
}

function renderBoard() {
  const panel = $("board");
  const story = routeStory();
  if (story) {
    const body = shell(panel, [document.createTextNode(`${story.key} · `), el("span", "count", `${story.tasks.length} tasks`)]);
    const crumb = el("div", "crumb");
    const back = el("button", "", "← stories");
    back.onclick = () => (location.hash = "");
    add(crumb, back, el("span", "bold", story.title));
    if (story.jiraUrl) add(crumb, link("Jira ↗", story.jiraUrl));
    add(crumb, el("span", "spacer"), btn("sync", () => syncStory(story), "small", "write PR states to the board"), btn("⋯", (ev) => {
      const r = ev.target.getBoundingClientRect();
      storyMenu(story, r.left, r.bottom);
    }, "small"));
    body.append(crumb);
    const cols = el("div", "columns");
    for (const [status, label] of TASK_COLUMNS) {
      const tasks = story.tasks.filter((t) => t.status === status);
      if (status === "dropped" && tasks.length === 0) continue;
      cols.append(column(label, tasks.length, ...tasks.map((t) => taskCard(story, t))));
    }
    body.append(cols);
    return;
  }
  const tools = el("div", "tools");
  const toggle = el("button", "toggle", ui.showDone ? "[ Hide done ]" : "[ Show done ]");
  toggle.onclick = toggleShowDone;
  tools.append(toggle);
  const body = shell(panel, [document.createTextNode("STORIES · "), el("span", "count", String(data.stories.length))], tools);
  if (data.stories.length === 0) add(body, el("div", "empty-note", "No stories yet. Use “+ new story” to break one down."));
  const groups = [
    ["doing", "DOING"],
    ["review", "IN REVIEW"],
    ["todo", "TO DO"],
  ];
  if (ui.showDone) groups.push(["done", "DONE"]);
  for (const [key, label] of groups) {
    const stories = data.stories.filter((s) => storyColumn(s) === key);
    if (stories.length === 0) continue;
    const sep = el("div", "sep");
    add(sep, document.createTextNode(`${label} · `), el("span", "count", String(stories.length)));
    body.append(sep);
    for (const s of stories) body.append(storyRow(s, key));
  }
  for (const b of data.broken) add(body, el("div", "red", `${b.key}: ${b.error}`));
}

function toggleShowDone() {
  ui.showDone = !ui.showDone;
  try {
    localStorage.setItem("capcom.showDone", ui.showDone ? "1" : "0");
  } catch (_) {}
  lastSig.board = null;
  render();
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
  if (check.state === "running" && check.startedAt) stamp(st, check.startedAt, "run", check.usual);
  else if (check.completedAt && check.startedAt) st.textContent = `${check.state} ${dur((Date.parse(check.completedAt) - Date.parse(check.startedAt)) / 1000)}`;
  else st.textContent = check.state;
  add(li, el("span", "icon", stageIcon(check.state)), name, st);
  return li;
}

// A PR with nothing to do takes one line: a draft with nothing wrong, or a green PR that needs no review.
function isQuiet(pr) {
  if (pr.red || pr.fix || pr.copilotOpen > 0 || pr.counts.running + pr.counts.queued > 0 || pr.merge === "conflicting") return false;
  return pr.draft || pr.waiting === "green";
}

function railStep(pr, s) {
  if (s.text === "CI running" && pr.leftSeconds > 0) return `CI ${approx(pr.leftSeconds)} left`;
  return s.text;
}

function tickButton(label, on, onclick, title) {
  return lampButton(label, on, onclick, title);
}

function prButtons(pr) {
  const row = el("div", "pr-buttons");
  if (pr.draft) add(row, btn("mark ready…", () => toggleReady(pr), "small primary", "take it out of draft so reviewers see it (gh pr ready)"));
  if (pr.managed) {
    add(row, tickButton("auto-fix", pr.autofix, () => toggleTick(pr, true), "when CI fails or Copilot comments, mark this PR so you can fix it in one click; a behind branch is updated for you"));
    add(row, tickButton("auto-review", pr.autoreview, () => toggleTick(pr, false), "ask Copilot to review this PR, once"));
    if (pr.automerge.available) add(row, tickButton("automerge", pr.automerge.on, () => toggleAutomerge(pr), "the merge bot merges this PR by itself once it is approved and green"));
  }
  add(row, el("span", "spacer"));
  if (pr.agent && pr.agent.link) add(row, btn("🤖", () => goToAgent(pr), "icon", "go to the Claude Code session that made this PR"));
  add(row, btn("📋", (ev) => copyLink(pr, ev.target), "icon", "copy the link of this PR"));
  add(row, btn("details", () => prSheet(pr.url), "small", "every check, the state and the link"));
  return row;
}

function prCard(pr) {
  const selected = ui.selPr === pr.url;
  const quiet = isQuiet(pr) && !selected;
  const card = el("article", "pr" + (pr.waiting === "reviewer" ? " go" : "") + (pr.red ? " red" : "") + (selected ? " selected" : "") + (quiet ? " quiet" : ""));
  card.dataset.url = pr.url;
  card.addEventListener("click", () => {
    ui.selPr = ui.selPr === pr.url && !isQuiet(pr) ? ui.selPr : pr.url;
    lastSig.prs = null;
    render();
  });
  card.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    prMenu(pr, ev.clientX, ev.clientY);
    lastSig.prs = null;
  });
  const badge = btn(pr.draft ? "DRAFT" : "READY", () => toggleReady(pr), "badge " + (pr.draft ? "dim" : "green"), pr.draft ? "mark ready for review…" : "turn back into a draft…");
  const line1 = el("div", "line1");
  add(line1, badge, link(prName(pr), pr.url, "cyan bold"));
  if (pr.tag) add(line1, el("span", "magenta bold", `${pr.tag.story} · ${pr.tag.task}`));
  else if (pr.pasted) add(line1, el("span", "magenta bold", "followed"));
  if (quiet) {
    add(line1, el("span", "ptitle-inline", pr.title));
    const right = el("span", "right");
    if (pr.counts.passed) right.append(el("span", "green", `✓ ${pr.counts.passed}  `));
    right.append(stamp(el("span"), pr.updatedAt, "ago"));
    line1.append(right);
    card.append(line1);
    return card;
  }
  for (const l of pr.labels) add(line1, el("span", "dim", `[${l}]`));
  const right = el("span", "right");
  if (pr.review !== "none") right.append(el("span", pr.review === "approved" ? "green" : pr.review === "changes" ? "red" : "yellow", { approved: "✔ approved", changes: "✗ changes requested", required: "● review required" }[pr.review] + "  "));
  if (pr.comments) right.append(document.createTextNode(`✎ ${pr.comments}  `));
  right.append(stamp(el("span"), pr.updatedAt, "ago"));
  line1.append(right);
  card.append(line1);
  card.append(link(pr.title, pr.url + "/files", "ptitle"));

  if (pr.branch) {
    const branch = el("div", "branch");
    const button = btn("📋", (ev) => copyBranch(pr, ev.target), "icon", "copy the branch name");
    add(branch, el("span", "", "⎇"), el("code", "", pr.branch), button);
    card.append(branch);
  }

  const rail = el("div", "rail");
  if (pr.waiting === "reviewer") add(rail, el("span", "golight", "GO · ◉ AWAITING REVIEW"));
  else if (pr.waiting === "merge") add(rail, el("span", "green bold", "✔ APPROVED · READY TO MERGE"));
  else if (pr.waiting === "green") add(rail, el("span", "green bold", "✔ ALL GREEN"));
  for (const [i, s] of pr.rail.entries()) {
    if (i === 0 && pr.draft) add(rail, btn(railStep(pr, s), () => toggleReady(pr), "step " + s.state + " toggle", "mark ready for review…"));
    else add(rail, el("span", "step " + s.state, railStep(pr, s)));
  }
  card.append(rail);

  if (pr.fix && (pr.autofix || pr.red)) {
    const fix = el("div", "fixbar");
    add(fix, el("span", "orange bold", `⚠ ${pr.fix.why}`));
    if (pr.fix.onlyBehind) add(fix, el("span", "dim", pr.autofix ? " · capcom updates it for you" : " · tick auto-fix and capcom updates it for you"));
    else add(fix, btn("fix it in a new session ↗", () => fixInSession(pr), "small primary", "opens a new Claude Code desktop session with /pr-address typed in: press Enter to send it"));
    card.append(fix);
  }

  const open = ui.openChecks.has(pr.url);
  const openChecks = pr.checks.filter((c) => ["running", "queued", "failed"].includes(c.state));
  const shown = open ? pr.checks : openChecks.slice(0, 8);
  if (shown.length) {
    const list = el("ul", "checks");
    for (const c of shown) list.append(checkRow(c));
    card.append(list);
  }
  if (pr.checks.length > shown.length || open) {
    const more = el("button", "toggle", open ? "show fewer checks" : `show all ${pr.checks.length} checks`);
    more.onclick = (ev) => {
      ev.stopPropagation();
      if (ui.openChecks.has(pr.url)) ui.openChecks.delete(pr.url);
      else ui.openChecks.add(pr.url);
      lastSig.prs = null;
      render();
    };
    card.append(more);
  }
  if (pr.blocker && !pr.draft) card.append(el("div", "mergeblock", `not merging yet: ${pr.blocker}`));
  card.append(prButtons(pr));
  return card;
}

function visiblePrs() {
  const story = routeStory();
  let list = data.prs;
  if (story && !ui.allPrs) list = list.filter((p) => p.tag && p.tag.story === story.key);
  return list;
}

function renderPrs() {
  const panel = $("prs");
  const story = routeStory();
  const list = visiblePrs();
  const waiting = list.filter((p) => p.waiting === "reviewer").length;
  const parts = [document.createTextNode("PULL REQUESTS · "), el("span", "count", String(list.length))];
  if (waiting) parts.push(el("span", "go", ` · ${waiting} awaiting review`));
  const tools = el("div", "tools");
  if (story) {
    const toggle = el("button", "toggle", ui.allPrs ? "[ this story's PRs ]" : "[ all PRs ]");
    toggle.onclick = () => {
      ui.allPrs = !ui.allPrs;
      lastSig.prs = null;
      render();
    };
    tools.append(toggle);
  }
  const refresh = el("button", "toggle", "↻");
  refresh.title = "look at GitHub now";
  refresh.onclick = () => act("refresh");
  tools.append(refresh);
  const body = shell(panel, parts, tools);
  if (list.length === 0) {
    if (data.loading && !data.prUpdated && !data.prError) body.append(pollNode("GitHub"));
    else add(body, el("div", "empty-note", data.prError ? "Could not read GitHub." : "No open pull requests."));
  }
  for (const pr of list) body.append(prCard(pr));
}

function scrollToSelected() {
  const card = ui.selPr && document.querySelector(`.pr[data-url="${CSS.escape(ui.selPr)}"]`);
  if (card) card.scrollIntoView({ block: "nearest" });
}

// ---- runs ---------------------------------------------------------------------------------

const RUN_WORD = { running: "RUNNING", queued: "QUEUED", waiting: "WAITING", success: "SUCCESS", failed: "FAILED", cancelled: "CANCELLED", skipped: "SKIPPED" };
const RUN_TONE = { running: "orange", queued: "yellow", waiting: "magenta", success: "green", failed: "red", cancelled: "dim", skipped: "dim" };

function runRow(run) {
  const row = el("article", "run");
  const head = el("div", "line1");
  add(head, el("span", `badge ${RUN_TONE[run.state]}`, RUN_WORD[run.state]), link(run.repo, run.url, "cyan bold"), el("span", "bold", run.name));
  if (run.watched) add(head, el("span", "cyan", "◎ watching"));
  const right = el("span", "right");
  right.append(stamp(el("span"), run.updatedAt, "ago"));
  head.append(right);
  row.append(head);
  const title = el("div", "run-title", run.title || "");
  if (run.branch) title.append(el("span", "dim", `  ⎇ ${run.branch}`));
  row.append(title);
  const open = run.jobs.filter((j) => ["running", "queued", "waiting", "failed"].includes(j.state));
  if (open.length) {
    const list = el("ul", "checks");
    for (const j of open.slice(0, 8)) list.append(checkRow({ label: j.name, state: j.state === "waiting" ? "queued" : j.state, url: j.url, startedAt: j.startedAt, completedAt: j.completedAt }));
    row.append(list);
  }
  const buttons = el("div", "pr-buttons");
  add(buttons, el("span", "spacer"));
  if (run.watched) add(buttons, btn("stop watching", () => act("run.unwatch", { repo: run.repo, id: run.id }), "small"));
  add(buttons, btn("details", () => runSheet(run.repo, run.id), "small"));
  row.append(buttons);
  return row;
}

function renderRuns() {
  const panel = $("runs");
  const r = data.runs;
  document.querySelector("main").classList.toggle("no-runs", !r.on);
  if (!r.on) {
    panel.hidden = true;
    panel.replaceChildren();
    return;
  }
  panel.hidden = false;
  const story = routeStory();
  let list = r.items;
  if (story && !ui.allPrs) {
    const repos = new Set(story.tasks.flatMap((t) => t.prs.map((p) => p.repo.toLowerCase())));
    list = list.filter((x) => repos.has(x.repo.toLowerCase()));
  }
  const active = list.filter((x) => x.active).length;
  const parts = [document.createTextNode("RUNS & WATCH · "), el("span", "count", String(list.length))];
  if (active) parts.push(el("span", "cyan", ` · ${active} running`));
  const body = shell(panel, parts);
  if (r.error) add(body, el("div", "red", `GitHub: ${r.error}`));
  if (list.length === 0) {
    if (r.loading && !r.updated) body.append(pollNode("workflow runs"));
    else add(body, el("div", "empty-note", "No manual runs of yours, and none watched. Paste a link to a run to watch it."));
  }
  for (const run of list) body.append(runRow(run));
}

// ---- sheets -------------------------------------------------------------------------------

function checkGroups(checks) {
  const order = ["running", "queued", "failed", "passed", "cancelled", "skipped"];
  return order.map((s) => [s, checks.filter((c) => c.state === s)]).filter(([, list]) => list.length);
}

function prSheet(url) {
  ui.selPr = url;
  openModal("Pull request", () => {
    const pr = prByUrl(url);
    if (!pr) return { body: el("p", "dim", "That pull request is no longer in the list."), foot: [btn("Close", closeModal)] };
    const body = el("div", "sheet");
    add(body, el("h3", "", `${prName(pr)} — ${pr.title}`));
    add(body, kv("State", pr.draft ? "draft" : "ready for review"));
    add(body, kv("Link", link(pr.url, pr.url)));
    if (pr.branch) add(body, kv("Branch", el("code", "", pr.branch)));
    if (pr.tag) add(body, kv("On task", `${pr.tag.story} · ${pr.tag.task}`));
    add(body, kv("Review", { approved: "approved", changes: "changes requested", required: "review required", none: "none" }[pr.review]));
    add(body, kv("Merge", { clean: "can merge", behind: "behind its base", conflicting: "merge conflicts", unknown: "not known yet" }[pr.merge]));
    if (pr.waiting === "reviewer") add(body, kv("Waiting", "a reviewer: it is green and out of draft"));
    else if (pr.waiting === "merge") add(body, kv("Waiting", "the merge: it is approved and green"));
    if (pr.blocker && !pr.draft) add(body, kv("Not merging", pr.blocker));
    add(body, kv("Copilot", pr.copilot === "none" ? "not asked" : pr.copilot === "requested" ? "reviewing" : `reviewed${pr.copilotReviewedAt ? " " + ago(pr.copilotReviewedAt) : ""}${pr.copilotOpen ? ` · ${pr.copilotOpen} comment${pr.copilotOpen === 1 ? "" : "s"} to address` : " · nothing to address"}`));
    if (pr.automerge.kodiak) add(body, kv("Automerge", pr.automerge.available ? (pr.automerge.on ? `on (label “${pr.automerge.labels.filter((l) => pr.labels.includes(l)).join("”, “")}”)` : `off (turn it on to add “${pr.automerge.labels[0]}”)`) : "this repo's merge bot has no automerge label"));
    if (pr.labels.length) add(body, kv("Labels", pr.labels.map((l) => `[${l}]`).join(" ")));
    if (pr.managed) add(body, kv("Auto-fix", pr.autofix ? "on" : "off"));
    if (pr.managed) add(body, kv("Auto-review", pr.autoreview ? "on" : "off"));
    if (pr.fix) add(body, kv("Needs fixing", `${pr.fix.why}`));
    const c = pr.counts;
    add(body, kv("Checks", `${c.passed} passed · ${c.failed} failed · ${c.running} running · ${c.queued} queued · ${c.skipped} skipped · ${c.cancelled} cancelled`));
    if (pr.leftSeconds > 0) add(body, kv("Time left", `${approx(pr.leftSeconds)} for the running checks`));
    for (const [state, list] of checkGroups(pr.checks)) {
      add(body, el("h4", "", `${state} · ${list.length}`));
      const ul = el("ul", "checks");
      for (const k of list) ul.append(checkRow(k));
      body.append(ul);
    }
    const foot = [btn("Open the diff", () => openDiff(pr)), btn("Copy link", (ev) => copyLink(pr, ev.target))];
    if (pr.branch) foot.push(btn("Copy branch", (ev) => copyBranch(pr, ev.target)));
    foot.push(btn(pr.draft ? "Mark ready…" : "Back to draft…", () => toggleReady(pr)));
    if (pr.fix) foot.push(btn("Fix in a new session ↗", () => fixInSession(pr), "primary"));
    if (pr.agent && pr.agent.link) foot.push(btn("🤖 go to its session", () => goToAgent(pr)));
    foot.push(btn("Close", closeModal));
    return { body, foot };
  }, { live: true, wide: true, kind: "pr" });
}

function runSheet(repo, id) {
  openModal("Workflow run", () => {
    const run = data.runs.items.find((r) => r.repo === repo && r.id === id);
    if (!run) return { body: el("p", "dim", "That run is no longer listed."), foot: [btn("Close", closeModal)] };
    const body = el("div", "sheet");
    add(body, el("h3", "", `${run.repo} — ${run.name}`));
    add(body, kv("State", RUN_WORD[run.state].toLowerCase()));
    add(body, kv("Title", run.title || "—"));
    if (run.branch) add(body, kv("Branch", el("code", "", run.branch)));
    add(body, kv("Started", run.startedAt ? ago(run.startedAt) : "not yet"));
    add(body, kv("Link", link(run.url, run.url)));
    if (run.watched) add(body, kv("Watching", "yes: you are told when it finishes, and it leaves the list half an hour later"));
    for (const [state, list] of checkGroups(run.jobs.map((j) => ({ label: j.name, state: j.state === "waiting" ? "queued" : j.state, url: j.url, startedAt: j.startedAt, completedAt: j.completedAt })))) {
      add(body, el("h4", "", `${state} · ${list.length}`));
      const ul = el("ul", "checks");
      for (const k of list) ul.append(checkRow(k));
      body.append(ul);
    }
    const foot = [btn("Open on GitHub", () => openExternal(run.url))];
    if (run.watched) foot.push(btn("Stop watching", () => { closeModal(); act("run.unwatch", { repo: run.repo, id: run.id }); }));
    foot.push(btn("Close", closeModal));
    return { body, foot };
  }, { live: true, wide: true, kind: "run" });
}

function taskSheet(storyKey, taskId) {
  let detail = null;
  const story = data.stories.find((s) => s.key === storyKey);
  openModal("Task", () => {
    const task = story && story.tasks.find((t) => t.id === taskId);
    if (!task) return { body: el("p", "dim", "That task is gone."), foot: [btn("Close", closeModal)] };
    const body = el("div", "sheet");
    add(body, el("h3", "", `${task.id} — ${task.title}`));
    const [text] = taskStatus(task);
    add(body, kv("Status", text));
    add(body, kv("Type", task.type));
    add(body, kv("Depends on", task.dependsOn.length ? task.dependsOn.join(", ") : "none"));
    add(body, kv("Repos", task.repos.length ? task.repos.join(", ") : "none"));
    add(body, kv("Jira", task.jira || "none"));
    if (detail && detail.agent) add(body, kv("Agent", `${detail.agent.pane} · ${detail.agent.skill} · since ${detail.agent.startedAt}`));
    if (detail) add(body, kv("File", detail.file));
    const prs = el("div", "chips");
    for (const p of task.prs) add(prs, link(`${p.repo}#${p.url.split("/").slice(-1)[0]} ${p.state.toUpperCase()}`, p.url, `chip ${p.state}`));
    add(body, kv("PRs", task.prs.length ? prs : "none"));
    add(body, el("h4", "", detail ? detail.label : "…"));
    add(body, el("pre", "doc", detail ? detail.body : "loading…"));
    const foot = [];
    if (task.status === "todo") foot.push(btn("Plan ↗", () => startSession(task.plan, `/story-plan-task ${storyKey} ${task.id}`)));
    if ((task.status === "todo" || task.status === "planned") && !task.waitingOn.length) foot.push(btn("Implement ↗", () => startSession(task.implement, `/story-implement-task ${storyKey} ${task.id}`)));
    if (task.status === "implementing") foot.push(btn("Finish…", () => finishTask(story, task)));
    foot.push(btn("Close", closeModal));
    return { body, foot };
  }, { live: true, wide: true, kind: "task" });
  fetch(`/api/task?story=${encodeURIComponent(storyKey)}&id=${encodeURIComponent(taskId)}`)
    .then((r) => (r.ok ? r.json() : null))
    .then((d) => {
      detail = d;
      fillModal();
    })
    .catch(() => {});
}

// ---- agents -------------------------------------------------------------------------------

function noteSummary(note, needs) {
  const first = (...parts) => parts.find((p) => p) || "";
  if (needs) return [`you: ${first(note.next, note.waiting, note.progress)}`, "note needs"];
  if (note.waiting) return [`waiting: ${note.waiting}`, "note waiting"];
  return [first(note.progress, note.task), "note"];
}

function noteBlock(note, needs) {
  const key = note.session;
  if (ui.openNotes.has(key)) {
    const full = el("div", "notefull");
    full.title = "click to show one line";
    for (const [label, text, cls] of [["asked", note.task, "asked"], ["now", note.progress, ""], ["wait", note.waiting, "wait"], ["you", note.next, "you"]]) {
      if (text) add(full, add(el("div", cls), el("b", "", label), el("span", "", text)));
    }
    full.onclick = () => {
      ui.openNotes.delete(key);
      lastSig.agents = null;
      render();
    };
    return full;
  }
  const [text, cls] = noteSummary(note, needs);
  const line = el("button", cls, text);
  line.title = "click to show all of it";
  line.onclick = () => {
    ui.openNotes.add(key);
    lastSig.agents = null;
    render();
  };
  return line;
}

function renderAgentRow(body, e, a) {
  const row = el("div", "agent " + e.state);
  row.style.paddingLeft = `${4 + e.depth * 14}px`;
  add(row, el("span", "icon", (e.depth ? "└ " : "") + e.icon), el("span", "label", e.label), e.source ? el("span", "src", e.source) : null, el("span", "word", e.status));
  if (e.deepLink && ui.opening.has(e.deepLink)) {
    row.classList.add("opening");
    row.querySelector(".word").textContent = "opening…";
  }
  if (e.deepLink) {
    row.classList.add("clickable");
    row.title = "open this session in Claude Code desktop";
    row.onclick = () => {
      if (ui.opening.has(e.deepLink)) return;
      ui.opening.add(e.deepLink);
      row.classList.add("opening");
      const word = row.querySelector(".word");
      if (word) word.textContent = "opening…";
      setTimeout(() => {
        ui.opening.delete(e.deepLink);
        lastSig.agents = null;
        render();
      }, 3000);
      openExternal(e.deepLink, "asked Claude Code desktop to open it: this can take a couple of seconds");
    };
  } else if (e.source === "cli") {
    row.title = "a terminal session: it can only be reached in its terminal";
  }
  body.append(row);
  if (e.note !== null && a.notes[e.note]) body.append(noteBlock(a.notes[e.note], e.state === "needs"));
  else if (e.topic && e.topic !== e.label) body.append(el("div", "dim", e.topic));
  for (const sub of e.subagents || []) {
    const r = el("div", "agent sub clickable " + (sub.running ? "working" : "idle"));
    r.style.paddingLeft = `${4 + (e.depth + sub.depth) * 14}px`;
    add(r, el("span", "icon", "└ " + (sub.running ? "◐" : "✓")), el("span", "label", sub.description || sub.kind), el("span", "src", sub.kind));
    add(r, sub.running ? el("span", "word", "working") : stamp(el("span", "when"), sub.updatedAt, "short"));
    r.title = "show what this subagent did";
    r.onclick = () => subagentSheet(e, sub);
    body.append(r);
  }
}

// A subagent's conversation, read from its transcript and kept up to date while it runs.
function subagentSheet(parent, sub) {
  let entries = null;
  let failed = false;
  let stuck = true;
  let pos = 0;
  const current = () => {
    for (const g of data.agents.groups) for (const e of g.entries) if (e.session === parent.session) return (e.subagents || []).find((x) => x.id === sub.id);
    return null;
  };
  const label = { prompt: "asked", text: "said", tool: "tool", result: "got", report: "report" };
  const sheet = openModal("Subagent", () => {
    const body = el("div", "sheet");
    add(body, el("h3", "", sub.description || sub.kind));
    add(body, kv("Type", sub.kind));
    add(body, kv("Of", parent.label));
    add(body, kv("State", sub.running ? "working" : "finished"));
    const log = el("div", "transcript");
    if (failed) add(log, el("p", "dim", "The transcript could not be read (the session may have ended)."));
    else if (!entries) add(log, el("p", "dim", "loading…"));
    else for (const t of entries) add(log, add(el("div", "t " + t.kind), el("b", "", label[t.kind] || t.kind), el("span", "", t.text)));
    log.onscroll = () => {
      pos = log.scrollTop;
      stuck = log.scrollTop + log.clientHeight >= log.scrollHeight - 8;
    };
    requestAnimationFrame(() => {
      log.scrollTop = stuck ? log.scrollHeight : pos;
    });
    add(body, log);
    const foot = [];
    if (parent.deepLink) foot.push(btn("Open its session ↗", () => openExternal(parent.deepLink, "asked Claude Code desktop to open it: this can take a couple of seconds")));
    foot.push(btn("Close", closeModal));
    return { body, foot };
  }, { wide: true, kind: "subagent" });
  const load = () => {
    if (modal !== sheet) return;
    fetch(`/api/subagent?session=${encodeURIComponent(parent.session)}&agent=${encodeURIComponent(sub.id)}`)
      .then((r) => (r.ok ? r.json() : Promise.reject()))
      .then((d) => {
        const now = current();
        if (now) sub = now;
        const before = entries ? entries.length : -1;
        entries = d.entries;
        failed = false;
        if (modal === sheet && entries.length !== before) fillModal();
      })
      .catch(() => {
        failed = true;
        if (modal === sheet) fillModal();
      })
      .finally(() => {
        if (sub.running && modal === sheet) setTimeout(load, POLL_MS);
      });
  };
  load();
}

function renderAgents() {
  const panel = $("agents");
  const a = data.agents;
  const hidden = a.count === 0 && a.earlier.length === 0;
  document.querySelector("main").classList.toggle("no-agents", hidden);
  panel.hidden = hidden;
  if (hidden) {
    panel.replaceChildren();
    return;
  }
  const parts = [document.createTextNode("AGENTS · "), el("span", "count", String(a.count))];
  if (a.working) parts.push(el("span", "cyan", ` · ${a.working} working`));
  if (a.needsYou) parts.push(el("span", "orange bold", ` · ${a.needsYou} need you`));
  const unread = a.groups.flatMap((g) => g.entries).filter((e) => e.state === "unread").length;
  if (unread) parts.push(el("span", "bold", ` · ${unread} unread`));
  const body = shell(panel, parts);
  // Desktop sessions first, then terminal ones; each keeps its folder groups. The two headings only
  // appear when both kinds are running.
  const isDesktop = (e) => e.source === "desktop";
  const kinds = [
    ["DESKTOP", (e) => isDesktop(e)],
    ["TERMINAL", (e) => !isDesktop(e)],
  ].map(([label, keep]) => {
    const groups = a.groups.map((g) => ({ ...g, entries: g.entries.filter(keep) })).filter((g) => g.entries.length > 0);
    return { label, groups, count: groups.reduce((n, g) => n + g.entries.length, 0) };
  }).filter((k) => k.count > 0);
  for (const kind of kinds) {
    if (kinds.length > 1) {
      const sep = el("div", "sep");
      add(sep, document.createTextNode(`${kind.label} · `), el("span", "count", String(kind.count)));
      body.append(sep);
    }
    for (const g of kind.groups) {
      body.append(el("div", "group " + (g.story ? "story" : "other"), g.label));
      for (const e of g.entries) renderAgentRow(body, e, a);
    }
  }
  if (a.earlier.length) {
    body.append(el("div", "group other", `EARLIER · ${a.earlier.length}`));
    for (const i of a.earlier) {
      const note = a.notes[i];
      const row = el("div", "agent " + (note.needsYou ? "needs" : "idle"));
      add(row, el("span", "icon", note.needsYou ? "!" : "○"), el("span", "label", note.task || `session ${note.session.slice(0, 8)}`), stamp(el("span", "when"), note.updatedAt, "short"));
      body.append(row);
      body.append(noteBlock(note, note.needsYou));
    }
  }
}
