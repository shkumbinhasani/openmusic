use spotify_client::{Api, Track, catalog, catalog::Artist, dealer::Message};

use crate::worker::{Ctx, cache};

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

pub enum Command {
    Search(String),
    Discover,
    Recommend(Box<Track>),
    Artist(String),
}

pub enum Event {
    Discovered(Discovery),
    Recommendations(Option<Mix>),
    SearchDone(Result<Vec<Track>, String>),
    DirectTrack(Result<Track, String>),
    Artist {
        uri: String,
        result: Result<Artist, String>,
    },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        let tracks: Vec<&Track> = match self {
            Self::SearchDone(Ok(tracks)) => tracks.iter().collect(),
            Self::DirectTrack(Ok(track)) => vec![track],
            Self::Discovered(home) => home
                .tracks
                .iter()
                .chain(home.mix.iter().flat_map(|mix| &mix.tracks))
                .collect(),
            Self::Recommendations(Some(mix)) => mix.tracks.iter().collect(),
            Self::Artist {
                result: Ok(artist), ..
            } => {
                return artist
                    .image
                    .iter()
                    .cloned()
                    .chain(
                        artist
                            .top_tracks
                            .iter()
                            .filter_map(|track| track.cover.clone()),
                    )
                    .chain(
                        artist
                            .releases
                            .iter()
                            .filter_map(|release| release.cover.clone()),
                    )
                    .chain(
                        artist
                            .related
                            .iter()
                            .filter_map(|related| related.image.clone()),
                    )
                    .collect();
            }
            _ => Vec::new(),
        };
        tracks
            .into_iter()
            .filter_map(|track| track.cover.clone())
            .collect()
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    let api = ctx.api.clone();
    match command {
        Command::Search(query) => ctx.tasks.spawn_keyed("search", search(api, query)),
        Command::Discover => ctx.tasks.spawn_keyed("discover", discover(api)),
        Command::Recommend(seed) => ctx.tasks.spawn_keyed("recommend", recommend(api, *seed)),
        Command::Artist(uri) => ctx.tasks.spawn(artist(api, uri)),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

async fn search(api: Api, query: String) -> crate::worker::Event {
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
    .into()
}

async fn discover(api: Api) -> crate::worker::Event {
    let tracks = catalog::liked(&api, 8).await.unwrap_or_default();
    let mix = match tracks.first() {
        Some(seed) => mix(&api, seed).await,
        None => None,
    };
    let discovery = Discovery {
        tracks,
        from_top_tracks: false,
        mix,
    };
    cache::save_discovery(&discovery);
    Event::Discovered(discovery).into()
}

async fn recommend(api: Api, seed: Track) -> crate::worker::Event {
    Event::Recommendations(mix(&api, &seed).await).into()
}

async fn artist(api: Api, uri: String) -> crate::worker::Event {
    let result = catalog::artist(&api, &uri)
        .await
        .map_err(|_| "Couldn’t load this artist. Try again.".to_owned());
    Event::Artist { uri, result }.into()
}

async fn mix(api: &Api, seed: &Track) -> Option<Mix> {
    let tracks = catalog::station(api, &seed.uri).await.ok()?;
    (!tracks.is_empty()).then(|| Mix {
        seed: seed.title.clone(),
        tracks,
    })
}
