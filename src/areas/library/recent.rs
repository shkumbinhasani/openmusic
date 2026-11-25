use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use gpui::{
    AnyElement, Context, Div, FontWeight, MouseButton, Pixels, Stateful, div, prelude::*, px, rgb,
    rgba,
};
use spotify_client::{
    Track,
    connect::PlayOptions,
    entity,
    library::{
        RecentDay, RecentEntry, RecentGroup, RecentKind, RecentsPage, context_uri, day_label,
        day_of, merge_played, recent_contexts, recent_days,
    },
};

use super::worker::Command;
use crate::{
    app::{OpenMusic, Page, page_for},
    icons,
    ui::{
        Palette,
        components::{
            ROW_RADIUS, Sentinel, artwork, explicit_badge, icon, scroll_sentinel, skeleton_rows,
        },
        menu::{Row, Surface, Target},
        tracks::now_ms,
    },
};

pub const RECENT_CONTEXTS: usize = 8;
pub const RECENT_ROW: &str = "recent-row";
const RECENT_GROUP: &str = "recent-row";
const FRESH_FOR: Duration = Duration::from_secs(120);
const ROW_HEIGHT: f32 = 72.;
const ART: f32 = 64.;
const CHILD_ART: f32 = 48.;
const CHILD_INDENT: f32 = 28.;
const STACK: usize = 3;

#[derive(Default)]
pub struct Recents {
    pub entries: Vec<RecentEntry>,
    pub days: Vec<RecentDay>,
    pub total: u32,
    pub next_offset: Option<u32>,
    pub loaded: bool,
    pub loading: bool,
    pub fallback: bool,
    pub error: Option<String>,
    pub request: u64,
    pub sentinel: Sentinel,
    pub expanded: HashSet<String>,
    pub fetched: Option<Instant>,
    pub stale: bool,
}

impl Recents {
    pub fn has_more(&self) -> bool {
        self.loaded && !self.fallback && self.next_offset.is_some()
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
        urls.extend(self.entries.iter().filter_map(|entry| entry.image.clone()));
    }

    pub fn take(&mut self, offset: u32, page: RecentsPage) {
        if offset == 0 || page.fallback {
            self.entries = page.entries;
            self.loaded = true;
            self.fetched = Some(Instant::now());
        } else {
            self.entries.extend(page.entries);
        }
        self.total = page.total;
        self.fallback = page.fallback;
        self.next_offset = page.next_offset.filter(|next| *next > offset);
        self.error = None;
        self.days = recent_days(&self.entries);
    }
}

#[derive(Default)]
pub struct Played {
    pub remote: Vec<RecentEntry>,
    pub local: Vec<RecentEntry>,
    pub entries: Vec<RecentEntry>,
    pub request: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub fetched: Option<Instant>,
    pub context: Option<String>,
}

impl Played {
    fn merge(&mut self) {
        self.entries = merge_played(self.local.iter().chain(&self.remote).cloned().collect());
    }

    fn touch(&mut self, uri: &str, at: i64) -> bool {
        let Some(mut entry) = self.entries.iter().find(|entry| entry.uri == uri).cloned() else {
            return false;
        };
        entry.played_at = Some(at);
        entry.day = Some(day_of(at));
        self.local.retain(|known| known.uri != uri);
        self.local.push(entry);
        self.merge();
        true
    }
}

fn worth_noting(uri: &str) -> bool {
    !matches!(
        RecentKind::from_uri(uri),
        RecentKind::Other | RecentKind::User
    )
}

fn track_of(entry: &RecentEntry) -> Track {
    Track {
        uri: entry.uri.clone(),
        title: entry.name.clone(),
        artist: entry.names(),
        cover: entry.image.clone(),
        artists: entry.contributors.clone(),
        artist_uri: entry
            .contributors
            .first()
            .map(|link| link.uri.clone())
            .filter(|uri| !uri.is_empty()),
        explicit: entry.explicit,
        ..Track::default()
    }
}

fn target_of(entry: &RecentEntry) -> Option<Target> {
    match entry.kind {
        RecentKind::Track => Some(Target::track(track_of(entry), None, Surface::Row)),
        _ => Target::for_uri(&entry.uri),
    }
}

fn playable(entry: &RecentEntry) -> bool {
    entry.kind.context() || matches!(entry.kind, RecentKind::Track | RecentKind::Episode)
}

impl OpenMusic {
    pub fn load_recents(&mut self) {
        let recent = &mut self.library.recent;
        recent.request += 1;
        recent.loading = true;
        recent.stale = false;
        let request = recent.request;
        self.send(Command::Recents { offset: 0, request });
    }

    pub fn more_recents(&mut self) {
        let recent = &mut self.library.recent;
        if recent.loading || recent.error.is_some() || !recent.has_more() {
            return;
        }
        let Some(offset) = recent.next_offset else {
            return;
        };
        recent.request += 1;
        recent.loading = true;
        let request = recent.request;
        self.send(Command::Recents { offset, request });
    }

    fn retry_recents(&mut self) {
        let recent = &mut self.library.recent;
        recent.error = None;
        if recent.loaded {
            self.more_recents();
        } else {
            self.load_recents();
        }
    }

    pub(super) fn on_recents(
        &mut self,
        request: u64,
        offset: u32,
        result: Result<RecentsPage, String>,
    ) {
        let recent = &mut self.library.recent;
        if request != recent.request {
            return;
        }
        recent.loading = false;
        match result {
            Ok(page) => recent.take(offset, page),
            Err(message) => recent.error = Some(message),
        }
    }

    pub fn load_played(&mut self) {
        let played = &mut self.library.played;
        played.request += 1;
        played.loading = true;
        let request = played.request;
        self.send(Command::Played { request });
    }

    fn sync_recent_contexts(&mut self) {
        self.library.recents = recent_contexts(&self.library.played.entries, RECENT_CONTEXTS);
    }

    pub(super) fn on_played(&mut self, request: u64, result: Result<Vec<RecentEntry>, String>) {
        let played = &mut self.library.played;
        if request != played.request {
            return;
        }
        played.loading = false;
        match result {
            Ok(entries) => {
                played.remote = entries;
                played.error = None;
                played.fetched = Some(Instant::now());
                played.merge();
                self.sync_recent_contexts();
            }
            Err(message) => played.error = Some(message),
        }
    }

    pub(super) fn on_played_context(&mut self, uri: String, entry: Option<RecentEntry>) {
        let Some(entry) = entry else { return };
        let played = &mut self.library.played;
        let uri = context_uri(&uri);
        played
            .local
            .retain(|known| known.uri != uri && known.uri != entry.uri);
        played.local.push(entry);
        played.merge();
        self.sync_recent_contexts();
    }

    pub fn note_played(&mut self, uri: &str) {
        let uri = context_uri(uri);
        if !worth_noting(&uri) {
            return;
        }
        let at = now_ms();
        if self.page != Page::Recent {
            self.library.recent.stale = true;
        }
        if self.library.played.touch(&uri, at) {
            self.sync_recent_contexts();
        } else {
            self.send(Command::PlayedContext { uri, at });
        }
    }

    pub(super) fn tick_recent(&mut self) -> bool {
        let context = self
            .playback
            .now
            .context_uri
            .clone()
            .filter(|uri| !uri.is_empty());
        if context != self.library.played.context {
            self.library.played.context = context.clone();
            if let Some(uri) = context {
                self.note_played(&uri);
            }
        }
        if self.library.recent.sentinel.take() {
            self.more_recents();
        }
        if self.page == Page::Recent {
            let recent = &self.library.recent;
            if recent.stale && !recent.loading {
                self.load_recents();
                return true;
            }
        }
        false
    }

    pub fn toggle_recent_group(&mut self, key: &str) {
        let expanded = &mut self.library.recent.expanded;
        if !expanded.remove(key) {
            expanded.insert(key.to_owned());
        }
    }

    fn recent_playing(&self, uri: &str) -> bool {
        self.playback_model()
            .and_then(|model| model.context_uri.as_deref())
            .or(self.playback.now.context_uri.as_deref())
            .is_some_and(|context| {
                context_uri(context) == uri
                    || (entity::is_liked_songs(context) && entity::is_liked_songs(uri))
            })
    }

    fn recent_current(&self, entry: &RecentEntry) -> bool {
        self.recent_playing(&entry.uri)
            || (entry.kind == RecentKind::Episode && self.is_current(&entry.uri))
    }

    fn play_recent(&mut self, entry: &RecentEntry, cx: &mut Context<Self>) {
        if self.recent_current(entry) {
            let playing = self.playback.now.playing;
            self.set_playing(!playing, cx);
            return;
        }
        let options = PlayOptions::titled(&entry.name);
        match entry.kind {
            RecentKind::Track => self.play(&[track_of(entry)], 0, cx),
            RecentKind::Episode => {
                let episode = [track_of(entry)];
                self.play_context_from(&entry.uri, &episode, 0, "recents", options, cx);
            }
            _ => self.play_context_from(&entry.uri, &[], 0, "recents", options, cx),
        }
    }

    fn open_recent(&mut self, entry: &RecentEntry, key: Option<&str>) {
        match page_for(&entry.uri) {
            Some(page) => self.navigate(page),
            None => {
                if let Some(key) = key {
                    self.toggle_recent_group(key);
                }
            }
        }
    }

    fn recent_art(&self, group: &RecentGroup, side: f32, palette: Palette) -> Div {
        let side_px = px(side);
        let radius = |kind: RecentKind, side: f32| {
            if kind.round() {
                px(side / 2.)
            } else {
                px((side * 0.12).clamp(4., 8.))
            }
        };
        let cover = |url: Option<&str>| url.and_then(|url| self.covers.get(url));
        if !group.stacked() {
            let item = &group.item;
            return artwork(
                cover(item.image.as_deref()),
                side_px,
                radius(item.kind, side),
                palette,
            );
        }
        let images = group.images();
        let tile = side * 0.7;
        let step = (side - tile) / (STACK as f32 - 1.);
        div()
            .size(side_px)
            .flex_shrink_0()
            .relative()
            .children((0..STACK).rev().map(|index| {
                let offset = px(step * index as f32);
                div()
                    .absolute()
                    .left(offset)
                    .top(offset)
                    .rounded(radius(RecentKind::Other, tile))
                    .shadow_sm()
                    .child(artwork(
                        cover(images.get(index).copied()),
                        px(tile),
                        radius(RecentKind::Other, tile),
                        palette,
                    ))
            }))
    }

    fn recent_play_button(
        &self,
        entry: &RecentEntry,
        side: f32,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if !playable(entry) || entry.uri.is_empty() {
            return None;
        }
        let current = self.recent_current(entry) && self.playback.now.playing;
        let target = entry.clone();
        let size = (side * 0.5).clamp(24., 32.);
        Some(
            div()
                .id("recent-play")
                .absolute()
                .top(px((side - size) / 2.))
                .left(px((side - size) / 2.))
                .size(px(size))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(palette.accent))
                .shadow_md()
                .cursor_pointer()
                .when(!current, |button| {
                    button
                        .opacity(0.)
                        .group_hover(RECENT_GROUP, |style| style.opacity(1.))
                })
                .child(icon(
                    if current { icons::PAUSE } else { icons::PLAY },
                    size * 0.45,
                    0xffffff,
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.play_recent(&target, cx);
                    cx.notify();
                })),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn recent_row(
        &self,
        row: Row,
        group: &RecentGroup,
        key: &str,
        child: bool,
        expanded: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let item = &group.item;
        let side = if child { CHILD_ART } else { ART };
        let title = group.title();
        let (added, subtitle) = group.subtitle();
        let playing = !item.uri.is_empty() && self.recent_playing(&item.uri);
        let has_children = !group.children.is_empty();
        let open_entry = item.clone();
        let menu_entry = item.clone();
        let open_key = key.to_owned();
        let toggle_key = key.to_owned();
        let stacked = group.stacked();
        div()
            .id(row)
            .group(RECENT_GROUP)
            .h(px(if child { ROW_HEIGHT - 12. } else { ROW_HEIGHT }))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_3()
            .pl(px(if child { 8. + CHILD_INDENT } else { 8. }))
            .pr_2()
            .rounded(px(ROW_RADIUS))
            .cursor_pointer()
            .when(self.menu_row(row), |line| line.bg(rgba(palette.press)))
            .hover(move |line| line.bg(rgba(palette.press)))
            .child(
                div()
                    .relative()
                    .flex_shrink_0()
                    .child(self.recent_art(group, side, palette))
                    .children(
                        (!stacked)
                            .then(|| self.recent_play_button(item, side, palette, cx))
                            .flatten(),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(14.))
                            .font_weight(FontWeight::MEDIUM)
                            .when(playing, |line| line.text_color(rgb(palette.accent)))
                            .child(title),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .min_w_0()
                            .when(added, |line| {
                                line.child(icon(icons::CHECK, 12., palette.accent))
                            })
                            .when(item.explicit && !stacked, |line| {
                                line.child(explicit_badge(palette))
                            })
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .text_color(rgb(palette.muted))
                                    .child(subtitle),
                            ),
                    ),
            )
            .when(has_children, |line| {
                line.child(
                    div()
                        .id("recent-expand")
                        .size(px(28.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .hover(move |button| button.bg(rgba(palette.line)))
                        .child(icon(
                            if expanded {
                                icons::CHEVRON_DOWN
                            } else {
                                icons::CHEVRON_RIGHT
                            },
                            12.,
                            palette.muted,
                        ))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_recent_group(&toggle_key);
                            cx.notify();
                        })),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                if stacked {
                    this.toggle_recent_group(&open_key);
                } else {
                    this.open_recent(&open_entry, has_children.then_some(open_key.as_str()));
                }
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Right,
                self.on_menu(Some(row), move |_| target_of(&menu_entry), cx),
            )
    }

    fn recent_message(
        &self,
        title: &'static str,
        body: Option<&'static str>,
        retry: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .py_6()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .children(body.map(|body| {
                div()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child(body)
            }))
            .when(retry, |message| {
                message.child(
                    div()
                        .id("recent-retry")
                        .mt_1()
                        .cursor_pointer()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.accent))
                        .child("Try again")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.retry_recents();
                            cx.notify();
                        })),
                )
            })
    }

    fn recent_day(
        &self,
        day: &RecentDay,
        now: i64,
        next_row: &mut usize,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut rows: Vec<AnyElement> = Vec::new();
        for group in &day.groups {
            let key = group.key(day.day);
            let expanded = self.library.recent.expanded.contains(&key);
            let row = (RECENT_ROW, *next_row);
            *next_row += 1;
            rows.push(
                self.recent_row(row, group, &key, false, expanded, palette, cx)
                    .into_any_element(),
            );
            if expanded {
                for child in &group.children {
                    let row = (RECENT_ROW, *next_row);
                    *next_row += 1;
                    let single = RecentGroup::single(child.clone());
                    rows.push(
                        self.recent_row(row, &single, &key, true, false, palette, cx)
                            .into_any_element(),
                    );
                }
            } else {
                *next_row += group.children.len();
            }
        }
        div()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                div()
                    .px_2()
                    .pb_1()
                    .text_size(px(16.))
                    .font_weight(FontWeight::BOLD)
                    .child(day_label(day.day, now)),
            )
            .children(rows)
    }

    pub fn recent_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let recent = &self.library.recent;
        let body = if !recent.loaded {
            match &recent.error {
                Some(_) if !recent.loading => {
                    self.recent_message("Something went wrong.", None, true, palette, cx)
                }
                _ => skeleton_rows("recent-rows", 8, false, palette),
            }
        } else if recent.days.is_empty() {
            self.recent_message(
                "Nothing here yet",
                Some("Music, podcasts and audiobooks you play or save show up here."),
                false,
                palette,
                cx,
            )
        } else {
            let now = now_ms();
            let mut next_row = 0;
            let days: Vec<Div> = recent
                .days
                .iter()
                .map(|day| self.recent_day(day, now, &mut next_row, palette, cx))
                .collect();
            div()
                .flex()
                .flex_col()
                .gap_6()
                .children(days)
                .child(scroll_sentinel(&recent.sentinel))
                .map(|list| match &recent.error {
                    Some(_) => list.child(self.recent_message(
                        "Couldn’t load more.",
                        None,
                        true,
                        palette,
                        cx,
                    )),
                    None if recent.loading && recent.has_more() => {
                        list.child(skeleton_rows("recent-more", 2, false, palette))
                    }
                    None => list,
                })
        };
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(div().child(self.back_button(palette, cx)))
            .child(
                div()
                    .text_size(px(34.))
                    .font_weight(FontWeight::BOLD)
                    .child("Recents"),
            )
            .child(body)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use spotify_client::library::{Activity, RecentsPage};

    use super::*;

    fn entry(uri: &str, kind: RecentKind, name: &str, at: i64) -> RecentEntry {
        RecentEntry {
            uri: uri.into(),
            kind,
            name: name.into(),
            played_at: Some(at),
            day: Some(day_of(at)),
            activity: Activity::Played,
            ..RecentEntry::default()
        }
    }

    #[test]
    fn keeps_recent_contexts_in_play_order() {
        let mut played = Played {
            remote: vec![
                entry("spotify:album:a", RecentKind::Album, "A", 300),
                entry("spotify:playlist:b", RecentKind::Playlist, "B", 200),
                entry("spotify:track:c", RecentKind::Track, "C", 100),
            ],
            ..Played::default()
        };
        played.merge();
        assert!(played.touch("spotify:playlist:b", 900));
        assert!(!played.touch("spotify:artist:z", 950));
        let names: Vec<&str> = played.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["B", "A", "C"]);
        let contexts = recent_contexts(&played.entries, RECENT_CONTEXTS);
        assert_eq!(contexts.len(), 2);
        played.remote = vec![entry("spotify:album:a", RecentKind::Album, "A", 1000)];
        played.merge();
        let names: Vec<&str> = played.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["A", "B"]);
        assert!(worth_noting("spotify:album:a"));
        assert!(worth_noting("spotify:user:someone:collection"));
        assert!(!worth_noting("spotify:station:album:a"));
        assert!(!worth_noting("spotify:user:someone"));
    }

    #[test]
    fn pages_the_recents_list() {
        let mut recent = Recents::default();
        assert!(recent.wants_reload());
        recent.take(
            0,
            RecentsPage {
                entries: vec![entry("spotify:album:a", RecentKind::Album, "A", 300)],
                total: 2,
                next_offset: Some(100),
                ..RecentsPage::default()
            },
        );
        assert!(recent.has_more() && !recent.wants_reload());
        recent.take(
            100,
            RecentsPage {
                entries: vec![entry("spotify:album:b", RecentKind::Album, "B", 200)],
                total: 2,
                offset: 100,
                next_offset: Some(100),
                ..RecentsPage::default()
            },
        );
        assert_eq!(recent.entries.len(), 2);
        assert!(!recent.has_more());
        recent.take(
            0,
            RecentsPage {
                entries: vec![entry("spotify:album:c", RecentKind::Album, "C", 300)],
                next_offset: Some(50),
                fallback: true,
                ..RecentsPage::default()
            },
        );
        assert!(recent.fallback && !recent.has_more());
        assert_eq!(recent.days.len(), 1);
    }

    #[test]
    fn rows_play_and_open_the_right_thing() {
        let song = entry("spotify:track:a", RecentKind::Track, "Song", 1);
        assert!(matches!(target_of(&song), Some(Target::Track(_))));
        assert_eq!(track_of(&song).title, "Song");
        let album = entry("spotify:album:a", RecentKind::Album, "Album", 1);
        assert_eq!(
            target_of(&album),
            Some(Target::Album {
                uri: "spotify:album:a".into()
            })
        );
        assert!(playable(&album) && playable(&song));
        let episode = entry("spotify:episode:a", RecentKind::Episode, "Talk", 1);
        assert!(playable(&episode));
        let chapter = entry("spotify:chapter:a", RecentKind::Chapter, "Part", 1);
        assert!(!playable(&chapter));
    }
}
