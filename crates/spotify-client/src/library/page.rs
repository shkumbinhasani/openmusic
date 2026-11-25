use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::rootlist::{RootNode, Rootlist};
use crate::{
    api::{Api, HttpStatus},
    entity::{self, CARD_IMAGE},
    pathfinder::{self, Operation, PathfinderError},
};

pub const LIBRARY_V3: Operation = Operation::query(
    "libraryV3",
    "390c78e5b951029bad359785e69b07b536a509c581cbcd0aded5e5067f187455",
);

pub const PAGE_SIZE: u32 = 50;
pub const FEATURES: [&str; 5] = [
    "LIKED_SONGS",
    "YOUR_EPISODES_V2",
    "PRERELEASES",
    "PRERELEASES_V2",
    "EVENTS",
];
pub const LIKED_SONGS: &str = "spotify:collection:tracks";
pub const YOUR_EPISODES: &str = "spotify:playlist:37i9dQZF1FgnTBfUlzkeKt";
pub const PLAYLISTS: &str = "Playlists";
pub const RECENTS: &str = "Recents";
pub const RECENTLY_ADDED: &str = "Recently Added";
pub const ALPHABETICAL: &str = "Alphabetical";
pub const CREATOR: &str = "Creator";
pub const CUSTOM_ORDER: &str = "Custom Order";
const HIDDEN_FILTERS: [&str; 1] = ["Writable"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemKind {
    Playlist,
    Album,
    Artist,
    Show,
    Audiobook,
    Folder,
    LikedSongs,
    YourEpisodes,
    PreRelease,
    Event,
}

impl ItemKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Playlist | Self::LikedSongs | Self::YourEpisodes => "Playlist",
            Self::Album => "Album",
            Self::Artist => "Artist",
            Self::Show => "Podcast",
            Self::Audiobook => "Audiobook",
            Self::Folder => "Folder",
            Self::PreRelease => "Upcoming album",
            Self::Event => "Event",
        }
    }

    pub fn round(self) -> bool {
        self == Self::Artist
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub uri: String,
    pub kind: ItemKind,
    pub name: String,
    pub detail: String,
    pub image: Option<String>,
    pub available: bool,
    pub pinned: bool,
    pub pinnable: bool,
    pub depth: u32,
    pub owner: Option<String>,
    pub editable: bool,
}

impl LibraryItem {
    pub fn new(uri: impl Into<String>, kind: ItemKind, name: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            kind,
            name: name.into(),
            detail: String::new(),
            image: None,
            available: true,
            pinned: false,
            pinnable: true,
            depth: 0,
            owner: None,
            editable: false,
        }
    }

    pub fn title(&self) -> String {
        if !self.available {
            return format!("This {} is not available", self.kind.label().to_lowercase());
        }
        if self.name.trim().is_empty() {
            return "Untitled".into();
        }
        self.name.clone()
    }

    pub fn subtitle(&self, with_kind: bool) -> String {
        match self.kind {
            _ if !self.available => String::new(),
            ItemKind::YourEpisodes | ItemKind::Folder => self.detail.clone(),
            _ if self.detail.is_empty() => self.kind.label().to_owned(),
            _ if with_kind => format!("{} • {}", self.kind.label(), self.detail),
            _ => self.detail.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chip {
    pub id: String,
    pub name: String,
}

impl Chip {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryQuery {
    pub filters: Vec<String>,
    pub sort: Option<String>,
    pub text: Option<String>,
    pub folder: Option<String>,
    pub expanded: Vec<String>,
}

impl LibraryQuery {
    pub fn flattened(&self) -> bool {
        self.text.is_some()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LibraryPage {
    pub items: Vec<LibraryItem>,
    pub total: u32,
    pub offset: u32,
    pub rows: u32,
    pub filters: Vec<Chip>,
    pub selected: Vec<Chip>,
    pub sorts: Vec<Chip>,
    pub sort: Option<Chip>,
    pub breadcrumbs: Vec<Chip>,
    pub fallback: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryError {
    InvalidQuery(String),
    FolderMissing,
}

impl std::fmt::Display for LibraryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidQuery(message) => write!(formatter, "{message}"),
            Self::FolderMissing => write!(formatter, "This folder no longer exists."),
        }
    }
}

impl std::error::Error for LibraryError {}

pub fn variables(query: &LibraryQuery, offset: u32, limit: u32) -> Value {
    let flatten = query.flattened();
    let mut variables = json!({
        "filters": query.filters,
        "order": query.sort,
        "textFilter": query.text,
        "features": FEATURES,
        "limit": limit,
        "offset": offset,
        "flatten": flatten,
        "folderUri": query.folder,
        "includeFoldersWhenFlattening": true,
    });
    if !flatten {
        variables["expandedFolders"] = json!(query.expanded);
    }
    variables
}

pub async fn page(api: &Api, query: &LibraryQuery, offset: u32) -> Result<LibraryPage> {
    let data = pathfinder::query(api, LIBRARY_V3, variables(query, offset, PAGE_SIZE)).await?;
    parse_page(&data, now_ms())
}

pub fn wants_fallback(error: &anyhow::Error) -> bool {
    error.downcast_ref::<PathfinderError>().is_some()
        || error
            .downcast_ref::<HttpStatus>()
            .is_some_and(|status| status.0.as_u16() == 400)
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

fn chips(value: &Value) -> Vec<Chip> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(chip)
        .filter(|chip| !HIDDEN_FILTERS.contains(&chip.id.as_str()))
        .collect()
}

fn chip(value: &Value) -> Option<Chip> {
    let id = text(&value["id"])
        .or_else(|| text(&value["uri"]))
        .or_else(|| text(&value["_uri"]))
        .or_else(|| text(&value["data"]["uri"]))?;
    let name = text(&value["name"])
        .or_else(|| text(&value["data"]["name"]))
        .unwrap_or(id);
    Some(Chip::new(id, name))
}

pub fn parse_page(data: &Value, now: i64) -> Result<LibraryPage> {
    let page = &data["me"]["libraryV3"];
    match page["__typename"].as_str() {
        Some("LibraryInvalidFilterIdError" | "LibraryInvalidSortOrderIdError") => {
            let message = text(&page["message"]).unwrap_or("Spotify did not accept this view.");
            return Err(LibraryError::InvalidQuery(message.to_owned()).into());
        }
        Some("NotFound") => return Err(LibraryError::FolderMissing.into()),
        Some("LibraryPage") | None if page.is_object() => {}
        _ => bail!("Spotify sent an unexpected library page."),
    }
    let entries = page["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let items: Vec<LibraryItem> = entries
        .iter()
        .filter_map(|entry| item(entry, now))
        .collect();
    let rows = entries.len() as u32;
    Ok(LibraryPage {
        total: page["totalCount"]
            .as_u64()
            .map_or(rows, |total| total as u32),
        offset: page["pagingInfo"]["offset"].as_u64().unwrap_or_default() as u32,
        rows,
        filters: chips(&page["availableFilters"]),
        selected: chips(&page["selectedFilters"]),
        sorts: chips(&page["availableSortOrders"]),
        sort: chip(&page["selectedSortOrder"]),
        breadcrumbs: chips(&page["breadcrumbs"]),
        fallback: false,
        items,
    })
}

fn image(sources: &Value) -> Option<String> {
    entity::pick_image(sources, CARD_IMAGE)
}

fn names(items: &Value, path: &[&str]) -> String {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| text(path.iter().fold(item, |value, key| &value[*key])))
        .collect::<Vec<_>>()
        .join(", ")
}

fn artists(data: &Value) -> String {
    names(&data["artists"]["items"], &["profile", "name"])
}

fn authors(data: &Value) -> String {
    let authors = &data["authorsV2"];
    if authors.is_array() {
        names(authors, &["name"])
    } else {
        names(&authors["items"], &["name"])
    }
}

pub fn count_label(count: u64, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

pub fn folder_detail(playlists: u64, folders: u64) -> String {
    let mut parts = Vec::new();
    if playlists > 0 || folders == 0 {
        parts.push(count_label(playlists, "playlist", "playlists"));
    }
    if folders > 0 {
        parts.push(count_label(folders, "folder", "folders"));
    }
    parts.join(", ")
}

fn attribute<'a>(data: &'a Value, key: &str) -> Option<&'a str> {
    data["attributes"]
        .as_array()?
        .iter()
        .find(|attribute| attribute["key"] == key)
        .and_then(|attribute| text(&attribute["value"]))
}

pub fn item(entry: &Value, now: i64) -> Option<LibraryItem> {
    let wrapper = &entry["item"];
    let typename = wrapper["__typename"].as_str()?;
    let data = &wrapper["data"];
    let kind = data["__typename"].as_str().unwrap_or_default();
    let uri = text(&wrapper["_uri"]).or_else(|| text(&data["uri"]))?;
    if kind == "NotFound" && typename != "PlaylistResponseWrapper" {
        return None;
    }
    let (expected, mut item) = match typename {
        "PlaylistResponseWrapper" if uri == YOUR_EPISODES => {
            let mut item = LibraryItem::new(
                uri,
                ItemKind::YourEpisodes,
                text(&data["name"]).unwrap_or("Your Episodes"),
            );
            item.detail = "Saved & downloaded episodes".into();
            item.image = image(&data["images"]["items"][0]["sources"]);
            ("Playlist", item)
        }
        "PlaylistResponseWrapper" => {
            let name = data["name"].as_str().unwrap_or_default();
            let name = match attribute(data, "list_pretitle") {
                Some(prefix) if !name.contains(prefix) => format!("{prefix} {name}"),
                _ => name.to_owned(),
            };
            let mut item = LibraryItem::new(uri, ItemKind::Playlist, name);
            let owner = &data["ownerV2"]["data"];
            item.detail = text(&owner["name"])
                .or_else(|| text(&owner["username"]))
                .unwrap_or_default()
                .to_owned();
            item.owner = text(&owner["username"])
                .or_else(|| text(&owner["id"]))
                .map(str::to_owned);
            item.editable = data["currentUserCapabilities"]["canEditItems"]
                .as_bool()
                .unwrap_or(false);
            item.image = image(&data["images"]["items"][0]["sources"]);
            if data["revisionId"].is_null() {
                item.available = false;
            }
            ("Playlist", item)
        }
        "AlbumResponseWrapper" => {
            let upcoming = text(&data["preReleaseEndDateTime"]["isoString"])
                .and_then(iso_ms)
                .is_some_and(|end| end > now);
            let kind = if upcoming {
                ItemKind::PreRelease
            } else {
                ItemKind::Album
            };
            let mut item = LibraryItem::new(uri, kind, text(&data["name"]).unwrap_or_default());
            item.detail = artists(data);
            item.image = image(&data["coverArt"]["sources"]);
            ("Album", item)
        }
        "PreReleaseResponseWrapper" => {
            let content = &data["preReleaseContent"];
            let mut item = LibraryItem::new(
                uri,
                ItemKind::PreRelease,
                text(&content["name"]).unwrap_or_default(),
            );
            item.detail = artists(content);
            item.image = image(&content["coverArt"]["sources"]);
            ("PreRelease", item)
        }
        "ArtistResponseWrapper" => {
            let mut item = LibraryItem::new(
                uri,
                ItemKind::Artist,
                text(&data["profile"]["name"]).unwrap_or_default(),
            );
            item.image = image(&data["visuals"]["avatarImage"]["sources"]);
            ("Artist", item)
        }
        "PodcastResponseWrapper" | "PodcastOrAudiobookResponseWrapper" if kind == "Audiobook" => {
            audiobook(uri, data)
        }
        "PodcastResponseWrapper" | "PodcastOrAudiobookResponseWrapper" => {
            let mut item =
                LibraryItem::new(uri, ItemKind::Show, text(&data["name"]).unwrap_or_default());
            item.detail = text(&data["publisher"]["name"])
                .unwrap_or_default()
                .to_owned();
            item.image = image(&data["coverArt"]["sources"]);
            ("Podcast", item)
        }
        "AudiobookResponseWrapper" if kind == "RestrictedContent" => return None,
        "AudiobookResponseWrapper" => audiobook(uri, data),
        "LibraryPseudoPlaylistResponseWrapper" => {
            let mut item = LibraryItem::new(
                uri,
                ItemKind::LikedSongs,
                text(&data["name"]).unwrap_or("Liked Songs"),
            );
            if let Some(count) = data["count"].as_u64() {
                item.detail = count_label(count, "song", "songs");
            }
            item.image = image(&data["image"]["sources"]);
            ("PseudoPlaylist", item)
        }
        "LibraryFolderResponseWrapper" => {
            let mut item = LibraryItem::new(
                text(&data["uri"]).unwrap_or(uri),
                ItemKind::Folder,
                text(&data["name"]).unwrap_or_default(),
            );
            item.detail = folder_detail(
                data["playlistCount"].as_u64().unwrap_or_default(),
                data["folderCount"].as_u64().unwrap_or_default(),
            );
            ("Folder", item)
        }
        "ConcertV2ResponseWrapper" | "ConcertCampaignResponseWrapper" => {
            let name = text(&data["title"]).or_else(|| text(&data["name"]))?;
            let mut item = LibraryItem::new(uri, ItemKind::Event, name);
            item.image =
                image(&data["image"]["sources"]).or_else(|| image(&data["coverArt"]["sources"]));
            (kind, item)
        }
        _ => return None,
    };
    if kind != expected {
        item.available = false;
    }
    item.pinned = entry["pinned"].as_bool().unwrap_or(false);
    item.pinnable = item.available && entry["pinnable"].as_bool().unwrap_or(false);
    item.depth = entry["depth"].as_u64().unwrap_or_default() as u32;
    Some(item)
}

fn audiobook(uri: &str, data: &Value) -> (&'static str, LibraryItem) {
    let mut item = LibraryItem::new(
        uri,
        ItemKind::Audiobook,
        text(&data["name"]).unwrap_or_default(),
    );
    item.detail = authors(data);
    item.image = image(&data["coverArt"]["sources"]);
    ("Audiobook", item)
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

pub(super) fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub fn iso_ms(iso: &str) -> Option<i64> {
    let number = |range: std::ops::Range<usize>| iso.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (hour, minute, second) = if iso.len() >= 19 {
        (number(11..13)?, number(14..16)?, number(17..19)?)
    } else {
        (0, 0, 0)
    };
    let millis = iso
        .get(19..)
        .and_then(|rest| rest.strip_prefix('.'))
        .map(|rest| {
            let digits: String = rest
                .chars()
                .take_while(char::is_ascii_digit)
                .take(3)
                .collect();
            format!("{digits:0<3}").parse::<i64>().unwrap_or(0)
        })
        .unwrap_or(0);
    let days = days_from_civil(year, month, day);
    Some(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis)
}

fn fallback_sorts() -> Vec<Chip> {
    [CUSTOM_ORDER, RECENTLY_ADDED, ALPHABETICAL, CREATOR]
        .into_iter()
        .map(|id| Chip::new(id, id))
        .collect()
}

struct Entry {
    item: LibraryItem,
    added_at: Option<i64>,
    children: Vec<RootNode>,
}

fn entry(node: &RootNode, user: &str) -> Entry {
    match node {
        RootNode::Playlist {
            playlist, added_at, ..
        } => {
            let mut item = LibraryItem::new(&playlist.uri, ItemKind::Playlist, &playlist.name);
            item.image = playlist.cover.clone();
            item.owner = playlist.owner.clone();
            item.editable = playlist.owner.as_deref() == Some(user);
            item.detail = match &playlist.owner {
                Some(owner) if owner != user => owner.clone(),
                _ => count_label(u64::from(playlist.length), "song", "songs"),
            };
            Entry {
                item,
                added_at: *added_at,
                children: Vec::new(),
            }
        }
        RootNode::Unavailable { uri, .. } => {
            let mut item = LibraryItem::new(uri, ItemKind::Playlist, "");
            item.available = false;
            item.pinnable = false;
            Entry {
                item,
                added_at: None,
                children: Vec::new(),
            }
        }
        RootNode::Folder(folder) => {
            let mut item = LibraryItem::new(&folder.uri, ItemKind::Folder, &folder.name);
            item.detail =
                folder_detail(folder.playlist_count() as u64, folder.folder_count() as u64);
            Entry {
                item,
                added_at: folder.added_at,
                children: folder.children.clone(),
            }
        }
    }
}

fn sorted(mut entries: Vec<Entry>, sort: &str) -> Vec<Entry> {
    let name = |entry: &Entry| entry.item.name.to_lowercase();
    match sort {
        ALPHABETICAL => entries.sort_by_key(name),
        RECENTLY_ADDED => entries.sort_by_key(|entry| std::cmp::Reverse(entry.added_at)),
        CREATOR => entries.sort_by_key(|entry| {
            (
                entry.item.kind == ItemKind::Folder,
                entry.item.owner.clone().unwrap_or_default().to_lowercase(),
                name(entry),
            )
        }),
        _ => {}
    }
    entries
}

fn walk(
    nodes: &[RootNode],
    depth: u32,
    query: &LibraryQuery,
    sort: &str,
    user: &str,
    items: &mut Vec<LibraryItem>,
) {
    let entries = nodes.iter().map(|node| entry(node, user)).collect();
    for Entry {
        mut item, children, ..
    } in sorted(entries, sort)
    {
        item.depth = depth;
        let open = item.kind == ItemKind::Folder && query.expanded.contains(&item.uri);
        items.push(item);
        if open {
            walk(&children, depth + 1, query, sort, user, items);
        }
    }
}

fn flatten(nodes: &[RootNode], user: &str, found: &mut Vec<Entry>) {
    for node in nodes {
        let entry = entry(node, user);
        let children = entry.children.clone();
        found.push(entry);
        flatten(&children, user, found);
    }
}

pub fn fallback_page(rootlist: &Rootlist, query: &LibraryQuery) -> LibraryPage {
    let sorts = fallback_sorts();
    let sort = query
        .sort
        .as_deref()
        .filter(|sort| sorts.iter().any(|chip| chip.id == *sort))
        .unwrap_or(CUSTOM_ORDER);
    let playlists_only = query.filters.first().map(String::as_str) == Some(PLAYLISTS);
    let other_filter = !query.filters.is_empty() && !playlists_only;
    let tree = rootlist.tree();
    let (nodes, breadcrumbs) = match &query.folder {
        Some(uri) => match rootlist.folder(uri) {
            Some(folder) => (
                folder.children.clone(),
                vec![Chip::new(&folder.uri, &folder.name)],
            ),
            None => (Vec::new(), Vec::new()),
        },
        None => (tree, Vec::new()),
    };
    let needle = query.text.as_deref().map(str::to_lowercase);
    let matches = |name: &str| {
        needle
            .as_deref()
            .is_none_or(|needle| name.to_lowercase().contains(needle))
    };
    let mut items = Vec::new();
    if !other_filter {
        if query.folder.is_none() && !playlists_only && matches("Liked Songs") {
            let mut liked = LibraryItem::new(LIKED_SONGS, ItemKind::LikedSongs, "Liked Songs");
            liked.pinned = true;
            items.push(liked);
        }
        if needle.is_some() {
            let mut found = Vec::new();
            flatten(&nodes, &rootlist.user, &mut found);
            found.retain(|entry| entry.item.available && matches(&entry.item.name));
            items.extend(sorted(found, sort).into_iter().map(|entry| entry.item));
        } else {
            walk(&nodes, 0, query, sort, &rootlist.user, &mut items);
        }
    }
    let selected = if playlists_only {
        vec![Chip::new(PLAYLISTS, PLAYLISTS)]
    } else {
        query
            .filters
            .iter()
            .map(|filter| Chip::new(filter.as_str(), filter.as_str()))
            .collect()
    };
    let filters = if query.filters.is_empty() {
        vec![Chip::new(PLAYLISTS, PLAYLISTS)]
    } else {
        Vec::new()
    };
    LibraryPage {
        total: items.len() as u32,
        offset: 0,
        rows: items.len() as u32,
        filters,
        selected,
        sorts,
        sort: Some(Chip::new(sort, sort)),
        breadcrumbs,
        fallback: true,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::rootlist;

    const NOW: i64 = 1_795_000_000_000;

    fn sources(url: &str) -> Value {
        json!({"sources": [{"url": url, "width": 300, "height": 300}]})
    }

    fn sample() -> Value {
        json!({"me": {"libraryV3": {
            "__typename": "LibraryPage",
            "availableFilters": [{"id": "Playlists", "name": "Playlists"}, {"id": "Writable", "name": "Writable"}, {"id": "Albums", "name": "Albums"}],
            "availableSortOrders": [{"id": "Recents", "name": "Recents"}, {"id": "Alphabetical", "name": "Alphabetical"}],
            "selectedSortOrder": {"id": "Recents", "name": "Recents"},
            "selectedFilters": [],
            "breadcrumbs": [],
            "pagingInfo": {"limit": 50, "offset": 50},
            "totalCount": 97,
            "items": [
                {"depth": 0, "pinnable": true, "pinned": true, "item": {
                    "__typename": "LibraryPseudoPlaylistResponseWrapper", "_uri": "spotify:collection:tracks",
                    "data": {"__typename": "PseudoPlaylist", "count": 64, "name": "Liked Songs", "uri": "spotify:collection:tracks",
                        "image": sources("https://misc.scdn.co/liked-songs/liked-songs-300.png")}}},
                {"depth": 0, "pinnable": true, "pinned": false, "item": {
                    "__typename": "PlaylistResponseWrapper", "_uri": "spotify:playlist:0000000000000000000001",
                    "data": {"__typename": "Playlist", "attributes": [{"key": "list_pretitle", "value": "Daily"}],
                        "currentUserCapabilities": {"canEditItems": true, "canView": true},
                        "format": "", "images": {"items": [sources("https://mosaic.scdn.co/300/abc")]},
                        "name": "Mix 1", "ownerV2": {"data": {"__typename": "User", "name": "Someone", "username": "someone"}},
                        "revisionId": "AAAA", "uri": "spotify:playlist:0000000000000000000001"}}},
                {"depth": 1, "pinnable": false, "pinned": false, "item": {
                    "__typename": "PlaylistResponseWrapper", "_uri": "spotify:playlist:0000000000000000000002",
                    "data": {"__typename": "Playlist", "attributes": [], "currentUserCapabilities": {}, "images": {"items": []},
                        "name": "Gone", "ownerV2": {"data": {}}, "revisionId": null}}},
                {"depth": 0, "pinnable": true, "pinned": false, "item": {
                    "__typename": "AlbumResponseWrapper", "_uri": "spotify:album:6k5AgJP2qUmq2rfp65l5cV",
                    "data": {"__typename": "Album", "name": "Album", "artists": {"items": [{"profile": {"name": "A"}}, {"profile": {"name": "B"}}]},
                        "coverArt": sources("https://i.scdn.co/image/album"), "preReleaseEndDateTime": null}}},
                {"depth": 0, "pinnable": true, "pinned": false, "item": {
                    "__typename": "AlbumResponseWrapper", "_uri": "spotify:album:0000000000000000000003",
                    "data": {"__typename": "Album", "name": "Soon", "artists": {"items": []},
                        "coverArt": sources("https://i.scdn.co/image/soon"), "preReleaseEndDateTime": {"isoString": "2027-01-01T00:00:00Z"}}}},
                {"depth": 0, "pinnable": true, "pinned": false, "item": {
                    "__typename": "AlbumResponseWrapper", "_uri": "spotify:album:0000000000000000000004",
                    "data": {"__typename": "NotFound"}}},
                {"depth": 0, "pinnable": true, "pinned": false, "item": {
                    "__typename": "ArtistResponseWrapper", "_uri": "spotify:artist:0000000000000000000005",
                    "data": {"__typename": "Artist", "profile": {"name": "Band"}, "visuals": {"avatarImage": sources("https://i.scdn.co/image/band")}}}},
                {"depth": 0, "pinnable": true, "pinned": false, "item": {
                    "__typename": "PodcastOrAudiobookResponseWrapper", "_uri": "spotify:show:0000000000000000000006",
                    "data": {"__typename": "Podcast", "name": "Show", "publisher": {"name": "Publisher"}, "coverArt": sources("https://i.scdn.co/image/show")}}},
                {"depth": 0, "pinnable": true, "pinned": false, "item": {
                    "__typename": "PodcastOrAudiobookResponseWrapper", "_uri": "spotify:show:0000000000000000000007",
                    "data": {"__typename": "Audiobook", "name": "Book", "authorsV2": {"items": [{"name": "Writer"}]}, "coverArt": sources("https://i.scdn.co/image/book")}}},
                {"depth": 0, "pinnable": true, "pinned": true, "item": {
                    "__typename": "PlaylistResponseWrapper", "_uri": "spotify:playlist:37i9dQZF1FgnTBfUlzkeKt",
                    "data": {"__typename": "Playlist", "name": "Your Episodes", "images": {"items": []}, "revisionId": "AAAA"}}},
                {"depth": 0, "pinnable": true, "pinned": false, "item": {
                    "__typename": "LibraryFolderResponseWrapper", "_uri": "spotify:user:someone:folder:1a2b3c4d5e6f7081",
                    "data": {"__typename": "Folder", "uri": "spotify:user:someone:folder:1a2b3c4d5e6f7081", "name": "Road trip", "playlistCount": 3, "folderCount": 1}}},
                {"depth": 0, "pinnable": true, "pinned": false, "item": {
                    "__typename": "ConcertV2ResponseWrapper", "_uri": "spotify:concert:x", "data": {"__typename": "ConcertV2"}}},
                {"depth": 0, "item": {"__typename": "SomethingNewResponseWrapper", "_uri": "spotify:new:x", "data": {}}}
            ]
        }}})
    }

    #[test]
    fn maps_every_kind_of_library_row() {
        let page = parse_page(&sample(), NOW).unwrap();
        assert_eq!((page.total, page.offset), (97, 50));
        assert_eq!((page.rows, page.items.len()), (13, 10));
        assert_eq!(
            page.filters,
            vec![
                Chip::new("Playlists", "Playlists"),
                Chip::new("Albums", "Albums")
            ]
        );
        assert_eq!(page.sorts.len(), 2);
        assert_eq!(page.sort, Some(Chip::new("Recents", "Recents")));
        let rows: Vec<(ItemKind, String, String, bool)> = page
            .items
            .iter()
            .map(|item| (item.kind, item.title(), item.subtitle(true), item.pinned))
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    ItemKind::LikedSongs,
                    "Liked Songs".into(),
                    "Playlist • 64 songs".into(),
                    true
                ),
                (
                    ItemKind::Playlist,
                    "Daily Mix 1".into(),
                    "Playlist • Someone".into(),
                    false
                ),
                (
                    ItemKind::Playlist,
                    "This playlist is not available".into(),
                    "".into(),
                    false
                ),
                (
                    ItemKind::Album,
                    "Album".into(),
                    "Album • A, B".into(),
                    false
                ),
                (
                    ItemKind::PreRelease,
                    "Soon".into(),
                    "Upcoming album".into(),
                    false
                ),
                (ItemKind::Artist, "Band".into(), "Artist".into(), false),
                (
                    ItemKind::Show,
                    "Show".into(),
                    "Podcast • Publisher".into(),
                    false
                ),
                (
                    ItemKind::Audiobook,
                    "Book".into(),
                    "Audiobook • Writer".into(),
                    false
                ),
                (
                    ItemKind::YourEpisodes,
                    "Your Episodes".into(),
                    "Saved & downloaded episodes".into(),
                    true
                ),
                (
                    ItemKind::Folder,
                    "Road trip".into(),
                    "3 playlists, 1 folder".into(),
                    false
                ),
            ]
        );
        let playlist = &page.items[1];
        assert_eq!(playlist.owner.as_deref(), Some("someone"));
        assert!(playlist.editable);
        assert_eq!(
            playlist.image.as_deref(),
            Some("https://mosaic.scdn.co/300/abc")
        );
        assert_eq!(playlist.subtitle(false), "Someone");
        let gone = &page.items[2];
        assert!(!gone.available && !gone.pinnable);
        assert_eq!(gone.depth, 1);
        assert_eq!(
            page.items[9].uri,
            "spotify:user:someone:folder:1a2b3c4d5e6f7081"
        );
    }

    #[test]
    fn reports_invalid_views_and_missing_folders() {
        let invalid = json!({"me": {"libraryV3": {"__typename": "LibraryInvalidFilterIdError", "message": "Nope is not a valid filter id", "invalidFilterId": "Nope"}}});
        let error = parse_page(&invalid, NOW).unwrap_err();
        assert_eq!(
            error.downcast_ref::<LibraryError>(),
            Some(&LibraryError::InvalidQuery(
                "Nope is not a valid filter id".into()
            ))
        );
        let missing =
            json!({"me": {"libraryV3": {"__typename": "NotFound", "message": "Folder not found"}}});
        assert_eq!(
            parse_page(&missing, NOW)
                .unwrap_err()
                .downcast_ref::<LibraryError>(),
            Some(&LibraryError::FolderMissing)
        );
        assert!(parse_page(&json!({"me": null}), NOW).is_err());
        let empty = json!({"me": {"libraryV3": {"__typename": "LibraryPage", "items": [], "totalCount": 0, "selectedFilters": [{"id": "Podcasts & Shows", "name": "Podcasts"}]}}});
        let page = parse_page(&empty, NOW).unwrap();
        assert_eq!(page.total, 0);
        assert_eq!(
            page.selected,
            vec![Chip::new("Podcasts & Shows", "Podcasts")]
        );
    }

    #[test]
    fn builds_variables_like_the_web_player() {
        let query = LibraryQuery::default();
        assert_eq!(
            variables(&query, 0, 50),
            json!({"filters": [], "order": null, "textFilter": null,
                "features": ["LIKED_SONGS", "YOUR_EPISODES_V2", "PRERELEASES", "PRERELEASES_V2", "EVENTS"],
                "limit": 50, "offset": 0, "flatten": false, "expandedFolders": [], "folderUri": null,
                "includeFoldersWhenFlattening": true})
        );
        let query = LibraryQuery {
            filters: vec!["Playlists".into(), "By you".into()],
            sort: Some("Custom Order".into()),
            text: Some("the".into()),
            expanded: vec!["spotify:user:someone:folder:1".into()],
            ..LibraryQuery::default()
        };
        let built = variables(&query, 100, 50);
        assert_eq!(built["filters"], json!(["Playlists", "By you"]));
        assert_eq!(built["order"], "Custom Order");
        assert_eq!(built["textFilter"], "the");
        assert_eq!(built["flatten"], true);
        assert_eq!(built["offset"], 100);
        assert!(built.get("expandedFolders").is_none());
        let folder = LibraryQuery {
            folder: Some("spotify:user:someone:folder:1".into()),
            expanded: vec!["spotify:user:someone:folder:2".into()],
            ..LibraryQuery::default()
        };
        let built = variables(&folder, 0, 50);
        assert_eq!(built["folderUri"], "spotify:user:someone:folder:1");
        assert_eq!(
            built["expandedFolders"],
            json!(["spotify:user:someone:folder:2"])
        );
    }

    #[test]
    fn reads_iso_times() {
        assert_eq!(iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_ms("2026-11-21T15:30:43Z"), Some(1_795_275_043_000));
        assert_eq!(iso_ms("2026-11-17T15:36:03.466Z"), Some(1_794_929_763_466));
        assert_eq!(iso_ms("2018-08-31"), Some(1_535_673_600_000));
        assert_eq!(iso_ms("nonsense"), None);
    }

    fn sample_rootlist() -> Rootlist {
        let entry = |uri: &str, name: &str, owner: &str, added: i64| {
            (
                json!({"uri": uri, "attributes": {"timestamp": added.to_string()}}),
                json!({"attributes": {"name": name}, "length": 2, "ownerUsername": owner}),
            )
        };
        let marker = |uri: &str| (json!({"uri": uri}), json!({}));
        let (items, meta): (Vec<Value>, Vec<Value>) = vec![
            entry(
                "spotify:playlist:0000000000000000000001",
                "Zebra",
                "someone",
                3,
            ),
            marker("spotify:start-group:aaaaaaaaaaaaaaaa:Folder"),
            entry(
                "spotify:playlist:0000000000000000000002",
                "Apple",
                "editor",
                5,
            ),
            entry(
                "spotify:playlist:0000000000000000000003",
                "The Mix",
                "someone",
                1,
            ),
            marker("spotify:end-group:aaaaaaaaaaaaaaaa"),
            entry(
                "spotify:playlist:0000000000000000000004",
                "Mango",
                "editor",
                4,
            ),
        ]
        .into_iter()
        .unzip();
        let page = rootlist::parse_page(
            &json!({"revision": "r", "contents": {"items": items, "metaItems": meta}}),
        );
        Rootlist {
            user: "someone".into(),
            revision: page.revision,
            items: page.items,
        }
    }

    fn titles(page: &LibraryPage) -> Vec<String> {
        page.items
            .iter()
            .map(|item| format!("{}{}", "  ".repeat(item.depth as usize), item.title()))
            .collect()
    }

    #[test]
    fn falls_back_to_the_rootlist() {
        let list = sample_rootlist();
        let page = fallback_page(&list, &LibraryQuery::default());
        assert!(page.fallback);
        assert_eq!(page.sort, Some(Chip::new(CUSTOM_ORDER, CUSTOM_ORDER)));
        assert_eq!(
            titles(&page),
            vec!["Liked Songs", "Zebra", "Folder", "Mango"]
        );
        assert_eq!(page.items[2].detail, "2 playlists");
        assert_eq!(page.items[1].subtitle(true), "Playlist • 2 songs");
        assert_eq!(page.items[3].subtitle(true), "Playlist • editor");
        assert_eq!(page.total, 4);

        let folder = "spotify:user:someone:folder:aaaaaaaaaaaaaaaa".to_owned();
        let expanded = LibraryQuery {
            expanded: vec![folder.clone()],
            sort: Some(ALPHABETICAL.into()),
            filters: vec![PLAYLISTS.into()],
            ..LibraryQuery::default()
        };
        let page = fallback_page(&list, &expanded);
        assert_eq!(
            titles(&page),
            vec!["Folder", "  Apple", "  The Mix", "Mango", "Zebra"]
        );
        assert_eq!(page.selected, vec![Chip::new(PLAYLISTS, PLAYLISTS)]);

        let searched = LibraryQuery {
            text: Some("the".into()),
            sort: Some(RECENTLY_ADDED.into()),
            ..LibraryQuery::default()
        };
        assert_eq!(titles(&fallback_page(&list, &searched)), vec!["The Mix"]);

        let inside = LibraryQuery {
            folder: Some(folder.clone()),
            sort: Some(RECENTLY_ADDED.into()),
            ..LibraryQuery::default()
        };
        let page = fallback_page(&list, &inside);
        assert_eq!(titles(&page), vec!["Apple", "The Mix"]);
        assert_eq!(page.breadcrumbs, vec![Chip::new(folder, "Folder")]);

        let albums = LibraryQuery {
            filters: vec!["Albums".into()],
            ..LibraryQuery::default()
        };
        let page = fallback_page(&list, &albums);
        assert!(page.items.is_empty());
        assert_eq!(page.selected, vec![Chip::new("Albums", "Albums")]);
        assert!(page.filters.is_empty());
        assert_eq!(
            fallback_page(&list, &LibraryQuery::default()).selected,
            Vec::new()
        );
    }

    #[test]
    fn falls_back_only_when_the_graphql_side_fails() {
        assert!(wants_fallback(&anyhow::Error::new(
            PathfinderError::StaleHash("libraryV3")
        )));
        assert!(wants_fallback(&anyhow::Error::new(HttpStatus(
            reqwest::StatusCode::BAD_REQUEST
        ))));
        assert!(!wants_fallback(&anyhow::Error::new(HttpStatus(
            reqwest::StatusCode::UNAUTHORIZED
        ))));
        assert!(!wants_fallback(&anyhow::anyhow!("offline")));
    }
}
