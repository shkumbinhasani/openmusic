use std::{
    cell::RefCell,
    collections::HashSet,
    time::{Duration, Instant},
};

use gpui::Subscription;
use spotify_client::library::LibraryPush;

use crate::{
    app::{OpenMusic, Session},
    worker::saved,
};

const PUSH_DELAY: Duration = Duration::from_millis(500);
const AWAY: Duration = Duration::from_secs(30);
const SEEN_LIMIT: usize = 512;

#[derive(Default)]
pub struct LiveSync {
    pub due: Option<Instant>,
    pub rootlist: bool,
    pub pushes: u32,
    pub last_push: Option<String>,
    pub focus_refreshes: u32,
    pub inactive_since: Option<Instant>,
    pub refreshed: Option<Instant>,
    pub seen: RefCell<HashSet<String>>,
    pub activation: Option<Subscription>,
}

impl LiveSync {
    pub fn saw(&self, uri: &str) {
        let mut seen = self.seen.borrow_mut();
        if seen.len() < SEEN_LIMIT && !seen.contains(uri) {
            seen.insert(uri.to_owned());
        }
    }

    pub fn schedule(&mut self, rootlist: bool, now: Instant) {
        self.rootlist |= rootlist;
        self.due.get_or_insert(now + PUSH_DELAY);
    }

    pub fn ready(&self, now: Instant) -> bool {
        self.due.is_some_and(|due| now >= due)
    }

    pub fn back_after_away(&mut self, active: bool, now: Instant) -> bool {
        if !active {
            self.inactive_since.get_or_insert(now);
            return false;
        }
        self.inactive_since
            .take()
            .is_some_and(|since| now.duration_since(since) >= AWAY)
    }
}

pub fn describe(push: &LibraryPush) -> String {
    match push {
        LibraryPush::Collection { set, liked_added } => {
            format!(
                "collection/{set}{}",
                if *liked_added { "+liked" } else { "" }
            )
        }
        LibraryPush::Rootlist => "rootlist".into(),
    }
}

impl OpenMusic {
    pub fn schedule_library_sync(&mut self, rootlist: bool) {
        self.library.sync.schedule(rootlist, Instant::now());
    }

    pub(super) fn on_library_push(&mut self, push: LibraryPush) {
        let sync = &mut self.library.sync;
        sync.pushes += 1;
        sync.last_push = Some(describe(&push));
        match push {
            LibraryPush::Collection { liked_added, .. } => {
                if liked_added && self.library.liked.loaded {
                    self.library.liked.stale = true;
                }
                self.schedule_library_sync(false);
            }
            LibraryPush::Rootlist => self.schedule_library_sync(true),
        }
    }

    pub(super) fn tick_sync(&mut self) {
        if !self.library.sync.ready(Instant::now())
            || !self.saved.pending.is_empty()
            || !self.library.pins.is_empty()
        {
            return;
        }
        let sync = &mut self.library.sync;
        let rootlist = std::mem::take(&mut sync.rootlist);
        sync.due = None;
        sync.refreshed = Some(Instant::now());
        self.library.resync = false;
        if rootlist {
            self.refresh_library();
        } else {
            self.reload_library_views();
        }
    }

    pub fn on_window_activation(&mut self, active: bool) {
        let back = self.library.sync.back_after_away(active, Instant::now());
        if back && matches!(self.session, Session::Connected) {
            self.refresh_on_focus();
        }
    }

    pub fn refresh_on_focus(&mut self) {
        self.library.sync.focus_refreshes += 1;
        self.schedule_library_sync(true);
        if self.library.liked.loaded {
            self.library.liked.stale = true;
        }
        if self.library.recent.loaded {
            self.library.recent.stale = true;
        }
        self.load_played();
        self.recheck_saved();
    }

    fn recheck_saved(&mut self) {
        let seen = std::mem::take(&mut *self.library.sync.seen.borrow_mut());
        let uris: Vec<String> = seen
            .into_iter()
            .filter(|uri| !self.saved.pending.contains_key(uri))
            .collect();
        if !uris.is_empty() {
            self.send(saved::Command::Lookup(uris));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_before_syncing_and_after_being_away() {
        let start = Instant::now();
        let mut sync = LiveSync::default();
        assert!(!sync.ready(start));
        sync.schedule(false, start);
        sync.schedule(true, start + Duration::from_millis(300));
        assert!(sync.rootlist);
        assert!(!sync.ready(start + Duration::from_millis(400)));
        assert!(sync.ready(start + PUSH_DELAY));
        assert!(!sync.back_after_away(true, start));
        assert!(!sync.back_after_away(false, start));
        assert!(!sync.back_after_away(false, start + Duration::from_secs(10)));
        assert!(!sync.back_after_away(true, start + Duration::from_secs(20)));
        assert!(!sync.back_after_away(false, start + Duration::from_secs(21)));
        assert!(sync.back_after_away(true, start + Duration::from_secs(60)));
        assert!(!sync.back_after_away(true, start + Duration::from_secs(61)));
        for index in 0..SEEN_LIMIT + 10 {
            sync.saw(&format!("spotify:track:{index}"));
        }
        sync.saw("spotify:track:0");
        assert_eq!(sync.seen.borrow().len(), SEEN_LIMIT);
        assert_eq!(
            describe(&LibraryPush::Collection {
                set: "collection".into(),
                liked_added: true
            }),
            "collection/collection+liked"
        );
        assert_eq!(describe(&LibraryPush::Rootlist), "rootlist");
    }
}
