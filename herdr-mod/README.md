# capcom-prs: the PR sidebar for the Herdr app

A Herdr app mod (`~/.config/herdr-app/mods/capcom-prs`) that lists your open and recently merged PRs under the
Agents list, in every tab. It runs `capcom prs` every 30 seconds, so it needs `capcom` on your PATH and no web
service.

- Dot: red (failing check or conflict), amber (checks running), green, purple (merged). `auto` means automerge is on.
- Click an open PR to expand it: the rail (ready, CI, copilot, threads, approved, automerge, merge), what is in the
  way, the checks that are not passing (click one to open it), and toggles for automerge, auto-fix, auto-review and
  ready/draft. Hover a row for its blocker.
- `🤖` goes to the live agent that made the PR; else resumes its session in a new tab; else opens the PR.
  A merged row does the same when clicked. Merged PRs drop off after 30 minutes.
- `↗` opens the PR on GitHub. `↻` asks GitHub again. `⚙` shows the defaults for auto-fix and auto-review.
- Changes go through `capcom pr-do`. Auto-fix and auto-review are only settings: `capcom web` (or `capcom-tui`)
  has to be running to act on them, and the `⚙` strip says so when `capcom web` is not.

Install: `scripts/install.sh` then `scripts/install-herdr-mod.sh` (copies the mod, clones the `main` layout to `capcom`
with a `prs` block, and switches to it).
