use spotify_client::{Api, Track, catalog};

use crate::worker::Event;

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Discovery {
    pub tracks: Vec<Track>,
    pub from_top_tracks: bool,
    pub mix: Option<Mix>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Mix {
    pub seed: String,
    pub tracks: Vec<Track>,
}

pub async fn search(api: Api, query: String) -> Event {
    match catalog::parse_link(&query) {
        Err(error) => Event::SearchDone(Err(error.to_string())),
        Ok(Some(uri)) if uri.starts_with("spotify:track:") => Event::DirectTrack(
            catalog::tracks(&api, &[uri])
                .await
                .ok()
                .and_then(|mut tracks| tracks.pop())
                .ok_or_else(|| "Could not load this song from Spotify.".to_owned()),
        ),
        Ok(Some(_)) => Event::SearchDone(Err(
            "Paste a Spotify song link. Albums and playlists aren’t supported yet.".to_owned(),
        )),
        Ok(None) => Event::SearchDone(
            catalog::search(&api, &query)
                .await
                .map_err(|_| "Spotify search is unavailable. Try again.".to_owned()),
        ),
    }
}

pub async fn discover(api: Api) -> Event {
    let tracks = catalog::liked(&api, 8).await.unwrap_or_default();
    let mix = match tracks.first() {
        Some(seed) => mix(&api, seed).await,
        None => None,
    };
    Event::Discovery(Discovery {
        tracks,
        from_top_tracks: false,
        mix,
    })
}

pub async fn recommend(api: Api, seed: Track) -> Event {
    Event::Recommendations(mix(&api, &seed).await)
}

pub async fn artist(api: Api, uri: String) -> Event {
    let result = catalog::artist(&api, &uri)
        .await
        .map_err(|_| "Couldn’t load this artist. Try again.".to_owned());
    Event::Artist { uri, result }
}

async fn mix(api: &Api, seed: &Track) -> Option<Mix> {
    let tracks = catalog::station(api, &seed.uri).await.ok()?;
    (!tracks.is_empty()).then(|| Mix {
        seed: seed.title.clone(),
        tracks,
    })
}
