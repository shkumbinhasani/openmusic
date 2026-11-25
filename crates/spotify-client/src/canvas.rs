use anyhow::{Context as _, Result};
use prost::Message;
use serde_json::{Value, json};
use spotify_proto::{
    canvaz::cache::{Type, entity_canvaz_response::Canvaz},
    extendedmetadata::ExtensionKind,
};

use crate::{
    api::Api,
    catalog,
    pathfinder::{self, Operation},
};

pub const CANVAS: Operation = Operation::query(
    "canvas",
    "575138ab27cd5c1b3e54da54d0a7cc8d85485402de26340c2145f0f6bb5e7a9f",
);

const HOSTS: [&str; 2] = ["scdn.co", "spotifycdn.com"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Looping,
    LoopingRandom,
    Gif,
}

impl Kind {
    pub const ALL: [Self; 5] = [
        Self::Image,
        Self::Video,
        Self::Looping,
        Self::LoopingRandom,
        Self::Gif,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Image => "IMAGE",
            Self::Video => "VIDEO",
            Self::Looping => "VIDEO_LOOPING",
            Self::LoopingRandom => "VIDEO_LOOPING_RANDOM",
            Self::Gif => "GIF",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    fn from_proto(value: i32) -> Option<Self> {
        Some(match Type::try_from(value).ok()? {
            Type::Image => Self::Image,
            Type::Video => Self::Video,
            Type::VideoLooping => Self::Looping,
            Type::VideoLoopingRandom => Self::LoopingRandom,
            Type::Gif => Self::Gif,
        })
    }

    pub fn moving(self) -> bool {
        matches!(self, Self::Video | Self::Looping | Self::LoopingRandom)
    }

    pub fn random_start(self) -> bool {
        self == Self::LoopingRandom
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Canvas {
    pub track_uri: String,
    pub url: String,
    pub kind: Kind,
    pub explicit: bool,
    pub uri: Option<String>,
    pub artist: Option<String>,
}

pub fn allowed(url: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            HOSTS.iter().any(|domain| {
                host == *domain
                    || host
                        .strip_suffix(domain)
                        .is_some_and(|rest| rest.ends_with('.'))
            })
        })
}

pub fn is_track(uri: &str) -> bool {
    matches!(
        uri.split(':').collect::<Vec<_>>().as_slice(),
        ["spotify", "track", id] if !id.is_empty()
    )
}

fn text(value: String) -> Option<String> {
    Some(value).filter(|value| !value.is_empty())
}

fn build(
    track_uri: &str,
    url: &str,
    kind: Option<Kind>,
    explicit: bool,
    uri: Option<String>,
    artist: Option<String>,
) -> Option<Canvas> {
    if !allowed(url) {
        return None;
    }
    Some(Canvas {
        track_uri: track_uri.to_owned(),
        url: url.to_owned(),
        kind: kind?,
        explicit,
        uri: uri.and_then(text),
        artist: artist.and_then(text),
    })
}

pub fn decode(track_uri: &str, bytes: &[u8]) -> Option<Canvas> {
    let canvaz = Canvaz::decode(bytes).ok()?;
    build(
        track_uri,
        &canvaz.url,
        Kind::from_proto(canvaz.r#type),
        canvaz.explicit,
        Some(canvaz.canvas_uri),
        canvaz.artist.map(|artist| artist.name),
    )
}

pub fn parse(track_uri: &str, data: &Value) -> Option<Canvas> {
    let canvas = data.get("trackUnion")?.get("canvas")?;
    let field = |name: &str| canvas.get(name).and_then(Value::as_str).map(str::to_owned);
    build(
        track_uri,
        &field("url")?,
        field("type").as_deref().and_then(Kind::from_name),
        canvas
            .get("explicit")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        field("uri"),
        canvas
            .pointer("/artist/name")
            .and_then(Value::as_str)
            .map(str::to_owned),
    )
}

pub fn variables(track_uri: &str) -> Value {
    json!({ "trackUri": track_uri })
}

async fn from_metadata(api: &Api, track_uri: &str) -> Result<Option<Canvas>> {
    let found = catalog::extended(api, &[(track_uri.to_owned(), ExtensionKind::CanvasV1)]).await?;
    Ok(found
        .iter()
        .find(|(uri, _)| uri == track_uri)
        .and_then(|(_, bytes)| decode(track_uri, bytes)))
}

async fn from_graphql(api: &Api, track_uri: &str) -> Result<Option<Canvas>> {
    let data = pathfinder::query(api, CANVAS, variables(track_uri)).await?;
    Ok(parse(track_uri, &data))
}

pub async fn canvas(api: &Api, track_uri: &str) -> Result<Option<Canvas>> {
    if !is_track(track_uri) {
        return Ok(None);
    }
    match from_metadata(api, track_uri).await {
        Ok(found) => Ok(found),
        Err(_) => from_graphql(api, track_uri)
            .await
            .context("Spotify didn’t answer the Canvas lookup"),
    }
}

#[cfg(test)]
mod tests {
    use spotify_proto::canvaz::cache::Artist;

    use super::*;

    const TRACK: &str = "spotify:track:00000000000000000000t1";
    const URL: &str = "https://canvaz.scdn.co/upload/artist/0000/video/0000.cnvs.mp4";

    fn canvaz(kind: Type, url: &str) -> Vec<u8> {
        Canvaz {
            id: "0000".into(),
            url: url.into(),
            file_id: "0000".into(),
            r#type: kind as i32,
            entity_uri: TRACK.into(),
            artist: Some(Artist {
                uri: "spotify:artist:00000000000000000000a1".into(),
                name: "An artist".into(),
                avatar: String::new(),
            }),
            explicit: true,
            uploaded_by: "artist".into(),
            canvas_uri: "spotify:canvas:00000000000000000000c1".into(),
            ..Canvaz::default()
        }
        .encode_to_vec()
    }

    #[test]
    fn decodes_extended_metadata() {
        let canvas = decode(TRACK, &canvaz(Type::VideoLoopingRandom, URL)).unwrap();
        assert_eq!(
            canvas,
            Canvas {
                track_uri: TRACK.into(),
                url: URL.into(),
                kind: Kind::LoopingRandom,
                explicit: true,
                uri: Some("spotify:canvas:00000000000000000000c1".into()),
                artist: Some("An artist".into()),
            }
        );
        assert!(canvas.kind.moving());
        assert!(canvas.kind.random_start());
        assert_eq!(decode(TRACK, &[]), None);
        assert_eq!(
            decode(TRACK, &canvaz(Type::Video, "http://canvaz.scdn.co/x.mp4")),
            None
        );
        let image = decode(TRACK, &canvaz(Type::Image, "https://i.scdn.co/image/0000")).unwrap();
        assert_eq!(image.kind, Kind::Image);
        assert!(!image.kind.moving());
    }

    #[test]
    fn parses_the_graphql_reply() {
        let data = json!({
            "trackUnion": {
                "__typename": "Track",
                "uri": TRACK,
                "canvas": {
                    "fileId": "0000",
                    "type": "VIDEO_LOOPING",
                    "uri": "spotify:canvas:00000000000000000000c1",
                    "url": URL
                }
            }
        });
        let canvas = parse(TRACK, &data).unwrap();
        assert_eq!(canvas.kind, Kind::Looping);
        assert_eq!(canvas.url, URL);
        assert!(!canvas.explicit);
        assert_eq!(canvas.artist, None);
        assert!(!canvas.kind.random_start());

        let none = json!({ "trackUnion": { "__typename": "Track", "uri": TRACK, "canvas": null } });
        assert_eq!(parse(TRACK, &none), None);
        let odd = json!({ "trackUnion": { "canvas": { "type": "HOLOGRAM", "url": URL } } });
        assert_eq!(parse(TRACK, &odd), None);
    }

    #[test]
    fn plays_only_from_spotify_hosts() {
        assert!(allowed(URL));
        assert!(allowed("https://scdn.co/x.mp4"));
        assert!(allowed("https://video-ak.cdn.spotifycdn.com/x.mp4"));
        assert!(!allowed("http://canvaz.scdn.co/x.mp4"));
        assert!(!allowed("https://example.com/x.mp4"));
        assert!(!allowed("https://scdn.co.example.com/x.mp4"));
        assert!(!allowed("https://notscdn.co/x.mp4"));
        assert!(!allowed("not a url"));
    }

    #[test]
    fn names_every_kind_and_asks_only_for_tracks() {
        for kind in Kind::ALL {
            assert_eq!(Kind::from_name(kind.name()), Some(kind));
        }
        assert!(is_track(TRACK));
        assert!(!is_track("spotify:episode:00000000000000000000e1"));
        assert!(!is_track("spotify:track:"));
        assert!(!is_track("spotify:local:a:b:c:1"));
        assert_eq!(
            pathfinder::body(CANVAS, variables(TRACK)),
            json!({
                "variables": { "trackUri": TRACK },
                "operationName": "canvas",
                "extensions": {
                    "persistedQuery": {
                        "version": 1,
                        "sha256Hash": "575138ab27cd5c1b3e54da54d0a7cc8d85485402de26340c2145f0f6bb5e7a9f"
                    }
                }
            })
        );
    }
}
