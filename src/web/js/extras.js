// ---- dialogs ------------------------------------------------------------------------------

function textDialog(title, hint, placeholder, yes, onSubmit) {
  openModal(title, () => {
    const input = el("input", "text");
    input.type = "text";
    input.placeholder = placeholder;
    input.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter") {
        ev.preventDefault();
        submit();
      }
    });
    function submit() {
      const text = input.value.trim();
      if (!text) return;
      closeModal();
      onSubmit(text);
    }
    return {
      body: [el("p", "", hint), input],
      foot: [btn("Cancel", closeModal), btn(yes, submit, "primary")],
    };
  });
}

function followDialog() {
  textDialog(
    "Follow a PR or a run",
    "Paste the link of someone else's pull request to follow it like your own (auto-fix and auto-review start off for it), or of a GitHub Actions run to watch it: you are told when it finishes.",
    "https://github.com/…/pull/12  or  …/actions/runs/123",
    "Follow",
    (text) => act("pr.follow", { text }),
  );
}

function newStoryDialog() {
  textDialog(
    "New story",
    "Paste a Jira key or link. This opens a new Claude Code session with /story-break-down typed in; press Enter there to start the break-down.",
    "PROJ-123",
    "Open the session",
    (key) => act("story.new", { key }),
  );
}

const SETTINGS = [
  ["autofix", "Auto-fix by default", "PRs without a tick of their own are marked when CI or Copilot needs fixing, and a behind branch is updated for you. (A followed PR always starts off.)", (s) => s.autofix],
  ["autoreview", "Auto-review by default", "Copilot is asked to review your PRs as they appear.", (s) => s.autoreview],
  ["auto_sync", "Auto-sync", "What GitHub says about the PRs recorded on tasks is written to the boards, so a merged PR finishes its task.", (s) => s.autoSync],
  ["notify", "Desktop notifications", "Tell me outside the page (notify-send) when a PR needs me, Copilot finishes, or a run ends. They need polling while the page is closed, which is off by default: with this on the service polls every couple of minutes even when nobody looks.", (s) => s.notify],
  ["slow_when_away", "Slow polling when nobody looks", "GitHub is asked about every 30 minutes (and at once when you come back) while no page is open. Turning this off keeps the normal pace always.", (s) => s.slowWhenAway],
];

function settingsDialog() {
  openModal("Settings", () => {
    const body = el("div", "settings");
    for (const [key, label, help, get] of SETTINGS) {
      const row = el("div", "setting");
      const on = get(data.settings);
      add(row, btn(on ? "[x]" : "[ ]", () => act("settings", { key, value: !on }), on ? "tick on" : "tick"), add(el("div", "setting-text"), el("div", "bold", label), el("div", "dim", help)));
      body.append(row);
    }
    add(body, el("p", "dim", `New Claude Code sessions start in ${data.workdir}. These settings are shared with capcom-tui.`));
    return { body, foot: [btn("Close", closeModal, "primary")] };
  }, { live: true, kind: "settings" });
}

function eventsDialog() {
  openModal("What capcom did", () => {
    const body = el("div", "events");
    if (data.events.length === 0) body.append(el("p", "dim", "Nothing yet. Updating a behind branch, asking Copilot, finishing runs and PRs that need you are listed here."));
    for (const e of data.events) add(body, add(el("div", "event"), stamp(el("span", "dim"), e.at, "short"), el("span", "", " " + e.text)));
    return { body, foot: [btn("Close", closeModal, "primary")] };
  }, { live: true, wide: true, kind: "events" });
}

const KEYS = [
  ["j / k, ↓ / ↑", "select the next or previous PR"],
  ["Enter", "details of the selected PR"],
  ["o", "open the selected PR's diff on GitHub"],
  ["c", "copy the selected PR's link"],
  ["b", "copy the selected PR's branch name"],
  ["m", "mark it ready for review, or back to a draft (asks first)"],
  ["t / v / g", "turn auto-fix / auto-review / automerge on or off for it"],
  ["f", "open a new Claude Code session that fixes it (when it needs fixing)"],
  ["a", "go to the Claude Code session that made it"],
  ["u", "stop following a PR you followed by link"],
  ["r", "look at GitHub now"],
  ["n", "new story · F follow a PR or run (or just paste its link) · d show or hide done stories"],
  ["Ctrl-K or /", "the command palette: every action that fits where you are"],
  ["Esc", "close a dialog, or go back to the stories"],
  ["right-click", "the actions of a PR or a story"],
];

function helpDialog() {
  openModal("Keys", () => {
    const body = el("div", "sheet");
    for (const [k, v] of KEYS) add(body, kv(k, v));
    add(body, el("h4", "", "Reading a PR"));
    add(body, el("p", "dim", "The rail is a PR's way to a merge: ready, CI, Copilot, its conversations, approval, automerge, the merge. ● is done, ◔ is under way, ✗ is wrong, ○ is not yet. A green PR waiting for a reviewer is lit up. A PR with nothing to do takes one line; click it to open it."));
    add(body, el("p", "dim", "Nothing here changes anything without asking: ready or draft, automerge, finishing a task, marking a story done and syncing all show what they will do first. Starting a skill opens a new Claude Code desktop session with the command typed in; you press Enter."));
    return { body, foot: [btn("Close", closeModal, "primary")] };
  }, { wide: true });
}

// ---- command palette ----------------------------------------------------------------------

function selectedPr() {
  return ui.selPr && data ? prByUrl(ui.selPr) : null;
}

function paletteItems() {
  const items = [];
  const push = (label, hint, run) => items.push({ label, hint: hint || "", run });
  const pr = selectedPr();
  const story = routeStory();
  if (pr) {
    const n = prName(pr);
    push(`Details of ${n}`, "Enter", () => prSheet(pr.url));
    push(`Open ${n}'s diff on GitHub`, "o", () => openDiff(pr));
    push(`Copy the link of ${n}`, "c", () => copyLink(pr));
    if (pr.branch) push(`Copy the branch name of ${n}`, "b", () => copyBranch(pr));
    push(`${pr.draft ? "Mark ready for review" : "Turn back into a draft"}: ${n}`, "m", () => toggleReady(pr));
    if (pr.managed) {
      push(`Turn auto-fix ${pr.autofix ? "off" : "on"} for ${n}`, "t", () => toggleTick(pr, true));
      push(`Turn auto-review ${pr.autoreview ? "off" : "on"} for ${n}`, "v", () => toggleTick(pr, false));
      if (pr.automerge.available) push(`Turn automerge ${pr.automerge.on ? "off" : "on"} for ${n}`, "g", () => toggleAutomerge(pr));
    }
    if (pr.fix) push(`Fix ${n} in a new Claude Code session`, "f", () => fixInSession(pr));
    if (pr.agent && pr.agent.link) push(`Go to the session that made ${n}`, "a", () => goToAgent(pr));
    if (pr.pasted) push(`Stop following ${n}`, "u", () => unfollow(pr));
  }
  if (story) {
    push(`Back to the stories`, "Esc", () => (location.hash = ""));
    for (const t of story.tasks) {
      push(`Details of ${t.id}: ${t.title}`, "", () => taskSheet(story.key, t.id));
      if (t.status === "todo") push(`Plan ${t.id} in a new session`, "", () => startSession(t.plan, `/story-plan-task ${story.key} ${t.id}`));
      if ((t.status === "todo" || t.status === "planned") && !t.waitingOn.length) push(`Implement ${t.id} in a new session`, "", () => startSession(t.implement, `/story-implement-task ${story.key} ${t.id}`));
      if (t.status === "implementing") push(`Finish ${t.id}`, "", () => finishTask(story, t));
    }
    push(`Sync ${story.key} with GitHub`, "", () => syncStory(story));
    if (story.allFinished && story.status === "in_progress") push(`Review ${story.key} in a new session`, "", () => startSession(story.review, "/story-review"));
    if (story.allFinished && story.status !== "done") push(`Mark ${story.key} done`, "", () => markStoryDone(story));
    if (story.status !== "in_progress") push(`Reopen ${story.key}`, "", () => reopenStory(story));
  }
  push("Look at GitHub now", "r", () => act("refresh"));
  push("Follow a PR or a run by its link", "F", followDialog);
  push("New story (break a Jira story down)", "n", newStoryDialog);
  push(ui.showDone ? "Hide done stories" : "Show done stories", "d", toggleShowDone);
  push("Settings", "", settingsDialog);
  push("What capcom did by itself", "", eventsDialog);
  push("Keys and what things mean", "?", helpDialog);
  for (const s of data.stories) push(`Open story ${s.key}: ${s.title}`, "", () => (location.hash = `#/story/${encodeURIComponent(s.key)}`));
  for (const p of data.prs) if (!pr || p.url !== pr.url) push(`Select ${prName(p)}: ${p.title}`, "", () => { ui.selPr = p.url; lastSig.prs = null; render(); scrollToSelected(); });
  for (const g of data.agents.groups) for (const e of g.entries) if (e.deepLink) push(`Go to agent ${e.label} (${e.status})`, "", () => openExternal(e.deepLink, "asked Claude Code desktop to open it"));
  for (const run of data.runs.items) push(`Details of run ${run.repo} ${run.name} (${run.state})`, "", () => runSheet(run.repo, run.id));
  return items;
}

function paletteDialog() {
  const all = paletteItems();
  let sel = 0;
  let shown = all;
  openModal("Command palette", () => {
    const input = el("input", "text");
    input.type = "text";
    input.placeholder = "type to narrow: every word must appear";
    const list = el("div", "palette");
    function draw() {
      const words = input.value.toLowerCase().split(/\s+/).filter(Boolean);
      shown = all.filter((i) => words.every((w) => (i.label + " " + i.hint).toLowerCase().includes(w)));
      sel = Math.min(sel, Math.max(0, shown.length - 1));
      list.replaceChildren();
      shown.slice(0, 60).forEach((item, n) => {
        const row = el("button", "palette-item" + (n === sel ? " sel" : ""));
        row.type = "button";
        add(row, el("span", "", item.label), item.hint ? el("span", "kbd", item.hint) : null);
        row.addEventListener("click", () => run(item));
        row.addEventListener("mousemove", () => {
          if (sel !== n) {
            sel = n;
            draw();
          }
        });
        list.append(row);
      });
      if (shown.length === 0) list.append(el("div", "dim", "nothing matches"));
      const current = list.querySelector(".sel");
      if (current) current.scrollIntoView({ block: "nearest" });
    }
    function run(item) {
      closeModal();
      item.run();
    }
    input.addEventListener("input", () => {
      sel = 0;
      draw();
    });
    input.addEventListener("keydown", (ev) => {
      if (ev.key === "ArrowDown") {
        sel = Math.min(sel + 1, Math.max(0, shown.length - 1));
        draw();
        ev.preventDefault();
      } else if (ev.key === "ArrowUp") {
        sel = Math.max(sel - 1, 0);
        draw();
        ev.preventDefault();
      } else if (ev.key === "Enter" && shown[sel]) {
        ev.preventDefault();
        run(shown[sel]);
      }
    });
    draw();
    return { body: [input, list], foot: [btn("Close", closeModal)] };
  }, { wide: true, kind: "palette" });
}

// ---- keys ---------------------------------------------------------------------------------

function typing(ev) {
  const t = ev.target;
  return t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);
}

function moveSelection(delta) {
  const list = visiblePrs();
  if (!list.length) return;
  const at = list.findIndex((p) => p.url === ui.selPr);
  const next = list[Math.max(0, Math.min(list.length - 1, at < 0 ? (delta > 0 ? 0 : list.length - 1) : at + delta))];
  ui.selPr = next.url;
  lastSig.prs = null;
  render();
  scrollToSelected();
}

document.addEventListener("keydown", (ev) => {
  if (!data) return;
  if ((ev.ctrlKey || ev.metaKey) && (ev.key === "k" || ev.key === "p")) {
    ev.preventDefault();
    paletteDialog();
    return;
  }
  if (ev.key === "Escape") {
    if (menuEl) closeMenu();
    else if (modal) closeModal();
    else if (location.hash) location.hash = "";
    return;
  }
  if (typing(ev) || ev.ctrlKey || ev.metaKey || ev.altKey) return;
  if (modal) return;
  const pr = selectedPr();
  switch (ev.key) {
    case "j":
    case "ArrowDown":
      ev.preventDefault();
      return moveSelection(1);
    case "k":
    case "ArrowUp":
      ev.preventDefault();
      return moveSelection(-1);
    case "/":
      ev.preventDefault();
      return paletteDialog();
    case "?":
      return helpDialog();
    case "r":
      return void act("refresh");
    case "n":
      ev.preventDefault();
      return newStoryDialog();
    case "F":
      ev.preventDefault();
      return followDialog();
    case "d":
      return toggleShowDone();
  }
  if (!pr) return;
  switch (ev.key) {
    case "Enter":
      return prSheet(pr.url);
    case "o":
      return openDiff(pr);
    case "c":
      return copyLink(pr);
    case "b":
      return pr.branch ? copyBranch(pr) : undefined;
    case "m":
      return toggleReady(pr);
    case "t":
      return toggleTick(pr, true);
    case "v":
      return toggleTick(pr, false);
    case "g":
      return toggleAutomerge(pr);
    case "f":
      return pr.fix ? fixInSession(pr) : toast("nothing to fix on this PR");
    case "a":
      return goToAgent(pr);
    case "u":
      return unfollow(pr);
  }
});

// Paste a link to a PR or a run anywhere (not in a text box) to follow it.
document.addEventListener("paste", (ev) => {
  if (typing(ev)) return;
  const text = (ev.clipboardData && ev.clipboardData.getData("text")) || "";
  if (/github\.com\/[^/\s]+\/[^/\s]+\/(pull\/\d+|actions\/runs\/\d+)/.test(text)) {
    ev.preventDefault();
    act("pr.follow", { text: text.trim() });
  }
});

// ---- the loop -----------------------------------------------------------------------------

function sig(value) {
  return JSON.stringify(value);
}

function render() {
  if (!data) return;
  const header = sig([data.red, data.redWithAutofix, data.awaitingReview, data.prError, data.prUpdated, data.loading, data.agents.needsYou, data.events.length]);
  if (header !== lastSig.header) {
    lastSig.header = header;
    renderHeader();
  }
  const agents = sig([data.agents, [...ui.openNotes]]);
  if (agents !== lastSig.agents) {
    lastSig.agents = agents;
    renderAgents();
  }
  const board = sig([data.stories, data.broken, location.hash, ui.showDone, data.prs.map((p) => [p.url, p.red, p.waiting, p.counts, p.review, p.copilotOpen])]);
  if (board !== lastSig.board) {
    lastSig.board = board;
    renderBoard();
  }
  const prs = sig([data.prs, data.prError, location.hash, ui.allPrs, [...ui.openChecks], ui.selPr]);
  if (prs !== lastSig.prs) {
    lastSig.prs = prs;
    renderPrs();
  }
  const runs = sig([data.runs, location.hash, ui.allPrs]);
  if (runs !== lastSig.runs) {
    lastSig.runs = runs;
    renderRuns();
  }
  refreshModal();
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
    if (!ui.selPr && data.prs.length) ui.selPr = data.prs[0].url;
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
  lastSig.board = lastSig.prs = lastSig.runs = null;
  render();
});
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) load();
});
setInterval(load, POLL_MS);
setInterval(tick, 1000);
load();
