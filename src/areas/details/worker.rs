use spotify_client::{
    Api,
    dealer::Message,
    details::{self, Credits, Unavailable},
    pathfinder::PathfinderError,
};

use crate::worker::Ctx;

pub enum Command {
    Credits(String),
}

pub enum Event {
    Credits {
        uri: String,
        result: Result<Credits, String>,
    },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Credits { .. } => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Credits(uri) => ctx
            .tasks
            .spawn_keyed("credits", credits(ctx.api.clone(), uri)),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

async fn credits(api: Api, uri: String) -> crate::worker::Event {
    let result = details::credits(&api, &uri)
        .await
        .map_err(|error| failure(&error, "Couldn’t load the credits. Try again."));
    Event::Credits { uri, result }.into()
}

fn failure(error: &anyhow::Error, fallback: &str) -> String {
    if error.downcast_ref::<Unavailable>().is_some()
        || error.downcast_ref::<PathfinderError>().is_some()
    {
        error.to_string()
    } else {
        fallback.to_owned()
    }
}
