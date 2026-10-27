# capcom web

The story boards, your open pull requests, your workflow runs and your Claude Code sessions as a page in a browser. It does what `capcom-tui` does, with two differences: it talks to Claude Code (the terminal program and the desktop app) where the TUI talks to Herdr, and it can run all the time as a service, polling GitHub only while a page is looking.

```
capcom web                  # port 1969, the year of Apollo 11; prints {"url": "http://127.0.0.1:1969"} and serves until stopped
capcom web --port 8080
capcom web --workdir ~/code # where new Claude Code sessions started from the page begin
capcom web --no-prs --no-runs
```

It needs the stories folder (`--root` or `CAPCOM_ROOT`) and the GitHub CLI (`gh`) signed in, like `capcom-tui` (it uses the token `gh auth token` prints). It shares with the TUI the settings file (`~/.config/capcom/tui.json`: the per-PR ticks, the defaults, the PRs and runs you follow), the cache of GitHub answers and the learned check durations, so you can run either, or both.

## The look, and every size

It keeps the TUI's 1960s mission control look, made light for a browser: hairline steel frames with corner brackets, a blue rule under a small-caps heading for each console, backlit lamps (the GO / NO-GO lights, the rail of each PR, the ticks, which are lamp switches), segmented gauges, a clock in the top rail, the 1960s go/no-go poll while GitHub's first answer is awaited, and the log on green-bar teletype paper. Frames take as little room as they can, so the content gets the window. Type is a condensed grotesque for labels (Eurostile, Bahnschrift or a narrow sans, whatever the machine has) and a monospace for data; no fonts are downloaded.

It lays itself out for the window: at 1900 px and wider, four consoles side by side (agents, stories, pull requests, runs); from 1300 px, three; from 1000 px, two, with the agents under the stories; below that, one console at a time, chosen on the switch bar at the bottom (stories, PRs, runs, agents, each with a lamp that lights for what needs you), with larger touch targets and dialogs that rise from the bottom on a phone. On a screen that is wide and tall enough each console scrolls by itself and keeps its place when the data refreshes.

**Resizing:** on those wide layouts a thin handle sits in each gap between consoles (it lights up when you point at it). Drag it to move the edge between the two consoles beside it, or focus it with the keyboard and use the arrow keys (`Shift` for bigger steps). Double-click a handle, press `Home` on it, or use "Reset the layout" in the palette to go back to the default. Each layout (how many columns there is room for, and which consoles are showing) remembers its own sizes in this browser.

## Plan usage and the working week

The top rail has three gauges: **5H** and **7D** are how much of your Claude plan's 5-hour and weekly limits is used (with when each starts again), and **WEEK** is how far through the working week it is (Monday to Friday, 06:00 to 17:00, local time; 100% from Friday 17:00 through the weekend). The usage ones turn yellow from 60% and red from 85%, and dim when the figures are old.

Claude Code gives the plan usage to a session's status line command (`rate_limits.five_hour.used_percentage`, `rate_limits.seven_day.used_percentage` and their `resets_at`, documented under the status line), and only while a session is running; nothing else on this machine has it. So the page reads a small file that your status line script writes. Add this to your status line script, after it has read its JSON input into `$input`:

```bash
usage_file="${CAPCOM_USAGE_FILE:-${XDG_STATE_HOME:-$HOME/.local/state}/capcom/usage.json}"
mkdir -p "$(dirname "$usage_file")" &&
    echo "$input" | jq -c '{at: (now | floor), rate_limits: .rate_limits}' > "$usage_file.tmp" &&
    mv "$usage_file.tmp" "$usage_file"
```

(Several sessions each write it; they all see the same account, so the last one wins and that is fine.) The figures are as old as the last time any session's status line ran, and the gauges say so; a window whose reset time has passed is not shown, since its figure belongs to the last window. The status line only runs in the terminal, so the CAPCOM pane mod (below) writes the same file from every session it runs in, desktop ones included. They exist for Pro and Max plans; with no usage in the file, only the WEEK gauge shows.

## What the page shows

- **Header:** a `GO` or `NO-GO · n red` light (click it to jump to the first red PR), how many PRs wait for a reviewer, how many agents need you, when the PRs were last read, and the buttons `+ follow`, `+ new story`, `log`, `⚙`, `⌘` (the palette) and `?`.
- **Agents:** the Claude Code sessions running on this machine, from the terminal program and from the desktop app, with their notes and an EARLIER list of sessions that ended (below).
- **Stories:** a list grouped into DOING, IN REVIEW and TO DO (`[ Show done ]` adds DONE), each with its progress bar. Click a story, or its `⋯`, or right-click it. Inside a story the tasks are in columns (TO DO, PLANNING, PLANNED, IMPLEMENTING, DONE) with what each waits for, a chip for the worst state of its PRs and links to them. An implementing task whose PRs are all merged says so and offers `finish ✓`.
- **Pull requests:** newest first, like the TUI. Each shows its badge, the story and task it belongs to, labels, review and comments, the branch with a 📋 button, and the **rail**: `ready · CI · copilot · threads · approved · automerge · merge`, coloured done, going, wrong or not yet, with `CI 4m left` while checks run (from how long they usually take). Running, queued and failed checks are listed with a running clock (`running 2m 05s of ~4m`). A PR that is green and waiting for a reviewer is lit up. A PR with nothing to do takes one line; click it to open it. The `details` sheet has every check grouped, the merge state, Copilot, automerge and what is in the way.
- **Runs & watch:** your manual workflow runs and the ones you follow, with their stages, in the repos of your open PRs and recorded PRs (plus `--deploy-repos`).

## What the page can change

Every change shows what it will do and waits for your click, except the cheap, reversible ones:

| Where | What | Asks first |
|---|---|---|
| a PR's badge, `mark ready…` on a draft, its `DRAFT` lamp, `m` | mark it ready for review, or back to a draft (`gh pr ready`) | yes |
| `[ ] automerge` | adds the merge bot's first label, or takes every one off | yes |
| `[ ] auto-fix`, `[ ] auto-review` | per-PR ticks, kept in the shared settings file | no |
| `+ follow`, or paste a link | follow someone else's PR (it joins the list marked *followed*; auto-fix and auto-review start off), or watch a run (you are told when it finishes) | no |
| a story's `⋯` or right-click | mark done (or accept a review), reopen, sync PR states with GitHub | yes |
| a task | finish it (its PRs are looked up first; it only finishes when they are all merged) | yes |
| `⚙` | the defaults, auto-sync, notifications, slow polling | no |

Changes to the boards go through the same code as `capcom` and the TUI, so the same rules apply (a story only moves to done when every task is done or dropped, and so on).

## Starting skills

The page cannot type into a running agent, but it can open a **new Claude Code desktop session with the command already typed in**, in the right folder (Claude Desktop's own `claude://code/new?prompt=…&folder=…` link). You press Enter. That is how a story is broken down (`+ new story`, or a story with no tasks), a task is planned or implemented (`plan ↗`, `implement ↗`), a finished story is reviewed, and a PR is fixed (`fix it ↗`, which puts `/pr-address <url>` on the clipboard and opens the session that made the PR when it is open in the desktop app, so you paste it in; with no such session it opens a new one with the command typed in, in the folder the PR's own session began in, else the repo's folder under the work folder). Nothing starts without you.

## What the autopilot does by itself

For the PRs you ticked (and the defaults in `⚙`), whenever the PR list changes:

- **auto-review:** asks Copilot to review a new PR, once.
- **auto-fix, a branch that is only behind its base:** brings it up to date through GitHub (what the "Update branch" button does), with no agent; not while Actions run, not on a PR whose merge bot keeps it up to date, at most once every 15 minutes, and a conflict is left for you.
- **auto-fix, anything else** (failing checks, Copilot's comments, a merge conflict): marks the PR with a banner and `fix it ↗`, and lists it under `log`. The TUI messages the agent through Herdr; Claude Code has no supported way for another program to type into a running session, so here the last step is yours.

It also tells you (under `log`, and as a desktop notification when `⚙ Desktop notifications` is on) when a PR turns green and waits for a reviewer, when Copilot finishes a review, when a PR needs fixing, and when a run you follow finishes. What it did is kept in the `log`.

## Agents

The **AGENTS** panel lists the Claude Code sessions running on this machine, from the terminal program and from the desktop app alike: the **desktop** sessions first, then the **terminal** ones, each tagged `cli` or `desktop` and grouped by the folder they run in, with whether each is working or idle. It reads the file Claude Code keeps for each live session (`~/.claude/sessions/<pid>.json`), so it needs no Herdr and no setup. Those files are Claude Code's own and not documented, so a future version of Claude Code could change them; a file that does not look right is skipped. A desktop session that is not working shows ● (bold) when Claude has said something you have not opened yet, or ○ once you have, as the app's own sidebar has it: capcom web reads the app's unread list from its local storage (`~/.config/Claude/Local Storage/leveldb`, the `epitaxy-unread-v1` key), which is also the app's own and undocumented. When it cannot find that list, or for a terminal session, it says idle as before.

**Subagents:** the subagents a session started with its Agent tool sit under it, indented (running ones, and finished ones for half an hour, newest first). Click one for a sheet with its conversation: what it was asked, what it said, each tool call and a clip of what came back, and its final report; it follows along while the subagent runs. They come from `~/.claude/projects/<folder>/<session>/subagents/`, Claude Code's own files, so the same caveat applies. Subagents are not sessions of their own, so the desktop app cannot open them; the sheet offers to open the session that started it.

Add `/agent-note` (see the README) and each agent also says what it was asked, where it has got to, what it waits on and what you need to do (`capcom note`, one small file per session). The note shows under the agent: `you: pick option A or B` in orange when it waits on you (the header counts those), `waiting: CI on the PR` when it waits on something else, otherwise where it has got to; click it for all of it. Below the running sessions, **EARLIER** lists the notes of sessions that have ended, for a week, newest first with how long ago. A session whose note says it needs you only shouts while the session is not working.

A `desktop` session is clickable: the click opens it in the Claude Code desktop app, through the app's own `claude://code/continue?session=…` link (the id is the one the app keeps for the session, `hostSessionId` in the session file). The 🤖 on a PR opens the session that made it: the same link when that session is running in the desktop app, else `claude://resume?session=…`, which has the app import the terminal session. The app takes a second or two to come forward (the system starts a whole second copy of it that hands the link over, about 1.9 s whatever the flags); the row says `opening…` meanwhile, and the same link clicked again within 4 seconds is ignored so clicks do not pile up launches. A `cli` session has no link of its own, because the desktop app does not know it. (`capcom-tui` lists the agents of Herdr instead; the notes are shared by both.)

## Keys

`j`/`k` or `↓`/`↑` select a PR, `Enter` its details, `o` open its diff, `c` copy its link, `b` copy its branch, `m` ready or draft, `t` / `v` / `g` auto-fix / auto-review / automerge, `f` fix it, `a` go to the session that made it, `u` stop following it, `r` look at GitHub now, `n` new story, `F` follow, `d` show done stories, `Ctrl-K` or `/` the palette (every action that fits where you are, narrowed as you type), `?` help, `Esc` closes a dialog or goes back to the stories. Right-click a PR or a story for its actions. Paste a link to a PR or a run anywhere on the page to follow it.

## Links open in your own browser

A click on a link (a PR, its diff, a check, Jira) asks the service to open it with your system's default browser (`xdg-open`; `--browser CMD` or `CAPCOM_BROWSER` picks another command), not inside whatever shows the page. The service starts the browser through `systemd-run --user` with `KillMode=process`, so it does not run inside the service's sandbox and systemd does not cut off the browser's launcher while it hands the link to the running browser (without that, `xdg-open` appears to succeed and nothing opens). The same route opens the `claude://` links.

## How it is kept safe

The server listens on `127.0.0.1` only, refuses any request whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>` (so a website cannot reach it by rebinding a name), and sends a strict content security policy. All text from GitHub and the boards is put in the page as text, never as HTML. It answers `GET`, and two kinds of `POST`, each only from the page itself: they need a header (`X-Capcom`) that a website cannot add without a CORS preflight this server never grants, the page's own origin, and no cross-site marker.

- `POST /open` only opens a link the page is showing at that moment (a link in a PR's title is not on the list).
- `POST /api/do` only does the actions in the table above, checks each against what the service knows (the PR must be in the list, the story must exist, the transition must be allowed), and refuses the ones that need confirming unless the request says it was confirmed.

## It polls only while someone looks

While no page has fetched the state for 20 seconds (a hidden browser tab pauses its own polling), GitHub is asked once every 30 minutes at most. When a page comes back it asks GitHub at once and shows the fresh answer within a second or two. With no browser open it costs next to nothing, which is what makes it fine to leave running. The cost of that is that nothing can notice a change while nobody looks: with `⚙ Desktop notifications` on, the service also polls every couple of minutes when nobody does, so a PR that turns green or a run that ends can still tell you.

## Keep it running

`contrib/capcom-web.service` is a systemd user service that starts `capcom web` at login:

```
mkdir -p ~/.config/systemd/user
cp contrib/capcom-web.service ~/.config/systemd/user/     # then edit CAPCOM_ROOT, CAPCOM_WORKDIR and the stories folder in it
systemctl --user daemon-reload && systemctl --user enable --now capcom-web
journalctl --user -u capcom-web                           # its log
```

It runs with a read-only view of your home folder, writing only `~/.cache/capcom`, the settings in `~/.config/capcom` and your stories folder (everything the page changes), no new privileges and a private `/tmp`; it reads `~/.claude/sessions` and the notes in `~/.local/state/capcom/notes`. After you reinstall `capcom`, run `systemctl --user restart capcom-web`.

## In the Claude Code desktop app

The desktop app's Browser pane can show a local server beside a session. According to Claude Code's docs (the Desktop page, "Configure preview servers"), it reads `.claude/launch.json` from the folder the session started in, and an entry with a `url` and no command attaches to a server that is already running:

```json
{
  "version": "0.0.1",
  "configurations": [{ "name": "capcom", "url": "http://127.0.0.1:1969", "port": 1969 }]
}
```

This repository has that file in `.claude/launch.json`. Put the same file in `.claude/` of any other folder you start sessions from. The docs describe no setting that opens the pane by itself and no folder-independent config, so the pane is opened from the **Dev servers** menu or by asking Claude to preview it.

## What is not the same as the TUI

- **Layout:** the page has a responsive layout of its own, with draggable edges between consoles (above) instead of the TUI's divider keys.
- **Herdr:** no workspaces, tabs or panes: skills start in new desktop sessions (above), and "go to the agent" opens a session in the desktop app.
- **Auto-fix's last step:** marked on the PR with a one-click session, not sent to the agent (above).
- **Loading screens, the bell and the terminal clipboard:** not ported (the page has a toast, the `log` and 📋 buttons).

## The console in every session

The repo is also a Claude Code plugin marketplace with one mod, `capcom` (in `mod/`), so you do not have to switch to the page: a pane beside every Claude Code session shows the GO / NO-GO light, the 5H and 7D gauges, the PRs that need you (red, needing a fix, a merge conflict, Copilot's comments, waiting for a reviewer or ready to merge) and the agents waiting on you with what they ask. A click opens the PR, `fix it ↗` copies `/pr-address <url>` and opens the PR's session to paste it into (or a new one), an agent opens its session, and `Open CAPCOM ↗` opens this page. Links are opened by `capcom web` itself (`POST /open`), so they work in the desktop app too.

Install it with `/plugin install capcom --marketplace philals/mission-control-capcom` (or, in the desktop app, + next to the prompt box, Plugins, Add marketplace, Add plugin). It asks `http://127.0.0.1:1969/api/state` every 5 seconds while a session is open, which also counts as a page looking, so GitHub is polled at the faster pace while any session is open. When `capcom web` does not answer the pane says so. It opens itself where a pane fits beside the chat (a wide terminal, the desktop app); `/capcom` opens it anywhere. Its buttons take the page's actions through the same endpoint (`POST /api/do`); there, a click on `[DRAFT]`/`[READY]` or the automerge tick takes effect at once, with no question.

