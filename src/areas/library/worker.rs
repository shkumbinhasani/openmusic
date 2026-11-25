use spotify_client::{
    Api,
    catalog::Playlist,
    dealer::Message,
    library::{self, LibraryError, LibraryPage, LibraryQuery, LikedPage, PinResult, Rootlist},
};

use super::{cache, state::LibraryPrefs};
use crate::worker::{self, Ctx, Mutation, User};

pub enum Command {
    Playlists,
    Library {
        query: LibraryQuery,
        offset: u32,
        request: u64,
    },
    Folder {
        uri: String,
        sort: Option<String>,
        offset: u32,
        request: u64,
    },
    Pin {
        uri: String,
        pinned: bool,
    },
    Liked {
        offset: u32,
        request: u64,
    },
}

pub enum Event {
    Playlists(Vec<Playlist>),
    Rootlist(Result<Box<Rootlist>, String>),
    Library {
        request: u64,
        query: LibraryQuery,
        offset: u32,
        result: Result<LibraryPage, String>,
    },
    Folder {
        request: u64,
        uri: String,
        offset: u32,
        result: Result<LibraryPage, String>,
    },
    Pinned {
        uri: String,
        pinned: bool,
        result: Result<PinResult, String>,
    },
    Liked {
        request: u64,
        offset: u32,
        result: Result<LikedPage, String>,
    },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Library {
                result: Ok(page), ..
            }
            | Self::Folder {
                result: Ok(page), ..
            } => page
                .items
                .iter()
                .filter_map(|item| item.image.clone())
                .collect(),
            Self::Liked {
                result: Ok(page), ..
            } => page
                .tracks
                .iter()
                .filter_map(|track| track.cover.clone())
                .collect(),
            _ => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    let (api, user) = (ctx.api.clone(), ctx.user.clone());
    match command {
        Command::Playlists => ctx
            .tasks
            .spawn_keyed("library-rootlist", rootlist(api, user)),
        Command::Library {
            query,
            offset,
            request,
        } => ctx
            .tasks
            .spawn_keyed("library-page", library(api, user, query, offset, request)),
        Command::Folder {
            uri,
            sort,
            offset,
            request,
        } => ctx.tasks.spawn_keyed(
            "library-folder",
            folder(api, user, uri, sort, offset, request),
        ),
        Command::Pin { uri, pinned } => {
            ctx.writes
                .push(Mutation::new(vec![format!("pin:{uri}")], async move {
                    let result = match user.name(&api).await {
                        Ok(name) => library::pin(&api, &name, &uri, pinned)
                            .await
                            .map_err(|error| error.to_string()),
                        Err(error) => Err(error.to_string()),
                    };
                    Ok(Event::Pinned {
                        uri,
                        pinned,
                        result,
                    }
                    .into())
                }))
        }
        Command::Liked { offset, request } => ctx.tasks.spawn_keyed("library-liked", async move {
            let result = library::liked(&api, offset, library::LIKED_PAGE_SIZE)
                .await
                .map_err(|error| error.to_string());
            Event::Liked {
                request,
                offset,
                result,
            }
            .into()
        }),
    }
}

pub fn on_push(_message: &Message, _ctx: &mut Ctx) {}

pub fn apply_settings(_prefs: &LibraryPrefs, _ctx: &mut Ctx) {}

async fn rootlist(api: Api, user: User) -> worker::Event {
    let result = match user.name(&api).await {
        Ok(name) => library::rootlist(&api, &name)
            .await
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };
    if let Ok(list) = &result {
        worker::cache::save_playlists(&list.playlists());
    }
    Event::Rootlist(result.map(Box::new)).into()
}

async fn fetch(
    api: &Api,
    user: &User,
    query: &LibraryQuery,
    offset: u32,
) -> anyhow::Result<LibraryPage> {
    match library::page(api, query, offset).await {
        Err(error) if library::wants_fallback(&error) => {
            let Ok(name) = user.name(api).await else {
                return Err(error);
            };
            match library::rootlist(api, &name).await {
                Ok(list) => Ok(library::fallback_page(&list, query)),
                Err(_) => Err(error),
            }
        }
        result => result,
    }
}

fn invalid(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<LibraryError>(),
        Some(LibraryError::InvalidQuery(_))
    )
}

async fn library(
    api: Api,
    user: User,
    query: LibraryQuery,
    offset: u32,
    request: u64,
) -> worker::Event {
    let (query, offset, result) = match fetch(&api, &user, &query, offset).await {
        Err(error) if invalid(&error) => {
            let reset = LibraryQuery {
                expanded: query.expanded,
                ..LibraryQuery::default()
            };
            let result = fetch(&api, &user, &reset, 0).await;
            (reset, 0, result)
        }
        result => (query, offset, result),
    };
    if offset == 0
        && let Ok(page) = &result
    {
        cache::save(&query, page);
    }
    Event::Library {
        request,
        query,
        offset,
        result: result.map_err(|error| error.to_string()),
    }
    .into()
}

async fn folder(
    api: Api,
    user: User,
    uri: String,
    sort: Option<String>,
    offset: u32,
    request: u64,
) -> worker::Event {
    let mut query = LibraryQuery {
        folder: Some(uri.clone()),
        sort,
        ..LibraryQuery::default()
    };
    let mut result = fetch(&api, &user, &query, offset).await;
    if result.as_ref().is_err_and(invalid) {
        query.sort = None;
        result = fetch(&api, &user, &query, offset).await;
    }
    Event::Folder {
        request,
        uri,
        offset,
        result: result.map_err(|error| error.to_string()),
    }
    .into()
}
