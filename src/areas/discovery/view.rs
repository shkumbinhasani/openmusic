use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Pixels, SharedString, div, prelude::*, px,
    rgb,
};
use spotify_client::{Track, catalog::Artist};

use super::{
    artist::{
        ArtistView, MORE_KEY, PLAY_KEY, POPULAR_ROWS, ReleaseKind, SHELF_CARDS, about_text,
        popular_count, related_cards, release_cards, release_kinds,
    },
    home::ARTIST_ORIGIN,
    home_view::CARD_TEXT_HEIGHT,
    search::SONG_ROWS,
    state::SearchTab,
};
use crate::{
    app::{OpenMusic, Page},
    icons,
    ui::{
        Palette,
        cards::CardOptions,
        components::{artwork, bone, skeleton_header, skeleton_rows},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

impl OpenMusic {
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
                        .child(skeleton_rows("artist-rows", POPULAR_ROWS, false, palette)),
                );
        };
        let artist = match result {
            Ok(artist) => artist,
            Err(error) => return page.child(self.page_status(error.clone(), palette)),
        };
        let view = &self.discovery.artist_view;
        let view = (view.uri == *loaded).then_some(view);
        let more_tracks = view.is_some_and(|view| view.more_tracks);
        let filter = view.and_then(|view| view.filter);
        let all_releases = view.is_some_and(|view| view.all_releases);
        let all_related = view.is_some_and(|view| view.all_related);
        let about_open = view.is_some_and(|view| view.about_open);
        let options = CardOptions::from(ARTIST_ORIGIN);
        let popular = popular_count(artist.top_tracks.len(), more_tracks);
        let kinds = release_kinds(artist);
        let releases = release_cards(artist, filter);
        let related = related_cards(artist);
        page.child(self.artist_header(loaded, artist, palette, cx))
            .when(!artist.top_tracks.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(artist_heading("Popular"))
                        .child(self.track_list(
                            &artist.top_tracks[..popular],
                            "artist-track",
                            0,
                            &self.artist_row_options(loaded),
                            palette,
                            cx,
                        ))
                        .when(artist.top_tracks.len() > POPULAR_ROWS, |list| {
                            list.child(toggle_link(
                                "artist-more-tracks",
                                if more_tracks { "Show less" } else { "See more" },
                                palette,
                                cx,
                                |view| view.more_tracks = !view.more_tracks,
                            ))
                        }),
                )
            })
            .when(!artist.releases.is_empty(), |page| {
                let expandable = releases.len() > SHELF_CARDS;
                let grid =
                    self.card_grid_with(&releases, "release", card_side, &options, palette, cx);
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
                                .child(artist_heading("Discography"))
                                .when(expandable, |row| {
                                    row.child(toggle_link(
                                        "artist-more-releases",
                                        if all_releases {
                                            "Show less"
                                        } else {
                                            "Show all"
                                        },
                                        palette,
                                        cx,
                                        |view| view.all_releases = !view.all_releases,
                                    ))
                                }),
                        )
                        .when(kinds.len() > 1, |section| {
                            section.child(self.release_chips(&kinds, filter, palette, cx))
                        })
                        .child(if all_releases {
                            grid
                        } else {
                            grid.max_h(card_side + px(CARD_TEXT_HEIGHT))
                                .overflow_hidden()
                        }),
                )
            })
            .children(self.artist_videos_section(loaded, card_side, palette, cx))
            .when_some(artist.biography.clone(), |page, biography| {
                let (text, long) = about_text(&biography, about_open);
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(artist_heading("About"))
                        .child(
                            div()
                                .max_w(px(760.))
                                .text_size(px(14.))
                                .line_height(px(22.))
                                .text_color(rgb(palette.muted))
                                .child(text),
                        )
                        .when(long, |about| {
                            about.child(toggle_link(
                                "artist-more-about",
                                if about_open { "Show less" } else { "Show more" },
                                palette,
                                cx,
                                |view| view.about_open = !view.about_open,
                            ))
                        }),
                )
            })
            .when(!related.is_empty(), |page| {
                let grid =
                    self.card_grid_with(&related, "related", card_side, &options, palette, cx);
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
                                .child(artist_heading("Fans also like"))
                                .when(related.len() > SHELF_CARDS, |row| {
                                    row.child(toggle_link(
                                        "artist-more-related",
                                        if all_related { "Show less" } else { "Show all" },
                                        palette,
                                        cx,
                                        |view| view.all_related = !view.all_related,
                                    ))
                                }),
                        )
                        .child(if all_related {
                            grid
                        } else {
                            grid.max_h(card_side + px(CARD_TEXT_HEIGHT))
                                .overflow_hidden()
                        }),
                )
            })
    }

    fn artist_header(
        &self,
        uri: &str,
        artist: &Artist,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let image = artist.image.as_ref().and_then(|url| self.covers.get(url));
        let target = Target::Artist {
            uri: uri.to_owned(),
        };
        let has_menu = self.has_menu(&target);
        let play = if self.artist_is_playing(uri) {
            "Pause"
        } else {
            "Play"
        };
        div()
            .flex()
            .items_center()
            .gap_6()
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(None, move |_| Some(target.clone()), cx),
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
                            .flex()
                            .gap_2()
                            .child(
                                self.text_control(PLAY_KEY, play, true, palette, cx)
                                    .w(px(96.)),
                            )
                            .children(self.library_button(uri, palette, cx))
                            .when(has_menu, |actions| {
                                actions.child(self.menu_button(MORE_KEY, &artist.name, palette, cx))
                            }),
                    ),
            )
    }

    fn release_chips(
        &self,
        kinds: &[ReleaseKind],
        filter: Option<ReleaseKind>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let chips: Vec<(SharedString, bool)> = kinds
            .iter()
            .map(|kind| (kind.label().into(), filter == Some(*kind)))
            .collect();
        let kinds = kinds.to_vec();
        self.chip_strip(
            "artist-release",
            &chips,
            move |app, index, _, _| {
                if let Some(kind) = kinds.get(index) {
                    app.discovery.artist_view.pick(*kind);
                }
            },
            palette,
            cx,
        )
    }
}

fn artist_heading(title: &'static str) -> Div {
    div()
        .min_w_0()
        .truncate()
        .text_size(px(19.))
        .font_weight(FontWeight::BOLD)
        .child(title)
}

fn toggle_link(
    id: &'static str,
    text: &'static str,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
    flip: impl Fn(&mut ArtistView) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_shrink_0()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted))
        .cursor_pointer()
        .hover(move |link| link.text_color(rgb(palette.text)).underline())
        .child(text)
        .on_click(cx.listener(move |this, _, _, cx| {
            flip(&mut this.discovery.artist_view);
            cx.notify();
        }))
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
            "Home",
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
                let tab = match app.page {
                    Page::Search(tab) => tab,
                    _ => SearchTab::All,
                };
                app.navigate(Page::Search(tab));
                if app.discovery.search_query.is_empty() {
                    app.input.update(cx, |input, cx| input.focus(window, cx));
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

pub fn row_dimmed(app: &OpenMusic, track: &Track, row: Row, _options: &RowOptions) -> bool {
    SONG_ROWS.contains(&row.0) && app.discovery.is_unplayable(&track.uri)
}
