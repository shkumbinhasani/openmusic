use spotify_client::{Api, catalog, catalog::Playlist, dealer::Message};

use super::state::LibraryPrefs;
use crate::worker::{Ctx, User, cache};

pub enum Command {
    Playlists,
}

pub enum Event {
    Playlists(Vec<Playlist>),
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Playlists(playlists) => playlists
                .iter()
                .filter_map(|playlist| playlist.cover.clone())
                .collect(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Playlists => ctx
            .tasks
            .spawn(playlists(ctx.api.clone(), ctx.user.clone())),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

pub fn apply_settings(_prefs: &LibraryPrefs, _ctx: &mut Ctx) {}

async fn playlists(api: Api, user: User) -> crate::worker::Event {
    let playlists = match user.name(&api).await {
        Ok(user) => catalog::playlists(&api, &user).await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    cache::save_playlists(&playlists);
    Event::Playlists(playlists).into()
}
