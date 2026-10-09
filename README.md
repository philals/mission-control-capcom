# CAPCOM

**Mission Control for story-driven work: one voice at the console.** Break a Jira story into PR-sized tasks, plan and implement each one with [Claude Code](https://claude.com/claude-code) skills, and watch all of it (tasks, draft PRs, CI stages, manual deploys) on one live board in your terminal.

[![CI](https://github.com/philals/mission-control-capcom/actions/workflows/ci.yml/badge.svg)](https://github.com/philals/mission-control-capcom/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

<p align="center">
  <img src="docs/img/board.svg" alt="capcom-tui showing a story board with tasks in the TODO, PLANNING, PLANNED, IMPLEMENTING and DONE columns, above panels of open pull requests with their CI stages and of manual workflow runs" width="100%">
</p>

> The picture is drawn from invented demo data. Try it yourself with no setup: `capcom-tui --demo`.

## The name

**CAPCOM** (short for Capsule Communicator) is the designated communication liaison at NASA's Mission Control Center in Houston, Texas, who serves as the single voice speaking directly to astronauts in space.

This app is meant to be that voice for your work: one place that tells you what is planned, what is in flight and what needs you. The app and its commands are called `capcom` and `capcom-tui`; the repository is `mission-control-capcom` so it is easy to tell apart from the video-game company Capcom, which this project has nothing to do with.

## Read this first: it is one person's workflow

CAPCOM is built around **how its author works**, not around a general idea of project management. The author takes a Jira story, splits it into tasks that each become one or more *draft* pull requests (often in several repositories), reviews and merges them by hand, and likes to keep the real state in plain local files rather than in a tool. The statuses, the "no stacked PRs" rule, the draft-only PRs, the 1960s space-programme look and the Herdr integration all come from that mental model.

If that sounds like you, great. If not, treat this as a worked example: fork it, delete what you do not need, change the statuses and skills to fit how you work. Issues and ideas are welcome, but changes that pull it away from this model may be politely declined. See [CONTRIBUTING.md](CONTRIBUTING.md).

## What is in the box

| Part | What it does |
|---|---|
| **`capcom`** (command) | The only writer of each story's `board.json`: tasks, statuses, dependencies, PRs. Refuses illegal moves. Everything it prints is JSON. |
| **Four Claude Code skills** | `/story-break-down`, `/story-plan-task`, `/story-implement-task`, `/story-review`. They talk to you, Jira and `git`/`gh`, and change the board only through `capcom`. |
| **`capcom-tui`** | A live terminal board (keyboard and mouse): your stories, your open PRs with their CI stages, and the GitHub Actions runs you started by hand. It can start the skills for you and finish tasks when their PRs merge. |
| **Herdr plugin** (optional) | Opens `capcom-tui` from a keyboard shortcut in [Herdr](https://herdr.dev), and lets the board launch agents in Herdr panes. |

## How it works

```
 story ──► break down ──► board.json (tasks, deps) ──► plan task ──► implement task ──► draft PRs
 (Jira)   /story-break-down                            /story-plan-task  /story-implement-task
```

- **Local files are the source of truth.** Each story is a folder with a `board.json`, a `story.md` and one markdown file per task. Jira only ever receives the subtasks, once, if you say yes.
- **Task statuses:** `todo → planning → planned → implementing → done`, plus `dropped`, and a `blocked` flag. A task can skip planning (`todo → implementing`) for spikes and manual testing. `done` means every expected PR is merged.
- **Story statuses:** `in_progress → in_review → done`, moved by `/story-review` once every task is finished.
- **PRs are always drafts, and never stacked.** You mark them ready and merge them yourself; nothing is merged for you.
- **A task can need several PRs.** List a repo once per expected PR (`--repos api,api,ui` means two PRs in `api` and one in `ui`); the task cannot finish until all of them exist and are merged.

Design notes: [docs/design.md](docs/design.md). Command reference: [docs/reference.md](docs/reference.md).

## Quick start

### 1. Look around (no setup)

```bash
git clone https://github.com/philals/mission-control-capcom && cd mission-control-capcom
cargo install --path . --locked      # installs `capcom` and `capcom-tui` into ~/.cargo/bin
capcom-tui --demo                    # invented stories, PRs and runs; nothing is read or written
```

### 2. Use it for real

You need: [Rust](https://rustup.rs) 1.89 or newer, the [GitHub CLI](https://cli.github.com) (`gh auth login`), and [Claude Code](https://claude.com/claude-code) with the Atlassian MCP server if you want the skills to read Jira.

```bash
# a folder for your story data (keep it in its own repo if you like; capcom never needs it inside this one)
mkdir -p ~/stories
export CAPCOM_ROOT=~/stories         # also add this to your shell profile and to `env` in ~/.claude/settings.json

# make the skills visible to Claude Code
for s in story-break-down story-plan-task story-implement-task story-review; do
  ln -s "$PWD/skills/$s" ~/.claude/skills/$s
done
```

Then, in Claude Code:

| Step | Command | What it does |
|---|---|---|
| 1 | `/story-break-down PROJ-123` | Reads the story, proposes tasks and dependencies, agrees them with you, writes `$CAPCOM_ROOT/PROJ-123/`. Offers to create Jira subtasks once. |
| 2 | `/story-plan-task PROJ-123 T1` | Researches one task, asks you questions, writes the plan into the task file. Never changes code. |
| 3 | `/story-implement-task PROJ-123 T1` | Asks main checkout or worktree, implements the plan, opens **draft** PRs titled like `feat: PROJ-124 add notice endpoint (T1)`. |
| 4 | `/story-review PROJ-123` | When every task is done: checks the subtasks, reviews the story's acceptance criteria and intent against what was built, and tells you what it found. |

And in another terminal, `capcom-tui` shows it all and updates as the files change.

A typical run, in order: break the story down and approve the table; plan each task in dependency order; implement it; review and mark the draft PRs ready on GitHub yourself; when they merge, drag the card to DONE in the TUI (or run `capcom refresh PROJ-123`); repeat; finish with `/story-review`. More detail on resuming tasks and cleaning up worktrees is in the skills themselves (`skills/*/SKILL.md`).

## The terminal board

<p align="center">
  <img src="docs/img/list.svg" alt="capcom-tui story board: demo stories in TO DO, DOING and IN REVIEW columns with progress bars and task counts, above the pull requests and manual runs panels" width="100%">
</p>

- **Stories and boards:** a kanban of your stories (TO DO, DOING, IN REVIEW) and a kanban per story with a card for each task (ready to implement, waits on dependencies, blocked, PRs merged). Drag a finished story from DOING to IN REVIEW and it opens a Herdr tab running `/story-review`.
- **Your PRs, with CI:** every open PR from a GitHub search you control, `[DRAFT]` or `[READY]`, with the CI stages that are running, queued or failed listed one per line. A PR that is all green and only waiting for a reviewer is flagged `◉ AWAITING REVIEW` and counted in the panel title. Click to open, copy the link, or mark a draft ready (it asks first).
- **Back to the agent:** a PR made through a task has an `[ agent ]` button that focuses the Claude Code session that made it in Herdr, or resumes it (`claude --resume`) if the tab was closed.
- **Your manual runs:** the "Run workflow" runs you started, such as a nonprod deploy, with their stages and a link to each.
- **Finishing tasks:** drag an IMPLEMENTING card to DONE (or press `x`) and capcom checks its PRs on GitHub, and only finishes the task if every one is merged.
- **Resizable and remembered:** drag any divider; sizes are saved.
- **Accessible by design:** a fixed theme with at least 4.5:1 text contrast on every screen (a test checks it), state shown with words and icons as well as colour, and focus shown by a double border.

Everything it does, every key and every setting is in [docs/tui.md](docs/tui.md).

## Configuration

| Setting | Flag | What it is | Default |
|---|---|---|---|
| `CAPCOM_ROOT` | `--root` | Folder holding one folder per story | required (not with `--demo`) |
| `CAPCOM_PR_QUERY` | `--pr-query` | GitHub search for the PR panel (`@me` is resolved by GitHub) | `is:pr author:@me state:open archived:false sort:updated-desc -label:icebox` |
| `CAPCOM_DEPLOY_REPOS` | `--deploy-repos` | Extra `owner/name` repos (comma separated) to look for your manual runs in | repos of your PRs and stories |
| `CAPCOM_WORKDIR` | `--workdir` | Where new Herdr workspaces start | the current folder |
| `CAPCOM_TUI_SETTINGS` | | File for the saved panel sizes | `~/.config/capcom/tui.json` |
| `CAPCOM_CACHE_DIR` | | Shared GitHub cache | `~/.cache/capcom` |
| | `--no-prs`, `--no-runs` | Turn the PR or runs panel off | |

## Herdr

If you use [Herdr](https://herdr.dev), `capcom-tui` run inside a Herdr pane can start the skills for you: paste a Jira key to start a breakdown, or drag a card onto PLANNING or IMPLEMENTING. The repo is also a Herdr plugin with an `open` action, so a key such as `prefix+y` opens the board as an overlay. Install it with `herdr plugin install philals/mission-control-capcom` (it builds the binaries, so it needs Rust; read [Herdr's trust note](https://herdr.dev/docs/plugins/#trust-and-security) first, since a plugin is ordinary code), then bind a key as described in [docs/tui.md](docs/tui.md#herdr-plugin). Nothing here needs Herdr; outside it those features are simply off.

## Privacy and security

- **No project data in this repo.** Your stories live in the folder you choose. The examples and tests use invented names (`acme/widgets`, `PROJ-123`, `DEMO-101`).
- **No tokens.** All GitHub access goes through your existing `gh` login; capcom never reads or stores a credential.
- **No telemetry, no network of its own.** The only calls are `gh` to GitHub and, when you ask the skills to, your Atlassian MCP server to Jira.
- **What is written to disk:** the boards (only by `capcom`, under a lock), your panel sizes, and a small cache of GitHub results that is readable only by you and tidied after a day.
- **Found a vulnerability?** See [SECURITY.md](SECURITY.md).

## Development

```bash
cargo build
cargo test                                   # the whole suite (it is quick)
cargo test --bin capcom-tui <name>           # one test
capcom-tui --demo                            # a quick look at UI changes
capcom-tui --screenshot docs/img/board.svg --screen board --size 150x34   # regenerate the README pictures
capcom-tui --screenshot docs/img/list.svg  --screen list  --size 150x30
```

```
src/                      the capcom command and its library (board model, rules, store, refresh)
src/bin/capcom-tui/       the terminal board (app state, drawing, GitHub polling, Herdr, cache, demo data)
skills/                   the four Claude Code skills
schemas/board.schema.json JSON Schema for board.json
herdr-plugin.toml         the Herdr plugin manifest (scripts/open.sh is its action)
docs/                     design notes, command reference, TUI guide, README pictures
```

## Contributing

Please read [CONTRIBUTING.md](CONTRIBUTING.md) first, especially the note at the top of this page about whose workflow this is.

## Credits

The look of the original board owes a debt to [herdr-board](https://github.com/nelsonPires5/herdr-board), and the whole thing to Herdr and Claude Code. The 1960s space-programme theme is a nod to the people who actually did mission control.

## License

[MIT](LICENSE)
