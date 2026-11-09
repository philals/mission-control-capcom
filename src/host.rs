//! Which app capcom is running in: Herdr, the Herdr app, Orca, the Claude desktop app, or a plain terminal.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Host {
    Herdr,
    HerdrApp,
    Orca,
    Desktop,
    Plain,
}

impl Host {
    pub const ALL: [Host; 5] = [Host::Herdr, Host::HerdrApp, Host::Orca, Host::Desktop, Host::Plain];

    /// The words `--host` and `CAPCOM_HOST` accept: every app but a plain terminal, which is only ever detected.
    pub const CHOICES: [Host; 4] = [Host::Herdr, Host::HerdrApp, Host::Orca, Host::Desktop];

    pub fn word(self) -> &'static str {
        match self {
            Host::Herdr => "herdr",
            Host::HerdrApp => "herdr-app",
            Host::Orca => "orca",
            Host::Desktop => "desktop",
            Host::Plain => "plain",
        }
    }

    /// The name to show people: `Herdr`, `Herdr app`, `Orca`, `Claude Desktop`, `a plain terminal`.
    pub fn name(self) -> &'static str {
        match self {
            Host::Herdr => "Herdr",
            Host::HerdrApp => "Herdr app",
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
/// The Herdr app's terminals set `HC_TERMINAL_SOCKET` (the socket of its terminal core), not `HERDR_ENV`.
pub fn detect(var: impl Fn(&str) -> Option<String>) -> Host {
    let is = |name: &str, value: &str| var(name).as_deref() == Some(value);
    if is("HERDR_ENV", "1") {
        Host::Herdr
    } else if var("HC_TERMINAL_SOCKET").is_some_and(|s| !s.is_empty()) {
        Host::HerdrApp
    } else if var("ORCA_TERMINAL_HANDLE").is_some_and(|h| !h.is_empty()) {
        Host::Orca
    } else if is("CLAUDE_CODE_ENTRYPOINT", "claude-desktop") || is("TERM_PROGRAM", "claude-desktop") {
        Host::Desktop
    } else {
        Host::Plain
    }
}

/// What was asked for with `--host` / `CAPCOM_HOST`: nothing (absent or blank), or one of `Host::CHOICES`.
pub fn asked(word: Option<&str>) -> Result<Option<Host>, String> {
    let Some(word) = word.map(str::trim).filter(|a| !a.is_empty()) else {
        return Ok(None);
    };
    Host::CHOICES.into_iter().find(|h| h.word() == word.to_lowercase()).map(Some).ok_or_else(|| {
        let words: Vec<&str> = Host::CHOICES.iter().map(|h| h.word()).collect();
        format!("unknown app {word:?}: use {}", words.join(", "))
    })
}

/// For the command line: the same check as `asked`, so a wrong word is refused when the arguments are read.
pub fn check(word: &str) -> Result<String, String> {
    asked(Some(word)).map(|_| word.to_string())
}

/// The app capcom-tui was told it runs in (`--host` / `CAPCOM_HOST`), else the one its environment shows.
pub fn choose(word: Option<&str>, var: impl Fn(&str) -> Option<String>) -> Result<Host, String> {
    Ok(asked(word)?.unwrap_or_else(|| detect(var)))
}

/// The app an agent runs in, from its pane id: `herdr-app:…`, `orca:…`, `claude-desktop:…`, `cli:…`, else a Herdr pane.
pub fn of_pane(pane: &str) -> Host {
    match pane.split(':').next().unwrap_or_default() {
        "herdr-app" => Host::HerdrApp,
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
    fn a_herdr_app_terminal_is_the_herdr_app() {
        assert_eq!(detect(env(&[("HC_TERMINAL_SOCKET", "/run/user/1/herdr-core/default.sock")])), Host::HerdrApp);
        assert_eq!(detect(env(&[("HC_TERMINAL_SOCKET", "")])), Host::Plain);
    }

    #[test]
    fn herdr_wins_over_the_herdr_app_it_runs_in() {
        assert_eq!(detect(env(&[("HERDR_ENV", "1"), ("HC_TERMINAL_SOCKET", "/s")])), Host::Herdr);
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
        assert_eq!(of_pane("herdr-app:t8"), Host::HerdrApp);
        assert_eq!(of_pane("w1-p2"), Host::Herdr);
    }

    #[test]
    fn an_app_asked_for_wins_over_the_environment() {
        assert_eq!(choose(Some("orca"), env(&[("HERDR_ENV", "1")])), Ok(Host::Orca));
        assert_eq!(choose(Some("desktop"), env(&[])), Ok(Host::Desktop));
        assert_eq!(choose(Some("herdr"), env(&[])), Ok(Host::Herdr));
        assert_eq!(choose(Some(" Herdr-App "), env(&[])), Ok(Host::HerdrApp), "case and spaces do not matter");
        assert_eq!(choose(None, env(&[("ORCA_TERMINAL_HANDLE", "t1")])), Ok(Host::Orca));
        assert_eq!(choose(Some(""), env(&[("ORCA_TERMINAL_HANDLE", "t1")])), Ok(Host::Orca), "empty means not asked");
        let err = choose(Some("vscode"), env(&[])).unwrap_err();
        assert!(err.contains("herdr, herdr-app, orca, desktop"), "{err}");
        assert!(choose(Some("plain"), env(&[])).is_err(), "plain is only ever detected");
    }

    #[test]
    fn the_command_line_refuses_a_wrong_app_but_does_not_need_one() {
        assert_eq!(check("herdr-app"), Ok("herdr-app".to_string()));
        assert_eq!(check(""), Ok(String::new()));
        assert_eq!(check("  "), Ok("  ".to_string()));
        for bad in ["vscode", "plain", "herdr app"] {
            assert!(check(bad).is_err(), "{bad}");
        }
        assert_eq!(asked(None), Ok(None));
    }

    #[test]
    fn words_round_trip() {
        for h in Host::ALL {
            assert_eq!(Host::parse(h.word()), Some(h));
        }
        assert_eq!(Host::parse("nope"), None);
    }
}
