use gpui::{AnyElement, Context};

use crate::{
    app::OpenMusic,
    ui::{
        Palette,
        menu::{Group, MenuItem, Target},
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
