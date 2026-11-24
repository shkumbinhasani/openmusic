use spotify_client::{Api, catalog};

use crate::worker::Event;

pub async fn playlists(api: Api) -> Event {
    Event::Playlists(catalog::playlists(&api).await.unwrap_or_default())
}
