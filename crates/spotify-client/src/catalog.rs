use std::collections::HashSet;

use anyhow::Result;
use prost::Message;
use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;
use spotify_proto::{
    extendedmetadata::{
        BatchedEntityRequest, BatchedExtensionResponse, EntityRequest, ExtensionKind,
        ExtensionQuery,
    },
    metadata::{self, ImageGroup},
};

use crate::api::Api;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Track {
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
    pub cover: Option<String>,
    pub artist_uri: Option<String>,
    pub album_uri: Option<String>,
}

impl Track {
    pub fn placeholder(uri: &str) -> Self {
        Self {
            uri: uri.to_owned(),
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            duration_ms: 0,
            cover: None,
            artist_uri: None,
            album_uri: None,
        }
    }

    pub fn id(&self) -> &str {
        self.uri.rsplit(':').next().unwrap_or_default()
    }

    pub fn is_track(&self) -> bool {
        self.uri.starts_with("spotify:track:")
    }
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

async fn extended(
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

pub fn parse_track_input(input: &str) -> Result<Option<String>> {
    let input = input.trim();
    let id = if let Some(id) = input.strip_prefix("spotify:track:") {
        id.to_owned()
    } else if input.starts_with("spotify:") {
        anyhow::bail!("Paste a Spotify song link. Albums and playlists aren’t supported yet.");
    } else if let Ok(url) = url::Url::parse(input)
        && url.host_str() == Some("open.spotify.com")
    {
        let mut segments = url
            .path_segments()
            .ok_or_else(|| anyhow::anyhow!("Enter a Spotify song link."))?
            .filter(|segment| !segment.starts_with("intl-"));
        if segments.next() != Some("track") {
            anyhow::bail!("Paste a Spotify song link. Albums and playlists aren’t supported yet.");
        }
        segments
            .next()
            .ok_or_else(|| anyhow::anyhow!("The Spotify song link is missing its track ID."))?
            .to_owned()
    } else {
        return Ok(None);
    };
    if id.len() != 22 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        anyhow::bail!("The Spotify track ID is invalid.");
    }
    Ok(Some(format!("spotify:track:{id}")))
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
            if uri.starts_with("spotify:episode:") {
                if let Ok(episode) = metadata::Episode::decode(value.as_slice()) {
                    found.push(Track {
                        title: episode.name.unwrap_or_default(),
                        artist: episode
                            .show
                            .as_ref()
                            .and_then(|show| show.name.clone())
                            .unwrap_or_default(),
                        album: String::new(),
                        duration_ms: episode.duration.unwrap_or_default().max(0) as u32,
                        cover: cover(episode.cover_image.as_ref()),
                        artist_uri: None,
                        album_uri: None,
                        uri,
                    });
                }
            } else if let Ok(track) = metadata::Track::decode(value.as_slice()) {
                found.push(Track {
                    title: track.name.unwrap_or_default(),
                    artist: track
                        .artist
                        .iter()
                        .filter_map(|artist| artist.name.clone())
                        .collect::<Vec<_>>()
                        .join(", "),
                    album: track
                        .album
                        .as_ref()
                        .and_then(|album| album.name.clone())
                        .unwrap_or_default(),
                    duration_ms: track.duration.unwrap_or_default().max(0) as u32,
                    cover: cover(
                        track
                            .album
                            .as_ref()
                            .and_then(|album| album.cover_group.as_ref()),
                    ),
                    artist_uri: track
                        .artist
                        .first()
                        .and_then(|artist| artist.gid.as_deref())
                        .and_then(|gid| uri_from_gid("artist", gid)),
                    album_uri: track
                        .album
                        .as_ref()
                        .and_then(|album| album.gid.as_deref())
                        .and_then(|gid| uri_from_gid("album", gid)),
                    uri,
                });
            }
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
}

fn playlist_cover(attributes: &Value) -> Option<String> {
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

async fn username(api: &Api) -> Result<String> {
    let value: Value = api
        .get_json("/context-resolve/v1/spotify:collection:tracks")
        .await?;
    value["metadata"]["context_owner"]
        .as_str()
        .and_then(|owner| owner.strip_prefix("spotify:user:"))
        .map(str::to_owned)
        .ok_or_else(|| anyhow::anyhow!("Spotify did not report your account."))
}

pub async fn playlists(api: &Api) -> Result<Vec<Playlist>> {
    let user = username(api).await?;
    let value: Value = api
        .get_json(&format!(
            "/playlist/v2/user/{user}/rootlist?decorate=revision,attributes,length,owner&from=0&length=200"
        ))
        .await?;
    let contents = &value["contents"];
    let items = contents["items"].as_array().cloned().unwrap_or_default();
    let meta = contents["metaItems"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(items
        .iter()
        .zip(meta.iter())
        .filter_map(|(item, meta)| {
            let uri = item["uri"].as_str()?;
            if !uri.starts_with("spotify:playlist:") {
                return None;
            }
            let attributes = &meta["attributes"];
            Some(Playlist {
                uri: uri.to_owned(),
                name: attributes["name"].as_str().unwrap_or("Untitled").to_owned(),
                description: strip_tags(attributes["description"].as_str().unwrap_or_default()),
                cover: playlist_cover(attributes),
                length: meta["length"].as_u64().unwrap_or_default() as u32,
            })
        })
        .collect())
}

pub async fn collection(api: &Api, uri: &str) -> Result<Collection> {
    let id = uri.rsplit(':').next().unwrap_or_default();
    if uri.starts_with("spotify:playlist:") {
        let value: Value = api
            .get_json(&format!(
                "/playlist/v2/playlist/{id}?decorate=attributes,length,owner&from=0&length=0"
            ))
            .await?;
        let attributes = &value["attributes"];
        let tracks = context_tracks(api, uri, 150).await?;
        let description = strip_tags(attributes["description"].as_str().unwrap_or_default());
        let count = value["length"].as_u64().unwrap_or(tracks.len() as u64);
        let subtitle = if description.is_empty() {
            format!("{count} songs")
        } else {
            format!("{description} · {count} songs")
        };
        return Ok(Collection {
            uri: uri.to_owned(),
            kind: "PLAYLIST".into(),
            name: attributes["name"].as_str().unwrap_or("Playlist").to_owned(),
            subtitle,
            cover: playlist_cover(attributes),
            artist_uri: None,
            tracks,
        });
    }
    if uri.starts_with("spotify:album:") {
        let data = extended(api, &[(uri.to_owned(), ExtensionKind::AlbumV4)]).await?;
        let album = data
            .into_iter()
            .next()
            .and_then(|(_, value)| metadata::Album::decode(value.as_slice()).ok())
            .ok_or_else(|| anyhow::anyhow!("Spotify has no details for this album."))?;
        let tracks = context_tracks(api, uri, 150).await?;
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
        });
    }
    anyhow::bail!("OpenMusic can only open albums and playlists.")
}

#[derive(Deserialize)]
struct ContextResponse {
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
}

fn page_path(url: &str) -> String {
    url.trim_start_matches("hm://").to_owned()
}

pub async fn context_uris(api: &Api, context: &str, limit: usize) -> Result<Vec<String>> {
    let encoded: String = url::form_urlencoded::byte_serialize(context.as_bytes()).collect();
    let mut response: ContextResponse = api
        .get_json(&format!(
            "/context-resolve/v1/{}",
            encoded.replace("%3A", ":")
        ))
        .await?;
    let mut seen = HashSet::new();
    let mut uris = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    loop {
        for page in response.pages.drain(..) {
            if page.tracks.is_empty() {
                if let Some(url) = page.page_url.or(page.next_page_url) {
                    pending.push(url);
                }
                continue;
            }
            for track in page.tracks {
                if (track.uri.starts_with("spotify:track:")
                    || track.uri.starts_with("spotify:episode:"))
                    && seen.insert(track.uri.clone())
                {
                    uris.push(track.uri);
                }
            }
        }
        if uris.len() >= limit {
            break;
        }
        let Some(next) = pending.pop() else { break };
        let page: ContextPage = api.get_json(&page_path(&next)).await?;
        response.pages.push(page);
    }
    uris.truncate(limit);
    Ok(uris)
}

pub async fn context_tracks(api: &Api, context: &str, limit: usize) -> Result<Vec<Track>> {
    let uris = context_uris(api, context, limit).await?;
    tracks(api, &uris).await
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
