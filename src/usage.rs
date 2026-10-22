//! How much of the Claude plan limits is used: the 5-hour and the weekly window. Claude Code gives these to
//! the status line command as JSON (`rate_limits.five_hour.used_percentage` and so on) while a session runs;
//! the status line script writes that JSON here (see docs/web.md), and the page reads it.
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// 0 to 100 (or more, when over the limit).
    pub used_percent: f64,
    /// When the window starts again, if the status line said.
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Usage {
    /// When the status line last wrote it: how old the figures are.
    pub at: DateTime<Utc>,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

/// `$CAPCOM_USAGE_FILE`, else `usage.json` beside the notes in the state folder.
pub fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("CAPCOM_USAGE_FILE") {
        return Some(PathBuf::from(p));
    }
    Some(crate::notes::dir()?.parent()?.join("usage.json"))
}

/// A time given as seconds or milliseconds since 1970, or as an RFC 3339 text.
fn time_of(v: &Value) -> Option<DateTime<Utc>> {
    if let Some(n) = v.as_f64() {
        let secs = if n > 1.0e11 { n / 1000.0 } else { n };
        return Utc.timestamp_opt(secs as i64, 0).single();
    }
    DateTime::parse_from_rfc3339(v.as_str()?).ok().map(|t| t.with_timezone(&Utc))
}

fn window(v: &Value) -> Option<Window> {
    let used = v["used_percentage"].as_f64().or_else(|| v["used_percent"].as_f64())?;
    if !used.is_finite() || used < 0.0 {
        return None;
    }
    Some(Window { used_percent: used, resets_at: time_of(&v["resets_at"]) })
}

pub fn parse(json: &str) -> Option<Usage> {
    let v: Value = serde_json::from_str(json).ok()?;
    let limits = &v["rate_limits"];
    let (five_hour, seven_day) = (window(&limits["five_hour"]), window(&limits["seven_day"]));
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    Some(Usage { at: time_of(&v["at"]).unwrap_or_else(Utc::now), five_hour, seven_day })
}

pub fn read(path: &Path) -> Option<Usage> {
    parse(&std::fs::read_to_string(path).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_windows_are_read_with_their_reset_times_in_any_form() {
        let u = parse(r#"{"at": 1790000000, "rate_limits": {"five_hour": {"used_percentage": 23.5, "resets_at": 1790003600}, "seven_day": {"used_percentage": 41, "resets_at": "2026-10-30T09:00:00Z"}}}"#).unwrap();
        assert_eq!(u.at.timestamp(), 1_790_000_000);
        let five = u.five_hour.unwrap();
        assert_eq!(five.used_percent, 23.5);
        assert_eq!(five.resets_at.unwrap().timestamp(), 1_790_003_600);
        let week = u.seven_day.unwrap();
        assert_eq!(week.used_percent, 41.0);
        assert_eq!(week.resets_at.unwrap().to_rfc3339(), "2026-10-30T09:00:00+00:00");
    }

    #[test]
    fn a_time_in_milliseconds_is_still_a_time() {
        let u = parse(r#"{"at": 1790000000000, "rate_limits": {"five_hour": {"used_percentage": 5, "resets_at": 1790003600000}}}"#).unwrap();
        assert_eq!(u.at.timestamp(), 1_790_000_000);
        assert_eq!(u.five_hour.unwrap().resets_at.unwrap().timestamp(), 1_790_003_600);
        assert!(u.seven_day.is_none(), "one window is enough");
    }

    #[test]
    fn nothing_usable_is_nothing() {
        assert!(parse("not json").is_none());
        assert!(parse(r#"{"at": 1, "rate_limits": null}"#).is_none());
        assert!(parse(r#"{"at": 1, "rate_limits": {"five_hour": {"used_percentage": -3}}}"#).is_none());
        assert!(parse(r#"{"at": 1, "rate_limits": {"five_hour": {}}}"#).is_none());
    }
}
