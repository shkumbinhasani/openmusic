use std::collections::HashSet;

use spotify_client::catalog::Playlist;

#[derive(Default)]
pub struct State {
    pub playlists: Vec<Playlist>,
    pub playlists_loading: bool,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        urls.extend(
            self.playlists
                .iter()
                .filter_map(|playlist| playlist.cover.clone()),
        );
    }
}
