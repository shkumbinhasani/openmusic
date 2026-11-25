use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, ObjectFit, Pixels, div, img,
    linear_color_stop, linear_gradient, prelude::*, px, rgb, rgba,
};
use spotify_client::{
    browse::BrowsePage,
    entity::Card,
    home::{Layout, Section},
};

use super::{
    home::SHELF_CARDS,
    home_view::CARD_TEXT_HEIGHT,
    search_view::{card_bones, heading_bone},
    state::SectionSource,
};
use crate::{
    app::{OpenMusic, Page, page_for},
    ui::{
        Palette,
        cards::card_target,
        components::{artwork, bone, scroll_sentinel},
    },
};

const TILE_FALLBACK: u32 = 0x535353;
const TILE_RATIO: f32 = 0.62;
const TILE_MIN_HEIGHT: f32 = 96.;
const TILE_ART: f32 = 0.45;
const HEADER_HEIGHT: f32 = 168.;
const HEADER_TINT: u32 = 0x3d;

pub fn tile_height(card_side: Pixels) -> Pixels {
    (card_side * TILE_RATIO).max(px(TILE_MIN_HEIGHT))
}

impl OpenMusic {
    pub fn tile_grid(
        &self,
        cards: &[Card],
        prefix: &'static str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let height = tile_height(card_side);
        let art = card_side * TILE_ART;
        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .children(cards.iter().enumerate().map(|(index, card)| {
                let image = card.image.as_ref().and_then(|url| self.covers.get(url));
                let page = page_for(&card.uri);
                let target = card_target(card);
                div()
                    .id((prefix, index))
                    .relative()
                    .flex_shrink_0()
                    .w(card_side)
                    .h(height)
                    .rounded(px(8.))
                    .overflow_hidden()
                    .bg(rgb(card.color.unwrap_or(TILE_FALLBACK)))
                    .cursor_pointer()
                    .hover(|tile| tile.opacity(0.88))
                    .child(
                        artwork(image, art, px(6.), palette)
                            .absolute()
                            .right(art * -0.1)
                            .bottom(art * -0.1)
                            .shadow_lg(),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(12.))
                            .left(px(14.))
                            .right(px(14.))
                            .text_size(px(18.))
                            .line_height(px(22.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0xffffff))
                            .line_clamp(2)
                            .child(card.title.clone()),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(page) = page.clone() {
                            this.navigate(page);
                        }
                        cx.notify();
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_menu(None, move |_| target.clone(), cx),
                    )
            }))
    }

    fn tile_bones(
        &self,
        key: &'static str,
        count: usize,
        card_side: Pixels,
        palette: Palette,
    ) -> Div {
        let height = tile_height(card_side);
        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .children((0..count).map(move |index| {
                bone(key, index, palette)
                    .w(card_side)
                    .h(height)
                    .rounded(px(8.))
            }))
    }

    pub fn browse_all_section(
        &self,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let state = &self.discovery.browse_all;
        let heading = |title: String| {
            div()
                .min_w_0()
                .truncate()
                .text_size(px(19.))
                .font_weight(FontWeight::BOLD)
                .child(title)
        };
        match &state.all {
            Some(all) => {
                let title = if all.title.is_empty() {
                    "Browse all".to_owned()
                } else {
                    all.title.clone()
                };
                Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(heading(title))
                        .child(self.tile_grid(
                            &all.tiles,
                            "browse-all-tile",
                            card_side,
                            palette,
                            cx,
                        )),
                )
            }
            None if state.pending => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(heading_bone("browse-all-title", palette))
                    .child(self.tile_bones("browse-all-bones", 8, card_side, palette)),
            ),
            None => state.error.clone().map(|error| {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(heading("Browse all".to_owned()))
                    .child(self.page_status(error, palette))
            }),
        }
    }

    pub fn browse_page(
        &self,
        uri: &str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.discovery.browse_pages.get(uri, None);
        let pending = state.is_none_or(|state| state.pending.is_some());
        let error = state.and_then(|state| state.error.clone());
        let page = div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)));
        let Some(browse) = state.and_then(|state| state.page.as_ref()) else {
            if pending {
                return page
                    .child(
                        bone("browse-header-bone", 0, palette)
                            .w_full()
                            .h(px(HEADER_HEIGHT))
                            .rounded(px(16.)),
                    )
                    .child(self.shelf_bones(card_side, palette))
                    .into_any_element();
            }
            return page
                .child(self.page_status(
                    error.unwrap_or_else(|| "Couldn’t load this page. Try again.".to_owned()),
                    palette,
                ))
                .into_any_element();
        };
        page.child(self.browse_header(browse, palette))
            .children(browse.sections.iter().enumerate().map(|(index, section)| {
                self.browse_section(index, section, card_side, palette, cx)
            }))
            .when(browse.sections.is_empty() && !pending, |page| {
                page.child(self.page_status("Nothing to show here yet.", palette))
            })
            .when(browse.next.is_some(), |page| {
                page.child(scroll_sentinel(&self.discovery.more_sentinel))
            })
            .when(pending, |page| {
                page.child(self.shelf_bones(card_side, palette))
            })
            .when_some(error, |page, error| {
                page.child(self.page_status(error, palette))
            })
            .into_any_element()
    }

    fn shelf_bones(&self, card_side: Pixels, palette: Palette) -> Div {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(heading_bone("browse-shelf-title", palette))
            .child(card_bones("browse-shelf-cards", 4, card_side, palette))
    }

    fn browse_header(&self, page: &BrowsePage, palette: Palette) -> Div {
        let image = page
            .image
            .as_ref()
            .and_then(|url| self.covers.get(url))
            .cloned();
        let on_image = image.is_some();
        let text = if on_image { 0xffffff } else { palette.text };
        let tint = page.color.unwrap_or(palette.accent);
        div()
            .relative()
            .w_full()
            .h(px(HEADER_HEIGHT))
            .rounded(px(16.))
            .overflow_hidden()
            .bg(rgba(tint << 8 | HEADER_TINT))
            .when_some(image, |band, image| {
                band.child(
                    img(image)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                )
                .child(div().absolute().inset_0().bg(linear_gradient(
                    180.,
                    linear_color_stop(rgba(0x0000_0010), 0.),
                    linear_color_stop(rgba(0x0000_0099), 1.),
                )))
            })
            .child(
                div()
                    .absolute()
                    .left(px(24.))
                    .right(px(24.))
                    .bottom(px(20.))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_color(rgb(text))
                    .child(
                        div()
                            .text_size(px(40.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(page.title.clone()),
                    )
                    .when_some(page.subtitle.clone(), |header, subtitle| {
                        header.child(
                            div()
                                .text_size(px(14.))
                                .opacity(0.8)
                                .truncate()
                                .child(subtitle),
                        )
                    }),
            )
    }

    fn browse_section(
        &self,
        index: usize,
        section: &Section,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body = match section.layout {
            Layout::Tiles => div()
                .flex()
                .flex_col()
                .gap_3()
                .when(!section.title.is_empty(), |body| {
                    body.child(self.section_heading(section, None, palette, cx))
                })
                .child(self.tile_grid(&section.cards, "browse-tile", card_side, palette, cx)),
            _ => {
                let show_all =
                    (section.has_more(SHELF_CARDS) && !section.uri.is_empty()).then(|| {
                        Page::Section {
                            uri: section.uri.clone(),
                            source: SectionSource::Browse,
                        }
                    });
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.section_heading(section, show_all, palette, cx))
                    .child(
                        self.card_grid(&section.cards, "browse-card", card_side, palette, cx)
                            .max_h(card_side + px(CARD_TEXT_HEIGHT))
                            .overflow_hidden(),
                    )
            }
        };
        div()
            .id(("browse-section", index))
            .child(body)
            .into_any_element()
    }

    pub fn section_page(
        &self,
        uri: &str,
        source: SectionSource,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.discovery.sections.get(uri, Some(source));
        let pending = state.is_none_or(|state| state.pending.is_some());
        let error = state.and_then(|state| state.error.clone());
        let loaded = state.and_then(|state| state.section.as_ref());
        let known = loaded.or_else(|| self.discovery.known_section(uri, source));
        let title = known
            .map(|section| section.title.clone())
            .filter(|title| !title.is_empty());
        let subtitle = known
            .map(|section| section.subtitle.clone())
            .filter(|subtitle| !subtitle.is_empty());
        let page = div()
            .flex()
            .flex_col()
            .gap_6()
            .child(div().child(self.back_button(palette, cx)))
            .map(|page| match title {
                Some(title) => page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(30.))
                                .font_weight(FontWeight::BOLD)
                                .truncate()
                                .child(title),
                        )
                        .when_some(subtitle, |header, subtitle| {
                            header.child(
                                div()
                                    .text_color(rgb(palette.muted))
                                    .truncate()
                                    .child(subtitle),
                            )
                        }),
                ),
                None if pending => page.child(
                    bone("section-title-bone", 0, palette)
                        .w(px(260.))
                        .h(px(30.))
                        .rounded(px(7.)),
                ),
                None => page,
            });
        let tiles = known.is_some_and(|section| section.layout == Layout::Tiles);
        let loading = |count: usize| {
            if tiles {
                self.tile_bones("section-tile-bones", count, card_side, palette)
            } else {
                card_bones("section-card-bones", count, card_side, palette)
            }
        };
        let Some(section) = loaded else {
            if pending {
                return page.child(loading(4)).child(loading(4)).into_any_element();
            }
            return page
                .child(self.page_status(
                    error.unwrap_or_else(|| "Couldn’t load this section. Try again.".to_owned()),
                    palette,
                ))
                .into_any_element();
        };
        let grid = if section.layout == Layout::Tiles {
            self.tile_grid(&section.cards, "section-tile", card_side, palette, cx)
        } else {
            self.card_grid(&section.cards, "section-card", card_side, palette, cx)
        };
        let paged = source == SectionSource::Browse && section.next.is_some();
        page.when(section.cards.is_empty() && !pending, |page| {
            page.child(self.page_status("Nothing to show here yet.", palette))
        })
        .child(grid)
        .when(paged, |page| {
            page.child(scroll_sentinel(&self.discovery.more_sentinel))
        })
        .when(pending, |page| page.child(loading(4)))
        .when_some(error, |page, error| {
            page.child(self.page_status(error, palette))
        })
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_keep_a_readable_height() {
        assert_eq!(tile_height(px(200.)), px(124.));
        assert_eq!(tile_height(px(110.)), px(TILE_MIN_HEIGHT));
    }
}
