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
- The code is the source of truth. Tickets, task files, PR descriptions and board notes can be out of sync with what shipped. Never base a verdict on them alone: confirm each acceptance criterion in the actual code (and its tests) on the default branch, citing `file:line`. If the code contradicts the ticket or notes, say so and trust the code. If a criterion's code is not in a repo on the board, find it (search the org's repos) rather than assuming a Done Jira subtask covers it. If you cannot locate it, mark the criterion "not verified" and say why.

## Steps

1. **Check the subtasks.** Run `capcom show <KEY>`. Every task must be `done` or `dropped`. If any is not, list each one with its status (and blocked reason, if any), say what finishes it (for example `/story-implement-task <KEY> <ID>`, or `capcom refresh <KEY>` if its PRs are merged), and stop. Do not move the story.
2. **Start the review.** If `story.status` is `in_progress`, run `capcom story-status <KEY> in_review`. If it is already `in_review`, continue. If it is `done`, ask the user whether to review again, and if so run `capcom story-status <KEY> in_progress` then `in_review`.
3. **Read the story.** Fetch the Jira story (`getAccessibleAtlassianResources` for the cloud id, then `getJiraIssue`): description, acceptance criteria, linked pages. Intent matters as much as the criteria, so note what problem the story is solving. If the lookup fails or returns several sites, ask the user which to use.
4. **Read what was built.** `story.md`; every task file (its `file` field in `show`, relative to the story folder): `## Plan`, `## Decisions` and `## Progress`; and each merged PR (`prs` in `show`; use `gh pr view <url>` and `gh pr diff <url>`, or a summary of the change). Dropped tasks: note why they were dropped.
   Then read the code. For every PR, read the full diff (`gh pr diff <url>`), not just the description or file list. For every acceptance criterion, including ones delivered outside the board (for example by Done Jira subtasks), locate the implementing code and tests in the current default branch, in each repo involved (use a local checkout or `gh api` / `gh search code`), and read them. Compare what the code does with the criterion, the task `## Plan` and `## Decisions`, and the PR description, and note every mismatch.
5. **Review.** For each acceptance criterion decide met, partly met or not met, and cite the evidence from the code (`file:line`, test name, PR), not only the ticket or notes. Then look for: gaps between the criteria and what shipped; work done that the story did not ask for; decisions recorded under `## Decisions` that depart from the story's intent; edge cases the story implies but no task covers; follow-ups left in `## Progress`; places where the ticket, task notes or PR text disagree with the code (out-of-sync claims).
6. **Feed back to the user.** Give a short report: overall verdict (ready to close, or needs work), a line saying what code you read (repos, files, PR diffs) and what you did not, each criterion with its verdict and `file:line` evidence, concerns, and suggested follow-up tasks. Ask the user what to do.
7. **Outcome.**
   - Accepted: run `capcom story-status <KEY> done`.
   - More work needed: point the user to `/story-break-down <KEY>` to add tasks (adding a task moves the story back to `in_progress`). If they want to leave it as is for now, say the story stays `in_review`.
