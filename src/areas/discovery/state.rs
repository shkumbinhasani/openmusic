use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use gpui::{App, Context, Window};
use gpui_component::input::InputEvent;
use spotify_client::{
    Track,
    catalog::Artist,
    search::{SearchKind, SearchResults},
};

use super::{
    artist::{ArtistView, FOLLOW_KEY, MORE_KEY, PLAY_KEY},
    browse::{BrowseAllState, BrowseState, Kept, SectionState},
    home::HomeFeed,
    search::{Keys, Recent, Suggest, TabState},
    worker::{Command, Event},
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
    pub feed: HomeFeed,
    pub browse_all: BrowseAllState,
    pub browse_pages: Kept<BrowseState>,
    pub sections: Kept<SectionState>,
    pub more_sentinel: Sentinel,
    pub artist: Option<(String, Result<Artist, String>)>,
    pub artist_view: ArtistView,
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
    pub fn artist_tracks(&self) -> &[Track] {
        match &self.artist {
            Some((_, Ok(artist))) => &artist.top_tracks,
            _ => &[],
        }
    }

    pub fn covers(&self, urls: &mut HashSet<String>) {
        self.search_covers(urls);
        urls.extend(self.feed.images());
        urls.extend(self.browse_images());
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

pub fn load(app: &mut OpenMusic, page: &Page) {
    match page {
        Page::Home => app.load_home(false),
        Page::Search(tab) => app.open_search_tab(*tab),
        Page::Browse(uri) => app.load_browse(uri),
        Page::Section { uri, source } => app.load_section(uri, *source),
        Page::Artist(uri) => {
            app.discovery.artist_view.show(uri);
            app.lookup_saved([uri.clone()]);
            let loaded = app
                .discovery
                .artist
                .as_ref()
                .is_some_and(|(loaded, result)| loaded == uri && result.is_ok());
            if !loaded {
                app.discovery.artist = None;
                app.send(Command::Artist(uri.clone()));
            }
        }
        _ => {}
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
        Event::Discovered(cached) => {
            if let Some(home) = cached.home {
                app.restore_home(home);
            }
            if let Some(all) = cached.browse {
                app.restore_browse_all(all);
            }
        }
        Event::Home { facet, result } => app.on_home(facet, result),
        Event::BrowseAll(result) => app.on_browse_all(result),
        Event::Browse {
            uri,
            offset,
            result,
        } => app.on_browse(uri, offset, result),
        Event::Section {
            uri,
            source,
            offset,
            result,
        } => app.on_section(uri, source, offset, result),
        Event::Artist { uri, result } => app.discovery.artist = Some((uri, result)),
        event => app.on_search_event(event, cx),
    }
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, cx: &mut Context<OpenMusic>) {
    app.discovery.feed.pending = None;
    app.discovery.browse_all.pending = false;
    app.load_home(true);
    app.watch_search_keys(cx);
    app.discovery.recent.pending = false;
    app.load_recent_searches(true);
}

pub fn signed_out(app: &mut OpenMusic) {
    app.discovery.reset_search();
    app.discovery.feed = HomeFeed::default();
    app.discovery.browse_all = BrowseAllState::default();
    app.discovery.browse_pages.clear();
    app.discovery.sections.clear();
    app.discovery.artist = None;
    app.discovery.artist_view = ArtistView::default();
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
        || app.feed_debug(verb, rest, directory, window, cx)
        || app.artist_debug(verb, rest, directory)
}

pub fn state_fragment(app: &OpenMusic) -> String {
    [
        app.discovery
            .search_fragment(&app.page, app.search_dropdown_visible()),
        app.home_fragment(),
        app.browse_fragment(),
        app.artist_fragment(),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" ")
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
        PLAY_KEY => app.play_artist(cx),
        FOLLOW_KEY => app.toggle_artist_follow(),
        "search-top-play" => app.play_top_result(cx),
        MORE_KEY => {
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
    if app.discovery.more_sentinel.take() && app.more_page_results() {
        cx.notify();
    }
}
