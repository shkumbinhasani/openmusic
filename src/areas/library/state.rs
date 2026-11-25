use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::Path,
    time::{Duration, Instant},
};

use gpui::{
    App, AppContext, Context, Entity, FocusHandle, KeyBinding, ScrollHandle, Window, actions,
    point, px,
};
use gpui_component::input::{InputEvent, InputState};
use serde::{Deserialize, Serialize};
use spotify_client::{
    catalog::Playlist,
    entity,
    library::{ItemKind, LibraryItem, LibraryPage, LibraryQuery, PinResult, Recent, Rootlist},
};

use super::{
    cache,
    liked::{Column, FIND_KEY, Liked, LikedSort, PLAY_KEY},
    saves::{self, FOLLOW_KEY, LIKE_CURRENT_KEY, SAVE_KEY, header_uri},
    view::LIBRARY_ROW,
    worker::{Command, Event},
};
use crate::{
    app::{OpenMusic, Page},
    areas::discovery::state::SearchTab,
    menus::{self, Handle},
    ui::{
        components::Sentinel,
        menu::Target,
        tracks::{RowDrag, RowDrop},
    },
};

actions!(library, [FilterLibrary]);

pub const LIBRARY_CONTEXT: &str = "YourLibrary";
const FILTER_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryPrefs {}

#[derive(Default)]
pub struct State {
    pub playlists: Vec<Playlist>,
    pub rootlist: Option<Rootlist>,
    pub playlist_rows: HashMap<String, usize>,
    pub query: LibraryQuery,
    pub page: Option<LibraryPage>,
    pub items: Vec<LibraryItem>,
    pub loading: bool,
    pub error: Option<String>,
    pub request: u64,
    pub next_offset: u32,
    pub sentinel: Sentinel,
    pub scroll: ScrollHandle,
    pub filter: Option<Entity<InputState>>,
    pub filter_open: bool,
    pub filter_edited: Option<Instant>,
    pub focus: Option<FocusHandle>,
    pub folder: Option<Folder>,
    pub folder_requests: u64,
    pub pins: HashMap<String, (bool, bool)>,
    pub recents: Vec<Recent>,
    pub wanted: RefCell<HashSet<String>>,
    pub asked: HashMap<String, Instant>,
    pub toggle_when_known: Option<(String, Instant)>,
    pub resync: bool,
    pub liked: Liked,
}

#[derive(Default)]
pub struct Folder {
    pub uri: String,
    pub page: Option<LibraryPage>,
    pub items: Vec<LibraryItem>,
    pub loading: bool,
    pub error: Option<String>,
    pub request: u64,
    pub next_offset: u32,
    pub sentinel: Sentinel,
}

impl Folder {
    pub fn has_more(&self) -> bool {
        self.page
            .as_ref()
            .is_some_and(|page| !page.fallback && self.next_offset < page.total)
    }
}

fn same_item(item: &LibraryItem, uri: &str) -> bool {
    item.uri == uri || (item.kind == ItemKind::LikedSongs && entity::is_liked_songs(uri))
}

fn merge(items: &mut Vec<LibraryItem>, page: &mut LibraryPage, offset: u32) -> u32 {
    let fresh = std::mem::take(&mut page.items);
    let count = fresh.len() as u32;
    if offset == 0 || page.fallback {
        *items = fresh;
        return count;
    }
    if count == 0 {
        page.total = offset;
    }
    let known: HashSet<(String, u32)> = items
        .iter()
        .map(|item| (item.uri.clone(), item.depth))
        .collect();
    items.extend(
        fresh
            .into_iter()
            .filter(|item| !known.contains(&(item.uri.clone(), item.depth))),
    );
    offset + count
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        self.liked.covers(urls);
        let folder = self.folder.iter().flat_map(|folder| &folder.items);
        urls.extend(
            self.items
                .iter()
                .chain(folder)
                .filter_map(|item| item.image.clone()),
        );
    }

    pub fn has_more(&self) -> bool {
        self.page
            .as_ref()
            .is_some_and(|page| !page.fallback && self.next_offset < page.total)
    }

    pub fn show_kind(&self) -> bool {
        self.query.filters.is_empty()
    }

    pub fn item(&self, uri: &str) -> Option<&LibraryItem> {
        let folder = self.folder.iter().flat_map(|folder| &folder.items);
        self.items
            .iter()
            .chain(folder)
            .find(|item| same_item(item, uri))
    }

    pub(super) fn items_mut(&mut self, uri: &str) -> impl Iterator<Item = &mut LibraryItem> {
        let folder = self.folder.iter_mut().flat_map(|folder| &mut folder.items);
        self.items
            .iter_mut()
            .chain(folder)
            .filter(move |item| same_item(item, uri))
    }

    pub(super) fn forget_items(&mut self, uris: &[&String]) {
        let before = self.items.len();
        self.items.retain(|item| !uris.contains(&&item.uri));
        let removed = (before - self.items.len()) as u32;
        if removed > 0 {
            self.next_offset = self.next_offset.saturating_sub(removed);
            if let Some(page) = &mut self.page {
                page.total = page.total.saturating_sub(removed);
            }
        }
    }

    fn set_playlists(&mut self, playlists: Vec<Playlist>) {
        self.playlist_rows = playlists
            .iter()
            .enumerate()
            .map(|(index, playlist)| (playlist.uri.clone(), index))
            .collect();
        self.playlists = playlists;
    }

    pub fn folder_name(&self, uri: &str) -> String {
        let crumb = self
            .folder
            .as_ref()
            .filter(|folder| folder.uri == uri)
            .and_then(|folder| folder.page.as_ref())
            .and_then(|page| page.breadcrumbs.iter().find(|crumb| crumb.id == uri))
            .map(|crumb| crumb.name.clone());
        crumb
            .or_else(|| self.item(uri).map(LibraryItem::title))
            .or_else(|| {
                self.rootlist
                    .as_ref()
                    .and_then(|list| list.folder(uri))
                    .map(|folder| folder.name)
            })
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Folder".to_owned())
    }
}

pub fn filter_placeholder(filter: Option<&str>) -> &'static str {
    match filter {
        Some("Playlists") => "Search in Playlists",
        Some("Albums") => "Search in Albums",
        Some("Artists") => "Search in Artists",
        Some("Authors") => "Search in Authors",
        Some("Podcasts & Shows") => "Search in Podcasts & Shows",
        Some("Audiobooks") => "Search in Audiobooks",
        Some("Events") => "Search in Events",
        _ => "Search in Your Library",
    }
}

pub fn pin_message(result: &Result<PinResult, String>, pinned: bool) -> Option<String> {
    Some(match result {
        Ok(PinResult::Done) => return None,
        Ok(PinResult::LimitReached(limit)) => {
            format!("You’re out of pins. You can have {limit} items pinned.")
        }
        Ok(PinResult::InFolder) => {
            "You can’t pin items that are inside a folder. Try to pin the folder instead.".into()
        }
        Ok(PinResult::NotSupported) => "This can’t be pinned.".into(),
        Ok(PinResult::NotInLibrary) => "Add this to Your Library before you pin it.".into(),
        Ok(PinResult::Refused) | Err(_) if pinned => "Couldn’t pin this. Try again.".into(),
        Ok(PinResult::Refused) | Err(_) => "Couldn’t unpin this. Try again.".into(),
    })
}

impl OpenMusic {
    pub fn recent_contexts(&self) -> &[Recent] {
        &self.library.recents
    }

    pub fn library_item(&self, uri: &str) -> Option<&LibraryItem> {
        self.library.item(uri)
    }

    pub fn reload_library(&mut self) {
        self.library.request += 1;
        self.library.loading = true;
        self.send(Command::Library {
            query: self.library.query.clone(),
            offset: 0,
            request: self.library.request,
        });
    }

    pub fn refresh_library(&mut self) {
        self.send(Command::Playlists);
        self.reload_library_views();
    }

    pub fn reload_library_views(&mut self) {
        self.reload_library();
        if let Some(uri) = self
            .library
            .folder
            .as_ref()
            .map(|folder| folder.uri.clone())
        {
            self.load_folder(uri);
        }
    }

    fn more_library(&mut self) {
        if self.library.loading || !self.library.has_more() {
            return;
        }
        self.library.request += 1;
        self.library.loading = true;
        self.send(Command::Library {
            query: self.library.query.clone(),
            offset: self.library.next_offset,
            request: self.library.request,
        });
    }

    pub fn set_library_query(&mut self, query: LibraryQuery) {
        if query == self.library.query {
            return;
        }
        let jump = query.filters != self.library.query.filters
            || query.sort != self.library.query.sort
            || query.text != self.library.query.text;
        self.library.query = query;
        if jump {
            self.library.scroll.set_offset(point(px(0.), px(0.)));
        }
        self.reload_library();
    }

    pub fn pick_library_filter(&mut self, id: String) {
        let mut query = self.library.query.clone();
        query.filters.push(id);
        query.folder = None;
        self.set_library_query(query);
    }

    pub fn keep_library_filters(&mut self, count: usize) {
        let selected: Vec<String> = match &self.library.page {
            Some(page) if !page.selected.is_empty() => {
                page.selected.iter().map(|chip| chip.id.clone()).collect()
            }
            _ => self.library.query.filters.clone(),
        };
        let mut query = self.library.query.clone();
        query.filters = selected.into_iter().take(count).collect();
        self.set_library_query(query);
    }

    pub fn set_library_sort(&mut self, sort: String) {
        let mut query = self.library.query.clone();
        query.sort = Some(sort);
        self.set_library_query(query);
    }

    pub fn set_library_text(&mut self, text: Option<String>) {
        let mut query = self.library.query.clone();
        query.text = text;
        self.set_library_query(query);
    }

    pub fn toggle_library_folder(&mut self, uri: &str) {
        let mut query = self.library.query.clone();
        if let Some(position) = query.expanded.iter().position(|open| open == uri) {
            query.expanded.remove(position);
        } else {
            query.expanded.push(uri.to_owned());
        }
        self.set_library_query(query);
    }

    pub fn open_library_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.library.filter.clone() else {
            return;
        };
        self.library.filter_open = true;
        let placeholder =
            filter_placeholder(self.library.query.filters.first().map(String::as_str));
        let clear = self.library.query.text.is_none();
        input.update(cx, |input, cx| {
            if clear {
                input.set_value("", window, cx);
            }
            input.set_placeholder(placeholder, window, cx);
            input.focus(window, cx);
        });
    }

    pub fn close_library_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.library.filter_open = false;
        self.library.filter_edited = None;
        if let Some(input) = self.library.filter.clone() {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.set_library_text(None);
    }

    fn apply_library_filter(&mut self, cx: &mut Context<Self>) {
        self.library.filter_edited = None;
        let Some(input) = &self.library.filter else {
            return;
        };
        let text = input.read(cx).value().trim().to_owned();
        self.set_library_text((!text.is_empty()).then_some(text));
    }

    pub fn set_pinned(&mut self, uri: &str, pinned: bool) {
        if self.library.pins.contains_key(uri) {
            return;
        }
        let before = self.library.item(uri).map_or(!pinned, |item| item.pinned);
        self.library.pins.insert(uri.to_owned(), (before, pinned));
        for item in self.library.items_mut(uri) {
            item.pinned = pinned;
        }
        self.send(Command::Pin {
            uri: uri.to_owned(),
            pinned,
        });
    }

    fn pinned(&mut self, uri: String, pinned: bool, result: Result<PinResult, String>) {
        let before = self
            .library
            .pins
            .remove(&uri)
            .map_or(!pinned, |(before, _)| before);
        match pin_message(&result, pinned) {
            None => self.reload_library_views(),
            Some(message) => {
                for item in self.library.items_mut(&uri) {
                    item.pinned = before;
                }
                self.show_toast(message, None);
            }
        }
    }

    fn load_folder(&mut self, uri: String) {
        self.library.folder_requests += 1;
        let request = self.library.folder_requests;
        let keep = self
            .library
            .folder
            .take()
            .filter(|folder| folder.uri == uri && folder.error.is_none());
        self.library.folder = Some(Folder {
            uri: uri.clone(),
            loading: true,
            request,
            ..keep.unwrap_or_default()
        });
        self.send(Command::Folder {
            uri,
            sort: self.library.query.sort.clone(),
            offset: 0,
            request,
        });
    }

    fn more_folder(&mut self) {
        let sort = self.library.query.sort.clone();
        let Some(folder) = self
            .library
            .folder
            .as_mut()
            .filter(|folder| !folder.loading && folder.has_more())
        else {
            return;
        };
        self.library.folder_requests += 1;
        let request = self.library.folder_requests;
        folder.loading = true;
        folder.request = request;
        let command = Command::Folder {
            uri: folder.uri.clone(),
            sort,
            offset: folder.next_offset,
            request,
        };
        self.send(command);
    }

    fn restore_library(&mut self) {
        let Some(cached) = cache::load() else { return };
        let mut page = cached.page;
        self.library.query = cached.query;
        self.library.next_offset = merge(&mut self.library.items, &mut page, 0);
        self.library.page = Some(page);
    }
}

pub fn load(app: &mut OpenMusic, page: &Page) {
    if *page == Page::Liked && app.library.liked.wants_reload() && !app.library.liked.loading {
        app.load_liked();
    }
    if let Page::Folder(uri) = page {
        let loaded = app
            .library
            .folder
            .as_ref()
            .is_some_and(|folder| folder.uri == *uri && folder.error.is_none());
        if !loaded {
            app.load_folder(uri.clone());
        }
    }
}

fn on_filter_input(
    app: &mut OpenMusic,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    match event {
        InputEvent::Change => app.library.filter_edited = Some(Instant::now()),
        InputEvent::PressEnter { .. } => app.apply_library_filter(cx),
        InputEvent::Blur => {
            let empty = app
                .library
                .filter
                .as_ref()
                .is_some_and(|input| input.read(cx).value().trim().is_empty());
            if empty && app.library.query.text.is_none() {
                app.library.filter_open = false;
            }
            if window.focused(cx).is_none() {
                window.focus(&app.focus);
            }
        }
        InputEvent::Focus => {}
    }
    cx.notify();
}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::Playlists(playlists) => {
            if app.library.rootlist.is_none() {
                app.library.set_playlists(playlists);
            }
        }
        Event::Rootlist(Ok(list)) => {
            app.library.set_playlists(list.playlists());
            app.library.rootlist = Some(*list);
        }
        Event::Rootlist(Err(_)) => {}
        Event::Library {
            request,
            query,
            offset,
            result,
        } => {
            if request != app.library.request {
                return;
            }
            app.library.loading = false;
            match result {
                Ok(mut page) => {
                    app.library.next_offset = merge(&mut app.library.items, &mut page, offset);
                    app.library.query = query;
                    app.library.page = Some(page);
                    app.library.error = None;
                    let pins: Vec<(String, bool)> = app
                        .library
                        .pins
                        .iter()
                        .map(|(uri, &(_, wanted))| (uri.clone(), wanted))
                        .collect();
                    for (uri, wanted) in pins {
                        for item in app.library.items_mut(&uri) {
                            item.pinned = wanted;
                        }
                    }
                }
                Err(message) => app.library.error = Some(message),
            }
        }
        Event::Folder {
            request,
            uri,
            offset,
            result,
        } => {
            let Some(folder) = app
                .library
                .folder
                .as_mut()
                .filter(|folder| folder.uri == uri && folder.request == request)
            else {
                return;
            };
            folder.loading = false;
            match result {
                Ok(mut page) => {
                    folder.next_offset = merge(&mut folder.items, &mut page, offset);
                    folder.page = Some(page);
                    folder.error = None;
                }
                Err(message) => folder.error = Some(message),
            }
        }
        Event::Pinned {
            uri,
            pinned,
            result,
        } => app.pinned(uri, pinned, result),
        Event::Liked {
            request,
            offset,
            result,
        } => app.on_liked(request, offset, result),
    }
}

pub fn connected(app: &mut OpenMusic, window: &mut Window, cx: &mut Context<OpenMusic>) {
    if app.library.filter.is_none() {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(filter_placeholder(None))
                .clean_on_escape()
        });
        cx.subscribe_in(&input, window, |app, _, event, window, cx| {
            on_filter_input(app, event, window, cx)
        })
        .detach();
        app.library.filter = Some(input);
    }
    if app.library.focus.is_none() {
        app.library.focus = Some(cx.focus_handle());
    }
    if app.library.page.is_none() {
        app.restore_library();
    }
    app.send(Command::Playlists);
    app.reload_library();
}

pub fn signed_out(app: &mut OpenMusic) {
    let filter = app.library.filter.take();
    let focus = app.library.focus.take();
    app.library = State {
        filter,
        focus,
        ..State::default()
    };
    if app.session_error.is_none() {
        cache::clear();
    }
}

fn dump(app: &OpenMusic, directory: &Path) {
    let library = &app.library;
    let query = &library.query;
    let mut lines = vec![format!(
        "query filters={:?} sort={:?} text={:?} folder={:?} expanded={}",
        query.filters.join("/"),
        query.sort,
        query.text,
        query.folder,
        query.expanded.len()
    )];
    if let Some(page) = &library.page {
        let names = |chips: &[spotify_client::library::Chip]| {
            chips
                .iter()
                .map(|chip| chip.id.as_str())
                .collect::<Vec<_>>()
                .join(",")
        };
        lines.push(format!(
            "page total={} loaded={} next={} fallback={} loading={} error={:?}",
            page.total,
            library.items.len(),
            library.next_offset,
            page.fallback,
            library.loading,
            library.error
        ));
        lines.push(format!("filters {}", names(&page.filters)));
        lines.push(format!("selected {}", names(&page.selected)));
        lines.push(format!("sorts {}", names(&page.sorts)));
        lines.push(format!(
            "sort {}",
            page.sort.as_ref().map_or("", |sort| sort.id.as_str())
        ));
    } else {
        lines.push(format!(
            "page none loading={} error={:?}",
            library.loading, library.error
        ));
    }
    let show_kind = library.show_kind();
    for item in &library.items {
        lines.push(format!(
            "{}{} {:?} pinned={} pinnable={} | {} | {}",
            "  ".repeat(item.depth as usize),
            item.uri,
            item.kind,
            item.pinned,
            item.pinnable,
            item.title(),
            item.subtitle(show_kind)
        ));
    }
    if let Some(folder) = &library.folder {
        lines.push(format!(
            "folder {} loaded={} total={} loading={} error={:?}",
            folder.uri,
            folder.items.len(),
            folder.page.as_ref().map_or(0, |page| page.total),
            folder.loading,
            folder.error
        ));
        for item in &folder.items {
            lines.push(format!("  {} {:?} | {}", item.uri, item.kind, item.title()));
        }
    }
    let _ = std::fs::write(directory.join("library.txt"), lines.join("\n"));
}

fn dump_liked(app: &OpenMusic, directory: &Path) {
    let liked = &app.library.liked;
    let mut lines = vec![format!(
        "total={} loaded={} shown={} next={} loading={} stale={} error={:?} sort={} {} unplayable={}",
        liked.total,
        liked.tracks.len(),
        liked.shown.len(),
        liked.next_offset,
        liked.loading,
        liked.stale,
        liked.error,
        liked.sort.column.slug(),
        if liked.sort.descending { "desc" } else { "asc" },
        liked.unplayable.len()
    )];
    lines.extend(liked.shown.iter().map(|track| {
        format!(
            "{} | {} | {} | {} | added={:?} saved={:?}",
            track.uri,
            track.title,
            track.artist,
            track.album,
            track.added_at,
            app.is_saved(&track.uri)
        )
    }));
    let _ = std::fs::write(directory.join("liked.txt"), lines.join("\n"));
}

fn dump_rootlist(app: &OpenMusic, directory: &Path) {
    use spotify_client::library::RootNode;
    fn outline(nodes: &[RootNode], depth: usize, lines: &mut Vec<String>) {
        let indent = "  ".repeat(depth);
        for node in nodes {
            match node {
                RootNode::Playlist {
                    playlist, index, ..
                } => lines.push(format!(
                    "{indent}{index} {} {}",
                    playlist.uri, playlist.name
                )),
                RootNode::Unavailable { uri, index } => {
                    lines.push(format!("{indent}{index} {uri} unavailable"))
                }
                RootNode::Folder(folder) => {
                    lines.push(format!(
                        "{indent}{} folder {} {} raw={}",
                        folder.start, folder.uri, folder.name, folder.raw_count
                    ));
                    outline(&folder.children, depth + 1, lines);
                }
            }
        }
    }
    let mut lines = Vec::new();
    match &app.library.rootlist {
        Some(list) => {
            lines.push(format!(
                "revision={:?} items={} playlists={}",
                list.revision,
                list.items.len(),
                list.playlists().len()
            ));
            outline(&list.tree(), 0, &mut lines);
        }
        None => lines.push(format!(
            "rootlist none playlists={}",
            app.library.playlists.len()
        )),
    }
    let _ = std::fs::write(directory.join("rootlist.txt"), lines.join("\n"));
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
        "library" => {
            let (action, argument) = rest.split_once(' ').unwrap_or((rest, ""));
            let argument = argument.trim();
            let text = (!argument.is_empty()).then(|| argument.to_owned());
            let mut query = app.library.query.clone();
            match action {
                "" | "dump" => {}
                "reload" => app.refresh_library(),
                "reset" => app.set_library_query(LibraryQuery::default()),
                "filter" => {
                    query.filters = argument
                        .split('/')
                        .map(str::trim)
                        .filter(|part| !part.is_empty())
                        .map(str::to_owned)
                        .collect();
                    app.set_library_query(query);
                }
                "sort" => {
                    query.sort = text;
                    app.set_library_query(query);
                }
                "text" => match text {
                    Some(text) => {
                        app.library.filter_open = true;
                        if let Some(input) = app.library.filter.clone() {
                            let value = text.clone();
                            input.update(cx, |input, cx| input.set_value(value, window, cx));
                        }
                        app.set_library_text(Some(text));
                    }
                    None => app.close_library_filter(window, cx),
                },
                "more" => app.more_library(),
                "expand" | "collapse" => {
                    let open = query.expanded.iter().any(|uri| uri == argument);
                    if open == (action == "collapse") && !argument.is_empty() {
                        app.toggle_library_folder(argument);
                    }
                }
                "folder" => app.navigate(Page::Folder(argument.to_owned())),
                _ => return false,
            }
            dump(app, directory);
        }
        "pin" | "unpin" => {
            for uri in rest.split_whitespace() {
                app.set_pinned(uri, verb == "pin");
            }
        }
        "rootlist" => dump_rootlist(app, directory),
        "togglesaved" => {
            for uri in rest.split_whitespace() {
                app.toggle_saved(uri);
            }
        }
        "likecurrent" => app.like_current(),
        "liked" => {
            let mut words = rest.split_whitespace();
            match (words.next(), words.next(), words.next()) {
                (Some("reload"), _, _) => app.load_liked(),
                (Some("more"), _, _) => app.more_liked(),
                (Some("sort"), None, _) => app.set_liked_sort(LikedSort::default()),
                (Some("sort"), Some(slug), order) => {
                    if let Some(column) = Column::from_slug(slug) {
                        app.set_liked_sort(LikedSort {
                            column,
                            descending: order
                                .map_or(column == Column::Added, |order| order == "desc"),
                        });
                    }
                }
                _ => {}
            }
            dump_liked(app, directory);
        }
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    let library = &app.library;
    format!(
        "library={}/{} filters={:?} sort={:?} text={:?} fallback={} library_loading={} rootlist={} folders_open={} pins_pending={} recents={} saved_asked={} resync={} liked={}/{} liked_sort={}",
        library.items.len(),
        library.page.as_ref().map_or(0, |page| page.total),
        library.query.filters.join("/"),
        library
            .page
            .as_ref()
            .and_then(|page| page.sort.as_ref())
            .map(|sort| sort.id.clone()),
        library.query.text,
        library.page.as_ref().is_some_and(|page| page.fallback),
        library.loading,
        library
            .rootlist
            .as_ref()
            .map_or(0, |list| list.playlists().len()),
        library.query.expanded.len(),
        library.pins.len(),
        app.recent_contexts().len(),
        library.asked.len(),
        library.resync,
        library.liked.tracks.len(),
        library.liked.total,
        library.liked.sort.column.slug()
    )
}

pub fn register_actions(cx: &mut App, handle: &Handle) {
    cx.bind_keys([KeyBinding::new(
        "cmd-f",
        FilterLibrary,
        Some(LIBRARY_CONTEXT),
    )]);
    menus::route(cx, handle, |_: &FilterLibrary, app, window, cx| {
        app.open_library_filter(window, cx);
        cx.notify();
    });
    menus::route(cx, handle, |_: &menus::ToggleLike, app, _, cx| {
        app.like_current();
        cx.notify();
    });
    menus::route(cx, handle, |_: &menus::GoToLikedSongs, app, _, cx| {
        app.navigate(Page::Liked);
        cx.notify();
    });
}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        "more-folder" => {
            if let Page::Folder(uri) = &app.page {
                let target = Target::Folder { uri: uri.clone() };
                app.open_menu_under(target, key);
            }
        }
        LIKE_CURRENT_KEY => app.like_current(),
        PLAY_KEY => app.play_all_liked(cx),
        FIND_KEY => {
            app.navigate(Page::Search(SearchTab::All));
            app.focus_search(window, cx);
        }
        SAVE_KEY | FOLLOW_KEY => {
            if let Some(uri) = header_uri(&app.page).map(str::to_owned) {
                app.toggle_saved(&uri);
            }
        }
        _ => return false,
    }
    true
}

pub fn row_dropped(
    app: &mut OpenMusic,
    drag: &RowDrag,
    at: RowDrop,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    if at.row.0 != LIBRARY_ROW {
        return false;
    }
    let liked = app
        .library
        .items
        .get(at.row.1)
        .is_some_and(|item| item.kind == ItemKind::LikedSongs);
    if !liked {
        return false;
    }
    let songs: Vec<String> = drag
        .uris
        .iter()
        .filter(|uri| uri.starts_with("spotify:track:"))
        .cloned()
        .collect();
    if songs.is_empty() {
        return true;
    }
    let message = match app.write_saved(songs, true) {
        0 => "Already in Liked Songs.".to_owned(),
        1 => "Added to Liked Songs.".to_owned(),
        count => format!("Added {count} songs to Liked Songs."),
    };
    app.show_toast(message, None);
    true
}

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    if saves::tick(app) | app.tick_liked() {
        cx.notify();
    }
    if app.library.sentinel.take() {
        app.more_library();
    }
    if app
        .library
        .folder
        .as_ref()
        .is_some_and(|folder| folder.sentinel.take())
    {
        app.more_folder();
    }
    if app
        .library
        .filter_edited
        .is_some_and(|edited| edited.elapsed() >= FILTER_DELAY)
    {
        app.apply_library_filter(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use spotify_client::library::Chip;

    use super::*;

    fn item(uri: &str, depth: u32) -> LibraryItem {
        let mut item = LibraryItem::new(uri, ItemKind::Playlist, uri);
        item.depth = depth;
        item
    }

    fn page(items: Vec<LibraryItem>, total: u32) -> LibraryPage {
        LibraryPage {
            items,
            total,
            ..LibraryPage::default()
        }
    }

    #[test]
    fn pages_append_without_repeating_rows() {
        let mut items = Vec::new();
        let mut first = page(vec![item("a", 0), item("b", 0)], 4);
        assert_eq!(merge(&mut items, &mut first, 0), 2);
        let mut second = page(vec![item("b", 0), item("b", 1), item("c", 0)], 4);
        assert_eq!(merge(&mut items, &mut second, 2), 5);
        let uris: Vec<(String, u32)> = items
            .iter()
            .map(|item| (item.uri.clone(), item.depth))
            .collect();
        assert_eq!(
            uris,
            vec![
                ("a".into(), 0),
                ("b".into(), 0),
                ("b".into(), 1),
                ("c".into(), 0)
            ]
        );
        let mut empty = page(Vec::new(), 40);
        assert_eq!(merge(&mut items, &mut empty, 5), 5);
        assert_eq!(empty.total, 5);
        let mut fresh = page(vec![item("z", 0)], 1);
        assert_eq!(merge(&mut items, &mut fresh, 0), 1);
        assert_eq!(items.len(), 1);
        let mut fallback = page(vec![item("x", 0), item("y", 0)], 2);
        fallback.fallback = true;
        assert_eq!(merge(&mut items, &mut fallback, 50), 2);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn drops_items_removed_from_the_library() {
        let mut state = State {
            items: vec![
                item("spotify:album:a", 0),
                item("spotify:artist:b", 0),
                item("spotify:playlist:c", 0),
            ],
            page: Some(page(Vec::new(), 40)),
            next_offset: 3,
            ..State::default()
        };
        let gone = ["spotify:album:a".to_owned(), "spotify:show:z".to_owned()];
        state.forget_items(&gone.iter().collect::<Vec<_>>());
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.next_offset, 2);
        assert_eq!(state.page.as_ref().map(|page| page.total), Some(39));
    }

    #[test]
    fn explains_why_a_pin_failed() {
        assert_eq!(pin_message(&Ok(PinResult::Done), true), None);
        assert_eq!(
            pin_message(&Ok(PinResult::LimitReached(4)), true).as_deref(),
            Some("You’re out of pins. You can have 4 items pinned.")
        );
        assert!(
            pin_message(&Ok(PinResult::InFolder), true)
                .unwrap()
                .contains("inside a folder")
        );
        assert_eq!(
            pin_message(&Err("offline".into()), false).as_deref(),
            Some("Couldn’t unpin this. Try again.")
        );
        assert_eq!(
            pin_message(&Ok(PinResult::Refused), true).as_deref(),
            Some("Couldn’t pin this. Try again.")
        );
    }

    #[test]
    fn finds_liked_songs_by_any_of_its_uris() {
        let mut state = State {
            items: vec![
                LibraryItem::new(
                    "spotify:collection:tracks",
                    ItemKind::LikedSongs,
                    "Liked Songs",
                ),
                item("spotify:playlist:x", 0),
            ],
            ..State::default()
        };
        assert!(state.item("spotify:user:someone:collection").is_some());
        assert!(state.item("spotify:playlist:x").is_some());
        assert!(state.item("spotify:playlist:y").is_none());
        state.folder = Some(Folder {
            uri: "spotify:user:someone:folder:f".into(),
            items: vec![item("spotify:playlist:y", 0)],
            page: Some(LibraryPage {
                breadcrumbs: vec![Chip::new("spotify:user:someone:folder:f", "Road trip")],
                ..LibraryPage::default()
            }),
            ..Folder::default()
        });
        assert!(state.item("spotify:playlist:y").is_some());
        assert_eq!(
            state.folder_name("spotify:user:someone:folder:f"),
            "Road trip"
        );
        assert_eq!(state.folder_name("spotify:user:someone:folder:g"), "Folder");
        assert_eq!(filter_placeholder(Some("Playlists")), "Search in Playlists");
        assert_eq!(filter_placeholder(None), "Search in Your Library");
    }
}
