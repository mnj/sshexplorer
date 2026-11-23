//! When a Disconnected Session tries to reconnect, and what its strip says, apart from
//! egui and the clock: the Session passes the time in, so the timing is plain arithmetic.
//!
//! Waits start at 1 s and double after each failed attempt, up to a minute, for as many
//! attempts as Settings → Connections → Reconnect attempts allows. Stop, or running out
//! of attempts, leaves the Session waiting for the operator's Reconnect, which starts the
//! attempts over.

use crate::settings::ReconnectAttempts;

/// The longest wait between two attempts, in seconds.
pub const LONGEST_WAIT: f64 = 60.0;

/// A Disconnected Session's reconnecting.
#[derive(Clone, Debug, PartialEq)]
pub struct Lost {
    reason: String,
    /// Attempts started since the drop, or since the operator's Reconnect.
    tries: u32,
    state: State,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    /// The next attempt starts at this time.
    Waiting(f64),
    /// An attempt is running.
    Trying,
    /// No attempt until the operator asks.
    Stopped,
}

/// The wait before attempt `n`, counted from 1.
fn wait_before(n: u32) -> f64 {
    2f64.powi(n.saturating_sub(1).min(16) as i32)
        .min(LONGEST_WAIT)
}

impl Lost {
    /// The connection dropped at `now`, for `reason`.
    pub fn new(reason: String, attempts: ReconnectAttempts, now: f64) -> Lost {
        let mut lost = Lost {
            reason,
            tries: 0,
            state: State::Stopped,
        };
        lost.next(attempts, now);
        lost
    }

    /// Waits for the next attempt, or stops when there are no more.
    fn next(&mut self, attempts: ReconnectAttempts, now: f64) {
        self.state = match attempts.limit() {
            Some(limit) if self.tries >= limit => State::Stopped,
            _ => State::Waiting(now + wait_before(self.tries + 1)),
        };
    }

    /// Why the connection dropped.
    pub fn reason(&self) -> &str {
        &self.reason
    }

    /// The strip's first line: "Connection lost: <reason>".
    pub fn headline(&self) -> String {
        format!("Connection lost: {}", self.reason)
    }

    /// Whether an attempt should start at `now`; if so, it counts as started.
    pub fn due(&mut self, now: f64) -> bool {
        match self.state {
            State::Waiting(at) if now >= at => {
                self.start();
                true
            }
            _ => false,
        }
    }

    fn start(&mut self) {
        self.tries += 1;
        self.state = State::Trying;
    }

    /// The attempt running failed at `now`: wait for the next, or stop.
    pub fn failed(&mut self, attempts: ReconnectAttempts, now: f64) {
        self.next(attempts, now);
    }

    /// Reconnect now, or Reconnect once stopped (which starts the attempts over): an
    /// attempt starts at once.
    pub fn now(&mut self) {
        if self.state == State::Stopped {
            self.tries = 0;
        }
        self.start();
    }

    /// Stop: no more attempts until the operator asks.
    pub fn stop(&mut self) {
        self.state = State::Stopped;
    }

    pub fn is_trying(&self) -> bool {
        self.state == State::Trying
    }

    pub fn is_stopped(&self) -> bool {
        self.state == State::Stopped
    }

    /// The seconds from `now` until the next attempt; `None` unless one is waiting.
    pub fn wait(&self, now: f64) -> Option<f64> {
        match self.state {
            State::Waiting(at) => Some((at - now).max(0.0)),
            _ => None,
        }
    }

    /// What the strip says of reconnecting, such as "Reconnecting in 8 s (attempt 3 of
    /// 5)…"; `None` once stopped.
    pub fn status(&self, now: f64, attempts: ReconnectAttempts) -> Option<String> {
        let of = match attempts.limit() {
            Some(limit) => format!("attempt {} of {}", self.tries.max(1), limit.max(1)),
            None => format!("attempt {}", self.tries.max(1)),
        };
        match self.state {
            State::Waiting(at) => {
                let next = match attempts.limit() {
                    Some(limit) => format!("attempt {} of {limit}", self.tries + 1),
                    None => format!("attempt {}", self.tries + 1),
                };
                let seconds = (at - now).max(0.0).ceil() as u64;
                Some(format!("Reconnecting in {seconds} s ({next})…"))
            }
            State::Trying => Some(format!("Reconnecting ({of})…")),
            State::Stopped => None,
        }
    }
}
