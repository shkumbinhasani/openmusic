use std::{collections::HashSet, path::Path};

use gpui::{App, Context, Window};

use spotify_client::share;

use super::{menu, worker::Event};
use crate::{
    app::{OpenMusic, Page},
    menus::Handle,
    ui::{
        menu::Surface,
        tracks::{RowDrag, RowDrop},
    },
};

#[derive(Default)]
pub struct State {}

impl State {
    pub fn covers(&self, _urls: &mut HashSet<String>) {}
}

pub fn load(_app: &mut OpenMusic, _page: &Page) {}

pub fn on_event(
    _app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {}
}

pub fn connected(_app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {}

pub fn signed_out(_app: &mut OpenMusic) {}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    let words: Vec<&str> = rest.split_whitespace().collect();
    let write = |name: &str, text: String| {
        let _ = std::fs::write(directory.join(name), text);
    };
    match (verb, words.as_slice()) {
        ("barmenu", []) => {
            let lines = match app.now_playing_target(Surface::NowPlaying) {
                Some(target) => {
                    let mut lines = vec![format!("target {}", target.describe())];
                    menu::describe(&app.menu_items(&target), 0, &mut lines);
                    lines
                }
                None => vec!["none".to_owned()],
            };
            write("barmenu.txt", lines.join("\n"));
        }
        ("share", ["link", uri]) => write(
            "share.txt",
            share::link(uri, &share::share_id()).unwrap_or_else(|| "none".into()),
        ),
        _ => return false,
    }
    true
}

pub fn state_fragment(_app: &OpenMusic) -> String {
    String::new()
}

pub fn register_actions(_cx: &mut App, _handle: &Handle) {}

pub fn on_press(
    _app: &mut OpenMusic,
    _key: &'static str,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
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

pub fn tick(_app: &mut OpenMusic, _cx: &mut Context<OpenMusic>) {}
