use std::path::Path;

use gpui::{App, Context, Window};
use serde::{Deserialize, Serialize};

use super::worker::Event;
use crate::{app::OpenMusic, menus::Handle};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioPrefs {}

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
