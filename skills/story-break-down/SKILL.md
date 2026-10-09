---
name: story-break-down
description: Break a Jira story into PR-sized tasks on a local kanban board (board.json) with dependencies called out. Use when asked to break down, split, slice or start work on a Jira story. Argument - the Jira story key, for example PROJ-123.
---

# Story break-down

Turns a Jira story into a small set of workable tasks saved under
`<root>/<KEY>/` (`<root>` is the stories folder printed by `capcom root`). Next steps for each task are
`/story-plan-task <KEY> <ID>` then `/story-implement-task <KEY> <ID>`.

## Rules

- Change `board.json` only through the `capcom` command (run `capcom --help`). Never edit it by hand.
- Consult the user on every split, merge and dependency. Propose, then wait.
- A task is a PR-level chunk: independently reviewable and mergeable. One task may need several PRs and several repos, and that is fine.
- There are no stacked PRs. A task starts only after its dependencies are merged, so prefer slices that can merge independently (backward-compatible changes, flags) and call out real dependencies explicitly.
- Types: `pr` (ends in merged PRs), `spike` (ends in an answer written into the task file), `decision` (ends in a recorded decision). Spikes and decisions need no PR.
- Local files are the source of truth. Never update Jira after creating subtasks.

## Steps

1. **Existing board?** Run `capcom show <KEY>`. If it fails and the error says the `board.json` file is missing, there is no board yet, so continue with step 2. If it fails for any other reason (for example an invalid board), stop and report the error. If it succeeds, this is a re-run: show the current board, propose only adds, splits, re-pointed dependencies and drops, and apply only what the user approves (add new tasks with `add-task`, change dependencies with `set-deps`, mark as dropped with `status <KEY> <ID> dropped`). After applying changes, skip to step 7 for newly added tasks only. Valid statuses: todo, planning, planned, implementing, done, dropped. If the story is `in_review` or `done` (see `story.status` in `show`), new tasks reopen it: an `in_review` story goes back to `in_progress` automatically when a task is added, and a `done` story must first be reopened with `capcom story-status <KEY> in_progress`, which you do only with the user's agreement. Never reuse ids and never touch tasks past todo without asking.
2. **Fetch the story.** Use `getAccessibleAtlassianResources` for the cloud id (if the lookup fails or returns several sites, ask the user which to use), then `getJiraIssue` for `<KEY>`. Read its description, acceptance criteria, linked issues and any linked Confluence pages (`getConfluencePage`).
3. **Light code scan.** Find the affected repositories (ask the user where their code lives if it is not obvious from the story or the current directory) and skim just enough to ground the slicing. This is not planning; deep research belongs to plan-task.
4. **Propose the breakdown.** Show a table: id placeholder, title, type, depends on, repos. Note the critical path and which tasks can run in parallel. Flag anything bigger than one reviewable change. Where there are unknowns, put a spike first and mark dependent tasks as provisional in their descriptions.
5. **Iterate with the user** until they approve the table. Ask one question at a time.
6. **Write it down.**
   - `capcom init <KEY> --title "<story title>" --jira-url <site URL>/browse/<KEY>` (the site URL comes from the lookup in step 2)
   - `capcom add-task <KEY> --title "..." --type pr|spike|decision [--depends T1,T2] [--repos a,b]` for each task (list a repo once per expected PR in it: `--repos api,api,ui` means two PRs in `api` and one in `ui`, and the task is not done until all exist and are merged), in dependency order. Each call prints the new id and file path. Record both `{id,file}` from every add-task output, map each placeholder id in the approved table to its real id, and use only the real ids in later `--depends` values. Never guess ids, never reuse ids across runs.
   - Fill each task file's `## Description` (at the path add-task printed, relative to the story folder) with scope and the acceptance criteria it covers. Fill `story.md` with the summary, acceptance criteria and breakdown notes (critical path, provisional tasks).
7. **Jira subtasks (one-off).** Run `capcom show <KEY>` to see which tasks need subtasks: only non-dropped tasks with no `jiraSubtask` yet. Ask the user whether to create Jira subtasks for those. If yes: find the subtask issue type with `getJiraProjectIssueTypesMetadata`, create each with `createJiraIssue` (parent `<KEY>`, summary = task title), then `capcom set-subtask <KEY> <ID> <SUBTASK-KEY>`. This is the only Jira write after step 2.
8. **Finish.** Run `capcom show <KEY>` and print the board grouped by status with dependencies, then say how to start: `/story-plan-task <KEY> T1`. If you are running inside Herdr (`HERDR_ENV` is `1` and `HERDR_WORKSPACE_ID` is set) and this workspace is labelled exactly `<KEY>` (check with `herdr workspace get "$HERDR_WORKSPACE_ID"`; that is how `capcom-tui` names the one it opens), give it a descriptive name: `herdr workspace rename "$HERDR_WORKSPACE_ID" "<KEY> - <two or three words naming the feature>"`, for example `PROJ-123 - Notification preferences`. Leave any other workspace name alone, and ignore a failure.
