use spotify_client::{
    Api,
    catalog::Link,
    dealer::Message,
    details::{self, Credits, NoRadio, Unavailable},
    pathfinder::PathfinderError,
};

use crate::worker::Ctx;

pub enum Command {
    Credits(String),
    CoverColor(String),
    TrackArtists(String),
    Radio(String),
}

pub enum Event {
    Credits {
        uri: String,
        result: Result<Credits, String>,
    },
    CoverColor {
        uri: String,
        color: Option<u32>,
    },
    TrackArtists {
        uri: String,
        artists: Vec<Link>,
    },
    Radio {
        seed: String,
        result: Result<String, String>,
    },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Credits { .. }
            | Self::CoverColor { .. }
            | Self::TrackArtists { .. }
            | Self::Radio { .. } => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    let api = ctx.api.clone();
    match command {
        Command::Credits(uri) => ctx.tasks.spawn_keyed("credits", credits(api, uri)),
        Command::CoverColor(uri) => ctx.tasks.spawn_keyed("cover-color", cover_color(api, uri)),
        Command::TrackArtists(uri) => ctx.tasks.spawn(async move {
            let artists = details::track_artists(&api, &uri).await.unwrap_or_default();
            Event::TrackArtists { uri, artists }.into()
        }),
        Command::Radio(seed) => ctx.tasks.spawn_keyed("radio", radio(api, seed)),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

async fn credits(api: Api, uri: String) -> crate::worker::Event {
    let result = details::credits(&api, &uri)
        .await
        .map_err(|error| failure(&error, "Couldn’t load the credits. Try again."));
    Event::Credits { uri, result }.into()
}

async fn cover_color(api: Api, uri: String) -> crate::worker::Event {
    let color = details::cover_color(&api, &uri).await.ok().flatten();
    Event::CoverColor { uri, color }.into()
}

async fn radio(api: Api, seed: String) -> crate::worker::Event {
    let result = details::radio(&api, &seed).await.map_err(|error| {
        if error.downcast_ref::<NoRadio>().is_some() {
            error.to_string()
        } else {
            "Couldn’t open the radio. Try again.".to_owned()
        }
    });
    Event::Radio { seed, result }.into()
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
