//! How long records are kept, and the clock that measures it.

use std::time::{SystemTime, UNIX_EPOCH};

/// "Keep errors for", stored as `error_journal.retention` in the IDE config.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Retention {
    /// Records of earlier sessions are deleted at startup (unseen crashes excepted).
    UntilRestart,
    Day,
    #[default]
    Week,
    Month,
}

impl Retention {
    pub const ALL: [Self; 4] = [Self::UntilRestart, Self::Day, Self::Week, Self::Month];

    /// The config value; anything unknown falls back to the default (a week).
    pub fn from_setting(value: &str) -> Self {
        match value.trim() {
            "restart" => Self::UntilRestart,
            "1d" => Self::Day,
            "7d" => Self::Week,
            "30d" => Self::Month,
            _ => Self::default(),
        }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::UntilRestart => "restart",
            Self::Day => "1d",
            Self::Week => "7d",
            Self::Month => "30d",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::UntilRestart => "Until restart",
            Self::Day => "1 day",
            Self::Week => "1 week",
            Self::Month => "1 month",
        }
    }

    /// Age limit in seconds, measured from `last_seen`; `None` for "until restart".
    pub fn max_age(self) -> Option<u64> {
        const DAY: u64 = 86_400;
        match self {
            Self::UntilRestart => None,
            Self::Day => Some(DAY),
            Self::Week => Some(7 * DAY),
            Self::Month => Some(30 * DAY),
        }
    }
}

/// Unix seconds; injected so time-based rules can be tested.
pub trait Clock: Send + Sync + std::fmt::Debug {
    fn now(&self) -> u64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

#[cfg(test)]
mod tests {
    use super::Retention;

    #[test]
    fn invalid_values_fall_back_to_a_week() {
        assert_eq!(Retention::from_setting("forever"), Retention::Week);
        assert_eq!(Retention::from_setting(""), Retention::Week);
        for retention in Retention::ALL {
            assert_eq!(Retention::from_setting(retention.as_setting()), retention);
        }
    }
}
