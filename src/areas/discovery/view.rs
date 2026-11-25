use std::sync::Arc;

use gpui::{AnyElement, Context, Div, FontWeight, MouseButton, Pixels, div, prelude::*, px, rgb};
use spotify_client::{
    Track,
    entity::{Card, Kind},
};

use super::state::{SearchTab, SectionSource};
use crate::{
    app::{OpenMusic, Page},
    icons,
    ui::{
        Palette,
        cards::card,
        components::{artwork, bone, skeleton_header, skeleton_rows},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Surface, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

impl OpenMusic {
    pub fn discover(
        &self,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mix = self.discovery.station();
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child("Listen Now"),
            )
            .when_some(mix, |page, mix| {
                let featured = &mix.tracks[0];
                let hero_track = featured.clone();
                let hero = div()
                    .flex()
                    .items_center()
                    .gap_6()
                    .on_mouse_down(
                        MouseButton::Right,
                        self.on_menu(
                            None,
                            move |_| Some(Target::track(hero_track.clone(), None, Surface::Header)),
                            cx,
                        ),
                    )
                    .child(
                        artwork(self.cover_for(featured), px(176.), px(16.), palette).shadow_lg(),
                    )
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
                                    .truncate()
                                    .w_full()
                                    .child(format!(
                                        "STATION · INSPIRED BY {}",
                                        mix.seed.to_uppercase()
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(28.))
                                    .font_weight(FontWeight::BOLD)
                                    .truncate()
                                    .w_full()
                                    .child(featured.title.clone()),
                            )
                            .child(
                                div()
                                    .text_color(rgb(palette.muted))
                                    .truncate()
                                    .w_full()
                                    .child(featured.artist.clone()),
                            )
                            .child(
                                div().mt_3().child(
                                    self.text_control("play-station", "Play", true, palette, cx)
                                        .w(px(96.)),
                                ),
                            ),
                    );
                let page = page.child(hero);
                if mix.tracks.len() > 1 {
                    page.child(self.mix_tracks(&mix.tracks, card_side, palette, cx))
                } else {
                    page
                }
            })
            .when(!self.discovery.home.tracks.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child(if self.discovery.home.from_top_tracks {
                                    "In Your Rotation"
                                } else {
                                    "From Your Liked Songs"
                                }),
                        )
                        .child(self.rows(
                            &self.discovery.home.tracks,
                            "home-track",
                            0,
                            Some("spotify:collection:tracks"),
                            palette,
                            cx,
                        )),
                )
            })
            .when(
                mix.is_none() && self.discovery.home.tracks.is_empty(),
                |page| {
                    if self.discovery.discovery_pending {
                        page.child(skeleton_header("home-hero", false, palette))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        bone("home-title", 0, palette)
                                            .w(px(220.))
                                            .h(px(20.))
                                            .rounded(px(5.)),
                                    )
                                    .child(div().flex().gap_4().children((0..4).map(|index| {
                                        div()
                                            .flex_1()
                                            .flex()
                                            .flex_col()
                                            .gap_2()
                                            .child(
                                                bone("home-card", index * 3, palette)
                                                    .size(card_side)
                                                    .rounded(px(12.)),
                                            )
                                            .child(
                                                bone("home-card", index * 3 + 1, palette)
                                                    .w(gpui::relative(0.7))
                                                    .h(px(12.))
                                                    .rounded(px(4.)),
                                            )
                                            .child(
                                                bone("home-card", index * 3 + 2, palette)
                                                    .w(gpui::relative(0.45))
                                                    .h(px(11.))
                                                    .rounded(px(4.)),
                                            )
                                    }))),
                            )
                            .child(skeleton_rows("home-rows", 4, false, palette))
                    } else {
                        page.child(
                            div()
                                .py_4()
                                .text_color(rgb(palette.muted))
                                .child("Play a song to get recommendations from Spotify."),
                        )
                    }
                },
            )
    }

    fn mix_tracks(
        &self,
        tracks: &[Track],
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let queue: Arc<[Track]> = tracks.into();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(19.))
                    .font_weight(FontWeight::BOLD)
                    .child("Up Next on This Station"),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .children(
                        tracks
                            .iter()
                            .enumerate()
                            .skip(1)
                            .take(4)
                            .map(|(index, track)| {
                                let queue = queue.clone();
                                let card_track = track.clone();
                                div()
                                    .id(("mix-card", index))
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.play(&queue, index, cx)
                                    }))
                                    .on_mouse_down(
                                        MouseButton::Right,
                                        self.on_menu(
                                            None,
                                            move |_| {
                                                Some(Target::track(
                                                    card_track.clone(),
                                                    None,
                                                    Surface::Card,
                                                ))
                                            },
                                            cx,
                                        ),
                                    )
                                    .child(
                                        artwork(self.cover_for(track), card_side, px(12.), palette)
                                            .shadow_md(),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .truncate()
                                                    .child(track.title.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(palette.muted))
                                                    .truncate()
                                                    .child(track.artist.clone()),
                                            ),
                                    )
                            }),
                    ),
            )
            .when(tracks.len() > 5, |section| {
                section.child(self.rows(tracks, "mix-track", 5, None, palette, cx))
            })
    }

    pub fn search_results(
        &self,
        _tab: SearchTab,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
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
                    .child(
                        div()
                            .text_color(rgb(palette.muted))
                            .truncate()
                            .child(format!("Results for “{}”", self.discovery.search_query)),
                    ),
            )
            .when(self.discovery.results.is_empty(), |page| {
                if self.discovery.search_pending {
                    return page.child(skeleton_rows("search-rows", 9, false, palette));
                }
                page.child(
                    div().py_4().text_color(rgb(palette.muted)).child(
                        self.discovery
                            .search_error
                            .clone()
                            .unwrap_or_else(|| "No songs found.".into()),
                    ),
                )
            })
            .child(self.rows(
                &self.discovery.results,
                "search-track",
                0,
                None,
                palette,
                cx,
            ))
    }

    pub fn artist_page(
        &self,
        uri: &str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)));
        let Some((loaded, result)) = self
            .discovery
            .artist
            .as_ref()
            .filter(|(loaded, _)| loaded == uri)
        else {
            return page
                .child(skeleton_header("artist-header", true, palette))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            bone("artist-title", 0, palette)
                                .w(px(110.))
                                .h(px(20.))
                                .rounded(px(5.)),
                        )
                        .child(skeleton_rows("artist-rows", 6, false, palette)),
                );
        };
        let artist = match result {
            Ok(artist) => artist,
            Err(error) => return page.child(self.page_status(error.clone(), palette)),
        };
        let image = artist.image.as_ref().and_then(|url| self.covers.get(url));
        let has_menu = self.has_menu(&Target::Artist {
            uri: loaded.clone(),
        });
        let header_uri = loaded.clone();
        let header = div()
            .flex()
            .items_center()
            .gap_6()
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(
                    None,
                    move |_| {
                        Some(Target::Artist {
                            uri: header_uri.clone(),
                        })
                    },
                    cx,
                ),
            )
            .child(
                div()
                    .size(px(176.))
                    .flex_shrink_0()
                    .rounded_full()
                    .overflow_hidden()
                    .shadow_lg()
                    .child(artwork(image, px(176.), px(88.), palette)),
            )
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
                            .child("ARTIST"),
                    )
                    .child(
                        div()
                            .text_size(px(40.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .w_full()
                            .child(artist.name.clone()),
                    )
                    .child(
                        div()
                            .mt_3()
                            .child(
                                self.text_control("play-artist", "Play", true, palette, cx)
                                    .w(px(96.)),
                            )
                            .when(has_menu, |actions| {
                                actions.flex().gap_2().child(self.menu_button(
                                    "more-artist",
                                    &artist.name,
                                    palette,
                                    cx,
                                ))
                            }),
                    ),
            );
        let releases: Vec<Card> = artist
            .releases
            .iter()
            .map(|release| {
                let subtitle = [
                    release.year.map(|year| year.to_string()),
                    Some(release.kind.clone()),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ");
                card(
                    &release.uri,
                    Kind::Album,
                    &release.name,
                    subtitle,
                    release.cover.clone(),
                )
            })
            .collect();
        let related: Vec<Card> = artist
            .related
            .iter()
            .map(|related| {
                card(
                    &related.uri,
                    Kind::Artist,
                    &related.name,
                    "Artist",
                    related.image.clone(),
                )
            })
            .collect();
        page.child(header)
            .when(!artist.top_tracks.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child("Popular"),
                        )
                        .child(self.rows(
                            &artist.top_tracks,
                            "artist-track",
                            0,
                            Some(loaded),
                            palette,
                            cx,
                        )),
                )
            })
            .when(!releases.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child("Releases"),
                        )
                        .child(self.card_grid(&releases, "release", card_side, palette, cx)),
                )
            })
            .when_some(artist.biography.clone(), |page, biography| {
                let short: String = biography.chars().take(900).collect();
                let short = if short.len() < biography.len() {
                    format!("{short}…")
                } else {
                    short
                };
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child("About"),
                        )
                        .child(
                            div()
                                .max_w(px(760.))
                                .text_size(px(14.))
                                .line_height(px(22.))
                                .text_color(rgb(palette.muted))
                                .child(short),
                        ),
                )
            })
            .when(!related.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child("Fans Also Like"),
                        )
                        .child(self.card_grid(&related, "related", card_side, palette, cx)),
                )
            })
    }
}

impl OpenMusic {
    pub fn browse_page(
        &self,
        _uri: &str,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn section_page(
        &self,
        _uri: &str,
        _source: SectionSource,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn search_suggestions(
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

pub fn nav_entries(app: &OpenMusic) -> Vec<NavEntry> {
    vec![
        NavEntry::new(
            "nav-home",
            100,
            icons::HOME,
            "Listen Now",
            app.page == Page::Home,
            |app, _, _| app.navigate(Page::Home),
        ),
        NavEntry::new(
            "nav-search",
            200,
            icons::SEARCH,
            "Search",
            matches!(app.page, Page::Search(_)),
            |app, window, cx| {
                if app.discovery.search_query.is_empty() {
                    app.input.update(cx, |input, cx| input.focus(window, cx));
                } else {
                    app.navigate(Page::Search(SearchTab::All));
                }
            },
        ),
    ]
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
