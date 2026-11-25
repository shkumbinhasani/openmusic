use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{Duration, Instant},
};

use gpui::{Context, Focusable, ParentElement, Styled, Subscription, Window, div, px};
use gpui_component::{
    ActiveTheme, WindowExt, button::ButtonVariant, dialog::DialogButtonProps, input::InputEvent,
};
use spotify_client::{
    Track, catalog,
    entity::{Card, Kind},
    search::{Hit, SearchKind, SearchPage, SearchResults, Suggestion, Suggestions, page_limit},
};

use super::{state::SearchTab, worker::Command};
use crate::{
    app::{OpenMusic, Page, Session, page_for},
    ui::{
        cards::CardOpen,
        tracks::{RowClick, RowOptions},
    },
};

pub const RECENT_STALE: Duration = Duration::from_secs(300);
const SUGGEST_FRESH: Duration = Duration::from_secs(300);
const SUGGEST_CACHE_LIMIT: usize = 64;
pub const QUERY_ROWS: usize = 4;
pub const ENTITY_ROWS: usize = 6;
pub const RECENT_ROWS: usize = 8;
pub const TOP_SONGS: usize = 4;
pub const SONG_ROWS: [&str; 2] = ["search-track", "search-tab-track"];

impl SearchTab {
    pub fn kind(self) -> Option<SearchKind> {
        Some(match self {
            Self::All => return None,
            Self::Tracks => SearchKind::Tracks,
            Self::Artists => SearchKind::Artists,
            Self::Albums => SearchKind::Albums,
            Self::Playlists => SearchKind::Playlists,
            Self::Podcasts => SearchKind::Podcasts,
            Self::Episodes => SearchKind::Episodes,
            Self::Audiobooks => SearchKind::Audiobooks,
            Self::Authors => SearchKind::Authors,
            Self::Users => SearchKind::Users,
            Self::Genres => SearchKind::Genres,
        })
    }

    pub fn of(kind: SearchKind) -> Self {
        match kind {
            SearchKind::Tracks => Self::Tracks,
            SearchKind::Artists => Self::Artists,
            SearchKind::Albums => Self::Albums,
            SearchKind::Playlists => Self::Playlists,
            SearchKind::Podcasts => Self::Podcasts,
            SearchKind::Episodes => Self::Episodes,
            SearchKind::Audiobooks => Self::Audiobooks,
            SearchKind::Authors => Self::Authors,
            SearchKind::Users => Self::Users,
            SearchKind::Genres => Self::Genres,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Tracks => "Songs",
            Self::Artists => "Artists",
            Self::Albums => "Albums",
            Self::Playlists => "Playlists",
            Self::Podcasts => "Podcasts & Shows",
            Self::Episodes => "Episodes",
            Self::Audiobooks => "Audiobooks",
            Self::Authors => "Authors",
            Self::Users => "Profiles",
            Self::Genres => "Genres & Moods",
        }
    }

    pub fn shelf_title(self) -> &'static str {
        match self {
            Self::Podcasts => "Shows",
            tab => tab.label(),
        }
    }

    pub fn from_alias(name: &str) -> Option<Self> {
        match name {
            "songs" => Some(Self::Tracks),
            "profiles" => Some(Self::Users),
            "shows" => Some(Self::Podcasts),
            name => Self::from_name(name),
        }
    }
}

#[derive(Default)]
pub struct TabState {
    pub items: Option<SearchPage>,
    pub next: Option<u32>,
    pub pending: Option<u32>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Suggest {
    pub text: String,
    pub focused: bool,
    pub open: bool,
    pub index: Option<usize>,
    pub current: Option<(String, Suggestions)>,
    cache: HashMap<String, (Instant, Suggestions)>,
}

#[derive(Default)]
pub struct Recent {
    pub items: Vec<Hit>,
    pub loaded_at: Option<Instant>,
    pub pending: bool,
    backup: Option<Vec<Hit>>,
}

#[derive(Default)]
pub struct Keys(Option<Subscription>);

#[derive(Clone, Debug, PartialEq)]
pub enum DropRow {
    Query(String),
    Entity(Box<Hit>),
    Recent(Box<Hit>),
}

impl DropRow {
    pub fn describe(&self) -> String {
        match self {
            Self::Query(text) => format!("query {text}"),
            Self::Entity(hit) => format!(
                "entity {:?} {} {}",
                hit.card.kind,
                hit.uri(),
                hit.card.title
            ),
            Self::Recent(hit) => format!(
                "recent {:?} {} {}",
                hit.card.kind,
                hit.uri(),
                hit.card.title
            ),
        }
    }
}

fn is_link(text: &str) -> bool {
    !matches!(catalog::parse_link(text), Ok(None))
}

pub fn link_page(text: &str) -> Option<Page> {
    let uri = catalog::parse_link(text).ok()??;
    if uri.starts_with("spotify:track:") {
        return None;
    }
    page_for(&uri)
}

fn starts_with_folded(text: &str, prefix: &str) -> bool {
    text.to_lowercase().starts_with(&prefix.to_lowercase())
}

pub fn drop_rows(suggest: &Suggest, recent: &[Hit]) -> Vec<DropRow> {
    if suggest.text.is_empty() {
        return recent
            .iter()
            .take(RECENT_ROWS)
            .cloned()
            .map(|hit| DropRow::Recent(Box::new(hit)))
            .collect();
    }
    let Some((prefix, suggestions)) = &suggest.current else {
        return Vec::new();
    };
    if !starts_with_folded(&suggest.text, prefix) || is_link(&suggest.text) {
        return Vec::new();
    }
    let queries = suggestions
        .items
        .iter()
        .filter_map(|item| match item {
            Suggestion::Query(text) => Some(DropRow::Query(text.clone())),
            Suggestion::Entity(_) => None,
        })
        .take(QUERY_ROWS);
    let entities = suggestions
        .items
        .iter()
        .filter_map(|item| match item {
            Suggestion::Entity(hit) => Some(DropRow::Entity(hit.clone())),
            Suggestion::Query(_) => None,
        })
        .take(ENTITY_ROWS);
    queries.chain(entities).collect()
}

pub fn remembered(recent: &mut Vec<Hit>, hit: Hit) {
    recent.retain(|known| known.uri() != hit.uri());
    recent.insert(0, hit);
}

pub fn step(index: Option<usize>, rows: usize, down: bool) -> Option<usize> {
    if rows == 0 {
        return None;
    }
    match (index, down) {
        (None, true) => Some(0),
        (None, false) => Some(rows - 1),
        (Some(index), true) => Some((index + 1).min(rows - 1)),
        (Some(0), false) => None,
        (Some(index), false) => Some(index.min(rows) - 1),
    }
}

impl super::state::State {
    pub fn search_tracks(&self) -> &[Track] {
        match &self.search {
            Some(Ok(results)) => &results.tracks.items,
            _ => &[],
        }
    }

    pub fn search_results(&self) -> Option<&SearchResults> {
        match &self.search {
            Some(Ok(results)) => Some(results),
            _ => None,
        }
    }

    pub fn is_unplayable(&self, uri: &str) -> bool {
        self.search_results()
            .is_some_and(|results| results.unplayable.contains(uri))
            || self.search_tabs.values().any(|tab| {
                matches!(&tab.items, Some(SearchPage::Tracks { unplayable, .. }) if unplayable.contains(uri))
            })
    }

    pub fn drop_rows(&self) -> Vec<DropRow> {
        drop_rows(&self.suggest, &self.recent.items)
    }

    pub fn suggestions_visible(&self) -> bool {
        self.suggest.open && self.suggest.focused && !self.drop_rows().is_empty()
    }

    pub fn recent_uris(&self) -> Vec<String> {
        self.recent
            .items
            .iter()
            .map(|hit| hit.uri().to_owned())
            .collect()
    }

    pub fn search_covers(&self, urls: &mut HashSet<String>) {
        if let Some(results) = self.search_results() {
            urls.extend(results.images());
        }
        for tab in self.search_tabs.values() {
            urls.extend(tab.items.iter().flat_map(SearchPage::images));
        }
        if let Some((_, suggestions)) = &self.suggest.current {
            urls.extend(suggestions.images());
        }
        urls.extend(
            self.recent
                .items
                .iter()
                .filter_map(|hit| hit.image().cloned()),
        );
    }

    pub fn reset_search(&mut self) {
        self.search_query.clear();
        self.search = None;
        self.search_pending = false;
        self.search_tabs.clear();
        self.suggest = Suggest::default();
        self.recent = Recent::default();
    }

    pub fn search_fragment(&self, page: &Page, dropdown: bool) -> String {
        let mut parts = vec![format!(
            "search_query={:?} search={}",
            self.search_query,
            match &self.search {
                None if self.search_pending => "pending".to_owned(),
                None => "none".to_owned(),
                Some(Err(_)) => "error".to_owned(),
                Some(Ok(results)) => format!(
                    "ok search_top={} search_songs={}/{} search_featured={} search_chips={}",
                    results.top.len(),
                    results.tracks.items.len(),
                    results.tracks.total,
                    results.featured.len(),
                    results
                        .chips
                        .iter()
                        .map(|kind| SearchTab::of(*kind).name())
                        .collect::<Vec<_>>()
                        .join(","),
                ),
            }
        )];
        if let Page::Search(tab) = page
            && let Some(kind) = tab.kind()
        {
            let state = self.search_tabs.get(&kind);
            parts.push(format!(
                "search_tab={} search_tab_items={} search_tab_total={} search_tab_next={:?} search_tab_pending={:?}",
                tab.name(),
                state
                    .and_then(|tab| tab.items.as_ref())
                    .map_or(0, SearchPage::len),
                state
                    .and_then(|tab| tab.items.as_ref())
                    .map_or(0, SearchPage::total),
                state.and_then(|tab| tab.next),
                state.and_then(|tab| tab.pending),
            ));
        }
        parts.push(format!(
            "search_suggestions={} suggest_open={} recent_searches={}",
            self.drop_rows().len(),
            dropdown,
            self.recent.items.len(),
        ));
        parts.join(" ")
    }
}

impl OpenMusic {
    pub fn search_dropdown_visible(&self) -> bool {
        let landing = self.discovery.suggest.text.is_empty()
            && self.discovery.search_query.is_empty()
            && matches!(self.page, Page::Search(_));
        !landing && self.discovery.suggestions_visible()
    }

    pub fn search(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value();
        let query = value.trim().to_owned();
        self.run_search(query, cx);
    }

    pub fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        if query.is_empty() || !matches!(self.session, Session::Connected) {
            return;
        }
        self.discovery.suggest.open = false;
        self.discovery.suggest.index = None;
        if let Some(page) = link_page(&query) {
            self.navigate(page);
            cx.notify();
            return;
        }
        let tab = match self.page {
            Page::Search(tab) => tab,
            _ => SearchTab::All,
        };
        let same = query == self.discovery.search_query
            && (self.discovery.search_pending || matches!(self.discovery.search, Some(Ok(_))));
        if !same {
            self.discovery.search_query = query.clone();
            self.discovery.search = None;
            self.discovery.search_pending = true;
            self.discovery.search_tabs.clear();
            self.send(Command::Search(query));
        }
        if self.page == Page::Search(tab) {
            self.open_search_tab(tab);
        } else {
            self.navigate(Page::Search(tab));
        }
        cx.notify();
    }

    pub fn open_search_tab(&mut self, tab: SearchTab) {
        if self.discovery.search_query.is_empty() {
            self.load_recent_searches(false);
            return;
        }
        if let Some(kind) = tab.kind() {
            let waiting = self
                .discovery
                .search_tabs
                .get(&kind)
                .is_none_or(|state| state.items.is_none() && state.pending.is_none());
            if waiting {
                self.request_search_page(kind, true);
            }
        }
    }

    pub fn request_search_page(&mut self, kind: SearchKind, first: bool) -> bool {
        let query = self.discovery.search_query.clone();
        if query.is_empty() {
            return false;
        }
        let known_total = self
            .discovery
            .search_results()
            .map(|results| results.total(kind))
            .filter(|total| *total > 0);
        let state = self.discovery.search_tabs.entry(kind).or_default();
        if state.pending.is_some() {
            return false;
        }
        let offset = match (&state.items, first) {
            (None, true) => 0,
            (Some(_), false) => match state.next {
                Some(next) => next,
                None => return false,
            },
            _ => return false,
        };
        let total = state.items.as_ref().map(SearchPage::total).or(known_total);
        state.pending = Some(offset);
        state.error = None;
        self.send(Command::SearchTab {
            query,
            kind,
            offset,
            limit: page_limit(kind, offset, total),
        });
        true
    }

    pub fn more_search_results(&mut self) -> bool {
        match self.page {
            Page::Search(tab) => tab
                .kind()
                .is_some_and(|kind| self.request_search_page(kind, false)),
            _ => false,
        }
    }

    pub fn load_recent_searches(&mut self, force: bool) {
        let recent = &mut self.discovery.recent;
        let fresh = recent
            .loaded_at
            .is_some_and(|at| at.elapsed() < RECENT_STALE);
        if recent.pending || (fresh && !force) || !matches!(self.session, Session::Connected) {
            return;
        }
        recent.pending = true;
        self.send(Command::RecentSearches);
    }

    pub fn remember_search(&mut self, hit: Hit) {
        let uri = hit.uri().to_owned();
        remembered(&mut self.discovery.recent.items, hit);
        self.send(Command::RememberSearches(vec![uri]));
    }

    pub fn remember_card(&mut self, card: &Card) {
        self.remember_search(Hit::of_card(card.clone()));
    }

    pub fn forget_search(&mut self, uri: &str) {
        let recent = &mut self.discovery.recent;
        if !recent.items.iter().any(|hit| hit.uri() == uri) {
            return;
        }
        recent.backup.get_or_insert_with(|| recent.items.clone());
        recent.items.retain(|hit| hit.uri() != uri);
        let count = self.discovery.drop_rows().len();
        let suggest = &mut self.discovery.suggest;
        suggest.index = suggest.index.filter(|index| *index < count);
        self.send(Command::ForgetSearches(vec![uri.to_owned()]));
    }

    pub fn clear_recent_searches(&mut self) {
        let uris = self.discovery.recent_uris();
        if uris.is_empty() {
            return;
        }
        let recent = &mut self.discovery.recent;
        recent.backup.get_or_insert_with(|| recent.items.clone());
        recent.items.clear();
        self.discovery.suggest.index = None;
        self.send(Command::ForgetSearches(uris));
    }

    pub fn confirm_clear_recent_searches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.discovery.recent.items.is_empty() {
            return;
        }
        let app = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let app = app.clone();
            dialog
                .title("Clear recent searches?")
                .child(
                    div()
                        .text_size(px(13.5))
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            "This removes your search history from Spotify on all your devices. You can’t undo this.",
                        ),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Clear")
                        .ok_variant(ButtonVariant::Danger),
                )
                .on_ok(move |_, _, cx| {
                    let _ = app.update(cx, |app, cx| {
                        app.clear_recent_searches();
                        cx.notify();
                    });
                    true
                })
                .w(px(400.))
                .bg(cx.theme().popover)
        });
    }

    pub fn open_hit(&mut self, hit: Hit, cx: &mut Context<Self>) {
        let track = hit.playable_track();
        let page = page_for(hit.uri());
        self.remember_search(hit);
        match (track, page) {
            (Some(track), _) => self.play(&[track], 0, cx),
            (None, Some(page)) => self.navigate(page),
            (None, None) => {}
        }
    }

    pub fn play_search_track(&mut self, tracks: &[Track], index: usize, cx: &mut Context<Self>) {
        let Some(track) = tracks.get(index) else {
            return;
        };
        if self.discovery.is_unplayable(&track.uri) {
            self.show_toast("This song isn’t available.", None);
            return;
        }
        self.remember_search(Hit::of_track(track.clone()));
        self.play(tracks, index, cx);
    }

    pub fn search_card_open(&self) -> CardOpen {
        std::rc::Rc::new(
            |app: &mut OpenMusic, card: &Card, _: &mut Window, _: &mut Context<OpenMusic>| {
                app.remember_card(card);
            },
        )
    }

    pub fn search_row_options(&self, show_album: bool) -> RowOptions {
        let click: RowClick = std::rc::Rc::new(|app, tracks, index, _, cx| {
            app.play_search_track(tracks, index, cx);
        });
        RowOptions {
            show_album,
            on_click: Some(click),
            ..RowOptions::default()
        }
    }

    pub fn play_top_result(&mut self, cx: &mut Context<Self>) {
        let Some(hit) = self
            .discovery
            .search_results()
            .and_then(|results| results.top.first())
            .cloned()
        else {
            return;
        };
        if let Some(track) = hit.playable_track() {
            let tracks = [track];
            self.play_search_track(&tracks, 0, cx);
            return;
        }
        if !top_result_plays(&hit.card) {
            return;
        }
        let uri = hit.uri().to_owned();
        self.remember_search(hit);
        self.play_context_from(
            &uri,
            &[],
            0,
            "search",
            spotify_client::connect::PlayOptions::default(),
            cx,
        );
    }

    pub fn move_suggestion(&mut self, down: bool) {
        let rows = self.discovery.drop_rows().len();
        let suggest = &mut self.discovery.suggest;
        suggest.index = step(suggest.index, rows, down);
    }

    pub fn close_suggestions(&mut self) {
        self.discovery.suggest.open = false;
        self.discovery.suggest.index = None;
    }

    pub fn activate_drop_row(&mut self, row: DropRow, window: &mut Window, cx: &mut Context<Self>) {
        self.close_suggestions();
        window.focus(&self.focus);
        match row {
            DropRow::Query(text) => {
                self.input
                    .update(cx, |input, cx| input.set_value(text.clone(), window, cx));
                self.discovery.suggest.text = text.clone();
                self.run_search(text, cx);
            }
            DropRow::Entity(hit) | DropRow::Recent(hit) => self.open_hit(*hit, cx),
        }
        cx.notify();
    }

    pub fn activate_selected_suggestion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.search_dropdown_visible() {
            return false;
        }
        let Some(row) = self
            .discovery
            .suggest
            .index
            .and_then(|index| self.discovery.drop_rows().into_iter().nth(index))
        else {
            return false;
        };
        self.activate_drop_row(row, window, cx);
        true
    }

    pub fn submit_search_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.activate_selected_suggestion(window, cx) {
            self.close_suggestions();
            self.search(cx);
        }
    }

    fn text_changed(&mut self, text: String, focused: bool) {
        let suggest = &mut self.discovery.suggest;
        if text == suggest.text {
            return;
        }
        suggest.text = text.clone();
        suggest.index = None;
        if text.is_empty() && !self.discovery.search_query.is_empty() {
            self.discovery.search_query.clear();
            self.discovery.search = None;
            self.discovery.search_pending = false;
            self.discovery.search_tabs.clear();
        }
        let suggest = &mut self.discovery.suggest;
        if !focused {
            return;
        }
        suggest.open = true;
        if text.is_empty() || is_link(&text) {
            suggest.current = None;
            return;
        }
        let folded = text.to_lowercase();
        if let Some((at, cached)) = suggest.cache.get(&folded)
            && at.elapsed() < SUGGEST_FRESH
        {
            suggest.current = Some((text, cached.clone()));
            return;
        }
        if suggest
            .current
            .as_ref()
            .is_some_and(|(prefix, _)| !starts_with_folded(&text, prefix))
        {
            suggest.current = None;
        }
        self.send(Command::Suggest(text));
    }

    pub fn search_input(
        &mut self,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::PressEnter { .. } => self.submit_search_field(window, cx),
            InputEvent::Change => {
                let text = self.input.read(cx).value().trim().to_owned();
                let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
                self.text_changed(text, focused);
            }
            InputEvent::Focus => {
                let suggest = &mut self.discovery.suggest;
                suggest.focused = true;
                suggest.open = true;
                suggest.index = None;
                self.load_recent_searches(false);
            }
            InputEvent::Blur => {
                self.discovery.suggest.focused = false;
                self.close_suggestions();
            }
        }
        cx.notify();
    }

    pub fn watch_search_keys(&mut self, cx: &mut Context<Self>) {
        if self.discovery.search_keys.0.is_some() {
            return;
        }
        let app = cx.entity().downgrade();
        let subscription = cx.intercept_keystrokes(move |event, window, cx| {
            let key = event.keystroke.key.as_str();
            if event.keystroke.modifiers.modified() || !matches!(key, "up" | "down" | "escape") {
                return;
            }
            let _ = app.update(cx, |app, cx| {
                if !app.search_dropdown_visible()
                    || !app.input.read(cx).focus_handle(cx).is_focused(window)
                {
                    return;
                }
                match key {
                    "up" => app.move_suggestion(false),
                    "down" => app.move_suggestion(true),
                    _ => app.close_suggestions(),
                }
                cx.stop_propagation();
                cx.notify();
            });
        });
        self.discovery.search_keys = Keys(Some(subscription));
    }

    pub fn on_search_event(&mut self, event: super::worker::Event, cx: &mut Context<Self>) {
        use super::worker::Event;
        match event {
            Event::Search { query, result } => {
                if query == self.discovery.search_query {
                    self.discovery.search_pending = false;
                    self.discovery.search = Some(result);
                }
            }
            Event::DirectTrack(result) => {
                self.discovery.search_pending = false;
                match result {
                    Ok(track) => {
                        let results = SearchResults {
                            query: self.discovery.search_query.clone(),
                            tracks: spotify_client::search::Paged {
                                items: vec![track.clone()],
                                total: 1,
                                next: None,
                            },
                            ..SearchResults::default()
                        };
                        self.discovery.search = Some(Ok(Box::new(results)));
                        self.play(&[track], 0, cx);
                    }
                    Err(error) => self.discovery.search = Some(Err(error)),
                }
            }
            Event::SearchTab {
                query,
                kind,
                offset,
                result,
            } => {
                if query != self.discovery.search_query {
                    return;
                }
                let Some(state) = self.discovery.search_tabs.get_mut(&kind) else {
                    return;
                };
                if state.pending != Some(offset) {
                    return;
                }
                state.pending = None;
                match result {
                    Ok(page) => match &mut state.items {
                        Some(items) if offset > 0 => {
                            items.append(page);
                            state.next = items.next();
                        }
                        _ => {
                            state.next = page.next();
                            state.items = Some(page);
                        }
                    },
                    Err(error) => state.error = Some(error),
                }
            }
            Event::Suggestions { prefix, result } => {
                let Ok(suggestions) = result else {
                    return;
                };
                let suggest = &mut self.discovery.suggest;
                if suggest.cache.len() >= SUGGEST_CACHE_LIMIT {
                    suggest
                        .cache
                        .retain(|_, (at, _)| at.elapsed() < SUGGEST_FRESH);
                    if suggest.cache.len() >= SUGGEST_CACHE_LIMIT {
                        suggest.cache.clear();
                    }
                }
                suggest
                    .cache
                    .insert(prefix.to_lowercase(), (Instant::now(), suggestions.clone()));
                if starts_with_folded(&suggest.text, &prefix) {
                    let newer = suggest.current.as_ref().is_some_and(|(current, _)| {
                        current.len() > prefix.len() && starts_with_folded(&suggest.text, current)
                    });
                    if !newer {
                        suggest.current = Some((prefix, suggestions));
                        suggest.index = None;
                    }
                }
            }
            Event::RecentSearches { result, notice } => {
                let recent = &mut self.discovery.recent;
                recent.pending = false;
                let backup = recent.backup.take();
                match result {
                    Ok(items) => {
                        recent.items = items;
                        recent.loaded_at = Some(Instant::now());
                    }
                    Err(_) => {
                        if notice.is_some()
                            && let Some(backup) = backup
                        {
                            recent.items = backup;
                        }
                    }
                }
                let count = self.discovery.drop_rows().len();
                let suggest = &mut self.discovery.suggest;
                suggest.index = suggest.index.filter(|index| *index < count);
                if let Some(notice) = notice {
                    self.show_toast(notice, None);
                }
            }
            _ => {}
        }
    }

    pub fn search_debug(
        &mut self,
        verb: &str,
        rest: &str,
        directory: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match verb {
            "tab" => match SearchTab::from_alias(rest) {
                Some(tab) => self.navigate(Page::Search(tab)),
                None => return false,
            },
            "more" if matches!(self.page, Page::Search(_)) => {
                self.more_search_results();
            }
            "link" if !rest.is_empty() => {
                let text = rest.to_owned();
                self.input
                    .update(cx, |input, cx| input.set_value(text.clone(), window, cx));
                self.run_search(text, cx);
            }
            "suggest" if rest.is_empty() => {
                let suggest = &self.discovery.suggest;
                let mut lines = vec![format!(
                    "text={:?} focused={} open={} visible={} index={:?} prefix={:?}",
                    suggest.text,
                    suggest.focused,
                    suggest.open,
                    self.search_dropdown_visible(),
                    suggest.index,
                    suggest.current.as_ref().map(|(prefix, _)| prefix.as_str()),
                )];
                lines.extend(self.discovery.drop_rows().iter().map(DropRow::describe));
                let _ = std::fs::write(directory.join("suggest.txt"), lines.join("\n"));
            }
            "suggest" => {
                let text = rest.to_owned();
                self.input.update(cx, |input, cx| {
                    input.focus(window, cx);
                    input.set_value(text, window, cx);
                });
            }
            "suggest-key" => match rest {
                "up" => self.move_suggestion(false),
                "down" => self.move_suggestion(true),
                "escape" => self.close_suggestions(),
                "enter" => self.submit_search_field(window, cx),
                _ => {}
            },
            "recent-searches" => {
                let (action, uri) = rest.split_once(' ').unwrap_or((rest, ""));
                match (action, uri.trim()) {
                    ("reload", _) => self.load_recent_searches(true),
                    ("save", uri) if !uri.is_empty() => {
                        let hit = self.known_hit(uri);
                        self.remember_search(hit);
                    }
                    ("remove", uri) if !uri.is_empty() => self.forget_search(uri),
                    _ => {
                        let lines: Vec<String> = self
                            .discovery
                            .recent
                            .items
                            .iter()
                            .map(|hit| {
                                format!("{:?} {} {}", hit.card.kind, hit.uri(), hit.card.title)
                            })
                            .collect();
                        let header = format!(
                            "count={} pending={} loaded={}",
                            lines.len(),
                            self.discovery.recent.pending,
                            self.discovery.recent.loaded_at.is_some()
                        );
                        let text = std::iter::once(header)
                            .chain(lines)
                            .collect::<Vec<_>>()
                            .join("\n");
                        let _ = std::fs::write(directory.join("recent-searches.txt"), text);
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn known_hit(&self, uri: &str) -> Hit {
        let results = self.discovery.search_results();
        let found = results.and_then(|results| {
            results
                .top
                .iter()
                .find(|hit| hit.uri() == uri)
                .cloned()
                .or_else(|| {
                    results
                        .tracks
                        .items
                        .iter()
                        .find(|track| track.uri == uri)
                        .cloned()
                        .map(Hit::of_track)
                })
                .or_else(|| {
                    results
                        .featured
                        .iter()
                        .chain(results.cards.iter().flat_map(|(_, page)| &page.items))
                        .find(|card| card.uri == uri)
                        .cloned()
                        .map(Hit::of_card)
                })
        });
        found.unwrap_or_else(|| {
            let kind = match uri.split(':').nth(1) {
                Some("track") => Kind::Track,
                Some("album") => Kind::Album,
                Some("playlist") => Kind::Playlist,
                Some("show") => Kind::Show,
                Some("episode") => Kind::Episode,
                Some("user") => Kind::User,
                Some("genre") => Kind::Genre,
                _ => Kind::Artist,
            };
            Hit::of_card(crate::ui::cards::card(uri, kind, uri, "", None))
        })
    }
}

pub fn top_result_plays(card: &Card) -> bool {
    card.playable
        && matches!(
            card.kind,
            Kind::Track | Kind::Album | Kind::Playlist | Kind::Artist
        )
}

#[cfg(test)]
mod tests {
    use spotify_client::entity::Kind;

    use super::*;

    fn hit(id: &str, kind: Kind) -> Hit {
        Hit::of_card(crate::ui::cards::card(
            format!("spotify:artist:{id}"),
            kind,
            id,
            "",
            None,
        ))
    }

    fn suggestions(prefix: &str) -> (String, Suggestions) {
        let items = vec![
            Suggestion::Query("radiohead".into()),
            Suggestion::Entity(Box::new(hit("a", Kind::Artist))),
            Suggestion::Query("radiohead creep".into()),
            Suggestion::Query("radiohead live".into()),
            Suggestion::Query("radiohead ok computer".into()),
            Suggestion::Query("radiohead in rainbows".into()),
        ]
        .into_iter()
        .chain(
            (0..8)
                .map(|index| Suggestion::Entity(Box::new(hit(&format!("e{index}"), Kind::Album)))),
        )
        .collect();
        (
            prefix.to_owned(),
            Suggestions {
                query: prefix.to_owned(),
                items,
            },
        )
    }

    #[test]
    fn lists_text_suggestions_before_entities() {
        let suggest = Suggest {
            text: "Radioh".into(),
            current: Some(suggestions("radio")),
            ..Suggest::default()
        };
        let rows = drop_rows(&suggest, &[]);
        assert_eq!(rows.len(), QUERY_ROWS + ENTITY_ROWS);
        assert!(
            rows[..QUERY_ROWS]
                .iter()
                .all(|row| matches!(row, DropRow::Query(_)))
        );
        assert_eq!(rows[0], DropRow::Query("radiohead".into()));
        assert!(matches!(&rows[QUERY_ROWS], DropRow::Entity(hit) if hit.card.title == "a"));
    }

    #[test]
    fn hides_suggestions_for_another_prefix_and_for_links() {
        let mut suggest = Suggest {
            text: "beatles".into(),
            current: Some(suggestions("radio")),
            ..Suggest::default()
        };
        assert!(drop_rows(&suggest, &[]).is_empty());
        suggest.text = "https://open.spotify.com/album/6dVIqQ8qmQ5GBnJ9shOYGE".into();
        suggest.current = Some(suggestions("https"));
        assert!(drop_rows(&suggest, &[]).is_empty());
    }

    #[test]
    fn shows_recent_searches_for_an_empty_field() {
        let recent: Vec<Hit> = (0..12)
            .map(|index| hit(&index.to_string(), Kind::Artist))
            .collect();
        let rows = drop_rows(&Suggest::default(), &recent);
        assert_eq!(rows.len(), RECENT_ROWS);
        assert_eq!(rows[0], DropRow::Recent(Box::new(recent[0].clone())));
    }

    #[test]
    fn moves_the_selection_with_arrow_keys() {
        assert_eq!(step(None, 3, true), Some(0));
        assert_eq!(step(Some(0), 3, true), Some(1));
        assert_eq!(step(Some(2), 3, true), Some(2));
        assert_eq!(step(Some(1), 3, false), Some(0));
        assert_eq!(step(Some(0), 3, false), None);
        assert_eq!(step(None, 3, false), Some(2));
        assert_eq!(step(Some(5), 3, false), Some(2));
        assert_eq!(step(None, 0, true), None);
    }

    #[test]
    fn remembering_moves_an_item_to_the_top() {
        let mut recent = vec![
            hit("a", Kind::Artist),
            hit("b", Kind::Artist),
            hit("c", Kind::Artist),
        ];
        remembered(&mut recent, hit("c", Kind::Artist));
        let ids: Vec<&str> = recent.iter().map(|hit| hit.card.title.as_str()).collect();
        assert_eq!(ids, ["c", "a", "b"]);
        remembered(&mut recent, hit("d", Kind::Artist));
        assert_eq!(recent.len(), 4);
        assert_eq!(recent[0].card.title, "d");
    }

    #[test]
    fn clearing_sends_every_recent_uri() {
        let mut state = super::super::state::State::default();
        state.recent.items = vec![hit("a", Kind::Artist), hit("b", Kind::Album)];
        assert_eq!(
            state.recent_uris(),
            ["spotify:artist:a".to_owned(), "spotify:artist:b".to_owned()]
        );
    }

    #[test]
    fn maps_tabs_to_kinds_and_back() {
        for tab in SearchTab::TABS {
            match tab.kind() {
                Some(kind) => assert_eq!(SearchTab::of(kind), tab),
                None => assert_eq!(tab, SearchTab::All),
            }
        }
        assert_eq!(SearchTab::from_alias("songs"), Some(SearchTab::Tracks));
        assert_eq!(SearchTab::from_alias("profiles"), Some(SearchTab::Users));
        assert_eq!(SearchTab::from_alias("albums"), Some(SearchTab::Albums));
        assert_eq!(SearchTab::Podcasts.shelf_title(), "Shows");
        assert_eq!(SearchTab::Users.label(), "Profiles");
    }

    #[test]
    fn routes_pasted_links_to_their_pages() {
        let cases = [
            (
                "https://open.spotify.com/album/6dVIqQ8qmQ5GBnJ9shOYGE?si=x",
                Some(Page::Collection(
                    "spotify:album:6dVIqQ8qmQ5GBnJ9shOYGE".into(),
                )),
            ),
            (
                "https://open.spotify.com/intl-de/artist/4Z8W4fKeB5YxbusRsdQVPb",
                Some(Page::Artist("spotify:artist:4Z8W4fKeB5YxbusRsdQVPb".into())),
            ),
            (
                "https://open.spotify.com/genre/0JQ5DAqbMKFDXXwE9BDJAr",
                Some(Page::Browse("spotify:page:0JQ5DAqbMKFDXXwE9BDJAr".into())),
            ),
            (
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
                Some(Page::Collection(
                    "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M".into(),
                )),
            ),
            (
                "https://open.spotify.com/show/5CfCWKI5pZ28U0uOzXkDHe",
                Some(Page::Show("spotify:show:5CfCWKI5pZ28U0uOzXkDHe".into())),
            ),
            (
                "https://open.spotify.com/user/someone",
                Some(Page::Profile("spotify:user:someone".into())),
            ),
            (
                "https://open.spotify.com/track/70LcF31zb1H0PyJoS1Sx1r",
                None,
            ),
            ("https://open.spotify.com/album/short", None),
            ("radiohead", None),
        ];
        for (text, page) in cases {
            assert_eq!(link_page(text), page, "{text}");
        }
    }

    #[test]
    fn top_results_play_music_only() {
        let mut card = crate::ui::cards::card("spotify:album:x", Kind::Album, "x", "", None);
        assert!(top_result_plays(&card));
        card.playable = false;
        assert!(!top_result_plays(&card));
        let show = crate::ui::cards::card("spotify:show:x", Kind::Show, "x", "", None);
        assert!(!top_result_plays(&show));
    }
}
