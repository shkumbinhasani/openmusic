use spotify_client::dealer::Message;

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
