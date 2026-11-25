use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Pixels, SharedString, Stateful, div,
    prelude::*, px, rgb, rgba,
};
use spotify_client::{
    Track,
    catalog::LyricLine,
    details::{TrackArtist, TrackPage},
};

use crate::{
    app::{OpenMusic, Page},
    ui::{
        Palette,
        components::{artwork, link, skeleton_header, skeleton_rows, timestamp},
        menu::{Surface, Target},
        tracks::RowOptions,
    },
};

use super::credits;

pub const ALBUM_ROWS: &str = "track-album";
pub const LYRICS_PREVIEW: usize = 420;
pub const TOP_TRACKS: usize = 5;
pub const OTHER_ARTISTS: usize = 5;
pub const CREDITS_SHOWN: usize = 6;
const OTHER_SHELVES: [&str; 10] = [
    "track-other-0",
    "track-other-1",
    "track-other-2",
    "track-other-3",
    "track-other-4",
    "track-other-5",
    "track-other-6",
    "track-other-7",
    "track-other-8",
    "track-other-9",
];

pub struct TrackView {
    pub uri: String,
    pub result: Option<Result<Box<TrackPage>, String>>,
    pub lyrics_open: bool,
    pub more_tracks: bool,
    pub all_artists: bool,
}

impl TrackView {
    pub fn new(uri: String) -> Self {
        Self {
            uri,
            result: None,
            lyrics_open: false,
            more_tracks: false,
            all_artists: false,
        }
    }

    pub fn page(&self) -> Option<&TrackPage> {
        self.result.as_ref()?.as_ref().ok().map(|page| &**page)
    }

    pub fn state(&self) -> &'static str {
        match &self.result {
            None => "loading",
            Some(Ok(_)) => "ready",
            Some(Err(_)) => "error",
        }
    }

    pub fn describe(&self) -> String {
        let mut lines = vec![format!("uri={} state={}", self.uri, self.state())];
        match &self.result {
            Some(Ok(page)) => lines.extend(describe_page(page)),
            Some(Err(message)) => lines.push(format!("error={message}")),
            None => {}
        }
        lines.join("\n")
    }
}

fn describe_page(page: &TrackPage) -> Vec<String> {
    let track = &page.track;
    let album = page.album.as_ref();
    let mut lines = vec![
        format!("title={}", track.title),
        format!(
            "artists={}",
            page.artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        format!(
            "album={} kind={}",
            album.map_or("", |album| album.name.as_str()),
            album.map_or("", |album| album.kind.as_str()),
        ),
        format!(
            "meta={}",
            header_meta(page)
                .into_iter()
                .map(|part| part.1)
                .collect::<Vec<_>>()
                .join(" • ")
        ),
        format!("playable={}", page.playable),
    ];
    if let Some(lyrics) = &page.lyrics {
        lines.push(format!(
            "lyrics={} preview={} provider={}",
            lyrics.lines.len(),
            lyrics_preview(&lyrics.lines, LYRICS_PREVIEW),
            lyrics.provider
        ));
    }
    if let Some(credits) = &page.credits {
        lines.push(format!(
            "credits={}",
            credits
                .groups
                .iter()
                .map(|group| group.people.len())
                .sum::<usize>()
        ));
    }
    lines.push(format!("recommended={}", page.recommended.len()));
    for artist in &page.artists {
        lines.push(format!(
            "artist {} top={} popular={} albums={} singles={} related={}",
            artist.uri,
            artist.top_tracks.len(),
            artist.popular.len(),
            artist.albums.len(),
            artist.singles.len(),
            artist.related.len()
        ));
    }
    lines.push(format!("album_tracks={}", page.album_tracks.len()));
    lines.extend(footer_lines(page));
    lines
}

pub fn lyrics_preview(lines: &[LyricLine], limit: usize) -> usize {
    let mut total = 0;
    for (index, line) in lines.iter().enumerate() {
        total += line.words.encode_utf16().count();
        if total > limit {
            return index;
        }
    }
    lines.len()
}

pub fn grouped_count(count: u64) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn from_label(kind: &str) -> &'static str {
    match kind {
        "SINGLE" => "From the single",
        "EP" => "From the EP",
        "COMPILATION" => "From the compilation",
        _ => "From the album",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Meta {
    Artist,
    Album,
    Plain,
}

fn header_meta(page: &TrackPage) -> Vec<(Meta, String)> {
    let mut parts = Vec::new();
    if let Some(artist) = page.artists.first() {
        parts.push((Meta::Artist, artist.name.clone()));
    }
    if let Some(album) = &page.album {
        parts.push((Meta::Album, album.name.clone()));
        if let Some(year) = album.date.as_ref().and_then(|date| date.year) {
            parts.push((Meta::Plain, year.to_string()));
        }
    }
    parts.push((Meta::Plain, timestamp(page.track.duration_ms)));
    if let Some(count) = page.playcount.filter(|count| *count > 0) {
        parts.push((Meta::Plain, grouped_count(count)));
    }
    parts
}

fn footer_lines(page: &TrackPage) -> Vec<String> {
    let album = page.album.as_ref();
    album
        .and_then(|album| album.date.as_ref()?.long())
        .into_iter()
        .chain(album.into_iter().flat_map(|album| album.copyrights.clone()))
        .chain(
            page.lyrics
                .as_ref()
                .filter(|lyrics| !lyrics.provider.is_empty())
                .map(|lyrics| format!("Lyrics provided by {}", lyrics.provider)),
        )
        .collect()
}

fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(px(19.))
        .font_weight(FontWeight::BOLD)
        .child(text.into())
}

fn muted(text: impl Into<SharedString>, size: f32, palette: Palette) -> Div {
    div()
        .flex_shrink_0()
        .text_size(px(size))
        .text_color(rgb(palette.muted))
        .child(text.into())
}

impl OpenMusic {
    fn track_toggle(
        &self,
        id: &'static str,
        label: &'static str,
        palette: Palette,
        cx: &mut Context<Self>,
        change: fn(&mut TrackView),
    ) -> Stateful<Div> {
        div()
            .id(id)
            .flex_shrink_0()
            .cursor_pointer()
            .text_size(px(13.))
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(palette.muted))
            .hover(move |toggle| toggle.text_color(rgb(palette.text)))
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(view) = this.details.track.as_mut() {
                    change(view);
                }
                cx.notify();
            }))
    }

    fn track_header(
        &self,
        uri: &str,
        page: &TrackPage,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let track = page.track.clone();
        let menu_track = track.clone();
        let playing = self.is_current(&track.uri) && self.playback.now.playing;
        let artist = page.artists.first();
        let album = page.album.as_ref();
        let mut meta = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1p5()
            .overflow_hidden();
        if let Some(artist) = artist {
            meta = meta.child(self.avatar(&artist.name, artist.image.as_deref(), px(24.), palette));
        }
        for (index, (kind, text)) in header_meta(page).into_iter().enumerate() {
            if index > 0 {
                meta = meta.child(muted("•", 13.5, palette));
            }
            meta = match kind {
                Meta::Artist => meta.child(div().flex().min_w_0().child(link(
                    ("track-header-artist", index),
                    text,
                    artist.map(|artist| Page::Artist(artist.uri.clone())),
                    13.5,
                    palette,
                    cx,
                ))),
                Meta::Album => meta.child(div().flex().min_w_0().child(link(
                    ("track-header-album", index),
                    text,
                    album.map(|album| Page::Collection(album.uri.clone())),
                    13.5,
                    palette,
                    cx,
                ))),
                Meta::Plain => meta.child(muted(text, 13.5, palette)),
            };
        }
        let actions = div()
            .mt_3()
            .flex()
            .items_center()
            .gap_2()
            .when(page.playable, |actions| {
                actions.child(
                    self.text_control(
                        "track-play",
                        if playing { "Pause" } else { "Play" },
                        true,
                        palette,
                        cx,
                    )
                    .w(px(96.)),
                )
            })
            .children(self.library_button(uri, palette, cx))
            .child(self.menu_button("more-track", &track.title, palette, cx));
        div()
            .flex()
            .items_center()
            .gap_6()
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(
                    None,
                    move |_| Some(Target::track(menu_track.clone(), None, Surface::Header)),
                    cx,
                ),
            )
            .child(artwork(self.cover_for(&track), px(184.), px(16.), palette).shadow_lg())
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
                            .child("SONG"),
                    )
                    .child(
                        div()
                            .text_size(px(34.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .w_full()
                            .child(track.title.clone()),
                    )
                    .child(meta)
                    .child(actions),
            )
    }

    fn track_lyrics(
        &self,
        view: &TrackView,
        page: &TrackPage,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let lyrics = page.lyrics.as_ref()?;
        let preview = lyrics_preview(&lyrics.lines, LYRICS_PREVIEW);
        let shown = if view.lyrics_open {
            lyrics.lines.len()
        } else {
            preview
        };
        let lines = lyrics.lines[..shown].iter().map(|line| {
            div()
                .min_h(px(12.))
                .text_size(px(15.))
                .line_height(px(24.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.text))
                .child(line.words.clone())
        });
        Some(
            div()
                .flex_grow()
                .flex_basis(px(360.))
                .min_w_0()
                .flex()
                .flex_col()
                .gap_3()
                .child(heading("Lyrics"))
                .child(div().flex().flex_col().gap_1().children(lines))
                .when(preview < lyrics.lines.len(), |block| {
                    block.child(div().flex().child(self.track_toggle(
                        "track-lyrics-more",
                        if view.lyrics_open {
                            "Show less"
                        } else {
                            "Show more"
                        },
                        palette,
                        cx,
                        |view| view.lyrics_open = !view.lyrics_open,
                    )))
                }),
        )
    }

    fn track_artist_cards(
        &self,
        artists: &[TrackArtist],
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        if artists.is_empty() {
            return None;
        }
        let cards = artists.iter().enumerate().map(|(index, artist)| {
            let page = Page::Artist(artist.uri.clone());
            let uri = artist.uri.clone();
            div()
                .id(("track-artist", index))
                .flex()
                .items_center()
                .gap_4()
                .p_2()
                .rounded(px(12.))
                .cursor_pointer()
                .hover(move |card| card.bg(rgba(palette.press)))
                .child(self.avatar(&artist.name, artist.image.as_deref(), px(72.), palette))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(palette.muted))
                                .child("Artist"),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(16.))
                                .font_weight(FontWeight::BOLD)
                                .child(artist.name.clone()),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.navigate(page.clone());
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    self.on_menu(None, move |_| Some(Target::Artist { uri: uri.clone() }), cx),
                )
        });
        Some(
            div()
                .flex_grow()
                .flex_basis(px(280.))
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .children(cards),
        )
    }

    fn track_credits(
        &self,
        page: &TrackPage,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let credits = page.credits.as_ref()?;
        let people: Vec<_> = credits
            .groups
            .iter()
            .flat_map(|group| &group.people)
            .take(CREDITS_SHOWN)
            .collect();
        if people.is_empty() {
            return None;
        }
        let uri = page.track.uri.clone();
        let title = page.track.title.clone();
        let cells = people.into_iter().enumerate().map(|(index, person)| {
            let cell = div()
                .id(("track-credit", index))
                .w(px(220.))
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .px_2()
                .py_1p5()
                .rounded(px(8.))
                .child(
                    div()
                        .truncate()
                        .text_size(px(14.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(person.name.clone()),
                )
                .when(!person.roles.is_empty(), |cell| {
                    cell.child(
                        div()
                            .truncate()
                            .text_size(px(12.5))
                            .text_color(rgb(palette.muted))
                            .child(credits::roles(&person.roles)),
                    )
                });
            match person.uri.clone() {
                Some(artist) => cell
                    .cursor_pointer()
                    .hover(move |cell| cell.bg(rgba(palette.press)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.navigate(Page::Artist(artist.clone()));
                        cx.notify();
                    })),
                None => cell,
            }
        });
        Some(
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
                        .child(heading("Credits"))
                        .child(
                            div()
                                .id("track-credits-all")
                                .flex_shrink_0()
                                .cursor_pointer()
                                .text_size(px(13.))
                                .text_color(rgb(palette.muted))
                                .hover(move |label| label.text_color(rgb(palette.text)).underline())
                                .child("Show all")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.show_credits(uri.clone(), title.clone(), window, cx);
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().flex().flex_wrap().gap_x_4().gap_y_1().children(cells)),
        )
    }

    fn track_songs(
        &self,
        view: &TrackView,
        page: &TrackPage,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let mut sections = Vec::new();
        if !page.recommended.is_empty() {
            sections.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(heading("Recommended"))
                            .child(muted("Based on this song", 13., palette)),
                    )
                    .child(self.track_list(
                        &page.recommended,
                        "track-recommended",
                        0,
                        &RowOptions::default(),
                        palette,
                        cx,
                    )),
            );
        }
        if let Some(artist) = page
            .artists
            .first()
            .filter(|artist| !artist.top_tracks.is_empty())
        {
            let tracks = &artist.top_tracks;
            let shown = if view.more_tracks {
                tracks.len()
            } else {
                tracks.len().min(TOP_TRACKS)
            };
            let options = RowOptions {
                context: Some(artist.uri.clone()),
                ..RowOptions::default()
            };
            sections.push(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(muted("Popular Tracks by", 13., palette))
                            .child(heading(artist.name.clone())),
                    )
                    .child(self.track_list(
                        &tracks[..shown],
                        "track-popular",
                        0,
                        &options,
                        palette,
                        cx,
                    ))
                    .when(tracks.len() > TOP_TRACKS, |section| {
                        section.child(div().flex().px_2().child(self.track_toggle(
                            "track-popular-more",
                            if view.more_tracks {
                                "Show less"
                            } else {
                                "See more"
                            },
                            palette,
                            cx,
                            |view| view.more_tracks = !view.more_tracks,
                        )))
                    }),
            );
        }
        sections
    }

    fn track_shelves(
        &self,
        view: &TrackView,
        page: &TrackPage,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<Div> {
        let mut shelves = Vec::new();
        let Some(first) = page.artists.first() else {
            return shelves;
        };
        let artist_page = Some(Page::Artist(first.uri.clone()));
        for (title, cards, prefix) in [
            (
                format!("Popular Releases by {}", first.name),
                &first.popular,
                "track-releases",
            ),
            (
                format!("Popular Albums by {}", first.name),
                &first.albums,
                "track-albums",
            ),
            (
                format!("Popular Singles and EPs by {}", first.name),
                &first.singles,
                "track-singles",
            ),
        ] {
            if !cards.is_empty() {
                shelves.push(self.shelf(
                    title,
                    artist_page.clone(),
                    cards,
                    prefix,
                    card_side,
                    palette,
                    cx,
                ));
            }
        }
        let others: Vec<&TrackArtist> = page.artists[1..]
            .iter()
            .filter(|artist| !artist.popular.is_empty())
            .collect();
        let shown = if view.all_artists {
            others.len()
        } else {
            others.len().min(OTHER_ARTISTS)
        };
        for (artist, prefix) in others[..shown].iter().zip(OTHER_SHELVES) {
            shelves.push(self.shelf(
                format!("Popular Releases by {}", artist.name),
                Some(Page::Artist(artist.uri.clone())),
                &artist.popular,
                prefix,
                card_side,
                palette,
                cx,
            ));
        }
        if others.len() > OTHER_ARTISTS {
            shelves.push(div().flex().child(self.track_toggle(
                "track-other-all",
                if view.all_artists {
                    "Show less"
                } else {
                    "See all artists"
                },
                palette,
                cx,
                |view| view.all_artists = !view.all_artists,
            )));
        }
        if !first.related.is_empty() {
            shelves.push(self.shelf(
                "Fans also like",
                None,
                &first.related,
                "track-related",
                card_side,
                palette,
                cx,
            ));
        }
        shelves
    }

    fn track_album(
        &self,
        page: &TrackPage,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let album = page.album.as_ref()?;
        let album_uri = album.uri.clone();
        let menu_uri = album.uri.clone();
        let track_uri = page.track.uri.clone();
        let cover = album.cover.as_ref().and_then(|url| self.covers.get(url));
        let header = div()
            .id("track-from-album")
            .flex()
            .items_center()
            .gap_4()
            .p_2()
            .rounded(px(12.))
            .cursor_pointer()
            .hover(move |card| card.bg(rgba(palette.press)))
            .when(!album.playable, |card| card.opacity(0.5))
            .child(artwork(cover, px(72.), px(8.), palette).shadow_md())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.muted))
                            .child(from_label(&album.kind)),
                    )
                    .child(heading(album.name.clone())),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.go_to_album(album_uri.clone(), track_uri.clone());
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(
                    None,
                    move |_| {
                        Some(Target::Album {
                            uri: menu_uri.clone(),
                        })
                    },
                    cx,
                ),
            );
        let options = RowOptions {
            numbered: true,
            show_album: false,
            context: Some(album.uri.clone()),
            ..RowOptions::default()
        };
        Some(div().flex().flex_col().gap_2().child(header).when(
            !page.album_tracks.is_empty(),
            |block| {
                block.child(self.track_list(
                    &page.album_tracks,
                    ALBUM_ROWS,
                    0,
                    &options,
                    palette,
                    cx,
                ))
            },
        ))
    }

    pub fn track_page(
        &self,
        uri: &str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)));
        let Some(view) = self.details.track.as_ref().filter(|view| view.uri == uri) else {
            return loading(page, palette);
        };
        let data = match &view.result {
            None => return loading(page, palette),
            Some(Err(message)) => {
                return page
                    .child(self.page_status(message.clone(), palette))
                    .into_any_element();
            }
            Some(Ok(data)) => data,
        };
        let lyrics = self.track_lyrics(view, data, palette, cx);
        let artists = self.track_artist_cards(&data.artists, palette, cx);
        let top = (lyrics.is_some() || artists.is_some()).then(|| {
            div()
                .flex()
                .flex_wrap()
                .items_start()
                .gap_8()
                .children(lyrics)
                .children(artists)
        });
        let footer = footer_lines(data);
        page.child(self.track_header(uri, data, palette, cx))
            .children(top)
            .children(self.track_credits(data, palette, cx))
            .children(self.track_songs(view, data, palette, cx))
            .children(self.track_shelves(view, data, card_side, palette, cx))
            .children(self.track_album(data, palette, cx))
            .when(!footer.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(footer.into_iter().map(|line| muted(line, 12., palette))),
                )
            })
            .into_any_element()
    }

    pub fn track_page_track(&self) -> Option<Track> {
        let Page::Track(uri) = &self.page else {
            return None;
        };
        self.details
            .track
            .as_ref()
            .filter(|view| view.uri == *uri)
            .and_then(TrackView::page)
            .map(|page| page.track.clone())
    }
}

fn loading(page: Div, palette: Palette) -> AnyElement {
    page.child(skeleton_header("track-header", false, palette))
        .child(skeleton_rows("track-rows", 5, false, palette))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use spotify_client::{
        catalog::Lyrics,
        details::{ReleaseDate, TrackAlbum},
    };

    use super::*;

    fn line(words: &str) -> LyricLine {
        LyricLine {
            start_ms: 0,
            words: words.into(),
            heading: false,
        }
    }

    #[test]
    fn previews_lyrics_up_to_four_hundred_twenty_characters() {
        let lines: Vec<LyricLine> = (0..12).map(|_| line(&"a".repeat(40))).collect();
        assert_eq!(lyrics_preview(&lines, LYRICS_PREVIEW), 10);
        let short: Vec<LyricLine> = (0..3).map(|_| line("short")).collect();
        assert_eq!(lyrics_preview(&short, LYRICS_PREVIEW), 3);
        assert_eq!(lyrics_preview(&[line(&"é".repeat(421))], LYRICS_PREVIEW), 0);
    }

    #[test]
    fn groups_play_counts_by_thousands() {
        assert_eq!(grouped_count(3_742_266_181), "3,742,266,181");
        assert_eq!(grouped_count(999), "999");
        assert_eq!(grouped_count(1000), "1,000");
        assert_eq!(grouped_count(0), "0");
    }

    #[test]
    fn names_the_release_a_song_comes_from() {
        assert_eq!(from_label("ALBUM"), "From the album");
        assert_eq!(from_label("SINGLE"), "From the single");
        assert_eq!(from_label("EP"), "From the EP");
        assert_eq!(from_label("COMPILATION"), "From the compilation");
    }

    #[test]
    fn describes_the_header_and_footer() {
        let page = TrackPage {
            track: Track {
                title: "Counting Stars".into(),
                duration_ms: 257_265,
                ..Track::placeholder("spotify:track:2tpWsVSb9UEmDRxAl1zhX1")
            },
            playable: true,
            playcount: Some(3_742_266_181),
            album: Some(TrackAlbum {
                uri: "spotify:album:20lOt6G8MHv8ZO7ViOmiP7".into(),
                name: "Native".into(),
                kind: "ALBUM".into(),
                date: Some(ReleaseDate {
                    iso: "2013-01-01T00:00:00Z".into(),
                    precision: "DAY".into(),
                    year: Some(2013),
                }),
                copyrights: vec!["© 2013 Mosley Music/Interscope Records".into()],
                ..TrackAlbum::default()
            }),
            artists: vec![TrackArtist {
                uri: "spotify:artist:5Pwc4xIPtQLFEnJriah9YJ".into(),
                name: "OneRepublic".into(),
                ..TrackArtist::default()
            }],
            lyrics: Some(Lyrics {
                synced: true,
                lines: vec![line("Lately, I've been, I've been losing sleep")],
                provider: "Musixmatch".into(),
            }),
            ..TrackPage::default()
        };
        assert_eq!(
            header_meta(&page)
                .into_iter()
                .map(|(_, text)| text)
                .collect::<Vec<_>>(),
            ["OneRepublic", "Native", "2013", "4:17", "3,742,266,181"]
        );
        assert_eq!(
            footer_lines(&page),
            [
                "January 1, 2013",
                "© 2013 Mosley Music/Interscope Records",
                "Lyrics provided by Musixmatch"
            ]
        );
        let mut view = TrackView::new(page.track.uri.clone());
        assert_eq!(view.state(), "loading");
        view.result = Some(Ok(Box::new(page)));
        let dump = view.describe();
        assert!(dump.contains("state=ready"));
        assert!(dump.contains("meta=OneRepublic • Native • 2013 • 4:17 • 3,742,266,181"));
        assert!(dump.contains("lyrics=1 preview=1 provider=Musixmatch"));
        view.result = Some(Err("Couldn’t find that song".into()));
        assert!(view.describe().ends_with("error=Couldn’t find that song"));
    }
}
