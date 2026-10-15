//! "Keep history" setting (SPEC §5).

use serde::{Deserialize, Serialize};

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Retention {
    Day,
    Week,
    #[default]
    Month,
    Year,
    Forever,
}

impl Retention {
    pub const ALL: [Retention; 5] = [
        Retention::Day,
        Retention::Week,
        Retention::Month,
        Retention::Year,
        Retention::Forever,
    ];

    pub fn duration_ms(self) -> Option<i64> {
        match self {
            Retention::Day => Some(DAY_MS),
            Retention::Week => Some(7 * DAY_MS),
            Retention::Month => Some(30 * DAY_MS),
            Retention::Year => Some(365 * DAY_MS),
            Retention::Forever => None,
        }
    }

    /// History items with `timestamp < cutoff` are expired.
    pub fn cutoff(self, now_ms: i64) -> Option<i64> {
        self.duration_ms().map(|d| now_ms - d)
    }

    pub fn label(self) -> &'static str {
        match self {
            Retention::Day => "Day",
            Retention::Week => "Week",
            Retention::Month => "Month",
            Retention::Year => "Year",
            Retention::Forever => "Forever",
        }
    }

    /// True when switching from `self` to `other` can delete items.
    pub fn is_shorter_than(self, other: Retention) -> bool {
        match (self.duration_ms(), other.duration_ms()) {
            (Some(a), Some(b)) => a < b,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cutoffs() {
        let now = 100 * DAY_MS;
        assert_eq!(Retention::Day.cutoff(now), Some(99 * DAY_MS));
        assert_eq!(Retention::Week.cutoff(now), Some(93 * DAY_MS));
        assert_eq!(Retention::Forever.cutoff(now), None);
    }

    #[test]
    fn ordering() {
        assert!(Retention::Day.is_shorter_than(Retention::Week));
        assert!(Retention::Year.is_shorter_than(Retention::Forever));
        assert!(!Retention::Forever.is_shorter_than(Retention::Day));
        assert!(!Retention::Month.is_shorter_than(Retention::Month));
    }

    #[test]
    fn serde_names() {
        assert_eq!(serde_json::to_string(&Retention::Week).unwrap(), "\"week\"");
        let r: Retention = serde_json::from_str("\"forever\"").unwrap();
        assert_eq!(r, Retention::Forever);
    }
}
