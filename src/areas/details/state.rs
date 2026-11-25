use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use gpui::{App, Context, Entity, ScrollHandle, Window};
use gpui_component::WindowExt;
use spotify_client::{catalog::Link, share};

use super::{
    credits::{self, CreditsView},
    embed::EmbedView,
    go::{ContentScroll, Highlight},
    menu,
    worker::{Command, Event},
};
use crate::{
    app::{OpenMusic, Page, Session},
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
    pub artists: HashMap<String, Vec<Link>>,
    pub highlight: Option<Highlight>,
    pub radio: Option<String>,
    pub scroll: ContentScroll,
}

impl State {
    pub fn covers(&self, _urls: &mut HashSet<String>) {}

    pub fn close_dialogs(&mut self, window: &mut Window, cx: &mut Context<OpenMusic>) {
        window.close_all_dialogs(cx);
        self.credits_view = None;
        self.embed_view = None;
    }

    pub fn content_scroll(&self, page: &str) -> ScrollHandle {
        self.scroll.for_page(page)
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
        Event::TrackArtists { uri, artists } => {
            app.details.artists.insert(uri, artists);
        }
        Event::Radio { seed, result } => app.radio_ready(&seed, result),
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
        ("radio", [seed]) => app.start_radio((*seed).to_owned()),
        ("goalbum", [album, track]) => app.go_to_album((*album).to_owned(), (*track).to_owned()),
        ("artists", [uri]) => {
            let lines = match app.details.artists.get(*uri) {
                Some(artists) if !artists.is_empty() => artists
                    .iter()
                    .map(|artist| format!("{} {}", artist.uri, artist.name))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Some(_) => "pending".to_owned(),
                None => {
                    app.details.artists.insert((*uri).to_owned(), Vec::new());
                    app.send(Command::TrackArtists((*uri).to_owned()));
                    "pending".to_owned()
                }
            };
            write("artists.txt", lines);
        }
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    let details = &app.details;
    let credits = details
        .credits_view
        .as_ref()
        .map(|(uri, _)| format!("credits={uri}"));
    let embed = details
        .embed_view
        .as_ref()
        .map(|(uri, _)| format!("embed={uri}"));
    let highlight = details
        .highlight
        .as_ref()
        .map(|highlight| format!("highlight={}", highlight.track));
    let radio = details.radio.as_ref().map(|seed| format!("radio={seed}"));
    [credits, embed, highlight, radio]
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

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    if !matches!(app.session, Session::Connected) {
        return;
    }
    app.lookup_artists();
    if app.tick_highlight() {
        cx.notify();
    }
}
