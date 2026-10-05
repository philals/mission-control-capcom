---
name: story-implement-task
description: Implement one planned task from a story board and open draft PRs, working through questions with the user. Use when asked to implement or build a task. Arguments - story key and task id, for example PROJ-123 T2.
---

# Implement a task

Arguments: `<KEY> <ID>`. Board data lives in `<root>/<KEY>/` (`<root>` is the stories folder printed by `storyboard root`). Valid task statuses: todo, planning, planned, implementing, done, dropped. There is no per-task review status: CI fixes and review comments are handled inside the task while it stays `implementing`; the story is reviewed as a whole later with `/story-review`.

## Rules

- Change `board.json` only through `storyboard`. Never edit it by hand.
- Every PR is a **draft**. The user marks PRs ready in GitHub. Never mark ready, never merge, never enable auto-merge.
- No stacked PRs. Branch from the up-to-date default branch of each repository.
- Run targeted checks locally only (a single test file or package, lint, typecheck, build). Do not run a full local test suite; push the draft PR and let CI report.
- Ask the user when something is unclear, one question at a time, and record answers under `## Decisions` in the task file.
- Always ask whether to work in the main repo or a worktree before touching any code (step 4).
- If the plan turns out wrong, stop, agree the change with the user, and update the task file's `## Plan` before continuing. If the plan must be redone from scratch, run `storyboard status <KEY> <ID> planned` (legal from `implementing`) and point the user to `/story-plan-task <KEY> <ID>`.
- Never leave the board in `implementing` after an abandoned run without saying how it stands: see the stop rule in step 8.
- Remove only worktrees recorded under `## Progress`, only after their PRs are merged and the tree is clean, and never with `--force` (step 10).

## Steps

1. **Gate.** **Clean up:** if the task is already `done`, or the user asks to tidy up after its PRs merged, skip to step 10. Otherwise run `storyboard show <KEY>` and `storyboard ready <KEY>`. `ready` lists every non-finished task whose dependencies are `done`, including tasks already `implementing`, so require BOTH that the task is listed by `ready` AND that its own status (from `show`) is `planned`. Otherwise explain exactly what is missing (not planned, status already `implementing`, waiting on which dependencies, blocked and why) and ask the user how to proceed. Do not force it. **Resume:** if the task is already `implementing` and the user confirms resuming it (or wants more work after review, for example CI failures or review comments on its PRs), skip step 2, run `storyboard unblock <KEY> <ID>` if the task is blocked, read `## Progress` and reuse the worktrees and branches recorded there (the step 4 choice stands), and open only new PRs, recording each with `add-pr`.
2. **Start.** `storyboard status <KEY> <ID> implementing` (legal from `planned`; storyboard refuses if its dependencies are not all `done` or the task is blocked). If it refuses, report the message and stop.
3. **Read.** The task file's `## Plan` and `## Decisions`, `story.md`, and the plans of its dependencies. The task file path is the task's `file` field in `storyboard show <KEY>`, relative to `<root>/<KEY>/`. The repositories to work in are the task's `repos` field (the plan's `### Affected repositories`).
4. **Choose where to work.** Ask the user whether to implement in the main repo checkout or in a git worktree (one question for all affected repos; accept a per-repo answer if they give one). Record the choice and each worktree path under `## Progress` in the task file. Skip steps 4 and 5 for a `spike` or `decision` that touches no repository.
   - **Main repo:** confirm a clean working tree, switch to the default branch and pull.
   - **Worktree:** fetch, then create a worktree from the up-to-date default branch using the `superpowers:using-git-worktrees` skill, leaving the main checkout untouched.
5. **Branch.** In each repo (or its worktree) create a branch named `feat/<TICKET>-<short-slug>` (use `fix/` for bug fixes) from the up-to-date default branch, where `TICKET` is the task's `jiraSubtask` field, or else the story key. Do all later edits, commits and targeted checks inside that checkout.
6. **Implement** following the plan, test first where it fits. Note progress, commits and surprises under `## Progress` in the task file. For a `spike` or `decision`, write the findings or decision into the task file instead of code.
7. **Open each PR as a draft.** Skip this step for a `spike` or `decision` (no PR is created). Delegate to the `git-commit-push` agent. Tell it: use the branch from step 5 (do not create its own), use `TICKET` = the task's `jiraSubtask` field (or the story key if none), create the PR as a **draft** (`gh pr create --draft`), title format `feat: TICKET <short description> (<ID>)` (substitute the real key for TICKET and the task id for `<ID>`, for example `feat: PROJ-124 add notice endpoint (T1)`), and skip auto-merge and labelling. Then record it: `storyboard add-pr <KEY> <ID> --repo <repo> --url <pr-url> --state draft`. PR states are draft, ready, merged, closed; a new PR is always `draft` (the default).
8. **Finish.**
   - For a `pr` task: leave the status as `implementing`. The task stays `implementing` through every round of CI fixes and review comments: use the `pr-looper` skill on its PRs (or resume this skill) until they are green and approved. Tell the user to mark the PRs ready in GitHub and merge them, then run `storyboard refresh <KEY>` (it needs `gh` authenticated; if it reports errors, tell the user the board may be stale). It moves the task to `done` once every PR is merged (for a multi-repo task, all of them); PR states on the board change only when `refresh` runs.
   - For a `spike` or `decision` with no PRs: run `storyboard status <KEY> <ID> done` once the user accepts the result (the only case where you move `implementing` to `done` yourself).
   - If the user stops or you cannot continue: if work is worth keeping, run `storyboard block <KEY> <ID> --reason "<why>"` (status stays `implementing` and the board shows it) and write where things stand under `## Progress` in the task file; if no work is worth keeping, run `storyboard status <KEY> <ID> planned`. Say which you did.
9. **Report** the PR urls, anything left for the user, and what is now unblocked (`storyboard ready <KEY>`). If the task is `done`, offer step 10. When every task on the board is `done` (or dropped), tell the user to run `/story-review <KEY>`.
10. **Clean up worktrees.** Run when the task is `done`, or when the user asks. Nothing else removes worktrees, so offer it whenever a task reaches `done`. If step 4 chose the main repo, there is no worktree: just offer to delete the merged local branch. Otherwise, for each worktree path recorded under `## Progress` in the task file (never any other worktree, and do not assume where worktrees live; confirm each path appears in `git worktree list` run in its repo):
    - **PR merged?** Check the task's PR for that repo is `merged` (`storyboard show <KEY>`, refreshed with `storyboard refresh <KEY>` if unsure). If not, leave the worktree in place and say why.
    - **Clean tree?** `git -C <path> status --porcelain -uall` must print nothing. If it prints files, show them and ask the user whether to commit them, move them, or delete them. Never use `--force` on your own.
    - **Remove.** From the main repo (not from inside the worktree): `git worktree remove <path>`, then `git worktree prune`, then `git branch -d <branch>`. If `-d` refuses because the PR was squash-merged, confirm with the user before using `-D`. Leave remote branches alone, and tell the user if one remains.
    - **Record.** Note under `## Progress` which worktrees were removed and which were left, and why.
    Worktrees from an abandoned run (the task reset to `planned`) may hold unmerged work: list them for the user and leave them in place.
