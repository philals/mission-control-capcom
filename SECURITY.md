# Security

capcom is a local tool: it reads and writes files under the folder you choose, runs the `gh` command you have already logged in, and (through the skills) talks to services you have already connected to Claude Code. It stores no credentials and has no server.

If you find a vulnerability, please report it privately through GitHub's "Report a vulnerability" button on the repository's Security tab rather than in a public issue. Please include what you did, what you expected and what happened. This is a personal project, so there is no formal response time, but reports are taken seriously.

Things worth reporting include: a way to make `capcom` or `capcom-tui` run a command or open a link you did not intend (for example from a crafted `board.json` or PR title), writing outside the stories folder, or leaking data from the cache folder.
