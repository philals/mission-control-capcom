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

// Which grid the window has room for (the same limits as the style sheet), or null when the page scrolls.
function layoutVariant() {
  if (innerHeight < 580 || innerWidth < 1000) return null;
  return innerWidth >= 1900 ? "w4" : innerWidth >= 1300 ? "w3" : "w2";
}

function showing(id) {
  const p = $(id);
  return p && !p.hidden ? p : null;
}

// The columns and rows that can be resized, as groups of consoles whose joint box is one track.
function layoutGeometry() {
  const v = layoutVariant();
  if (!v) return null;
  const agents = showing("agents");
  const runs = showing("runs");
  const board = $("board");
  const prs = $("prs");
  const side = $("side");
  const some = (...xs) => xs.filter(Boolean);
  let cols;
  let rows = null;
  let sideRows = null;
  if (v === "w4") {
    cols = some(agents, board, prs, runs).map((p) => [p]);
  } else if (v === "w3") {
    cols = some(agents, board, side).map((p) => [p]);
    if (runs) sideRows = [prs, runs];
  } else {
    cols = [some(board, agents), [side]];
    if (agents) rows = [board, agents];
    if (runs) sideRows = [prs, runs];
  }
  return { v, key: `${v}:${agents ? 1 : 0}${runs ? 1 : 0}`, cols, rows, sideRows };
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
  const saved = g && layoutState[g.key];
  main.style.gridTemplateColumns = g && saved && saved.cols && saved.cols.length === g.cols.length ? track(saved.cols) : "";
  main.style.gridTemplateRows = g && saved && saved.rows && g.rows ? track(saved.rows) : "";
  $("side").style.gridTemplateRows = g && saved && saved.side && g.sideRows ? track(saved.side) : "";
}

function resetLayout() {
  const g = layoutGeometry();
  if (g) delete layoutState[g.key];
  saveLayout();
  applyLayout();
  layoutHandles();
  toast("layout reset");
}

// The sizes now, in pixels, of what a handle moves.
function sizesOf(handle, g) {
  if (handle.axis === "col") return g.cols.map((group) => { const u = union(group); return u.right - u.left; });
  if (handle.axis === "row") return g.rows.map((p) => p.getBoundingClientRect().height);
  return g.sideRows.map((p) => p.getBoundingClientRect().height);
}

function setSizes(handle, g, sizes) {
  const entry = (layoutState[g.key] = layoutState[g.key] || {});
  const weights = sizes.map((x) => +(x / 100).toFixed(3));
  entry[handle.axis === "col" ? "cols" : handle.axis === "row" ? "rows" : "side"] = weights;
  applyLayout();
}

// Move the edge `handle.i` by `delta` pixels from `start` sizes.
function nudge(handle, g, start, delta) {
  const min = handle.axis === "col" ? MIN_COL : MIN_ROW;
  const total = start[handle.i] + start[handle.i + 1];
  const a = Math.max(min, Math.min(total - min, start[handle.i] + delta));
  const sizes = start.slice();
  sizes[handle.i] = a;
  sizes[handle.i + 1] = total - a;
  setSizes(handle, g, sizes);
}

function describeHandles(g) {
  const out = [];
  g.cols.forEach((group, i) => {
    if (i < g.cols.length - 1) out.push({ axis: "col", i, label: "Resize the consoles either side of this edge" });
  });
  if (g.rows) out.push({ axis: "row", i: 0, label: "Resize the stories and agents consoles" });
  if (g.sideRows) out.push({ axis: "side", i: 0, label: "Resize the pull requests and runs consoles" });
  return out;
}

// Where a handle sits, relative to <main>.
function placeHandle(el, handle, g, mainRect) {
  let x;
  let y;
  let w;
  let h;
  if (handle.axis === "col") {
    const a = union(g.cols[handle.i]);
    const b = union(g.cols[handle.i + 1]);
    x = (a.right + b.left) / 2;
    y = Math.min(a.top, b.top);
    h = Math.max(a.bottom, b.bottom) - y;
    el.style.left = `${x - mainRect.left}px`;
    el.style.top = `${y - mainRect.top}px`;
    el.style.height = `${h}px`;
    el.style.width = "";
    el.style.marginLeft = "";
  } else {
    const [top, bottom] = handle.axis === "row" ? g.rows : g.sideRows;
    const a = top.getBoundingClientRect();
    const b = bottom.getBoundingClientRect();
    y = (a.bottom + b.top) / 2;
    x = a.left;
    w = a.width;
    el.style.left = `${x - mainRect.left}px`;
    el.style.top = `${y - mainRect.top}px`;
    el.style.width = `${w}px`;
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
  const node = el("div", "split " + (handle.axis === "col" ? "v" : "h"));
  node.dataset.axis = handle.axis;
  node.dataset.i = String(handle.i);
  node.tabIndex = 0;
  node.setAttribute("role", "separator");
  node.setAttribute("aria-orientation", handle.axis === "col" ? "vertical" : "horizontal");
  node.title = "drag to resize · arrow keys · double-click to reset";
  node.setAttribute("aria-label", handle.label);
  node.addEventListener("dblclick", resetLayout);
  node.addEventListener("pointerdown", (ev) => {
    const g = layoutGeometry();
    if (!g || ev.button !== 0 || ev.buttons !== 1) return;
    ev.preventDefault();
    node.setPointerCapture(ev.pointerId);
    const start = sizesOf(handle, g);
    const from = handle.axis === "col" ? ev.clientX : ev.clientY;
    node.classList.add("drag");
    document.body.classList.add("dragging", handle.axis === "col" ? "v" : "h");
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
      nudge(handle, now, start, (handle.axis === "col" ? e.clientX : e.clientY) - from);
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
    const keys = handle.axis === "col" ? { ArrowLeft: -1, ArrowRight: 1 } : { ArrowUp: -1, ArrowDown: 1 };
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
