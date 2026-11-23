use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use reqwest::Method;
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use crate::{
    api::{Api, Body},
    catalog::{Track, image_url},
    connect::{self, Remote},
    dealer::{self, DealerEvent, Message},
    oauth::{self, Token},
    player::{Media, Player, PlayerEvent},
};

const CLIENT_VERSION: &str = "harmony:4.67.0-887cfb0c3";
const GAIA_VOLUME: f32 = 65535.;

#[derive(Clone, Debug)]
pub enum Control {
    PlayTracks {
        uris: Vec<String>,
        index: usize,
    },
    PlayContext {
        uri: String,
        index: usize,
        track_uri: Option<String>,
    },
    SetPlaying(bool),
    Next,
    Previous,
    Seek(u32),
    Volume(u16),
    PlayHere,
    SignOut,
}

#[derive(Clone, Debug)]
pub struct NowPlaying {
    pub track: Option<Track>,
    pub playing: bool,
    pub position_ms: u32,
    pub observed: Instant,
    pub device: Option<String>,
    pub local: bool,
    pub volume: u16,
}

impl Default for NowPlaying {
    fn default() -> Self {
        Self {
            track: None,
            playing: false,
            position_ms: 0,
            observed: Instant::now(),
            device: None,
            local: false,
            volume: u16::MAX,
        }
    }
}

impl NowPlaying {
    pub fn position(&self) -> u32 {
        let elapsed = if self.playing {
            self.observed.elapsed().as_millis() as u32
        } else {
            0
        };
        let position = self.position_ms.saturating_add(elapsed);
        match self.track.as_ref().map(|track| track.duration_ms) {
            Some(duration) if duration > 0 => position.min(duration),
            _ => position,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Update {
    Ready,
    Now(NowPlaying),
    Notice(String),
    RefreshToken(String),
    Ended(Option<String>),
}

pub struct Handle {
    controls: mpsc::UnboundedSender<Control>,
    pub api: Api,
}

impl Handle {
    pub fn send(&self, control: Control) {
        let _ = self.controls.send(control);
    }
}

pub fn start(
    http: reqwest::Client,
    api: Api,
    token: Token,
    device_name: String,
) -> (Handle, mpsc::UnboundedReceiver<Update>) {
    let (controls, control_receiver) = mpsc::unbounded_channel();
    let (updates, update_receiver) = mpsc::unbounded_channel();
    let engine_api = api.clone();
    tokio::spawn(async move {
        let reason = run(
            http,
            engine_api,
            token,
            device_name,
            control_receiver,
            &updates,
        )
        .await;
        let _ = updates.send(Update::Ended(reason.err().map(|error| error.to_string())));
    });
    (Handle { controls, api }, update_receiver)
}

struct Playing {
    machine: Value,
    index: usize,
    paused: bool,
    generation: u64,
    format: Value,
    bitrate: u64,
    duration_ms: u32,
    position_ms: u32,
    previous_position_ms: u32,
    started_reported: bool,
    threshold_reported: bool,
    last_progress: Instant,
}

impl Playing {
    fn state(&self) -> Option<&Value> {
        self.machine["states"].get(self.index)
    }

    fn reference(&self) -> Value {
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

    fn metadata(&self) -> Option<Track> {
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
        })
    }

    fn transition(&self, name: &str) -> Option<(usize, bool)> {
        let transition = self.state()?.get("transitions")?.get(name)?;
        let index = transition["state_index"].as_u64()? as usize;
        Some((index, transition["paused"].as_bool().unwrap_or(false)))
    }
}

struct Engine {
    api: Api,
    http: reqwest::Client,
    player: Player,
    device_id: String,
    device_name: String,
    connection_id: Option<String>,
    registered: bool,
    seq: u64,
    volume: u16,
    generation: u64,
    playing: Option<Playing>,
    remote: Option<Remote>,
    certificate: Option<Arc<Vec<u8>>>,
    license_url: Option<(String, Instant)>,
    last_report: Option<Value>,
    volume_due: Option<tokio::time::Instant>,
    published: Option<(PublishedKey, u32, Instant)>,
}

#[derive(PartialEq)]
struct PublishedKey {
    track: Option<String>,
    title: Option<String>,
    cover: Option<String>,
    duration: u32,
    playing: bool,
    device: Option<String>,
    local: bool,
    volume: u16,
}

const POSITION_DRIFT_MS: u32 = 1500;

async fn run(
    http: reqwest::Client,
    api: Api,
    mut token: Token,
    device_name: String,
    mut controls: mpsc::UnboundedReceiver<Control>,
    updates: &mpsc::UnboundedSender<Update>,
) -> Result<()> {
    let (token_sender, token_receiver) = watch::channel(token.access_token.clone());
    let (mut dealer_events, dealer_task) = dealer::spawn(http.clone(), token_receiver);
    let _dealer = AbortOnDrop(dealer_task);
    let (player_events_sender, mut player_events) = mpsc::unbounded_channel();
    let player = Player::new(
        player_events_sender,
        http.clone(),
        tokio::runtime::Handle::current(),
    );
    let device_id: String = (0..20)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect();
    let mut engine = Engine {
        api,
        http: http.clone(),
        player,
        device_id,
        device_name,
        connection_id: None,
        registered: false,
        seq: 0,
        volume: u16::MAX,
        generation: 0,
        playing: None,
        remote: None,
        certificate: None,
        license_url: None,
        last_report: None,
        volume_due: None,
        published: None,
    };
    let mut refresh_at = tokio::time::Instant::from_std(token.refresh_due());

    loop {
        let volume_due = engine.volume_due;
        tokio::select! {
            control = controls.recv() => {
                let Some(control) = control else {
                    engine.deregister().await;
                    return Ok(());
                };
                if matches!(control, Control::SignOut) {
                    engine.deregister().await;
                    return Ok(());
                }
                if let Err(error) = engine.control(control).await {
                    let _ = updates.send(Update::Notice(error.to_string()));
                }
                engine.publish(updates);
            }
            event = dealer_events.recv() => {
                let Some(event) = event else { return Err(anyhow!("Spotify connection ended.")) };
                match event {
                    DealerEvent::Connected { connection_id } => {
                        engine.connection_id = Some(connection_id);
                        match engine.register().await {
                            Ok(()) => {
                                let _ = updates.send(Update::Ready);
                            }
                            Err(error) => {
                                let _ = updates.send(Update::Notice(error.to_string()));
                            }
                        }
                        engine.observe().await;
                        engine.publish(updates);
                    }
                    DealerEvent::Message(message) => {
                        engine.message(message).await;
                        engine.publish(updates);
                    }
                    DealerEvent::Disconnected => {
                        engine.registered = false;
                    }
                }
            }
            event = player_events.recv() => {
                let Some(event) = event else { continue };
                engine.player_event(event).await;
                engine.publish(updates);
            }
            _ = tokio::time::sleep_until(refresh_at) => {
                match oauth::refresh(&http, oauth::DESKTOP_CLIENT_ID, &token.refresh_token).await {
                    Ok(fresh) => {
                        if fresh.refresh_token != token.refresh_token {
                            let _ = updates.send(Update::RefreshToken(fresh.refresh_token.clone()));
                        }
                        token = fresh;
                        engine.api.set_token(token.access_token.clone());
                        engine.player.update_token(token.access_token.clone());
                        let _ = token_sender.send(token.access_token.clone());
                        refresh_at = tokio::time::Instant::from_std(token.refresh_due());
                    }
                    Err(error) if error.downcast_ref::<oauth::Rejected>().is_some() => {
                        engine.deregister().await;
                        return Err(anyhow!("Your Spotify session expired. Sign in again."));
                    }
                    Err(_) => refresh_at = tokio::time::Instant::now() + Duration::from_secs(30),
                }
            }
            _ = async { tokio::time::sleep_until(volume_due.unwrap_or_else(tokio::time::Instant::now)).await }, if volume_due.is_some() => {
                engine.volume_due = None;
                engine.report_volume("").await;
            }
        }
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl Engine {
    fn device_info(&self) -> Value {
        json!({
            "brand": "public_js-sdk",
            "capabilities": {
                "change_volume": true,
                "enable_play_token": true,
                "supports_file_media_type": true,
                "play_token_lost_behavior": "pause",
                "disable_connect": false,
                "audio_podcasts": true,
                "lossless_playback": false,
                "manifest_formats": ["file_ids_mp4_cbcs"],
                "supports_preferred_media_type": true,
                "supports_playback_offsets": true,
                "supports_playback_speed": false
            },
            "device_id": self.device_id,
            "device_type": "computer",
            "metadata": {},
            "model": "harmony-safari.18-mac",
            "name": self.device_name,
            "platform_identifier": "Partner public_js-sdk harmony-safari.18-mac;tp",
            "is_group": false
        })
    }

    fn is_active_here(&self) -> bool {
        self.playing.is_some()
            && self
                .remote
                .as_ref()
                .and_then(|remote| remote.device_id.as_deref())
                .is_none_or(|active| active == self.device_id)
    }

    fn remote_target(&self) -> Option<String> {
        let remote = self.remote.as_ref()?;
        let active = remote.device_id.clone()?;
        (active != self.device_id).then_some(active)
    }

    async fn register(&mut self) -> Result<()> {
        let connection_id = self
            .connection_id
            .clone()
            .context("Spotify did not provide a connection.")?;
        let mut body = json!({
            "device": self.device_info(),
            "outro_endcontent_snooping": false,
            "connection_id": connection_id,
            "client_version": CLIENT_VERSION,
            "volume": self.volume,
        });
        if let Some(playing) = &self.playing {
            body["previous_session_state"] =
                self.state_payload(Some(playing.reference()), "register");
        }
        let response: Value = self
            .api
            .json(Method::POST, "/track-playback/v1/devices", &body)
            .await
            .map_err(|error| {
                if error
                    .downcast_ref::<crate::api::HttpStatus>()
                    .is_some_and(|status| status.0 == reqwest::StatusCode::FORBIDDEN)
                {
                    anyhow!("Spotify Premium is required to play music.")
                } else {
                    anyhow!("Spotify did not register OpenMusic as a device. {error}")
                }
            })?;
        self.seq = response["initial_seq_num"].as_u64().unwrap_or_default();
        self.registered = true;
        Ok(())
    }

    async fn deregister(&mut self) {
        self.player.stop();
        if !self.registered {
            return;
        }
        self.seq += 1;
        let body =
            json!({"seq_num": self.seq, "state_ref": Value::Null, "debug_source": "deregister"});
        let _ = self
            .api
            .send(
                Method::DELETE,
                &format!("/track-playback/v1/devices/{}", self.device_id),
                Body::Json(body),
                "application/json",
            )
            .await;
        self.registered = false;
    }

    async fn observe(&mut self) {
        let Some(connection_id) = self.connection_id.clone() else {
            return;
        };
        if let Ok(cluster) = connect::observe(&self.api, &self.device_id, &connection_id).await {
            self.remote = Some(connect::remote_state(&cluster));
        }
    }

    fn now(&self) -> NowPlaying {
        if let Some(playing) = self.playing.as_ref().filter(|_| self.is_active_here()) {
            return NowPlaying {
                track: playing.metadata(),
                playing: !playing.paused,
                position_ms: playing.position_ms,
                observed: Instant::now(),
                device: Some(self.device_name.clone()),
                local: true,
                volume: self.volume,
            };
        }
        let Some(remote) = &self.remote else {
            return NowPlaying::default();
        };
        NowPlaying {
            track: remote.track.clone(),
            playing: remote.playing,
            position_ms: remote.position_ms,
            observed: Instant::now(),
            device: remote.device_name.clone(),
            local: false,
            volume: remote.volume.unwrap_or(u16::MAX),
        }
    }

    fn publish(&mut self, updates: &mpsc::UnboundedSender<Update>) {
        let now = self.now();
        let key = PublishedKey {
            track: now.track.as_ref().map(|track| track.uri.clone()),
            title: now.track.as_ref().map(|track| track.title.clone()),
            cover: now.track.as_ref().and_then(|track| track.cover.clone()),
            duration: now.track.as_ref().map_or(0, |track| track.duration_ms),
            playing: now.playing,
            device: now.device.clone(),
            local: now.local,
            volume: now.volume,
        };
        if let Some((last, position, at)) = &self.published {
            let elapsed = if now.playing {
                at.elapsed().as_millis() as u32
            } else {
                0
            };
            let expected = position.saturating_add(elapsed);
            if *last == key && expected.abs_diff(now.position_ms) < POSITION_DRIFT_MS {
                return;
            }
        }
        self.published = Some((key, now.position_ms, Instant::now()));
        let _ = updates.send(Update::Now(now));
    }

    fn state_payload(&self, state_ref: Option<Value>, source: &str) -> Value {
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

    async fn report(&mut self, source: &str) {
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
            } else {
                let _ = Box::pin(self.start_state(machine, index, paused, None)).await;
            }
        }
    }

    async fn report_volume(&mut self, command_id: &str) {
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

    async fn message(&mut self, message: Message) {
        if message.uri.starts_with(connect::CLUSTER_URI) {
            if let Some(cluster) = connect::cluster_from(&message) {
                let remote = connect::remote_state(&cluster);
                let moved_away = remote
                    .device_id
                    .as_deref()
                    .is_some_and(|active| active != self.device_id)
                    && self.playing.is_some();
                self.remote = Some(remote);
                if moved_away {
                    self.player.stop();
                    self.playing = None;
                }
            }
            return;
        }
        if message.uri.starts_with("hm://play-token/lost") {
            self.pause_local().await;
            return;
        }
        if !message.uri.starts_with("hm://track-playback/v1") {
            return;
        }
        let Some(command) = message.json().cloned() else {
            return;
        };
        match command["type"].as_str() {
            Some("replace_state") => self.replace_state(command).await,
            Some("set_volume") => {
                let volume = command["volume"].as_u64().unwrap_or(65535).min(65535) as u16;
                self.volume = volume;
                self.player.set_volume(f32::from(volume) / GAIA_VOLUME);
                let id = command["command_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                self.report_volume(&id).await;
            }
            Some("ping") => self.report("ping").await,
            Some("log_out") => {
                self.player.stop();
                self.playing = None;
                self.report("state_clear").await;
            }
            _ => {}
        }
    }

    async fn replace_state(&mut self, command: Value) {
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
            self.player.stop();
            self.playing = None;
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
            self.report(source).await;
            return;
        }
        if let Err(error) = self.start_state(machine, index, paused, seek).await {
            self.playing = None;
            self.player.stop();
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
        self.generation += 1;
        let state = machine["states"].get(index).cloned().unwrap_or(Value::Null);
        let initial = state["initial_playback_position"]
            .as_u64()
            .unwrap_or_default() as u32;
        let mut playing = Playing {
            machine,
            index,
            paused,
            generation: self.generation,
            format: Value::Null,
            bitrate: 0,
            duration_ms: 0,
            position_ms: seek.unwrap_or(initial),
            previous_position_ms: 0,
            started_reported: false,
            threshold_reported: false,
            last_progress: Instant::now(),
        };
        let track = playing
            .track()
            .cloned()
            .context("Spotify sent no track to play.")?;
        let files = track["manifest"]["file_ids_mp4_cbcs"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let file = files
            .iter()
            .max_by_key(|file| file["bitrate"].as_u64().unwrap_or_default())
            .cloned()
            .context("Spotify offered no FairPlay audio for this track.")?;
        let file_id = file["file_id"]
            .as_str()
            .context("Missing file id.")?
            .to_owned();
        playing.format = file["format"].clone();
        playing.bitrate = file["bitrate"].as_u64().unwrap_or(128_000);
        playing.duration_ms = track["metadata"]["duration"].as_u64().unwrap_or_default() as u32;
        let format_number = file["format"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| file["format"].to_string());
        self.playing = Some(playing);
        self.report("before_track_load").await;

        let media = self.media(&file_id, &format_number).await?;
        let media = Media {
            start_ms: seek.unwrap_or(initial),
            play: !paused,
            ..media
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
        Ok(Media {
            file_id: file_id.to_owned(),
            cdn_url,
            init_size,
            segments,
            certificate,
            license_url,
            token: self.api.token(),
            start_ms: 0,
            play: true,
        })
    }

    async fn player_event(&mut self, event: PlayerEvent) {
        match event {
            PlayerEvent::Ready {
                generation,
                duration_ms,
            } => {
                if let Some(playing) = self
                    .playing
                    .as_mut()
                    .filter(|playing| playing.generation == generation)
                    && duration_ms > 0
                {
                    playing.duration_ms = duration_ms;
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
            }
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
                let current = self.playing.as_ref().map(|playing| playing.generation);
                if generation == 0 || Some(generation) == current {
                    self.report("error").await;
                    let _ = message;
                }
            }
        }
    }

    async fn advance(&mut self, transition: &str) {
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
        self.report("track_data_finalized").await;
        if let Err(error) = self.start_state(machine, index, paused, None).await {
            let _ = error;
            self.report("error").await;
        }
    }

    async fn pause_local(&mut self) {
        if let Some(playing) = self.playing.as_mut()
            && !playing.paused
        {
            playing.paused = true;
            self.player.pause();
            self.report("pause").await;
        }
    }

    async fn control(&mut self, control: Control) -> Result<()> {
        if let Some(target) = self.remote_target().filter(|_| {
            !matches!(
                control,
                Control::PlayTracks { .. } | Control::PlayContext { .. } | Control::PlayHere
            )
        }) {
            let name = self
                .remote
                .as_ref()
                .and_then(|remote| remote.device_name.clone())
                .unwrap_or_else(|| "that device".into());
            let result = match control {
                Control::SetPlaying(true) => {
                    connect::command(
                        &self.api,
                        &self.device_id,
                        &target,
                        json!({"endpoint": "resume"}),
                    )
                    .await
                }
                Control::SetPlaying(false) => {
                    connect::command(
                        &self.api,
                        &self.device_id,
                        &target,
                        json!({"endpoint": "pause"}),
                    )
                    .await
                }
                Control::Next => {
                    connect::command(
                        &self.api,
                        &self.device_id,
                        &target,
                        json!({"endpoint": "skip_next"}),
                    )
                    .await
                }
                Control::Previous => {
                    connect::command(
                        &self.api,
                        &self.device_id,
                        &target,
                        json!({"endpoint": "skip_prev"}),
                    )
                    .await
                }
                Control::Seek(position) => {
                    connect::command(
                        &self.api,
                        &self.device_id,
                        &target,
                        json!({"endpoint": "seek_to", "value": position}),
                    )
                    .await
                }
                Control::Volume(volume) => {
                    connect::set_volume(&self.api, &self.device_id, &target, volume).await
                }
                _ => Ok(()),
            };
            return result.map_err(|_| anyhow!("Spotify didn’t accept that command for {name}."));
        }
        match control {
            Control::PlayTracks { uris, index } => {
                connect::command(
                    &self.api,
                    &self.device_id,
                    &self.device_id,
                    connect::play_command(&uris, index, 0),
                )
                .await
                .context("Spotify didn’t start playback.")?;
            }
            Control::PlayContext {
                uri,
                index,
                track_uri,
            } => {
                connect::command(
                    &self.api,
                    &self.device_id,
                    &self.device_id,
                    connect::play_context_command(&uri, index, track_uri.as_deref()),
                )
                .await
                .context("Spotify didn’t start playback.")?;
            }
            Control::PlayHere => {
                if let Some(target) = self.remote_target() {
                    connect::transfer(&self.api, &target, &self.device_id)
                        .await
                        .context("Spotify didn’t move playback to OpenMusic.")?;
                } else if self.playing.is_none() {
                    self.resume_last().await?;
                }
            }
            Control::SetPlaying(true) => {
                if let Some(playing) = self.playing.as_mut() {
                    if playing.paused {
                        playing.paused = false;
                        self.player.play();
                        self.report("resume").await;
                    }
                } else {
                    self.resume_last().await?;
                }
            }
            Control::SetPlaying(false) => self.pause_local().await,
            Control::Next => self.advance("skip_next").await,
            Control::Previous => {
                if self
                    .playing
                    .as_ref()
                    .is_some_and(|playing| playing.position_ms > 3000)
                {
                    if let Some(playing) = self.playing.as_mut() {
                        playing.position_ms = 0;
                    }
                    self.player.seek(0);
                    self.report("position_changed").await;
                } else {
                    self.advance("skip_prev").await;
                }
            }
            Control::Seek(position) => {
                if let Some(playing) = self.playing.as_mut() {
                    playing.position_ms = position;
                    self.player.seek(position);
                    self.report("position_changed").await;
                }
            }
            Control::Volume(volume) => {
                self.volume = volume;
                self.player.set_volume(f32::from(volume) / GAIA_VOLUME);
                self.volume_due = Some(tokio::time::Instant::now() + Duration::from_secs(1));
            }
            Control::SignOut => {}
        }
        Ok(())
    }

    async fn resume_last(&mut self) -> Result<()> {
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        let Some(track) = remote.track.clone() else {
            return Ok(());
        };
        let position = remote.position_ms;
        connect::command(
            &self.api,
            &self.device_id,
            &self.device_id,
            connect::play_command(&[track.uri], 0, position),
        )
        .await
        .context("Spotify didn’t start playback.")
    }
}
