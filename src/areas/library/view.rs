use gpui::{AnyElement, Context, FontWeight, MouseButton, Pixels, div, prelude::*, px, rgb, rgba};
use spotify_client::Track;

use crate::{
    app::{OpenMusic, Page},
    ui::{
        Palette,
        components::{ROW_RADIUS, artwork, bone},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::{RowDrag, RowDrop, RowOptions},
    },
};

pub const PLAYLIST_ROW: &str = "playlist";

impl OpenMusic {
    pub fn playlist_list(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                div()
                    .px_2p5()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child("Playlists"),
            )
            .child(
                div()
                    .id("sidebar-playlists")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(
                        self.library.playlists.is_empty() && self.library.playlists_loading,
                        |list| {
                            list.children((0..7).map(|index| {
                                div()
                                    .h(px(36.))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_1p5()
                                    .child(
                                        bone("sidebar-playlist", index * 2, palette)
                                            .size(px(26.))
                                            .rounded(px(5.)),
                                    )
                                    .child(
                                        bone("sidebar-playlist", index * 2 + 1, palette)
                                            .w(px([120., 90., 140., 104., 128., 84., 112.][index]))
                                            .h(px(11.))
                                            .rounded(px(4.)),
                                    )
                            }))
                        },
                    )
                    .children(self.library.playlists.iter().enumerate().map(
                        |(index, playlist)| {
                            let page = Page::Collection(playlist.uri.clone());
                            let selected = self.page == page;
                            let image =
                                playlist.cover.as_ref().and_then(|url| self.covers.get(url));
                            let uri = playlist.uri.clone();
                            let row = (PLAYLIST_ROW, index);
                            div()
                                .id(row)
                                .h(px(36.))
                                .flex()
                                .items_center()
                                .gap_2()
                                .px_1p5()
                                .rounded(px(ROW_RADIUS))
                                .cursor_pointer()
                                .when(selected, |item| {
                                    item.bg(rgba(palette.press))
                                        .font_weight(FontWeight::SEMIBOLD)
                                })
                                .when(self.menu_row(row), |item| item.bg(rgba(palette.press)))
                                .on_mouse_down(
                                    MouseButton::Right,
                                    self.on_menu(Some(row), move |_| Target::for_uri(&uri), cx),
                                )
                                .hover(move |item| item.bg(rgba(palette.press)))
                                .drag_over::<RowDrag>(move |item, _, _, _| {
                                    item.bg(rgba(palette.press))
                                })
                                .on_drop(cx.listener(move |this, drag: &RowDrag, window, cx| {
                                    this.drop_rows(drag, RowDrop { row, after: false }, window, cx);
                                }))
                                .child(artwork(image, px(26.), px(5.), palette))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(12.5))
                                        .child(playlist.name.clone()),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.navigate(page.clone());
                                    cx.notify();
                                }))
                                .map(|line| {
                                    self.library_drag(
                                        line,
                                        &playlist.uri,
                                        &playlist.name,
                                        palette,
                                        cx,
                                    )
                                })
                        },
                    )),
            )
    }
}

impl OpenMusic {
    pub fn liked_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn recent_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn folder_page(
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
