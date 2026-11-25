use std::time::Duration;

use spotify_client::{
    Api, Track, catalog,
    catalog::Artist,
    dealer::Message,
    pathfinder::PathfinderError,
    search::{self, Hit, SearchKind, SearchPage, SearchResults, Suggestions},
};

use crate::worker::{Ctx, Mutation, cache};

pub const SUGGEST_DELAY: Duration = Duration::from_millis(200);

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
    SearchTab {
        query: String,
        kind: SearchKind,
        offset: u32,
        limit: u32,
    },
    Suggest(String),
    RecentSearches,
    RememberSearches(Vec<String>),
    ForgetSearches(Vec<String>),
    Discover,
    Recommend(Box<Track>),
    Artist(String),
}

pub enum Event {
    Discovered(Discovery),
    Recommendations(Option<Mix>),
    Search {
        query: String,
        result: Result<Box<SearchResults>, String>,
    },
    SearchTab {
        query: String,
        kind: SearchKind,
        offset: u32,
        result: Result<SearchPage, String>,
    },
    Suggestions {
        prefix: String,
        result: Result<Suggestions, String>,
    },
    RecentSearches {
        result: Result<Vec<Hit>, String>,
        notice: Option<String>,
    },
    DirectTrack(Result<Track, String>),
    Artist {
        uri: String,
        result: Result<Artist, String>,
    },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        let tracks: Vec<&Track> = match self {
            Self::Search {
                result: Ok(results),
                ..
            } => return results.images(),
            Self::SearchTab {
                result: Ok(page), ..
            } => return page.images(),
            Self::Suggestions {
                result: Ok(suggestions),
                ..
            } => return suggestions.images(),
            Self::RecentSearches {
                result: Ok(hits), ..
            } => return hits.iter().filter_map(|hit| hit.image().cloned()).collect(),
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
        Command::SearchTab {
            query,
            kind,
            offset,
            limit,
        } => ctx.tasks.spawn_keyed(
            format!("search-{}", kind.chip()),
            search_tab(api, query, kind, offset, limit),
        ),
        Command::Suggest(prefix) => ctx.tasks.spawn_keyed("suggest", suggest(api, prefix)),
        Command::RecentSearches => ctx
            .tasks
            .spawn_keyed("recent-searches", recent_searches(api)),
        Command::RememberSearches(uris) => ctx.writes.push(Mutation::new(
            write_keys(&uris),
            remember_searches(api, uris),
        )),
        Command::ForgetSearches(uris) => ctx
            .writes
            .push(Mutation::new(write_keys(&uris), forget_searches(api, uris))),
        Command::Discover => ctx.tasks.spawn_keyed("discover", discover(api)),
        Command::Recommend(seed) => ctx.tasks.spawn_keyed("recommend", recommend(api, *seed)),
        Command::Artist(uri) => ctx.tasks.spawn(artist(api, uri)),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

fn search_error(error: anyhow::Error) -> String {
    match error.downcast_ref::<PathfinderError>() {
        Some(stale @ PathfinderError::StaleHash(_)) => stale.to_string(),
        _ => "Spotify search is unavailable. Try again.".to_owned(),
    }
}

fn write_keys(uris: &[String]) -> Vec<String> {
    uris.iter()
        .map(|uri| format!("recent-search:{uri}"))
        .collect()
}

async fn search(api: Api, query: String) -> crate::worker::Event {
    match catalog::parse_link(&query) {
        Err(error) => Event::Search {
            query,
            result: Err(error.to_string()),
        },
        Ok(Some(uri)) if uri.starts_with("spotify:track:") => Event::DirectTrack(
            catalog::tracks(&api, &[uri])
                .await
                .ok()
                .and_then(|mut tracks| tracks.pop())
                .ok_or_else(|| "Could not load this song from Spotify.".to_owned()),
        ),
        Ok(Some(_)) => Event::Search {
            query,
            result: Err("OpenMusic can’t open this kind of Spotify link.".to_owned()),
        },
        Ok(None) => {
            let result = search::search(&api, &query)
                .await
                .map(Box::new)
                .map_err(search_error);
            Event::Search { query, result }
        }
    }
    .into()
}

async fn search_tab(
    api: Api,
    query: String,
    kind: SearchKind,
    offset: u32,
    limit: u32,
) -> crate::worker::Event {
    let result = search::search_kind(&api, &query, kind, offset, limit)
        .await
        .map_err(search_error);
    Event::SearchTab {
        query,
        kind,
        offset,
        result,
    }
    .into()
}

async fn suggest(api: Api, prefix: String) -> crate::worker::Event {
    tokio::time::sleep(SUGGEST_DELAY).await;
    let result = search::suggestions(&api, &prefix)
        .await
        .map_err(search_error);
    Event::Suggestions { prefix, result }.into()
}

async fn read_recent(api: &Api) -> Result<Vec<Hit>, String> {
    search::recent_searches(api)
        .await
        .map_err(|_| "Couldn’t load your recent searches.".to_owned())
}

async fn recent_searches(api: Api) -> crate::worker::Event {
    Event::RecentSearches {
        result: read_recent(&api).await,
        notice: None,
    }
    .into()
}

async fn remember_searches(api: Api, uris: Vec<String>) -> Result<crate::worker::Event, String> {
    let _ = search::remember(&api, &uris).await;
    Ok(Event::RecentSearches {
        result: read_recent(&api).await,
        notice: None,
    }
    .into())
}

async fn forget_searches(api: Api, uris: Vec<String>) -> Result<crate::worker::Event, String> {
    let notice = search::forget(&api, &uris).await.err().map(|_| {
        if uris.len() > 1 {
            "Couldn’t clear your recent searches. Try again.".to_owned()
        } else {
            "Couldn’t remove this from your recent searches. Try again.".to_owned()
        }
    });
    Ok(Event::RecentSearches {
        result: read_recent(&api).await,
        notice,
    }
    .into())
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
