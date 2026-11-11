---
name: story-plan
description: Plan a whole Jira story once, working through open questions with the user, then split it into PR-sized subtasks on a local kanban board (board.json) that are ready to implement. Use when asked to plan, break down, split, slice or start work on a Jira story. Argument - the Jira story key or URL, for example PROJ-123 or https://site.atlassian.net/browse/PROJ-123.
---

# Story plan

Turns a Jira story into one cohesive plan plus a small set of `planned` tasks saved under
`<root>/<KEY>/` (`<root>` is the stories folder printed by `capcom root`). The plan lives in `story.md`; the tasks
are the PR-sized slices of it. The next step for each task is `/story-implement-task <KEY> <ID>`. There is no
per-task planning.

## Rules

- An extra task id after the key (older launchers still pass one) is ignored: plan the whole story.
- Argument: a Jira key (`PROJ-123`) or a Jira URL. For a URL take the key from the `/browse/<KEY>` path (or the `selectedIssue` query parameter); use only the key from then on.
- Planning only. Do not change code in any repository.
- Change `board.json` only through the `capcom` command (run `capcom --help`). Never edit it by hand.
- Ask questions one at a time and wait. Record every answer under `## Decisions` in `story.md` so no later session asks again.
- Consult the user on every split, merge and dependency. Propose, then wait.
- A task is a PR-level chunk: independently reviewable and mergeable. One task may need several PRs and several repos, and that is fine.
- There are no stacked PRs. A task starts only after its dependencies are merged, so prefer slices that can merge independently (backward-compatible changes, flags) and call out real dependencies explicitly.
- Types: `pr` (ends in merged PRs), `spike` (ends in an answer written into the task file), `decision` (ends in a recorded decision). Spikes and decisions need no PR.
- Local files are the source of truth. Do not create Jira subtasks unless the user asks for them, and never update Jira after creating any.

## Steps

1. **Existing board?** Run `capcom show <KEY>`. If it fails and the error says the `board.json` file is missing, there is no board yet, so continue with step 2. If it fails for any other reason (for example an invalid board), stop and report the error. If it succeeds, this is a re-run: show the current board and the plan in `story.md`, propose only changes to the plan, and adds, splits, re-pointed dependencies and drops of tasks; apply only what the user approves (add new tasks with `add-task`, change dependencies with `set-deps`, mark as dropped with `status <KEY> <ID> dropped`). Update `## Plan` in `story.md` only after approval. New tasks go through steps 6 and 7 as tasks. Valid statuses: todo, planning, planned, implementing, done, dropped. If the story is `in_review` or `done` (see `story.status` in `show`), new tasks reopen it: an `in_review` story goes back to `in_progress` automatically when a task is added, and a `done` story must first be reopened with `capcom story-status <KEY> in_progress`, which you do only with the user's agreement. Never reuse ids. Never touch tasks past `planned` without asking. A task found in `todo` or `planning` (from an older board) is planned as part of this run, then moved to `planned`.
2. **Fetch the story.** Use `getAccessibleAtlassianResources` for the cloud id (if the lookup fails or returns several sites, ask the user which to use; for a URL, pick the site whose URL matches), then `getJiraIssue` for `<KEY>`. Read its description, acceptance criteria, linked issues and any linked Confluence pages (`getConfluencePage`).
3. **Research.** Find the affected repositories (ask the user where their code lives if it is not obvious from the story or the current directory) and read the code across them deeply enough to plan: how things work today, where the change goes, what could go wrong. For unknowns that cannot be settled by reading, plan a `spike` first and mark dependent tasks as provisional in their descriptions.
4. **Resolve questions** with the user, one at a time. Record each answer under `## Decisions` in `story.md` (create the section on the first answer).
5. **Draft the plan and the split.** Show the plan summary, then a table of tasks: id placeholder, title, type, depends on, repos. Note the critical path and which tasks can run in parallel. Flag anything bigger than one reviewable change. The plan has these parts (the text, not yet saved):
   - `### Affected repositories` (repo names, one per line)
   - `### Approach` (the whole story, end to end)
   - `### Files` (files to create or change, per repo)
   - `### Tests` (what to test and how, using targeted runs; CI covers the full suites)
   - `### Tasks` (per task: the slice of the approach it covers, its expected PRs per repo in merge order, and the acceptance criteria it satisfies)
   - `### Risks and open items`
6. **Iterate with the user** until they approve the plan and the task table together. Ask one question at a time.
7. **Write it down.** Only after approval.
   - `capcom init <KEY> --title "<story title>" --jira-url <site URL>/browse/<KEY>` (the site URL comes from the lookup in step 2). Skip on a re-run.
   - `capcom add-task <KEY> --title "..." --type pr|spike|decision [--depends T1,T2] [--repos a,b]` for each task (list a repo once per expected PR: `--repos api,api,ui` means two PRs in `api` and one in `ui`, and the task is not done until all exist and are merged), in dependency order. Each call prints the new id and file path. Record both `{id,file}` from every add-task output, map each placeholder id in the approved table to its real id, and use only the real ids in later `--depends` values. Never guess ids, never reuse ids across runs.
   - For every task that touches code (`pr`, and any spike or decision that reads a repo), run `capcom set-folder <KEY> <ID> <absolute path>` with the local checkout of the task's main repo (find it in step 3, or ask the user). This is required, not optional: Herdr opens the task's workspace and tabs in that folder, so they land in the repo rather than the default work folder. For a task spanning several repos, use the folder that holds them or the main repo. Leave it unset only for tasks with no code. Confirm every such task shows a `folder` in `capcom show <KEY>` before finishing.
   - Write `story.md`: the summary, the acceptance criteria, `## Decisions` and `## Plan` (the parts from step 5, plus the critical path and provisional tasks).
   - Fill each task file's `## Description` (at the path add-task printed, relative to the story folder) with the task's scope, the acceptance criteria it covers and a pointer to its entry under `### Tasks` in the story plan. Do not copy the plan into the task file.
   - Move each new task to `planned`: `capcom status <KEY> <ID> planning` then `capcom status <KEY> <ID> planned` (there is no direct `todo → planned`).
8. **Jira subtasks (only on request).** Do not ask about them and do not create them by default; skip this step unless the user has asked for subtasks. If they have: run `capcom show <KEY>` to see which tasks need subtasks (only non-dropped tasks with no `jiraSubtask` yet), find the subtask issue type with `getJiraProjectIssueTypesMetadata`, create each with `createJiraIssue` (parent `<KEY>`, summary = task title), then `capcom set-subtask <KEY> <ID> <SUBTASK-KEY>`. This is the only Jira write after step 2.
9. **Finish.** Run `capcom show <KEY>` and `capcom ready <KEY>`, print the board grouped by status with dependencies, then say how to start: `/story-implement-task <KEY> <ID>` for each ready task. If you are running inside the Herdr desktop app (`HC_TERMINAL_ID` is set; `HERDR_ENV` is not used and the bare `herdr` CLI is a different session) and this space is named exactly `<KEY>` (find your space with `herdr-app ls`: the space whose terminals include `$HC_TERMINAL_ID`; that is how `capcom-tui` names the one it opens), give it a descriptive name: `herdr-app do space.rename --space <space id> --name "<KEY> - <two or three words naming the feature>"` (check the argument names with `herdr-app commands`), for example `PROJ-123 - Notification preferences`. Leave any other space name alone, and ignore a failure.
