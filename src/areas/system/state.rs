use std::{collections::HashSet, path::Path, time::Instant};

use gpui::{App, Context, Window};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::mpsc::unbounded_channel;

use super::{
    now_playing::{self, Snapshot},
    worker::Event,
};
use crate::{
    app::{OpenMusic, Session, intent::Intent},
    menus::Handle,
    ui::tracks::{RowDrag, RowDrop},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemPrefs {}

#[derive(Default)]
pub struct State {
    pub published: Option<(Snapshot, Instant)>,
    pub waiting: Option<bool>,
    pub publishes: u64,
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
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
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
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    format!("published={}", app.system.publishes)
}

pub fn register_actions(_cx: &mut App, _handle: &Handle) {}

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
