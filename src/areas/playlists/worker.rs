use spotify_client::{Api, catalog};

use crate::worker::Event;

pub async fn collection(api: Api, uri: String) -> Event {
    let result = catalog::collection(&api, &uri)
        .await
        .map_err(|_| "Couldn’t load this. Try again.".to_owned());
    Event::Collection { uri, result }
}
