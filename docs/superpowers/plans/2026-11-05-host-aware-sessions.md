# Sessions open in the app you are using: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** capcom starts new Claude sessions in whichever app you are using (Claude Desktop, Herdr or Orca), and clicking an agent takes you to the app it runs in.

**Architecture:** Two new library modules both front ends share: `host` (which app am I in, from the environment or a web request) and `orca` (every call to Orca's CLI, behind an injectable runner like `herdr::Cli::with_runner`), plus `starting` (short-lived "starting" marks against duplicate starts). capcom-tui keeps its `trait Herdr` seam and gains `Orca` and `Desktop` implementations behind a `Router` that launches in the home app and focuses by the agent's own app. capcom web learns the viewer's app per request and gets two server-side actions for Orca.

**Tech Stack:** Rust (existing crate, `serde_json`, `std::process::Command`), plain JS in `src/web/js`, the Claude Code mod in `mod/hooks` (TypeScript).

**Spec:** `docs/superpowers/specs/2026-11-05-host-aware-sessions-design.md`

## Global Constraints

- A session never opens in an app other than the one decided by the spec's rules; when that app can't be reached, say so and start nothing.
- Never call bare `orca`: CLI lookup order is `$ORCA_CLI_BIN_DIR/orca`, then `~/.config/orca/linux-orca-cli-shim/orca`, then `orca-ide` on PATH.
- Orca agents always launch with `--surface terminal`.
- Every Orca call goes through `src/orca.rs`; tests use a fake runner, never the real CLI.
- Today's Herdr and Claude Desktop behaviour must not change.
- Only scoped test runs locally (`cargo test --lib <module>`, `cargo test --bin capcom-tui <module>::`), never the full suite. capcom commits straight to `main`.
- Minimal comments.

## Review Focus

1. **Orca not running or its CLI missing:** an action that needs Orca gives a one-line notice naming the problem and starts nothing (no fallback to Desktop). Test in Task 3 and Task 5.
2. **Task folder unset, gone, or unknown to Orca:** notice says so and gives `orca repo add --path <folder>`. Test in Task 3.
3. **Same task clicked twice quickly, or its agent already running in another app:** the second click focuses or reports the existing agent; no second start. Test in Task 4 and Task 5.
4. **Herdr running inside an Orca terminal:** home app is Herdr. Test in Task 2.
5. **Web page in an ordinary browser with no hint:** behaves exactly as today (`claude://` links). Test in Task 7.

---

### Task 1: Spike the unknowns (no product code)

**Files:**
- Modify: `docs/superpowers/specs/2026-11-05-host-aware-sessions-design.md` (add a "Spike results" section)

- [ ] **Step 1:** Start a Claude agent from Orca (`orca agent launch --agent claude --surface terminal --worktree path:/home/PAlsford/code/capcom --json`, using the CLI path rule above). While it runs, record whether a new `~/.claude/sessions/<pid>.json` appears and its `entrypoint`; record the agent's row from `orca worktree ps --json` and the matching row of `orca terminal list --json` (how `paneKey` joins to `tabId`/`leafId` and the handle). Close it with `orca terminal close`.
- [ ] **Step 2:** In Orca's built-in browser open `http://127.0.0.1:1969/` and record `navigator.userAgent` (`orca` browser eval, see `orca skills get orca-cli --reference references/browser.md`).
- [ ] **Step 3:** In Claude Desktop's Terminal panel, record `env | grep -E '^(CLAUDE|TERM_PROGRAM)'`.
- [ ] **Step 4:** From a shell outside Herdr, run `herdr agent list`; record whether it answers.
- [ ] **Step 5:** Run `xdg-open 'claude://code/continue?session=<a local_ id>&prompt=hello'`; record whether the prompt is typed in.
- [ ] **Step 6:** From inside the web sandbox, test reaching Orca: `systemd-run --user --pty -p PrivateTmp=true -p ProtectHome=read-only ~/.config/orca/linux-orca-cli-shim/orca worktree ps --json`. Record the result; if it fails, pick the smallest unit-file change that works (expected: `PrivateTmp=false`) and record it.
- [ ] **Step 7:** Write "Spike results" into the spec with one line per step, and note any decision in Tasks 2–8 the results overturn. Commit:

```bash
git add docs/superpowers/specs/2026-11-05-host-aware-sessions-design.md
git commit -m "capcom: spike results for opening sessions in Claude Desktop, Herdr or Orca"
```

### Task 2: `host` module (which app am I in)

**Files:**
- Create: `src/host.rs`; register `pub mod host;` in `src/lib.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Host { Herdr, Orca, Desktop, Plain }`
  - `impl Host { pub fn word(self) -> &'static str }` → `"herdr" | "orca" | "desktop" | "plain"`, and `pub fn parse(word: &str) -> Option<Host>` (inverse)
  - `pub fn detect(var: impl Fn(&str) -> Option<String>) -> Host`
  - `pub fn of_pane(pane: &str) -> Host` — `"orca:…"` → Orca, `"claude-desktop:…"` → Desktop, `"cli:…"` → Plain, anything else (Herdr pane ids) → Herdr

- [ ] **Step 1: Write failing tests in `src/host.rs`**

```rust
#[test] fn herdr_wins_even_inside_an_orca_terminal() {
    let env = |k: &str| match k { "HERDR_ENV" => Some("1".into()), "ORCA_TERMINAL_HANDLE" => Some("t1".into()), _ => None };
    assert_eq!(detect(env), Host::Herdr);
}
#[test] fn an_orca_terminal_is_orca() { assert_eq!(detect(|k| (k == "ORCA_TERMINAL_HANDLE").then(|| "t1".into())), Host::Orca); }
#[test] fn the_desktop_app_is_desktop() { assert_eq!(detect(|k| (k == "CLAUDE_CODE_ENTRYPOINT").then(|| "claude-desktop".into())), Host::Desktop); }
#[test] fn nothing_known_is_plain() { assert_eq!(detect(|_| None), Host::Plain); }
#[test] fn herdr_env_must_be_1() { assert_eq!(detect(|k| (k == "HERDR_ENV").then(|| "0".into())), Host::Plain); }
#[test] fn panes_name_their_app() {
    assert_eq!(of_pane("orca:term_abc"), Host::Orca);
    assert_eq!(of_pane("claude-desktop:7:local_a"), Host::Desktop);
    assert_eq!(of_pane("cli:7"), Host::Plain);
    assert_eq!(of_pane("w1-p2"), Host::Herdr);
}
#[test] fn words_round_trip() { for h in [Host::Herdr, Host::Orca, Host::Desktop, Host::Plain] { assert_eq!(Host::parse(h.word()), Some(h)); } }
```

- [ ] **Step 2:** `cargo test --lib host` → FAIL (module missing).
- [ ] **Step 3:** Implement. Order: `HERDR_ENV=="1"`, then `ORCA_TERMINAL_HANDLE` non-empty, then `CLAUDE_CODE_ENTRYPOINT=="claude-desktop"`, else Plain. (If Task 1 Step 3 found a different Desktop terminal marker, use that.)
- [ ] **Step 4:** `cargo test --lib host` → PASS.
- [ ] **Step 5:** Commit `capcom: tell which app capcom is running in (Herdr, Orca, Claude Desktop)`.

### Task 3: `orca` module (Orca's CLI)

**Files:**
- Create: `src/orca.rs`; register `pub mod orca;` in `src/lib.rs`

**Interfaces:**
- Consumes: `crate::agents::{AgentInfo, Status}`
- Produces:
  - `pub type Runner = Box<dyn Fn(&[String]) -> Result<serde_json::Value, String> + Send + Sync>`
  - `pub fn find_cli(var: impl Fn(&str) -> Option<String>, home: &Path, on_path: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf>`
  - `pub struct Orca` with `pub fn new(cli: PathBuf) -> Orca` and `pub fn with_runner(run: Runner) -> Orca`
  - `pub fn launch(&self, folder: &str, prompt: &str, focus: bool) -> Result<String, String>` → the agent's terminal handle (`result.launch.outcome.handle`)
  - `pub fn resume(&self, checkout: &str, session_cwd: &str, session: &str, focus: bool) -> Result<String, String>` → handle; command is `cd <session_cwd shell-quoted> && claude --resume <session>`
  - `pub fn agents(&self) -> Result<Vec<AgentInfo>, String>` — Claude agents only; `pane` = `"orca:<handle>"`, `session` = the Claude session id from `paneKey`, `workspace` = the worktree's `displayName`, `cwd` = worktree `path`
  - `pub fn switch(&self, handle: &str) -> Result<(), String>`, `pub fn send(&self, handle: &str, text: &str) -> Result<(), String>`, `pub fn close(&self, handle: &str) -> Result<(), String>`
  - `pub fn handle_of(pane: &str) -> Option<&str>` (strips `orca:`)
  - `pub fn checkout_of(&self, folder: &str) -> Result<String, String>` — the Orca worktree path that is `folder` or encloses it, from `orca worktree list --json`

- [ ] **Step 1: Write failing tests** with a recording fake runner (copy the `cli(...)`/`Calls` pattern from `src/bin/capcom-tui/herdr.rs` tests):
  - `the_cli_is_found_from_orca_cli_bin_dir_then_the_shim_never_bare_orca`: env dir wins; else `<home>/.config/orca/linux-orca-cli-shim/orca` when it exists; else `orca-ide` from PATH; `on_path("orca")` is never asked (assert the closure never sees `"orca"`).
  - `launch_asks_for_a_terminal_in_the_folder_and_returns_the_handle`: args equal `["agent","launch","--agent","claude","--surface","terminal","--worktree","path:/w/app","--prompt","/story-implement-task K-1 T2","--json"]` (+ `"--focus"` when focus); fake returns `{"ok":true,"result":{"launch":{"outcome":{"kind":"terminal","handle":"term_9"}}}}`; result `Ok("term_9")`.
  - `a_folder_orca_does_not_know_says_how_to_add_it`: `checkout_of("/w/other")` with a list lacking it → `Err` containing `orca repo add --path /w/other`.
  - `a_session_in_a_claude_worktree_resumes_from_the_enclosing_checkout`: `checkout_of("/w/app/.claude/worktrees/x")` → `Ok("/w/app")`; `resume` sends `terminal create --worktree path:/w/app --command "cd '/w/app/.claude/worktrees/x' && claude --resume s-1" --json`.
  - `agents_are_claude_rows_of_worktree_ps_with_session_ids_and_handles`: from a `worktree ps` fixture shaped like Task 1 Step 1's record, a codex row is dropped and a claude row maps to `AgentInfo { pane: "orca:term_9", session: Some("654367da-be09-46d0-a58d-b6100bd5b313"), .. }` with status from `state` (`working`→Working, `done`→Done, `waiting`/`blocked`→Blocked, else Idle).
  - `orca_not_running_is_an_error_naming_orca`: runner returns `Err("Orca CLI is unavailable…")` → `launch` `Err` starts with `"Orca: "`.
- [ ] **Step 2:** `cargo test --lib orca` → FAIL.
- [ ] **Step 3:** Implement. Real runner: `Command::new(cli).args(args)`, parse stdout as JSON, `ok:false` → `Err(error.message)`. Every error is prefixed `"Orca: "`. Join `worktree ps` agents to handles through `terminal list` as Task 1 recorded.
- [ ] **Step 4:** `cargo test --lib orca` → PASS.
- [ ] **Step 5:** Commit `capcom: talk to Orca's CLI (launch, resume, list, switch, send, close)`.

### Task 4: `starting` marks (no double starts)

**Files:**
- Create: `src/starting.rs`; register in `src/lib.rs`

**Interfaces:**
- Produces:
  - `pub struct Starting { path: PathBuf }`, `pub fn new(path: PathBuf) -> Starting` (file `~/.cache/capcom/starting.json`, via `cache::Cache::default_dir()` at call sites)
  - `pub const KEPT: Duration = Duration::from_secs(300)`
  - `pub fn mark(&self, key: &str, task: &str, host: Host, now: u64) -> bool` — false if a live mark already exists (then nothing is written)
  - `pub fn clear(&self, key: &str, task: &str)`
  - `pub fn live(&self, key: &str, task: &str, now: u64) -> Option<Host>`

- [ ] **Step 1: Write failing tests:** `a_second_mark_within_five_minutes_is_refused`; `a_mark_older_than_five_minutes_is_gone` (`live(.., now + 301)` is None and `mark` succeeds again); `clear_removes_only_that_task`; `a_missing_or_broken_file_is_no_marks`.
- [ ] **Step 2:** `cargo test --lib starting` → FAIL.
- [ ] **Step 3:** Implement as a small JSON map `"KEY/TASK" -> {host, at}`; write via temp file + rename. Recording a session (`capcom set-session`, `src/ops.rs`) calls `clear`.
- [ ] **Step 4:** `cargo test --lib starting` → PASS.
- [ ] **Step 5:** Commit `capcom: a start leaves a five-minute mark so a second click doesn't start a second agent`.

### Task 5: capcom-tui launches in its own app and focuses in the agent's app

**Files:**
- Create: `src/bin/capcom-tui/hosts.rs` (`OrcaHost`, `DesktopHost`, `Router`)
- Modify: `src/bin/capcom-tui/main.rs:183-187` (choose the home app), `src/bin/capcom-tui/app.rs` (notices that say "Herdr": `run_launch` 2632, `open_task_agent`, PR go-to-agent 2337-2360, `agents_panel` 2168, `start_agents` 2129; start paths check `starting`)
- Test: in `hosts.rs`

**Interfaces:**
- Consumes: `capcom::host::{Host, detect, of_pane}`, `capcom::orca::Orca`, `capcom::starting::Starting`, `herdr::{Herdr, Launch, Resume, Outcome, Cli}`, `capcom::claude::{live, desktop_link, import_link}`
- Produces:
  - `pub struct OrcaHost(pub capcom::orca::Orca)` and `pub struct DesktopHost { pub open: Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>, pub config: PathBuf }`, both `impl Herdr`
  - `pub struct Router { pub home: Host, pub hosts: Vec<(Host, Arc<dyn Herdr>)> }`, `impl Herdr for Router`
  - `pub fn router(home: Host, workdir: PathBuf) -> Option<Router>` — None for `Host::Plain`; includes Herdr only when home is Herdr (unless Task 1 Step 4 says Herdr answers from outside), Orca when its CLI is found, Desktop always
  - `App::herdr` keeps its type `Option<Arc<dyn Herdr>>`; `App` gains `pub home: Host` for notice wording

Behaviour:
- `DesktopHost::launch` opens `claude://code/new?prompt=…&folder=…` (move `new_session_link` from `src/web/view.rs` to `capcom::claude::new_session_link` and use it from both). `resume` with a running desktop session opens `desktop_link`; with a recorded session not running, `import_link`; `agents()` = `claude::live(config)` rows whose entrypoint is `claude-desktop`; `focus(pane)` opens `desktop_link(pane)`.
- `OrcaHost::launch` = `checkout_of(folder)` then `Orca::launch` (focus true); `resume` = running agent with that session → `switch` (and `send` when a prompt is given and it is not working), else `Orca::resume`.
- `Router::launch`: if any host's `agents()` has the task's recorded session or a live `starting` mark exists → focus that agent (`Outcome::Focused`) or report "already starting in <app>"; else `starting.mark` then the home host's `launch`.
- `Router::focus/close(pane)`: dispatch by `of_pane(pane)`; a host not in `hosts` → `Err("it's running in <App>; switch to <App> to reach it")`.
- `Router::agents`: concatenation over `hosts`, a failing host skipped.

- [ ] **Step 1: Write failing tests:**
  - `launching_from_desktop_opens_a_new_session_link_in_the_task_folder`
  - `launching_from_orca_uses_orca_never_the_desktop_link` (Orca fake returns error → `Err` starting `"Orca: "`, desktop `open` never called)
  - `an_agent_running_in_another_app_is_focused_not_started_again` (Orca home, task session live in Desktop → desktop `open` called with `claude://code/continue?session=…`, no Orca launch)
  - `a_second_click_while_starting_reports_it_and_starts_nothing`
  - `focus_goes_to_the_agents_own_app`
  - `a_herdr_agent_seen_from_outside_herdr_names_herdr`
  - `plain_terminals_get_no_router`
- [ ] **Step 2:** `cargo test --bin capcom-tui hosts::` → FAIL.
- [ ] **Step 3:** Implement `hosts.rs`; in `main.rs` replace the `inside_herdr()` block with `let home = capcom::host::detect(|k| std::env::var(k).ok()); if let Some(r) = hosts::router(home, cwd) { app.home = home; app.herdr = Some(Arc::new(r)); app.start_agents(); }` (keep `watch_focus` for Herdr only). Replace "not running inside Herdr" notices with ones naming `app.home` ("not running inside Herdr, Orca or Claude Desktop, so no agent can be started"), and "started … in Herdr workspace …" with the home app's name.
- [ ] **Step 4:** `cargo test --bin capcom-tui hosts::` and `cargo test --bin capcom-tui herdr::` → PASS (Herdr tests unchanged).
- [ ] **Step 5:** Commit `capcom-tui: start agents in the app it runs in (Herdr, Orca or Claude Desktop) and go to each agent in its own app`.

### Task 6: capcom-tui automatic prompts

**Files:**
- Modify: `src/bin/capcom-tui/app.rs:1442` area (auto-fix uses `Resume { focus: false }`), `hosts.rs`

**Interfaces:**
- Consumes: `Router` (Task 5)

Behaviour (spec "Automatic prompts"): Herdr/Orca agent running → prompt in place, no focus, `Busy` when working. Desktop agent running → open it with the prompt waiting (link from Task 1 Step 5: `claude://code/continue?session=…&prompt=…` if it worked, else `claude://code/new?prompt=…&folder=<session cwd>`). No agent running → home app's launch.

- [ ] **Step 1: Write failing tests in `hosts.rs`:** `an_automatic_prompt_to_a_desktop_agent_opens_it_with_the_prompt`; `an_automatic_prompt_to_an_orca_agent_is_sent_without_switching` (`send` called, `switch` not); `an_automatic_prompt_to_a_working_orca_agent_is_busy`; `an_automatic_prompt_with_no_agent_starts_one_in_the_home_app`.
- [ ] **Step 2:** `cargo test --bin capcom-tui hosts::` → FAIL.
- [ ] **Step 3:** Implement in `Router::resume` (the `prompt.is_some() && !focus` path).
- [ ] **Step 4:** `cargo test --bin capcom-tui hosts::` → PASS.
- [ ] **Step 5:** Commit `capcom-tui: automatic prompts go to the app the agent runs in`.

capcom web's autopilot only notifies today (`src/web/autopilot.rs:note_fix_need`) and starts no agents, so it is unchanged.

### Task 7: capcom web knows the viewer's app; Orca actions

**Files:**
- Modify: `src/web.rs` (`/api/state` and `/api/do` read the host), `src/web/actions.rs` (actions `session.start`, `agent.open`), `src/web/view.rs` (agents list includes Orca agents; `source: "orca"`), `src/web/js/core.js` (send host), `src/web/js/panels.js` + `extras.js` (`startSession` and agent clicks call the server when host is orca), `mod/hooks/register.tsx` (send `x-capcom-host: desktop` on every request), `~/.config/systemd/user/capcom-web.service` per Task 1 Step 6, `docs/web.md` service section
- Test: `src/web/tests_more.rs`

**Interfaces:**
- Consumes: `host::Host`, `orca::Orca`, `starting::Starting`, `claude::new_session_link`
- Produces:
  - Request host: header `x-capcom-host: <word>`; the page sets it from `?host=` on its own address (kept in `sessionStorage`) or, if Task 1 Step 2 found a distinctive user agent, from that; absent → `Desktop`
  - `POST /api/do {"action":"session.start","key","task","kind":"plan"|"implement"|"review"|"breakdown","host"}` → `{ok, message, link?}`: for `desktop` returns the existing `claude://code/new` link for the page to open (unchanged flow); for `orca` launches via `Orca` and returns no link; both check and set the `starting` mark
  - `POST /api/do {"action":"agent.open","pane"}` → Desktop panes: `{ok, link}` (existing deep link); Orca panes: `switch`, `{ok, message}`; Herdr panes: `{ok:false, message:"it's running in Herdr; switch to Herdr to reach it"}` unless Task 1 Step 4 says Herdr is reachable

- [ ] **Step 1: Write failing tests in `tests_more.rs`:**
  - `a_page_with_no_host_still_gets_claude_links` (Review Focus 5: `session.start` without host returns a `claude://code/new` link and calls no Orca runner)
  - `an_orca_page_starts_the_session_in_orca` (fake Orca records `agent launch … --surface terminal …`)
  - `orca_unreachable_from_an_orca_page_says_so_and_returns_no_link`
  - `a_second_start_within_five_minutes_is_refused`
  - `orca_agents_show_in_the_list_with_source_orca`
  - `opening_an_orca_agent_switches_to_it`
- [ ] **Step 2:** `cargo test --lib web::tests_more` → FAIL.
- [ ] **Step 3:** Implement. The Orca runner is injected through the existing tools struct the tests already fake (`fn open` at `src/web/tests_more.rs:295`), so add an `orca: Option<Arc<Orca>>` alongside.
- [ ] **Step 4:** `cargo test --lib web::tests_more` → PASS. Apply the unit-file change, `systemctl --user daemon-reload && systemctl --user restart capcom-web`, open `http://127.0.0.1:1969/?host=orca` in Orca's browser and implement a throwaway task; confirm an Orca terminal opens with the prompt.
- [ ] **Step 5:** Commit `capcom web: start sessions in the app the page is open in, including Orca's browser`.

### Task 8: Docs

**Files:**
- Modify: `README.md`, `docs/tui.md`, `docs/web.md`, `docs/reference.md`

- [ ] **Step 1:** Replace "inside Herdr" wording with the three-app rule, add a short Orca section (where capcom finds the CLI, `orca repo add` for unknown folders, terminals not chats, `?host=orca` for the web page), and note Claude Desktop's terminal panel per Task 1 Step 3.
- [ ] **Step 2:** `cargo build --locked` → succeeds; `cargo install --path . --root ~/.local --locked`; `systemctl --user restart capcom-web`.
- [ ] **Step 3:** Commit `capcom docs: Claude Desktop, Herdr and Orca` and push to `main`.
