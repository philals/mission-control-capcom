---
name: story-plan-task
description: Research and plan one task from a story board, working through open questions with the user, and save the plan into the task file. Use when asked to plan a task. Arguments - story key and task id, for example PROJ-123 T2.
---

# Plan a task

Arguments: `<KEY> <ID>`. Board data lives in `<root>/<KEY>/` (`<root>` is the stories folder printed by `capcom root`).

## Rules

- Planning only. Do not change code in any repository.
- Change `board.json` only through `capcom`. Never edit it by hand.
- Ask questions one at a time and wait. Record every answer under `## Decisions` in the task file so no later session asks again.
- No stacked PRs: plan each PR so it can merge on its own once its dependencies are merged.
- If research errors or you cannot continue, do not leave the board in `planning`: restore status before stopping (see Gate rule and Approve step).

## Steps

1. **Load.** `capcom show <KEY>`. Read `story.md`, the task file (path is the task's `file` field, relative to `<root>/<KEY>/`), and the task files of everything in `dependsOn` (their plans and decisions constrain this one; `dependsOn` is shown by `capcom show`).
2. **Gate.** The task must be `todo` or `planned` for a re-plan (ask first). If it is `planning`, ask whether an earlier run was abandoned before continuing; if the user says that run is still active, stop and tell the user (do not start a second planning run). Otherwise ask which status to restore if this run is stopped (`todo` or `planned`, depending on whether an approved plan already exists in `## Plan`). For a task found in `planning`, the restore status the user chose decides the case for steps 6 and 7: `todo` means a first-time plan, `planned` means a re-plan. Refuse `implementing`, `done` and `dropped`. Valid statuses: todo, planning, planned, implementing, done, dropped. Re-plan rule: if restarting from `planned`, keep the existing plan untouched while drafting; replace `## Plan` only after user approval. If the user stops or you cannot continue, run `capcom status <KEY> <ID> planned` (restoring from `planning`) and say the old plan stands. For first-time plans from `todo`, use `capcom status <KEY> <ID> todo` on stop.
3. **Start.** If the task is already `planning`, skip the status command (it is already in the right state); otherwise run `capcom status <KEY> <ID> planning`. Then record this conversation: `capcom set-session <KEY> <ID> --skill story-plan-task` (it reads `$CLAUDE_CODE_SESSION_ID`; skip quietly if that is not set).
4. **Research.** Read the code across the affected repositories (ask the user where their code lives if it is not obvious). For a `spike`, the plan is the question, the probe and the expected output. For a `decision`, the plan is the options and who decides.
5. **Resolve questions** with the user, one at a time. Record each answer under `## Decisions`.
6. **Draft the plan** with these subsections (the text, not yet in the task file):
   - `### Affected repositories` (repo names, one per line)
   - `### Approach`
   - `### Files` (files to create or change, per repo)
   - `### Tests` (what to test and how, using targeted runs; CI covers the full suites)
   - `### PRs` (each expected PR per repo, in merge order across repos)
   - `### Risks and open items`
   For a first-time plan (started from `todo`), write this into `## Plan` in the task file, then run `capcom set-repos <KEY> <ID> --repos a,b` using the affected repositories (comma-separated list; omit `--repos` to clear; pass the full list every time; **list a repository once per expected PR**, so two PRs in `api` and one in `ui` is `--repos api,api,ui`, because the task cannot be marked done until every expected PR exists and is merged), before asking for approval. For a re-plan (started from `planned`), keep the draft in the conversation until after the user approves.
7. **Approve.** Show the plan summary and ask the user to approve it. On approval of a first-time plan (the plan and repos are already saved), run `capcom status <KEY> <ID> planned`. On approval of a re-plan, write the plan into `## Plan`, run `capcom set-repos <KEY> <ID> --repos a,b` using the affected repositories (comma-separated list; omit `--repos` to clear; pass the full list every time; **list a repository once per expected PR**, so two PRs in `api` and one in `ui` is `--repos api,api,ui`, because the task cannot be marked done until every expected PR exists and is merged), then run `capcom status <KEY> <ID> planned`. Either way the approved task ends `planned`. If the user stops before approving, run `capcom status <KEY> <ID> planned` (re-plan) or `capcom status <KEY> <ID> todo` (first-time plan), and say the old or draft plan does not yet stand.
8. **Finish.** Check `capcom ready <KEY>` and the task's own status. If `capcom show` shows the task as blocked, say so and give the reason instead of suggesting implement-task. Say the next step `/story-implement-task <KEY> <ID>` only if this task is listed, not blocked, and its status is `planned`; otherwise say what it is waiting on using `dependsOn` and each dependency's status from `capcom show <KEY>`.
