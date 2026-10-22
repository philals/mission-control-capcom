# capcom web

The story boards and your open pull requests as a page in a browser, with the same look as `capcom-tui`: a kanban of stories (click one for its task columns) and the PR list with each PR's way to a merge and its CI stages.

```
capcom web                  # prints {"url": "http://127.0.0.1:7878"} and serves until stopped
capcom web --port 8080
capcom web --no-prs         # boards only, no GitHub
```

It needs the stories folder (`--root` or `CAPCOM_ROOT`) and the GitHub CLI (`gh`) signed in, like `capcom-tui` (it uses the token `gh auth token` prints). It is a separate program, so you can run it instead of `capcom-tui`, or beside it: both share one cache of GitHub answers in `~/.cache/capcom`, so two of them do not ask GitHub twice.

## What the page shows

- **Header:** a `GO` or `NO-GO · n red` light, how many PRs wait for a reviewer, and when the PRs were last read.
- **Stories:** TO DO, DOING and IN REVIEW columns (`[ Show done ]` adds DONE) with a progress bar and the task counts. Click a story for its tasks in columns (TO DO, PLANNING, PLANNED, IMPLEMENTING, DONE), each with what it is waiting for, a chip for the worst state of its PRs and links to the PRs.
- **Pull requests:** newest first. Each has the draft or ready badge, the repo and number, the story and task it belongs to, labels, how long ago it changed, the branch with a 📋 button that copies its name, and the rail (`draft · CI · copilot · threads · approved · automerge · merge`, coloured done, going, wrong or not yet). A PR that is green and waiting for a reviewer is lit green. Running, queued and failed checks are listed under it with a running clock; `show all checks` lists the rest. Inside a story, only that story's PRs are shown (`[ all PRs ]` shows every one).

## Agents

The **AGENTS** panel lists the Claude Code sessions running on this machine, from the terminal program and from the desktop app alike, each tagged `cli` or `desktop`, grouped by the folder they run in, with whether each is working or idle. It reads the file Claude Code keeps for each live session (`~/.claude/sessions/<pid>.json`), so it needs no Herdr and no setup. Those files are Claude Code's own and not documented, so a future version of Claude Code could change them; a file that does not look right is skipped.

Add `/agent-note` (see the README) and each agent also says what it was asked, where it has got to, what it waits on and what you need to do (`capcom note`, one small file per session). The note shows under the agent: `you: pick option A or B` in orange when it waits on you (the header counts those), `waiting: CI on the PR` when it waits on something else, otherwise where it has got to; click it for all of it. Below the running sessions, **EARLIER** lists the notes of sessions that have ended, for a week, newest first with how long ago: the "what was that agent doing yesterday" view. A session whose note says it needs you only shouts while the session is not working (a note from before it carried on is ignored).

A `desktop` session is clickable: the click opens it in the Claude Code desktop app, through the app's own `claude://code/continue?session=…` link (the id is the one the app keeps for the session, `hostSessionId` in the session file; the app's link handler accepts only that kind of id). A `cli` session has no such link, because the desktop app does not know it.

The page shows the panel only when there is something to show. The agents are looked at every 3 seconds while a page is looking and not at all otherwise. (`capcom-tui` lists the agents of Herdr instead; the notes are shared by both.)

## Links open in your own browser

A click on a link (a PR, its diff, a check, Jira) asks the service to open it with your system's default browser (`xdg-open`; `--browser CMD` or `CAPCOM_BROWSER` picks another command), not inside whatever shows the page. The service starts the browser through `systemd-run --user` with `KillMode=process`, so it does not run inside the service's sandbox and systemd does not cut off the browser's launcher while it hands the link to the running browser (without that, `xdg-open` appears to succeed and nothing opens). The same route opens a desktop session's `claude://` link in the desktop app. If that fails, the link opens the usual way.

## It does not change anything

Nothing on the page changes anything in capcom, GitHub or Jira. The one thing a click does is open a link. The server listens on `127.0.0.1` only, refuses any request whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>` (so a website cannot read it through your browser by rebinding a name), answers `GET`, and accepts one `POST`, `/open`. That one only opens a link the page is showing at that moment, and only when the request carries a header (`X-Capcom`) that a website cannot add without a CORS preflight this server never grants, comes from the page's own origin and is not cross-site. It sends a strict content security policy. All text from GitHub and the boards is put in the page as text, never as HTML.

## It polls only while someone looks

While no page has fetched the state for 20 seconds (a hidden browser tab pauses its own polling), GitHub is asked once every 30 minutes at most. When a page comes back it asks GitHub at once and shows the fresh answer within a second or two. With no browser open it costs next to nothing, which is what makes it fine to leave running.

## Keep it running

`contrib/capcom-web.service` is a systemd user service that starts `capcom web` at login:

```
mkdir -p ~/.config/systemd/user
cp contrib/capcom-web.service ~/.config/systemd/user/     # then edit CAPCOM_ROOT in it
systemctl --user daemon-reload && systemctl --user enable --now capcom-web
journalctl --user -u capcom-web                           # its log
```

It runs with a read-only view of your home folder (it writes only `~/.cache/capcom`), no new privileges and a private `/tmp`; it reads `~/.claude/sessions` and the notes in `~/.local/state/capcom/notes`. After you reinstall `capcom`, run `systemctl --user restart capcom-web`.

## In the Claude Code desktop app

The desktop app's Browser pane can show a local server beside a session. According to Claude Code's docs (the Desktop page, "Configure preview servers"), it reads `.claude/launch.json` from the folder the session started in, and an entry with a `url` and no command attaches to a server that is already running:

```json
{
  "version": "0.0.1",
  "configurations": [{ "name": "capcom", "url": "http://127.0.0.1:7878", "port": 7878 }]
}
```

This repository has that file in `.claude/launch.json`. Put the same file in `.claude/` of any other folder you start sessions from. The docs describe no setting that opens the pane by itself and no folder-independent config, so the pane is opened from the **Dev servers** menu or by asking Claude to preview it; the page refreshes its own data every few seconds, so it never needs reloading.
