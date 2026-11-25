use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{Bounds, IntoElement, Pixels, ScrollHandle, Styled, Window, canvas, point, px};
use spotify_client::{Track, catalog::Link};

use super::{menu::Facts, worker::Command};
use crate::{
    app::{OpenMusic, Page},
    ui::menu::{Group, MenuItem, TrackTarget},
};

pub const HIGHLIGHT_FOR: Duration = Duration::from_secs(3);
const DOCK_CLEARANCE: f32 = 140.;
const SCROLL_TO: f32 = 0.3;

pub const SONG_RADIO: &str = "Go to song radio";
pub const ARTIST_RADIO: &str = "Go to artist radio";

pub struct Highlight {
    pub album: String,
    pub track: String,
    pub shown: Option<Instant>,
    pub scrolled: Rc<Cell<bool>>,
}

#[derive(Default)]
pub struct ContentScroll(RefCell<(String, ScrollHandle)>);

impl ContentScroll {
    pub fn for_page(&self, id: &str) -> ScrollHandle {
        let mut current = self.0.borrow_mut();
        if current.0 != id {
            *current = (id.to_owned(), ScrollHandle::new());
        }
        current.1.clone()
    }

    pub fn handle(&self) -> ScrollHandle {
        self.0.borrow().1.clone()
    }
}

fn radio_item(title: &'static str, seed: String) -> MenuItem {
    MenuItem::action(title, move |app, _, _| app.start_radio(seed.clone()))
        .key("r")
        .symbol("dot.radiowaves.left.and.right")
}

fn artist_page(artist: &Link) -> Page {
    Page::Artist(artist.uri.clone())
}

pub fn artists_of(track: &Track, facts: &Facts) -> Vec<Link> {
    let mut artists = if !track.artists.is_empty() {
        track.artists.clone()
    } else if let Some(found) = facts
        .artists
        .get(&track.uri)
        .filter(|found| !found.is_empty())
    {
        found.clone()
    } else {
        track
            .artist_uri
            .iter()
            .map(|uri| Link {
                uri: uri.clone(),
                name: track.artist.clone(),
            })
            .collect()
    };
    artists.retain(|artist| artist.uri.starts_with("spotify:artist:"));
    let mut seen = std::collections::HashSet::new();
    artists.retain(|artist| seen.insert(artist.uri.clone()));
    artists
}

fn artist_item(track: &Track, facts: &Facts) -> Option<MenuItem> {
    let artists = artists_of(track, facts);
    match artists.as_slice() {
        [] => None,
        [artist] => {
            let page = artist_page(artist);
            (*facts.page != page).then(|| {
                MenuItem::action("Go to artist", move |app, _, _| app.navigate(page.clone()))
                    .key("alt-a")
                    .symbol("music.mic")
            })
        }
        several => {
            let items: Vec<MenuItem> = several
                .iter()
                .map(|artist| (artist.name.clone(), artist_page(artist)))
                .filter(|(_, page)| facts.page != page)
                .map(|(name, page)| {
                    MenuItem::action(name, move |app, _, _| app.navigate(page.clone()))
                })
                .collect();
            Some(MenuItem::submenu("Go to artist", items).symbol("music.mic"))
        }
    }
}

pub fn track_items(target: &TrackTarget, facts: &Facts) -> Vec<(Group, MenuItem)> {
    let track = &target.track;
    if !track.is_track() {
        return Vec::new();
    }
    let mut items = vec![radio_item(SONG_RADIO, track.uri.clone())];
    items.extend(artist_item(track, facts));
    if let Some(album) = track
        .album_uri
        .clone()
        .filter(|album| !matches!(facts.page, Page::Collection(open) if open == album))
    {
        let uri = track.uri.clone();
        items.push(
            MenuItem::action("Go to album", move |app, _, _| {
                app.go_to_album(album.clone(), uri.clone())
            })
            .key("a")
            .symbol("square.stack"),
        );
    }
    items
        .into_iter()
        .map(|item| (Group::Navigate, item))
        .collect()
}

pub fn album_seed(uri: &str, facts: &Facts) -> String {
    facts
        .collection
        .filter(|collection| collection.uri == uri)
        .and_then(|collection| collection.artist_uri.clone())
        .unwrap_or_else(|| uri.to_owned())
}

pub fn album_items(uri: &str, facts: &Facts) -> Vec<(Group, MenuItem)> {
    vec![(
        Group::Navigate,
        radio_item(ARTIST_RADIO, album_seed(uri, facts)),
    )]
}

pub fn artist_items(uri: &str) -> Vec<(Group, MenuItem)> {
    vec![(Group::Navigate, radio_item(ARTIST_RADIO, uri.to_owned()))]
}

pub fn scroll_target(
    viewport: Bounds<Pixels>,
    offset: Pixels,
    max: Pixels,
    row: Bounds<Pixels>,
) -> Option<Pixels> {
    let visible_bottom = viewport.bottom() - px(DOCK_CLEARANCE);
    if row.top() >= viewport.top() && row.bottom() <= visible_bottom {
        return None;
    }
    let wanted = viewport.top() + viewport.size.height * SCROLL_TO;
    Some((offset + wanted - row.top()).clamp(-max, px(0.)))
}

fn scroll_into_view(
    handle: &ScrollHandle,
    done: &Cell<bool>,
    row: Bounds<Pixels>,
    window: &Window,
) {
    let viewport = handle.bounds();
    if done.get() || viewport.size.height <= px(0.) {
        return;
    }
    done.set(true);
    let offset = handle.offset();
    if let Some(y) = scroll_target(viewport, offset.y, handle.max_offset().height, row) {
        handle.set_offset(point(offset.x, y));
        window.on_next_frame(|window, _| window.refresh());
    }
}

pub fn scroll_anchor(handle: ScrollHandle, done: Rc<Cell<bool>>) -> impl IntoElement {
    canvas(
        move |row, window, _| scroll_into_view(&handle, &done, row, window),
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

impl OpenMusic {
    pub fn go_to_album(&mut self, album: String, track: String) {
        self.details.highlight = Some(Highlight {
            album: album.clone(),
            track,
            shown: None,
            scrolled: Rc::default(),
        });
        self.navigate(Page::Collection(album));
    }

    pub fn start_radio(&mut self, seed: String) {
        self.details.radio = Some(seed.clone());
        self.send(Command::Radio(seed));
    }

    pub fn radio_ready(&mut self, seed: &str, result: Result<String, String>) {
        if self.details.radio.as_deref() != Some(seed) {
            return;
        }
        self.details.radio = None;
        match result {
            Ok(playlist) => self.navigate(Page::Collection(playlist)),
            Err(message) => self.show_toast(message, None),
        }
    }

    pub fn highlights(&self, track: &Track) -> bool {
        self.details.highlight.as_ref().is_some_and(|highlight| {
            track.uri == highlight.track
                && matches!(&self.page, Page::Collection(uri) if *uri == highlight.album)
        })
    }

    pub fn tick_highlight(&mut self) -> bool {
        let Some(highlight) = &self.details.highlight else {
            return false;
        };
        let on_page = matches!(&self.page, Page::Collection(uri) if *uri == highlight.album);
        let loaded = self
            .playlists
            .collection
            .as_ref()
            .is_some_and(|(uri, _)| *uri == highlight.album);
        let expired = highlight
            .shown
            .is_some_and(|shown| shown.elapsed() >= HIGHLIGHT_FOR);
        if !on_page || expired {
            self.details.highlight = None;
            return true;
        }
        if loaded
            && let Some(highlight) = self.details.highlight.as_mut()
            && highlight.shown.is_none()
        {
            highlight.shown = Some(Instant::now());
        }
        false
    }

    pub fn lookup_artists(&mut self) {
        let Some(track) = self.track().filter(|track| track.is_track()) else {
            return;
        };
        if !track.artists.is_empty() || self.details.artists.contains_key(&track.uri) {
            return;
        }
        let uri = track.uri.clone();
        self.details.artists.insert(uri.clone(), Vec::new());
        self.send(Command::TrackArtists(uri));
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, point, px, size};

    use super::*;

    fn bounds(top: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(0.), px(top)), size(px(600.), px(height)))
    }

    #[test]
    fn scrolls_rows_out_of_sight_into_the_upper_third() {
        let viewport = bounds(0., 700.);
        assert_eq!(
            scroll_target(viewport, px(0.), px(2000.), bounds(200., 56.)),
            None
        );
        assert_eq!(
            scroll_target(viewport, px(0.), px(2000.), bounds(900., 56.)),
            Some(px(-690.))
        );
        assert_eq!(
            scroll_target(viewport, px(0.), px(300.), bounds(900., 56.)),
            Some(px(-300.))
        );
        assert_eq!(
            scroll_target(viewport, px(-800.), px(2000.), bounds(-100., 56.)),
            Some(px(-490.))
        );
        assert_eq!(
            scroll_target(viewport, px(0.), px(2000.), bounds(540., 56.)),
            Some(px(-330.))
        );
    }

    #[test]
    fn keeps_one_scroll_handle_per_page() {
        let scroll = ContentScroll::default();
        let first = scroll.for_page("page-a");
        first.set_offset(point(px(0.), px(-120.)));
        assert_eq!(scroll.for_page("page-a").offset().y, px(-120.));
        assert_eq!(scroll.handle().offset().y, px(-120.));
        assert_eq!(scroll.for_page("page-b").offset().y, px(0.));
    }
}
