use gpui::{AnyElement, Context};
use spotify_client::Track;

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
    pub fn playback_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn audio_quality_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn volume_section(&self, _palette: Palette, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    pub fn device_section(&self, _palette: Palette, _cx: &mut Context<Self>) -> Option<AnyElement> {
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
    _app: &OpenMusic,
    _roomy: bool,
    _palette: Palette,
    _cx: &mut Context<OpenMusic>,
) -> Vec<DockItem> {
    Vec::new()
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
