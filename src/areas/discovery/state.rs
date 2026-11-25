use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use gpui::{App, Context, Window};
use gpui_component::input::InputEvent;
use spotify_client::{
    catalog::Artist,
    engine::NowPlaying,
    search::{SearchKind, SearchResults},
};

use super::{
    search::{Keys, Recent, Suggest, TabState},
    worker::{Command, Discovery, Event, Mix},
};
use crate::{
    app::{OpenMusic, Page, intent::Intent},
    menus::{self, FocusSearch, Handle},
    ui::{
        components::Sentinel,
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
    pub home: Discovery,
    pub newest_mix: Option<Mix>,
    pub last_seed: Option<String>,
    pub discovery_pending: bool,
    pub artist: Option<(String, Result<Artist, String>)>,
    pub search_query: String,
    pub search_pending: bool,
    pub search: Option<Result<Box<SearchResults>, String>>,
    pub search_tabs: HashMap<SearchKind, TabState>,
    pub search_sentinel: Sentinel,
    pub suggest: Suggest,
    pub recent: Recent,
    pub search_keys: Keys,
}

impl State {
    pub fn station(&self) -> Option<&Mix> {
        self.newest_mix.as_ref().or(self.home.mix.as_ref())
    }

    pub fn covers(&self, urls: &mut HashSet<String>) {
        self.search_covers(urls);
        let tracks = self
            .home
            .tracks
            .iter()
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
    if let Page::Search(tab) = page {
        app.open_search_tab(*tab);
    }
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
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    app.search_input(event, window, cx);
}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    match event {
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
        event => app.on_search_event(event, cx),
    }
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, cx: &mut Context<OpenMusic>) {
    app.discovery.discovery_pending = true;
    app.send(Command::Discover);
    app.watch_search_keys(cx);
    app.discovery.recent.pending = false;
    app.load_recent_searches(true);
}

pub fn signed_out(app: &mut OpenMusic) {
    app.discovery.reset_search();
    app.discovery.home = Discovery::default();
    app.discovery.newest_mix = None;
    app.discovery.last_seed = None;
    app.discovery.discovery_pending = false;
    app.discovery.artist = None;
}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    app.search_debug(verb, rest, directory, window, cx)
}

pub fn state_fragment(app: &OpenMusic) -> String {
    app.discovery
        .search_fragment(&app.page, app.search_dropdown_visible())
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
        "search-top-play" => app.play_top_result(cx),
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

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    if app.discovery.search_sentinel.take() && app.more_search_results() {
        cx.notify();
    }
}
