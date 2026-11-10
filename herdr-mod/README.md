# capcom-prs: the PR sidebar for the Herdr app

A Herdr app mod (`~/.config/herdr-app/mods/capcom-prs`) that lists your open and recently merged PRs under the
Agents list, in every tab. It runs `capcom prs` every 30 seconds, so it needs `capcom` on your PATH and no web
service.

- Dot: red (failing check or conflict), amber (checks running), green, purple (merged). `auto` means automerge is on.
- Click a row: go to the live agent that made the PR; else resume its session in a new tab; else open the PR.
- `↗` opens the PR on GitHub. `↻` asks GitHub again.

Install: `scripts/install.sh` then `scripts/install-herdr-mod.sh` (copies the mod, clones the `main` layout to `capcom`
with a `prs` block, and switches to it).
