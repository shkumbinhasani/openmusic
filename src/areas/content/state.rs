use std::path::Path;

use gpui::{App, Context, ScrollHandle, Window};
use spotify_client::{catalog::Lyrics, engine::NowPlaying};

use super::worker::{Command, Event};
use crate::{
    app::{OpenMusic, Page},
    menus::Handle,
};

#[derive(Default)]
pub struct State {
    pub lyrics: Option<(String, Option<Lyrics>)>,
    pub lyrics_scroll: ScrollHandle,
    pub lyrics_line: Option<usize>,
    pub lyrics_loading: bool,
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
}

pub fn load(_app: &mut OpenMusic, _page: &Page) {}

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
    }
}

pub fn connected(_app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {}

pub fn signed_out(app: &mut OpenMusic) {
    app.content.lyrics = None;
}

pub fn debug(
    _app: &mut OpenMusic,
    _verb: &str,
    _rest: &str,
    _directory: &Path,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
}

pub fn state_fragment(_app: &OpenMusic) -> String {
    String::new()
}

pub fn register_actions(_cx: &mut App, _handle: &Handle) {}
