use gpui::{AnyElement, Context, Styled, px};
use spotify_client::Track;

use super::{mini_player, native_window};
use crate::{
    app::OpenMusic,
    overlay::Label,
    ui::{
        Palette,
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

pub const MINI_BUTTON: &str = "mini-player";
pub const MINI_BUTTON_WEIGHT: u32 = 950;
pub const MINI_BUTTON_WINDOW: f32 = 1280.;

impl OpenMusic {
    pub fn startup_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn notifications_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }
}

pub fn menu_items(_target: &Target, _app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    Vec::new()
}

pub fn nav_entries(_app: &OpenMusic) -> Vec<NavEntry> {
    Vec::new()
}

pub fn dock_items(
    app: &OpenMusic,
    roomy: bool,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Vec<DockItem> {
    if !roomy || native_window::width() < MINI_BUTTON_WINDOW || !app.mini_allowed() {
        return Vec::new();
    }
    let symbol = if mini_player::is_visible() {
        "pip.exit"
    } else {
        "pip.enter"
    };
    vec![DockItem::new(
        MINI_BUTTON,
        MINI_BUTTON_WEIGHT,
        48.,
        app.dock_control(MINI_BUTTON, Label::Symbol(symbol), false, palette, cx)
            .size(px(48.)),
    )]
}

pub fn row_accessories(
    _app: &OpenMusic,
    _track: &Track,
    _row: Row,
    _options: &RowOptions,
    _palette: Palette,
    _cx: &mut Context<OpenMusic>,
) -> Vec<AnyElement> {
    Vec::new()
}

pub fn row_dimmed(_app: &OpenMusic, _track: &Track, _row: Row, _options: &RowOptions) -> bool {
    false
}
