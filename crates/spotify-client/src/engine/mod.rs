pub mod playback;
pub mod remote;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
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
    clienttoken,
    connect::{
        self, Allowed, ConnectDevice, PlayOptions, Playback, QueueItem, QueueKey, Repeat, Restore,
        Shuffle, SleepState,
    },
    dealer::{self, DealerEvent, Message},
    oauth::{self, Token},
    player::Player,
};
use playback::Playing;

const CLIENT_VERSION: &str = "harmony:4.67.0-887cfb0c3";
const GAIA_VOLUME: f32 = 65535.;
const DEVICE_IDS: &str = "device-ids.json";

pub fn is_push(uri: &str) -> bool {
    uri.starts_with("hm://collection/")
        || uri.starts_with("hm://playlist/v2/")
        || uri.ends_with("social-connect/v2/session_update")
        || uri.contains("playback-settings/")
        || uri.contains("spotify:user:attributes:mutated")
        || uri.starts_with("ap://product-state-update")
}

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
        origin: &'static str,
        options: PlayOptions,
    },
    SetPlaying(bool),
    Next,
    Previous,
    Seek(u32),
    Volume(u16),
    PlayHere,
    SignOut,
    Remote(remote::Control),
    Audio(playback::Control),
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
    pub context_uri: Option<String>,
    pub uid: Option<String>,
    pub provider: Option<String>,
    pub shuffle: Shuffle,
    pub repeat: Repeat,
    pub speed: f32,
    pub episode: bool,
    pub sleep: SleepState,
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
            context_uri: None,
            uid: None,
            provider: None,
            shuffle: Shuffle::Off,
            repeat: Repeat::Off,
            speed: 1.,
            episode: false,
            sleep: SleepState::Off,
        }
    }
}

impl NowPlaying {
    pub fn position(&self) -> u32 {
        let elapsed = if self.playing {
            (self.observed.elapsed().as_millis() as f64 * f64::from(self.speed.max(0.))) as u32
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
    Playback(Box<Playback>),
    Push(Message),
    Audio(playback::Update),
    Notice(String),
    RefreshToken(String),
    Ended(Option<String>),
}

pub struct Handle {
    controls: mpsc::UnboundedSender<Control>,
    pub api: Api,
    pub device_id: String,
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
    let device_id = stable_device_id(device_ids_path().as_deref(), &device_name);
    let engine_api = api.clone();
    let engine_device = device_id.clone();
    tokio::spawn(async move {
        let reason = run(
            http,
            engine_api,
            token,
            engine_device,
            device_name,
            control_receiver,
            &updates,
        )
        .await;
        let _ = updates.send(Update::Ended(reason.err().map(|error| error.to_string())));
    });
    (
        Handle {
            controls,
            api,
            device_id,
        },
        update_receiver,
    )
}

fn device_ids_path() -> Option<PathBuf> {
    Some(
        PathBuf::from(std::env::var_os("HOME")?)
            .join("Library/Application Support/OpenMusic")
            .join(DEVICE_IDS),
    )
}

fn new_device_id() -> String {
    (0..20)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect()
}

fn valid_device_id(id: &str) -> bool {
    id.len() == 40 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn stable_device_id(path: Option<&Path>, name: &str) -> String {
    let Some(path) = path else {
        return new_device_id();
    };
    let mut ids: BTreeMap<String, String> = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if let Some(id) = ids.get(name).filter(|id| valid_device_id(id)) {
        return id.clone();
    }
    let id = new_device_id();
    ids.insert(name.to_owned(), id.clone());
    let _ = save_device_ids(path, &ids);
    id
}

fn save_device_ids(path: &Path, ids: &BTreeMap<String, String>) -> Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let temporary = path.with_extension("tmp");
    clienttoken::write_private(&temporary, &serde_json::to_vec_pretty(ids)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
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
    playback: Option<Playback>,
    certificate: Option<Arc<Vec<u8>>>,
    license_url: Option<(String, Instant)>,
    last_report: Option<Value>,
    volume_due: Option<tokio::time::Instant>,
    published: Option<(PublishedKey, u32, Instant)>,
    published_playback: Option<PlaybackKey>,
    remote: remote::State,
    audio: playback::State,
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
    context_uri: Option<String>,
    uid: Option<String>,
    provider: Option<String>,
    shuffle: Shuffle,
    repeat: Repeat,
    speed: u32,
    sleep: SleepState,
}

#[derive(PartialEq)]
struct PlaybackKey {
    active: Option<String>,
    context_uri: Option<String>,
    context_title: Option<String>,
    current: Option<QueueKey>,
    shuffle: Shuffle,
    repeat: Repeat,
    speed: u32,
    allowed: Allowed,
    revision: String,
    devices: Vec<ConnectDevice>,
    signals: Vec<String>,
    sleep: SleepState,
    jam: Option<String>,
}

impl PlaybackKey {
    fn of(playback: &Playback) -> Self {
        Self {
            active: playback.active_device_id.clone(),
            context_uri: playback.context_uri.clone(),
            context_title: playback.context_title.clone(),
            current: playback.queue.current.as_ref().map(QueueItem::key),
            shuffle: playback.shuffle,
            repeat: playback.repeat,
            speed: playback.playback_speed.to_bits(),
            allowed: playback.allowed,
            revision: playback.queue.revision.clone(),
            devices: playback.devices.clone(),
            signals: playback.signals.clone(),
            sleep: playback.sleep_timer,
            jam: playback.jam_mode.clone(),
        }
    }
}

const POSITION_DRIFT_MS: u32 = 1500;

async fn run(
    http: reqwest::Client,
    api: Api,
    mut token: Token,
    device_id: String,
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
        playback: None,
        certificate: None,
        license_url: None,
        last_report: None,
        volume_due: None,
        published: None,
        published_playback: None,
        remote: remote::State::default(),
        audio: playback::State::default(),
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
                    DealerEvent::Message(message) if is_push(&message.uri) => {
                        let _ = updates.send(Update::Push(message));
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
                "manifest_formats": playback::MANIFEST_FORMATS,
                "supports_preferred_media_type": true,
                "supports_playback_offsets": true,
                "supports_playback_speed": true,
                "supports_external_url_playback_speed": true
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
                .playback
                .as_ref()
                .and_then(|playback| playback.active_device_id.as_deref())
                .is_none_or(|active| active == self.device_id)
    }

    fn remote_target(&self) -> Option<String> {
        let playback = self.playback.as_ref()?;
        let active = playback.active_device_id.clone()?;
        (active != self.device_id).then_some(active)
    }

    fn playback_here(&self) -> Option<&Playback> {
        self.playback
            .as_ref()
            .filter(|playback| playback.active_device_id.as_deref() == Some(&self.device_id))
    }

    async fn send_command(&mut self, target: &str, command: Value) -> Result<Option<String>> {
        self.routed_command(target, command).await
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
            self.playback = Some(connect::playback(&cluster, &self.device_id));
        }
    }

    fn now(&self) -> NowPlaying {
        if let Some(now) = self.local_now() {
            return now;
        }
        if let Some(playing) = self.playing.as_ref().filter(|_| self.is_active_here()) {
            let track = playing.metadata();
            let here = self.playback_here();
            let (shuffle, repeat) = here
                .map(|playback| (playback.shuffle, playback.repeat))
                .unwrap_or_else(|| playing.options());
            let provider = here
                .filter(|playback| {
                    playback.track.as_ref().map(|track| &track.uri)
                        == track.as_ref().map(|track| &track.uri)
                })
                .and_then(|playback| playback.provider.clone());
            return NowPlaying {
                context_uri: here
                    .and_then(|playback| playback.context_uri.clone())
                    .or_else(|| playing.context_uri()),
                uid: track.as_ref().and_then(|track| track.uid.clone()),
                provider,
                shuffle,
                repeat,
                speed: playing.speed,
                episode: track.as_ref().is_some_and(is_episode),
                sleep: match self.local_sleep() {
                    SleepState::Off => {
                        here.map_or(SleepState::Off, |playback| playback.sleep_timer)
                    }
                    local => local,
                },
                track,
                playing: !playing.paused,
                position_ms: playing.position_ms,
                observed: Instant::now(),
                device: Some(self.device_name.clone()),
                local: true,
                volume: self.volume,
            };
        }
        let Some(playback) = &self.playback else {
            return NowPlaying::default();
        };
        NowPlaying {
            track: playback.track.clone(),
            playing: playback.playing,
            position_ms: playback.position_ms,
            observed: Instant::now(),
            device: playback.active_device_name.clone(),
            local: false,
            volume: playback.active_volume.unwrap_or(u16::MAX),
            context_uri: playback.context_uri.clone(),
            uid: playback.track.as_ref().and_then(|track| track.uid.clone()),
            provider: playback.provider.clone(),
            shuffle: playback.shuffle,
            repeat: playback.repeat,
            speed: playback.playback_speed,
            episode: playback.track.as_ref().is_some_and(is_episode),
            sleep: playback.sleep_timer,
        }
    }

    fn publish(&mut self, updates: &mpsc::UnboundedSender<Update>) {
        for update in self.audio_updates() {
            let _ = updates.send(Update::Audio(update));
        }
        if let Some(playback) = &self.playback {
            let key = PlaybackKey::of(playback);
            if self.published_playback.as_ref() != Some(&key) {
                self.published_playback = Some(key);
                let _ = updates.send(Update::Playback(Box::new(playback.clone())));
            }
        }
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
            context_uri: now.context_uri.clone(),
            uid: now.uid.clone(),
            provider: now.provider.clone(),
            shuffle: now.shuffle,
            repeat: now.repeat,
            speed: now.speed.to_bits(),
            sleep: now.sleep,
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
                let playback = connect::playback(&cluster, &self.device_id);
                let moved_away = playback
                    .active_device_id
                    .as_deref()
                    .is_some_and(|active| active != self.device_id)
                    && self.playing.is_some();
                self.playback = Some(playback);
                if moved_away {
                    self.stop_local();
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
                self.stop_local();
                self.report("state_clear").await;
            }
            _ => {}
        }
    }

    async fn control(&mut self, control: Control) -> Result<()> {
        let control = match control {
            Control::Remote(control) => return self.remote_control(control).await,
            Control::Audio(control) => return self.audio(control).await,
            control => control,
        };
        let Some(control) = self.local_control(control).await else {
            return Ok(());
        };
        if let Some(target) = self.control_target(&control).filter(|_| {
            !matches!(
                control,
                Control::PlayTracks { .. } | Control::PlayContext { .. } | Control::PlayHere
            )
        }) {
            return self.remote(target, control).await;
        }
        let own = self.device_id.clone();
        match control {
            Control::PlayTracks { uris, index } => {
                self.send_command(&own, connect::play_command(&uris, index, 0))
                    .await
                    .context("Spotify didn’t start playback.")?;
            }
            Control::PlayContext {
                uri,
                index,
                track_uri,
                origin,
                options,
            } => {
                let command = connect::play_context_command(
                    &uri,
                    index,
                    track_uri.as_deref(),
                    origin,
                    &options,
                );
                self.send_command(&own, command)
                    .await
                    .context("Spotify didn’t start playback.")?;
            }
            Control::PlayHere => {
                if let Some(target) = self.remote_target() {
                    connect::transfer(&self.api, &target, &own, Restore::Resume, None)
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
            Control::SignOut | Control::Remote(_) | Control::Audio(_) => {}
        }
        Ok(())
    }

    async fn resume_last(&mut self) -> Result<()> {
        let Some(playback) = &self.playback else {
            return Ok(());
        };
        let Some(track) = playback.track.clone() else {
            return Ok(());
        };
        let position = playback.position_ms;
        let own = self.device_id.clone();
        self.send_command(&own, connect::play_command(&[track.uri], 0, position))
            .await
            .context("Spotify didn’t start playback.")?;
        Ok(())
    }
}

fn is_episode(track: &Track) -> bool {
    track.uri.starts_with("spotify:episode:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_pushes_by_topic() {
        for uri in [
            "hm://collection/collection/someone/json",
            "hm://collection/artist/someone/json",
            "hm://playlist/v2/user/someone/rootlist",
            "hm://playlist/v2/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "hm://social-connect/v2/session_update",
            "social-connect/v2/session_update",
            "hm://playback-settings/user-settings-update",
            "playback-settings/content-settings-update",
            "spotify:user:attributes:mutated",
            "hm://spotify:user:attributes:mutated",
            "ap://product-state-update",
        ] {
            assert!(is_push(uri), "{uri}");
        }
        for uri in [
            "hm://connect-state/v1/cluster",
            "hm://track-playback/v1/command",
            "hm://play-token/lost",
            "hm://pusher/v1/connections/abc",
            "hm://social-connect/v2/session_update/extra",
        ] {
            assert!(!is_push(uri), "{uri}");
        }
    }

    #[test]
    fn keeps_one_device_id_per_name() {
        let directory =
            std::env::temp_dir().join(format!("openmusic-device-ids-{}", new_device_id()));
        let path = directory.join(DEVICE_IDS);
        let first = stable_device_id(Some(&path), "OpenMusic");
        assert!(valid_device_id(&first));
        assert_eq!(stable_device_id(Some(&path), "OpenMusic"), first);
        let other = stable_device_id(Some(&path), "OpenMusic Dev");
        assert_ne!(other, first);
        assert_eq!(stable_device_id(Some(&path), "OpenMusic Dev"), other);
        let saved: BTreeMap<String, String> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.len(), 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::write(&path, b"{\"OpenMusic\":\"short\"}").unwrap();
        let replaced = stable_device_id(Some(&path), "OpenMusic");
        assert!(valid_device_id(&replaced));
        assert_ne!(replaced, first);
        assert_eq!(stable_device_id(Some(&path), "OpenMusic"), replaced);
        std::fs::remove_dir_all(directory).unwrap();
        assert_ne!(
            stable_device_id(None, "OpenMusic"),
            stable_device_id(None, "OpenMusic")
        );
    }
}
