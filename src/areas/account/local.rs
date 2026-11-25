use std::{
    cmp::Ordering,
    collections::HashSet,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use gpui::{AppContext, Context, Entity, PathPromptOptions, Window};
use gpui_component::input::{InputEvent, InputState};
use spotify_client::{Track, engine::NowPlaying};

use super::{
    local_match::{self, LocalIndex, MatchedRows},
    local_scan::{
        LocalFile, LocalSources, MAX_FILES, ScanReport, ScanUpdate, home, index_path, is_local,
    },
    worker::Command,
};
use crate::{
    app::{OpenMusic, Page, Session},
    icons,
    ui::{
        components::Sentinel,
        menu::{Group, MenuItem, Place, Row, Target},
        sidebar::NavEntry,
    },
};

pub const NAV_KEY: &str = "nav-local-files";
pub const NAV_WEIGHT: u32 = 700;
pub const ROW: &str = "localfiles-track";
pub const PLAY_KEY: &str = "localfiles-play";
pub const MORE_KEY: &str = "more-localfiles";
pub const SORT_KEY: &str = "localfiles-sort";
pub const ADD_KEY: &str = "localfiles-add";
pub const RESCAN_KEY: &str = "localfiles-rescan";
pub const ENABLE_KEY: &str = "localfiles-enable";
pub const PRIVACY_KEY: &str = "localfiles-privacy";
pub const PRIVACY_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_FilesAndFolders";
pub const PAGE_STEP: usize = 200;
pub const STALE: Duration = Duration::from_secs(60);
pub const MISSING_FILE: &str =
    "This file isn’t available. Connect the drive it’s on, then try again.";
pub const NOTHING_TO_PLAY: &str = "None of these files are available right now.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Added,
    Title,
    Artist,
    Album,
    Duration,
}

impl Column {
    pub const ALL: [Self; 5] = [
        Self::Added,
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::Duration,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Added => "Date added",
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::Duration => "Duration",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Duration => "duration",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|column| column.name() == name)
    }

    fn newest_first(self) -> bool {
        self == Self::Added
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub column: Column,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Self {
            column: Column::Added,
            descending: true,
        }
    }
}

impl Sort {
    pub fn pick(self, column: Column) -> Self {
        if column == self.column {
            Self {
                column,
                descending: !self.descending,
            }
        } else {
            Self {
                column,
                descending: column.newest_first(),
            }
        }
    }
}

struct Folded {
    title: String,
    artist: String,
    album: String,
    name: String,
}

impl Folded {
    fn of(file: &LocalFile) -> Self {
        Self {
            title: file.title.to_lowercase(),
            artist: file.artist.to_lowercase(),
            album: file.album.to_lowercase(),
            name: file
                .path
                .file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default(),
        }
    }

    fn matches(&self, words: &[String]) -> bool {
        words.iter().all(|word| {
            self.title.contains(word.as_str())
                || self.artist.contains(word.as_str())
                || self.album.contains(word.as_str())
                || self.name.contains(word.as_str())
        })
    }
}

fn album_order(a: (&LocalFile, &Folded), b: (&LocalFile, &Folded)) -> Ordering {
    a.1.album
        .cmp(&b.1.album)
        .then(
            a.0.disc_number
                .unwrap_or(0)
                .cmp(&b.0.disc_number.unwrap_or(0)),
        )
        .then(
            a.0.track_number
                .unwrap_or(0)
                .cmp(&b.0.track_number.unwrap_or(0)),
        )
}

fn compare(a: (&LocalFile, &Folded), b: (&LocalFile, &Folded), column: Column) -> Ordering {
    match column {
        Column::Added => a.0.added_ms.cmp(&b.0.added_ms),
        Column::Title => a.1.title.cmp(&b.1.title),
        Column::Artist => a.1.artist.cmp(&b.1.artist).then_with(|| album_order(a, b)),
        Column::Album => album_order(a, b),
        Column::Duration => a.0.duration_ms.cmp(&b.0.duration_ms),
    }
}

pub fn order(files: &[LocalFile], sort: Sort, query: &str) -> Vec<usize> {
    let folded: Vec<Folded> = files.iter().map(Folded::of).collect();
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut order: Vec<usize> = (0..files.len())
        .filter(|&index| folded[index].matches(&words))
        .collect();
    order.sort_by(|&a, &b| {
        let first = (&files[a], &folded[a]);
        let second = (&files[b], &folded[b]);
        let primary = compare(first, second, sort.column);
        let primary = if sort.descending {
            primary.reverse()
        } else {
            primary
        };
        primary.then_with(|| files[a].path.cmp(&files[b].path))
    });
    order
}

pub fn display_path(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| path.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

pub fn count_label(count: usize) -> String {
    let digits = count.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    let unit = if count == 1 { "song" } else { "songs" };
    format!("{grouped} {unit}")
}

pub fn total_label(duration_ms: u64) -> String {
    let minutes = duration_ms / 60_000;
    match (minutes / 60, minutes % 60) {
        (0, 0) => format!("{} sec", duration_ms / 1000),
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} hr"),
        (hours, minutes) => format!("{hours} hr {minutes} min"),
    }
}

pub fn checked_label(elapsed: Duration) -> String {
    match elapsed.as_secs() / 60 {
        0 => "Checked just now.".to_owned(),
        1 => "Checked 1 minute ago.".to_owned(),
        minutes if minutes < 60 => format!("Checked {minutes} minutes ago."),
        minutes if minutes < 120 => "Checked 1 hour ago.".to_owned(),
        minutes => format!("Checked {} hours ago.", minutes / 60),
    }
}

pub struct LocalLibrary {
    pub files: Vec<LocalFile>,
    pub loaded: bool,
    pub scanning: bool,
    pub generation: u64,
    pub scanned: Option<LocalSources>,
    pub seen: usize,
    pub read: usize,
    pub finished: Option<Instant>,
    pub report: Option<ScanReport>,
    pub sort: Sort,
    pub query: String,
    pub filter: Option<Entity<InputState>>,
    pub order: Vec<usize>,
    pub shown: Vec<Track>,
    pub limit: usize,
    pub sentinel: Sentinel,
    pub art_requested: HashSet<String>,
    pub index: LocalIndex,
    pub version: u64,
    pub rows: MatchedRows,
}

impl Default for LocalLibrary {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            loaded: false,
            scanning: false,
            generation: 0,
            scanned: None,
            seen: 0,
            read: 0,
            finished: None,
            report: None,
            sort: Sort::default(),
            query: String::new(),
            filter: None,
            order: Vec::new(),
            shown: Vec::new(),
            limit: PAGE_STEP,
            sentinel: Sentinel::default(),
            art_requested: HashSet::new(),
            index: LocalIndex::default(),
            version: 0,
            rows: MatchedRows::default(),
        }
    }
}

impl LocalLibrary {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        urls.extend(
            self.shown
                .iter()
                .take(self.limit)
                .filter_map(|track| track.cover.clone()),
        );
        urls.extend(self.rows.keys().cloned());
    }

    pub fn forget_art(&mut self, keys: &[String]) {
        for key in keys {
            self.art_requested.remove(key);
        }
    }

    pub fn reindex(&mut self) {
        self.index = LocalIndex::build(&self.files);
        self.version += 1;
    }

    pub fn refresh(&mut self) {
        self.order = order(&self.files, self.sort, &self.query);
        self.shown = self
            .order
            .iter()
            .map(|&index| self.files[index].track())
            .collect();
    }

    pub fn apply(&mut self, update: ScanUpdate) -> bool {
        if update.generation != self.generation {
            return false;
        }
        self.seen = update.seen;
        self.read = update.read;
        let changed = update.files.is_some();
        if let Some(files) = update.files {
            self.files = files;
            self.loaded = true;
            self.reindex();
            self.refresh();
        }
        if let Some(report) = update.finished {
            self.scanning = false;
            self.finished = Some(Instant::now());
            self.report = Some(report);
        }
        changed
    }

    pub fn file_at(&self, row: usize) -> Option<&LocalFile> {
        self.order.get(row).map(|&index| &self.files[index])
    }

    pub fn missing_at(&self, row: usize) -> bool {
        self.file_at(row).is_some_and(|file| file.missing)
    }

    pub fn file_for(&self, uri: &str) -> Option<&LocalFile> {
        self.index
            .exact(uri)
            .and_then(|position| self.files.get(position))
    }

    pub fn matching(&self, uri: &str) -> Option<&LocalFile> {
        self.index
            .find(&self.files, uri)
            .and_then(|position| self.files.get(position))
    }

    pub fn queue_from(&self, row: usize) -> Option<(Vec<LocalFile>, usize)> {
        let target = *self.order.get(row)?;
        if self.files[target].missing {
            return None;
        }
        let available: Vec<usize> = self
            .order
            .iter()
            .copied()
            .filter(|&index| !self.files[index].missing)
            .collect();
        let index = available.iter().position(|&index| index == target)?;
        let files = available
            .into_iter()
            .map(|index| self.files[index].clone())
            .collect();
        Some((files, index))
    }

    pub fn visible_art(&self) -> Vec<(String, PathBuf)> {
        let mut seen = HashSet::new();
        self.order
            .iter()
            .take(self.limit)
            .filter_map(|&index| {
                let file = &self.files[index];
                let key = file.artwork.clone()?;
                seen.insert(key.clone()).then(|| (key, file.path.clone()))
            })
            .collect()
    }

    pub fn total_duration(&self) -> u64 {
        self.order
            .iter()
            .map(|&index| u64::from(self.files[index].duration_ms))
            .sum()
    }

    pub fn missing_count(&self) -> usize {
        self.files.iter().filter(|file| file.missing).count()
    }

    pub fn status(&self) -> String {
        if self.scanning {
            return match self.seen {
                0 => "Checking your folders…".to_owned(),
                seen => format!("Checking your folders… {} so far.", count_label(seen)),
            };
        }
        let Some(finished) = self.finished else {
            return if self.loaded {
                format!("{} from the last check.", count_label(self.files.len()))
            } else {
                "Not checked yet.".to_owned()
            };
        };
        let mut status = format!(
            "{} on this Mac. {}",
            count_label(self.files.len()),
            checked_label(finished.elapsed())
        );
        if self.report.as_ref().is_some_and(|report| report.truncated) {
            status.push_str(&format!(
                " Only the first {} are listed.",
                count_label(MAX_FILES)
            ));
        }
        status
    }
}

impl OpenMusic {
    pub fn local_sources(&self) -> &LocalSources {
        &self.settings.account.local_files
    }

    pub fn local_files_on(&self) -> bool {
        self.local_sources().enabled
    }

    pub fn ensure_local_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.account.local.filter.is_some() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search in Local Files"));
        cx.subscribe_in(&input, window, |this, input, event, window, cx| {
            match event {
                InputEvent::Change => {
                    let query = input.read(cx).value().to_string();
                    this.filter_local_files(query);
                }
                InputEvent::Blur if window.focused(cx).is_none() => window.focus(&this.focus),
                _ => {}
            }
            cx.notify();
        })
        .detach();
        self.account.local.filter = Some(input);
    }

    pub fn sync_local_files(&mut self) {
        if !matches!(self.session, Session::Connected) {
            return;
        }
        let sources = self.local_sources();
        let local = &self.account.local;
        if !sources.enabled {
            if local.scanned.is_some() || local.scanning {
                self.stop_local_scan();
            }
            return;
        }
        if local.scanned.as_ref() != Some(sources) {
            self.start_local_scan();
        }
    }

    pub fn start_local_scan(&mut self) {
        let sources = self.local_sources().clone();
        let local = &mut self.account.local;
        local.generation += 1;
        local.scanning = true;
        local.seen = 0;
        local.read = 0;
        local.scanned = Some(sources.clone());
        let command = Command::LocalScan {
            sources,
            generation: local.generation,
            load: !local.loaded,
        };
        self.send(command);
    }

    pub fn stop_local_scan(&mut self) {
        let local = &mut self.account.local;
        local.generation += 1;
        local.scanning = false;
        local.scanned = None;
        let generation = local.generation;
        self.send(Command::LocalStop(generation));
    }

    pub fn rescan_local_files(&mut self) {
        if self.local_files_on() && matches!(self.session, Session::Connected) {
            self.start_local_scan();
        }
    }

    pub fn set_local_sources(&mut self, change: impl FnOnce(&mut LocalSources)) {
        self.update_settings(|settings| change(&mut settings.account.local_files));
        self.sync_local_files();
    }

    pub fn add_local_sources(&mut self, paths: Vec<PathBuf>) {
        self.set_local_sources(|sources| {
            sources.enabled = true;
            for path in paths {
                if !sources.folders.contains(&path) {
                    sources.folders.push(path);
                }
            }
        });
    }

    pub fn remove_local_source(&mut self, path: &Path) {
        self.set_local_sources(|sources| sources.folders.retain(|folder| folder != path));
    }

    pub fn add_local_folders(&mut self, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Add".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = answer.await {
                let _ = this.update(cx, |app, cx| {
                    app.add_local_sources(paths);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn filter_local_files(&mut self, query: String) {
        let local = &mut self.account.local;
        if local.query == query {
            return;
        }
        local.query = query;
        local.limit = PAGE_STEP;
        local.refresh();
        self.request_local_art();
    }

    pub fn sort_local_files(&mut self, sort: Sort) {
        let local = &mut self.account.local;
        if local.sort == sort {
            return;
        }
        local.sort = sort;
        local.limit = PAGE_STEP;
        local.refresh();
        self.request_local_art();
    }

    pub fn sort_local_by(&mut self, column: Column) {
        let sort = self.account.local.sort.pick(column);
        self.sort_local_files(sort);
    }

    pub fn request_local_art(&mut self) {
        let wanted: Vec<(String, PathBuf)> = self
            .account
            .local
            .visible_art()
            .into_iter()
            .filter(|(key, _)| {
                !self.covers.contains_key(key) && !self.account.local.art_requested.contains(key)
            })
            .collect();
        if wanted.is_empty() {
            return;
        }
        self.account
            .local
            .art_requested
            .extend(wanted.iter().map(|(key, _)| key.clone()));
        self.send(Command::LocalArt(wanted));
    }

    pub fn play_local_row(&mut self, row: usize) {
        match self.account.local.queue_from(row) {
            Some((files, index)) => self.send(Command::PlayLocal { files, index }),
            None => self.show_toast(MISSING_FILE, None),
        }
    }

    pub fn local_playing(&self) -> bool {
        plays_local_file_here(
            &self.account.local,
            self.local_files_on(),
            &self.playback.now,
        )
    }

    pub fn play_local_files(&mut self, cx: &mut Context<Self>) {
        if self.local_playing() {
            self.toggle(cx);
            return;
        }
        let first =
            (0..self.account.local.order.len()).find(|&row| !self.account.local.missing_at(row));
        match first {
            Some(row) => self.play_local_row(row),
            None => self.show_toast(NOTHING_TO_PLAY, None),
        }
    }

    pub fn open_local_menu(&mut self) {
        let items = vec![
            MenuItem::action("Rescan Folders", |app, _, _| app.rescan_local_files())
                .symbol("arrow.clockwise")
                .enabled(self.local_files_on()),
            MenuItem::action("Add Folder…", |app, _, cx| app.add_local_folders(cx))
                .symbol("folder.badge.plus"),
            MenuItem::Separator,
            MenuItem::action("Local Files Settings…", |app, _, _| {
                app.navigate(Page::Settings)
            })
            .symbol("gearshape"),
        ];
        self.show_menu(items, Place::Under(MORE_KEY), None);
    }

    pub fn open_local_sort_menu(&mut self) {
        let sort = self.account.local.sort;
        let mut items = vec![MenuItem::header("Sort by")];
        items.extend(Column::ALL.into_iter().map(|column| {
            let active = sort.column == column;
            let item = MenuItem::action(column.label(), move |app, _, _| app.sort_local_by(column))
                .checked(active);
            match (active, sort.descending) {
                (true, true) => item.symbol("arrow.down"),
                (true, false) => item.symbol("arrow.up"),
                (false, _) => item,
            }
        }));
        self.show_menu(items, Place::Under(SORT_KEY), None);
    }
}

pub fn plays_local_file_here(local: &LocalLibrary, enabled: bool, now: &NowPlaying) -> bool {
    now.local
        && now
            .track
            .as_ref()
            .is_some_and(|track| local_match::matched_file(local, enabled, &track.uri).is_some())
}

pub fn load(app: &mut OpenMusic) {
    let local = &mut app.account.local;
    local.limit = PAGE_STEP;
    local.art_requested.clear();
    let stale = local
        .finished
        .is_none_or(|finished| finished.elapsed() >= STALE);
    if app.local_files_on() && !app.account.local.scanning && stale {
        app.start_local_scan();
    }
    app.request_local_art();
}

pub fn connected(app: &mut OpenMusic, window: &mut Window, cx: &mut Context<OpenMusic>) {
    app.ensure_local_filter(window, cx);
    let local = &mut app.account.local;
    local.scanned = None;
    local.scanning = false;
    app.sync_local_files();
}

pub fn signed_out(local: &mut LocalLibrary) {
    local.scanning = false;
    local.scanned = None;
    local.art_requested.clear();
}

pub fn on_scan(app: &mut OpenMusic, update: ScanUpdate) {
    if app.account.local.apply(update) && app.page == Page::LocalFiles {
        app.request_local_art();
    }
}

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    if !matches!(app.session, Session::Connected) {
        return;
    }
    app.sync_local_files();
    let local = &mut app.account.local;
    if app.page != Page::LocalFiles || !local.sentinel.take() || local.limit >= local.order.len() {
        return;
    }
    local.limit += PAGE_STEP;
    app.request_local_art();
    cx.notify();
}

pub fn nav_entry(app: &OpenMusic) -> Option<NavEntry> {
    if !app.local_files_on() || app.account.local.files.is_empty() {
        return None;
    }
    Some(NavEntry::new(
        NAV_KEY,
        NAV_WEIGHT,
        icons::LOCAL,
        "Local Files",
        app.page == Page::LocalFiles,
        |app, _, _| app.navigate(Page::LocalFiles),
    ))
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let Target::Track(target) = target else {
        return Vec::new();
    };
    if !is_local(&target.track.uri) {
        return Vec::new();
    }
    let Some(file) = app
        .account
        .local
        .file_for(&target.track.uri)
        .or_else(|| app.local_match(&target.track.uri))
    else {
        return Vec::new();
    };
    let path = file.path.clone();
    vec![(
        Group::Navigate,
        MenuItem::action("Show in Finder", move |_, _, cx| cx.reveal_path(&path))
            .symbol("folder")
            .enabled(!file.missing),
    )]
}

pub fn row_dimmed(app: &OpenMusic, row: Row) -> bool {
    row.0 == ROW && app.account.local.missing_at(row.1)
}

pub fn state_fragment(app: &OpenMusic) -> String {
    let local = &app.account.local;
    let count = if app.local_files_on() {
        local.files.len().to_string()
    } else {
        "off".to_owned()
    };
    format!("local={count} local_scanning={}", local.scanning)
}

pub fn describe(app: &OpenMusic) -> String {
    let local = &app.account.local;
    let sources = app.local_sources();
    let report = local.report.clone().unwrap_or_default();
    let paths = |paths: &[PathBuf]| {
        paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("|")
    };
    let mut lines = vec![format!(
        "enabled={} downloads={} music={} folders={} scanning={} generation={} seen={} read={} loaded={} count={} shown={} limit={} missing={} denied={} missing_roots={} truncated={} sort={} {} filter={:?} finished={} index={}",
        sources.enabled,
        sources.downloads,
        sources.music,
        paths(&sources.folders),
        local.scanning,
        local.generation,
        local.seen,
        local.read,
        local.loaded,
        local.files.len(),
        local.order.len(),
        local.limit,
        local.missing_count(),
        paths(&report.denied),
        paths(&report.missing),
        report.truncated,
        local.sort.column.name(),
        if local.sort.descending { "desc" } else { "asc" },
        local.query,
        local.finished.map_or("never".to_owned(), |at| format!(
            "{}s",
            at.elapsed().as_secs()
        )),
        index_path().map_or("none".to_owned(), |path| path.display().to_string()),
    )];
    lines.extend(local.order.iter().take(local.limit).map(|&index| {
        let file = &local.files[index];
        let art = file.artwork.as_deref().unwrap_or("none");
        format!(
            "{} | {} | {} | {} | {} | added={} | art={} loaded={} | missing={} | {}",
            file.uri,
            file.title,
            file.artist,
            file.album,
            file.duration_ms,
            file.added_ms,
            art,
            file.artwork
                .as_ref()
                .is_some_and(|key| app.covers.contains_key(key)),
            file.missing,
            display_path(&file.path, home().as_deref()),
        )
    }));
    lines.join("\n")
}

fn switch(text: &str) -> Option<bool> {
    match text {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

pub fn debug(
    app: &mut OpenMusic,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    let (word, argument) = rest.split_once(' ').unwrap_or((rest, ""));
    let argument = argument.trim();
    match (word, switch(argument)) {
        ("on" | "off", _) => {
            let on = word == "on";
            app.set_local_sources(|sources| sources.enabled = on);
        }
        ("scan", _) => app.rescan_local_files(),
        ("downloads", Some(on)) => app.set_local_sources(|sources| sources.downloads = on),
        ("music", Some(on)) => app.set_local_sources(|sources| sources.music = on),
        ("add", _) if !argument.is_empty() => app.add_local_sources(vec![argument.into()]),
        ("remove", _) if !argument.is_empty() => app.remove_local_source(Path::new(argument)),
        ("prompt", _) => app.add_local_folders(cx),
        ("sort", _) => {
            let mut words = argument.split_whitespace();
            if let Some(column) = words.next().and_then(Column::from_name) {
                let descending = match words.next() {
                    Some("desc") => true,
                    Some("asc") => false,
                    _ => column.newest_first(),
                };
                app.sort_local_files(Sort { column, descending });
            }
        }
        ("filter", _) => {
            if let Some(input) = app.account.local.filter.clone() {
                let text = argument.to_owned();
                input.update(cx, |input, cx| input.set_value(text, window, cx));
            }
            app.filter_local_files(argument.to_owned());
        }
        ("play", _) => {
            if let Ok(row) = argument.parse() {
                app.play_local_row(row);
            }
        }
        ("art", _) => app.request_local_art(),
        ("menu", _) => app.open_local_menu(),
        ("sort-menu", _) => app.open_local_sort_menu(),
        ("rows" | "match" | "playrow", _) => local_match::debug(app, word, argument, directory, cx),
        _ => {}
    }
    let _ = std::fs::write(directory.join("localfiles.txt"), describe(app));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, title: &str, artist: &str, album: &str, added: i64) -> LocalFile {
        LocalFile {
            path: PathBuf::from(path),
            uri: format!("spotify:local:{artist}:{album}:{title}:60"),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            added_ms: added,
            duration_ms: 60_000,
            ..LocalFile::default()
        }
    }

    fn library() -> Vec<LocalFile> {
        vec![
            LocalFile {
                track_number: Some(2),
                duration_ms: 200_000,
                ..file("/m/b.mp3", "bravo", "Zed", "Second", 300)
            },
            LocalFile {
                track_number: Some(1),
                duration_ms: 100_000,
                ..file("/m/a.mp3", "Alpha", "zed", "Second", 100)
            },
            LocalFile {
                missing: true,
                ..file("/x/c.flac", "charlie", "Amy", "First", 200)
            },
        ]
    }

    fn titles(files: &[LocalFile], order: &[usize]) -> Vec<String> {
        order
            .iter()
            .map(|&index| files[index].title.clone())
            .collect()
    }

    #[test]
    fn sorts_and_filters_local_files() {
        let files = library();
        let newest = Sort::default();
        assert_eq!(
            titles(&files, &order(&files, newest, "")),
            ["bravo", "charlie", "Alpha"]
        );
        let by_title = newest.pick(Column::Title);
        assert!(!by_title.descending);
        assert_eq!(
            titles(&files, &order(&files, by_title, "")),
            ["Alpha", "bravo", "charlie"]
        );
        let back = by_title.pick(Column::Title);
        assert!(back.descending);
        assert_eq!(
            titles(&files, &order(&files, back, "")),
            ["charlie", "bravo", "Alpha"]
        );
        let by_artist = newest.pick(Column::Artist);
        assert_eq!(
            titles(&files, &order(&files, by_artist, "")),
            ["charlie", "Alpha", "bravo"]
        );
        let by_album = newest.pick(Column::Album);
        assert_eq!(
            titles(&files, &order(&files, by_album, "")),
            ["charlie", "Alpha", "bravo"]
        );
        let longest = Sort {
            column: Column::Duration,
            descending: true,
        };
        assert_eq!(
            titles(&files, &order(&files, longest, "")),
            ["bravo", "Alpha", "charlie"]
        );
        assert_eq!(
            titles(&files, &order(&files, by_title, "ZED sec")),
            ["Alpha", "bravo"]
        );
        assert_eq!(
            titles(&files, &order(&files, by_title, "c.flac")),
            ["charlie"]
        );
        assert!(order(&files, by_title, "nothing").is_empty());
        assert_eq!(Column::from_name("album"), Some(Column::Album));
        assert_eq!(Column::from_name("year"), None);
    }

    #[test]
    fn queues_only_files_that_are_there() {
        let mut local = LocalLibrary {
            files: library(),
            sort: Sort::default().pick(Column::Title),
            ..LocalLibrary::default()
        };
        local.refresh();
        assert_eq!(local.shown.len(), 3);
        let (queue, index) = local.queue_from(1).unwrap();
        assert_eq!(
            queue
                .iter()
                .map(|file| file.title.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "bravo"]
        );
        assert_eq!(index, 1);
        assert!(local.queue_from(2).is_none());
        assert!(local.missing_at(2));
        assert!(!local.missing_at(0));
        assert!(local.queue_from(9).is_none());
        assert_eq!(local.missing_count(), 1);
        assert_eq!(local.total_duration(), 360_000);
    }

    #[test]
    fn applies_scan_updates_of_the_current_scan_only() {
        let mut local = LocalLibrary {
            generation: 4,
            scanning: true,
            ..LocalLibrary::default()
        };
        let stale = ScanUpdate {
            generation: 3,
            files: Some(library()),
            ..ScanUpdate::default()
        };
        assert!(!local.apply(stale));
        assert!(local.files.is_empty());
        let partial = ScanUpdate {
            generation: 4,
            files: Some(library()),
            seen: 500,
            read: 20,
            finished: None,
        };
        assert!(local.apply(partial));
        assert!(local.loaded && local.scanning);
        assert_eq!(local.shown.len(), 3);
        assert!(
            local
                .status()
                .starts_with("Checking your folders… 500 songs")
        );
        let counts = ScanUpdate {
            generation: 4,
            seen: 1000,
            ..ScanUpdate::default()
        };
        assert!(!local.apply(counts));
        assert_eq!(local.files.len(), 3);
        let done = ScanUpdate {
            generation: 4,
            files: Some(library()[..1].to_vec()),
            finished: Some(ScanReport::default()),
            ..ScanUpdate::default()
        };
        assert!(local.apply(done));
        assert!(!local.scanning);
        assert_eq!(local.status(), "1 song on this Mac. Checked just now.");
    }

    #[test]
    fn asks_for_each_cover_once_within_the_shown_rows() {
        let mut files = library();
        files[0].artwork = Some("local-art:1".into());
        files[1].artwork = Some("local-art:1".into());
        files[2].artwork = Some("local-art:2".into());
        let mut local = LocalLibrary {
            files,
            limit: 1,
            ..LocalLibrary::default()
        };
        local.refresh();
        assert_eq!(
            local.visible_art(),
            vec![("local-art:1".to_owned(), PathBuf::from("/m/b.mp3"))]
        );
        local.limit = 3;
        assert_eq!(
            local.visible_art(),
            vec![
                ("local-art:1".to_owned(), PathBuf::from("/m/b.mp3")),
                ("local-art:2".to_owned(), PathBuf::from("/x/c.flac")),
            ]
        );
        let mut urls = HashSet::new();
        local.covers(&mut urls);
        assert_eq!(urls.len(), 2);
    }

    #[test]
    fn asks_again_for_art_that_was_dropped() {
        let mut local = LocalLibrary {
            art_requested: ["local-art:1", "local-art:2", "local-art:3"]
                .map(str::to_owned)
                .into(),
            ..LocalLibrary::default()
        };
        local.forget_art(&["local-art:2".to_owned(), "https://i.scdn.co/x".to_owned()]);
        assert_eq!(
            local.art_requested,
            ["local-art:1", "local-art:3"].map(str::to_owned).into()
        );
        signed_out(&mut local);
        assert!(local.art_requested.is_empty());
    }

    #[test]
    fn plays_a_matched_file_only_when_this_mac_plays_it() {
        let mut local = LocalLibrary {
            files: library(),
            loaded: true,
            ..LocalLibrary::default()
        };
        local.reindex();
        let matched = local.files[0].track();
        let here = NowPlaying {
            track: Some(matched.clone()),
            local: true,
            ..NowPlaying::default()
        };
        assert!(plays_local_file_here(&local, true, &here));
        assert!(!plays_local_file_here(&local, false, &here));
        let phone = NowPlaying {
            local: false,
            device: Some("Phone".into()),
            ..here.clone()
        };
        assert!(!plays_local_file_here(&local, true, &phone));
        let streamed = NowPlaying {
            track: Some(Track {
                uri: "spotify:track:00000000000000000000t1".into(),
                ..matched
            }),
            ..here
        };
        assert!(!plays_local_file_here(&local, true, &streamed));
    }

    #[test]
    fn labels_counts_paths_and_times() {
        assert_eq!(count_label(1), "1 song");
        assert_eq!(count_label(0), "0 songs");
        assert_eq!(count_label(1234), "1,234 songs");
        assert_eq!(count_label(50_000), "50,000 songs");
        assert_eq!(count_label(1_234_567), "1,234,567 songs");
        assert_eq!(total_label(42_000), "42 sec");
        assert_eq!(total_label(185_000), "3 min");
        assert_eq!(total_label(7_200_000), "2 hr");
        assert_eq!(total_label(54_300_000), "15 hr 5 min");
        let home = Path::new("/Users/someone");
        assert_eq!(
            display_path(Path::new("/Users/someone/Music"), Some(home)),
            "~/Music"
        );
        assert_eq!(display_path(home, Some(home)), "~");
        assert_eq!(
            display_path(Path::new("/Volumes/Drive"), Some(home)),
            "/Volumes/Drive"
        );
        assert_eq!(checked_label(Duration::from_secs(20)), "Checked just now.");
        assert_eq!(
            checked_label(Duration::from_secs(61)),
            "Checked 1 minute ago."
        );
        assert_eq!(
            checked_label(Duration::from_secs(600)),
            "Checked 10 minutes ago."
        );
        assert_eq!(
            checked_label(Duration::from_secs(3700)),
            "Checked 1 hour ago."
        );
        assert_eq!(
            checked_label(Duration::from_secs(4 * 3600)),
            "Checked 4 hours ago."
        );
    }
}
