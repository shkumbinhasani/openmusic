use std::{collections::HashSet, path::Path};

use gpui::{App, Context, Window};
use serde::{Deserialize, Serialize};
use spotify_client::account::AccountInfo;

use super::worker::Event;
use crate::{
    app::{OpenMusic, Page},
    menus::{self, Handle, SignOut},
    ui::tracks::{RowDrag, RowDrop},
    worker,
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountPrefs {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileListKind {
    Followers,
    Following,
    Playlists,
    RecentArtists,
}

impl ProfileListKind {
    pub const KINDS: [Self; 4] = [
        Self::Followers,
        Self::Following,
        Self::Playlists,
        Self::RecentArtists,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Followers => "followers",
            Self::Following => "following",
            Self::Playlists => "playlists",
            Self::RecentArtists => "recentartists",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::KINDS.into_iter().find(|kind| kind.name() == name)
    }
}

#[derive(Default)]
pub struct State {
    pub info: Option<AccountInfo>,
}

impl State {
    pub fn covers(&self, _urls: &mut HashSet<String>) {}
}

impl OpenMusic {
    pub fn account(&self) -> Option<&AccountInfo> {
        self.account.info.as_ref()
    }
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

pub fn signed_out(app: &mut OpenMusic) {
    app.account.info = None;
}

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

pub fn state_fragment(app: &OpenMusic) -> String {
    format!(
        "plan={}",
        app.account()
            .map(|account| account.product.as_str())
            .filter(|product| !product.is_empty())
            .unwrap_or("unknown")
    )
}

pub fn register_actions(cx: &mut App, handle: &Handle) {
    menus::route(cx, handle, |_: &SignOut, app, _, _| {
        app.send(worker::Command::SignOut)
    });
}

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
