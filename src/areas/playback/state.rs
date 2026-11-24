use std::{collections::HashSet, path::Path, time::Duration};

use gpui::{Context, Window};
use serde::{Deserialize, Serialize};
use spotify_client::{
    Track,
    connect::{Repeat, Shuffle},
    engine::NowPlaying,
};

use super::worker::{Command, Event};
use crate::app::{OpenMusic, Page};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackPrefs {}

#[derive(Default)]
pub struct State {
    pub now: NowPlaying,
    pub notice: Option<String>,
    pub seek_hold: Option<(u32, std::time::Instant)>,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        urls.extend(
            self.now
                .track
                .iter()
                .filter_map(|track| track.cover.clone()),
        );
    }
}

impl OpenMusic {
    pub fn track(&self) -> Option<&Track> {
        self.playback.now.track.as_ref()
    }

    pub fn duration(&self) -> Option<u32> {
        self.track()
            .map(|track| track.duration_ms)
            .filter(|duration| *duration > 0)
    }

    pub fn is_current(&self, uri: &str) -> bool {
        self.track().is_some_and(|track| track.uri == *uri)
    }

    pub fn shuffle_mode(&self) -> Shuffle {
        self.playback.now.shuffle
    }

    pub fn repeat_mode(&self) -> Repeat {
        self.playback.now.repeat
    }

    pub fn toggle_shuffle(&mut self, _cx: &mut Context<Self>) {}

    pub fn cycle_repeat(&mut self, _cx: &mut Context<Self>) {}

    pub fn play(&mut self, tracks: &[Track], index: usize, cx: &mut Context<Self>) {
        let Some(track) = tracks.get(index) else {
            return;
        };
        self.playback.now.track = Some(track.clone());
        self.playback.now.playing = true;
        self.playback.now.position_ms = 0;
        self.playback.now.observed = std::time::Instant::now();
        self.playback.notice = None;
        self.send(Command::PlayTracks {
            tracks: tracks.iter().map(|track| track.uri.clone()).collect(),
            index,
        });
        cx.notify();
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.track().is_none() {
            return;
        }
        self.set_playing(!self.playback.now.playing, cx);
    }

    pub fn set_playing(&mut self, playing: bool, cx: &mut Context<Self>) {
        if self.track().is_some() && self.playback.now.playing == playing {
            return;
        }
        self.playback.now.position_ms = self.playback.now.position();
        self.playback.now.observed = std::time::Instant::now();
        self.playback.now.playing = playing;
        self.playback.notice = None;
        self.send(Command::SetPlaying(playing));
        cx.notify();
    }

    pub fn seek_to(&mut self, position: u32, cx: &mut Context<Self>) {
        self.playback.now.position_ms = position;
        self.playback.now.observed = std::time::Instant::now();
        self.playback.seek_hold = Some((position, std::time::Instant::now()));
        self.send(Command::Seek(position));
        self.follow_lyrics();
        cx.notify();
    }

    pub fn play_context(
        &mut self,
        uri: &str,
        tracks: &[Track],
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(track) = tracks.get(index) {
            self.playback.now.track = Some(track.clone());
            self.playback.now.playing = true;
            self.playback.now.position_ms = 0;
            self.playback.now.observed = std::time::Instant::now();
        }
        self.playback.notice = None;
        self.send(Command::PlayContext {
            uri: uri.to_owned(),
            index,
            track_uri: tracks.get(index).map(|track| track.uri.clone()),
        });
        cx.notify();
    }

    pub fn set_now_playing(&mut self, now: NowPlaying) {
        let expected = self.playback.now.position();
        let held = self.playback.seek_hold.filter(|(_, at)| {
            at.elapsed() < Duration::from_secs(3)
                && now.track.as_ref().map(|track| &track.uri)
                    == self.playback.now.track.as_ref().map(|track| &track.uri)
                && now.position().abs_diff(expected) > 1500
        });
        let (position, observed) = (self.playback.now.position_ms, self.playback.now.observed);
        self.playback.now = now;
        if held.is_some() {
            self.playback.now.position_ms = position;
            self.playback.now.observed = observed;
        } else {
            self.playback.seek_hold = None;
        }
    }
}

pub fn load(_app: &mut OpenMusic, _page: &Page) {}

pub fn on_event(
    _app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {}
}

pub fn connected(_app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {}

pub fn signed_out(app: &mut OpenMusic) {
    app.playback.now = NowPlaying::default();
    app.playback.notice = None;
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

pub fn state_fragment(app: &OpenMusic) -> String {
    format!(
        "shuffle={:?} repeat={:?}",
        app.shuffle_mode(),
        app.repeat_mode()
    )
}
