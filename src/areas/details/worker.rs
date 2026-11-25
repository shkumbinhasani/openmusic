use spotify_client::{
    Api,
    catalog::Link,
    dealer::Message,
    details::{self, Ban, BanProvider, Credits, NoRadio, SetPush, Unavailable, WriteFormat},
    pathfinder::PathfinderError,
};

use crate::worker::{Ctx, Mutation, User};

pub enum Command {
    Credits(String),
    CoverColor(String),
    LoadSets,
    Exclude {
        uri: String,
        excluded: bool,
    },
    Hide {
        ban: Ban,
        provider: BanProvider,
        hidden: bool,
    },
    Ban {
        uri: String,
        banned: bool,
    },
    Editable(String),
    TrackArtists(String),
    Radio(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub excluded: Vec<String>,
    pub hidden: Vec<Ban>,
    pub banned: Vec<String>,
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
    Sets(Result<Snapshot, String>),
    Pushed(SetPush),
    Excluded {
        uri: String,
        excluded: bool,
        result: Result<(), String>,
    },
    Hidden {
        ban: Ban,
        hidden: bool,
        result: Result<WriteFormat, String>,
    },
    Banned {
        uri: String,
        banned: bool,
        result: Result<(), String>,
    },
    Editable {
        uri: String,
        editable: Option<bool>,
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
            | Self::Sets(_)
            | Self::Pushed(_)
            | Self::Excluded { .. }
            | Self::Hidden { .. }
            | Self::Banned { .. }
            | Self::Editable { .. }
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
        Command::LoadSets => ctx
            .tasks
            .spawn_keyed("sets", load_sets(api, ctx.user.clone())),
        Command::Exclude { uri, excluded } => {
            let user = ctx.user.clone();
            let key = format!("ignoreinrecs {uri}");
            ctx.writes.push(Mutation::new(vec![key], async move {
                let result = async {
                    let name = user.name(&api).await?;
                    details::set_excluded(&api, &name, &uri, excluded).await
                }
                .await
                .map_err(|_| "Couldn’t update your taste profile. Try again.".to_owned());
                Ok(Event::Excluded {
                    uri,
                    excluded,
                    result,
                }
                .into())
            }));
        }
        Command::Hide {
            ban,
            provider,
            hidden,
        } => {
            let user = ctx.user.clone();
            let key = format!("playcontextbans {} {}", ban.context, ban.track);
            ctx.writes.push(Mutation::new(vec![key], async move {
                let result = async {
                    let name = user.name(&api).await?;
                    details::set_banned(&api, &name, &ban, provider, hidden).await
                }
                .await
                .map_err(|_| {
                    if hidden {
                        "Couldn’t hide this song. Try again.".to_owned()
                    } else {
                        "Couldn’t show this song again. Try again.".to_owned()
                    }
                });
                Ok(Event::Hidden {
                    ban,
                    hidden,
                    result,
                }
                .into())
            }));
        }
        Command::Ban { uri, banned } => {
            let user = ctx.user.clone();
            let key = format!("artistban {uri}");
            ctx.writes.push(Mutation::new(vec![key], async move {
                let result = async {
                    let name = user.name(&api).await?;
                    details::set_artist_banned(&api, &name, &uri, banned).await
                }
                .await
                .map_err(|_| ban_failure(banned).to_owned());
                Ok(Event::Banned {
                    uri,
                    banned,
                    result,
                }
                .into())
            }));
        }
        Command::Editable(uri) => ctx.tasks.spawn(async move {
            let editable = details::playlist_editable(&api, &uri).await.ok();
            Event::Editable { uri, editable }.into()
        }),
        Command::TrackArtists(uri) => ctx.tasks.spawn(async move {
            let artists = details::track_artists(&api, &uri).await.unwrap_or_default();
            Event::TrackArtists { uri, artists }.into()
        }),
        Command::Radio(seed) => ctx.tasks.spawn_keyed("radio", radio(api, seed)),
    }
}

pub fn on_push(message: &Message, ctx: &mut Ctx) {
    if let Some(push) = details::set_push(message) {
        ctx.publish(Event::Pushed(push));
    }
}

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

async fn load_sets(api: Api, user: User) -> crate::worker::Event {
    let result = async {
        let name = user.name(&api).await?;
        let excluded = details::excluded(&api, &name).await?;
        let hidden = details::bans(&api, &name).await?;
        let banned = details::banned_artists(&api, &name).await?;
        anyhow::Ok(Snapshot {
            excluded,
            hidden,
            banned,
        })
    }
    .await
    .map_err(|error| error.to_string());
    Event::Sets(result).into()
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

fn ban_failure(banned: bool) -> &'static str {
    if banned {
        "Couldn’t stop this artist from playing. Try again."
    } else {
        "Couldn’t allow this artist to play. Try again."
    }
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
