# Story → Task workflow: design

Date: 2026-10-02
Status: the original design notes. Some parts have moved on (the terminal board and the Herdr plugin now exist, see `reference.md` and `tui.md` for current behaviour).

## Goal

Replace an ad-hoc, one-skill-per-ticket habit with a skill workflow that takes a Jira story to draft PRs, tracked on a local kanban board that a Herdr plugin can display and drive.

Success: for any story, you can see every task, its status, its dependencies and its PRs on one board. You can launch the right agent for the next step from that board, and no state exists only in a chat session.

## Decisions already made

- Three skills: **break-down**, **plan-task**, **implement-task**.
- Local files are the source of truth. No Jira status sync.
- Jira is touched once: break-down may create subtasks under the story, then never updates them.
- One board per story. Tasks are split in consultation with the user.
- A task may have several PRs, across several repos.
- No stacked PRs. A task starts only after its dependencies are merged.
- All PRs are created as drafts. The user marks them ready in GitHub.
- PR titles follow `feat: TICKET-1234 <description> (T1)`, where `T1` is the board task id. The skill uses a commit-and-push helper if the user has one, and plain `git` and `gh` otherwise.
- The board UI ships as a new Herdr plugin, planned and built as a separate sub-project.
- The Jira key in PR titles is the subtask key when subtasks exist, otherwise the story key.
- Delivery is split into two plans. **Plan 1** (schema, board tool, three skills) is useful on its own and ships first. **Plan 2** (the Herdr plugin) sits over `board.json` and triggers the skills, and is iterated separately.

## Story review (added 2026-10-05)

The story has its own status on the board: `in_progress` (default), `in_review`, `done`, changed with `capcom story-status`. `in_review` needs every task `done` or `dropped` (and at least one `done`); `done` follows `in_review`; moving back to `in_progress` reopens the story. Adding a task moves an `in_review` story back to `in_progress` and is refused on a `done` story until it is reopened.

The `story-review` skill (`/story-review PROJ-123`) checks the subtasks are complete, reads the Jira story's acceptance criteria and intent, the task files and the merged PRs, reviews each criterion against the evidence, and gives feedback to the user. It writes nothing to Jira. Boards written before this change read an `in_review` task as `implementing`.

## Layout

```
<CAPCOM_ROOT>/PROJ-123/
  board.json
  story.md                 # Jira summary, acceptance criteria, agreed breakdown notes
  tasks/
    T1-add-endpoint.md     # one file per task, grows through plan and implement
    T2-spike-xyz.md
```

`board.json` is the index and holds status. The task file holds the content. A task's status is never duplicated in its markdown.

## Statuses

`todo → planning → planned → implementing → done`, plus a `dropped` terminal state and a `blocked` flag. (Updated 2026-10-09: `todo → implementing` is allowed too, for spikes and manual testing that need no plan.) (Updated 2026-10-05: the per-task `in_review` status was removed. Review is a story-level activity, see Story review above.)

| Status | Meaning | Entered by |
|---|---|---|
| `todo` | Task exists, not planned | break-down |
| `planning` | A plan-task run is active | plan-task start |
| `planned` | Plan written, questions answered | plan-task finish |
| `implementing` | Work in progress, draft PRs may exist | implement-task start |
| `done` | All PRs merged; a spike or decision with no PR finishes here directly | board refresh, or implement-task for PR-less tasks |
| `dropped` | Cancelled | user |

- CI fixes and review comments happen inside the task while it stays `implementing`. `done` means every PR is merged.
- `blocked` is a flag with a reason, valid in any non-terminal status.
- A task is **ready** when every task in `dependsOn` is `done`. Readiness is computed, never stored.
- Implement-task refuses to start on a task that is not `planned` and ready, and asks the user instead.

## board.json

One file per story, with a JSON Schema at `schemas/board.schema.json` in the tool's repo and `schemaVersion` for migrations.

```json
{
  "schemaVersion": 1,
  "story": { "key": "PROJ-123", "title": "...", "jiraUrl": "..." },
  "tasks": [{
    "id": "T1",
    "title": "...",
    "type": "pr",
    "status": "todo",
    "blocked": null,
    "dependsOn": [],
    "file": "tasks/T1-add-endpoint.md",
    "repos": ["api"],
    "prs": [{ "repo": "api", "url": "...", "state": "draft" }],
    "jiraSubtask": "PROJ-124",
    "agent": { "pane": "w1:p3", "skill": "plan-task", "startedAt": "..." }
  }]
}
```

- `type`: `pr`, `spike` or `decision`. Spikes and decisions need no PR, and their result is written into the task file.
- `prs[].state`: `draft`, `ready`, `merged`, `closed`.
- `agent` records the live agent run, kept apart from `status`. A task in `planning` with no live agent is shown as stale.
- Task ids are never reused, including after a task is dropped.

## Board tool

A small command-line tool, the only writer of `board.json`. Skills and the plugin both call it, so nobody hand-edits the JSON.

- Validates against the schema and writes atomically.
- Commands cover: set status, set or clear blocked, add and update PRs, set agent, add task, drop task, compute ready tasks.
- `refresh` reads PR states from GitHub (`gh`) and marks a task `done` once every PR is merged.
- Written in Rust, named `capcom` (the name `board` is taken by herdr-board). Core logic (types, validation, operations) lives in a library crate shared with the plugin in Plan 2. The skills depend only on the command interface.

## Skills

### break-down

Input: a Jira story key.

1. Fetch the story (summary, acceptance criteria, linked pages) and do a light scan of the affected code.
2. Propose tasks. Each is tagged `pr`, `spike` or `decision`, with dependencies called out explicitly and a critical-path note.
3. Consult the user on every split, merge and dependency. Anything bigger than one reviewable change is flagged. Unknowns get a spike first, with dependants marked provisional.
4. Write `story.md`, `board.json` and one stub file per task.
5. Ask whether to create Jira subtasks under the story. If yes, create them and store the keys in `jiraSubtask`. Never touch them again.
6. Re-running on an existing board proposes adds, splits and drops, and applies only what the user approves.

### plan-task

Input: story key and task id.

1. Read `story.md`, the task stub and the plans of its dependencies.
2. Set status `planning`, then research the code across the affected repos.
3. Work through open questions with the user, recording every answer under **Decisions** in the task file.
4. Write the plan into the task file: affected repos, approach, files, tests, risks, and the expected PRs per repo with their merge order.
5. Set status `planned`.

### implement-task

Input: story key and task id.

1. Check the gate: status `planned`, all dependencies `done`. Otherwise stop and ask.
2. Set status `implementing`. Ask the user whether to work in the main repo checkout or a git worktree. Then create a branch per repo from the up-to-date default branch, since there are no stacked PRs.
3. Follow the plan, stopping to update the plan with the user if it proves wrong. Record new answers under **Decisions**.
4. Commit and open each PR as a **draft** through the user's commit-and-push helper (or plain `git` and `gh`), supplying the task's `jiraSubtask` key (or the story key if there is none) so the title follows `feat: TICKET-1234 <description>`. Draft means that agent skips auto-merge and labelling.
5. Record each PR in `prs` with state `draft`. Stop. The user marks PRs ready in GitHub, and board refresh marks the task `done` once every PR is merged. CI fixes and review comments are handled in the task while it stays `implementing`.
6. Review rounds (CI fixes and comments) can use a looping CI-and-review skill if the user has one, and leave status unchanged.

## Herdr plugin (separate sub-project)

Herdr plugins are a directory with `herdr-plugin.toml` declaring panes and actions, linked with `herdr plugin link <path>`. The installed `herdr-tsk` plugin is a close template: an `open-board` action that opens or focuses one board pane, plus a launcher script.

The plugin provides:

- A board pane showing `board.json` as columns (`todo`, `planning`, `planned`, `implementing`, `done`), with dependency, blocked, stale-agent and PR-state indicators.
- Next-action launching per task: `todo` runs plan-task, `planned` and ready runs implement-task, `implementing` offers resume. It opens a Herdr pane, starts a Claude agent with the skill command, records the pane in `agent`, and lets the user answer the skill's questions in that pane.
- Jump to a task's agent pane, and timed and on-demand `refresh`.
- Read-only for anything the skills own. All writes go through the board tool.

This is a new plugin, not an extension of `herdr-board`. It reads `board.json` directly through the shared library crate, so local files stay the only source of truth. Its detailed design belongs to Plan 2.

## Out of scope

Jira status sync, stacked PRs, auto-marking PRs ready, auto-merge, and a global cross-story board.

## Open questions

None blocking Plan 1. Plan 2 must verify how a plugin pane is linked and built (`herdr plugin link`, build steps) and how to start an agent in a pane from the plugin.

## Delivery

**Plan 1: skills and board file** (ships first, usable immediately)
1. Board schema and `capcom` tool.
2. break-down skill, to validate the layout end to end.
3. plan-task and implement-task.
4. Retire the two old skills.

**Plan 2: Herdr plugin** (separate, iterated after Plan 1 is in use)
1. Plugin skeleton, link and open-board action.
2. Read-only board view.
3. Next-action launching and agent tracking.
4. GitHub refresh, indicators, polish.
