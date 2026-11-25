use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use prost::Message;
use reqwest::Method;
use serde_json::{Value, json};
use spotify_proto::{
    collection::{
        itemdata::{
            ItemData, PlayContextBansItemData, item_data::Data,
            play_context_bans_item_data::ItemProvider,
        },
        proto::v2::{CollectionItem, PageRequest, PageResponse, WriteRequest},
    },
    extendedmetadata::ExtensionKind,
    metadata,
};

use crate::{
    Track,
    api::{Api, Body, HttpStatus},
    catalog::{self, Capabilities, Link, Lyrics},
    collection::{self, IGNORE_IN_RECS, PAGE_LIMIT, PLAY_CONTEXT_BANS},
    dealer::Message as Push,
    entity,
    pathfinder::{self, Operation},
};

pub const TRACK_CREDITS: Operation = Operation::query(
    "queryTrackCreditsGroupedModal",
    "5c109593bca8ea4cb28104e178820ff035418fb4694d34d10c89cc755fe64694",
);

pub const TRACK_ARTISTS: Operation = Operation::query(
    "queryTrackArtists",
    "ee2b038198f5e62c679c3996584d9249bbee55fe69fc212271c56492a022c798",
);

pub const GET_TRACK: Operation = Operation::query(
    "getTrack",
    "e7b1bf4e95cb3d733d48b442f93006e9712fd504e8b88d7a82e6e3bac9306a6b",
);

pub const TRACK_RECOMMENDATIONS: Operation = Operation::query(
    "internalLinkRecommenderTrack",
    "642923b2e705fceb9984697c1620fecd42615fd8b116b8a9faf2786b389d3e4a",
);

pub const RECOMMENDED: usize = 5;

const EXTRACTED_COLOR: &str = "b2d6d99fb6237952dfa6638385f9c6085f1b1fb9d0468753ca6ad98ff45adc6f";

pub const CONTRIBUTORS_LIMIT: u32 = 100;

pub const OTHER_GROUP: &str = "Other";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Credits {
    pub uri: String,
    pub track: String,
    pub groups: Vec<CreditGroup>,
    pub sources: Vec<String>,
}

impl Credits {
    pub fn linked_artists(&self) -> Vec<String> {
        self.groups
            .iter()
            .flat_map(|group| &group.people)
            .filter_map(|person| person.uri.clone())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreditGroup {
    pub name: String,
    pub people: Vec<Contributor>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Contributor {
    pub name: String,
    pub roles: Vec<String>,
    pub uri: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Unavailable;

impl std::fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Credits aren’t available for this song.")
    }
}

impl std::error::Error for Unavailable {}

pub fn credits_variables(uri: &str) -> Value {
    json!({
        "trackUri": uri,
        "contributorsLimit": CONTRIBUTORS_LIMIT,
        "contributorsOffset": 0,
    })
}

pub async fn credits(api: &Api, uri: &str) -> Result<Credits> {
    let data = pathfinder::query(api, TRACK_CREDITS, credits_variables(uri)).await?;
    parse_credits(uri, &data)
}

pub fn parse_credits(uri: &str, data: &Value) -> Result<Credits> {
    let track = &data["trackUnion"];
    if track["__typename"].as_str() != Some("Track") {
        bail!(Unavailable);
    }
    let main: HashSet<String> = track["artists"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artist| artist["uri"].as_str())
        .map(str::to_owned)
        .collect();
    let credits = &track["creditsTrait"];
    let contributors: Vec<Value> = credits["contributors"]["items"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let sources = credits["sources"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|source| source["name"].as_str())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    Ok(Credits {
        uri: uri.to_owned(),
        track: track["name"].as_str().unwrap_or_default().to_owned(),
        groups: group(&contributors, &main),
        sources,
    })
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn contributor_uri(row: &Value) -> Option<String> {
    text(&row["uri"]).or_else(|| text(&row["reference"]["uri"]))
}

fn contributor_url(row: &Value) -> Option<String> {
    text(&row["url"]).or_else(|| text(&row["reference"]["url"]))
}

pub fn group(contributors: &[Value], main: &HashSet<String>) -> Vec<CreditGroup> {
    let mut order: Vec<String> = Vec::new();
    let mut rows: HashMap<String, Vec<&Value>> = HashMap::new();
    for row in contributors {
        let name = row["roleGroup"]["name"]
            .as_str()
            .unwrap_or(OTHER_GROUP)
            .to_owned();
        if !rows.contains_key(&name) {
            order.push(name.clone());
        }
        rows.entry(name).or_default().push(row);
    }
    order
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let people = merge(&rows[&name], index == 0, main);
            CreditGroup { name, people }
        })
        .collect()
}

fn merge(rows: &[&Value], links: bool, main: &HashSet<String>) -> Vec<Contributor> {
    let linkable =
        |uri: &Option<String>| links && uri.as_ref().is_some_and(|uri| main.contains(uri));
    let mut people: Vec<Contributor> = Vec::new();
    for row in rows {
        let Some(name) = text(&row["name"]) else {
            continue;
        };
        let role = text(&row["role"]);
        let uri = contributor_uri(row);
        let url = contributor_url(row);
        if let Some(person) = people.iter_mut().find(|person| person.name == name) {
            let Some(role) = role else {
                continue;
            };
            person.roles.push(role);
            if linkable(&uri) && person.uri.is_none() {
                person.uri = uri;
            }
            if person.uri.is_none() && person.url.is_none() {
                person.url = url;
            }
            continue;
        }
        let uri = linkable(&uri).then_some(uri).flatten();
        let url = if uri.is_some() { None } else { url };
        people.push(Contributor {
            name,
            roles: role.into_iter().collect(),
            uri,
            url,
        });
    }
    people
}

pub fn cover_color_operation(uri: &str) -> Option<Operation> {
    let name = match uri.split(':').nth(1)? {
        "track" => "fetchExtractedColorForTrackEntity",
        "album" => "fetchExtractedColorForAlbumEntity",
        "artist" => "fetchExtractedColorForArtistEntity",
        "playlist" => "fetchExtractedColorForPlaylistEntity",
        "episode" => "fetchExtractedColorForEpisodeEntity",
        "show" => "fetchExtractedColorForPodcastEntity",
        _ => return None,
    };
    Some(Operation::query(name, EXTRACTED_COLOR))
}

pub async fn cover_color(api: &Api, uri: &str) -> Result<Option<u32>> {
    let Some(operation) = cover_color_operation(uri) else {
        return Ok(None);
    };
    let data = pathfinder::query(api, operation, json!({ "uri": uri })).await?;
    Ok(parse_cover_color(&data))
}

pub fn parse_cover_color(data: &Value) -> Option<u32> {
    let image = [
        &data["albumUnion"]["coverArt"],
        &data["artistUnion"]["visuals"]["avatarImage"],
        &data["episodeUnionV2"]["coverArt"],
        &data["playlistV2"]["images"]["items"][0],
        &data["podcastUnionV2"]["coverArt"],
        &data["trackUnion"]["albumOfTrack"]["coverArt"],
    ]
    .into_iter()
    .find(|image| image.is_object())?;
    entity::color(image["extractedColors"]["colorDark"]["hex"].as_str()?)
}

pub fn track_artists_variables(uri: &str) -> Value {
    json!({ "trackUri": uri })
}

pub async fn track_artists(api: &Api, uri: &str) -> Result<Vec<Link>> {
    let data = pathfinder::query(api, TRACK_ARTISTS, track_artists_variables(uri)).await?;
    Ok(parse_track_artists(&data))
}

pub fn parse_track_artists(data: &Value) -> Vec<Link> {
    data["trackUnion"]["artists"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artist| {
            Some(Link {
                uri: text(&artist["uri"])?,
                name: text(&artist["profile"]["name"])?,
            })
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub struct NoRadio;

impl std::fmt::Display for NoRadio {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Radio is not available because personalized recommendations are turned off"
        )
    }
}

impl std::error::Error for NoRadio {}

pub fn radio_path(seed: &str) -> String {
    format!("/inspiredby-mix/v2/seed_to_playlist/{seed}?response-format=json")
}

pub fn parse_radio(reply: &Value) -> Option<String> {
    reply["mediaItems"]
        .as_array()?
        .iter()
        .find_map(|item| text(&item["uri"]))
}

pub async fn album_artist(api: &Api, album: &str) -> Result<Option<String>> {
    let data = catalog::extended(api, &[(album.to_owned(), ExtensionKind::AlbumV4)]).await?;
    Ok(data
        .into_iter()
        .next()
        .and_then(|(_, value)| metadata::Album::decode(value.as_slice()).ok())
        .and_then(|album| {
            let gid = album.artist.first()?.gid.clone()?;
            catalog::uri_from_gid("artist", &gid)
        }))
}

pub async fn radio(api: &Api, seed: &str) -> Result<String> {
    let seed = if seed.starts_with("spotify:album:") {
        album_artist(api, seed).await?.ok_or(NoRadio)?
    } else {
        seed.to_owned()
    };
    match api.get_json::<Value>(&radio_path(&seed)).await {
        Ok(reply) => parse_radio(&reply).ok_or_else(|| NoRadio.into()),
        Err(error) if refused(&error) => Err(NoRadio.into()),
        Err(error) => Err(error),
    }
}

fn refused(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<HttpStatus>()
        .is_some_and(|HttpStatus(status)| {
            status.is_client_error() && status.as_u16() != 401 && status.as_u16() != 429
        })
}

pub fn hide_context(uri: &str) -> Option<String> {
    let parts: Vec<&str> = uri.split(':').collect();
    match parts.as_slice() {
        ["spotify", kind @ ("playlist" | "album"), id] if !id.is_empty() => {
            Some(format!("spotify:{kind}:{id}"))
        }
        ["spotify", "user", _, "playlist", id] if !id.is_empty() => {
            Some(format!("spotify:playlist:{id}"))
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Ban {
    pub track: String,
    pub context: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BanProvider {
    #[default]
    Context,
    Autoplay,
}

impl BanProvider {
    pub fn of(provider: Option<&str>) -> Option<Self> {
        match provider {
            None | Some("" | "context") => Some(Self::Context),
            Some("autoplay") => Some(Self::Autoplay),
            Some(_) => None,
        }
    }

    fn item_provider(self) -> ItemProvider {
        match self {
            Self::Context => ItemProvider::ProviderContext,
            Self::Autoplay => ItemProvider::ProviderAutoplay,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteFormat {
    Protobuf,
    Json,
}

impl WriteFormat {
    pub fn name(self) -> &'static str {
        match self {
            Self::Protobuf => "protobuf",
            Self::Json => "json",
        }
    }
}

pub fn client_update_id() -> String {
    format!("{:016x}", rand::random::<u64>())
}

pub fn bans_page_request(user: &str, token: Option<&str>) -> Vec<u8> {
    PageRequest {
        username: user.to_owned(),
        set: PLAY_CONTEXT_BANS.to_owned(),
        pagination_token: token.unwrap_or_default().to_owned(),
        limit: PAGE_LIMIT as i32,
    }
    .encode_to_vec()
}

pub fn parse_bans_page(bytes: &[u8]) -> Result<(Vec<Ban>, Option<String>)> {
    let page = PageResponse::decode(bytes)?;
    let bans = page
        .items
        .into_iter()
        .filter(|item| !item.is_removed && !item.uri.is_empty())
        .filter_map(|item| {
            Some(Ban {
                context: hide_context(item.context_uri.as_deref()?)?,
                track: item.uri,
            })
        })
        .collect();
    let next = Some(page.next_page_token).filter(|token| !token.is_empty());
    Ok((bans, next))
}

async fn proto_post(api: &Api, path: &str, body: Vec<u8>) -> Result<Vec<u8>> {
    api.request(
        Method::POST,
        path,
        Body::Bytes(body, collection::PROTOBUF),
        collection::PROTOBUF,
    )
    .await
}

pub fn json_bans(items: Vec<collection::Item>) -> Vec<Ban> {
    items
        .into_iter()
        .filter_map(|item| {
            Some(Ban {
                context: hide_context(item.context_uri.as_deref()?)?,
                track: item.uri,
            })
        })
        .collect()
}

pub async fn bans(api: &Api, user: &str) -> Result<Vec<Ban>> {
    match protobuf_bans(api, user).await {
        Ok(bans) => Ok(bans),
        Err(_) => Ok(json_bans(
            collection::all(api, user, PLAY_CONTEXT_BANS).await?,
        )),
    }
}

async fn protobuf_bans(api: &Api, user: &str) -> Result<Vec<Ban>> {
    let mut bans = Vec::new();
    let mut seen = HashSet::new();
    let mut token: Option<String> = None;
    loop {
        let bytes = proto_post(
            api,
            "/collection/v2/paging",
            bans_page_request(user, token.as_deref()),
        )
        .await?;
        let (page, next) = parse_bans_page(&bytes)?;
        let added = page.len();
        bans.extend(page);
        match next {
            Some(next) if added > 0 && seen.insert(next.clone()) => token = Some(next),
            _ => break,
        }
    }
    Ok(bans)
}

pub fn ban_write_request(
    user: &str,
    ban: &Ban,
    provider: BanProvider,
    hidden: bool,
    client_update_id: &str,
) -> Vec<u8> {
    let data = PlayContextBansItemData {
        item_provider: provider.item_provider() as i32,
    };
    WriteRequest {
        username: user.to_owned(),
        set: PLAY_CONTEXT_BANS.to_owned(),
        items: vec![CollectionItem {
            uri: ban.track.clone(),
            added_at: 0,
            is_removed: !hidden,
            context_uri: Some(ban.context.clone()),
            item_data: Some(ItemData {
                save_source: None,
                data: Some(Data::PlayContextBansItemData(data)),
            }),
        }],
        client_update_id: client_update_id.to_owned(),
    }
    .encode_to_vec()
}

pub fn ban_write_json(user: &str, ban: &Ban, hidden: bool) -> Value {
    collection::write_body(
        user,
        PLAY_CONTEXT_BANS,
        &[collection::Write {
            uri: ban.track.clone(),
            context_uri: Some(ban.context.clone()),
            removed: !hidden,
        }],
    )
}

pub async fn set_banned(
    api: &Api,
    user: &str,
    ban: &Ban,
    provider: BanProvider,
    hidden: bool,
) -> Result<WriteFormat> {
    let request = ban_write_request(user, ban, provider, hidden, &client_update_id());
    let protobuf = match proto_post(api, "/collection/v2/write", request).await {
        Ok(_) => return Ok(WriteFormat::Protobuf),
        Err(error) => error,
    };
    let _: Value = api
        .json(
            Method::POST,
            "/collection/v2/write",
            &ban_write_json(user, ban, hidden),
        )
        .await
        .map_err(|json| json.context(format!("protobuf write failed too: {protobuf}")))?;
    Ok(WriteFormat::Json)
}

pub const ARTIST_BAN: &str = "artistban";

fn artist_ban(uri: &str, banned: bool) -> collection::Write {
    collection::Write {
        uri: uri.to_owned(),
        context_uri: None,
        removed: !banned,
    }
}

pub async fn banned_artists(api: &Api, user: &str) -> Result<Vec<String>> {
    Ok(collection::all(api, user, ARTIST_BAN)
        .await?
        .into_iter()
        .map(|item| item.uri)
        .filter(|uri| uri.starts_with("spotify:artist:"))
        .collect())
}

pub async fn set_artist_banned(api: &Api, user: &str, uri: &str, banned: bool) -> Result<()> {
    collection::write(api, user, ARTIST_BAN, &[artist_ban(uri, banned)]).await
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackSet {
    Excluded,
    Banned,
    Hidden,
}

impl FeedbackSet {
    pub fn of(set: &str) -> Option<Self> {
        match set {
            IGNORE_IN_RECS => Some(Self::Excluded),
            ARTIST_BAN => Some(Self::Banned),
            PLAY_CONTEXT_BANS => Some(Self::Hidden),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Excluded => IGNORE_IN_RECS,
            Self::Banned => ARTIST_BAN,
            Self::Hidden => PLAY_CONTEXT_BANS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pushed {
    pub uri: String,
    pub context: Option<String>,
    pub removed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetPush {
    pub set: FeedbackSet,
    pub items: Vec<Pushed>,
    pub unreadable: usize,
}

pub fn set_push(message: &Push) -> Option<SetPush> {
    let mut path = message.uri.strip_prefix("hm://collection/")?.split('/');
    let set = FeedbackSet::of(path.next()?)?;
    path.next().filter(|user| !user.is_empty())?;
    if path.next() != Some("json") {
        return None;
    }
    let decoded;
    let payload = match message.json() {
        Some(payload) => payload,
        None => {
            decoded = serde_json::from_slice::<Value>(&message.bytes()?).ok()?;
            &decoded
        }
    };
    let rows = payload["items"].as_array()?;
    let items: Vec<Pushed> = rows.iter().filter_map(pushed).collect();
    Some(SetPush {
        set,
        unreadable: rows.len() - items.len(),
        items,
    })
}

fn pushed(item: &Value) -> Option<Pushed> {
    let uri = pushed_uri(item["type"].as_str()?, item["identifier"].as_str()?)?;
    let context = ["context_uri", "contextUri", "context"]
        .iter()
        .find_map(|key| item[*key].as_str())
        .and_then(hide_context);
    Some(Pushed {
        uri,
        context,
        removed: item["removed"].as_bool().unwrap_or(false),
    })
}

fn pushed_uri(kind: &str, id: &str) -> Option<String> {
    if id.starts_with("spotify:") {
        return Some(id.to_owned());
    }
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
    {
        return None;
    }
    if id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let gid = (0..16)
            .map(|index| u8::from_str_radix(&id[index * 2..index * 2 + 2], 16).ok())
            .collect::<Option<Vec<u8>>>()?;
        return catalog::uri_from_gid(kind, &gid);
    }
    (id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then(|| format!("spotify:{kind}:{id}"))
}

pub async fn excluded(api: &Api, user: &str) -> Result<Vec<String>> {
    Ok(collection::all(api, user, IGNORE_IN_RECS)
        .await?
        .into_iter()
        .map(|item| item.uri)
        .collect())
}

pub async fn set_excluded(api: &Api, user: &str, uri: &str, excluded: bool) -> Result<()> {
    collection::write(
        api,
        user,
        IGNORE_IN_RECS,
        &[collection::Write {
            uri: uri.to_owned(),
            context_uri: None,
            removed: !excluded,
        }],
    )
    .await
}

pub fn playlist_path(uri: &str) -> Option<String> {
    let playlist = hide_context(uri)?;
    let id = playlist.strip_prefix("spotify:playlist:")?;
    Some(format!(
        "/playlist/v2/playlist/{id}?decorate=revision,length,attributes,timestamp,owner,capabilities&from=0&length=0"
    ))
}

pub async fn playlist_editable(api: &Api, uri: &str) -> Result<bool> {
    let Some(path) = playlist_path(uri) else {
        return Ok(false);
    };
    let value: Value = api.get_json(&path).await?;
    Ok(Capabilities::from_playlist(&value["capabilities"]).edit_items)
}

#[derive(Debug, PartialEq, Eq)]
pub struct TrackMissing;

impl std::fmt::Display for TrackMissing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Couldn’t find that song")
    }
}

impl std::error::Error for TrackMissing {}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReleaseDate {
    pub iso: String,
    pub precision: String,
    pub year: Option<i64>,
}

impl ReleaseDate {
    pub fn long(&self) -> Option<String> {
        let mut parts = self.iso.get(..10).unwrap_or(&self.iso).split('-');
        let year = parts
            .next()
            .and_then(|year| year.parse::<i64>().ok())
            .or(self.year)?;
        let month = parts
            .next()
            .and_then(|month| month.parse::<usize>().ok())
            .and_then(|month| MONTHS.get(month.checked_sub(1)?));
        let day = parts.next().and_then(|day| day.parse::<u32>().ok());
        Some(match (self.precision.as_str(), month, day) {
            ("YEAR", _, _) | (_, None, _) => year.to_string(),
            ("MONTH", Some(month), _) | (_, Some(month), None) => format!("{month} {year}"),
            (_, Some(month), Some(day)) => format!("{month} {day}, {year}"),
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackAlbum {
    pub uri: String,
    pub name: String,
    pub kind: String,
    pub cover: Option<String>,
    pub date: Option<ReleaseDate>,
    pub copyrights: Vec<String>,
    pub playable: bool,
    pub tracks: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackArtist {
    pub uri: String,
    pub name: String,
    pub image: Option<String>,
    pub top_tracks: Vec<Track>,
    pub popular: Vec<entity::Card>,
    pub albums: Vec<entity::Card>,
    pub singles: Vec<entity::Card>,
    pub related: Vec<entity::Card>,
}

#[derive(Clone, Debug, Default)]
pub struct TrackPage {
    pub track: Track,
    pub playable: bool,
    pub playcount: Option<u64>,
    pub album: Option<TrackAlbum>,
    pub artists: Vec<TrackArtist>,
    pub recommended: Vec<Track>,
    pub album_tracks: Vec<Track>,
    pub lyrics: Option<Lyrics>,
    pub credits: Option<Credits>,
}

impl TrackPage {
    pub fn images(&self) -> Vec<String> {
        let tracks = std::iter::once(&self.track)
            .chain(&self.recommended)
            .chain(&self.album_tracks)
            .chain(self.artists.iter().flat_map(|artist| &artist.top_tracks))
            .filter_map(|track| track.cover.clone());
        let cards = self
            .artists
            .iter()
            .flat_map(|artist| {
                artist
                    .popular
                    .iter()
                    .chain(&artist.albums)
                    .chain(&artist.singles)
                    .chain(&artist.related)
            })
            .filter_map(|card| card.image.clone());
        let mut images: Vec<String> = tracks
            .chain(cards)
            .chain(
                self.artists
                    .iter()
                    .filter_map(|artist| artist.image.clone()),
            )
            .chain(self.album.iter().filter_map(|album| album.cover.clone()))
            .collect();
        let mut seen = HashSet::new();
        images.retain(|url| seen.insert(url.clone()));
        images
    }
}

pub fn track_variables(uri: &str) -> Value {
    json!({ "uri": uri })
}

pub fn recommendations_variables(uri: &str) -> Value {
    json!({ "uri": uri, "limit": RECOMMENDED })
}

fn links(items: &Value) -> Vec<Link> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|artist| {
            Some(Link {
                uri: text(&artist["uri"])?,
                name: text(&artist["profile"]["name"])?,
            })
        })
        .collect()
}

fn song(data: &Value) -> Option<Track> {
    let artists = links(&data["artists"]["items"]);
    let album = &data["albumOfTrack"];
    Some(Track {
        uri: text(&data["uri"]).filter(|uri| uri.starts_with("spotify:track:"))?,
        title: text(&data["name"])?,
        artist: artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        album: text(&album["name"]).unwrap_or_default(),
        duration_ms: data["duration"]["totalMilliseconds"]
            .as_u64()
            .unwrap_or_default()
            .min(u64::from(u32::MAX)) as u32,
        cover: entity::pick_image(&album["coverArt"]["sources"], entity::CARD_IMAGE),
        artist_uri: artists.first().map(|artist| artist.uri.clone()),
        album_uri: text(&album["uri"]),
        artists,
        explicit: entity::explicit(data),
        ..Track::default()
    })
}

fn release_kind(kind: &str) -> &'static str {
    match kind {
        "SINGLE" => "Single",
        "EP" => "EP",
        "COMPILATION" => "Compilation",
        _ => "Album",
    }
}

fn release_card(data: &Value) -> Option<entity::Card> {
    let kind = release_kind(data["type"].as_str().unwrap_or_default());
    let subtitle = match data["date"]["year"].as_i64() {
        Some(year) => format!("{year} • {kind}"),
        None => kind.to_owned(),
    };
    Some(entity::Card {
        uri: text(&data["uri"]).filter(|uri| uri.starts_with("spotify:album:"))?,
        kind: entity::Kind::Album,
        title: text(&data["name"])?,
        subtitle,
        image: entity::pick_image(&data["coverArt"]["sources"], entity::CARD_IMAGE),
        format: None,
        color: None,
        reason: None,
        explicit: false,
        playable: entity::playable(data),
    })
}

fn artist_card(data: &Value) -> Option<entity::Card> {
    Some(entity::Card {
        uri: text(&data["uri"]).filter(|uri| uri.starts_with("spotify:artist:"))?,
        kind: entity::Kind::Artist,
        title: text(&data["profile"]["name"])?,
        subtitle: "Artist".into(),
        image: avatar(data),
        format: None,
        color: None,
        reason: None,
        explicit: false,
        playable: true,
    })
}

fn avatar(data: &Value) -> Option<String> {
    entity::pick_image(
        &data["visuals"]["avatarImage"]["sources"],
        entity::CARD_IMAGE,
    )
}

fn items(value: &Value) -> impl Iterator<Item = &Value> {
    value["items"].as_array().into_iter().flatten()
}

fn track_artist(data: &Value) -> Option<TrackArtist> {
    let discography = &data["discography"];
    let first_release = |group: &Value| -> Vec<entity::Card> {
        items(group)
            .filter_map(|release| release_card(&release["releases"]["items"][0]))
            .collect()
    };
    Some(TrackArtist {
        uri: text(&data["uri"]).filter(|uri| uri.starts_with("spotify:artist:"))?,
        name: text(&data["profile"]["name"])?,
        image: avatar(data),
        top_tracks: items(&discography["topTracks"])
            .filter_map(|item| song(&item["track"]))
            .collect(),
        popular: items(&discography["popularReleasesAlbums"])
            .filter_map(release_card)
            .collect(),
        albums: first_release(&discography["albums"]),
        singles: first_release(&discography["singles"]),
        related: items(&data["relatedContent"]["relatedArtists"])
            .filter_map(artist_card)
            .collect(),
    })
}

fn copyright(item: &Value) -> Option<String> {
    let line = text(&item["text"])?;
    let symbol = match item["type"].as_str() {
        Some("C") => "©",
        Some("P") => "℗",
        _ => return Some(line),
    };
    Some(if line.starts_with(symbol) {
        line
    } else {
        format!("{symbol} {line}")
    })
}

fn track_album(data: &Value) -> Option<TrackAlbum> {
    let date = &data["date"];
    Some(TrackAlbum {
        uri: text(&data["uri"]).filter(|uri| uri.starts_with("spotify:album:"))?,
        name: text(&data["name"])?,
        kind: text(&data["type"]).unwrap_or_else(|| "ALBUM".into()),
        cover: entity::pick_image(&data["coverArt"]["sources"], entity::CARD_IMAGE),
        date: text(&date["isoString"]).map(|iso| ReleaseDate {
            iso,
            precision: text(&date["precision"]).unwrap_or_default(),
            year: date["year"].as_i64(),
        }),
        copyrights: items(&data["copyright"]).filter_map(copyright).collect(),
        playable: entity::playable(data),
        tracks: items(&data["tracks"])
            .filter_map(|item| text(&item["track"]["uri"]))
            .filter(|uri| uri.starts_with("spotify:track:"))
            .collect(),
    })
}

pub fn parse_track_page(data: &Value) -> Result<TrackPage> {
    let union = &data["trackUnion"];
    if union["__typename"].as_str() != Some("Track") {
        bail!(TrackMissing);
    }
    let Some(track) = entity::track(union) else {
        bail!(TrackMissing);
    };
    let artists = items(&union["firstArtist"])
        .chain(items(&union["otherArtists"]))
        .filter_map(track_artist)
        .collect();
    Ok(TrackPage {
        track,
        playable: entity::playable(union),
        playcount: union["playcount"]
            .as_str()
            .and_then(|count| count.parse().ok())
            .or_else(|| union["playcount"].as_u64()),
        album: track_album(&union["albumOfTrack"]),
        artists,
        ..TrackPage::default()
    })
}

pub fn parse_recommendations(data: &Value) -> Vec<Track> {
    items(&data["seoRecommendedTrack"])
        .map(|item| &item["data"])
        .filter(|data| data["__typename"].as_str() == Some("Track"))
        .filter_map(song)
        .take(RECOMMENDED)
        .collect()
}

pub async fn track_page(api: &Api, uri: &str) -> Result<TrackPage> {
    let (track, recommended, credits, lyrics) = futures::join!(
        pathfinder::query(api, GET_TRACK, track_variables(uri)),
        pathfinder::query(api, TRACK_RECOMMENDATIONS, recommendations_variables(uri)),
        credits(api, uri),
        catalog::lyrics(api, uri),
    );
    let mut page = parse_track_page(&track?)?;
    page.recommended = recommended
        .map(|data| parse_recommendations(&data))
        .unwrap_or_default();
    page.credits = credits.ok();
    page.lyrics = lyrics
        .ok()
        .flatten()
        .filter(|lyrics| !lyrics.lines.is_empty());
    if let Some(album) = &page.album
        && !album.tracks.is_empty()
    {
        page.album_tracks = catalog::tracks(api, &album.tracks)
            .await
            .unwrap_or_default();
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RADIOHEAD: &str = "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb";
    const HAMMOND: &str = "spotify:artist:34E3csCxpXunPGEkOVVX2g";

    fn person(name: &str, role: &str, group: &str, uri: Option<&str>) -> Value {
        json!({
            "name": name,
            "role": role,
            "roleGroup": {"name": group},
            "uri": uri,
            "url": null,
            "aiAttribution": {"hasAiAttribution": false},
        })
    }

    fn creep() -> Value {
        json!({
            "trackUnion": {
                "__typename": "Track",
                "name": "Creep",
                "artists": {"items": [{"uri": RADIOHEAD}]},
                "creditsTrait": {
                    "contributors": {"items": [
                        person("Radiohead", "Main Artist", "Artist", Some(RADIOHEAD)),
                        person("Albert Hammond", "Composer", "Composition & Lyrics", Some(HAMMOND)),
                        person("Thom Yorke", "Composer", "Composition & Lyrics", None),
                        person("Thom Yorke", "Lyricist", "Composition & Lyrics", None),
                        person("Colin Greenwood", "Bass Guitar", "Performers", None),
                        person("Thom Yorke", "Voice", "Performers", None),
                        person("Sean Slade", "Producer", "Production & Engineering", None),
                        person("Paul Q. Kolderie", "Mixing Engineer", "Production & Engineering", None),
                        person("Radiohead", "Producer", "Production & Engineering", Some(RADIOHEAD)),
                    ]},
                    "sources": {"items": [{"name": "XL Recordings"}]},
                },
            }
        })
    }

    #[test]
    fn asks_for_the_first_hundred_contributors() {
        assert_eq!(
            credits_variables("spotify:track:70LcF31zb1H0PyJoS1Sx1r"),
            json!({
                "trackUri": "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
                "contributorsLimit": 100,
                "contributorsOffset": 0,
            })
        );
    }

    #[test]
    fn groups_credits_like_spotify() {
        let credits = parse_credits("spotify:track:70LcF31zb1H0PyJoS1Sx1r", &creep()).unwrap();
        assert_eq!(credits.track, "Creep");
        assert_eq!(credits.sources, ["XL Recordings"]);
        let names: Vec<&str> = credits
            .groups
            .iter()
            .map(|group| group.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Artist",
                "Composition & Lyrics",
                "Performers",
                "Production & Engineering"
            ]
        );
        assert_eq!(
            credits.groups[0].people,
            [Contributor {
                name: "Radiohead".into(),
                roles: vec!["Main Artist".into()],
                uri: Some(RADIOHEAD.into()),
                url: None,
            }]
        );
        let writers = &credits.groups[1].people;
        assert_eq!(writers.len(), 2);
        assert_eq!(writers[0].uri, None);
        assert_eq!(writers[1].name, "Thom Yorke");
        assert_eq!(writers[1].roles, ["Composer", "Lyricist"]);
        let producers = &credits.groups[3].people;
        assert_eq!(producers[2].name, "Radiohead");
        assert_eq!(producers[2].uri, None);
        assert_eq!(credits.linked_artists(), [RADIOHEAD]);
    }

    #[test]
    fn keeps_external_urls_and_an_other_group() {
        let contributors = [
            json!({"name": "Studio Band", "role": "Performer", "url": "https://example.com/band"}),
            json!({"name": "Studio Band", "role": "Arranger", "reference": {"url": "https://example.com/other"}}),
            json!({"name": "", "role": "Ghost"}),
            json!({"name": "Someone", "role": ""}),
        ];
        let groups = group(&contributors, &HashSet::new());
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, OTHER_GROUP);
        assert_eq!(
            groups[0].people,
            [
                Contributor {
                    name: "Studio Band".into(),
                    roles: vec!["Performer".into(), "Arranger".into()],
                    uri: None,
                    url: Some("https://example.com/band".into()),
                },
                Contributor {
                    name: "Someone".into(),
                    roles: Vec::new(),
                    uri: None,
                    url: None,
                },
            ]
        );
    }

    #[test]
    fn refuses_missing_tracks() {
        let error = parse_credits(
            "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
            &json!({"trackUnion": {"__typename": "NotFound"}}),
        )
        .unwrap_err();
        assert_eq!(error.downcast_ref::<Unavailable>(), Some(&Unavailable));
        assert_eq!(error.to_string(), "Credits aren’t available for this song.");
        let empty = parse_credits(
            "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
            &json!({"trackUnion": {"__typename": "Track", "name": "Quiet"}}),
        )
        .unwrap();
        assert!(empty.groups.is_empty());
        assert!(empty.sources.is_empty());
    }

    #[test]
    fn picks_the_cover_color_query_per_kind() {
        let names: Vec<Option<&str>> = [
            "spotify:track:70LcF31zb1H0PyJoS1Sx1r",
            "spotify:album:1b5KESuOMQomF3wntoLAsk",
            "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb",
            "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            "spotify:episode:0123456789abcdefghijkl",
            "spotify:show:0123456789abcdefghijkl",
            "spotify:user:someone",
        ]
        .into_iter()
        .map(|uri| cover_color_operation(uri).map(|operation| operation.name))
        .collect();
        assert_eq!(
            names,
            [
                Some("fetchExtractedColorForTrackEntity"),
                Some("fetchExtractedColorForAlbumEntity"),
                Some("fetchExtractedColorForArtistEntity"),
                Some("fetchExtractedColorForPlaylistEntity"),
                Some("fetchExtractedColorForEpisodeEntity"),
                Some("fetchExtractedColorForPodcastEntity"),
                None,
            ]
        );
    }

    #[test]
    fn reads_the_dark_cover_color() {
        let color = |data: Value| parse_cover_color(&data);
        let swatch = json!({"extractedColors": {"colorDark": {"hex": "#4F3B2A"}}});
        assert_eq!(
            color(json!({"trackUnion": {"albumOfTrack": {"coverArt": swatch.clone()}}})),
            Some(0x4f3b2a)
        );
        assert_eq!(
            color(json!({"playlistV2": {"images": {"items": [swatch.clone()]}}})),
            Some(0x4f3b2a)
        );
        assert_eq!(
            color(json!({"artistUnion": {"visuals": {"avatarImage": swatch}}})),
            Some(0x4f3b2a)
        );
        assert_eq!(color(json!({"albumUnion": {"coverArt": {}}})), None);
        assert_eq!(color(json!({})), None);
    }

    const SONG: &str = "spotify:track:70LcF31zb1H0PyJoS1Sx1r";
    const RADIO: &str = "spotify:playlist:37i9dQZF1E8Redacted001";

    #[test]
    fn reads_every_artist_of_a_track() {
        assert_eq!(
            track_artists_variables("spotify:track:2nrG5UtAcqXbYttu7MXP1p"),
            json!({"trackUri": "spotify:track:2nrG5UtAcqXbYttu7MXP1p"})
        );
        let data = json!({"trackUnion": {"__typename": "Track", "artists": {"items": [
            {"profile": {"name": "Queen"}, "uri": "spotify:artist:1dfeR4HaWDbWqFHLkxsg1d"},
            {"profile": {"name": "David Bowie"}, "uri": "spotify:artist:0oSGxfWSnnOXhD2fKuz2Gy"},
            {"profile": {"name": ""}, "uri": "spotify:artist:nameless"},
            {"profile": {"name": "No link"}}
        ]}, "uri": "spotify:track:2nrG5UtAcqXbYttu7MXP1p"}});
        assert_eq!(
            parse_track_artists(&data),
            [
                Link {
                    uri: "spotify:artist:1dfeR4HaWDbWqFHLkxsg1d".into(),
                    name: "Queen".into(),
                },
                Link {
                    uri: "spotify:artist:0oSGxfWSnnOXhD2fKuz2Gy".into(),
                    name: "David Bowie".into(),
                },
            ]
        );
        assert!(parse_track_artists(&json!({"trackUnion": {"__typename": "NotFound"}})).is_empty());
    }

    #[test]
    fn finds_the_radio_playlist_for_a_seed() {
        assert_eq!(
            radio_path(SONG),
            "/inspiredby-mix/v2/seed_to_playlist/spotify:track:70LcF31zb1H0PyJoS1Sx1r?response-format=json"
        );
        assert_eq!(
            parse_radio(&json!({"total": 1, "mediaItems": [{"uri": RADIO}]})).as_deref(),
            Some(RADIO)
        );
        assert_eq!(parse_radio(&json!({"total": 0, "mediaItems": []})), None);
        assert_eq!(parse_radio(&json!({})), None);
        let status = |code: u16| {
            anyhow::Error::new(HttpStatus(reqwest::StatusCode::from_u16(code).unwrap()))
        };
        assert!(refused(&status(403)));
        assert!(refused(&status(404)));
        assert!(!refused(&status(401)));
        assert!(!refused(&status(429)));
        assert!(!refused(&status(502)));
        assert_eq!(
            NoRadio.to_string(),
            "Radio is not available because personalized recommendations are turned off"
        );
    }

    #[test]
    fn hides_only_in_playlists_and_albums() {
        let cases = [
            (RADIO, Some(RADIO)),
            (
                "spotify:user:someone:playlist:37i9dQZF1E8Redacted001",
                Some(RADIO),
            ),
            (
                "spotify:album:1b5KESuOMQomF3wntoLAsk",
                Some("spotify:album:1b5KESuOMQomF3wntoLAsk"),
            ),
            ("spotify:collection:tracks", None),
            ("spotify:user:someone:collection", None),
            ("spotify:artist:4Z8W4fKeB5YxbusRsdQVPb", None),
            ("spotify:station:track:70LcF31zb1H0PyJoS1Sx1r", None),
            ("spotify:playlist:", None),
        ];
        for (uri, context) in cases {
            assert_eq!(hide_context(uri).as_deref(), context, "{uri}");
        }
        assert_eq!(BanProvider::of(None), Some(BanProvider::Context));
        assert_eq!(BanProvider::of(Some("context")), Some(BanProvider::Context));
        assert_eq!(
            BanProvider::of(Some("autoplay")),
            Some(BanProvider::Autoplay)
        );
        assert_eq!(BanProvider::of(Some("queue")), None);
    }

    #[test]
    fn reads_banned_songs_from_protobuf_pages() {
        assert_eq!(
            PageRequest::decode(bans_page_request("someone", Some("next")).as_slice()).unwrap(),
            PageRequest {
                username: "someone".into(),
                set: "playcontextbans".into(),
                pagination_token: "next".into(),
                limit: 300,
            }
        );
        let item = |uri: &str, context: Option<&str>, removed: bool| CollectionItem {
            uri: uri.into(),
            added_at: 1_725_636_253,
            is_removed: removed,
            context_uri: context.map(str::to_owned),
            item_data: None,
        };
        let page = PageResponse {
            items: vec![
                item(SONG, Some(RADIO), false),
                item(
                    "spotify:track:4uLU6hMCjMI75M1A2tKUQC",
                    Some("spotify:user:someone:playlist:37i9dQZF1E8Redacted001"),
                    false,
                ),
                item("spotify:track:gone", Some(RADIO), true),
                item("spotify:track:loose", None, false),
            ],
            next_page_token: "token".into(),
            sync_token: "sync".into(),
        };
        let (bans, next) = parse_bans_page(&page.encode_to_vec()).unwrap();
        assert_eq!(
            bans,
            [
                Ban {
                    track: SONG.into(),
                    context: RADIO.into(),
                },
                Ban {
                    track: "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into(),
                    context: RADIO.into(),
                },
            ]
        );
        assert_eq!(next.as_deref(), Some("token"));
        let (empty, next) = parse_bans_page(&[]).unwrap();
        assert!(empty.is_empty());
        assert_eq!(next, None);
        let json = collection::parse_page(&json!({"items": [
            {"uri": SONG, "added_at": "1725636253", "context_uri": RADIO},
            {"uri": "spotify:track:loose", "added_at": 1},
            {"uri": "spotify:track:artist", "context_uri": "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb"}
        ]}));
        assert_eq!(
            json_bans(json.items),
            [Ban {
                track: SONG.into(),
                context: RADIO.into(),
            }]
        );
    }

    #[test]
    fn writes_bans_like_the_desktop_core() {
        let ban = Ban {
            track: SONG.into(),
            context: RADIO.into(),
        };
        let request = WriteRequest::decode(
            ban_write_request(
                "someone",
                &ban,
                BanProvider::Autoplay,
                true,
                "0123456789abcdef",
            )
            .as_slice(),
        )
        .unwrap();
        assert_eq!(request.username, "someone");
        assert_eq!(request.set, "playcontextbans");
        assert_eq!(request.client_update_id, "0123456789abcdef");
        let item = &request.items[0];
        assert_eq!(item.uri, SONG);
        assert_eq!(item.context_uri.as_deref(), Some(RADIO));
        assert!(!item.is_removed);
        assert_eq!(
            item.item_data.as_ref().and_then(|data| data.data.clone()),
            Some(Data::PlayContextBansItemData(PlayContextBansItemData {
                item_provider: ItemProvider::ProviderAutoplay as i32,
            }))
        );
        let shown = WriteRequest::decode(
            ban_write_request("someone", &ban, BanProvider::Context, false, "x").as_slice(),
        )
        .unwrap();
        assert!(shown.items[0].is_removed);
        assert_eq!(
            ban_write_json("someone", &ban, true),
            json!({"username": "someone", "set": "playcontextbans", "items": [
                {"uri": SONG, "context_uri": RADIO}
            ]})
        );
        assert_eq!(
            ban_write_json("someone", &ban, false)["items"][0]["is_removed"],
            json!(true)
        );
        let id = client_update_id();
        assert_eq!(id.len(), 16);
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(WriteFormat::Protobuf.name(), "protobuf");
        assert_eq!(WriteFormat::Json.name(), "json");
    }

    #[test]
    fn bans_artists_like_the_web_client() {
        assert_eq!(
            collection::write_body(
                "someone",
                ARTIST_BAN,
                &[artist_ban("spotify:artist:4Z8W4fKeB5YxbusRsdQVPb", true)]
            ),
            json!({"username": "someone", "set": "artistban", "items": [
                {"uri": "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb"}
            ]})
        );
        assert_eq!(
            collection::write_body(
                "someone",
                ARTIST_BAN,
                &[artist_ban("spotify:artist:4Z8W4fKeB5YxbusRsdQVPb", false)]
            ),
            json!({"username": "someone", "set": "artistban", "items": [
                {"uri": "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb", "is_removed": true}
            ]})
        );
    }

    fn push(uri: &str, payload: Value) -> Push {
        Push {
            uri: uri.to_owned(),
            headers: std::collections::HashMap::new(),
            payloads: vec![payload],
        }
    }

    #[test]
    fn reads_feedback_set_pushes() {
        use base64::Engine;

        let body = json!({"items": [
            {"type": "playlist", "identifier": "37i9dQZF1E8Redacted001", "removed": false},
            {"type": "track", "identifier": "70LcF31zb1H0PyJoS1Sx1r", "removed": true},
            {"type": "artist", "identifier": "00000000000000000000000000000001"},
            {"type": "track", "identifier": "short"},
            {"identifier": "70LcF31zb1H0PyJoS1Sx1r"}
        ]});
        let encoded = base64::engine::general_purpose::STANDARD.encode(body.to_string());
        let pushed = set_push(&push(
            "hm://collection/ignoreinrecs/someone/json",
            Value::String(encoded),
        ))
        .unwrap();
        assert_eq!(pushed.set, FeedbackSet::Excluded);
        assert_eq!(pushed.unreadable, 2);
        assert_eq!(
            pushed.items,
            [
                Pushed {
                    uri: RADIO.into(),
                    context: None,
                    removed: false,
                },
                Pushed {
                    uri: SONG.into(),
                    context: None,
                    removed: true,
                },
                Pushed {
                    uri: "spotify:artist:0000000000000000000001".into(),
                    context: None,
                    removed: false,
                },
            ]
        );
        let banned = set_push(&push(
            "hm://collection/artistban/someone/json",
            json!({"items": [{"type": "artist", "identifier": "4Z8W4fKeB5YxbusRsdQVPb"}]}),
        ))
        .unwrap();
        assert_eq!(banned.set, FeedbackSet::Banned);
        assert_eq!(banned.items[0].uri, RADIOHEAD);
        let hidden = set_push(&push(
            "hm://collection/playcontextbans/someone/json",
            json!({"items": [
                {"type": "track", "identifier": "70LcF31zb1H0PyJoS1Sx1r",
                 "context_uri": "spotify:user:someone:playlist:37i9dQZF1E8Redacted001"},
                {"type": "track", "identifier": "70LcF31zb1H0PyJoS1Sx1r", "removed": true}
            ]}),
        ))
        .unwrap();
        assert_eq!(hidden.set, FeedbackSet::Hidden);
        assert_eq!(hidden.items[0].context.as_deref(), Some(RADIO));
        assert_eq!(hidden.items[1].context, None);
        for uri in [
            "hm://collection/collection/someone/json",
            "hm://collection/artistban/someone",
            "hm://collection/artistban//json",
            "hm://playlist/v2/user/someone/rootlist",
        ] {
            assert_eq!(set_push(&push(uri, json!({"items": []}))), None, "{uri}");
        }
        assert_eq!(
            FeedbackSet::of("artistban").map(FeedbackSet::name),
            Some("artistban")
        );
    }

    fn sources(name: &str) -> Value {
        json!({"sources": [
            {"url": format!("https://i.scdn.co/image/{name}-64"), "width": 64, "height": 64},
            {"url": format!("https://i.scdn.co/image/{name}-300"), "width": 300, "height": 300},
            {"url": format!("https://i.scdn.co/image/{name}-640"), "width": 640, "height": 640}
        ]})
    }

    fn release(uri: &str, name: &str, kind: &str, year: i64) -> Value {
        json!({"uri": uri, "name": name, "type": kind, "date": {"year": year},
            "coverArt": sources(name), "playability": {"playable": true}})
    }

    fn counting_stars() -> Value {
        json!({"trackUnion": {
            "__typename": "Track",
            "uri": "spotify:track:2tpWsVSb9UEmDRxAl1zhX1",
            "name": "Counting Stars",
            "duration": {"totalMilliseconds": 257265},
            "playcount": "3742266181",
            "contentRating": {"label": "NONE"},
            "playability": {"playable": true, "reason": "PLAYABLE"},
            "albumOfTrack": {
                "uri": "spotify:album:20lOt6G8MHv8ZO7ViOmiP7",
                "name": "Native",
                "type": "ALBUM",
                "date": {"isoString": "2013-01-01T00:00:00Z", "precision": "DAY", "year": 2013},
                "copyright": {"items": [
                    {"text": "© 2013 Mosley Music/Interscope Records", "type": "C"},
                    {"text": "2013 Mosley Music/Interscope Records", "type": "P"}
                ], "totalCount": 2},
                "coverArt": sources("native"),
                "playability": {"playable": true},
                "tracks": {"totalCount": 2, "items": [
                    {"track": {"trackNumber": 1, "uri": "spotify:track:6HWeQq0zRRHQs0Sj3GpZb3"}},
                    {"track": {"trackNumber": 2, "uri": "spotify:track:2tpWsVSb9UEmDRxAl1zhX1"}}
                ]}
            },
            "firstArtist": {"totalCount": 1, "items": [{
                "uri": "spotify:artist:5Pwc4xIPtQLFEnJriah9YJ",
                "id": "5Pwc4xIPtQLFEnJriah9YJ",
                "profile": {"name": "OneRepublic"},
                "visuals": {"avatarImage": sources("onerepublic")},
                "discography": {
                    "topTracks": {"items": [
                        {"track": {"uri": "spotify:track:2tpWsVSb9UEmDRxAl1zhX1", "name": "Counting Stars",
                            "duration": {"totalMilliseconds": 257265}, "playcount": "3742266181",
                            "contentRating": {"label": "NONE"},
                            "artists": {"items": [{"uri": "spotify:artist:5Pwc4xIPtQLFEnJriah9YJ", "profile": {"name": "OneRepublic"}}]},
                            "albumOfTrack": {"uri": "spotify:album:20lOt6G8MHv8ZO7ViOmiP7", "name": "Native", "coverArt": sources("native")}}},
                        {"track": {"uri": "spotify:episode:notasong", "name": "Skip"}}
                    ]},
                    "popularReleasesAlbums": {"items": [release("spotify:album:20lOt6G8MHv8ZO7ViOmiP7", "Native", "ALBUM", 2013)]},
                    "albums": {"totalCount": 1, "items": [{"releases": {"items": [release("spotify:album:0000000000000000000001", "Human", "ALBUM", 2021)]}}]},
                    "singles": {"totalCount": 1, "items": [{"releases": {"items": [release("spotify:album:0000000000000000000002", "Single", "SINGLE", 2024)]}}]}
                },
                "relatedContent": {"relatedArtists": {"totalCount": 1, "items": [
                    {"uri": "spotify:artist:0000000000000000000003", "profile": {"name": "Related"}, "visuals": {"avatarImage": sources("related")}}
                ]}}
            }]},
            "otherArtists": {"items": [
                {"uri": "spotify:artist:0000000000000000000004", "profile": {"name": "Guest"}, "visuals": {"avatarImage": null}}
            ]}
        }})
    }

    #[test]
    fn reads_a_track_page() {
        assert_eq!(
            track_variables("spotify:track:2tpWsVSb9UEmDRxAl1zhX1"),
            json!({"uri": "spotify:track:2tpWsVSb9UEmDRxAl1zhX1"})
        );
        let page = parse_track_page(&counting_stars()).unwrap();
        assert_eq!(page.track.title, "Counting Stars");
        assert_eq!(page.track.artist, "OneRepublic, Guest");
        assert_eq!(page.track.duration_ms, 257265);
        assert_eq!(
            page.track.cover.as_deref(),
            Some("https://i.scdn.co/image/native-300")
        );
        assert!(page.playable);
        assert_eq!(page.playcount, Some(3_742_266_181));
        let album = page.album.as_ref().unwrap();
        assert_eq!(album.name, "Native");
        assert_eq!(album.kind, "ALBUM");
        assert_eq!(
            album.copyrights,
            [
                "© 2013 Mosley Music/Interscope Records",
                "℗ 2013 Mosley Music/Interscope Records"
            ]
        );
        assert_eq!(
            album.date.as_ref().and_then(ReleaseDate::long).as_deref(),
            Some("January 1, 2013")
        );
        assert_eq!(album.tracks.len(), 2);
        assert_eq!(page.artists.len(), 2);
        let first = &page.artists[0];
        assert_eq!(first.name, "OneRepublic");
        assert_eq!(
            first.image.as_deref(),
            Some("https://i.scdn.co/image/onerepublic-300")
        );
        assert_eq!(first.top_tracks.len(), 1);
        assert_eq!(first.top_tracks[0].album, "Native");
        assert_eq!(
            first.top_tracks[0].artist_uri.as_deref(),
            Some("spotify:artist:5Pwc4xIPtQLFEnJriah9YJ")
        );
        assert_eq!(first.popular[0].subtitle, "2013 • Album");
        assert_eq!(first.albums[0].title, "Human");
        assert_eq!(first.singles[0].subtitle, "2024 • Single");
        assert_eq!(first.related[0].kind, entity::Kind::Artist);
        let guest = &page.artists[1];
        assert_eq!(guest.name, "Guest");
        assert_eq!(guest.image, None);
        assert!(guest.top_tracks.is_empty() && guest.popular.is_empty());
        let images = page.images();
        assert!(images.contains(&"https://i.scdn.co/image/related-300".to_owned()));
        assert_eq!(
            images
                .iter()
                .filter(|url| url.ends_with("native-300"))
                .count(),
            1
        );
        let missing = parse_track_page(&json!({"trackUnion": {"__typename": "NotFound"}}));
        let error = missing.unwrap_err();
        assert_eq!(error.downcast_ref::<TrackMissing>(), Some(&TrackMissing));
        assert_eq!(error.to_string(), "Couldn’t find that song");
    }

    #[test]
    fn reads_recommended_songs() {
        assert_eq!(
            recommendations_variables(SONG),
            json!({"uri": SONG, "limit": 5})
        );
        let data = json!({"seoRecommendedTrack": {"items": [
            {"data": {"__typename": "Track", "name": "Bohemian Rhapsody",
                "uri": "spotify:track:2JiDi0qAXsPwhPqA2qaKGt",
                "duration": {"totalMilliseconds": 354321},
                "contentRating": {"label": "EXPLICIT"},
                "artists": {"items": [{"profile": {"name": "Queen"}, "uri": "spotify:artist:1dfeR4HaWDbWqFHLkxsg1d"}]},
                "albumOfTrack": {"uri": "spotify:album:1TkbyIkf6GSrO5e7gWS4AM", "name": "A Night at the Opera", "coverArt": sources("opera")}}},
            {"data": {"__typename": "NotFound"}},
            {"data": {"__typename": "Track", "name": "No uri"}}
        ]}});
        let songs = parse_recommendations(&data);
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0].title, "Bohemian Rhapsody");
        assert_eq!(songs[0].artist, "Queen");
        assert!(songs[0].explicit);
        assert_eq!(
            songs[0].cover.as_deref(),
            Some("https://i.scdn.co/image/opera-300")
        );
        assert!(parse_recommendations(&json!({})).is_empty());
    }

    #[test]
    fn writes_release_dates_like_spotify() {
        let date = |iso: &str, precision: &str| ReleaseDate {
            iso: iso.into(),
            precision: precision.into(),
            year: None,
        };
        assert_eq!(
            date("2025-08-29T00:00:00Z", "DAY").long().as_deref(),
            Some("August 29, 2025")
        );
        assert_eq!(
            date("2025-08-01T00:00:00Z", "MONTH").long().as_deref(),
            Some("August 2025")
        );
        assert_eq!(
            date("1997-01-01T00:00:00Z", "YEAR").long().as_deref(),
            Some("1997")
        );
        assert_eq!(date("", "DAY").long(), None);
        assert_eq!(
            ReleaseDate {
                year: Some(1997),
                ..date("", "YEAR")
            }
            .long()
            .as_deref(),
            Some("1997")
        );
    }

    #[test]
    fn asks_playlists_for_their_capabilities() {
        assert_eq!(
            playlist_path("spotify:user:someone:playlist:37i9dQZF1E8Redacted001").as_deref(),
            Some(
                "/playlist/v2/playlist/37i9dQZF1E8Redacted001?decorate=revision,length,attributes,timestamp,owner,capabilities&from=0&length=0"
            )
        );
        assert_eq!(playlist_path("spotify:album:1b5KESuOMQomF3wntoLAsk"), None);
    }
}
