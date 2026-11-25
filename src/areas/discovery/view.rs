use gpui::{AnyElement, Context, FontWeight, MouseButton, Pixels, div, prelude::*, px, rgb};
use spotify_client::{
    Track,
    catalog::Artist,
    entity::{Card, Kind},
};

use super::{home::ARTIST_ORIGIN, search::SONG_ROWS, state::SearchTab};
use crate::{
    app::{OpenMusic, Page},
    icons,
    ui::{
        Palette,
        cards::{CardOptions, card},
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
        let releases = release_cards(artist);
        let related = related_cards(artist);
        let options = CardOptions::from(ARTIST_ORIGIN);
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
                        .child(self.card_grid_with(
                            &releases, "release", card_side, &options, palette, cx,
                        )),
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
                        .child(
                            self.card_grid_with(
                                &related, "related", card_side, &options, palette, cx,
                            ),
                        ),
                )
            })
    }
}

pub fn release_cards(artist: &Artist) -> Vec<Card> {
    artist
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
            .join(" • ");
            card(
                &release.uri,
                Kind::Album,
                &release.name,
                subtitle,
                release.cover.clone(),
            )
        })
        .collect()
}

pub fn related_cards(artist: &Artist) -> Vec<Card> {
    artist
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
        .collect()
}

impl OpenMusic {
    pub fn artist_cards(&self) -> Vec<Card> {
        match &self.discovery.artist {
            Some((_, Ok(artist))) => release_cards(artist)
                .into_iter()
                .chain(related_cards(artist))
                .collect(),
            _ => Vec::new(),
        }
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

#[cfg(test)]
mod tests {
    use spotify_client::catalog::{ArtistRef, Release};

    use super::*;

    #[test]
    fn turns_artist_releases_and_peers_into_cards() {
        let artist = Artist {
            uri: "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb".into(),
            name: "Radiohead".into(),
            image: None,
            biography: None,
            top_tracks: Vec::new(),
            releases: vec![
                Release {
                    uri: "spotify:album:6dVIqQ8qmQ5GBnJ9shOYGE".into(),
                    name: "OK Computer".into(),
                    year: Some(1997),
                    cover: Some("https://i.scdn.co/image/ok".into()),
                    kind: "Album".into(),
                },
                Release {
                    uri: "spotify:album:x".into(),
                    name: "Single".into(),
                    year: None,
                    cover: None,
                    kind: "Single".into(),
                },
            ],
            related: vec![ArtistRef {
                uri: "spotify:artist:y".into(),
                name: "Peer".into(),
                image: None,
            }],
        };
        let releases = release_cards(&artist);
        assert_eq!(releases[0].subtitle, "1997 • Album");
        assert_eq!(releases[0].kind, Kind::Album);
        assert_eq!(releases[1].subtitle, "Single");
        let related = related_cards(&artist);
        assert_eq!(related[0].kind, Kind::Artist);
        assert_eq!(related[0].subtitle, "Artist");
    }
}
