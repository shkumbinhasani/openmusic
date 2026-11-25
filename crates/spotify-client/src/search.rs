use std::collections::HashSet;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::Api,
    catalog::Track,
    entity::{self, Card, Kind},
    pathfinder::{self, Operation},
};

pub const SEARCH_DESKTOP: Operation = Operation::query(
    "searchDesktop",
    "eef7cc54888d91bdd6802623477873caa3948ae173a0c34fd86827b267e94c03",
);
pub const SEARCH_SUGGESTIONS: Operation = Operation::query(
    "searchSuggestions",
    "f244254b94c0e824d458ac216e0f8406a73f2f69a9ff52831a2f78fa774ff6ce",
);
pub const SEARCH_TRACKS: Operation = Operation::query(
    "searchTracks",
    "b02683192a98dde7966b5e6655a79eeb62713eab703eda9902c932818dd52751",
);
pub const SEARCH_ARTISTS: Operation = Operation::query(
    "searchArtists",
    "7bf95d754fdbe32c8b161fbbe54d1ae50974900df4dce4c8f1afcbcad153224d",
);
pub const SEARCH_ALBUMS: Operation = Operation::query(
    "searchAlbums",
    "202cb3305e31e5a0767ba7925f28bd728cf8f8b0217e6da43909056071cd70e9",
);
pub const SEARCH_PLAYLISTS: Operation = Operation::query(
    "searchPlaylists",
    "d520014e748f9ea44f7707d8df1819867ac1205e8b7f3e28f22fe5fc858921b1",
);
pub const SEARCH_PODCASTS: Operation = Operation::query(
    "searchPodcasts",
    "0195d9f61b43606d490bca64c3456e3593528cea6cc05c7e822c7c42beed0f4e",
);
pub const SEARCH_EPISODES: Operation = Operation::query(
    "searchEpisodes",
    "dbc56e15e7c1254f11d499e8647343bbf28a5594e14a8f5c8b1e7eee67a55740",
);
pub const SEARCH_AUDIOBOOKS: Operation = Operation::query(
    "searchAudiobooks",
    "e05ac765d02c084f8783d3c1572b23d57761c43f47eb8b87ce2f9ccced3fa068",
);
pub const SEARCH_AUTHORS: Operation = Operation::query(
    "searchAuthors",
    "4a9d403a7cbc7e19da5520d619a865472b35382b043bfa458154e73a5c6f46bd",
);
pub const SEARCH_USERS: Operation = Operation::query(
    "searchUsers",
    "8f358dd82e62f61dd4ceaa9f8cd0889e644c9b707f1b724fbfb356a757cb7e5a",
);
pub const SEARCH_GENRES: Operation = Operation::query(
    "searchGenres",
    "9e1c0e056c46239dd1956ea915b988913c87c04ce3dadccdb537774490266f46",
);
pub const RECENT_SEARCHES: Operation = Operation::query(
    "recentSearches",
    "ece96d7b82c820fb9938dd1df50dee3e53737d808c3883e3d01c467c423601b0",
);
pub const SAVE_RECENT_SEARCHES: Operation = Operation::mutation(
    "saveRecentSearches",
    "ece96d7b82c820fb9938dd1df50dee3e53737d808c3883e3d01c467c423601b0",
);
pub const REMOVE_RECENT_SEARCHES: Operation = Operation::mutation(
    "removeRecentSearches",
    "ece96d7b82c820fb9938dd1df50dee3e53737d808c3883e3d01c467c423601b0",
);

pub const ALL_LIMIT: u32 = 10;
pub const TOP_RESULTS: u32 = 5;
pub const TAB_TOP_RESULTS: u32 = 20;
pub const SONGS_PAGE: u32 = 20;
pub const CARDS_PAGE: u32 = 30;
pub const MAX_PAGE: u32 = 100;
pub const SUGGESTIONS_LIMIT: u32 = 30;
pub const RECENT_LIMIT: u32 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchKind {
    Tracks,
    Artists,
    Albums,
    Playlists,
    Podcasts,
    Episodes,
    Audiobooks,
    Authors,
    Users,
    Genres,
}

impl SearchKind {
    pub const ALL: [Self; 10] = [
        Self::Tracks,
        Self::Artists,
        Self::Albums,
        Self::Playlists,
        Self::Podcasts,
        Self::Episodes,
        Self::Audiobooks,
        Self::Authors,
        Self::Users,
        Self::Genres,
    ];

    pub fn operation(self) -> Operation {
        match self {
            Self::Tracks => SEARCH_TRACKS,
            Self::Artists => SEARCH_ARTISTS,
            Self::Albums => SEARCH_ALBUMS,
            Self::Playlists => SEARCH_PLAYLISTS,
            Self::Podcasts => SEARCH_PODCASTS,
            Self::Episodes => SEARCH_EPISODES,
            Self::Audiobooks => SEARCH_AUDIOBOOKS,
            Self::Authors => SEARCH_AUTHORS,
            Self::Users => SEARCH_USERS,
            Self::Genres => SEARCH_GENRES,
        }
    }

    pub fn reply_key(self) -> &'static str {
        match self {
            Self::Tracks => "tracksV2",
            Self::Artists => "artists",
            Self::Albums => "albumsV2",
            Self::Playlists => "playlists",
            Self::Podcasts => "podcasts",
            Self::Episodes => "episodes",
            Self::Audiobooks => "audiobooks",
            Self::Authors => "authors",
            Self::Users => "users",
            Self::Genres => "genres",
        }
    }

    pub fn chip(self) -> &'static str {
        match self {
            Self::Tracks => "TRACKS",
            Self::Artists => "ARTISTS",
            Self::Albums => "ALBUMS",
            Self::Playlists => "PLAYLISTS",
            Self::Podcasts => "PODCASTS",
            Self::Episodes => "EPISODES",
            Self::Audiobooks => "AUDIOBOOKS",
            Self::Authors => "AUTHORS",
            Self::Users => "USERS",
            Self::Genres => "GENRES",
        }
    }

    pub fn from_chip(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.chip() == name)
    }

    pub fn page_size(self) -> u32 {
        match self {
            Self::Tracks => SONGS_PAGE,
            _ => CARDS_PAGE,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Paged<T> {
    pub items: Vec<T>,
    pub total: u32,
    pub next: Option<u32>,
}

impl<T> Default for Paged<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            total: 0,
            next: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub card: Card,
    pub track: Option<Track>,
}

impl Hit {
    pub fn of_track(track: Track) -> Self {
        let card = Card {
            uri: track.uri.clone(),
            kind: Kind::Track,
            title: track.title.clone(),
            subtitle: track.artist.clone(),
            image: track.cover.clone(),
            format: None,
            color: None,
            reason: None,
            explicit: track.explicit,
            playable: true,
        };
        Self {
            card,
            track: Some(track),
        }
    }

    pub fn of_card(card: Card) -> Self {
        Self { card, track: None }
    }

    pub fn uri(&self) -> &str {
        &self.card.uri
    }

    pub fn image(&self) -> Option<&String> {
        self.card.image.as_ref()
    }

    pub fn playable_track(&self) -> Option<Track> {
        if self.card.kind != Kind::Track {
            return None;
        }
        Some(self.track.clone().unwrap_or_else(|| Track {
            uri: self.card.uri.clone(),
            title: self.card.title.clone(),
            artist: self.card.subtitle.clone(),
            cover: self.card.image.clone(),
            explicit: self.card.explicit,
            ..Track::default()
        }))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchResults {
    pub query: String,
    pub top: Vec<Hit>,
    pub featured: Vec<Card>,
    pub tracks: Paged<Track>,
    pub cards: Vec<(SearchKind, Paged<Card>)>,
    pub chips: Vec<SearchKind>,
    pub unplayable: HashSet<String>,
}

impl SearchResults {
    pub fn featuring(&self) -> Option<&str> {
        self.top
            .iter()
            .find(|hit| hit.card.kind == Kind::Artist)
            .map(|hit| hit.card.title.as_str())
    }

    pub fn cards_of(&self, kind: SearchKind) -> Option<&Paged<Card>> {
        self.cards
            .iter()
            .find(|(found, _)| *found == kind)
            .map(|(_, page)| page)
    }

    pub fn total(&self, kind: SearchKind) -> u32 {
        match kind {
            SearchKind::Tracks => self.tracks.total,
            kind => self.cards_of(kind).map_or(0, |page| page.total),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.top.is_empty()
            && self.tracks.items.is_empty()
            && self.featured.is_empty()
            && self.cards.iter().all(|(_, page)| page.items.is_empty())
    }

    pub fn images(&self) -> Vec<String> {
        self.top
            .iter()
            .filter_map(|hit| hit.image().cloned())
            .chain(self.featured.iter().filter_map(|card| card.image.clone()))
            .chain(
                self.tracks
                    .items
                    .iter()
                    .filter_map(|track| track.cover.clone()),
            )
            .chain(
                self.cards
                    .iter()
                    .flat_map(|(_, page)| &page.items)
                    .filter_map(|card| card.image.clone()),
            )
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SearchPage {
    Tracks {
        page: Paged<Track>,
        unplayable: HashSet<String>,
    },
    Cards(Paged<Card>),
}

impl SearchPage {
    pub fn total(&self) -> u32 {
        match self {
            Self::Tracks { page, .. } => page.total,
            Self::Cards(page) => page.total,
        }
    }

    pub fn next(&self) -> Option<u32> {
        match self {
            Self::Tracks { page, .. } => page.next,
            Self::Cards(page) => page.next,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Tracks { page, .. } => page.items.len(),
            Self::Cards(page) => page.items.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn append(&mut self, more: SearchPage) -> usize {
        fn merge<T>(page: &mut Paged<T>, more: Paged<T>, uri: impl Fn(&T) -> &str) -> usize {
            let known: HashSet<String> =
                page.items.iter().map(|item| uri(item).to_owned()).collect();
            let before = page.items.len();
            page.items.extend(
                more.items
                    .into_iter()
                    .filter(|item| !known.contains(uri(item))),
            );
            let added = page.items.len() - before;
            page.total = more.total.max(page.total);
            page.next = if added == 0 { None } else { more.next };
            added
        }
        match (self, more) {
            (
                Self::Tracks { page, unplayable },
                Self::Tracks {
                    page: more,
                    unplayable: blocked,
                },
            ) => {
                unplayable.extend(blocked);
                merge(page, more, |track: &Track| track.uri.as_str())
            }
            (Self::Cards(page), Self::Cards(more)) => {
                merge(page, more, |card: &Card| card.uri.as_str())
            }
            (current, more) => {
                *current = more;
                current.len()
            }
        }
    }

    pub fn images(&self) -> Vec<String> {
        match self {
            Self::Tracks { page, .. } => page
                .items
                .iter()
                .filter_map(|track| track.cover.clone())
                .collect(),
            Self::Cards(page) => page
                .items
                .iter()
                .filter_map(|card| card.image.clone())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Suggestion {
    Query(String),
    Entity(Box<Hit>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Suggestions {
    pub query: String,
    pub items: Vec<Suggestion>,
}

impl Suggestions {
    pub fn images(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|item| match item {
                Suggestion::Entity(hit) => hit.image().cloned(),
                Suggestion::Query(_) => None,
            })
            .collect()
    }
}

pub fn all_variables(query: &str) -> Value {
    json!({
        "searchTerm": query,
        "offset": 0,
        "limit": ALL_LIMIT,
        "numberOfTopResults": TOP_RESULTS,
        "includeAudiobooks": true,
        "includeArtistHasConcertsField": false,
        "includePreReleases": true,
        "includeLocalConcertsField": false,
        "includeAuthors": false,
    })
}

pub fn kind_variables(query: &str, kind: SearchKind, offset: u32, limit: u32) -> Value {
    json!({
        "searchTerm": query,
        "offset": offset,
        "limit": limit.clamp(1, MAX_PAGE),
        "numberOfTopResults": TAB_TOP_RESULTS,
        "includeAudiobooks": true,
        "includePreReleases": kind == SearchKind::Audiobooks,
        "includeAlbumPreReleases": false,
        "includeAuthors": kind == SearchKind::Authors,
        "includeEpisodeContentRatingsV2": false,
    })
}

pub fn page_limit(kind: SearchKind, offset: u32, total: Option<u32>) -> u32 {
    let size = kind.page_size();
    match total {
        Some(total) if total > offset => size.min(total - offset),
        _ => size,
    }
}

pub fn suggestion_variables(prefix: &str) -> Value {
    json!({
        "query": prefix,
        "limit": SUGGESTIONS_LIMIT,
        "numberOfTopResults": SUGGESTIONS_LIMIT,
        "offset": 0,
        "includeAuthors": false,
        "includeAlbumPreReleases": false,
        "includeEpisodeContentRatingsV2": false,
    })
}

pub fn recent_variables() -> Value {
    json!({
        "limit": RECENT_LIMIT,
        "includeAuthors": false,
        "includeEpisodeContentRatingsV2": false,
    })
}

pub fn uris_variables(uris: &[String]) -> Value {
    json!({ "uris": uris })
}

fn count(value: &Value) -> u32 {
    value
        .as_u64()
        .map_or(0, |count| count.min(u64::from(u32::MAX)) as u32)
}

fn hit_item(value: &Value) -> &Value {
    if value["item"].is_object() {
        &value["item"]
    } else {
        value
    }
}

fn wrapped_data(value: &Value) -> &Value {
    let item = hit_item(value);
    if item["data"].is_object() {
        &item["data"]
    } else {
        item
    }
}

pub fn hit(value: &Value) -> Option<Hit> {
    let item = hit_item(value);
    let card = entity::card(item)?;
    let track = match card.kind {
        Kind::Track => entity::track(item),
        _ => None,
    };
    Some(Hit { card, track })
}

fn items(list: &Value) -> impl Iterator<Item = &Value> {
    list["items"].as_array().into_iter().flatten()
}

fn next_offset(list: &Value, offset: u32, received: usize, total: u32) -> Option<u32> {
    let next = u32::try_from(list["pagingInfo"]["nextOffset"].as_u64()?).ok()?;
    (received > 0 && next > offset && (total == 0 || next < total)).then_some(next)
}

fn track_page(list: &Value, offset: u32, unplayable: &mut HashSet<String>) -> Paged<Track> {
    let tracks: Vec<Track> = items(list)
        .filter_map(|value| {
            let track = entity::track(hit_item(value))?;
            if !entity::playable(wrapped_data(value)) {
                unplayable.insert(track.uri.clone());
            }
            Some(track)
        })
        .collect();
    let received = items(list).count();
    let total = count(&list["totalCount"]);
    Paged {
        next: next_offset(list, offset, received, total),
        items: tracks,
        total,
    }
}

fn card_page(list: &Value, offset: u32) -> Paged<Card> {
    let received = items(list).count();
    let total = count(&list["totalCount"]);
    Paged {
        items: items(list)
            .filter_map(|value| entity::card(hit_item(value)))
            .collect(),
        total,
        next: next_offset(list, offset, received, total),
    }
}

pub fn parse_results(query: &str, data: &Value) -> SearchResults {
    let search = &data["searchV2"];
    let mut unplayable = HashSet::new();
    let top: Vec<Hit> = search["topResultsV2"]["itemsV2"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let found = hit(value)?;
            if found.card.kind == Kind::Track && !entity::playable(wrapped_data(value)) {
                unplayable.insert(found.card.uri.clone());
            }
            Some(found)
        })
        .collect();
    let featured = search["topResultsV2"]["featured"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(entity::card)
        .collect();
    let tracks = track_page(&search["tracksV2"], 0, &mut unplayable);
    let cards: Vec<(SearchKind, Paged<Card>)> = SearchKind::ALL
        .into_iter()
        .filter(|kind| *kind != SearchKind::Tracks)
        .map(|kind| (kind, card_page(&search[kind.reply_key()], 0)))
        .collect();
    let mut results = SearchResults {
        query: search["query"].as_str().unwrap_or(query).to_owned(),
        top,
        featured,
        tracks,
        cards,
        chips: Vec::new(),
        unplayable,
    };
    let ordered: Vec<SearchKind> = search["chipOrder"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|chip| chip["typeName"].as_str().and_then(SearchKind::from_chip))
        .collect();
    let ordered = if ordered.is_empty() {
        SearchKind::ALL.to_vec()
    } else {
        ordered
    };
    let mut seen = HashSet::new();
    results.chips = ordered
        .into_iter()
        .filter(|kind| seen.insert(*kind) && results.total(*kind) > 0)
        .collect();
    results
}

pub fn parse_page(kind: SearchKind, data: &Value, offset: u32) -> SearchPage {
    let list = &data["searchV2"][kind.reply_key()];
    match kind {
        SearchKind::Tracks => {
            let mut unplayable = HashSet::new();
            let page = track_page(list, offset, &mut unplayable);
            SearchPage::Tracks { page, unplayable }
        }
        _ => SearchPage::Cards(card_page(list, offset)),
    }
}

pub fn parse_suggestions(prefix: &str, data: &Value) -> Suggestions {
    let search = &data["searchV2"];
    let items = search["topResultsV2"]["itemsV2"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let item = hit_item(value);
            if item["__typename"] == "SearchAutoCompleteEntity" {
                let text = item["data"]["text"].as_str()?.trim();
                return (!text.is_empty()).then(|| Suggestion::Query(text.to_owned()));
            }
            hit(item).map(|hit| Suggestion::Entity(Box::new(hit)))
        })
        .collect();
    Suggestions {
        query: search["query"].as_str().unwrap_or(prefix).to_owned(),
        items,
    }
}

pub fn parse_recent(data: &Value) -> Vec<Hit> {
    let mut seen = HashSet::new();
    data["recentSearches"]["recentSearchesItems"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(hit)
        .filter(|hit| seen.insert(hit.card.uri.clone()))
        .collect()
}

pub fn check_write(data: &Value, key: &str) -> Result<()> {
    let reply = &data[key];
    if reply["__typename"] == "GenericError" {
        let message = reply["message"]
            .as_str()
            .filter(|message| !message.is_empty())
            .unwrap_or("Spotify could not update your recent searches.");
        bail!(message.to_owned());
    }
    Ok(())
}

pub async fn search(api: &Api, query: &str) -> Result<SearchResults> {
    let data = pathfinder::query(api, SEARCH_DESKTOP, all_variables(query)).await?;
    Ok(parse_results(query, &data))
}

pub async fn search_kind(
    api: &Api,
    query: &str,
    kind: SearchKind,
    offset: u32,
    limit: u32,
) -> Result<SearchPage> {
    let variables = kind_variables(query, kind, offset, limit);
    let data = pathfinder::query(api, kind.operation(), variables).await?;
    Ok(parse_page(kind, &data, offset))
}

pub async fn suggestions(api: &Api, prefix: &str) -> Result<Suggestions> {
    let data = pathfinder::query(api, SEARCH_SUGGESTIONS, suggestion_variables(prefix)).await?;
    Ok(parse_suggestions(prefix, &data))
}

pub async fn recent_searches(api: &Api) -> Result<Vec<Hit>> {
    let data = pathfinder::query(api, RECENT_SEARCHES, recent_variables()).await?;
    Ok(parse_recent(&data))
}

pub async fn remember(api: &Api, uris: &[String]) -> Result<()> {
    if uris.is_empty() {
        return Ok(());
    }
    let data = pathfinder::mutate(api, SAVE_RECENT_SEARCHES, uris_variables(uris)).await?;
    check_write(&data, "saveToRecentSearches")
}

pub async fn forget(api: &Api, uris: &[String]) -> Result<()> {
    if uris.is_empty() {
        return Ok(());
    }
    let data = pathfinder::mutate(api, REMOVE_RECENT_SEARCHES, uris_variables(uris)).await?;
    check_write(&data, "removeFromRecentSearches")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cover(name: &str) -> Value {
        json!({"sources": [
            {"url": format!("https://i.scdn.co/image/{name}-64"), "width": 64, "height": 64},
            {"url": format!("https://i.scdn.co/image/{name}-300"), "width": 300, "height": 300}
        ]})
    }

    fn track(name: &str, id: &str, playable: bool) -> Value {
        json!({"__typename": "TrackResponseWrapper", "data": {
            "__typename": "Track",
            "name": name,
            "uri": format!("spotify:track:{id}"),
            "albumOfTrack": {"name": "Pablo Honey", "uri": "spotify:album:3gBVdu4a1MMJVMy6vwPEb8", "coverArt": cover(id)},
            "artists": {"items": [{"profile": {"name": "Radiohead"}, "uri": "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb"}]},
            "contentRating": {"label": "EXPLICIT"},
            "duration": {"totalMilliseconds": 238640},
            "playability": {"playable": playable, "reason": if playable { "PLAYABLE" } else { "NOT_AVAILABLE" }},
            "trackMediaType": "AUDIO"
        }})
    }

    fn artist(name: &str, id: &str) -> Value {
        json!({"__typename": "ArtistResponseWrapper", "data": {
            "__typename": "Artist", "profile": {"name": name},
            "uri": format!("spotify:artist:{id}"), "visuals": {"avatarImage": cover(id)}
        }})
    }

    fn album(name: &str, id: &str) -> Value {
        json!({"__typename": "AlbumResponseWrapper", "data": {
            "__typename": "Album", "name": name, "type": "ALBUM", "date": {"year": 1997},
            "artists": {"items": [{"profile": {"name": "Radiohead"}, "uri": "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb"}]},
            "coverArt": cover(id), "uri": format!("spotify:album:{id}")
        }})
    }

    fn playlist(name: &str, id: &str) -> Value {
        json!({"__typename": "PlaylistResponseWrapper", "data": {
            "__typename": "Playlist", "name": name, "format": "inspiredby-mix", "description": "",
            "uri": format!("spotify:playlist:{id}"),
            "images": {"items": [{"sources": [{"url": format!("https://pickasso.spotifycdn.com/{id}"), "width": null, "height": null}]}]},
            "ownerV2": {"data": {"name": "Spotify", "uri": "spotify:user:spotify"}}
        }})
    }

    fn desktop_reply() -> Value {
        json!({"searchV2": {
            "query": "radiohead",
            "topResultsV2": {
                "itemsV2": [
                    {"item": track("Creep", "70LcF31zb1H0PyJoS1Sx1r", true), "matchedFields": []},
                    {"item": artist("Radiohead", "4Z8W4fKeB5YxbusRsdQVPb"), "matchedFields": []},
                    {"item": {"__typename": "UnknownTypeWrapper", "_uri": "spotify:concert:x"}, "matchedFields": []},
                    {"item": {"__typename": "AlbumResponseWrapper", "data": {"__typename": "RestrictedContent"}}, "matchedFields": []}
                ],
                "featured": [
                    playlist("Radiohead Radio", "37i9dQZF1E4Redacted002"),
                    {"__typename": "PlaylistResponseWrapper", "data": {"__typename": "NotFound"}}
                ]
            },
            "tracksV2": {
                "items": [
                    {"item": track("Creep", "70LcF31zb1H0PyJoS1Sx1r", true), "matchedFields": []},
                    {"item": track("Karma Police", "63OQupATfueTdZMWTxW03A", false), "matchedFields": []}
                ],
                "totalCount": 433
            },
            "albumsV2": {"__typename": "AlbumOrPrereleasePage", "items": [album("OK Computer", "6dVIqQ8qmQ5GBnJ9shOYGE")], "totalCount": 203},
            "artists": {"items": [artist("Radiohead", "4Z8W4fKeB5YxbusRsdQVPb"), artist("Radiohead", "0aYAhmQ7fEGLMqnVsFj6Nk")], "totalCount": 119},
            "playlists": {"items": [playlist("This Is Radiohead", "37i9dQZF1DZ06evO1KPu1E")], "totalCount": 200},
            "podcasts": {"items": [], "totalCount": 0},
            "episodes": {"items": [], "totalCount": 1000},
            "audiobooks": {"items": [], "totalCount": 0},
            "users": {"items": [{"__typename": "UserResponseWrapper", "data": {"__typename": "User", "displayName": "Someone", "username": "someone", "uri": "spotify:user:someone"}}], "totalCount": 831},
            "genres": {"items": [], "totalCount": 0},
            "chipOrder": {"items": [
                {"typeName": "TRACKS"}, {"typeName": "VIDEOS"}, {"typeName": "ARTISTS"},
                {"typeName": "ALBUMS"}, {"typeName": "PLAYLISTS"}, {"typeName": "USERS"},
                {"typeName": "AUDIOBOOKS"}, {"typeName": "EPISODES"}, {"typeName": "PODCASTS"},
                {"typeName": "GENRES"}
            ]}
        }})
    }

    #[test]
    fn builds_the_desktop_search_variables() {
        assert_eq!(
            all_variables("radiohead"),
            json!({"searchTerm": "radiohead", "offset": 0, "limit": 10, "numberOfTopResults": 5,
                "includeAudiobooks": true, "includeArtistHasConcertsField": false,
                "includePreReleases": true, "includeLocalConcertsField": false, "includeAuthors": false})
        );
        let body = pathfinder::body(SEARCH_DESKTOP, all_variables("radiohead"));
        assert_eq!(body["operationName"], "searchDesktop");
        assert_eq!(
            body["extensions"]["persistedQuery"]["sha256Hash"],
            SEARCH_DESKTOP.hash
        );
    }

    #[test]
    fn maps_the_all_tab() {
        let results = parse_results("radiohead", &desktop_reply());
        assert_eq!(results.query, "radiohead");
        let top: Vec<(Kind, &str)> = results
            .top
            .iter()
            .map(|hit| (hit.card.kind, hit.card.title.as_str()))
            .collect();
        assert_eq!(top, [(Kind::Track, "Creep"), (Kind::Artist, "Radiohead")]);
        assert_eq!(
            results.top[0]
                .track
                .as_ref()
                .map(|track| track.album.as_str()),
            Some("Pablo Honey")
        );
        assert_eq!(results.top[1].track, None);
        assert_eq!(results.featuring(), Some("Radiohead"));
        assert_eq!(results.featured.len(), 1);
        assert_eq!(results.featured[0].title, "Radiohead Radio");
        assert_eq!(results.tracks.items.len(), 2);
        assert_eq!(results.tracks.total, 433);
        assert!(
            results
                .unplayable
                .contains("spotify:track:63OQupATfueTdZMWTxW03A")
        );
        assert!(
            !results
                .unplayable
                .contains("spotify:track:70LcF31zb1H0PyJoS1Sx1r")
        );
        let artists = results.cards_of(SearchKind::Artists).unwrap();
        assert_eq!(artists.items.len(), 2);
        assert_eq!(artists.total, 119);
        assert_eq!(
            results.cards_of(SearchKind::Albums).unwrap().items[0].subtitle,
            "1997 • Radiohead"
        );
        assert_eq!(results.total(SearchKind::Episodes), 1000);
        assert!(!results.is_empty());
        assert!(
            results
                .images()
                .contains(&"https://i.scdn.co/image/70LcF31zb1H0PyJoS1Sx1r-300".to_owned())
        );
    }

    #[test]
    fn orders_chips_like_the_server_and_drops_empty_kinds() {
        let results = parse_results("radiohead", &desktop_reply());
        assert_eq!(
            results.chips,
            [
                SearchKind::Tracks,
                SearchKind::Artists,
                SearchKind::Albums,
                SearchKind::Playlists,
                SearchKind::Users,
                SearchKind::Episodes,
            ]
        );
        let mut reply = desktop_reply();
        reply["searchV2"]["chipOrder"] = Value::Null;
        let results = parse_results("radiohead", &reply);
        assert_eq!(results.chips[0], SearchKind::Tracks);
        assert_eq!(results.chips[1], SearchKind::Artists);
        assert!(!results.chips.contains(&SearchKind::Genres));
    }

    #[test]
    fn reports_nothing_found() {
        let reply = json!({"searchV2": {"query": "zzqx", "topResultsV2": {"itemsV2": [], "featured": []},
            "tracksV2": {"items": [], "totalCount": 0}, "chipOrder": {"items": []}}});
        let results = parse_results("zzqx", &reply);
        assert!(results.is_empty());
        assert!(results.chips.is_empty());
        assert_eq!(results.featuring(), None);
    }

    #[test]
    fn builds_tab_variables_and_limits() {
        assert_eq!(
            kind_variables("radiohead", SearchKind::Tracks, 20, 20),
            json!({"searchTerm": "radiohead", "offset": 20, "limit": 20, "numberOfTopResults": 20,
                "includeAudiobooks": true, "includePreReleases": false, "includeAlbumPreReleases": false,
                "includeAuthors": false, "includeEpisodeContentRatingsV2": false})
        );
        let books = kind_variables("dune", SearchKind::Audiobooks, 0, 30);
        assert_eq!(books["includePreReleases"], true);
        let authors = kind_variables("stephen king", SearchKind::Authors, 0, 500);
        assert_eq!(authors["includeAuthors"], true);
        assert_eq!(authors["limit"], 100);
        assert_eq!(page_limit(SearchKind::Tracks, 0, None), 20);
        assert_eq!(page_limit(SearchKind::Artists, 0, None), 30);
        assert_eq!(page_limit(SearchKind::Artists, 110, Some(119)), 9);
        assert_eq!(page_limit(SearchKind::Tracks, 40, Some(440)), 20);
        assert_eq!(SearchKind::Episodes.operation().name, "searchEpisodes");
        assert_eq!(SearchKind::Users.reply_key(), "users");
        assert_eq!(
            SearchKind::from_chip("PODCASTS"),
            Some(SearchKind::Podcasts)
        );
        assert_eq!(SearchKind::from_chip("VIDEOS"), None);
    }

    #[test]
    fn pages_through_a_tab() {
        let reply = json!({"searchV2": {"query": "radiohead", "tracksV2": {
            "items": [
                {"item": track("Creep", "70LcF31zb1H0PyJoS1Sx1r", true), "matchedFields": []},
                {"item": track("Nude", "2vVKQsNJYJiX3rOn6DrhfT", false), "matchedFields": []}
            ],
            "pagingInfo": {"limit": 20, "nextOffset": 40},
            "totalCount": 440
        }}});
        let SearchPage::Tracks { page, unplayable } = parse_page(SearchKind::Tracks, &reply, 20)
        else {
            panic!("songs come back as tracks");
        };
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.next, Some(40));
        assert_eq!(page.total, 440);
        assert!(unplayable.contains("spotify:track:2vVKQsNJYJiX3rOn6DrhfT"));

        let reply = json!({"searchV2": {"query": "rock", "genres": {
            "items": [],
            "pagingInfo": {"limit": 20, "nextOffset": null},
            "totalCount": 6
        }}});
        let page = parse_page(SearchKind::Genres, &reply, 20);
        assert!(page.is_empty());
        assert_eq!(page.next(), None);

        let reply = json!({"searchV2": {"query": "stephen king", "authors": {
            "items": [
                {"__typename": "AuthorResponseWrapper", "data": {"__typename": "NotFound"}},
                {"__typename": "AuthorResponseWrapper", "data": {"__typename": "AuthorV2", "name": "Stephen King", "uri": "spotify:author:3K0Bhm7TfhTDpg2xVJlCQS"}}
            ],
            "pagingInfo": {"limit": 20, "nextOffset": 20},
            "totalCount": 28
        }}});
        let SearchPage::Cards(page) = parse_page(SearchKind::Authors, &reply, 0) else {
            panic!("authors come back as cards");
        };
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].kind, Kind::Author);
        assert_eq!(page.next, Some(20));
    }

    #[test]
    fn appends_pages_without_repeats() {
        let card = |id: &str| entity::Card {
            uri: format!("spotify:artist:{id}"),
            kind: Kind::Artist,
            title: id.to_owned(),
            subtitle: "Artist".into(),
            image: None,
            format: None,
            color: None,
            reason: None,
            explicit: false,
            playable: true,
        };
        let mut page = SearchPage::Cards(Paged {
            items: vec![card("a"), card("b")],
            total: 5,
            next: Some(2),
        });
        let added = page.append(SearchPage::Cards(Paged {
            items: vec![card("b"), card("c")],
            total: 5,
            next: Some(4),
        }));
        assert_eq!(added, 1);
        assert_eq!(page.len(), 3);
        assert_eq!(page.next(), Some(4));
        let added = page.append(SearchPage::Cards(Paged {
            items: vec![card("a")],
            total: 5,
            next: Some(6),
        }));
        assert_eq!(added, 0);
        assert_eq!(page.next(), None);

        let song = |id: &str| Track {
            uri: format!("spotify:track:{id}"),
            ..Track::default()
        };
        let mut songs = SearchPage::Tracks {
            page: Paged {
                items: vec![song("a")],
                total: 3,
                next: Some(1),
            },
            unplayable: HashSet::new(),
        };
        songs.append(SearchPage::Tracks {
            page: Paged {
                items: vec![song("b")],
                total: 3,
                next: Some(2),
            },
            unplayable: HashSet::from(["spotify:track:b".to_owned()]),
        });
        let SearchPage::Tracks { page, unplayable } = &songs else {
            panic!("still songs");
        };
        assert_eq!(page.items.len(), 2);
        assert!(unplayable.contains("spotify:track:b"));
    }

    #[test]
    fn turns_tracks_into_hits() {
        let track = Track {
            uri: "spotify:track:70LcF31zb1H0PyJoS1Sx1r".into(),
            title: "Creep".into(),
            artist: "Radiohead".into(),
            cover: Some("https://i.scdn.co/image/creep".into()),
            explicit: true,
            ..Track::default()
        };
        let found = Hit::of_track(track.clone());
        assert_eq!(found.card.kind, Kind::Track);
        assert_eq!(found.card.subtitle, "Radiohead");
        assert!(found.card.explicit);
        assert_eq!(found.playable_track(), Some(track));
        let bare = Hit::of_card(found.card.clone());
        assert_eq!(
            bare.playable_track().map(|track| track.title),
            Some("Creep".to_owned())
        );
        let artist = parse_recent(
            &json!({"recentSearches": {"recentSearchesItems": {"items": [
                artist("Radiohead", "4Z8W4fKeB5YxbusRsdQVPb")
            ]}}}),
        );
        assert_eq!(artist[0].playable_track(), None);
    }

    #[test]
    fn stops_paging_when_the_server_repeats_itself() {
        let list = json!({"items": [{}, {}], "pagingInfo": {"nextOffset": 20}, "totalCount": 100});
        assert_eq!(next_offset(&list, 20, 2, 100), None);
        assert_eq!(next_offset(&list, 0, 2, 100), Some(20));
        let list = json!({"items": [], "pagingInfo": {"nextOffset": 0}, "totalCount": 0});
        assert_eq!(next_offset(&list, 0, 0, 0), None);
        let list = json!({"items": [{}], "pagingInfo": {"nextOffset": 1000}, "totalCount": 1000});
        assert_eq!(next_offset(&list, 980, 20, 1000), None);
    }

    #[test]
    fn maps_suggestions() {
        assert_eq!(
            suggestion_variables("radioh"),
            json!({"query": "radioh", "limit": 30, "numberOfTopResults": 30, "offset": 0,
                "includeAuthors": false, "includeAlbumPreReleases": false, "includeEpisodeContentRatingsV2": false})
        );
        let reply = json!({"searchV2": {"__typename": "SearchResponse", "query": "radioh", "topResultsV2": {"itemsV2": [
            {"__typename": "TopResultHit", "item": {"__typename": "SearchAutoCompleteEntity", "data": {"text": "radiohead", "uri": "spotify:search:radiohead"}}},
            {"__typename": "TopResultHit", "item": {"__typename": "SearchAutoCompleteEntity", "data": {"text": "radiohead let down", "uri": "spotify:search:radiohead+let+down"}}},
            {"__typename": "TopResultHit", "item": artist("Radiohead", "4Z8W4fKeB5YxbusRsdQVPb")},
            {"item": track("Creep", "70LcF31zb1H0PyJoS1Sx1r", true)},
            {"item": {"__typename": "SearchAutoCompleteEntity", "data": {"text": "  "}}},
            {"item": {"__typename": "PlaylistResponseWrapper", "data": {"__typename": "NotFound"}}}
        ]}}});
        let suggestions = parse_suggestions("radioh", &reply);
        assert_eq!(suggestions.query, "radioh");
        assert_eq!(suggestions.items.len(), 4);
        assert_eq!(
            suggestions.items[0],
            Suggestion::Query("radiohead".to_owned())
        );
        assert_eq!(
            suggestions.items[1],
            Suggestion::Query("radiohead let down".to_owned())
        );
        let Suggestion::Entity(artist) = &suggestions.items[2] else {
            panic!("an artist row");
        };
        assert_eq!(artist.card.kind, Kind::Artist);
        let Suggestion::Entity(song) = &suggestions.items[3] else {
            panic!("a song row");
        };
        assert_eq!(
            song.track.as_ref().map(|track| track.title.as_str()),
            Some("Creep")
        );
        assert_eq!(suggestions.images().len(), 2);
    }

    #[test]
    fn maps_recent_searches() {
        assert_eq!(
            recent_variables(),
            json!({"limit": 50, "includeAuthors": false, "includeEpisodeContentRatingsV2": false})
        );
        let reply = json!({"recentSearches": {"__typename": "RecentSearchesResult",
        "recentSearchesItems": {"__typename": "RecentSearchesPage", "items": [
            track("Creep", "70LcF31zb1H0PyJoS1Sx1r", true),
            artist("Radiohead", "4Z8W4fKeB5YxbusRsdQVPb"),
            album("OK Computer", "6dVIqQ8qmQ5GBnJ9shOYGE"),
            artist("Radiohead", "4Z8W4fKeB5YxbusRsdQVPb"),
            {"__typename": "PlaylistResponseWrapper", "data": {"__typename": "NotFound"}},
            playlist("Radiohead Radio", "37i9dQZF1E4Redacted002")
        ]}}});
        let recent = parse_recent(&reply);
        let uris: Vec<&str> = recent.iter().map(Hit::uri).collect();
        assert_eq!(
            uris,
            [
                "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
                "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
                "spotify:album:6dVIqQ8qmQ5GBnJ9shOYGE",
                "spotify:playlist:37i9dQZF1E4Redacted002",
            ]
        );
        assert!(recent[0].track.is_some());
        assert!(parse_recent(&json!({"recentSearches": null})).is_empty());
    }

    #[test]
    fn writes_recent_searches_with_every_uri() {
        let uris = vec![
            "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb".to_owned(),
            "spotify:album:6dVIqQ8qmQ5GBnJ9shOYGE".to_owned(),
        ];
        let body = pathfinder::body(REMOVE_RECENT_SEARCHES, uris_variables(&uris));
        assert_eq!(
            body,
            json!({"variables": {"uris": uris}, "operationName": "removeRecentSearches",
                "extensions": {"persistedQuery": {"version": 1, "sha256Hash": "ece96d7b82c820fb9938dd1df50dee3e53737d808c3883e3d01c467c423601b0"}}})
        );
        let body = pathfinder::body(SAVE_RECENT_SEARCHES, uris_variables(&uris[..1]));
        assert_eq!(body["operationName"], "saveRecentSearches");
        assert_eq!(
            body["variables"]["uris"],
            json!(["spotify:artist:4Z8W4fKeB5YxbusRsdQVPb"])
        );
        assert_eq!(SAVE_RECENT_SEARCHES.kind, pathfinder::Kind::Mutation);
        assert_eq!(REMOVE_RECENT_SEARCHES.kind, pathfinder::Kind::Mutation);
        assert_eq!(RECENT_SEARCHES.kind, pathfinder::Kind::Query);
    }

    #[test]
    fn reads_write_replies() {
        let ok = json!({"saveToRecentSearches": {"__typename": "SaveToRecentSearchesResponse"}});
        assert!(check_write(&ok, "saveToRecentSearches").is_ok());
        let failed =
            json!({"removeFromRecentSearches": {"__typename": "GenericError", "message": "nope"}});
        let error = check_write(&failed, "removeFromRecentSearches").unwrap_err();
        assert_eq!(error.to_string(), "nope");
        let bare = json!({"removeFromRecentSearches": {"__typename": "GenericError"}});
        assert!(check_write(&bare, "removeFromRecentSearches").is_err());
    }

    #[tokio::test]
    async fn skips_empty_writes() {
        let api = Api::new(
            reqwest::Client::new(),
            "https://spclient.example",
            String::new(),
        );
        assert!(remember(&api, &[]).await.is_ok());
        assert!(forget(&api, &[]).await.is_ok());
    }
}
