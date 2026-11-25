use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{App, Context, ScrollHandle, Window};
use serde::{Deserialize, Serialize};
use spotify_client::{
    catalog::Lyrics,
    engine::NowPlaying,
    entity::{Card, Kind},
    library::ItemKind,
    podcast::{
        self, Availability, Episode, Played, Show, ShowKind,
        your_episodes::{Filter, Sort},
    },
};

use super::{
    ask::RETRY_AFTER,
    canvas::Canvases,
    screen::{ALBUM_KEY, FULL_KEY, Screen, VIDEO_KEY},
    transcript::{SYNC_KEY, TranscriptView},
    video::{self, NOW_PLAYING_KEY},
    worker::{Command, Event},
    yours::{SORT_KEY, Yours},
};
use crate::{
    app::{OpenMusic, Page},
    areas::account::view::filters_explicit,
    menus::{GoLyrics, GoNowPlaying, Handle, route},
    ui::{
        components::Sentinel,
        menu::Target,
        tracks::{RowDrag, RowDrop},
    },
};

const REFRESH_AFTER_PLAYBACK_MS: u64 = 3000;
const REFRESH_AFTER_MARK_MS: u64 = 1500;
const PROGRESS_REDRAW: Duration = Duration::from_secs(2);
const LOOKUP_PATIENCE: Duration = Duration::from_secs(10);
const LOCATE_PATIENCE: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentPrefs {
    pub canvas: bool,
    pub music_videos: bool,
    pub video_podcasts: bool,
}

impl Default for ContentPrefs {
    fn default() -> Self {
        Self {
            canvas: true,
            music_videos: true,
            video_podcasts: true,
        }
    }
}

#[derive(Default)]
pub struct State {
    pub lyrics: Option<(String, Option<Rc<Lyrics>>)>,
    pub lyrics_scroll: ScrollHandle,
    pub lyrics_line: Option<usize>,
    pub lyrics_loading: bool,
    pub lyrics_failed: Option<Instant>,
    pub show: Option<(String, Result<Show, String>)>,
    pub show_loading: bool,
    pub show_error: Option<String>,
    pub show_end: Sentinel,
    pub about_open: bool,
    pub show_filter: Option<Filter>,
    pub episode: Option<(String, Result<Episode, String>)>,
    pub marks: HashMap<String, bool>,
    pub marking: HashMap<String, Option<bool>>,
    pub unavailable: HashSet<String>,
    pub progress_drawn: Option<Instant>,
    pub wanted: HashMap<String, (bool, Instant)>,
    pub yours: Yours,
    pub transcript: Option<TranscriptView>,
    pub similar: Option<(String, Result<Vec<Card>, String>)>,
    pub kinds: HashMap<String, ShowKind>,
    pub shows: HashMap<String, String>,
    pub locating: Option<(String, Instant)>,
    pub watched: Option<(String, bool)>,
    pub canvas: Canvases,
    pub screen: Screen,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        if let Some((_, Ok(show))) = &self.show {
            urls.extend(show.cover.clone());
            urls.extend(
                show.episodes
                    .iter()
                    .filter_map(|episode| episode.cover.clone()),
            );
        }
        if let Some((_, Ok(episode))) = &self.episode {
            urls.extend(episode.large_cover.clone());
        }
        self.yours.covers(urls);
        if let Some((_, Ok(cards))) = &self.similar {
            urls.extend(cards.iter().filter_map(|card| card.image.clone()));
        }
        self.screen.covers(urls);
    }

    pub fn loaded_show(&self) -> Option<&Show> {
        self.show.as_ref().and_then(|(_, show)| show.as_ref().ok())
    }

    pub fn loaded_episode(&self, uri: &str) -> Option<&Episode> {
        self.episode
            .as_ref()
            .and_then(|(_, episode)| episode.as_ref().ok())
            .filter(|episode| episode.uri == uri)
            .or_else(|| {
                self.loaded_show()?
                    .episodes
                    .iter()
                    .find(|episode| episode.uri == uri)
            })
            .or_else(|| self.yours.episodes.get(uri))
    }

    pub fn show_of(&self, uri: &str) -> Option<String> {
        self.loaded_episode(uri)
            .and_then(|episode| episode.show_uri.clone())
            .or_else(|| self.shows.get(uri).cloned())
    }

    fn note_kinds(&mut self, episodes: &[Episode]) {
        for episode in episodes {
            if let Some(show) = &episode.show_uri {
                let kind = if episode.chapter {
                    ShowKind::Audiobook
                } else {
                    ShowKind::Podcast
                };
                self.kinds.insert(show.clone(), kind);
                self.shows.insert(episode.uri.clone(), show.clone());
            }
        }
    }

    fn note_cards(&mut self, cards: &[Card]) {
        for card in cards {
            let kind = match card.kind {
                Kind::Audiobook => ShowKind::Audiobook,
                Kind::Show => ShowKind::Podcast,
                _ => continue,
            };
            self.kinds.insert(card.uri.clone(), kind);
        }
    }

    fn note(&mut self, episodes: &[Episode]) {
        self.note_kinds(episodes);
        for episode in episodes.iter().filter(|episode| !episode.uri.is_empty()) {
            if episode.playable() {
                self.unavailable.remove(&episode.uri);
            } else {
                self.unavailable.insert(episode.uri.clone());
            }
        }
    }

    fn ask_words(&mut self, uri: String) {
        self.lyrics = Some((uri, None));
        self.lyrics_loading = true;
        self.lyrics_failed = None;
        self.lyrics_line = None;
        self.transcript = None;
    }

    fn took_words(
        &mut self,
        uri: String,
        result: Result<Option<Lyrics>, String>,
        at: Instant,
    ) -> bool {
        if !self
            .lyrics
            .as_ref()
            .is_some_and(|(pending, _)| *pending == uri)
        {
            return false;
        }
        let (lyrics, failed) = match result {
            Ok(lyrics) => (lyrics, None),
            Err(_) => (None, Some(at)),
        };
        self.transcript = lyrics
            .as_ref()
            .filter(|_| is_episode(&uri))
            .map(|lyrics| TranscriptView::new(uri.clone(), lyrics.lines.len()));
        self.lyrics = Some((uri, lyrics.map(Rc::new)));
        self.lyrics_loading = false;
        self.lyrics_failed = failed;
        self.lyrics_line = None;
        true
    }

    fn failed_words(&self, now: Instant, wait: Duration) -> Option<&str> {
        let at = self.lyrics_failed?;
        let (uri, _) = self.lyrics.as_ref()?;
        (now.saturating_duration_since(at) >= wait).then_some(uri.as_str())
    }

    fn patch(&mut self, fresh: &Episode) {
        if let Some((_, Ok(show))) = &mut self.show {
            for episode in show
                .episodes
                .iter_mut()
                .filter(|episode| episode.uri == fresh.uri)
            {
                episode.played = fresh.played;
                episode.availability = fresh.availability;
                if fresh.duration_ms > 0 {
                    episode.duration_ms = fresh.duration_ms;
                }
            }
        }
        if let Some((uri, episode)) = &mut self.episode
            && *uri == fresh.uri
        {
            *episode = Ok(fresh.clone());
        }
        self.yours.patch(fresh);
        self.note(std::slice::from_ref(fresh));
        if !self.marking.contains_key(&fresh.uri)
            && self
                .marks
                .get(&fresh.uri)
                .is_some_and(|mark| *mark == fresh.played.finished())
        {
            self.marks.remove(&fresh.uri);
        }
    }
}

fn is_episode(uri: &str) -> bool {
    uri.starts_with("spotify:episode:")
}

fn wants_episode(page: &Page, loaded: Option<&str>, uri: &str) -> bool {
    matches!(page, Page::Episode(open) if open == uri) || loaded == Some(uri)
}

fn left_episode<'a>(
    before: Option<&'a (String, bool)>,
    track: Option<&str>,
    playing: bool,
) -> Option<&'a str> {
    let (uri, was_playing) = before?;
    let changed = track != Some(uri.as_str());
    let paused = *was_playing && !playing;
    (changed || paused).then_some(uri.as_str())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveStep {
    Keep,
    Write,
    LookUp,
}

fn save_step(current: Option<bool>, saved: bool) -> SaveStep {
    match current {
        Some(current) if current == saved => SaveStep::Keep,
        Some(_) => SaveStep::Write,
        None => SaveStep::LookUp,
    }
}

impl OpenMusic {
    pub fn current_lyrics(&self) -> Option<&Lyrics> {
        let (uri, lyrics) = self.content.lyrics.as_ref()?;
        if !self.is_current(uri) {
            return None;
        }
        lyrics.as_deref()
    }

    fn lyric_line(&self) -> Option<usize> {
        let lyrics = self.current_lyrics().filter(|lyrics| lyrics.synced)?;
        let position = self.playback.now.position();
        lyrics
            .lines
            .iter()
            .rposition(|line| !line.heading && line.start_ms <= position)
    }

    pub fn follow_lyrics(&mut self) -> bool {
        let line = self.lyric_line();
        if line == self.content.lyrics_line {
            return false;
        }
        self.content.lyrics_line = line;
        if self.page == Page::Lyrics
            && let Some(line) = line
        {
            match self.transcript_view() {
                Some(view) => view.follow(line),
                None => self
                    .content
                    .lyrics_scroll
                    .scroll_to_top_of_item((line + 1).saturating_sub(3)),
            }
        }
        true
    }

    fn transcript_view(&self) -> Option<&TranscriptView> {
        let uri = &self.track()?.uri;
        self.content
            .transcript
            .as_ref()
            .filter(|view| view.uri == *uri)
    }

    pub fn show_lyrics(&mut self, show: bool) {
        if show {
            self.retry_lyrics(Duration::ZERO);
            self.navigate(Page::Lyrics);
            self.content.lyrics_line = None;
            if let Some(view) = self.transcript_view() {
                view.sync(None);
            }
            self.follow_lyrics();
        } else if self.page == Page::Lyrics {
            self.back();
        }
    }

    pub fn request_lyrics(&mut self, now: &NowPlaying) {
        self.canvas_now(now);
        self.watch_episode(now);
        let Some(track) = &now.track else {
            return;
        };
        if !(track.is_track() || is_episode(&track.uri))
            || self
                .content
                .lyrics
                .as_ref()
                .is_some_and(|(uri, _)| *uri == track.uri)
        {
            return;
        }
        self.ask_words(track.uri.clone());
    }

    fn ask_words(&mut self, uri: String) {
        self.content.ask_words(uri.clone());
        if is_episode(&uri) {
            let known = self
                .content
                .loaded_episode(&uri)
                .map(|episode| episode.transcripts.clone())
                .filter(|transcripts| !transcripts.is_empty());
            self.send(Command::Transcript { uri, known });
        } else {
            self.send(Command::Lyrics(uri));
        }
    }

    pub fn lyrics_failed(&self) -> bool {
        self.content
            .failed_words(Instant::now(), Duration::ZERO)
            .is_some_and(|uri| self.is_current(uri))
    }

    pub fn retry_lyrics(&mut self, wait: Duration) -> bool {
        let Some(uri) = self
            .content
            .failed_words(Instant::now(), wait)
            .filter(|uri| self.is_current(uri))
            .map(str::to_owned)
        else {
            return false;
        };
        self.ask_words(uri);
        true
    }

    fn watch_episode(&mut self, now: &NowPlaying) {
        let track = now.track.as_ref().map(|track| track.uri.as_str());
        let next = track
            .filter(|uri| is_episode(uri))
            .map(|uri| (uri.to_owned(), now.playing));
        let before = std::mem::replace(&mut self.content.watched, next);
        if let Some(uri) = left_episode(before.as_ref(), track, now.playing)
            && self.content.loaded_episode(uri).is_some()
        {
            self.send(Command::Refresh {
                uris: vec![uri.to_owned()],
                delay_ms: REFRESH_AFTER_PLAYBACK_MS,
            });
        }
    }

    pub fn played(&self, episode: &Episode) -> Played {
        let base = if self.is_current(&episode.uri) {
            match self.playback.now.position() {
                0 => episode.played,
                position => Played::InProgress(position),
            }
        } else {
            episode.played
        };
        podcast::effective(base, self.content.marks.get(&episode.uri).copied())
    }

    pub fn played_uri(&self, uri: &str) -> Option<Played> {
        self.content
            .loaded_episode(uri)
            .map(|episode| self.played(episode))
            .or_else(|| {
                self.content
                    .marks
                    .get(uri)
                    .map(|mark| podcast::effective(Played::NotStarted, Some(*mark)))
            })
    }

    pub fn show_resume(&self, show: &Show) -> Option<(usize, bool)> {
        let items: Vec<(Played, Availability)> = show
            .episodes
            .iter()
            .map(|episode| {
                let availability = if self.episode_filtered(episode) {
                    Availability::Other
                } else {
                    episode.availability
                };
                (self.played(episode), availability)
            })
            .collect();
        podcast::resume_index(show.order, &items)
    }

    pub fn show_playing(&self, show: &Show) -> bool {
        self.playback.now.playing
            && (self.playback.now.context_uri.as_deref() == Some(show.uri.as_str())
                || self.track().is_some_and(|track| {
                    show.episodes.iter().any(|episode| episode.uri == track.uri)
                }))
    }

    pub fn play_show_episode(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((uri, Ok(show))) = &self.content.show else {
            return;
        };
        let Some(episode) = show
            .episodes
            .get(index)
            .filter(|episode| episode.playable())
        else {
            return;
        };
        if self.is_current(&episode.uri) {
            self.toggle(cx);
            return;
        }
        let (uri, tracks) = (uri.clone(), show.tracks());
        if self.refuses_explicit(tracks.get(index), cx) {
            return;
        }
        self.play_context(&uri, &tracks, index, cx);
    }

    pub fn play_episode(&mut self, episode: &Episode, cx: &mut Context<Self>) {
        if !episode.playable() {
            return;
        }
        if self.is_current(&episode.uri) {
            self.toggle(cx);
            return;
        }
        let track = episode.as_track();
        if self.refuses_explicit(Some(&track), cx) {
            return;
        }
        self.play_context(&episode.uri, &[track], 0, cx);
    }

    pub fn episode_filtered(&self, episode: &Episode) -> bool {
        filters_explicit(self.account(), episode.explicit)
    }

    pub fn episode_startable(&self, episode: &Episode) -> bool {
        episode.playable() && !self.episode_filtered(episode)
    }

    pub fn play_show(&mut self, cx: &mut Context<Self>) {
        let Some(show) = self.content.loaded_show() else {
            return;
        };
        if self.show_playing(show) {
            self.set_playing(false, cx);
            return;
        }
        let paused_here = !self.playback.now.playing
            && self
                .track()
                .is_some_and(|track| show.episodes.iter().any(|episode| episode.uri == track.uri));
        if paused_here {
            self.set_playing(true, cx);
            return;
        }
        if let Some((index, _)) = self.show_resume(show) {
            self.play_show_episode(index, cx);
        }
    }

    pub fn mark_played(&mut self, uris: Vec<String>, played: bool) {
        let uris: Vec<String> = uris
            .into_iter()
            .filter(|uri| {
                (is_episode(uri) || uri.starts_with("spotify:show:"))
                    && !self.content.marking.contains_key(uri)
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if uris.is_empty() {
            return;
        }
        for uri in &uris {
            let before = self.content.marks.insert(uri.clone(), played);
            self.content.marking.insert(uri.clone(), before);
        }
        self.send(Command::MarkPlayed { uris, played });
    }

    pub fn toggle_played(&mut self, uri: &str) {
        let finished = self.played_uri(uri).is_some_and(Played::finished);
        self.mark_played(vec![uri.to_owned()], !finished);
    }

    pub fn go_to_show(&mut self, episode: &str, known: Option<String>) {
        match known.or_else(|| self.content.show_of(episode)) {
            Some(show) => self.navigate(Page::Show(show)),
            None => {
                self.content.locating = Some((episode.to_owned(), Instant::now()));
                self.send(Command::Locate(episode.to_owned()));
            }
        }
    }

    fn located(&mut self, uri: String, result: Result<Box<Episode>, String>) {
        let Some((wanted, since)) = self.content.locating.take() else {
            return;
        };
        if wanted != uri {
            self.content.locating = Some((wanted, since));
            return;
        }
        if since.elapsed() > LOCATE_PATIENCE {
            return;
        }
        match result {
            Ok(episode) => {
                self.content.note(std::slice::from_ref(&episode));
                match episode.show_uri {
                    Some(show) => self.navigate(Page::Show(show)),
                    None => self.show_toast("Couldn’t open this podcast. Try again.", None),
                }
            }
            Err(message) => self.show_toast(message, None),
        }
    }

    pub fn load_more_episodes(&mut self) {
        let Some((uri, Ok(show))) = &self.content.show else {
            return;
        };
        let Some(offset) = show.next_offset else {
            return;
        };
        if self.content.show_loading || self.content.show_error.is_some() {
            return;
        }
        let command = Command::ShowPage {
            uri: uri.clone(),
            kind: show.kind,
            offset,
        };
        self.content.show_loading = true;
        self.send(command);
    }

    pub fn retry_episodes(&mut self) {
        self.content.show_error = None;
        self.load_more_episodes();
    }

    pub fn saved_or_known(&self, uri: &str) -> Option<bool> {
        self.is_saved(uri)
            .or_else(|| {
                self.content
                    .loaded_show()
                    .filter(|show| show.uri == uri)
                    .map(|show| show.saved)
            })
            .or_else(|| self.library_item(uri).map(|_| true))
    }

    pub fn show_kind(&self, uri: &str) -> Option<ShowKind> {
        self.content
            .loaded_show()
            .filter(|show| show.uri == uri)
            .map(|show| show.kind)
            .or_else(|| self.content.kinds.get(uri).copied())
            .or_else(|| match self.library_item(uri)?.kind {
                ItemKind::Audiobook => Some(ShowKind::Audiobook),
                ItemKind::Show => Some(ShowKind::Podcast),
                _ => None,
            })
    }

    pub fn saving(&self, uri: &str) -> bool {
        self.saved.pending.contains_key(uri) || self.content.wanted.contains_key(uri)
    }

    pub fn save_when_known(&mut self, uri: &str, saved: bool) {
        match save_step(self.saved_or_known(uri), saved) {
            SaveStep::Keep => {}
            SaveStep::Write => self.write_save(uri, saved),
            SaveStep::LookUp => {
                self.content
                    .wanted
                    .insert(uri.to_owned(), (saved, Instant::now()));
                self.lookup_saved([uri.to_owned()]);
            }
        }
    }

    fn write_save(&mut self, uri: &str, saved: bool) {
        if self.write_saved(vec![uri.to_owned()], saved) > 0 {
            self.announce_save(uri, saved);
        }
    }

    pub fn toggle_saved_uri(&mut self, uri: &str) {
        let saved = self.saved_or_known(uri).unwrap_or(false);
        self.change_saved(uri, !saved);
    }

    fn settle_wanted(&mut self) -> bool {
        if self.content.wanted.is_empty() {
            return false;
        }
        let mut changed = false;
        for (uri, (saved, since)) in std::mem::take(&mut self.content.wanted) {
            match self.is_saved(&uri) {
                Some(current) => {
                    if current != saved {
                        self.write_save(&uri, saved);
                    }
                    changed = true;
                }
                None if since.elapsed() >= LOOKUP_PATIENCE => {
                    self.show_toast(
                        if saved {
                            "Couldn’t add this to your library. Try again."
                        } else {
                            "Couldn’t remove this from your library. Try again."
                        },
                        None,
                    );
                    changed = true;
                }
                None => {
                    self.content.wanted.insert(uri, (saved, since));
                }
            }
        }
        changed
    }

    fn lookup_episodes(&mut self, episodes: &[Episode]) {
        let uris: Vec<String> = episodes
            .iter()
            .filter(|episode| !episode.chapter && episode.listed())
            .map(|episode| episode.uri.clone())
            .collect();
        self.lookup_saved(uris);
    }
}

pub fn web_link(uri: &str) -> Option<String> {
    let mut parts = uri.split(':');
    if parts.next() != Some("spotify") {
        return None;
    }
    let kind = parts.next().filter(|kind| !kind.is_empty())?;
    let id = parts.next().filter(|id| !id.is_empty())?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!("https://open.spotify.com/{kind}/{id}"))
}

#[cfg(target_os = "macos")]
fn spotify_installed() -> bool {
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};

    NSURL::URLWithString(&NSString::from_str("spotify:")).is_some_and(|url| {
        NSWorkspace::sharedWorkspace()
            .URLForApplicationToOpenURL(&url)
            .is_some()
    })
}

#[cfg(not(target_os = "macos"))]
fn spotify_installed() -> bool {
    false
}

pub fn open_in_spotify(uri: &str, cx: &mut App) {
    let Some(web) = web_link(uri) else {
        return;
    };
    if spotify_installed() {
        cx.open_url(uri);
    } else {
        cx.open_url(&web);
    }
}

pub fn load(app: &mut OpenMusic, page: &Page) {
    match page {
        Page::Show(uri) => {
            if app
                .content
                .show
                .as_ref()
                .is_none_or(|(loaded, show)| loaded != uri || show.is_err())
            {
                app.content.show = None;
                app.content.show_loading = false;
                app.content.show_error = None;
                app.content.about_open = false;
                app.content.show_filter = None;
                app.content.similar = None;
                app.send(Command::Show(uri.clone()));
            }
            app.lookup_saved([uri.clone()]);
        }
        Page::Episode(uri) => {
            if app
                .content
                .episode
                .as_ref()
                .is_none_or(|(loaded, episode)| loaded != uri || episode.is_err())
            {
                let known = app.content.loaded_episode(uri).cloned();
                app.content.episode = known.map(|episode| (uri.clone(), Ok(episode)));
            }
            app.send(Command::Episode(uri.clone()));
            app.lookup_saved([uri.clone()]);
        }
        Page::YourEpisodes => {
            app.load_your_episodes();
            app.decorate_your_episodes();
        }
        _ => {}
    }
}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::Lyrics { uri, lyrics } => {
            if app.content.took_words(uri, lyrics, Instant::now()) {
                app.follow_lyrics();
            }
        }
        Event::Show { uri, result } => {
            if app.page != Page::Show(uri.clone()) && app.content.show.is_some() {
                return;
            }
            if let Ok(show) = &result {
                app.content.note(&show.episodes);
                app.content.kinds.insert(show.uri.clone(), show.kind);
                app.lookup_episodes(&show.episodes);
                app.send(Command::Similar {
                    uri: show.uri.clone(),
                    kind: show.kind,
                });
            }
            app.content.show = Some((uri, result.map(|show| *show)));
            app.content.show_loading = false;
            app.content.show_error = None;
        }
        Event::ShowPage {
            uri,
            offset,
            result,
        } => {
            let Some((loaded, Ok(show))) = &mut app.content.show else {
                return;
            };
            if *loaded != uri {
                return;
            }
            app.content.show_loading = false;
            match result {
                Ok(page) => {
                    let episodes = page.episodes.clone();
                    if show.append(offset, page) {
                        app.content.note(&episodes);
                        app.lookup_episodes(&episodes);
                    }
                }
                Err(message) => app.content.show_error = Some(message),
            }
        }
        Event::Episode { uri, result } => {
            let loaded = app.content.episode.as_ref();
            let wanted = wants_episode(&app.page, loaded.map(|(loaded, _)| loaded.as_str()), &uri);
            let shown = loaded.is_some_and(|(loaded, episode)| *loaded == uri && episode.is_ok());
            match result {
                Ok(episode) => {
                    if wanted {
                        app.content.episode = Some((uri, Ok(*episode.clone())));
                    }
                    app.content.patch(&episode);
                }
                Err(message) => {
                    if wanted && !shown {
                        app.content.episode = Some((uri, Err(message)));
                    }
                }
            }
        }
        Event::Refreshed(episodes) => {
            for episode in &episodes {
                app.content.patch(episode);
            }
        }
        Event::Located { uri, result } => app.located(uri, result),
        Event::Canvas { uri, result } => app.took_canvas(uri, result),
        Event::VideoOf { uri, result } => app.content.screen.took_video(uri, result),
        Event::SongOf { uri, result } => app.content.screen.took_song(uri, result),
        Event::ArtistVideos {
            uri,
            offset,
            result,
        } => app.content.screen.took_artist_videos(&uri, offset, result),
        Event::NowEpisode { uri, result } => {
            if let Ok(episode) = &result {
                app.content.note(std::slice::from_ref(episode));
            }
            app.content
                .screen
                .took_episode(uri, result.map(|episode| *episode));
        }
        Event::Artwork(_) => {}
        Event::YourEpisodes(result) => {
            app.content.yours.take_entries(result);
            app.took_your_episodes();
        }
        Event::Decorated { uris, result } => {
            if let Ok(found) = &result {
                let episodes: Vec<Episode> = found.iter().flatten().cloned().collect();
                app.content.note(&episodes);
            }
            app.content.yours.take_decorated(uris, result);
            if app.page == Page::YourEpisodes {
                app.your_episodes_tick();
            }
        }
        Event::Similar { uri, result } => {
            if let Ok(cards) = &result {
                app.content.note_cards(cards);
            }
            if app
                .content
                .loaded_show()
                .is_some_and(|show| show.uri == uri)
            {
                app.content.similar = Some((uri, result));
            }
        }
        Event::Marked { uris, error } => {
            let mut settled = Vec::new();
            for uri in &uris {
                let before = app.content.marking.remove(uri).flatten();
                if error.is_some() {
                    match before {
                        Some(mark) => app.content.marks.insert(uri.clone(), mark),
                        None => app.content.marks.remove(uri),
                    };
                } else if is_episode(uri) {
                    settled.push(uri.clone());
                }
            }
            if let Some(message) = error {
                app.show_toast(message, None);
            } else if !settled.is_empty() {
                app.send(Command::Refresh {
                    uris: settled,
                    delay_ms: REFRESH_AFTER_MARK_MS,
                });
            }
        }
    }
}

pub fn connected(_app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {}

pub fn signed_out(app: &mut OpenMusic) {
    app.content = State::default();
}

fn describe_show(app: &OpenMusic, show: &Show, lines: &mut String) {
    let saved = match app.is_saved(&show.uri) {
        Some(saved) => saved.to_string(),
        None => format!("{}?", show.saved),
    };
    let resume = app
        .show_resume(show)
        .map_or("-".to_owned(), |(index, started)| {
            format!("{index}{}", if started { "+" } else { "" })
        });
    let _ = writeln!(
        lines,
        "show {} kind={:?} media={:?} order={:?} saved={saved} rating={} total={} loaded={} next={} restricted={} resume={resume} filter={} loading={} error={:?} name={:?}",
        show.uri,
        show.kind,
        show.media,
        show.order,
        show.rating.map(podcast::rating_label).unwrap_or_default(),
        show.total,
        show.episodes.len(),
        show.next_offset
            .map_or("-".to_owned(), |next| next.to_string()),
        show.restricted(),
        app.content.show_filter.map_or("none", Filter::name),
        app.content.show_loading,
        app.content.show_error,
        show.name,
    );
    match &app.content.similar {
        Some((uri, Ok(cards))) if *uri == show.uri => {
            let _ = writeln!(lines, "similar {}", cards.len());
            for card in cards {
                let _ = writeln!(
                    lines,
                    "  similar {} {:?} {:?}",
                    card.uri, card.kind, card.title
                );
            }
        }
        Some((uri, Err(error))) if *uri == show.uri => {
            let _ = writeln!(lines, "similar error={error:?}");
        }
        _ => lines.push_str("similar loading\n"),
    }
    for (index, episode) in show.episodes.iter().enumerate() {
        let _ = writeln!(
            lines,
            "  {index} {} {:?} {} {}ms video={} saved={} {:?}",
            episode.uri,
            episode.availability,
            describe_played(app.played(episode)),
            episode.duration_ms,
            episode.video,
            describe_saved(app, &episode.uri),
            episode.name,
        );
    }
}

fn describe_saved(app: &OpenMusic, uri: &str) -> String {
    let state = app
        .is_saved(uri)
        .map_or("unknown".to_owned(), |saved| saved.to_string());
    if app.saving(uri) {
        format!("{state}+pending")
    } else {
        state
    }
}

fn describe_words(app: &OpenMusic, lines: &mut String) {
    let Some((uri, lyrics)) = &app.content.lyrics else {
        lines.push_str("words none\n");
        return;
    };
    let kind = if is_episode(uri) {
        "transcript"
    } else {
        "lyrics"
    };
    let Some(lyrics) = lyrics else {
        let state = if app.content.lyrics_loading {
            "loading"
        } else if app.content.lyrics_failed.is_some() {
            "failed"
        } else {
            "none"
        };
        let _ = writeln!(lines, "words {kind} {uri} {state}");
        return;
    };
    let headings = lyrics.lines.iter().filter(|line| line.heading).count();
    let detached = app
        .content
        .transcript
        .as_ref()
        .filter(|view| view.uri == *uri)
        .map(|view| view.detached.get());
    let _ = writeln!(
        lines,
        "words {kind} {uri} lines={} headings={headings} synced={} line={:?} detached={detached:?} note={:?}",
        lyrics.lines.len(),
        lyrics.synced,
        app.content.lyrics_line,
        lyrics.provider,
    );
    for line in lyrics.lines.iter().take(5) {
        let _ = writeln!(
            lines,
            "  {} {}{:?}",
            line.start_ms,
            if line.heading { "# " } else { "" },
            line.words
        );
    }
}

pub(super) fn describe_played(played: Played) -> String {
    match played {
        Played::InProgress(position) => format!("in-progress@{position}"),
        played => played.name().to_owned(),
    }
}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    let words: Vec<&str> = rest.split_whitespace().collect();
    match (verb, words.as_slice()) {
        ("show", [uri]) => app.navigate(Page::Show((*uri).to_owned())),
        ("episode", [uri]) => app.navigate(Page::Episode((*uri).to_owned())),
        ("more", []) => {
            app.content.show_error = None;
            app.load_more_episodes();
        }
        ("markplayed", [uri, setting @ ("on" | "off")]) => {
            app.mark_played(vec![(*uri).to_owned()], *setting == "on");
        }
        ("episodesave" | "showfollow", [uri, setting @ ("on" | "off")]) => {
            app.change_saved(uri, *setting == "on");
        }
        ("gotoshow", [uri]) => app.go_to_show(uri, None),
        ("showfilter", ["none"]) => app.content.show_filter = None,
        ("showfilter", [name]) => match Filter::from_name(name) {
            Some(filter) => app.content.show_filter = Some(filter),
            None => return false,
        },
        ("canvas", []) => {
            let mut lines = String::new();
            app.describe_canvas(&mut lines);
            let _ = std::fs::write(directory.join("canvas.txt"), lines);
        }
        ("video", []) => {
            let mut lines = String::new();
            app.describe_screen(&mut lines);
            app.describe_artist_videos(&mut lines);
            let _ = std::fs::write(directory.join("video.txt"), lines);
        }
        ("video", [kind @ ("music" | "podcasts"), setting @ ("on" | "off")]) => {
            let on = *setting == "on";
            let music = *kind == "music";
            app.update_settings(|settings| {
                if music {
                    settings.content.music_videos = on;
                } else {
                    settings.content.video_podcasts = on;
                }
            });
        }
        ("video", ["switch"]) => app.switch_video(cx),
        ("video", ["fullscreen"]) => {
            video::toggle_full_screen();
        }
        ("watch", [uri]) => app.send(Command::Watch {
            uri: (*uri).to_owned(),
            position_ms: 0,
        }),
        ("watch", [uri, position]) => match position.parse() {
            Ok(position_ms) => app.send(Command::Watch {
                uri: (*uri).to_owned(),
                position_ms,
            }),
            Err(_) => return false,
        },
        ("canvas", ["setting", setting @ ("on" | "off")]) => app.set_canvas(*setting == "on"),
        ("canvas", ["stop"]) => app.preview_canvas(None),
        ("canvas", [uri]) if spotify_client::canvas::is_track(uri) => {
            app.preview_canvas(Some((*uri).to_owned()));
        }
        ("yours", []) => {
            let mut lines = String::new();
            app.describe_your_episodes(&mut lines);
            let _ = std::fs::write(directory.join("yours.txt"), lines);
        }
        ("yours", ["reload"]) => app.load_your_episodes(),
        ("yours", ["more"]) => {
            app.content.yours.error = None;
            app.decorate_your_episodes();
        }
        ("yours", ["sort", name]) => match Sort::from_name(name) {
            Some(sort) => app.set_your_sort(sort),
            None => return false,
        },
        ("yours", ["filter", "none"]) => app.set_your_filter(None),
        ("yours", ["filter", name]) => match Filter::from_name(name) {
            Some(filter) => app.set_your_filter(Some(filter)),
            None => return false,
        },
        ("content", []) => {
            let mut lines = String::new();
            match &app.content.show {
                Some((_, Ok(show))) => describe_show(app, show, &mut lines),
                Some((uri, Err(error))) => {
                    let _ = writeln!(lines, "show {uri} error={error:?}");
                }
                None => lines.push_str("show none\n"),
            }
            match &app.content.episode {
                Some((uri, Ok(episode))) => {
                    let saved = describe_saved(app, uri);
                    let _ = writeln!(
                        lines,
                        "episode {uri} {:?} {} {}ms chapter={} video={} saved={saved} show={:?} transcripts={} {:?}",
                        episode.availability,
                        describe_played(app.played(episode)),
                        episode.duration_ms,
                        episode.chapter,
                        episode.video,
                        episode.show_uri,
                        episode.transcripts.len(),
                        episode.name,
                    );
                }
                Some((uri, Err(error))) => {
                    let _ = writeln!(lines, "episode {uri} error={error:?}");
                }
                None => lines.push_str("episode none\n"),
            }
            for (uri, mark) in &app.content.marks {
                let pending = if app.content.marking.contains_key(uri) {
                    " pending"
                } else {
                    ""
                };
                let _ = writeln!(lines, "mark {uri} {mark}{pending}");
            }
            for (uri, (saved, _)) in &app.content.wanted {
                let _ = writeln!(lines, "wanted {uri} saved={saved}");
            }
            if let Some((uri, since)) = &app.content.locating {
                let _ = writeln!(
                    lines,
                    "locating {uri} for {}ms",
                    since.elapsed().as_millis()
                );
            }
            describe_words(app, &mut lines);
            app.describe_your_episodes(&mut lines);
            app.describe_canvas(&mut lines);
            let _ = std::fs::write(directory.join("content.txt"), lines);
        }
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    let mut parts = vec![page_fragment(app)];
    if let Some(status) = app.canvas_status() {
        parts.push(format!("canvas={status}"));
    }
    if let Some(video) = app.video_status() {
        parts.push(format!(
            "video={}:{}",
            video.kind.name(),
            if video.on { "on" } else { "off" }
        ));
    }
    parts.retain(|part| !part.is_empty());
    parts.join(" ")
}

fn page_fragment(app: &OpenMusic) -> String {
    match &app.page {
        Page::Show(uri) => match &app.content.show {
            Some((loaded, Ok(show))) if loaded == uri => {
                format!("show={uri} episodes={}/{}", show.episodes.len(), show.total)
            }
            Some((loaded, Err(_))) if loaded == uri => format!("show={uri} episodes=error"),
            _ => format!("show={uri} episodes=loading"),
        },
        Page::Lyrics => match app.track() {
            Some(track) if is_episode(&track.uri) => match &app.content.lyrics {
                Some((uri, Some(lyrics))) if *uri == track.uri => format!(
                    "transcript={} line={:?} detached={}",
                    lyrics.lines.len(),
                    app.content.lyrics_line,
                    app.transcript_view()
                        .is_some_and(|view| view.detached.get())
                ),
                Some((uri, None)) if *uri == track.uri && app.content.lyrics_loading => {
                    "transcript=loading".to_owned()
                }
                Some((uri, None)) if *uri == track.uri && app.content.lyrics_failed.is_some() => {
                    "transcript=failed".to_owned()
                }
                _ => "transcript=none".to_owned(),
            },
            _ => String::new(),
        },
        Page::YourEpisodes => match &app.content.yours.entries {
            Some(Ok(entries)) => format!(
                "your_episodes={}/{} decorated={}",
                app.your_episode_list().len(),
                entries.len(),
                app.content.yours.episodes.len()
            ),
            Some(Err(_)) => "your_episodes=error".to_owned(),
            None => "your_episodes=loading".to_owned(),
        },
        Page::Episode(uri) => match &app.content.episode {
            Some((loaded, Ok(episode))) if loaded == uri => {
                format!(
                    "episode={uri} played={}",
                    describe_played(app.played(episode))
                )
            }
            Some((loaded, Err(_))) if loaded == uri => format!("episode={uri} played=error"),
            _ => format!("episode={uri} played=loading"),
        },
        _ => String::new(),
    }
}

pub fn register_actions(cx: &mut App, handle: &Handle) {
    route(cx, handle, |_: &GoLyrics, app, _, cx| {
        app.show_lyrics(app.page != Page::Lyrics);
        cx.notify();
    });
    route(cx, handle, |_: &GoNowPlaying, app, _, cx| {
        app.toggle_now_playing();
        cx.notify();
    });
}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    let show_uri = || match &app.page {
        Page::Show(uri) => Some(uri.clone()),
        _ => None,
    };
    let episode_uri = || match &app.page {
        Page::Episode(uri) => Some(uri.clone()),
        _ => None,
    };
    match key {
        "lyrics" => app.show_lyrics(app.page != Page::Lyrics),
        "show-play" => app.play_show(cx),
        "show-follow" => {
            if let Some(uri) = show_uri() {
                app.toggle_saved_uri(&uri);
            }
        }
        "show-open" | "episode-open" => {
            let uri = if key == "show-open" {
                show_uri()
            } else {
                episode_uri().and_then(|uri| app.content.loaded_episode(&uri)?.show_uri.clone())
            };
            if let Some(uri) = uri {
                open_in_spotify(&uri, cx);
            }
        }
        "more-show" => {
            if let Some(uri) = show_uri() {
                app.open_menu_under(Target::Show { uri }, key);
            }
        }
        "episode-play" => {
            if let Some(episode) =
                episode_uri().and_then(|uri| app.content.loaded_episode(&uri).cloned())
            {
                app.play_episode(&episode, cx);
            }
        }
        "episode-save" => {
            if let Some(uri) = episode_uri() {
                app.toggle_saved_uri(&uri);
            }
        }
        "episode-played" => {
            if let Some(uri) = episode_uri() {
                app.toggle_played(&uri);
            }
        }
        "episode-show" => {
            if let Some(show) =
                episode_uri().and_then(|uri| app.content.loaded_episode(&uri)?.show_uri.clone())
            {
                app.navigate(Page::Show(show));
            }
        }
        "your-episodes-play" => app.play_your_episodes(cx),
        NOW_PLAYING_KEY => app.toggle_now_playing(),
        VIDEO_KEY => app.switch_video(cx),
        FULL_KEY => {
            video::toggle_full_screen();
        }
        ALBUM_KEY => app.go_to_video_album(),
        SYNC_KEY => app.sync_transcript(),
        SORT_KEY => app.open_your_sort_menu(),
        "more-episode" => {
            if let Some(uri) = episode_uri() {
                app.open_menu_under(Target::Episode { uri }, key);
            }
        }
        _ => return false,
    }
    true
}

pub fn row_dropped(
    _app: &mut OpenMusic,
    _drag: &RowDrag,
    _at: RowDrop,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
}

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    if app.retry_lyrics(RETRY_AFTER) {
        cx.notify();
    }
    let reduce_motion = app.content.canvas.reduce_motion;
    app.canvas_tick();
    app.screen_tick(cx);
    if reduce_motion != app.content.canvas.reduce_motion && app.page == Page::Settings {
        cx.notify();
    }
    if app.settle_wanted() {
        cx.notify();
    }
    if matches!(app.page, Page::Show(_)) && app.content.show_end.take() {
        app.load_more_episodes();
        cx.notify();
    }
    if app.page == Page::YourEpisodes && app.your_episodes_tick() {
        cx.notify();
    }
    let showing_progress = app.playback.now.playing
        && matches!(
            app.page,
            Page::Show(_) | Page::Episode(_) | Page::YourEpisodes
        )
        && app
            .track()
            .is_some_and(|track| app.content.loaded_episode(&track.uri).is_some());
    if showing_progress
        && app
            .content
            .progress_drawn
            .is_none_or(|drawn| drawn.elapsed() >= PROGRESS_REDRAW)
    {
        app.content.progress_drawn = Some(Instant::now());
        cx.notify();
    }
}

pub fn show_kind_label(show: &Show) -> &'static str {
    match (show.kind, show.video()) {
        (ShowKind::Audiobook, _) => "AUDIOBOOK",
        (ShowKind::Podcast, true) => "VIDEO PODCAST",
        (ShowKind::Podcast, false) => "PODCAST",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_to_the_web_player() {
        assert_eq!(
            web_link("spotify:show:0000000000000000000ab1").as_deref(),
            Some("https://open.spotify.com/show/0000000000000000000ab1")
        );
        assert_eq!(
            web_link("spotify:episode:00000000000000000000a1").as_deref(),
            Some("https://open.spotify.com/episode/00000000000000000000a1")
        );
        assert_eq!(web_link("spotify:user:someone:collection"), None);
        assert_eq!(web_link("https://example.com"), None);
        assert_eq!(web_link("spotify:show:"), None);
    }

    fn episode(uri: &str, played: Played, availability: Availability) -> Episode {
        Episode {
            uri: uri.to_owned(),
            name: uri.to_owned(),
            duration_ms: 60_000,
            played,
            availability,
            ..Episode::default()
        }
    }

    #[test]
    fn patches_fresh_played_state_and_settles_marks() {
        let mut state = State::default();
        let show = Show {
            uri: "spotify:show:s".into(),
            episodes: vec![
                Episode {
                    uid: Some("row".into()),
                    ..episode(
                        "spotify:episode:a",
                        Played::NotStarted,
                        Availability::Playable,
                    )
                },
                episode(
                    "spotify:episode:b",
                    Played::NotStarted,
                    Availability::Restricted,
                ),
            ],
            ..Show::default()
        };
        state.note(&show.episodes);
        assert!(state.unavailable.contains("spotify:episode:b"));
        state.show = Some(("spotify:show:s".into(), Ok(show)));
        state.marks.insert("spotify:episode:a".into(), true);
        state.marking.insert("spotify:episode:a".into(), None);

        let fresh = episode(
            "spotify:episode:a",
            Played::Completed,
            Availability::Playable,
        );
        state.patch(&fresh);
        let patched = state.loaded_episode("spotify:episode:a").unwrap();
        assert_eq!(patched.played, Played::Completed);
        assert_eq!(patched.uid.as_deref(), Some("row"));
        assert!(state.marks.contains_key("spotify:episode:a"));

        state.marking.clear();
        state.patch(&fresh);
        assert!(!state.marks.contains_key("spotify:episode:a"));

        state.marks.insert("spotify:episode:a".into(), false);
        state.patch(&fresh);
        assert_eq!(state.marks.get("spotify:episode:a"), Some(&false));
    }

    #[test]
    fn remembers_which_shows_are_books() {
        let mut state = State::default();
        let chapter = Episode {
            chapter: true,
            show_uri: Some("spotify:show:book".into()),
            ..episode(
                "spotify:episode:c",
                Played::NotStarted,
                Availability::PaymentRequired,
            )
        };
        let podcast_episode = Episode {
            show_uri: Some("spotify:show:podcast".into()),
            ..episode(
                "spotify:episode:p",
                Played::NotStarted,
                Availability::Playable,
            )
        };
        state.note(&[chapter, podcast_episode]);
        assert_eq!(
            state.kinds.get("spotify:show:book"),
            Some(&ShowKind::Audiobook)
        );
        assert_eq!(
            state.kinds.get("spotify:show:podcast"),
            Some(&ShowKind::Podcast)
        );
        assert!(state.unavailable.contains("spotify:episode:c"));
        assert_eq!(
            state.show_of("spotify:episode:p").as_deref(),
            Some("spotify:show:podcast")
        );
        assert_eq!(
            state.show_of("spotify:episode:c").as_deref(),
            Some("spotify:show:book")
        );
        assert_eq!(state.show_of("spotify:episode:unknown"), None);

        let card = |uri: &str, kind: Kind| Card {
            uri: uri.into(),
            kind,
            title: uri.into(),
            subtitle: String::new(),
            image: None,
            format: None,
            color: None,
            reason: None,
            explicit: false,
            playable: true,
        };
        state.note_cards(&[
            card("spotify:show:other-book", Kind::Audiobook),
            card("spotify:show:other-show", Kind::Show),
            card("spotify:album:x", Kind::Album),
        ]);
        assert_eq!(
            state.kinds.get("spotify:show:other-book"),
            Some(&ShowKind::Audiobook)
        );
        assert_eq!(
            state.kinds.get("spotify:show:other-show"),
            Some(&ShowKind::Podcast)
        );
        assert!(!state.kinds.contains_key("spotify:album:x"));
    }

    #[test]
    fn refreshes_an_episode_once_playback_leaves_or_pauses_it() {
        let a = "spotify:episode:a";
        let playing = Some((a.to_owned(), true));
        let paused = Some((a.to_owned(), false));
        assert_eq!(left_episode(None, Some(a), true), None);
        assert_eq!(left_episode(playing.as_ref(), Some(a), true), None);
        assert_eq!(
            left_episode(playing.as_ref(), Some("spotify:track:song"), true),
            Some(a)
        );
        assert_eq!(
            left_episode(playing.as_ref(), Some("spotify:episode:b"), true),
            Some(a)
        );
        assert_eq!(left_episode(playing.as_ref(), None, false), Some(a));
        assert_eq!(left_episode(playing.as_ref(), Some(a), false), Some(a));
        assert_eq!(left_episode(paused.as_ref(), Some(a), false), None);
        assert_eq!(left_episode(paused.as_ref(), Some(a), true), None);
        assert_eq!(
            left_episode(paused.as_ref(), Some("spotify:track:song"), true),
            Some(a)
        );
    }

    #[test]
    fn filters_explicit_episodes_like_explicit_songs() {
        use spotify_client::account::AccountInfo;

        use crate::areas::account::view::filtered;

        let filtering = AccountInfo {
            explicit_filtered: true,
            ..AccountInfo::default()
        };
        let explicit = Episode {
            explicit: true,
            ..episode(
                "spotify:episode:e",
                Played::NotStarted,
                Availability::Playable,
            )
        };
        let clean = episode(
            "spotify:episode:c",
            Played::NotStarted,
            Availability::Playable,
        );
        for episode in [&explicit, &clean] {
            assert_eq!(
                filters_explicit(Some(&filtering), episode.explicit),
                filtered(Some(&filtering), &episode.as_track())
            );
        }
        assert!(filtered(Some(&filtering), &explicit.as_track()));
        assert!(!filtered(Some(&filtering), &clean.as_track()));
    }

    #[test]
    fn tells_a_failed_lyrics_read_from_none_and_asks_again() {
        let song = "spotify:track:00000000000000000000s1";
        let mut state = State::default();
        let now = Instant::now();
        state.ask_words(song.into());
        assert!(state.lyrics_loading);
        assert!(!state.took_words("spotify:track:other".into(), Ok(None), now));
        assert!(state.lyrics_loading);

        assert!(state.took_words(song.into(), Err("offline".into()), now));
        assert!(!state.lyrics_loading);
        assert_eq!(state.lyrics_failed, Some(now));
        assert_eq!(state.failed_words(now, Duration::ZERO), Some(song));
        assert_eq!(state.failed_words(now, RETRY_AFTER), None);
        assert_eq!(
            state.failed_words(now + RETRY_AFTER, RETRY_AFTER),
            Some(song)
        );

        state.ask_words(song.into());
        assert_eq!(state.lyrics_failed, None);
        assert!(state.took_words(song.into(), Ok(None), now));
        assert_eq!(state.lyrics_failed, None);
        assert!(matches!(&state.lyrics, Some((uri, None)) if uri == song));
        assert_eq!(state.failed_words(now + RETRY_AFTER, RETRY_AFTER), None);

        state.ask_words(song.into());
        let found = Lyrics {
            synced: false,
            lines: Vec::new(),
            provider: String::new(),
        };
        assert!(state.took_words(song.into(), Ok(Some(found)), now));
        assert!(matches!(&state.lyrics, Some((_, Some(_)))));
        assert!(state.transcript.is_none());
    }

    #[test]
    fn keeps_an_episode_page_on_the_episode_it_opened() {
        let (a, b) = ("spotify:episode:a", "spotify:episode:b");
        let open_b = Page::Episode(b.to_owned());
        assert!(!wants_episode(&open_b, None, a));
        assert!(wants_episode(&open_b, None, b));
        assert!(wants_episode(&open_b, Some(b), b));
        assert!(!wants_episode(&open_b, Some(b), a));
        assert!(wants_episode(&Page::Home, Some(a), a));
        assert!(!wants_episode(&Page::Home, None, a));
    }

    #[test]
    fn saves_only_what_is_known_to_differ() {
        assert_eq!(save_step(Some(true), true), SaveStep::Keep);
        assert_eq!(save_step(Some(false), false), SaveStep::Keep);
        assert_eq!(save_step(Some(false), true), SaveStep::Write);
        assert_eq!(save_step(Some(true), false), SaveStep::Write);
        assert_eq!(save_step(None, true), SaveStep::LookUp);
        assert_eq!(save_step(None, false), SaveStep::LookUp);
    }

    #[test]
    fn shows_canvas_unless_turned_off() {
        assert!(ContentPrefs::default().canvas);
        let empty: ContentPrefs = serde_json::from_str("{}").unwrap();
        assert!(empty.canvas);
        let off: ContentPrefs = serde_json::from_str(r#"{"canvas":false}"#).unwrap();
        assert!(!off.canvas);
        assert!(off.music_videos && off.video_podcasts);
        assert_eq!(
            serde_json::to_value(&off).unwrap(),
            serde_json::json!({ "canvas": false, "music_videos": true, "video_podcasts": true })
        );
        let audio_only: ContentPrefs =
            serde_json::from_str(r#"{"music_videos":false,"video_podcasts":false}"#).unwrap();
        assert!(audio_only.canvas);
        let prefs = super::super::worker::video_prefs(&audio_only);
        assert!(!prefs.music && !prefs.podcasts);
        assert_eq!(
            super::super::worker::video_prefs(&ContentPrefs::default()),
            spotify_client::engine::playback::VideoPrefs::default()
        );
    }

    #[test]
    fn labels_show_kinds() {
        let mut show = Show::default();
        assert_eq!(show_kind_label(&show), "PODCAST");
        show.media = podcast::MediaKind::Mixed;
        assert_eq!(show_kind_label(&show), "VIDEO PODCAST");
        show.kind = ShowKind::Audiobook;
        assert_eq!(show_kind_label(&show), "AUDIOBOOK");
    }
}
