# storyboard

A local kanban workflow for Jira stories: a Rust tool (`storyboard`) that is the only writer of each story's `board.json`, plus four Claude Code skills that break a story down, plan and implement its tasks as draft PRs, and review the finished story. Your story data lives in a separate folder you choose, so this repo contains no project data.

## The story workflow

Take a Jira story, break it into PR-sized tasks, plan each task, implement each task as draft PRs. Progress for every story lives in one local file, `<root>/<KEY>/board.json`, shown as a kanban.

```
 story ──► break down ──► board.json (tasks, deps) ──► plan task ──► implement task ──► draft PRs
 (Jira)   /story-break-down                            /story-plan-task  /story-implement-task
```

Local files are the source of truth. Jira is written only once: `story-break-down` can create subtasks, and never updates them afterwards.

### Triggering the skills

Type these in Claude Code (they are also picked up when you ask for the same thing in plain words):

| Step | Command | What it does |
|---|---|---|
| 1 | `/story-break-down PROJ-123` | Reads the story, proposes tasks and dependencies, agrees them with you, writes `stories/PROJ-123/`. Offers to create Jira subtasks once. |
| 2 | `/story-plan-task PROJ-123 T1` | Researches one task, asks you questions, writes the plan into the task file. Never changes code. |
| 3 | `/story-implement-task PROJ-123 T1` | Asks main checkout or worktree, implements the plan, opens draft PRs through the `git-commit-push` agent. PR titles end with the task id, for example `feat: PROJ-124 add notice endpoint (T1)`. |
| 4 | `/story-review PROJ-123` | When every task is done: checks the subtasks are complete, reviews the story's acceptance criteria and intent against what was built, and gives you feedback. |

Task ids (`T1`, `T2`, ...) come from the board. `storyboard show PROJ-123` lists them. `storyboard root` prints the stories folder in use.

### Typical run

1. `/story-break-down PROJ-123`, then approve the task table.
2. For each task, in dependency order: `/story-plan-task PROJ-123 T1`, answer the questions, approve the plan.
3. `/story-implement-task PROJ-123 T1`. It only starts when the task is `planned` and every dependency is `done`. Choose main repo or worktree when asked.
4. In GitHub, review the draft PRs and mark them **ready** yourself. Nothing is marked ready or merged for you. CI fixes and review comments are handled inside the task (the `pr-looper` skill, or resume `/story-implement-task`); the task stays `implementing` throughout.
5. Once the PRs are merged, run `storyboard refresh PROJ-123` (needs `gh` logged in). It marks the task `done` when every PR is merged.
6. Dependent tasks unblock once their dependencies are `done`. Repeat from step 2 for the next task.
7. When every task is `done`, run `/story-review PROJ-123`. It reviews the whole story against its acceptance criteria and intent and reports back. Accept it to mark the story `done`, or add follow-up tasks with `/story-break-down PROJ-123` to reopen it.

Tasks that are spikes or decisions have no PR. Their result is written into the task file and they are marked `done` when you accept it.

To resume a task that is already `implementing` (after a pause, or for CI failures and review comments on its PRs), run `/story-implement-task PROJ-123 T1` again and confirm you want to resume.

Worktrees are not removed automatically. Once a task is `done` (its PRs merged), run `/story-implement-task PROJ-123 T1` again to clean up: it removes only the worktrees recorded in the task file, only if their PRs are merged and the tree is clean, and asks you before touching anything else.

### Statuses

```
todo ─► planning ─► planned ─► implementing ─► done      (task: done = every PR merged)
                          dropped = cancelled, from any unfinished status

in_progress ─► in_review ─► done                          (story, via /story-review)
```

`blocked` is a flag with a reason, not a status. No stacked PRs: a task cannot start until its dependencies are merged. There is no per-task review status; review happens on the story. A story can only go `in_review` when every task is `done` or `dropped`. Adding a task reopens it.

## Requirements

Rust (to build the tool), `gh` authenticated (for `storyboard refresh` and PR work), the Atlassian MCP server (the skills read Jira and `story-break-down` can create subtasks), a `git-commit-push` agent that opens the PRs, and the `pr-looper` skill for CI and review rounds.

## Layout

```
<root>/<KEY>/             # <root> = your stories folder (STORYBOARD_ROOT), kept in a separate repo
  board.json               # tasks, statuses, dependencies, PRs (never edit by hand)
  story.md                 # summary, acceptance criteria, breakdown notes
  tasks/T1-<slug>.md       # one file per task: Description, Plan, Decisions, Progress
```

This repo:

```
src/, tests/, Cargo.toml   # the storyboard tool, the only writer of board.json
skills/                    # story-break-down, story-plan-task, story-implement-task, story-review
schemas/board.schema.json  # JSON Schema for board.json
docs/design.md             # design spec
docs/reference.md          # command reference
```

Choose where stories live and tell the tool, in your shell profile and in Claude Code's settings (`env` in `~/.claude/settings.json`) so the skills see it too:

```bash
export STORYBOARD_ROOT=~/path/to/your/stories
```

From the root of this repo, symlink the skills into `~/.claude/skills/` so Claude Code can find them:

```bash
ln -s "$PWD/skills/story-break-down" ~/.claude/skills/story-break-down
ln -s "$PWD/skills/story-plan-task" ~/.claude/skills/story-plan-task
ln -s "$PWD/skills/story-implement-task" ~/.claude/skills/story-implement-task
ln -s "$PWD/skills/story-review" ~/.claude/skills/story-review
```

## The storyboard tool

Install (Rust required), then check it is on your `PATH`:

```bash
cargo install --path . --root ~/.local --locked
storyboard --help
```

Everyday commands:

```bash
storyboard list                      # all stories with task counts
storyboard show PROJ-123             # full board as JSON
storyboard ready PROJ-123            # tasks whose dependencies are done
storyboard status PROJ-123 T2 done   # change a task status (illegal moves are refused)
storyboard story-status PROJ-123 in_review   # story status (refused until every task is done)
storyboard block PROJ-123 T3 --reason "waiting on schema change"
storyboard refresh PROJ-123          # sync PR states from GitHub
```

All commands print JSON, and errors exit non-zero. `ready` also lists tasks already `implementing`, so check a task's own status as well. Full command reference: [`docs/reference.md`](docs/reference.md).

## What is not built yet

A Herdr plugin that shows `board.json` as a kanban and launches the skills from it is planned separately. Until then, use `storyboard show` and `storyboard list`.
