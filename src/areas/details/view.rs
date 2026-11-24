use gpui::{AnyElement, Context, Pixels, prelude::*};

use crate::{app::OpenMusic, ui::Palette};

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
