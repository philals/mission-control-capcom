# Pokemon mode for the Agents panel (design)

Status: approved by the user on 2026-11-16. Size: small change.

## Intent

A toggle in the Agents panel switches each agent's portrait between its SG-1 character and a Pokemon.
Only the Agents panel changes. Mockups: https://claude.ai/artifact/EQaeLCkVrZk4hRbVQmeVC7

## Goals

- SG-1 | Pokemon toggle in the Agents header, next to the sort icon; the choice persists.
- Pokemon mode shows a PMDCollab portrait whose expression follows the agent's status.
- Each agent keeps both its SG-1 persona and its Pokemon; toggling never reshuffles either.

## Non-goals

Animation, the roaming strip, cries, scenery, a hand-picker for a Pokemon, any change to My PRs, My Watch, Daily,
tabs or panes (they stay on SG-1; `capcom.persona` stays the SG-1 persona).

## Decisions

| Topic | Decision |
|---|---|
| Pokemon | #001-#151 only. Each agent gets an unused one at random and keeps it (like `assign()` for SG-1, stored per agent address). |
| Expression | needs you: Worried, working: Determined, done: Joyous, idle: Normal. |
| Row | Chevron ring, status badge, grouping unchanged. Second line shows the Pokemon's name instead of the SG-1 name. |
| Portraits | Not bundled. Downloaded from PMDCollab SpriteCollab on first use, cached under the app data dir, via curl. Until a portrait is cached the row shows the SG-1 face. |

## Approaches considered

1. Bundle all portraits in the mod: no network, but ships Pokemon art with attribution terms; both reference repos avoid it. Rejected.
2. Download on first use and cache (chosen): matches the reference repos, small repo.
3. Port the whole strip/sidebar mods: far more than the user wants. Rejected.

## Left to the builder

Where the cache lives, how the download is run (mirror `exec` use in `herdr-mod/capcom-agents/main.js`), the toggle's icon,
and the Pokemon names list (151 names in a data file).
