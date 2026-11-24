use gpui::{AnyElement, Context, FontWeight, Pixels, div, prelude::*, px, rgb};

use crate::{
    app::{OpenMusic, Page},
    ui::{
        Palette,
        components::{artwork, link, skeleton_header, skeleton_rows},
    },
};

impl OpenMusic {
    pub fn collection_page(
        &self,
        uri: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)));
        let Some((loaded, result)) = self
            .playlists
            .collection
            .as_ref()
            .filter(|(loaded, _)| loaded == uri)
        else {
            let numbered = uri.starts_with("spotify:album:");
            return page
                .child(skeleton_header("collection-header", false, palette))
                .child(skeleton_rows("collection-rows", 8, numbered, palette));
        };
        let collection = match result {
            Ok(collection) => collection,
            Err(error) => return page.child(self.page_status(error.clone(), palette)),
        };
        let image = collection
            .cover
            .as_ref()
            .and_then(|url| self.covers.get(url));
        let artist_page = collection.artist_uri.clone().map(Page::Artist);
        page.child(
            div()
                .flex()
                .items_center()
                .gap_6()
                .child(artwork(image, px(184.), px(16.), palette).shadow_lg())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_1p5()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.accent))
                                .child(collection.kind.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(34.))
                                .font_weight(FontWeight::BOLD)
                                .truncate()
                                .w_full()
                                .child(collection.name.clone()),
                        )
                        .child(div().w_full().flex().min_w_0().child(link(
                            ("collection-subtitle", 0),
                            collection.subtitle.clone(),
                            artist_page,
                            13.5,
                            palette,
                            cx,
                        )))
                        .child(
                            div().mt_3().child(
                                self.text_control("play-collection", "Play", true, palette, cx)
                                    .w(px(96.)),
                            ),
                        ),
                ),
        )
        .child(if collection.tracks.is_empty() {
            self.page_status("This is empty.", palette)
        } else {
            self.track_list(
                &collection.tracks,
                "collection-track",
                0,
                Some(loaded),
                collection.kind == "ALBUM",
                palette,
                cx,
            )
        })
    }
}

impl OpenMusic {
    pub fn blend_invitation_page(
        &self,
        _id: &str,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }
}
