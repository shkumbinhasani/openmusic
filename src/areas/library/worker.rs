use std::sync::Arc;

use spotify_client::{
    Api, Track,
    catalog::{self, ContextItem, Playlist},
    dealer::Message,
    library::{
        self, LIKED_PAGE_SIZE, LIKED_SONGS, LibraryError, LibraryPage, LibraryPush, LibraryQuery,
        LikedPage, PinResult, PlayedContext, RecentEntry, RecentsPage, Rootlist,
    },
};

use super::{cache, state::LibraryPrefs};
use crate::worker::{self, Ctx, Mutation, User};

pub type LikedList = Arc<[ContextItem]>;

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
        fallback: Option<LikedList>,
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
        fallback: Option<LikedList>,
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
        Command::Liked {
            offset,
            request,
            fallback,
        } => ctx
            .tasks
            .spawn_keyed("library-liked", liked(api, offset, request, fallback)),
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

async fn liked(api: Api, offset: u32, request: u64, fallback: Option<LikedList>) -> worker::Event {
    let (result, fallback) = match fallback {
        Some(list) => (liked_from(&api, &list, offset).await, Some(list)),
        None => match library::liked(&api, offset, LIKED_PAGE_SIZE).await {
            Err(error) if offset == 0 && library::wants_fallback(&error) => {
                match catalog::context_uris(&api, LIKED_SONGS, usize::MAX).await {
                    Ok(items) => {
                        let list = LikedList::from(items);
                        let page = liked_from(&api, &list, 0).await.map_err(|_| error);
                        (page, Some(list))
                    }
                    Err(_) => (Err(error), None),
                }
            }
            result => (result, None),
        },
    };
    Event::Liked {
        request,
        offset,
        result: result.map_err(|error| error.to_string()),
        fallback,
    }
    .into()
}

fn liked_slice(list: &[ContextItem], offset: u32) -> &[ContextItem] {
    let start = (offset as usize).min(list.len());
    let end = (start + LIKED_PAGE_SIZE as usize).min(list.len());
    &list[start..end]
}

async fn liked_from(api: &Api, list: &[ContextItem], offset: u32) -> anyhow::Result<LikedPage> {
    let slice = liked_slice(list, offset);
    let tracks = catalog::item_tracks(api, slice).await?;
    Ok(liked_fallback(tracks, list.len(), offset, slice.len()))
}

fn liked_fallback(tracks: Vec<Track>, total: usize, offset: u32, rows: usize) -> LikedPage {
    LikedPage {
        tracks,
        unplayable: Vec::new(),
        total: total as u32,
        offset,
        rows: rows as u32,
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
    fn liked_fallback_pages_through_the_whole_collection() {
        let list: Vec<ContextItem> = (0..120)
            .map(|index| ContextItem {
                uri: format!("spotify:track:{index:022}"),
                ..ContextItem::default()
            })
            .collect();
        assert_eq!(liked_slice(&list, 0).len(), 50);
        assert_eq!(liked_slice(&list, 100).len(), 20);
        assert_eq!(liked_slice(&list, 100)[0].uri, list[100].uri);
        assert!(liked_slice(&list, 500).is_empty());
        let track = Track {
            uri: list[100].uri.clone(),
            ..Track::default()
        };
        let page = liked_fallback(vec![track], list.len(), 100, 20);
        assert_eq!((page.total, page.rows, page.offset), (120, 20, 100));
        assert!(page.unplayable.is_empty());
    }
}
