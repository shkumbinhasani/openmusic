use gpui::{AnyElement, Context, FontWeight, Pixels, div, prelude::*, px};
use spotify_client::Track;

use super::state::ProfileListKind;
use crate::{
    app::{OpenMusic, settings::SECTIONS},
    ui::{
        Palette, Panel,
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

impl OpenMusic {
    pub fn settings_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sections: Vec<AnyElement> = SECTIONS
            .iter()
            .filter_map(|section| section(self, palette, cx))
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child("Settings"),
            )
            .map(|page| {
                if sections.is_empty() {
                    page.child(self.page_status("Coming soon", palette))
                } else {
                    page.children(sections)
                }
            })
            .into_any_element()
    }

    pub fn profile_page(
        &self,
        _uri: &str,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn profile_list_page(
        &self,
        _uri: &str,
        _kind: ProfileListKind,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn local_files_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn account_chip(&self, _palette: Palette, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    pub fn right_panel(&self, _palette: Palette, _cx: &mut Context<Self>) -> Option<Panel> {
        None
    }

    pub fn account_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn content_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn social_section(&self, _palette: Palette, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    pub fn library_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn language_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn storage_section(
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

pub fn row_dimmed(_app: &OpenMusic, _track: &Track, _row: Row, _options: &RowOptions) -> bool {
    false
}
