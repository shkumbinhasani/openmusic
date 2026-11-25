use std::collections::HashMap;

use anyhow::{Result, bail};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use spotify_proto::connectstate::ProvidedTrack;

use crate::{
    api::{Api, Body, HttpStatus},
    connect::Playback,
};

pub const DJ_PLAYLIST: &str = "spotify:playlist:37i9dQZF1EYkqdzj48dyYq";
pub const DJ_PLAYLISTS: [&str; 6] = [
    DJ_PLAYLIST,
    "spotify:playlist:37i9dQZF1EYkqdzj8OSOas",
    "spotify:playlist:37i9dQZF1EYkqdzjdvy3mw",
    "spotify:playlist:37i9dQZF1EYkqdzjicdiyA",
    "spotify:playlist:37i9dQZF1EYkqdzjmSSxKE",
    "spotify:playlist:37i9dQZF1EYkqdzjrzxMWI",
];
pub const FEATURE: &str = "your_dj_home_card";
pub const REFERRER: &str = "home";
pub const JUMP: &str = "jump";
pub const CONTEXT_URL: &str = "lexicon_context_url";
pub const LEXICON: &str = "your_dj";
pub const NARRATION_COLOR: &str = "#0A59B8";
const DELIMITERS: [&str; 2] = ["spotify:delimiter", "spotify:meta:delimiter"];
const NARRATION_KEYS: [&str; 2] = ["narration.intro.ssml", "narration.jump.ssml"];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dj {
    pub name: String,
    pub context_url: String,
    pub cover: Option<String>,
}

pub fn metadata_path() -> String {
    let id = DJ_PLAYLIST.rsplit(':').next().unwrap_or_default();
    format!("/playlist/v2/playlist/{id}?decorate=attributes,length,owner&from=0&length=0")
}

fn format_attribute<'a>(attributes: &'a Value, key: &str) -> Option<&'a str> {
    attributes["formatAttributes"]
        .as_array()?
        .iter()
        .find(|attribute| attribute["key"] == key)?["value"]
        .as_str()
        .filter(|value| !value.is_empty())
}

fn cover(attributes: &Value) -> Option<String> {
    let pictures = attributes["pictureSize"].as_array()?;
    pictures
        .iter()
        .find(|picture| picture["targetName"] == "default")
        .or_else(|| pictures.first())?["url"]
        .as_str()
        .map(str::to_owned)
}

pub fn parse(value: &Value) -> Option<Dj> {
    let attributes = &value["attributes"];
    let context_url = format_attribute(attributes, CONTEXT_URL)?.to_owned();
    Some(Dj {
        name: attributes["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .unwrap_or("DJ")
            .to_owned(),
        context_url,
        cover: cover(attributes),
    })
}

pub fn lookup_reply(status: StatusCode, body: &[u8]) -> Result<Option<Dj>> {
    if status == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !status.is_success() {
        bail!(HttpStatus(status));
    }
    Ok(serde_json::from_slice::<Value>(body)
        .ok()
        .as_ref()
        .and_then(parse))
}

pub async fn lookup(api: &Api) -> Result<Option<Dj>> {
    let (status, body) = api
        .send(
            Method::GET,
            &metadata_path(),
            Body::Empty,
            "application/json",
        )
        .await?;
    lookup_reply(status, &body)
}

pub fn is_dj_context(uri: Option<&str>, metadata: &HashMap<String, String>) -> bool {
    uri.is_some_and(|uri| DJ_PLAYLISTS.contains(&uri))
        || metadata.get("lexicon_set_type").map(String::as_str) == Some(LEXICON)
}

pub fn is_narration(uri: &str, metadata: &HashMap<String, String>) -> bool {
    uri.starts_with("spotify:media:")
        || metadata.get("is_narration").map(String::as_str) == Some("true")
}

pub fn in_dj(playback: &Playback) -> bool {
    is_dj_context(playback.context_uri.as_deref(), &playback.context_metadata)
}

pub fn narrating(playback: &Playback) -> bool {
    in_dj(playback)
        && playback
            .track
            .as_ref()
            .is_some_and(|track| is_narration(&track.uri, &playback.track_metadata))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Jump {
    Signal,
    SkipTo(Box<ProvidedTrack>),
}

fn upcoming(playback: &Playback) -> impl Iterator<Item = &ProvidedTrack> {
    playback
        .queue
        .raw_next
        .iter()
        .take_while(|track| !DELIMITERS.contains(&track.uri.as_str()))
}

fn filled(track: &ProvidedTrack, key: &str) -> bool {
    track
        .metadata
        .get(key)
        .is_some_and(|value| !value.is_empty() && value != "false")
}

pub fn jump(playback: &Playback) -> Option<Jump> {
    if playback.signals.iter().any(|signal| signal == JUMP) {
        return Some(Jump::Signal);
    }
    if !in_dj(playback) || playback.track.is_none() || narrating(playback) {
        return None;
    }
    upcoming(playback)
        .find(|track| filled(track, "jump_track"))
        .or_else(|| {
            upcoming(playback).find(|track| NARRATION_KEYS.iter().any(|key| filled(track, key)))
        })
        .map(|track| Jump::SkipTo(Box::new(track.clone())))
}

pub fn play_command(context_url: &str) -> Value {
    json!({
        "endpoint": "play",
        "context": { "uri": DJ_PLAYLIST, "url": context_url, "metadata": {} },
        "play_origin": { "feature_identifier": FEATURE, "referrer_identifier": REFERRER },
        "options": {},
    })
}

pub fn signal_command(id: &str) -> Value {
    json!({ "endpoint": "signal", "signal_id": id })
}

#[cfg(test)]
mod tests {
    use crate::connect::{Allowed, Queue, Repeat, Shuffle, SleepState};
    use crate::{Track, connect};

    use super::*;

    fn metadata(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn item(uri: &str, pairs: &[(&str, &str)]) -> ProvidedTrack {
        ProvidedTrack {
            uri: uri.into(),
            uid: format!("uid-{uri}"),
            metadata: metadata(pairs),
            ..Default::default()
        }
    }

    fn playback(context: &str, current: &str, next: Vec<ProvidedTrack>) -> Playback {
        Playback {
            active_device_id: Some("aaaa".into()),
            active_device_name: None,
            active_volume: None,
            own_device_id: "aaaa".into(),
            devices: Vec::new(),
            context_uri: Some(context.into()),
            context_title: None,
            context_metadata: HashMap::new(),
            session_id: String::new(),
            track: Some(Track::placeholder(current)),
            track_metadata: HashMap::new(),
            provider: None,
            playing: true,
            position_ms: 0,
            playback_speed: 1.,
            shuffle: Shuffle::Off,
            repeat: Repeat::Off,
            allowed: Allowed::default(),
            queue: Queue {
                raw_next: next,
                ..Queue::default()
            },
            signals: Vec::new(),
            sleep_timer: SleepState::Off,
            jam_mode: None,
        }
    }

    #[test]
    fn reads_the_dj_playlist() {
        let body = json!({"attributes": {
            "name": "DJ",
            "formatAttributes": [
                {"key": "lexicon_context_url", "value": "hm://lexicon-session-provider/context-resolve/v2/session?contextUri=spotify:playlist:37i9dQZF1EYkqdzj48dyYq"},
                {"key": "other", "value": "x"}
            ],
            "pictureSize": [{"targetName": "large", "url": "https://i.scdn.co/image/large"}, {"targetName": "default", "url": "https://i.scdn.co/image/dj"}]
        }});
        let dj = parse(&body).unwrap();
        assert_eq!(dj.name, "DJ");
        assert!(dj.context_url.starts_with("hm://lexicon-session-provider/"));
        assert_eq!(dj.cover.as_deref(), Some("https://i.scdn.co/image/dj"));
        assert_eq!(parse(&json!({"attributes": {"name": "DJ"}})), None);
        assert_eq!(lookup_reply(StatusCode::NOT_FOUND, b"").unwrap(), None);
        assert_eq!(
            lookup_reply(StatusCode::OK, &serde_json::to_vec(&body).unwrap()).unwrap(),
            Some(dj)
        );
        assert!(lookup_reply(StatusCode::BAD_GATEWAY, b"").is_err());
        assert_eq!(
            metadata_path(),
            "/playlist/v2/playlist/37i9dQZF1EYkqdzj48dyYq?decorate=attributes,length,owner&from=0&length=0"
        );
    }

    #[test]
    fn detects_dj_contexts_and_narration() {
        let empty = HashMap::new();
        assert!(is_dj_context(Some(DJ_PLAYLIST), &empty));
        assert!(is_dj_context(
            Some("spotify:playlist:37i9dQZF1EYkqdzjrzxMWI"),
            &empty
        ));
        assert!(is_dj_context(
            Some("spotify:station:x"),
            &metadata(&[("lexicon_set_type", "your_dj")])
        ));
        assert!(!is_dj_context(
            Some("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"),
            &empty
        ));
        assert!(!is_dj_context(None, &empty));
        assert!(is_narration("spotify:media:abc", &empty));
        assert!(is_narration(
            "spotify:track:1111111111111111111111",
            &metadata(&[("is_narration", "true")])
        ));
        assert!(!is_narration(
            "spotify:track:1111111111111111111111",
            &empty
        ));
    }

    #[test]
    fn finds_where_the_dj_jumps() {
        let song = "spotify:track:1111111111111111111111";
        let plain = item("spotify:track:2222222222222222222222", &[]);
        let intro = item(
            "spotify:track:3333333333333333333333",
            &[("narration.intro.ssml", "<speak/>")],
        );
        let marked = item(
            "spotify:track:4444444444444444444444",
            &[("jump_track", "true")],
        );
        let model = playback(
            DJ_PLAYLIST,
            song,
            vec![plain.clone(), intro.clone(), marked.clone()],
        );
        assert_eq!(jump(&model), Some(Jump::SkipTo(Box::new(marked.clone()))));
        let without_mark = playback(DJ_PLAYLIST, song, vec![plain.clone(), intro.clone()]);
        assert_eq!(
            jump(&without_mark),
            Some(Jump::SkipTo(Box::new(intro.clone())))
        );
        let behind_delimiter = playback(
            DJ_PLAYLIST,
            song,
            vec![
                plain.clone(),
                item("spotify:delimiter", &[]),
                marked.clone(),
            ],
        );
        assert_eq!(jump(&behind_delimiter), None);
        let mut narrating_now = playback(DJ_PLAYLIST, "spotify:media:talk", vec![marked.clone()]);
        assert!(narrating(&narrating_now));
        assert_eq!(jump(&narrating_now), None);
        narrating_now.signals = vec!["jump".into()];
        assert_eq!(jump(&narrating_now), Some(Jump::Signal));
        let elsewhere = playback(
            "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            song,
            vec![marked],
        );
        assert!(!in_dj(&elsewhere));
        assert_eq!(jump(&elsewhere), None);
    }

    #[test]
    fn builds_dj_commands() {
        let play = play_command("hm://lexicon/x");
        assert_eq!(play["endpoint"], "play");
        assert_eq!(play["context"]["uri"], DJ_PLAYLIST);
        assert_eq!(play["context"]["url"], "hm://lexicon/x");
        assert_eq!(play["play_origin"]["feature_identifier"], FEATURE);
        assert!(play["options"].get("skip_to").is_none());
        assert_eq!(
            signal_command(JUMP),
            json!({"endpoint": "signal", "signal_id": "jump"})
        );
        let wrapped = connect::envelope(signal_command(JUMP), "abc", None);
        assert_eq!(wrapped["command"]["signal_id"], "jump");
    }
}
