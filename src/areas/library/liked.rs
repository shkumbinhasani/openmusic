use std::{
    cmp::Ordering,
    collections::HashSet,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Pixels, Stateful, div, prelude::*, px, rgb,
    rgba,
};
use spotify_client::{
    Track,
    library::{ItemKind, LIKED_SONGS, LibraryItem, LikedPage, count_label},
};

use super::worker::{Command, LikedList};
use crate::{
    app::{OpenMusic, Page},
    icons, overlay,
    ui::{
        Palette,
        components::{Sentinel, icon, scroll_sentinel, skeleton_rows},
        menu::{MenuItem, Place, Target},
        tracks::RowOptions,
    },
};

pub const LIKED_ROW: &str = "liked-track";
pub const PLAY_KEY: &str = "liked-play";
pub const FIND_KEY: &str = "liked-find";
const SORT_ANCHOR: &str = "liked-sort";
const FRESH_FOR: Duration = Duration::from_secs(300);
const PLAY_BEFORE: usize = 100;
const PLAY_WINDOW: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Column {
    Title,
    Artist,
    Album,
    Added,
    Duration,
}

impl Column {
    pub const ALL: [Self; 5] = [
        Self::Title,
        Self::Artist,
        Self::Album,
        Self::Added,
        Self::Duration,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Artist => "Artist",
            Self::Album => "Album",
            Self::Added => "Date added",
            Self::Duration => "Duration",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Artist => "artist",
            Self::Album => "album",
            Self::Added => "added",
            Self::Duration => "duration",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|column| column.slug() == slug)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LikedSort {
    pub column: Column,
    pub descending: bool,
}

impl Default for LikedSort {
    fn default() -> Self {
        Self {
            column: Column::Added,
            descending: true,
        }
    }
}

impl LikedSort {
    pub fn server_order(self) -> bool {
        self == Self::default()
    }

    pub fn pick(self, column: Column) -> Self {
        if column == self.column {
            Self {
                column,
                descending: !self.descending,
            }
        } else {
            Self {
                column,
                descending: column == Column::Added,
            }
        }
    }
}

fn folded(text: &str) -> String {
    text.to_lowercase()
}

fn compare(a: &Track, b: &Track, column: Column) -> Ordering {
    match column {
        Column::Title => folded(&a.title).cmp(&folded(&b.title)),
        Column::Artist => folded(&a.artist)
            .cmp(&folded(&b.artist))
            .then_with(|| folded(&a.album).cmp(&folded(&b.album))),
        Column::Album => folded(&a.album)
            .cmp(&folded(&b.album))
            .then_with(|| folded(&a.artist).cmp(&folded(&b.artist))),
        Column::Added => a.added_at.cmp(&b.added_at),
        Column::Duration => a.duration_ms.cmp(&b.duration_ms),
    }
}

pub fn sorted(tracks: &[Track], sort: LikedSort) -> Vec<Track> {
    let mut shown = tracks.to_vec();
    if !sort.server_order() {
        shown.sort_by(|a, b| {
            let order = compare(a, b, sort.column);
            if sort.descending {
                order.reverse()
            } else {
                order
            }
        });
    }
    shown
}

#[derive(Default)]
pub struct Liked {
    pub tracks: Vec<Track>,
    pub shown: Vec<Track>,
    pub unplayable: HashSet<String>,
    pub total: u32,
    pub next_offset: u32,
    pub loaded: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub request: u64,
    pub sentinel: Sentinel,
    pub sort: LikedSort,
    pub stale: bool,
    pub fetched: Option<Instant>,
    pub fallback: Option<LikedList>,
}

impl Liked {
    pub fn has_more(&self) -> bool {
        self.loaded && self.next_offset < self.total
    }

    pub fn wants_reload(&self) -> bool {
        !self.loaded
            || self.stale
            || self.error.is_some()
            || self
                .fetched
                .is_none_or(|fetched| fetched.elapsed() >= FRESH_FOR)
    }

    pub fn covers(&self, urls: &mut HashSet<String>) {
        urls.extend(self.tracks.iter().filter_map(|track| track.cover.clone()));
    }

    fn reshow(&mut self) {
        self.shown = sorted(&self.tracks, self.sort);
    }

    pub fn take(&mut self, offset: u32, page: LikedPage) {
        self.total = page.total;
        if offset == 0 {
            self.tracks = page.tracks;
            self.unplayable = page.unplayable.into_iter().collect();
            self.next_offset = page.rows;
            self.loaded = true;
            self.fetched = Some(Instant::now());
        } else {
            let known: HashSet<String> =
                self.tracks.iter().map(|track| track.uri.clone()).collect();
            self.tracks.extend(
                page.tracks
                    .into_iter()
                    .filter(|track| !known.contains(&track.uri)),
            );
            self.unplayable.extend(page.unplayable);
            self.next_offset = offset + page.rows;
            if page.rows == 0 {
                self.total = offset;
            }
        }
        self.error = None;
        self.reshow();
    }

    pub fn forget(&mut self, gone: impl Fn(&str) -> bool) -> bool {
        let before = self.tracks.len();
        self.tracks.retain(|track| !gone(&track.uri));
        let removed = (before - self.tracks.len()) as u32;
        if removed == 0 {
            return false;
        }
        if let Some(list) = &mut self.fallback {
            *list = list
                .iter()
                .filter(|item| !gone(&item.uri))
                .cloned()
                .collect();
        }
        self.total = self.total.saturating_sub(removed);
        self.next_offset = self.next_offset.saturating_sub(removed);
        self.reshow();
        true
    }
}

impl OpenMusic {
    pub fn load_liked(&mut self) {
        let liked = &mut self.library.liked;
        liked.request += 1;
        liked.loading = true;
        liked.stale = false;
        let request = liked.request;
        self.send(Command::Liked {
            offset: 0,
            request,
            fallback: None,
        });
    }

    pub fn more_liked(&mut self) {
        let liked = &mut self.library.liked;
        if liked.loading || liked.error.is_some() || !liked.has_more() {
            return;
        }
        liked.request += 1;
        liked.loading = true;
        let command = Command::Liked {
            offset: liked.next_offset,
            request: liked.request,
            fallback: liked.fallback.clone(),
        };
        self.send(command);
    }

    fn retry_liked(&mut self) {
        let liked = &mut self.library.liked;
        liked.error = None;
        if liked.loaded {
            self.more_liked();
        } else {
            self.load_liked();
        }
    }

    pub(super) fn on_liked(
        &mut self,
        request: u64,
        offset: u32,
        result: Result<LikedPage, String>,
        fallback: Option<LikedList>,
    ) {
        if request != self.library.liked.request {
            return;
        }
        self.library.liked.loading = false;
        match result {
            Ok(page) => {
                for track in &page.tracks {
                    if !self.saved.pending.contains_key(&track.uri) {
                        self.saved.known.insert(track.uri.clone(), true);
                    }
                }
                if offset == 0 {
                    self.library.liked.fallback = fallback;
                }
                self.library.liked.take(offset, page);
            }
            Err(message) => self.library.liked.error = Some(message),
        }
    }

    pub fn set_liked_sort(&mut self, sort: LikedSort) {
        let liked = &mut self.library.liked;
        if liked.sort != sort {
            liked.sort = sort;
            liked.reshow();
        }
    }

    pub fn sort_liked_by(&mut self, column: Column) {
        let sort = self.library.liked.sort.pick(column);
        self.set_liked_sort(sort);
    }

    pub fn play_liked(&mut self, tracks: &[Track], index: usize, cx: &mut Context<Self>) {
        if self.library.liked.sort.server_order() {
            self.play_context(LIKED_SONGS, tracks, index, cx);
            return;
        }
        let start = index.saturating_sub(PLAY_BEFORE);
        let end = (start + PLAY_WINDOW).min(tracks.len());
        if index < end {
            self.play(&tracks[start..end], index - start, cx);
        }
    }

    pub fn play_all_liked(&mut self, cx: &mut Context<Self>) {
        let shown = self.library.liked.shown.clone();
        self.play_liked(&shown, 0, cx);
    }

    pub(super) fn tick_liked(&mut self) -> bool {
        let saved = &self.saved;
        let changed = self
            .library
            .liked
            .forget(|uri| saved.known.get(uri) == Some(&false) && !saved.pending.contains_key(uri));
        if self.library.liked.sentinel.take() {
            self.more_liked();
        }
        if self.page == Page::Liked {
            let liked = &self.library.liked;
            if !liked.sort.server_order() && liked.has_more() {
                self.more_liked();
            }
            let writing = self
                .saved
                .pending
                .keys()
                .any(|uri| uri.starts_with("spotify:track:"));
            let liked = &self.library.liked;
            if liked.stale && !liked.loading && !writing {
                self.load_liked();
            }
        }
        changed
    }

    pub fn open_liked_sort_menu(&mut self) {
        let sort = self.library.liked.sort;
        let mut items = vec![MenuItem::header("Sort by")];
        items.extend(Column::ALL.into_iter().map(|column| {
            let current = column == sort.column;
            let item = MenuItem::action(column.name(), move |app, _, cx| {
                app.sort_liked_by(column);
                cx.notify();
            })
            .checked(current);
            match (current, sort.descending) {
                (false, _) => item,
                (true, true) => item.symbol("arrow.down"),
                (true, false) => item.symbol("arrow.up"),
            }
        }));
        self.show_menu(items, Place::Under(SORT_ANCHOR), None);
    }

    fn liked_sort_button(&self, palette: Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(SORT_ANCHOR)
            .relative()
            .h(px(30.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1p5()
            .px_2p5()
            .rounded_full()
            .cursor_pointer()
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(palette.muted))
            .hover(move |button| button.bg(rgba(palette.press)))
            .child(self.library.liked.sort.column.name())
            .child(icon(icons::QUEUE, 13., palette.muted))
            .child(overlay::anchor(SORT_ANCHOR))
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.open_liked_sort_menu();
                cx.notify();
            }))
    }

    fn liked_column(
        &self,
        column: Column,
        label: Option<&'static str>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let sort = self.library.liked.sort;
        let active = sort.column == column;
        let color = if active { palette.text } else { palette.muted };
        div()
            .id(("liked-column", column as usize))
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(color))
            .hover(move |cell| cell.text_color(rgb(palette.text)))
            .map(|cell| match label {
                Some(label) => cell.child(div().min_w_0().truncate().child(label)),
                None => cell.child(icon(icons::CLOCK, 14., color)),
            })
            .when(active, |cell| {
                cell.child(icon(
                    if sort.descending {
                        icons::CHEVRON_DOWN
                    } else {
                        icons::CHEVRON_UP
                    },
                    10.,
                    palette.accent,
                ))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sort_liked_by(column);
                cx.notify();
            }))
    }

    fn liked_columns(&self, wide: bool, palette: Palette, cx: &mut Context<Self>) -> Div {
        div()
            .h(px(32.))
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .mb_1()
            .border_b_1()
            .border_color(rgba(palette.line))
            .child(div().w(px(40.)).flex_shrink_0())
            .child(
                self.liked_column(Column::Title, Some("Title"), palette, cx)
                    .flex_1(),
            )
            .child(div().w_1_4().flex().min_w_0().child(self.liked_column(
                Column::Album,
                Some("Album"),
                palette,
                cx,
            )))
            .when(wide, |row| {
                row.child(
                    div()
                        .w(px(112.))
                        .flex_shrink_0()
                        .flex()
                        .child(self.liked_column(Column::Added, Some("Date added"), palette, cx)),
                )
            })
            .child(div().w(px(28.)).flex_shrink_0())
            .child(
                div()
                    .w(px(48.))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    .child(self.liked_column(Column::Duration, None, palette, cx)),
            )
    }

    fn liked_empty(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        div()
            .py_8()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Songs you like will appear here"),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child("Save songs by tapping the heart icon."),
            )
            .child(div().mt_3().child(self.text_control(
                FIND_KEY,
                "Find songs",
                false,
                palette,
                cx,
            )))
    }

    fn liked_failed(&self, error: String, palette: Palette, cx: &mut Context<Self>) -> Div {
        div()
            .py_6()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(
                div()
                    .text_size(px(13.5))
                    .text_color(rgb(palette.muted))
                    .child(error),
            )
            .child(
                div()
                    .id("liked-retry")
                    .cursor_pointer()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.accent))
                    .child("Try again")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.retry_liked();
                        cx.notify();
                    })),
            )
    }

    pub fn liked_page(
        &self,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let liked = &self.library.liked;
        let wide = card_side >= px(150.);
        let item =
            self.library.item(LIKED_SONGS).cloned().unwrap_or_else(|| {
                LibraryItem::new(LIKED_SONGS, ItemKind::LikedSongs, "Liked Songs")
            });
        let count = if liked.loaded {
            count_label(u64::from(liked.total), "song", "songs")
        } else {
            item.detail.clone()
        };
        let owner = self
            .account()
            .map(|account| account.name.clone())
            .filter(|name| !name.trim().is_empty());
        let subtitle = owner
            .into_iter()
            .chain(Some(count).filter(|count| !count.is_empty()))
            .collect::<Vec<_>>()
            .join(" • ");
        let header = div()
            .flex()
            .items_center()
            .gap_6()
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(None, |_| Target::for_uri(LIKED_SONGS), cx),
            )
            .child(self.library_art(&item, px(184.), palette).shadow_lg())
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
                            .child("Playlist"),
                    )
                    .child(
                        div()
                            .text_size(px(34.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .w_full()
                            .child("Liked Songs"),
                    )
                    .child(
                        div()
                            .text_size(px(13.5))
                            .text_color(rgb(palette.muted))
                            .child(subtitle),
                    )
                    .child(
                        div()
                            .mt_3()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                self.text_control(PLAY_KEY, "Play", true, palette, cx)
                                    .w(px(96.)),
                            )
                            .child(div().flex_1())
                            .when(!liked.shown.is_empty(), |actions| {
                                actions.child(self.liked_sort_button(palette, cx))
                            }),
                    ),
            );
        let body = if !liked.loaded {
            match &liked.error {
                Some(_) if !liked.loading => self.liked_failed(
                    "Something went wrong while loading your songs.".into(),
                    palette,
                    cx,
                ),
                _ => skeleton_rows("liked-rows", 8, false, palette),
            }
        } else if liked.shown.is_empty() {
            self.liked_empty(palette, cx)
        } else {
            let options = RowOptions {
                context: Some(LIKED_SONGS.to_owned()),
                show_added: wide,
                draggable: true,
                on_click: Some(Rc::new(|app, tracks, index, _, cx| {
                    app.play_liked(tracks, index, cx)
                })),
                ..RowOptions::default()
            };
            div()
                .flex()
                .flex_col()
                .child(self.liked_columns(wide, palette, cx))
                .child(self.track_list(&liked.shown, LIKED_ROW, 0, &options, palette, cx))
                .child(scroll_sentinel(&liked.sentinel))
                .map(|list| match &liked.error {
                    Some(_) => list.child(self.liked_failed(
                        "Couldn’t load more songs.".into(),
                        palette,
                        cx,
                    )),
                    None if liked.has_more() => {
                        list.child(skeleton_rows("liked-more", 2, false, palette))
                    }
                    None => list,
                })
        };
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)))
            .child(header)
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(uri: &str, title: &str, artist: &str, added: i64, duration: u32) -> Track {
        Track {
            uri: uri.into(),
            title: title.into(),
            artist: artist.into(),
            album: format!("{title} album"),
            added_at: Some(added),
            duration_ms: duration,
            ..Track::default()
        }
    }

    fn sample() -> Vec<Track> {
        vec![
            track("spotify:track:c", "charlie", "Zed", 300, 90_000),
            track("spotify:track:a", "Alpha", "yan", 200, 300_000),
            track("spotify:track:b", "bravo", "Xu", 100, 120_000),
        ]
    }

    fn uris(tracks: &[Track]) -> Vec<&str> {
        tracks.iter().map(|track| track.uri.as_str()).collect()
    }

    #[test]
    fn sorts_liked_songs_on_the_client() {
        let tracks = sample();
        let server = LikedSort::default();
        assert_eq!(
            uris(&sorted(&tracks, server)),
            ["spotify:track:c", "spotify:track:a", "spotify:track:b"]
        );
        let title = server.pick(Column::Title);
        assert!(!title.descending);
        assert_eq!(
            uris(&sorted(&tracks, title)),
            ["spotify:track:a", "spotify:track:b", "spotify:track:c"]
        );
        let title_back = title.pick(Column::Title);
        assert!(title_back.descending);
        assert_eq!(
            uris(&sorted(&tracks, title_back)),
            ["spotify:track:c", "spotify:track:b", "spotify:track:a"]
        );
        let artist = server.pick(Column::Artist);
        assert_eq!(
            uris(&sorted(&tracks, artist)),
            ["spotify:track:b", "spotify:track:a", "spotify:track:c"]
        );
        let oldest = server.pick(Column::Added);
        assert!(!oldest.descending);
        assert_eq!(
            uris(&sorted(&tracks, oldest)),
            ["spotify:track:b", "spotify:track:a", "spotify:track:c"]
        );
        let longest = LikedSort {
            column: Column::Duration,
            descending: true,
        };
        assert_eq!(
            uris(&sorted(&tracks, longest)),
            ["spotify:track:a", "spotify:track:b", "spotify:track:c"]
        );
        assert_eq!(Column::from_slug("added"), Some(Column::Added));
        assert_eq!(Column::from_slug("nope"), None);
    }

    fn page(tracks: Vec<Track>, total: u32, rows: u32) -> LikedPage {
        LikedPage {
            tracks,
            total,
            rows,
            ..LikedPage::default()
        }
    }

    #[test]
    fn pages_and_forgets_liked_songs() {
        let mut liked = Liked::default();
        assert!(liked.wants_reload());
        let mut first = sample();
        first.truncate(2);
        liked.take(0, page(first, 4, 2));
        assert!(liked.loaded && liked.has_more() && !liked.wants_reload());
        let more = vec![
            track("spotify:track:a", "Alpha", "yan", 200, 300_000),
            track("spotify:track:d", "delta", "W", 50, 1),
        ];
        liked.take(2, page(more, 4, 2));
        assert_eq!(
            uris(&liked.tracks),
            ["spotify:track:c", "spotify:track:a", "spotify:track:d"]
        );
        assert_eq!(liked.next_offset, 4);
        assert!(!liked.has_more());
        assert!(liked.forget(|uri| uri == "spotify:track:a"));
        assert!(!liked.forget(|uri| uri == "spotify:track:z"));
        assert_eq!((liked.total, liked.next_offset), (3, 3));
        assert_eq!(uris(&liked.shown), ["spotify:track:c", "spotify:track:d"]);
        liked.take(3, page(Vec::new(), 9, 0));
        assert_eq!(liked.total, 3);
        liked.stale = true;
        assert!(liked.wants_reload());
    }

    #[test]
    fn forgets_unliked_songs_in_the_fallback_list_too() {
        let mut liked = Liked::default();
        let tracks = sample();
        liked.fallback = Some(
            tracks
                .iter()
                .map(|track| spotify_client::catalog::ContextItem {
                    uri: track.uri.clone(),
                    ..Default::default()
                })
                .collect(),
        );
        liked.take(0, page(tracks, 3, 3));
        assert!(liked.forget(|uri| uri == "spotify:track:a"));
        let left: Vec<&str> = liked
            .fallback
            .iter()
            .flat_map(|list| list.iter())
            .map(|item| item.uri.as_str())
            .collect();
        assert_eq!(left, ["spotify:track:c", "spotify:track:b"]);
        assert_eq!((liked.total, liked.next_offset), (2, 2));
    }
}
