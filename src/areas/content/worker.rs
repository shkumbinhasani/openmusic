use spotify_client::{Api, catalog};

use crate::worker::Event;

pub async fn lyrics(api: Api, uri: String) -> Event {
    let found = catalog::lyrics(&api, &uri).await.ok().flatten();
    Event::Lyrics { uri, lyrics: found }
}
