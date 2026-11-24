use std::collections::{HashMap, HashSet};

use spotify_client::library;

use crate::{app::OpenMusic, worker::saved};

#[derive(Default)]
pub struct Saved {
    pub known: HashMap<String, bool>,
    pub pending: HashMap<String, Option<bool>>,
    requested: HashSet<String>,
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

    pub fn toggle_like(&mut self) {
        let Some(uri) = self.track().map(|track| track.uri.clone()) else {
            return;
        };
        match self.is_saved(&uri) {
            Some(saved) => self.set_saved(vec![uri], !saved),
            None => self.lookup_saved([uri]),
        }
    }

    pub(super) fn on_saved(&mut self, event: saved::Event) {
        match event {
            saved::Event::Known(found) => {
                for (uri, saved) in found {
                    self.saved.requested.remove(&uri);
                    if let Some(saved) = saved
                        && !self.saved.pending.contains_key(&uri)
                    {
                        self.saved.known.insert(uri, saved);
                    }
                }
            }
            saved::Event::Written { uris, saved } => {
                for uri in uris {
                    self.saved.pending.remove(&uri);
                    self.saved.known.insert(uri, saved);
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
