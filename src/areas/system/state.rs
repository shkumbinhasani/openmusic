use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, Instant},
};

use gpui::{App, Context, Keystroke, Window};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc::unbounded_channel;

use super::{
    now_playing::{self, Snapshot},
    shortcuts,
    worker::Event,
};
use crate::{
    app::{OpenMusic, Page, Session, intent::Intent},
    menus::{
        GoBack, GoForward, GoHome, Handle, HideApp, HideOthers, Minimize, Next, PlayPause,
        Previous, SeekBackward, SeekForward, ShowAll, ShowShortcuts, ToggleFullScreen,
        TogglePlayback, VolumeDown, VolumeUp, Zoom, route,
    },
    ui::tracks::{RowDrag, RowDrop},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemPrefs {}

pub const TRACK_SEEK_MS: i32 = 5_000;
pub const EPISODE_SEEK_MS: i32 = 15_000;
pub const VOLUME_STEP: i32 = 4096;
pub const THROTTLE: Duration = Duration::from_millis(100);

#[derive(Default)]
pub struct Throttle {
    seek: Option<Instant>,
    volume: Option<Instant>,
}

pub fn allow(last: &mut Option<Instant>, now: Instant) -> bool {
    if last.is_some_and(|at| now.saturating_duration_since(at) < THROTTLE) {
        return false;
    }
    *last = Some(now);
    true
}

pub fn seek_step(uri: Option<&str>, episode: bool) -> i32 {
    if episode || uri.is_some_and(|uri| uri.starts_with("spotify:episode:")) {
        EPISODE_SEEK_MS
    } else {
        TRACK_SEEK_MS
    }
}

#[derive(Default)]
pub struct State {
    pub published: Option<(Snapshot, Instant)>,
    pub waiting: Option<bool>,
    pub publishes: u64,
    pub throttle: Throttle,
    pub shortcuts: bool,
}

impl State {
    pub fn covers(&self, _urls: &mut HashSet<String>) {}
}

pub fn install(cx: &mut App) {
    cx.on_window_closed(|cx| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();
}

pub fn window_opened(_window: &mut Window, _cx: &mut App) {}

impl OpenMusic {
    pub fn show_main_window(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.activate_window();
    }

    pub fn toggle_mini_player(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    pub fn act(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        self.perform(intent, window, cx);
        self.sync_system();
    }

    pub fn seek_key(&mut self, direction: i32, window: &mut Window, cx: &mut Context<Self>) {
        if !allow(&mut self.system.throttle.seek, Instant::now()) {
            return;
        }
        let step = seek_step(
            self.track().map(|track| track.uri.as_str()),
            self.playback.now.episode,
        );
        self.act(Intent::SeekBy(direction * step), window, cx);
    }

    pub fn volume_key(&mut self, direction: i32, window: &mut Window, cx: &mut Context<Self>) {
        if !allow(&mut self.system.throttle.volume, Instant::now()) {
            return;
        }
        self.act(Intent::VolumeBy(direction * VOLUME_STEP), window, cx);
    }

    pub fn now_playing_snapshot(&self) -> Option<Snapshot> {
        if !matches!(self.session, Session::Connected) {
            return None;
        }
        let now = &self.playback.now;
        let track = now.track.as_ref()?;
        Some(Snapshot {
            uri: track.uri.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            duration_ms: track.duration_ms,
            position_ms: now.position(),
            playing: now.playing,
            speed: if now.speed > 0. { now.speed } else { 1. },
            cover: track
                .cover
                .clone()
                .filter(|url| self.covers.contains_key(url)),
        })
    }

    pub fn sync_system(&mut self) {
        self.sync_now_playing();
    }

    fn sync_now_playing(&mut self) {
        match self.now_playing_snapshot() {
            None => {
                let shown = self.system.published.take().is_some();
                if self.system.waiting.take().is_some() || shown {
                    now_playing::clear();
                }
            }
            Some(next) if next.title.is_empty() => {
                if self.system.waiting != Some(next.playing) {
                    now_playing::set_state(next.playing);
                    self.system.waiting = Some(next.playing);
                }
            }
            Some(next) => {
                let last = self.system.published.as_ref();
                let elapsed = last.map_or(0, |(_, at)| at.elapsed().as_millis() as u64);
                if now_playing::should_publish(last.map(|(snapshot, _)| snapshot), &next, elapsed) {
                    let cover = next.cover.as_ref().and_then(|url| self.covers.get(url));
                    now_playing::publish(&next, cover);
                    self.system.published = Some((next, Instant::now()));
                    self.system.waiting = None;
                    self.system.publishes += 1;
                }
            }
        }
    }
}

pub fn on_event(
    _app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {}
}

pub fn connected(app: &mut OpenMusic, window: &mut Window, cx: &mut Context<OpenMusic>) {
    if !now_playing::installed() {
        let (intents, mut inbox) = unbounded_channel();
        now_playing::install(move |intent| {
            let _ = intents.send(intent);
        });
        cx.spawn_in(window, async move |view, window| {
            while let Some(intent) = inbox.recv().await {
                if view
                    .update_in(window, |app, window, cx| app.act(intent, window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }
    app.sync_system();
}

pub fn signed_out(app: &mut OpenMusic) {
    app.sync_system();
}

fn write(directory: &Path, name: &str, contents: String) {
    let _ = std::fs::write(directory.join(name), contents);
}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match verb {
        "nowplaying" => {
            app.sync_system();
            let mut dump = now_playing::dump();
            dump["published"] = json!(app.system.publishes);
            let text = serde_json::to_string_pretty(&dump).unwrap_or_default();
            write(directory, "nowplaying.json", text);
        }
        "remote" => {
            let status = now_playing::parse_remote(rest)
                .map_or("unknown", |intent| now_playing::fire(intent).name());
            write(directory, "remote.txt", format!("{rest} {status}"));
        }
        "key" => {
            let Ok(keystroke) = Keystroke::parse(rest) else {
                write(directory, "key.txt", format!("{rest} invalid"));
                return true;
            };
            window.defer(cx, move |window, cx| {
                let handled = window.dispatch_keystroke(keystroke, cx);
                if let Some(directory) = crate::debug::directory() {
                    write(&directory, "key.txt", format!("handled={handled}"));
                }
            });
        }
        "shortcuts" => {
            match rest {
                "" => app.toggle_shortcuts(window, cx),
                "open" => app.open_shortcuts(window, cx),
                "close" => app.close_shortcuts(window, cx),
                "dump" => {}
                _ => return false,
            }
            let groups = app.shortcut_groups(cx);
            write(
                directory,
                "shortcuts.txt",
                format!(
                    "open={}\n{}",
                    app.system.shortcuts,
                    shortcuts::describe(&groups)
                ),
            );
        }
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    format!(
        "published={} shortcuts={}",
        app.system.publishes, app.system.shortcuts
    )
}

pub fn register_actions(cx: &mut App, handle: &Handle) {
    route(cx, handle, |_: &PlayPause, app, window, cx| {
        app.act(Intent::PlayPause, window, cx)
    });
    route(cx, handle, |_: &TogglePlayback, app, window, cx| {
        app.act(Intent::PlayPause, window, cx)
    });
    route(cx, handle, |_: &Next, app, window, cx| {
        app.act(Intent::Next, window, cx)
    });
    route(cx, handle, |_: &Previous, app, window, cx| {
        app.act(Intent::Previous, window, cx)
    });
    route(cx, handle, |_: &SeekForward, app, window, cx| {
        app.seek_key(1, window, cx)
    });
    route(cx, handle, |_: &SeekBackward, app, window, cx| {
        app.seek_key(-1, window, cx)
    });
    route(cx, handle, |_: &VolumeUp, app, window, cx| {
        app.volume_key(1, window, cx)
    });
    route(cx, handle, |_: &VolumeDown, app, window, cx| {
        app.volume_key(-1, window, cx)
    });
    route(cx, handle, |_: &GoBack, app, window, cx| {
        app.perform(Intent::Back, window, cx)
    });
    route(cx, handle, |_: &GoForward, app, window, cx| {
        app.perform(Intent::Forward, window, cx)
    });
    route(cx, handle, |_: &GoHome, app, window, cx| {
        app.perform(Intent::Navigate(Page::Home), window, cx)
    });
    route(cx, handle, |_: &ShowShortcuts, app, window, cx| {
        app.toggle_shortcuts(window, cx)
    });
    route(cx, handle, |_: &ToggleFullScreen, _, window, _| {
        window.toggle_fullscreen()
    });
    route(cx, handle, |_: &Minimize, _, window, _| {
        window.minimize_window()
    });
    route(cx, handle, |_: &Zoom, _, window, _| window.zoom_window());
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
}

pub fn on_press(
    _app: &mut OpenMusic,
    _key: &'static str,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
}

pub fn row_dropped(
    _app: &mut OpenMusic,
    _drag: &RowDrag,
    _at: RowDrop,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
}

pub fn tick(app: &mut OpenMusic, _cx: &mut Context<OpenMusic>) {
    app.sync_system();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_steps_follow_the_kind_of_item() {
        assert_eq!(
            seek_step(Some("spotify:track:4uLU6hMCjMI75M1A2tKUQC"), false),
            5_000
        );
        assert_eq!(
            seek_step(Some("spotify:episode:512ojhOuo1ktJprKbVcKyQ"), false),
            15_000
        );
        assert_eq!(
            seek_step(Some("spotify:track:4uLU6hMCjMI75M1A2tKUQC"), true),
            15_000
        );
        assert_eq!(seek_step(None, false), 5_000);
    }

    #[test]
    fn repeats_closer_than_the_throttle_are_dropped() {
        let start = Instant::now();
        let mut last = None;
        assert!(allow(&mut last, start));
        assert!(!allow(&mut last, start + Duration::from_millis(40)));
        assert!(!allow(&mut last, start + Duration::from_millis(99)));
        assert!(allow(&mut last, start + Duration::from_millis(100)));
        assert!(!allow(&mut last, start + Duration::from_millis(150)));
        assert!(allow(&mut last, start + Duration::from_millis(230)));
    }

    #[test]
    fn four_volume_steps_make_a_quarter() {
        assert_eq!(VOLUME_STEP * 4, 16_384);
        assert_eq!(VOLUME_STEP * 16, 65_536);
    }
}
