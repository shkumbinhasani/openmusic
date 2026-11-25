use spotify_client::{
    Api, Track,
    catalog::{self, Playlist},
    dealer::Message,
    library::{
        self, LibraryError, LibraryPage, LibraryPush, LibraryQuery, LikedPage, PinResult,
        PlayedContext, RecentEntry, RecentsPage, Rootlist,
    },
};

use super::{cache, state::LibraryPrefs};
use crate::worker::{self, Ctx, Mutation, User};

const LIKED_FALLBACK: usize = 1000;

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
    Grid {
        query: LibraryQuery,
        offset: u32,
        request: u64,
    },
    Recents {
        offset: u32,
        request: u64,
    },
    Played {
        request: u64,
    },
    PlayedContext {
        uri: String,
        at: i64,
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
    Grid {
        request: u64,
        query: LibraryQuery,
        offset: u32,
        result: Result<LibraryPage, String>,
    },
    Recents {
        request: u64,
        offset: u32,
        result: Result<RecentsPage, String>,
    },
    Played {
        request: u64,
        result: Result<Vec<RecentEntry>, String>,
    },
    PlayedContext {
        uri: String,
        entry: Option<RecentEntry>,
    },
    Pushed(LibraryPush),
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Library {
                result: Ok(page), ..
            }
            | Self::Folder {
                result: Ok(page), ..
            }
            | Self::Grid {
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
            Self::Recents {
                result: Ok(page), ..
            } => page
                .entries
                .iter()
                .filter_map(|entry| entry.image.clone())
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
        Command::Liked { offset, request } => ctx
            .tasks
            .spawn_keyed("library-liked", liked(api, offset, request)),
        Command::Grid {
            query,
            offset,
            request,
        } => ctx
            .tasks
            .spawn_keyed("library-grid", grid(api, user, query, offset, request)),
        Command::Recents { offset, request } => ctx
            .tasks
            .spawn_keyed("library-recents", recents(api, user, offset, request)),
        Command::Played { request } => ctx.tasks.spawn_keyed("library-played", async move {
            let result = match user.name(&api).await {
                Ok(name) => library::recently_played(&api, &name)
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            Event::Played { request, result }.into()
        }),
        Command::PlayedContext { uri, at } => {
            ctx.tasks.spawn_keyed("library-played-context", async move {
                let context = PlayedContext {
                    uri: library::context_uri(&uri),
                    played_at: at,
                };
                let entry = library::decorate(&api, &[context])
                    .await
                    .ok()
                    .and_then(|entries| entries.into_iter().next());
                Event::PlayedContext { uri, entry }.into()
            })
        }
    }
}

async fn liked(api: Api, offset: u32, request: u64) -> worker::Event {
    let result = match library::liked(&api, offset, library::LIKED_PAGE_SIZE).await {
        Err(error) if offset == 0 && library::wants_fallback(&error) => {
            catalog::liked(&api, LIKED_FALLBACK)
                .await
                .map(liked_fallback)
                .map_err(|_| error)
        }
        result => result,
    };
    Event::Liked {
        request,
        offset,
        result: result.map_err(|error| error.to_string()),
    }
    .into()
}

pub(super) fn liked_fallback(tracks: Vec<Track>) -> LikedPage {
    let count = tracks.len() as u32;
    LikedPage {
        tracks,
        unplayable: Vec::new(),
        total: count,
        offset: 0,
        rows: count,
    }
}

async fn recents(api: Api, user: User, offset: u32, request: u64) -> worker::Event {
    let result = match library::recents(&api, offset, library::RECENTS_PAGE_SIZE).await {
        Err(error) if offset == 0 => match user.name(&api).await {
            Ok(name) => library::recently_played(&api, &name)
                .await
                .map(library::fallback_recents)
                .map_err(|_| error),
            Err(_) => Err(error),
        },
        result => result,
    };
    Event::Recents {
        request,
        offset,
        result: result.map_err(|error| error.to_string()),
    }
    .into()
}

pub fn on_push(message: &Message, ctx: &mut Ctx) {
    if let Some(push) = library::library_push(message) {
        ctx.publish(Event::Pushed(push));
    }
}

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

async fn grid(
    api: Api,
    user: User,
    query: LibraryQuery,
    offset: u32,
    request: u64,
) -> worker::Event {
    let (query, offset, result) = match fetch(&api, &user, &query, offset).await {
        Err(error) if invalid(&error) => {
            let reset = LibraryQuery::default();
            let result = fetch(&api, &user, &reset, 0).await;
            (reset, 0, result)
        }
        result => (query, offset, result),
    };
    Event::Grid {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liked_fallback_is_one_whole_page() {
        let track = |uri: &str| Track {
            uri: uri.into(),
            ..Track::default()
        };
        let page = liked_fallback(vec![
            track("spotify:track:0000000000000000000001"),
            track("spotify:track:0000000000000000000002"),
        ]);
        assert_eq!((page.total, page.rows, page.offset), (2, 2, 0));
        assert!(page.unplayable.is_empty());
    }
}
