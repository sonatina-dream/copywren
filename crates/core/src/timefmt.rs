//! Human-readable times for card headers and the pause indicator.

use chrono::{DateTime, Datelike, Local, TimeZone};

const MINUTE: i64 = 60 * 1000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// "Just now", "5 min ago", "3 hr ago", "Yesterday", "4 days ago", "Mar 3",
/// "Mar 3, 2024".
pub fn relative(then_ms: i64, now_ms: i64) -> String {
    let delta = (now_ms - then_ms).max(0);
    if delta < MINUTE {
        "Just now".into()
    } else if delta < HOUR {
        format!("{} min ago", delta / MINUTE)
    } else if delta < DAY {
        format!("{} hr ago", delta / HOUR)
    } else if delta < 2 * DAY {
        "Yesterday".into()
    } else if delta < 7 * DAY {
        format!("{} days ago", delta / DAY)
    } else {
        let then = local(then_ms);
        if then.year() == local(now_ms).year() {
            then.format("%b %-d").to_string()
        } else {
            then.format("%b %-d, %Y").to_string()
        }
    }
}

/// Local wall-clock time, e.g. "14:05".
pub fn clock(ms: i64) -> String {
    local(ms).format("%H:%M").to_string()
}

fn local(ms: i64) -> DateTime<Local> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(|| Local.timestamp_millis_opt(0).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets() {
        let now = 1_000 * DAY;
        assert_eq!(relative(now, now), "Just now");
        assert_eq!(relative(now + 5 * MINUTE, now), "Just now");
        assert_eq!(relative(now - 59 * 1000, now), "Just now");
        assert_eq!(relative(now - 5 * MINUTE, now), "5 min ago");
        assert_eq!(relative(now - 3 * HOUR, now), "3 hr ago");
        assert_eq!(relative(now - 30 * HOUR, now), "Yesterday");
        assert_eq!(relative(now - 4 * DAY, now), "4 days ago");
        assert!(!relative(now - 30 * DAY, now).contains("ago"));
    }

    #[test]
    fn clock_format() {
        let text = clock(0);
        assert_eq!(text.len(), 5);
        assert_eq!(&text[2..3], ":");
    }
}
