use spotify_client::{
    Api, Track,
    api::HttpStatus,
    catalog::{self, Collection},
    dealer::Message,
    pathfinder::PathfinderError,
    playlist::{self, Details, ItemPosition, Level, Permissions, Position, Rootlist},
};

use crate::worker::{Ctx, Mutation, User};

pub enum Cover {
    Keep,
    Set(Vec<u8>),
    Remove,
}

pub enum Write {
    Create {
        name: Option<String>,
        folder: Option<String>,
        tracks: Vec<String>,
        edit: bool,
    },
    Details {
        uri: String,
        details: Details,
        cover: Cover,
    },
    Follow {
        uri: String,
        follow: bool,
    },
    Private {
        uri: String,
        private: bool,
    },
    Profile {
        uri: String,
        public: bool,
    },
    Invite {
        uri: String,
    },
    Member {
        uri: String,
        username: String,
        level: Level,
    },
    RemoveMember {
        uri: String,
        username: String,
    },
    Leave {
        uri: String,
    },
    Add {
        uri: String,
        uris: Vec<String>,
        position: ItemPosition,
    },
    Remove {
        uri: String,
        uids: Vec<String>,
    },
    Move {
        uri: String,
        uids: Vec<String>,
        position: ItemPosition,
    },
}

impl Write {
    fn affects(&self) -> (Option<String>, bool) {
        match self {
            Self::Create { .. } | Self::Follow { .. } | Self::Profile { .. } => (None, true),
            Self::Invite { .. } => (None, false),
            Self::Details { uri, .. } | Self::Private { uri, .. } | Self::Leave { uri } => {
                (Some(uri.clone()), true)
            }
            Self::Member { uri, .. }
            | Self::RemoveMember { uri, .. }
            | Self::Add { uri, .. }
            | Self::Remove { uri, .. }
            | Self::Move { uri, .. } => (Some(uri.clone()), false),
        }
    }

    fn failure(&self) -> &'static str {
        match self {
            Self::Create { .. } => "Couldn’t create the playlist. Try again.",
            Self::Details { .. } => "Failed to save playlist changes. Please try again.",
            Self::Follow { follow: true, .. } => "Couldn’t add this to Your Library. Try again.",
            Self::Follow { follow: false, .. } => {
                "Couldn’t remove this from Your Library. Try again."
            }
            Self::Private { .. } => "Couldn’t change who can see this playlist. Try again.",
            Self::Profile { .. } => "Couldn’t change your profile. Try again.",
            Self::Invite { .. } => "Couldn’t create an invite link. Try again.",
            Self::Member { .. } | Self::RemoveMember { .. } => {
                "Couldn’t change the collaborators. Try again."
            }
            Self::Leave { .. } => "Couldn’t leave the playlist. Try again.",
            Self::Add { .. } => "Couldn’t add to the playlist. Try again.",
            Self::Remove { .. } => "Couldn’t remove from the playlist. Try again.",
            Self::Move { .. } => "Couldn’t move the songs. Try again.",
        }
    }
}

pub enum Command {
    Collection(String),
    Reload { uri: String, upto: usize },
    More { uri: String, from: usize },
    Rootlist,
    Permissions(String),
    Base(String),
    Write { write: Write, done: Option<String> },
}

pub enum Event {
    Collection {
        uri: String,
        result: Result<Box<Collection>, String>,
    },
    More {
        uri: String,
        from: usize,
        result: Result<(Vec<Track>, Option<usize>), String>,
    },
    Rootlist(Option<(String, Rootlist)>),
    Permissions {
        uri: String,
        permissions: Permissions,
    },
    Base {
        uri: String,
        level: Option<Level>,
    },
    Created {
        uri: String,
        edit: bool,
    },
    Invited {
        link: String,
    },
    Changed {
        uri: Option<String>,
        rootlist: bool,
        done: Option<String>,
    },
    Failed {
        uri: Option<String>,
        rootlist: bool,
        message: String,
    },
}

fn track_covers(tracks: &[Track]) -> impl Iterator<Item = String> + '_ {
    tracks.iter().filter_map(|track| track.cover.clone())
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
                .chain(track_covers(&collection.tracks))
                .collect(),
            Self::More {
                result: Ok((tracks, _)),
                ..
            } => track_covers(tracks).collect(),
            Self::Permissions { permissions, .. } => permissions
                .members
                .iter()
                .filter_map(|member| member.image.clone())
                .collect(),
            _ => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    let api = ctx.api.clone();
    match command {
        Command::Collection(uri) => ctx.tasks.spawn(collection(api, uri, playlist::PAGE)),
        Command::Reload { uri, upto } => ctx
            .tasks
            .spawn_keyed(format!("playlist-reload {uri}"), collection(api, uri, upto)),
        Command::More { uri, from } => ctx.tasks.spawn_keyed("playlist-more", more(api, uri, from)),
        Command::Rootlist => ctx
            .tasks
            .spawn_keyed("rootlist", rootlist(api, ctx.user.clone())),
        Command::Permissions(uri) => ctx
            .tasks
            .spawn_keyed(format!("playlist-permissions {uri}"), permissions(api, uri)),
        Command::Base(uri) => ctx.tasks.spawn(base(api, uri)),
        Command::Write { write, done } => {
            let user = ctx.user.clone();
            ctx.writes.push(Mutation::new(Vec::new(), async move {
                Ok(run(api, user, write, done).await)
            }));
        }
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

async fn collection(api: Api, uri: String, upto: usize) -> crate::worker::Event {
    let result = if playlist::playlist_id(&uri).is_some() {
        playlist::collection(&api, &uri, upto).await
    } else {
        catalog::collection(&api, &uri).await
    };
    Event::Collection {
        uri,
        result: result
            .map(Box::new)
            .map_err(|_| "Couldn’t load this. Try again.".to_owned()),
    }
    .into()
}

async fn more(api: Api, uri: String, from: usize) -> crate::worker::Event {
    let result = playlist::more(&api, &uri, from)
        .await
        .map_err(|_| "Couldn’t load more songs.".to_owned());
    Event::More { uri, from, result }.into()
}

async fn rootlist(api: Api, user: User) -> crate::worker::Event {
    let loaded = async {
        let user = user.name(&api).await?;
        let rootlist = playlist::rootlist(&api, &user).await?;
        anyhow::Ok((user, rootlist))
    };
    Event::Rootlist(loaded.await.ok()).into()
}

async fn permissions(api: Api, uri: String) -> crate::worker::Event {
    match playlist::permissions(&api, &uri).await {
        Ok(permissions) => Event::Permissions { uri, permissions }.into(),
        Err(_) => Event::Base { uri, level: None }.into(),
    }
}

async fn base(api: Api, uri: String) -> crate::worker::Event {
    let level = playlist::base_permission(&api, &uri).await.ok().flatten();
    Event::Base { uri, level }.into()
}

pub fn readable(error: &anyhow::Error, fallback: &str) -> String {
    if let Some(PathfinderError::StaleHash(_)) = error.downcast_ref::<PathfinderError>() {
        return error.to_string();
    }
    let plain = error.chain().count() == 1
        && error.downcast_ref::<HttpStatus>().is_none()
        && error.downcast_ref::<PathfinderError>().is_none();
    if plain {
        error.to_string()
    } else {
        fallback.to_owned()
    }
}

async fn run(api: Api, user: User, write: Write, done: Option<String>) -> crate::worker::Event {
    let (uri, rootlist) = write.affects();
    let fallback = write.failure();
    match perform(&api, &user, write).await {
        Ok(Some(event)) => event.into(),
        Ok(None) => Event::Changed {
            uri,
            rootlist,
            done,
        }
        .into(),
        Err(error) => Event::Failed {
            uri,
            rootlist,
            message: readable(&error, fallback),
        }
        .into(),
    }
}

async fn perform(api: &Api, user: &User, write: Write) -> anyhow::Result<Option<Event>> {
    match write {
        Write::Create {
            name,
            folder,
            tracks,
            edit,
        } => {
            let me = user.name(api).await?;
            let name = match name {
                Some(name) => name,
                None => playlist::rootlist(api, &me)
                    .await
                    .map(|list| list.default_name())
                    .unwrap_or_else(|_| "My Playlist".to_owned()),
            };
            let position = folder
                .as_deref()
                .and_then(playlist::folder_hash)
                .map_or(Position::Start, Position::in_folder);
            let uri = playlist::create(api, &me, &name, &position).await?;
            if !tracks.is_empty() {
                playlist::add_items(api, &uri, &tracks, &ItemPosition::Top).await?;
            }
            Ok(Some(Event::Created { uri, edit }))
        }
        Write::Details {
            uri,
            mut details,
            cover,
        } => {
            match cover {
                Cover::Keep => {}
                Cover::Set(jpeg) => {
                    let picture = playlist::cover_picture(api, &uri, jpeg)
                        .await
                        .map_err(|_| {
                            anyhow::anyhow!("Failed to upload image. Please try again.")
                        })?;
                    details.picture = Some(Some(picture));
                }
                Cover::Remove => details.picture = Some(None),
            }
            playlist::update_details(api, &uri, &details).await?;
            Ok(None)
        }
        Write::Follow { uri, follow } => {
            let me = user.name(api).await?;
            playlist::follow(api, &me, &uri, follow).await?;
            Ok(None)
        }
        Write::Private { uri, private } => {
            playlist::set_private(api, &uri, private).await?;
            Ok(None)
        }
        Write::Profile { uri, public } => {
            let me = user.name(api).await?;
            playlist::set_on_profile(api, &me, &uri, public).await?;
            Ok(None)
        }
        Write::Invite { uri } => {
            let link = playlist::invite_link(api, &uri).await?;
            Ok(Some(Event::Invited { link }))
        }
        Write::Member {
            uri,
            username,
            level,
        } => {
            playlist::set_member(api, &uri, &username, level).await?;
            Ok(None)
        }
        Write::RemoveMember { uri, username } => {
            playlist::remove_member(api, &uri, &username).await?;
            Ok(None)
        }
        Write::Leave { uri } => {
            let me = user.name(api).await?;
            playlist::remove_member(api, &uri, &me).await?;
            Ok(None)
        }
        Write::Add {
            uri,
            uris,
            position,
        } => {
            playlist::add_items(api, &uri, &uris, &position).await?;
            Ok(None)
        }
        Write::Remove { uri, uids } => {
            playlist::remove_items(api, &uri, &uids).await?;
            Ok(None)
        }
        Write::Move {
            uri,
            uids,
            position,
        } => {
            playlist::move_items(api, &uri, &uids, &position).await?;
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_only_messages_written_for_people() {
        let plain = anyhow::anyhow!("Playlist name is required.");
        assert_eq!(readable(&plain, "fallback"), "Playlist name is required.");
        let status = anyhow::Error::new(HttpStatus(reqwest::StatusCode::BAD_GATEWAY));
        assert_eq!(readable(&status, "fallback"), "fallback");
        let wrapped = anyhow::Error::new(std::io::Error::other("socket"))
            .context("Could not reach Spotify (/x).");
        assert_eq!(readable(&wrapped, "fallback"), "fallback");
        let graphql = anyhow::Error::new(PathfinderError::GraphQl(vec!["internal".into()]));
        assert_eq!(readable(&graphql, "fallback"), "fallback");
        let stale = anyhow::Error::new(PathfinderError::StaleHash("addToPlaylist"));
        assert!(readable(&stale, "fallback").contains("addToPlaylist"));
    }

    #[test]
    fn reloads_what_each_write_touches() {
        let uri = "spotify:playlist:x".to_owned();
        let follow = Write::Follow {
            uri: uri.clone(),
            follow: true,
        };
        assert_eq!(follow.affects(), (None, true));
        let remove = Write::Remove {
            uri: uri.clone(),
            uids: vec!["u".into()],
        };
        assert_eq!(remove.affects(), (Some(uri.clone()), false));
        let details = Write::Details {
            uri: uri.clone(),
            details: Details::default(),
            cover: Cover::Keep,
        };
        assert_eq!(details.affects(), (Some(uri), true));
        assert_eq!(Write::Invite { uri: "x".into() }.affects(), (None, false));
    }
}
