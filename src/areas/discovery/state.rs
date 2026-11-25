use std::{collections::HashSet, path::Path};

use gpui::{App, Context, Window};
use gpui_component::input::InputEvent;
use spotify_client::{Track, catalog::Artist, engine::NowPlaying};

use super::worker::{Command, Discovery, Event, Mix};
use crate::{
    app::{OpenMusic, Page, Session, intent::Intent},
    menus::{self, FocusSearch, Handle},
    ui::{
        menu::Target,
        tracks::{RowDrag, RowDrop},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchTab {
    All,
    Tracks,
    Artists,
    Albums,
    Playlists,
    Podcasts,
    Episodes,
    Audiobooks,
    Authors,
    Users,
    Genres,
}

impl SearchTab {
    pub const TABS: [Self; 11] = [
        Self::All,
        Self::Tracks,
        Self::Artists,
        Self::Albums,
        Self::Playlists,
        Self::Podcasts,
        Self::Episodes,
        Self::Audiobooks,
        Self::Authors,
        Self::Users,
        Self::Genres,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Tracks => "tracks",
            Self::Artists => "artists",
            Self::Albums => "albums",
            Self::Playlists => "playlists",
            Self::Podcasts => "podcasts",
            Self::Episodes => "episodes",
            Self::Audiobooks => "audiobooks",
            Self::Authors => "authors",
            Self::Users => "users",
            Self::Genres => "genres",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::TABS.into_iter().find(|tab| tab.name() == name)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SectionSource {
    Home,
    Browse,
}

impl SectionSource {
    pub const SOURCES: [Self; 2] = [Self::Home, Self::Browse];

    pub fn name(self) -> &'static str {
        match self {
            Self::Home => "home",
            Self::Browse => "browse",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::SOURCES
            .into_iter()
            .find(|source| source.name() == name)
    }
}

#[derive(Default)]
pub struct State {
    pub results: Vec<Track>,
    pub home: Discovery,
    pub newest_mix: Option<Mix>,
    pub last_seed: Option<String>,
    pub discovery_pending: bool,
    pub artist: Option<(String, Result<Artist, String>)>,
    pub search_query: String,
    pub search_pending: bool,
    pub search_error: Option<String>,
}

impl State {
    pub fn station(&self) -> Option<&Mix> {
        self.newest_mix.as_ref().or(self.home.mix.as_ref())
    }

    pub fn covers(&self, urls: &mut HashSet<String>) {
        let tracks = self
            .results
            .iter()
            .chain(&self.home.tracks)
            .chain(self.home.mix.iter().flat_map(|mix| &mix.tracks))
            .chain(self.newest_mix.iter().flat_map(|mix| &mix.tracks));
        urls.extend(tracks.filter_map(|track| track.cover.clone()));
        if let Some((_, Ok(artist))) = &self.artist {
            urls.extend(artist.image.clone());
            urls.extend(
                artist
                    .top_tracks
                    .iter()
                    .filter_map(|track| track.cover.clone()),
            );
            urls.extend(
                artist
                    .releases
                    .iter()
                    .filter_map(|release| release.cover.clone()),
            );
            urls.extend(
                artist
                    .related
                    .iter()
                    .filter_map(|related| related.image.clone()),
            );
        }
    }
}

impl OpenMusic {
    pub fn search(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value();
        let query = value.trim();
        if query.is_empty()
            || self.discovery.search_pending
            || !matches!(self.session, Session::Connected)
        {
            return;
        }
        self.navigate(Page::Search(SearchTab::All));
        self.discovery.search_query = query.to_owned();
        self.discovery.search_pending = true;
        self.discovery.search_error = None;
        self.discovery.results.clear();
        self.send(Command::Search(query.to_owned()));
        cx.notify();
    }

    pub fn recommend_from(&mut self, now: &NowPlaying) {
        if let Some(track) = &now.track
            && track.is_track()
            && !track.title.is_empty()
            && self.discovery.last_seed.as_ref() != Some(&track.uri)
        {
            self.discovery.last_seed = Some(track.uri.clone());
            self.send(Command::Recommend(Box::new(track.clone())));
        }
    }
}

pub fn load(app: &mut OpenMusic, page: &Page) {
    if let Page::Artist(uri) = page
        && app
            .discovery
            .artist
            .as_ref()
            .is_none_or(|(loaded, _)| loaded != uri)
    {
        app.discovery.artist = None;
        app.send(Command::Artist(uri.clone()));
    }
}

pub fn on_input(
    app: &mut OpenMusic,
    event: &InputEvent,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    if let InputEvent::PressEnter { .. } = event {
        app.search(cx);
    }
}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::SearchDone(result) => {
            app.discovery.search_pending = false;
            match result {
                Ok(tracks) => app.discovery.results = tracks,
                Err(error) => app.discovery.search_error = Some(error),
            }
        }
        Event::DirectTrack(result) => {
            app.discovery.search_pending = false;
            match result {
                Ok(track) => {
                    app.discovery.results = vec![track.clone()];
                    app.play(&[track], 0, cx);
                }
                Err(error) => app.discovery.search_error = Some(error),
            }
        }
        Event::Discovered(home) => {
            app.discovery.discovery_pending = false;
            app.discovery.home = home;
        }
        Event::Recommendations(mix) => {
            if mix.is_some() {
                app.discovery.newest_mix = mix;
            }
        }
        Event::Artist { uri, result } => app.discovery.artist = Some((uri, result)),
    }
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {
    app.discovery.discovery_pending = true;
    app.send(Command::Discover);
}

pub fn signed_out(app: &mut OpenMusic) {
    app.discovery.search_error = None;
    app.discovery.results.clear();
    app.discovery.home = Discovery::default();
    app.discovery.newest_mix = None;
    app.discovery.last_seed = None;
    app.discovery.discovery_pending = false;
    app.discovery.artist = None;
    app.discovery.search_pending = false;
}

pub fn debug(
    _app: &mut OpenMusic,
    _verb: &str,
    _rest: &str,
    _directory: &Path,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
}

pub fn state_fragment(_app: &OpenMusic) -> String {
    String::new()
}

pub fn register_actions(cx: &mut App, handle: &Handle) {
    menus::route(cx, handle, |_: &FocusSearch, app, window, cx| {
        app.perform(Intent::FocusSearch, window, cx)
    });
}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        "play-station" => {
            if let Some(mix) = app.discovery.station() {
                let tracks = mix.tracks.clone();
                app.play(&tracks, 0, cx);
            }
        }
        "play-artist" => {
            if let Some((uri, Ok(artist))) = &app.discovery.artist {
                let (uri, tracks) = (uri.clone(), artist.top_tracks.clone());
                app.play_context(&uri, &tracks, 0, cx);
            }
        }
        "more-artist" => {
            if let Page::Artist(uri) = &app.page
                && let Some(target) = Target::for_uri(uri)
            {
                app.open_menu_under(target, key);
            }
        }
        _ => return false,
    }
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

pub fn tick(_app: &mut OpenMusic, _cx: &mut Context<OpenMusic>) {}
