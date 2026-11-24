use spotify_client::{dealer::Message, engine::playback::Update};

use super::state::AudioPrefs;
use crate::worker::Ctx;

pub enum Command {}

pub enum Event {}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match *self {}
    }
}

pub fn handle(command: Command, _ctx: &mut Ctx) {
    match command {}
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

pub fn on_update(update: Update, _ctx: &mut Ctx) {
    match update {}
}

pub fn apply_settings(_prefs: &AudioPrefs, _ctx: &mut Ctx) {}
