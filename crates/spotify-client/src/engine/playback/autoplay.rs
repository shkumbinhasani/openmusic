use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::{Value, json};
use spotify_proto::{
    Message as _, connectstate::ProvidedTrack, spotify::player::proto::AutoplayContextRequest,
};

use super::{Playing, gain::SIDE_LOOKUP, narration::is_narration, source::is_spoken};
use crate::{
    api::{Api, Body},
    connect::Repeat,
    engine::Engine,
};

pub const ORIGIN: &str = "autoplay";
pub const DESCRIPTION: &str = "Autoplay";
const PROVIDER: &str = "autoplay";
const RESOLVE: &str = "/context-resolve/v1/autoplay";
const DELIMITERS: [&str; 2] = ["spotify:delimiter", "spotify:meta:delimiter"];
const RECENT: usize = 10;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Upcoming {
    Context,
    Autoplay,
    End,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Ahead {
    pub(super) upcoming: Upcoming,
    pub(super) in_autoplay: bool,
}

impl Ahead {
    pub(super) fn enters_autoplay(self) -> bool {
        self.upcoming == Upcoming::Autoplay && !self.in_autoplay
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ending {
    Continue,
    Hold,
    Autoplay,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Seed {
    pub(super) context: String,
    pub(super) station: String,
    pub(super) recent: Vec<String>,
}

fn hidden(track: &ProvidedTrack) -> bool {
    track
        .metadata
        .get("hidden")
        .is_some_and(|value| value == "true")
}

pub(super) fn ahead(
    current: Option<(&str, &str)>,
    provider: Option<&str>,
    next: &[ProvidedTrack],
    uri: &str,
    uid: Option<&str>,
) -> Ahead {
    let same = current.is_some_and(|(current_uri, current_uid)| {
        current_uri == uri || uid.is_some_and(|uid| !uid.is_empty() && uid == current_uid)
    });
    if !same {
        return Ahead::default();
    }
    let mut upcoming = Upcoming::End;
    for track in next {
        if DELIMITERS.contains(&track.uri.as_str()) {
            break;
        }
        if track.uri.starts_with("spotify:meta:") || hidden(track) {
            continue;
        }
        upcoming = if track.provider == PROVIDER {
            Upcoming::Autoplay
        } else {
            Upcoming::Context
        };
        break;
    }
    Ahead {
        upcoming,
        in_autoplay: provider == Some(PROVIDER),
    }
}

pub(super) fn context_ended(next: Option<(usize, bool)>, index: usize, upcoming: Upcoming) -> bool {
    match next {
        None => !matches!(upcoming, Upcoming::Context | Upcoming::Autoplay),
        Some((_, false)) => false,
        Some((target, true)) => match upcoming {
            Upcoming::End => true,
            Upcoming::Unknown => target <= index,
            Upcoming::Context | Upcoming::Autoplay => false,
        },
    }
}

fn id_of(uri: &str) -> Option<&str> {
    uri.rsplit(':')
        .next()
        .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
}

fn context_kind(context: &str) -> Option<(&'static str, &str)> {
    let parts: Vec<&str> = context.split(':').collect();
    match parts.as_slice() {
        ["spotify", "album", id] => Some(("album", id)),
        ["spotify", "playlist", id] | ["spotify", "user", _, "playlist", id] => {
            Some(("playlist", id))
        }
        ["spotify", "artist", id] => Some(("artist", id)),
        _ => None,
    }
}

fn endless(context: &str) -> bool {
    context.starts_with("spotify:station:") || context.starts_with("spotify:radio:")
}

fn collection(context: &str) -> bool {
    context.starts_with("spotify:collection")
        || (context.starts_with("spotify:user:") && context.contains(":collection"))
}

pub(super) fn seed(context: Option<&str>, current: &str, mut recent: Vec<String>) -> Option<Seed> {
    let context = context.filter(|context| !context.is_empty());
    if context.is_some_and(endless) || is_spoken(current) {
        return None;
    }
    recent.retain(|uri| uri.starts_with("spotify:track:") && uri != current);
    if current.starts_with("spotify:track:") {
        recent.push(current.to_owned());
    }
    let skip = recent.len().saturating_sub(RECENT);
    recent.drain(..skip);
    if let Some((kind, id)) = context.and_then(context_kind)
        && id_of(id).is_some()
    {
        return Some(Seed {
            context: format!("spotify:{kind}:{id}"),
            station: format!("spotify:station:{kind}:{id}"),
            recent,
        });
    }
    if !current.starts_with("spotify:track:") {
        return None;
    }
    let id = id_of(current)?;
    Some(Seed {
        context: context
            .filter(|context| collection(context))
            .unwrap_or(current)
            .to_owned(),
        station: format!("spotify:station:track:{id}"),
        recent,
    })
}

pub(super) fn request_body(seed: &Seed) -> Vec<u8> {
    AutoplayContextRequest {
        context_uri: seed.context.clone(),
        recent_track_uri: seed.recent.clone(),
        is_video: None,
    }
    .encode_to_vec()
}

pub(super) fn resolved(value: &Value) -> Option<(String, String)> {
    let uri = value["uri"]
        .as_str()
        .filter(|uri| uri.starts_with("spotify:") && !uri.contains(char::is_whitespace))?;
    let url = value["url"]
        .as_str()
        .filter(|url| url.starts_with("context://"))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("context://{uri}"));
    Some((uri.to_owned(), url))
}

pub(super) fn command(uri: &str, url: &str) -> Value {
    json!({
        "endpoint": "play",
        "context": {
            "uri": uri,
            "url": url,
            "metadata": {"context_description": DESCRIPTION},
        },
        "play_origin": {"feature_identifier": ORIGIN},
        "options": {"skip_to": {"track_index": 0}},
    })
}

async fn resolve(api: &Api, seed: &Seed) -> Result<(String, String)> {
    let bytes = api
        .request(
            Method::POST,
            RESOLVE,
            Body::Bytes(request_body(seed), "application/x-protobuf"),
            "application/json",
        )
        .await?;
    let value: Value = serde_json::from_slice(&bytes).context("Unreadable autoplay reply.")?;
    resolved(&value).context("Spotify sent no autoplay station.")
}

impl Playing {
    pub(super) fn uid(&self) -> Option<&str> {
        self.state()?["track_uid"]
            .as_str()
            .filter(|uid| !uid.is_empty())
    }
}

impl Engine {
    fn ahead(&self) -> Ahead {
        let (Some(playing), Some(playback)) = (self.playing.as_ref(), self.playback_here()) else {
            return Ahead::default();
        };
        let current = playback
            .track
            .as_ref()
            .map(|track| (track.uri.as_str(), track.uid.as_deref().unwrap_or_default()));
        ahead(
            current,
            playback.provider.as_deref(),
            &playback.queue.raw_next,
            playing.uri(),
            playing.uid(),
        )
    }

    fn repeats(&self) -> bool {
        let Some(playing) = self.playing.as_ref() else {
            return false;
        };
        playing.options().1 != Repeat::Off
            || self
                .playback_here()
                .is_some_and(|playback| playback.repeat != Repeat::Off)
    }

    fn autoplay_seed(&self) -> Option<Seed> {
        let playing = self.playing.as_ref()?;
        let track = playing.track()?;
        if is_narration(track) {
            return None;
        }
        let here = self.playback_here();
        let context = here
            .and_then(|playback| playback.context_uri.clone())
            .or_else(|| playing.context_uri());
        let recent = here
            .map(|playback| {
                playback
                    .queue
                    .raw_prev
                    .iter()
                    .map(|track| track.uri.clone())
                    .collect()
            })
            .unwrap_or_default();
        seed(context.as_deref(), playing.uri(), recent)
    }

    pub(super) fn ending(&self, transition: &str) -> Ending {
        let Some(playing) = self.playing.as_ref() else {
            return Ending::Continue;
        };
        if self.local_active()
            || self.remote_target().is_some()
            || (transition == "advance" && self.audio.sleep.ends_track())
        {
            return Ending::Continue;
        }
        let next = playing.transition(transition);
        let ahead = self.ahead();
        let autoplay = self.audio.settings.autoplay;
        if context_ended(next, playing.index, ahead.upcoming) {
            if autoplay && !self.repeats() && self.autoplay_seed().is_some() {
                return Ending::Autoplay;
            }
            return Ending::Continue;
        }
        if !autoplay && ahead.enters_autoplay() && next.is_some_and(|(_, paused)| !paused) {
            return Ending::Hold;
        }
        Ending::Continue
    }

    pub(super) async fn start_autoplay(&mut self) -> bool {
        let Some(seed) = self.autoplay_seed() else {
            return false;
        };
        let Some(old) = self.playing.as_mut().map(|playing| {
            playing.paused = true;
            playing.position_ms = playing.duration_ms.max(playing.position_ms);
            playing.generation
        }) else {
            return false;
        };
        self.report("track_data_finalized").await;
        if self.playing.as_ref().map(|playing| playing.generation) != Some(old) {
            return true;
        }
        let found = tokio::time::timeout(SIDE_LOOKUP, resolve(&self.api, &seed)).await;
        let (uri, url, how) = match found {
            Ok(Ok((uri, url))) => (uri, url, "resolved"),
            _ => (
                seed.station.clone(),
                format!("context://{}", seed.station),
                "station",
            ),
        };
        let own = self.device_id.clone();
        match self.send_command(&own, command(&uri, &url)).await {
            Ok(_) => {
                self.audio.note(format!("autoplay {how} {uri}"));
                true
            }
            Err(error) => {
                self.audio.note(format!("autoplay failed {error}"));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn item(uri: &str, provider: &str) -> ProvidedTrack {
        ProvidedTrack {
            uri: uri.into(),
            uid: format!("uid-{uri}"),
            provider: provider.into(),
            ..ProvidedTrack::default()
        }
    }

    fn hidden_item(uri: &str) -> ProvidedTrack {
        ProvidedTrack {
            metadata: HashMap::from([("hidden".to_owned(), "true".to_owned())]),
            ..item(uri, "context")
        }
    }

    const NOW: &str = "spotify:track:0000000000000000000001";
    const NEXT: &str = "spotify:track:0000000000000000000002";

    #[test]
    fn reads_what_comes_next_from_the_cluster() {
        let current = Some((NOW, "u1"));
        let look = |next: &[ProvidedTrack]| ahead(current, Some("context"), next, NOW, None);
        assert_eq!(look(&[item(NEXT, "context")]).upcoming, Upcoming::Context);
        assert_eq!(look(&[item(NEXT, "queue")]).upcoming, Upcoming::Context);
        assert_eq!(look(&[item(NEXT, "autoplay")]).upcoming, Upcoming::Autoplay);
        assert!(look(&[item(NEXT, "autoplay")]).enters_autoplay());
        assert_eq!(
            look(&[item("spotify:delimiter", "context"), item(NEXT, "context")]).upcoming,
            Upcoming::End
        );
        assert_eq!(
            look(&[item("spotify:meta:delimiter", "context")]).upcoming,
            Upcoming::End
        );
        assert_eq!(
            look(&[
                item("spotify:meta:page:1", "context"),
                hidden_item(NEXT),
                item(NEXT, "autoplay")
            ])
            .upcoming,
            Upcoming::Autoplay
        );
        assert_eq!(look(&[]).upcoming, Upcoming::End);

        let in_autoplay = ahead(
            current,
            Some("autoplay"),
            &[item(NEXT, "autoplay")],
            NOW,
            None,
        );
        assert!(in_autoplay.in_autoplay && !in_autoplay.enters_autoplay());

        let other = ahead(Some((NEXT, "u2")), None, &[], NOW, None);
        assert_eq!(other.upcoming, Upcoming::Unknown);
        let by_uid = ahead(Some((NEXT, "u1")), None, &[], NOW, Some("u1"));
        assert_eq!(by_uid.upcoming, Upcoming::End);
        assert_eq!(ahead(None, None, &[], NOW, None), Ahead::default());
    }

    #[test]
    fn decides_when_the_context_ends() {
        assert!(context_ended(None, 4, Upcoming::Unknown));
        assert!(context_ended(None, 4, Upcoming::End));
        assert!(!context_ended(None, 4, Upcoming::Context));
        assert!(!context_ended(None, 4, Upcoming::Autoplay));
        assert!(!context_ended(Some((5, false)), 4, Upcoming::End));
        assert!(context_ended(Some((0, true)), 4, Upcoming::End));
        assert!(context_ended(Some((0, true)), 4, Upcoming::Unknown));
        assert!(!context_ended(Some((5, true)), 4, Upcoming::Unknown));
        assert!(!context_ended(Some((0, true)), 4, Upcoming::Context));
        assert!(!context_ended(Some((5, true)), 4, Upcoming::Autoplay));
    }

    #[test]
    fn seeds_a_station_from_the_context() {
        let album = seed(
            Some("spotify:album:0000000000000000000003"),
            NOW,
            vec![NEXT.into(), "spotify:episode:x".into(), NOW.into()],
        )
        .unwrap();
        assert_eq!(album.context, "spotify:album:0000000000000000000003");
        assert_eq!(
            album.station,
            "spotify:station:album:0000000000000000000003"
        );
        assert_eq!(album.recent, vec![NEXT.to_owned(), NOW.to_owned()]);

        let playlist = seed(
            Some("spotify:user:someone:playlist:0000000000000000000004"),
            NOW,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(playlist.context, "spotify:playlist:0000000000000000000004");
        assert_eq!(
            playlist.station,
            "spotify:station:playlist:0000000000000000000004"
        );

        let app = seed(
            Some(&format!("spotify:app:openmusic:{NOW}")),
            NOW,
            Vec::new(),
        )
        .unwrap();
        assert_eq!(app.context, NOW);
        assert_eq!(app.station, "spotify:station:track:0000000000000000000001");

        let liked = seed(Some("spotify:user:someone:collection"), NOW, Vec::new()).unwrap();
        assert_eq!(liked.context, "spotify:user:someone:collection");
        assert_eq!(
            liked.station,
            "spotify:station:track:0000000000000000000001"
        );

        assert_eq!(seed(None, NOW, Vec::new()).unwrap().context, NOW);
        assert!(seed(Some("spotify:station:album:x"), NOW, Vec::new()).is_none());
        assert!(
            seed(
                Some("spotify:show:0000000000000000000005"),
                "spotify:episode:0000000000000000000006",
                Vec::new()
            )
            .is_none()
        );
        assert!(seed(None, "spotify:local:a:b:c:1", Vec::new()).is_none());

        let long: Vec<String> = (0..15)
            .map(|index| format!("spotify:track:{index:022}"))
            .collect();
        let capped = seed(Some("spotify:album:0000000000000000000003"), NOW, long).unwrap();
        assert_eq!(capped.recent.len(), RECENT);
        assert_eq!(capped.recent.last().map(String::as_str), Some(NOW));
    }

    #[test]
    fn asks_spotify_for_the_autoplay_station() {
        let seed = Seed {
            context: "spotify:album:0000000000000000000003".into(),
            station: "spotify:station:album:0000000000000000000003".into(),
            recent: vec![NOW.into()],
        };
        let body = request_body(&seed);
        let decoded = AutoplayContextRequest::decode(body.as_slice()).unwrap();
        assert_eq!(decoded.context_uri, seed.context);
        assert_eq!(decoded.recent_track_uri, seed.recent);
        assert_eq!(decoded.is_video, None);
        assert_eq!(body[0], 0x0a);

        let reply = json!({
            "uri": "spotify:station:album:0000000000000000000003",
            "url": "context://spotify:station:album:0000000000000000000003?prev_uris=0000000000000000000001&autoplay=true",
            "restrictions": {"disallow_toggling_shuffle_reasons": ["radio"]},
            "pages": [{"tracks": [{"uri": NEXT, "uid": "e4fd570c856fe85d"}]}]
        });
        let (uri, url) = resolved(&reply).unwrap();
        assert_eq!(uri, "spotify:station:album:0000000000000000000003");
        assert!(url.ends_with("&autoplay=true"));
        let (_, url) = resolved(&json!({"uri": "spotify:station:track:1"})).unwrap();
        assert_eq!(url, "context://spotify:station:track:1");
        assert!(resolved(&json!({"url": "context://x"})).is_none());
        assert!(resolved(&json!({"uri": ""})).is_none());
    }

    #[test]
    fn plays_the_station_as_autoplay() {
        assert_eq!(
            command(
                "spotify:station:track:1",
                "context://spotify:station:track:1"
            ),
            json!({
                "endpoint": "play",
                "context": {
                    "uri": "spotify:station:track:1",
                    "url": "context://spotify:station:track:1",
                    "metadata": {"context_description": "Autoplay"}
                },
                "play_origin": {"feature_identifier": "autoplay"},
                "options": {"skip_to": {"track_index": 0}}
            })
        );
    }
}
