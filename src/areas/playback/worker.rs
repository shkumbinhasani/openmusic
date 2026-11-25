use spotify_client::{
    Track, catalog,
    connect::{self, ContextOptions, PlayOptions, Playback, QueueKey, Repeat, Shuffle},
    dealer::Message,
    engine::{Control, remote},
};

use super::state::PlaybackPrefs;
use crate::worker::Ctx;

pub enum Command {
    PlayContext {
        uri: String,
        index: usize,
        track_uri: Option<String>,
        origin: &'static str,
        options: PlayOptions,
    },
    PlayTracks {
        tracks: Vec<String>,
        index: usize,
    },
    SetPlaying(bool),
    Next,
    Previous,
    Seek(u32),
    Volume(u16),
    PlayHere,
    Shuffle(Shuffle),
    Repeat(Repeat),
    ShufflePlay(String),
    AddToQueue(Vec<String>),
    PlayNext(Vec<String>),
    RemoveFromQueue(Vec<QueueKey>),
    MoveInQueue {
        moved: Vec<QueueKey>,
        anchor: QueueKey,
        after: bool,
    },
    ClearQueue,
    JumpTo(QueueKey),
    RefreshCluster,
    LookupTracks(Vec<String>),
    Remember(Vec<Track>),
    SmartShuffleEligibility(String),
    ContextPlayerOptions(String),
    Transfer(String),
    DeviceVolume {
        device: String,
        volume: u16,
    },
    QueueContext {
        uri: String,
        next: bool,
    },
}

pub const QUEUE_CONTEXT_LIMIT: usize = 200;

pub struct PlaybackView {
    pub playback: Playback,
    pub tracks: Vec<Track>,
}

impl PlaybackView {
    pub fn uris(playback: &Playback) -> Vec<String> {
        let queue = &playback.queue;
        queue
            .current
            .iter()
            .chain(&queue.queued)
            .chain(&queue.next_up)
            .chain(&queue.recent(RECENT_LIMIT))
            .map(|item| item.uri.clone())
            .collect()
    }
}

pub const RECENT_LIMIT: usize = 10;

pub enum Event {
    Playback(Box<PlaybackView>),
    Tracks {
        asked: Vec<String>,
        found: Option<Vec<Track>>,
    },
    SmartShuffle {
        uri: String,
        available: bool,
    },
    ContextOptions {
        uri: String,
        options: Option<ContextOptions>,
    },
    Resolved {
        next: bool,
        uris: Option<Vec<String>>,
    },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Playback(view) => {
                let queue = &view.playback.queue;
                queue
                    .current
                    .iter()
                    .chain(&queue.queued)
                    .chain(&queue.next_up)
                    .filter_map(|item| item.cover.clone())
                    .chain(view.tracks.iter().filter_map(|track| track.cover.clone()))
                    .collect()
            }
            Self::Tracks { found, .. } => found
                .iter()
                .flatten()
                .filter_map(|track| track.cover.clone())
                .collect(),
            Self::SmartShuffle { .. } | Self::ContextOptions { .. } | Self::Resolved { .. } => {
                Vec::new()
            }
        }
    }
}

pub fn lookup_uri(uri: &str) -> bool {
    uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:")
}

fn complete(track: &Track) -> bool {
    !track.title.is_empty() && track.cover.is_some()
}

fn known(ctx: &Ctx, uri: &str) -> Option<Track> {
    ctx.known
        .tracks
        .get(uri)
        .filter(|track| complete(track))
        .cloned()
}

fn remote(control: remote::Control) -> Control {
    Control::Remote(control)
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    let control = match command {
        Command::PlayContext {
            uri,
            index,
            track_uri,
            origin,
            options,
        } => Control::PlayContext {
            uri,
            index,
            track_uri,
            origin,
            options,
        },
        Command::PlayTracks { tracks, index } => Control::PlayTracks {
            uris: tracks,
            index,
        },
        Command::SetPlaying(playing) => Control::SetPlaying(playing),
        Command::Next => Control::Next,
        Command::Previous => Control::Previous,
        Command::Seek(position) => Control::Seek(position),
        Command::Volume(volume) => Control::Volume(volume),
        Command::PlayHere => Control::PlayHere,
        Command::Shuffle(mode) => remote(remote::Control::SetShuffle(mode)),
        Command::Repeat(mode) => remote(remote::Control::SetRepeat(mode)),
        Command::ShufflePlay(uri) => remote(remote::Control::ShufflePlay {
            uri,
            origin: connect::ORIGIN,
            options: PlayOptions::default(),
        }),
        Command::AddToQueue(uris) => remote(remote::Control::AddToQueue(uris)),
        Command::PlayNext(uris) => remote(remote::Control::PlayNext(uris)),
        Command::RemoveFromQueue(keys) => remote(remote::Control::RemoveFromQueue(keys)),
        Command::MoveInQueue {
            moved,
            anchor,
            after,
        } => remote(remote::Control::MoveInQueue {
            moved,
            anchor,
            after,
        }),
        Command::ClearQueue => remote(remote::Control::ClearQueue),
        Command::JumpTo(key) => remote(remote::Control::JumpTo(key)),
        Command::RefreshCluster => remote(remote::Control::RefreshCluster),
        Command::LookupTracks(uris) => return lookup(uris, ctx),
        Command::Remember(tracks) => {
            for track in tracks.into_iter().filter(complete) {
                ctx.known.tracks.insert(track.uri.clone(), track);
            }
            return;
        }
        Command::SmartShuffleEligibility(uri) => {
            let api = ctx.api.clone();
            ctx.tasks
                .spawn_keyed(format!("smart-shuffle {uri}"), async move {
                    let available = connect::smart_shuffle(&api, &uri).await.unwrap_or(false);
                    Event::SmartShuffle { uri, available }.into()
                });
            return;
        }
        Command::ContextPlayerOptions(uri) => {
            let api = ctx.api.clone();
            let device = ctx.handle.device_id.clone();
            ctx.tasks
                .spawn_keyed(format!("context-options {uri}"), async move {
                    let options = connect::context_options(&api, &uri, &device).await.ok();
                    Event::ContextOptions { uri, options }.into()
                });
            return;
        }
        Command::Transfer(device) => remote(remote::Control::Transfer(device)),
        Command::DeviceVolume { device, volume } => {
            remote(remote::Control::DeviceVolume { device, volume })
        }
        Command::QueueContext { uri, next } => {
            let api = ctx.api.clone();
            ctx.tasks
                .spawn_keyed(format!("queue-context {uri}"), async move {
                    let uris = catalog::context_uris(&api, &uri, QUEUE_CONTEXT_LIMIT)
                        .await
                        .ok()
                        .map(|items| {
                            items
                                .into_iter()
                                .map(|item| item.uri)
                                .filter(|uri| lookup_uri(uri))
                                .collect()
                        });
                    Event::Resolved { next, uris }.into()
                });
            return;
        }
    };
    ctx.handle.send(control);
}

fn lookup(asked: Vec<String>, ctx: &mut Ctx) {
    let mut found = Vec::new();
    let mut missing = Vec::new();
    for uri in asked.iter().filter(|uri| lookup_uri(uri)) {
        match known(ctx, uri) {
            Some(track) => found.push(track),
            None if !missing.contains(uri) => missing.push(uri.clone()),
            None => {}
        }
    }
    if missing.is_empty() {
        ctx.publish(Event::Tracks {
            asked,
            found: Some(found),
        });
        return;
    }
    let api = ctx.api.clone();
    ctx.tasks.spawn(async move {
        let looked_up = catalog::tracks(&api, &missing).await.ok();
        Event::Tracks {
            asked,
            found: looked_up.map(|mut tracks| {
                tracks.extend(found);
                tracks
            }),
        }
        .into()
    });
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

pub fn on_playback(playback: Box<Playback>, ctx: &mut Ctx) {
    let mut tracks: Vec<Track> = Vec::new();
    for uri in PlaybackView::uris(&playback) {
        if !tracks.iter().any(|track| track.uri == uri)
            && let Some(track) = known(ctx, &uri)
        {
            tracks.push(track);
        }
    }
    ctx.publish(Event::Playback(Box::new(PlaybackView {
        playback: *playback,
        tracks,
    })));
}

pub fn apply_settings(_prefs: &PlaybackPrefs, _ctx: &mut Ctx) {}
