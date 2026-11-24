use std::collections::HashSet;

use gpui::Context;
use spotify_client::{Track, engine::NowPlaying};

use crate::{app::OpenMusic, worker::Command};

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
        self.playback.now.position_ms = self.playback.now.position();
        self.playback.now.observed = std::time::Instant::now();
        self.playback.now.playing = !self.playback.now.playing;
        self.playback.notice = None;
        self.send(Command::SetPlaying(self.playback.now.playing));
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
}
