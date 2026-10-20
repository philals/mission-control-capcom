---
name: agent-note
description: Keep a short note about what this session is doing, so the user can see at a glance in capcom-tui's agents panel what you were asked, how far you got, what you are waiting on and what they need to do. Use at the start of any task that may run for a while or be left alone, whenever you stop to wait for the user, hit a blocker, finish, or start a side quest. Also use when asked to leave a note, a status or a TLDR.
---

# Keep a note for the agents panel

The user runs many agents and comes back to them hours or days later, forgetting what each was for. `capcom note` saves a few lines about this session. capcom-tui shows them under the agent in its agents panel, and keeps them after the agent has ended. It costs one shell call, so update it when something changes, not on a timer.

```
capcom note --task "<what you were asked>" --progress "<where you are>" \
            --waiting "<what you are waiting on>" --needs-you true|false --next "<what the user must do>"
```

The session is found from `$CLAUDE_CODE_SESSION_ID`; no other setup is needed. Leave an option out to keep what the note says now; pass an empty text (`--waiting ""`) to clear it. Each field is one line (400 characters at most), so write for someone who has forgotten everything.

## When to update

- **At the start**: `--task` in the user's words, shortened, plus a first `--progress`. For a side quest or a sub-agent you start, say that it is one and what for. When you start a sub-agent, tell it to use this skill too.
- **When you learn or decide something that changes the picture**: `--progress` only.
- **Whenever you stop to wait for the user** (a question, an approval, a decision, a manual step): `--waiting` with what for, `--needs-you true`, and `--next` with the one thing they should do, for example "reply A or B", "approve the plan in T2", "run the migration by hand".
- **When you are waiting on something that is not the user** (CI, a review, another agent, a deploy): `--waiting "CI on PR 123"`, `--needs-you false`, `--next ""`.
- **When the user has answered and you carry on**: `--waiting "" --needs-you false --next ""`.
- **When you finish**: `--progress "done: <the outcome in a line>"`, and clear the waiting fields. If the user must do something afterwards, say it in `--next` with `--needs-you true`.

## Good and bad

- Bad: `--progress "working on it"`, `--next "see above"`.
- Good: `--progress "found the retry: handler re-queues on any 5xx; fix drafted, tests not run"`, `--next "say whether to also cap retries at 3"`.

Keep credentials and personal details out of a note: it is a plain file in `~/.local/state/capcom/notes`.
