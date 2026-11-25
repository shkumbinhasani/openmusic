use gpui::{AnyElement, Context, Pixels, prelude::*};
use spotify_client::{Track, details::hide_context};

use super::{
    feedback::first_artist,
    go,
    menu::{self, Facts},
};
use crate::{
    app::OpenMusic,
    ui::{
        Palette,
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

impl OpenMusic {
    pub fn track_page(
        &self,
        _uri: &str,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }
}

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

pub fn row_dimmed(app: &OpenMusic, track: &Track, _row: Row, options: &RowOptions) -> bool {
    let sets = &app.details.sets;
    let hidden = || {
        options
            .context
            .as_deref()
            .and_then(hide_context)
            .is_some_and(|context| sets.is_hidden(&track.uri, &context))
    };
    let banned = || first_artist(track).is_some_and(|artist| sets.is_banned(artist));
    track.is_track() && (hidden() || banned())
}

pub fn row_highlighted(app: &OpenMusic, track: &Track) -> bool {
    app.highlights(track)
}
