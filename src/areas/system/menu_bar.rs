use gpui::{Action, OwnedMenuItem};
use spotify_client::connect::{Repeat, Shuffle};

pub use native::{about, arrange_in_front, items, sync};

pub const PLAYBACK: &str = "Playback";
pub const PLAY: &str = "Play";
pub const PAUSE: &str = "Pause";
pub const SHUFFLE: &str = "Shuffle";
pub const REPEAT: &str = "Repeat";
pub const SAVE: &str = "Save to Liked Songs";
pub const REMOVE: &str = "Remove from Liked Songs";
pub const SAVE_EPISODE: &str = "Save to Your Episodes";
pub const REMOVE_EPISODE: &str = "Remove from Your Episodes";
pub const LIKE_TITLES: [&str; 4] = [SAVE, REMOVE, SAVE_EPISODE, REMOVE_EPISODE];
pub const NOTIFY: &str = "Notify on Song Change";
pub const MINI_ON_MINIMIZE: &str = "Show Mini Player When Minimized";
pub const WINDOW: &str = "Window";
pub const MINI_PLAYER: &str = "Mini Player";
pub const VIEW: &str = "View";
pub const ENTER_FULL_SCREEN: &str = "Enter Full Screen";
pub const EXIT_FULL_SCREEN: &str = "Exit Full Screen";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Off,
    On,
    Mixed,
}

impl Check {
    fn glyph(self) -> &'static str {
        match self {
            Check::Off => "",
            Check::On => "  ✓",
            Check::Mixed => "  –",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    pub playing: bool,
    pub shuffle: Shuffle,
    pub repeat: Repeat,
    pub saved: Option<bool>,
    pub episode: bool,
    pub notify: bool,
    pub mini_on_minimize: bool,
    pub mini_open: bool,
    pub full_screen: bool,
}

impl MenuState {
    pub fn play_title(&self) -> &'static str {
        if self.playing { PAUSE } else { PLAY }
    }

    pub fn full_screen_title(&self) -> &'static str {
        if self.full_screen {
            EXIT_FULL_SCREEN
        } else {
            ENTER_FULL_SCREEN
        }
    }

    pub fn like_title(&self) -> &'static str {
        match (self.episode, self.saved == Some(true)) {
            (false, false) => SAVE,
            (false, true) => REMOVE,
            (true, false) => SAVE_EPISODE,
            (true, true) => REMOVE_EPISODE,
        }
    }

    pub fn shuffle_check(&self) -> Check {
        match self.shuffle {
            Shuffle::Off => Check::Off,
            Shuffle::On | Shuffle::Smart => Check::On,
        }
    }

    pub fn repeat_check(&self) -> Check {
        match self.repeat {
            Repeat::Off => Check::Off,
            Repeat::Context => Check::On,
            Repeat::Track => Check::Mixed,
        }
    }
}

pub fn toggle(on: bool) -> Check {
    if on { Check::On } else { Check::Off }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeItem {
    pub title: String,
    pub key: String,
    pub check: Option<Check>,
    pub separator: bool,
    pub children: Vec<NativeItem>,
}

pub fn describe(
    native: &[NativeItem],
    owned: &[OwnedMenuItem],
    depth: usize,
    available: &mut dyn FnMut(&dyn Action) -> bool,
    lines: &mut Vec<String>,
) {
    for (index, item) in native.iter().enumerate() {
        let indent = "  ".repeat(depth);
        if item.separator {
            lines.push(format!("{indent}──"));
            continue;
        }
        let mut line = format!("{indent}{}", item.title);
        if !item.key.is_empty() {
            line.push_str("  ");
            line.push_str(&item.key);
        }
        line.push_str(item.check.unwrap_or(Check::Off).glyph());
        match owned.get(index) {
            Some(OwnedMenuItem::Action { action, .. }) => {
                if !available(action.as_ref()) {
                    line.push_str("  [x]");
                }
            }
            Some(OwnedMenuItem::Submenu(_) | OwnedMenuItem::SystemMenu(_)) => {}
            Some(OwnedMenuItem::Separator) | None => line.push_str("  (system)"),
        }
        lines.push(line);
        let children = match owned.get(index) {
            Some(OwnedMenuItem::Submenu(menu)) => menu.items.as_slice(),
            _ => &[],
        };
        describe(&item.children, children, depth + 1, available, lines);
    }
}

pub fn native_key(key: &str, control: bool, alt: bool, shift: bool, command: bool) -> String {
    let key = match key {
        "\u{F700}" => "up",
        "\u{F701}" => "down",
        "\u{F702}" => "left",
        "\u{F703}" => "right",
        " " => "space",
        key => key,
    };
    crate::menus::glyphs(control, alt, shift, command, key)
}

#[cfg(target_os = "macos")]
mod native {
    use objc2::{MainThreadMarker, rc::Retained, sel};
    use objc2_app_kit::{
        NSApplication, NSControlStateValueMixed, NSControlStateValueOff, NSControlStateValueOn,
        NSEventModifierFlags, NSMenu,
    };
    use objc2_foundation::NSString;

    use super::{
        Check, ENTER_FULL_SCREEN, EXIT_FULL_SCREEN, LIKE_TITLES, MINI_ON_MINIMIZE, MINI_PLAYER,
        MenuState, NOTIFY, NativeItem, PLAYBACK, REPEAT, SHUFFLE, VIEW, WINDOW, native_key, toggle,
    };

    fn main_menu() -> Option<Retained<NSMenu>> {
        NSApplication::sharedApplication(MainThreadMarker::new()?).mainMenu()
    }

    fn find(menu: &NSMenu, titles: &[&str]) -> Option<Retained<objc2_app_kit::NSMenuItem>> {
        titles
            .iter()
            .find_map(|title| menu.itemWithTitle(&NSString::from_str(title)))
    }

    fn mark(menu: &NSMenu, title: &str, check: Check) {
        if let Some(item) = find(menu, &[title]) {
            item.setState(match check {
                Check::Off => NSControlStateValueOff,
                Check::On => NSControlStateValueOn,
                Check::Mixed => NSControlStateValueMixed,
            });
        }
    }

    fn retitle(menu: &NSMenu, titles: &[&str], title: &str) {
        if let Some(item) = find(menu, titles)
            && item.title().to_string() != title
        {
            item.setTitle(&NSString::from_str(title));
        }
    }

    fn retitle_full_screen(view: &NSMenu, title: &str) {
        let ours = (0..view.numberOfItems())
            .filter_map(|index| view.itemAtIndex(index))
            .find(|item| {
                let current = item.title().to_string();
                (current == ENTER_FULL_SCREEN || current == EXIT_FULL_SCREEN)
                    && item.action() != Some(sel!(toggleFullScreen:))
            });
        if let Some(item) = ours
            && item.title().to_string() != title
        {
            item.setTitle(&NSString::from_str(title));
        }
    }

    pub fn sync(state: &MenuState) {
        let Some(main) = main_menu() else {
            return;
        };
        if let Some(app) = main.itemAtIndex(0).and_then(|item| item.submenu()) {
            mark(&app, NOTIFY, toggle(state.notify));
            mark(&app, MINI_ON_MINIMIZE, toggle(state.mini_on_minimize));
        }
        if let Some(view) = find(&main, &[VIEW]).and_then(|item| item.submenu()) {
            retitle_full_screen(&view, state.full_screen_title());
        }
        if let Some(window) = find(&main, &[WINDOW]).and_then(|item| item.submenu()) {
            mark(&window, MINI_PLAYER, toggle(state.mini_open));
        }
        let Some(playback) = find(&main, &[PLAYBACK]).and_then(|item| item.submenu()) else {
            return;
        };
        if let Some(item) = playback.itemAtIndex(0) {
            let title = state.play_title();
            if item.title().to_string() != title {
                item.setTitle(&NSString::from_str(title));
            }
        }
        mark(&playback, SHUFFLE, state.shuffle_check());
        mark(&playback, REPEAT, state.repeat_check());
        retitle(&playback, &LIKE_TITLES, state.like_title());
    }

    fn read(menu: &NSMenu) -> Vec<NativeItem> {
        (0..menu.numberOfItems())
            .filter_map(|index| menu.itemAtIndex(index))
            .map(|item| {
                let mask = item.keyEquivalentModifierMask();
                let key = item.keyEquivalent().to_string();
                let state = item.state();
                NativeItem {
                    title: item.title().to_string(),
                    key: if key.is_empty() {
                        String::new()
                    } else {
                        native_key(
                            &key,
                            mask.contains(NSEventModifierFlags::Control),
                            mask.contains(NSEventModifierFlags::Option),
                            mask.contains(NSEventModifierFlags::Shift),
                            mask.contains(NSEventModifierFlags::Command),
                        )
                    },
                    check: Some(if state == NSControlStateValueOn {
                        Check::On
                    } else if state == NSControlStateValueMixed {
                        Check::Mixed
                    } else {
                        Check::Off
                    }),
                    separator: item.isSeparatorItem(),
                    children: item.submenu().map(|menu| read(&menu)).unwrap_or_default(),
                }
            })
            .collect()
    }

    pub fn items() -> Vec<NativeItem> {
        main_menu().map(|menu| read(&menu)).unwrap_or_default()
    }

    pub fn about() {
        if let Some(mtm) = MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm).orderFrontStandardAboutPanel(None);
        }
    }

    pub fn arrange_in_front() {
        if let Some(mtm) = MainThreadMarker::new() {
            NSApplication::sharedApplication(mtm).arrangeInFront(None);
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::{MenuState, NativeItem};

    pub fn sync(_: &MenuState) {}

    pub fn items() -> Vec<NativeItem> {
        Vec::new()
    }

    pub fn about() {}

    pub fn arrange_in_front() {}
}

#[cfg(test)]
mod tests {
    use gpui::{Menu, MenuItem};

    use super::*;
    use crate::menus::{Next, ToggleShuffle};

    fn state() -> MenuState {
        MenuState {
            playing: false,
            shuffle: Shuffle::Off,
            repeat: Repeat::Off,
            saved: None,
            episode: false,
            notify: false,
            mini_on_minimize: false,
            mini_open: false,
            full_screen: false,
        }
    }

    #[test]
    fn play_item_swaps_its_title() {
        assert_eq!(state().play_title(), "Play");
        let playing = MenuState {
            playing: true,
            ..state()
        };
        assert_eq!(playing.play_title(), "Pause");
    }

    #[test]
    fn full_screen_item_swaps_its_title() {
        assert_eq!(state().full_screen_title(), "Enter Full Screen");
        let full = MenuState {
            full_screen: true,
            ..state()
        };
        assert_eq!(full.full_screen_title(), "Exit Full Screen");
    }

    #[test]
    fn like_item_reads_the_saved_state() {
        assert_eq!(state().like_title(), "Save to Liked Songs");
        for (saved, title) in [
            (Some(true), "Remove from Liked Songs"),
            (Some(false), "Save to Liked Songs"),
            (None, "Save to Liked Songs"),
        ] {
            assert_eq!(MenuState { saved, ..state() }.like_title(), title);
        }
        let episode = MenuState {
            episode: true,
            ..state()
        };
        assert_eq!(episode.like_title(), "Save to Your Episodes");
        let saved = MenuState {
            saved: Some(true),
            ..episode
        };
        assert_eq!(saved.like_title(), "Remove from Your Episodes");
    }

    #[test]
    fn modes_map_to_checkmarks() {
        let checks = |shuffle, repeat| {
            let state = MenuState {
                shuffle,
                repeat,
                ..state()
            };
            (state.shuffle_check(), state.repeat_check())
        };
        assert_eq!(checks(Shuffle::Off, Repeat::Off), (Check::Off, Check::Off));
        assert_eq!(checks(Shuffle::On, Repeat::Context), (Check::On, Check::On));
        assert_eq!(
            checks(Shuffle::Smart, Repeat::Track),
            (Check::On, Check::Mixed)
        );
        assert_eq!(toggle(true), Check::On);
        assert_eq!(toggle(false), Check::Off);
    }

    #[test]
    fn native_keys_read_as_glyphs() {
        assert_eq!(native_key("\u{F703}", false, false, true, true), "⇧⌘→");
        assert_eq!(native_key("\u{F700}", false, false, false, true), "⌘↑");
        assert_eq!(native_key("s", false, false, false, true), "⌘S");
        assert_eq!(native_key("1", false, true, false, true), "⌥⌘1");
    }

    #[test]
    fn dump_marks_disabled_and_checked_items() {
        let owned = Menu {
            name: "Playback".into(),
            items: vec![
                MenuItem::action("Next", Next),
                MenuItem::separator(),
                MenuItem::action("Shuffle", ToggleShuffle),
            ],
        }
        .owned();
        let native = vec![
            NativeItem {
                title: "Next".into(),
                key: "⌘→".into(),
                check: Some(Check::Off),
                ..NativeItem::default()
            },
            NativeItem {
                separator: true,
                ..NativeItem::default()
            },
            NativeItem {
                title: "Shuffle".into(),
                key: "⌘S".into(),
                check: Some(Check::On),
                ..NativeItem::default()
            },
            NativeItem {
                title: "Start Dictation…".into(),
                ..NativeItem::default()
            },
        ];
        let mut lines = Vec::new();
        let mut available = |action: &dyn Action| action.partial_eq(&Next);
        describe(&native, &owned.items, 1, &mut available, &mut lines);
        assert_eq!(
            lines,
            [
                "  Next  ⌘→",
                "  ──",
                "  Shuffle  ⌘S  ✓  [x]",
                "  Start Dictation…  (system)",
            ]
        );
    }
}
