use std::time::Duration;

use spotify_client::{
    Api, Track,
    account::{self, AccountInfo},
    api::HttpStatus,
    dealer::Message,
    social::{self, PlaylistRef, Profile, ProfileRef, TopContent},
};

use super::state::{AccountPrefs, ProfileListKind, chosen_language, system_language};
use crate::worker::{Ctx, cache};

const SECOND_LOOK: Duration = Duration::from_secs(5);

pub enum Command {
    Refresh,
    Language(Option<String>),
    CacheSize,
    ClearCache,
    Profile(String),
    ProfileList {
        uri: String,
        kind: ProfileListKind,
        offset: usize,
    },
}

pub struct ProfileView {
    pub profile: Profile,
    pub top: Option<TopContent>,
    pub followers: Vec<ProfileRef>,
    pub following: Vec<ProfileRef>,
}

impl ProfileView {
    pub fn images(&self) -> Vec<String> {
        let profile = &self.profile;
        let mut urls: Vec<String> = profile.image.iter().cloned().collect();
        urls.extend(playlist_images(&profile.playlists));
        urls.extend(people_images(&profile.recent_artists));
        urls.extend(people_images(&self.followers));
        urls.extend(people_images(&self.following));
        if let Some(top) = &self.top {
            urls.extend(people_images(&top.artists));
            urls.extend(track_images(&top.tracks));
        }
        urls
    }
}

pub enum Listing {
    People(Vec<ProfileRef>),
    Playlists { items: Vec<PlaylistRef>, total: u64 },
    Tracks(Vec<Track>),
}

impl Listing {
    pub fn images(&self) -> Vec<String> {
        match self {
            Self::People(people) => people_images(people).collect(),
            Self::Playlists { items, .. } => playlist_images(items).collect(),
            Self::Tracks(tracks) => track_images(tracks).collect(),
        }
    }

    pub fn count(&self) -> usize {
        match self {
            Self::People(people) => people.len(),
            Self::Playlists { items, .. } => items.len(),
            Self::Tracks(tracks) => tracks.len(),
        }
    }
}

pub fn people_images(people: &[ProfileRef]) -> impl Iterator<Item = String> + '_ {
    people.iter().filter_map(|person| person.image.clone())
}

pub fn playlist_images(playlists: &[PlaylistRef]) -> impl Iterator<Item = String> + '_ {
    playlists
        .iter()
        .filter_map(|playlist| playlist.image.clone())
}

pub fn track_images(tracks: &[Track]) -> impl Iterator<Item = String> + '_ {
    tracks.iter().filter_map(|track| track.cover.clone())
}

pub enum Event {
    Account(Result<AccountInfo, String>),
    Cache {
        bytes: u64,
        cleared: bool,
    },
    Profile {
        uri: String,
        result: Result<Box<ProfileView>, String>,
    },
    ProfileList {
        uri: String,
        kind: ProfileListKind,
        offset: usize,
        result: Result<Listing, String>,
    },
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Account(Ok(info)) => info.avatar.iter().cloned().collect(),
            Self::Profile {
                result: Ok(view), ..
            } => view.images(),
            Self::ProfileList {
                result: Ok(listing),
                ..
            } => listing.images(),
            Self::Account(Err(_))
            | Self::Cache { .. }
            | Self::Profile { .. }
            | Self::ProfileList { .. } => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Refresh => refresh(ctx),
        Command::Language(language) => apply_language(language.as_deref(), ctx),
        Command::CacheSize => ctx.tasks.spawn_keyed("cache-size", cache_size(false)),
        Command::ClearCache => ctx.tasks.spawn(cache_size(true)),
        Command::Profile(uri) => ctx
            .tasks
            .spawn_keyed("profile", profile_view(ctx.api.clone(), uri)),
        Command::ProfileList { uri, kind, offset } => ctx.tasks.spawn_keyed(
            "profile-list",
            profile_list(ctx.api.clone(), uri, kind, offset),
        ),
    }
}

pub fn on_push(message: &Message, ctx: &mut Ctx) {
    if account::attributes_changed(&message.uri) {
        refresh(ctx);
        ctx.tasks
            .spawn_keyed("account-later", fetch(ctx.api.clone(), Some(SECOND_LOOK)));
    }
}

pub fn apply_settings(prefs: &AccountPrefs, ctx: &mut Ctx) {
    apply_language(prefs.language.as_deref(), ctx);
}

fn apply_language(language: Option<&str>, ctx: &mut Ctx) {
    let language = chosen_language(language, system_language);
    if ctx.api.language() != language {
        ctx.api.set_language(language);
    }
}

fn refresh(ctx: &mut Ctx) {
    ctx.tasks
        .spawn_keyed("account", fetch(ctx.api.clone(), None));
}

async fn fetch(api: Api, delay: Option<Duration>) -> crate::worker::Event {
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
    let result = account::account(&api)
        .await
        .map_err(|_| "Couldn’t load your account.".to_owned());
    Event::Account(result).into()
}

async fn cache_size(clear: bool) -> crate::worker::Event {
    let bytes = tokio::task::spawn_blocking(move || {
        if clear {
            cache::clear_all();
        }
        cache::size()
    })
    .await
    .unwrap_or_default();
    Event::Cache {
        bytes,
        cleared: clear,
    }
    .into()
}

fn missing(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<HttpStatus>()
        .is_some_and(|status| status.0 == reqwest::StatusCode::NOT_FOUND)
}

async fn profile_view(api: Api, uri: String) -> crate::worker::Event {
    let result = load_profile(&api, &uri)
        .await
        .map(Box::new)
        .map_err(|error| {
            if missing(&error) {
                "This profile doesn’t exist.".to_owned()
            } else {
                "Couldn’t load this profile.".to_owned()
            }
        });
    Event::Profile { uri, result }.into()
}

async fn load_profile(api: &Api, uri: &str) -> anyhow::Result<ProfileView> {
    let profile = social::profile(api, uri).await?;
    let own = profile.is_current_user;
    let lists = own || profile.show_follows;
    let (top, followers, following) = tokio::join!(
        async {
            if own {
                social::top_content(api, social::TOP_PREVIEW_ARTISTS, social::TOP_PREVIEW_TRACKS)
                    .await
                    .ok()
            } else {
                None
            }
        },
        async {
            if lists {
                social::followers(api, uri).await.unwrap_or_default()
            } else {
                Vec::new()
            }
        },
        async {
            if lists {
                social::following(api, uri).await.unwrap_or_default()
            } else {
                Vec::new()
            }
        },
    );
    Ok(ProfileView {
        profile,
        top,
        followers,
        following,
    })
}

async fn profile_list(
    api: Api,
    uri: String,
    kind: ProfileListKind,
    offset: usize,
) -> crate::worker::Event {
    let result = match kind {
        ProfileListKind::Followers => social::followers(&api, &uri).await.map(Listing::People),
        ProfileListKind::Following => social::following(&api, &uri).await.map(Listing::People),
        ProfileListKind::Playlists => {
            social::public_playlists(&api, &uri, offset, social::PLAYLIST_PAGE)
                .await
                .map(|(items, total)| Listing::Playlists { items, total })
        }
        ProfileListKind::RecentArtists => {
            social::recent_artists(&api, &uri, social::RECENT_ARTISTS)
                .await
                .map(Listing::People)
        }
        ProfileListKind::TopArtists => social::top_content(&api, social::TOP_ALL, 0)
            .await
            .map(|top| Listing::People(top.artists)),
        ProfileListKind::TopTracks => social::top_content(&api, 0, social::TOP_ALL)
            .await
            .map(|top| Listing::Tracks(top.tracks)),
    }
    .map_err(|error| {
        if missing(&error) {
            "This profile doesn’t exist.".to_owned()
        } else {
            "Couldn’t load this list.".to_owned()
        }
    });
    Event::ProfileList {
        uri,
        kind,
        offset,
        result,
    }
    .into()
}
