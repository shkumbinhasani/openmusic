use gpui::{AnyElement, Context, prelude::*};
use spotify_client::Track;

use super::{
    feedback::refusal,
    go,
    menu::{self, Facts},
    track::ALBUM_ROWS,
};
use crate::{
    app::{OpenMusic, Page},
    ui::{
        Palette,
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    menu::items(target, &Facts::of(app))
}

pub fn nav_entries(_app: &OpenMusic) -> Vec<NavEntry> {
    Vec::new()
}

pub fn dock_items(
    _app: &OpenMusic,
    _roomy: bool,
    _palette: Palette,
    _cx: &mut Context<OpenMusic>,
) -> Vec<DockItem> {
    Vec::new()
}

pub fn row_accessories(
    app: &OpenMusic,
    track: &Track,
    _row: Row,
    _options: &RowOptions,
    _palette: Palette,
    _cx: &mut Context<OpenMusic>,
) -> Vec<AnyElement> {
    match &app.details.highlight {
        Some(highlight) if !highlight.scrolled.get() && app.highlights(track) => {
            vec![
                go::scroll_anchor(app.details.scroll.handle(), highlight.scrolled.clone())
                    .into_any_element(),
            ]
        }
        _ => Vec::new(),
    }
}

pub fn pinned_row(app: &OpenMusic, tracks: &[Track]) -> Option<usize> {
    app.details
        .highlight
        .as_ref()
        .filter(|highlight| !highlight.scrolled.get())?;
    tracks.iter().position(|track| app.highlights(track))
}

pub fn row_dimmed(app: &OpenMusic, track: &Track, _row: Row, options: &RowOptions) -> bool {
    refusal(&app.details.sets, track, options.context.as_deref()).is_some()
}

pub fn row_highlighted(app: &OpenMusic, track: &Track, row: Row) -> bool {
    app.highlights(track)
        || (row.0 == ALBUM_ROWS && matches!(&app.page, Page::Track(uri) if *uri == track.uri))
}

pub fn title_page(app: &OpenMusic, track: &Track) -> Option<Page> {
    let page = Page::Track(track.uri.clone());
    (track.is_track() && app.page != page).then_some(page)
}
