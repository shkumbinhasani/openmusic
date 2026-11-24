mod nav;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use gpui::{AppContext, Context, Entity, Image, ImageFormat, Window, px, size};
use gpui_component::input::{InputEvent, InputState};
use spotify_client::engine::NowPlaying;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    areas::{content, discovery, discovery::worker::Discovery, library, playback, playlists},
    debug, overlay, ui,
    worker::{Command, Event},
};
pub use nav::Page;

pub enum Account {
    SignedOut,
    Connecting,
    Pairing { code: String, url: String },
    Connected,
}

pub struct OpenMusic {
    commands: UnboundedSender<Command>,
    pub account: Account,
    initial_track: Option<String>,
    pub input: Entity<InputState>,
    pub page: Page,
    pub history: Vec<Page>,
    pub covers: HashMap<String, Arc<Image>>,
    pub account_error: Option<String>,
    pub playback: playback::state::State,
    pub library: library::state::State,
    pub playlists: playlists::state::State,
    pub discovery: discovery::state::State,
    pub content: content::state::State,
}

impl OpenMusic {
    pub fn new(
        commands: UnboundedSender<Command>,
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
        cx.subscribe_in(&input, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.search(cx);
            }
        })
        .detach();

        Self {
            commands,
            account: Account::Connecting,
            initial_track,
            input,
            page: Page::Home,
            history: Vec::new(),
            covers: HashMap::new(),
            account_error: None,
            playback: Default::default(),
            library: Default::default(),
            playlists: Default::default(),
            discovery: Default::default(),
            content: Default::default(),
        }
    }

    pub fn debug_command(
        &mut self,
        line: &str,
        directory: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
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
                });
            }
            "home" => self.navigate(Page::Home),
            "lyrics" => self.show_lyrics(true),
            "back" => self.back(),
            "search" => {
                let query = rest.to_owned();
                self.input
                    .update(cx, |input, cx| input.set_value(query, window, cx));
                self.discovery.search_pending = false;
                self.search(cx);
            }
            "artist" => self.navigate(Page::Artist(rest.to_owned())),
            "open" => self.navigate(Page::Collection(rest.to_owned())),
            "play" => {
                let (uri, index) = rest.split_once(' ').unwrap_or((rest, "0"));
                self.send(Command::PlayContext {
                    uri: uri.to_owned(),
                    index: index.trim().parse().unwrap_or(0),
                    track_uri: None,
                });
            }
            "toggle" => self.toggle(cx),
            "size" => {
                let mut parts = rest
                    .split_whitespace()
                    .filter_map(|part| part.parse::<f32>().ok());
                if let (Some(width), Some(height)) = (parts.next(), parts.next()) {
                    window.resize(size(px(width), px(height)));
                }
            }
            "state" => {
                let state = format!(
                    "account={} page={} track={:?} playing={} position={} device={:?} playlists={} notice={:?}",
                    matches!(self.account, Account::Connected),
                    match &self.page {
                        Page::Home => "home".to_owned(),
                        Page::Search => "search".to_owned(),
                        Page::Lyrics => "lyrics".to_owned(),
                        Page::Artist(uri) => format!("artist {uri}"),
                        Page::Collection(uri) => format!("collection {uri}"),
                    },
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
                );
                let _ = std::fs::write(directory.join("state.txt"), state);
            }
            _ => {}
        }
        cx.notify();
    }

    fn referenced_covers(&self) -> HashSet<String> {
        let mut urls = HashSet::new();
        self.playback.covers(&mut urls);
        self.library.covers(&mut urls);
        self.discovery.covers(&mut urls);
        self.playlists.covers(&mut urls);
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
                if let Some(render) = image.clone().get_render_image(window, cx) {
                    let _ = window.drop_image(render);
                }
                image.remove_asset(cx);
            }
        }
        if !stale.is_empty() {
            self.send(Command::ForgetCovers(stale));
        }
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.account, Account::Connected) {
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.playback.now.playing {
            return;
        }
        overlay::progress(self.playback.now.position(), self.duration().unwrap_or(0));
        if self.follow_lyrics() && self.page == Page::Lyrics {
            cx.notify();
        }
    }

    pub fn handle_control(&mut self, action: overlay::Action, cx: &mut Context<Self>) {
        match action {
            overlay::Action::Press("previous") => self.send(Command::Previous),
            overlay::Action::Press("next") => self.send(Command::Next),
            overlay::Action::Press("toggle-playback") => self.toggle(cx),
            overlay::Action::Press("play-station") => {
                if let Some(mix) = self.discovery.station() {
                    let tracks = mix.tracks.clone();
                    self.play(&tracks, 0, cx);
                }
            }
            overlay::Action::Press("sign-in") => {
                self.account_error = None;
                self.account = Account::Connecting;
                self.send(Command::SignIn);
            }
            overlay::Action::Press("cancel") => self.send(Command::Cancel),
            overlay::Action::Press("copy-code") => {
                if let Account::Pairing { code, .. } = &self.account {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(code.clone()));
                }
            }
            overlay::Action::Press("open-pairing") => {
                if let Account::Pairing { url, .. } = &self.account {
                    cx.open_url(url);
                }
            }
            overlay::Action::Press("play-here") => self.send(Command::PlayHere),
            overlay::Action::Press("lyrics") => self.show_lyrics(self.page != Page::Lyrics),
            overlay::Action::Press("back") => self.back(),
            overlay::Action::Press("play-artist") => {
                if let Some((uri, Ok(artist))) = &self.discovery.artist {
                    let (uri, tracks) = (uri.clone(), artist.top_tracks.clone());
                    self.play_context(&uri, &tracks, 0, cx);
                }
            }
            overlay::Action::Press("play-collection") => {
                if let Some((uri, Ok(collection))) = &self.playlists.collection {
                    let (uri, tracks) = (uri.clone(), collection.tracks.clone());
                    self.play_context(&uri, &tracks, 0, cx);
                }
            }
            overlay::Action::Press(_) => {}
            overlay::Action::Seek(fraction) => {
                if let Some(duration) = self.duration() {
                    let position = (fraction * duration.saturating_sub(1) as f32) as u32;
                    self.seek_to(position, cx);
                }
            }
            overlay::Action::Volume(fraction) => {
                let volume = (fraction.clamp(0., 1.) * f32::from(u16::MAX)).round() as u16;
                self.playback.now.volume = volume;
                self.send(Command::Volume(volume));
            }
        }
        cx.notify();
    }

    pub fn handle_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Connecting => self.account = Account::Connecting,
            Event::Pairing { code, url } => self.account = Account::Pairing { code, url },
            Event::Connected => {
                self.account = Account::Connected;
                self.discovery.discovery_pending = true;
                self.send(Command::Discover);
                self.library.playlists_loading = true;
                self.send(Command::Playlists);
                if let Some(track) = self.initial_track.take() {
                    self.input
                        .update(cx, |input, cx| input.set_value(track, window, cx));
                    self.search(cx);
                }
            }
            Event::SignedOut(message) => {
                self.account = Account::SignedOut;
                self.account_error = message;
                self.discovery.search_error = None;
                self.discovery.results.clear();
                self.discovery.home = Discovery::default();
                self.discovery.newest_mix = None;
                self.discovery.last_seed = None;
                self.discovery.discovery_pending = false;
                self.page = Page::Home;
                self.history.clear();
                self.library.playlists.clear();
                self.discovery.artist = None;
                self.playlists.collection = None;
                self.covers.clear();
                self.playback.now = NowPlaying::default();
                self.content.lyrics = None;
                self.playback.notice = None;
                self.discovery.search_pending = false;
            }
            Event::Playlists(playlists) => {
                self.library.playlists = playlists;
                self.library.playlists_loading = false;
            }
            Event::Artist { uri, result } => self.discovery.artist = Some((uri, result)),
            Event::Collection { uri, result } => self.playlists.collection = Some((uri, result)),
            Event::SearchDone(result) => {
                self.discovery.search_pending = false;
                match result {
                    Ok(tracks) => self.discovery.results = tracks,
                    Err(error) => self.discovery.search_error = Some(error),
                }
            }
            Event::DirectTrack(result) => {
                self.discovery.search_pending = false;
                match result {
                    Ok(track) => {
                        self.discovery.results = vec![track.clone()];
                        self.play(&[track], 0, cx);
                    }
                    Err(error) => self.discovery.search_error = Some(error),
                }
            }
            Event::Discovery(home) => {
                self.discovery.discovery_pending = false;
                self.discovery.home = home;
            }
            Event::Recommendations(mix) => {
                if mix.is_some() {
                    self.discovery.newest_mix = mix;
                }
            }
            Event::Cover { url, jpeg } => {
                self.covers
                    .insert(url, Arc::new(Image::from_bytes(image_format(&jpeg), jpeg)));
                if self.covers.len() > COVER_CACHE_LIMIT {
                    self.evict_covers(window, cx);
                }
            }
            Event::NowPlaying(now) => {
                if let Some(track) = &now.track
                    && track.is_track()
                    && self
                        .content
                        .lyrics
                        .as_ref()
                        .is_none_or(|(uri, _)| *uri != track.uri)
                {
                    self.content.lyrics = Some((track.uri.clone(), None));
                    self.content.lyrics_loading = true;
                    self.content.lyrics_line = None;
                    self.send(Command::Lyrics(track.uri.clone()));
                }
                if let Some(track) = &now.track
                    && track.is_track()
                    && !track.title.is_empty()
                    && self.discovery.last_seed.as_ref() != Some(&track.uri)
                {
                    self.discovery.last_seed = Some(track.uri.clone());
                    self.send(Command::Recommend(track.clone()));
                }
                let expected = self.playback.now.position();
                let held = self.playback.seek_hold.filter(|(_, at)| {
                    at.elapsed() < Duration::from_secs(3)
                        && now.track.as_ref().map(|track| &track.uri)
                            == self.playback.now.track.as_ref().map(|track| &track.uri)
                        && now.position().abs_diff(expected) > 1500
                });
                let (position, observed) =
                    (self.playback.now.position_ms, self.playback.now.observed);
                self.playback.now = now;
                if held.is_some() {
                    self.playback.now.position_ms = position;
                    self.playback.now.observed = observed;
                } else {
                    self.playback.seek_hold = None;
                }
                self.follow_lyrics();
            }
            Event::Notice(message) => {
                self.playback.notice = Some(message);
                self.playback.seek_hold = None;
            }
            Event::Lyrics { uri, lyrics } => {
                if self
                    .content
                    .lyrics
                    .as_ref()
                    .is_some_and(|(pending, _)| *pending == uri)
                {
                    self.content.lyrics = Some((uri, lyrics));
                    self.content.lyrics_loading = false;
                    self.content.lyrics_line = None;
                    self.follow_lyrics();
                }
            }
        }
        cx.notify();
    }
}

const COVER_CACHE_LIMIT: usize = 220;

fn image_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else {
        ImageFormat::Jpeg
    }
}
