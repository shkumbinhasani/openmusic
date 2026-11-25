use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    path::Path,
    time::{Duration, Instant},
};

use gpui::{App, Context, ScrollHandle, Window};
use serde::{Deserialize, Serialize};
use spotify_client::{
    catalog::Lyrics,
    engine::NowPlaying,
    podcast::{self, Availability, Episode, Played, Show, ShowKind},
};

use super::worker::{Command, Event};
use crate::{
    app::{OpenMusic, Page},
    menus::Handle,
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

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentPrefs {}

#[derive(Default)]
pub struct State {
    pub lyrics: Option<(String, Option<Lyrics>)>,
    pub lyrics_scroll: ScrollHandle,
    pub lyrics_line: Option<usize>,
    pub lyrics_loading: bool,
    pub show: Option<(String, Result<Show, String>)>,
    pub show_loading: bool,
    pub show_error: Option<String>,
    pub show_end: Sentinel,
    pub about_open: bool,
    pub episode: Option<(String, Result<Episode, String>)>,
    pub marks: HashMap<String, bool>,
    pub marking: HashMap<String, Option<bool>>,
    pub unavailable: HashSet<String>,
    pub progress_drawn: Option<Instant>,
    pub wanted: HashMap<String, (bool, Instant)>,
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
    }

    fn note(&mut self, episodes: &[Episode]) {
        for episode in episodes {
            if episode.playable() {
                self.unavailable.remove(&episode.uri);
            } else {
                self.unavailable.insert(episode.uri.clone());
            }
        }
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
        lyrics.as_ref()
    }

    fn lyric_line(&self) -> Option<usize> {
        let lyrics = self.current_lyrics().filter(|lyrics| lyrics.synced)?;
        let position = self.playback.now.position();
        lyrics
            .lines
            .iter()
            .rposition(|line| line.start_ms <= position)
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
            self.content
                .lyrics_scroll
                .scroll_to_top_of_item((line + 1).saturating_sub(3));
        }
        true
    }

    pub fn show_lyrics(&mut self, show: bool) {
        if show {
            self.navigate(Page::Lyrics);
            self.content.lyrics_line = None;
            self.follow_lyrics();
        } else if self.page == Page::Lyrics {
            self.back();
        }
    }

    pub fn request_lyrics(&mut self, now: &NowPlaying) {
        self.watch_episode(now);
        if let Some(track) = &now.track
            && track.is_track()
            && self
                .content
                .lyrics
                .as_ref()
                .is_none_or(|(uri, _)| *uri != track.uri)
        {
            self.content.lyrics = Some((track.uri.clone(), None));
            self.content.lyrics_loading = true;
            self.content.lyrics_line = None;
            self.send(Command::Lyrics(track.uri.clone()));
        }
    }

    fn watch_episode(&mut self, now: &NowPlaying) {
        let Some(before) = self
            .track()
            .map(|track| track.uri.clone())
            .filter(|uri| is_episode(uri))
        else {
            return;
        };
        let changed = now.track.as_ref().is_none_or(|track| track.uri != before);
        let paused = self.playback.now.playing && !now.playing;
        if (changed || paused) && self.content.loaded_episode(&before).is_some() {
            self.send(Command::Refresh {
                uris: vec![before],
                delay_ms: REFRESH_AFTER_PLAYBACK_MS,
            });
        }
    }

    fn refresh_current_episode(&self) {
        if let Some(track) = self.track()
            && is_episode(&track.uri)
            && self.content.loaded_episode(&track.uri).is_some()
        {
            self.send(Command::Refresh {
                uris: vec![track.uri.clone()],
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
            .map(|episode| (self.played(episode), episode.availability))
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
        self.refresh_current_episode();
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
        self.refresh_current_episode();
        let track = episode.as_track();
        self.play_context(&episode.uri, &[track], 0, cx);
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
        self.is_saved(uri).or_else(|| {
            self.content
                .loaded_show()
                .filter(|show| show.uri == uri)
                .map(|show| show.saved)
        })
    }

    pub fn saving(&self, uri: &str) -> bool {
        self.saved.pending.contains_key(uri) || self.content.wanted.contains_key(uri)
    }

    pub fn save_when_known(&mut self, uri: &str, saved: bool) {
        match save_step(self.saved_or_known(uri), saved) {
            SaveStep::Keep => {}
            SaveStep::Write => self.set_saved(vec![uri.to_owned()], saved),
            SaveStep::LookUp => {
                self.content
                    .wanted
                    .insert(uri.to_owned(), (saved, Instant::now()));
                self.lookup_saved([uri.to_owned()]);
            }
        }
    }

    pub fn toggle_saved_uri(&mut self, uri: &str) {
        let saved = self.saved_or_known(uri).unwrap_or(false);
        self.save_when_known(uri, !saved);
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
                        self.set_saved(vec![uri], saved);
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
            .filter(|episode| !episode.chapter && !episode.restricted())
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
            if app
                .content
                .lyrics
                .as_ref()
                .is_some_and(|(pending, _)| *pending == uri)
            {
                app.content.lyrics = Some((uri, lyrics));
                app.content.lyrics_loading = false;
                app.content.lyrics_line = None;
                app.follow_lyrics();
            }
        }
        Event::Show { uri, result } => {
            if app.page != Page::Show(uri.clone()) && app.content.show.is_some() {
                return;
            }
            if let Ok(show) = &result {
                app.content.note(&show.episodes);
                app.lookup_episodes(&show.episodes);
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
            if app
                .content
                .episode
                .as_ref()
                .is_some_and(|(loaded, _)| *loaded != uri)
            {
                return;
            }
            match result {
                Ok(episode) => {
                    app.content.episode = Some((uri, Ok(*episode.clone())));
                    app.content.patch(&episode);
                }
                Err(message) => {
                    let shown = app
                        .content
                        .episode
                        .as_ref()
                        .is_some_and(|(loaded, episode)| *loaded == uri && episode.is_ok());
                    if !shown {
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
        "show {} kind={:?} media={:?} order={:?} saved={saved} rating={} total={} loaded={} next={} restricted={} resume={resume} loading={} error={:?} name={:?}",
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
        app.content.show_loading,
        app.content.show_error,
        show.name,
    );
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

fn describe_played(played: Played) -> String {
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
    _cx: &mut Context<OpenMusic>,
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
            app.save_when_known(uri, *setting == "on");
        }
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
            let _ = std::fs::write(directory.join("content.txt"), lines);
        }
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    match &app.page {
        Page::Show(uri) => match &app.content.show {
            Some((loaded, Ok(show))) if loaded == uri => {
                format!("show={uri} episodes={}/{}", show.episodes.len(), show.total)
            }
            Some((loaded, Err(_))) if loaded == uri => format!("show={uri} episodes=error"),
            _ => format!("show={uri} episodes=loading"),
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

pub fn register_actions(_cx: &mut App, _handle: &Handle) {}

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
    if app.settle_wanted() {
        cx.notify();
    }
    if matches!(app.page, Page::Show(_)) && app.content.show_end.take() {
        app.load_more_episodes();
        cx.notify();
    }
    let showing_progress = app.playback.now.playing
        && matches!(app.page, Page::Show(_) | Page::Episode(_))
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
    fn saves_only_what_is_known_to_differ() {
        assert_eq!(save_step(Some(true), true), SaveStep::Keep);
        assert_eq!(save_step(Some(false), false), SaveStep::Keep);
        assert_eq!(save_step(Some(false), true), SaveStep::Write);
        assert_eq!(save_step(Some(true), false), SaveStep::Write);
        assert_eq!(save_step(None, true), SaveStep::LookUp);
        assert_eq!(save_step(None, false), SaveStep::LookUp);
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
