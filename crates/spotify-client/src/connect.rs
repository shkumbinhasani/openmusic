use std::{
    collections::{HashMap, HashSet},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail};
use prost::Message as _;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use spotify_proto::connectstate::{
    Capabilities, Cluster, ClusterUpdate, Device, DeviceInfo, MemberType, PlayerState,
    ProvidedTrack, PutStateReason, PutStateRequest, Restrictions, devices::DeviceType,
    sleep_timer::TimerType,
};

use crate::{
    api::{Api, Body, HttpStatus},
    catalog::{Track, image_url},
    dealer::Message,
    pathfinder::{self, Operation},
};

pub const CLUSTER_URI: &str = "hm://connect-state/v1/cluster";
pub const ORIGIN: &str = "openmusic";
const PROTOBUF: &str = "application/x-protobuf";
const DELIMITERS: [&str; 2] = ["spotify:delimiter", "spotify:meta:delimiter"];
const DEFAULT_VOLUME_STEPS: u32 = 64;
const ALIAS_SEPARATOR: &str = "::a_";

pub fn observer_id(device_id: &str) -> String {
    format!("hobs_{}", &device_id[..device_id.len().min(35)])
}

pub async fn observe(api: &Api, device_id: &str, connection_id: &str) -> Result<Cluster> {
    let observer = observer_id(device_id);
    let request = PutStateRequest {
        member_type: MemberType::ConnectState as i32,
        put_state_reason: PutStateReason::NewDevice as i32,
        device: Some(Device {
            device_info: Some(DeviceInfo {
                name: "OpenMusic".into(),
                device_id: observer.clone(),
                capabilities: Some(Capabilities {
                    hidden: true,
                    is_observable: true,
                    needs_full_player_state: true,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let bytes = api
        .http()
        .put(api.url(&format!("/connect-state/v1/devices/{observer}")))
        .bearer_auth(api.token())
        .header("x-spotify-connection-id", connection_id)
        .header("content-type", PROTOBUF)
        .header("accept", PROTOBUF)
        .body(request.encode_to_vec())
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(Cluster::decode(bytes.as_ref())?)
}

pub async fn fetch_cluster(api: &Api) -> Result<Cluster> {
    let bytes = api
        .request(
            Method::GET,
            "/connect-state/v1/cluster",
            Body::Empty,
            PROTOBUF,
        )
        .await?;
    Ok(Cluster::decode(bytes.as_slice())?)
}

pub fn cluster_from(message: &Message) -> Option<Cluster> {
    if !message.uri.starts_with(CLUSTER_URI) {
        return None;
    }
    ClusterUpdate::decode(message.bytes()?.as_slice())
        .ok()?
        .cluster
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shuffle {
    #[default]
    Off,
    On,
    Smart,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Repeat {
    #[default]
    Off,
    Context,
    Track,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SleepState {
    #[default]
    Off,
    At(SystemTime),
    EndOfTrack,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Allowed {
    pub pause: bool,
    pub resume: bool,
    pub seek: bool,
    pub skip_prev: bool,
    pub skip_next: bool,
    pub shuffle: bool,
    pub smart_shuffle: bool,
    pub repeat_context: bool,
    pub repeat_track: bool,
    pub add_to_queue: bool,
    pub play_next: bool,
    pub set_queue: bool,
    pub remove_from_queue: bool,
    pub reorder_queue: bool,
    pub jump_in_queue: bool,
    pub view_queue: bool,
    pub transfer: bool,
    pub set_speed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct QueueKey {
    pub uid: String,
    pub iteration: String,
    pub uri: String,
}

impl QueueKey {
    pub fn of(track: &ProvidedTrack) -> Self {
        Self {
            uid: track.uid.clone(),
            iteration: track.metadata.get("iteration").cloned().unwrap_or_default(),
            uri: track.uri.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueueItem {
    pub uri: String,
    pub uid: String,
    pub iteration: String,
    pub provider: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub cover: Option<String>,
    pub metadata: HashMap<String, String>,
}

impl QueueItem {
    pub fn of(track: &ProvidedTrack) -> Self {
        let field = |key: &str| {
            track
                .metadata
                .get(key)
                .filter(|value| !value.is_empty())
                .cloned()
        };
        Self {
            uri: track.uri.clone(),
            uid: track.uid.clone(),
            iteration: field("iteration").unwrap_or_default(),
            provider: track.provider.clone(),
            title: field("title"),
            artist: field("artist_name"),
            cover: field("image_url").map(|image| image_url(&image)),
            metadata: track.metadata.clone(),
        }
    }

    pub fn key(&self) -> QueueKey {
        QueueKey {
            uid: self.uid.clone(),
            iteration: self.iteration.clone(),
            uri: self.uri.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queue {
    pub revision: String,
    pub current: Option<QueueItem>,
    pub queued: Vec<QueueItem>,
    pub next_up: Vec<QueueItem>,
    pub raw_next: Vec<ProvidedTrack>,
    pub raw_prev: Vec<ProvidedTrack>,
}

impl Queue {
    pub fn recent(&self, limit: usize) -> Vec<QueueItem> {
        let mut seen = HashSet::new();
        self.raw_prev
            .iter()
            .rev()
            .filter(|track| {
                !DELIMITERS.contains(&track.uri.as_str())
                    && !track.uri.starts_with("spotify:meta:")
                    && !flagged(track, "hidden")
                    && !flagged(track, "hidden_in_queue")
            })
            .filter(|track| track.uid.is_empty() || seen.insert(track.uid.as_str()))
            .take(limit)
            .map(QueueItem::of)
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    Computer,
    Smartphone,
    Tablet,
    Speaker,
    Tv,
    Receiver,
    GameConsole,
    Car,
    Watch,
    Group,
    #[default]
    Other,
}

impl DeviceKind {
    pub fn from_type(device_type: DeviceType) -> Self {
        match device_type {
            DeviceType::Computer | DeviceType::Chromebook => Self::Computer,
            DeviceType::Smartphone => Self::Smartphone,
            DeviceType::Tablet => Self::Tablet,
            DeviceType::Speaker | DeviceType::CastAudio | DeviceType::AudioDongle => Self::Speaker,
            DeviceType::Tv | DeviceType::CastVideo | DeviceType::Stb => Self::Tv,
            DeviceType::Avr => Self::Receiver,
            DeviceType::GameConsole => Self::GameConsole,
            DeviceType::Automobile => Self::Car,
            DeviceType::Smartwatch => Self::Watch,
            _ => Self::Other,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Computer => "laptopcomputer",
            Self::Smartphone => "iphone",
            Self::Tablet => "ipad",
            Self::Speaker => "hifispeaker",
            Self::Tv => "tv",
            Self::Receiver => "hifireceiver",
            Self::GameConsole => "gamecontroller",
            Self::Car => "car",
            Self::Watch => "applewatch",
            Self::Group => "hifispeaker.2",
            Self::Other => "speaker.wave.2",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectDevice {
    pub id: String,
    pub alias_id: Option<u32>,
    pub name: String,
    pub kind: DeviceKind,
    pub active: bool,
    pub this_mac: bool,
    pub volume: Option<u16>,
    pub volume_steps: u32,
    pub blocked: Option<String>,
    pub supports_set_options: bool,
}

impl ConnectDevice {
    pub fn target(&self) -> (&str, Option<u32>) {
        device_target(&self.id)
    }
}

pub fn device_target(id: &str) -> (&str, Option<u32>) {
    match id.split_once(ALIAS_SEPARATOR) {
        Some((device, alias)) => match alias.parse() {
            Ok(alias) => (device, Some(alias)),
            Err(_) => (id, None),
        },
        None => (id, None),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Playback {
    pub active_device_id: Option<String>,
    pub active_device_name: Option<String>,
    pub active_volume: Option<u16>,
    pub own_device_id: String,
    pub devices: Vec<ConnectDevice>,
    pub context_uri: Option<String>,
    pub context_title: Option<String>,
    pub context_metadata: HashMap<String, String>,
    pub session_id: String,
    pub track: Option<Track>,
    pub track_metadata: HashMap<String, String>,
    pub provider: Option<String>,
    pub playing: bool,
    pub position_ms: u32,
    pub playback_speed: f32,
    pub shuffle: Shuffle,
    pub repeat: Repeat,
    pub allowed: Allowed,
    pub queue: Queue,
    pub signals: Vec<String>,
    pub sleep_timer: SleepState,
    pub jam_mode: Option<String>,
}

impl Playback {
    pub fn active_device(&self) -> Option<&ConnectDevice> {
        self.devices.iter().find(|device| device.active)
    }
}

pub fn playback(cluster: &Cluster, own_device_id: &str) -> Playback {
    let active = Some(cluster.active_device_id.clone()).filter(|id| !id.is_empty());
    let active_info = active.as_ref().and_then(|id| cluster.device.get(id));
    let state = cluster.player_state.as_ref();
    let provided = state
        .and_then(|state| state.track.as_ref())
        .filter(|track| !track.uri.is_empty());
    let playing =
        active.is_some() && state.is_some_and(|state| state.is_playing && !state.is_paused);
    let options = state.and_then(|state| state.options.as_ref());
    let mode = |key: &str| options.and_then(|options| options.modes.get(key)).cloned();
    let shuffling = options.is_some_and(|options| options.shuffling_context);
    let shuffle = match (shuffling, mode("context_enhancement").as_deref()) {
        (true, Some("RECOMMENDATION")) => Shuffle::Smart,
        (true, _) => Shuffle::On,
        _ => Shuffle::Off,
    };
    let repeat = match options {
        Some(options) if options.repeating_track => Repeat::Track,
        Some(options) if options.repeating_context => Repeat::Context,
        _ => Repeat::Off,
    };
    let smart_capable = active_info
        .and_then(|info| info.capabilities.as_ref())
        .is_some_and(|capabilities| capabilities.supports_smart_shuffle_mode);
    let mut allowed = match (provided, state) {
        (Some(_), Some(state)) => allowed(state.restrictions.as_ref(), smart_capable),
        _ => Allowed::default(),
    };
    if active.is_none() && provided.is_some() {
        allowed.pause = false;
        allowed.resume = true;
    }
    let context_metadata = state
        .map(|state| state.context_metadata.clone())
        .unwrap_or_default();
    let speed = options
        .and_then(|options| options.playback_speed)
        .filter(|speed| *speed > 0.)
        .or_else(|| {
            state
                .map(|state| state.playback_speed as f32)
                .filter(|speed| *speed > 0.)
        })
        .unwrap_or(1.);
    Playback {
        active_device_name: active_info.map(|info| info.name.clone()),
        active_volume: active_info.map(|info| info.volume.min(u32::from(u16::MAX)) as u16),
        own_device_id: own_device_id.to_owned(),
        devices: devices(cluster, active.as_deref(), own_device_id, state),
        context_uri: state
            .map(|state| state.context_uri.clone())
            .filter(|uri| !uri.is_empty()),
        context_title: context_title(&context_metadata),
        context_metadata,
        session_id: state
            .map(|state| state.session_id.clone())
            .unwrap_or_default(),
        track: state.and_then(|state| provided.map(|provided| track(provided, state))),
        track_metadata: provided
            .map(|provided| provided.metadata.clone())
            .unwrap_or_default(),
        provider: provided
            .map(|provided| provided.provider.clone())
            .filter(|provider| !provider.is_empty()),
        playing,
        position_ms: state.map_or(0, |state| position(cluster, state, playing)),
        playback_speed: speed,
        shuffle,
        repeat,
        allowed,
        queue: queue(state, provided),
        signals: state.map(|state| state.signals.clone()).unwrap_or_default(),
        sleep_timer: sleep(state),
        jam_mode: mode("jam"),
        active_device_id: active,
    }
}

fn position(cluster: &Cluster, state: &PlayerState, playing: bool) -> u32 {
    let elapsed = if playing && state.timestamp > 0 && cluster.server_timestamp_ms > state.timestamp
    {
        cluster.server_timestamp_ms - state.timestamp
    } else {
        0
    };
    (state.position_as_of_timestamp + elapsed).max(0) as u32
}

fn track(provided: &ProvidedTrack, state: &PlayerState) -> Track {
    let field = |key: &str| provided.metadata.get(key).cloned().unwrap_or_default();
    let artist = Some(field("artist_name"))
        .filter(|artist| !artist.is_empty())
        .unwrap_or_else(|| field("album_title"));
    Track {
        uri: provided.uri.clone(),
        title: field("title"),
        artist,
        album: field("album_title"),
        duration_ms: state.duration.max(0) as u32,
        cover: provided
            .metadata
            .get("image_url")
            .map(|image| image_url(image)),
        artist_uri: (!provided.artist_uri.is_empty()).then(|| provided.artist_uri.clone()),
        album_uri: (!provided.album_uri.is_empty()).then(|| provided.album_uri.clone()),
        uid: (!provided.uid.is_empty()).then(|| provided.uid.clone()),
        ..Track::default()
    }
}

fn allowed(restrictions: Option<&Restrictions>, smart_capable: bool) -> Allowed {
    let fallback = Restrictions::default();
    let restrictions = restrictions.unwrap_or(&fallback);
    let free = |reasons: &[String]| reasons.is_empty();
    let shuffle = free(&restrictions.disallow_toggling_shuffle_reasons);
    let recommendation_free = restrictions
        .disallow_setting_modes
        .get("context_enhancement")
        .and_then(|modes| modes.values.get("RECOMMENDATION"))
        .is_none_or(|reasons| reasons.reasons.iter().all(|reason| reason == "already_set"));
    Allowed {
        pause: free(&restrictions.disallow_pausing_reasons),
        resume: free(&restrictions.disallow_resuming_reasons),
        seek: free(&restrictions.disallow_seeking_reasons),
        skip_prev: free(&restrictions.disallow_skipping_prev_reasons),
        skip_next: free(&restrictions.disallow_skipping_next_reasons),
        shuffle,
        smart_shuffle: shuffle && recommendation_free && smart_capable,
        repeat_context: free(&restrictions.disallow_toggling_repeat_context_reasons),
        repeat_track: free(&restrictions.disallow_toggling_repeat_track_reasons),
        add_to_queue: free(&restrictions.disallow_add_to_queue_reasons),
        play_next: free(&restrictions.disallow_play_as_next_in_queue_reasons),
        set_queue: free(&restrictions.disallow_set_queue_reasons),
        remove_from_queue: free(&restrictions.disallow_removing_from_next_tracks_reasons),
        reorder_queue: free(&restrictions.disallow_reordering_in_next_tracks_reasons),
        jump_in_queue: free(&restrictions.disallow_skipping_next_reasons)
            && free(&restrictions.disallow_tap_to_play_track_in_next_tracks_reasons),
        view_queue: free(&restrictions.disallow_viewing_queue_reasons),
        transfer: free(&restrictions.disallow_transferring_playback_reasons),
        set_speed: free(&restrictions.disallow_setting_playback_speed_reasons),
    }
}

fn context_title(metadata: &HashMap<String, String>) -> Option<String> {
    let field = |key: &str| {
        metadata
            .get(key)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
    };
    if let Some(description) = field("context_description") {
        return Some(description.to_owned());
    }
    let title = field("station_title")?;
    Some(match field("station_subtitle") {
        Some(subtitle) => format!("{title} • {subtitle}"),
        None => title.to_owned(),
    })
}

fn flagged(track: &ProvidedTrack, key: &str) -> bool {
    track.metadata.get(key).is_some_and(|value| value == "true")
}

fn queue(state: Option<&PlayerState>, provided: Option<&ProvidedTrack>) -> Queue {
    let Some(state) = state else {
        return Queue::default();
    };
    let mut seen = HashSet::new();
    let mut queued = Vec::new();
    let mut next_up = Vec::new();
    for track in &state.next_tracks {
        if DELIMITERS.contains(&track.uri.as_str()) {
            break;
        }
        if track.uri.starts_with("spotify:meta:")
            || flagged(track, "hidden")
            || flagged(track, "hidden_in_queue")
            || (!track.uid.is_empty() && !seen.insert(track.uid.as_str()))
        {
            continue;
        }
        if next_up.is_empty() && track.provider == "queue" {
            queued.push(QueueItem::of(track));
        } else {
            next_up.push(QueueItem::of(track));
        }
    }
    Queue {
        revision: state.queue_revision.clone(),
        current: provided.map(QueueItem::of),
        queued,
        next_up,
        raw_next: state.next_tracks.clone(),
        raw_prev: state.prev_tracks.clone(),
    }
}

fn item_type(track: &ProvidedTrack) -> String {
    let media = track
        .metadata
        .get("media.type")
        .or_else(|| track.metadata.get("track_player"))
        .filter(|media| !media.is_empty())
        .map_or("audio", String::as_str);
    let kind = track.uri.split(':').nth(1).unwrap_or_default();
    format!("{media}/{kind}")
}

fn blocked(
    info: &DeviceInfo,
    capabilities: Option<&Capabilities>,
    item: Option<&str>,
    transfer_blocked: bool,
) -> Option<String> {
    if !info.can_play
        || info.is_offline
        || !info.disallow_transfer_reasons.is_empty()
        || !info.disallow_playback_reasons.is_empty()
    {
        return Some("Unavailable".into());
    }
    let supported = capabilities.map_or(&[][..], |capabilities| {
        capabilities.supported_types.as_slice()
    });
    if let Some(item) = item
        && !supported.is_empty()
        && !supported.iter().any(|kind| kind == item)
    {
        return Some("Can’t play this".into());
    }
    transfer_blocked.then(|| "Can’t move this right now".into())
}

fn devices(
    cluster: &Cluster,
    active: Option<&str>,
    own_device_id: &str,
    state: Option<&PlayerState>,
) -> Vec<ConnectDevice> {
    let item = state
        .and_then(|state| state.track.as_ref())
        .filter(|track| !track.uri.is_empty())
        .map(item_type);
    let transfer_blocked = state
        .and_then(|state| state.restrictions.as_ref())
        .is_some_and(|restrictions| {
            !restrictions
                .disallow_transferring_playback_reasons
                .is_empty()
        });
    let mut list = Vec::new();
    for (id, info) in &cluster.device {
        let capabilities = info.capabilities.as_ref();
        if capabilities.is_some_and(|capabilities| capabilities.hidden) {
            continue;
        }
        let is_active = active == Some(id.as_str());
        let volume = (!capabilities.is_some_and(|capabilities| capabilities.disable_volume))
            .then(|| info.volume.min(u32::from(u16::MAX)) as u16);
        let volume_steps = capabilities
            .map(|capabilities| capabilities.volume_steps)
            .filter(|steps| *steps > 0)
            .map_or(DEFAULT_VOLUME_STEPS, |steps| steps as u32);
        let blocked = if is_active {
            None
        } else {
            blocked(info, capabilities, item.as_deref(), transfer_blocked)
        };
        let kind = if info.is_group {
            DeviceKind::Group
        } else {
            DeviceKind::from_type(info.device_type())
        };
        let device = ConnectDevice {
            id: id.clone(),
            alias_id: None,
            name: info.name.clone(),
            kind,
            active: is_active,
            this_mac: id == own_device_id,
            volume,
            volume_steps,
            blocked,
            supports_set_options: capabilities
                .is_some_and(|capabilities| capabilities.supports_set_options_command),
        };
        if info.device_aliases.is_empty() {
            list.push(device);
            continue;
        }
        let mut aliases: Vec<_> = info.device_aliases.iter().collect();
        aliases.sort_by_key(|(alias_id, _)| **alias_id);
        for (alias_id, alias) in aliases {
            list.push(ConnectDevice {
                id: format!("{id}{ALIAS_SEPARATOR}{alias_id}"),
                alias_id: Some(*alias_id),
                name: alias.display_name.clone(),
                kind: if alias.is_group {
                    DeviceKind::Group
                } else {
                    device.kind
                },
                active: is_active && info.selected_alias_id == *alias_id,
                ..device.clone()
            });
        }
    }
    list.sort_by(|left, right| {
        right
            .active
            .cmp(&left.active)
            .then(right.this_mac.cmp(&left.this_mac))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
    list
}

fn sleep(state: Option<&PlayerState>) -> SleepState {
    match state
        .and_then(|state| state.sleep_timer.as_ref())
        .and_then(|timer| timer.timer_type.as_ref())
    {
        Some(TimerType::Timestamp(at)) => {
            SleepState::At(UNIX_EPOCH + Duration::from_millis(at.timestamp.max(0) as u64))
        }
        Some(TimerType::EndOfTrack(_)) => SleepState::EndOfTrack,
        _ => SleepState::Off,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandError {
    Refused(String),
    DeviceGone,
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(body) if body.trim().is_empty() => {
                write!(formatter, "Spotify refused the command.")
            }
            Self::Refused(body) => write!(formatter, "Spotify refused the command: {body}"),
            Self::DeviceGone => write!(formatter, "That device is no longer available."),
        }
    }
}

impl std::error::Error for CommandError {}

pub fn command_id() -> String {
    (0..16)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect()
}

pub fn envelope(command: Value, command_id: &str, alias: Option<u32>) -> Value {
    let mut command = command;
    if !command["logging_params"].is_object() {
        command["logging_params"] = json!({});
    }
    if command["logging_params"].get("command_id").is_none() {
        command["logging_params"]["command_id"] = json!(command_id);
    }
    let mut body = json!({ "command": command });
    if let Some(alias) = alias {
        body["target_alias_id"] = json!(alias);
    }
    body
}

pub fn command_reply(status: StatusCode, body: &[u8]) -> Result<Option<String>> {
    match status {
        StatusCode::FORBIDDEN => {
            bail!(CommandError::Refused(
                String::from_utf8_lossy(body).into_owned()
            ))
        }
        StatusCode::NOT_FOUND => bail!(CommandError::DeviceGone),
        status if !status.is_success() => bail!(HttpStatus(status)),
        _ => Ok(serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|reply| {
                reply["ack_id"]
                    .as_str()
                    .filter(|ack| !ack.is_empty())
                    .map(str::to_owned)
            })),
    }
}

pub async fn command(
    api: &Api,
    from: &str,
    to: &str,
    command: Value,
    alias: Option<u32>,
) -> Result<Option<String>> {
    let (status, bytes) = api
        .send(
            Method::POST,
            &format!("/connect-state/v1/player/command/from/{from}/to/{to}"),
            Body::Json(envelope(command, &command_id(), alias)),
            "application/json",
        )
        .await?;
    command_reply(status, &bytes)
}

pub fn volume_body(volume: u16, alias: Option<u32>) -> Value {
    let mut body = json!({ "volume": volume });
    if let Some(alias) = alias {
        body["command_options"] = json!({ "target_alias_id": alias });
    }
    body
}

pub async fn set_volume(
    api: &Api,
    from: &str,
    to: &str,
    volume: u16,
    alias: Option<u32>,
) -> Result<()> {
    api.request(
        Method::PUT,
        &format!("/connect-state/v1/connect/volume/from/{from}/to/{to}"),
        Body::Json(volume_body(volume, alias)),
        "application/json",
    )
    .await?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Restore {
    #[default]
    Keep,
    Pause,
    Resume,
}

impl Restore {
    fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "restore",
            Self::Pause => "pause",
            Self::Resume => "resume",
        }
    }
}

pub fn transfer_body(restore: Restore, command_id: &str, alias: Option<u32>) -> Value {
    let mut body = json!({
        "transfer_options": { "restore_paused": restore.as_str() },
        "command_id": command_id,
    });
    if let Some(alias) = alias {
        body["target_alias_id"] = json!(alias);
    }
    body
}

pub async fn transfer(
    api: &Api,
    from: &str,
    to: &str,
    restore: Restore,
    alias: Option<u32>,
) -> Result<()> {
    api.request(
        Method::POST,
        &format!("/connect-state/v1/connect/transfer/from/{from}/to/{to}"),
        Body::Json(transfer_body(restore, &command_id(), alias)),
        "application/json",
    )
    .await?;
    Ok(())
}

pub fn play_command(uris: &[String], index: usize, position_ms: u32) -> Value {
    let tracks: Vec<Value> = uris.iter().map(|uri| json!({ "uri": uri })).collect();
    let mut options = json!({ "skip_to": { "track_index": index, "track_uri": uris.get(index) } });
    if position_ms > 0 {
        options["seek_to"] = json!(position_ms);
    }
    json!({
        "endpoint": "play",
        "context": {
            "uri": format!("spotify:app:openmusic:{}", uris.get(index).map(String::as_str).unwrap_or_default()),
            "url": "",
            "metadata": {},
            "pages": [{ "tracks": tracks }],
        },
        "play_origin": { "feature_identifier": ORIGIN },
        "options": options,
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayOptions {
    pub description: Option<String>,
    pub shuffle_override: Option<Shuffle>,
}

pub fn player_options(shuffle: Shuffle) -> Value {
    json!({
        "shuffling_context": shuffle != Shuffle::Off,
        "modes": {
            "context_enhancement": if shuffle == Shuffle::Smart { "RECOMMENDATION" } else { "NONE" },
        },
    })
}

pub fn play_context_command(
    context: &str,
    index: usize,
    track_uri: Option<&str>,
    origin: &str,
    options: &PlayOptions,
) -> Value {
    let mut skip_to = json!({ "track_index": index });
    if let Some(track_uri) = track_uri {
        skip_to["track_uri"] = json!(track_uri);
    }
    let mut metadata = json!({});
    if let Some(description) = &options.description {
        metadata["context_description"] = json!(description);
    }
    let mut play_options = json!({ "skip_to": skip_to });
    if let Some(shuffle) = options.shuffle_override {
        play_options["player_options_override"] = player_options(shuffle);
    }
    json!({
        "endpoint": "play",
        "context": { "uri": context, "url": format!("context://{context}"), "metadata": metadata },
        "play_origin": { "feature_identifier": origin },
        "options": play_options,
    })
}

pub fn shuffle_play_command(context: &str, origin: &str, options: &PlayOptions) -> Value {
    let options = PlayOptions {
        shuffle_override: Some(
            options
                .shuffle_override
                .filter(|mode| *mode != Shuffle::Off)
                .unwrap_or(Shuffle::On),
        ),
        ..options.clone()
    };
    let mut command = play_context_command(context, 0, None, origin, &options);
    if let Some(play_options) = command["options"].as_object_mut() {
        play_options.remove("skip_to");
    }
    command
}

fn enhancement(mode: Shuffle) -> &'static str {
    if mode == Shuffle::Smart {
        "RECOMMENDATION"
    } else {
        "NONE"
    }
}

pub fn shuffle_command(mode: Shuffle, use_options: bool) -> Value {
    if use_options {
        json!({
            "endpoint": "set_options",
            "shuffling_context": mode != Shuffle::Off,
            "modes": { "context_enhancement": enhancement(mode) },
        })
    } else {
        json!({ "endpoint": "set_shuffling_context", "value": mode != Shuffle::Off })
    }
}

pub fn repeat_commands(mode: Repeat, allowed: &Allowed, use_options: bool) -> Vec<Value> {
    let context = mode != Repeat::Off;
    let track = mode == Repeat::Track;
    if use_options {
        let mut command = json!({ "endpoint": "set_options" });
        if allowed.repeat_context {
            command["repeating_context"] = json!(context);
        }
        if allowed.repeat_track {
            command["repeating_track"] = json!(track);
        }
        return if command.as_object().is_some_and(|fields| fields.len() > 1) {
            vec![command]
        } else {
            Vec::new()
        };
    }
    let mut commands = Vec::new();
    if allowed.repeat_context {
        commands.push(json!({ "endpoint": "set_repeating_context", "value": context }));
    }
    if allowed.repeat_track {
        commands.push(json!({ "endpoint": "set_repeating_track", "value": track }));
    }
    commands
}

pub fn queue_item(track: &ProvidedTrack) -> Value {
    let mut item = json!({
        "uri": track.uri,
        "uid": track.uid,
        "metadata": track.metadata,
        "provider": track.provider,
    });
    if !track.removed.is_empty() {
        item["removed"] = json!(track.removed);
    }
    if !track.blocked.is_empty() {
        item["blocked"] = json!(track.blocked);
    }
    item
}

pub fn set_queue_command(next: &[ProvidedTrack], prev: &[ProvidedTrack], revision: &str) -> Value {
    json!({
        "endpoint": "set_queue",
        "next_tracks": next.iter().map(queue_item).collect::<Vec<_>>(),
        "prev_tracks": prev.iter().map(queue_item).collect::<Vec<_>>(),
        "queue_revision": revision,
    })
}

pub fn skip_to_command(item: &ProvidedTrack) -> Value {
    json!({ "endpoint": "skip_next", "track": queue_item(item) })
}

pub fn add_to_queue_command(uri: &str) -> Value {
    json!({
        "endpoint": "add_to_queue",
        "track": { "uri": uri, "metadata": { "is_queued": "true" }, "provider": "queue" },
    })
}

pub fn play_next_command(uris: &[String]) -> Value {
    json!({
        "endpoint": "play_as_next_in_queue",
        "tracks": uris
            .iter()
            .map(|uri| json!({ "uri": uri, "uid": "", "metadata": {} }))
            .collect::<Vec<_>>(),
    })
}

pub fn update_context_command(context_uri: &str, session_id: &str) -> Value {
    json!({
        "endpoint": "update_context",
        "context": { "url": format!("context://{context_uri}") },
        "session_id": session_id,
    })
}

fn same_item(track: &ProvidedTrack, key: &QueueKey) -> bool {
    if !track.uid.is_empty() && !key.uid.is_empty() {
        let iteration = track.metadata.get("iteration").map_or("", String::as_str);
        return track.uid == key.uid && iteration == key.iteration;
    }
    track.uri == key.uri
}

fn is_queued(track: &ProvidedTrack) -> bool {
    track.provider == "queue"
}

fn mark_queued(track: &mut ProvidedTrack) {
    track.provider = "queue".into();
    track.metadata.insert("is_queued".into(), "true".into());
}

pub fn queue_without(next: &[ProvidedTrack], keys: &[QueueKey]) -> Vec<ProvidedTrack> {
    next.iter()
        .filter(|track| !keys.iter().any(|key| same_item(track, key)))
        .cloned()
        .collect()
}

pub fn queue_cleared(next: &[ProvidedTrack]) -> Vec<ProvidedTrack> {
    next.iter()
        .filter(|track| !is_queued(track))
        .cloned()
        .collect()
}

pub fn queue_moved(
    next: &[ProvidedTrack],
    moved: &[QueueKey],
    anchor: &QueueKey,
    after: bool,
) -> Vec<ProvidedTrack> {
    if moved.iter().any(|key| key == anchor) {
        return next.to_vec();
    }
    let (mut taken, mut rest): (Vec<ProvidedTrack>, Vec<ProvidedTrack>) = next
        .iter()
        .cloned()
        .partition(|track| moved.iter().any(|key| same_item(track, key)));
    let Some(position) = rest.iter().position(|track| same_item(track, anchor)) else {
        return next.to_vec();
    };
    if taken.is_empty() {
        return next.to_vec();
    }
    if is_queued(&rest[position]) {
        taken.iter_mut().for_each(mark_queued);
    }
    let at = if after { position + 1 } else { position };
    rest.splice(at..at, taken);
    rest
}

fn transparent(track: &ProvidedTrack) -> bool {
    track.uri.starts_with("spotify:meta:")
        || flagged(track, "hidden")
        || flagged(track, "is_narration")
}

pub fn queue_with_added(next: &[ProvidedTrack], uris: &[String]) -> Vec<ProvidedTrack> {
    let mut at = 0;
    for (index, track) in next.iter().enumerate() {
        if DELIMITERS.contains(&track.uri.as_str()) {
            break;
        }
        if is_queued(track) || flagged(track, "is_narration") {
            at = index + 1;
        } else if !transparent(track) {
            break;
        }
    }
    let added = uris.iter().map(|uri| {
        let mut track = ProvidedTrack {
            uri: uri.clone(),
            ..Default::default()
        };
        mark_queued(&mut track);
        track
    });
    let mut list = next.to_vec();
    list.splice(at..at, added);
    list
}

pub const SMART_SHUFFLE: Operation = Operation::query(
    "smartShuffle",
    "3384085be84fbf2f855b024f99bc06cded1c0fd71af3a8fb8abb84e9656faba2",
);
pub const CONTEXT_PLAYER_OPTIONS: Operation = Operation::query(
    "centralisedStatePlayerOptions",
    "e2dcfcab470854d4d1c7cb1a851438f14fe0a94d57db7f0b9dde492559d5395d",
);
pub const LIKED_SONGS_LENS: &str = "spotify:playlist:37i9dQZF1F5p3rmiWPIYgZ";
const PLAYER_CLIENT_VERSION: &str = "harmony:4.67.0-887cfb0c3";

pub fn smart_shuffle_uri(context: &str) -> String {
    if crate::entity::is_liked_songs(context) {
        LIKED_SONGS_LENS.to_owned()
    } else {
        context.to_owned()
    }
}

pub fn parse_smart_shuffle(data: &Value) -> bool {
    data["lookup"]
        .as_array()
        .and_then(|lookup| lookup.first())
        .and_then(|entry| entry["data"]["smartShuffle"]["available"].as_bool())
        .unwrap_or(false)
}

pub async fn smart_shuffle(api: &Api, context: &str) -> Result<bool> {
    let data = pathfinder::query(
        api,
        SMART_SHUFFLE,
        json!({ "uris": [smart_shuffle_uri(context)] }),
    )
    .await?;
    Ok(parse_smart_shuffle(&data))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextOptions {
    pub shuffle: Shuffle,
    pub shuffle_allowed: bool,
    pub smart_allowed: bool,
}

pub fn context_options_variables(context: &str, device_id: &str) -> Value {
    json!({
        "uri": context,
        "deviceInfo": {
            "deviceId": device_id,
            "deviceType": "computer",
            "clientId": crate::oauth::DESKTOP_CLIENT_ID,
            "clientVersion": PLAYER_CLIENT_VERSION,
            "productId": "1",
        },
    })
}

pub fn parse_context_options(data: &Value) -> Option<ContextOptions> {
    let options = data
        .get("playerOptions")
        .filter(|value| value.is_object())?;
    let modes = options["modes"].as_array();
    let enhancement = modes
        .into_iter()
        .flatten()
        .find(|mode| mode["modeKey"] == "context_enhancement")
        .and_then(|mode| mode["modeValue"].as_str())
        .unwrap_or("NONE");
    let shuffling = options["shufflingContext"].as_bool().unwrap_or(false);
    let restrictions = &options["restrictions"];
    let empty = |value: &Value| value.as_array().is_none_or(|reasons| reasons.is_empty());
    let shuffle_allowed = empty(&restrictions["disallowTogglingShuffleReasons"]);
    let recommendation_free = restrictions["disallowSettingModes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|mode| mode["modeKey"] == "context_enhancement")
        .flat_map(|mode| mode["modeRestrictions"].as_array().into_iter().flatten())
        .filter(|restriction| restriction["modeValue"] == "RECOMMENDATION")
        .flat_map(|restriction| restriction["reasons"].as_array().into_iter().flatten())
        .all(|reason| reason == "already_set");
    Some(ContextOptions {
        shuffle: match (shuffling, enhancement) {
            (true, "RECOMMENDATION") => Shuffle::Smart,
            (true, _) => Shuffle::On,
            _ => Shuffle::Off,
        },
        shuffle_allowed,
        smart_allowed: shuffle_allowed && recommendation_free,
    })
}

pub async fn context_options(api: &Api, context: &str, device_id: &str) -> Result<ContextOptions> {
    let data = pathfinder::query(
        api,
        CONTEXT_PLAYER_OPTIONS,
        context_options_variables(context, device_id),
    )
    .await?;
    parse_context_options(&data).ok_or_else(|| anyhow::anyhow!("Spotify sent no player options."))
}

#[cfg(test)]
mod tests {
    use spotify_proto::connectstate::{
        ContextIndex, ContextPlayerOptions, ModeRestrictions, RestrictionReasons, SleepTimer,
        devices::DeviceAlias, sleep_timer,
    };

    use super::*;

    const OWN: &str = "aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000";
    const DESKTOP: &str = "bbbb1111bbbb1111bbbb1111bbbb1111bbbb1111";
    const TV: &str = "cccc2222cccc2222cccc2222cccc2222cccc2222";
    const GROUP: &str = "dddd3333dddd3333dddd3333dddd3333dddd3333";
    const OBSERVER: &str = "hobs_aaaa0000aaaa0000aaaa0000aaaa0000aaa";
    const PHONE: &str = "eeee4444eeee4444eeee4444eeee4444eeee4444";

    fn metadata(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn item(uri: &str, uid: &str, provider: &str, pairs: &[(&str, &str)]) -> ProvidedTrack {
        ProvidedTrack {
            uri: uri.into(),
            uid: uid.into(),
            provider: provider.into(),
            metadata: metadata(pairs),
            ..Default::default()
        }
    }

    fn reasons(list: &[&str]) -> Vec<String> {
        list.iter().map(|reason| (*reason).to_owned()).collect()
    }

    fn capabilities(types: &[&str]) -> Capabilities {
        Capabilities {
            volume_steps: 64,
            supported_types: reasons(types),
            supports_set_options_command: true,
            supports_smart_shuffle_mode: true,
            ..Default::default()
        }
    }

    fn device(id: &str, name: &str, kind: DeviceType, volume: u32) -> DeviceInfo {
        DeviceInfo {
            device_id: id.into(),
            name: name.into(),
            device_type: kind as i32,
            can_play: true,
            volume,
            capabilities: Some(capabilities(&["audio/track", "audio/episode"])),
            ..Default::default()
        }
    }

    fn modes(enhancement: &str) -> HashMap<String, String> {
        metadata(&[
            ("context_enhancement", enhancement),
            ("media", "audio"),
            ("jam", "off"),
        ])
    }

    fn restrictions(recommendation: &[&str]) -> Restrictions {
        let mut values = HashMap::new();
        values.insert(
            "NONE".to_owned(),
            RestrictionReasons {
                reasons: reasons(&["already_set"]),
            },
        );
        values.insert(
            "RECOMMENDATION".to_owned(),
            RestrictionReasons {
                reasons: reasons(recommendation),
            },
        );
        let mut setting_modes = HashMap::new();
        setting_modes.insert(
            "context_enhancement".to_owned(),
            ModeRestrictions { values },
        );
        Restrictions {
            disallow_resuming_reasons: reasons(&["not_paused"]),
            disallow_setting_playback_speed_reasons: reasons(&[
                "not_supported_by_device",
                "not_supported_by_content_type",
            ]),
            disallow_sleep_timer_clear_reasons: reasons(&["no_sleep_timer_set"]),
            disallow_setting_modes: setting_modes,
            ..Default::default()
        }
    }

    fn cluster() -> Cluster {
        let mut devices = HashMap::new();
        devices.insert(
            OWN.to_owned(),
            device(OWN, "OpenMusic", DeviceType::Computer, 13577),
        );
        devices.insert(
            DESKTOP.to_owned(),
            device(DESKTOP, "Desktop", DeviceType::Computer, 65535),
        );
        let mut tv = device(TV, "Television", DeviceType::Tv, 17476);
        tv.capabilities = Some(Capabilities {
            disable_volume: true,
            volume_steps: 0,
            ..capabilities(&["audio/track", "audio/episode", "video/track"])
        });
        devices.insert(TV.to_owned(), tv);
        let mut observer = device(OBSERVER, "OpenMusic", DeviceType::Computer, 0);
        observer.capabilities = Some(Capabilities {
            hidden: true,
            ..Default::default()
        });
        devices.insert(OBSERVER.to_owned(), observer);
        let mut group = device(GROUP, "Speakers", DeviceType::Speaker, 30000);
        group.selected_alias_id = 3;
        group.device_aliases.insert(
            3,
            DeviceAlias {
                id: 3,
                display_name: "Kitchen".into(),
                is_group: false,
                ..Default::default()
            },
        );
        group.device_aliases.insert(
            7,
            DeviceAlias {
                id: 7,
                display_name: "Everywhere".into(),
                is_group: true,
                ..Default::default()
            },
        );
        devices.insert(GROUP.to_owned(), group);
        let mut phone = device(PHONE, "Phone", DeviceType::Smartphone, 40000);
        phone.capabilities = Some(capabilities(&["audio/episode"]));
        devices.insert(PHONE.to_owned(), phone);

        let current = item(
            "spotify:track:0123456789abcdefABCDEF",
            "0011223344556677aabb",
            "context",
            &[
                ("title", "Song"),
                ("artist_name", "Singer"),
                ("album_title", "Record"),
                ("image_url", "spotify:image:ab67616d00001e02aa"),
                ("iteration", "0"),
                ("track_player", "audio"),
            ],
        );
        Cluster {
            active_device_id: OWN.into(),
            server_timestamp_ms: 1_000_005_000,
            device: devices,
            player_state: Some(PlayerState {
                timestamp: 1_000_000_000,
                context_uri: "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M".into(),
                context_metadata: metadata(&[("context_description", "Mix")]),
                index: Some(ContextIndex { page: 0, track: 11 }),
                track: Some(current),
                session_id: "session".into(),
                playback_speed: 1.,
                position_as_of_timestamp: 30_000,
                duration: 152_054,
                is_playing: true,
                options: Some(ContextPlayerOptions {
                    modes: modes("NONE"),
                    playback_speed: Some(1.),
                    ..Default::default()
                }),
                restrictions: Some(restrictions(&["not_supported_by_content_type"])),
                next_tracks: vec![
                    item(
                        "spotify:track:1111111111111111111111",
                        "q1",
                        "queue",
                        &[("is_queued", "true"), ("title", "Queued one")],
                    ),
                    item("spotify:meta:page:1", "", "context", &[("hidden", "true")]),
                    item(
                        "spotify:track:2222222222222222222222",
                        "q2",
                        "queue",
                        &[("is_queued", "true")],
                    ),
                    item(
                        "spotify:track:3333333333333333333333",
                        "c1",
                        "context",
                        &[("iteration", "0"), ("artist_name", "Band")],
                    ),
                    item(
                        "spotify:track:4444444444444444444444",
                        "c1",
                        "context",
                        &[("iteration", "0")],
                    ),
                    item(
                        "spotify:track:5555555555555555555555",
                        "c2",
                        "context",
                        &[("hidden", "true")],
                    ),
                    item(
                        "spotify:track:6666666666666666666666",
                        "c3",
                        "context",
                        &[("hidden_in_queue", "true")],
                    ),
                    item(
                        "spotify:track:7777777777777777777777",
                        "q3",
                        "queue",
                        &[("is_queued", "true")],
                    ),
                    item(
                        "spotify:delimiter",
                        "delimiter0",
                        "context",
                        &[("hidden", "true"), ("iteration", "0")],
                    ),
                    item(
                        "spotify:track:8888888888888888888888",
                        "c4",
                        "context",
                        &[("iteration", "1")],
                    ),
                ],
                prev_tracks: vec![item(
                    "spotify:track:9999999999999999999999",
                    "p1",
                    "context",
                    &[],
                )],
                queue_revision: "1234567890123456789".into(),
                sleep_timer: Some(SleepTimer {
                    timer_type: Some(TimerType::None(sleep_timer::None {})),
                }),
                signals: vec!["jump".into()],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn with_options(shuffle: bool, enhancement: &str, context: bool, track: bool) -> Playback {
        let mut cluster = cluster();
        let state = cluster.player_state.as_mut().unwrap();
        state.options = Some(ContextPlayerOptions {
            shuffling_context: shuffle,
            repeating_context: context,
            repeating_track: track,
            modes: modes(enhancement),
            ..Default::default()
        });
        playback(&cluster, OWN)
    }

    #[test]
    fn maps_shuffle_and_repeat() {
        let plain = with_options(false, "NONE", false, false);
        assert_eq!((plain.shuffle, plain.repeat), (Shuffle::Off, Repeat::Off));
        let shuffled = with_options(true, "NONE", true, false);
        assert_eq!(
            (shuffled.shuffle, shuffled.repeat),
            (Shuffle::On, Repeat::Context)
        );
        let smart = with_options(true, "RECOMMENDATION", true, true);
        assert_eq!(
            (smart.shuffle, smart.repeat),
            (Shuffle::Smart, Repeat::Track)
        );
        let recommendation_only = with_options(false, "RECOMMENDATION", false, true);
        assert_eq!(
            (recommendation_only.shuffle, recommendation_only.repeat),
            (Shuffle::Off, Repeat::Track)
        );
    }

    #[test]
    fn reads_the_playing_state() {
        let model = playback(&cluster(), OWN);
        assert_eq!(model.active_device_id.as_deref(), Some(OWN));
        assert_eq!(model.active_device_name.as_deref(), Some("OpenMusic"));
        assert_eq!(model.active_volume, Some(13577));
        assert_eq!(model.own_device_id, OWN);
        assert!(model.playing);
        assert_eq!(model.position_ms, 35_000);
        assert_eq!(
            model.context_uri.as_deref(),
            Some("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M")
        );
        assert_eq!(model.context_title.as_deref(), Some("Mix"));
        assert_eq!(model.session_id, "session");
        assert_eq!(model.provider.as_deref(), Some("context"));
        assert_eq!(model.signals, vec!["jump".to_owned()]);
        assert_eq!(model.sleep_timer, SleepState::Off);
        assert_eq!(model.playback_speed, 1.);
        assert_eq!(model.jam_mode.as_deref(), Some("off"));
        assert_eq!(model.track_metadata["track_player"], "audio");
        let track = model.track.as_ref().unwrap();
        assert_eq!(track.uri, "spotify:track:0123456789abcdefABCDEF");
        assert_eq!(track.title, "Song");
        assert_eq!(track.artist, "Singer");
        assert_eq!(track.album, "Record");
        assert_eq!(track.duration_ms, 152_054);
        assert_eq!(
            track.cover.as_deref(),
            Some("https://i.scdn.co/image/ab67616d00001e02aa")
        );
        assert_eq!(track.uid.as_deref(), Some("0011223344556677aabb"));
        assert_eq!(
            model.active_device().map(|device| device.id.as_str()),
            Some(OWN)
        );
    }

    #[test]
    fn checks_restrictions() {
        let model = playback(&cluster(), OWN);
        let allowed = model.allowed;
        assert!(allowed.pause && !allowed.resume && allowed.seek);
        assert!(allowed.skip_prev && allowed.skip_next);
        assert!(allowed.shuffle && !allowed.smart_shuffle);
        assert!(allowed.repeat_context && allowed.repeat_track);
        assert!(allowed.add_to_queue && allowed.play_next && allowed.set_queue);
        assert!(allowed.remove_from_queue && allowed.reorder_queue && allowed.jump_in_queue);
        assert!(allowed.view_queue && allowed.transfer && !allowed.set_speed);

        let mut eligible = cluster();
        eligible
            .player_state
            .as_mut()
            .unwrap()
            .restrictions
            .replace(restrictions(&["already_set"]));
        assert!(playback(&eligible, OWN).allowed.smart_shuffle);
        let mut empty = cluster();
        empty
            .player_state
            .as_mut()
            .unwrap()
            .restrictions
            .replace(restrictions(&[]));
        assert!(playback(&empty, OWN).allowed.smart_shuffle);
        if let Some(capabilities) = empty
            .device
            .get_mut(OWN)
            .and_then(|info| info.capabilities.as_mut())
        {
            capabilities.supports_smart_shuffle_mode = false;
        }
        assert!(!playback(&empty, OWN).allowed.smart_shuffle);
    }

    #[test]
    fn splits_the_queue() {
        let queue = playback(&cluster(), OWN).queue;
        assert_eq!(queue.revision, "1234567890123456789");
        assert_eq!(
            queue.current.as_ref().map(|item| item.uid.as_str()),
            Some("0011223344556677aabb")
        );
        let uids = |items: &[QueueItem]| {
            items
                .iter()
                .map(|item| item.uid.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(uids(&queue.queued), vec!["q1", "q2"]);
        assert_eq!(uids(&queue.next_up), vec!["c1", "q3"]);
        assert_eq!(queue.queued[0].title.as_deref(), Some("Queued one"));
        assert_eq!(queue.queued[0].metadata["is_queued"], "true");
        assert_eq!(queue.next_up[0].artist.as_deref(), Some("Band"));
        assert_eq!(queue.next_up[0].iteration, "0");
        assert_eq!(
            queue.next_up[0].key(),
            QueueKey {
                uid: "c1".into(),
                iteration: "0".into(),
                uri: "spotify:track:3333333333333333333333".into(),
            }
        );
        assert_eq!(queue.raw_next.len(), 10);
        assert_eq!(queue.raw_prev.len(), 1);
        assert_eq!(QueueKey::of(&queue.raw_next[3]), queue.next_up[0].key());
    }

    #[test]
    fn lists_recently_played_newest_first() {
        let queue = Queue {
            raw_prev: vec![
                item("spotify:track:0000000000000000000000", "p0", "context", &[]),
                item(
                    "spotify:delimiter",
                    "delimiter0",
                    "context",
                    &[("hidden", "true")],
                ),
                item("spotify:track:1111111111111111111111", "p1", "context", &[]),
                item("spotify:meta:page:0", "", "context", &[]),
                item(
                    "spotify:track:2222222222222222222222",
                    "p2",
                    "context",
                    &[("hidden", "true")],
                ),
                item("spotify:track:3333333333333333333333", "p1", "context", &[]),
                item(
                    "spotify:track:4444444444444444444444",
                    "p4",
                    "queue",
                    &[("title", "Queued")],
                ),
            ],
            ..Queue::default()
        };
        let recent = queue.recent(10);
        let uids: Vec<&str> = recent.iter().map(|item| item.uid.as_str()).collect();
        assert_eq!(uids, vec!["p4", "p1", "p0"]);
        assert_eq!(recent[0].title.as_deref(), Some("Queued"));
        assert_eq!(recent[1].uri, "spotify:track:3333333333333333333333");
        assert_eq!(queue.recent(2).len(), 2);
    }

    #[test]
    fn lists_devices() {
        let devices = playback(&cluster(), OWN).devices;
        let summary: Vec<_> = devices
            .iter()
            .map(|device| {
                (
                    device.id.as_str(),
                    device.name.as_str(),
                    device.kind,
                    device.active,
                    device.this_mac,
                )
            })
            .collect();
        let kitchen = format!("{GROUP}::a_3");
        let everywhere = format!("{GROUP}::a_7");
        assert_eq!(
            summary,
            vec![
                (OWN, "OpenMusic", DeviceKind::Computer, true, true),
                (DESKTOP, "Desktop", DeviceKind::Computer, false, false),
                (
                    everywhere.as_str(),
                    "Everywhere",
                    DeviceKind::Group,
                    false,
                    false
                ),
                (
                    kitchen.as_str(),
                    "Kitchen",
                    DeviceKind::Speaker,
                    false,
                    false
                ),
                (PHONE, "Phone", DeviceKind::Smartphone, false, false),
                (TV, "Television", DeviceKind::Tv, false, false),
            ]
        );
        let by_id = |id: &str| devices.iter().find(|device| device.id == id).unwrap();
        assert_eq!(by_id(TV).volume, None);
        assert_eq!(by_id(TV).volume_steps, DEFAULT_VOLUME_STEPS);
        assert_eq!(by_id(OWN).volume, Some(13577));
        assert_eq!(by_id(PHONE).blocked.as_deref(), Some("Can’t play this"));
        assert_eq!(by_id(DESKTOP).blocked, None);
        assert_eq!(by_id(&kitchen).alias_id, Some(3));
        assert_eq!(by_id(&kitchen).target(), (GROUP, Some(3)));
        assert_eq!(by_id(OWN).target(), (OWN, None));
        assert!(by_id(OWN).supports_set_options);
        assert_eq!(DeviceKind::Group.symbol(), "hifispeaker.2");
        assert_eq!(DeviceKind::Tv.symbol(), "tv");
    }

    #[test]
    fn marks_blocked_devices_and_active_aliases() {
        let mut cluster = cluster();
        cluster.active_device_id = GROUP.into();
        if let Some(desktop) = cluster.device.get_mut(DESKTOP) {
            desktop.is_offline = true;
        }
        if let Some(restrictions) = cluster
            .player_state
            .as_mut()
            .and_then(|state| state.restrictions.as_mut())
        {
            restrictions.disallow_transferring_playback_reasons = reasons(&["dj"]);
        }
        let model = playback(&cluster, OWN);
        let by_id = |id: &str| model.devices.iter().find(|device| device.id == id).unwrap();
        assert_eq!(model.devices[0].id, format!("{GROUP}::a_3"));
        assert!(model.devices[0].active);
        assert!(!by_id(&format!("{GROUP}::a_7")).active);
        assert!(model.devices[1].this_mac);
        assert_eq!(by_id(DESKTOP).blocked.as_deref(), Some("Unavailable"));
        assert_eq!(
            by_id(OWN).blocked.as_deref(),
            Some("Can’t move this right now")
        );
        assert_eq!(model.active_device_name.as_deref(), Some("Speakers"));
        assert_eq!(
            model.active_device().map(|device| device.name.as_str()),
            Some("Kitchen")
        );
        assert!(!model.allowed.transfer);
    }

    #[test]
    fn treats_an_orphaned_state_as_paused() {
        let mut cluster = cluster();
        cluster.active_device_id.clear();
        let model = playback(&cluster, OWN);
        assert_eq!(model.active_device_id, None);
        assert!(!model.playing);
        assert_eq!(model.position_ms, 30_000);
        assert!(model.allowed.resume && !model.allowed.pause);
        assert!(model.track.is_some());

        let empty = Cluster {
            device: cluster.device.clone(),
            ..Default::default()
        };
        let model = playback(&empty, OWN);
        assert_eq!(model.allowed, Allowed::default());
        assert_eq!(model.queue, Queue::default());
        assert!(model.track.is_none());
        assert_eq!(model.devices.len(), 6);
    }

    #[test]
    fn names_stations_and_sleep_timers() {
        let mut cluster = cluster();
        let state = cluster.player_state.as_mut().unwrap();
        state.context_metadata = metadata(&[
            ("station_title", "Song Radio"),
            ("station_subtitle", "With Singer"),
        ]);
        state.sleep_timer = Some(SleepTimer {
            timer_type: Some(TimerType::Timestamp(sleep_timer::Timestamp {
                timestamp: 1_800_000_000_000,
            })),
        });
        state.options.as_mut().unwrap().playback_speed = None;
        state.playback_speed = 1.5;
        let model = playback(&cluster, OWN);
        assert_eq!(
            model.context_title.as_deref(),
            Some("Song Radio • With Singer")
        );
        assert_eq!(
            model.sleep_timer,
            SleepState::At(UNIX_EPOCH + Duration::from_secs(1_800_000_000))
        );
        assert_eq!(model.playback_speed, 1.5);
        let state = cluster.player_state.as_mut().unwrap();
        state.sleep_timer = Some(SleepTimer {
            timer_type: Some(TimerType::EndOfTrack(sleep_timer::EndOfTrack {})),
        });
        state.context_metadata.clear();
        let model = playback(&cluster, OWN);
        assert_eq!(model.sleep_timer, SleepState::EndOfTrack);
        assert_eq!(model.context_title, None);
    }

    #[test]
    fn wraps_commands_in_the_envelope() {
        let id = command_id();
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(
            envelope(json!({"endpoint": "pause"}), "abc", None),
            json!({"command": {"endpoint": "pause", "logging_params": {"command_id": "abc"}}})
        );
        assert_eq!(
            envelope(
                json!({"endpoint": "skip_next", "logging_params": {"command_id": "keep", "page_instance_ids": []}}),
                "abc",
                Some(3)
            ),
            json!({
                "command": {"endpoint": "skip_next", "logging_params": {"command_id": "keep", "page_instance_ids": []}},
                "target_alias_id": 3,
            })
        );
        assert_eq!(
            transfer_body(Restore::Keep, "abc", None),
            json!({"transfer_options": {"restore_paused": "restore"}, "command_id": "abc"})
        );
        assert_eq!(
            transfer_body(Restore::Resume, "abc", Some(3)),
            json!({"transfer_options": {"restore_paused": "resume"}, "command_id": "abc", "target_alias_id": 3})
        );
        assert_eq!(
            transfer_body(Restore::Pause, "abc", None)["transfer_options"]["restore_paused"],
            "pause"
        );
        assert_eq!(volume_body(500, None), json!({"volume": 500}));
        assert_eq!(
            volume_body(500, Some(3)),
            json!({"volume": 500, "command_options": {"target_alias_id": 3}})
        );
    }

    #[test]
    fn reads_command_replies() {
        assert_eq!(
            command_reply(StatusCode::OK, br#"{"ack_id":"ack-1"}"#).unwrap(),
            Some("ack-1".to_owned())
        );
        assert_eq!(command_reply(StatusCode::ACCEPTED, b"").unwrap(), None);
        assert_eq!(command_reply(StatusCode::OK, b"{}").unwrap(), None);
        let refused = command_reply(StatusCode::FORBIDDEN, br#"{"reason":"x"}"#).unwrap_err();
        assert_eq!(
            refused.downcast_ref::<CommandError>(),
            Some(&CommandError::Refused(r#"{"reason":"x"}"#.into()))
        );
        let gone = command_reply(StatusCode::NOT_FOUND, b"").unwrap_err();
        assert_eq!(
            gone.downcast_ref::<CommandError>(),
            Some(&CommandError::DeviceGone)
        );
        let failed = command_reply(StatusCode::BAD_GATEWAY, b"").unwrap_err();
        assert_eq!(
            failed.downcast_ref::<HttpStatus>().map(|status| status.0),
            Some(StatusCode::BAD_GATEWAY)
        );
    }

    #[test]
    fn builds_play_context_commands() {
        assert_eq!(
            play_context_command(
                "spotify:album:1",
                2,
                Some("spotify:track:9"),
                ORIGIN,
                &PlayOptions::default()
            ),
            json!({
                "endpoint": "play",
                "context": {"uri": "spotify:album:1", "url": "context://spotify:album:1", "metadata": {}},
                "play_origin": {"feature_identifier": "openmusic"},
                "options": {"skip_to": {"track_index": 2, "track_uri": "spotify:track:9"}},
            })
        );
        let options = PlayOptions {
            description: Some("Mix".into()),
            shuffle_override: Some(Shuffle::On),
        };
        assert_eq!(
            play_context_command("spotify:playlist:1", 0, None, "home", &options),
            json!({
                "endpoint": "play",
                "context": {
                    "uri": "spotify:playlist:1",
                    "url": "context://spotify:playlist:1",
                    "metadata": {"context_description": "Mix"},
                },
                "play_origin": {"feature_identifier": "home"},
                "options": {
                    "skip_to": {"track_index": 0},
                    "player_options_override": {"shuffling_context": true, "modes": {"context_enhancement": "NONE"}},
                },
            })
        );
        assert_eq!(
            player_options(Shuffle::Smart),
            json!({"shuffling_context": true, "modes": {"context_enhancement": "RECOMMENDATION"}})
        );
        assert_eq!(
            player_options(Shuffle::Off),
            json!({"shuffling_context": false, "modes": {"context_enhancement": "NONE"}})
        );
    }

    #[test]
    fn builds_shuffle_and_repeat_commands() {
        assert_eq!(
            shuffle_command(Shuffle::On, true),
            json!({"endpoint": "set_options", "shuffling_context": true, "modes": {"context_enhancement": "NONE"}})
        );
        assert_eq!(
            shuffle_command(Shuffle::Smart, true),
            json!({"endpoint": "set_options", "shuffling_context": true, "modes": {"context_enhancement": "RECOMMENDATION"}})
        );
        assert_eq!(
            shuffle_command(Shuffle::Off, true),
            json!({"endpoint": "set_options", "shuffling_context": false, "modes": {"context_enhancement": "NONE"}})
        );
        assert_eq!(
            shuffle_command(Shuffle::On, false),
            json!({"endpoint": "set_shuffling_context", "value": true})
        );
        assert_eq!(
            shuffle_command(Shuffle::Off, false),
            json!({"endpoint": "set_shuffling_context", "value": false})
        );

        let all = Allowed {
            repeat_context: true,
            repeat_track: true,
            ..Allowed::default()
        };
        assert_eq!(
            repeat_commands(Repeat::Off, &all, true),
            vec![
                json!({"endpoint": "set_options", "repeating_context": false, "repeating_track": false})
            ]
        );
        assert_eq!(
            repeat_commands(Repeat::Context, &all, true),
            vec![
                json!({"endpoint": "set_options", "repeating_context": true, "repeating_track": false})
            ]
        );
        assert_eq!(
            repeat_commands(Repeat::Track, &all, true),
            vec![
                json!({"endpoint": "set_options", "repeating_context": true, "repeating_track": true})
            ]
        );
        let track_only = Allowed {
            repeat_track: true,
            ..Allowed::default()
        };
        assert_eq!(
            repeat_commands(Repeat::Track, &track_only, true),
            vec![json!({"endpoint": "set_options", "repeating_track": true})]
        );
        assert!(repeat_commands(Repeat::Track, &Allowed::default(), true).is_empty());
        assert_eq!(
            repeat_commands(Repeat::Track, &all, false),
            vec![
                json!({"endpoint": "set_repeating_context", "value": true}),
                json!({"endpoint": "set_repeating_track", "value": true}),
            ]
        );
        assert_eq!(
            repeat_commands(Repeat::Off, &track_only, false),
            vec![json!({"endpoint": "set_repeating_track", "value": false})]
        );
    }

    #[test]
    fn builds_queue_commands() {
        let next = vec![
            item(
                "spotify:track:1111111111111111111111",
                "q1",
                "queue",
                &[("is_queued", "true")],
            ),
            ProvidedTrack {
                removed: vec!["x".into()],
                ..item(
                    "spotify:delimiter",
                    "delimiter0",
                    "context",
                    &[("hidden", "true")],
                )
            },
        ];
        let prev = vec![item(
            "spotify:track:9999999999999999999999",
            "p1",
            "context",
            &[],
        )];
        assert_eq!(
            set_queue_command(&next, &prev, "42"),
            json!({
                "endpoint": "set_queue",
                "next_tracks": [
                    {"uri": "spotify:track:1111111111111111111111", "uid": "q1", "metadata": {"is_queued": "true"}, "provider": "queue"},
                    {"uri": "spotify:delimiter", "uid": "delimiter0", "metadata": {"hidden": "true"}, "provider": "context", "removed": ["x"]},
                ],
                "prev_tracks": [
                    {"uri": "spotify:track:9999999999999999999999", "uid": "p1", "metadata": {}, "provider": "context"},
                ],
                "queue_revision": "42",
            })
        );
        assert_eq!(
            skip_to_command(&next[0]),
            json!({
                "endpoint": "skip_next",
                "track": {"uri": "spotify:track:1111111111111111111111", "uid": "q1", "metadata": {"is_queued": "true"}, "provider": "queue"},
            })
        );
        assert_eq!(
            add_to_queue_command("spotify:track:1"),
            json!({"endpoint": "add_to_queue", "track": {"uri": "spotify:track:1", "metadata": {"is_queued": "true"}, "provider": "queue"}})
        );
        assert_eq!(
            play_next_command(&["spotify:track:1".into(), "spotify:track:2".into()]),
            json!({"endpoint": "play_as_next_in_queue", "tracks": [
                {"uri": "spotify:track:1", "uid": "", "metadata": {}},
                {"uri": "spotify:track:2", "uid": "", "metadata": {}},
            ]})
        );
        assert_eq!(
            update_context_command("spotify:playlist:1", "session"),
            json!({"endpoint": "update_context", "context": {"url": "context://spotify:playlist:1"}, "session_id": "session"})
        );
        assert_eq!(
            shuffle_play_command(
                "spotify:playlist:1",
                ORIGIN,
                &PlayOptions {
                    description: Some("Mix".into()),
                    shuffle_override: None,
                }
            ),
            json!({
                "endpoint": "play",
                "context": {"uri": "spotify:playlist:1", "url": "context://spotify:playlist:1", "metadata": {"context_description": "Mix"}},
                "play_origin": {"feature_identifier": "openmusic"},
                "options": {"player_options_override": {"shuffling_context": true, "modes": {"context_enhancement": "NONE"}}},
            })
        );
        assert_eq!(
            shuffle_play_command(
                "spotify:playlist:1",
                ORIGIN,
                &PlayOptions {
                    description: None,
                    shuffle_override: Some(Shuffle::Smart),
                }
            )["options"]["player_options_override"]["modes"]["context_enhancement"],
            "RECOMMENDATION"
        );
    }

    fn uids(list: &[ProvidedTrack]) -> Vec<String> {
        list.iter()
            .map(|track| {
                if track.uid.is_empty() {
                    track.uri.clone()
                } else {
                    track.uid.clone()
                }
            })
            .collect()
    }

    fn raw_next() -> Vec<ProvidedTrack> {
        cluster().player_state.unwrap().next_tracks
    }

    fn key(uid: &str, iteration: &str, uri: &str) -> QueueKey {
        QueueKey {
            uid: uid.into(),
            iteration: iteration.into(),
            uri: uri.into(),
        }
    }

    #[test]
    fn removes_and_clears_queue_items() {
        let next = raw_next();
        let without = queue_without(
            &next,
            &[
                key("q2", "", "spotify:track:2222222222222222222222"),
                key("c4", "0", "spotify:track:8888888888888888888888"),
            ],
        );
        assert_eq!(
            uids(&without),
            vec![
                "q1",
                "spotify:meta:page:1",
                "c1",
                "c1",
                "c2",
                "c3",
                "q3",
                "delimiter0",
                "c4"
            ]
        );
        let by_uri = queue_without(
            &next,
            &[key("", "", "spotify:track:7777777777777777777777")],
        );
        assert!(by_uri.iter().all(|track| track.uid != "q3"));
        assert_eq!(by_uri.len(), next.len() - 1);
        let removed = without
            .iter()
            .find(|track| track.uri == "spotify:meta:page:1")
            .unwrap();
        assert_eq!(removed.metadata["hidden"], "true");

        let cleared = queue_cleared(&next);
        assert_eq!(
            uids(&cleared),
            vec![
                "spotify:meta:page:1",
                "c1",
                "c1",
                "c2",
                "c3",
                "delimiter0",
                "c4"
            ]
        );
    }

    #[test]
    fn moves_queue_items() {
        let next = raw_next();
        let q1 = key("q1", "", "spotify:track:1111111111111111111111");
        let q2 = key("q2", "", "spotify:track:2222222222222222222222");
        let c3 = key("c3", "", "spotify:track:6666666666666666666666");
        let swapped = queue_moved(&next, std::slice::from_ref(&q2), &q1, false);
        assert_eq!(&uids(&swapped)[..3], &["q2", "q1", "spotify:meta:page:1"]);
        assert_eq!(swapped.len(), next.len());

        let mut with_context = next.clone();
        with_context[3].uid = "c0".into();
        let c0 = key("c0", "0", "spotify:track:3333333333333333333333");
        let queued = queue_moved(&with_context, std::slice::from_ref(&c0), &q1, true);
        let moved = queued.iter().find(|track| track.uid == "c0").unwrap();
        assert_eq!(uids(&queued)[..2], ["q1", "c0"]);
        assert_eq!(moved.provider, "queue");
        assert_eq!(moved.metadata["is_queued"], "true");
        assert_eq!(moved.metadata["iteration"], "0");

        let after_context = queue_moved(&next, std::slice::from_ref(&q1), &c3, true);
        let position = after_context
            .iter()
            .position(|track| track.uid == "q1")
            .unwrap();
        assert_eq!(after_context[position - 1].uid, "c3");
        assert_eq!(after_context[position].provider, "queue");

        assert_eq!(
            queue_moved(&next, std::slice::from_ref(&q1), &q1, true),
            next
        );
        let missing = key("nope", "", "spotify:track:0000000000000000000000");
        assert_eq!(
            queue_moved(&next, std::slice::from_ref(&q1), &missing, true),
            next
        );
        assert_eq!(
            queue_moved(&next, std::slice::from_ref(&missing), &q1, true),
            next
        );
    }

    #[test]
    fn adds_items_after_the_queued_ones() {
        let next = raw_next();
        let added = queue_with_added(&next, &["spotify:track:aaaaaaaaaaaaaaaaaaaaaa".into()]);
        assert_eq!(added.len(), next.len() + 1);
        assert_eq!(added[3].uri, "spotify:track:aaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(added[3].provider, "queue");
        assert_eq!(added[3].metadata["is_queued"], "true");
        assert_eq!(added[2].uid, "q2");
        assert_eq!(added[4].uid, "c1");

        let context_first = vec![
            item(
                "spotify:media:narration",
                "n1",
                "context",
                &[("is_narration", "true")],
            ),
            item("spotify:track:3333333333333333333333", "c1", "context", &[]),
        ];
        let added = queue_with_added(
            &context_first,
            &["spotify:track:a".into(), "spotify:track:b".into()],
        );
        assert_eq!(
            uids(&added),
            vec!["n1", "spotify:track:a", "spotify:track:b", "c1"]
        );
        let empty = queue_with_added(&[], &["spotify:track:a".into()]);
        assert_eq!(uids(&empty), vec!["spotify:track:a"]);
    }

    #[test]
    fn reads_smart_shuffle_and_context_options() {
        assert_eq!(
            smart_shuffle_uri("spotify:collection:tracks"),
            LIKED_SONGS_LENS
        );
        assert_eq!(
            smart_shuffle_uri("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"),
            "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"
        );
        assert!(parse_smart_shuffle(&json!({"lookup": [{
            "__typename": "PlaylistResponseWrapper",
            "_uri": "spotify:playlist:37i9dQZF1F5p3rmiWPIYgZ",
            "data": {"__typename": "Playlist", "smartShuffle": {"available": true, "uri": ""}},
        }]})));
        assert!(!parse_smart_shuffle(&json!({"lookup": [{
            "__typename": "PlaylistResponseWrapper",
            "_uri": "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
            "data": {"__typename": "Playlist", "smartShuffle": {"available": false, "uri": ""}},
        }]})));
        assert!(!parse_smart_shuffle(
            &json!({"lookup": [{"__typename": "AlbumResponseWrapper"}]})
        ));
        assert!(!parse_smart_shuffle(&json!({})));

        let variables = context_options_variables("spotify:playlist:1", OWN);
        assert_eq!(variables["uri"], "spotify:playlist:1");
        assert_eq!(variables["deviceInfo"]["deviceId"], OWN);
        assert_eq!(variables["deviceInfo"]["deviceType"], "computer");
        assert_eq!(
            variables["deviceInfo"]["clientId"],
            crate::oauth::DESKTOP_CLIENT_ID
        );
        assert_eq!(variables["deviceInfo"]["productId"], "1");

        let data = json!({"playerOptions": {
            "__typename": "PlayerOptions",
            "modes": [
                {"modeKey": "context_enhancement", "modeValue": "NONE"},
                {"modeKey": "media", "modeValue": "audio"},
                {"modeKey": "jam", "modeValue": "off"},
            ],
            "restrictions": {
                "disallowSettingModes": [
                    {"modeKey": "context_enhancement", "modeRestrictions": [
                        {"modeValue": "NONE", "reasons": ["already_set"]},
                        {"modeValue": "RECOMMENDATION", "reasons": []},
                    ]},
                    {"modeKey": "media", "modeRestrictions": [
                        {"modeValue": "audio", "reasons": ["already_set"]},
                        {"modeValue": "video", "reasons": ["not_playing_track"]},
                    ]},
                ],
                "disallowTogglingShuffleReasons": [],
            },
            "shufflingContext": false,
        }});
        assert_eq!(
            parse_context_options(&data),
            Some(ContextOptions {
                shuffle: Shuffle::Off,
                shuffle_allowed: true,
                smart_allowed: true,
            })
        );
        let mut smart = data.clone();
        smart["playerOptions"]["shufflingContext"] = json!(true);
        smart["playerOptions"]["modes"][0]["modeValue"] = json!("RECOMMENDATION");
        smart["playerOptions"]["restrictions"]["disallowSettingModes"][0]["modeRestrictions"][1]
            ["reasons"] = json!(["already_set"]);
        assert_eq!(
            parse_context_options(&smart),
            Some(ContextOptions {
                shuffle: Shuffle::Smart,
                shuffle_allowed: true,
                smart_allowed: true,
            })
        );
        let mut blocked = data.clone();
        blocked["playerOptions"]["shufflingContext"] = json!(true);
        blocked["playerOptions"]["restrictions"]["disallowSettingModes"][0]["modeRestrictions"]
            [1]["reasons"] = json!(["not_supported_by_content_type"]);
        assert_eq!(
            parse_context_options(&blocked),
            Some(ContextOptions {
                shuffle: Shuffle::On,
                shuffle_allowed: true,
                smart_allowed: false,
            })
        );
        blocked["playerOptions"]["restrictions"]["disallowTogglingShuffleReasons"] = json!(["dj"]);
        assert!(!parse_context_options(&blocked).unwrap().shuffle_allowed);
        assert_eq!(parse_context_options(&json!({"playerOptions": null})), None);
    }

    #[test]
    fn splits_alias_targets() {
        assert_eq!(device_target("abc::a_12"), ("abc", Some(12)));
        assert_eq!(device_target("abc"), ("abc", None));
        assert_eq!(device_target("abc::a_x"), ("abc::a_x", None));
    }
}
