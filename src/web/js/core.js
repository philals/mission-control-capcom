"use strict";
// Every piece of data from the server goes into the page as text, never as HTML.

const POLL_MS = 3000;
const $ = (id) => document.getElementById(id);

let data = null;
let failures = 0;
const lastSig = {};
const ui = {
  selPr: null, // the url of the selected PR
  selStory: null,
  openChecks: new Set(), // PRs that show every check
  openNotes: new Set(),
  opening: new Set(),
  allPrs: false,
  showDone: false,
  view: "board", // on a narrow screen, which console shows
};
try {
  ui.showDone = localStorage.getItem("capcom.showDone") === "1";
  ui.view = localStorage.getItem("capcom.view") || "board";
} catch (_) {}

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

function btn(text, onclick, cls, title) {
  const b = el("button", cls ? `btn ${cls}` : "btn", text);
  b.type = "button";
  if (title) b.title = title;
  b.addEventListener("click", (ev) => {
    ev.stopPropagation();
    onclick(ev);
  });
  return b;
}

function safeUrl(u) {
  try {
    const url = new URL(u);
    return url.protocol === "https:" ? url.href : null;
  } catch (_) {
    return null;
  }
}

let toastTimer = null;
function toast(text) {
  const t = $("toast");
  t.textContent = text;
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), 3500);
}

// A link opens in your own browser: the server starts it. If that cannot be done, the browser
// showing this page opens the link itself.
async function openExternal(href, what) {
  try {
    const response = await fetch("/open", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Capcom": "open" },
      body: JSON.stringify({ url: href }),
    });
    if (response.status === 204) {
      toast(what || "opened in your browser");
      return;
    }
  } catch (_) {}
  if (safeUrl(href)) window.open(href, "_blank", "noopener,noreferrer");
  else toast("could not open that link");
}

function link(text, url, cls) {
  const href = safeUrl(url);
  if (!href) return el("span", cls, text);
  const a = el("a", cls, text);
  a.href = href;
  a.target = "_blank";
  a.rel = "noopener noreferrer";
  a.addEventListener("click", (ev) => {
    ev.stopPropagation();
    if (ev.button !== 0 || ev.ctrlKey || ev.metaKey || ev.shiftKey || ev.altKey) return;
    ev.preventDefault();
    openExternal(href);
  });
  return a;
}

// A change: the server checks it against what it knows, and the confirmed kinds need `confirm`.
async function act(action, args = {}) {
  try {
    const response = await fetch("/api/do", {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Capcom": "do" },
      body: JSON.stringify({ action, ...args }),
    });
    if (!response.ok) {
      toast(`refused (${response.status})`);
      return { ok: false };
    }
    const out = await response.json();
    if (out.message) toast(out.message);
    load();
    return out;
  } catch (_) {
    toast("could not reach capcom web");
    return { ok: false };
  }
}

// ---- time ---------------------------------------------------------------------------------

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

function shortAgo(iso) {
  const t = Date.parse(iso);
  if (!t) return "";
  const m = Math.max(0, Math.floor((Date.now() - t) / 60000));
  if (m < 60) return `${m}m`;
  if (m < 1440) return `${Math.floor(m / 60)}h`;
  return `${Math.floor(m / 1440)}d`;
}

function dur(seconds) {
  const s = Math.max(0, Math.round(seconds));
  const m = Math.floor(s / 60);
  return m > 0 ? `${m}m ${String(s % 60).padStart(2, "0")}s` : `${s}s`;
}

// `~4m`, `~45s`, `~1h 20m`: a rounded figure for an estimate.
function approx(seconds) {
  if (seconds <= 59) return `~${Math.max(5, Math.floor((seconds + 2) / 5) * 5)}s`;
  if (seconds < 3600) return `~${Math.floor((seconds + 30) / 60)}m`;
  return `~${Math.floor(seconds / 3600)}h ${String(Math.floor((seconds % 3600) / 60)).padStart(2, "0")}m`;
}

function stamp(node, iso, kind, usual) {
  node.dataset.ts = iso;
  node.dataset.kind = kind;
  if (usual) node.dataset.usual = String(usual);
  return node;
}

function runningText(iso, usual) {
  const took = (Date.now() - Date.parse(iso)) / 1000;
  if (!usual) return `running ${dur(took)}`;
  return took < usual ? `running ${dur(took)} of ${approx(usual)}` : `running ${dur(took)} · usual ${approx(usual)}`;
}

function tick() {
  for (const n of document.querySelectorAll("[data-ts]")) {
    const iso = n.dataset.ts;
    if (n.dataset.kind === "ago") n.textContent = ago(iso);
    else if (n.dataset.kind === "short") n.textContent = shortAgo(iso);
    else if (n.dataset.kind === "run") n.textContent = runningText(iso, Number(n.dataset.usual) || 0);
  }
  if (typeof tickClock === "function") tickClock();
}

// ---- clipboard ----------------------------------------------------------------------------

function copy(text, button) {
  const done = () => {
    if (button) {
      const old = button.textContent;
      button.textContent = "✓";
      setTimeout(() => (button.textContent = old), 1200);
    }
    toast("copied");
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

// ---- dialogs and menus --------------------------------------------------------------------

// One dialog at a time. `build` makes the body and is called again whenever new data arrives
// (when `live`), so a sheet about a running check keeps up.
let modal = null;

function closeModal() {
  if (!modal) return;
  modal.bg.remove();
  modal = null;
}

function openModal(title, build, opts = {}) {
  closeModal();
  closeMenu();
  const bg = el("div", "modal-bg");
  const box = el("div", "modal" + (opts.wide ? " wide" : ""));
  const head = el("div", "modal-head");
  add(head, el("span", "modal-title", title), btn("✕", closeModal, "x", "close (Esc)"));
  const body = el("div", "modal-body");
  const foot = el("div", "modal-foot");
  add(box, head, body, foot);
  bg.append(box);
  bg.addEventListener("mousedown", (ev) => {
    if (ev.target === bg) closeModal();
  });
  $("modal-root").append(bg);
  modal = { bg, body, foot, build, live: !!opts.live, kind: opts.kind || "" };
  fillModal();
  const first = box.querySelector("input, textarea, .btn.primary") || box.querySelector(".btn");
  if (first) first.focus();
  return modal;
}

// The body can return buttons for the foot through `foot` in the result: {body, foot}.
function fillModal() {
  if (!modal) return;
  const made = modal.build();
  const body = made && made.body ? made.body : made;
  const foot = made && made.foot ? made.foot : [];
  const focused = document.activeElement;
  const keep = focused && modal.body.contains(focused) && focused.tagName === "INPUT" ? { value: focused.value, at: focused.selectionStart } : null;
  modal.body.replaceChildren(...(Array.isArray(body) ? body : [body]));
  modal.foot.replaceChildren(...foot);
  if (keep) {
    const again = modal.body.querySelector("input");
    if (again) {
      again.value = keep.value;
      again.focus();
      again.setSelectionRange(keep.at, keep.at);
    }
  }
}

function refreshModal() {
  if (modal && modal.live) fillModal();
}

// "Are you sure": what will happen, and a button that does it.
function ask(title, lines, yes, action, args, then, kind = "primary") {
  openModal(title, () => ({
    body: lines.map((l) => el("p", "", l)),
    foot: [
      btn("Cancel", closeModal),
      btn(yes, async () => {
        closeModal();
        const out = await act(action, { ...args, confirm: true });
        if (then) then(out);
      }, kind),
    ],
  }), { kind: "ask" });
}

let menuEl = null;
function closeMenu() {
  if (menuEl) menuEl.remove();
  menuEl = null;
}

// A small list of actions where you clicked. Each item is [label, fn] (a falsy item is skipped).
function openMenu(x, y, items) {
  closeMenu();
  const m = el("div", "menu");
  for (const item of items.filter(Boolean)) {
    const b = el("button", "menu-item", item[0]);
    b.type = "button";
    b.addEventListener("click", (ev) => {
      ev.stopPropagation();
      closeMenu();
      item[1]();
    });
    m.append(b);
  }
  document.body.append(m);
  const w = m.offsetWidth;
  const h = m.offsetHeight;
  m.style.left = `${Math.max(4, Math.min(x, window.innerWidth - w - 4))}px`;
  m.style.top = `${Math.max(4, Math.min(y, window.innerHeight - h - 4))}px`;
  menuEl = m;
}

document.addEventListener("mousedown", (ev) => {
  if (menuEl && !menuEl.contains(ev.target)) closeMenu();
});
