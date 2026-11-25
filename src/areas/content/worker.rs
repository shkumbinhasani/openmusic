use std::time::Duration;

use spotify_client::{
    Api,
    canvas::{self, Canvas},
    catalog,
    catalog::Lyrics,
    dealer::Message,
    entity::Card,
    podcast::{
        self, Episode, EpisodePage, Show, ShowKind, Transcript, Unavailable, similar, transcript,
        your_episodes::{self, Entry},
    },
};

use super::state::ContentPrefs;
use crate::worker::{Ctx, Mutation, User};

pub enum Command {
    Lyrics(String),
    Transcript {
        uri: String,
        known: Option<Vec<Transcript>>,
    },
    Show(String),
    ShowPage {
        uri: String,
        kind: ShowKind,
        offset: u32,
    },
    Episode(String),
    Refresh {
        uris: Vec<String>,
        delay_ms: u64,
    },
    MarkPlayed {
        uris: Vec<String>,
        played: bool,
    },
    YourEpisodes,
    DecorateEpisodes(Vec<String>),
    Similar {
        uri: String,
        kind: ShowKind,
    },
    Locate(String),
    Canvas(String),
}

pub enum Event {
    Lyrics {
        uri: String,
        lyrics: Option<Lyrics>,
    },
    Show {
        uri: String,
        result: Result<Box<Show>, String>,
    },
    ShowPage {
        uri: String,
        offset: u32,
        result: Result<EpisodePage, String>,
    },
    Episode {
        uri: String,
        result: Result<Box<Episode>, String>,
    },
    Refreshed(Vec<Episode>),
    Marked {
        uris: Vec<String>,
        error: Option<String>,
    },
    YourEpisodes(Result<Vec<Entry>, String>),
    Decorated {
        uris: Vec<String>,
        result: Result<Vec<Option<Episode>>, String>,
    },
    Similar {
        uri: String,
        result: Result<Vec<Card>, String>,
    },
    Located {
        uri: String,
        result: Result<Box<Episode>, String>,
    },
    Canvas {
        uri: String,
        result: Result<Option<Canvas>, String>,
    },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Show {
                result: Ok(show), ..
            } => show
                .cover
                .iter()
                .cloned()
                .chain(
                    show.episodes
                        .iter()
                        .filter_map(|episode| episode.cover.clone()),
                )
                .collect(),
            Self::ShowPage {
                result: Ok(page), ..
            } => page
                .episodes
                .iter()
                .filter_map(|episode| episode.cover.clone())
                .collect(),
            Self::Episode {
                result: Ok(episode),
                ..
            } => episode.large_cover.iter().cloned().collect(),
            Self::Decorated {
                result: Ok(found), ..
            } => found
                .iter()
                .flatten()
                .filter_map(|episode| episode.cover.clone())
                .collect(),
            Self::Similar {
                result: Ok(cards), ..
            } => cards.iter().filter_map(|card| card.image.clone()).collect(),
            Self::Lyrics { .. }
            | Self::Show { .. }
            | Self::ShowPage { .. }
            | Self::Episode { .. }
            | Self::Refreshed(_)
            | Self::Marked { .. }
            | Self::YourEpisodes(_)
            | Self::Decorated { .. }
            | Self::Similar { .. }
            | Self::Located { .. }
            | Self::Canvas { .. } => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Lyrics(uri) => ctx
            .tasks
            .spawn_keyed("lyrics", lyrics(ctx.api.clone(), uri)),
        Command::Transcript { uri, known } => ctx
            .tasks
            .spawn_keyed("lyrics", read_transcript(ctx.api.clone(), uri, known)),
        Command::Show(uri) => ctx.tasks.spawn_keyed("show", show(ctx.api.clone(), uri)),
        Command::ShowPage { uri, kind, offset } => ctx
            .tasks
            .spawn_keyed("show-page", show_page(ctx.api.clone(), uri, kind, offset)),
        Command::Episode(uri) => ctx
            .tasks
            .spawn_keyed("episode", episode(ctx.api.clone(), uri)),
        Command::Refresh { uris, delay_ms } => {
            for uri in uris {
                ctx.tasks.spawn_keyed(
                    format!("played {uri}"),
                    refresh(ctx.api.clone(), uri, Duration::from_millis(delay_ms)),
                );
            }
        }
        Command::MarkPlayed { uris, played } => {
            let keys = uris.iter().map(|uri| format!("played {uri}")).collect();
            let run = mark(ctx.api.clone(), ctx.user.clone(), uris, played);
            ctx.writes
                .push(Mutation::new(keys, async move { Ok(run.await) }));
        }
        Command::YourEpisodes => ctx.tasks.spawn_keyed(
            "your-episodes",
            your_episode_list(ctx.api.clone(), ctx.user.clone()),
        ),
        Command::DecorateEpisodes(uris) => ctx
            .tasks
            .spawn_keyed("your-episodes-decorate", decorate(ctx.api.clone(), uris)),
        Command::Similar { uri, kind } => ctx
            .tasks
            .spawn_keyed("show-similar", similar_shows(ctx.api.clone(), uri, kind)),
        Command::Locate(uri) => ctx
            .tasks
            .spawn_keyed("locate", locate(ctx.api.clone(), uri)),
        Command::Canvas(uri) => ctx
            .tasks
            .spawn_keyed("canvas", find_canvas(ctx.api.clone(), uri)),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

pub fn apply_settings(_prefs: &ContentPrefs, _ctx: &mut Ctx) {}

fn failure(error: &anyhow::Error, what: &str) -> String {
    match error.downcast_ref::<Unavailable>() {
        Some(Unavailable::Restricted) => format!("This {what} isn’t available in your country."),
        Some(Unavailable::Missing) => format!("Spotify couldn’t find this {what}."),
        None => format!("Couldn’t load this {what}. Try again."),
    }
}

async fn lyrics(api: Api, uri: String) -> crate::worker::Event {
    let found = catalog::lyrics(&api, &uri).await.ok().flatten();
    Event::Lyrics { uri, lyrics: found }.into()
}

async fn read_transcript(
    api: Api,
    uri: String,
    known: Option<Vec<Transcript>>,
) -> crate::worker::Event {
    let found = transcript::read(&api, &uri, known).await.ok().flatten();
    Event::Lyrics { uri, lyrics: found }.into()
}

async fn show(api: Api, uri: String) -> crate::worker::Event {
    let result = podcast::show(&api, &uri)
        .await
        .map(Box::new)
        .map_err(|error| failure(&error, "podcast"));
    Event::Show { uri, result }.into()
}

async fn show_page(api: Api, uri: String, kind: ShowKind, offset: u32) -> crate::worker::Event {
    let result = podcast::page(&api, &uri, kind, offset, podcast::PAGE_SIZE)
        .await
        .map_err(|_| "Couldn’t load more episodes.".to_owned());
    Event::ShowPage {
        uri,
        offset,
        result,
    }
    .into()
}

async fn episode(api: Api, uri: String) -> crate::worker::Event {
    let result = podcast::episode(&api, &uri)
        .await
        .map(Box::new)
        .map_err(|error| failure(&error, "episode"));
    Event::Episode { uri, result }.into()
}

async fn find_canvas(api: Api, uri: String) -> crate::worker::Event {
    let result = canvas::canvas(&api, &uri)
        .await
        .map_err(|error| error.to_string());
    Event::Canvas { uri, result }.into()
}

async fn locate(api: Api, uri: String) -> crate::worker::Event {
    let result = podcast::episode(&api, &uri)
        .await
        .map(Box::new)
        .map_err(|error| match error.downcast_ref::<Unavailable>() {
            Some(_) => failure(&error, "episode"),
            None => "Couldn’t open this podcast. Try again.".to_owned(),
        });
    Event::Located { uri, result }.into()
}

async fn refresh(api: Api, uri: String, delay: Duration) -> crate::worker::Event {
    tokio::time::sleep(delay).await;
    let fresh = podcast::episode(&api, &uri).await.ok();
    Event::Refreshed(fresh.into_iter().collect()).into()
}

async fn mark(api: Api, user: User, uris: Vec<String>, played: bool) -> crate::worker::Event {
    let written = match user.name(&api).await {
        Ok(name) => podcast::mark_played(&api, &name, &uris, played).await,
        Err(error) => Err(error),
    };
    let error = written.err().map(|_| {
        if played {
            "Couldn’t mark this as finished. Try again.".to_owned()
        } else {
            "Couldn’t mark this as not started. Try again.".to_owned()
        }
    });
    Event::Marked { uris, error }.into()
}

async fn your_episode_list(api: Api, user: User) -> crate::worker::Event {
    let result = match user.name(&api).await {
        Ok(name) => your_episodes::entries(&api, &name).await,
        Err(error) => Err(error),
    };
    Event::YourEpisodes(result.map_err(|_| "Couldn’t load Your Episodes.".to_owned())).into()
}

async fn similar_shows(api: Api, uri: String, kind: ShowKind) -> crate::worker::Event {
    let result = similar::similar(&api, &uri, kind)
        .await
        .map_err(|error| error.to_string());
    Event::Similar { uri, result }.into()
}

async fn decorate(api: Api, uris: Vec<String>) -> crate::worker::Event {
    let result = your_episodes::decorate(&api, &uris)
        .await
        .map_err(|_| "Couldn’t load more episodes.".to_owned());
    Event::Decorated { uris, result }.into()
}
