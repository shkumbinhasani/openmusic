use gpui::{App, Entity, Menu, MenuItem, OsAction, Window, actions};
use gpui_component::input::{Copy, Cut, Paste, SelectAll};

use crate::{app::OpenMusic, worker::Command};

actions!(openmusic, [Quit, FocusSearch, SignOut]);

pub fn install(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.bind_keys([
        gpui::KeyBinding::new("cmd-q", Quit, None),
        gpui::KeyBinding::new("cmd-f", FocusSearch, None),
    ]);
    cx.set_menus(vec![
        Menu {
            name: "OpenMusic".into(),
            items: vec![
                MenuItem::action("Sign Out of Spotify", SignOut),
                MenuItem::separator(),
                MenuItem::action("Quit OpenMusic", Quit),
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
                MenuItem::action("Find Music", FocusSearch),
            ],
        },
    ]);
}

pub fn connect(player: &Entity<OpenMusic>, window: &Window, cx: &mut App) {
    let handle = window.window_handle();
    let target = player.downgrade();
    cx.on_action(move |_: &FocusSearch, cx| {
        let target = target.clone();
        let _ = handle.update(cx, |_, window, cx| {
            let _ = target.update(cx, |this, cx| this.focus_search(window, cx));
        });
    });
    let target = player.downgrade();
    cx.on_action(move |_: &SignOut, cx| {
        let _ = target.update(cx, |this, _| this.send(Command::SignOut));
    });
}
