use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Pixels, SharedString, Stateful, anchored,
    deferred, div, point, prelude::*, px, relative, rgb, rgba,
};
use spotify_client::{
    entity::Kind,
    search::{Hit, SearchKind, SearchPage, SearchResults},
};

use super::{
    search::{DropRow, SONGS_ONLY, TOP_SONGS, top_result_plays},
    state::SearchTab,
};
use crate::{
    app::{OpenMusic, Page, page_for},
    icons,
    ui::{
        Palette,
        cards::{card_target, hero_subtitle, play_side, round_kind},
        components::{artwork, bone, explicit_badge, icon, link, scroll_sentinel, skeleton_rows},
    },
};

const DROPDOWN_WIDTH: f32 = 252.;
const DROPDOWN_HEIGHT: f32 = 420.;
const SIDEBAR_GAP: f32 = 16.;
const CARD_TEXT_HEIGHT: f32 = 50.;
const RECENT_CARDS: usize = 16;
const RECENT_GROUP: &str = "search-recent-card";
const SUGGESTION_GROUP: &str = "search-suggestion-row";

const SHELVES: [(SearchKind, &str); 9] = [
    (SearchKind::Artists, "search-artists"),
    (SearchKind::Albums, "search-albums"),
    (SearchKind::Playlists, "search-playlists"),
    (SearchKind::Podcasts, "search-podcasts"),
    (SearchKind::Episodes, "search-episodes"),
    (SearchKind::Audiobooks, "search-audiobooks"),
    (SearchKind::Authors, "search-authors"),
    (SearchKind::Users, "search-users"),
    (SearchKind::Genres, "search-genres"),
];

pub fn card_bones(key: &'static str, count: usize, card_side: Pixels, palette: Palette) -> Div {
    div().flex().gap_4().children((0..count).map(move |index| {
        div()
            .w(card_side)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                bone(key, index * 3, palette)
                    .size(card_side)
                    .rounded(px(12.)),
            )
            .child(
                bone(key, index * 3 + 1, palette)
                    .w(relative(0.7))
                    .h(px(12.))
                    .rounded(px(4.)),
            )
            .child(
                bone(key, index * 3 + 2, palette)
                    .w(relative(0.45))
                    .h(px(11.))
                    .rounded(px(4.)),
            )
    }))
}

pub fn heading_bone(key: &'static str, palette: Palette) -> Div {
    bone(key, 99, palette)
        .w(px(140.))
        .h(px(20.))
        .rounded(px(5.))
}

impl OpenMusic {
    pub fn paging_error(
        &self,
        key: &'static str,
        error: String,
        retry: impl Fn(&mut OpenMusic) -> bool + 'static,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .py_6()
            .flex()
            .items_center()
            .gap_3()
            .text_color(rgb(palette.muted))
            .child(div().min_w_0().child(error))
            .child(
                div()
                    .id(key)
                    .flex_shrink_0()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .hover(move |link| link.text_color(rgb(palette.text)).underline())
                    .child("Try again")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        retry(this);
                        cx.notify();
                    })),
            )
    }

    pub fn search_results(
        &self,
        tab: SearchTab,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if self.discovery.search_query.is_empty() {
            return self.search_landing(card_side, palette, cx);
        }
        let body = match tab.kind() {
            None => self.search_all(card_side, palette, cx),
            Some(kind) => self.search_tab_page(kind, card_side, palette, cx),
        };
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(self.search_header(palette))
            .child(self.search_chips(tab, palette, cx))
            .child(body)
            .into_any_element()
    }

    fn search_header(&self, palette: Palette) -> Div {
        let query = &self.discovery.search_query;
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child("Search"),
            )
            .when(!query.is_empty(), |header| {
                header.child(
                    div()
                        .text_color(rgb(palette.muted))
                        .truncate()
                        .child(format!("Results for “{query}”")),
                )
            })
    }

    fn search_chips(&self, tab: SearchTab, palette: Palette, cx: &mut Context<Self>) -> Div {
        let mut tabs = vec![SearchTab::All];
        if let Some(results) = self.discovery.search_results() {
            tabs.extend(results.chips.iter().copied().map(SearchTab::of));
        }
        if !tabs.contains(&tab) {
            tabs.push(tab);
        }
        let labels: Vec<SharedString> = tabs.iter().map(|tab| tab.label().into()).collect();
        let selected = tabs.iter().position(|shown| *shown == tab);
        self.chip_row(
            "search-chip",
            &labels,
            selected,
            move |app, index, _, _| {
                if let Some(tab) = tabs.get(index) {
                    app.navigate(Page::Search(*tab));
                }
            },
            palette,
            cx,
        )
    }

    fn search_heading(
        &self,
        title: impl Into<SharedString>,
        show_all: Option<Page>,
        prefix: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let heading = div()
            .id((prefix, usize::MAX))
            .min_w_0()
            .truncate()
            .text_size(px(19.))
            .font_weight(FontWeight::BOLD)
            .child(title.into());
        let heading = match show_all.clone() {
            Some(page) => heading
                .cursor_pointer()
                .hover(|heading| heading.underline())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.navigate(page.clone());
                    cx.notify();
                })),
            None => heading,
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(heading)
            .when_some(show_all, |row, page| {
                row.child(div().flex_shrink_0().child(link(
                    (prefix, usize::MAX - 1),
                    "Show all".to_owned(),
                    Some(page),
                    13.,
                    palette,
                    cx,
                )))
            })
    }

    fn search_all(
        &self,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let results = match &self.discovery.search {
            None => return self.search_skeleton(card_side, palette),
            Some(Err(error)) => {
                return self.page_status(error.clone(), palette).into_any_element();
            }
            Some(Ok(results)) => results,
        };
        if results.is_empty() {
            return self.search_nothing(palette).into_any_element();
        }
        if results.fallback {
            return self.search_songs_only(results, palette, cx);
        }
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(self.search_top_row(results, card_side, palette, cx))
            .when(!results.featured.is_empty(), |page| {
                let title = format!(
                    "Featuring {}",
                    results.featuring().unwrap_or(&self.discovery.search_query)
                );
                page.child(self.shelf_with(
                    title,
                    None,
                    &results.featured,
                    "search-featured",
                    card_side,
                    &self.search_card_options(),
                    palette,
                    cx,
                ))
            })
            .children(SHELVES.into_iter().filter_map(|(kind, prefix)| {
                let page = results
                    .cards_of(kind)
                    .filter(|page| !page.items.is_empty())?;
                let tab = SearchTab::of(kind);
                Some(self.shelf_with(
                    tab.shelf_title(),
                    Some(Page::Search(tab)),
                    &page.items,
                    prefix,
                    card_side,
                    &self.search_card_options(),
                    palette,
                    cx,
                ))
            }))
            .into_any_element()
    }

    fn search_top_row(
        &self,
        results: &SearchResults,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let wide = card_side >= px(178.);
        let top = results.top.first().map(|hit| {
            let play = top_result_plays(&hit.card).then_some("search-top-play");
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(self.search_heading("Top result", None, "search-top-title", palette, cx))
                .child(self.hero_card_with(
                    &hit.card,
                    "search-top",
                    play,
                    Some(self.search_card_open()),
                    palette,
                    cx,
                ))
        });
        let songs = &results.tracks.items[..results.tracks.items.len().min(TOP_SONGS)];
        let more_songs = results.tracks.total as usize > songs.len();
        let songs = (!songs.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(self.search_heading(
                    "Songs",
                    more_songs.then_some(Page::Search(SearchTab::Tracks)),
                    "search-songs",
                    palette,
                    cx,
                ))
                .child(self.track_list(
                    songs,
                    "search-track",
                    0,
                    &self.search_row_options(!wide),
                    palette,
                    cx,
                ))
        });
        if wide {
            div()
                .flex()
                .items_start()
                .gap_6()
                .children(top.map(|top| top.w(relative(0.4)).flex_shrink_0()))
                .children(songs.map(|songs| songs.flex_1().min_w_0()))
        } else {
            div()
                .flex()
                .flex_col()
                .gap_6()
                .children(top)
                .children(songs)
        }
    }

    fn search_songs_only(
        &self,
        results: &SearchResults,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child(SONGS_ONLY),
            )
            .child(self.search_heading("Songs", None, "search-songs", palette, cx))
            .child(self.track_list(
                &results.tracks.items,
                "search-track",
                0,
                &self.search_row_options(true),
                palette,
                cx,
            ))
            .into_any_element()
    }

    fn search_nothing(&self, palette: Palette) -> Div {
        div()
            .py_10()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_size(px(19.))
                    .font_weight(FontWeight::BOLD)
                    .text_center()
                    .child(format!(
                        "No results found for “{}”",
                        self.discovery.search_query
                    )),
            )
            .child(
                div()
                    .text_size(px(13.5))
                    .text_color(rgb(palette.muted))
                    .text_center()
                    .child(
                        "Please make sure your words are spelled correctly, or use fewer or different keywords.",
                    ),
            )
    }

    fn search_skeleton(&self, card_side: Pixels, palette: Palette) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(heading_bone("search-skeleton-top", palette))
                    .child(
                        bone("search-skeleton-hero", 0, palette)
                            .w(relative(0.4))
                            .min_w(px(260.))
                            .h(px(124.))
                            .rounded(px(16.)),
                    ),
            )
            .child(skeleton_rows("search-rows", 4, false, palette))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(heading_bone("search-skeleton-shelf", palette))
                    .child(card_bones("search-skeleton-cards", 4, card_side, palette)),
            )
            .into_any_element()
    }

    fn search_tab_page(
        &self,
        kind: SearchKind,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.discovery.search_tabs.get(&kind);
        let pending = state.is_none_or(|state| state.pending.is_some());
        let error = state.and_then(|state| state.error.clone());
        let loading = |count: usize| match kind {
            SearchKind::Tracks => skeleton_rows("search-tab-rows", count * 2, false, palette),
            _ => card_bones("search-tab-cards", count, card_side, palette),
        };
        let Some(items) = state.and_then(|state| state.items.as_ref()) else {
            if pending {
                return loading(4).into_any_element();
            }
            return self
                .paging_error(
                    "search-tab-retry",
                    error.unwrap_or_else(|| "Couldn’t load these results.".into()),
                    move |app| app.retry_search_page(kind),
                    palette,
                    cx,
                )
                .into_any_element();
        };
        if items.is_empty() {
            return self
                .page_status(
                    format!(
                        "No {} found for “{}”.",
                        SearchTab::of(kind).label().to_lowercase(),
                        self.discovery.search_query
                    ),
                    palette,
                )
                .into_any_element();
        }
        let list = match items {
            SearchPage::Tracks { page, .. } => self.track_list(
                &page.items,
                "search-tab-track",
                0,
                &self.search_row_options(true),
                palette,
                cx,
            ),
            SearchPage::Cards(page) => self.card_grid_with(
                &page.items,
                "search-tab-card",
                card_side,
                &self.search_card_options(),
                palette,
                cx,
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(list)
            .when(error.is_none(), |page| {
                page.child(scroll_sentinel(&self.discovery.search_sentinel))
            })
            .when(pending, |page| page.child(loading(2)))
            .when_some(error, |page, error| {
                page.child(self.paging_error(
                    "search-tab-retry",
                    error,
                    move |app| app.retry_search_page(kind),
                    palette,
                    cx,
                ))
            })
            .into_any_element()
    }

    fn search_landing(
        &self,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let recent = &self.discovery.recent;
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(self.search_header(palette))
            .when(!recent.items.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_4()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(19.))
                                        .font_weight(FontWeight::BOLD)
                                        .child("Recent searches"),
                                )
                                .child(
                                    div()
                                        .id("search-recent-clear")
                                        .flex_shrink_0()
                                        .text_size(px(13.))
                                        .text_color(rgb(palette.muted))
                                        .cursor_pointer()
                                        .hover(move |clear| {
                                            clear.text_color(rgb(palette.text)).underline()
                                        })
                                        .child("Clear recent searches")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.confirm_clear_recent_searches(window, cx);
                                        })),
                                ),
                        )
                        .child(self.recent_grid(card_side, palette, cx)),
                )
            })
            .when(recent.items.is_empty() && recent.pending, |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(heading_bone("search-recent-title", palette))
                        .child(card_bones("search-recent-bones", 4, card_side, palette)),
                )
            })
            .children(self.browse_all_section(card_side, palette, cx))
            .when(
                recent.items.is_empty()
                    && !recent.pending
                    && self.discovery.browse_all.all.is_none()
                    && !self.discovery.browse_all.pending,
                |page| {
                    page.child(
                        div()
                            .text_color(rgb(palette.muted))
                            .child("Search for songs, artists, albums, playlists, and podcasts. Press Return to see every result."),
                    )
                },
            )
            .into_any_element()
    }

    fn recent_grid(&self, card_side: Pixels, palette: Palette, cx: &mut Context<Self>) -> Div {
        let rows_height = (card_side + px(CARD_TEXT_HEIGHT)) * 2. + px(16.);
        let mut cards = Vec::new();
        for (index, hit) in self
            .discovery
            .recent
            .items
            .iter()
            .take(RECENT_CARDS)
            .enumerate()
        {
            cards.push(self.recent_card(index, hit, card_side, palette, cx));
        }
        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .max_h(rows_height)
            .overflow_hidden()
            .children(cards)
    }

    fn recent_card(
        &self,
        index: usize,
        hit: &Hit,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let card = &hit.card;
        let radius = if round_kind(card.kind) {
            card_side / 2.
        } else {
            px(12.)
        };
        let art = self.card_art(card, card_side, radius, palette);
        let play = self
            .play_bubble(
                RECENT_GROUP,
                card,
                &self.search_card_options(),
                play_side(card_side),
                palette,
                cx,
            )
            .map(|play| play.absolute().right(px(8.)).bottom(px(8.)));
        let open = hit.clone();
        let target = card_target(card);
        let uri = card.uri.clone();
        div()
            .id(("search-recent", index))
            .group(RECENT_GROUP)
            .w(card_side)
            .flex()
            .flex_col()
            .gap_2()
            .cursor_pointer()
            .when(!card.playable, |card| card.opacity(0.5))
            .child(
                div()
                    .relative()
                    .child(art.shadow_md())
                    .children(play)
                    .child(
                        div()
                            .id(("search-recent-remove", index))
                            .absolute()
                            .top(px(6.))
                            .right(px(6.))
                            .size(px(26.))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgba(0x000000a6))
                            .opacity(0.)
                            .group_hover(RECENT_GROUP, |remove| remove.opacity(1.))
                            .hover(|remove| remove.bg(rgba(0x000000e0)))
                            .child(icon(icons::CLOSE, 11., 0xffffff))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.forget_search(&uri);
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(card.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_w_0()
                            .when(card.explicit, |line| line.child(explicit_badge(palette)))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(rgb(palette.muted))
                                    .child(hero_subtitle(card)),
                            ),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                let page = page_for(open.uri());
                this.remember_search(open.clone());
                if let Some(page) = page {
                    this.navigate(page);
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(None, move |_| target.clone(), cx),
            )
    }

    pub fn search_suggestions(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.search_dropdown_visible() {
            return None;
        }
        let rows = self.discovery.drop_rows();
        let selected = self.discovery.suggest.index;
        let recent_mode = self.discovery.suggest.text.is_empty();
        let panel = div()
            .id("search-suggestions")
            .occlude()
            .w(px(DROPDOWN_WIDTH))
            .max_h(px(DROPDOWN_HEIGHT))
            .overflow_y_scroll()
            .p_1p5()
            .flex()
            .flex_col()
            .gap_0p5()
            .rounded(px(12.))
            .bg(rgb(palette.raised))
            .border_1()
            .border_color(rgba(palette.line))
            .shadow_lg()
            .text_color(rgb(palette.text))
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .when(recent_mode, |panel| {
                panel.child(
                    div()
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.muted))
                                .child("Recent searches"),
                        )
                        .child(
                            div()
                                .id("search-suggestions-clear")
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.accent))
                                .cursor_pointer()
                                .hover(|clear| clear.underline())
                                .child("Clear")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_clear_recent_searches(window, cx);
                                })),
                        ),
                )
            })
            .children(rows.into_iter().enumerate().map(|(index, row)| {
                self.drop_row(index, row, selected == Some(index), palette, cx)
            }));
        Some(
            div()
                .relative()
                .h_0()
                .mt(px(-SIDEBAR_GAP))
                .child(
                    deferred(
                        anchored()
                            .offset(point(px(0.), px(6.)))
                            .snap_to_window_with_margin(px(8.))
                            .child(panel),
                    )
                    .with_priority(1),
                )
                .into_any_element(),
        )
    }

    fn drop_row(
        &self,
        index: usize,
        row: DropRow,
        selected: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = if selected { 0xffffff } else { palette.muted };
        let base = div()
            .id(("search-suggestion", index))
            .group(SUGGESTION_GROUP)
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2p5()
            .px_2()
            .rounded(px(8.))
            .cursor_pointer()
            .when(selected, |row| {
                row.bg(rgb(palette.accent)).text_color(rgb(0xffffff))
            })
            .when(!selected, |row| {
                row.hover(|row| row.bg(rgba(palette.press)))
            });
        let activate = row.clone();
        let base = base.on_click(cx.listener(move |this, _, window, cx| {
            this.activate_drop_row(activate.clone(), window, cx);
        }));
        match row {
            DropRow::Query(text) => base
                .h(px(32.))
                .child(icon(icons::SEARCH, 13., muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(13.))
                        .child(text),
                )
                .into_any_element(),
            DropRow::Entity(hit) => base
                .h(px(44.))
                .child(self.drop_art(&hit, palette))
                .child(drop_text(&hit, muted))
                .into_any_element(),
            DropRow::Recent(hit) => {
                let uri = hit.uri().to_owned();
                base.h(px(44.))
                    .child(self.drop_art(&hit, palette))
                    .child(drop_text(&hit, muted))
                    .child(
                        div()
                            .id(("search-suggestion-remove", index))
                            .size(px(22.))
                            .flex_shrink_0()
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .opacity(if selected { 1. } else { 0. })
                            .group_hover(SUGGESTION_GROUP, |remove| remove.opacity(1.))
                            .hover(|remove| remove.bg(rgba(palette.press)))
                            .child(icon(icons::CLOSE, 10., muted))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.forget_search(&uri);
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            }
        }
    }

    fn drop_art(&self, hit: &Hit, palette: Palette) -> Div {
        let side = px(32.);
        let card = &hit.card;
        if card.kind == Kind::User {
            return self.avatar(&card.title, card.image.as_deref(), side, palette);
        }
        let image = card.image.as_ref().and_then(|url| self.covers.get(url));
        let radius = if round_kind(card.kind) {
            side / 2.
        } else {
            px(5.)
        };
        artwork(image, side, radius, palette)
    }
}

fn drop_text(hit: &Hit, muted: u32) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::MEDIUM)
                .truncate()
                .child(hit.card.title.clone()),
        )
        .child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(muted))
                .truncate()
                .child(hero_subtitle(&hit.card)),
        )
}
