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

Install: `scripts/install.sh` then `scripts/install-herdr-mod.sh`. This repo is the source of truth for every capcom
mod (`capcom-prs`, `capcom-agents`, `capcom-board`, `capcom-jira`; `capcom-tabs` is Herdr's `tabs` mod plus `capcom-tabs.patch`, built at install) and for the `capcom` layout
(`herdr-mod/layouts/capcom.json`); the script copies them into the app and switches to the layout. Edit them here, not
in `~/.config/herdr-app`, or the next install overwrites the change.

## capcom-agents: SG-1 personas

Each agent gets a Stargate SG-1 character (O'Neill, Carter, Daniel, Teal'c, Hammond, Vala, then the cast again with a
number), kept for as long as the agent exists. Its portrait sits in a ring of seven chevrons: one lights per step (in plain light, not the character colour, so it reads without hue) of
the agent's PR that is done (ready, CI, copilot, threads, approved, automerge, merge; six when the repo has no automerge).
No PR yet is a dark ring; merged is a full one.

- The portraits and rings are drawn by `scripts/gen-avatars.mjs` (needs `node`) when you run `scripts/install-herdr-mod.sh`.
- To use real photos, save square images as `~/.config/herdr-app/mods/capcom-agents/avatars/<id>.jpg` (or `.png`), with
  `<id>` one of `oneill carter daniel tealc hammond vala`. They are picked up within 30 seconds and are not in the repo.
- The cast is in `capcom-agents/characters.js`.

## capcom-jira: your Jira tickets

A Jira button in the tab bar opens a board of the tickets assigned to you, to the right of the Capcom board, grouped by
Jira status. Tickets whose status category is Done are not fetched. Click a card to expand it.

- Each card has ID and URL copy buttons, `↗` to open in Jira, and a `▶` button for the next skill: `Plan`
  (`/story-plan KEY`) for a ticket with no board, `Do T3` / `Resume T3` for the next ready task, `Review` once every
  task is done. The tag `Capcom 2/4` shows a ticket that already has a board, followed by one PR icon per PR on its tasks (coloured by CI, purple once merged). Hover one for the same card as an agent's PR icon; click it to open the PR.
- The expanded card adds progress by task status, copy actions (ID, URL, `KEY: title`), open in Jira, and every
  skill that fits: plan or re-plan, implement a task, review, or implement directly (a small bug or change with no board).
- Data comes from `scripts/capcom-jira-me`, installed to `~/.local/bin`. It runs `claude -p` (Haiku) with the Atlassian
  connector and writes `~/.cache/capcom/jira-me.json` (override with `CAPCOM_JIRA_FILE`; set `CAPCOM_JIRA_MODEL` for
  another model). One run takes about 15 seconds and costs under a cent.
- It fetches when the pane opens if the file is over 5 minutes old, then every 5 minutes while the pane is open, and
  on `↻`. Closing the pane stops it. A second fetch never starts while one is running.
