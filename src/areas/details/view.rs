use gpui::{AnyElement, Context, Pixels, prelude::*};

use crate::{
    app::OpenMusic,
    ui::{
        Palette,
        menu::{Group, MenuItem, Target},
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

pub fn menu_items(_target: &Target, _app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    Vec::new()
}
