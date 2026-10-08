---
name: story-review
description: Review a whole Jira story once all its tasks are done. Checks every subtask on the board is complete, then reviews the story's acceptance criteria and intent against what was built and gives feedback to the user. Use when asked to review a story or when every task is done. Argument - the Jira story key, for example PROJ-123.
---

# Story review

Argument: `<KEY>`. Board data lives in `<root>/<KEY>/` (`<root>` is the stories folder printed by `capcom root`). Valid story statuses: in_progress, in_review, done. Valid task statuses: todo, planning, planned, implementing, done, dropped.

CI fixes and review comments belong to each task (they are handled while the task is `implementing`, before it is `done`). This skill does not re-run them. It reviews the story as a whole.

## Rules

- Change the board only through `capcom`. Never edit `board.json` by hand.
- Review only. Do not change code, and write nothing to Jira.
- Do not force a story into review: the tool refuses it while any task is unfinished.
- Report evidence, not impressions: every verdict names the task, PR or file it rests on.

## Steps

1. **Check the subtasks.** Run `capcom show <KEY>`. Every task must be `done` or `dropped`. If any is not, list each one with its status (and blocked reason, if any), say what finishes it (for example `/story-implement-task <KEY> <ID>`, or `capcom refresh <KEY>` if its PRs are merged), and stop. Do not move the story.
2. **Start the review.** If `story.status` is `in_progress`, run `capcom story-status <KEY> in_review`. If it is already `in_review`, continue. If it is `done`, ask the user whether to review again, and if so run `capcom story-status <KEY> in_progress` then `in_review`.
3. **Read the story.** Fetch the Jira story (`getAccessibleAtlassianResources` for the cloud id, then `getJiraIssue`): description, acceptance criteria, linked pages. Intent matters as much as the criteria, so note what problem the story is solving. If the lookup fails or returns several sites, ask the user which to use.
4. **Read what was built.** `story.md`; every task file (its `file` field in `show`, relative to the story folder): `## Plan`, `## Decisions` and `## Progress`; and each merged PR (`prs` in `show`; use `gh pr view <url>` and `gh pr diff <url>`, or a summary of the change). Dropped tasks: note why they were dropped.
5. **Review.** For each acceptance criterion decide met, partly met or not met, and cite the evidence (task, PR, test, file). Then look for: gaps between the criteria and what shipped; work done that the story did not ask for; decisions recorded under `## Decisions` that depart from the story's intent; edge cases the story implies but no task covers; follow-ups left in `## Progress`.
6. **Feed back to the user.** Give a short report: overall verdict (ready to close, or needs work), each criterion with its verdict and evidence, concerns, and suggested follow-up tasks. Ask the user what to do.
7. **Outcome.**
   - Accepted: run `capcom story-status <KEY> done`.
   - More work needed: point the user to `/story-break-down <KEY>` to add tasks (adding a task moves the story back to `in_progress`). If they want to leave it as is for now, say the story stays `in_review`.
