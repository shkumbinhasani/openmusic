use gpui::{AnyElement, Context, FontWeight, MouseButton, Pixels, div, prelude::*, px, rgb, rgba};

use crate::{
    app::{OpenMusic, Page},
    ui::{
        Palette,
        components::{ROW_RADIUS, artwork, bone},
        menu::{Group, MenuItem, Target},
    },
};

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
                            div()
                                .id(("playlist", index))
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
                                .when(self.menu_row(("playlist", index)), |item| {
                                    item.bg(rgba(palette.press))
                                })
                                .on_mouse_down(
                                    MouseButton::Right,
                                    self.on_menu(
                                        Some(("playlist", index)),
                                        move |_| Target::for_uri(&uri),
                                        cx,
                                    ),
                                )
                                .hover(move |item| item.bg(rgba(palette.press)))
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
