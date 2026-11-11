// Draws the SG-1 portraits and the gate rings into <out>/portraits and <out>/rings.
// Usage: node scripts/gen-avatars.mjs <mod folder>
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const src = fs.readFileSync(path.join(here, "../herdr-mod/capcom-agents/characters.js"), "utf8");
const { CHARACTERS } = await import("data:text/javascript;base64," + Buffer.from(src).toString("base64"));
const out = process.argv[2];
if (!out) throw new Error("usage: gen-avatars.mjs <mod folder>");

function portrait(a) {
  const s = a.skin, h = a.hair;
  let back = "", front = "";
  if (a.style === "bob") back = `<path d="M19 28Q19 12 32 12Q45 12 45 28L45 45L37 45L37 30L27 30L27 45L19 45Z" fill="${h}"/>`;
  if (a.style === "long") back = `<path d="M17 28Q17 11 32 11Q47 11 47 28L48 60L38 60L38 30L26 30L26 60L16 60Z" fill="${h}"/>`;
  if (a.style === "short") front = `<path d="M20 28Q19 11 32 11Q45 11 44 28Q41 20 32 20Q23 20 20 28Z" fill="${h}"/>`;
  if (a.style === "bob" || a.style === "long") front = `<path d="M20 29Q19 11 32 11Q45 11 44 29Q40 19 32 19Q24 19 20 29Z" fill="${h}"/>`;
  if (a.style === "cap") front = `<path d="M20 22Q20 10 32 10Q44 10 44 22Z" fill="${a.cap}"/><rect x="18" y="21" width="28" height="3" rx="1.5" fill="${a.cap}"/><path d="M20 26h3v4h-3zM41 26h3v4h-3z" fill="${h}"/>`;
  if (a.style === "fringe") front = `<path d="M20 26h3v7h-3zM41 26h3v7h-3z" fill="${h}"/>`;
  const gl = a.glasses ? `<g fill="none" stroke="#1b1b1b" stroke-width="1.4"><circle cx="27" cy="29" r="4"/><circle cx="37" cy="29" r="4"/><path d="M31 29h2"/></g>` : "";
  const mark = a.mark ? `<circle cx="32" cy="20.5" r="2.4" fill="#e6b84a"/><path d="M31 20.5h2M32 19.5v2" stroke="#7a5a10" stroke-width=".6"/>` : "";
  const stars = a.stars ? `<g fill="#f0cf5a"><circle cx="19" cy="52" r="2"/><circle cx="45" cy="52" r="2"/></g>` : "";
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><defs><clipPath id="c"><circle cx="32" cy="32" r="32"/></clipPath></defs><g clip-path="url(#c)"><rect width="64" height="64" fill="${a.bg}"/>${back}<path d="M6 64Q9 45 32 44Q55 45 58 64Z" fill="${a.uni}"/><rect x="27.5" y="37" width="9" height="9" fill="${s}" opacity=".85"/><ellipse cx="32" cy="28" rx="12" ry="14" fill="${s}"/>${front}${mark}<circle cx="27" cy="29" r="1.5" fill="#1b1b1b"/><circle cx="37" cy="29" r="1.5" fill="#1b1b1b"/>${gl}<path d="M28.5 36q3.5 2 7 0" stroke="#7a4a3a" stroke-width="1.2" fill="none" stroke-linecap="round"/>${stars}</g></svg>`;
}

const chevron = (i, total) => `<path transform="rotate(${((i * 360) / total).toFixed(2)} 28 28)" d="M23.4 1.5H32.6V7.8L28 11L23.4 7.8Z" fill="#000"/>`;
const ring = (total, mask) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 56 56">${Array.from({ length: total }, (_, i) => i).filter((i) => mask === null || mask & (1 << i)).map((i) => chevron(i, total)).join("")}</svg>`;

fs.mkdirSync(path.join(out, "portraits"), { recursive: true });
fs.mkdirSync(path.join(out, "rings"), { recursive: true });
for (const c of CHARACTERS) fs.writeFileSync(path.join(out, "portraits", `${c.id}.svg`), portrait(c));
for (const total of [6, 7]) {
  fs.writeFileSync(path.join(out, "rings", `dim${total}.svg`), ring(total, null));
  for (let mask = 1; mask < 1 << total; mask++) fs.writeFileSync(path.join(out, "rings", `r${total}-${mask}.svg`), ring(total, mask));
}
console.log(`drew ${CHARACTERS.length} portraits and ${(1 << 6) + (1 << 7) - 2 + 2} rings in ${out}`);
