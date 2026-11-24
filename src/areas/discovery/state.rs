use std::collections::HashSet;

use gpui::Context;
use spotify_client::{Track, catalog::Artist};

use super::worker::{Discovery, Mix};
use crate::{
    app::{Account, OpenMusic, Page},
    worker::Command,
};

#[derive(Default)]
pub struct State {
    pub results: Vec<Track>,
    pub home: Discovery,
    pub newest_mix: Option<Mix>,
    pub last_seed: Option<String>,
    pub discovery_pending: bool,
    pub artist: Option<(String, Result<Artist, String>)>,
    pub search_query: String,
    pub search_pending: bool,
    pub search_error: Option<String>,
}

impl State {
    pub fn station(&self) -> Option<&Mix> {
        self.newest_mix.as_ref().or(self.home.mix.as_ref())
    }

    pub fn covers(&self, urls: &mut HashSet<String>) {
        let tracks = self
            .results
            .iter()
            .chain(&self.home.tracks)
            .chain(self.home.mix.iter().flat_map(|mix| &mix.tracks))
            .chain(self.newest_mix.iter().flat_map(|mix| &mix.tracks));
        urls.extend(tracks.filter_map(|track| track.cover.clone()));
        if let Some((_, Ok(artist))) = &self.artist {
            urls.extend(artist.image.clone());
            urls.extend(
                artist
                    .top_tracks
                    .iter()
                    .filter_map(|track| track.cover.clone()),
            );
            urls.extend(
                artist
                    .releases
                    .iter()
                    .filter_map(|release| release.cover.clone()),
            );
            urls.extend(
                artist
                    .related
                    .iter()
                    .filter_map(|related| related.image.clone()),
            );
        }
    }
}

impl OpenMusic {
    pub fn search(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value();
        let query = value.trim();
        if query.is_empty()
            || self.discovery.search_pending
            || !matches!(self.account, Account::Connected)
        {
            return;
        }
        self.navigate(Page::Search);
        self.discovery.search_query = query.to_owned();
        self.discovery.search_pending = true;
        self.discovery.search_error = None;
        self.discovery.results.clear();
        self.send(Command::Search(query.to_owned()));
        cx.notify();
    }
}
