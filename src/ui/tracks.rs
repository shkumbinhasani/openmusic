use std::{
    rc::Rc,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, SharedString, Window, div, prelude::*, px,
    rgb, rgba,
};
use spotify_client::{Track, entity};

use super::{
    Palette,
    components::{artwork, explicit_badge, icon, link, timestamp},
    menu::{Row, Surface, Target},
};
use crate::{
    app::{OpenMusic, Page, page_for},
    areas::{account, audio, content, details, discovery, library, playback, playlists, system},
    icons,
    ui::gallery,
};

pub const ROW_GROUP: &str = "track-row";
pub const DIMMED_OPACITY: f32 = 0.45;

pub type Accessories =
    fn(&OpenMusic, &Track, Row, &RowOptions, Palette, &mut Context<OpenMusic>) -> Vec<AnyElement>;

pub const ACCESSORIES: &[Accessories] = &[
    account::view::row_accessories,
    audio::view::row_accessories,
    content::view::row_accessories,
    details::view::row_accessories,
    discovery::view::row_accessories,
    library::view::row_accessories,
    playback::view::row_accessories,
    playlists::view::row_accessories,
    system::view::row_accessories,
];

pub type Dimmed = fn(&OpenMusic, &Track, Row, &RowOptions) -> bool;

pub const DIMMED: &[Dimmed] = &[
    account::view::row_dimmed,
    audio::view::row_dimmed,
    content::view::row_dimmed,
    details::view::row_dimmed,
    discovery::view::row_dimmed,
    library::view::row_dimmed,
    playback::view::row_dimmed,
    playlists::view::row_dimmed,
    system::view::row_dimmed,
];

pub type RowClick =
    Rc<dyn Fn(&mut OpenMusic, &[Track], usize, &mut Window, &mut Context<OpenMusic>)>;

#[derive(Clone)]
pub struct RowOptions {
    pub numbered: bool,
    pub context: Option<String>,
    pub show_album: bool,
    pub show_added: bool,
    pub editable: bool,
    pub draggable: bool,
    pub menu_surface: Option<Surface>,
    pub on_click: Option<RowClick>,
}

impl Default for RowOptions {
    fn default() -> Self {
        Self {
            numbered: false,
            context: None,
            show_album: true,
            show_added: false,
            editable: false,
            draggable: false,
            menu_surface: Some(Surface::Row),
            on_click: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RowDrag {
    pub uris: Vec<String>,
    pub uids: Vec<Option<String>>,
    pub source: Row,
    pub context: Option<String>,
}

impl RowDrag {
    pub fn describe(&self) -> String {
        let uids: Vec<&str> = self
            .uids
            .iter()
            .map(|uid| uid.as_deref().unwrap_or("-"))
            .collect();
        format!(
            "uris={} uids={} source={}:{} context={}",
            self.uris.join(","),
            uids.join(","),
            self.source.0,
            self.source.1,
            self.context.as_deref().unwrap_or("-"),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowDrop {
    pub row: Row,
    pub after: bool,
}

struct RowGhost {
    title: SharedString,
    palette: Palette,
}

impl Render for RowGhost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(32.))
            .max_w(px(280.))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .rounded_full()
            .bg(rgb(self.palette.raised))
            .border_1()
            .border_color(rgba(self.palette.line))
            .shadow_lg()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(self.palette.text))
            .child(icon(icons::NOTE, 14., self.palette.accent))
            .child(div().min_w_0().truncate().child(self.title.clone()))
    }
}

const SECOND: i64 = 1000;
const MINUTE: i64 = 60 * SECOND;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

pub fn added_label(added_ms: i64, now_ms: i64) -> String {
    let age = (now_ms - added_ms).max(0);
    let ago = |count: i64, unit: &str| {
        let plural = if count == 1 { "" } else { "s" };
        format!("{count} {unit}{plural} ago")
    };
    match age {
        age if age < MINUTE => ago(age / SECOND, "second"),
        age if age < HOUR => ago(age / MINUTE, "minute"),
        age if age < DAY => ago(age / HOUR, "hour"),
        age if age < 7 * DAY => ago(age / DAY, "day"),
        age if age < 30 * DAY => ago(age / (7 * DAY), "week"),
        _ => civil_date(added_ms)
            .and_then(|iso| entity::short_date(&iso))
            .unwrap_or_default(),
    }
}

fn civil_date(ms: i64) -> Option<String> {
    let days = ms.div_euclid(DAY) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year > 0).then(|| format!("{year:04}-{month:02}-{day:02}"))
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn artist_links(track: &Track) -> Vec<(String, Option<Page>)> {
    if track.artists.is_empty() {
        return vec![(
            track.artist.clone(),
            track.artist_uri.clone().map(Page::Artist),
        )];
    }
    track
        .artists
        .iter()
        .map(|artist| (artist.name.clone(), page_for(&artist.uri)))
        .collect()
}

impl OpenMusic {
    pub fn rows(
        &self,
        tracks: &[Track],
        prefix: &'static str,
        start: usize,
        context: Option<&str>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let options = RowOptions {
            context: context.map(str::to_owned),
            ..RowOptions::default()
        };
        self.track_list(tracks, prefix, start, &options, palette, cx)
    }

    pub fn row_accessories(
        &self,
        track: &Track,
        row: Row,
        options: &RowOptions,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        ACCESSORIES
            .iter()
            .flat_map(|hook| hook(self, track, row, options, palette, cx))
            .collect()
    }

    pub fn row_dimmed(&self, track: &Track, row: Row, options: &RowOptions) -> bool {
        DIMMED.iter().any(|hook| hook(self, track, row, options))
    }

    pub fn drop_rows(
        &mut self,
        drag: &RowDrag,
        at: RowDrop,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = gallery::row_dropped(self, drag, at, window, cx)
            || account::state::row_dropped(self, drag, at, window, cx)
            || audio::state::row_dropped(self, drag, at, window, cx)
            || content::state::row_dropped(self, drag, at, window, cx)
            || details::state::row_dropped(self, drag, at, window, cx)
            || discovery::state::row_dropped(self, drag, at, window, cx)
            || library::state::row_dropped(self, drag, at, window, cx)
            || playback::state::row_dropped(self, drag, at, window, cx)
            || playlists::state::row_dropped(self, drag, at, window, cx)
            || system::state::row_dropped(self, drag, at, window, cx);
        cx.notify();
    }

    fn drop_zone(
        &self,
        row: Row,
        after: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let line = palette.accent;
        div()
            .absolute()
            .left_0()
            .right_0()
            .h_1_2()
            .map(|zone| if after { zone.bottom_0() } else { zone.top_0() })
            .drag_over::<RowDrag>(move |style, _, _, _| {
                if after {
                    style.border_b_2().border_color(rgb(line))
                } else {
                    style.border_t_2().border_color(rgb(line))
                }
            })
            .on_drop(cx.listener(move |this, drag: &RowDrag, window, cx| {
                this.drop_rows(drag, RowDrop { row, after }, window, cx);
            }))
    }

    pub fn track_list(
        &self,
        tracks: &[Track],
        prefix: &'static str,
        start: usize,
        options: &RowOptions,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let queue: Arc<[Track]> = tracks.into();
        let context: Option<Arc<str>> = options.context.as_deref().map(Into::into);
        let now = now_ms();
        div()
            .flex()
            .flex_col()
            .children(tracks.iter().enumerate().skip(start).map(|(index, track)| {
                let id = (prefix, index);
                let queue = queue.clone();
                let context = context.clone();
                let click = options.on_click.clone();
                let menu_queue = queue.clone();
                let menu_context = context.clone();
                let selected = self.is_current(&track.uri);
                let local = track.uri.starts_with("spotify:local:");
                let album_uri = track.album_uri.clone();
                let artists = artist_links(track);
                let last_artist = artists.len().saturating_sub(1);
                let dimmed = self.row_dimmed(track, id, options);
                div()
                    .id(id)
                    .group(ROW_GROUP)
                    .relative()
                    .when(dimmed, |row| row.opacity(DIMMED_OPACITY))
                    .h(px(56.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_2()
                    .rounded(px(10.))
                    .cursor_pointer()
                    .when(selected || self.menu_row(id), |row| {
                        row.bg(rgba(palette.press))
                    })
                    .when(details::view::row_highlighted(self, track), |row| {
                        row.bg(rgba(palette.accent << 8 | 0x33))
                    })
                    .hover(move |row| row.bg(rgba(palette.press)))
                    .on_click(
                        cx.listener(move |this, _, window, cx| match (&click, &context) {
                            (Some(click), _) => click(this, &queue, index, window, cx),
                            (None, Some(context)) => this.play_context(context, &queue, index, cx),
                            (None, None) => this.play(&queue, index, cx),
                        }),
                    )
                    .when_some(options.menu_surface, |row_element, surface| {
                        row_element.on_mouse_down(
                            MouseButton::Right,
                            self.on_menu(
                                Some(id),
                                move |_| {
                                    let track = menu_queue.get(index)?.clone();
                                    let context = menu_context.as_deref().map(str::to_owned);
                                    Some(Target::track(track, context, surface))
                                },
                                cx,
                            ),
                        )
                    })
                    .when(options.draggable, |row_element| {
                        let drag = RowDrag {
                            uris: vec![track.uri.clone()],
                            uids: vec![track.uid.clone()],
                            source: id,
                            context: options.context.clone(),
                        };
                        let title: SharedString = track.title.clone().into();
                        row_element.on_drag(drag, move |drag, _, _, cx| {
                            let title = if drag.uris.len() == 1 {
                                title.clone()
                            } else {
                                format!("{} songs", drag.uris.len()).into()
                            };
                            cx.new(|_| RowGhost { title, palette })
                        })
                    })
                    .map(|row| {
                        if options.numbered {
                            row.child(
                                div()
                                    .w(px(40.))
                                    .flex_shrink_0()
                                    .text_center()
                                    .text_size(px(13.))
                                    .text_color(rgb(if selected {
                                        palette.accent
                                    } else {
                                        palette.muted
                                    }))
                                    .child(if selected {
                                        "♪".to_owned()
                                    } else {
                                        (index + 1).to_string()
                                    }),
                            )
                        } else {
                            row.child(artwork(self.cover_for(track), px(40.), px(6.), palette))
                        }
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .when(selected, |title| title.text_color(rgb(palette.accent)))
                                    .truncate()
                                    .child(track.title.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .when(track.explicit, |line| {
                                        line.child(explicit_badge(palette))
                                    })
                                    .when(local, |line| {
                                        line.child(icon(icons::LOCAL, 14., palette.muted))
                                    })
                                    .child(div().flex().min_w_0().children(
                                        artists.into_iter().enumerate().map(
                                            |(position, (name, page))| {
                                                div()
                                                    .flex()
                                                    .min_w_0()
                                                    .child(link(
                                                        ("artist", position),
                                                        name,
                                                        page,
                                                        12.,
                                                        palette,
                                                        cx,
                                                    ))
                                                    .when(position < last_artist, |artist| {
                                                        artist.child(
                                                            div()
                                                                .flex_shrink_0()
                                                                .text_size(px(12.))
                                                                .text_color(rgb(palette.muted))
                                                                .child(", "),
                                                        )
                                                    })
                                            },
                                        ),
                                    )),
                            ),
                    )
                    .when(options.show_album, |row| {
                        row.child(div().w_1_4().flex().min_w_0().child(link(
                            ("album", 0usize),
                            track.album.clone(),
                            album_uri.map(Page::Collection),
                            12.5,
                            palette,
                            cx,
                        )))
                    })
                    .when(options.show_added, |row| {
                        row.child(
                            div()
                                .w(px(112.))
                                .flex_shrink_0()
                                .truncate()
                                .text_size(px(12.5))
                                .text_color(rgb(palette.muted))
                                .child(
                                    track
                                        .added_at
                                        .map(|added| added_label(added, now))
                                        .unwrap_or_default(),
                                ),
                        )
                    })
                    .children(self.row_accessories(track, id, options, palette, cx))
                    .child(
                        div()
                            .w(px(48.))
                            .text_size(px(12.5))
                            .text_color(rgb(palette.muted))
                            .text_right()
                            .child(timestamp(track.duration_ms)),
                    )
                    .when(options.editable, |row_element| {
                        row_element
                            .child(self.drop_zone(id, false, palette, cx))
                            .child(self.drop_zone(id, true, palette, cx))
                    })
            }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_764_000_000_000;

    #[test]
    fn labels_recent_additions_relative_to_now() {
        assert_eq!(added_label(NOW - 5 * SECOND, NOW), "5 seconds ago");
        assert_eq!(added_label(NOW - MINUTE, NOW), "1 minute ago");
        assert_eq!(added_label(NOW - 3 * HOUR - 10, NOW), "3 hours ago");
        assert_eq!(added_label(NOW - DAY, NOW), "1 day ago");
        assert_eq!(added_label(NOW - 6 * DAY, NOW), "6 days ago");
        assert_eq!(added_label(NOW - 15 * DAY, NOW), "2 weeks ago");
        assert_eq!(added_label(NOW + HOUR, NOW), "0 seconds ago");
    }

    #[test]
    fn labels_older_additions_with_their_date() {
        assert_eq!(added_label(1_625_150_017_000, NOW), "Jul 1, 2021");
        assert_eq!(added_label(951_782_400_000, NOW), "Feb 29, 2000");
        assert_eq!(added_label(0, NOW), "Jan 1, 1970");
    }

    #[test]
    fn falls_back_to_the_artist_line_without_artist_links() {
        let track = Track {
            artist: "Solo".into(),
            artist_uri: Some("spotify:artist:a".into()),
            ..Track::default()
        };
        assert_eq!(
            artist_links(&track),
            vec![(
                "Solo".to_owned(),
                Some(Page::Artist("spotify:artist:a".into()))
            )]
        );
        let track = Track {
            artists: vec![
                spotify_client::catalog::Link {
                    uri: "spotify:artist:a".into(),
                    name: "A".into(),
                },
                spotify_client::catalog::Link {
                    uri: "spotify:show:s".into(),
                    name: "S".into(),
                },
            ],
            ..track
        };
        assert_eq!(
            artist_links(&track),
            vec![
                (
                    "A".to_owned(),
                    Some(Page::Artist("spotify:artist:a".into()))
                ),
                ("S".to_owned(), Some(Page::Show("spotify:show:s".into()))),
            ]
        );
    }
}
