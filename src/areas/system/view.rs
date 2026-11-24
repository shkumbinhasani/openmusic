use gpui::{AnyElement, Context};

use crate::{app::OpenMusic, ui::Palette};

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
