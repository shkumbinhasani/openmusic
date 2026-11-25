use std::collections::HashSet;

use spotify_client::{
    Api, Track,
    api::HttpStatus,
    catalog::{self, Collection},
    dealer::Message,
    pathfinder::PathfinderError,
    playlist::{
        self, Details, EditTarget, ItemPosition, Level, Permissions, Position, RootPlace, Rootlist,
    },
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
        open: bool,
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
    Curate {
        items: Vec<String>,
        changes: Vec<(String, bool)>,
    },
    CreateFolder {
        name: Option<String>,
        parent: Option<String>,
        playlists: Vec<String>,
        rename: bool,
    },
    RenameFolder {
        uri: String,
        name: String,
    },
    DeleteFolder {
        uri: String,
    },
    MoveFolder {
        uri: String,
        to: RootPlace,
    },
    MovePlaylists {
        uris: Vec<String>,
        to: RootPlace,
    },
}

impl Write {
    fn affects(&self) -> (Option<String>, bool) {
        match self {
            Self::Create { .. }
            | Self::Follow { .. }
            | Self::Profile { .. }
            | Self::CreateFolder { .. }
            | Self::RenameFolder { .. }
            | Self::DeleteFolder { .. }
            | Self::MoveFolder { .. }
            | Self::MovePlaylists { .. } => (None, true),
            Self::Invite { .. } | Self::Curate { .. } => (None, false),
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

    fn rows(&self) -> bool {
        matches!(self, Self::Remove { .. } | Self::Move { .. })
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
            Self::Curate { .. } => "Couldn’t save your changes. Try again.",
            Self::CreateFolder { .. } => "Couldn’t create the folder. Try again.",
            Self::RenameFolder { .. } => "Couldn’t rename the folder. Try again.",
            Self::DeleteFolder { .. } => "Couldn’t delete the folder. Try again.",
            Self::MoveFolder { .. } | Self::MovePlaylists { .. } => {
                "Couldn’t change the order of Your Library. Try again."
            }
        }
    }
}

pub struct Ready {
    pub target: String,
    pub name: String,
    pub uris: Vec<String>,
    pub duplicates: Vec<String>,
    pub position: ItemPosition,
}

pub enum Command {
    Collection(String),
    Reload {
        uri: String,
        upto: usize,
    },
    More {
        uri: String,
        from: usize,
    },
    Rootlist,
    Permissions(String),
    Base(String),
    Editable(Vec<String>),
    PrepareAdd {
        target: String,
        name: String,
        sources: Vec<String>,
        position: ItemPosition,
        known: Option<HashSet<String>>,
    },
    Write {
        write: Write,
        done: Option<String>,
    },
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
        name: String,
        open: bool,
        edit: bool,
    },
    Editable {
        uris: Vec<String>,
        result: Result<Vec<EditTarget>, String>,
    },
    AddReady(Ready),
    Curated {
        items: Vec<String>,
        changes: Vec<(String, bool)>,
        done: Option<String>,
    },
    FolderCreated {
        uri: String,
        name: String,
        rename: bool,
    },
    Invited {
        link: String,
    },
    Changed {
        uri: Option<String>,
        rootlist: bool,
        rows: bool,
        done: Option<String>,
    },
    Failed {
        uri: Option<String>,
        rootlist: bool,
        rows: bool,
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
            Self::Editable {
                result: Ok(targets),
                ..
            } => targets
                .iter()
                .filter_map(|target| target.cover.clone())
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
        Command::Editable(uris) => ctx.tasks.spawn_keyed(
            format!("playlist-editable {}", uris.join(",")),
            editable(api, uris),
        ),
        Command::PrepareAdd {
            target,
            name,
            sources,
            position,
            known,
        } => ctx.tasks.spawn(prepare_add(
            api,
            Adding {
                target,
                name,
                sources,
                position,
            },
            known,
        )),
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

async fn editable(api: Api, uris: Vec<String>) -> crate::worker::Event {
    let result = playlist::editable(&api, &uris)
        .await
        .map_err(|error| readable(&error, "Couldn’t load your playlists."));
    Event::Editable { uris, result }.into()
}

struct Adding {
    target: String,
    name: String,
    sources: Vec<String>,
    position: ItemPosition,
}

async fn prepare_add(
    api: Api,
    adding: Adding,
    known: Option<HashSet<String>>,
) -> crate::worker::Event {
    let checked = async {
        let uris = playlist::expand(&api, &adding.sources).await?;
        if uris.is_empty() {
            return anyhow::Ok((uris, Vec::new()));
        }
        let existing = match known {
            Some(existing) => existing,
            None => playlist::item_uris(&api, &adding.target, playlist::CONTENTS_LIMIT)
                .await?
                .into_iter()
                .collect(),
        };
        let (duplicates, _) = playlist::split_duplicates(&existing, &uris);
        Ok((uris, duplicates))
    };
    match checked.await {
        Ok((uris, duplicates)) => Event::AddReady(Ready {
            target: adding.target,
            name: adding.name,
            uris,
            duplicates,
            position: adding.position,
        })
        .into(),
        Err(error) => Event::Failed {
            uri: None,
            rootlist: false,
            rows: false,
            message: readable(&error, "Couldn’t add to the playlist. Try again."),
        }
        .into(),
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
    let rows = write.rows();
    let fallback = write.failure();
    match perform(&api, &user, write, done.clone()).await {
        Ok(Some(event)) => event.into(),
        Ok(None) => Event::Changed {
            uri,
            rootlist,
            rows,
            done,
        }
        .into(),
        Err(error) => Event::Failed {
            uri,
            rootlist,
            rows,
            message: readable(&error, fallback),
        }
        .into(),
    }
}

fn hash_of(uri: &str) -> anyhow::Result<&str> {
    playlist::folder_hash(uri)
        .ok_or_else(|| anyhow::anyhow!("This folder is no longer in Your Library."))
}

async fn perform(
    api: &Api,
    user: &User,
    write: Write,
    done: Option<String>,
) -> anyhow::Result<Option<Event>> {
    match write {
        Write::Create {
            name,
            folder,
            tracks,
            open,
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
                let tracks = playlist::expand(api, &tracks).await?;
                playlist::add_items(api, &uri, &tracks, &ItemPosition::Top).await?;
            }
            Ok(Some(Event::Created {
                uri,
                name,
                open,
                edit,
            }))
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
        Write::Curate { items, changes } => {
            playlist::apply_curations(api, &items, &changes).await?;
            Ok(Some(Event::Curated {
                items,
                changes,
                done,
            }))
        }
        Write::CreateFolder {
            name,
            parent,
            playlists,
            rename,
        } => {
            let me = user.name(api).await?;
            let name = name.unwrap_or_else(|| playlist::DEFAULT_FOLDER_NAME.to_owned());
            let position = parent
                .as_deref()
                .and_then(playlist::folder_hash)
                .map_or(Position::Start, Position::in_folder);
            let uri = playlist::create_folder(api, &me, &name, &position).await?;
            if !playlists.is_empty() {
                playlist::move_playlists_to(api, &me, &playlists, &RootPlace::Into(uri.clone()))
                    .await?;
            }
            Ok(Some(Event::FolderCreated { uri, name, rename }))
        }
        Write::RenameFolder { uri, name } => {
            let me = user.name(api).await?;
            playlist::rename_folder(api, &me, hash_of(&uri)?, &name).await?;
            Ok(None)
        }
        Write::DeleteFolder { uri } => {
            let me = user.name(api).await?;
            playlist::delete_folder(api, &me, hash_of(&uri)?).await?;
            Ok(None)
        }
        Write::MoveFolder { uri, to } => {
            let me = user.name(api).await?;
            playlist::move_folder_to(api, &me, hash_of(&uri)?, &to).await?;
            Ok(None)
        }
        Write::MovePlaylists { uris, to } => {
            let me = user.name(api).await?;
            playlist::move_playlists_to(api, &me, &uris, &to).await?;
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
        let moved = Write::Move {
            uri: "spotify:playlist:x".into(),
            uids: vec!["u".into()],
            position: ItemPosition::Top,
        };
        assert!(moved.rows() && remove.rows());
        assert!(!follow.rows());
        let folder = Write::MovePlaylists {
            uris: vec!["spotify:playlist:x".into()],
            to: RootPlace::Start,
        };
        assert_eq!(folder.affects(), (None, true));
        let curate = Write::Curate {
            items: vec!["spotify:track:t".into()],
            changes: vec![("spotify:playlist:x".into(), true)],
        };
        assert_eq!(curate.affects(), (None, false));
    }
}
