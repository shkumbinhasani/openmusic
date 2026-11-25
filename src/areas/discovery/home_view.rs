use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Pixels, SharedString, Stateful, div,
    linear_color_stop, linear_gradient, prelude::*, px, rgb, rgba,
};
use spotify_client::{
    entity::{Card, Kind},
    home::{Layout, Section},
};

use super::{
    home::{ChipEntry, SHELF_CARDS, card_plays, chip_entries},
    search_view::{card_bones, heading_bone},
    state::SectionSource,
};
use crate::{
    app::{OpenMusic, Page, page_for},
    icons,
    ui::{
        Palette,
        cards::{card_target, round_kind},
        components::{artwork, bone, icon, link},
    },
};

pub const CARD_TEXT_HEIGHT: f32 = 50.;
const SHORTCUT_GROUP: &str = "home-shortcut";
const SHORTCUT_HEIGHT: f32 = 56.;
const WIDE_CARD: f32 = 200.;
const LIKED_FROM: u32 = 0x450af5;
const LIKED_TO: u32 = 0x8ed8b8;

pub fn hover_fill(palette: Palette) -> u32 {
    (palette.press & 0xffff_ff00) | ((palette.press & 0xff) + 0x14).min(0xff)
}

pub fn shortcut_columns(card_side: Pixels) -> usize {
    if card_side >= px(WIDE_CARD) { 4 } else { 2 }
}

impl OpenMusic {
    pub fn discover(
        &self,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let feed = &self.discovery.feed;
        let current = feed.current();
        let greeting = current
            .map(|(home, _)| home.greeting.clone())
            .filter(|greeting| !greeting.is_empty());
        let loading = current.is_none() && feed.error.is_none();
        let page = div().flex().flex_col().gap_6().map(|page| match greeting {
            Some(greeting) => page.child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .truncate()
                    .child(greeting),
            ),
            None if loading => page.child(
                bone("home-greeting", 0, palette)
                    .w(px(240.))
                    .h(px(30.))
                    .rounded(px(7.)),
            ),
            None => page.child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child("Home"),
            ),
        });
        let chips = chip_entries(feed.chips(), &feed.facet);
        let page = page.when(chips.len() > 1, |page| {
            page.child(self.home_chips(chips, palette, cx))
        });
        let Some((home, dimmed)) = current else {
            return match &feed.error {
                Some(error) => page.child(self.page_status(error.clone(), palette)),
                None => page.child(self.home_skeleton(card_side, palette)),
            }
            .into_any_element();
        };
        page.when(feed.refresh_failed, |page| {
            page.child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child("Couldn’t refresh Home."),
            )
        })
        .when(home.sections.is_empty(), |page| {
            page.child(self.page_status("Nothing to show here yet.", palette))
        })
        .child(
            div()
                .flex()
                .flex_col()
                .gap_8()
                .when(dimmed, |sections| sections.opacity(0.5))
                .children(home.sections.iter().enumerate().map(|(index, section)| {
                    self.home_section(index, section, card_side, palette, cx)
                })),
        )
        .into_any_element()
    }

    fn home_chips(&self, chips: Vec<ChipEntry>, palette: Palette, cx: &mut Context<Self>) -> Div {
        let labels: Vec<(SharedString, bool)> = chips
            .iter()
            .map(|chip| (chip.label.clone(), chip.selected))
            .collect();
        let ids: Vec<String> = chips.into_iter().map(|chip| chip.id).collect();
        self.chip_strip(
            "home-chip",
            &labels,
            move |app, index, _, _| {
                if let Some(id) = ids.get(index) {
                    app.pick_home_chip(id);
                }
            },
            palette,
            cx,
        )
    }

    fn home_section(
        &self,
        index: usize,
        section: &Section,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let body = match section.layout {
            Layout::Shortcuts => self.shortcuts(
                &section.cards,
                shortcut_columns(card_side),
                "home",
                palette,
                cx,
            ),
            Layout::Feed => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.section_heading(section, None, palette, cx))
                .child(self.card_grid(&section.cards, "home-card", card_side, palette, cx)),
            Layout::Shelf | Layout::Tiles => {
                let show_all =
                    (section.has_more(SHELF_CARDS) && !section.uri.is_empty()).then(|| {
                        Page::Section {
                            uri: section.uri.clone(),
                            source: SectionSource::Home,
                        }
                    });
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.section_heading(section, show_all, palette, cx))
                    .child(
                        self.card_grid(&section.cards, "home-card", card_side, palette, cx)
                            .max_h(card_side + px(CARD_TEXT_HEIGHT))
                            .overflow_hidden(),
                    )
            }
        };
        div()
            .id(("home-section", index))
            .child(body)
            .into_any_element()
    }

    pub fn section_heading(
        &self,
        section: &Section,
        show_all: Option<Page>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let title = div()
            .id("section-title")
            .min_w_0()
            .truncate()
            .text_size(px(19.))
            .font_weight(FontWeight::BOLD)
            .child(section.title.clone());
        let title = match show_all.clone() {
            Some(page) => title
                .cursor_pointer()
                .hover(|title| title.underline())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.navigate(page.clone());
                    cx.notify();
                })),
            None => title,
        };
        let header = section.header.as_ref().map(|header| {
            let page = page_for(&header.uri);
            div()
                .id("section-header")
                .flex_shrink_0()
                .cursor_pointer()
                .child(self.card_art(header, px(40.), px(6.), palette))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(page) = page.clone() {
                        this.navigate(page);
                    }
                    cx.notify();
                }))
        });
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_w_0()
                    .children(header)
                    .child(div().flex().flex_col().min_w_0().child(title).when(
                        !section.subtitle.is_empty(),
                        |text| {
                            text.child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(palette.muted))
                                    .truncate()
                                    .child(section.subtitle.clone()),
                            )
                        },
                    )),
            )
            .when_some(show_all, |row, page| {
                row.child(div().flex_shrink_0().child(link(
                    "section-show-all",
                    "Show all".to_owned(),
                    Some(page),
                    13.,
                    palette,
                    cx,
                )))
            })
    }

    pub fn card_art(&self, card: &Card, side: Pixels, radius: Pixels, palette: Palette) -> Div {
        match card.kind {
            Kind::LikedSongs => div()
                .size(side)
                .flex_shrink_0()
                .rounded(radius)
                .flex()
                .items_center()
                .justify_center()
                .bg(linear_gradient(
                    135.,
                    linear_color_stop(rgb(LIKED_FROM), 0.),
                    linear_color_stop(rgb(LIKED_TO), 1.),
                ))
                .child(icon(icons::HEART_FILL, f32::from(side) * 0.4, 0xffffff)),
            Kind::User => self.avatar(&card.title, card.image.as_deref(), side, palette),
            kind => {
                let image = card.image.as_ref().and_then(|url| self.covers.get(url));
                let radius = if round_kind(kind) { side / 2. } else { px(6.) };
                artwork(image, side, radius, palette)
            }
        }
    }

    pub fn play_bubble(
        &self,
        group: &'static str,
        card: &Card,
        origin: &'static str,
        side: f32,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        let current = self.card_is_current(card);
        if !current && !card_plays(card) {
            return None;
        }
        let playing = current && self.playback.now.playing;
        let card = card.clone();
        let label = if playing {
            format!("Pause {}", card.title)
        } else {
            format!("Play {}", card.title)
        };
        Some(
            div()
                .id("card-play")
                .size(px(side))
                .flex_shrink_0()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(palette.accent))
                .shadow_md()
                .cursor_pointer()
                .when(!current, |bubble| {
                    bubble
                        .opacity(0.)
                        .group_hover(group, |bubble| bubble.opacity(1.))
                })
                .tooltip(move |window, cx| {
                    gpui_component::tooltip::Tooltip::new(label.clone()).build(window, cx)
                })
                .child(icon(
                    if playing { icons::PAUSE } else { icons::PLAY },
                    side * 0.42,
                    0xffffff,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.play_card(&card, origin, cx);
                    cx.notify();
                })),
        )
    }

    pub fn shortcuts(
        &self,
        cards: &[Card],
        columns: usize,
        origin: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let columns = columns.max(1);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(cards.chunks(columns).enumerate().map(|(row, chunk)| {
                div()
                    .flex()
                    .gap_2()
                    .children(chunk.iter().enumerate().map(|(column, card)| {
                        self.shortcut(row * columns + column, card, origin, palette, cx)
                            .into_any_element()
                    }))
                    .children(
                        (chunk.len()..columns).map(|_| div().flex_1().min_w_0().into_any_element()),
                    )
            }))
    }

    fn shortcut(
        &self,
        index: usize,
        card: &Card,
        origin: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let page = page_for(&card.uri);
        let target = card_target(card);
        div()
            .id(("home-shortcut", index))
            .group(SHORTCUT_GROUP)
            .flex_1()
            .min_w_0()
            .h(px(SHORTCUT_HEIGHT))
            .flex()
            .items_center()
            .gap_3()
            .pr_2()
            .rounded(px(8.))
            .overflow_hidden()
            .bg(rgba(palette.press))
            .hover(move |tile| tile.bg(rgba(hover_fill(palette))))
            .when(page.is_some(), |tile| tile.cursor_pointer())
            .when(!card.playable, |tile| tile.opacity(0.5))
            .child(self.card_art(card, px(SHORTCUT_HEIGHT), px(0.), palette))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .line_height(px(17.))
                    .font_weight(FontWeight::BOLD)
                    .line_clamp(2)
                    .child(card.title.clone()),
            )
            .children(self.play_bubble(SHORTCUT_GROUP, card, origin, 32., palette, cx))
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
    }

    fn home_skeleton(&self, card_side: Pixels, palette: Palette) -> Div {
        let columns = shortcut_columns(card_side);
        let rows = 8 / columns;
        div()
            .flex()
            .flex_col()
            .gap_8()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children((0..rows).map(move |row| {
                        div()
                            .flex()
                            .gap_2()
                            .children((0..columns).map(move |column| {
                                bone("home-shortcut-bone", row * columns + column, palette)
                                    .flex_1()
                                    .h(px(SHORTCUT_HEIGHT))
                                    .rounded(px(8.))
                            }))
                    })),
            )
            .children(
                [
                    ("home-skeleton-title-a", "home-skeleton-cards-a"),
                    ("home-skeleton-title-b", "home-skeleton-cards-b"),
                ]
                .into_iter()
                .map(move |(title, cards)| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(heading_bone(title, palette))
                        .child(card_bones(cards, 4, card_side, palette))
                }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widens_the_shortcut_grid_on_wide_windows() {
        assert_eq!(shortcut_columns(px(110.)), 2);
        assert_eq!(shortcut_columns(px(186.)), 2);
        assert_eq!(shortcut_columns(px(200.)), 4);
    }

    #[test]
    fn hover_fill_stays_in_range() {
        let dark = 0xffff_ff1f;
        assert_eq!(
            hover_fill(Palette {
                press: dark,
                ..test_palette()
            }),
            0xffff_ff33
        );
        assert_eq!(
            hover_fill(Palette {
                press: 0x0000_00f8,
                ..test_palette()
            }),
            0x0000_00ff
        );
    }

    fn test_palette() -> Palette {
        Palette {
            dark: true,
            canvas: 0,
            raised: 0,
            text: 0,
            muted: 0,
            accent: 0,
            line: 0,
            fill: 0,
            press: 0,
            rim: 0,
        }
    }
}
