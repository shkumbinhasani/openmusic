use std::{collections::HashSet, path::Path};

use gpui::{App, Context, Entity, Window};
use gpui_component::WindowExt;
use spotify_client::share;

use super::{
    credits::{self, CreditsView},
    embed::EmbedView,
    menu,
    worker::Event,
};
use crate::{
    app::{OpenMusic, Page},
    menus::Handle,
    ui::{
        menu::Surface,
        tracks::{RowDrag, RowDrop},
    },
};

#[derive(Default)]
pub struct State {
    pub credits: credits::Cache,
    pub credits_view: Option<(String, Entity<CreditsView>)>,
    pub embed_view: Option<(String, Entity<EmbedView>)>,
}

impl State {
    pub fn covers(&self, _urls: &mut HashSet<String>) {}

    pub fn close_dialogs(&mut self, window: &mut Window, cx: &mut Context<OpenMusic>) {
        window.close_all_dialogs(cx);
        self.credits_view = None;
        self.embed_view = None;
    }
}

pub fn load(_app: &mut OpenMusic, _page: &Page) {}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::Credits { uri, result } => {
            if let Ok(credits) = &result {
                app.details.credits.insert(credits.clone());
                app.lookup_saved(credits.linked_artists());
            }
            if let Some((open, view)) = &app.details.credits_view
                && *open == uri
            {
                view.update(cx, |view, cx| {
                    view.result = Some(result);
                    cx.notify();
                });
            }
        }
        Event::CoverColor { uri, color } => {
            if let Some((open, view)) = &app.details.embed_view
                && *open == uri
            {
                view.update(cx, |view, cx| {
                    view.swatch = color;
                    cx.notify();
                });
            }
        }
    }
}

pub fn connected(_app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {}

pub fn signed_out(app: &mut OpenMusic) {
    app.details = State::default();
}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
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
        ("share", ["embed", uri]) => write(
            "share.txt",
            share::embed_code(uri, &share::Embed::default()).unwrap_or_else(|| "none".into()),
        ),
        ("embed", ["dump"]) => write(
            "embed.txt",
            app.details
                .embed_view
                .as_ref()
                .map(|(_, view)| view.read(cx).describe())
                .unwrap_or_else(|| "closed".into()),
        ),
        ("embed" | "credits", ["close"]) => app.details.close_dialogs(window, cx),
        ("embed", [setting, value @ ..])
            if app.details.embed_view.is_some() && !value.is_empty() =>
        {
            let value = value.join(" ");
            if let Some((_, view)) = app.details.embed_view.clone() {
                view.update(cx, |view, cx| {
                    match *setting {
                        "theme" => {
                            view.theme = if value == "dark" {
                                share::Theme::Dark
                            } else {
                                share::Theme::Dynamic
                            }
                        }
                        "size" => {
                            view.size = if value == "compact" {
                                share::Size::Compact
                            } else {
                                share::Size::Normal
                            }
                        }
                        "width" => view.set_width(&value, window, cx),
                        "start" if value == "off" => view.start_on = false,
                        "start" => {
                            view.start_on = true;
                            view.set_start(&value, window, cx);
                        }
                        "code" => view.show_code = value == "on",
                        _ => {}
                    }
                    cx.notify();
                });
            }
        }
        ("embed", ["copy"]) => {
            if let Some((_, view)) = app.details.embed_view.clone() {
                view.update(cx, |view, cx| view.copy(cx));
            }
        }
        ("embed", [uri]) => app.show_embed((*uri).to_owned(), window, cx),
        ("credits", ["dump"]) => write(
            "credits.txt",
            app.details
                .credits_view
                .as_ref()
                .map(|(_, view)| view.read(cx).describe())
                .unwrap_or_else(|| "closed".into()),
        ),
        ("credits", [uri]) => {
            let title = app
                .track()
                .filter(|track| track.uri == *uri)
                .map(|track| track.title.clone())
                .unwrap_or_default();
            app.show_credits((*uri).to_owned(), title, window, cx);
        }
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    let credits = app
        .details
        .credits_view
        .as_ref()
        .map(|(uri, _)| format!("credits={uri}"));
    let embed = app
        .details
        .embed_view
        .as_ref()
        .map(|(uri, _)| format!("embed={uri}"));
    [credits, embed]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
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
