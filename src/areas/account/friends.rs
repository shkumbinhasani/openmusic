use std::{
    cell::Cell,
    collections::HashSet,
    path::Path,
    time::{Duration, Instant},
};

use gpui::{Context, Window, px};
use spotify_client::social::Friend;

use super::worker::Command;
use crate::{
    app::{OpenMusic, Session},
    ui::{ROOMY_WIDTH, tracks::now_ms},
};

pub const POLL: Duration = Duration::from_secs(60);
pub const BACK_OFF: Duration = Duration::from_secs(300);
pub const GRACE: Duration = Duration::from_secs(15);
pub const PANEL_WIDTH: f32 = 280.;
pub const LOAD_FAILED: &str = "Couldn’t load Friend Activity.";
pub const NARROW: &str = "Make the window wider to see Friend Activity.";

#[derive(Default)]
pub struct Friends {
    pub items: Vec<Friend>,
    pub loaded: bool,
    pub error: Option<String>,
    pub loading: bool,
    pub asked: Option<Instant>,
    pub slow: bool,
    pub drawn: Cell<Option<Instant>>,
}

impl Friends {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        urls.extend(self.items.iter().flat_map(Friend::images));
    }

    pub fn wait(&self) -> Duration {
        if self.slow { BACK_OFF } else { POLL }
    }

    pub fn shown(&self, now: Instant) -> bool {
        self.drawn
            .get()
            .is_some_and(|at| now.saturating_duration_since(at) < self.wait() + GRACE)
    }

    pub fn due(&self, now: Instant) -> bool {
        !self.loading
            && self
                .asked
                .is_none_or(|at| now.saturating_duration_since(at) >= self.wait())
    }

    pub fn ask(&mut self, now: Instant) {
        self.loading = true;
        self.asked = Some(now);
    }

    pub fn answer(&mut self, result: Result<Vec<Friend>, String>, slow: bool) {
        self.loading = false;
        self.slow = slow;
        match result {
            Ok(items) => {
                self.items = items;
                self.loaded = true;
                self.error = None;
            }
            Err(message) => self.error = Some(message),
        }
    }
}

pub fn ago(timestamp_ms: i64, now_ms: i64) -> String {
    let minutes = (now_ms.saturating_sub(timestamp_ms) / 60_000).max(1);
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes < 60 {
        format!("{minutes} m")
    } else if hours < 24 {
        format!("{hours} hr")
    } else if days < 7 {
        format!("{days} d")
    } else {
        format!("{} w", days / 7)
    }
}

impl OpenMusic {
    pub fn friends_on(&self) -> bool {
        self.settings.account.show_friends
    }

    pub fn set_friend_activity(&mut self, show: bool) {
        self.update_settings(|settings| settings.account.show_friends = show);
        if !show {
            self.account.friends.drawn.set(None);
        }
    }

    pub fn toggle_friend_activity(&mut self, window: &Window) {
        let show = !self.friends_on();
        self.set_friend_activity(show);
        if show && window.viewport_size().width < px(ROOMY_WIDTH) {
            self.show_toast(NARROW, None);
        }
    }

    pub fn refresh_friends(&mut self) {
        let friends = &mut self.account.friends;
        if friends.loading {
            return;
        }
        friends.ask(Instant::now());
        self.send(Command::Friends);
    }
}

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    if !app.friends_on() || !matches!(app.session, Session::Connected) {
        return;
    }
    let now = Instant::now();
    let friends = &app.account.friends;
    if !friends.shown(now) || !friends.due(now) {
        return;
    }
    app.refresh_friends();
    cx.notify();
}

pub fn on_friends(app: &mut OpenMusic, result: Result<Vec<Friend>, String>, slow: bool) {
    app.account.friends.answer(result, slow);
}

pub fn describe(app: &OpenMusic) -> String {
    let friends = &app.account.friends;
    let now = now_ms();
    let mut lines = vec![format!(
        "setting={} shown={} loaded={} loading={} count={} wait={} error={}",
        if app.friends_on() { "on" } else { "off" },
        friends.shown(Instant::now()),
        friends.loaded,
        friends.loading,
        friends.items.len(),
        friends.wait().as_secs(),
        friends.error.as_deref().unwrap_or("none"),
    )];
    lines.extend(friends.items.iter().map(|friend| {
        format!(
            "{} | {} | playing={} | {} | {} | {} | {} | {}",
            friend.user.uri,
            friend.user.name,
            friend.playing_now(now),
            friend.track.uri,
            friend.track.title,
            friend.track.artist,
            friend.context_uri.as_deref().unwrap_or("-"),
            ago(friend.timestamp_ms, now),
        )
    }));
    lines.join("\n")
}

pub fn debug(app: &mut OpenMusic, rest: &str, directory: &Path) {
    match rest {
        "on" => app.set_friend_activity(true),
        "off" => app.set_friend_activity(false),
        "refresh" => app.refresh_friends(),
        _ => {}
    }
    let _ = std::fs::write(directory.join("friends.txt"), describe(app));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polls_every_minute_while_the_panel_is_drawn() {
        let start = Instant::now();
        let mut friends = Friends::default();
        assert!(!friends.shown(start));
        assert!(friends.due(start));
        friends.drawn.set(Some(start));
        assert!(friends.shown(start));
        friends.ask(start);
        assert!(!friends.due(start + POLL));
        friends.answer(Ok(Vec::new()), false);
        assert!(friends.loaded);
        assert!(!friends.due(start + POLL - Duration::from_secs(1)));
        assert!(friends.due(start + POLL));
        assert!(friends.shown(start + POLL + GRACE - Duration::from_secs(1)));
        assert!(!friends.shown(start + POLL + GRACE));
    }

    #[test]
    fn backs_off_after_a_rate_limit_and_keeps_the_last_list() {
        let start = Instant::now();
        let mut friends = Friends::default();
        friends.answer(Ok(vec![Friend::default()]), false);
        friends.ask(start);
        friends.answer(Err(LOAD_FAILED.into()), true);
        assert_eq!(friends.items.len(), 1);
        assert_eq!(friends.error.as_deref(), Some(LOAD_FAILED));
        assert_eq!(friends.wait(), BACK_OFF);
        assert!(!friends.due(start + POLL));
        assert!(friends.due(start + BACK_OFF));
        friends.answer(Ok(Vec::new()), false);
        assert_eq!(friends.wait(), POLL);
        assert!(friends.error.is_none());
    }

    #[test]
    fn labels_how_long_ago() {
        let now = 10_000_000_000;
        assert_eq!(ago(now, now), "1 m");
        assert_eq!(ago(now - 5 * 60_000, now), "5 m");
        assert_eq!(ago(now - 2 * 3_600_000, now), "2 hr");
        assert_eq!(ago(now - 3 * 86_400_000, now), "3 d");
        assert_eq!(ago(now - 15 * 86_400_000, now), "2 w");
        assert_eq!(ago(now + 60_000, now), "1 m");
    }
}
