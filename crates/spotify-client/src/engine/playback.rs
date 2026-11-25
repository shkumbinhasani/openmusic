use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use reqwest::Method;
use serde_json::{Value, json};

use super::{Engine, GAIA_VOLUME};
use crate::{
    api::Body,
    catalog::{LocalTrack, Track, image_url},
    connect::{Repeat, Shuffle},
    playback_settings::LoudnessLevel,
    player::{FairPlay, Join, Media, PlayerEvent, Snapshot, Source},
};
use join::{Joins, Taken, choose_file};

mod join;

#[derive(Clone, Debug)]
pub enum Control {
    PlayLocal {
        tracks: Vec<LocalTrack>,
        index: usize,
    },
    Settings(AudioSettings),
    Report,
}

#[derive(Clone, Debug)]
pub enum Update {
    Report(Box<AudioReport>),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioSettings {
    pub gapless: bool,
    pub crossfade: bool,
    pub crossfade_ms: u32,
    pub normalize: bool,
    pub level: LoudnessLevel,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            gapless: true,
            crossfade: false,
            crossfade_ms: 0,
            normalize: true,
            level: LoudnessLevel::Normal,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReportItem {
    pub generation: u64,
    pub uri: String,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub paused: bool,
    pub format: String,
    pub bitrate: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReportPrepared {
    pub generation: u64,
    pub uri: String,
    pub join: Join,
    pub ready: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AudioReport {
    pub settings: AudioSettings,
    pub playing: Option<ReportItem>,
    pub prepared: Option<ReportPrepared>,
    pub joins: Vec<String>,
    pub player: Snapshot,
}

impl fmt::Display for AudioReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let settings = &self.settings;
        writeln!(
            f,
            "settings gapless={} crossfade={} crossfade_ms={} normalize={} level={}",
            settings.gapless,
            settings.crossfade,
            settings.crossfade_ms,
            settings.normalize,
            settings.level.name()
        )?;
        match &self.playing {
            Some(item) => writeln!(
                f,
                "playing generation={} uri={} position={} duration={} paused={} format={} bitrate={}",
                item.generation,
                item.uri,
                item.position_ms,
                item.duration_ms,
                item.paused,
                item.format,
                item.bitrate
            )?,
            None => writeln!(f, "playing none")?,
        }
        match &self.prepared {
            Some(prepared) => writeln!(
                f,
                "prepared generation={} uri={} join={} ready={}",
                prepared.generation,
                prepared.uri,
                prepared.join.name(),
                prepared.ready
            )?,
            None => writeln!(f, "prepared none")?,
        }
        let player = &self.player;
        writeln!(
            f,
            "player active={} master={:.3} playing={} ramp={}",
            player.active,
            player.master,
            player.playing,
            player
                .ramp
                .map_or_else(|| "none".to_owned(), |progress| format!("{progress:.2}"))
        )?;
        for (index, deck) in player.decks.iter().enumerate() {
            writeln!(
                f,
                "deck {index} items={} current={} fade={:.3} volume={:.3} rate={:.2}",
                deck.items
                    .iter()
                    .map(u64::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
                deck.current
                    .map_or_else(|| "-".to_owned(), |generation| generation.to_string()),
                deck.fade,
                deck.volume,
                deck.rate
            )?;
        }
        for join in &self.joins {
            writeln!(f, "join {join}")?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct State {
    settings: AudioSettings,
    updates: Vec<Update>,
    joins: Joins,
}

impl State {
    pub(super) fn take_updates(&mut self) -> Vec<Update> {
        std::mem::take(&mut self.updates)
    }
}

pub(super) struct Playing {
    machine: Value,
    index: usize,
    pub(super) paused: bool,
    generation: u64,
    format: Value,
    bitrate: u64,
    duration_ms: u32,
    pub(super) position_ms: u32,
    previous_position_ms: u32,
    started_reported: bool,
    threshold_reported: bool,
    last_progress: Instant,
}

impl Playing {
    fn new(machine: Value, index: usize, paused: bool, generation: u64, position_ms: u32) -> Self {
        Self {
            machine,
            index,
            paused,
            generation,
            format: Value::Null,
            bitrate: 0,
            duration_ms: 0,
            position_ms,
            previous_position_ms: 0,
            started_reported: false,
            threshold_reported: false,
            last_progress: Instant::now(),
        }
    }

    fn uri(&self) -> &str {
        self.track()
            .and_then(|track| track["metadata"]["uri"].as_str())
            .unwrap_or_default()
    }

    fn state(&self) -> Option<&Value> {
        self.machine["states"].get(self.index)
    }

    pub(super) fn reference(&self) -> Value {
        json!({
            "state_machine_id": self.machine["state_machine_id"],
            "state_id": self.state().map(|state| state["state_id"].clone()).unwrap_or(Value::Null),
            "paused": self.paused,
        })
    }

    fn track(&self) -> Option<&Value> {
        let index = self.state()?["track"].as_u64()? as usize;
        self.machine["tracks"].get(index)
    }

    pub(super) fn metadata(&self) -> Option<Track> {
        let track = self.track()?;
        let metadata = &track["metadata"];
        let uri = metadata["uri"].as_str()?.to_owned();
        Some(Track {
            uri,
            title: metadata["name"].as_str().unwrap_or_default().to_owned(),
            artist: metadata["authors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|author| author["name"].as_str())
                .collect::<Vec<_>>()
                .join(", "),
            album: metadata["group_name"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            duration_ms: if self.duration_ms > 0 {
                self.duration_ms
            } else {
                metadata["duration"].as_u64().unwrap_or_default() as u32
            },
            cover: metadata["images"]
                .as_array()
                .into_iter()
                .flatten()
                .min_by_key(|image| (image["width"].as_i64().unwrap_or(300) - 300).abs())
                .and_then(|image| image["url"].as_str())
                .map(image_url),
            artist_uri: metadata["authors"]
                .as_array()
                .and_then(|authors| authors.first())
                .and_then(|author| author["uri"].as_str())
                .map(str::to_owned),
            album_uri: metadata["group_uri"].as_str().map(str::to_owned),
            uid: self
                .state()
                .and_then(|state| state["track_uid"].as_str())
                .filter(|uid| !uid.is_empty())
                .map(str::to_owned),
            ..Track::default()
        })
    }

    fn transition(&self, name: &str) -> Option<(usize, bool)> {
        let transition = self.state()?.get("transitions")?.get(name)?;
        let index = transition["state_index"].as_u64()? as usize;
        Some((index, transition["paused"].as_bool().unwrap_or(false)))
    }

    pub(super) fn context_uri(&self) -> Option<String> {
        self.track()?["metadata"]["context_uri"]
            .as_str()
            .filter(|uri| !uri.is_empty())
            .map(str::to_owned)
    }

    pub(super) fn options(&self) -> (Shuffle, Repeat) {
        let options = self
            .state()
            .map(|state| &state["options"])
            .filter(|options| options.is_object())
            .unwrap_or(&self.machine["attributes"]["options"]);
        let flag = |key: &str| options[key].as_bool().unwrap_or(false);
        let shuffle = match (
            flag("shuffling_context"),
            options["modes"]["context_enhancement"].as_str(),
        ) {
            (true, Some("RECOMMENDATION")) => Shuffle::Smart,
            (true, _) => Shuffle::On,
            _ => Shuffle::Off,
        };
        let repeat = if flag("repeating_track") {
            Repeat::Track
        } else if flag("repeating_context") {
            Repeat::Context
        } else {
            Repeat::Off
        };
        (shuffle, repeat)
    }
}

impl Engine {
    pub(super) async fn audio(&mut self, control: Control) -> Result<()> {
        match control {
            Control::PlayLocal { .. } => bail!("Local files can’t play yet."),
            Control::Settings(settings) => {
                self.audio.settings = settings;
                self.check_prepared();
            }
            Control::Report => {
                let report = self.audio_report();
                self.audio.updates.push(Update::Report(Box::new(report)));
            }
        }
        Ok(())
    }

    fn audio_report(&self) -> AudioReport {
        AudioReport {
            settings: self.audio.settings,
            playing: self.playing.as_ref().map(|playing| ReportItem {
                generation: playing.generation,
                uri: playing.uri().to_owned(),
                position_ms: playing.position_ms,
                duration_ms: playing.duration_ms,
                paused: playing.paused,
                format: playing
                    .format
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| playing.format.to_string()),
                bitrate: playing.bitrate,
            }),
            prepared: self
                .audio
                .joins
                .prepared
                .as_ref()
                .map(|prepared| prepared.report()),
            joins: self.audio.joins.log.clone(),
            player: self.player.snapshot(),
        }
    }

    pub(super) fn state_payload(&self, state_ref: Option<Value>, source: &str) -> Value {
        let sub_state = self.playing.as_ref().map(|playing| {
            json!({
                "playback_speed": if playing.paused { 0 } else { 1 },
                "position": playing.position_ms,
                "duration": playing.duration_ms,
                "media_type": "AUDIO",
                "bitrate": playing.bitrate,
                "audio_quality": "HIGH",
                "format": playing.format,
            })
        });
        json!({
            "seq_num": self.seq,
            "state_ref": state_ref.unwrap_or(Value::Null),
            "sub_state": sub_state.unwrap_or(Value::Null),
            "previous_position": self.playing.as_ref().map(|playing| playing.previous_position_ms).unwrap_or_default(),
            "debug_source": source,
        })
    }

    pub(super) async fn report(&mut self, source: &str) {
        if !self.registered {
            return;
        }
        let state_ref = self.playing.as_ref().map(Playing::reference);
        let mut payload = self.state_payload(state_ref.clone(), source);
        let comparable = {
            let mut copy = payload.clone();
            copy["seq_num"] = Value::Null;
            copy["debug_source"] = Value::Null;
            copy
        };
        if self.last_report.as_ref() == Some(&comparable) && source != "ping" {
            return;
        }
        self.last_report = Some(comparable);
        self.seq += 1;
        payload["seq_num"] = json!(self.seq);
        let response = self
            .api
            .json::<Value, _>(
                Method::PUT,
                &format!("/track-playback/v1/devices/{}/state", self.device_id),
                &payload,
            )
            .await;
        if let Some(playing) = self.playing.as_mut() {
            playing.previous_position_ms = playing.position_ms;
        }
        let Ok(response) = response else { return };
        if response["state_machine"].is_object()
            && self.playing.as_ref().map(Playing::reference) == state_ref
            && let Some(updated) = response["updated_state_ref"].as_object()
        {
            let machine = response["state_machine"].clone();
            let index = updated
                .get("state_index")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize;
            let paused = updated
                .get("paused")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let same_state = self
                .playing
                .as_ref()
                .and_then(|playing| playing.state().map(|state| state["state_id"].clone()))
                == machine["states"]
                    .get(index)
                    .map(|state| state["state_id"].clone());
            if same_state {
                if let Some(playing) = self.playing.as_mut() {
                    playing.machine = machine;
                    playing.index = index;
                }
                self.check_prepared();
            } else {
                let _ = Box::pin(self.start_state(machine, index, paused, None)).await;
            }
        }
    }

    pub(super) async fn report_volume(&mut self, command_id: &str) {
        if !self.registered {
            return;
        }
        self.seq += 1;
        let body = json!({"seq_num": self.seq, "volume": self.volume, "command_id": command_id});
        let _ = self
            .api
            .send(
                Method::PUT,
                &format!("/track-playback/v1/devices/{}/volume", self.device_id),
                Body::Json(body),
                "application/json",
            )
            .await;
    }

    pub(super) async fn replace_state(&mut self, command: Value) {
        let current = self.playing.as_ref().map(Playing::reference);
        let previous = command["prev_state_ref"].clone();
        let matches = match (&current, previous.is_null()) {
            (None, true) => true,
            (Some(current), false) => {
                current["state_machine_id"] == previous["state_machine_id"]
                    && current["state_id"] == previous["state_id"]
            }
            (Some(_), true) => true,
            (None, false) => true,
        };
        if !matches {
            self.reject(&command).await;
            return;
        }
        let seek = command["seek_to"]
            .as_u64()
            .or_else(|| {
                command["seek_to"]
                    .as_str()
                    .and_then(|seek| seek.parse().ok())
            })
            .map(|seek| seek as u32);
        let Some(state_ref) = command["state_ref"].as_object() else {
            self.stop_local();
            self.report("state_clear").await;
            return;
        };
        let machine = command["state_machine"].clone();
        let index = state_ref
            .get("state_index")
            .and_then(Value::as_u64)
            .unwrap_or_default() as usize;
        let paused = state_ref
            .get("paused")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let new_state_id = machine["states"]
            .get(index)
            .map(|state| state["state_id"].clone());
        let same = self
            .playing
            .as_ref()
            .and_then(|playing| playing.state().map(|state| state["state_id"].clone()))
            == new_state_id
            && new_state_id.is_some();
        if same {
            let Some(playing) = self.playing.as_mut() else {
                return;
            };
            playing.machine = machine;
            playing.index = index;
            let mut changed = false;
            if playing.paused != paused {
                playing.paused = paused;
                changed = true;
                if paused {
                    self.player.pause();
                } else {
                    self.player.play();
                }
            }
            if let Some(seek) = seek {
                playing.position_ms = seek;
                self.player.seek(seek);
                changed = true;
            }
            let source = if !changed {
                "modify_current_state"
            } else if seek.is_some() {
                "position_changed"
            } else if paused {
                "pause"
            } else {
                "resume"
            };
            self.check_prepared();
            self.report(source).await;
            return;
        }
        if let Err(error) = self.start_state(machine, index, paused, seek).await {
            self.stop_local();
            self.report("error").await;
            let _ = error;
        }
    }

    async fn reject(&mut self, command: &Value) {
        let rejected = json!({
            "state_machine_id": command["state_machine"]["state_machine_id"],
            "state_id": command["state_ref"]["state_index"]
                .as_u64()
                .and_then(|index| command["state_machine"]["states"].get(index as usize))
                .map(|state| state["state_id"].clone())
                .unwrap_or(Value::Null),
            "paused": command["state_ref"]["paused"],
        });
        self.seq += 1;
        let mut body = self.state_payload(
            self.playing.as_ref().map(Playing::reference),
            "state_conflict",
        );
        body["seq_num"] = Value::Null;
        body["seq_nums"] = json!([self.seq]);
        body["rejected_state_refs"] = json!([rejected]);
        let response = self
            .api
            .json::<Value, _>(
                Method::POST,
                &format!(
                    "/track-playback/v1/devices/{}/state_conflict",
                    self.device_id
                ),
                &body,
            )
            .await;
        if let Ok(response) = response {
            for command in response["commands"].as_array().cloned().unwrap_or_default() {
                if command["type"] == "replace_state" {
                    Box::pin(self.replace_state(command)).await;
                }
            }
        }
    }

    async fn start_state(
        &mut self,
        machine: Value,
        index: usize,
        paused: bool,
        seek: Option<u32>,
    ) -> Result<()> {
        self.discard_prepared();
        self.audio.joins.tried = None;
        self.generation += 1;
        let state = machine["states"].get(index).cloned().unwrap_or(Value::Null);
        let initial = state["initial_playback_position"]
            .as_u64()
            .unwrap_or_default() as u32;
        let mut playing = Playing::new(
            machine,
            index,
            paused,
            self.generation,
            seek.unwrap_or(initial),
        );
        let track = playing
            .track()
            .cloned()
            .context("Spotify sent no track to play.")?;
        let file =
            choose_file(&track).context("Spotify offered no FairPlay audio for this track.")?;
        playing.format = file.format.clone();
        playing.bitrate = file.bitrate;
        playing.duration_ms = track["metadata"]["duration"].as_u64().unwrap_or_default() as u32;
        self.playing = Some(playing);
        self.audio.joins.record(format!("cold {}", self.generation));
        self.report("before_track_load").await;

        let media = Media {
            start_ms: seek.unwrap_or(initial),
            play: !paused,
            ..self.media(&file.id, &file.format_number).await?
        };
        self.player.set_volume(f32::from(self.volume) / GAIA_VOLUME);
        self.player.load(self.generation, media);
        Ok(())
    }

    async fn media(&mut self, file_id: &str, format: &str) -> Result<Media> {
        let resolved: Value = self
            .api
            .get_json(&format!(
                "/storage-resolve/v2/files/audio/interactive/{format}/{file_id}?version=10000000&product=9&platform=39&alt=json"
            ))
            .await
            .context("Spotify did not provide the audio file location.")?;
        let cdn_url = resolved["cdnurl"]
            .as_array()
            .and_then(|urls| urls.first())
            .and_then(Value::as_str)
            .context("Spotify returned no audio servers.")?
            .to_owned();
        let seektable: Value = self
            .http
            .get(format!(
                "https://seektables.scdn.co/seektable/{file_id}.json"
            ))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("The audio segment table was unreadable.")?;
        let timescale = seektable["timescale"]
            .as_f64()
            .filter(|scale| *scale > 0.)
            .unwrap_or(44_100.);
        let segments = seektable["segments"]
            .as_array()
            .context("The audio segment table had no segments.")?
            .iter()
            .filter_map(|segment| {
                let pair = segment.as_array()?;
                Some((pair.first()?.as_u64()?, pair.get(1)?.as_f64()? / timescale))
            })
            .collect();
        let init_size = seektable["offset"]
            .as_u64()
            .context("Missing init segment size.")?;
        let certificate = match &self.certificate {
            Some(certificate) => certificate.clone(),
            None => {
                let bytes = self
                    .api
                    .request(
                        Method::GET,
                        "/fairplay-license/v1/application-certificate",
                        Body::Empty,
                        "*/*",
                    )
                    .await
                    .context("Could not get Spotify's FairPlay certificate.")?;
                let certificate = Arc::new(bytes);
                self.certificate = Some(certificate.clone());
                certificate
            }
        };
        let license_url = match &self.license_url {
            Some((url, expires)) if *expires > Instant::now() => url.clone(),
            _ => {
                let response: Value = self
                    .api
                    .get_json("/melody/v1/license_url?keysystem=com.apple.fps.1_0&mediatype=audio&sdk_name=harmony&sdk_version=4.67.0")
                    .await
                    .unwrap_or(Value::Null);
                let uri = response["uri"]
                    .as_str()
                    .unwrap_or("fairplay-license/v1/audio/license")
                    .replace("@webgate/", "");
                let url = self.api.url(&uri);
                self.license_url = Some((url.clone(), Instant::now() + Duration::from_secs(1800)));
                url
            }
        };
        Ok(Media::new(Source::FairPlay(FairPlay {
            file_id: file_id.to_owned(),
            cdn_url,
            init_size,
            segments,
            certificate,
            license_url,
            token: self.api.token(),
        })))
    }

    pub(super) async fn player_event(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::Ready {
                generation,
                duration_ms,
            } => {
                if let Some(playing) = self
                    .playing
                    .as_mut()
                    .filter(|playing| playing.generation == generation)
                {
                    if duration_ms > 0 {
                        playing.duration_ms = duration_ms;
                    }
                } else {
                    self.prepared_ready(generation, duration_ms);
                }
            }
            PlayerEvent::Position {
                generation,
                position_ms,
                playing: is_playing,
            } => {
                let Some(playing) = self
                    .playing
                    .as_mut()
                    .filter(|playing| playing.generation == generation)
                else {
                    return;
                };
                playing.position_ms = position_ms;
                let _ = is_playing;
                let threshold = playing
                    .track()
                    .and_then(|track| track["ms_played_until_update"].as_u64())
                    .unwrap_or(30_000) as u32;
                let interval = playing
                    .track()
                    .and_then(|track| track["ms_playing_update_interval"].as_u64())
                    .unwrap_or(0);
                let source = if !playing.started_reported && position_ms > 1000 && !playing.paused {
                    playing.started_reported = true;
                    Some("started_playing")
                } else if !playing.threshold_reported && position_ms >= threshold {
                    playing.threshold_reported = true;
                    playing.last_progress = Instant::now();
                    Some("played_threshold_reached")
                } else if playing.threshold_reported
                    && interval > 0
                    && playing.last_progress.elapsed() >= Duration::from_millis(interval)
                {
                    playing.last_progress = Instant::now();
                    Some("progress")
                } else {
                    None
                };
                if let Some(source) = source {
                    self.report(source).await;
                }
                self.prepare_next().await;
            }
            PlayerEvent::Advanced { from, to } => self.joined(from, to).await,
            PlayerEvent::Ended { generation } => {
                if self
                    .playing
                    .as_ref()
                    .is_some_and(|playing| playing.generation == generation)
                {
                    self.advance("advance").await;
                }
            }
            PlayerEvent::Failed {
                generation,
                message,
            } => {
                if self.prepared_failed(generation) {
                    return;
                }
                let current = self.playing.as_ref().map(|playing| playing.generation);
                if generation == 0 || Some(generation) == current {
                    self.report("error").await;
                    let _ = message;
                }
            }
        }
    }

    pub(super) async fn advance(&mut self, transition: &str) {
        let how = if transition == "advance" {
            "promote"
        } else {
            "skip"
        };
        match self.take_prepared(transition, how).await {
            Taken::Joined(generation) => self.player.promote(generation),
            Taken::Superseded => {}
            Taken::Missing => self.advance_cold(transition).await,
        }
    }

    async fn advance_cold(&mut self, transition: &str) {
        let Some(playing) = self.playing.as_ref() else {
            return;
        };
        let Some((index, paused)) = playing.transition(transition) else {
            if transition == "advance" {
                if let Some(playing) = self.playing.as_mut() {
                    playing.paused = true;
                    playing.position_ms = playing.duration_ms;
                }
                self.report("track_data_finalized").await;
            }
            return;
        };
        let machine = playing.machine.clone();
        let old = playing.generation;
        if transition == "advance"
            && let Some(playing) = self.playing.as_mut()
        {
            playing.position_ms = playing.duration_ms.max(playing.position_ms);
        }
        self.report("track_data_finalized").await;
        if self.playing.as_ref().map(|playing| playing.generation) != Some(old) {
            return;
        }
        if let Err(error) = self.start_state(machine, index, paused, None).await {
            let _ = error;
            self.report("error").await;
        }
    }

    pub(super) fn stop_local(&mut self) {
        self.player.stop();
        self.playing = None;
        self.audio.joins.reset();
    }

    pub(super) async fn pause_local(&mut self) {
        if let Some(playing) = self.playing.as_mut()
            && !playing.paused
        {
            playing.paused = true;
            self.player.pause();
            self.report("pause").await;
        }
    }
}
