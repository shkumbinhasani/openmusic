use std::collections::HashSet;

use spotify_client::catalog::Collection;

#[derive(Default)]
pub struct State {
    pub collection: Option<(String, Result<Collection, String>)>,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        if let Some((_, Ok(collection))) = &self.collection {
            urls.extend(collection.cover.clone());
            urls.extend(
                collection
                    .tracks
                    .iter()
                    .filter_map(|track| track.cover.clone()),
            );
        }
    }
}
