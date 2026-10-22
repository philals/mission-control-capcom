//! Whether anyone is looking at the board. When nobody is, GitHub is asked about far less often.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How much slower the feeds go while nobody is looking, and the least they wait.
const AWAY_FACTOR: u32 = 4;
const AWAY_MINIMUM: Duration = Duration::from_secs(60);

/// Two signals say the board is out of sight: the terminal reports its window lost focus, and
/// Herdr says another pane has focus. Either one is enough. Unknown counts as being looked at.
pub struct Attention {
    terminal: AtomicBool,
    herdr: AtomicBool,
    /// The user's switch: off means always poll at the normal pace.
    slow_when_away: AtomicBool,
    /// The least the feeds wait while away, in seconds.
    away_minimum: AtomicU64,
}

impl Attention {
    pub fn new() -> Arc<Attention> {
        Attention::waiting_at_least(AWAY_MINIMUM)
    }

    /// Like `new`, with a different least wait while away (a page nobody has open can wait very long).
    pub fn waiting_at_least(away_minimum: Duration) -> Arc<Attention> {
        Arc::new(Attention { terminal: AtomicBool::new(true), herdr: AtomicBool::new(true), slow_when_away: AtomicBool::new(true), away_minimum: AtomicU64::new(away_minimum.as_secs()) })
    }

    pub fn set_terminal(&self, focused: bool) {
        self.terminal.store(focused, Ordering::Relaxed);
    }

    pub fn set_herdr(&self, focused: bool) {
        self.herdr.store(focused, Ordering::Relaxed);
    }

    /// Change the least wait while away (a page with notifications on cannot wait half an hour).
    pub fn set_away_minimum(&self, least: Duration) {
        self.away_minimum.store(least.as_secs(), Ordering::Relaxed);
    }

    pub fn set_slow_when_away(&self, on: bool) {
        self.slow_when_away.store(on, Ordering::Relaxed);
    }

    pub fn slow_when_away(&self) -> bool {
        self.slow_when_away.load(Ordering::Relaxed)
    }

    /// Out of sight, so polling is slowed down (when the user allows it).
    pub fn away(&self) -> bool {
        self.slow_when_away() && !(self.terminal.load(Ordering::Relaxed) && self.herdr.load(Ordering::Relaxed))
    }

    /// The wait before the next poll: unchanged while looked at, much longer while away.
    pub fn wait(&self, base: Duration) -> Duration {
        if self.away() {
            base.saturating_mul(AWAY_FACTOR).max(Duration::from_secs(self.away_minimum.load(Ordering::Relaxed)))
        } else {
            base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_slows_when_either_signal_says_nobody_is_looking() {
        let a = Attention::new();
        let busy = Duration::from_secs(5);
        assert_eq!(a.wait(busy), busy, "looked at: normal pace");
        a.set_terminal(false);
        assert!(a.away());
        assert_eq!(a.wait(busy), Duration::from_secs(60), "never less than a minute away");
        assert_eq!(a.wait(Duration::from_secs(30)), Duration::from_secs(120));
        a.set_terminal(true);
        a.set_herdr(false);
        assert!(a.away(), "Herdr focus alone is enough");
        a.set_herdr(true);
        assert!(!a.away());
    }

    #[test]
    fn a_longer_least_wait_applies_only_while_away() {
        let a = Attention::waiting_at_least(Duration::from_secs(1800));
        let base = Duration::from_secs(5);
        assert_eq!(a.wait(base), base);
        a.set_terminal(false);
        assert_eq!(a.wait(base), Duration::from_secs(1800));
        assert_eq!(a.wait(Duration::from_secs(600)), Duration::from_secs(2400), "four times a long base is still longer");
    }

    #[test]
    fn the_least_wait_can_change_while_running() {
        let a = Attention::waiting_at_least(Duration::from_secs(1800));
        a.set_terminal(false);
        assert_eq!(a.wait(Duration::from_secs(5)), Duration::from_secs(1800));
        a.set_away_minimum(Duration::from_secs(120));
        assert_eq!(a.wait(Duration::from_secs(5)), Duration::from_secs(120));
    }

    #[test]
    fn the_switch_keeps_the_normal_pace_whatever_the_signals_say() {
        let a = Attention::new();
        a.set_terminal(false);
        a.set_slow_when_away(false);
        assert!(!a.away());
        assert_eq!(a.wait(Duration::from_secs(5)), Duration::from_secs(5));
    }
}
