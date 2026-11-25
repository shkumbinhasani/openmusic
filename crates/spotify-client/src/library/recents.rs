use anyhow::{Result, bail};
use base64::Engine;
use prost::Message as _;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    BATCH, Recent,
    page::{count_label, days_from_civil},
};
use crate::{
    api::Api,
    catalog::Link,
    entity::{self, CARD_IMAGE},
    pathfinder::{self, Operation},
};

pub const RECENTS_LIST: Operation = Operation::query(
    "recents",
    "94be830be40f0f3759ef83a4ea6e4dd1bef1f6108f7502c11ee1fa7fb1bb1658",
);
pub const FETCH_ENTITIES_FOR_RECENTLY_PLAYED: Operation = Operation::query(
    "fetchEntitiesForRecentlyPlayed",
    "cf5d2e94ffd82788470788ae1f6090cc3e9e774fb8fd383580634c6e6f50f7be",
);
pub const RECENTS_PAGE: &str = "spotify:list:recents:page";
pub const RECENTS_PAGE_SIZE: u32 = 100;
pub const RECENTLY_PLAYED_LIMIT: u32 = 50;
pub const DAY_MS: i64 = 86_400_000;
const EXPLICIT: &str = "CONTENT_RATING_EXPLICIT";
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const STACK_NAMES: usize = 3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RecentKind {
    Track,
    Album,
    Artist,
    Playlist,
    Show,
    Episode,
    Audiobook,
    Chapter,
    User,
    LikedSongs,
    YourEpisodes,
    #[default]
    Other,
}

impl RecentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Track => "Song",
            Self::Album => "Album",
            Self::Artist => "Artist",
            Self::Playlist | Self::LikedSongs | Self::YourEpisodes => "Playlist",
            Self::Show => "Podcast",
            Self::Episode => "Episode",
            Self::Audiobook | Self::Chapter => "Audiobook",
            Self::User => "Profile",
            Self::Other => "",
        }
    }

    pub fn round(self) -> bool {
        matches!(self, Self::Artist | Self::User)
    }

    pub fn context(self) -> bool {
        matches!(
            self,
            Self::Album
                | Self::Artist
                | Self::Playlist
                | Self::Show
                | Self::Audiobook
                | Self::LikedSongs
                | Self::YourEpisodes
        )
    }

    fn from_entity(value: &str) -> Option<Self> {
        Some(match value {
            "ENTITY_TYPE_TRACK" => Self::Track,
            "ENTITY_TYPE_ALBUM" => Self::Album,
            "ENTITY_TYPE_ARTIST" => Self::Artist,
            "ENTITY_TYPE_PLAYLIST" => Self::Playlist,
            "ENTITY_TYPE_PODCAST_SHOW" | "ENTITY_TYPE_SHOW" => Self::Show,
            "ENTITY_TYPE_PODCAST_EPISODE" | "ENTITY_TYPE_EPISODE" => Self::Episode,
            "ENTITY_TYPE_PODCAST_CHAPTER" | "ENTITY_TYPE_CHAPTER" => Self::Chapter,
            "ENTITY_TYPE_AUDIOBOOK" => Self::Audiobook,
            "ENTITY_TYPE_USER" => Self::User,
            _ => return None,
        })
    }

    pub fn from_uri(uri: &str) -> Self {
        if entity::is_liked_songs(uri) {
            return Self::LikedSongs;
        }
        if is_your_episodes(uri) {
            return Self::YourEpisodes;
        }
        let parts: Vec<&str> = uri.split(':').collect();
        match parts.as_slice() {
            ["spotify", "track", _] => Self::Track,
            ["spotify", "album", _] => Self::Album,
            ["spotify", "artist", _] => Self::Artist,
            ["spotify", "playlist", _] | ["spotify", "user", _, "playlist", _] => Self::Playlist,
            ["spotify", "show", _] => Self::Show,
            ["spotify", "episode", _] => Self::Episode,
            ["spotify", "chapter", _] => Self::Chapter,
            ["spotify", "user", _] => Self::User,
            _ => Self::Other,
        }
    }
}

pub fn is_your_episodes(uri: &str) -> bool {
    uri == "spotify:collection:your-episodes"
        || (uri.starts_with("spotify:user:") && uri.ends_with(":collection:your-episodes"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Activity {
    #[default]
    Unknown,
    Played,
    Saved,
    Shared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Content {
    Music,
    Podcasts,
    Audiobooks,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentEntry {
    pub uri: String,
    pub kind: RecentKind,
    pub name: String,
    pub image: Option<String>,
    pub contributors: Vec<Link>,
    pub parent: Option<Link>,
    pub day: Option<i64>,
    pub played_at: Option<i64>,
    pub activity: Activity,
    pub contents: Vec<Content>,
    pub count: u32,
    pub group: Option<String>,
    pub children_group: Option<String>,
    pub explicit: bool,
}

fn joined(parts: impl IntoIterator<Item = Option<String>>) -> String {
    parts
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" • ")
}

impl RecentEntry {
    fn new(uri: impl Into<String>, kind: RecentKind, name: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            kind,
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn names(&self) -> String {
        self.contributors
            .iter()
            .map(|link| link.name.as_str())
            .filter(|name| !name.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn played(&self) -> bool {
        self.activity == Activity::Played
    }

    pub fn count_text(&self) -> Option<String> {
        let count = u64::from(self.count);
        if count == 0 {
            return None;
        }
        if self.activity == Activity::Shared {
            return Some(count_label(count, "share", "shares"));
        }
        let podcast = matches!(
            RecentKind::from_uri(&self.uri),
            RecentKind::Show | RecentKind::Episode
        );
        let contents: &[Content] = if podcast {
            &[Content::Podcasts]
        } else {
            &self.contents
        };
        let saved = self.activity == Activity::Saved;
        let (one, many) = match (contents, saved) {
            ([], _) => return None,
            ([Content::Music], false) => ("song played", "songs played"),
            ([Content::Music], true) => ("song added", "songs added"),
            ([Content::Podcasts], false) => ("episode played", "episodes played"),
            ([Content::Podcasts], true) => ("episode added", "episodes added"),
            ([Content::Audiobooks], false) => ("audiobook played", "audiobooks played"),
            ([Content::Audiobooks], true) => ("audiobook added", "audiobooks added"),
            (_, false) => ("item played", "items played"),
            (_, true) => ("item added", "items added"),
        };
        Some(count_label(count, one, many))
    }

    pub fn subtitle(&self) -> (bool, String) {
        let label = Some(self.kind.label().to_owned());
        let names = Some(self.names());
        let songs_played = (self.count > 0)
            .then(|| count_label(u64::from(self.count), "song played", "songs played"));
        match self.kind {
            RecentKind::Episode | RecentKind::Chapter => (
                false,
                joined([
                    label,
                    self.parent.as_ref().map(|parent| parent.name.clone()),
                ]),
            ),
            RecentKind::LikedSongs | RecentKind::YourEpisodes if self.played() => (
                false,
                self.count_text()
                    .unwrap_or_else(|| self.kind.label().to_owned()),
            ),
            RecentKind::Playlist if self.played() => {
                (false, joined([self.count_text(), label, names]))
            }
            RecentKind::Playlist | RecentKind::LikedSongs | RecentKind::YourEpisodes => (
                true,
                self.count_text()
                    .unwrap_or_else(|| "Playlist added".to_owned()),
            ),
            RecentKind::Album if self.played() => (false, joined([label, names, songs_played])),
            RecentKind::Album => (true, "Album added".to_owned()),
            RecentKind::Artist => (false, joined([label, songs_played])),
            RecentKind::Audiobook if self.played() => (false, joined([label, names])),
            RecentKind::Audiobook => (true, "Audiobook added".to_owned()),
            _ => {
                let text = joined([self.count_text(), names]);
                if text.is_empty() {
                    (false, self.kind.label().to_owned())
                } else {
                    (false, text)
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentGroup {
    pub item: RecentEntry,
    pub children: Vec<RecentEntry>,
    children_group: Option<String>,
}

impl RecentGroup {
    pub fn single(item: RecentEntry) -> Self {
        Self {
            item,
            children: Vec::new(),
            children_group: None,
        }
    }

    pub fn stacked(&self) -> bool {
        self.item.name.is_empty() && !self.children.is_empty()
    }

    pub fn title(&self) -> String {
        if self.stacked() {
            return self.item.count_text().unwrap_or_default();
        }
        if !self.children.is_empty()
            && let Some(parent) = self.item.parent.as_ref().filter(|p| !p.name.is_empty())
        {
            return parent.name.clone();
        }
        self.item.name.clone()
    }

    pub fn subtitle(&self) -> (bool, String) {
        if !self.stacked() {
            return self.item.subtitle();
        }
        let mut names: Vec<&str> = self
            .children
            .iter()
            .map(|child| child.name.as_str())
            .filter(|name| !name.is_empty())
            .take(STACK_NAMES)
            .collect();
        if self.children.len() > STACK_NAMES {
            names.push("…");
        }
        (false, names.join(", "))
    }

    pub fn key(&self, day: i64) -> String {
        let id = self
            .children_group
            .as_deref()
            .filter(|_| self.item.uri.is_empty())
            .unwrap_or(&self.item.uri);
        format!("{day}:{id}")
    }

    pub fn images(&self) -> Vec<&str> {
        let own = self.item.image.as_deref().filter(|_| !self.stacked());
        own.into_iter()
            .chain(
                self.children
                    .iter()
                    .filter_map(|child| child.image.as_deref()),
            )
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentDay {
    pub day: i64,
    pub groups: Vec<RecentGroup>,
}

pub fn recent_days(entries: &[RecentEntry]) -> Vec<RecentDay> {
    let mut days: Vec<(i64, Vec<&RecentEntry>)> = Vec::new();
    for entry in entries {
        let Some(day) = entry.day else { continue };
        match days.iter_mut().find(|(known, _)| *known == day) {
            Some((_, list)) => list.push(entry),
            None => days.push((day, vec![entry])),
        }
    }
    days.sort_by_key(|(day, _)| std::cmp::Reverse(*day));
    days.into_iter()
        .map(|(day, list)| {
            let mut groups: Vec<RecentGroup> = Vec::new();
            for entry in list {
                let parent = entry.group.as_deref().and_then(|id| {
                    groups
                        .iter_mut()
                        .find(|group| group.children_group.as_deref() == Some(id))
                });
                match parent {
                    Some(group) => group.children.push(entry.clone()),
                    None => groups.push(RecentGroup {
                        item: entry.clone(),
                        children: Vec::new(),
                        children_group: entry
                            .children_group
                            .clone()
                            .filter(|_| entry.group.is_none()),
                    }),
                }
            }
            groups.retain(|group| !group.children.is_empty() || !group.item.name.is_empty());
            RecentDay { day, groups }
        })
        .filter(|day| !day.groups.is_empty())
        .collect()
}

fn civil_from_days(days: i64) -> (i64, usize, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month as usize, day)
}

pub fn day_of(ms: i64) -> i64 {
    ms.div_euclid(DAY_MS) * DAY_MS
}

pub fn day_label(day: i64, now: i64) -> String {
    let today = day_of(now);
    if day == today {
        return "Today".into();
    }
    if day == today - DAY_MS {
        return "Yesterday".into();
    }
    let days = day.div_euclid(DAY_MS);
    let (year, month, date) = civil_from_days(days);
    let weekday = WEEKDAYS[(days + 4).rem_euclid(7) as usize];
    let mut label = format!("{weekday}, {} {date}", MONTHS[month - 1]);
    if year != civil_from_days(today.div_euclid(DAY_MS)).0 {
        label.push_str(&format!(", {year}"));
    }
    label
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecentsPage {
    pub entries: Vec<RecentEntry>,
    pub total: u32,
    pub offset: u32,
    pub next_offset: Option<u32>,
    pub fallback: bool,
}

pub fn recents_variables(offset: u32, limit: u32) -> Value {
    json!({ "uris": [RECENTS_PAGE], "offset": offset, "limit": limit })
}

pub async fn recents(api: &Api, offset: u32, limit: u32) -> Result<RecentsPage> {
    let data = pathfinder::query(api, RECENTS_LIST, recents_variables(offset, limit)).await?;
    parse_recents(&data, offset)
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

fn image(sources: &Value) -> Option<String> {
    entity::pick_image(sources, CARD_IMAGE)
}

#[derive(Clone, PartialEq, prost::Message)]
struct GroupMetadata {
    #[prost(int32, tag = "1")]
    items_count: i32,
}

pub fn group_count(value: &str) -> u32 {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .ok()
        .and_then(|bytes| GroupMetadata::decode(bytes.as_slice()).ok())
        .map_or(0, |metadata| metadata.items_count.max(0) as u32)
}

pub fn parse_recents(data: &Value, offset: u32) -> Result<RecentsPage> {
    let list = &data["lists"][0];
    if list["__typename"] != "List" {
        bail!("Spotify sent an unexpected Recents list.");
    }
    let items = &list["items"];
    let entries: Vec<RecentEntry> = items["items"]
        .as_array()
        .into_iter()
        .flatten()
        .map(recent_entry)
        .collect();
    let count = entries.len() as u32;
    Ok(RecentsPage {
        total: items["totalCount"]
            .as_u64()
            .map_or(offset + count, |total| total as u32),
        offset: items["pagingInfo"]["offset"]
            .as_u64()
            .map_or(offset, |value| value as u32),
        next_offset: items["pagingInfo"]["nextOffset"]
            .as_u64()
            .map(|value| value as u32),
        fallback: false,
        entries,
    })
}

fn contributors(items: &Value) -> Vec<Link> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(Link {
                uri: text(&item["uri"]).unwrap_or_default().to_owned(),
                name: text(&item["name"])
                    .or_else(|| text(&item["profile"]["name"]))?
                    .to_owned(),
            })
        })
        .collect()
}

fn square_image(visual: &Value) -> Option<String> {
    let square = &visual["squareCoverImage"];
    image(&square["image"]["data"]["sources"]).or_else(|| {
        let instances = square["originalInstances"].as_array()?;
        let url = |instance: &Value| text(&instance["flatFile"]["cdnUrl"]).map(str::to_owned);
        instances
            .iter()
            .find(|instance| instance["size"] == "IMAGE_SIZE_DEFAULT")
            .and_then(url)
            .or_else(|| instances.iter().find_map(url))
    })
}

fn recent_entry(item: &Value) -> RecentEntry {
    let attributes: Vec<(&str, &str)> = item["formatListAttributes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|attribute| {
            Some((
                attribute["key"].as_str()?,
                attribute["value"].as_str().unwrap_or_default(),
            ))
        })
        .collect();
    let has = |key: &str| attributes.iter().any(|(found, _)| *found == key);
    let value = |key: &str| {
        attributes
            .iter()
            .find(|(found, _)| *found == key)
            .map(|(_, value)| *value)
            .filter(|value| !value.is_empty())
    };
    let wrapper = &item["entity"];
    let data = &wrapper["data"];
    let entity = if data["__typename"] == "Entity" {
        data
    } else {
        &Value::Null
    };
    let identity = &entity["identityTrait"];
    let uri = text(&wrapper["_uri"])
        .or_else(|| text(&entity["uri"]))
        .unwrap_or_default();
    let by_uri = RecentKind::from_uri(uri);
    let kind = match by_uri {
        RecentKind::LikedSongs | RecentKind::YourEpisodes => by_uri,
        _ => entity["entityTypeTrait"]["type"]
            .as_str()
            .and_then(RecentKind::from_entity)
            .unwrap_or(by_uri),
    };
    let mut entry = RecentEntry::new(uri, kind, text(&identity["name"]).unwrap_or_default());
    entry.image = square_image(&entity["visualIdentityTrait"]);
    entry.contributors = contributors(&identity["contributors"]["items"]);
    let parent = &identity["contentHierarchyParent"];
    if parent["__typename"]
        .as_str()
        .is_none_or(|name| name == "Entity")
    {
        entry.parent = text(&parent["identityTrait"]["name"]).map(|name| Link {
            uri: text(&parent["uri"]).unwrap_or_default().to_owned(),
            name: name.to_owned(),
        });
    }
    let added = &item["addedAt"];
    entry.day = added["year"].as_i64().map(|year| {
        let month = added["month"].as_i64().unwrap_or(1);
        let day = added["day"].as_i64().unwrap_or(1);
        days_from_civil(year, month, day) * DAY_MS
    });
    entry.explicit = entity["consumptionExperienceTrait"]["contentRatings"]
        .as_array()
        .is_some_and(|ratings| ratings.iter().any(|rating| rating == EXPLICIT));
    entry.activity = if has("recent_type_shared") {
        Activity::Shared
    } else if has("recent_type_saved") {
        Activity::Saved
    } else if has("recent_type_played") {
        Activity::Played
    } else {
        Activity::Unknown
    };
    let mut contents: Vec<Content> = attributes
        .iter()
        .filter_map(|(key, _)| match *key {
            "content_type_music" => Some(Content::Music),
            "content_type_podcasts" => Some(Content::Podcasts),
            "content_type_audiobooks" => Some(Content::Audiobooks),
            _ => None,
        })
        .collect();
    contents.sort();
    contents.dedup();
    entry.contents = contents;
    entry.count = value("group_metadata").map_or(0, group_count);
    entry.group = attributes
        .iter()
        .find_map(|(key, _)| key.strip_prefix("group_id_"))
        .filter(|id| !id.is_empty() && *id != "0")
        .map(str::to_owned);
    entry.children_group = value("children_group_id").map(str::to_owned);
    entry
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct PlayedContext {
    pub uri: String,
    #[serde(rename = "lastPlayedTime", default)]
    pub played_at: i64,
}

pub fn recently_played_path(user: &str) -> String {
    let user: String = url::form_urlencoded::byte_serialize(user.as_bytes()).collect();
    format!(
        "/recently-played/v3/user/{user}/recently-played?format=json&offset=0&limit={RECENTLY_PLAYED_LIMIT}&filter=default,collection-new-episodes"
    )
}

pub fn context_uri(uri: &str) -> String {
    let parts: Vec<&str> = uri.split(':').collect();
    match parts.as_slice() {
        ["spotify", "user", _, "playlist", id] => format!("spotify:playlist:{id}"),
        _ => uri.to_owned(),
    }
}

pub fn parse_played(value: &Value) -> Vec<PlayedContext> {
    value["playContexts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|context| serde_json::from_value::<PlayedContext>(context.clone()).ok())
        .filter(|context| !context.uri.is_empty())
        .map(|context| PlayedContext {
            uri: context_uri(&context.uri),
            ..context
        })
        .collect()
}

pub fn lookup_entry(value: &Value) -> Option<RecentEntry> {
    let typename = value["__typename"].as_str()?;
    if matches!(
        typename,
        "UnknownTypeWrapper" | "PreReleaseResponseWrapper" | "AuthorResponseWrapper"
    ) {
        return None;
    }
    let data = &value["data"];
    let uri = |data: &Value| {
        text(&data["uri"])
            .or_else(|| text(&value["_uri"]))
            .unwrap_or_default()
            .to_owned()
    };
    let publisher = |show: &Value| {
        text(&show["publisher"]["name"])
            .map(|name| Link {
                uri: String::new(),
                name: name.to_owned(),
            })
            .into_iter()
            .collect::<Vec<_>>()
    };
    let authors = |book: &Value| {
        let authors = &book["authorsV2"];
        if authors.is_array() {
            contributors(authors)
        } else {
            contributors(&authors["items"])
        }
    };
    let entry = match data["__typename"].as_str()? {
        "Album" => {
            let mut entry = RecentEntry::new(uri(data), RecentKind::Album, text(&data["name"])?);
            entry.image = image(&data["coverArt"]["sources"]);
            entry.contributors = contributors(&data["artists"]["items"]);
            entry
        }
        "Track" => {
            let mut entry = RecentEntry::new(uri(data), RecentKind::Track, text(&data["name"])?);
            entry.image = image(&data["albumOfTrack"]["coverArt"]["sources"]);
            entry.contributors = contributors(&data["artists"]["items"]);
            entry.explicit = data["contentRating"]["label"] == "EXPLICIT";
            entry
        }
        "Playlist" => {
            let mut entry = RecentEntry::new(uri(data), RecentKind::Playlist, text(&data["name"])?);
            entry.image = image(&data["images"]["items"][0]["sources"]);
            let owner = &data["ownerV2"]["data"];
            entry.contributors = text(&owner["name"])
                .or_else(|| text(&owner["username"]))
                .map(|name| Link {
                    uri: text(&owner["uri"]).unwrap_or_default().to_owned(),
                    name: name.to_owned(),
                })
                .into_iter()
                .collect();
            entry
        }
        "Podcast" => {
            let mut entry = RecentEntry::new(uri(data), RecentKind::Show, text(&data["name"])?);
            entry.image = image(&data["coverArt"]["sources"]);
            entry.contributors = publisher(data);
            entry
        }
        "Audiobook" => {
            let mut entry =
                RecentEntry::new(uri(data), RecentKind::Audiobook, text(&data["name"])?);
            entry.image = image(&data["coverArt"]["sources"]);
            entry.contributors = authors(data);
            entry
        }
        "Artist" => {
            let mut entry = RecentEntry::new(
                uri(data),
                RecentKind::Artist,
                text(&data["profile"]["name"])?,
            );
            entry.image = image(&data["visuals"]["avatarImage"]["sources"]);
            entry
        }
        "Episode" => {
            let show = &data["podcastV2"]["data"];
            if show["__typename"] != "Podcast" {
                return None;
            }
            let mut entry =
                RecentEntry::new(text(&show["uri"])?, RecentKind::Show, text(&show["name"])?);
            entry.image = image(&show["coverArt"]["sources"]);
            entry.contributors = publisher(show);
            entry
        }
        "Chapter" => {
            let book = &data["audiobookV2"]["data"];
            if book["__typename"] != "Audiobook" {
                return None;
            }
            let mut entry = RecentEntry::new(
                text(&book["uri"])?,
                RecentKind::Audiobook,
                text(&book["name"])?,
            );
            entry.image = image(&book["coverArt"]["sources"]);
            entry.contributors = authors(book);
            entry
        }
        _ => return None,
    };
    (!entry.uri.is_empty()).then_some(entry)
}

fn played_entry(mut entry: RecentEntry, played_at: i64) -> RecentEntry {
    entry.played_at = Some(played_at);
    entry.day = Some(day_of(played_at));
    entry.activity = Activity::Played;
    entry
}

pub fn decorated(contexts: &[&PlayedContext], data: &Value) -> Result<Vec<RecentEntry>> {
    let Some(lookup) = data["lookup"].as_array() else {
        bail!("Spotify did not describe your recently played items.");
    };
    Ok(lookup
        .iter()
        .zip(contexts)
        .filter_map(|(value, context)| {
            lookup_entry(value).map(|entry| played_entry(entry, context.played_at))
        })
        .collect())
}

fn collection_entry(context: &PlayedContext) -> RecentEntry {
    let (kind, name) = if is_your_episodes(&context.uri) {
        (RecentKind::YourEpisodes, "Your Episodes")
    } else {
        (RecentKind::LikedSongs, "Liked Songs")
    };
    played_entry(
        RecentEntry::new(&context.uri, kind, name),
        context.played_at,
    )
}

pub fn merge_played(entries: Vec<RecentEntry>) -> Vec<RecentEntry> {
    let mut merged: Vec<RecentEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        match merged.iter_mut().find(|known| known.uri == entry.uri) {
            Some(known) if known.played_at < entry.played_at => *known = entry,
            Some(_) => {}
            None => merged.push(entry),
        }
    }
    merged.sort_by_key(|entry| std::cmp::Reverse(entry.played_at));
    merged
}

pub async fn decorate(api: &Api, contexts: &[PlayedContext]) -> Result<Vec<RecentEntry>> {
    let mut entries = Vec::new();
    let mut remote = Vec::new();
    for context in contexts {
        match RecentKind::from_uri(&context.uri) {
            RecentKind::LikedSongs | RecentKind::YourEpisodes => {
                entries.push(collection_entry(context))
            }
            RecentKind::Other | RecentKind::User => {}
            _ => remote.push(context),
        }
    }
    for chunk in remote.chunks(BATCH) {
        let uris: Vec<&str> = chunk.iter().map(|context| context.uri.as_str()).collect();
        let data = pathfinder::query(
            api,
            FETCH_ENTITIES_FOR_RECENTLY_PLAYED,
            json!({ "uris": uris }),
        )
        .await?;
        entries.extend(decorated(chunk, &data)?);
    }
    Ok(merge_played(entries))
}

pub async fn recently_played(api: &Api, user: &str) -> Result<Vec<RecentEntry>> {
    let value: Value = api.get_json(&recently_played_path(user)).await?;
    decorate(api, &parse_played(&value)).await
}

pub fn recent_contexts(entries: &[RecentEntry], limit: usize) -> Vec<Recent> {
    entries
        .iter()
        .filter(|entry| entry.kind.context() && !entry.name.trim().is_empty())
        .take(limit)
        .map(|entry| Recent {
            uri: entry.uri.clone(),
            name: entry.name.clone(),
        })
        .collect()
}

pub fn fallback_recents(entries: Vec<RecentEntry>) -> RecentsPage {
    RecentsPage {
        total: entries.len() as u32,
        offset: 0,
        next_offset: None,
        fallback: true,
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_795_600_000_000;

    fn metadata(count: i32) -> String {
        base64::engine::general_purpose::STANDARD
            .encode(GroupMetadata { items_count: count }.encode_to_vec())
    }

    fn attributes(pairs: &[(&str, &str)]) -> Value {
        Value::Array(
            pairs
                .iter()
                .map(|(key, value)| json!({"key": key, "value": value}))
                .collect(),
        )
    }

    fn entity(uri: &str, kind: &str, name: &str) -> Value {
        json!({"__typename": "EntityResponseWrapper", "_uri": uri, "data": {"__typename": "Entity",
            "entityTypeTrait": {"__typename": "EntityTypeTrait", "type": kind},
            "identityTrait": {"__typename": "IdentityTrait", "name": name,
                "contributors": {"items": [{"name": "Someone", "uri": "spotify:artist:0000000000000000000009"}]}},
            "visualIdentityTrait": {"squareCoverImage": {"image": {"data": {"__typename": "ImageV2",
                "sources": [{"maxWidth": 300, "maxHeight": 300, "url": format!("https://image-cdn-ak.spotifycdn.com/image/{name}")}]}}}},
            "consumptionExperienceTrait": {"contentRatings": ["CONTENT_RATING_EXPLICIT"]}
        }})
    }

    fn sample() -> Value {
        json!({"lists": [{"__typename": "List", "uri": RECENTS_PAGE, "formatListAttributes": [],
        "items": {"totalCount": 240, "pagingInfo": {"offset": 0, "limit": 100, "nextOffset": 100}, "items": [
            {"addedAt": {"year": 2026, "month": 11, "day": 24},
             "entity": entity("spotify:album:0000000000000000000001", "ENTITY_TYPE_ALBUM", "First"),
             "formatListAttributes": attributes(&[("group_id_0", ""), ("recent_type_played", ""), ("content_type_music", ""), ("group_metadata", &metadata(7))])},
            {"addedAt": {"year": 2026, "month": 11, "day": 24},
             "entity": null,
             "formatListAttributes": attributes(&[("group_id_0", ""), ("recent_type_saved", ""), ("content_type_music", ""), ("children_group_id", "5"), ("group_metadata", &metadata(2))])},
            {"addedAt": {"year": 2026, "month": 11, "day": 24},
             "entity": entity("spotify:track:0000000000000000000002", "ENTITY_TYPE_TRACK", "Song A"),
             "formatListAttributes": attributes(&[("group_id_5", ""), ("recent_type_saved", "")])},
            {"addedAt": {"year": 2026, "month": 11, "day": 24},
             "entity": entity("spotify:track:0000000000000000000003", "ENTITY_TYPE_TRACK", "Song B"),
             "formatListAttributes": attributes(&[("group_id_5", ""), ("recent_type_saved", "")])},
            {"addedAt": {"year": 2026, "month": 11, "day": 22},
             "entity": entity("spotify:playlist:0000000000000000000004", "ENTITY_TYPE_PLAYLIST", "Mix"),
             "formatListAttributes": attributes(&[("group_id_0", ""), ("recent_type_saved", "")])},
            {"addedAt": {"year": 2026, "month": 11, "day": 25},
             "entity": entity("spotify:artist:0000000000000000000005", "ENTITY_TYPE_ARTIST", "Band"),
             "formatListAttributes": attributes(&[("group_id_0", ""), ("recent_type_played", ""), ("content_type_music", ""), ("group_metadata", &metadata(1))])},
            {"addedAt": {"year": 2026, "month": 11, "day": 25},
             "entity": {"__typename": "EntityResponseWrapper", "_uri": "spotify:episode:0000000000000000000006", "data": {"__typename": "Entity",
                "entityTypeTrait": {"type": "ENTITY_TYPE_PODCAST_EPISODE"},
                "identityTrait": {"name": "Episode 1", "contentHierarchyParent": {"__typename": "Entity", "uri": "spotify:show:0000000000000000000007",
                    "identityTrait": {"name": "The Show"}}}}},
             "formatListAttributes": attributes(&[("group_id_0", ""), ("recent_type_played", ""), ("content_type_podcasts", "")])},
            {"addedAt": {"year": 2026, "month": 11, "day": 25},
             "entity": null,
             "formatListAttributes": attributes(&[("group_id_0", "")])}
        ]}}]})
    }

    #[test]
    fn reads_the_recents_list() {
        assert_eq!(
            recents_variables(100, 100),
            json!({"uris": ["spotify:list:recents:page"], "offset": 100, "limit": 100})
        );
        let page = parse_recents(&sample(), 0).unwrap();
        assert_eq!(page.total, 240);
        assert_eq!(page.next_offset, Some(100));
        assert_eq!(page.entries.len(), 8);
        let album = &page.entries[0];
        assert_eq!(album.kind, RecentKind::Album);
        assert_eq!(album.name, "First");
        assert_eq!(album.activity, Activity::Played);
        assert_eq!(album.contents, vec![Content::Music]);
        assert_eq!(album.count, 7);
        assert!(album.explicit);
        assert_eq!(album.group, None);
        assert_eq!(
            album.image.as_deref(),
            Some("https://image-cdn-ak.spotifycdn.com/image/First")
        );
        assert_eq!(album.names(), "Someone");
        let stack = &page.entries[1];
        assert_eq!((stack.uri.as_str(), stack.count), ("", 2));
        assert_eq!(stack.children_group.as_deref(), Some("5"));
        assert_eq!(page.entries[2].group.as_deref(), Some("5"));
        let episode = &page.entries[6];
        assert_eq!(episode.kind, RecentKind::Episode);
        assert_eq!(
            episode.parent.as_ref().map(|parent| parent.name.as_str()),
            Some("The Show")
        );
        assert!(parse_recents(&json!({"lists": [{"__typename": "NotFound"}]}), 0).is_err());
        assert_eq!(group_count("not base64"), 0);
    }

    #[test]
    fn groups_recents_by_day_and_stack() {
        let page = parse_recents(&sample(), 0).unwrap();
        let days = recent_days(&page.entries);
        let dates: Vec<i64> = days.iter().map(|day| day.day).collect();
        let day = |date: i64| days_from_civil(2026, 11, date) * DAY_MS;
        assert_eq!(dates, vec![day(25), day(24), day(22)]);
        assert_eq!(days[0].groups.len(), 2);
        let today = &days[1].groups;
        assert_eq!(today.len(), 2);
        assert!(today[1].stacked());
        assert_eq!(today[1].children.len(), 2);
        assert_eq!(today[1].title(), "2 songs added");
        assert_eq!(today[1].subtitle(), (false, "Song A, Song B".to_owned()));
        assert_eq!(
            today[1].images(),
            vec![
                "https://image-cdn-ak.spotifycdn.com/image/Song A",
                "https://image-cdn-ak.spotifycdn.com/image/Song B"
            ]
        );
        assert_eq!(today[1].key(day(24)), format!("{}:5", day(24)));
        assert_eq!(today[0].title(), "First");
        assert_eq!(
            today[0].subtitle(),
            (false, "Album • Someone • 7 songs played".to_owned())
        );
        assert_eq!(
            days[2].groups[0].subtitle(),
            (true, "Playlist added".to_owned())
        );
        assert_eq!(
            days[0].groups[0].subtitle(),
            (false, "Artist • 1 song played".to_owned())
        );
        assert_eq!(
            days[0].groups[1].subtitle(),
            (false, "Episode • The Show".to_owned())
        );
    }

    #[test]
    fn counts_like_the_desktop() {
        let mut entry = RecentEntry::new("spotify:playlist:x", RecentKind::Playlist, "Mix");
        entry.count = 3;
        assert_eq!(entry.count_text(), None);
        entry.contents = vec![Content::Music];
        entry.activity = Activity::Played;
        assert_eq!(entry.count_text().as_deref(), Some("3 songs played"));
        assert_eq!(
            entry.subtitle(),
            (false, "3 songs played • Playlist".to_owned())
        );
        entry.activity = Activity::Saved;
        assert_eq!(entry.subtitle(), (true, "3 songs added".to_owned()));
        entry.contents = vec![Content::Music, Content::Podcasts];
        assert_eq!(entry.count_text().as_deref(), Some("3 items added"));
        entry.activity = Activity::Shared;
        assert_eq!(entry.count_text().as_deref(), Some("3 shares"));
        let mut show = RecentEntry::new("spotify:show:x", RecentKind::Show, "Talk");
        show.count = 1;
        show.activity = Activity::Played;
        assert_eq!(show.count_text().as_deref(), Some("1 episode played"));
        let mut liked = RecentEntry::new(
            "spotify:user:someone:collection",
            RecentKind::LikedSongs,
            "Liked Songs",
        );
        liked.activity = Activity::Played;
        assert_eq!(liked.subtitle(), (false, "Playlist".to_owned()));
    }

    #[test]
    fn labels_days_like_the_desktop() {
        let today = day_of(NOW);
        assert_eq!(day_label(today, NOW), "Today");
        assert_eq!(day_label(today - DAY_MS, NOW), "Yesterday");
        let date = days_from_civil(2026, 11, 17) * DAY_MS;
        assert_eq!(day_label(date, NOW), "Tue, Nov 17");
        let old = days_from_civil(2025, 2, 28) * DAY_MS;
        assert_eq!(day_label(old, NOW), "Fri, Feb 28, 2025");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }

    #[test]
    fn decorates_recently_played_contexts() {
        assert_eq!(
            recently_played_path("someone"),
            "/recently-played/v3/user/someone/recently-played?format=json&offset=0&limit=50&filter=default,collection-new-episodes"
        );
        let played = parse_played(&json!({"playContexts": [
            {"uri": "spotify:album:0000000000000000000001", "lastPlayedTime": 1_795_472_846_537_i64, "lastPlayedTrackUri": "spotify:track:x"},
            {"uri": "spotify:user:someone:playlist:0000000000000000000002", "lastPlayedTime": 1_795_460_282_225_i64},
            {"uri": "spotify:episode:0000000000000000000003", "lastPlayedTime": 1_795_400_000_000_i64},
            {"uri": "spotify:user:someone:collection", "lastPlayedTime": 1_795_300_000_000_i64},
            {"lastPlayedTime": 1}
        ]}));
        assert_eq!(played.len(), 4);
        assert_eq!(played[1].uri, "spotify:playlist:0000000000000000000002");
        let remote: Vec<&PlayedContext> = played[..3].iter().collect();
        let data = json!({"lookup": [
            {"__typename": "AlbumResponseWrapper", "_uri": "spotify:album:0000000000000000000001", "data": {"__typename": "Album",
                "name": "First", "uri": "spotify:album:0000000000000000000001",
                "artists": {"items": [{"profile": {"name": "Someone"}, "uri": "spotify:artist:0000000000000000000009"}]},
                "coverArt": {"sources": [{"height": 300, "url": "https://i.scdn.co/image/a", "width": 300}]}}},
            {"__typename": "PlaylistResponseWrapper", "_uri": "spotify:playlist:0000000000000000000002", "data": {"__typename": "Playlist",
                "name": "Mix", "uri": "spotify:playlist:0000000000000000000002",
                "images": {"items": [{"sources": [{"height": null, "url": "https://mosaic.scdn.co/640/b", "width": null}]}]},
                "ownerV2": {"data": {"__typename": "User", "name": "Someone", "uri": "spotify:user:someone", "username": "someone"}}}},
            {"__typename": "EpisodeOrChapterResponseWrapper", "_uri": "spotify:episode:0000000000000000000003", "data": {"__typename": "Episode",
                "podcastV2": {"data": {"__typename": "Podcast", "name": "The Show", "uri": "spotify:show:0000000000000000000007",
                    "publisher": {"name": "Studio"}, "coverArt": {"sources": [{"url": "https://i.scdn.co/image/c", "width": 300}]}}}}}
        ]});
        let mut entries = decorated(&remote, &data).unwrap();
        entries.push(collection_entry(&played[3]));
        let entries = merge_played(entries);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["First", "Mix", "The Show", "Liked Songs"]);
        assert_eq!(entries[2].uri, "spotify:show:0000000000000000000007");
        assert_eq!(entries[2].kind, RecentKind::Show);
        assert_eq!(
            entries[1].subtitle(),
            (false, "Playlist • Someone".to_owned())
        );
        assert_eq!(entries[0].day, Some(day_of(1_795_472_846_537)));
        assert!(decorated(&remote, &json!({})).is_err());
        assert_eq!(
            lookup_entry(&json!({"__typename": "UnknownTypeWrapper", "_uri": "spotify:station:x"})),
            None
        );
        let contexts = recent_contexts(&entries, 3);
        assert_eq!(
            contexts
                .iter()
                .map(|recent| recent.name.as_str())
                .collect::<Vec<_>>(),
            ["First", "Mix", "The Show"]
        );
        let page = fallback_recents(entries);
        assert!(page.fallback);
        assert_eq!(recent_days(&page.entries).len(), 2);
        let newer = merge_played(vec![
            played_entry(
                RecentEntry::new("spotify:album:a", RecentKind::Album, "A"),
                5,
            ),
            played_entry(
                RecentEntry::new("spotify:album:a", RecentKind::Album, "A"),
                9,
            ),
            played_entry(
                RecentEntry::new("spotify:album:b", RecentKind::Album, "B"),
                7,
            ),
        ]);
        assert_eq!(
            newer
                .iter()
                .map(|entry| (entry.uri.as_str(), entry.played_at))
                .collect::<Vec<_>>(),
            [("spotify:album:a", Some(9)), ("spotify:album:b", Some(7))]
        );
    }
}
