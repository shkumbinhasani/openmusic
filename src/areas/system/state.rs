use std::{
    collections::HashSet,
    path::Path,
    time::{Duration, Instant},
};

use gpui::{App, Context, Keystroke, Window};
use serde::{Deserialize, Serialize};
use serde_json::json;
use spotify_client::library;
use tokio::sync::mpsc::unbounded_channel;

use super::{
    dock_menu::{self, DockState, RECENT_LIMIT, recent_name},
    menu_bar::{self, MenuState},
    mini_player::{self, MiniState},
    native_window,
    notify::{self, Authorization, Seen, TrackNote, Watch},
    now_playing::{self, Snapshot},
    shortcuts, view,
    worker::Event,
};
use crate::{
    app::{OpenMusic, Page, Session, intent::Intent},
    menus::{
        About, BringAllToFront, CloseWindow, GoBack, GoForward, GoHome, Handle, HideApp,
        HideOthers, Minimize, Next, PlayPause, PlayRecent, Previous, SeekBackward, SeekForward,
        ShowAll, ShowMainWindow, ShowShortcuts, ToggleFullScreen, ToggleMiniOnMinimize,
        ToggleMiniPlayer, ToggleNotifications, TogglePlayback, VolumeDown, VolumeUp, Zoom, route,
    },
    ui::tracks::{RowDrag, RowDrop},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SystemPrefs {
    pub notify_on_track_change: bool,
    pub mini_player_when_minimized: bool,
}

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
    pub menu: Option<MenuState>,
    pub throttle: Throttle,
    pub shortcuts: bool,
    pub dock: Option<DockState>,
    pub dock_builds: u64,
    pub mini: Option<MiniState>,
    pub mini_auto: bool,
    pub watch: Watch,
    pub notes: u64,
    pub notify_asked: bool,
    pub on_settings: bool,
    pub was_active: bool,
}

impl State {
    pub fn covers(&self, _urls: &mut HashSet<String>) {}
}

pub fn install(cx: &mut App) {
    if cfg!(not(target_os = "macos")) {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    }
}

pub fn window_opened(window: &mut Window, cx: &mut App) {
    notify::install();
    if native_window::install(window) {
        window.on_window_should_close(cx, |_, _| {
            native_window::hide();
            false
        });
    }
}

impl OpenMusic {
    pub fn show_main_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if native_window::state().installed {
            native_window::show();
        } else {
            window.activate_window();
        }
        cx.activate(true);
    }

    pub fn close_main_window(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if native_window::state().installed {
            native_window::hide();
        } else {
            window.remove_window();
        }
    }

    pub fn toggle_mini_player(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if mini_player::is_visible() {
            self.close_mini_player();
        } else if !self.open_mini_player() {
            self.show_toast("Play something to open the mini player.", None);
        }
    }

    pub fn mini_allowed(&self) -> bool {
        matches!(self.session, Session::Connected)
            && (self.track().is_some() || self.playback.now.device.is_some())
    }

    pub fn mini_state(&self) -> Option<MiniState> {
        if !matches!(self.session, Session::Connected) {
            return None;
        }
        let now = &self.playback.now;
        let track = self.track();
        Some(MiniState {
            title: track
                .map(|track| track.title.clone())
                .filter(|title| !title.is_empty())
                .unwrap_or_else(|| "Not playing".to_owned()),
            artist: track.map(|track| track.artist.clone()).unwrap_or_default(),
            device: now.device.clone().filter(|_| !now.local && track.is_some()),
            playing: track.is_some() && now.playing,
            saved: track.and_then(|track| self.is_saved(&track.uri)),
            likable: track.is_some_and(|track| library::savable(&track.uri)),
            cover: track
                .and_then(|track| track.cover.clone())
                .filter(|url| self.covers.contains_key(url)),
            duration_ms: self.duration().unwrap_or(0),
        })
    }

    pub fn open_mini_player(&mut self) -> bool {
        if !self.mini_allowed() {
            return false;
        }
        let Some(state) = self.mini_state() else {
            return false;
        };
        let cover = state.cover.as_ref().and_then(|url| self.covers.get(url));
        if !mini_player::show(&state, cover, self.playback.now.position()) {
            return false;
        }
        self.system.mini = Some(state);
        true
    }

    pub fn close_mini_player(&mut self) {
        mini_player::hide();
        self.system.mini = None;
        self.system.mini_auto = false;
    }

    fn sync_mini_player(&mut self) {
        if !mini_player::is_visible() {
            self.system.mini = None;
            return;
        }
        let Some(state) = self.mini_state() else {
            self.close_mini_player();
            return;
        };
        if self.system.mini.as_ref() != Some(&state) {
            let cover = state.cover.as_ref().and_then(|url| self.covers.get(url));
            mini_player::update(&state, cover);
            self.system.mini = Some(state);
        }
        mini_player::progress(self.playback.now.position());
    }

    pub fn track_note(&self) -> Option<TrackNote> {
        let track = self.track().filter(|track| !track.title.is_empty())?;
        let cover = track
            .cover
            .as_ref()
            .and_then(|url| self.covers.get(url))
            .and_then(|image| {
                notify::cover_extension(image.format)
                    .map(|extension| (image.bytes().to_vec(), extension))
            });
        Some(TrackNote {
            uri: track.uri.clone(),
            title: track.title.clone(),
            artist: track.artist.clone(),
            album: track.album.clone(),
            cover,
        })
    }

    pub fn toggle_notifications(&mut self, cx: &mut App) {
        let on = !self.settings.system.notify_on_track_change;
        self.update_settings(|settings| settings.system.notify_on_track_change = on);
        if on {
            self.system.notify_asked = true;
            notify::authorize();
        }
        self.sync_system(cx);
    }

    fn notification_status(&mut self, cx: &mut App) {
        if !self.system.notify_asked {
            return;
        }
        match notify::status() {
            Authorization::Denied => {
                self.system.notify_asked = false;
                if self.settings.system.notify_on_track_change {
                    self.show_toast(
                        "Turn on notifications for OpenMusic in System Settings.",
                        None,
                    );
                    cx.open_url(notify::SETTINGS_URL);
                }
            }
            Authorization::Authorized | Authorization::Unavailable => {
                self.system.notify_asked = false;
            }
            Authorization::Unknown | Authorization::NotDetermined => {}
        }
    }

    fn sync_notifications(&mut self) {
        if !notify::available() {
            return;
        }
        notify::set_app_active(native_window::state().frontmost());
        let enabled = self.settings.system.notify_on_track_change
            && matches!(self.session, Session::Connected);
        let now = &self.playback.now;
        let seen = now.track.as_ref().map(|track| Seen {
            uri: &track.uri,
            title: &track.title,
            playing: now.playing,
        });
        let Some(uri) = self.system.watch.observe(seen, enabled, Instant::now()) else {
            return;
        };
        if let Some(note) = self.track_note().filter(|note| note.uri == uri)
            && notify::post(&note, false)
        {
            self.system.notes += 1;
        }
    }

    pub fn set_mini_on_minimize(&mut self, on: bool, cx: &mut App) {
        self.update_settings(|settings| settings.system.mini_player_when_minimized = on);
        self.sync_system(cx);
    }

    fn refresh_settings_status(&mut self) {
        notify::refresh_status();
    }

    pub fn settings_summary(&self) -> String {
        let prefs = &self.settings.system;
        let status = notify::status();
        [
            format!(
                "startup mini_on_minimize={}",
                prefs.mini_player_when_minimized
            ),
            format!(
                "notifications shown={} status={} notify={} denied={}",
                notify::available(),
                status.name(),
                prefs.notify_on_track_change,
                status == Authorization::Denied
            ),
        ]
        .join("\n")
    }

    fn watch_settings_page(&mut self, cx: &mut Context<Self>) {
        let open = self.page == Page::Settings;
        let active = native_window::state().app_active;
        if open && (!self.system.on_settings || (active && !self.system.was_active)) {
            self.refresh_settings_status();
            cx.notify();
        }
        self.system.on_settings = open;
        self.system.was_active = active;
    }

    fn window_miniaturized(&mut self) {
        if self.settings.system.mini_player_when_minimized
            && !mini_player::is_visible()
            && self.open_mini_player()
        {
            self.system.mini_auto = true;
        }
    }

    fn window_restored(&mut self) {
        if self.system.mini_auto {
            self.close_mini_player();
        }
    }

    pub fn act(&mut self, intent: Intent, window: &mut Window, cx: &mut Context<Self>) {
        self.perform(intent, window, cx);
        self.sync_system(cx);
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

    pub fn menu_state(&self) -> MenuState {
        MenuState {
            playing: self.playback.now.playing,
            shuffle: self.shuffle_mode(),
            repeat: self.repeat_mode(),
            saved: self.track().and_then(|track| self.is_saved(&track.uri)),
            episode: self.playback.now.episode
                || self
                    .track()
                    .is_some_and(|track| track.uri.starts_with("spotify:episode:")),
            notify: self.settings.system.notify_on_track_change,
            mini_on_minimize: self.settings.system.mini_player_when_minimized,
            mini_open: mini_player::is_visible(),
        }
    }

    pub fn sync_system(&mut self, cx: &mut App) {
        self.sync_native();
        self.sync_dock(cx);
    }

    pub fn sync_native(&mut self) {
        self.sync_now_playing();
        self.sync_mini_player();
        self.sync_notifications();
        let menu = self.menu_state();
        if self.system.menu.as_ref() != Some(&menu) {
            menu_bar::sync(&menu);
            self.system.menu = Some(menu);
        }
    }

    pub fn dock_state(&self) -> DockState {
        let connected = matches!(self.session, Session::Connected);
        let track = self
            .track()
            .filter(|track| connected && !track.title.is_empty());
        let recents = if connected {
            self.recent_contexts()
                .iter()
                .filter_map(|recent| {
                    recent_name(&recent.uri, &recent.name).map(|name| (recent.uri.clone(), name))
                })
                .take(RECENT_LIMIT)
                .collect()
        } else {
            Vec::new()
        };
        DockState {
            connected,
            title: track.map(|track| track.title.clone()),
            artist: track.map(|track| track.artist.clone()),
            playing: track.is_some() && self.playback.now.playing,
            recents,
            mini_open: mini_player::is_visible(),
        }
    }

    fn sync_dock(&mut self, cx: &mut App) {
        let state = self.dock_state();
        if self.system.dock.as_ref() == Some(&state) {
            return;
        }
        cx.set_dock_menu(dock_menu::menu(&state));
        dock_menu::strip_keys();
        self.system.dock = Some(state);
        self.system.dock_builds += 1;
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

    fn liked_label(&self) -> &'static str {
        match self.track().and_then(|track| self.is_saved(&track.uri)) {
            Some(true) => "saved",
            Some(false) => "not-saved",
            None => "unknown",
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
    app.system.watch.reset();
    app.sync_system(cx);
}

pub fn signed_out(app: &mut OpenMusic) {
    app.system.watch.reset();
    app.close_mini_player();
    app.sync_native();
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
            app.sync_system(cx);
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
        "menubar" => {
            app.sync_system(cx);
            let native = menu_bar::items();
            let owned = cx.get_menus().unwrap_or_default();
            let mut lines = Vec::new();
            let mut available = |action: &dyn gpui::Action| {
                window.is_action_available(action, cx) || cx.is_action_available(action)
            };
            for (index, menu) in native.iter().enumerate() {
                lines.push(menu.title.clone());
                let items = owned
                    .get(index)
                    .map_or(&[][..], |menu| menu.items.as_slice());
                menu_bar::describe(&menu.children, items, 1, &mut available, &mut lines);
            }
            write(directory, "menubar.txt", lines.join("\n"));
        }
        "dock" => {
            app.sync_system(cx);
            let items = dock_menu::menu(app.system.dock.as_ref().unwrap_or(&DockState::default()));
            let mut available = |action: &dyn gpui::Action| {
                window.is_action_available(action, cx) || cx.is_action_available(action)
            };
            let mut lines = vec![format!("builds={}", app.system.dock_builds)];
            lines.extend(dock_menu::describe(&items, &mut available));
            lines.push(match dock_menu::native_items() {
                Some(native) => format!(
                    "native items={} keys={}",
                    native.len(),
                    native.iter().filter(|(_, key)| *key).count()
                ),
                None => "native none".to_owned(),
            });
            write(directory, "dock.txt", lines.join("\n"));
        }
        "mini" => {
            let (command, argument) = rest.split_once(' ').unwrap_or((rest, ""));
            match command {
                "open" => {
                    if !mini_player::is_visible() {
                        app.open_mini_player();
                    }
                }
                "close" => app.close_mini_player(),
                "toggle" => app.act(Intent::ToggleMiniPlayer, window, cx),
                "hover" => mini_player::set_hover(argument.trim() == "on"),
                "size" => {
                    let mut parts = argument
                        .split_whitespace()
                        .filter_map(|part| part.parse::<f64>().ok());
                    if let (Some(width), Some(height)) = (parts.next(), parts.next()) {
                        mini_player::resize(width, height);
                    }
                }
                "press" => {
                    if let Some(key) = mini_player::KEYS
                        .iter()
                        .find(|key| **key == argument.trim())
                    {
                        on_press(app, key, window, cx);
                    }
                }
                "state" | "" => {}
                _ => return false,
            }
            app.sync_system(cx);
            write(
                directory,
                "mini.txt",
                format!(
                    "{}\nauto={} allowed={}",
                    mini_player::describe(),
                    app.system.mini_auto,
                    app.mini_allowed()
                ),
            );
        }
        "notify" => {
            let posted = match rest {
                "" | "post" => app
                    .track_note()
                    .is_some_and(|note| notify::post(&note, true)),
                "state" => false,
                _ => return false,
            };
            if posted {
                app.system.notes += 1;
            }
            notify::refresh_status();
            write(
                directory,
                "notify.txt",
                format!(
                    "available={} status={} enabled={} posted={posted} notes={} due={:?}",
                    notify::available(),
                    notify::status().name(),
                    app.settings.system.notify_on_track_change,
                    app.system.notes,
                    app.system.watch.due(),
                ),
            );
        }
        "system-settings" => {
            let (name, value) = rest.split_once(' ').unwrap_or((rest, ""));
            let on = match value.trim() {
                "on" => Some(true),
                "off" => Some(false),
                _ => None,
            };
            match (name, on) {
                ("", _) => {}
                ("mini", Some(on)) => app.set_mini_on_minimize(on, cx),
                ("notify", Some(on)) => {
                    if on != app.settings.system.notify_on_track_change && notify::available() {
                        app.toggle_notifications(cx);
                    }
                }
                _ => return false,
            }
            app.refresh_settings_status();
            cx.notify();
            write(directory, "system-settings.txt", app.settings_summary());
        }
        "window" => {
            match rest {
                "hide" => app.close_main_window(window, cx),
                "close" => native_window::close(),
                "show" => app.act(Intent::ShowMainWindow, window, cx),
                "state" | "" => {}
                _ => return false,
            }
            write(directory, "window.txt", native_window::state().describe());
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

fn window_label(state: native_window::WindowState) -> &'static str {
    if !state.installed {
        "unknown"
    } else if state.miniaturized {
        "minimized"
    } else if state.visible {
        "shown"
    } else {
        "hidden"
    }
}

pub fn state_fragment(app: &OpenMusic) -> String {
    format!(
        "published={} liked={} shortcuts={} window={} dock={} mini={} notes={}",
        app.system.publishes,
        app.liked_label(),
        app.system.shortcuts,
        window_label(native_window::state()),
        app.system.dock_builds,
        mini_player::is_visible(),
        app.system.notes,
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
    route(cx, handle, |_: &CloseWindow, app, window, cx| {
        app.close_main_window(window, cx)
    });
    route(cx, handle, |_: &ShowMainWindow, app, window, cx| {
        app.act(Intent::ShowMainWindow, window, cx)
    });
    route(cx, handle, |action: &PlayRecent, app, window, cx| {
        app.act(Intent::PlayContext(action.uri.clone()), window, cx)
    });
    route(cx, handle, |_: &ToggleMiniPlayer, app, window, cx| {
        app.act(Intent::ToggleMiniPlayer, window, cx)
    });
    if notify::available() {
        route(cx, handle, |_: &ToggleNotifications, app, _, cx| {
            app.toggle_notifications(cx)
        });
    }
    route(cx, handle, |_: &ToggleMiniOnMinimize, app, _, cx| {
        let on = !app.settings.system.mini_player_when_minimized;
        app.set_mini_on_minimize(on, cx);
    });
    cx.on_action(|_: &About, _| menu_bar::about());
    cx.on_action(|_: &HideApp, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &BringAllToFront, _| menu_bar::arrange_in_front());
}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        native_window::REOPEN | mini_player::EXPAND => app.act(Intent::ShowMainWindow, window, cx),
        native_window::MINIATURIZED => app.window_miniaturized(),
        native_window::RESTORED => app.window_restored(),
        mini_player::PREVIOUS => app.act(Intent::Previous, window, cx),
        mini_player::TOGGLE => app.act(Intent::PlayPause, window, cx),
        mini_player::NEXT => app.act(Intent::Next, window, cx),
        mini_player::LIKE => app.act(Intent::ToggleLike, window, cx),
        mini_player::CLOSE => app.close_mini_player(),
        notify::OPEN => app.act(Intent::ShowMainWindow, window, cx),
        notify::STATUS => app.notification_status(cx),
        view::NOTIFICATION_SETTINGS => cx.open_url(notify::SETTINGS_URL),
        view::MINI_BUTTON => app.act(Intent::ToggleMiniPlayer, window, cx),
        _ => return false,
    }
    app.sync_system(cx);
    true
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

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    app.sync_system(cx);
    app.watch_settings_page(cx);
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
