//! When to ask GitHub again: fast while a run is moving and the panel is on screen, slow when
//! idle, never while the window is in the background; slower still on errors or a low rate limit.

use std::time::Duration;

use crate::github::RateLimit;

pub const ACTIVE: Duration = Duration::from_secs(4);
pub const IDLE: Duration = Duration::from_secs(30);
/// Hidden panel: only the HEAD badge needs to stay roughly current.
pub const HIDDEN: Duration = Duration::from_secs(60);
pub const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// Below this many requests left, poll no faster than [`IDLE`].
const LOW_REMAINING: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollInput {
    pub window_active: bool,
    pub panel_visible: bool,
    /// Some run or job shown is queued or in progress.
    pub any_active: bool,
    /// A run on HEAD or one started from the IDE is in progress: keep polling (slowly) even
    /// in the background, so its "finished" notification arrives.
    pub watching: bool,
    /// Failed polls in a row.
    pub failures: u32,
    pub rate: Option<RateLimit>,
    /// A rate-limit error said to wait until then (Unix seconds).
    pub blocked_until: Option<i64>,
    pub now_secs: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poll {
    /// Do not poll until something changes (window activation, panel shown).
    Paused,
    After(Duration),
}

pub fn next_poll(input: &PollInput) -> Poll {
    let mut wait = match (input.window_active, input.panel_visible, input.any_active) {
        (false, ..) if input.watching => HIDDEN,
        (false, ..) => return Poll::Paused,
        (true, true, true) => ACTIVE,
        (true, true, false) => IDLE,
        (true, false, _) => HIDDEN,
    };
    if input.failures > 0 {
        let factor = 1u32 << input.failures.min(8);
        wait = (wait * factor).min(MAX_BACKOFF).max(wait);
    }
    if let Some(rate) = input.rate {
        if rate.remaining == 0 {
            wait = wait.max(until(rate.reset_at, input.now_secs));
        } else if rate.remaining < LOW_REMAINING {
            wait = wait.max(IDLE);
        }
    }
    if let Some(blocked) = input.blocked_until {
        wait = wait.max(until(blocked, input.now_secs));
    }
    Poll::After(wait)
}

/// Time until `reset_at`, plus a second of slack; at least a minute when the reset time is unknown.
fn until(reset_at: i64, now_secs: i64) -> Duration {
    if reset_at <= 0 {
        return Duration::from_secs(60);
    }
    Duration::from_secs(u64::try_from(reset_at - now_secs).unwrap_or(0) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> PollInput {
        PollInput {
            window_active: true,
            panel_visible: true,
            any_active: false,
            watching: false,
            failures: 0,
            rate: None,
            blocked_until: None,
            now_secs: 1_000,
        }
    }

    #[test]
    fn adapts_to_visibility_and_activity() {
        assert_eq!(next_poll(&input()), Poll::After(IDLE));
        assert_eq!(
            next_poll(&PollInput {
                any_active: true,
                ..input()
            }),
            Poll::After(ACTIVE)
        );
        assert_eq!(
            next_poll(&PollInput {
                panel_visible: false,
                any_active: true,
                ..input()
            }),
            Poll::After(HIDDEN)
        );
        assert_eq!(
            next_poll(&PollInput {
                window_active: false,
                ..input()
            }),
            Poll::Paused
        );
        assert_eq!(
            next_poll(&PollInput {
                window_active: false,
                watching: true,
                ..input()
            }),
            Poll::After(HIDDEN)
        );
    }

    #[test]
    fn backs_off_exponentially_on_failures_up_to_a_cap() {
        let active = PollInput {
            any_active: true,
            ..input()
        };
        assert_eq!(
            next_poll(&PollInput {
                failures: 1,
                ..active
            }),
            Poll::After(ACTIVE * 2)
        );
        assert_eq!(
            next_poll(&PollInput {
                failures: 3,
                ..active
            }),
            Poll::After(ACTIVE * 8)
        );
        assert_eq!(
            next_poll(&PollInput {
                failures: 30,
                ..active
            }),
            Poll::After(MAX_BACKOFF)
        );
    }

    #[test]
    fn respects_the_rate_limit() {
        let rate = |remaining| {
            Some(RateLimit {
                remaining,
                limit: 5000,
                reset_at: 1_120,
            })
        };
        let active = PollInput {
            any_active: true,
            ..input()
        };
        assert_eq!(
            next_poll(&PollInput {
                rate: rate(4000),
                ..active
            }),
            Poll::After(ACTIVE)
        );
        assert_eq!(
            next_poll(&PollInput {
                rate: rate(50),
                ..active
            }),
            Poll::After(IDLE)
        );
        assert_eq!(
            next_poll(&PollInput {
                rate: rate(0),
                ..active
            }),
            Poll::After(Duration::from_secs(121))
        );
        assert_eq!(
            next_poll(&PollInput {
                blocked_until: Some(1_300),
                ..active
            }),
            Poll::After(Duration::from_secs(301))
        );
    }
}
