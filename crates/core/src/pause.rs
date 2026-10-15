//! Pausing capture, with automatic resume (SPEC §7).

const MINUTE_MS: i64 = 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PauseState {
    #[default]
    Running,
    Until(i64),
    Indefinitely,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseDuration {
    Minutes15,
    Hour1,
    Hours8,
    UntilResumed,
}

impl PauseDuration {
    pub const ALL: [PauseDuration; 4] = [
        PauseDuration::Minutes15,
        PauseDuration::Hour1,
        PauseDuration::Hours8,
        PauseDuration::UntilResumed,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PauseDuration::Minutes15 => "For 15 Minutes",
            PauseDuration::Hour1 => "For 1 Hour",
            PauseDuration::Hours8 => "For 8 Hours",
            PauseDuration::UntilResumed => "Until Resumed",
        }
    }

    pub fn start(self, now_ms: i64) -> PauseState {
        match self {
            PauseDuration::Minutes15 => PauseState::Until(now_ms + 15 * MINUTE_MS),
            PauseDuration::Hour1 => PauseState::Until(now_ms + 60 * MINUTE_MS),
            PauseDuration::Hours8 => PauseState::Until(now_ms + 8 * 60 * MINUTE_MS),
            PauseDuration::UntilResumed => PauseState::Indefinitely,
        }
    }
}

impl PauseState {
    pub fn is_paused(self, now_ms: i64) -> bool {
        match self {
            PauseState::Running => false,
            PauseState::Until(resume_at) => now_ms < resume_at,
            PauseState::Indefinitely => true,
        }
    }

    /// Switches an expired timed pause back to `Running`. Returns true when
    /// that happened, so the caller can announce "resumed".
    pub fn expire(&mut self, now_ms: i64) -> bool {
        if let PauseState::Until(resume_at) = *self
            && now_ms >= resume_at
        {
            *self = PauseState::Running;
            return true;
        }
        false
    }

    pub fn resume_at(self) -> Option<i64> {
        match self {
            PauseState::Until(t) => Some(t),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_pause_expires() {
        let mut state = PauseDuration::Minutes15.start(1_000);
        assert!(state.is_paused(1_000));
        assert!(state.is_paused(1_000 + 15 * MINUTE_MS - 1));
        assert!(!state.is_paused(1_000 + 15 * MINUTE_MS));
        assert!(!state.expire(1_000 + MINUTE_MS));
        assert!(state.expire(1_000 + 15 * MINUTE_MS));
        assert_eq!(state, PauseState::Running);
        assert!(!state.expire(i64::MAX));
    }

    #[test]
    fn indefinite_pause() {
        let mut state = PauseDuration::UntilResumed.start(0);
        assert!(state.is_paused(i64::MAX));
        assert!(!state.expire(i64::MAX));
        assert_eq!(state.resume_at(), None);
        assert_eq!(
            PauseDuration::Hours8.start(0).resume_at(),
            Some(8 * 60 * MINUTE_MS)
        );
        assert!(!PauseState::Running.is_paused(0));
    }
}
