---
name: story-implement-task
description: Implement one planned task from a story board and open draft PRs, working through questions with the user. Use when asked to implement or build a task. Arguments - story key and task id, for example PROJ-123 T2. Also supports direct mode: a Jira key or URL with no board, for small bugs and changes.
---

# Implement a task

Arguments: `<KEY> <ID>`. Board data lives in `<root>/<KEY>/` (`<root>` is the stories folder printed by `capcom root`). Valid task statuses: todo, planning, planned, implementing, done, dropped. There is no per-task review status: CI fixes and review comments are handled inside the task while it stays `implementing`; the story is reviewed as a whole later with `/story-review`.

## Direct mode (no board)

Use when the argument is a Jira key or URL (for example `PROJ-123` or `https://.../browse/PROJ-123`) with no task id, or the user says there is no board. Check quickly with `grep -rl <KEY> "$(capcom root | jq -r .root)"`; if no board mentions it, go direct without asking. Intended for small bugs and changes, so keep the process light:

- Skip all `capcom` commands (`show`, `ready`, `status`, `set-session`, `add-pr`, `refresh`, `block`) and the task file. No `## Plan`, `## Decisions` or `## Progress` to maintain.
- Read the Jira ticket (Atlassian MCP) for the problem and acceptance criteria. Agree a short approach with the user in a few lines in chat, then implement, test first where it fits.
- Work where the session already is: if the current checkout is already a worktree on a non-default branch, use it as is. Otherwise follow steps 4-5 (worktree, `fix/<KEY>-<slug>` or `feat/<KEY>-<slug>` branch from the up-to-date default branch). Same exceptions apply for `flower-power-dd` and `capcom`.
- Open the PR as in step 7: draft, title `fix: <KEY> <short description>` (or `feat:`; no task id suffix), Copilot review requested, no auto-merge. Skip the `capcom add-pr` call.
- Finish: report the PR url, tell the user to mark it ready and merge it. Worktree cleanup (step 10) is by hand: after the PR merges, offer to remove the worktree only if the user asks, using the same safety checks (merged, clean tree, no `--force`).
- All other Rules (draft PRs only, no stacked PRs, targeted local checks only, ask one question at a time) still apply.

## Rules

- Change `board.json` only through `capcom`. Never edit it by hand.
- Every PR is a **draft**. The user marks PRs ready in GitHub. Never mark ready, never merge, never enable auto-merge.
- No stacked PRs. Branch from the up-to-date default branch of each repository.
- Run targeted checks locally only (a single test file or package, lint, typecheck, build). Do not run a full local test suite; push the draft PR and let CI report.
- Ask the user when something is unclear, one question at a time, and record answers under `## Decisions` in the task file (decisions made while planning are in `story.md`).
- Default to a git worktree for every repo without asking. Work in the main checkout only for `flower-power-dd` and `capcom` (they push straight to the default branch, no PR), or if the user explicitly asks for the main repo (step 4).
- If the plan turns out wrong, stop, agree the change with the user, and update `## Plan` in `story.md` before continuing (note the change under `## Progress` in the task file). If the plan must be redone from scratch, run `capcom status <KEY> <ID> planned` (legal from `implementing`) and point the user to `/story-plan <KEY>`.
- Never leave the board in `implementing` after an abandoned run without saying how it stands: see the stop rule in step 8.
- Remove only worktrees recorded under `## Progress`, only after their PRs are merged and the tree is clean, and never with `--force` (step 10).

## Steps

1. **Gate.** **Clean up:** if the task is already `done`, or the user asks to tidy up after its PRs merged, skip to step 10. Otherwise run `capcom show <KEY>` and `capcom ready <KEY>`. `ready` lists every non-finished task whose dependencies are `done`, including tasks already `implementing`, so require BOTH that the task is listed by `ready` AND that its own status (from `show`) is `planned`. A task still in `todo` or `planning` (from an older board) has not been planned as part of the story: point the user to `/story-plan <KEY>`. Otherwise explain exactly what is missing (status already `implementing`, waiting on which dependencies, blocked and why) and ask the user how to proceed. Do not force it. **Resume:** if the task is already `implementing` and the user confirms resuming it (or wants more work after review, for example CI failures or review comments on its PRs), skip step 2, run `capcom unblock <KEY> <ID>` if the task is blocked, read `## Progress` and reuse the worktrees and branches recorded there (the step 4 choice stands), and open only new PRs, recording each with `add-pr`.
2. **Start.** `capcom status <KEY> <ID> implementing` (legal from `planned`; capcom refuses if its dependencies are not all `done` or the task is blocked). If it refuses, report the message and stop. Then record this conversation so the board can resume it from the task's PRs: `capcom set-session <KEY> <ID> --skill story-implement-task` (it reads `$CLAUDE_CODE_SESSION_ID`; if that is not set, skip this quietly). Do this on a resume too.
3. **Read.** `story.md` (the story's `## Plan`, especially this task's entry under `### Tasks`, and `## Decisions`), the task file (`## Description`, `## Decisions`, `## Progress`), and the task files of its dependencies for what they recorded while being built. The task file path is the task's `file` field in `capcom show <KEY>`, relative to `<root>/<KEY>/`. The repositories to work in are the task's `repos` field (a subset of the plan's `### Affected repositories`).
4. **Choose where to work.** Do not ask: use a git worktree for every affected repo, except `flower-power-dd` and `capcom`, which use the main checkout (commit and push straight to the default branch, no PR) unless the user says otherwise. Record the choice and each worktree path under `## Progress` in the task file. Skip steps 4 and 5 for a `spike` or `decision` that touches no repository.
   - **Main repo:** confirm a clean working tree, switch to the default branch and pull.
   - **Worktree:** fetch, then create a worktree from the up-to-date default branch using the `superpowers:using-git-worktrees` skill, leaving the main checkout untouched.
5. **Branch.** In each repo (or its worktree) create a branch named `feat/<TICKET>-<short-slug>` (use `fix/` for bug fixes) from the up-to-date default branch, where `TICKET` is the task's `jiraSubtask` field, or else the story key. Do all later edits, commits and targeted checks inside that checkout.
6. **Implement** following the plan, test first where it fits. Note progress, commits and surprises under `## Progress` in the task file. For a `spike` or `decision`, write the findings or decision into the task file instead of code.
7. **Open each PR as a draft.** Skip this step for a `spike` or `decision` (no PR is created). Commit, push and open the PR with your usual commit-and-push helper if you have one (a skill or agent), otherwise with `git` and `gh` directly: use the branch from step 5 (do not create another), use `TICKET` = the task's `jiraSubtask` field (or the story key if none), create the PR as a **draft** (`gh pr create --draft`), title format `feat: TICKET <short description> (<ID>)` (substitute the real key for TICKET and the task id for `<ID>`, for example `feat: PROJ-124 add notice endpoint (T1)`), and skip auto-merge and labelling. Request a Copilot review: `gh pr edit <pr-url> --add-reviewer @copilot` (skip quietly if it errors, for example when Copilot is not enabled for the repo). Then record it: `capcom add-pr <KEY> <ID> --repo <repo> --url <pr-url> --state draft`. PR states are draft, ready, merged, closed; a new PR is always `draft` (the default).
8. **Finish.**
   - For a `pr` task: leave the status as `implementing`. The task stays `implementing` through every round of CI fixes and review comments: watch CI and review comments with whatever you use for that (for example a looping skill), or resume this skill until they are green and approved. Tell the user to mark the PRs ready in GitHub and merge them, then run `capcom refresh <KEY>` (it needs `gh` authenticated; if it reports errors, tell the user the board may be stale). It moves the task to `done` once every PR is merged (for a multi-repo task, all of them); PR states on the board change only when `refresh` runs.
   - For a `spike` or `decision` with no PRs: run `capcom status <KEY> <ID> done` once the user accepts the result (the only case where you move `implementing` to `done` yourself).
   - If the user stops or you cannot continue: if work is worth keeping, run `capcom block <KEY> <ID> --reason "<why>"` (status stays `implementing` and the board shows it) and write where things stand under `## Progress` in the task file; if no work is worth keeping, run `capcom status <KEY> <ID> planned`. Say which you did.
9. **Report** the PR urls, anything left for the user, and what is now unblocked (`capcom ready <KEY>`). If the task is `done`, offer step 10. When every task on the board is `done` (or dropped), tell the user to run `/story-review <KEY>`.
10. **Clean up worktrees.** Run when the task is `done`, or when the user asks. Nothing else removes worktrees, so offer it whenever a task reaches `done`. If step 4 chose the main repo, there is no worktree: just offer to delete the merged local branch. Otherwise, for each worktree path recorded under `## Progress` in the task file (never any other worktree, and do not assume where worktrees live; confirm each path appears in `git worktree list` run in its repo):
    - **PR merged?** Check the task's PR for that repo is `merged` (`capcom show <KEY>`, refreshed with `capcom refresh <KEY>` if unsure). If not, leave the worktree in place and say why.
    - **Clean tree?** `git -C <path> status --porcelain -uall` must print nothing. If it prints files, show them and ask the user whether to commit them, move them, or delete them. Never use `--force` on your own.
    - **Remove.** From the main repo (not from inside the worktree): `git worktree remove <path>`, then `git worktree prune`, then `git branch -d <branch>`. If `-d` refuses because the PR was squash-merged, confirm with the user before using `-D`. Leave remote branches alone, and tell the user if one remains.
    - **Record.** Note under `## Progress` which worktrees were removed and which were left, and why.
    Worktrees from an abandoned run (the task reset to `planned`) may hold unmerged work: list them for the user and leave them in place.
