use spotify_client::{Api, catalog};

use crate::worker::{Event, User};

pub async fn playlists(api: Api, user: User) -> Event {
    let playlists = match user.name(&api).await {
        Ok(user) => catalog::playlists(&api, &user).await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    Event::Playlists(playlists)
}
