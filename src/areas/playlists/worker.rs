use spotify_client::{Api, catalog, catalog::Collection, dealer::Message};

use crate::worker::Ctx;

pub enum Command {
    Collection(String),
}

pub enum Event {
    Collection {
        uri: String,
        result: Result<Collection, String>,
    },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Collection {
                result: Ok(collection),
                ..
            } => collection
                .cover
                .iter()
                .cloned()
                .chain(
                    collection
                        .tracks
                        .iter()
                        .filter_map(|track| track.cover.clone()),
                )
                .collect(),
            Self::Collection { result: Err(_), .. } => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Collection(uri) => ctx.tasks.spawn(collection(ctx.api.clone(), uri)),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

async fn collection(api: Api, uri: String) -> crate::worker::Event {
    let result = catalog::collection(&api, &uri)
        .await
        .map_err(|_| "Couldn’t load this. Try again.".to_owned());
    Event::Collection { uri, result }.into()
}
