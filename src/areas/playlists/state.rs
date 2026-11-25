use std::{collections::HashSet, path::Path};

use gpui::{App, Context, Window};
use spotify_client::catalog::Collection;

use super::worker::{Command, Event};
use crate::{
    app::{OpenMusic, Page},
    menus::Handle,
    ui::{
        menu::Target,
        tracks::{RowDrag, RowDrop},
    },
};

#[derive(Default)]
pub struct State {
    pub collection: Option<(String, Result<Collection, String>)>,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        if let Some((_, Ok(collection))) = &self.collection {
            urls.extend(collection.cover.clone());
            urls.extend(
                collection
                    .tracks
                    .iter()
                    .filter_map(|track| track.cover.clone()),
            );
        }
    }
}

pub fn load(app: &mut OpenMusic, page: &Page) {
    if let Page::Collection(uri) = page
        && app
            .playlists
            .collection
            .as_ref()
            .is_none_or(|(loaded, _)| loaded != uri)
    {
        app.playlists.collection = None;
        app.send(Command::Collection(uri.clone()));
    }
}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::Collection { uri, result } => app.playlists.collection = Some((uri, result)),
    }
}

pub fn connected(_app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {}

pub fn signed_out(app: &mut OpenMusic) {
    app.playlists.collection = None;
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

pub fn state_fragment(_app: &OpenMusic) -> String {
    String::new()
}

pub fn register_actions(_cx: &mut App, _handle: &Handle) {}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        "play-collection" => {
            if let Some((uri, Ok(collection))) = &app.playlists.collection {
                let (uri, tracks) = (uri.clone(), collection.tracks.clone());
                app.play_context(&uri, &tracks, 0, cx);
            }
        }
        "more-collection" => {
            if let Page::Collection(uri) = &app.page
                && let Some(target) = Target::for_uri(uri)
            {
                app.open_menu_under(target, key);
            }
        }
        _ => return false,
    }
    true
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
