use std::{collections::HashSet, path::Path};

use gpui::{App, Context, Window};

use super::worker::Event;
use crate::{
    app::{OpenMusic, Page},
    menus::Handle,
    ui::tracks::{RowDrag, RowDrop},
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
    _app: &mut OpenMusic,
    _verb: &str,
    _rest: &str,
    _directory: &Path,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
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
