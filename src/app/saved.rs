use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use spotify_client::library;

use crate::{app::OpenMusic, worker::saved};

#[derive(Default)]
pub struct Saved {
    pub known: HashMap<String, bool>,
    pub pending: HashMap<String, Option<bool>>,
    requested: HashSet<String>,
    settled: HashMap<String, Instant>,
}

impl Saved {
    pub fn learn(&mut self, uri: String, saved: bool, asked: Option<Instant>) {
        let overtaken =
            asked.is_some_and(|asked| self.settled.get(&uri).is_some_and(|at| *at > asked));
        if !overtaken && !self.pending.contains_key(&uri) {
            self.known.insert(uri, saved);
        }
    }

    fn settle(&mut self, uri: String, saved: bool) {
        self.pending.remove(&uri);
        self.settled.insert(uri.clone(), Instant::now());
        self.known.insert(uri, saved);
    }
}

impl OpenMusic {
    pub fn is_saved(&self, uri: &str) -> Option<bool> {
        self.saved.known.get(uri).copied()
    }

    pub fn lookup_saved(&mut self, uris: impl IntoIterator<Item = String>) {
        let wanted: Vec<String> = uris
            .into_iter()
            .filter(|uri| {
                library::savable(uri)
                    && !self.saved.known.contains_key(uri)
                    && !self.saved.pending.contains_key(uri)
                    && !self.saved.requested.contains(uri)
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if wanted.is_empty() {
            return;
        }
        self.saved.requested.extend(wanted.iter().cloned());
        self.send(saved::Command::Lookup(wanted));
    }

    pub fn set_saved(&mut self, uris: Vec<String>, saved: bool) {
        let uris: Vec<String> = uris
            .into_iter()
            .filter(|uri| library::savable(uri) && !self.saved.pending.contains_key(uri))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if uris.is_empty() {
            return;
        }
        for uri in &uris {
            let before = self.saved.known.insert(uri.clone(), saved);
            self.saved.pending.insert(uri.clone(), before);
        }
        self.send(saved::Command::Set { uris, saved });
    }

    pub(super) fn on_saved(&mut self, event: saved::Event) {
        match event {
            saved::Event::Known { found, asked } => {
                for (uri, saved) in found {
                    self.saved.requested.remove(&uri);
                    if let Some(saved) = saved {
                        self.saved.learn(uri, saved, asked);
                    }
                }
            }
            saved::Event::Written { uris, saved } => {
                for uri in uris {
                    self.saved.settle(uri, saved);
                }
            }
        }
    }

    pub(super) fn saved_failed(&mut self, keys: &[String]) {
        for key in keys {
            match self.saved.pending.remove(key) {
                Some(Some(before)) => {
                    self.saved.known.insert(key.clone(), before);
                }
                Some(None) => {
                    self.saved.known.remove(key);
                }
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const SONG: &str = "spotify:track:70LcF31zb1H0PyJoS1Sx1r";

    #[test]
    fn an_answer_read_before_a_write_settled_does_not_undo_it() {
        let mut saved = Saved::default();
        let before_write = Instant::now() - Duration::from_millis(5);
        saved.pending.insert(SONG.into(), Some(true));
        saved.learn(SONG.into(), true, Some(before_write));
        assert_eq!(saved.known.get(SONG), None);
        saved.settle(SONG.into(), false);
        saved.learn(SONG.into(), true, Some(before_write));
        assert_eq!(saved.known.get(SONG), Some(&false));
        saved.learn(
            SONG.into(),
            true,
            Some(Instant::now() + Duration::from_millis(5)),
        );
        assert_eq!(saved.known.get(SONG), Some(&true));
        saved.learn(SONG.into(), false, None);
        assert_eq!(saved.known.get(SONG), Some(&false));
    }
}
