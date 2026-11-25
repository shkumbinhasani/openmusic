use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    path::Path,
    time::{Duration, Instant},
};

use gpui::{App, Context, ParentElement, Styled, Window, px};
use gpui_component::{ActiveTheme, WindowExt, button::ButtonVariant, dialog::DialogButtonProps};
use serde::{Deserialize, Serialize};
use spotify_client::{
    Track,
    connect::{
        self, Allowed, ConnectDevice, ContextOptions, DeviceKind, PlayOptions, Playback, QueueItem,
        QueueKey, Repeat, Shuffle,
    },
    dj::{self, Dj},
    engine::NowPlaying,
    entity,
    jam::{JoinMode, VolumeControl},
};

use super::{
    jam::{self as jams, GUEST_WARNING, HOST_VOLUME, JamState, LEAVE_FIRST},
    worker::{Command, Event, PlaybackView, RECENT_LIMIT, lookup_uri},
};
use crate::{
    app::{OpenMusic, Page, intent::Intent},
    menus::{
        self, CycleRepeat, Handle, PlayOnThisMac, ShowDevices, ShowQueue, StartJam, ToggleShuffle,
    },
    ui::{
        menu::{self, Place, Row, Target},
        tracks::{RowDrag, RowDrop},
    },
};

pub const OPTIMISTIC: Duration = Duration::from_secs(3);
const REFRESH_SPACING: Duration = Duration::from_secs(2);
pub const TRANSFER_TIMEOUT: Duration = Duration::from_secs(10);
const QUEUE_TOAST: Duration = Duration::from_secs(4);
pub const ADDED_TO_QUEUE: &str = "Added to queue";
pub const CONNECTING: &str = "Connecting…";
pub const DEVICES_BUTTON: &str = "devices";
pub const DEVICE_PANEL: &str = "device-panel";
const LIKED_SONGS: &str = "spotify:collection:tracks";
const LOCAL_VOLUME_STEPS: u32 = 64;

pub const QUEUE_CURRENT: &str = "queue-current";
pub const QUEUE_QUEUED: &str = "queue-queued";
pub const QUEUE_NEXT: &str = "queue-next";
pub const QUEUE_AUTOPLAY: &str = "queue-autoplay";
pub const QUEUE_RECENT: &str = "queue-recent";
const MOVABLE: [&str; 2] = [QUEUE_QUEUED, QUEUE_NEXT];
pub const DJ_JUMP: &str = "dj-jump";
pub const DJ_UNAVAILABLE: &str = "DJ isn’t available right now.";
pub const DJ_VOICE: &str = "In beta, the DJ’s voice is unavailable on this speaker";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackPrefs {}

#[derive(Clone, Debug, PartialEq)]
pub struct QueueRow {
    pub key: QueueKey,
    pub provider: String,
    pub track: Track,
    pub queued_by: Option<String>,
}

#[derive(Default)]
pub struct DjState {
    pub available: Option<Dj>,
    pub checked: bool,
    voice_noted: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerAction {
    StartJam,
    OpenJam,
    CopyJamLink,
    LeaveJam,
    EndJam,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueView {
    pub current: Option<QueueRow>,
    pub queued: Vec<QueueRow>,
    pub next: Vec<QueueRow>,
    pub autoplay: Vec<QueueRow>,
    pub recent: Vec<QueueRow>,
}

impl QueueView {
    pub fn section(&self, prefix: &str) -> &[QueueRow] {
        match prefix {
            QUEUE_CURRENT => self.current.as_slice(),
            QUEUE_QUEUED => &self.queued,
            QUEUE_NEXT => &self.next,
            QUEUE_AUTOPLAY => &self.autoplay,
            QUEUE_RECENT => &self.recent,
            _ => &[],
        }
    }

    pub fn row(&self, row: Row) -> Option<&QueueRow> {
        self.section(row.0).get(row.1)
    }

    pub fn upcoming(&self) -> usize {
        self.queued.len() + self.next.len() + self.autoplay.len()
    }
}

struct PendingQueue {
    revision: String,
    hidden: Vec<QueueKey>,
    cleared: bool,
    since: Instant,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ContextShuffle {
    pub server: Option<ContextOptions>,
    pub smart: Option<bool>,
    pub chosen: Option<Shuffle>,
    requested: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingTransfer {
    pub id: String,
    pub name: String,
    since: Instant,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PickerRow {
    Header(&'static str),
    Device {
        id: String,
        title: String,
        symbol: &'static str,
        current: bool,
        enabled: bool,
        subtitle: Option<String>,
    },
    Volume(f32),
    Note(&'static str),
    Separator,
    Action {
        action: PickerAction,
        title: String,
        subtitle: Option<String>,
        symbol: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueSource {
    Items(Vec<String>),
    Context(String),
}

#[derive(Default)]
pub struct State {
    pub now: NowPlaying,
    pub notice: Option<String>,
    pub seek_hold: Option<(u32, Instant)>,
    pub model: Option<Box<Playback>>,
    pub tracks: HashMap<String, Track>,
    pub queue: QueueView,
    pub contexts: HashMap<String, ContextShuffle>,
    pub refreshes: u32,
    pub transfer: Option<PendingTransfer>,
    pub dock_devices: Cell<bool>,
    pub jam: JamState,
    pub dj: DjState,
    missing: HashSet<String>,
    requested: HashSet<String>,
    pending_shuffle: Option<(Shuffle, Instant)>,
    pending_repeat: Option<(Repeat, Instant)>,
    pending_queue: Option<PendingQueue>,
    last_refresh: Option<Instant>,
    queue_toast: Option<Instant>,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        urls.extend(
            self.now
                .track
                .iter()
                .filter_map(|track| track.cover.clone()),
        );
        let queue = &self.queue;
        urls.extend(
            queue
                .current
                .iter()
                .chain(&queue.queued)
                .chain(&queue.next)
                .chain(&queue.autoplay)
                .chain(&queue.recent)
                .filter_map(|row| row.track.cover.clone()),
        );
        self.jam.covers(urls);
        urls.extend(self.dj.available.iter().filter_map(|dj| dj.cover.clone()));
    }
}

pub fn next_shuffle(current: Shuffle, allowed: bool, smart: bool) -> Shuffle {
    if !allowed {
        return current;
    }
    match current {
        Shuffle::Off => Shuffle::On,
        Shuffle::On if smart => Shuffle::Smart,
        Shuffle::On | Shuffle::Smart => Shuffle::Off,
    }
}

pub fn next_repeat(current: Repeat, allowed: &Allowed) -> Repeat {
    match current {
        Repeat::Off if allowed.repeat_context => Repeat::Context,
        Repeat::Off if allowed.repeat_track => Repeat::Track,
        Repeat::Off => Repeat::Off,
        Repeat::Context if allowed.repeat_track => Repeat::Track,
        Repeat::Context | Repeat::Track => Repeat::Off,
    }
}

fn metadata_track(item: &QueueItem, title: &str) -> Track {
    let field = |key: &str| {
        item.metadata
            .get(key)
            .filter(|value| !value.is_empty())
            .cloned()
    };
    let album = field("album_title").unwrap_or_default();
    Track {
        uri: item.uri.clone(),
        title: item.title.clone().unwrap_or_else(|| title.to_owned()),
        artist: item.artist.clone().unwrap_or_else(|| album.clone()),
        album,
        duration_ms: field("duration")
            .and_then(|duration| duration.parse().ok())
            .unwrap_or(0),
        cover: item.cover.clone(),
        artist_uri: field("artist_uri"),
        album_uri: field("album_uri"),
        explicit: field("is_explicit").as_deref() == Some("true"),
        ..Track::default()
    }
}

fn uid(item: &QueueItem) -> Option<String> {
    (!item.uid.is_empty()).then(|| item.uid.clone())
}

fn queued_by(item: &QueueItem) -> Option<String> {
    item.metadata
        .get("queued_by")
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn hidden(key: &QueueKey, pending: Option<&PendingQueue>) -> bool {
    pending.is_some_and(|pending| pending.hidden.contains(key))
}

pub fn device_label(device: &ConnectDevice) -> String {
    if device.this_mac {
        "This Mac".to_owned()
    } else {
        device.name.clone()
    }
}

fn picker_device(device: &ConnectDevice, current: bool, pending: Option<&str>) -> PickerRow {
    let subtitle = if pending == Some(device.id.as_str()) {
        Some(CONNECTING.to_owned())
    } else if current {
        None
    } else {
        device.blocked.clone()
    };
    PickerRow::Device {
        id: device.id.clone(),
        title: device_label(device),
        symbol: device.kind.symbol(),
        current,
        enabled: current || device.blocked.is_none(),
        subtitle,
    }
}

pub fn picker_rows(model: Option<&Playback>, pending: Option<&str>, volume: u16) -> Vec<PickerRow> {
    let devices = model.map_or(&[][..], |model| model.devices.as_slice());
    let current = devices
        .iter()
        .find(|device| device.active)
        .or_else(|| devices.iter().find(|device| device.this_mac))
        .cloned()
        .unwrap_or_else(|| ConnectDevice {
            id: model
                .map(|model| model.own_device_id.clone())
                .unwrap_or_default(),
            name: "This Mac".to_owned(),
            kind: DeviceKind::Computer,
            this_mac: true,
            volume: Some(volume),
            ..ConnectDevice::default()
        });
    let mut rows = vec![
        PickerRow::Header("Current device"),
        picker_device(&current, true, pending),
    ];
    if current.this_mac || current.volume.is_some() {
        rows.push(PickerRow::Volume(f32::from(volume) / f32::from(u16::MAX)));
    }
    rows.push(PickerRow::Header("Select another device"));
    let others: Vec<PickerRow> = devices
        .iter()
        .filter(|device| device.id != current.id)
        .map(|device| picker_device(device, false, pending))
        .collect();
    if others.is_empty() {
        rows.push(PickerRow::Note("No other devices found"));
    }
    rows.extend(others);
    rows
}

fn transferred(model: &Playback, id: &str) -> bool {
    model.active_device().is_some_and(|device| device.id == id)
        || model.active_device_id.as_deref() == Some(connect::device_target(id).0)
}

pub fn queue_source(target: &Target) -> Option<QueueSource> {
    match target {
        Target::Track(track) if lookup_uri(&track.track.uri) => {
            Some(QueueSource::Items(vec![track.track.uri.clone()]))
        }
        Target::Episode { uri } if lookup_uri(uri) => Some(QueueSource::Items(vec![uri.clone()])),
        Target::Album { uri } => Some(QueueSource::Context(uri.clone())),
        Target::Playlist { uri } if entity::is_liked_songs(uri) => {
            Some(QueueSource::Context(LIKED_SONGS.to_owned()))
        }
        Target::Playlist { uri } => Some(QueueSource::Context(uri.clone())),
        _ => None,
    }
}

impl OpenMusic {
    pub fn track(&self) -> Option<&Track> {
        self.playback.now.track.as_ref()
    }

    pub fn duration(&self) -> Option<u32> {
        self.track()
            .map(|track| track.duration_ms)
            .filter(|duration| *duration > 0)
    }

    pub fn is_current(&self, uri: &str) -> bool {
        self.track().is_some_and(|track| track.uri == *uri)
    }

    pub fn playback_model(&self) -> Option<&Playback> {
        self.playback.model.as_deref()
    }

    pub fn playback_allowed(&self) -> Allowed {
        self.playback_model()
            .filter(|model| model.track.is_some())
            .map(|model| model.allowed)
            .unwrap_or_default()
    }

    pub fn in_jam(&self) -> bool {
        self.jam().is_some()
            || self
                .playback_model()
                .and_then(|model| model.jam_mode.as_deref())
                .is_some_and(|mode| !mode.is_empty() && mode != "off")
    }

    pub fn dj_context(&self) -> bool {
        self.playback_model().is_some_and(dj::in_dj)
    }

    pub fn dj_narrating(&self) -> bool {
        self.playback_model().is_some_and(dj::narrating)
    }

    pub fn dj_jump_available(&self) -> bool {
        self.playback_model().and_then(dj::jump).is_some()
    }

    pub fn dj_jump(&mut self, cx: &mut Context<Self>) {
        if !self.dj_jump_available() {
            return;
        }
        self.playback.notice = None;
        self.send(Command::DjJump);
        cx.notify();
    }

    pub fn play_dj(&mut self, cx: &mut Context<Self>) {
        if self.needs_premium_here(false, cx) {
            return;
        }
        self.playback.notice = None;
        self.send(Command::DjCheck { play: true });
        cx.notify();
    }

    pub fn smart_shuffle_allowed(&self) -> bool {
        self.playback_allowed().smart_shuffle && !self.in_jam()
    }

    pub fn repeat_allowed(&self) -> bool {
        let allowed = self.playback_allowed();
        allowed.repeat_context || allowed.repeat_track
    }

    pub fn shuffle_mode(&self) -> Shuffle {
        self.playback
            .pending_shuffle
            .map_or(self.playback.now.shuffle, |(mode, _)| mode)
    }

    pub fn repeat_mode(&self) -> Repeat {
        self.playback
            .pending_repeat
            .map_or(self.playback.now.repeat, |(mode, _)| mode)
    }

    pub fn guest_locked(&mut self) -> bool {
        if !self.jam_controls_locked() {
            return false;
        }
        self.show_toast(GUEST_WARNING, None);
        true
    }

    pub fn toggle_shuffle(&mut self, cx: &mut Context<Self>) {
        if self.guest_locked() {
            cx.notify();
            return;
        }
        let allowed = self.playback_allowed();
        if !allowed.shuffle {
            return;
        }
        let next = next_shuffle(self.shuffle_mode(), true, self.smart_shuffle_allowed());
        self.set_shuffle(next, cx);
    }

    pub fn cycle_repeat(&mut self, cx: &mut Context<Self>) {
        if self.guest_locked() {
            cx.notify();
            return;
        }
        let next = next_repeat(self.repeat_mode(), &self.playback_allowed());
        if next != self.repeat_mode() {
            self.set_repeat(next, cx);
        }
    }

    pub fn set_shuffle(&mut self, mode: Shuffle, cx: &mut Context<Self>) {
        self.playback.pending_shuffle = Some((mode, Instant::now()));
        self.playback.notice = None;
        self.send(Command::Shuffle(mode));
        cx.notify();
    }

    pub fn set_repeat(&mut self, mode: Repeat, cx: &mut Context<Self>) {
        self.playback.pending_repeat = Some((mode, Instant::now()));
        self.playback.notice = None;
        self.send(Command::Repeat(mode));
        cx.notify();
    }

    pub fn playing_context(&self, uri: &str) -> bool {
        self.playback_model()
            .and_then(|model| model.context_uri.as_deref())
            .or(self.playback.now.context_uri.as_deref())
            .is_some_and(|context| context == uri)
    }

    pub fn request_context_options(&mut self, uri: &str) {
        let entry = self.playback.contexts.entry(uri.to_owned()).or_default();
        if entry.requested {
            return;
        }
        entry.requested = true;
        self.send(Command::SmartShuffleEligibility(uri.to_owned()));
        self.send(Command::ContextPlayerOptions(uri.to_owned()));
    }

    pub fn context_shuffle(&self, uri: &str) -> Shuffle {
        if self.playing_context(uri) {
            return self.shuffle_mode();
        }
        self.playback
            .contexts
            .get(uri)
            .and_then(|state| {
                state
                    .chosen
                    .or_else(|| state.server.map(|options| options.shuffle))
            })
            .unwrap_or_default()
    }

    pub fn context_smart_available(&self, uri: &str) -> bool {
        if self.playing_context(uri) {
            return self.smart_shuffle_allowed();
        }
        self.playback.contexts.get(uri).is_some_and(|state| {
            state.smart == Some(true) && state.server.is_none_or(|options| options.smart_allowed)
        })
    }

    pub fn toggle_context_shuffle(&mut self, uri: &str, cx: &mut Context<Self>) {
        if self.playing_context(uri) {
            self.toggle_shuffle(cx);
            return;
        }
        let allowed = self
            .playback
            .contexts
            .get(uri)
            .and_then(|state| state.server)
            .is_none_or(|options| options.shuffle_allowed);
        let next = next_shuffle(
            self.context_shuffle(uri),
            allowed,
            self.context_smart_available(uri),
        );
        self.playback
            .contexts
            .entry(uri.to_owned())
            .or_default()
            .chosen = Some(next);
        cx.notify();
    }

    pub fn context_play_options(&self, uri: &str) -> PlayOptions {
        PlayOptions {
            shuffle_override: self
                .playback
                .contexts
                .get(uri)
                .and_then(|state| state.chosen),
            ..PlayOptions::default()
        }
    }

    pub fn shuffle_play(&mut self, uri: &str, cx: &mut Context<Self>) {
        if self.needs_premium_here(false, cx) {
            return;
        }
        self.playback.notice = None;
        self.send(Command::ShufflePlay(uri.to_owned()));
        cx.notify();
    }

    pub fn play(&mut self, tracks: &[Track], index: usize, cx: &mut Context<Self>) {
        let Some(track) = tracks.get(index) else {
            return;
        };
        if self.needs_premium_here(false, cx) {
            return;
        }
        self.playback.now.track = Some(track.clone());
        self.playback.now.playing = true;
        self.playback.now.position_ms = 0;
        self.playback.now.observed = Instant::now();
        self.playback.notice = None;
        self.send(Command::PlayTracks {
            tracks: tracks.iter().map(|track| track.uri.clone()).collect(),
            index,
        });
        cx.notify();
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.track().is_none() {
            return;
        }
        self.set_playing(!self.playback.now.playing, cx);
    }

    pub fn set_playing(&mut self, playing: bool, cx: &mut Context<Self>) {
        if self.track().is_some() && self.playback.now.playing == playing {
            return;
        }
        if playing && self.needs_premium_here(true, cx) {
            return;
        }
        self.playback.now.position_ms = self.playback.now.position();
        self.playback.now.observed = Instant::now();
        self.playback.now.playing = playing;
        self.playback.notice = None;
        self.send(Command::SetPlaying(playing));
        cx.notify();
    }

    pub fn seek_to(&mut self, position: u32, cx: &mut Context<Self>) {
        self.playback.now.position_ms = position;
        self.playback.now.observed = Instant::now();
        self.playback.seek_hold = Some((position, Instant::now()));
        self.send(Command::Seek(position));
        self.follow_lyrics();
        cx.notify();
    }

    pub fn play_context(
        &mut self,
        uri: &str,
        tracks: &[Track],
        index: usize,
        cx: &mut Context<Self>,
    ) {
        self.play_context_from(
            uri,
            tracks,
            index,
            connect::ORIGIN,
            PlayOptions::default(),
            cx,
        );
    }

    pub fn play_context_from(
        &mut self,
        uri: &str,
        tracks: &[Track],
        index: usize,
        origin: &'static str,
        options: PlayOptions,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.streamable_start(tracks, index, cx) else {
            return;
        };
        if self.needs_premium_here(false, cx) {
            return;
        }
        if let Some(track) = tracks.get(index) {
            self.playback.now.track = Some(track.clone());
            self.playback.now.playing = true;
            self.playback.now.position_ms = 0;
            self.playback.now.observed = Instant::now();
        }
        self.playback.notice = None;
        self.send(Command::PlayContext {
            uri: uri.to_owned(),
            index,
            track_uri: tracks.get(index).map(|track| track.uri.clone()),
            origin,
            options,
        });
        cx.notify();
    }

    pub fn set_now_playing(&mut self, now: NowPlaying) {
        let expected = self.playback.now.position();
        let held = self.playback.seek_hold.filter(|(_, at)| {
            at.elapsed() < Duration::from_secs(3)
                && now.track.as_ref().map(|track| &track.uri)
                    == self.playback.now.track.as_ref().map(|track| &track.uri)
                && now.position().abs_diff(expected) > 1500
        });
        let (position, observed) = (self.playback.now.position_ms, self.playback.now.observed);
        self.playback.now = now;
        if held.is_some() {
            self.playback.now.position_ms = position;
            self.playback.now.observed = observed;
        } else {
            self.playback.seek_hold = None;
        }
        self.refresh_queue();
    }

    fn decorate(&self, item: &QueueItem) -> Track {
        if let Some(track) = self.playback.tracks.get(&item.uri) {
            return Track {
                uid: uid(item),
                ..track.clone()
            };
        }
        let placeholder = if !lookup_uri(&item.uri) || self.playback.missing.contains(&item.uri) {
            "Unknown song"
        } else {
            "Loading…"
        };
        Track {
            uid: uid(item),
            ..metadata_track(item, placeholder)
        }
    }

    fn queue_row(&self, item: &QueueItem) -> QueueRow {
        QueueRow {
            key: item.key(),
            provider: item.provider.clone(),
            track: self.decorate(item),
            queued_by: queued_by(item),
        }
    }

    fn refresh_queue(&mut self) {
        let Some(model) = self.playback.model.as_deref() else {
            self.playback.queue = QueueView::default();
            return;
        };
        let queue = &model.queue;
        let pending = self.playback.pending_queue.as_ref();
        let visible = |item: &&QueueItem| !hidden(&item.key(), pending);
        let current = queue.current.as_ref().map(|item| {
            let mut row = self.queue_row(item);
            if let Some(track) = self
                .playback
                .now
                .track
                .as_ref()
                .filter(|track| track.uri == item.uri && !track.title.is_empty())
            {
                row.track = Track {
                    uid: uid(item),
                    ..track.clone()
                };
            }
            row
        });
        let cleared = pending.is_some_and(|pending| pending.cleared);
        let queued = queue
            .queued
            .iter()
            .filter(|_| !cleared)
            .filter(visible)
            .map(|item| self.queue_row(item))
            .collect();
        let (autoplay, next): (Vec<&QueueItem>, Vec<&QueueItem>) = queue
            .next_up
            .iter()
            .filter(|item| !(cleared && item.provider == "queue"))
            .filter(visible)
            .partition(|item| item.provider == "autoplay");
        let view = QueueView {
            current,
            queued,
            next: next.into_iter().map(|item| self.queue_row(item)).collect(),
            autoplay: autoplay
                .into_iter()
                .map(|item| self.queue_row(item))
                .collect(),
            recent: queue
                .recent(RECENT_LIMIT)
                .iter()
                .map(|item| self.queue_row(item))
                .collect(),
        };
        self.playback.queue = view;
    }

    fn request_queue_tracks(&mut self) {
        let Some(model) = self.playback.model.as_deref() else {
            return;
        };
        let uris = PlaybackView::uris(model);
        let wanted: HashSet<&str> = uris.iter().map(String::as_str).collect();
        self.playback
            .tracks
            .retain(|uri, _| wanted.contains(uri.as_str()));
        self.playback
            .requested
            .retain(|uri| wanted.contains(uri.as_str()));
        self.playback
            .missing
            .retain(|uri| wanted.contains(uri.as_str()));
        let mut ask: Vec<String> = Vec::new();
        for uri in uris {
            if lookup_uri(&uri)
                && !self.playback.tracks.contains_key(&uri)
                && !self.playback.requested.contains(&uri)
                && !ask.contains(&uri)
            {
                ask.push(uri);
            }
        }
        if ask.is_empty() {
            return;
        }
        self.playback.requested.extend(ask.iter().cloned());
        self.send(Command::LookupTracks(ask));
    }

    fn on_playback(&mut self, view: PlaybackView) {
        let PlaybackView { playback, tracks } = view;
        let previous = self
            .playback
            .model
            .as_ref()
            .map(|model| model.queue.revision.clone());
        for track in tracks {
            self.playback.tracks.insert(track.uri.clone(), track);
        }
        if self
            .playback
            .pending_shuffle
            .is_some_and(|(mode, _)| mode == playback.shuffle)
        {
            self.playback.pending_shuffle = None;
        }
        if self
            .playback
            .pending_repeat
            .is_some_and(|(mode, _)| mode == playback.repeat)
        {
            self.playback.pending_repeat = None;
        }
        if self
            .playback
            .pending_queue
            .as_ref()
            .is_some_and(|pending| pending.revision != playback.queue.revision)
        {
            self.playback.pending_queue = None;
        }
        let revised = previous
            .as_deref()
            .is_some_and(|previous| previous != playback.queue.revision);
        let starved = playback.track.is_some() && playback.queue.raw_next.is_empty();
        let spaced = self
            .playback
            .last_refresh
            .is_none_or(|at| at.elapsed() >= REFRESH_SPACING);
        if revised && starved && spaced {
            self.playback.last_refresh = Some(Instant::now());
            self.playback.refreshes += 1;
            self.send(Command::RefreshCluster);
        }
        if self
            .playback
            .transfer
            .as_ref()
            .is_some_and(|pending| transferred(&playback, &pending.id))
        {
            self.playback.transfer = None;
        }
        let jam_mode =
            |mode: Option<&str>| mode.is_some_and(|mode| !mode.is_empty() && mode != "off");
        let jam_changed = jam_mode(playback.jam_mode.as_deref())
            != jam_mode(
                self.playback
                    .model
                    .as_ref()
                    .and_then(|model| model.jam_mode.as_deref()),
            );
        self.playback.model = Some(Box::new(playback));
        self.request_queue_tracks();
        self.refresh_queue();
        self.request_people();
        if jam_changed {
            self.refresh_jam();
        }
    }

    pub fn queue_editable(&self) -> bool {
        self.playback_allowed().set_queue
            && !self.in_jam()
            && self
                .playback
                .pending_queue
                .as_ref()
                .is_none_or(|pending| pending.since.elapsed() >= OPTIMISTIC)
    }

    fn hide_in_queue(&mut self, hidden: Vec<QueueKey>, cleared: bool) {
        let Some(revision) = self
            .playback_model()
            .map(|model| model.queue.revision.clone())
        else {
            return;
        };
        self.playback.pending_queue = Some(PendingQueue {
            revision,
            hidden,
            cleared,
            since: Instant::now(),
        });
        self.playback.notice = None;
        self.refresh_queue();
    }

    pub fn remove_from_queue(&mut self, keys: Vec<QueueKey>, cx: &mut Context<Self>) {
        if keys.is_empty() || !self.queue_editable() {
            return;
        }
        self.hide_in_queue(keys.clone(), false);
        self.send(Command::RemoveFromQueue(keys));
        cx.notify();
    }

    pub fn clear_queue(&mut self, cx: &mut Context<Self>) {
        if !self.queue_editable() {
            return;
        }
        self.hide_in_queue(Vec::new(), true);
        self.send(Command::ClearQueue);
        cx.notify();
    }

    pub fn move_in_queue(
        &mut self,
        moved: Vec<QueueKey>,
        anchor: QueueKey,
        after: bool,
        cx: &mut Context<Self>,
    ) {
        if moved.is_empty() || moved.contains(&anchor) || !self.queue_editable() {
            return;
        }
        self.hide_in_queue(Vec::new(), false);
        self.send(Command::MoveInQueue {
            moved,
            anchor,
            after,
        });
        cx.notify();
    }

    pub fn jump_in_queue(&mut self, row: Row, cx: &mut Context<Self>) {
        let Some(target) = self.playback.queue.row(row).cloned() else {
            return;
        };
        if row.0 == QUEUE_RECENT {
            self.play(&[target.track], 0, cx);
            return;
        }
        if row.0 == QUEUE_CURRENT {
            self.toggle(cx);
            return;
        }
        if !self.playback_allowed().jump_in_queue {
            return;
        }
        self.playback.notice = None;
        self.send(Command::JumpTo(target.key));
        cx.notify();
    }

    pub fn confirm_clear_queue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.queue_editable() || self.playback.queue.queued.is_empty() {
            return;
        }
        let app = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let app = app.clone();
            dialog
                .title("Clear queue?")
                .child("You can’t undo this.")
                .w(px(360.))
                .bg(cx.theme().popover)
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Clear")
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text("Cancel"),
                )
                .on_ok(move |_, _, cx| {
                    let _ = app.update(cx, |app, cx| app.clear_queue(cx));
                    true
                })
        });
    }

    pub fn toggle_queue_page(&mut self) {
        if self.page == Page::Queue {
            self.back();
        } else {
            self.navigate(Page::Queue);
        }
    }

    pub fn next_from_title(&self) -> Option<String> {
        let model = self.playback_model()?;
        if let Some(title) = &model.context_title {
            return Some(title.clone());
        }
        let uri = model.context_uri.as_deref()?;
        if let Some((loaded, Ok(collection))) = &self.playlists.collection
            && loaded == uri
        {
            return Some(collection.name.clone());
        }
        if let Some(playlist) = self
            .library
            .playlists
            .iter()
            .find(|playlist| playlist.uri == uri)
        {
            return Some(playlist.name.clone());
        }
        if let Some((loaded, Ok(artist))) = &self.discovery.artist
            && loaded == uri
        {
            return Some(artist.name.clone());
        }
        entity::is_liked_songs(uri).then(|| "Liked Songs".to_owned())
    }

    pub fn active_device(&self) -> Option<&ConnectDevice> {
        self.playback_model()?.active_device()
    }

    pub fn remote_device(&self) -> Option<&ConnectDevice> {
        self.active_device().filter(|device| !device.this_mac)
    }

    pub fn picker_rows(&self) -> Vec<PickerRow> {
        picker_rows(
            self.playback_model(),
            self.playback
                .transfer
                .as_ref()
                .map(|pending| pending.id.as_str()),
            self.playback.now.volume,
        )
    }

    pub fn show_devices(&mut self, anchor: Option<&'static str>) -> bool {
        let anchor = anchor.unwrap_or(if self.playback.dock_devices.get() {
            DEVICES_BUTTON
        } else {
            DEVICE_PANEL
        });
        let items = self.device_menu();
        self.show_menu(items, Place::Above(anchor), None)
    }

    pub fn transfer_to(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.jam_redirected() {
            self.show_toast(LEAVE_FIRST, None);
            cx.notify();
            return;
        }
        let Some(model) = self.playback_model() else {
            return;
        };
        let Some(device) = model.devices.iter().find(|device| device.id == id) else {
            return;
        };
        let idle_here = device.this_mac && model.active_device_id.is_none() && !model.orphaned();
        if device.active || device.blocked.is_some() || idle_here {
            return;
        }
        let this_mac = device.this_mac;
        let name = device_label(device);
        if this_mac && self.needs_premium_here(false, cx) {
            return;
        }
        self.playback.transfer = Some(PendingTransfer {
            id: id.to_owned(),
            name,
            since: Instant::now(),
        });
        self.playback.notice = None;
        self.send(Command::Transfer(id.to_owned()));
        cx.notify();
    }

    pub fn play_here(&mut self, cx: &mut Context<Self>) {
        if self.jam_redirected() {
            self.show_toast(LEAVE_FIRST, None);
            cx.notify();
            return;
        }
        if self.needs_premium_here(false, cx) {
            return;
        }
        if let Some(own) = self
            .playback_model()
            .filter(|_| self.remote_device().is_some())
            .and_then(|model| model.devices.iter().find(|device| device.this_mac))
        {
            self.playback.transfer = Some(PendingTransfer {
                id: own.id.clone(),
                name: device_label(own),
                since: Instant::now(),
            });
        }
        self.playback.notice = None;
        self.send(Command::PlayHere);
        cx.notify();
    }

    pub fn volume_state(&self) -> (bool, Option<String>) {
        let host_speaker = self.remote_device().is_some() || self.active_device().is_none();
        if self.jam().is_some_and(|session| {
            !session.is_owner && session.volume_control != VolumeControl::Enabled
        }) && host_speaker
        {
            return (false, Some(HOST_VOLUME.to_owned()));
        }
        match self.remote_device() {
            Some(device) if device.volume.is_none() => (
                false,
                Some(format!("You can’t change the volume on {}", device.name)),
            ),
            Some(device) => (true, Some(format!("Changes only apply to {}", device.name))),
            None => (true, None),
        }
    }

    pub fn volume_allowed(&self) -> bool {
        self.volume_state().0
    }

    fn volume_steps(&self) -> u32 {
        self.remote_device()
            .map_or(LOCAL_VOLUME_STEPS, |device| device.volume_steps)
    }

    pub fn volume_step(&self) -> u16 {
        connect::volume_step(self.volume_steps())
    }

    pub fn set_volume(&mut self, volume: u16, cx: &mut Context<Self>) {
        if !self.volume_allowed() {
            return;
        }
        self.playback.now.volume = volume;
        self.send(Command::Volume(volume));
        cx.notify();
    }

    pub fn step_volume(&mut self, up: bool, cx: &mut Context<Self>) {
        let volume = connect::stepped_volume(self.playback.now.volume, up, self.volume_steps());
        self.set_volume(volume, cx);
    }

    pub fn queue_allowed(&self, next: bool) -> bool {
        if self.jam_guest() {
            return true;
        }
        let allowed = self.playback_allowed();
        self.playback_model()
            .is_some_and(|model| model.active_device_id.is_some())
            && if next {
                allowed.play_next
            } else {
                allowed.add_to_queue
            }
    }

    pub fn add_to_queue(&mut self, uris: Vec<String>, next: bool, cx: &mut Context<Self>) {
        if uris.is_empty() || !self.queue_allowed(next) {
            return;
        }
        self.playback.notice = None;
        self.send(if next {
            Command::PlayNext(uris)
        } else {
            Command::AddToQueue(uris)
        });
        self.show_toast(ADDED_TO_QUEUE, None);
        self.playback.queue_toast = Some(Instant::now());
        cx.notify();
    }

    pub fn queue_from(&mut self, source: QueueSource, next: bool, cx: &mut Context<Self>) {
        match source {
            QueueSource::Items(uris) => self.add_to_queue(uris, next, cx),
            QueueSource::Context(uri) if self.queue_allowed(next) => {
                self.playback.notice = None;
                self.send(Command::QueueContext { uri, next });
            }
            QueueSource::Context(_) => {}
        }
    }
}

pub fn load(app: &mut OpenMusic, page: &Page) {
    match page {
        Page::Queue => {
            app.request_queue_tracks();
            app.refresh_queue();
        }
        Page::Jam => {
            app.request_queue_tracks();
            app.refresh_queue();
            app.refresh_jam();
            app.request_people();
        }
        Page::JamInvite(uri) => {
            if let Some(token) = spotify_client::jam::token_from_uri(uri) {
                app.playback.jam.mode = JoinMode::default();
                app.preview_jam(&token);
            }
        }
        _ => {}
    }
}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::JamUpdate(update) => app.on_jam_update(*update, window, cx),
        Event::JamCurrent(result) => app.on_jam_current(result, window, cx),
        Event::JamWritten { action, result } => app.on_jam_written(action, result, window, cx),
        Event::JamPreview { token, result } => app.on_jam_preview(token, result),
        Event::People { asked, found } => app.on_people(asked, found),
        Event::Dj { play, dj } => {
            app.playback.dj.checked = true;
            app.playback.dj.available = dj.clone();
            if play {
                match dj {
                    Some(dj) => app.send(Command::PlayDj(dj.context_url)),
                    None => app.show_toast(DJ_UNAVAILABLE, None),
                }
            }
        }
        Event::Playback(view) => app.on_playback(*view),
        Event::Tracks { asked, found } => {
            match found {
                Some(found) => {
                    for uri in &asked {
                        if !found.iter().any(|track| track.uri == *uri) {
                            app.playback.missing.insert(uri.clone());
                        }
                    }
                    for track in &found {
                        app.playback.tracks.insert(track.uri.clone(), track.clone());
                    }
                    app.send(Command::Remember(found));
                }
                None => {
                    for uri in &asked {
                        app.playback.requested.remove(uri);
                    }
                }
            }
            app.refresh_queue();
        }
        Event::SmartShuffle { uri, available } => {
            app.playback.contexts.entry(uri).or_default().smart = Some(available);
        }
        Event::ContextOptions { uri, options } => {
            app.playback.contexts.entry(uri).or_default().server = options;
        }
        Event::Resolved { next, uris } => match uris {
            Some(uris) if !uris.is_empty() => app.add_to_queue(uris, next, cx),
            Some(_) => app.show_toast("There’s nothing here to add to the queue.", None),
            None => app.show_toast("Couldn’t add that to the queue.", None),
        },
    }
    cx.notify();
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {
    app.refresh_jam();
    app.send(Command::DjCheck { play: false });
}

pub fn reconnected(app: &mut OpenMusic) {
    app.refresh_jam();
}

pub fn signed_out(app: &mut OpenMusic) {
    app.playback = State::default();
}

fn parse_shuffle(text: &str) -> Option<Shuffle> {
    match text {
        "off" => Some(Shuffle::Off),
        "on" => Some(Shuffle::On),
        "smart" => Some(Shuffle::Smart),
        _ => None,
    }
}

fn parse_repeat(text: &str) -> Option<Repeat> {
    match text {
        "off" => Some(Repeat::Off),
        "context" => Some(Repeat::Context),
        "track" => Some(Repeat::Track),
        _ => None,
    }
}

fn find_key(app: &OpenMusic, uid: &str) -> Option<QueueKey> {
    let queue = &app.playback.queue;
    queue
        .queued
        .iter()
        .chain(&queue.next)
        .chain(&queue.autoplay)
        .find(|row| row.key.uid == uid || row.key.uri == uid)
        .map(|row| row.key.clone())
}

fn queue_dump(app: &OpenMusic) -> String {
    let queue = &app.playback.queue;
    let mut lines = vec![format!(
        "editable={} jam={} view={} revision={} next_from={:?} refreshes={}",
        app.queue_editable(),
        app.in_jam(),
        app.playback_allowed().view_queue,
        app.playback_model()
            .map_or("", |model| model.queue.revision.as_str()),
        app.next_from_title(),
        app.playback.refreshes,
    )];
    for (name, prefix) in [
        ("current", QUEUE_CURRENT),
        ("queued", QUEUE_QUEUED),
        ("next", QUEUE_NEXT),
        ("autoplay", QUEUE_AUTOPLAY),
        ("recent", QUEUE_RECENT),
    ] {
        for row in queue.section(prefix) {
            lines.push(format!(
                "{name} {} uid={} iteration={} provider={} title={:?} artist={:?} cover={}",
                row.key.uri,
                row.key.uid,
                row.key.iteration,
                row.provider,
                row.track.title,
                row.track.artist,
                row.track
                    .cover
                    .as_ref()
                    .is_some_and(|url| app.covers.contains_key(url)),
            ));
        }
    }
    lines.join("\n")
}

fn devices_dump(app: &OpenMusic) -> String {
    let (volume_enabled, volume_tooltip) = app.volume_state();
    let mut lines = vec![format!(
        "pending={} dock={} volume={} volume_enabled={volume_enabled} volume_tooltip={volume_tooltip:?} step={}",
        app.playback
            .transfer
            .as_ref()
            .map_or("-", |pending| pending.id.as_str()),
        app.playback.dock_devices.get(),
        app.playback.now.volume,
        app.volume_step(),
    )];
    for device in app.playback_model().map_or(&[][..], |model| &model.devices) {
        lines.push(format!(
            "{} name={:?} kind={:?} active={} this_mac={} volume={} steps={} blocked={:?} set_options={}",
            device.id,
            device.name,
            device.kind,
            device.active,
            device.this_mac,
            device
                .volume
                .map_or_else(|| "none".to_owned(), |volume| volume.to_string()),
            device.volume_steps,
            device.blocked,
            device.supports_set_options,
        ));
    }
    lines.join("\n")
}

fn queue_target(uri: &str) -> Option<QueueSource> {
    if lookup_uri(uri) {
        return Some(QueueSource::Items(vec![uri.to_owned()]));
    }
    Target::for_uri(uri).and_then(|target| queue_source(&target))
}

fn context_dump(app: &OpenMusic, uri: &str) -> String {
    let state = app.playback.contexts.get(uri).copied().unwrap_or_default();
    format!(
        "{uri} shuffle={:?} smart_available={} chosen={:?} server={:?} smart={:?} playing={} override={:?}",
        app.context_shuffle(uri),
        app.context_smart_available(uri),
        state.chosen,
        state.server,
        state.smart,
        app.playing_context(uri),
        app.context_play_options(uri).shuffle_override,
    )
}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    let mut words = rest.split_whitespace();
    match verb {
        "shuffle" => {
            if let Some(mode) = parse_shuffle(rest) {
                app.set_shuffle(mode, cx);
            }
        }
        "repeat" => {
            if let Some(mode) = parse_repeat(rest) {
                app.set_repeat(mode, cx);
            }
        }
        "queue" => {
            let _ = std::fs::write(directory.join("queue.txt"), queue_dump(app));
        }
        "enqueue" if !rest.is_empty() => {
            app.send(Command::AddToQueue(words.map(str::to_owned).collect()))
        }
        "playnext" if !rest.is_empty() => {
            app.send(Command::PlayNext(words.map(str::to_owned).collect()))
        }
        "dequeue" => {
            let keys = words.filter_map(|uid| find_key(app, uid)).collect();
            app.remove_from_queue(keys, cx);
        }
        "clearqueue" => {
            if rest == "confirm" {
                app.confirm_clear_queue(window, cx);
            } else {
                app.clear_queue(cx);
            }
        }
        "jump" => {
            let queue = &app.playback.queue;
            let row = [QUEUE_QUEUED, QUEUE_NEXT, QUEUE_AUTOPLAY]
                .into_iter()
                .find_map(|prefix| {
                    queue
                        .section(prefix)
                        .iter()
                        .position(|row| row.key.uid == rest || row.key.uri == rest)
                        .map(|index| (prefix, index))
                });
            if let Some(row) = row {
                app.jump_in_queue(row, cx);
            }
        }
        "movequeue" => {
            let (Some(moved), Some(anchor), Some(place)) =
                (words.next(), words.next(), words.next())
            else {
                return true;
            };
            if let (Some(moved), Some(anchor)) = (find_key(app, moved), find_key(app, anchor)) {
                app.move_in_queue(vec![moved], anchor, place == "after", cx);
            }
        }
        "shuffleplay" if !rest.is_empty() => app.shuffle_play(rest, cx),
        "contextshuffle" => {
            let (uri, action) = rest.split_once(' ').unwrap_or((rest, ""));
            if uri.is_empty() {
                return true;
            }
            app.request_context_options(uri);
            if action == "toggle" {
                app.toggle_context_shuffle(uri, cx);
            }
            let _ = std::fs::write(directory.join("context.txt"), context_dump(app, uri));
        }
        "refreshcluster" => app.send(Command::RefreshCluster),
        "devices" => {
            let _ = std::fs::write(directory.join("devices.txt"), devices_dump(app));
        }
        "devicemenu" => {
            let lines = menu::describe_items(app.device_menu());
            let _ = std::fs::write(directory.join("devices-menu.txt"), lines.join("\n"));
        }
        "showdevices" => {
            let anchor = match rest {
                "dock" => Some(DEVICES_BUTTON),
                "panel" => Some(DEVICE_PANEL),
                _ => None,
            };
            app.show_devices(anchor);
        }
        "transfer" if !rest.is_empty() => app.transfer_to(rest, cx),
        "devvolume" => {
            if let (Some(device), Some(Ok(volume))) =
                (words.next(), words.next().map(str::parse::<u16>))
            {
                app.send(Command::DeviceVolume {
                    device: device.to_owned(),
                    volume,
                });
            }
        }
        "volumestep" => match rest {
            "up" => app.step_volume(true, cx),
            "down" => app.step_volume(false, cx),
            _ => {}
        },
        "addqueue" | "addnext" => {
            if let Some(source) = queue_target(rest) {
                app.queue_from(source, verb == "addnext", cx);
            }
        }
        "jam" => {
            let _ = std::fs::write(directory.join("jam.txt"), jams::dump(app));
        }
        "jamrefresh" => app.refresh_jam(),
        "jamstart" => app.start_jam(None, cx),
        "jamleave" if rest == "confirm" => app.confirm_leave_jam(window, cx),
        "jamleave" => app.leave_jam(cx),
        "jamend" if rest == "confirm" => app.confirm_end_jam(window, cx),
        "jamend" => app.end_jam(cx),
        "jamjoin" => {
            let link = words.next().unwrap_or_default();
            let mode = match words.next() {
                Some("inperson") => JoinMode::InPerson,
                _ => JoinMode::Remote,
            };
            if let Some(token) = spotify_client::jam::token(link) {
                app.join_jam(token, mode, cx);
            }
        }
        "jampreview" => {
            if let Some(token) = spotify_client::jam::token(rest) {
                app.navigate(Page::JamInvite(spotify_client::jam::invite_uri(&token)));
            }
        }
        "jamqueueonly" => app.set_jam_queue_only(rest == "on", cx),
        "jamvolume" => app.set_jam_volume_control(rest == "on", cx),
        "jamremove" => {
            let member = words.next().unwrap_or_default().to_owned();
            if words.next() == Some("confirm") {
                app.confirm_remove_from_jam(&member, window, cx);
            } else {
                app.remove_from_jam(member, cx);
            }
        }
        "jamlink" => app.copy_jam_link(cx),
        "jamfixture" => {
            let session = std::fs::read(rest)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                .as_ref()
                .and_then(spotify_client::jam::parse);
            app.show_jam_fixture(session);
        }
        "dj" => {
            let _ = std::fs::write(directory.join("dj.txt"), dj_dump(app));
        }
        "djcheck" => app.send(Command::DjCheck { play: false }),
        "djplay" => app.play_dj(cx),
        "djjump" => app.dj_jump(cx),
        _ => return false,
    }
    true
}

fn dj_dump(app: &OpenMusic) -> String {
    let model = app.playback_model();
    format!(
        "available={} checked={} name={:?} context={} narrating={} jump={:?} dock={}",
        app.playback.dj.available.is_some(),
        app.playback.dj.checked,
        app.playback
            .dj
            .available
            .as_ref()
            .map(|dj| dj.name.as_str()),
        app.dj_context(),
        app.dj_narrating(),
        model.and_then(dj::jump).map(|jump| match jump {
            dj::Jump::Signal => "signal".to_owned(),
            dj::Jump::SkipTo(track) => format!("skip_to {}", track.uri),
        }),
        app.dj_context() && app.dj_jump_available(),
    )
}

pub fn state_fragment(app: &OpenMusic) -> String {
    let model = app.playback_model();
    format!(
        "shuffle={:?} repeat={:?} queued={} next_up={} revision={} active={} refreshes={} transfer={} volume_enabled={} jam={} jam_members={} jam_redirect={} dj={} dj_context={}",
        app.shuffle_mode(),
        app.repeat_mode(),
        model.map_or(0, |model| model.queue.queued.len()),
        model.map_or(0, |model| model.queue.next_up.len()),
        model.map_or("-", |model| model.queue.revision.as_str()),
        model
            .and_then(|model| model.active_device_id.as_deref())
            .unwrap_or("-"),
        app.playback.refreshes,
        app.playback
            .transfer
            .as_ref()
            .map_or("-", |pending| pending.id.as_str()),
        app.volume_allowed(),
        app.jam().map_or("-", |session| session.id.as_str()),
        app.jam().map_or(0, |session| session.members.len()),
        app.playback.jam.redirect().is_some(),
        app.playback.dj.available.is_some(),
        app.dj_context(),
    )
}

pub fn register_actions(cx: &mut App, handle: &Handle) {
    menus::route(cx, handle, |_: &ToggleShuffle, app, window, cx| {
        app.perform(Intent::ToggleShuffle, window, cx)
    });
    menus::route(cx, handle, |_: &CycleRepeat, app, window, cx| {
        app.perform(Intent::CycleRepeat, window, cx)
    });
    menus::route(cx, handle, |_: &ShowQueue, app, _, cx| {
        app.toggle_queue_page();
        cx.notify();
    });
    menus::route(cx, handle, |_: &ShowDevices, app, _, cx| {
        if app.show_devices(None) {
            cx.notify();
        }
    });
    menus::route(cx, handle, |_: &PlayOnThisMac, app, _, cx| {
        app.play_here(cx)
    });
    menus::route(cx, handle, |_: &StartJam, app, _, cx| {
        app.start_jam(None, cx);
        cx.notify();
    });
}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    if jams::on_press(app, key, window, cx) {
        return true;
    }
    match key {
        DJ_JUMP => app.dj_jump(cx),
        "previous" => app.perform(Intent::Previous, window, cx),
        "next" => app.perform(Intent::Next, window, cx),
        "toggle-playback" => app.perform(Intent::PlayPause, window, cx),
        "play-here" => app.play_here(cx),
        DEVICES_BUTTON | DEVICE_PANEL => {
            app.show_devices(Some(key));
        }
        "shuffle" => app.perform(Intent::ToggleShuffle, window, cx),
        "repeat" => app.perform(Intent::CycleRepeat, window, cx),
        "queue" => app.toggle_queue_page(),
        "queue-clear" => app.confirm_clear_queue(window, cx),
        _ => return false,
    }
    true
}

pub fn row_dropped(
    app: &mut OpenMusic,
    drag: &RowDrag,
    at: RowDrop,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    if !MOVABLE.contains(&at.row.0) || !MOVABLE.contains(&drag.source.0) {
        return false;
    }
    let queue = &app.playback.queue;
    let (Some(moved), Some(anchor)) = (queue.row(drag.source), queue.row(at.row)) else {
        return true;
    };
    let (moved, anchor) = (moved.key.clone(), anchor.key.clone());
    app.move_in_queue(vec![moved], anchor, at.after, cx);
    true
}

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    let failed = app.playback.notice.is_some();
    let expired = |since: Instant| failed || since.elapsed() >= OPTIMISTIC;
    let mut changed = false;
    if app
        .playback
        .pending_shuffle
        .is_some_and(|(_, since)| expired(since))
    {
        app.playback.pending_shuffle = None;
        changed = true;
    }
    if app
        .playback
        .pending_repeat
        .is_some_and(|(_, since)| expired(since))
    {
        app.playback.pending_repeat = None;
        changed = true;
    }
    if app
        .playback
        .pending_queue
        .as_ref()
        .is_some_and(|pending| expired(pending.since))
    {
        app.playback.pending_queue = None;
        app.refresh_queue();
        changed = true;
    }
    if let Some(pending) = &app.playback.transfer {
        if failed {
            app.playback.transfer = None;
            changed = true;
        } else if pending.since.elapsed() >= TRANSFER_TIMEOUT {
            let message = format!("Couldn’t connect to {}.", pending.name);
            app.playback.transfer = None;
            app.show_toast(message, None);
            changed = true;
        }
    }
    if let Some(since) = app.playback.queue_toast {
        let showing = app
            .toast
            .as_ref()
            .is_some_and(|toast| toast.text == ADDED_TO_QUEUE);
        if failed && showing && since.elapsed() < QUEUE_TOAST {
            app.toast = None;
            app.playback.queue_toast = None;
            changed = true;
        } else if since.elapsed() >= QUEUE_TOAST || !showing {
            app.playback.queue_toast = None;
        }
    }
    if let Page::Collection(uri) = &app.page
        && !app
            .playback
            .contexts
            .get(uri)
            .is_some_and(|state| state.requested)
    {
        let uri = uri.clone();
        app.request_context_options(&uri);
    }
    if app.jam_tick() {
        changed = true;
    }
    if note_dj_voice(app) {
        changed = true;
    }
    if changed {
        cx.notify();
    }
}

fn note_dj_voice(app: &mut OpenMusic) -> bool {
    let here = app.active_device().is_some_and(|device| device.this_mac);
    let context = app
        .playback_model()
        .filter(|_| here && app.dj_context())
        .and_then(|model| model.context_uri.clone());
    let Some(context) = context else {
        return false;
    };
    if app.playback.dj.voice_noted.as_ref() == Some(&context) {
        return false;
    }
    app.playback.dj.voice_noted = Some(context);
    app.show_toast(DJ_VOICE, None);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_shuffle_modes() {
        assert_eq!(next_shuffle(Shuffle::Off, true, true), Shuffle::On);
        assert_eq!(next_shuffle(Shuffle::On, true, true), Shuffle::Smart);
        assert_eq!(next_shuffle(Shuffle::Smart, true, true), Shuffle::Off);
        assert_eq!(next_shuffle(Shuffle::On, true, false), Shuffle::Off);
        assert_eq!(next_shuffle(Shuffle::Smart, true, false), Shuffle::Off);
        assert_eq!(next_shuffle(Shuffle::Off, false, true), Shuffle::Off);
    }

    #[test]
    fn cycles_repeat_modes_skipping_disallowed_ones() {
        let all = Allowed {
            repeat_context: true,
            repeat_track: true,
            ..Allowed::default()
        };
        assert_eq!(next_repeat(Repeat::Off, &all), Repeat::Context);
        assert_eq!(next_repeat(Repeat::Context, &all), Repeat::Track);
        assert_eq!(next_repeat(Repeat::Track, &all), Repeat::Off);
        let context_only = Allowed {
            repeat_context: true,
            ..Allowed::default()
        };
        assert_eq!(next_repeat(Repeat::Context, &context_only), Repeat::Off);
        let track_only = Allowed {
            repeat_track: true,
            ..Allowed::default()
        };
        assert_eq!(next_repeat(Repeat::Off, &track_only), Repeat::Track);
        assert_eq!(next_repeat(Repeat::Off, &Allowed::default()), Repeat::Off);
    }

    #[test]
    fn builds_rows_from_queue_metadata() {
        let item = QueueItem {
            uri: "spotify:track:1111111111111111111111".into(),
            uid: "u1".into(),
            iteration: "0".into(),
            provider: "queue".into(),
            title: Some("Song".into()),
            artist: None,
            cover: Some("https://i.scdn.co/image/ab".into()),
            metadata: [
                ("album_title", "Record"),
                ("album_uri", "spotify:album:2222222222222222222222"),
                ("duration", "152048"),
                ("is_explicit", "true"),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect(),
        };
        let track = metadata_track(&item, "Loading…");
        assert_eq!(track.title, "Song");
        assert_eq!(track.artist, "Record");
        assert_eq!(track.album, "Record");
        assert_eq!(
            track.album_uri.as_deref(),
            Some("spotify:album:2222222222222222222222")
        );
        assert_eq!(track.duration_ms, 152_048);
        assert!(track.explicit);
        assert_eq!(uid(&item).as_deref(), Some("u1"));
        let bare = QueueItem {
            title: None,
            metadata: HashMap::new(),
            ..item
        };
        assert_eq!(metadata_track(&bare, "Loading…").title, "Loading…");
        assert_eq!(metadata_track(&bare, "Loading…").duration_ms, 0);
        assert_eq!(queued_by(&bare), None);
        let jam_item = QueueItem {
            metadata: [("queued_by".to_owned(), " friend ".to_owned())]
                .into_iter()
                .collect(),
            ..bare.clone()
        };
        assert_eq!(queued_by(&jam_item).as_deref(), Some("friend"));
        let blank = QueueItem {
            metadata: [("queued_by".to_owned(), String::new())]
                .into_iter()
                .collect(),
            ..bare
        };
        assert_eq!(queued_by(&blank), None);
    }

    const OWN: &str = "aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000";
    const DESKTOP: &str = "bbbb1111bbbb1111bbbb1111bbbb1111bbbb1111";
    const SPEAKER: &str = "cccc2222cccc2222cccc2222cccc2222cccc2222";
    const TV: &str = "dddd3333dddd3333dddd3333dddd3333dddd3333";

    fn connect_device(id: &str, name: &str, kind: DeviceKind) -> ConnectDevice {
        ConnectDevice {
            id: id.into(),
            name: name.into(),
            kind,
            this_mac: id == OWN,
            volume: Some(30_000),
            volume_steps: 64,
            supports_set_options: true,
            ..ConnectDevice::default()
        }
    }

    fn model(devices: Vec<ConnectDevice>) -> Playback {
        Playback {
            active_device_id: devices
                .iter()
                .find(|device| device.active)
                .map(|device| device.id.clone()),
            active_device_name: None,
            active_volume: None,
            own_device_id: OWN.into(),
            devices,
            context_uri: None,
            context_title: None,
            context_metadata: HashMap::new(),
            session_id: String::new(),
            track: None,
            track_metadata: HashMap::new(),
            provider: None,
            playing: false,
            position_ms: 0,
            playback_speed: 1.,
            shuffle: Shuffle::Off,
            repeat: Repeat::Off,
            allowed: Allowed::default(),
            queue: connect::Queue::default(),
            signals: Vec::new(),
            sleep_timer: connect::SleepState::Off,
            jam_mode: None,
        }
    }

    fn device_row(id: &str, title: &str, symbol: &'static str, current: bool) -> PickerRow {
        PickerRow::Device {
            id: id.into(),
            title: title.into(),
            symbol,
            current,
            enabled: true,
            subtitle: None,
        }
    }

    #[test]
    fn lists_devices_for_the_picker() {
        let desktop = ConnectDevice {
            active: true,
            ..connect_device(DESKTOP, "Desktop", DeviceKind::Computer)
        };
        let speaker = ConnectDevice {
            blocked: Some("Unavailable".into()),
            ..connect_device(SPEAKER, "Kitchen", DeviceKind::Speaker)
        };
        let tv = ConnectDevice {
            volume: None,
            ..connect_device(TV, "Living Room", DeviceKind::Tv)
        };
        let mac = connect_device(OWN, "OpenMusic", DeviceKind::Computer);
        let playback = model(vec![desktop, mac, speaker, tv]);
        assert_eq!(
            picker_rows(Some(&playback), Some(TV), 32_768),
            vec![
                PickerRow::Header("Current device"),
                device_row(DESKTOP, "Desktop", "laptopcomputer", true),
                PickerRow::Volume(32_768. / 65_535.),
                PickerRow::Header("Select another device"),
                device_row(OWN, "This Mac", "laptopcomputer", false),
                PickerRow::Device {
                    id: SPEAKER.into(),
                    title: "Kitchen".into(),
                    symbol: "hifispeaker",
                    current: false,
                    enabled: false,
                    subtitle: Some("Unavailable".into()),
                },
                PickerRow::Device {
                    id: TV.into(),
                    title: "Living Room".into(),
                    symbol: "tv",
                    current: false,
                    enabled: true,
                    subtitle: Some(CONNECTING.into()),
                },
            ]
        );
    }

    #[test]
    fn shows_this_mac_when_nothing_plays_and_hides_fixed_volumes() {
        let mac = connect_device(OWN, "OpenMusic", DeviceKind::Computer);
        let desktop = connect_device(DESKTOP, "Desktop", DeviceKind::Computer);
        let idle = model(vec![mac.clone(), desktop]);
        assert_eq!(
            picker_rows(Some(&idle), None, 65_535),
            vec![
                PickerRow::Header("Current device"),
                device_row(OWN, "This Mac", "laptopcomputer", true),
                PickerRow::Volume(1.),
                PickerRow::Header("Select another device"),
                device_row(DESKTOP, "Desktop", "laptopcomputer", false),
            ]
        );
        let tv = ConnectDevice {
            active: true,
            volume: None,
            ..connect_device(TV, "Living Room", DeviceKind::Tv)
        };
        let fixed = model(vec![tv, mac]);
        let rows = picker_rows(Some(&fixed), None, 20_000);
        assert!(!rows.iter().any(|row| matches!(row, PickerRow::Volume(_))));
        assert_eq!(rows[1], device_row(TV, "Living Room", "tv", true));
        let alone = model(vec![connect_device(OWN, "OpenMusic", DeviceKind::Computer)]);
        assert_eq!(
            picker_rows(Some(&alone), None, 0).last(),
            Some(&PickerRow::Note("No other devices found"))
        );
        assert_eq!(
            picker_rows(None, None, 0),
            vec![
                PickerRow::Header("Current device"),
                device_row("", "This Mac", "laptopcomputer", true),
                PickerRow::Volume(0.),
                PickerRow::Header("Select another device"),
                PickerRow::Note("No other devices found"),
            ]
        );
    }

    #[test]
    fn resolves_transfers_by_entry_or_device() {
        let alias = ConnectDevice {
            active: true,
            alias_id: Some(2),
            ..connect_device(&format!("{SPEAKER}::a_2"), "Upstairs", DeviceKind::Group)
        };
        let mut playback = model(vec![alias]);
        playback.active_device_id = Some(SPEAKER.into());
        assert!(transferred(&playback, &format!("{SPEAKER}::a_2")));
        assert!(transferred(&playback, &format!("{SPEAKER}::a_5")));
        assert!(transferred(&playback, SPEAKER));
        assert!(!transferred(&playback, TV));
        assert!(!transferred(&playback, &format!("{TV}::a_2")));
    }

    #[test]
    fn picks_what_to_queue_from_a_menu_target() {
        let track = Target::track(
            Track::placeholder("spotify:track:1111111111111111111111"),
            None,
            crate::ui::menu::Surface::Row,
        );
        assert_eq!(
            queue_source(&track),
            Some(QueueSource::Items(vec![
                "spotify:track:1111111111111111111111".into()
            ]))
        );
        let local = Target::track(
            Track::placeholder("spotify:local:Artist:Album:Song:180"),
            None,
            crate::ui::menu::Surface::Row,
        );
        assert_eq!(queue_source(&local), None);
        let cases = [
            (
                "spotify:album:2222222222222222222222",
                Some(QueueSource::Context(
                    "spotify:album:2222222222222222222222".into(),
                )),
            ),
            (
                "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M",
                Some(QueueSource::Context(
                    "spotify:playlist:37i9dQZF1DXcBWIGoYBM5M".into(),
                )),
            ),
            (
                "spotify:user:someone:collection",
                Some(QueueSource::Context(LIKED_SONGS.into())),
            ),
            (
                "spotify:episode:3333333333333333333333",
                Some(QueueSource::Items(vec![
                    "spotify:episode:3333333333333333333333".into(),
                ])),
            ),
            ("spotify:artist:4444444444444444444444", None),
            ("spotify:show:5555555555555555555555", None),
            ("spotify:user:someone", None),
        ];
        for (uri, expected) in cases {
            let target = Target::for_uri(uri).expect(uri);
            assert_eq!(queue_source(&target), expected, "{uri}");
        }
        assert_eq!(
            queue_target("spotify:track:1111111111111111111111"),
            Some(QueueSource::Items(vec![
                "spotify:track:1111111111111111111111".into()
            ]))
        );
    }

    #[test]
    fn finds_rows_by_section() {
        let row = |uid: &str| QueueRow {
            key: QueueKey {
                uid: uid.into(),
                iteration: String::new(),
                uri: format!("spotify:track:{uid}"),
            },
            provider: "queue".into(),
            track: Track::placeholder(&format!("spotify:track:{uid}")),
            queued_by: None,
        };
        let view = QueueView {
            current: Some(row("now")),
            queued: vec![row("a"), row("b")],
            next: vec![row("c")],
            autoplay: vec![row("d")],
            recent: vec![row("e")],
        };
        assert_eq!(
            view.row((QUEUE_QUEUED, 1)).map(|row| row.key.uid.as_str()),
            Some("b")
        );
        assert_eq!(
            view.row((QUEUE_CURRENT, 0)).map(|row| row.key.uid.as_str()),
            Some("now")
        );
        assert_eq!(
            view.row((QUEUE_RECENT, 0)).map(|row| row.key.uid.as_str()),
            Some("e")
        );
        assert_eq!(view.row((QUEUE_NEXT, 3)), None);
        assert_eq!(view.row(("playlist", 0)), None);
        assert_eq!(view.upcoming(), 4);
    }
}
