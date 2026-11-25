use gpui::{Action, MenuItem};
use spotify_client::entity;

use crate::menus::{DockLabel, Next, PlayPause, PlayRecent, Previous, ToggleMiniPlayer};

pub use native::{items as native_items, strip_keys};

pub const RECENT_LIMIT: usize = 8;
pub const TITLE_LIMIT: usize = 60;
pub const RECENTLY_PLAYED: &str = "Recently Played";
pub const OPEN_MINI: &str = "Open Mini Player";
pub const CLOSE_MINI: &str = "Close Mini Player";
const INDENT: &str = "   ";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DockState {
    pub connected: bool,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub playing: bool,
    pub recents: Vec<(String, String)>,
    pub mini_open: bool,
}

pub fn cut(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= TITLE_LIMIT {
        return text.to_owned();
    }
    let kept: String = text.chars().take(TITLE_LIMIT - 1).collect();
    format!("{}…", kept.trim_end())
}

pub fn recent_name(uri: &str, name: &str) -> Option<String> {
    if !name.trim().is_empty() {
        Some(name.trim().to_owned())
    } else if entity::is_liked_songs(uri) {
        Some("Liked Songs".to_owned())
    } else {
        None
    }
}

pub fn menu(state: &DockState) -> Vec<MenuItem> {
    if !state.connected {
        return Vec::new();
    }
    let mut items = Vec::new();
    let section = |items: &mut Vec<MenuItem>| {
        if !items.is_empty() {
            items.push(MenuItem::separator());
        }
    };
    if let Some(title) = state.title.as_deref().filter(|title| !title.is_empty()) {
        items.push(MenuItem::action(cut(title), DockLabel));
        if let Some(artist) = state.artist.as_deref().filter(|artist| !artist.is_empty()) {
            items.push(MenuItem::action(cut(artist), DockLabel));
        }
        section(&mut items);
        let play = if state.playing { "Pause" } else { "Play" };
        items.push(MenuItem::action(play, PlayPause));
        items.push(MenuItem::action("Next", Next));
        items.push(MenuItem::action("Previous", Previous));
    }
    if !state.recents.is_empty() {
        section(&mut items);
        items.push(MenuItem::action(RECENTLY_PLAYED, DockLabel));
        for (uri, name) in state.recents.iter().take(RECENT_LIMIT) {
            items.push(MenuItem::action(
                format!("{INDENT}{}", cut(name)),
                PlayRecent { uri: uri.clone() },
            ));
        }
    }
    if state.title.is_some() || state.mini_open {
        section(&mut items);
        let title = if state.mini_open {
            CLOSE_MINI
        } else {
            OPEN_MINI
        };
        items.push(MenuItem::action(title, ToggleMiniPlayer));
    }
    items
}

pub fn describe(items: &[MenuItem], available: &mut dyn FnMut(&dyn Action) -> bool) -> Vec<String> {
    items
        .iter()
        .map(|item| match item {
            MenuItem::Separator => "──".to_owned(),
            MenuItem::Action { name, action, .. } => {
                let mut line = name.to_string();
                if let Some(recent) = action.as_any().downcast_ref::<PlayRecent>() {
                    line.push_str(&format!("  → {}", recent.uri));
                }
                if !available(action.as_ref()) {
                    line.push_str("  [x]");
                }
                line
            }
            MenuItem::Submenu(menu) => format!("{} ▸", menu.name),
            MenuItem::SystemMenu(menu) => format!("{} (system)", menu.name),
        })
        .collect()
}

#[cfg(target_os = "macos")]
mod native {
    use objc2::{MainThreadMarker, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{NSApplication, NSEventModifierFlags, NSMenu};
    use objc2_foundation::NSString;

    fn menu() -> Option<Retained<NSMenu>> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new()?);
        let delegate = app.delegate()?;
        let delegate: &AnyObject = delegate.as_ref();
        let application: &AnyObject = &app;
        unsafe { msg_send![delegate, applicationDockMenu: application] }
    }

    pub fn strip_keys() {
        let Some(menu) = menu() else {
            return;
        };
        let empty = NSString::from_str("");
        for item in (0..menu.numberOfItems()).filter_map(|index| menu.itemAtIndex(index)) {
            item.setKeyEquivalent(&empty);
            item.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
        }
    }

    pub fn items() -> Option<Vec<(String, bool)>> {
        let menu = menu()?;
        Some(
            (0..menu.numberOfItems())
                .filter_map(|index| menu.itemAtIndex(index))
                .map(|item| (item.title().to_string(), !item.keyEquivalent().is_empty()))
                .collect(),
        )
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub fn strip_keys() {}

    pub fn items() -> Option<Vec<(String, bool)>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(state: &DockState) -> Vec<String> {
        let mut all = |_: &dyn Action| true;
        describe(&menu(state), &mut all)
    }

    fn playing() -> DockState {
        DockState {
            connected: true,
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            playing: true,
            recents: vec![
                (
                    "spotify:album:1b5KESuOMQomF3wntoLAsk".into(),
                    "Album".into(),
                ),
                (
                    "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M".into(),
                    "Playlist".into(),
                ),
            ],
            mini_open: false,
        }
    }

    #[test]
    fn lists_the_track_controls_and_recents() {
        assert_eq!(
            titles(&playing()),
            [
                "Song",
                "Artist",
                "──",
                "Pause",
                "Next",
                "Previous",
                "──",
                "Recently Played",
                "   Album  → spotify:album:1b5KESuOMQomF3wntoLAsk",
                "   Playlist  → spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
                "──",
                "Open Mini Player",
            ]
        );
    }

    #[test]
    fn drops_blocks_that_have_nothing_to_show() {
        let idle = DockState {
            title: None,
            artist: None,
            playing: false,
            ..playing()
        };
        assert_eq!(
            titles(&idle),
            [
                "Recently Played",
                "   Album  → spotify:album:1b5KESuOMQomF3wntoLAsk",
                "   Playlist  → spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            ]
        );
        let paused = DockState {
            playing: false,
            recents: Vec::new(),
            mini_open: true,
            ..playing()
        };
        assert_eq!(
            titles(&paused),
            [
                "Song",
                "Artist",
                "──",
                "Play",
                "Next",
                "Previous",
                "──",
                "Close Mini Player"
            ]
        );
        let signed_out = DockState {
            connected: false,
            ..playing()
        };
        assert!(menu(&signed_out).is_empty());
    }

    #[test]
    fn labels_are_disabled_and_recents_capped() {
        let mut state = playing();
        state.recents = (0..12)
            .map(|index| {
                (
                    format!("spotify:album:{index:0>22}"),
                    format!("Album {index}"),
                )
            })
            .collect();
        let mut working = |action: &dyn Action| !action.partial_eq(&DockLabel);
        let lines = describe(&menu(&state), &mut working);
        assert_eq!(lines[0], "Song  [x]");
        assert_eq!(lines[1], "Artist  [x]");
        assert_eq!(lines[7], "Recently Played  [x]");
        assert_eq!(
            lines.iter().filter(|line| line.starts_with(INDENT)).count(),
            RECENT_LIMIT
        );
    }

    #[test]
    fn long_titles_are_cut() {
        let long = "a".repeat(80);
        let cut = cut(&long);
        assert_eq!(cut.chars().count(), TITLE_LIMIT);
        assert!(cut.ends_with('…'));
        assert_eq!(super::cut("  Short  "), "Short");
    }

    #[test]
    fn liked_songs_get_a_name() {
        assert_eq!(
            recent_name("spotify:user:someone:collection", ""),
            Some("Liked Songs".into())
        );
        assert_eq!(
            recent_name("spotify:album:1b5KESuOMQomF3wntoLAsk", " "),
            None
        );
        assert_eq!(
            recent_name("spotify:album:1b5KESuOMQomF3wntoLAsk", "Album"),
            Some("Album".into())
        );
    }
}
