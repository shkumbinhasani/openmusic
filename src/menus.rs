use std::{path::Path, rc::Rc};

use gpui::{
    Action, AnyWindowHandle, App, Context, Focusable, KeyBinding, Menu, MenuItem, OsAction,
    SystemMenuType, WeakEntity, Window, actions,
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

fn key_bindings() -> Vec<KeyBinding> {
    let quiet = Some("!Input");
    vec![
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-h", HideApp, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-l", FocusSearch, None),
        KeyBinding::new("cmd-f", FocusSearch, None),
        KeyBinding::new("cmd-k", FocusSearch, None),
        KeyBinding::new("space", TogglePlayback, quiet),
        KeyBinding::new("cmd-right", Next, None),
        KeyBinding::new("cmd-left", Previous, None),
        KeyBinding::new("shift-cmd-right", SeekForward, None),
        KeyBinding::new("shift-cmd-left", SeekBackward, None),
        KeyBinding::new("cmd-s", ToggleShuffle, None),
        KeyBinding::new("cmd-r", CycleRepeat, None),
        KeyBinding::new("cmd-up", VolumeUp, None),
        KeyBinding::new("cmd-down", VolumeDown, None),
        KeyBinding::new("alt-shift-b", ToggleLike, quiet),
        KeyBinding::new("shift-cmd-j", StartJam, None),
        KeyBinding::new("alt-cmd-left", GoBack, None),
        KeyBinding::new("cmd-[", GoBack, None),
        KeyBinding::new("alt-left", GoBack, quiet),
        KeyBinding::new("alt-cmd-right", GoForward, None),
        KeyBinding::new("cmd-]", GoForward, None),
        KeyBinding::new("alt-right", GoForward, quiet),
        KeyBinding::new("alt-shift-h", GoHome, quiet),
        KeyBinding::new("alt-shift-j", GoNowPlaying, quiet),
        KeyBinding::new("alt-shift-q", ShowQueue, quiet),
        KeyBinding::new("alt-shift-s", GoToLikedSongs, quiet),
        KeyBinding::new("alt-shift-y", GoToRecents, quiet),
        KeyBinding::new("alt-shift-d", ShowDevices, quiet),
        KeyBinding::new("alt-shift-u", ToggleFriendActivity, quiet),
        KeyBinding::new("cmd-n", NewPlaylist, None),
        KeyBinding::new("shift-cmd-n", NewFolder, None),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("alt-cmd-1", ShowMainWindow, None),
        KeyBinding::new("alt-cmd-m", ToggleMiniPlayer, None),
        KeyBinding::new("cmd-/", ShowShortcuts, None),
        KeyBinding::new("shift-/", ShowShortcuts, quiet),
    ]
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

pub fn route<A: Action>(
    cx: &mut App,
    handle: &Handle,
    run: impl Fn(&A, &mut OpenMusic, &mut Window, &mut Context<OpenMusic>) + 'static,
) {
    let handle = handle.clone();
    let run = Rc::new(run);
    cx.on_action(move |action: &A, cx| {
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
