mod playback;
mod remote;

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
    catalog::Track,
    connect::{self, Remote},
    dealer::{self, DealerEvent, Message},
    oauth::{self, Token},
    player::Player,
};
use playback::Playing;

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
    Now(Box<NowPlaying>),
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
        let _ = updates.send(Update::Now(Box::new(now)));
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

    async fn control(&mut self, control: Control) -> Result<()> {
        if let Some(target) = self.remote_target().filter(|_| {
            !matches!(
                control,
                Control::PlayTracks { .. } | Control::PlayContext { .. } | Control::PlayHere
            )
        }) {
            return self.remote(target, control).await;
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
