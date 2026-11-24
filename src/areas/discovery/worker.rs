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
    match catalog::parse_track_input(&query) {
        Err(error) => Event::SearchDone(Err(error.to_string())),
        Ok(Some(uri)) => Event::DirectTrack(
            catalog::tracks(&api, &[uri])
                .await
                .ok()
                .and_then(|mut tracks| tracks.pop())
                .ok_or_else(|| "Could not load this song from Spotify.".to_owned()),
        ),
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

#[cfg(test)]
mod tests {
    use spotify_client::catalog::parse_track_input;

    #[test]
    fn distinguishes_tracks_from_search_queries() {
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        let uri = format!("spotify:track:{id}");
        for input in [
            uri.clone(),
            format!("https://open.spotify.com/track/{id}?si=123"),
            format!("https://open.spotify.com/intl-de/track/{id}"),
        ] {
            assert_eq!(parse_track_input(&input).unwrap().unwrap(), uri);
        }
        assert!(parse_track_input("Nina Simone").unwrap().is_none());
        assert!(parse_track_input("spotify:track:broken").is_err());
        assert!(parse_track_input("https://open.spotify.com/album/abc").is_err());
    }
}
