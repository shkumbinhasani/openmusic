use std::{sync::Mutex, time::Duration};

use spotify_client::{
    Api, Track,
    browse::{self, BrowseAll, BrowsePage},
    catalog,
    catalog::Artist,
    dealer::Message,
    home::{self, Home, Section},
    pathfinder::PathfinderError,
    search::{self, Hit, SearchKind, SearchPage, SearchResults, Suggestions},
};

use super::state::SectionSource;
use crate::worker::{Ctx, Mutation, cache};

pub const SUGGEST_DELAY: Duration = Duration::from_millis(200);

static CACHE: Mutex<()> = Mutex::new(());

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Discovery {
    pub home: Option<Home>,
    pub browse: Option<BrowseAll>,
}

impl Discovery {
    pub fn is_empty(&self) -> bool {
        self.home.is_none() && self.browse.is_none()
    }

    pub fn images(&self) -> Vec<String> {
        self.home
            .iter()
            .flat_map(Home::images)
            .chain(
                self.browse
                    .iter()
                    .flat_map(|all| &all.tiles)
                    .filter_map(|card| card.image.clone()),
            )
            .collect()
    }
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
    Home(String),
    HomeSection(String),
    BrowseAll,
    Browse {
        uri: String,
        offset: u32,
    },
    BrowseSection {
        uri: String,
        offset: u32,
    },
    Artist(String),
}

pub enum Event {
    Discovered(Discovery),
    Home {
        facet: String,
        result: Result<Box<Home>, String>,
    },
    BrowseAll(Result<BrowseAll, String>),
    Browse {
        uri: String,
        offset: u32,
        result: Result<Box<BrowsePage>, String>,
    },
    Section {
        uri: String,
        source: SectionSource,
        offset: u32,
        result: Result<Box<Section>, String>,
    },
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
    RecentSearches(Result<Vec<Hit>, String>),
    SearchesWritten {
        result: Result<Vec<Hit>, String>,
        notice: Option<String>,
    },
    DirectTrack {
        link: String,
        result: Result<Track, String>,
    },
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
            Self::RecentSearches(Ok(hits))
            | Self::SearchesWritten {
                result: Ok(hits), ..
            } => return hits.iter().filter_map(|hit| hit.image().cloned()).collect(),
            Self::DirectTrack {
                result: Ok(track), ..
            } => vec![track],
            Self::Discovered(discovery) => return discovery.images(),
            Self::Home {
                result: Ok(home), ..
            } => return home.images(),
            Self::BrowseAll(Ok(all)) => {
                return all
                    .tiles
                    .iter()
                    .filter_map(|card| card.image.clone())
                    .collect();
            }
            Self::Browse {
                result: Ok(page), ..
            } => return page.images(),
            Self::Section {
                result: Ok(section),
                ..
            } => return section.images().collect(),
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
        Command::Home(facet) => ctx.tasks.spawn_keyed("home", load_home(api, facet)),
        Command::HomeSection(uri) => ctx
            .tasks
            .spawn_keyed(format!("section:{uri}"), home_section(api, uri)),
        Command::BrowseAll => ctx.tasks.spawn_keyed("browse-all", browse_all(api)),
        Command::Browse { uri, offset } => ctx
            .tasks
            .spawn_keyed(format!("browse:{uri}"), browse_page(api, uri, offset)),
        Command::BrowseSection { uri, offset } => ctx
            .tasks
            .spawn_keyed(format!("section:{uri}"), browse_section(api, uri, offset)),
        Command::Artist(uri) => ctx.tasks.spawn_keyed("artist", artist(api, uri)),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

fn search_error(error: anyhow::Error) -> String {
    match error.downcast_ref::<PathfinderError>() {
        Some(stale @ PathfinderError::StaleHash(_)) => stale.to_string(),
        _ => "Spotify search is unavailable. Try again.".to_owned(),
    }
}

fn page_error(error: anyhow::Error, fallback: &str) -> String {
    match error.downcast_ref::<PathfinderError>() {
        Some(stale @ PathfinderError::StaleHash(_)) => stale.to_string(),
        _ => fallback.to_owned(),
    }
}

fn update_cache(update: impl FnOnce(&mut Discovery)) {
    let _guard = CACHE.lock();
    let mut discovery = cache::load()
        .and_then(|library| library.discovery)
        .unwrap_or_default();
    update(&mut discovery);
    cache::save_discovery(&discovery);
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
        Ok(Some(uri)) if uri.starts_with("spotify:track:") => Event::DirectTrack {
            result: catalog::tracks(&api, &[uri])
                .await
                .ok()
                .and_then(|mut tracks| tracks.pop())
                .ok_or_else(|| "Could not load this song from Spotify.".to_owned()),
            link: query,
        },
        Ok(Some(_)) => Event::Search {
            query,
            result: Err("OpenMusic can’t open this kind of Spotify link.".to_owned()),
        },
        Ok(None) => {
            let result = search::search_or_songs(&api, &query)
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
    let result = if kind == SearchKind::Tracks && offset == 0 {
        search::songs_page_or_songs(&api, &query, limit).await
    } else {
        search::search_kind(&api, &query, kind, offset, limit).await
    }
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
    Event::RecentSearches(read_recent(&api).await).into()
}

async fn remember_searches(api: Api, uris: Vec<String>) -> Result<crate::worker::Event, String> {
    let _ = search::remember(&api, &uris).await;
    Ok(Event::SearchesWritten {
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
    Ok(Event::SearchesWritten {
        result: read_recent(&api).await,
        notice,
    }
    .into())
}

async fn load_home(api: Api, facet: String) -> crate::worker::Event {
    let result = home::home(&api, &facet, &home::time_zone())
        .await
        .map_err(|error| page_error(error, "Couldn’t load Home. Try again."));
    if facet.is_empty()
        && let Ok(home) = &result
        && !home.sections.is_empty()
    {
        let home = home.clone();
        update_cache(move |discovery| discovery.home = Some(home));
    }
    Event::Home {
        facet,
        result: result.map(Box::new),
    }
    .into()
}

async fn home_section(api: Api, uri: String) -> crate::worker::Event {
    let result = home::home_section(&api, &uri, &home::time_zone())
        .await
        .map(Box::new)
        .map_err(|error| page_error(error, "Couldn’t load this section. Try again."));
    Event::Section {
        uri,
        source: SectionSource::Home,
        offset: 0,
        result,
    }
    .into()
}

async fn browse_all(api: Api) -> crate::worker::Event {
    let result = browse::browse_all(&api)
        .await
        .map_err(|error| page_error(error, "Couldn’t load Browse all. Try again."));
    if let Ok(all) = &result
        && !all.tiles.is_empty()
    {
        let all = all.clone();
        update_cache(move |discovery| discovery.browse = Some(all));
    }
    Event::BrowseAll(result).into()
}

async fn browse_page(api: Api, uri: String, offset: u32) -> crate::worker::Event {
    let result = browse::browse_page(&api, &uri, offset)
        .await
        .map(Box::new)
        .map_err(|error| page_error(error, "Couldn’t load this page. Try again."));
    Event::Browse {
        uri,
        offset,
        result,
    }
    .into()
}

async fn browse_section(api: Api, uri: String, offset: u32) -> crate::worker::Event {
    let result = browse::browse_section(&api, &uri, offset)
        .await
        .map(Box::new)
        .map_err(|error| page_error(error, "Couldn’t load this section. Try again."));
    Event::Section {
        uri,
        source: SectionSource::Browse,
        offset,
        result,
    }
    .into()
}

async fn artist(api: Api, uri: String) -> crate::worker::Event {
    let result = catalog::artist(&api, &uri)
        .await
        .map_err(|_| "Couldn’t load this artist. Try again.".to_owned());
    Event::Artist { uri, result }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_old_station_caches_as_empty() {
        let old =
            r#"{"tracks": [], "from_top_tracks": false, "mix": {"seed": "Song", "tracks": []}}"#;
        let discovery: Discovery = serde_json::from_str(old).unwrap();
        assert!(discovery.is_empty());
        let saved = Discovery {
            home: Some(Home {
                greeting: "Good morning".into(),
                ..Home::default()
            }),
            browse: None,
        };
        let text = serde_json::to_string(&saved).unwrap();
        let back: Discovery = serde_json::from_str(&text).unwrap();
        assert!(!back.is_empty());
        assert_eq!(
            back.home.map(|home| home.greeting).as_deref(),
            Some("Good morning")
        );
    }
}
