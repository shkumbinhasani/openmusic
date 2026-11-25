use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail};
use prost::Message;
use reqwest::Method;
use serde_json::{Value, json};

use crate::{
    account::{IdentityProfile, Wrapped, identity_path},
    api::{Api, Body},
    catalog::{self, Link, Track},
    entity,
    pathfinder::{self, Operation},
};

pub const IS_FOLLOWING_USERS: Operation = Operation::query(
    "isFollowingUsers",
    "c00e0cb6c7766e7230fc256cf4fe07aec63b53d1160a323940fce7b664e95596",
);
pub const FOLLOW_USERS: Operation = Operation::mutation(
    "followUsers",
    "c00e0cb6c7766e7230fc256cf4fe07aec63b53d1160a323940fce7b664e95596",
);
pub const UNFOLLOW_USERS: Operation = Operation::mutation(
    "unfollowUsers",
    "c00e0cb6c7766e7230fc256cf4fe07aec63b53d1160a323940fce7b664e95596",
);
pub const USER_TOP_CONTENT: Operation = Operation::query(
    "userTopContent",
    "61b7353d3625780022dbc63d459eb64cd609e7e5e2a820f9160d65030efe2e15",
);

pub const PROFILE_PREVIEW: usize = 10;
pub const PLAYLIST_PAGE: usize = 200;
pub const RECENT_ARTISTS: usize = 50;
pub const TOP_PREVIEW_ARTISTS: usize = 10;
pub const TOP_PREVIEW_TRACKS: usize = 4;
pub const TOP_ALL: usize = 50;
pub const PRIVACY_READ: &str = "/profile-privacy/v2/read-settings";
pub const PRIVACY_STORE: &str = "/profile-privacy/v2/store-settings";
pub const BUDDYLIST: &str = "/presence-view/v1/buddylist";
pub const PLAYING_WINDOW_MS: i64 = 15 * 60 * 1000;
const IMAGE_SIZE: u32 = 300;
const PROTOBUF: &str = "application/x-protobuf";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RefKind {
    User,
    Artist,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProfileRef {
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
    pub followers: u64,
    pub is_following: bool,
    pub is_followed: bool,
}

impl ProfileRef {
    pub fn kind(&self) -> RefKind {
        if username(&self.uri).is_some() {
            RefKind::User
        } else if self.uri.starts_with("spotify:artist:") {
            RefKind::Artist
        } else {
            RefKind::Other
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistRef {
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
    pub owner: String,
    pub owner_uri: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Profile {
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
    pub color: Option<u32>,
    pub followers: u64,
    pub following: u64,
    pub is_current_user: bool,
    pub is_following: bool,
    pub allow_follows: bool,
    pub show_follows: bool,
    pub name_locked: bool,
    pub playlists: Vec<PlaylistRef>,
    pub total_playlists: u64,
    pub recent_artists: Vec<ProfileRef>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TopContent {
    pub artists: Vec<ProfileRef>,
    pub artists_total: u64,
    pub tracks: Vec<Track>,
    pub tracks_total: u64,
}

pub fn username(uri: &str) -> Option<String> {
    let name = uri.strip_prefix("spotify:user:")?;
    if name.is_empty() || name.contains(':') {
        return None;
    }
    Some(decode(name))
}

pub fn user_uri(username: &str) -> String {
    format!("spotify:user:{}", segment(username))
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn segment(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

pub fn profile_path(uri: &str, tail: &str) -> Result<String> {
    let Some(name) = username(uri) else {
        bail!("This isn’t a Spotify profile.");
    };
    Ok(format!(
        "/user-profile-view/v3/profile/{}{tail}",
        segment(&name)
    ))
}

pub fn image(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    if text.starts_with("https://") {
        return Some(text.to_owned());
    }
    if let Some(id) = text.strip_prefix("spotify:image:") {
        return (!id.is_empty()).then(|| catalog::image_url(id));
    }
    let tiles = text.strip_prefix("spotify:mosaic:")?;
    let first = tiles.split(':').next().filter(|id| !id.is_empty())?;
    Some(catalog::image_url(first))
}

fn string(value: &Value) -> String {
    value.as_str().map(str::trim).unwrap_or_default().to_owned()
}

fn count(value: &Value) -> u64 {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or_default()
}

fn flag(value: &Value) -> bool {
    value.as_bool().unwrap_or_default()
}

pub fn parse_ref(value: &Value) -> Option<ProfileRef> {
    let uri = string(&value["uri"]);
    if uri.is_empty() {
        return None;
    }
    let name = match string(&value["name"]) {
        name if !name.is_empty() => name,
        _ => username(&uri)?,
    };
    Some(ProfileRef {
        uri,
        name,
        image: image(&value["image_url"]),
        followers: count(&value["followers_count"]),
        is_following: flag(&value["is_following"]),
        is_followed: flag(&value["is_followed"]),
    })
}

pub fn parse_playlist(value: &Value) -> Option<PlaylistRef> {
    let uri = string(&value["uri"]);
    let name = string(&value["name"]);
    if uri.is_empty() || name.is_empty() {
        return None;
    }
    let owner_uri = Some(string(&value["owner_uri"])).filter(|uri| !uri.is_empty());
    Some(PlaylistRef {
        uri,
        name,
        image: image(&value["image_url"]),
        owner: string(&value["owner_name"]),
        owner_uri,
    })
}

pub fn parse_refs(reply: &Value, key: &str) -> Vec<ProfileRef> {
    reply[key]
        .as_array()
        .map(|items| items.iter().filter_map(parse_ref).collect())
        .unwrap_or_default()
}

pub fn parse_playlists(reply: &Value) -> (Vec<PlaylistRef>, u64) {
    let playlists: Vec<PlaylistRef> = reply["public_playlists"]
        .as_array()
        .map(|items| items.iter().filter_map(parse_playlist).collect())
        .unwrap_or_default();
    let total = count(&reply["total_public_playlists_count"]).max(playlists.len() as u64);
    (playlists, total)
}

pub fn parse_profile(reply: &Value) -> Option<Profile> {
    let uri = string(&reply["uri"]);
    let fallback = username(&uri)?;
    let name = Some(string(&reply["name"]))
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback);
    let (playlists, total_playlists) = parse_playlists(reply);
    Some(Profile {
        name,
        image: image(&reply["image_url"]),
        color: reply["color"]
            .as_u64()
            .and_then(|color| u32::try_from(color & 0xff_ffff).ok()),
        followers: count(&reply["followers_count"]),
        following: count(&reply["following_count"]),
        is_current_user: flag(&reply["is_current_user"]),
        is_following: flag(&reply["is_following"]),
        allow_follows: flag(&reply["allow_follows"]),
        show_follows: flag(&reply["show_follows"]),
        name_locked: flag(&reply["edit_name_disabled"]) || flag(&reply["edit_profile_disabled"]),
        playlists,
        total_playlists,
        recent_artists: parse_refs(reply, "recently_played_artists"),
        uri,
    })
}

pub fn top_variables(artists: usize, tracks: usize) -> Value {
    let input = |limit: usize| json!({"offset": 0, "limit": limit.max(1), "sortBy": "AFFINITY", "timeRange": "SHORT_TERM"});
    json!({
        "includeTopArtists": artists > 0,
        "topArtistsInput": input(artists),
        "includeTopTracks": tracks > 0,
        "topTracksInput": input(tracks),
    })
}

pub fn parse_top(data: &Value) -> TopContent {
    let profile = &data["me"]["profile"];
    let artists_page = &profile["topArtists"];
    let tracks_page = &profile["topTracks"];
    let items = |page: &Value, typename: &str| -> Vec<Value> {
        if page["__typename"].as_str() != Some(typename) {
            return Vec::new();
        }
        page["items"]
            .as_array()
            .map(|items| items.iter().map(|item| item["data"].clone()).collect())
            .unwrap_or_default()
    };
    let artists: Vec<ProfileRef> = items(artists_page, "ArtistPageV2")
        .iter()
        .filter(|data| data["__typename"].as_str() == Some("Artist"))
        .filter_map(|data| {
            let uri = string(&data["uri"]);
            let name = string(&data["profile"]["name"]);
            (!uri.is_empty() && !name.is_empty()).then(|| ProfileRef {
                uri,
                name,
                image: entity::pick_image(&data["visuals"]["avatarImage"]["sources"], IMAGE_SIZE),
                ..ProfileRef::default()
            })
        })
        .collect();
    let tracks: Vec<Track> = items(tracks_page, "TrackPageV2")
        .iter()
        .filter(|data| data["__typename"].as_str() == Some("Track"))
        .filter(|data| data["albumOfTrack"].is_object())
        .filter_map(entity::track)
        .collect();
    TopContent {
        artists_total: count(&artists_page["totalCount"]).max(artists.len() as u64),
        tracks_total: count(&tracks_page["totalCount"]).max(tracks.len() as u64),
        artists,
        tracks,
    }
}

pub fn follow_request(uris: &[String], follow: bool) -> (Operation, Value) {
    let usernames: Vec<String> = uris.iter().filter_map(|uri| username(uri)).collect();
    let operation = if follow { FOLLOW_USERS } else { UNFOLLOW_USERS };
    (operation, json!({ "usernames": usernames }))
}

pub fn parse_follow_reply(
    data: &Value,
    uris: &[String],
    follow: bool,
) -> Vec<(String, Option<bool>)> {
    let (field, typename) = if follow {
        ("followUsers", "FollowUserResult")
    } else {
        ("unfollowUsers", "UnfollowUserResult")
    };
    let done: HashMap<String, bool> = data[field]["responses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|response| response["__typename"].as_str() == Some(typename))
        .filter_map(|response| {
            let name = response["username"].as_str()?;
            Some((name.to_owned(), flag(&response["result"])))
        })
        .collect();
    uris.iter()
        .map(|uri| {
            let result = username(uri).and_then(|name| done.get(&name).copied());
            (
                uri.clone(),
                result.map(|done| if done { follow } else { !follow }),
            )
        })
        .collect()
}

pub fn parse_following(data: &Value, uris: &[String]) -> Vec<(String, bool)> {
    let users = data["users"].as_array().cloned().unwrap_or_default();
    uris.iter()
        .enumerate()
        .map(|(index, uri)| {
            let found = users
                .iter()
                .find(|user| user["uri"].as_str() == Some(uri.as_str()))
                .or_else(|| users.get(index));
            let following = found.is_some_and(|user| {
                user["__typename"].as_str() == Some("User") && flag(&user["following"])
            });
            (uri.clone(), following)
        })
        .collect()
}

pub async fn profile(api: &Api, uri: &str) -> Result<Profile> {
    let path = profile_path(
        uri,
        &format!(
            "?playlist_limit={PROFILE_PREVIEW}&artist_limit={PROFILE_PREVIEW}&episode_limit={PROFILE_PREVIEW}&market=from_token"
        ),
    )?;
    let reply: Value = api.get_json(&path).await?;
    match parse_profile(&reply) {
        Some(profile) => Ok(profile),
        None => bail!("Spotify sent an unreadable profile."),
    }
}

pub async fn followers(api: &Api, uri: &str) -> Result<Vec<ProfileRef>> {
    let reply: Value = api.get_json(&profile_path(uri, "/followers")?).await?;
    Ok(parse_refs(&reply, "profiles"))
}

pub async fn following(api: &Api, uri: &str) -> Result<Vec<ProfileRef>> {
    let reply: Value = api.get_json(&profile_path(uri, "/following")?).await?;
    Ok(parse_refs(&reply, "profiles"))
}

pub async fn public_playlists(
    api: &Api,
    uri: &str,
    offset: usize,
    limit: usize,
) -> Result<(Vec<PlaylistRef>, u64)> {
    let path = profile_path(uri, &format!("/playlists?offset={offset}&limit={limit}"))?;
    let reply: Value = api.get_json(&path).await?;
    let (playlists, total) = parse_playlists(&reply);
    let seen = (offset + playlists.len()) as u64;
    Ok((playlists, total.max(seen)))
}

pub async fn recent_artists(api: &Api, uri: &str, limit: usize) -> Result<Vec<ProfileRef>> {
    let reply: Value = api
        .get_json(&profile_path(uri, &format!("/artists?limit={limit}"))?)
        .await?;
    Ok(parse_refs(&reply, "artists"))
}

pub async fn top_content(api: &Api, artists: usize, tracks: usize) -> Result<TopContent> {
    let data = pathfinder::query(api, USER_TOP_CONTENT, top_variables(artists, tracks)).await?;
    Ok(parse_top(&data))
}

pub async fn is_following(api: &Api, uris: &[String]) -> Result<Vec<(String, bool)>> {
    let uris: Vec<String> = uris
        .iter()
        .filter(|uri| username(uri).is_some())
        .cloned()
        .collect();
    if uris.is_empty() {
        return Ok(Vec::new());
    }
    let data = pathfinder::query(api, IS_FOLLOWING_USERS, json!({ "uris": uris })).await?;
    Ok(parse_following(&data, &uris))
}

pub async fn set_following(
    api: &Api,
    uris: &[String],
    follow: bool,
) -> Result<Vec<(String, Option<bool>)>> {
    let (operation, variables) = follow_request(uris, follow);
    if variables["usernames"]
        .as_array()
        .is_none_or(|names| names.is_empty())
    {
        bail!("Only Spotify profiles can be followed here.");
    }
    let data = pathfinder::mutate(api, operation, variables).await?;
    Ok(parse_follow_reply(&data, uris, follow))
}

#[derive(Clone, PartialEq, Message)]
pub struct Contains {
    #[prost(bool, tag = "1")]
    pub contains_result: bool,
}

pub fn blocking_path(me: &str, target: &str) -> Result<String> {
    let Some(target) = username(target) else {
        bail!("Only Spotify profiles can be blocked.");
    };
    if me.is_empty() || me == target {
        bail!("You can’t block yourself.");
    }
    Ok(format!(
        "/socialgraph/v4/{}/is-blocking/{}",
        segment(me),
        segment(&target)
    ))
}

pub fn parse_blocking(bytes: &[u8]) -> Result<bool> {
    let reply = Contains::decode(bytes).context("Unreadable block state from Spotify.")?;
    Ok(reply.contains_result)
}

pub async fn is_blocking(api: &Api, me: &str, target: &str) -> Result<bool> {
    let bytes = api
        .request(
            Method::GET,
            &blocking_path(me, target)?,
            Body::Empty,
            PROTOBUF,
        )
        .await?;
    parse_blocking(&bytes)
}

pub async fn set_blocking(api: &Api, me: &str, target: &str, block: bool) -> Result<()> {
    let method = if block { Method::POST } else { Method::DELETE };
    api.request(method, &blocking_path(me, target)?, Body::Empty, "*/*")
        .await?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Privacy {
    pub allow_follows: bool,
    pub show_follows: bool,
    pub show_playlists: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PrivacyField {
    AllowFollows,
    ShowFollows,
    ShowPlaylists,
}

impl PrivacyField {
    pub fn path(self) -> &'static str {
        match self {
            Self::AllowFollows => "allow_follows",
            Self::ShowFollows => "show_follows",
            Self::ShowPlaylists => "show_published_playlists",
        }
    }

    pub fn get(self, privacy: &Privacy) -> bool {
        match self {
            Self::AllowFollows => privacy.allow_follows,
            Self::ShowFollows => privacy.show_follows,
            Self::ShowPlaylists => privacy.show_playlists,
        }
    }

    pub fn set(self, privacy: &mut Privacy, value: bool) {
        match self {
            Self::AllowFollows => privacy.allow_follows = value,
            Self::ShowFollows => privacy.show_follows = value,
            Self::ShowPlaylists => privacy.show_playlists = value,
        }
    }
}

#[derive(Clone, PartialEq, Message)]
pub struct PrivacyRequest {
    #[prost(string, tag = "1")]
    pub username: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct PrivacySettings {
    #[prost(bool, tag = "3")]
    pub allow_follows: bool,
    #[prost(bool, tag = "4")]
    pub show_follows: bool,
    #[prost(bool, tag = "5")]
    pub show_published_playlists: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct FieldMask {
    #[prost(string, repeated, tag = "1")]
    pub paths: Vec<String>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PrivacyUpdate {
    #[prost(string, tag = "1")]
    pub username: String,
    #[prost(message, optional, tag = "3")]
    pub mask: Option<FieldMask>,
    #[prost(bool, optional, tag = "4")]
    pub allow_follows: Option<bool>,
    #[prost(bool, optional, tag = "5")]
    pub show_follows: Option<bool>,
    #[prost(bool, optional, tag = "6")]
    pub show_published_playlists: Option<bool>,
}

pub fn privacy_request(username: &str) -> Vec<u8> {
    PrivacyRequest {
        username: username.to_owned(),
    }
    .encode_to_vec()
}

pub fn parse_privacy(bytes: &[u8]) -> Result<Privacy> {
    let settings =
        PrivacySettings::decode(bytes).context("Unreadable profile settings from Spotify.")?;
    Ok(Privacy {
        allow_follows: settings.allow_follows,
        show_follows: settings.show_follows,
        show_playlists: settings.show_published_playlists,
    })
}

pub fn privacy_update(username: &str, field: PrivacyField, value: bool) -> Vec<u8> {
    let only = |wanted: PrivacyField| (field == wanted).then_some(value);
    PrivacyUpdate {
        username: username.to_owned(),
        mask: Some(FieldMask {
            paths: vec![field.path().to_owned()],
        }),
        allow_follows: only(PrivacyField::AllowFollows),
        show_follows: only(PrivacyField::ShowFollows),
        show_published_playlists: only(PrivacyField::ShowPlaylists),
    }
    .encode_to_vec()
}

pub async fn privacy(api: &Api, username: &str) -> Result<Privacy> {
    if username.is_empty() {
        bail!("No Spotify username to read settings for.");
    }
    let bytes = api
        .protobuf(Method::POST, PRIVACY_READ, privacy_request(username))
        .await?;
    parse_privacy(&bytes)
}

pub async fn set_privacy(
    api: &Api,
    username: &str,
    field: PrivacyField,
    value: bool,
) -> Result<()> {
    if username.is_empty() {
        bail!("No Spotify username to change settings for.");
    }
    api.protobuf(
        Method::POST,
        PRIVACY_STORE,
        privacy_update(username, field, value),
    )
    .await?;
    Ok(())
}

#[derive(Clone, PartialEq, Message)]
pub struct ProfileUpdate {
    #[prost(message, optional, tag = "1")]
    pub mask: Option<FieldMask>,
    #[prost(message, optional, tag = "2")]
    pub user_profile: Option<IdentityProfile>,
}

pub fn display_name(text: &str) -> Option<String> {
    let name = text.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

pub fn name_update(name: &str) -> Vec<u8> {
    ProfileUpdate {
        mask: Some(FieldMask {
            paths: vec!["name".to_owned()],
        }),
        user_profile: Some(IdentityProfile {
            name: Some(Wrapped {
                value: name.to_owned(),
            }),
            ..IdentityProfile::default()
        }),
    }
    .encode_to_vec()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Friend {
    pub user: ProfileRef,
    pub track: Track,
    pub context_uri: Option<String>,
    pub context_name: Option<String>,
    pub timestamp_ms: i64,
    pub playing: Option<bool>,
}

impl Friend {
    pub fn playing_now(&self, now_ms: i64) -> bool {
        self.playing
            .unwrap_or(now_ms.saturating_sub(self.timestamp_ms) < PLAYING_WINDOW_MS)
    }

    pub fn images(&self) -> impl Iterator<Item = String> + '_ {
        self.user.image.iter().chain(&self.track.cover).cloned()
    }
}

fn web_image(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    let Some(rest) = text.strip_prefix("http://") else {
        return image(value);
    };
    let host = rest.split('/').next().unwrap_or_default();
    let spotify = host == "scdn.co"
        || host.ends_with(".scdn.co")
        || host == "spotifycdn.com"
        || host.ends_with(".spotifycdn.com");
    spotify.then(|| format!("https://{rest}"))
}

fn named_link(value: &Value) -> Option<Link> {
    let uri = string(&value["uri"]);
    let name = string(&value["name"]);
    (!uri.is_empty() && !name.is_empty()).then_some(Link { uri, name })
}

fn timestamp(value: &Value) -> i64 {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
        .unwrap_or_default()
}

pub fn parse_friend(value: &Value) -> Option<Friend> {
    let person = &value["user"];
    let user_uri = string(&person["uri"]);
    let fallback = username(&user_uri)?;
    let user = ProfileRef {
        name: Some(string(&person["name"]))
            .filter(|name| !name.is_empty())
            .unwrap_or(fallback),
        image: web_image(&person["imageUrl"]).or_else(|| web_image(&person["image_url"])),
        uri: user_uri,
        ..ProfileRef::default()
    };
    let song = &value["track"];
    let uri = string(&song["uri"]);
    if uri.is_empty() {
        return None;
    }
    let artist = named_link(&song["artist"]);
    let album = named_link(&song["album"]);
    let context = &song["context"];
    let track = Track {
        title: Some(string(&song["name"]))
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| uri.clone()),
        artist: artist
            .as_ref()
            .map(|artist| artist.name.clone())
            .unwrap_or_default(),
        artist_uri: artist.as_ref().map(|artist| artist.uri.clone()),
        album: album
            .as_ref()
            .map(|album| album.name.clone())
            .unwrap_or_default(),
        album_uri: album.map(|album| album.uri),
        cover: web_image(&song["imageUrl"]).or_else(|| web_image(&song["image_url"])),
        artists: artist.into_iter().collect(),
        uri,
        ..Track::default()
    };
    let context_uri = Some(string(&context["uri"])).filter(|uri| !uri.is_empty());
    let context_name = Some(string(&context["name"])).filter(|name| !name.is_empty());
    Some(Friend {
        user,
        track,
        context_name: context_uri.as_ref().and(context_name),
        context_uri,
        timestamp_ms: timestamp(&value["timestamp"]),
        playing: value["isPlaying"]
            .as_bool()
            .or_else(|| value["is_playing"].as_bool()),
    })
}

pub fn parse_friends(reply: &Value) -> Vec<Friend> {
    let mut friends: Vec<Friend> = reply["friends"]
        .as_array()
        .map(|items| items.iter().filter_map(parse_friend).collect())
        .unwrap_or_default();
    friends.sort_by_key(|friend| std::cmp::Reverse(friend.timestamp_ms));
    let mut seen = HashSet::new();
    friends.retain(|friend| seen.insert(friend.user.uri.clone()));
    friends
}

pub async fn friends(api: &Api) -> Result<Vec<Friend>> {
    let reply: Value = api.get_json(BUDDYLIST).await?;
    Ok(parse_friends(&reply))
}

pub async fn set_display_name(api: &Api, username: &str, name: &str) -> Result<()> {
    let Some(name) = display_name(name) else {
        bail!("Display name is required.");
    };
    if username.is_empty() {
        bail!("No Spotify username to rename.");
    }
    api.protobuf(Method::POST, &identity_path(username), name_update(&name))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own_profile() -> Value {
        json!({
            "uri": "spotify:user:someone",
            "name": "Someone Example",
            "image_url": "https://i.scdn.co/image/ab6775700000ee85aaaa",
            "followers_count": 1,
            "following_count": 20,
            "public_playlists": [
                {"uri": "spotify:playlist:0000000000000000000001", "name": "Mix One",
                 "image_url": "spotify:mosaic:ab67616d0000b273aaaa:ab67616d0000b273bbbb",
                 "owner_name": "Someone Example", "owner_uri": "spotify:user:someone",
                 "is_following": true},
                {"uri": "spotify:playlist:0000000000000000000002", "name": "Mix Two",
                 "image_url": "spotify:image:ab67706c0000da84cccc",
                 "owner_name": "Someone Example", "owner_uri": "spotify:user:someone"},
                {"uri": "spotify:playlist:0000000000000000000003", "name": ""}
            ],
            "total_public_playlists_count": 24,
            "is_current_user": true,
            "has_spotify_name": true,
            "has_spotify_image": true,
            "color": 1234567,
            "allow_follows": true,
            "show_follows": true,
            "account_id": "abcdefghij"
        })
    }

    #[test]
    fn reads_your_own_profile() {
        let profile = parse_profile(&own_profile()).unwrap();
        assert_eq!(profile.uri, "spotify:user:someone");
        assert_eq!(profile.name, "Someone Example");
        assert_eq!(
            profile.image.as_deref(),
            Some("https://i.scdn.co/image/ab6775700000ee85aaaa")
        );
        assert_eq!(profile.color, Some(1234567));
        assert_eq!((profile.followers, profile.following), (1, 20));
        assert!(profile.is_current_user);
        assert!(!profile.is_following);
        assert!(profile.allow_follows && profile.show_follows);
        assert_eq!(profile.total_playlists, 24);
        assert_eq!(profile.playlists.len(), 2);
        assert_eq!(
            profile.playlists[0].image.as_deref(),
            Some("https://i.scdn.co/image/ab67616d0000b273aaaa")
        );
        assert_eq!(
            profile.playlists[1].image.as_deref(),
            Some("https://i.scdn.co/image/ab67706c0000da84cccc")
        );
        assert_eq!(
            profile.playlists[0].owner_uri.as_deref(),
            Some("spotify:user:someone")
        );
        assert!(profile.recent_artists.is_empty());
    }

    #[test]
    fn reads_someone_elses_profile_with_absent_fields() {
        let reply = json!({
            "uri": "spotify:user:friend",
            "name": "A Friend",
            "followers_count": 2,
            "following_count": 7,
            "is_following": true,
            "recently_played_artists": [
                {"uri": "spotify:artist:0000000000000000000001", "name": "Band",
                 "image_url": "https://i.scdn.co/image/band", "followers_count": 1000}
            ],
            "color": 7654321
        });
        let profile = parse_profile(&reply).unwrap();
        assert!(!profile.is_current_user);
        assert!(profile.is_following);
        assert!(!profile.allow_follows);
        assert!(!profile.show_follows);
        assert_eq!(profile.image, None);
        assert!(profile.playlists.is_empty());
        assert_eq!(profile.total_playlists, 0);
        assert_eq!(profile.recent_artists.len(), 1);
        assert_eq!(profile.recent_artists[0].followers, 1000);
        assert_eq!(profile.recent_artists[0].kind(), RefKind::Artist);

        let bare = parse_profile(&json!({"uri": "spotify:user:quiet"})).unwrap();
        assert_eq!(bare.name, "quiet");
        assert_eq!(bare.followers, 0);
        assert_eq!(parse_profile(&json!({"name": "No uri"})), None);
        assert_eq!(parse_profile(&json!({"uri": "spotify:artist:x"})), None);
    }

    #[test]
    fn reads_mixed_following_lists() {
        let reply = json!({"profiles": [
            {"uri": "spotify:artist:0000000000000000000001", "name": "Band",
             "image_url": "https://i.scdn.co/image/band", "followers_count": 5, "is_following": true},
            {"uri": "spotify:user:friend", "name": "A Friend", "is_following": true,
             "is_followed": true, "color": 1, "account_id": "x"},
            {"uri": "spotify:user:silent"},
            {"name": "No uri"}
        ]});
        let people = parse_refs(&reply, "profiles");
        assert_eq!(people.len(), 3);
        assert_eq!(people[0].kind(), RefKind::Artist);
        assert_eq!(people[1].kind(), RefKind::User);
        assert!(people[1].is_followed);
        assert_eq!(people[2].name, "silent");
        assert!(!people[2].is_following);
        assert!(parse_refs(&json!({}), "artists").is_empty());
    }

    #[test]
    fn reads_public_playlist_pages() {
        let reply = json!({
            "public_playlists": [
                {"uri": "spotify:playlist:0000000000000000000001", "name": "Mix One",
                 "owner_name": "Someone Example"}
            ],
            "total_public_playlists_count": 24
        });
        let (playlists, total) = parse_playlists(&reply);
        assert_eq!(playlists.len(), 1);
        assert_eq!(playlists[0].owner, "Someone Example");
        assert_eq!(playlists[0].owner_uri, None);
        assert_eq!(total, 24);
        assert_eq!(parse_playlists(&json!({})), (Vec::new(), 0));
    }

    #[test]
    fn maps_profile_images() {
        assert_eq!(
            image(&json!("spotify:image:ab67")),
            Some("https://i.scdn.co/image/ab67".into())
        );
        assert_eq!(
            image(&json!("spotify:mosaic:aa:bb:cc:dd")),
            Some("https://i.scdn.co/image/aa".into())
        );
        assert_eq!(
            image(&json!("https://mosaic.scdn.co/640/aa")),
            Some("https://mosaic.scdn.co/640/aa".into())
        );
        assert_eq!(image(&json!("")), None);
        assert_eq!(image(&json!("spotify:image:")), None);
        assert_eq!(image(&json!(null)), None);
    }

    #[test]
    fn turns_user_uris_into_paths() {
        assert_eq!(username("spotify:user:someone").as_deref(), Some("someone"));
        assert_eq!(username("spotify:user:a%2Bb").as_deref(), Some("a+b"));
        assert_eq!(username("spotify:user:"), None);
        assert_eq!(username("spotify:user:someone:playlist:x"), None);
        assert_eq!(username("spotify:artist:x"), None);
        assert_eq!(username("spotify:user:bad%zz").as_deref(), Some("bad%zz"));
        assert_eq!(
            profile_path("spotify:user:a%2Fb", "/followers").unwrap(),
            "/user-profile-view/v3/profile/a%2Fb/followers"
        );
        assert_eq!(
            profile_path("spotify:user:someone", "?market=from_token").unwrap(),
            "/user-profile-view/v3/profile/someone?market=from_token"
        );
        assert!(profile_path("spotify:artist:x", "").is_err());
        assert_eq!(user_uri("a b"), "spotify:user:a%20b");
    }

    #[test]
    fn asks_for_top_content() {
        let variables = top_variables(10, 4);
        assert_eq!(variables["includeTopArtists"], true);
        assert_eq!(variables["topArtistsInput"]["limit"], 10);
        assert_eq!(variables["topArtistsInput"]["sortBy"], "AFFINITY");
        assert_eq!(variables["topArtistsInput"]["timeRange"], "SHORT_TERM");
        assert_eq!(variables["topTracksInput"]["limit"], 4);
        assert_eq!(variables["topTracksInput"]["offset"], 0);
        let artists_only = top_variables(50, 0);
        assert_eq!(artists_only["includeTopTracks"], false);
        assert_eq!(artists_only["topTracksInput"]["limit"], 1);
    }

    #[test]
    fn reads_top_artists_and_tracks() {
        let data = json!({"me": {"profile": {
            "topArtists": {"__typename": "ArtistPageV2", "totalCount": 70, "items": [
                {"data": {"__typename": "Artist", "uri": "spotify:artist:0000000000000000000001",
                    "profile": {"name": "Band"},
                    "visuals": {"avatarImage": {"sources": [
                        {"height": 640, "url": "https://i.scdn.co/image/band640", "width": 640},
                        {"height": 320, "url": "https://i.scdn.co/image/band320", "width": 320}
                    ]}}}},
                {"data": {"__typename": "NotFound"}}
            ]},
            "topTracks": {"__typename": "TrackPageV2", "totalCount": 814, "items": [
                {"data": {"__typename": "Track", "uri": "spotify:track:0000000000000000000001",
                    "name": "Song", "duration": {"totalMilliseconds": 163319},
                    "contentRating": {"label": "EXPLICIT"}, "mediaType": "AUDIO",
                    "albumOfTrack": {"uri": "spotify:album:0000000000000000000001", "name": "Record",
                        "coverArt": {"sources": [{"height": 300, "url": "https://i.scdn.co/image/rec", "width": 300}]}},
                    "artists": {"items": [{"uri": "spotify:artist:0000000000000000000001",
                        "profile": {"name": "Band"}}]},
                    "saved": false}},
                {"data": {"__typename": "Track", "uri": "spotify:track:0000000000000000000002",
                    "name": "Orphan", "albumOfTrack": null}}
            ]}
        }}});
        let top = parse_top(&data);
        assert_eq!(top.artists_total, 70);
        assert_eq!(top.artists.len(), 1);
        assert_eq!(top.artists[0].name, "Band");
        assert_eq!(
            top.artists[0].image.as_deref(),
            Some("https://i.scdn.co/image/band320")
        );
        assert_eq!(top.tracks_total, 814);
        assert_eq!(top.tracks.len(), 1);
        let song = &top.tracks[0];
        assert_eq!(song.title, "Song");
        assert_eq!(song.album, "Record");
        assert_eq!(song.duration_ms, 163319);
        assert!(song.explicit);
        assert_eq!(song.artists.len(), 1);

        let hidden = json!({"me": {"profile": {"topArtists": null, "topTracks": {"__typename": "GenericError"}}}});
        assert_eq!(parse_top(&hidden), TopContent::default());
    }

    #[test]
    fn builds_follow_requests() {
        let uris = vec![
            "spotify:user:friend".to_owned(),
            "spotify:user:a%40b".to_owned(),
            "spotify:artist:x".to_owned(),
        ];
        let (operation, variables) = follow_request(&uris, true);
        assert_eq!(operation, FOLLOW_USERS);
        assert_eq!(variables, json!({"usernames": ["friend", "a@b"]}));
        let (operation, _) = follow_request(&uris, false);
        assert_eq!(operation, UNFOLLOW_USERS);
        assert_eq!(operation.kind, pathfinder::Kind::Mutation);
        assert_eq!(IS_FOLLOWING_USERS.kind, pathfinder::Kind::Query);
    }

    #[test]
    fn reads_follow_replies() {
        let uris = vec![
            "spotify:user:friend".to_owned(),
            "spotify:user:closed".to_owned(),
            "spotify:user:missing".to_owned(),
        ];
        let data = json!({"followUsers": {"responses": [
            {"__typename": "FollowUserResult", "username": "friend", "result": true},
            {"__typename": "FollowUserResult", "username": "closed", "result": false}
        ]}});
        assert_eq!(
            parse_follow_reply(&data, &uris, true),
            vec![
                ("spotify:user:friend".to_owned(), Some(true)),
                ("spotify:user:closed".to_owned(), Some(false)),
                ("spotify:user:missing".to_owned(), None),
            ]
        );
        let data = json!({"unfollowUsers": {"responses": [
            {"__typename": "UnfollowUserResult", "username": "friend", "result": true}
        ]}});
        assert_eq!(
            parse_follow_reply(&data, &uris[..1], false),
            vec![("spotify:user:friend".to_owned(), Some(false))]
        );
    }

    #[test]
    fn builds_block_paths() {
        assert_eq!(
            blocking_path("someone", "spotify:user:friend").unwrap(),
            "/socialgraph/v4/someone/is-blocking/friend"
        );
        assert_eq!(
            blocking_path("a b", "spotify:user:c%2Fd").unwrap(),
            "/socialgraph/v4/a%20b/is-blocking/c%2Fd"
        );
        assert!(blocking_path("someone", "spotify:artist:x").is_err());
        assert!(blocking_path("someone", "spotify:user:someone").is_err());
        assert!(blocking_path("", "spotify:user:friend").is_err());
    }

    #[test]
    fn reads_block_states() {
        assert!(!parse_blocking(&[]).unwrap());
        assert!(parse_blocking(&[0x08, 0x01]).unwrap());
        assert!(!parse_blocking(&[0x08, 0x00]).unwrap());
        assert!(parse_blocking(&[0x08, 0x01, 0x12, 0x01, b'x']).unwrap());
        assert!(parse_blocking(&[0x0a]).is_err());
    }

    #[test]
    fn asks_for_privacy_settings() {
        let mut expected = vec![0x0a, 7];
        expected.extend(b"someone");
        assert_eq!(privacy_request("someone"), expected);
        assert_eq!(
            parse_privacy(&[0x18, 0x01, 0x20, 0x01, 0x28, 0x01]).unwrap(),
            Privacy {
                allow_follows: true,
                show_follows: true,
                show_playlists: true,
            }
        );
        assert_eq!(
            parse_privacy(&[0x18, 0x01, 0x28, 0x01]).unwrap(),
            Privacy {
                allow_follows: true,
                show_follows: false,
                show_playlists: true,
            }
        );
        assert_eq!(parse_privacy(&[]).unwrap(), Privacy::default());
        assert!(parse_privacy(&[0x18]).is_err());
    }

    #[test]
    fn writes_one_privacy_field_at_a_time() {
        let mut expected = vec![0x0a, 7];
        expected.extend(b"someone");
        expected.extend([0x1a, 14, 0x0a, 12]);
        expected.extend(b"show_follows");
        expected.extend([0x28, 0x00]);
        assert_eq!(
            privacy_update("someone", PrivacyField::ShowFollows, false),
            expected
        );

        let update = PrivacyUpdate::decode(
            privacy_update("someone", PrivacyField::ShowPlaylists, true).as_slice(),
        )
        .unwrap();
        assert_eq!(update.mask.unwrap().paths, vec!["show_published_playlists"]);
        assert_eq!(update.show_published_playlists, Some(true));
        assert_eq!(update.show_follows, None);
        assert_eq!(update.allow_follows, None);

        let mut privacy = Privacy::default();
        for field in [
            PrivacyField::AllowFollows,
            PrivacyField::ShowFollows,
            PrivacyField::ShowPlaylists,
        ] {
            field.set(&mut privacy, true);
            assert!(field.get(&privacy));
        }
        assert_eq!(PrivacyField::AllowFollows.path(), "allow_follows");
    }

    #[test]
    fn asks_to_change_only_the_name() {
        let mut expected = vec![0x0a, 6, 0x0a, 4];
        expected.extend(b"name");
        expected.extend([0x12, 12, 0x12, 10, 0x0a, 8]);
        expected.extend(b"New Name");
        assert_eq!(name_update("New Name"), expected);

        let update = ProfileUpdate::decode(name_update("Zoë").as_slice()).unwrap();
        let profile = update.user_profile.unwrap();
        assert_eq!(profile.name.unwrap().value, "Zoë");
        assert_eq!(profile.username, None);
        assert!(profile.images.is_empty());

        assert_eq!(display_name("  New  Name \n").as_deref(), Some("New  Name"));
        assert_eq!(display_name(" \t "), None);
    }

    #[test]
    fn reads_whether_the_name_can_change() {
        let mut reply = own_profile();
        assert!(!parse_profile(&reply).unwrap().name_locked);
        reply["edit_name_disabled"] = json!(true);
        assert!(parse_profile(&reply).unwrap().name_locked);
        let mut reply = own_profile();
        reply["edit_profile_disabled"] = json!(true);
        assert!(parse_profile(&reply).unwrap().name_locked);
    }

    fn buddylist() -> Value {
        json!({"friends": [
            {"timestamp": 1_795_470_000_000_i64,
             "user": {"uri": "spotify:user:friend", "name": "A Friend",
                      "imageUrl": "https://i.scdn.co/image/friend"},
             "track": {"uri": "spotify:track:0000000000000000000001", "name": "Song",
                       "imageUrl": "http://i.scdn.co/image/cover",
                       "album": {"uri": "spotify:album:0000000000000000000001", "name": "Record"},
                       "artist": {"uri": "spotify:artist:0000000000000000000001", "name": "Band"},
                       "context": {"uri": "spotify:playlist:0000000000000000000001",
                                   "name": "Mix", "index": 0}}},
            {"timestamp": "1795471000000", "isPlaying": true,
             "user": {"uri": "spotify:user:pal", "imageUrl": "https://example.com/pal.jpg"},
             "track": {"uri": "spotify:episode:0000000000000000000001", "name": "Episode",
                       "album": {"uri": "spotify:show:0000000000000000000001", "name": "Show"},
                       "context": {"uri": "", "name": "Nothing"}}},
            {"timestamp": 1_795_460_000_000_i64,
             "user": {"uri": "spotify:user:friend", "name": "A Friend"},
             "track": {"uri": "spotify:track:0000000000000000000002", "name": "Older"}},
            {"timestamp": 1, "user": {"uri": "spotify:artist:x", "name": "Not a user"},
             "track": {"uri": "spotify:track:0000000000000000000003"}},
            {"timestamp": 1, "user": {"uri": "spotify:user:quiet"}, "track": {}}
        ]})
    }

    #[test]
    fn reads_friend_activity() {
        let friends = parse_friends(&buddylist());
        assert_eq!(friends.len(), 2);

        let pal = &friends[0];
        assert_eq!(pal.user.uri, "spotify:user:pal");
        assert_eq!(pal.user.name, "pal");
        assert_eq!(
            pal.user.image.as_deref(),
            Some("https://example.com/pal.jpg")
        );
        assert_eq!(pal.timestamp_ms, 1_795_471_000_000);
        assert_eq!(pal.playing, Some(true));
        assert_eq!(pal.track.album, "Show");
        assert_eq!(
            pal.track.album_uri.as_deref(),
            Some("spotify:show:0000000000000000000001")
        );
        assert_eq!(pal.track.artist_uri, None);
        assert!(pal.track.artists.is_empty());
        assert_eq!(
            (pal.context_uri.as_deref(), pal.context_name.as_deref()),
            (None, None)
        );

        let friend = &friends[1];
        assert_eq!(friend.user.name, "A Friend");
        assert_eq!(friend.track.title, "Song");
        assert_eq!(friend.track.artist, "Band");
        assert_eq!(friend.track.artists.len(), 1);
        assert_eq!(
            friend.track.cover.as_deref(),
            Some("https://i.scdn.co/image/cover")
        );
        assert_eq!(
            friend.context_uri.as_deref(),
            Some("spotify:playlist:0000000000000000000001")
        );
        assert_eq!(friend.context_name.as_deref(), Some("Mix"));
        assert_eq!(friend.playing, None);
        assert_eq!(friend.images().count(), 2);

        assert!(parse_friends(&json!({"friends": []})).is_empty());
        assert!(parse_friends(&json!({})).is_empty());
    }

    #[test]
    fn decides_who_is_playing_now() {
        let mut friend = Friend {
            timestamp_ms: 1_000_000_000,
            ..Friend::default()
        };
        assert!(friend.playing_now(1_000_000_000 + PLAYING_WINDOW_MS - 1));
        assert!(!friend.playing_now(1_000_000_000 + PLAYING_WINDOW_MS));
        friend.playing = Some(false);
        assert!(!friend.playing_now(1_000_000_001));
        friend.playing = Some(true);
        assert!(friend.playing_now(i64::MAX));
    }

    #[test]
    fn keeps_only_spotify_images_from_plain_http() {
        assert_eq!(
            web_image(&json!("http://i.scdn.co/image/x")).as_deref(),
            Some("https://i.scdn.co/image/x")
        );
        assert_eq!(
            web_image(&json!("http://image-cdn-ak.spotifycdn.com/image/x")).as_deref(),
            Some("https://image-cdn-ak.spotifycdn.com/image/x")
        );
        assert_eq!(web_image(&json!("http://evil.example/scdn.co/x")), None);
        assert_eq!(web_image(&json!("http://notscdn.co/x")), None);
        assert_eq!(
            web_image(&json!("spotify:image:ab67")).as_deref(),
            Some("https://i.scdn.co/image/ab67")
        );
    }

    #[test]
    fn reads_follow_lookups() {
        let uris = vec![
            "spotify:user:friend".to_owned(),
            "spotify:user:stranger".to_owned(),
            "spotify:user:gone".to_owned(),
        ];
        let data = json!({"users": [
            {"__typename": "User", "following": true, "uri": "spotify:user:friend"},
            {"__typename": "User", "following": false, "uri": "spotify:user:stranger"},
            {"__typename": "NotFound", "uri": "spotify:user:gone"}
        ]});
        assert_eq!(
            parse_following(&data, &uris),
            vec![
                ("spotify:user:friend".to_owned(), true),
                ("spotify:user:stranger".to_owned(), false),
                ("spotify:user:gone".to_owned(), false),
            ]
        );
    }
}
