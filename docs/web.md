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

## It only reads

Nothing on the page changes anything: PRs, checks and Jira link out, and that is all. The server listens on `127.0.0.1` only, answers `GET` only, refuses any request whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>` (so a website cannot read it through your browser by rebinding a name), and sends a strict content security policy. All text from GitHub and the boards is put in the page as text, never as HTML.

## It polls only while someone looks

While no page has fetched the state for 20 seconds, the poll slows down to a quarter of its pace (at least a minute). When a page comes back it asks GitHub at once. A hidden browser tab pauses its own polling. With no browser open it costs next to nothing.

## In the Claude Code desktop app

The desktop app's Browser pane can show a local server beside a session. According to Claude Code's docs (the Desktop page, "Configure preview servers"), it reads `.claude/launch.json` from the folder the session started in:

```json
{
  "version": "0.0.1",
  "configurations": [
    { "name": "capcom", "runtimeExecutable": "capcom", "runtimeArgs": ["web", "--port", "7878"], "port": 7878 }
  ]
}
```

with `CAPCOM_ROOT` set in your environment (or add `"--root", "/path/to/stories"` to the arguments). The docs say nothing about the pane reloading by itself; the page refreshes its own data every few seconds, so it does not need to.
