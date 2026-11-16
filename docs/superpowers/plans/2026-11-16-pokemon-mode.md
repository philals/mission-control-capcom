# Pokemon Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A toggle in the Agents panel switches each agent's portrait between its SG-1 character and a Pokemon (#001-#151) whose expression follows the agent's status.

**Architecture:** A pure module `pokemon.js` holds the 151 names, the random pick, the status-to-expression map and the portrait path/URL. `capcom-agents/main.js` stores a Pokemon per agent (like `persona`), reads the mode from `shared("agents.portraits")`, downloads missing portraits with `curl` into the app data dir, and draws the Pokemon in `row()` once the file exists (SG-1 face until then). The toggle is a second section in the existing sort popup.

**Tech Stack:** Herdr mod JS (`gpui-kit`, `gpui-base`, `herdr`), Node 20 `node --test` for the pure module, `curl`.

**Spec:** `docs/superpowers/specs/2026-11-16-pokemon-mode-design.md`

## Global Constraints

- Pokemon #001-#151 only.
- Expressions: needs you (`blocked`) Worried, `working` Determined, `done` Joyous, `idle` Normal.
- Only the Agents panel changes. `capcom.persona` (read by capcom-pane and capcom-tabs) stays the SG-1 persona.
- Portraits are not committed. Source: `https://raw.githubusercontent.com/PMDCollab/SpriteCollab/master/portrait/<NNNN>/<Expression>.png` (4-digit zero-padded number, e.g. `0004/Worried.png`).
- Cache: `<appHome()>/data/capcom-agents/pokemon/<NNNN>/<Expression>.png`.
- Each agent keeps both its SG-1 persona and its Pokemon; toggling never reshuffles either.
- Minimal comments (user style). No full test suite locally; scoped `node --test` only. CapCom pushes straight to `main`, no worktree or PR.

## Review Focus

- Every one of the 151 is already taken by live agents: reuse a Pokemon rather than throw or loop forever.
- Stored `pokemon` entry that is not an integer 1-151 (hand-edited or corrupt): ignore it and assign a new one.
- Portrait missing, offline, or `curl` absent: row shows the SG-1 face, no error, one retry no sooner than 5 minutes later.
- An agent status outside the four known ones (e.g. `error`): Normal expression.
- The same portrait wanted by several rows at once: one download in flight, not several.

---

### Task 1: The pure Pokemon module

**Files:**
- Create: `herdr-mod/capcom-agents/pokemon.js`
- Test: `scripts/pokemon.test.mjs`

**Interfaces:**
- Produces (all named exports of `pokemon.js`, no imports from `herdr`):
  - `POKEMON: string[]` (151 names, index 0 is Bulbasaur, 3 Charmander, 24 Pikachu, 150 Mew)
  - `nameOf(n: number): string` (n is 1-151)
  - `validNumber(v: unknown): boolean` (integer 1-151)
  - `pickPokemon(taken: Set<number>, rand?: () => number): number` (a number not in `taken`; if all 151 are taken, any of 1-151)
  - `expressionFor(status: string): "Worried" | "Determined" | "Joyous" | "Normal"`
  - `portraitFile(n: number, expression: string): string` (`"0004/Worried.png"`)
  - `portraitUrl(n: number, expression: string): string`

- [ ] **Step 1: Write the failing tests** in `scripts/pokemon.test.mjs` using `node:test` and `node:assert/strict`, importing from `../herdr-mod/capcom-agents/pokemon.js`:
  - `POKEMON` has length 151 with `[0]==="Bulbasaur"`, `[3]==="Charmander"`, `[24]==="Pikachu"`, `[150]==="Mew"`; `nameOf(25)==="Pikachu"`.
  - `validNumber`: true for 1 and 151; false for 0, 152, 1.5, `"4"`, `null`, `NaN`.
  - `expressionFor`: `blocked`->Worried, `working`->Determined, `done`->Joyous, `idle`->Normal, `error`->Normal, `undefined`->Normal.
  - `portraitFile(4,"Worried")==="0004/Worried.png"`; `portraitUrl(25,"Joyous")` equals the Global Constraints URL with `0025/Joyous.png`.
  - `pickPokemon(new Set([1,2,3]), () => 0)` returns 4 (first free when `rand` is 0) and never returns a taken number over 200 random calls with 150 taken (must return the one free number, 151).
  - `pickPokemon(new Set(range 1..151))` returns an integer 1-151 and does not hang.

- [ ] **Step 2: Run to verify it fails**

Run: `node --test scripts/pokemon.test.mjs`
Expected: FAIL (module not found).

- [ ] **Step 3: Implement `pokemon.js`** with the exports above. `pickPokemon` chooses uniformly among the free numbers (`rand` scales an index into the free list), falling back to a uniform pick of 1-151 when none are free. Names are the standard English names of the original 151 (Nidoran♀/♂ written `Nidoran F` / `Nidoran M`; `Farfetch'd`, `Mr. Mime`).

- [ ] **Step 4: Run to verify it passes**

Run: `node --test scripts/pokemon.test.mjs`
Expected: all tests PASS.

- [ ] **Step 5: Commit** (to `main`)

```bash
git add herdr-mod/capcom-agents/pokemon.js scripts/pokemon.test.mjs docs/superpowers
git commit -m "Herdr: pokemon module for the Agents panel (first 151, expression by status)"
```

---

### Task 2: Mode, assignment, download and row in the Agents panel

**Files:**
- Modify: `herdr-mod/capcom-agents/main.js` (imports at line 7-11; `init` ~line 88; after `assign` ~line 170; `character` ~line 211; `avatar` ~line 234; `sortMenu` ~line 296; `render` ~line 424; `row` ~lines 523 and 619-644; the sort button ~line 725)

**Interfaces:**
- Consumes: everything exported by `pokemon.js` (Task 1).
- Produces (on the `Agents` class):
  - `portraitsMode(): "sg1" | "pokemon"`, reading `shared("agents.portraits")`, default `"sg1"`
  - `this.pokemon: Record<address, number>`, persisted in `localStorage` key `"pokemon"`
  - `assignPokemon(list: Agent[]): void`
  - `this.cached: Set<string>` of `portraitFile` keys present on disk, `this.fetching: Set<string>`, `this.failed: Map<string, number>` (ms of last failure)
  - `wantPortrait(cx, n: number, expression: string): string | null`, returning the app-home-relative path when cached, else starting a download (once, not within 5 min of a failure) and returning `null`

- [ ] **Step 1: Imports and state.** Import `* as pokemonMod`-style named imports from `./pokemon.js`. In `init`, load `this.pokemon` from `localStorage` (drop entries failing `validNumber`), init `this.cached`, `this.fetching`, `this.failed`, and prefill `this.cached` by listing `<appHome()>/data/capcom-agents/pokemon/*/*.png` with `fs/promises` (same pattern as `scanPhotos`), then `cx.notify()`.

- [ ] **Step 2: `assignPokemon(list)`**, called in `render` next to `this.assign(list)`: give each agent without a stored number `pickPokemon(taken)` where `taken` is the set of numbers held by the listed agents; persist; prune entries for gone agents with the same rule `assign` uses (`!live.has(k) && !this.changed[k]`).

- [ ] **Step 3: `wantPortrait`.** Build the relative path `data/capcom-agents/pokemon/<portraitFile>`. If cached return it. Otherwise, unless in `this.fetching` or failed less than 300000 ms ago, add to `fetching` and `cx.spawn` an `exec` of `curl -fsSL --create-dirs -o <abs path> <portraitUrl>` through the same `SH` wrapper `noteOf` uses (`exec(SH[0], [...SH.slice(1), "curl", ...])`, 20000 ms timeout); on success add to `cached`, on any failure set `failed`; always remove from `fetching` and `cx.notify()`. Write to a `.part` path and `mv` it into place so a half download is never shown (`sh -c 'curl ... -o "$2.part" "$1" && mv "$2.part" "$2"'`).

- [ ] **Step 4: Row.** In `row()`, when `portraitsMode()==="pokemon"` and the agent has a stored number and `wantPortrait(cx, n, expressionFor(a.status))` returns a path, draw the portrait with a new `pokemonAvatar(t, path)` (`Avatar` with `rounded(8)`, background `#2a2f37`, image `size_full`, same `AV` size) instead of `this.avatar(...)`, and use `nameOf(n)` in place of `who.label` in the second line. When it returns `null`, draw the SG-1 avatar and label unchanged. `capcom.persona` publishing in `render` is not touched. `row` needs `cx`: pass it from `render(cx)`.

- [ ] **Step 5: Toggle.** In `sortMenu(t)`, below the sort items add a "Portraits" caption and two items (`SG-1`, `Pokémon`) styled exactly like the sort items, a check on the active one, calling `setShared("agents.portraits", "sg1" | "pokemon")` and `closeOverlay(props().overlay)`. Raise the popup `height` at the sort button (line ~725) from 140 to fit the extra caption and two rows (about 140 + 20 + 62).

- [ ] **Step 6: Syntax check and install**

Run: `bash scripts/install-herdr-mod.sh`
Expected: ends with `capcom mods and layout installed ...` and `herdr-app check` reports no error for `capcom-agents`. (Per memory, run the installer yourself after changes.)

- [ ] **Step 7: Look at it in the running app.** In the Herdr desktop app (not the `herdr` CLI), open the sort popup, choose Pokémon, and confirm: rows show Pokemon portraits after a moment; the expression changes when an agent goes working, needs you, done; switching back to SG-1 returns the original faces; unplugging the network (or `HC`-less run) leaves SG-1 faces without an error toast.

- [ ] **Step 8: Commit**

```bash
git add herdr-mod/capcom-agents/main.js
git commit -m "Herdr: Pokemon mode for the Agents panel, toggled in the sort popup"
```

---

### Task 3: Docs

**Files:**
- Modify: `herdr-mod/README.md` (the `## capcom-agents: SG-1 personas` section)

- [ ] **Step 1:** Add a short "Pokemon mode" paragraph: how to toggle (sort icon, Portraits), the 151 limit, the status-to-expression table, that portraits download on first use into `<app home>/data/capcom-agents/pokemon/` from PMDCollab SpriteCollab (needs `curl`; SG-1 faces show until they arrive), and that other panels stay on SG-1.
- [ ] **Step 2: Commit and push** to `main`

```bash
git add herdr-mod/README.md
git commit -m "Docs: Pokemon mode in the Agents panel"
git push origin main
```

---

## Self-review

- Spec coverage: toggle (Task 2 step 5), persisted choice (`shared` key), expression map (Task 1 + step 4), per-agent persistence and independent of SG-1 (step 2), 1-151 (Task 1), not bundled, download and cache and SG-1 fallback (step 3-4), other panels untouched (step 4), docs (Task 3).
- One change from the approved design: the toggle sits inside the sort icon's popup, not as a new button, because the header icon already opens this popup and the header is this mod's own layout.
- Types: `portraitFile`, `portraitUrl`, `expressionFor`, `pickPokemon`, `validNumber`, `nameOf` are used with the same names in Tasks 1 and 2.
- Gap: the herdr mod has no automated test harness, so Task 2 is verified by `install-herdr-mod.sh` (runs `herdr-app check`) and a manual look.
