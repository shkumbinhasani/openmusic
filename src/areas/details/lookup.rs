use std::{collections::HashMap, time::Instant};

use super::feedback::RETRY_AFTER;

enum Answer<T> {
    Asked,
    Found(T),
    Failed(Instant),
}

pub struct Lookups<T>(HashMap<String, Answer<T>>);

impl<T> Default for Lookups<T> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<T> Lookups<T> {
    pub fn get(&self, key: &str) -> Option<&T> {
        match self.0.get(key)? {
            Answer::Found(value) => Some(value),
            Answer::Asked | Answer::Failed(_) => None,
        }
    }

    pub fn ask(&mut self, key: &str) -> bool {
        self.ask_at(key, Instant::now())
    }

    fn ask_at(&mut self, key: &str, now: Instant) -> bool {
        let due = match self.0.get(key) {
            None => true,
            Some(Answer::Failed(at)) => now.saturating_duration_since(*at) >= RETRY_AFTER,
            Some(Answer::Asked | Answer::Found(_)) => false,
        };
        if due {
            self.0.insert(key.to_owned(), Answer::Asked);
        }
        due
    }

    pub fn answer(&mut self, key: String, result: Result<T, String>) {
        let answer = match result {
            Ok(value) => Answer::Found(value),
            Err(_) => Answer::Failed(Instant::now()),
        };
        self.0.insert(key, answer);
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, Option<&T>)> {
        self.0.iter().map(|(key, answer)| match answer {
            Answer::Found(value) => (key, Some(value)),
            Answer::Asked | Answer::Failed(_) => (key, None),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYLIST: &str = "spotify:playlist:37i9dQZF1E8Redacted001";

    #[test]
    fn asks_once_and_keeps_what_it_finds() {
        let mut lookups = Lookups::default();
        assert!(lookups.ask(PLAYLIST));
        assert!(!lookups.ask(PLAYLIST));
        assert_eq!(lookups.get(PLAYLIST), None);
        lookups.answer(PLAYLIST.into(), Ok(true));
        assert_eq!(lookups.get(PLAYLIST), Some(&true));
        assert!(!lookups.ask_at(PLAYLIST, Instant::now() + RETRY_AFTER));
        assert_eq!(
            lookups.iter().collect::<Vec<_>>(),
            [(&PLAYLIST.into(), Some(&true))]
        );
    }

    #[test]
    fn asks_again_a_while_after_a_failure() {
        let mut lookups = Lookups::<bool>::default();
        assert!(lookups.ask(PLAYLIST));
        lookups.answer(PLAYLIST.into(), Err("offline".into()));
        assert_eq!(lookups.get(PLAYLIST), None);
        assert!(!lookups.ask(PLAYLIST));
        assert!(lookups.ask_at(PLAYLIST, Instant::now() + RETRY_AFTER));
        assert!(!lookups.ask_at(PLAYLIST, Instant::now() + RETRY_AFTER));
        lookups.answer(PLAYLIST.into(), Ok(false));
        assert_eq!(lookups.get(PLAYLIST), Some(&false));
    }
}
