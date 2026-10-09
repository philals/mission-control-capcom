# capcom

Local kanban boards for Jira stories. One `board.json` per story at
`$CAPCOM_ROOT/<KEY>/`. The stories folder is required: set `CAPCOM_ROOT` or pass `--root DIR`; `capcom root` prints the one in use. This tool is the only
writer of `board.json`; skills and the Herdr plugin call it.

A repo may appear more than once in a task's `repos`: it then expects that many live (not closed) PRs there before the task can be `done`.
Task statuses: `todo → planning → planned → implementing → done`, plus `dropped`. A task may also go
straight from `todo` to `implementing` (spikes, manual testing), under the same dependency and blocked checks.
`blocked` is a flag with a reason. A task may enter `implementing` only when every
dependency is `done`.

## Install

    cargo install --path . --root ~/.local --locked

`~/.local/bin` must be on your `PATH`.

## Commands

All commands print JSON. Our own errors go to stderr with exit status 1; clap usage errors
(unknown flag, missing argument) exit with status 2. Every command accepts `--root DIR`
(or `CAPCOM_ROOT`) to choose the stories directory.

Valid task statuses: `todo`, `planning`, `planned`, `implementing`, `done`, `dropped`. Valid story statuses: `in_progress` (default), `in_review`, `done`.
`--type` is `pr` (the default), `spike` or `decision`. PR states: `draft`, `ready`, `merged`, `closed`.

    capcom init <KEY> --title T [--jira-url U]
    capcom root                              # the stories folder in use
    capcom list
    capcom show <KEY>
    capcom validate <KEY>
    capcom ready <KEY>                       # unfinished tasks whose dependencies are done
    capcom add-task <KEY> --title T [--type pr|spike|decision] [--depends T1,T2] [--repos a,b]
    capcom set-deps <KEY> <ID> [--depends T1,T2]
    capcom set-repos <KEY> <ID> [--repos a,b]
    capcom status <KEY> <ID> <STATUS>
    capcom story-status <KEY> <in_progress|in_review|done>
    capcom block <KEY> <ID> --reason R
    capcom unblock <KEY> <ID>
    capcom add-pr <KEY> <ID> --repo R --url U [--state draft]
    capcom set-pr <KEY> <ID> --url U --state draft|ready|merged|closed
    capcom set-agent <KEY> <ID> --pane P --skill S
    capcom clear-agent <KEY> <ID>
    capcom set-subtask <KEY> <ID> <JIRA-KEY>
    capcom refresh <KEY>                     # reads PR states with gh, marks tasks done once every PR is merged

`ready` lists non-finished tasks whose dependencies are all `done`. That includes tasks already
`implementing`, so check a task's own status too.

`refresh` needs `gh` authenticated. It looks up every PR of unfinished tasks (30 second limit per
lookup, done without holding the board lock), then moves an `implementing` task to `done` when
every PR is merged. A task stays `implementing` through CI fixes and review comments. A `pr` task
with `repos` also waits until every listed repo has a non-closed PR, and `done` is refused until
then.

The story has its own status. `story-status <KEY> in_review` is refused until every task is `done`
or `dropped` (and at least one is `done`); `done` only follows `in_review`; moving back to
`in_progress` reopens the story. Adding a task to an `in_review` story moves it back to
`in_progress`, and adding one to a `done` story is refused until it is reopened. Boards written
before this change that contain an `in_review` task read it as `implementing`.

Writes take an exclusive lock on `<story dir>/.board.lock` (git-ignored) around load, change and
save, so concurrent writers do not lose updates.

Schema: `../schemas/board.schema.json`.

## capcom-tui

    capcom-tui [KEY] [--root DIR] [--pr-query QUERY] [--no-prs] [--deploy-repos REPOS] [--no-runs] [--workdir DIR] [--demo]

A live kanban view with a story list (completed stories hidden by default, `d` or the header
button toggles them), a board per story, and a pull requests panel. Keyboard and mouse. With `KEY`
it opens that story's board directly, otherwise the list. It re-reads each `board.json` about four
times a second and redraws when the content changes; if a board fails to load it keeps the last
good view and shows the error in the footer.

The pull requests panel runs one `gh api graphql` search (default
`is:pr author:@me state:open archived:false sort:updated-desc -label:icebox`, override with
`--pr-query` or `CAPCOM_PR_QUERY`) in a background thread: every 5 seconds while a check is
running or queued, every 30 seconds otherwise, or immediately on `r`. Nothing is written to disk.
`--no-prs` disables it. See the README for the full key and mouse table.

The manual runs panel lists the GitHub Actions runs you started with the "Run workflow" button
(`event=workflow_dispatch`, filtered to your login, which is looked up at run time through `gh` and
kept in memory). The repos come from your open PRs, the PRs recorded on stories, and
`--deploy-repos` / `CAPCOM_DEPLOY_REPOS` (comma separated `owner/name`). One REST call per repo
every 10 seconds while any run is active, every 30 seconds otherwise; stages are fetched only for
active or failed runs and cached once finished. A finished run is dropped 3 hours after it finished.
`--no-runs` disables it.

Panel sizes (PR/runs split, bottom height, kanban column widths) are saved when you change them, in
`$CAPCOM_TUI_SETTINGS`, else `$XDG_CONFIG_HOME/capcom/tui.json`, else
`~/.config/capcom/tui.json`. Values are clamped on load; a missing or damaged file means defaults.

### Herdr launches

Only when `HERDR_ENV=1` (the TUI is running in a Herdr pane). `--workdir` / `CAPCOM_WORKDIR` (default: the current
folder) is where new workspaces and tabs start. A launch looks for a workspace labelled with the story key (creating it,
`--no-focus`, if missing), puts the agent in a new tab of it (the root tab of a new workspace), then runs
`herdr agent start NAME --kind claude --pane P` and `herdr agent prompt NAME "/skill KEY [ID]"`. A live agent with the
same name is focused instead. Names: `<key>-<id>-plan`, `<key>-<id>-impl`, `<key>-breakdown`, lowercased, at most 32 characters.

A TODO task sent to IMPLEMENTING first asks "Does an agent need to implement this?". Yes launches as above. No moves the
task with the same rules as `capcom status <KEY> <ID> implementing` and starts nothing, for work you do yourself.

### Shared cache

`capcom-tui` keeps `prs-<hash of query>.json` and `runs-<hash of repo list>.json` in `$CAPCOM_CACHE_DIR`, else
`$XDG_CACHE_HOME/capcom`, else `~/.cache/capcom` (mode 0600 files in a 0700 folder; files older than a day are removed).
Each holds `{"fetched_at", "value"}`. A poll tick reads it and calls GitHub only when `now - fetched_at` is at least the
interval for that data (busy data is stale sooner). The fetcher holds an exclusive lock on `<name>.lock`; copies that find
it taken wait up to 75 seconds and reuse the result. A manual refresh reuses data fetched in the last 3 seconds. Errors are
never cached. If the folder cannot be created, the TUI simply polls on its own.

### Herdr plugin

`herdr-plugin.toml` defines the `tui` overlay pane (runs `capcom-tui`) and the `open` action (`scripts/open.sh`). The
script finds a pane titled `capcom-tui` in the active workspace (`capcom-tui --find-pane` reads `herdr pane list` JSON on
stdin), focuses it, or else opens a new overlay with the `KEY=VALUE` lines of `<plugin config dir>/env` passed as
`--env`. The TUI sets its terminal title to `capcom-tui` at start-up so it can be found again.

### Finishing tasks from the board

`x` or a drop on DONE: `refresh::lookup_all` for the task's PR urls (`gh pr view`), then `refresh::apply` under the board lock
(`store::update`); a task with no PRs goes through `rules::transition` to done, which refuses a `pr` task without a merged PR.
`R` does the same for every unfinished task (`capcom refresh KEY`). A background thread rechecks the recorded PRs that the open
PR list does not show every 60 seconds (merged and closed results are cached for an hour, shared through the cache folder); results
only feed the card hint unless auto-sync (`S`, saved as `auto_sync` in `tui.json`) is on, in which case changes are applied to the board.
