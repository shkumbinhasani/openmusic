use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use gpui::{App, Context, Entity, ScrollHandle, Window};
use spotify_client::{
    catalog::Link,
    dealer::Message,
    details::{self, Ban, BanProvider, SetPush, hide_context},
    share,
};

use super::{
    credits::{self, CreditsView},
    dialog::{self, Slot},
    embed::EmbedView,
    feedback::Sets,
    go::{ContentScroll, Highlight},
    lookup::Lookups,
    menu,
    track::TrackView,
    worker::{Command, Event},
};
use crate::{
    app::{OpenMusic, Page, Session},
    menus::Handle,
    ui::{
        menu::{Surface, Target},
        tracks::{RowDrag, RowDrop},
    },
};

#[derive(Clone)]
pub enum Shown {
    Credits(String, Entity<CreditsView>),
    Embed(String, Entity<EmbedView>),
}

#[derive(Default)]
pub struct State {
    pub credits: credits::Cache,
    pub dialog: Slot<Shown>,
    pub sets: Sets,
    pub artists: Lookups<Vec<Link>>,
    pub highlight: Option<Highlight>,
    pub radio: Option<String>,
    pub scroll: ContentScroll,
    pub track: Option<TrackView>,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        if let Some(page) = self.track.as_ref().and_then(TrackView::page) {
            urls.extend(page.images());
        }
    }

    pub fn credits_view(&self) -> Option<(String, Entity<CreditsView>)> {
        match self.dialog.get()? {
            Shown::Credits(uri, view) => Some((uri, view)),
            Shown::Embed(..) => None,
        }
    }

    pub fn embed_view(&self) -> Option<(String, Entity<EmbedView>)> {
        match self.dialog.get()? {
            Shown::Embed(uri, view) => Some((uri, view)),
            Shown::Credits(..) => None,
        }
    }

    pub fn show(
        &mut self,
        shown: Shown,
        build: dialog::Build,
        window: &mut Window,
        cx: &mut Context<OpenMusic>,
    ) {
        if self
            .embed_view()
            .is_some_and(|(_, view)| view.read(cx).typing(window, cx))
        {
            self.dialog.close(window, cx);
        }
        self.dialog.show(shown, build, window, cx);
    }

    pub fn content_scroll(&self, page: &str) -> ScrollHandle {
        self.scroll.for_page(page)
    }
}

pub fn load(app: &mut OpenMusic, page: &Page) {
    let Page::Track(uri) = page else {
        return;
    };
    let fresh = app
        .details
        .track
        .as_ref()
        .is_some_and(|view| view.uri == *uri && !matches!(view.result, Some(Err(_))));
    if fresh {
        return;
    }
    app.details.track = Some(TrackView::new(uri.clone()));
    app.send(Command::TrackPage(uri.clone()));
}

fn load_sets(app: &mut OpenMusic) {
    app.details.sets.loading();
    app.send(Command::LoadSets);
}

fn take_push(app: &mut OpenMusic, push: &SetPush) {
    if app.details.sets.push(push) {
        load_sets(app);
    }
}

fn lookup_editable(app: &mut OpenMusic) {
    let Some(context) = app
        .playback
        .now
        .context_uri
        .as_deref()
        .and_then(hide_context)
        .filter(|context| context.starts_with("spotify:playlist:"))
    else {
        return;
    };
    let known = menu::Facts::of(app).editable(&context).is_some();
    if !known && app.details.sets.editable.ask(&context) {
        app.send(Command::Editable(context));
    }
}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::Sets(result) => app.details.sets.apply(result),
        Event::Pushed(push) => take_push(app, &push),
        Event::Excluded {
            uri,
            excluded,
            result,
        } => {
            app.details
                .sets
                .excluded_written(&uri, excluded, result.is_ok());
            if let Err(message) = result {
                app.show_toast(message, None);
            }
        }
        Event::Hidden {
            ban,
            hidden,
            result,
        } => {
            app.details.sets.hidden_written(&ban, hidden, &result);
            if let Err(message) = result {
                app.show_toast(message, None);
            }
        }
        Event::Banned {
            uri,
            banned,
            result,
        } => {
            app.details
                .sets
                .banned_written(&uri, banned, result.is_ok());
            if let Err(message) = result {
                app.show_toast(message, None);
            }
        }
        Event::Editable { uri, editable } => {
            app.details.sets.editable.answer(uri, editable);
        }
        Event::TrackArtists { uri, artists } => {
            app.details.artists.answer(uri, artists);
        }
        Event::Radio { seed, result } => app.radio_ready(&seed, result),
        Event::TrackPage { uri, result } => {
            if let Some(credits) = result.as_ref().ok().and_then(|page| page.credits.clone()) {
                app.details.credits.insert(credits);
            }
            if let Some(view) = app.details.track.as_mut().filter(|view| view.uri == uri) {
                view.result = Some(result);
            }
        }
        Event::Credits { uri, result } => {
            if let Ok(credits) = &result {
                app.details.credits.insert(credits.clone());
                app.lookup_saved(credits.linked_artists());
            }
            if let Some((open, view)) = app.details.credits_view()
                && open == uri
            {
                view.update(cx, |view, cx| {
                    view.result = Some(result);
                    cx.notify();
                });
            }
        }
        Event::CoverColor { uri, color } => {
            if let Some((open, view)) = app.details.embed_view()
                && open == uri
            {
                view.update(cx, |view, cx| {
                    view.swatch = color;
                    cx.notify();
                });
            }
        }
    }
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {
    app.details.sets.resend_skips();
    load_sets(app);
}

pub fn reconnected(app: &mut OpenMusic) {
    load_sets(app);
}

fn sets_report(app: &OpenMusic) -> String {
    let sets = &app.details.sets;
    let mut lines = vec![format!(
        "load={} excluded={} hidden={} banned={} format={} pushes={} reloading={}",
        sets.load.name(),
        sets.excluded.len(),
        sets.hidden_count(),
        sets.banned.len(),
        sets.last_format.map_or("none", |format| format.name()),
        sets.pushes,
        sets.loading_now(),
    )];
    let mut excluded: Vec<&String> = sets.excluded.iter().collect();
    excluded.sort();
    lines.extend(excluded.into_iter().map(|uri| format!("excluded {uri}")));
    let mut banned: Vec<&String> = sets.banned.iter().collect();
    banned.sort();
    lines.extend(banned.into_iter().map(|uri| format!("banned {uri}")));
    let mut hidden: Vec<String> = sets
        .hidden
        .iter()
        .flat_map(|(context, tracks)| {
            tracks
                .iter()
                .map(move |track| format!("hidden {track} in {context}"))
        })
        .collect();
    hidden.sort();
    lines.extend(hidden);
    let mut editable: Vec<String> = sets
        .editable
        .iter()
        .map(|(uri, editable)| format!("editable {uri} {editable:?}"))
        .collect();
    editable.sort();
    lines.extend(editable);
    lines.join("\n")
}

fn switch(value: &str) -> Option<bool> {
    match value {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

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
                .embed_view()
                .map(|(_, view)| view.read(cx).describe())
                .unwrap_or_else(|| "closed".into()),
        ),
        ("embed" | "credits", ["close"]) => app.details.dialog.close(window, cx),
        ("embed", [setting, value @ ..])
            if app.details.embed_view().is_some() && !value.is_empty() =>
        {
            let value = value.join(" ");
            if let Some((_, view)) = app.details.embed_view() {
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
            if let Some((_, view)) = app.details.embed_view() {
                view.update(cx, |view, cx| view.copy(cx));
            }
        }
        ("embed", [uri]) => app.show_embed((*uri).to_owned(), window, cx),
        ("credits", ["dump"]) => write(
            "credits.txt",
            app.details
                .credits_view()
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
        ("sets", []) => write("sets.txt", sets_report(app)),
        ("sets", ["reload"]) => load_sets(app),
        ("setpush", [uri, payload @ ..]) if !payload.is_empty() => {
            let message = serde_json::from_str(&payload.join(" "))
                .ok()
                .map(|payload| Message {
                    uri: (*uri).to_owned(),
                    headers: HashMap::new(),
                    payloads: vec![payload],
                });
            let line = match message.as_ref().and_then(details::set_push) {
                Some(push) => {
                    take_push(app, &push);
                    format!(
                        "set={} items={} unreadable={}",
                        push.set.name(),
                        push.items.len(),
                        push.unreadable
                    )
                }
                None => "ignored".to_owned(),
            };
            write("setpush.txt", line);
        }
        ("exclude", [uri, value]) => {
            if let Some(excluded) = switch(value) {
                app.set_excluded((*uri).to_owned(), excluded);
            }
        }
        ("ban", [uri, value]) => {
            if let Some(banned) = switch(value) {
                app.set_artist_banned((*uri).to_owned(), banned);
            }
        }
        ("hide", [track, context, value]) => {
            if let (Some(context), Some(hidden)) = (hide_context(context), switch(value)) {
                let ban = Ban {
                    track: (*track).to_owned(),
                    context,
                };
                app.set_hidden(ban, BanProvider::Context, hidden);
            }
        }
        ("trackpage", []) => write(
            "trackpage.txt",
            app.details
                .track
                .as_ref()
                .map(TrackView::describe)
                .unwrap_or_else(|| "none".into()),
        ),
        ("trackpage", [part]) => {
            if let Some(view) = app.details.track.as_mut() {
                match *part {
                    "lyrics" => view.lyrics_open = !view.lyrics_open,
                    "more" => view.more_tracks = !view.more_tracks,
                    "artists" => view.all_artists = !view.all_artists,
                    _ => {}
                }
            }
        }
        ("radio", [seed]) => app.start_radio((*seed).to_owned()),
        ("goalbum", [album, track]) => app.go_to_album((*album).to_owned(), (*track).to_owned()),
        ("artists", [uri]) => {
            let lines = match app.details.artists.get(uri) {
                Some(artists) if !artists.is_empty() => artists
                    .iter()
                    .map(|artist| format!("{} {}", artist.uri, artist.name))
                    .collect::<Vec<_>>()
                    .join("\n"),
                Some(_) => "none".to_owned(),
                None => {
                    if app.details.artists.ask(uri) {
                        app.send(Command::TrackArtists((*uri).to_owned()));
                    }
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
        .credits_view()
        .map(|(uri, _)| format!("credits={uri}"));
    let embed = details.embed_view().map(|(uri, _)| format!("embed={uri}"));
    let sets = Some(format!(
        "sets={} excluded={} hidden={} banned={}",
        details.sets.load.name(),
        details.sets.excluded.len(),
        details.sets.hidden_count(),
        details.sets.banned.len(),
    ));
    let highlight = details
        .highlight
        .as_ref()
        .map(|highlight| format!("highlight={}", highlight.track));
    let radio = details.radio.as_ref().map(|seed| format!("radio={seed}"));
    let track = details
        .track
        .as_ref()
        .filter(|view| matches!(&app.page, Page::Track(uri) if *uri == view.uri))
        .map(|view| format!("trackpage={}", view.state()));
    [credits, embed, sets, highlight, radio, track]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn register_actions(_cx: &mut App, _handle: &Handle) {}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        "track-play" => {
            if let Some(track) = app.track_page_track() {
                if app.is_current(&track.uri) {
                    app.toggle(cx);
                } else if !app.refuses_track(Some(&track), None, cx) {
                    app.play(&[track], 0, cx);
                }
            }
        }
        "more-track" => {
            if let Some(track) = app.track_page_track() {
                app.open_menu_under(Target::track(track, None, Surface::Header), key);
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

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    if !matches!(app.session, Session::Connected) {
        return;
    }
    if app.details.sets.due() {
        load_sets(app);
    }
    app.send_skips();
    app.lookup_artists();
    lookup_editable(app);
    if app.tick_highlight() {
        cx.notify();
    }
}
