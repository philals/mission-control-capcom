# storyboard

Local kanban boards for Jira stories. One `board.json` per story at
`$STORYBOARD_ROOT/<KEY>/`. The stories folder is required: set `STORYBOARD_ROOT` or pass `--root DIR`; `storyboard root` prints the one in use. This tool is the only
writer of `board.json`; skills and the Herdr plugin call it.

Task statuses: `todo → planning → planned → implementing → done`, plus `dropped`.
`blocked` is a flag with a reason. A task may enter `implementing` only when every
dependency is `done`.

## Install

    cargo install --path . --root ~/.local --locked

`~/.local/bin` must be on your `PATH`.

## Commands

All commands print JSON. Our own errors go to stderr with exit status 1; clap usage errors
(unknown flag, missing argument) exit with status 2. Every command accepts `--root DIR`
(or `STORYBOARD_ROOT`) to choose the stories directory.

Valid task statuses: `todo`, `planning`, `planned`, `implementing`, `done`, `dropped`. Valid story statuses: `in_progress` (default), `in_review`, `done`.
`--type` is `pr` (the default), `spike` or `decision`. PR states: `draft`, `ready`, `merged`, `closed`.

    storyboard init <KEY> --title T [--jira-url U]
    storyboard root                              # the stories folder in use
    storyboard list
    storyboard show <KEY>
    storyboard validate <KEY>
    storyboard ready <KEY>                       # unfinished tasks whose dependencies are done
    storyboard add-task <KEY> --title T [--type pr|spike|decision] [--depends T1,T2] [--repos a,b]
    storyboard set-deps <KEY> <ID> [--depends T1,T2]
    storyboard set-repos <KEY> <ID> [--repos a,b]
    storyboard status <KEY> <ID> <STATUS>
    storyboard story-status <KEY> <in_progress|in_review|done>
    storyboard block <KEY> <ID> --reason R
    storyboard unblock <KEY> <ID>
    storyboard add-pr <KEY> <ID> --repo R --url U [--state draft]
    storyboard set-pr <KEY> <ID> --url U --state draft|ready|merged|closed
    storyboard set-agent <KEY> <ID> --pane P --skill S
    storyboard clear-agent <KEY> <ID>
    storyboard set-subtask <KEY> <ID> <JIRA-KEY>
    storyboard refresh <KEY>                     # reads PR states with gh, marks tasks done once every PR is merged

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
