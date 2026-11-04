//! Which app capcom is running in: Herdr, Orca, the Claude desktop app, or a plain terminal.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Herdr,
    Orca,
    Desktop,
    Plain,
}

impl Host {
    pub const ALL: [Host; 4] = [Host::Herdr, Host::Orca, Host::Desktop, Host::Plain];

    pub fn word(self) -> &'static str {
        match self {
            Host::Herdr => "herdr",
            Host::Orca => "orca",
            Host::Desktop => "desktop",
            Host::Plain => "plain",
        }
    }

    /// The name to show people: `Herdr`, `Orca`, `Claude Desktop`, `a plain terminal`.
    pub fn name(self) -> &'static str {
        match self {
            Host::Herdr => "Herdr",
            Host::Orca => "Orca",
            Host::Desktop => "Claude Desktop",
            Host::Plain => "a plain terminal",
        }
    }

    pub fn parse(word: &str) -> Option<Host> {
        Host::ALL.into_iter().find(|h| h.word() == word)
    }
}

/// Herdr first, since Herdr can run inside an Orca terminal: whichever wraps capcom most closely wins.
pub fn detect(var: impl Fn(&str) -> Option<String>) -> Host {
    let is = |name: &str, value: &str| var(name).as_deref() == Some(value);
    if is("HERDR_ENV", "1") {
        Host::Herdr
    } else if var("ORCA_TERMINAL_HANDLE").is_some_and(|h| !h.is_empty()) {
        Host::Orca
    } else if is("CLAUDE_CODE_ENTRYPOINT", "claude-desktop") || is("TERM_PROGRAM", "claude-desktop") {
        Host::Desktop
    } else {
        Host::Plain
    }
}

/// The app an agent runs in, from its pane id: `orca:…`, `claude-desktop:…`, `cli:…`, else a Herdr pane.
pub fn of_pane(pane: &str) -> Host {
    match pane.split(':').next().unwrap_or_default() {
        "orca" => Host::Orca,
        "claude-desktop" => Host::Desktop,
        "cli" => Host::Plain,
        _ => Host::Herdr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn herdr_wins_even_inside_an_orca_terminal() {
        assert_eq!(detect(env(&[("HERDR_ENV", "1"), ("ORCA_TERMINAL_HANDLE", "t1")])), Host::Herdr);
    }

    #[test]
    fn an_orca_terminal_is_orca() {
        assert_eq!(detect(env(&[("ORCA_TERMINAL_HANDLE", "t1")])), Host::Orca);
    }

    #[test]
    fn the_desktop_app_is_desktop() {
        assert_eq!(detect(env(&[("CLAUDE_CODE_ENTRYPOINT", "claude-desktop")])), Host::Desktop);
    }

    #[test]
    fn the_desktop_apps_terminal_panel_is_desktop() {
        assert_eq!(detect(env(&[("TERM_PROGRAM", "claude-desktop")])), Host::Desktop);
    }

    #[test]
    fn nothing_known_is_plain() {
        assert_eq!(detect(env(&[])), Host::Plain);
    }

    #[test]
    fn herdr_env_must_be_1() {
        assert_eq!(detect(env(&[("HERDR_ENV", "0")])), Host::Plain);
    }

    #[test]
    fn an_empty_orca_handle_is_not_orca() {
        assert_eq!(detect(env(&[("ORCA_TERMINAL_HANDLE", "")])), Host::Plain);
    }

    #[test]
    fn panes_name_their_app() {
        assert_eq!(of_pane("orca:term_abc"), Host::Orca);
        assert_eq!(of_pane("claude-desktop:7:local_a"), Host::Desktop);
        assert_eq!(of_pane("cli:7"), Host::Plain);
        assert_eq!(of_pane("w1-p2"), Host::Herdr);
    }

    #[test]
    fn words_round_trip() {
        for h in Host::ALL {
            assert_eq!(Host::parse(h.word()), Some(h));
        }
        assert_eq!(Host::parse("nope"), None);
    }
}
