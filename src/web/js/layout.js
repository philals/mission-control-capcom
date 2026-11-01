// ---- resizable consoles -------------------------------------------------------------------
// On a wide screen the consoles sit in a grid, and a thin handle in each gap moves the edge between
// two neighbours: drag it, or focus it and use the arrow keys. A double-click puts the layout back.
// Each layout (how many columns the window has room for, and which consoles are showing) remembers
// its own sizes in this browser.

const LAYOUT_KEY = "capcom.layout";
const MIN_COL = 200;
const MIN_ROW = 110;
let layoutState = {};
try {
  layoutState = JSON.parse(localStorage.getItem(LAYOUT_KEY) || "{}") || {};
} catch (_) {}

function saveLayout() {
  try {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify(layoutState));
  } catch (_) {}
}

// Whether the consoles fill the window (the same limits as the style sheet), or null when the page scrolls.
function layoutVariant() {
  if (innerHeight < 580 || innerWidth < 1000) return null;
  return innerWidth >= 1900 ? "tui-xl" : "tui";
}

function showing(id) {
  const p = $(id);
  return p && !p.hidden ? p : null;
}

// What can be resized, as in capcom-tui: the agents | stories edge, the top | bottom edge, and the
// pull requests | runs edge in the bottom strip. Each is a list of groups of consoles, one track each.
function layoutGeometry() {
  const v = layoutVariant();
  if (!v) return null;
  const agents = showing("agents");
  const runs = showing("runs");
  const board = $("board");
  const side = $("side");
  return {
    v,
    key: `${v}:${agents ? 1 : 0}${runs ? 1 : 0}`,
    cols: agents ? [[agents], [board]] : null,
    rows: [agents ? [agents, board] : [board], [side]],
    sideCols: runs ? [[$("prs")], [runs]] : null,
  };
}

function union(group) {
  const rects = group.map((p) => p.getBoundingClientRect());
  return {
    left: Math.min(...rects.map((r) => r.left)),
    right: Math.max(...rects.map((r) => r.right)),
    top: Math.min(...rects.map((r) => r.top)),
    bottom: Math.max(...rects.map((r) => r.bottom)),
  };
}

function track(weights) {
  return weights.map((w) => `minmax(0, ${w}fr)`).join(" ");
}

// Put the remembered sizes on the grid (or take the inline sizes off, to let the style sheet decide).
function applyLayout() {
  const main = document.querySelector("main");
  const g = layoutGeometry();
  const saved = (g && layoutState[g.key]) || {};
  const fits = (w, groups) => g && groups && w && w.length === groups.length;
  main.style.gridTemplateColumns = fits(saved.cols, g && g.cols) ? track(saved.cols) : "";
  main.style.gridTemplateRows = fits(saved.rows, g && g.rows) ? track(saved.rows) : "";
  $("side").style.gridTemplateColumns = fits(saved.side, g && g.sideCols) ? track(saved.side) : "";
}

function resetLayout() {
  const g = layoutGeometry();
  if (g) delete layoutState[g.key];
  saveLayout();
  applyLayout();
  layoutHandles();
  toast("layout reset");
}

const vertical = (handle) => handle.axis !== "row";
const groupsOf = (handle, g) => (handle.axis === "col" ? g.cols : handle.axis === "row" ? g.rows : g.sideCols);

// The sizes now, in pixels, of what a handle moves.
function sizesOf(handle, g) {
  return groupsOf(handle, g).map((group) => {
    const u = union(group);
    return vertical(handle) ? u.right - u.left : u.bottom - u.top;
  });
}

function setSizes(handle, g, sizes) {
  const entry = (layoutState[g.key] = layoutState[g.key] || {});
  entry[handle.axis === "col" ? "cols" : handle.axis === "row" ? "rows" : "side"] = sizes.map((x) => +(x / 100).toFixed(3));
  applyLayout();
}

// Move the edge `handle.i` by `delta` pixels from `start` sizes.
function nudge(handle, g, start, delta) {
  const min = vertical(handle) ? MIN_COL : MIN_ROW;
  const total = start[handle.i] + start[handle.i + 1];
  const a = Math.max(min, Math.min(total - min, start[handle.i] + delta));
  const sizes = start.slice();
  sizes[handle.i] = a;
  sizes[handle.i + 1] = total - a;
  setSizes(handle, g, sizes);
}

function describeHandles(g) {
  const out = [];
  if (g.cols) out.push({ axis: "col", i: 0, label: "Resize the agents and stories consoles" });
  out.push({ axis: "row", i: 0, label: "Resize the top consoles and the bottom strip" });
  if (g.sideCols) out.push({ axis: "side", i: 0, label: "Resize the pull requests and runs consoles" });
  return out;
}

// Where a handle sits, relative to <main>.
function placeHandle(el, handle, g, mainRect) {
  const groups = groupsOf(handle, g);
  const a = union(groups[handle.i]);
  const b = union(groups[handle.i + 1]);
  if (vertical(handle)) {
    const top = Math.min(a.top, b.top);
    el.style.left = `${(a.right + b.left) / 2 - mainRect.left}px`;
    el.style.top = `${top - mainRect.top}px`;
    el.style.height = `${Math.max(a.bottom, b.bottom) - top}px`;
    el.style.width = "";
    el.style.marginLeft = "";
  } else {
    const left = Math.min(a.left, b.left);
    el.style.left = `${left - mainRect.left}px`;
    el.style.top = `${(a.bottom + b.top) / 2 - mainRect.top}px`;
    el.style.width = `${Math.max(a.right, b.right) - left}px`;
    el.style.height = "";
    el.style.marginLeft = "0";
  }
}

function layoutHandles() {
  const main = document.querySelector("main");
  const g = layoutGeometry();
  const have = [...main.querySelectorAll(":scope > .split")];
  if (!g) {
    have.forEach((h) => h.remove());
    return;
  }
  const wanted = describeHandles(g);
  const same = have.length === wanted.length && have.every((h, n) => h.dataset.axis === wanted[n].axis && Number(h.dataset.i) === wanted[n].i);
  if (!same) {
    have.forEach((h) => h.remove());
    wanted.forEach((handle) => main.append(makeHandle(handle)));
  }
  const rect = main.getBoundingClientRect();
  [...main.querySelectorAll(":scope > .split")].forEach((node, n) => placeHandle(node, wanted[n], g, rect));
}

function makeHandle(handle) {
  const node = el("div", "split " + (vertical(handle) ? "v" : "h"));
  node.dataset.axis = handle.axis;
  node.dataset.i = String(handle.i);
  node.tabIndex = 0;
  node.setAttribute("role", "separator");
  node.setAttribute("aria-orientation", vertical(handle) ? "vertical" : "horizontal");
  node.title = "drag to resize · arrow keys · double-click to reset";
  node.setAttribute("aria-label", handle.label);
  node.addEventListener("dblclick", resetLayout);
  node.addEventListener("pointerdown", (ev) => {
    const g = layoutGeometry();
    if (!g || ev.button !== 0 || ev.buttons !== 1) return;
    ev.preventDefault();
    node.setPointerCapture(ev.pointerId);
    const start = sizesOf(handle, g);
    const from = vertical(handle) ? ev.clientX : ev.clientY;
    node.classList.add("drag");
    document.body.classList.add("dragging", vertical(handle) ? "v" : "h");
    const up = () => {
      node.removeEventListener("pointermove", move);
      node.removeEventListener("pointerup", up);
      node.removeEventListener("pointercancel", up);
      node.removeEventListener("lostpointercapture", up);
      node.classList.remove("drag");
      document.body.classList.remove("dragging", "v", "h");
      saveLayout();
      layoutHandles();
    };
    const move = (e) => {
      // a button released somewhere we did not see ends the drag
      if (e.buttons !== 1) return up();
      const now = layoutGeometry();
      if (!now) return;
      nudge(handle, now, start, (vertical(handle) ? e.clientX : e.clientY) - from);
      layoutHandles();
    };
    node.addEventListener("pointermove", move);
    node.addEventListener("pointerup", up);
    node.addEventListener("pointercancel", up);
    node.addEventListener("lostpointercapture", up);
  });
  node.addEventListener("keydown", (ev) => {
    const g = layoutGeometry();
    if (!g) return;
    const keys = vertical(handle) ? { ArrowLeft: -1, ArrowRight: 1 } : { ArrowUp: -1, ArrowDown: 1 };
    if (ev.key === "Home" || ev.key === "Escape") {
      if (ev.key === "Home") {
        ev.preventDefault();
        resetLayout();
      }
      return;
    }
    if (!(ev.key in keys)) return;
    ev.preventDefault();
    nudge(handle, g, sizesOf(handle, g), keys[ev.key] * (ev.shiftKey ? 80 : 24));
    saveLayout();
    layoutHandles();
  });
  return node;
}

// Called whenever the consoles may have moved: the data changed, or the window did.
function layoutRefresh() {
  if (document.body.classList.contains("dragging")) return;
  applyLayout();
  layoutHandles();
}

addEventListener("resize", layoutRefresh);
addEventListener("load", layoutRefresh);
if (window.ResizeObserver) new ResizeObserver(layoutRefresh).observe(document.querySelector("main"));
