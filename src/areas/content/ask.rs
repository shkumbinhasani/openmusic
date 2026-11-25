use std::time::{Duration, Instant};

pub const ANSWER_WITHIN: Duration = Duration::from_secs(20);
pub const RETRY_AFTER: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Asks {
    pending: Option<(String, Instant)>,
    failed: Option<(String, Instant)>,
}

impl Asks {
    pub fn asking(&self, uri: &str, now: Instant) -> bool {
        self.pending.as_ref().is_some_and(|(pending, at)| {
            pending == uri && now.saturating_duration_since(*at) < ANSWER_WITHIN
        })
    }

    pub fn due(&self, uri: &str, now: Instant) -> bool {
        !self.asking(uri, now)
            && !self.failed.as_ref().is_some_and(|(failed, at)| {
                failed == uri && now.saturating_duration_since(*at) < RETRY_AFTER
            })
    }

    pub fn ask(&mut self, uri: &str, now: Instant) -> bool {
        let due = self.due(uri, now);
        if due {
            self.pending = Some((uri.to_owned(), now));
        }
        due
    }

    pub fn answered(&mut self, uri: &str) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| pending == uri)
        {
            self.pending = None;
        }
    }

    pub fn fail(&mut self, uri: String, at: Instant) {
        self.answered(&uri);
        self.failed = Some((uri, at));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "spotify:track:00000000000000000000a1";
    const B: &str = "spotify:track:00000000000000000000b1";

    #[test]
    fn waits_for_an_answer_then_asks_again() {
        let mut asks = Asks::default();
        let now = Instant::now();
        assert!(asks.ask(A, now));
        assert!(asks.asking(A, now));
        assert!(!asks.ask(A, now));
        assert!(asks.ask(A, now + ANSWER_WITHIN));
        asks.answered(B);
        assert!(asks.asking(A, now + ANSWER_WITHIN));
        asks.answered(A);
        assert!(!asks.asking(A, now + ANSWER_WITHIN));
    }

    #[test]
    fn a_newer_ask_replaces_the_one_it_cancels() {
        let mut asks = Asks::default();
        let now = Instant::now();
        assert!(asks.ask(A, now));
        assert!(asks.ask(B, now));
        assert!(!asks.asking(A, now));
        assert!(asks.ask(A, now));
    }

    #[test]
    fn asks_again_a_while_after_a_failure() {
        let mut asks = Asks::default();
        let now = Instant::now();
        assert!(asks.ask(A, now));
        asks.fail(A.into(), now);
        assert!(!asks.asking(A, now));
        assert!(!asks.due(A, now + Duration::from_secs(5)));
        assert!(!asks.ask(A, now + Duration::from_secs(5)));
        assert!(asks.due(B, now));
        assert!(asks.ask(A, now + RETRY_AFTER));
    }
}
