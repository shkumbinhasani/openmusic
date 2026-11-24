use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui::{App, Context, MouseDownEvent, Pixels, Point, Window, point};
use spotify_client::{Track, entity};

use crate::{
    app::{OpenMusic, Session},
    areas::{account, audio, content, details, discovery, library, playback, playlists, system},
    overlay::{self, Action, Entry},
};

pub type Run = Rc<dyn Fn(&mut OpenMusic, &mut Window, &mut Context<OpenMusic>)>;

pub type Row = (&'static str, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Surface {
    Row,
    Card,
    NowPlaying,
    Header,
}

impl Surface {
    pub fn name(self) -> &'static str {
        match self {
            Self::Row => "row",
            Self::Card => "card",
            Self::NowPlaying => "nowplaying",
            Self::Header => "header",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackTarget {
    pub track: Track,
    pub uid: Option<String>,
    pub context: Option<String>,
    pub provider: Option<String>,
    pub surface: Surface,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Track(Box<TrackTarget>),
    Album { uri: String },
    Artist { uri: String },
    Playlist { uri: String },
    Folder { uri: String },
    Episode { uri: String },
    Show { uri: String },
    User { uri: String },
}

impl Target {
    pub fn track(track: Track, context: Option<String>, surface: Surface) -> Self {
        Self::Track(Box::new(TrackTarget {
            uid: track.uid.clone(),
            track,
            context,
            provider: None,
            surface,
        }))
    }

    pub fn for_uri(uri: &str) -> Option<Self> {
        let uri = uri.to_owned();
        if entity::is_liked_songs(&uri) {
            return Some(Self::Playlist { uri });
        }
        let parts: Vec<&str> = uri.split(':').collect();
        match parts.as_slice() {
            ["spotify", "album", _] => Some(Self::Album { uri }),
            ["spotify", "artist", _] => Some(Self::Artist { uri }),
            ["spotify", "playlist", _] | ["spotify", "user", _, "playlist", _] => {
                Some(Self::Playlist { uri })
            }
            ["spotify", "user", _, "folder", _] => Some(Self::Folder { uri }),
            ["spotify", "episode", _] => Some(Self::Episode { uri }),
            ["spotify", "show", _] => Some(Self::Show { uri }),
            ["spotify", "user", _] => Some(Self::User { uri }),
            _ => None,
        }
    }

    pub fn parse(kind: &str, uri: &str, context: Option<&str>, app: &OpenMusic) -> Option<Self> {
        let uri = uri.to_owned();
        Some(match kind {
            "track" => {
                let track = app
                    .track()
                    .filter(|track| track.uri == uri)
                    .cloned()
                    .unwrap_or_else(|| Track::placeholder(&uri));
                Self::track(track, context.map(str::to_owned), Surface::Row)
            }
            "album" => Self::Album { uri },
            "artist" => Self::Artist { uri },
            "playlist" => Self::Playlist { uri },
            "folder" => Self::Folder { uri },
            "episode" => Self::Episode { uri },
            "show" => Self::Show { uri },
            "user" => Self::User { uri },
            _ => return None,
        })
    }

    pub fn uri(&self) -> &str {
        match self {
            Self::Track(target) => &target.track.uri,
            Self::Album { uri }
            | Self::Artist { uri }
            | Self::Playlist { uri }
            | Self::Folder { uri }
            | Self::Episode { uri }
            | Self::Show { uri }
            | Self::User { uri } => uri,
        }
    }

    pub fn describe(&self) -> String {
        let kind = match self {
            Self::Track(_) => "track",
            Self::Album { .. } => "album",
            Self::Artist { .. } => "artist",
            Self::Playlist { .. } => "playlist",
            Self::Folder { .. } => "folder",
            Self::Episode { .. } => "episode",
            Self::Show { .. } => "show",
            Self::User { .. } => "user",
        };
        let mut line = format!("{kind} {}", self.uri());
        if let Self::Track(target) = self {
            for (name, value) in [
                ("uid", &target.uid),
                ("context", &target.context),
                ("provider", &target.provider),
            ] {
                if let Some(value) = value {
                    line.push_str(&format!(" {name}={value}"));
                }
            }
            line.push_str(&format!(" surface={}", target.surface.name()));
        }
        line
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Library,
    Queue,
    Edit,
    Feedback,
    Navigate,
    Info,
    Share,
}

impl Group {
    fn section(self) -> u8 {
        match self {
            Self::Library | Self::Queue | Self::Edit | Self::Feedback => 0,
            Self::Navigate | Self::Info => 1,
            Self::Share => 2,
        }
    }
}

pub enum MenuItem {
    Action {
        title: String,
        enabled: bool,
        checked: bool,
        symbol: Option<&'static str>,
        key: Option<&'static str>,
        subtitle: Option<String>,
        alternate: Option<(String, Run)>,
        run: Run,
    },
    Note(String),
    Header(String),
    Separator,
    Submenu {
        title: String,
        symbol: Option<&'static str>,
        items: Vec<MenuItem>,
        search: Option<&'static str>,
        id: Option<&'static str>,
    },
}

impl MenuItem {
    pub fn action(
        title: impl Into<String>,
        run: impl Fn(&mut OpenMusic, &mut Window, &mut Context<OpenMusic>) + 'static,
    ) -> Self {
        Self::Action {
            title: title.into(),
            enabled: true,
            checked: false,
            symbol: None,
            key: None,
            subtitle: None,
            alternate: None,
            run: Rc::new(run),
        }
    }

    pub fn note(text: impl Into<String>) -> Self {
        Self::Note(text.into())
    }

    pub fn header(text: impl Into<String>) -> Self {
        Self::Header(text.into())
    }

    pub fn submenu(title: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self::Submenu {
            title: title.into(),
            symbol: None,
            items,
            search: None,
            id: None,
        }
    }

    pub fn enabled(mut self, value: bool) -> Self {
        if let Self::Action { enabled, .. } = &mut self {
            *enabled = value;
        }
        self
    }

    pub fn checked(mut self, value: bool) -> Self {
        if let Self::Action { checked, .. } = &mut self {
            *checked = value;
        }
        self
    }

    pub fn symbol(mut self, name: &'static str) -> Self {
        if let Self::Action { symbol, .. } | Self::Submenu { symbol, .. } = &mut self {
            *symbol = Some(name);
        }
        self
    }

    pub fn key(mut self, value: &'static str) -> Self {
        if let Self::Action { key, .. } = &mut self {
            *key = Some(value);
        }
        self
    }

    pub fn subtitle(mut self, text: impl Into<String>) -> Self {
        if let Self::Action { subtitle, .. } = &mut self {
            *subtitle = Some(text.into());
        }
        self
    }

    pub fn alternate(
        mut self,
        title: impl Into<String>,
        run: impl Fn(&mut OpenMusic, &mut Window, &mut Context<OpenMusic>) + 'static,
    ) -> Self {
        if let Self::Action { alternate, .. } = &mut self {
            *alternate = Some((title.into(), Rc::new(run)));
        }
        self
    }

    pub fn search(mut self, placeholder: &'static str) -> Self {
        if let Self::Submenu { search, .. } = &mut self {
            *search = Some(placeholder);
        }
        self
    }

    pub fn id(mut self, value: &'static str) -> Self {
        if let Self::Submenu { id, .. } = &mut self {
            *id = Some(value);
        }
        self
    }
}

pub type Provider = fn(&Target, &OpenMusic) -> Vec<(Group, MenuItem)>;

pub const PROVIDERS: &[Provider] = &[
    account::view::menu_items,
    audio::view::menu_items,
    content::view::menu_items,
    details::view::menu_items,
    discovery::view::menu_items,
    library::view::menu_items,
    playback::view::menu_items,
    playlists::view::menu_items,
    system::view::menu_items,
];

pub fn grouped(mut items: Vec<(Group, MenuItem)>) -> Vec<MenuItem> {
    items.sort_by_key(|(group, _)| *group);
    let mut menu = Vec::with_capacity(items.len() + 2);
    let mut last: Option<Group> = None;
    for (group, item) in items {
        if last.is_some_and(|last| last.section() != group.section()) {
            menu.push(MenuItem::Separator);
        }
        last = Some(group);
        menu.push(item);
    }
    menu
}

fn push(runs: &mut Vec<Run>, run: Run) -> u32 {
    runs.push(run);
    runs.len() as u32 - 1
}

fn lower(
    items: Vec<MenuItem>,
    runs: &mut Vec<Run>,
    submenus: &mut HashMap<&'static str, u32>,
) -> Vec<Entry> {
    items
        .into_iter()
        .map(|item| match item {
            MenuItem::Action {
                title,
                enabled,
                checked,
                symbol,
                key,
                subtitle,
                alternate,
                run,
            } => {
                let id = push(runs, run);
                let alternate = alternate.map(|(title, run)| (title, push(runs, run)));
                Entry::Item {
                    title,
                    id,
                    enabled,
                    checked,
                    symbol,
                    key,
                    alternate,
                    subtitle,
                }
            }
            MenuItem::Note(text) => Entry::Note(text),
            MenuItem::Header(text) => Entry::Header(text),
            MenuItem::Separator => Entry::Separator,
            MenuItem::Submenu {
                title,
                symbol,
                items,
                search,
                id,
            } => {
                let id = id.map(|key| {
                    let next = submenus.len() as u32;
                    *submenus.entry(key).or_insert(next)
                });
                Entry::Submenu {
                    title,
                    symbol,
                    entries: lower(items, runs, submenus),
                    search,
                    id,
                }
            }
        })
        .collect()
}

pub fn dump(entries: &[Entry], depth: usize, lines: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    for entry in entries {
        match entry {
            Entry::Item {
                title,
                enabled,
                checked,
                symbol,
                key,
                alternate,
                subtitle,
                ..
            } => {
                let mut line = format!("{indent}{title}");
                if let Some(key) = key {
                    line.push_str(&format!("  {key}"));
                }
                if let Some(subtitle) = subtitle {
                    line.push_str(&format!("  — {subtitle}"));
                }
                if let Some(symbol) = symbol {
                    line.push_str(&format!("  <{symbol}>"));
                }
                if *checked {
                    line.push_str(" ✓");
                }
                if !enabled {
                    line.push_str(" [x]");
                }
                lines.push(line);
                if let Some((title, _)) = alternate {
                    lines.push(format!("{indent}(⌥) {title}"));
                }
            }
            Entry::Note(text) => lines.push(format!("{indent}{text} [x]")),
            Entry::Header(text) => lines.push(format!("{indent}# {text}")),
            Entry::Separator => lines.push(format!("{indent}──")),
            Entry::Submenu {
                title,
                symbol,
                entries,
                search,
                ..
            } => {
                let mut line = format!("{indent}{title} ▸");
                if let Some(symbol) = symbol {
                    line.push_str(&format!("  <{symbol}>"));
                }
                lines.push(line);
                if let Some(placeholder) = search {
                    lines.push(format!("{indent}  [search “{placeholder}”]"));
                }
                dump(entries, depth + 1, lines);
            }
        }
    }
}

pub enum Place {
    Point(Point<Pixels>),
    Under(&'static str),
}

#[derive(Default)]
pub struct Menus {
    pub row: Option<Row>,
    runs: Vec<Run>,
    submenus: HashMap<&'static str, u32>,
    bar: Vec<Entry>,
    bar_runs: Vec<Run>,
    sample: Option<PathBuf>,
}

impl OpenMusic {
    pub fn menu_items(&self, target: &Target) -> Vec<MenuItem> {
        let mut items: Vec<(Group, MenuItem)> = PROVIDERS
            .iter()
            .flat_map(|provider| provider(target, self))
            .collect();
        if let Some(directory) = &self.menus.sample {
            items.extend(sample(directory));
        }
        grouped(items)
    }

    pub fn has_menu(&self, target: &Target) -> bool {
        !self.menu_items(target).is_empty()
    }

    pub fn show_menu(&mut self, items: Vec<MenuItem>, place: Place, row: Option<Row>) -> bool {
        if items.is_empty() {
            return false;
        }
        self.menus.runs.clear();
        self.menus.submenus.clear();
        let entries = lower(items, &mut self.menus.runs, &mut self.menus.submenus);
        let opened = match place {
            Place::Point(at) => overlay::context_menu(entries, at),
            Place::Under(anchor) => overlay::menu_at(anchor, entries),
        };
        if opened {
            self.menus.row = row;
        }
        opened
    }

    pub fn open_menu(&mut self, target: Target, at: Point<Pixels>, row: Option<Row>) -> bool {
        let items = self.menu_items(&target);
        self.show_menu(items, Place::Point(at), row)
    }

    pub fn open_menu_under(&mut self, target: Target, anchor: &'static str) -> bool {
        let items = self.menu_items(&target);
        self.show_menu(items, Place::Under(anchor), None)
    }

    pub fn update_submenu(&mut self, id: &'static str, items: Vec<MenuItem>) {
        let Some(&number) = self.menus.submenus.get(id) else {
            return;
        };
        let entries = lower(items, &mut self.menus.runs, &mut HashMap::new());
        overlay::update_menu(number, entries);
    }

    pub fn on_menu(
        &self,
        row: Option<Row>,
        target: impl Fn(&OpenMusic) -> Option<Target> + 'static,
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, event: &MouseDownEvent, _, cx| {
            let Some(target) = target(this) else {
                return;
            };
            cx.stop_propagation();
            if this.open_menu(target, event.position, row) {
                cx.notify();
            }
        })
    }

    pub fn menu_row(&self, row: Row) -> bool {
        self.menus.row == Some(row)
    }

    pub fn menu_action(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let run = match action {
            Action::Menu(id) => self.menus.runs.get(id as usize).cloned(),
            Action::BarMenu(id) => self.menus.bar_runs.get(id as usize).cloned(),
            Action::MenuClosed => {
                self.menus.row = None;
                None
            }
            Action::Press(_) | Action::Seek(_) | Action::Volume(_) => None,
        };
        if let Some(run) = run {
            run(self, window, cx);
        }
    }

    pub fn now_playing_target(&self, surface: Surface) -> Option<Target> {
        if !matches!(self.session, Session::Connected) {
            return None;
        }
        let now = &self.playback.now;
        Some(Target::Track(Box::new(TrackTarget {
            track: now.track.clone()?,
            uid: now.uid.clone(),
            context: now.context_uri.clone(),
            provider: now.provider.clone(),
            surface,
        })))
    }

    pub fn refresh_bar_menu(&mut self) {
        let items = self
            .now_playing_target(Surface::NowPlaying)
            .map(|target| self.menu_items(&target))
            .unwrap_or_default();
        let mut runs = Vec::new();
        let entries = lower(items, &mut runs, &mut HashMap::new());
        self.menus.bar_runs = runs;
        if entries != self.menus.bar {
            self.menus.bar = entries.clone();
            overlay::bar_menu(entries);
        }
    }
}

const SAMPLE_PLAYLISTS: &str = "sample-playlists";

fn record(
    directory: &Path,
    title: impl Into<String>,
) -> impl Fn(&mut OpenMusic, &mut Window, &mut Context<OpenMusic>) + 'static {
    let path = directory.join("menu-choice.txt");
    let title = title.into();
    move |_, _, _| {
        let _ = std::fs::write(&path, &title);
    }
}

fn sample(directory: &Path) -> Vec<(Group, MenuItem)> {
    let item = |title: &str| MenuItem::action(title, record(directory, title));
    vec![
        (
            Group::Library,
            item("Sample: Save to your Liked Songs")
                .symbol("heart")
                .key("l"),
        ),
        (Group::Queue, item("Sample: Add to queue").key("q")),
        (
            Group::Edit,
            MenuItem::submenu(
                "Sample: Add to playlist",
                vec![MenuItem::note("Loading playlists…")],
            )
            .symbol("plus")
            .search("Find a playlist")
            .id(SAMPLE_PLAYLISTS),
        ),
        (
            Group::Feedback,
            item("Sample: Hide in this playlist").enabled(false),
        ),
        (
            Group::Navigate,
            item("Sample: Go to song radio").subtitle("Spotify radio"),
        ),
        (
            Group::Info,
            item("Sample: View credits").checked(true).key("i"),
        ),
        (
            Group::Share,
            MenuItem::submenu(
                "Sample: Share",
                vec![
                    MenuItem::header("Links"),
                    item("Copy link to Song")
                        .key("c")
                        .alternate("Copy Spotify URI", record(directory, "Copy Spotify URI")),
                    MenuItem::Separator,
                    item("Embed track"),
                ],
            )
            .symbol("square.and.arrow.up"),
        ),
    ]
}

fn sample_playlists(app: &OpenMusic, directory: &Path) -> Vec<MenuItem> {
    app.library
        .playlists
        .iter()
        .take(40)
        .map(|playlist| MenuItem::action(playlist.name.clone(), record(directory, &playlist.name)))
        .collect()
}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let write = |name: &str, text: String| {
        let _ = std::fs::write(directory.join(name), text);
    };
    match (verb, words.as_slice()) {
        ("menu", ["sample", "on"]) => app.menus.sample = Some(directory.to_owned()),
        ("menu", ["sample", "off"]) => app.menus.sample = None,
        ("menu", [kind, uri, context @ ..]) if context.len() <= 1 => {
            let Some(target) = Target::parse(kind, uri, context.first().copied(), app) else {
                return true;
            };
            let items = app.menu_items(&target);
            let entries = lower(items, &mut Vec::new(), &mut HashMap::new());
            let mut lines = vec![format!("target {}", target.describe())];
            dump(&entries, 0, &mut lines);
            write("menu.txt", lines.join("\n"));
        }
        ("popup", ["dump"]) => {
            let tree = overlay::menu_tree().unwrap_or_else(|| "closed".into());
            write("popup.txt", format!("row={:?}\n{tree}", app.menus.row));
        }
        ("popup", ["menubar"]) => write(
            "popup.txt",
            overlay::menu_bar_tree().unwrap_or_else(|| "none".into()),
        ),
        ("popup", ["bar"]) => write(
            "popup.txt",
            overlay::bar_menu_tree().unwrap_or_else(|| "none".into()),
        ),
        ("popup", ["pick", path]) => {
            let picked = overlay::pick(path);
            write("popup.txt", format!("picked {path} {picked}"));
        }
        ("popup", ["close"]) => {
            let closed = overlay::close_menu();
            write("popup.txt", format!("closed {closed}"));
        }
        ("popup", ["filter", text @ ..]) => {
            overlay::filter_menu(&text.join(" "));
        }
        ("popup", ["fill"]) => {
            let items = sample_playlists(app, directory);
            app.update_submenu(SAMPLE_PLAYLISTS, items);
        }
        ("popup", [kind, uri, context @ ..]) if context.len() <= 1 => {
            if let Some(target) = Target::parse(kind, uri, context.first().copied(), app) {
                let size = window.viewport_size();
                app.open_menu(target, point(size.width / 2., size.height / 2.), None);
            }
        }
        ("rightclick", [x, y]) => {
            if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                overlay::right_click(x, y);
            }
        }
        ("press", [key]) => {
            if let Some(key) = overlay::key(key) {
                app.handle_control(Action::Press(key), window, cx);
            }
        }
        ("overlay", []) => write("overlay.txt", overlay::describe()),
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop() -> impl Fn(&mut OpenMusic, &mut Window, &mut Context<OpenMusic>) + 'static {
        |_, _, _| {}
    }

    fn titles(items: &[MenuItem]) -> Vec<String> {
        items
            .iter()
            .map(|item| match item {
                MenuItem::Action { title, .. } | MenuItem::Submenu { title, .. } => title.clone(),
                MenuItem::Note(text) | MenuItem::Header(text) => text.clone(),
                MenuItem::Separator => "--".into(),
            })
            .collect()
    }

    #[test]
    fn groups_sort_and_separate_sections() {
        let items = grouped(vec![
            (Group::Share, MenuItem::action("Share", noop())),
            (Group::Info, MenuItem::action("View credits", noop())),
            (Group::Library, MenuItem::action("Save", noop())),
            (Group::Navigate, MenuItem::action("Go to album", noop())),
            (Group::Queue, MenuItem::action("Add to queue", noop())),
            (Group::Library, MenuItem::action("Pin", noop())),
        ]);
        assert_eq!(
            titles(&items),
            [
                "Save",
                "Pin",
                "Add to queue",
                "--",
                "Go to album",
                "View credits",
                "--",
                "Share"
            ]
        );
        assert!(grouped(Vec::new()).is_empty());
    }

    #[test]
    fn lowering_numbers_items_alternates_and_submenus() {
        let mut runs = Vec::new();
        let mut submenus = HashMap::new();
        let entries = lower(
            vec![
                MenuItem::action("Copy link", noop())
                    .key("c")
                    .alternate("Copy URI", noop()),
                MenuItem::Separator,
                MenuItem::submenu(
                    "Add to playlist",
                    vec![MenuItem::action("Mix", noop()).checked(true)],
                )
                .search("Find a playlist")
                .id("playlists"),
            ],
            &mut runs,
            &mut submenus,
        );
        assert_eq!(runs.len(), 3);
        assert_eq!(submenus.get("playlists"), Some(&0));
        assert_eq!(
            entries,
            [
                Entry::Item {
                    title: "Copy link".into(),
                    id: 0,
                    enabled: true,
                    checked: false,
                    symbol: None,
                    key: Some("c"),
                    alternate: Some(("Copy URI".into(), 1)),
                    subtitle: None,
                },
                Entry::Separator,
                Entry::Submenu {
                    title: "Add to playlist".into(),
                    symbol: None,
                    entries: vec![Entry::Item {
                        title: "Mix".into(),
                        id: 2,
                        enabled: true,
                        checked: true,
                        symbol: None,
                        key: None,
                        alternate: None,
                        subtitle: None,
                    }],
                    search: Some("Find a playlist"),
                    id: Some(0),
                },
            ]
        );
        let mut lines = Vec::new();
        dump(&entries, 0, &mut lines);
        assert_eq!(
            lines,
            [
                "Copy link  c",
                "(⌥) Copy URI",
                "──",
                "Add to playlist ▸",
                "  [search “Find a playlist”]",
                "  Mix ✓",
            ]
        );
    }

    #[test]
    fn maps_uris_to_targets() {
        let cases = [
            ("spotify:album:1b5KESuOMQomF3wntoLAsk", "album"),
            ("spotify:artist:7G1GBhoKtEPnP86X2PvEYO", "artist"),
            ("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M", "playlist"),
            (
                "spotify:user:someone:playlist:37i9dQZF1DXcBWIGoYBM5M",
                "playlist",
            ),
            ("spotify:collection:tracks", "playlist"),
            ("spotify:user:someone:folder:0123456789abcdef", "folder"),
            ("spotify:episode:0123456789abcdefghijkl", "episode"),
            ("spotify:show:0123456789abcdefghijkl", "show"),
            ("spotify:user:someone", "user"),
        ];
        for (uri, kind) in cases {
            let target = Target::for_uri(uri).expect(uri);
            assert_eq!(target.uri(), uri);
            assert_eq!(target.describe(), format!("{kind} {uri}"));
        }
        assert_eq!(
            Target::for_uri("spotify:track:4uLU6hMCjMI75M1A2tKUQC"),
            None
        );
        assert_eq!(Target::for_uri("https://example.com"), None);
    }

    #[test]
    fn describes_track_targets() {
        let mut track = Track::placeholder("spotify:track:4uLU6hMCjMI75M1A2tKUQC");
        track.uid = Some("u1".into());
        let target = Target::track(
            track,
            Some("spotify:album:1b5KESuOMQomF3wntoLAsk".into()),
            Surface::Card,
        );
        assert_eq!(
            target.describe(),
            "track spotify:track:4uLU6hMCjMI75M1A2tKUQC uid=u1 context=spotify:album:1b5KESuOMQomF3wntoLAsk surface=card"
        );
    }
}
