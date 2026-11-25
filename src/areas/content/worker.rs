use spotify_client::{Api, catalog, catalog::Lyrics, dealer::Message};

use super::state::ContentPrefs;
use crate::worker::Ctx;

pub enum Command {
    Lyrics(String),
}

pub enum Event {
    Lyrics { uri: String, lyrics: Option<Lyrics> },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Lyrics { .. } => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Lyrics(uri) => ctx
            .tasks
            .spawn_keyed("lyrics", lyrics(ctx.api.clone(), uri)),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

pub fn apply_settings(_prefs: &ContentPrefs, _ctx: &mut Ctx) {}

async fn lyrics(api: Api, uri: String) -> crate::worker::Event {
    let found = catalog::lyrics(&api, &uri).await.ok().flatten();
    Event::Lyrics { uri, lyrics: found }.into()
}
