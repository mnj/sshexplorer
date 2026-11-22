//! When Auto-refresh ticks, apart from egui and the clock: the Session passes the time
//! in, so the timing is plain arithmetic.
//!
//! Each tick is a Refresh, and the interval restarts after any Refresh. The Session only
//! asks while it is shown; a tick that falls due while it is not shown, while the window
//! is minimized, or while a Refresh runs is held back, and the Session refreshes once as
//! soon as it can again.

use crate::settings::AutoRefresh;

/// One Session's Auto-refresh interval.
#[derive(Clone, Debug, Default)]
pub struct Timer {
    /// When the interval started, in seconds; `None` until the first ask.
    since: Option<f64>,
    /// The interval it started with.
    every: AutoRefresh,
}

impl Timer {
    /// Starts the interval again at `now`: a Refresh has just finished.
    pub fn restart(&mut self, now: f64) {
        self.since = Some(now);
    }

    /// Whether to Refresh now, at `now` seconds, with the interval set to `every`.
    /// `ready` is false while the window is minimized or a Refresh runs: a due tick then
    /// waits until it is true. A tick restarts the interval.
    pub fn tick(&mut self, now: f64, every: AutoRefresh, ready: bool) -> bool {
        let Some(seconds) = every.seconds() else {
            self.since = None;
            self.every = every;
            return false;
        };
        let since = match self.since {
            Some(since) if self.every == every => since,
            _ => {
                self.since = Some(now);
                self.every = every;
                return false;
            }
        };
        if ready && now - since >= seconds {
            self.since = Some(now);
            return true;
        }
        false
    }

    /// The seconds from `now` until the next tick falls due, for a repaint; `None` when
    /// Auto-refresh is off.
    pub fn wait(&self, now: f64, every: AutoRefresh) -> Option<f64> {
        let seconds = every.seconds()?;
        let since = self.since.filter(|_| self.every == every).unwrap_or(now);
        Some((since + seconds - now).max(0.0))
    }
}
