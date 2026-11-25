use std::{path::Path, rc::Rc};

use gpui::{
    Action, AnyWindowHandle, App, Context, DummyKeyboardMapper, Focusable, KeyBinding,
    KeyBindingContextPredicate, Keystroke, Menu, MenuItem, OsAction, SystemMenuType, WeakEntity,
    Window, actions,
};
use gpui_component::input::{Copy, Cut, Paste, SelectAll};

use crate::{
    app::OpenMusic,
    areas::{account, audio, content, details, discovery, library, playback, playlists, system},
};

actions!(
    openmusic,
    [
        Quit,
        SignOut,
        About,
        OpenSettings,
        HideApp,
        HideOthers,
        ShowAll,
        FocusSearch,
        PlayPause,
        TogglePlayback,
        Next,
        Previous,
        SeekForward,
        SeekBackward,
        ToggleShuffle,
        CycleRepeat,
        VolumeUp,
        VolumeDown,
        ToggleLike,
        PlayOnThisMac,
        SleepAtEndOfTrack,
        StartJam,
        GoBack,
        GoForward,
        GoHome,
        GoNowPlaying,
        GoLyrics,
        ShowQueue,
        GoToLikedSongs,
        GoToRecents,
        ShowDevices,
        ToggleFriendActivity,
        NewPlaylist,
        NewFolder,
        ToggleFullScreen,
        CloseWindow,
        Minimize,
        Zoom,
        ShowMainWindow,
        ToggleMiniPlayer,
        BringAllToFront,
        ShowShortcuts,
        ToggleNotifications,
        ToggleMiniOnMinimize,
        DockLabel,
    ]
);

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = openmusic, no_json)]
pub struct PlayRecent {
    pub uri: String,
}

#[derive(Clone, PartialEq, Debug, gpui::Action)]
#[action(namespace = openmusic, no_json)]
pub struct SleepTimer {
    pub minutes: u32,
}

const SLEEP_MINUTES: [(u32, &str); 7] = [
    (0, "Off"),
    (5, "5 Minutes"),
    (10, "10 Minutes"),
    (15, "15 Minutes"),
    (30, "30 Minutes"),
    (45, "45 Minutes"),
    (60, "1 Hour"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Basic,
    Playback,
    Navigation,
    Window,
}

impl Category {
    pub const ALL: [Category; 4] = [
        Category::Basic,
        Category::Playback,
        Category::Navigation,
        Category::Window,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Category::Basic => "Basic",
            Category::Playback => "Playback",
            Category::Navigation => "Navigation",
            Category::Window => "Window",
        }
    }
}

pub struct Shortcut {
    pub keys: &'static str,
    pub action: fn() -> Box<dyn Action>,
    pub context: Option<&'static str>,
    pub category: Category,
    pub label: &'static str,
}

fn boxed<A: Action + Default>() -> Box<dyn Action> {
    Box::new(A::default())
}

const QUIET: Option<&str> = Some("!Input");

const fn shortcut(
    keys: &'static str,
    action: fn() -> Box<dyn Action>,
    context: Option<&'static str>,
    category: Category,
    label: &'static str,
) -> Shortcut {
    Shortcut {
        keys,
        action,
        context,
        category,
        label,
    }
}

use Category::{Basic, Navigation, Playback, Window as Windows};

pub const SHORTCUTS: &[Shortcut] = &[
    shortcut("cmd-l", boxed::<FocusSearch>, None, Basic, "Search"),
    shortcut("cmd-f", boxed::<FocusSearch>, QUIET, Basic, "Search"),
    shortcut("cmd-k", boxed::<FocusSearch>, None, Basic, "Search"),
    shortcut(
        "cmd-/",
        boxed::<ShowShortcuts>,
        None,
        Basic,
        "Keyboard shortcuts",
    ),
    shortcut(
        "?",
        boxed::<ShowShortcuts>,
        QUIET,
        Basic,
        "Keyboard shortcuts",
    ),
    shortcut("cmd-,", boxed::<OpenSettings>, None, Basic, "Settings"),
    shortcut("cmd-n", boxed::<NewPlaylist>, None, Basic, "New playlist"),
    shortcut(
        "shift-cmd-n",
        boxed::<NewFolder>,
        None,
        Basic,
        "New playlist folder",
    ),
    shortcut("cmd-h", boxed::<HideApp>, None, Basic, "Hide OpenMusic"),
    shortcut("alt-cmd-h", boxed::<HideOthers>, None, Basic, "Hide others"),
    shortcut("cmd-q", boxed::<Quit>, None, Basic, "Quit OpenMusic"),
    shortcut(
        "space",
        boxed::<TogglePlayback>,
        QUIET,
        Playback,
        "Play or pause",
    ),
    shortcut("cmd-right", boxed::<Next>, QUIET, Playback, "Next"),
    shortcut("cmd-left", boxed::<Previous>, QUIET, Playback, "Previous"),
    shortcut(
        "shift-cmd-right",
        boxed::<SeekForward>,
        QUIET,
        Playback,
        "Seek forward",
    ),
    shortcut(
        "shift-cmd-left",
        boxed::<SeekBackward>,
        QUIET,
        Playback,
        "Seek backward",
    ),
    shortcut("cmd-s", boxed::<ToggleShuffle>, None, Playback, "Shuffle"),
    shortcut("cmd-r", boxed::<CycleRepeat>, None, Playback, "Repeat"),
    shortcut("cmd-up", boxed::<VolumeUp>, QUIET, Playback, "Volume up"),
    shortcut(
        "cmd-down",
        boxed::<VolumeDown>,
        QUIET,
        Playback,
        "Volume down",
    ),
    shortcut(
        "alt-shift-b",
        boxed::<ToggleLike>,
        QUIET,
        Playback,
        "Save to Liked Songs",
    ),
    shortcut(
        "shift-cmd-j",
        boxed::<StartJam>,
        None,
        Playback,
        "Start a Jam",
    ),
    shortcut("alt-cmd-left", boxed::<GoBack>, None, Navigation, "Go back"),
    shortcut("cmd-[", boxed::<GoBack>, None, Navigation, "Go back"),
    shortcut("alt-left", boxed::<GoBack>, QUIET, Navigation, "Go back"),
    shortcut(
        "alt-cmd-right",
        boxed::<GoForward>,
        None,
        Navigation,
        "Go forward",
    ),
    shortcut("cmd-]", boxed::<GoForward>, None, Navigation, "Go forward"),
    shortcut(
        "alt-right",
        boxed::<GoForward>,
        QUIET,
        Navigation,
        "Go forward",
    ),
    shortcut("alt-shift-h", boxed::<GoHome>, QUIET, Navigation, "Home"),
    shortcut(
        "alt-shift-j",
        boxed::<GoNowPlaying>,
        QUIET,
        Navigation,
        "Now playing",
    ),
    shortcut(
        "alt-shift-q",
        boxed::<ShowQueue>,
        QUIET,
        Navigation,
        "Queue",
    ),
    shortcut(
        "alt-shift-s",
        boxed::<GoToLikedSongs>,
        QUIET,
        Navigation,
        "Liked Songs",
    ),
    shortcut(
        "alt-shift-y",
        boxed::<GoToRecents>,
        QUIET,
        Navigation,
        "Recently played",
    ),
    shortcut(
        "alt-shift-0",
        boxed::<library::state::ShowLibrary>,
        QUIET,
        Navigation,
        "Your Library",
    ),
    shortcut(
        "alt-shift-1",
        boxed::<library::state::ShowPlaylists>,
        QUIET,
        Navigation,
        "Your playlists",
    ),
    shortcut(
        "alt-shift-2",
        boxed::<library::state::ShowPodcasts>,
        QUIET,
        Navigation,
        "Your podcasts and shows",
    ),
    shortcut(
        "alt-shift-3",
        boxed::<library::state::ShowArtists>,
        QUIET,
        Navigation,
        "Your artists",
    ),
    shortcut(
        "alt-shift-4",
        boxed::<library::state::ShowAlbums>,
        QUIET,
        Navigation,
        "Your albums",
    ),
    shortcut(
        "alt-shift-5",
        boxed::<library::state::ShowAudiobooks>,
        QUIET,
        Navigation,
        "Your audiobooks",
    ),
    shortcut(
        "alt-shift-6",
        boxed::<library::state::ShowAuthors>,
        QUIET,
        Navigation,
        "Your authors",
    ),
    shortcut(
        "alt-shift-7",
        boxed::<library::state::ShowEvents>,
        QUIET,
        Navigation,
        "Your events",
    ),
    shortcut(
        "alt-shift-d",
        boxed::<ShowDevices>,
        QUIET,
        Navigation,
        "Devices",
    ),
    shortcut(
        "alt-shift-u",
        boxed::<ToggleFriendActivity>,
        QUIET,
        Navigation,
        "Friend activity",
    ),
    shortcut(
        "alt-cmd-1",
        boxed::<ShowMainWindow>,
        None,
        Windows,
        "Show main window",
    ),
    shortcut(
        "alt-cmd-m",
        boxed::<ToggleMiniPlayer>,
        None,
        Windows,
        "Mini player",
    ),
    shortcut(
        "ctrl-cmd-f",
        boxed::<ToggleFullScreen>,
        None,
        Windows,
        "Full screen",
    ),
    shortcut("cmd-m", boxed::<Minimize>, None, Windows, "Minimize"),
    shortcut("cmd-w", boxed::<CloseWindow>, None, Windows, "Close window"),
];

impl Shortcut {
    pub fn binding(&self) -> KeyBinding {
        let predicate = self.context.map(|context| {
            Rc::new(KeyBindingContextPredicate::parse(context).expect("valid key context"))
        });
        KeyBinding::load(
            self.keys,
            (self.action)(),
            predicate,
            false,
            None,
            &DummyKeyboardMapper,
        )
        .expect("valid key binding")
    }
}

fn key_bindings() -> Vec<KeyBinding> {
    SHORTCUTS.iter().map(Shortcut::binding).collect()
}

pub struct SheetRow {
    pub label: &'static str,
    pub keys: Vec<&'static str>,
    pub action: fn() -> Box<dyn Action>,
}

pub fn sheet() -> Vec<(Category, Vec<SheetRow>)> {
    Category::ALL
        .into_iter()
        .map(|category| {
            let mut rows: Vec<SheetRow> = Vec::new();
            for shortcut in SHORTCUTS
                .iter()
                .filter(|shortcut| shortcut.category == category)
            {
                match rows.iter_mut().find(|row| row.label == shortcut.label) {
                    Some(row) => row.keys.push(shortcut.keys),
                    None => rows.push(SheetRow {
                        label: shortcut.label,
                        keys: vec![shortcut.keys],
                        action: shortcut.action,
                    }),
                }
            }
            (category, rows)
        })
        .filter(|(_, rows)| !rows.is_empty())
        .collect()
}

pub fn glyphs(control: bool, alt: bool, shift: bool, command: bool, key: &str) -> String {
    let mut label = String::new();
    for (held, glyph) in [(control, '⌃'), (alt, '⌥'), (shift, '⇧'), (command, '⌘')] {
        if held {
            label.push(glyph);
        }
    }
    match key {
        "right" => label.push('→'),
        "left" => label.push('←'),
        "up" => label.push('↑'),
        "down" => label.push('↓'),
        "space" => label.push_str("Space"),
        "escape" => label.push('⎋'),
        "enter" => label.push('↩'),
        "backspace" => label.push('⌫'),
        "delete" => label.push('⌦'),
        key => label.push_str(&key.to_uppercase()),
    }
    label
}

pub fn key_label(keys: &str) -> String {
    match Keystroke::parse(keys) {
        Ok(stroke) => glyphs(
            stroke.modifiers.control,
            stroke.modifiers.alt,
            stroke.modifiers.shift,
            stroke.modifiers.platform,
            &stroke.key,
        ),
        Err(_) => keys.to_owned(),
    }
}

fn menu_bar() -> Vec<Menu> {
    vec![
        Menu {
            name: "OpenMusic".into(),
            items: vec![
                MenuItem::action("About OpenMusic", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::action("Notify on Song Change", ToggleNotifications),
                MenuItem::action("Show Mini Player When Minimized", ToggleMiniOnMinimize),
                MenuItem::separator(),
                MenuItem::action("Sign Out of Spotify", SignOut),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide OpenMusic", HideApp),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit OpenMusic", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Playlist", NewPlaylist),
                MenuItem::action("New Playlist Folder", NewFolder),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Search", FocusSearch),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Go Back", GoBack),
                MenuItem::action("Go Forward", GoForward),
                MenuItem::separator(),
                MenuItem::action("Home", GoHome),
                MenuItem::action("Now Playing", GoNowPlaying),
                MenuItem::action("Lyrics", GoLyrics),
                MenuItem::action("Queue", ShowQueue),
                MenuItem::action("Liked Songs", GoToLikedSongs),
                MenuItem::action("Recently Played", GoToRecents),
                MenuItem::action("Devices", ShowDevices),
                MenuItem::action("Friend Activity", ToggleFriendActivity),
                MenuItem::separator(),
                MenuItem::action("Enter Full Screen", ToggleFullScreen),
            ],
        },
        Menu {
            name: "Playback".into(),
            items: vec![
                MenuItem::action("Play", PlayPause),
                MenuItem::separator(),
                MenuItem::action("Next", Next),
                MenuItem::action("Previous", Previous),
                MenuItem::action("Seek Forward", SeekForward),
                MenuItem::action("Seek Backward", SeekBackward),
                MenuItem::separator(),
                MenuItem::action("Shuffle", ToggleShuffle),
                MenuItem::action("Repeat", CycleRepeat),
                MenuItem::separator(),
                MenuItem::action("Volume Up", VolumeUp),
                MenuItem::action("Volume Down", VolumeDown),
                MenuItem::separator(),
                MenuItem::action("Save to Liked Songs", ToggleLike),
                MenuItem::action("Play on This Mac", PlayOnThisMac),
                MenuItem::submenu(Menu {
                    name: "Sleep Timer".into(),
                    items: SLEEP_MINUTES
                        .into_iter()
                        .map(|(minutes, title)| MenuItem::action(title, SleepTimer { minutes }))
                        .chain([MenuItem::action("End of Track", SleepAtEndOfTrack)])
                        .collect(),
                }),
                MenuItem::separator(),
                MenuItem::action("Start a Jam", StartJam),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Close", CloseWindow),
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
                MenuItem::separator(),
                MenuItem::action("Mini Player", ToggleMiniPlayer),
                MenuItem::action("OpenMusic", ShowMainWindow),
                MenuItem::separator(),
                MenuItem::action("Bring All to Front", BringAllToFront),
            ],
        },
        Menu {
            name: "Help".into(),
            items: vec![MenuItem::action("Keyboard Shortcuts", ShowShortcuts)],
        },
    ]
}

pub fn install(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys(key_bindings());
    cx.set_menus(menu_bar());
}

#[derive(Clone)]
pub struct Handle {
    app: WeakEntity<OpenMusic>,
    window: AnyWindowHandle,
}

fn repeats(action: &dyn Action) -> bool {
    let action = action.as_any();
    action.is::<SeekForward>()
        || action.is::<SeekBackward>()
        || action.is::<VolumeUp>()
        || action.is::<VolumeDown>()
}

#[cfg(target_os = "macos")]
fn key_held() -> bool {
    crate::appkit::key_held()
}

#[cfg(not(target_os = "macos"))]
fn key_held() -> bool {
    false
}

pub fn route<A: Action>(
    cx: &mut App,
    handle: &Handle,
    run: impl Fn(&A, &mut OpenMusic, &mut Window, &mut Context<OpenMusic>) + 'static,
) {
    let handle = handle.clone();
    let run = Rc::new(run);
    cx.on_action(move |action: &A, cx| {
        if key_held() && !repeats(action) {
            return;
        }
        let (handle, run, action) = (handle.clone(), run.clone(), action.boxed_clone());
        cx.defer(move |cx| {
            let Some(action) = action.as_any().downcast_ref::<A>() else {
                return;
            };
            let _ = handle.window.update(cx, |_, window, cx| {
                let _ = handle
                    .app
                    .update(cx, |this, cx| run(action, this, window, cx));
            });
        });
    });
}

pub fn route_visible<A: Action>(
    cx: &mut App,
    handle: &Handle,
    run: impl Fn(&A, &mut OpenMusic, &mut Window, &mut Context<OpenMusic>) + 'static,
) {
    route(cx, handle, move |action: &A, app, window, cx| {
        app.reveal_main_window(window, cx);
        run(action, app, window, cx);
    });
}

pub fn connect(app: WeakEntity<OpenMusic>, window: AnyWindowHandle, cx: &mut App) {
    let handle = Handle { app, window };
    account::state::register_actions(cx, &handle);
    audio::state::register_actions(cx, &handle);
    content::state::register_actions(cx, &handle);
    details::state::register_actions(cx, &handle);
    discovery::state::register_actions(cx, &handle);
    library::state::register_actions(cx, &handle);
    playback::state::register_actions(cx, &handle);
    playlists::state::register_actions(cx, &handle);
    system::state::register_actions(cx, &handle);
}

fn build(name: &str, argument: &str, cx: &App) -> Option<Box<dyn Action>> {
    match name {
        "PlayRecent" if !argument.is_empty() => Some(Box::new(PlayRecent {
            uri: argument.to_owned(),
        })),
        "SleepTimer" => Some(Box::new(SleepTimer {
            minutes: argument.parse().ok()?,
        })),
        name if name.contains("::") => cx.build_action(name, None).ok(),
        name => cx.build_action(&format!("openmusic::{name}"), None).ok(),
    }
}

pub fn debug(
    app: &OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    let (file, result) = match verb {
        "action" => {
            let (name, argument) = rest.split_once(' ').unwrap_or((rest, ""));
            let result = match build(name, argument.trim(), cx) {
                Some(action) => {
                    let available = cx.is_action_available(action.as_ref());
                    window.dispatch_action(action, cx);
                    format!("{name} available={available}")
                }
                None => format!("{name} unknown"),
            };
            ("action.txt", result)
        }
        "focus" => {
            let search = app.input.read(cx).focus_handle(cx).is_focused(window);
            let root = app.focus.is_focused(window);
            ("focus.txt", format!("search={search} root={root}"))
        }
        _ => return false,
    };
    let _ = std::fs::write(directory.join(file), result);
    true
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn every_shortcut_binds_once() {
        let mut seen = HashSet::new();
        for shortcut in SHORTCUTS {
            let binding = shortcut.binding();
            assert_eq!(binding.keystrokes().len(), 1, "{}", shortcut.keys);
            assert!(
                seen.insert((shortcut.keys, shortcut.context)),
                "{} is bound twice",
                shortcut.keys
            );
        }
    }

    #[test]
    fn space_never_reaches_the_menu() {
        let space = SHORTCUTS
            .iter()
            .find(|shortcut| shortcut.keys == "space")
            .unwrap();
        assert!((space.action)().partial_eq(&TogglePlayback));
        assert_eq!(space.context, Some("!Input"));
        assert!(
            !SHORTCUTS
                .iter()
                .any(|shortcut| (shortcut.action)().partial_eq(&PlayPause))
        );
    }

    #[test]
    fn text_fields_keep_their_caret_keys() {
        use gpui::{KeyContext, Keymap};
        use gpui_component::input::{
            MoveEnd, MoveHome, MoveToEnd, MoveToStart, Search, SelectToEndOfLine,
            SelectToStartOfLine,
        };

        let caret: Vec<(&str, Box<dyn Action>)> = vec![
            ("cmd-right", Box::new(MoveEnd)),
            ("cmd-left", Box::new(MoveHome)),
            ("shift-cmd-right", Box::new(SelectToEndOfLine)),
            ("shift-cmd-left", Box::new(SelectToStartOfLine)),
            ("cmd-up", Box::new(MoveToStart)),
            ("cmd-down", Box::new(MoveToEnd)),
            ("cmd-f", Box::new(Search)),
        ];
        let mut bindings: Vec<KeyBinding> = caret
            .iter()
            .map(|(keys, action)| {
                let input = KeyBindingContextPredicate::parse("Input").unwrap();
                KeyBinding::load(
                    keys,
                    action.boxed_clone(),
                    Some(Rc::new(input)),
                    false,
                    None,
                    &DummyKeyboardMapper,
                )
                .unwrap()
            })
            .collect();
        bindings.extend(key_bindings());
        let keymap = Keymap::new(bindings);
        let browsing = [KeyContext::parse("OpenMusic").unwrap()];
        let typing = [browsing[0].clone(), KeyContext::parse("Input").unwrap()];
        for (keys, action) in &caret {
            let stroke = Keystroke::parse(keys).unwrap();
            let (found, _) = keymap.bindings_for_input(std::slice::from_ref(&stroke), &typing);
            assert!(found[0].action().partial_eq(action.as_ref()), "{keys}");
            let (found, _) = keymap.bindings_for_input(&[stroke], &browsing);
            assert!(!found.is_empty(), "{keys}");
            assert!(!found[0].action().partial_eq(action.as_ref()), "{keys}");
        }
    }

    #[test]
    fn only_seek_and_volume_follow_a_held_key() {
        let repeating: Vec<&str> = SHORTCUTS
            .iter()
            .filter(|shortcut| repeats((shortcut.action)().as_ref()))
            .map(|shortcut| shortcut.keys)
            .collect();
        assert_eq!(
            repeating,
            ["shift-cmd-right", "shift-cmd-left", "cmd-up", "cmd-down"]
        );
    }

    #[test]
    fn question_mark_opens_the_sheet() {
        let typed = Keystroke {
            modifiers: gpui::Modifiers::default(),
            key: "?".into(),
            key_char: Some("?".into()),
        };
        let matching: Vec<&Shortcut> = SHORTCUTS
            .iter()
            .filter(|shortcut| {
                shortcut
                    .binding()
                    .match_keystrokes(std::slice::from_ref(&typed))
                    == Some(false)
            })
            .collect();
        assert_eq!(matching.len(), 1);
        assert!((matching[0].action)().partial_eq(&ShowShortcuts));
        assert_eq!(matching[0].context, Some("!Input"));
    }

    #[test]
    fn menu_keys_come_first() {
        let first = |action: &dyn Action| {
            SHORTCUTS
                .iter()
                .find(|shortcut| (shortcut.action)().partial_eq(action))
                .map(|shortcut| shortcut.keys)
        };
        assert_eq!(first(&FocusSearch), Some("cmd-l"));
        assert_eq!(first(&GoBack), Some("alt-cmd-left"));
        assert_eq!(first(&GoForward), Some("alt-cmd-right"));
        assert_eq!(first(&ShowShortcuts), Some("cmd-/"));
    }

    #[test]
    fn sheet_groups_keys_by_row() {
        let sheet = sheet();
        let categories: Vec<Category> = sheet.iter().map(|(category, _)| *category).collect();
        assert_eq!(categories, Category::ALL);
        let row = |label: &str| {
            sheet
                .iter()
                .flat_map(|(_, rows)| rows)
                .find(|row| row.label == label)
                .unwrap()
        };
        assert_eq!(row("Search").keys, ["cmd-l", "cmd-f", "cmd-k"]);
        assert_eq!(row("Go back").keys, ["alt-cmd-left", "cmd-[", "alt-left"]);
        assert_eq!(row("Keyboard shortcuts").keys, ["cmd-/", "?"]);
        assert_eq!(row("Your Library").keys, ["alt-shift-0"]);
        assert_eq!(row("Your events").keys, ["alt-shift-7"]);
        let total: usize = sheet
            .iter()
            .flat_map(|(_, rows)| rows)
            .map(|row| row.keys.len())
            .sum();
        assert_eq!(total, SHORTCUTS.len());
    }

    #[test]
    fn labels_use_mac_glyphs() {
        assert_eq!(key_label("shift-cmd-right"), "⇧⌘→");
        assert_eq!(key_label("cmd-left"), "⌘←");
        assert_eq!(key_label("alt-shift-b"), "⌥⇧B");
        assert_eq!(key_label("ctrl-cmd-f"), "⌃⌘F");
        assert_eq!(key_label("alt-cmd-1"), "⌥⌘1");
        assert_eq!(key_label("cmd-,"), "⌘,");
        assert_eq!(key_label("space"), "Space");
        assert_eq!(key_label("?"), "?");
        assert_eq!(key_label("cmd-/"), "⌘/");
        assert_eq!(key_label("cmd-up"), "⌘↑");
    }
}
