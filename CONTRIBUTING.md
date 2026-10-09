# Contributing

Thanks for looking. First, a warning that is also in the README: capcom is built around **one person's workflow** (Jira stories split into draft-PR tasks, local files as the source of truth, Claude Code skills, GitHub through `gh`, optionally Herdr). It is shared as a working example, not as a general project-management tool.

That shapes what is likely to be accepted:

- **Welcome:** bug fixes, clearer docs, accessibility improvements, tests, portability fixes (macOS, other terminals), and small improvements that fit the existing model.
- **Probably not:** new statuses or workflows that change the model, support for other trackers or hosts as first-class features, or anything that adds a daemon, a database or stored credentials. A fork is the right home for those, and you are very welcome to make one (it is MIT licensed).

If you are unsure, open an issue and ask before writing code.

## Working on it

```bash
cargo build
cargo test                       # the whole suite, it is quick
cargo test --bin capcom-tui <name>
capcom-tui --demo                # try UI changes against invented data
```

- **Tests first.** Behaviour changes come with a test that fails without them. The terminal UI is tested by drawing to an in-memory screen and clicking on what it drew (see `src/bin/capcom-tui/ui.rs`), so most behaviour can be tested without a terminal.
- **Accessibility is a requirement.** Every colour must come from `theme.rs`, and the readability test in `ui.rs` checks contrast on every screen. State must never be shown by colour alone.
- **No private data.** Examples, tests and docs use invented names (`acme/widgets`, `PROJ-123`, `DEMO-101`). Please keep real company names, people, tickets, hosts, emails and paths out of every commit and every commit message.
- **README pictures** are generated from the demo data: see the Development section of the README.
- **Comments** are short and only where the code cannot say it.

## Commits and pull requests

Small, focused pull requests with a clear description. Please use a noreply address for your commits if you do not want your email in the history.
