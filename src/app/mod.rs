pub mod intent;
mod nav;
mod saved;
pub mod settings;
mod toast;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::Arc,
};

use gpui::{
    AppContext, Context, Entity, FocusHandle, Image, ImageFormat, Styled, Window, px, size,
};
use gpui_component::{
    ActiveTheme, WindowExt,
    input::{InputEvent, InputState},
};

use crate::{
    areas::{account, audio, content, details, discovery, library, playback, playlists, system},
    debug, menus, overlay,
    ui::{self, gallery::Gallery, menu::Menus},
    worker::{Command, Event},
};
use intent::Intent;
pub use nav::{Page, page_for};
use saved::Saved;
use settings::Settings;
use toast::Toast;

pub enum Session {
    SignedOut,
    Connecting,
    Pairing { code: String, url: String },
    Connected,
}

pub struct OpenMusic {
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    pub session: Session,
    initial_track: Option<String>,
    pub input: Entity<InputState>,
    pub focus: FocusHandle,
    pub page: Page,
    pub history: Vec<Page>,
    pub ahead: Vec<Page>,
    pub covers: HashMap<String, Arc<Image>>,
    cover_scan: Option<(usize, Page)>,
    pub session_error: Option<String>,
    pub modal: bool,
    pub toast: Option<Toast>,
    pub settings: Settings,
    pub saved: Saved,
    pub menus: Menus,
    pub gallery: Option<Gallery>,
    pub account: account::state::State,
    pub audio: audio::state::State,
    pub content: content::state::State,
    pub details: details::state::State,
    pub discovery: discovery::state::State,
    pub library: library::state::State,
    pub playback: playback::state::State,
    pub playlists: playlists::state::State,
    pub system: system::state::State,
}

impl OpenMusic {
    pub fn new(
        commands: tokio::sync::mpsc::UnboundedSender<Command>,
        initial_track: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_window_appearance(window, |_, window, cx| {
            ui::apply_theme(window.appearance(), cx);
            cx.notify();
        })
        .detach();
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Songs, artists, or links"));
        cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Blur) && window.focused(cx).is_none() {
                window.focus(&this.focus);
            }
            discovery::state::on_input(this, event, window, cx);
        })
        .detach();

        Self {
            commands,
            session: Session::Connecting,
            initial_track,
            input,
            focus: cx.focus_handle(),
            page: Page::Home,
            history: Vec::new(),
            ahead: Vec::new(),
            covers: HashMap::new(),
            cover_scan: None,
            session_error: None,
            modal: false,
            toast: None,
            settings: settings::load(),
            saved: Saved::default(),
            menus: Menus::default(),
            gallery: None,
            account: Default::default(),
            audio: Default::default(),
            content: Default::default(),
            details: Default::default(),
            discovery: Default::default(),
            library: Default::default(),
            playback: Default::default(),
            playlists: Default::default(),
            system: Default::default(),
        }
    }

    pub fn debug_command(
        &mut self,
        line: &str,
        directory: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        let _ = self.shared_debug(verb, rest, directory, window, cx)
            || ui::debug(self, verb, rest, directory, window, cx)
            || ui::menu::debug(self, verb, rest, directory, window, cx)
            || menus::debug(self, verb, rest, directory, window, cx)
            || account::state::debug(self, verb, rest, directory, window, cx)
            || audio::state::debug(self, verb, rest, directory, window, cx)
            || content::state::debug(self, verb, rest, directory, window, cx)
            || details::state::debug(self, verb, rest, directory, window, cx)
            || discovery::state::debug(self, verb, rest, directory, window, cx)
            || library::state::debug(self, verb, rest, directory, window, cx)
            || playback::state::debug(self, verb, rest, directory, window, cx)
            || playlists::state::debug(self, verb, rest, directory, window, cx)
            || system::state::debug(self, verb, rest, directory, window, cx);
        cx.notify();
    }

    fn shared_debug(
        &mut self,
        verb: &str,
        rest: &str,
        directory: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match verb {
            "snapshot" => {
                let name = if rest.is_empty() { "snapshot" } else { rest };
                let png = directory.join(format!("{name}.png"));
                let marker = directory.join(format!("{name}.done"));
                let _ = std::fs::remove_file(&marker);
                window.refresh();
                window.on_next_frame(move |window, _| {
                    window.on_next_frame(move |_, _| {
                        debug::capture(png, move |result| {
                            let _ =
                                std::fs::write(marker, result.err().unwrap_or_else(|| "ok".into()));
                        });
                    });
                    window.refresh();
                    debug::redraw(window);
                });
                debug::redraw(window);
            }
            "home" => self.navigate(Page::Home),
            "lyrics" => self.show_lyrics(true),
            "back" => self.back(),
            "forward" => self.forward(),
            "search" => {
                let query = rest.to_owned();
                self.input
                    .update(cx, |input, cx| input.set_value(query, window, cx));
                self.discovery.search_pending = false;
                self.search(cx);
            }
            "artist" => self.navigate(Page::Artist(rest.to_owned())),
            "open" => {
                let page = page_for(rest).unwrap_or_else(|| Page::Collection(rest.to_owned()));
                self.navigate(page);
            }
            "page" => {
                if let Some(page) = Page::parse(rest) {
                    self.navigate(page);
                }
            }
            "intent" => {
                if let Some(intent) = Intent::parse(rest) {
                    self.perform(intent, window, cx);
                }
            }
            "play" => {
                let (uri, index) = rest.split_once(' ').unwrap_or((rest, "0"));
                self.send(playback::worker::Command::PlayContext {
                    uri: uri.to_owned(),
                    index: index.trim().parse().unwrap_or(0),
                    track_uri: None,
                    origin: spotify_client::connect::ORIGIN,
                    options: Default::default(),
                });
            }
            "toggle" => self.toggle(cx),
            "like" | "unlike" => {
                let uris = rest.split_whitespace().map(str::to_owned).collect();
                self.set_saved(uris, verb == "like");
            }
            "saved" => {
                let uris: Vec<String> = rest.split_whitespace().map(str::to_owned).collect();
                self.lookup_saved(uris.iter().cloned());
                let lines: Vec<String> = uris
                    .iter()
                    .map(|uri| {
                        let saved = match self.is_saved(uri) {
                            Some(true) => "saved",
                            Some(false) => "not-saved",
                            None => "unknown",
                        };
                        let pending = if self.saved.pending.contains_key(uri) {
                            " pending"
                        } else {
                            ""
                        };
                        format!("{uri} {saved}{pending}")
                    })
                    .collect();
                let _ = std::fs::write(directory.join("saved.txt"), lines.join("\n"));
            }
            "toast" if rest == "undo" => self.undo_toast(cx),
            "toast" => self.show_toast(rest, None),
            "dialog" => {
                if rest.is_empty() {
                    window.close_all_dialogs(cx);
                } else {
                    let text: gpui::SharedString = rest.to_owned().into();
                    window.open_dialog(cx, move |dialog, _, cx| {
                        dialog.title(text.clone()).bg(cx.theme().popover)
                    });
                }
            }
            "settings" => {
                if !rest.is_empty()
                    && let Ok(settings) = serde_json::from_str::<Settings>(rest)
                {
                    self.update_settings(|current| *current = settings);
                }
                if let Ok(text) = serde_json::to_string_pretty(&self.settings) {
                    let _ = std::fs::write(directory.join("settings.txt"), text);
                }
            }
            "size" => {
                let mut parts = rest
                    .split_whitespace()
                    .filter_map(|part| part.parse::<f32>().ok());
                if let (Some(width), Some(height)) = (parts.next(), parts.next()) {
                    window.resize(size(px(width), px(height)));
                }
            }
            "scroll" => {
                if let Ok(y) = rest.trim().parse::<f32>() {
                    let handle = self.details.content_scroll(&self.content_scroll_id());
                    handle.set_offset(gpui::point(px(0.), px(-y)));
                    cx.notify();
                }
            }
            "nudge" => {
                let mut parts = rest
                    .split_whitespace()
                    .filter_map(|part| part.parse::<f64>().ok());
                if let (Some(dx), Some(dy)) = (parts.next(), parts.next()) {
                    overlay::nudge_window(dx, dy);
                }
            }
            "state" => {
                let _ = std::fs::write(directory.join("state.txt"), self.state_line());
            }
            _ => return false,
        }
        true
    }

    fn state_line(&self) -> String {
        let mut state = format!(
            "account={} page={} track={:?} playing={} position={} device={:?} playlists={} notice={:?} ahead={} saved={} pending={} toast={:?} modal={}",
            matches!(self.session, Session::Connected),
            self.page.describe(),
            self.playback
                .now
                .track
                .as_ref()
                .map(|track| format!("{} - {}", track.title, track.artist)),
            self.playback.now.playing,
            self.playback.now.position(),
            self.playback.now.device,
            self.library.playlists.len(),
            self.playback.notice,
            self.ahead.len(),
            self.saved.known.len(),
            self.saved.pending.len(),
            self.toast.as_ref().map(|toast| toast.text.as_str()),
            self.modal,
        );
        let fragments = [
            account::state::state_fragment(self),
            audio::state::state_fragment(self),
            content::state::state_fragment(self),
            details::state::state_fragment(self),
            discovery::state::state_fragment(self),
            library::state::state_fragment(self),
            playback::state::state_fragment(self),
            playlists::state::state_fragment(self),
            system::state::state_fragment(self),
        ];
        for fragment in fragments.iter().filter(|fragment| !fragment.is_empty()) {
            state.push(' ');
            state.push_str(fragment);
        }
        state
    }

    fn referenced_covers(&self) -> HashSet<String> {
        let mut urls = HashSet::new();
        self.account.covers(&mut urls);
        self.audio.covers(&mut urls);
        self.content.covers(&mut urls);
        self.details.covers(&mut urls);
        self.discovery.covers(&mut urls);
        self.library.covers(&mut urls);
        self.playback.covers(&mut urls);
        self.playlists.covers(&mut urls);
        self.system.covers(&mut urls);
        urls
    }

    fn evict_covers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keep = self.referenced_covers();
        let stale: Vec<String> = self
            .covers
            .keys()
            .filter(|url| !keep.contains(*url))
            .cloned()
            .collect();
        for url in &stale {
            if let Some(image) = self.covers.remove(url) {
                let drawn = ui::drawn::forget(&image);
                release(image, drawn, window, cx);
            }
        }
        if !stale.is_empty() {
            self.account.local.forget_art(&stale);
            self.send(Command::ForgetCovers(stale));
        }
    }

    fn trim_covers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scanned = self
            .cover_scan
            .as_ref()
            .is_some_and(|(count, page)| *count == self.covers.len() && *page == self.page);
        if self.covers.len() > COVER_CACHE_LIMIT && !scanned {
            self.evict_covers(window, cx);
            self.cover_scan = Some((self.covers.len(), self.page.clone()));
        }
        for image in ui::drawn::offscreen(COVER_CACHE_LIMIT) {
            release(image, true, window, cx);
        }
    }

    fn release_covers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (_, image) in self.covers.drain() {
            let drawn = ui::drawn::forget(&image);
            release(image, drawn, window, cx);
        }
        for image in ui::drawn::drain() {
            release(image, true, window, cx);
        }
        self.cover_scan = None;
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.session, Session::Connected) {
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    pub fn modal_open(&self) -> bool {
        self.modal
    }

    pub fn send(&self, command: impl Into<Command>) {
        let _ = self.commands.send(command.into());
    }

    pub fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.expire_toast() {
            cx.notify();
        }
        self.trim_covers(window, cx);
        ui::gallery::tick(self, cx);
        account::state::tick(self, cx);
        audio::state::tick(self, cx);
        content::state::tick(self, cx);
        details::state::tick(self, cx);
        discovery::state::tick(self, cx);
        library::state::tick(self, cx);
        playback::state::tick(self, cx);
        playlists::state::tick(self, cx);
        system::state::tick(self, cx);
        self.refresh_bar_menu();
        if !self.playback.now.playing {
            return;
        }
        overlay::progress(self.playback.now.position(), self.duration().unwrap_or(0));
        if self.follow_lyrics() && self.page == Page::Lyrics {
            cx.notify();
        }
    }

    pub fn handle_control(
        &mut self,
        action: overlay::Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            overlay::Action::Press("sign-in") => {
                self.session_error = None;
                self.session = Session::Connecting;
                self.send(Command::SignIn);
            }
            overlay::Action::Press("cancel") => self.send(Command::Cancel),
            overlay::Action::Press("copy-code") => {
                if let Session::Pairing { code, .. } = &self.session {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(code.clone()));
                }
            }
            overlay::Action::Press("open-pairing") => {
                if let Session::Pairing { url, .. } = &self.session {
                    cx.open_url(url);
                }
            }
            overlay::Action::Press("back") => self.perform(Intent::Back, window, cx),
            overlay::Action::Press(key) => {
                let _ = ui::gallery::on_press(self, key)
                    || account::state::on_press(self, key, window, cx)
                    || audio::state::on_press(self, key, window, cx)
                    || content::state::on_press(self, key, window, cx)
                    || details::state::on_press(self, key, window, cx)
                    || discovery::state::on_press(self, key, window, cx)
                    || library::state::on_press(self, key, window, cx)
                    || playback::state::on_press(self, key, window, cx)
                    || playlists::state::on_press(self, key, window, cx)
                    || system::state::on_press(self, key, window, cx);
            }
            overlay::Action::Menu(_)
            | overlay::Action::MenuClosed
            | overlay::Action::BarMenu { .. } => self.menu_action(action, window, cx),
            overlay::Action::Seek(fraction) => {
                if let Some(duration) = self.duration() {
                    let position = (fraction * duration.saturating_sub(1) as f32) as u32;
                    self.seek_to(position, cx);
                }
            }
            overlay::Action::Volume(fraction) => {
                let volume = (fraction.clamp(0., 1.) * f32::from(u16::MAX)).round() as u16;
                self.set_volume(volume, cx);
            }
        }
        cx.notify();
    }

    pub fn handle_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Connecting => self.session = Session::Connecting,
            Event::Pairing { code, url } => self.session = Session::Pairing { code, url },
            Event::Connected => {
                self.session = Session::Connected;
                account::state::connected(self, window, cx);
                audio::state::connected(self, window, cx);
                content::state::connected(self, window, cx);
                details::state::connected(self, window, cx);
                discovery::state::connected(self, window, cx);
                library::state::connected(self, window, cx);
                playback::state::connected(self, window, cx);
                playlists::state::connected(self, window, cx);
                system::state::connected(self, window, cx);
                self.send(Command::ApplySettings(self.settings.clone()));
                if let Some(track) = self.initial_track.take() {
                    self.input
                        .update(cx, |input, cx| input.set_value(track, window, cx));
                    self.search(cx);
                }
            }
            Event::Reconnected => {
                account::state::reconnected(self);
                audio::state::reconnected(self);
                details::state::reconnected(self);
                library::state::reconnected(self);
                playback::state::reconnected(self);
                playlists::state::reconnected(self);
            }
            Event::SignedOut(message) => {
                self.session = Session::SignedOut;
                self.session_error = message;
                self.page = Page::Home;
                self.history.clear();
                self.ahead.clear();
                self.release_covers(window, cx);
                self.saved = Saved::default();
                self.toast = None;
                account::state::signed_out(self);
                audio::state::signed_out(self);
                content::state::signed_out(self);
                details::state::signed_out(self);
                discovery::state::signed_out(self);
                library::state::signed_out(self);
                playback::state::signed_out(self);
                playlists::state::signed_out(self);
                system::state::signed_out(self);
            }
            Event::Cover { url, jpeg } => {
                self.covers
                    .insert(url, Arc::new(Image::from_bytes(image_format(&jpeg), jpeg)));
            }
            Event::NowPlaying(now) => {
                self.request_lyrics(&now);
                self.set_now_playing(now);
                self.refresh_bar_menu();
                self.follow_lyrics();
                if let Some(uri) = self.track().map(|track| track.uri.clone()) {
                    self.lookup_saved([uri]);
                }
            }
            Event::Notice(message) => {
                self.playback.notice = Some(message);
                self.playback.seek_hold = None;
            }
            Event::MutationFailed { keys, message } => {
                self.saved_failed(&keys);
                self.show_toast(message, None);
            }
            Event::Saved(event) => self.on_saved(event),
            Event::Account(event) => account::state::on_event(self, event, window, cx),
            Event::Audio(event) => audio::state::on_event(self, event, window, cx),
            Event::Content(event) => content::state::on_event(self, event, window, cx),
            Event::Details(event) => details::state::on_event(self, event, window, cx),
            Event::Discovery(event) => discovery::state::on_event(self, event, window, cx),
            Event::Library(event) => library::state::on_event(self, event, window, cx),
            Event::Playback(event) => playback::state::on_event(self, event, window, cx),
            Event::Playlists(event) => playlists::state::on_event(self, event, window, cx),
            Event::System(event) => system::state::on_event(self, event, window, cx),
        }
        cx.notify();
    }
}

const COVER_CACHE_LIMIT: usize = 220;

fn release(image: Arc<Image>, drawn: bool, window: &mut Window, cx: &mut Context<OpenMusic>) {
    if drawn && let Some(render) = image.clone().get_render_image(window, cx) {
        let _ = window.drop_image(render);
    }
    image.remove_asset(cx);
}

fn image_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else {
        ImageFormat::Jpeg
    }
}
