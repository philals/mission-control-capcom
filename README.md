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

## Viewing the board

`storyboard-tui` is a live kanban view of your stories. It is read-only, and it reloads on its own whenever a `board.json` changes, so you can leave it open next to the agents that are updating the board. It works with the keyboard and the mouse.

```bash
storyboard-tui              # the story list; click or press Enter to open one
storyboard-tui PROJ-123     # open a story's board straight away
```

**Story list.** One row per story with its key, title, story status, `done/total` tasks, a progress bar and a count per task status. Completed stories (story status `done`) are hidden by default; click `[ Show done ]` in the header or press `d` to show them, and again to hide them.

**Board.** One column per task status (a Dropped column appears only if something was dropped), boxed cards with a status line (ready to implement, waits on dependencies, blocked, PRs merged), the story status in the header, and a detail sheet for the selected card. Narrow terminals (under 60 columns) show one column at a time.

| Where | Mouse | Keys |
|---|---|---|
| List | click a row to open it; click `[ Show done ]` / `[ Hide done ]`; wheel moves the selection | `↑↓`/`jk` select, `Enter`/`→` open, `d` show or hide done |
| Board | click `‹ Stories` to go back; click a column or card to select it; click the selected card again for its detail; click outside a sheet to close it; wheel scrolls a column | `←→`/`hl` column, `↑↓`/`jk` card, `[` `]` switch story, `Tab` PR panel, `Enter` detail, `Esc`/`b` back to the list |
| PR panel | click a row to select it, click again for the sheet; wheel scrolls | `Tab` focus, `↑↓` select, `Enter` sheet, `o` open on GitHub, `r` refresh |
| Anywhere | | `r` reload, `?` help, `q` or Ctrl-C quit |

**Pull requests panel.** A bottom panel lists your open pull requests with their CI stages, refreshed from GitHub in the background:

- **Main screen:** *all* your open PRs from the query, including ones unrelated to any story. PRs recorded on a storyboard task are tagged `PROJ-123 · T2`.
- **Inside a story:** only that story's PRs (the ones recorded on its tasks), tagged with the task id. A PR that GitHub no longer lists as open (for example a merged one) still shows from the board data.
- **Each row:** a `[READY]` (green) or `[DRAFT]` (grey) badge, the repo and number, title, labels, review state, comment count and time since update. A PR that is only known from the board shows `[MERGED]` or `[CLOSED]` instead. Under it a CI summary such as `✓ 12  ✗ 1  ◔ 2  ● 1` (the orange filled circle is pending/queued), then every stage that is still running, queued or failed on its own line, in that order (`◔ CI / build  running 2m 05s`, `● CI / deploy  queued`, `✗ CI / unit  failed 1m 10s`). Passed and skipped stages are only counted; a long list is capped with `… +N more`. A thin line separates one PR from the next.
- **Detail sheet:** select a row and press Enter (or click it twice) for the full sheet: draft or ready for review, every check grouped running, queued, failed, passed, with durations. `o` (or the button) opens the PR in your browser.
- **Keys:** `Tab` moves focus to the panel and back, `↑↓` select a PR, `Enter` opens the sheet, `o` opens it on GitHub, `r` refreshes now. The mouse works too: click a row, click it again for the sheet, scroll over the panel.

It fetches with one `gh api graphql` call (it uses your existing `gh` login, so no token is handled by this tool). It polls about every 5 seconds while any check is running or queued and about every 30 seconds otherwise, and costs roughly one GraphQL rate-limit point per poll. Nothing from GitHub is written to disk: PR data lives in memory only. If a fetch fails, the last data stays and the error shows in the panel.

The query defaults to `is:pr author:@me state:open archived:false sort:updated-desc -label:icebox`. Change it with `--pr-query "..."` or `STORYBOARD_PR_QUERY`. `--no-prs` turns the panel off (for example when `gh` is not installed). On a short terminal (under about 16 rows) the panel is hidden.

It needs `STORYBOARD_ROOT` or `--root` like the tool. While it runs, the terminal's own text selection is replaced by mouse clicks (hold Shift to select text in most terminals).

## Requirements

Rust (to build the tool), `gh` authenticated (for `storyboard refresh`, PR work and the pull requests panel), the Atlassian MCP server (the skills read Jira and `story-break-down` can create subtasks), a `git-commit-push` agent that opens the PRs, and the `pr-looper` skill for CI and review rounds.

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

Everyday commands (`storyboard-tui` is installed alongside it):

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
