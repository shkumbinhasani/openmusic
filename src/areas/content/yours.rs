use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    rc::Rc,
};

use gpui::{AnyElement, Context, Div, FontWeight, Pixels, SharedString, div, prelude::*, px, rgb};
use spotify_client::podcast::{
    Episode,
    your_episodes::{self, Entry, Filter, Sort},
};

use super::{
    view::{RowIds, hairline, heading, kind_label, this_year},
    worker::Command,
};
use crate::{
    app::{OpenMusic, Page},
    icons, overlay,
    ui::{
        Palette,
        components::{Sentinel, icon, link, scroll_sentinel, skeleton_header, skeleton_rows},
        menu::{MenuItem, Place},
    },
};

const TILE_TOP: u32 = 0x056952;
const TILE_BOTTOM: u32 = 0x0c8a6a;
const TILE_MARK: u32 = 0x1ed760;
const ROWS: RowIds = RowIds {
    row: "your-episode",
    play: "your-episode-play",
    save: "your-episode-save",
    show: "your-episode-show",
};
pub const SORT_KEY: &str = "your-episodes-sort";
const FIND_PODCASTS: &str = "spotify:genre:podcasts-web";

type Undo = Rc<dyn Fn(&mut OpenMusic, &mut Context<OpenMusic>)>;

#[derive(Default)]
pub struct Yours {
    pub entries: Option<Result<Vec<Entry>, String>>,
    pub loading: bool,
    pub episodes: HashMap<String, Episode>,
    pub gone: HashSet<String>,
    pub decorating: bool,
    pub error: Option<String>,
    pub sort: Sort,
    pub filter: Option<Filter>,
    pub end: Sentinel,
}

impl Yours {
    pub fn entries(&self) -> &[Entry] {
        match &self.entries {
            Some(Ok(entries)) => entries,
            _ => &[],
        }
    }

    fn undecorated(&self) -> impl Iterator<Item = &Entry> {
        self.entries().iter().filter(|entry| {
            !self.episodes.contains_key(&entry.uri) && !self.gone.contains(&entry.uri)
        })
    }

    pub fn remaining(&self) -> usize {
        self.undecorated().count()
    }

    fn next_batch(&self) -> Vec<String> {
        self.undecorated()
            .take(your_episodes::BATCH)
            .map(|entry| entry.uri.clone())
            .collect()
    }

    fn arranged(&self) -> bool {
        self.sort != Sort::Added || self.filter.is_some()
    }

    pub fn covers(&self, urls: &mut HashSet<String>) {
        urls.extend(
            self.episodes
                .values()
                .filter_map(|episode| episode.cover.clone()),
        );
    }

    pub fn patch(&mut self, fresh: &Episode) {
        if let Some(episode) = self.episodes.get_mut(&fresh.uri) {
            episode.played = fresh.played;
            episode.availability = fresh.availability;
            if fresh.duration_ms > 0 {
                episode.duration_ms = fresh.duration_ms;
            }
        }
    }

    pub fn take_entries(&mut self, result: Result<Vec<Entry>, String>) {
        self.loading = false;
        match result {
            Ok(entries) => {
                let listed: HashSet<&str> =
                    entries.iter().map(|entry| entry.uri.as_str()).collect();
                self.episodes.retain(|uri, _| listed.contains(uri.as_str()));
                self.gone.retain(|uri| listed.contains(uri.as_str()));
                self.entries = Some(Ok(entries));
                self.error = None;
            }
            Err(message) => {
                if !matches!(self.entries, Some(Ok(_))) {
                    self.entries = Some(Err(message));
                }
            }
        }
    }

    pub fn take_decorated(
        &mut self,
        uris: Vec<String>,
        result: Result<Vec<Option<Episode>>, String>,
    ) {
        self.decorating = false;
        match result {
            Ok(found) => {
                for (uri, episode) in uris.into_iter().zip(found) {
                    match episode {
                        Some(episode) => {
                            self.episodes.insert(uri, episode);
                        }
                        None => {
                            self.gone.insert(uri);
                        }
                    }
                }
            }
            Err(message) => self.error = Some(message),
        }
    }
}

impl OpenMusic {
    pub fn load_your_episodes(&mut self) {
        if self.content.yours.loading {
            return;
        }
        self.content.yours.loading = true;
        self.send(Command::YourEpisodes);
    }

    pub fn decorate_your_episodes(&mut self) {
        let yours = &self.content.yours;
        if yours.decorating || yours.error.is_some() {
            return;
        }
        let batch = yours.next_batch();
        if batch.is_empty() {
            return;
        }
        self.content.yours.decorating = true;
        self.send(Command::DecorateEpisodes(batch));
    }

    pub fn retry_your_episodes(&mut self) {
        self.content.yours.error = None;
        if matches!(self.content.yours.entries, Some(Err(_))) {
            self.content.yours.entries = None;
            self.load_your_episodes();
        } else {
            self.decorate_your_episodes();
        }
    }

    pub(super) fn took_your_episodes(&mut self) {
        let listed: Vec<String> = self
            .content
            .yours
            .entries()
            .iter()
            .map(|entry| entry.uri.clone())
            .collect();
        for uri in listed {
            if !self.saved.known.contains_key(&uri) && !self.saved.pending.contains_key(&uri) {
                self.saved.known.insert(uri, true);
            }
        }
        self.decorate_your_episodes();
    }

    pub fn your_episode_list(&self) -> Vec<&Episode> {
        let yours = &self.content.yours;
        let listed: Vec<&Episode> = yours
            .entries()
            .iter()
            .filter(|entry| self.is_saved(&entry.uri) != Some(false))
            .filter_map(|entry| yours.episodes.get(&entry.uri))
            .collect();
        your_episodes::arrange(
            &listed,
            |episode| self.played(episode),
            yours.sort,
            yours.filter,
        )
    }

    pub fn your_episode_count(&self) -> usize {
        self.content
            .yours
            .entries()
            .iter()
            .filter(|entry| self.is_saved(&entry.uri) != Some(false))
            .count()
    }

    fn your_episodes_playing(&self) -> (bool, bool) {
        let here = self.track().is_some_and(|track| {
            self.content.yours.episodes.contains_key(&track.uri)
                && self.is_saved(&track.uri) != Some(false)
        });
        (
            here && self.playback.now.playing,
            here && !self.playback.now.playing,
        )
    }

    pub fn play_your_episode(&mut self, index: usize, cx: &mut Context<Self>) {
        let list = self.your_episode_list();
        let Some(episode) = list.get(index).filter(|episode| episode.playable()) else {
            return;
        };
        if self.is_current(&episode.uri) {
            self.toggle(cx);
            return;
        }
        let tracks: Vec<_> = list.iter().map(|episode| episode.as_track()).collect();
        self.refresh_current_episode();
        self.play_context(your_episodes::CONTEXT, &tracks, index, cx);
    }

    pub fn play_your_episodes(&mut self, cx: &mut Context<Self>) {
        match self.your_episodes_playing() {
            (true, _) => self.set_playing(false, cx),
            (_, true) => self.set_playing(true, cx),
            _ => {
                if let Some(index) = self
                    .your_episode_list()
                    .iter()
                    .position(|episode| episode.playable())
                {
                    self.play_your_episode(index, cx);
                }
            }
        }
    }

    pub fn set_your_sort(&mut self, sort: Sort) {
        self.content.yours.sort = sort;
        self.decorate_your_episodes();
    }

    pub fn set_your_filter(&mut self, filter: Option<Filter>) {
        self.content.yours.filter = filter;
        self.decorate_your_episodes();
    }

    pub fn open_your_sort_menu(&mut self) {
        let current = self.content.yours.sort;
        let mut items = vec![MenuItem::header("Sort by")];
        items.extend(Sort::ALL.into_iter().map(|sort| {
            MenuItem::action(sort.label(), move |app, _, _| app.set_your_sort(sort))
                .checked(sort == current)
        }));
        self.show_menu(items, Place::Under(SORT_KEY), None);
    }

    pub(super) fn your_episodes_tick(&mut self) -> bool {
        let yours = &self.content.yours;
        let wanted = yours.end.take() || yours.arranged();
        if wanted && !yours.decorating && yours.error.is_none() && yours.remaining() > 0 {
            self.decorate_your_episodes();
            return true;
        }
        false
    }

    pub(super) fn describe_your_episodes(&self, lines: &mut String) {
        let yours = &self.content.yours;
        let state = match &yours.entries {
            None => "loading".to_owned(),
            Some(Err(error)) => format!("error {error:?}"),
            Some(Ok(entries)) => format!("entries={}", entries.len()),
        };
        let _ = writeln!(
            lines,
            "yours {state} count={} decorated={} gone={} remaining={} loading={} decorating={} error={:?} sort={} filter={}",
            self.your_episode_count(),
            yours.episodes.len(),
            yours.gone.len(),
            yours.remaining(),
            yours.loading,
            yours.decorating,
            yours.error,
            yours.sort.name(),
            yours.filter.map_or("none", Filter::name),
        );
        for (index, episode) in self.your_episode_list().into_iter().enumerate() {
            let added = yours
                .entries()
                .iter()
                .find(|entry| entry.uri == episode.uri)
                .and_then(|entry| entry.added_ms)
                .map_or("-".to_owned(), |added| added.to_string());
            let _ = writeln!(
                lines,
                "  {index} {} added={added} {:?} {} saved={:?} video={} show={:?} {:?}",
                episode.uri,
                episode.availability,
                super::state::describe_played(self.played(episode)),
                self.is_saved(&episode.uri),
                episode.video,
                episode.show_name,
                episode.name,
            );
        }
    }

    pub fn your_episodes_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_6()
            .child(div().child(self.back_button(palette, cx)));
        let yours = &self.content.yours;
        match &yours.entries {
            None => {
                return page
                    .child(skeleton_header("your-episodes-header", false, palette))
                    .child(skeleton_rows("your-episodes-rows", 6, false, palette))
                    .into_any_element();
            }
            Some(Err(error)) => {
                return page
                    .child(self.your_episodes_header(palette, cx))
                    .child(self.retry_line("your-episodes-retry", error.clone(), palette, cx))
                    .into_any_element();
            }
            Some(Ok(_)) => {}
        }
        let count = self.your_episode_count();
        let page = page.child(self.your_episodes_header(palette, cx));
        if count == 0 {
            return page
                .child(
                    div()
                        .pt_4()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(heading("Add to Your Episodes"))
                        .child(
                            div()
                                .text_size(px(14.))
                                .text_color(rgb(palette.muted))
                                .child("Save episodes to this playlist by tapping the plus icon."),
                        )
                        .child(div().pt_2().flex().child(link(
                            "your-episodes-find",
                            "Find podcasts".to_owned(),
                            Some(Page::Browse(FIND_PODCASTS.to_owned())),
                            14.,
                            palette,
                            cx,
                        ))),
                )
                .into_any_element();
        }
        let filters: Vec<SharedString> = Filter::ALL
            .iter()
            .map(|filter| SharedString::from(filter.label()))
            .collect();
        let selected = yours
            .filter
            .and_then(|filter| Filter::ALL.iter().position(|each| *each == filter));
        let list = self.your_episode_list();
        let year = this_year();
        let mut rows = div().flex().flex_col();
        for (index, episode) in list.iter().enumerate() {
            if index > 0 {
                rows = rows.child(hairline(palette));
            }
            rows = rows.child(self.episode_row(
                ROWS,
                index,
                episode,
                true,
                move |app, cx| app.play_your_episode(index, cx),
                year,
                palette,
                cx,
            ));
        }
        let remaining = yours.remaining();
        let waiting = remaining > 0 && yours.error.is_none();
        let gone = yours.gone.len();
        page.child(self.chip_row(
            "your-episodes-filter",
            &filters,
            selected,
            |app, index, _, _| {
                let picked = Filter::ALL.get(index).copied();
                let filter = if app.content.yours.filter == picked {
                    None
                } else {
                    picked
                };
                app.set_your_filter(filter);
            },
            palette,
            cx,
        ))
        .when(list.is_empty() && !waiting, |page| {
            page.child(self.page_status(
                match yours.filter {
                    Some(filter) => format!("No {} episodes.", filter.label().to_lowercase()),
                    None => "These episodes aren’t available.".to_owned(),
                },
                palette,
            ))
        })
        .child(rows)
        .when(waiting && (yours.decorating || yours.arranged()), |page| {
            page.child(skeleton_rows("your-episodes-more", 3, false, palette))
        })
        .when_some(yours.error.clone(), |page, error| {
            page.child(self.retry_line("your-episodes-more-retry", error, palette, cx))
        })
        .when(waiting && !yours.decorating && !yours.arranged(), |page| {
            page.child(scroll_sentinel(&yours.end))
        })
        .when(gone > 0 && !waiting, |page| {
            page.child(
                div()
                    .text_size(px(12.5))
                    .text_color(rgb(palette.muted))
                    .child(match gone {
                        1 => "1 episode isn’t available.".to_owned(),
                        count => format!("{count} episodes aren’t available."),
                    }),
            )
        })
        .into_any_element()
    }

    fn your_episodes_header(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let count = self.your_episode_count();
        let (playing, paused) = self.your_episodes_playing();
        let playable = self
            .your_episode_list()
            .iter()
            .any(|episode| episode.playable());
        let label = if playing {
            "Pause"
        } else if paused {
            "Resume"
        } else {
            "Play"
        };
        let ready = matches!(self.content.yours.entries, Some(Ok(_))) && count > 0;
        div()
            .flex()
            .items_center()
            .gap_6()
            .child(your_episodes_tile(px(184.)).shadow_lg())
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1p5()
                    .child(kind_label("PLAYLIST", palette))
                    .child(
                        div()
                            .text_size(px(32.))
                            .line_height(px(38.))
                            .font_weight(FontWeight::BOLD)
                            .child("Your Episodes"),
                    )
                    .when(ready, |column| {
                        column.child(
                            div()
                                .text_size(px(13.))
                                .text_color(rgb(palette.muted))
                                .child(match count {
                                    1 => "1 episode".to_owned(),
                                    count => format!("{count} episodes"),
                                }),
                        )
                    })
                    .when(ready, |column| {
                        column.child(
                            div()
                                .mt_3()
                                .flex()
                                .items_center()
                                .gap_2()
                                .when(playing || paused || playable, |actions| {
                                    actions.child(
                                        self.text_control(
                                            "your-episodes-play",
                                            label,
                                            true,
                                            palette,
                                            cx,
                                        )
                                        .w(px(96.)),
                                    )
                                })
                                .child(
                                    self.text_control(
                                        SORT_KEY,
                                        self.content.yours.sort.label(),
                                        false,
                                        palette,
                                        cx,
                                    )
                                    .child(overlay::anchor(SORT_KEY)),
                                ),
                        )
                    }),
            )
    }

    fn retry_line(
        &self,
        id: &'static str,
        message: String,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        div().flex().child(
            div()
                .id(id)
                .py_2()
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(rgb(palette.muted))
                .hover(move |retry| retry.text_color(rgb(palette.text)).underline())
                .child(format!("{message} Try again."))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.retry_your_episodes();
                    cx.notify();
                })),
        )
    }
}

pub fn your_episodes_tile(side: Pixels) -> Div {
    div()
        .size(side)
        .flex_shrink_0()
        .rounded(px((f32::from(side) * 0.09).clamp(4., 16.)))
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::linear_gradient(
            135.,
            gpui::linear_color_stop(rgb(TILE_TOP), 0.),
            gpui::linear_color_stop(rgb(TILE_BOTTOM), 1.),
        ))
        .child(icon(icons::BOOKMARK, f32::from(side) * 0.36, TILE_MARK))
}

impl OpenMusic {
    pub fn change_saved(&mut self, uri: &str, saved: bool) {
        self.save_when_known(uri, saved);
        let listed =
            self.page == Page::YourEpisodes && self.content.yours.episodes.contains_key(uri);
        if saved || !listed {
            return;
        }
        let target = uri.to_owned();
        let undo: Undo = Rc::new(move |app, _| app.save_when_known(&target, true));
        self.show_toast("Removed from Your Episodes.", Some(undo));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Entry {
        Entry {
            uri: format!("spotify:episode:{id}"),
            added_ms: None,
        }
    }

    fn found(id: &str) -> Episode {
        Episode {
            uri: format!("spotify:episode:{id}"),
            name: id.to_owned(),
            ..Episode::default()
        }
    }

    #[test]
    fn decorates_in_batches_and_keeps_what_it_knows() {
        let mut yours = Yours::default();
        let ids: Vec<String> = (0..45).map(|index| format!("{index:022}")).collect();
        yours.take_entries(Ok(ids.iter().map(|id| entry(id)).collect()));
        assert_eq!(yours.remaining(), 45);
        let batch = yours.next_batch();
        assert_eq!(batch.len(), your_episodes::BATCH);
        let mut answers: Vec<Option<Episode>> = ids[..your_episodes::BATCH]
            .iter()
            .map(|id| Some(found(id)))
            .collect();
        answers[3] = None;
        yours.decorating = true;
        yours.take_decorated(batch, Ok(answers));
        assert!(!yours.decorating);
        assert_eq!(yours.episodes.len(), your_episodes::BATCH - 1);
        assert_eq!(yours.gone.len(), 1);
        assert_eq!(yours.remaining(), 25);
        assert_eq!(
            yours.next_batch()[0],
            format!("spotify:episode:{}", ids[20])
        );

        yours.take_decorated(vec![], Err("Couldn’t load more episodes.".into()));
        assert!(yours.error.is_some());

        yours.take_entries(Ok(ids[..2].iter().map(|id| entry(id)).collect()));
        assert_eq!(yours.episodes.len(), 2);
        assert!(yours.gone.is_empty());
        assert_eq!(yours.remaining(), 0);
        assert!(yours.error.is_none());

        yours.take_entries(Err("offline".into()));
        assert_eq!(yours.entries().len(), 2);
    }

    #[test]
    fn needs_everything_only_when_sorted_or_filtered() {
        let mut yours = Yours::default();
        assert!(!yours.arranged());
        yours.sort = Sort::Title;
        assert!(yours.arranged());
        yours.sort = Sort::Added;
        yours.filter = Some(Filter::Video);
        assert!(yours.arranged());
    }
}
