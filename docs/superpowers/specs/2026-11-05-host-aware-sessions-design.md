# Sessions open in the app you are using (Claude Desktop, Herdr, Orca)

## Why

You switch between three apps during the day: Claude Desktop, Herdr and Orca. capcom only knows how to
start and find sessions in two of them, and only in some of its views:

| Host | capcom-tui today | capcom web (page and `/capcom` pane) today |
|---|---|---|
| Claude Desktop | nothing (no launching, no agents panel) | new sessions through `claude://code/new?prompt=…&folder=…`; a running desktop session opens through `claude://code/continue?session=local_…` |
| Herdr | launch, resume, prompt, focus and close agents, only when `HERDR_ENV=1` (`src/bin/capcom-tui/herdr.rs`) | nothing |
| Orca | nothing | nothing |

The last time you moved a task to IMPLEMENTING you were in Claude Desktop and wanted the session to open
there. The rule you gave: **whatever app you do the moving in is where the session opens.**

## Goals

1. **Starting a session opens it in the app you are in.** This covers every action that starts a new
   Claude session: moving a task to PLANNING or IMPLEMENTING, the PR fix/address prompts, and resuming
   a recorded session that is no longer running.
2. **Clicking an agent takes you to it, wherever it runs.** An agent running in Herdr is focused in
   Herdr, one in Orca is switched to in Orca, one in Claude Desktop is opened in Claude Desktop, even
   when you clicked from a different app. *(This is Claude's proposal; you have not objected to it.)*
3. **Orca is a full host**, used both through capcom-tui in an Orca terminal and through the capcom web
   page in Orca's built-in browser.
4. **capcom-tui works in Claude Desktop's terminal panel** the way the web page does: new sessions go
   through `claude://` links.

## Non-goals

- Moving a running session from one app to another.
- Supporting agents other than Claude (Codex and others in Orca).
- Driving Orca's GUI or Herdr's layout beyond what their CLIs offer.
- Changing which skill or prompt a column move sends; only *where* it goes changes.

## Anti-goals (would count as failure even if it works)

- A session silently opening in an app you are not looking at. If capcom cannot reach the app you are
  in, it says so and starts nothing; it never falls back to another app on its own.
- Starting a second agent for a task whose agent is already running anywhere. Today's Herdr rule (focus
  or prompt the running one, never start twice) applies across all three apps.
- Breaking today's Herdr or Claude Desktop behaviour.

## How capcom tells which app you are in

**capcom-tui** decides once, at start-up, from its environment. First match wins:

| Check | Host | Source |
|---|---|---|
| `HERDR_ENV=1` | Herdr | already used by `inside_herdr()` |
| `ORCA_TERMINAL_HANDLE` set | Orca | Orca's shell wrappers in `~/.config/orca/shell-wrappers` export it, with `ORCA_WORKSPACE_ID`, `ORCA_WORKTREE_PATH` and `ORCA_CLI_BIN_DIR` |
| `CLAUDE_CODE_ENTRYPOINT=claude-desktop` | Claude Desktop | set in Claude Desktop's sessions; **not yet confirmed** for its Terminal panel |
| none | plain terminal | today's behaviour (no launching), unless the builder finds a better default |

Herdr is checked before Orca because Herdr can run inside an Orca terminal; whichever wraps the TUI most
closely wins.

**capcom web** is one long-running service (`capcom-web.service`), so it cannot use its own environment.
It decides per request:

| Request comes from | Host | How it is known |
|---|---|---|
| the `/capcom` pane in Claude Desktop | Claude Desktop | the pane's requests already carry an `x-capcom` header on actions; `/api/state` reads should carry one too |
| the page in Orca's built-in browser | Orca | to be found out: Orca's browser user agent if it is distinctive, else a `?host=orca` on the page's address that the page remembers and sends with each action |
| the page in any other browser | Claude Desktop | today's behaviour (`claude://` links) |

## How each host is driven

| Action | Claude Desktop | Herdr | Orca |
|---|---|---|---|
| Start a session with a prompt | `claude://code/new?prompt=…&folder=…` (existing) | `herdr agent start` + `agent prompt` (existing) | `orca agent launch --agent claude --surface terminal --worktree path:<folder> --prompt "…" --focus --json` |
| Resume a recorded session | `claude://resume?session=…` (existing) | `agent start … -- --resume <id>` (existing) | `orca terminal create --worktree path:<folder> --command "claude --resume <id>"` |
| Prompt a running agent | not possible: open it and leave the prompt to you | `herdr agent prompt` (existing) | `orca terminal send --terminal <handle> --text "…" --enter` |
| Bring an agent to the front | `claude://code/continue?session=local_…` (existing) | `herdr agent focus` (existing) | `orca terminal switch --terminal <handle>` |
| List running agents | `~/.claude/sessions/<pid>.json` (existing, `src/claude.rs`) | `herdr agent list` (existing) | `orca worktree ps --json` (agents carry a `paneKey` holding the Claude session id) joined to `orca terminal list --json` for the terminal handle |
| Close an agent | not offered | `herdr pane close` (existing) | `orca terminal close --terminal <handle>` |

Orca details decided so far:

- **Always `--surface terminal`.** Orca can open Claude as a chat, but no CLI command brings a chat to
  the front, so a chat agent could never be clicked to.
- **Workspace = the task's folder.** Herdr gets a workspace per story; Orca's workspaces are repo
  checkouts, so capcom uses the task's `folder` field (`capcom set-folder`) as `path:<folder>`. If Orca
  does not know that folder, capcom says so (and how to add it: `orca repo add --path …`) rather than
  adding repos itself. *(Claude's call.)*
- **Finding the CLI:** `ORCA_CLI_BIN_DIR` when set, else `~/.config/orca/linux-orca-cli-shim/orca`, else
  `orca-ide` on PATH. Never bare `orca`: on Linux that is the GNOME screen reader.

## Constraints

- `capcom-web.service` runs with `PrivateTmp=true` and `ProtectHome=read-only`. Orca's CLI shim runs a
  launcher from `/tmp/.mount_orca-…` (the AppImage mount), which a private `/tmp` hides. The web service
  needs a way to reach Orca: relax `PrivateTmp`, bind the mount in, or talk to Orca's socket
  (`~/.config/orca/*.sock`) differently. Builder's choice; the service file lives in
  `~/.config/systemd/user/capcom-web.service` and is documented in `docs/web.md`.
- Running full test suites locally is off limits; scoped tests (one file or module) are fine.
- capcom commits straight to `main`, no PR.
- Orca's CLI is 1.4.247 today and changes often; every Orca call is behind one module so a CLI change is
  one place to fix, with tests using a fake runner the way `herdr.rs` does (`Cli::with_runner`).

## Automatic prompts (autopilot, auto-fix)

These fire with no click, so there is no "app you are in".

- **Agent running in Herdr or Orca:** prompt it in place, without taking focus (as Herdr does today,
  `Resume { focus: false }`). A working agent is left alone and tried again later (`Outcome::Busy`).
- **Agent running in Claude Desktop:** open that session in Claude Desktop with the prompt waiting,
  even though this takes the screen. *(Your choice.)* Whether `claude://code/continue` can carry a
  prompt is unknown (spike item 5); if not, open a new desktop session in the same folder with the
  prompt, through `claude://code/new`.
- **No agent running:** start one in the app of the front end that fired it: the TUI's own app, or for
  capcom web the app of the most recent page or pane request, falling back to Claude Desktop. *(Claude's
  call; you did not answer this part.)*

## Calls Claude made after review (check these)

1. **Spotting a task's running agent in any app.** Match the task's recorded sessions
   (`capcom set-session`) against the live sessions from every app: `~/.claude/sessions`, `herdr agent
   list` and `orca worktree ps`. A start also leaves a short-lived "starting" mark on the task (a few
   minutes, or until its session is recorded), so a second click in that gap opens or reports the
   starting one instead of starting another. The web page gets the same check, which it lacks today.
2. **Resume folder.** Resume in the folder the session was started in (its recorded cwd, often a
   `.claude/worktrees/…` folder), as Herdr resume already does. For Orca, pass the enclosing checkout as
   `--worktree` and run `cd <session folder> && claude --resume <id>` as the command.
3. **If Claude Desktop's terminal panel cannot be detected**, a TUI there behaves like a plain terminal
   (no launching, as today) rather than guessing Claude Desktop. Goal 4 is then dropped and the docs say so.
4. **Links from Orca's browser.** The page already sends links to the server's `/open`, which runs
   `xdg-open`, so `claude://` links work from any browser. Orca starts (launch, switch) are new
   server-side actions under `/api/do`, not links.

## Unknowns to settle first (a short spike)

1. Do Claude agents started by Orca write `~/.claude/sessions/<pid>.json`, and with what `entrypoint`?
   This decides whether the existing session list can tell Orca agents apart, or whether Orca's own
   `worktree ps` is the only source.
2. What user agent does Orca's built-in browser send?
3. Does Claude Desktop's Terminal panel set `CLAUDE_CODE_ENTRYPOINT=claude-desktop` (or anything else
   capcom-tui can see)?
4. Can `herdr agent focus` be called from outside Herdr (from capcom web or an Orca terminal)? If not,
   clicking a Herdr agent from elsewhere says "it's in Herdr" instead.
5. Can `claude://code/continue?session=…` take a prompt, the way `claude://code/new` does?

## Left to the builder

- The shape of the abstraction (today's `trait Herdr` is the natural seam; renaming it to something like
  `Host` and adding Orca and Claude Desktop implementations is expected but not required).
- How the TUI's agents panel merges agents from several hosts, and how it marks which app each is in.
- Wording of notices.
- Docs: `docs/tui.md`, `docs/web.md`, `docs/reference.md` and the README describe Herdr today and need
  the other two apps added.
