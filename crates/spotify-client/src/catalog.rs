use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result};
use prost::Message;
use reqwest::Method;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use spotify_proto::{
    extendedmetadata::{
        BatchedEntityRequest, BatchedExtensionResponse, EntityRequest, ExtensionKind,
        ExtensionQuery,
    },
    metadata::{self, ImageGroup},
};

use crate::api::{Api, Body};

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Link {
    pub uri: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
    pub cover: Option<String>,
    pub artist_uri: Option<String>,
    pub album_uri: Option<String>,
    #[serde(default)]
    pub artists: Vec<Link>,
    #[serde(default)]
    pub uid: Option<String>,
    #[serde(default)]
    pub added_at: Option<i64>,
    #[serde(default)]
    pub explicit: bool,
}

impl Track {
    pub fn placeholder(uri: &str) -> Self {
        Self {
            uri: uri.to_owned(),
            ..Self::default()
        }
    }

    pub fn id(&self) -> &str {
        self.uri.rsplit(':').next().unwrap_or_default()
    }

    pub fn is_track(&self) -> bool {
        self.uri.starts_with("spotify:track:")
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LocalTrack {
    pub path: std::path::PathBuf,
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
}

pub fn image_url(id: &str) -> String {
    if id.starts_with("https://") {
        return id.to_owned();
    }
    format!(
        "https://i.scdn.co/image/{}",
        id.trim_start_matches("spotify:image:")
    )
}

fn cover(group: Option<&ImageGroup>) -> Option<String> {
    group?
        .image
        .iter()
        .filter_map(|image| Some((image.width.unwrap_or(300), image.file_id.as_ref()?)))
        .min_by_key(|(width, _)| (width - 300).abs())
        .map(|(_, id)| image_url(&hex(id)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

const BASE62: &[u8; 62] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

pub fn uri_from_gid(kind: &str, gid: &[u8]) -> Option<String> {
    let bytes = <[u8; 16]>::try_from(gid).ok()?;
    let mut value = u128::from_be_bytes(bytes);
    let mut digits = [b'0'; 22];
    for slot in digits.iter_mut().rev() {
        *slot = BASE62[(value % 62) as usize];
        value /= 62;
    }
    Some(format!(
        "spotify:{kind}:{}",
        String::from_utf8_lossy(&digits)
    ))
}

fn largest(images: &[metadata::Image], target: i32) -> Option<String> {
    images
        .iter()
        .filter_map(|image| Some((image.width.unwrap_or(target), image.file_id.as_ref()?)))
        .min_by_key(|(width, _)| (width - target).abs())
        .map(|(_, id)| image_url(&hex(id)))
}

pub(crate) async fn extended(
    api: &Api,
    entries: &[(String, ExtensionKind)],
) -> Result<Vec<(String, Vec<u8>)>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    let request = BatchedEntityRequest {
        header: None,
        entity_request: entries
            .iter()
            .map(|(uri, kind)| EntityRequest {
                entity_uri: uri.clone(),
                query: vec![ExtensionQuery {
                    extension_kind: *kind as i32,
                    etag: String::new(),
                }],
            })
            .collect(),
    };
    let bytes = api
        .protobuf(
            Method::POST,
            "/extended-metadata/v0/extended-metadata",
            request.encode_to_vec(),
        )
        .await?;
    let response = BatchedExtensionResponse::decode(bytes.as_slice())?;
    Ok(response
        .extended_metadata
        .into_iter()
        .flat_map(|array| array.extension_data)
        .filter_map(|data| Some((data.entity_uri, data.extension_data?.value)))
        .collect())
}

const LINK_KINDS: [&str; 6] = ["track", "album", "playlist", "artist", "show", "episode"];

fn base62_id(id: &str) -> bool {
    id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn plain(part: &str) -> bool {
    !part.is_empty() && !part.contains(char::is_whitespace)
}

fn link_uri(parts: &[&str]) -> Result<String> {
    match parts {
        [kind, id] if LINK_KINDS.contains(kind) => {
            if !base62_id(id) {
                anyhow::bail!("The Spotify {kind} ID is invalid.");
            }
            Ok(format!("spotify:{kind}:{id}"))
        }
        [kind] if LINK_KINDS.contains(kind) => {
            anyhow::bail!("The Spotify {kind} link is missing its ID.")
        }
        ["user", user, "playlist", id] if plain(user) => link_uri(&["playlist", id]),
        ["user", user] if plain(user) => Ok(format!("spotify:user:{user}")),
        ["genre", id] if base62_id(id) => Ok(format!("spotify:page:{id}")),
        ["genre", id] if plain(id) => Ok(format!("spotify:genre:{id}")),
        ["page", id] if plain(id) => Ok(format!("spotify:page:{id}")),
        ["socialsession", token] if plain(token) => Ok(format!("spotify:socialsession:{token}")),
        ["blend", "taste-match", id] if crate::blend::valid_id(id) => {
            Ok(format!("spotify:blend:taste-match:{id}"))
        }
        _ => anyhow::bail!("OpenMusic can’t open this kind of Spotify link."),
    }
}

pub fn parse_link(input: &str) -> Result<Option<String>> {
    let input = input.trim();
    let parts: Vec<String> = if let Some(rest) = input.strip_prefix("spotify:") {
        rest.split(':').map(str::to_owned).collect()
    } else if let Ok(url) = url::Url::parse(input)
        && url.host_str() == Some("open.spotify.com")
    {
        url.path_segments()
            .into_iter()
            .flatten()
            .filter(|segment| {
                !segment.is_empty() && !segment.starts_with("intl-") && *segment != "embed"
            })
            .map(str::to_owned)
            .collect()
    } else {
        return Ok(None);
    };
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    link_uri(&parts).map(Some)
}

fn show_link(show: Option<&metadata::Show>) -> Option<Link> {
    let show = show?;
    Some(Link {
        uri: uri_from_gid("show", show.gid.as_deref()?)?,
        name: show.name.clone()?,
    })
}

fn episode_track(uri: String, episode: metadata::Episode) -> Track {
    Track {
        title: episode.name.clone().unwrap_or_default(),
        artist: episode
            .show
            .as_ref()
            .and_then(|show| show.name.clone())
            .unwrap_or_default(),
        duration_ms: episode.duration.unwrap_or_default().max(0) as u32,
        cover: cover(episode.cover_image.as_ref()),
        artists: show_link(episode.show.as_ref()).into_iter().collect(),
        explicit: episode.explicit.unwrap_or_default(),
        uri,
        ..Track::default()
    }
}

fn music_track(uri: String, track: metadata::Track) -> Track {
    let album = track.album.as_ref();
    Track {
        title: track.name.clone().unwrap_or_default(),
        artist: track
            .artist
            .iter()
            .filter_map(|artist| artist.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
        album: album
            .and_then(|album| album.name.clone())
            .unwrap_or_default(),
        duration_ms: track.duration.unwrap_or_default().max(0) as u32,
        cover: cover(album.and_then(|album| album.cover_group.as_ref())),
        artist_uri: track
            .artist
            .first()
            .and_then(|artist| artist.gid.as_deref())
            .and_then(|gid| uri_from_gid("artist", gid)),
        album_uri: album
            .and_then(|album| album.gid.as_deref())
            .and_then(|gid| uri_from_gid("album", gid)),
        artists: track
            .artist
            .iter()
            .filter_map(|artist| {
                Some(Link {
                    uri: uri_from_gid("artist", artist.gid.as_deref()?)?,
                    name: artist.name.clone()?,
                })
            })
            .collect(),
        explicit: track.explicit.unwrap_or_default(),
        uri,
        ..Track::default()
    }
}

fn decode_track(uri: String, bytes: &[u8]) -> Option<Track> {
    if uri.starts_with("spotify:episode:") {
        let episode = metadata::Episode::decode(bytes).ok()?;
        Some(episode_track(uri, episode))
    } else {
        let track = metadata::Track::decode(bytes).ok()?;
        Some(music_track(uri, track))
    }
}

pub async fn tracks(api: &Api, uris: &[String]) -> Result<Vec<Track>> {
    let mut found = Vec::new();
    for chunk in uris.chunks(100) {
        let entries: Vec<(String, ExtensionKind)> = chunk
            .iter()
            .map(|uri| {
                let kind = if uri.starts_with("spotify:episode:") {
                    ExtensionKind::EpisodeV4
                } else {
                    ExtensionKind::TrackV4
                };
                (uri.clone(), kind)
            })
            .collect();
        for (uri, value) in extended(api, &entries).await? {
            found.extend(decode_track(uri, &value));
        }
    }
    let order: Vec<&String> = uris.iter().collect();
    found.sort_by_key(|track| {
        order
            .iter()
            .position(|uri| **uri == track.uri)
            .unwrap_or(usize::MAX)
    });
    Ok(found)
}

#[derive(Clone, Debug)]
pub struct Release {
    pub uri: String,
    pub name: String,
    pub year: Option<i32>,
    pub cover: Option<String>,
    pub kind: String,
}

#[derive(Clone, Debug)]
pub struct ArtistRef {
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Artist {
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
    pub biography: Option<String>,
    pub top_tracks: Vec<Track>,
    pub releases: Vec<Release>,
    pub related: Vec<ArtistRef>,
}

pub(crate) fn strip_tags(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut inside = false;
    for character in text.chars() {
        match character {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => plain.push(character),
            _ => {}
        }
    }
    plain
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_owned()
}

pub async fn artist(api: &Api, uri: &str) -> Result<Artist> {
    let data = extended(api, &[(uri.to_owned(), ExtensionKind::ArtistV4)]).await?;
    let value = data
        .into_iter()
        .next()
        .map(|(_, value)| value)
        .ok_or_else(|| anyhow::anyhow!("Spotify has no details for this artist."))?;
    let artist = metadata::Artist::decode(value.as_slice())?;
    let image = artist
        .portrait_group
        .as_ref()
        .and_then(|group| largest(&group.image, 640))
        .or_else(|| largest(&artist.portrait, 640));
    let biography = artist
        .biography
        .first()
        .and_then(|biography| biography.text.as_deref())
        .map(strip_tags)
        .filter(|text| !text.is_empty());

    let mut release_ids: Vec<(String, &'static str)> = Vec::new();
    for (groups, kind) in [
        (&artist.album_group, "Album"),
        (&artist.single_group, "Single"),
        (&artist.compilation_group, "Compilation"),
    ] {
        for group in groups.iter() {
            if let Some(gid) = group.album.first().and_then(|album| album.gid.as_deref())
                && let Some(uri) = uri_from_gid("album", gid)
            {
                release_ids.push((uri, kind));
            }
        }
    }
    release_ids.truncate(24);
    let entries: Vec<(String, ExtensionKind)> = release_ids
        .iter()
        .map(|(uri, _)| (uri.clone(), ExtensionKind::AlbumV4))
        .collect();
    let mut releases: Vec<Release> = extended(api, &entries)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(uri, value)| {
            let album = metadata::Album::decode(value.as_slice()).ok()?;
            let kind = release_ids
                .iter()
                .find(|(id, _)| *id == uri)
                .map(|(_, kind)| (*kind).to_owned())
                .unwrap_or_default();
            Some(Release {
                name: album.name.unwrap_or_default(),
                year: album.date.and_then(|date| date.year),
                cover: album
                    .cover_group
                    .as_ref()
                    .and_then(|group| largest(&group.image, 300))
                    .or_else(|| largest(&album.cover, 300)),
                kind,
                uri,
            })
        })
        .collect();
    releases.sort_by_key(|release| std::cmp::Reverse(release.year.unwrap_or_default()));

    let related_ids: Vec<(String, ExtensionKind)> = artist
        .related
        .iter()
        .filter_map(|related| uri_from_gid("artist", related.gid.as_deref()?))
        .take(8)
        .map(|uri| (uri, ExtensionKind::ArtistV4))
        .collect();
    let related = extended(api, &related_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(uri, value)| {
            let related = metadata::Artist::decode(value.as_slice()).ok()?;
            Some(ArtistRef {
                name: related.name.unwrap_or_default(),
                image: related
                    .portrait_group
                    .as_ref()
                    .and_then(|group| largest(&group.image, 300))
                    .or_else(|| largest(&related.portrait, 300)),
                uri,
            })
        })
        .collect();

    let top_tracks = context_tracks(api, uri, 10).await.unwrap_or_default();
    Ok(Artist {
        uri: uri.to_owned(),
        name: artist.name.unwrap_or_default(),
        image,
        biography,
        top_tracks,
        releases,
        related,
    })
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Playlist {
    pub uri: String,
    pub name: String,
    pub description: String,
    pub cover: Option<String>,
    pub length: u32,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub view: bool,
    pub edit_items: bool,
    pub edit_metadata: bool,
    pub administrate: bool,
    pub publish: bool,
    pub delete: bool,
    pub cancel_membership: bool,
}

impl Capabilities {
    pub fn from_playlist(capabilities: &Value) -> Self {
        let flag = |key: &str| capabilities[key].as_bool().unwrap_or_default();
        Self {
            view: flag("canView"),
            edit_items: flag("canEditItems"),
            edit_metadata: flag("canEditMetadata"),
            administrate: flag("canAdministratePermissions"),
            publish: flag("canPublishToProfile"),
            delete: flag("canDelete"),
            cancel_membership: flag("canCancelMembership"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Collection {
    pub uri: String,
    pub kind: String,
    pub name: String,
    pub subtitle: String,
    pub cover: Option<String>,
    pub artist_uri: Option<String>,
    pub tracks: Vec<Track>,
    pub owner: Option<String>,
    pub description: String,
    pub revision: Option<String>,
    pub format: Option<String>,
    pub capabilities: Capabilities,
    pub metadata: HashMap<String, String>,
    pub total: usize,
    pub next: Option<usize>,
    pub picture: Option<String>,
    pub attributions: HashMap<String, Vec<crate::blend::Attribution>>,
}

pub(crate) fn filled(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

pub(crate) fn playlist_cover(attributes: &Value) -> Option<String> {
    if let Some(url) = attributes["pictureSize"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|picture| picture["targetName"] == "default")
        .or_else(|| {
            attributes["pictureSize"]
                .as_array()
                .and_then(|pictures| pictures.first())
        })
        .and_then(|picture| picture["url"].as_str())
    {
        return Some(url.to_owned());
    }
    let picture = attributes["picture"].as_str()?;
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, picture).ok()?;
    Some(image_url(&hex(&bytes)))
}

pub async fn username(api: &Api) -> Result<String> {
    let value: Value = api
        .get_json("/context-resolve/v1/spotify:collection:tracks")
        .await?;
    value["metadata"]["context_owner"]
        .as_str()
        .and_then(|owner| owner.strip_prefix("spotify:user:"))
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("Spotify did not report your account."))
}

pub async fn playlists(api: &Api, user: &str) -> Result<Vec<Playlist>> {
    Ok(crate::library::rootlist(api, user).await?.playlists())
}

pub async fn collection(api: &Api, uri: &str) -> Result<Collection> {
    if crate::playlist::playlist_id(uri).is_some() {
        return crate::playlist::collection(api, uri, crate::playlist::PAGE).await;
    }
    if uri.starts_with("spotify:album:") {
        let data = extended(api, &[(uri.to_owned(), ExtensionKind::AlbumV4)]).await?;
        let album = data
            .into_iter()
            .next()
            .and_then(|(_, value)| metadata::Album::decode(value.as_slice()).ok())
            .ok_or_else(|| anyhow::anyhow!("Spotify has no details for this album."))?;
        let resolved = resolve(api, uri, 150).await?;
        let tracks = item_tracks(api, &resolved.items).await?;
        let total = tracks.len();
        let artist = album.artist.first();
        let year = album.date.and_then(|date| date.year);
        let subtitle = [
            artist.and_then(|artist| artist.name.clone()),
            year.map(|year| year.to_string()),
            Some(format!("{} songs", tracks.len())),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        return Ok(Collection {
            uri: uri.to_owned(),
            kind: "ALBUM".into(),
            name: album.name.unwrap_or_default(),
            subtitle,
            cover: album
                .cover_group
                .as_ref()
                .and_then(|group| largest(&group.image, 640))
                .or_else(|| largest(&album.cover, 640)),
            artist_uri: artist
                .and_then(|artist| artist.gid.as_deref())
                .and_then(|gid| uri_from_gid("artist", gid)),
            tracks,
            owner: None,
            description: String::new(),
            revision: None,
            format: None,
            capabilities: Capabilities {
                view: true,
                ..Capabilities::default()
            },
            metadata: resolved.metadata,
            total,
            next: None,
            picture: None,
            attributions: HashMap::new(),
        });
    }
    anyhow::bail!("OpenMusic can only open albums and playlists.")
}

fn strings<'de, D>(deserializer: D) -> std::result::Result<HashMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let values = Option::<HashMap<String, Value>>::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| match value {
            Value::String(text) => Some((key, text)),
            Value::Number(number) => Some((key, number.to_string())),
            Value::Bool(flag) => Some((key, flag.to_string())),
            _ => None,
        })
        .collect())
}

#[derive(Deserialize)]
struct ContextResponse {
    #[serde(default, deserialize_with = "strings")]
    metadata: HashMap<String, String>,
    #[serde(default)]
    pages: Vec<ContextPage>,
}

#[derive(Deserialize)]
struct ContextPage {
    #[serde(default)]
    tracks: Vec<ContextTrack>,
    #[serde(default)]
    page_url: Option<String>,
    #[serde(default)]
    next_page_url: Option<String>,
}

#[derive(Deserialize)]
struct ContextTrack {
    #[serde(default)]
    uri: String,
    #[serde(default)]
    uid: Option<String>,
    #[serde(default, deserialize_with = "strings")]
    metadata: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextItem {
    pub uri: String,
    pub uid: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl ContextItem {
    pub fn added_at(&self) -> Option<i64> {
        self.metadata.get("added_at")?.parse().ok()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Resolved {
    pub metadata: HashMap<String, String>,
    pub items: Vec<ContextItem>,
}

fn page_path(url: &str) -> String {
    url.trim_start_matches("hm://").to_owned()
}

fn take_page(
    page: ContextPage,
    seen: &mut HashSet<String>,
    items: &mut Vec<ContextItem>,
    pending: &mut Vec<String>,
) {
    if page.tracks.is_empty() {
        pending.extend(page.page_url.or(page.next_page_url));
        return;
    }
    for track in page.tracks {
        if (track.uri.starts_with("spotify:track:") || track.uri.starts_with("spotify:episode:"))
            && seen.insert(track.uri.clone())
        {
            items.push(ContextItem {
                uri: track.uri,
                uid: track.uid.filter(|uid| !uid.is_empty()),
                metadata: track.metadata,
            });
        }
    }
}

fn parse_lossy<T: DeserializeOwned>(bytes: &[u8]) -> serde_json::Result<T> {
    serde_json::from_str(&String::from_utf8_lossy(bytes))
}

async fn lossy_json<T: DeserializeOwned>(api: &Api, path: &str) -> Result<T> {
    let bytes = api
        .request(Method::GET, path, Body::Empty, "application/json")
        .await?;
    parse_lossy(&bytes).with_context(|| format!("Unreadable reply from {path}."))
}

pub async fn resolve(api: &Api, context: &str, limit: usize) -> Result<Resolved> {
    let encoded: String = url::form_urlencoded::byte_serialize(context.as_bytes()).collect();
    let mut response: ContextResponse = lossy_json(
        api,
        &format!("/context-resolve/v1/{}", encoded.replace("%3A", ":")),
    )
    .await?;
    let mut seen = HashSet::new();
    let mut items = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    loop {
        for page in response.pages.drain(..) {
            take_page(page, &mut seen, &mut items, &mut pending);
        }
        if items.len() >= limit {
            break;
        }
        let Some(next) = pending.pop() else { break };
        let page: ContextPage = lossy_json(api, &page_path(&next)).await?;
        response.pages.push(page);
    }
    items.truncate(limit);
    Ok(Resolved {
        metadata: response.metadata,
        items,
    })
}

pub async fn context_uris(api: &Api, context: &str, limit: usize) -> Result<Vec<ContextItem>> {
    Ok(resolve(api, context, limit).await?.items)
}

pub async fn item_tracks(api: &Api, items: &[ContextItem]) -> Result<Vec<Track>> {
    let uris: Vec<String> = items.iter().map(|item| item.uri.clone()).collect();
    let by_uri: HashMap<&str, &ContextItem> =
        items.iter().map(|item| (item.uri.as_str(), item)).collect();
    let mut found = tracks(api, &uris).await?;
    for track in &mut found {
        if let Some(item) = by_uri.get(track.uri.as_str()) {
            track.uid = item.uid.clone();
            track.added_at = item.added_at();
        }
    }
    Ok(found)
}

pub async fn context_tracks(api: &Api, context: &str, limit: usize) -> Result<Vec<Track>> {
    let items = context_uris(api, context, limit).await?;
    item_tracks(api, &items).await
}

pub async fn search(api: &Api, query: &str) -> Result<Vec<Track>> {
    let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
    context_tracks(api, &format!("spotify:search:{encoded}"), 20).await
}

pub async fn station(api: &Api, seed: &str) -> Result<Vec<Track>> {
    let id = seed.rsplit(':').next().unwrap_or_default();
    let mut tracks = context_tracks(api, &format!("spotify:station:track:{id}"), 13).await?;
    tracks.retain(|track| track.uri != seed);
    tracks.truncate(12);
    Ok(tracks)
}

pub async fn liked(api: &Api, limit: usize) -> Result<Vec<Track>> {
    context_tracks(api, "spotify:collection:tracks", limit).await
}

#[derive(Clone, Debug)]
pub struct Lyrics {
    pub synced: bool,
    pub lines: Vec<LyricLine>,
    pub provider: String,
}

#[derive(Clone, Debug)]
pub struct LyricLine {
    pub start_ms: u32,
    pub words: String,
}

pub async fn lyrics(api: &Api, track_uri: &str) -> Result<Option<Lyrics>> {
    let Some(id) = track_uri.strip_prefix("spotify:track:") else {
        return Ok(None);
    };
    let value: Value = match api
        .get_json(&format!(
            "/color-lyrics/v2/track/{id}?format=json&market=from_token"
        ))
        .await
    {
        Ok(value) => value,
        Err(error)
            if error
                .downcast_ref::<crate::api::HttpStatus>()
                .is_some_and(|status| status.0 == reqwest::StatusCode::NOT_FOUND) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let body = &value["lyrics"];
    let lines: Vec<LyricLine> = body["lines"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|line| LyricLine {
            start_ms: line["startTimeMs"]
                .as_str()
                .and_then(|start| start.parse().ok())
                .unwrap_or_default(),
            words: line["words"]
                .as_str()
                .unwrap_or_default()
                .replace('♪', "")
                .trim()
                .to_owned(),
        })
        .collect();
    if lines.iter().all(|line| line.words.is_empty()) {
        return Ok(None);
    }
    Ok(Some(Lyrics {
        synced: body["syncType"].as_str() == Some("LINE_SYNCED"),
        lines,
        provider: body["providerDisplayName"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const TRACK_ID: &str = "4uLU6hMCjMI75M1A2tKUQC";

    fn uri(input: &str) -> String {
        parse_link(input).unwrap().unwrap()
    }

    #[test]
    fn distinguishes_tracks_from_search_queries() {
        let track = format!("spotify:track:{TRACK_ID}");
        for input in [
            track.clone(),
            format!("https://open.spotify.com/track/{TRACK_ID}?si=123"),
            format!("https://open.spotify.com/intl-de/track/{TRACK_ID}"),
            format!("  https://open.spotify.com/embed/track/{TRACK_ID}  "),
        ] {
            assert_eq!(uri(&input), track);
        }
        assert!(parse_link("Nina Simone").unwrap().is_none());
        assert!(parse_link("https://example.com/track/x").unwrap().is_none());
        assert!(parse_link("spotify:track:broken").is_err());
        assert!(parse_link("https://open.spotify.com/album/abc").is_err());
        assert!(parse_link("https://open.spotify.com/track").is_err());
        assert!(parse_link("https://open.spotify.com/concert/abc").is_err());
    }

    #[test]
    fn parses_links_of_every_kind() {
        for kind in ["album", "playlist", "artist", "show", "episode"] {
            let expected = format!("spotify:{kind}:{TRACK_ID}");
            assert_eq!(uri(&expected), expected);
            assert_eq!(
                uri(&format!(
                    "https://open.spotify.com/intl-fr/{kind}/{TRACK_ID}?si=abc"
                )),
                expected
            );
        }
        assert_eq!(
            uri(&format!(
                "https://open.spotify.com/user/someone/playlist/{TRACK_ID}"
            )),
            format!("spotify:playlist:{TRACK_ID}")
        );
        assert_eq!(
            uri("https://open.spotify.com/user/someone"),
            "spotify:user:someone"
        );
        assert_eq!(uri("spotify:user:someone"), "spotify:user:someone");
        assert_eq!(
            uri("https://open.spotify.com/genre/0JQ5DAqbMKFDXXwE9BDJAr"),
            "spotify:page:0JQ5DAqbMKFDXXwE9BDJAr"
        );
        assert_eq!(
            uri("https://open.spotify.com/genre/entertainment_v2_podcasts-page"),
            "spotify:genre:entertainment_v2_podcasts-page"
        );
        assert_eq!(
            uri("spotify:genre:0JQ5DAqbMKFOOxftoKZxod"),
            "spotify:page:0JQ5DAqbMKFOOxftoKZxod"
        );
        assert_eq!(
            uri("spotify:page:0JQ5DAt0tbjZptfcdMSKl3"),
            "spotify:page:0JQ5DAt0tbjZptfcdMSKl3"
        );
        assert_eq!(
            uri("https://open.spotify.com/socialsession/jamtoken123"),
            "spotify:socialsession:jamtoken123"
        );
        assert_eq!(
            uri("spotify:socialsession:jamtoken123"),
            "spotify:socialsession:jamtoken123"
        );
        assert_eq!(
            uri("https://open.spotify.com/blend/taste-match/AbC-123_x?si=1"),
            "spotify:blend:taste-match:AbC-123_x"
        );
        assert!(parse_link("https://open.spotify.com/blend/taste-match/").is_err());
        assert!(parse_link("spotify:user:").is_err());
        assert!(parse_link("spotify:collection:tracks").is_err());
    }

    fn gid(byte: u8) -> Vec<u8> {
        vec![byte; 16]
    }

    #[test]
    fn maps_every_artist_and_the_explicit_flag() {
        let track = metadata::Track {
            gid: Some(gid(1)),
            name: Some("Song".into()),
            album: Some(metadata::Album {
                gid: Some(gid(2)),
                name: Some("Record".into()),
                ..Default::default()
            }),
            artist: vec![
                metadata::Artist {
                    gid: Some(gid(3)),
                    name: Some("First".into()),
                    ..Default::default()
                },
                metadata::Artist {
                    gid: Some(gid(4)),
                    name: Some("Second".into()),
                    ..Default::default()
                },
            ],
            duration: Some(1000),
            explicit: Some(true),
            ..Default::default()
        };
        let uri = uri_from_gid("track", &gid(1)).unwrap();
        let mapped = decode_track(uri.clone(), &track.encode_to_vec()).unwrap();
        assert_eq!(mapped.uri, uri);
        assert_eq!(mapped.artist, "First, Second");
        assert_eq!(mapped.album, "Record");
        assert!(mapped.explicit);
        assert_eq!(mapped.artist_uri, uri_from_gid("artist", &gid(3)));
        assert_eq!(mapped.album_uri, uri_from_gid("album", &gid(2)));
        assert_eq!(
            mapped.artists,
            [
                Link {
                    uri: uri_from_gid("artist", &gid(3)).unwrap(),
                    name: "First".into(),
                },
                Link {
                    uri: uri_from_gid("artist", &gid(4)).unwrap(),
                    name: "Second".into(),
                },
            ]
        );
        assert_eq!((mapped.uid, mapped.added_at), (None, None));

        let clean = metadata::Track {
            explicit: None,
            ..track
        };
        assert!(!decode_track(uri, &clean.encode_to_vec()).unwrap().explicit);
    }

    #[test]
    fn maps_episodes_to_their_show() {
        let episode = metadata::Episode {
            name: Some("Episode".into()),
            duration: Some(60_000),
            explicit: Some(true),
            show: Some(metadata::Show {
                gid: Some(gid(5)),
                name: Some("Show".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let uri = uri_from_gid("episode", &gid(6)).unwrap();
        let mapped = decode_track(uri, &episode.encode_to_vec()).unwrap();
        assert_eq!(mapped.artist, "Show");
        assert!(mapped.explicit);
        assert_eq!(
            mapped.artists,
            [Link {
                uri: uri_from_gid("show", &gid(5)).unwrap(),
                name: "Show".into(),
            }]
        );
    }

    #[test]
    fn keeps_uids_and_metadata_from_context_pages() {
        let response: ContextResponse = serde_json::from_value(json!({
            "uri": "spotify:playlist:x",
            "metadata": {
                "format_list_type": "blend",
                "context_owner": "spotify:user:someone",
                "count": 3,
                "nested": {"ignored": true}
            },
            "pages": [
                {"tracks": [
                    {"uri": "spotify:track:a", "uid": "65346664",
                     "metadata": {"added_at": "1625150017000", "added_by_username": "someone"}},
                    {"uri": "spotify:delimiter", "uid": "delimiter0"},
                    {"uri": "spotify:track:a", "uid": "77777777"},
                    {"uri": "spotify:episode:b", "uid": "", "metadata": {"added_at": "soon"}},
                    {"uri": "spotify:track:c"}
                ]},
                {"page_url": "hm://context-resolve/v1/page/2"}
            ]
        }))
        .unwrap();
        assert_eq!(
            response
                .metadata
                .get("format_list_type")
                .map(String::as_str),
            Some("blend")
        );
        assert_eq!(
            response.metadata.get("count").map(String::as_str),
            Some("3")
        );
        assert!(!response.metadata.contains_key("nested"));

        let (mut seen, mut items, mut pending) = (HashSet::new(), Vec::new(), Vec::new());
        for page in response.pages {
            take_page(page, &mut seen, &mut items, &mut pending);
        }
        assert_eq!(pending, ["hm://context-resolve/v1/page/2"]);
        let uris: Vec<&str> = items.iter().map(|item| item.uri.as_str()).collect();
        assert_eq!(
            uris,
            ["spotify:track:a", "spotify:episode:b", "spotify:track:c"]
        );
        assert_eq!(items[0].uid.as_deref(), Some("65346664"));
        assert_eq!(items[0].added_at(), Some(1_625_150_017_000));
        assert_eq!(
            items[0]
                .metadata
                .get("added_by_username")
                .map(String::as_str),
            Some("someone")
        );
        assert_eq!((items[1].uid.as_deref(), items[1].added_at()), (None, None));
        assert!(items[2].metadata.is_empty());
    }

    #[test]
    fn tolerates_raw_bytes_in_context_metadata() {
        let mut body = br#"{"metadata":{"context_description":"Show","image_url":""#.to_vec();
        body.extend([0xff, 0xfe, b'g', 0x80]);
        body.extend(br#""},"pages":[{"tracks":[{"uri":"spotify:episode:a"}]}]}"#);
        let response: ContextResponse = parse_lossy(&body).unwrap();
        assert_eq!(
            response
                .metadata
                .get("context_description")
                .map(String::as_str),
            Some("Show")
        );
        assert_eq!(response.pages[0].tracks[0].uri, "spotify:episode:a");
    }

    #[test]
    fn reads_playlist_capabilities() {
        let capabilities = Capabilities::from_playlist(&json!({
            "canView": true, "canAdministratePermissions": false, "canEditMetadata": true,
            "canEditItems": true, "listAttributeCapabilities": {}, "canPublishToProfile": true
        }));
        assert_eq!(
            capabilities,
            Capabilities {
                view: true,
                edit_items: true,
                edit_metadata: true,
                administrate: false,
                publish: true,
                delete: false,
                cancel_membership: false,
            }
        );
        assert_eq!(
            Capabilities::from_playlist(&Value::Null),
            Capabilities::default()
        );
    }

    #[test]
    fn reads_cache_files_written_before_the_new_fields() {
        let track: Track = serde_json::from_value(json!({
            "uri": "spotify:track:a", "title": "Song", "artist": "First", "album": "Record",
            "duration_ms": 1000, "cover": null, "artist_uri": null, "album_uri": null
        }))
        .unwrap();
        assert_eq!(
            track,
            Track {
                uri: "spotify:track:a".into(),
                title: "Song".into(),
                artist: "First".into(),
                album: "Record".into(),
                duration_ms: 1000,
                ..Track::default()
            }
        );
        let playlist: Playlist = serde_json::from_value(json!({
            "uri": "spotify:playlist:x", "name": "Mix", "description": "", "cover": null, "length": 2
        }))
        .unwrap();
        assert_eq!((playlist.format, playlist.owner), (None, None));
    }
}
