use spotify_client::{
    connect::{self, PlayOptions, Playback},
    dealer::Message,
    engine::Control,
};

use super::state::PlaybackPrefs;
use crate::worker::Ctx;

pub enum Command {
    PlayContext {
        uri: String,
        index: usize,
        track_uri: Option<String>,
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
}

pub enum Event {}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match *self {}
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    let control = match command {
        Command::PlayContext {
            uri,
            index,
            track_uri,
        } => Control::PlayContext {
            uri,
            index,
            track_uri,
            origin: connect::ORIGIN,
            options: PlayOptions::default(),
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
    };
    ctx.handle.send(control);
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

pub fn on_playback(_playback: Box<Playback>, _ctx: &mut Ctx) {}

pub fn apply_settings(_prefs: &PlaybackPrefs, _ctx: &mut Ctx) {}
