use std::{collections::HashSet, path::Path};

use gpui::{App, Context, Window};
use serde::{Deserialize, Serialize};
use spotify_client::{catalog::Playlist, library::Recent};

use super::worker::{Command, Event};
use crate::{
    app::{OpenMusic, Page},
    menus::Handle,
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryPrefs {}

#[derive(Default)]
pub struct State {
    pub playlists: Vec<Playlist>,
    pub playlists_loading: bool,
    pub recents: Vec<Recent>,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        urls.extend(
            self.playlists
                .iter()
                .filter_map(|playlist| playlist.cover.clone()),
        );
    }
}

impl OpenMusic {
    pub fn recent_contexts(&self) -> &[Recent] {
        &self.library.recents
    }
}

pub fn load(_app: &mut OpenMusic, _page: &Page) {}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::Playlists(playlists) => {
            app.library.playlists = playlists;
            app.library.playlists_loading = false;
        }
    }
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {
    app.library.playlists_loading = true;
    app.send(Command::Playlists);
}

pub fn signed_out(app: &mut OpenMusic) {
    app.library.playlists.clear();
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
    format!("recents={}", app.recent_contexts().len())
}

pub fn register_actions(_cx: &mut App, _handle: &Handle) {}
