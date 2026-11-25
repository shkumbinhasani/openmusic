use std::fmt;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::Api,
    catalog::{Link, Track},
    collection::{self, Write},
    entity::{self, CARD_IMAGE, pick_image},
    pathfinder::{self, Operation},
};

pub const SHOW_METADATA: Operation = Operation::query(
    "queryShowMetadataV2",
    "b475447846f37cc426add800b995a7859c169c53c72b62de6338c0c40a5dacea",
);
pub const SHOW_EPISODES: Operation = Operation::query(
    "queryPodcastEpisodes",
    "3539d746cf882f3909660de40b4ef472b3f5893a0761d617c282541de5d412c5",
);
pub const BOOK_CHAPTERS: Operation = Operation::query(
    "queryBookChapters",
    "8f342d1c624755901657fa65cbb80dd3bacbcca2f6d802f570ae3269d59a403e",
);
pub const EPISODE_OR_CHAPTER: Operation = Operation::query(
    "getEpisodeOrChapter",
    "5f77db47e5a2ce6680330bb61317aa09e17ddd1d189513cb218e7cd3e43288be",
);
pub const SHOW_PLAYED_STATE: Operation = Operation::query(
    "showItemsPlayedState",
    "4a070b9bfab2e8537a5271e6839bc3ef51501dcac8170b5fb69a98f967c5fb60",
);
pub const DECORATE_EPISODES: Operation = Operation::query(
    "decorateContextEpisodesOrChapters",
    "383de00240775c39a6afe0b1055dc562b2a3930894201f9762f3fc32a74971c7",
);
pub const AUDIOBOOKS_METADATA: Operation = Operation::query(
    "getAudiobooksMetadata",
    "523c71c64749a628f83e6b31a122c76663243730bf02df01fd64abf0f62f572f",
);
pub const NPV_EPISODE: Operation = Operation::query(
    "queryNpvEpisode",
    "5704293e4a52b987c87bc11afe210aaca52813405c85ff8d0632e7587ebc6e18",
);

pub const PAGE_SIZE: u32 = 50;
pub const HEADER_IMAGE: u32 = 640;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unavailable {
    Restricted,
    Missing,
}

impl fmt::Display for Unavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Restricted => "This isn’t available in your country.",
            Self::Missing => "Spotify couldn’t find this.",
        })
    }
}

impl std::error::Error for Unavailable {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Played {
    #[default]
    NotStarted,
    InProgress(u32),
    Completed,
}

impl Played {
    pub fn position(self) -> u32 {
        match self {
            Self::InProgress(position) => position,
            Self::NotStarted | Self::Completed => 0,
        }
    }

    pub fn finished(self) -> bool {
        self == Self::Completed
    }

    pub fn started(self) -> bool {
        matches!(self, Self::InProgress(_))
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::NotStarted => "not-started",
            Self::InProgress(_) => "in-progress",
            Self::Completed => "completed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Availability {
    #[default]
    Playable,
    Restricted,
    PaymentRequired,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub language: String,
    pub url: String,
    pub fixed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Episode {
    pub uri: String,
    pub uid: Option<String>,
    pub name: String,
    pub chapter: bool,
    pub show_uri: Option<String>,
    pub show_name: String,
    pub description: String,
    pub duration_ms: u32,
    pub released: Option<String>,
    pub cover: Option<String>,
    pub large_cover: Option<String>,
    pub wide_cover: Option<String>,
    pub video: bool,
    pub explicit: bool,
    pub played: Played,
    pub availability: Availability,
    pub transcripts: Vec<Transcript>,
}

impl Episode {
    pub fn playable(&self) -> bool {
        self.availability == Availability::Playable
    }

    pub fn restricted(&self) -> bool {
        self.availability == Availability::Restricted
    }

    pub fn as_track(&self) -> Track {
        let show = self.show_uri.clone().map(|uri| Link {
            uri,
            name: self.show_name.clone(),
        });
        Track {
            uri: self.uri.clone(),
            title: self.name.clone(),
            artist: self.show_name.clone(),
            duration_ms: self.duration_ms,
            cover: self.cover.clone(),
            artists: show.into_iter().collect(),
            uid: self.uid.clone(),
            explicit: self.explicit,
            ..Track::default()
        }
    }

    pub fn summary(&self) -> String {
        let flat = self.description.split_whitespace().collect::<Vec<_>>();
        let mut summary = String::new();
        for word in flat {
            if summary.len() + word.len() > 280 {
                summary.push('…');
                break;
            }
            if !summary.is_empty() {
                summary.push(' ');
            }
            summary.push_str(word);
        }
        summary
    }

    pub fn transcript(&self, language: &str) -> Option<&Transcript> {
        let wanted = language.to_ascii_lowercase();
        let base = wanted.split(['-', '_']).next().unwrap_or_default();
        self.transcripts
            .iter()
            .find(|transcript| transcript.language.eq_ignore_ascii_case(&wanted))
            .or_else(|| {
                self.transcripts.iter().find(|transcript| {
                    !base.is_empty()
                        && transcript
                            .language
                            .split(['-', '_'])
                            .next()
                            .is_some_and(|language| language.eq_ignore_ascii_case(base))
                })
            })
            .or_else(|| self.transcripts.first())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShowKind {
    #[default]
    Podcast,
    Audiobook,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    #[default]
    Audio,
    Video,
    Mixed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Order {
    #[default]
    Episodic,
    Sequential,
    Recent,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rating {
    pub average: f32,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Topic {
    pub title: String,
    pub uri: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Price {
    pub amount: String,
    pub list: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    pub signifier: Option<String>,
    pub title: Option<String>,
    pub body: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub authors: Vec<Link>,
    pub narrators: Vec<String>,
    pub duration_ms: u32,
    pub published: Option<String>,
    pub price: Option<Price>,
    pub access: Option<Access>,
    pub copyrights: Vec<String>,
    pub genres: Vec<String>,
    pub pre_release: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EpisodePage {
    pub episodes: Vec<Episode>,
    pub total: u32,
    pub next_offset: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Show {
    pub uri: String,
    pub kind: ShowKind,
    pub name: String,
    pub publisher: String,
    pub description: String,
    pub cover: Option<String>,
    pub thumbnail: Option<String>,
    pub media: MediaKind,
    pub order: Order,
    pub saved: bool,
    pub playable: bool,
    pub rating: Option<Rating>,
    pub topics: Vec<Topic>,
    pub trailer: Option<String>,
    pub episodes: Vec<Episode>,
    pub total: u32,
    pub next_offset: Option<u32>,
    pub book: Option<Book>,
}

impl Show {
    pub fn video(&self) -> bool {
        self.media != MediaKind::Audio
    }

    pub fn append(&mut self, offset: u32, page: EpisodePage) -> bool {
        if offset as usize != self.episodes.len() {
            return false;
        }
        let mut page = page;
        for episode in &mut page.episodes {
            if episode.show_uri.is_none() {
                episode.show_uri = Some(self.uri.clone());
            }
            if episode.show_name.is_empty() {
                episode.show_name = self.name.clone();
            }
        }
        self.episodes.extend(page.episodes);
        self.total = page.total.max(self.episodes.len() as u32);
        self.next_offset = page
            .next_offset
            .filter(|next| *next as usize == self.episodes.len() && *next < self.total);
        true
    }

    pub fn tracks(&self) -> Vec<Track> {
        self.episodes.iter().map(Episode::as_track).collect()
    }

    pub fn restricted(&self) -> usize {
        self.episodes
            .iter()
            .filter(|episode| episode.restricted())
            .count()
    }
}

pub fn resume_index(order: Order, items: &[(Played, Availability)]) -> Option<(usize, bool)> {
    let playable = |index: &usize| items[*index].1 == Availability::Playable;
    let mut indices = 0..items.len();
    if let Some(index) = indices
        .clone()
        .filter(playable)
        .find(|index| items[*index].0.started())
    {
        return Some((index, true));
    }
    if order == Order::Sequential
        && let Some(index) = indices
            .clone()
            .filter(playable)
            .find(|index| !items[*index].0.finished())
    {
        return Some((index, false));
    }
    indices.find(playable).map(|index| (index, false))
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

fn owned(value: &Value) -> Option<String> {
    text(value).map(str::to_owned)
}

fn number(value: &Value) -> u32 {
    value
        .as_u64()
        .or_else(|| value.as_f64().map(|float| float.max(0.) as u64))
        .unwrap_or_default()
        .min(u64::from(u32::MAX)) as u32
}

pub fn decode_entities(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        let tail = &rest[start..];
        let entity = tail
            .find(';')
            .filter(|end| *end <= 10)
            .map(|end| (&tail[1..end], end));
        let replacement = entity.and_then(|(name, end)| {
            let character = match name {
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                    .or_else(|| {
                        name.strip_prefix('#')
                            .and_then(|digits| digits.parse().ok())
                    })
                    .and_then(char::from_u32),
            };
            character.map(|character| (character, end))
        });
        match replacement {
            Some((character, end)) => {
                decoded.push(character);
                rest = &tail[end + 1..];
            }
            None => {
                decoded.push('&');
                rest = &tail[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

pub fn description_text(html: &str) -> String {
    let mut plain = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        plain.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            rest = &rest[start..];
            break;
        };
        let tag = rest[start + 1..start + end]
            .trim()
            .trim_end_matches('/')
            .trim()
            .to_ascii_lowercase();
        let name = tag.split_whitespace().next().unwrap_or_default();
        match name {
            "br" | "/li" | "/div" | "/h1" | "/h2" | "/h3" => plain.push('\n'),
            "/p" => plain.push_str("\n\n"),
            "li" => plain.push_str("• "),
            _ => {}
        }
        rest = &rest[start + end + 1..];
    }
    plain.push_str(rest);
    let decoded = decode_entities(&plain);
    let mut lines: Vec<&str> = Vec::new();
    let mut blank = 0;
    for line in decoded.lines().map(str::trim) {
        if line.is_empty() {
            blank += 1;
            if blank == 1 && !lines.is_empty() {
                lines.push("");
            }
        } else {
            blank = 0;
            lines.push(line);
        }
    }
    while lines.last() == Some(&"") {
        lines.pop();
    }
    lines.join("\n")
}

fn description(data: &Value) -> String {
    ["htmlDescriptionPlain", "htmlDescription"]
        .iter()
        .find_map(|key| text(&data[*key]))
        .map(description_text)
        .filter(|text| !text.is_empty())
        .or_else(|| text(&data["description"]).map(|plain| decode_entities(plain.trim())))
        .unwrap_or_default()
}

fn played(data: &Value) -> Played {
    let state = &data["playedState"];
    match state["state"].as_str() {
        Some("IN_PROGRESS") => Played::InProgress(number(&state["playPositionMilliseconds"])),
        Some("COMPLETED") => Played::Completed,
        _ => Played::NotStarted,
    }
}

fn availability(data: &Value) -> Availability {
    let playability = &data["playability"];
    if playability["playable"].as_bool().unwrap_or(true) {
        return Availability::Playable;
    }
    let reasons = playability["unplayabilityReasons"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    match playability["reason"].as_str() {
        Some("PAYMENT_REQUIRED") => Availability::PaymentRequired,
        Some("COUNTRY_RESTRICTED" | "NOT_AVAILABLE_IN_REGION") => Availability::Restricted,
        _ if reasons.clone().any(|reason| reason == "PAYMENT_REQUIRED") => {
            Availability::PaymentRequired
        }
        _ => Availability::Other,
    }
}

fn transcripts(data: &Value) -> Vec<Transcript> {
    data["transcripts"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let url = owned(&item["readAlongUrlV2"])?;
            let trusted = url::Url::parse(&url).is_ok_and(|parsed| {
                parsed.scheme() == "https"
                    && parsed.host_str().is_some_and(|host| {
                        host == "spotify.com"
                            || host.ends_with(".spotify.com")
                            || host.ends_with(".spotifycdn.com")
                    })
            });
            trusted.then(|| Transcript {
                language: owned(&item["language"]).unwrap_or_default(),
                url,
                fixed: item["isStatic"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

fn episode_from(typename: &str, data: &Value, uri: String, uid: Option<String>) -> Option<Episode> {
    match typename {
        "Episode" | "Chapter" => {
            let chapter = typename == "Chapter";
            let show = if chapter {
                &data["audiobookV2"]["data"]
            } else {
                &data["podcastV2"]["data"]
            };
            let media = data["mediaTypes"].as_array();
            Some(Episode {
                name: owned(&data["name"]).unwrap_or_default(),
                chapter,
                show_uri: owned(&show["uri"]),
                show_name: owned(&show["name"]).unwrap_or_default(),
                description: description(data),
                duration_ms: number(&data["duration"]["totalMilliseconds"]),
                released: owned(&data["releaseDate"]["isoString"]),
                cover: pick_image(&data["coverArt"]["sources"], CARD_IMAGE),
                large_cover: pick_image(&data["coverArt"]["sources"], HEADER_IMAGE),
                wide_cover: pick_image(
                    &data["visualIdentity"]["sixteenByNineCoverImage"]["image"]["data"]["sources"],
                    HEADER_IMAGE,
                ),
                video: media
                    .into_iter()
                    .flatten()
                    .any(|kind| kind.as_str() == Some("VIDEO")),
                explicit: entity::explicit(data),
                played: played(data),
                availability: availability(data),
                transcripts: transcripts(data),
                uri,
                uid,
            })
        }
        "RestrictedContent" => Some(Episode {
            uri,
            uid,
            availability: Availability::Restricted,
            ..Episode::default()
        }),
        _ => None,
    }
}

pub fn parse_item(item: &Value) -> Option<Episode> {
    let entity = if item["entity"].is_object() {
        &item["entity"]
    } else {
        item
    };
    let data = if entity["data"].is_object() {
        &entity["data"]
    } else {
        entity
    };
    let typename = data["__typename"].as_str()?;
    let uri = owned(&data["uri"]).or_else(|| owned(&entity["_uri"]))?;
    episode_from(typename, data, uri, owned(&item["uid"]))
}

pub fn parse_episode(data: &Value) -> Result<Episode> {
    let union = &data["episodeUnionV2"];
    match union["__typename"].as_str() {
        Some("RestrictedContent") => bail!(Unavailable::Restricted),
        Some("Episode" | "Chapter") => {}
        _ => bail!(Unavailable::Missing),
    }
    let Some(uri) = owned(&union["uri"]) else {
        bail!(Unavailable::Missing);
    };
    let typename = union["__typename"].as_str().unwrap_or_default();
    episode_from(typename, union, uri, None).ok_or_else(|| Unavailable::Missing.into())
}

pub fn parse_page(data: &Value) -> Result<EpisodePage> {
    let union = &data["podcastUnionV2"];
    if union["__typename"] == "RestrictedContent" {
        bail!(Unavailable::Restricted);
    }
    let page = ["episodesV2", "chaptersV2"]
        .iter()
        .map(|key| &union[*key])
        .find(|page| page.is_object());
    let Some(page) = page else {
        bail!("Spotify sent no episodes for this show.");
    };
    let episodes: Vec<Episode> = page["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(parse_item)
        .collect();
    let total = number(&page["totalCount"]);
    let next_offset = page["pagingInfo"]["nextOffset"]
        .as_u64()
        .map(|next| next.min(u64::from(u32::MAX)) as u32)
        .filter(|next| !episodes.is_empty() && *next < total);
    Ok(EpisodePage {
        episodes,
        total,
        next_offset,
    })
}

fn copyright(item: &Value) -> Option<String> {
    let line = decode_entities(text(&item["text"])?.trim());
    Some(match item["type"].as_str() {
        Some("C") if !line.starts_with('©') => format!("© {line}"),
        Some("P") if !line.starts_with('℗') => format!("℗ {line}"),
        _ => line,
    })
}

fn book(data: &Value) -> Book {
    let price = &data["price"];
    let amount = owned(&price["finalPrice"]["formattedPrice"]);
    let list = owned(&price["finalListPrice"]["formattedPrice"]);
    let access = &data["accessInfo"];
    let explanation = &access["accessExplanation"];
    let access = Access {
        signifier: owned(&access["signifier"]["text"]),
        title: owned(&explanation["title"]),
        body: owned(&explanation["body"]),
    };
    Book {
        authors: data["authorsV2"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|author| {
                Some(Link {
                    name: owned(&author["name"])?,
                    uri: owned(&author["uri"]).unwrap_or_default(),
                })
            })
            .collect(),
        narrators: data["narrators"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|narrator| owned(&narrator["name"]))
            .collect(),
        duration_ms: number(&data["duration"]["totalMilliseconds"]),
        published: owned(&data["publishDate"]["isoString"]),
        price: amount.map(|amount| Price {
            list: list.filter(|list| *list != amount),
            amount,
        }),
        access: (access != Access::default()).then_some(access),
        copyrights: data["copyrights"]["items"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(copyright)
            .collect(),
        genres: data["genres"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|genre| {
                owned(&genre["shortName"]).or_else(|| owned(&genre["contextualName"]))
            })
            .collect(),
        pre_release: data["isPreRelease"].as_bool().unwrap_or(false),
    }
}

pub fn parse_show(data: &Value) -> Result<Show> {
    let union = &data["podcastUnionV2"];
    let kind = match union["__typename"].as_str() {
        Some("Podcast") => ShowKind::Podcast,
        Some("Audiobook") => ShowKind::Audiobook,
        Some("RestrictedContent") => bail!(Unavailable::Restricted),
        _ => bail!(Unavailable::Missing),
    };
    let uri =
        owned(&union["uri"]).or_else(|| text(&union["id"]).map(|id| format!("spotify:show:{id}")));
    let (Some(uri), Some(name)) = (uri, owned(&union["name"])) else {
        bail!(Unavailable::Missing);
    };
    let average = &union["rating"]["averageRating"];
    let rating = average["showAverage"]
        .as_bool()
        .unwrap_or(false)
        .then(|| Rating {
            average: average["average"].as_f64().unwrap_or_default() as f32,
            count: number(&average["totalRatings"]),
        })
        .filter(|rating| rating.count > 0 && rating.average > 0.);
    let topics = union["topics"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|topic| {
            Some(Topic {
                title: owned(&topic["title"])?,
                uri: owned(&topic["uri"])?,
            })
        })
        .collect();
    let trailer = &union["trailerV2"];
    Ok(Show {
        kind,
        publisher: owned(&union["publisher"]["name"]).unwrap_or_default(),
        description: description(union),
        cover: pick_image(&union["coverArt"]["sources"], HEADER_IMAGE),
        thumbnail: pick_image(&union["coverArt"]["sources"], CARD_IMAGE),
        media: match union["mediaType"].as_str() {
            Some("VIDEO") => MediaKind::Video,
            Some("MIXED") => MediaKind::Mixed,
            _ => MediaKind::Audio,
        },
        order: match union["consumptionOrderV2"].as_str() {
            Some("SEQUENTIAL") => Order::Sequential,
            Some("RECENT") => Order::Recent,
            _ => Order::Episodic,
        },
        saved: union["saved"].as_bool().unwrap_or(false),
        playable: entity::playable(union),
        rating,
        topics,
        trailer: owned(&trailer["_uri"]).or_else(|| owned(&trailer["data"]["uri"])),
        book: (kind == ShowKind::Audiobook).then(|| book(union)),
        uri,
        name,
        ..Show::default()
    })
}

pub fn parse_played_states(data: &Value) -> Vec<Played> {
    let union = &data["lookup"][0]["data"];
    ["episodesV2", "chaptersV2"]
        .iter()
        .map(|key| &union[*key]["items"])
        .find_map(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| played(&item["entity"]["data"]))
        .collect()
}

pub fn show_variables(uri: &str) -> Value {
    json!({
        "uri": uri,
        "includeContentCapabilityTrait": true,
        "includeEpisodeContentRatingsV2": false,
    })
}

pub fn page_request(uri: &str, kind: ShowKind, offset: u32, limit: u32) -> (Operation, Value) {
    match kind {
        ShowKind::Podcast => (
            SHOW_EPISODES,
            json!({
                "uri": uri,
                "offset": offset,
                "limit": limit,
                "includeEpisodeContentRatingsV2": false,
            }),
        ),
        ShowKind::Audiobook => (
            BOOK_CHAPTERS,
            json!({ "uri": uri, "offset": offset, "limit": limit }),
        ),
    }
}

pub fn episode_variables(uri: &str) -> Value {
    json!({ "uri": uri, "includeEpisodeContentRatingsV2": false })
}

pub async fn page(
    api: &Api,
    uri: &str,
    kind: ShowKind,
    offset: u32,
    limit: u32,
) -> Result<EpisodePage> {
    let (operation, variables) = page_request(uri, kind, offset, limit);
    let data = pathfinder::query(api, operation, variables).await?;
    parse_page(&data)
}

pub async fn show(api: &Api, uri: &str) -> Result<Show> {
    let (metadata, first) = tokio::join!(
        pathfinder::query(api, SHOW_METADATA, show_variables(uri)),
        page(api, uri, ShowKind::Podcast, 0, PAGE_SIZE),
    );
    let mut show = parse_show(&metadata?)?;
    let first = match show.kind {
        ShowKind::Podcast => first?,
        ShowKind::Audiobook => page(api, uri, ShowKind::Audiobook, 0, PAGE_SIZE).await?,
    };
    show.append(0, first);
    Ok(show)
}

pub async fn episode(api: &Api, uri: &str) -> Result<Episode> {
    let data = pathfinder::query(api, EPISODE_OR_CHAPTER, episode_variables(uri)).await?;
    parse_episode(&data)
}

pub async fn played_states(api: &Api, uri: &str, limit: u32) -> Result<Vec<Played>> {
    let data = pathfinder::query(
        api,
        SHOW_PLAYED_STATE,
        json!({ "uri": uri, "limit": limit }),
    )
    .await?;
    Ok(parse_played_states(&data))
}

pub fn played_writes(uris: &[String], played: bool) -> Vec<Write> {
    uris.iter()
        .map(|uri| Write {
            uri: uri.clone(),
            context_uri: None,
            removed: !played,
        })
        .collect()
}

pub async fn mark_played(api: &Api, user: &str, uris: &[String], played: bool) -> Result<()> {
    collection::write(
        api,
        user,
        collection::MARKED_AS_FINISHED,
        &played_writes(uris, played),
    )
    .await
}

pub fn effective(played: Played, mark: Option<bool>) -> Played {
    match (mark, played) {
        (Some(true), _) => Played::Completed,
        (Some(false), Played::Completed) => Played::NotStarted,
        _ => played,
    }
}

pub fn year(iso: &str) -> Option<i32> {
    iso.get(..4)?.parse().ok()
}

pub fn year_of(unix_ms: i64) -> i32 {
    let days = unix_ms.div_euclid(86_400_000) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let january_or_february = (5 * day_of_year + 2) / 153 >= 10;
    (year_of_era + era * 400 + i64::from(january_or_february)) as i32
}

pub fn release_label(iso: &str, this_year: i32) -> Option<String> {
    let full = entity::short_date(iso)?;
    if year(iso) == Some(this_year) {
        return full.rsplit_once(", ").map(|(day, _)| day.to_owned());
    }
    Some(full)
}

pub fn length_label(duration_ms: u32, played: Played) -> Option<String> {
    match played {
        Played::InProgress(position) if position < duration_ms => {
            entity::long_duration((duration_ms - position).max(1000))
                .map(|left| format!("{left} left"))
        }
        _ => entity::long_duration(duration_ms),
    }
}

pub fn compact_count(count: u32) -> String {
    let short = |value: f64, unit: &str| {
        let text = format!("{value:.1}");
        format!("{}{unit}", text.trim_end_matches(".0"))
    };
    match count {
        0..1_000 => count.to_string(),
        1_000..10_000 => short(f64::from(count) / 1_000., "K"),
        10_000..1_000_000 => format!("{}K", (f64::from(count) / 1_000.).round()),
        _ => short(f64::from(count) / 1_000_000., "M"),
    }
}

pub fn rating_label(rating: Rating) -> String {
    format!(
        "★ {:.1} ({})",
        (rating.average * 10.).round() / 10.,
        compact_count(rating.count)
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SHOW: &str = "spotify:show:0000000000000000000ab1";
    const BOOK: &str = "spotify:show:0000000000000000000bk1";

    fn sources(stem: &str) -> Value {
        json!({"sources": [
            {"url": format!("https://i.scdn.co/image/{stem}-64"), "width": 64, "height": 64},
            {"url": format!("https://i.scdn.co/image/{stem}-300"), "width": 300, "height": 300},
            {"url": format!("https://i.scdn.co/image/{stem}-640"), "width": 640, "height": 640}
        ]})
    }

    fn episode_data(id: &str, state: &str, position: u64) -> Value {
        json!({
            "__typename": "Episode",
            "uri": format!("spotify:episode:{id}"),
            "id": id,
            "name": format!("Episode {id}"),
            "type": "PODCAST_EPISODE",
            "mediaTypes": ["AUDIO", "VIDEO"],
            "duration": {"totalMilliseconds": 5_655_661},
            "releaseDate": {"isoString": "2026-11-23T06:00:00Z", "precision": "MINUTE"},
            "playedState": {"playPositionMilliseconds": position, "state": state},
            "playability": {"playable": true, "reason": "PLAYABLE", "unplayabilityReasons": []},
            "restrictions": {"paywallContent": false},
            "contentRating": {"label": "EXPLICIT"},
            "coverArt": sources(id),
            "visualIdentity": {"sixteenByNineCoverImage": {"image": {"data": {
                "__typename": "ImageV2",
                "sources": [
                    {"maxHeight": 360, "maxWidth": 640, "url": format!("https://image-cdn-fa.spotifycdn.com/image/{id}-wide")},
                    {"maxHeight": 720, "maxWidth": 1280, "url": format!("https://image-cdn-fa.spotifycdn.com/image/{id}-wider")}
                ]
            }}}},
            "description": "Plain text.",
            "htmlDescription": "<p>First line &amp; more.</p><p>Second<br />line with <a href=\"https://example.com\">a link</a>.</p>",
            "transcripts": {"items": [
                {"language": "en-us", "isStatic": false, "readAlongUrlV2": format!("https://spclient.wg.spotify.com/transcript-read-along/v2/episode/{id}")},
                {"language": "de-de", "isStatic": false, "readAlongUrlV2": format!("https://spclient.wg.spotify.com/transcript-read-along/v2/episode/{id}/de")},
                {"language": "fr-fr", "isStatic": true, "readAlongUrlV2": "https://example.com/transcript"}
            ]},
            "podcastV2": {"data": {"__typename": "Podcast", "name": "Sample Show", "uri": SHOW}}
        })
    }

    fn episodes_reply() -> Value {
        json!({"podcastUnionV2": {
            "__typename": "Podcast",
            "uri": SHOW,
            "name": "Sample Show",
            "episodesV2": {
                "__typename": "ContextEpisodePage",
                "totalCount": 916,
                "pagingInfo": {"nextOffset": 4},
                "items": [
                    {"uid": "uid-restricted", "entity": {"_uri": "spotify:episode:00000000000000000000r1", "data": {"__typename": "RestrictedContent"}}},
                    {"uid": "uid-a", "entity": {"_uri": "spotify:episode:00000000000000000000a1", "data": episode_data("00000000000000000000a1", "NOT_STARTED", 0)}},
                    {"uid": "uid-b", "entity": {"_uri": "spotify:episode:00000000000000000000b1", "data": episode_data("00000000000000000000b1", "IN_PROGRESS", 61_000)}},
                    {"uid": "uid-c", "entity": {"_uri": "spotify:episode:00000000000000000000c1", "data": episode_data("00000000000000000000c1", "COMPLETED", 0)}}
                ]
            }
        }})
    }

    fn show_reply() -> Value {
        json!({"podcastUnionV2": {
            "__typename": "Podcast",
            "uri": SHOW,
            "id": "0000000000000000000ab1",
            "name": "Sample Show",
            "publisher": {"name": "Sample Studio"},
            "mediaType": "MIXED",
            "consumptionOrderV2": "EPISODIC",
            "saved": false,
            "playability": {"playable": true, "reason": "PLAYABLE"},
            "coverArt": sources("show"),
            "description": "Plain",
            "htmlDescription": "<p>About the show.<br /><br />More about it.</p>",
            "rating": {"averageRating": {"average": 4.6068, "showAverage": true, "totalRatings": 88073}, "canRate": true},
            "topics": {"items": [
                {"__typename": "PodcastTopic", "title": "Business", "uri": "spotify:genre:0JQ5DAqbMKFK0EBNV8Wn6R"},
                {"__typename": "PodcastTopic", "title": "Careers"}
            ]},
            "trailerV2": {"_uri": "spotify:episode:00000000000000000000t1", "data": {"__typename": "Episode"}},
            "episodesV2": {"__typename": "ContextEpisodePage", "items": [{"entity": {"__typename": "EpisodeResponseWrapper", "data": {"__typename": "RestrictedContent"}}}]}
        }})
    }

    fn book_reply() -> Value {
        json!({"podcastUnionV2": {
            "__typename": "Audiobook",
            "uri": BOOK,
            "name": "Sample Book",
            "authorsV2": [{"name": "Some Author", "uri": "spotify:author:0000000000000000000au1"}],
            "narrators": [{"name": "Some Narrator"}, {"name": "Another Narrator"}],
            "publisher": {"name": "Some Author"},
            "mediaType": "AUDIO",
            "duration": {"totalMilliseconds": 18_871_526},
            "publishDate": {"isoString": "2023-07-18T00:00:00Z"},
            "copyrights": {"items": [{"text": "Some Author 2023", "type": "C"}, {"text": "℗ Studio", "type": "P"}], "totalCount": 2},
            "genres": [{"contextualName": "Happiness & Success", "shortName": "Happiness & Success"}, {"contextualName": "Self-Help"}],
            "isPreRelease": false,
            "saved": true,
            "price": {"finalListPrice": {"formattedPrice": "€13.49"}, "finalPrice": {"formattedPrice": "€10.49"}},
            "accessInfo": {
                "signifier": {"text": "Access with Audiobooks+"},
                "accessExplanation": {"__typename": "ConsumptionCappedAccessExplanation", "title": "You're out of audiobook hours", "body": "Add hours to keep the story going."}
            },
            "rating": {"averageRating": {"average": 4.197, "showAverage": true, "totalRatings": 203}},
            "playability": {"playable": true, "reason": "PLAYABLE"},
            "coverArt": sources("book"),
            "htmlDescription": "Author(s): Some Author<br/>Narrator(s): Some Narrator<br/>This is the year.",
            "htmlDescriptionPlain": "<p>This is the year.</p><p>There&#39;s a saying.</p>",
            "topics": {"items": []},
            "trailerV2": {"_uri": "spotify:episode:00000000000000000000t2"}
        }})
    }

    fn chapters_reply() -> Value {
        let chapter = |id: &str, name: &str, playable: bool| {
            json!({"__typename": "ContextChapterV2", "uid": format!("uid-{id}"), "entity": {
                "__typename": "ChapterResponseWrapper",
                "_uri": format!("spotify:episode:{id}"),
                "data": {
                    "__typename": "Chapter",
                    "uri": format!("spotify:episode:{id}"),
                    "name": name,
                    "duration": {"totalMilliseconds": 16_927},
                    "mediaTypes": ["AUDIO"],
                    "description": "",
                    "htmlDescription": "",
                    "coverArt": sources("book"),
                    "contentRating": {"label": "NONE"},
                    "playability": if playable {
                        json!({"playable": true, "reason": "PLAYABLE", "unplayabilityReasons": []})
                    } else {
                        json!({"playable": false, "reason": "PAYMENT_REQUIRED", "unplayabilityReasons": ["PAYMENT_REQUIRED"]})
                    },
                    "playedState": {"playPositionMilliseconds": 0, "state": "NOT_STARTED"},
                    "restrictions": {"paywallContent": !playable},
                    "audiobookV2": {"__typename": "AudiobookResponseWrapper", "data": {"__typename": "Audiobook", "name": "Sample Book", "uri": BOOK}}
                }
            }})
        };
        json!({"podcastUnionV2": {
            "__typename": "Audiobook",
            "uri": BOOK,
            "chaptersV2": {
                "__typename": "ContextChapterPage",
                "totalCount": 3,
                "pagingInfo": {"nextOffset": 3},
                "items": [
                    chapter("00000000000000000000h1", "Opening Credits", false),
                    chapter("00000000000000000000h2", "Dedication", false),
                    chapter("00000000000000000000h3", "Sample", true)
                ]
            }
        }})
    }

    #[test]
    fn reads_a_page_of_episodes() {
        let page = parse_page(&episodes_reply()).unwrap();
        assert_eq!(page.total, 916);
        assert_eq!(page.next_offset, Some(4));
        assert_eq!(page.episodes.len(), 4);

        let restricted = &page.episodes[0];
        assert_eq!(restricted.uri, "spotify:episode:00000000000000000000r1");
        assert_eq!(restricted.uid.as_deref(), Some("uid-restricted"));
        assert_eq!(restricted.availability, Availability::Restricted);
        assert!(restricted.name.is_empty());

        let episode = &page.episodes[1];
        assert_eq!(episode.uri, "spotify:episode:00000000000000000000a1");
        assert_eq!(episode.uid.as_deref(), Some("uid-a"));
        assert_eq!(episode.name, "Episode 00000000000000000000a1");
        assert!(!episode.chapter);
        assert_eq!(episode.show_uri.as_deref(), Some(SHOW));
        assert_eq!(episode.show_name, "Sample Show");
        assert_eq!(episode.duration_ms, 5_655_661);
        assert_eq!(episode.released.as_deref(), Some("2026-11-23T06:00:00Z"));
        assert_eq!(
            episode.cover.as_deref(),
            Some("https://i.scdn.co/image/00000000000000000000a1-300")
        );
        assert_eq!(
            episode.large_cover.as_deref(),
            Some("https://i.scdn.co/image/00000000000000000000a1-640")
        );
        assert_eq!(
            episode.wide_cover.as_deref(),
            Some("https://image-cdn-fa.spotifycdn.com/image/00000000000000000000a1-wide")
        );
        assert!(episode.video);
        assert!(episode.explicit);
        assert_eq!(episode.played, Played::NotStarted);
        assert_eq!(episode.availability, Availability::Playable);
        assert_eq!(
            episode.description,
            "First line & more.\n\nSecond\nline with a link."
        );
        assert_eq!(episode.transcripts.len(), 2);

        assert_eq!(page.episodes[2].played, Played::InProgress(61_000));
        assert_eq!(page.episodes[3].played, Played::Completed);
    }

    #[test]
    fn stops_paging_at_the_end() {
        let mut reply = episodes_reply();
        reply["podcastUnionV2"]["episodesV2"]["pagingInfo"]["nextOffset"] = json!(916);
        assert_eq!(parse_page(&reply).unwrap().next_offset, None);
        reply["podcastUnionV2"]["episodesV2"]["items"] = json!([]);
        reply["podcastUnionV2"]["episodesV2"]["pagingInfo"]["nextOffset"] = json!(4);
        assert_eq!(parse_page(&reply).unwrap().next_offset, None);
        let restricted = json!({"podcastUnionV2": {"__typename": "RestrictedContent"}});
        assert_eq!(
            parse_page(&restricted)
                .unwrap_err()
                .downcast_ref::<Unavailable>(),
            Some(&Unavailable::Restricted)
        );
    }

    #[test]
    fn maps_an_episode_to_a_track() {
        let page = parse_page(&episodes_reply()).unwrap();
        let track = page.episodes[1].as_track();
        assert_eq!(track.uri, "spotify:episode:00000000000000000000a1");
        assert_eq!(track.title, "Episode 00000000000000000000a1");
        assert_eq!(track.artist, "Sample Show");
        assert_eq!(track.album, "");
        assert_eq!(track.album_uri, None);
        assert_eq!(track.artist_uri, None);
        assert_eq!(track.duration_ms, 5_655_661);
        assert_eq!(track.uid.as_deref(), Some("uid-a"));
        assert!(track.explicit);
        assert_eq!(
            track.artists,
            vec![Link {
                uri: SHOW.into(),
                name: "Sample Show".into()
            }]
        );
        assert!(!track.is_track());
    }

    #[test]
    fn picks_the_transcript_for_the_language() {
        let page = parse_page(&episodes_reply()).unwrap();
        let episode = &page.episodes[1];
        assert!(episode.transcript("de-AT").unwrap().url.ends_with("/de"));
        assert!(episode.transcript("en").unwrap().url.ends_with("a1"));
        assert!(episode.transcript("ja").unwrap().url.ends_with("a1"));
        assert!(Episode::default().transcript("en").is_none());
    }

    #[test]
    fn reads_show_metadata() {
        let show = parse_show(&show_reply()).unwrap();
        assert_eq!(show.uri, SHOW);
        assert_eq!(show.kind, ShowKind::Podcast);
        assert_eq!(show.name, "Sample Show");
        assert_eq!(show.publisher, "Sample Studio");
        assert_eq!(show.description, "About the show.\n\nMore about it.");
        assert_eq!(
            show.cover.as_deref(),
            Some("https://i.scdn.co/image/show-640")
        );
        assert_eq!(
            show.thumbnail.as_deref(),
            Some("https://i.scdn.co/image/show-300")
        );
        assert_eq!(show.media, MediaKind::Mixed);
        assert!(show.video());
        assert_eq!(show.order, Order::Episodic);
        assert!(!show.saved);
        assert!(show.playable);
        assert_eq!(
            show.rating,
            Some(Rating {
                average: 4.6068,
                count: 88073
            })
        );
        assert_eq!(
            show.topics,
            vec![Topic {
                title: "Business".into(),
                uri: "spotify:genre:0JQ5DAqbMKFK0EBNV8Wn6R".into()
            }]
        );
        assert_eq!(
            show.trailer.as_deref(),
            Some("spotify:episode:00000000000000000000t1")
        );
        assert!(show.episodes.is_empty());
        assert!(show.book.is_none());
    }

    #[test]
    fn refuses_restricted_and_unknown_shows() {
        let restricted = json!({"podcastUnionV2": {"__typename": "RestrictedContent"}});
        assert_eq!(
            parse_show(&restricted)
                .unwrap_err()
                .downcast_ref::<Unavailable>(),
            Some(&Unavailable::Restricted)
        );
        let missing = json!({"podcastUnionV2": null});
        assert_eq!(
            parse_show(&missing)
                .unwrap_err()
                .downcast_ref::<Unavailable>(),
            Some(&Unavailable::Missing)
        );
    }

    #[test]
    fn reads_audiobook_metadata_and_chapters() {
        let mut show = parse_show(&book_reply()).unwrap();
        assert_eq!(show.kind, ShowKind::Audiobook);
        assert_eq!(show.description, "This is the year.\n\nThere's a saying.");
        assert!(show.saved);
        assert_eq!(show.media, MediaKind::Audio);
        let book = show.book.clone().unwrap();
        assert_eq!(
            book.authors,
            vec![Link {
                name: "Some Author".into(),
                uri: "spotify:author:0000000000000000000au1".into()
            }]
        );
        assert_eq!(book.narrators, vec!["Some Narrator", "Another Narrator"]);
        assert_eq!(book.duration_ms, 18_871_526);
        assert_eq!(book.published.as_deref(), Some("2023-07-18T00:00:00Z"));
        assert_eq!(
            book.price,
            Some(Price {
                amount: "€10.49".into(),
                list: Some("€13.49".into())
            })
        );
        assert_eq!(
            book.access,
            Some(Access {
                signifier: Some("Access with Audiobooks+".into()),
                title: Some("You're out of audiobook hours".into()),
                body: Some("Add hours to keep the story going.".into()),
            })
        );
        assert_eq!(book.copyrights, vec!["© Some Author 2023", "℗ Studio"]);
        assert_eq!(book.genres, vec!["Happiness & Success", "Self-Help"]);
        assert!(!book.pre_release);

        let chapters = parse_page(&chapters_reply()).unwrap();
        assert_eq!(chapters.total, 3);
        assert_eq!(chapters.next_offset, None);
        assert!(show.append(0, chapters));
        assert_eq!(show.episodes.len(), 3);
        let first = &show.episodes[0];
        assert!(first.chapter);
        assert_eq!(first.name, "Opening Credits");
        assert_eq!(first.show_uri.as_deref(), Some(BOOK));
        assert_eq!(first.show_name, "Sample Book");
        assert_eq!(first.availability, Availability::PaymentRequired);
        assert_eq!(first.uid.as_deref(), Some("uid-00000000000000000000h1"));
        assert_eq!(show.episodes[2].availability, Availability::Playable);
    }

    #[test]
    fn appends_pages_in_order_only() {
        let mut show = parse_show(&show_reply()).unwrap();
        let page = parse_page(&episodes_reply()).unwrap();
        assert!(!show.append(50, page.clone()));
        assert!(show.append(0, page.clone()));
        assert_eq!(show.total, 916);
        assert_eq!(show.next_offset, Some(4));
        assert_eq!(show.restricted(), 1);
        assert_eq!(show.episodes[0].show_uri.as_deref(), Some(SHOW));
        assert_eq!(show.episodes[0].show_name, "Sample Show");
        assert!(!show.append(0, page.clone()));
        let last = EpisodePage {
            next_offset: None,
            ..page
        };
        assert!(show.append(4, last));
        assert_eq!(show.episodes.len(), 8);
        assert_eq!(show.next_offset, None);
        assert_eq!(show.tracks().len(), 8);
    }

    #[test]
    fn reads_one_episode_or_chapter() {
        let reply =
            json!({"episodeUnionV2": episode_data("00000000000000000000a1", "IN_PROGRESS", 1_000)});
        let episode = parse_episode(&reply).unwrap();
        assert_eq!(episode.uri, "spotify:episode:00000000000000000000a1");
        assert_eq!(episode.played, Played::InProgress(1_000));
        assert_eq!(episode.uid, None);

        let restricted = json!({"episodeUnionV2": {"__typename": "RestrictedContent"}});
        assert_eq!(
            parse_episode(&restricted)
                .unwrap_err()
                .downcast_ref::<Unavailable>(),
            Some(&Unavailable::Restricted)
        );
        let missing = json!({"episodeUnionV2": {"__typename": "NotFound"}});
        assert_eq!(
            parse_episode(&missing)
                .unwrap_err()
                .downcast_ref::<Unavailable>(),
            Some(&Unavailable::Missing)
        );
    }

    #[test]
    fn reads_played_states_in_order() {
        let reply = json!({"lookup": [{"__typename": "PodcastOrAudiobookResponseWrapper", "data": {
            "__typename": "Podcast",
            "episodesV2": {"__typename": "ContextEpisodePage", "items": [
                {"entity": {"data": {"__typename": "Episode", "playedState": {"playPositionMilliseconds": 0, "state": "NOT_STARTED"}}}},
                {"entity": {"data": {"__typename": "Episode", "playedState": {"playPositionMilliseconds": 70_000, "state": "IN_PROGRESS"}}}},
                {"entity": {"data": {"__typename": "Episode", "playedState": {"playPositionMilliseconds": 0, "state": "COMPLETED"}}}},
                {"entity": {"data": {"__typename": "RestrictedContent"}}}
            ]}
        }}]});
        assert_eq!(
            parse_played_states(&reply),
            vec![
                Played::NotStarted,
                Played::InProgress(70_000),
                Played::Completed,
                Played::NotStarted
            ]
        );
        assert!(parse_played_states(&json!({})).is_empty());
    }

    #[test]
    fn builds_requests() {
        assert_eq!(
            show_variables(SHOW),
            json!({"uri": SHOW, "includeContentCapabilityTrait": true, "includeEpisodeContentRatingsV2": false})
        );
        let (operation, variables) = page_request(SHOW, ShowKind::Podcast, 50, 50);
        assert_eq!(operation, SHOW_EPISODES);
        assert_eq!(
            variables,
            json!({"uri": SHOW, "offset": 50, "limit": 50, "includeEpisodeContentRatingsV2": false})
        );
        let (operation, variables) = page_request(BOOK, ShowKind::Audiobook, 0, 50);
        assert_eq!(operation, BOOK_CHAPTERS);
        assert_eq!(variables, json!({"uri": BOOK, "offset": 0, "limit": 50}));
        assert_eq!(
            episode_variables("spotify:episode:00000000000000000000a1"),
            json!({"uri": "spotify:episode:00000000000000000000a1", "includeEpisodeContentRatingsV2": false})
        );
    }

    #[test]
    fn marks_episodes_through_the_finished_set() {
        let uris = vec!["spotify:episode:00000000000000000000a1".to_owned()];
        let body = collection::write_body(
            "someone",
            collection::MARKED_AS_FINISHED,
            &played_writes(&uris, true),
        );
        assert_eq!(
            body,
            json!({"username": "someone", "set": "markedasfinished", "items": [{"uri": "spotify:episode:00000000000000000000a1"}]})
        );
        let body = collection::write_body(
            "someone",
            collection::MARKED_AS_FINISHED,
            &played_writes(&uris, false),
        );
        assert_eq!(
            body,
            json!({"username": "someone", "set": "markedasfinished", "items": [{"uri": "spotify:episode:00000000000000000000a1", "is_removed": true}]})
        );
    }

    #[test]
    fn applies_marks_over_server_state() {
        assert_eq!(effective(Played::NotStarted, Some(true)), Played::Completed);
        assert_eq!(
            effective(Played::InProgress(5), Some(true)),
            Played::Completed
        );
        assert_eq!(
            effective(Played::Completed, Some(false)),
            Played::NotStarted
        );
        assert_eq!(
            effective(Played::InProgress(5), Some(false)),
            Played::InProgress(5)
        );
        assert_eq!(effective(Played::Completed, None), Played::Completed);
    }

    #[test]
    fn picks_the_episode_to_play_or_resume() {
        use Availability::{PaymentRequired, Playable, Restricted};
        use Played::{Completed, InProgress, NotStarted};
        let items = [
            (NotStarted, Restricted),
            (Completed, Playable),
            (NotStarted, Playable),
            (InProgress(9), Playable),
        ];
        assert_eq!(resume_index(Order::Episodic, &items), Some((3, true)));
        let items = [
            (InProgress(9), Restricted),
            (Completed, Playable),
            (NotStarted, Playable),
        ];
        assert_eq!(resume_index(Order::Episodic, &items), Some((1, false)));
        assert_eq!(resume_index(Order::Recent, &items), Some((1, false)));
        assert_eq!(resume_index(Order::Sequential, &items), Some((2, false)));
        let locked = [(NotStarted, PaymentRequired), (NotStarted, Restricted)];
        assert_eq!(resume_index(Order::Episodic, &locked), None);
        assert_eq!(resume_index(Order::Episodic, &[]), None);
    }

    #[test]
    fn turns_html_into_text() {
        assert_eq!(
            description_text("<p>One<br/>Two</p><p>Three &amp; <b>four</b>&nbsp;&#8217;&#x27;</p>"),
            "One\nTwo\n\nThree & four ’'"
        );
        assert_eq!(
            description_text("<ul><li>A</li><li>B</li></ul>"),
            "• A\n• B"
        );
        assert_eq!(description_text("<p></p><p></p>   "), "");
        assert_eq!(description_text("a < b"), "a < b");
        assert_eq!(decode_entities("&unknown; & &amp"), "&unknown; & &amp");
    }

    #[test]
    fn labels_dates_lengths_and_ratings() {
        assert_eq!(
            release_label("2026-11-23T06:00:00Z", 2026).as_deref(),
            Some("Nov 23")
        );
        assert_eq!(
            release_label("2025-01-02T06:00:00Z", 2026).as_deref(),
            Some("Jan 2, 2025")
        );
        assert_eq!(release_label("bad", 2026), None);
        assert_eq!(year_of(0), 1970);
        assert_eq!(year_of(951_782_400_000), 2000);
        assert_eq!(year_of(1_764_000_000_000), 2025);
        assert_eq!(year_of(1_767_225_599_999), 2025);
        assert_eq!(year_of(1_767_225_600_000), 2026);
        assert_eq!(
            length_label(5_655_661, Played::NotStarted).as_deref(),
            Some("1 hr 34 min")
        );
        assert_eq!(
            length_label(5_655_661, Played::InProgress(3_600_000)).as_deref(),
            Some("34 min left")
        );
        assert_eq!(
            length_label(60_000, Played::InProgress(59_900)).as_deref(),
            Some("1 sec left")
        );
        assert_eq!(
            length_label(2_040_000, Played::Completed).as_deref(),
            Some("34 min")
        );
        assert_eq!(compact_count(203), "203");
        assert_eq!(compact_count(1_260), "1.3K");
        assert_eq!(compact_count(2_000), "2K");
        assert_eq!(compact_count(88_073), "88K");
        assert_eq!(compact_count(1_200_000), "1.2M");
        assert_eq!(
            rating_label(Rating {
                average: 4.6068,
                count: 88073
            }),
            "★ 4.6 (88K)"
        );
    }

    #[test]
    fn shortens_descriptions_for_rows() {
        let episode = Episode {
            description: format!("Line one\n\nLine two {}", "word ".repeat(100)),
            ..Episode::default()
        };
        let summary = episode.summary();
        assert!(summary.starts_with("Line one Line two word"));
        assert!(summary.ends_with('…'));
        assert!(summary.len() <= 290);
        assert!(!summary.contains('\n'));
    }
}
