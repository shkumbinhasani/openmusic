use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};

use spotify_client::{
    Api, Track,
    account::{self, AccountInfo},
    api::HttpStatus,
    dealer::Message,
    engine::{Control, playback},
    social::{self, Friend, PlaylistRef, Privacy, PrivacyField, Profile, ProfileRef, TopContent},
};
use tokio::sync::mpsc::UnboundedSender;

use super::{
    local_scan::{self, CoverCache, LocalFile, LocalSources, ScanReport, ScanUpdate},
    state::{AccountPrefs, ProfileListKind, chosen_language, system_language},
};
use crate::worker::{Ctx, Mutation, User, cache};

const SECOND_LOOK: Duration = Duration::from_secs(5);
const LOOKUP_BATCH: usize = 50;

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
    Follow {
        uris: Vec<String>,
        follow: bool,
    },
    FollowLookup(Vec<String>),
    Block {
        uri: String,
        block: bool,
    },
    BlockLookup(Vec<String>),
    Privacy,
    SetPrivacy {
        field: PrivacyField,
        value: bool,
    },
    SetName {
        name: String,
        editor: Option<u64>,
    },
    RefreshSoon,
    Friends,
    LocalScan {
        sources: LocalSources,
        generation: u64,
        load: bool,
    },
    LocalStop(u64),
    LocalArt(Vec<(String, PathBuf)>),
    PlayLocal {
        files: Vec<LocalFile>,
        index: usize,
    },
}

pub struct ProfileView {
    pub profile: Profile,
    pub top: Option<TopContent>,
    pub followers: Vec<ProfileRef>,
    pub following: Vec<ProfileRef>,
    pub blocking: Option<bool>,
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
        asked: Instant,
    },
    ProfileList {
        uri: String,
        kind: ProfileListKind,
        offset: usize,
        result: Result<Listing, String>,
    },
    Followed {
        states: Vec<(String, bool)>,
        message: Option<String>,
    },
    FollowFailed {
        uris: Vec<String>,
        message: String,
    },
    FollowState {
        states: Vec<(String, bool)>,
        asked: Instant,
    },
    Blocked {
        uri: String,
        block: bool,
        state: Option<bool>,
    },
    BlockState(Vec<(String, bool)>),
    Privacy(Result<Privacy, String>),
    PrivacySaved {
        field: PrivacyField,
        value: bool,
        saved: bool,
        now: Option<Privacy>,
    },
    NameSaved {
        name: String,
        done: bool,
        editor: Option<u64>,
    },
    Friends {
        result: Result<Vec<Friend>, String>,
        slow: bool,
    },
    LocalScan(Box<ScanUpdate>),
    LocalArt(Vec<String>),
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
            Self::Friends {
                result: Ok(friends),
                ..
            } => friends.iter().flat_map(Friend::images).collect(),
            Self::LocalArt(keys) => keys.clone(),
            Self::Account(Err(_))
            | Self::Cache { .. }
            | Self::Profile { .. }
            | Self::ProfileList { .. }
            | Self::Followed { .. }
            | Self::FollowFailed { .. }
            | Self::FollowState { .. }
            | Self::Blocked { .. }
            | Self::BlockState(_)
            | Self::Privacy(_)
            | Self::PrivacySaved { .. }
            | Self::NameSaved { .. }
            | Self::Friends { .. }
            | Self::LocalScan(_) => Vec::new(),
        }
    }
}

pub fn handle(command: Command, ctx: &mut Ctx) {
    match command {
        Command::Refresh => refresh(ctx),
        Command::Language(language) => apply_language(language.as_deref(), ctx),
        Command::CacheSize => ctx.tasks.spawn_keyed("cache-size", cache_size(false)),
        Command::ClearCache => ctx.tasks.spawn(cache_size(true)),
        Command::Profile(uri) => ctx.tasks.spawn_keyed(
            "profile",
            profile_view(ctx.api.clone(), ctx.user.clone(), uri, Instant::now()),
        ),
        Command::ProfileList { uri, kind, offset } => ctx.tasks.spawn_keyed(
            "profile-list",
            profile_list(ctx.api.clone(), uri, kind, offset),
        ),
        Command::Follow { uris, follow } => {
            let keys = uris.iter().map(|uri| format!("follow:{uri}")).collect();
            let api = ctx.api.clone();
            ctx.writes.push(Mutation::new(keys, async move {
                Ok(follow_users(api, uris, follow).await)
            }));
        }
        Command::FollowLookup(uris) => {
            ctx.tasks
                .spawn(follow_lookup(ctx.api.clone(), uris, Instant::now()))
        }
        Command::Block { uri, block } => {
            let keys = vec![format!("block:{uri}")];
            let (api, user) = (ctx.api.clone(), ctx.user.clone());
            ctx.writes.push(Mutation::new(keys, async move {
                Ok(block_user(api, user, uri, block).await)
            }));
        }
        Command::BlockLookup(uris) => {
            ctx.tasks
                .spawn(block_lookup(ctx.api.clone(), ctx.user.clone(), uris))
        }
        Command::Privacy => ctx
            .tasks
            .spawn_keyed("privacy", privacy_read(ctx.api.clone(), ctx.user.clone())),
        Command::SetPrivacy { field, value } => {
            let keys = vec![format!("privacy:{}", field.path())];
            let (api, user) = (ctx.api.clone(), ctx.user.clone());
            ctx.writes.push(Mutation::new(keys, async move {
                Ok(privacy_write(api, user, field, value).await)
            }));
        }
        Command::SetName { name, editor } => {
            let (api, user) = (ctx.api.clone(), ctx.user.clone());
            ctx.writes
                .push(Mutation::new(vec!["profile:name".to_owned()], async move {
                    Ok(rename(api, user, name, editor).await)
                }));
        }
        Command::RefreshSoon => ctx
            .tasks
            .spawn_keyed("account-later", fetch(ctx.api.clone(), Some(SECOND_LOOK))),
        Command::Friends => ctx
            .tasks
            .spawn_keyed("friends", friend_activity(ctx.api.clone())),
        Command::LocalScan {
            sources,
            generation,
            load,
        } => {
            local_scan::supersede(generation);
            let events = ctx.events.clone();
            ctx.tasks.spawn_keyed(
                "local-scan",
                scan_local_files(sources, generation, load, events),
            );
        }
        Command::LocalStop(generation) => local_scan::supersede(generation),
        Command::LocalArt(items) => ctx.tasks.spawn(local_art(items)),
        Command::PlayLocal { files, index } => {
            for file in &files {
                ctx.known.tracks.insert(file.uri.clone(), file.track());
            }
            let tracks = files.iter().map(LocalFile::local_track).collect();
            ctx.handle
                .send(Control::Audio(playback::Control::PlayLocal {
                    tracks,
                    index,
                }));
        }
    }
}

async fn scan_local_files(
    sources: LocalSources,
    generation: u64,
    load: bool,
    events: UnboundedSender<crate::worker::Event>,
) -> crate::worker::Event {
    let finished = tokio::task::spawn_blocking(move || {
        let send = |update: ScanUpdate| {
            let _ = events.send(Event::LocalScan(Box::new(update)).into());
        };
        let path = local_scan::index_path();
        let previous = path
            .as_deref()
            .map(local_scan::read_index)
            .unwrap_or_default();
        if load {
            send(ScanUpdate {
                generation,
                files: Some(previous.clone()),
                ..ScanUpdate::default()
            });
        }
        let alive = || local_scan::live(generation);
        let mut report = send;
        let update = local_scan::scan(
            &sources,
            local_scan::home().as_deref(),
            previous,
            generation,
            &CoverCache,
            &alive,
            &mut report,
        )?;
        if let (Some(path), Some(files)) = (&path, &update.files)
            && alive()
        {
            local_scan::write_index(path, files);
        }
        Some(update)
    })
    .await
    .ok()
    .flatten()
    .unwrap_or(ScanUpdate {
        generation,
        finished: Some(ScanReport::default()),
        ..ScanUpdate::default()
    });
    Event::LocalScan(Box::new(finished)).into()
}

async fn local_art(items: Vec<(String, PathBuf)>) -> crate::worker::Event {
    let keys = tokio::task::spawn_blocking(move || {
        items
            .into_iter()
            .filter(|(key, path)| local_scan::ensure_art(key, path, &CoverCache))
            .map(|(key, _)| key)
            .collect()
    })
    .await
    .unwrap_or_default();
    Event::LocalArt(keys).into()
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

fn status(error: &anyhow::Error) -> Option<reqwest::StatusCode> {
    error.downcast_ref::<HttpStatus>().map(|status| status.0)
}

fn missing(error: &anyhow::Error) -> bool {
    status(error) == Some(reqwest::StatusCode::NOT_FOUND)
}

async fn profile_view(api: Api, user: User, uri: String, asked: Instant) -> crate::worker::Event {
    let result = load_profile(&api, &user, &uri)
        .await
        .map(Box::new)
        .map_err(|error| {
            if missing(&error) {
                "This profile doesn’t exist.".to_owned()
            } else {
                "Couldn’t load this profile.".to_owned()
            }
        });
    Event::Profile { uri, result, asked }.into()
}

async fn load_profile(api: &Api, user: &User, uri: &str) -> anyhow::Result<ProfileView> {
    let profile = social::profile(api, uri).await?;
    let own = profile.is_current_user;
    let lists = own || profile.show_follows;
    let (top, followers, following, blocking) = tokio::join!(
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
        async {
            if own {
                return None;
            }
            let me = user.name(api).await.ok()?;
            social::is_blocking(api, &me, uri).await.ok()
        },
    );
    Ok(ProfileView {
        profile,
        top,
        followers,
        following,
        blocking,
    })
}

async fn block_user(api: Api, user: User, uri: String, block: bool) -> crate::worker::Event {
    let state = async {
        let me = user.name(&api).await.ok()?;
        social::set_blocking(&api, &me, &uri, block).await.ok()?;
        Some(social::is_blocking(&api, &me, &uri).await.unwrap_or(block))
    }
    .await;
    Event::Blocked { uri, block, state }.into()
}

async fn block_lookup(api: Api, user: User, uris: Vec<String>) -> crate::worker::Event {
    let mut states = Vec::new();
    if let Ok(me) = user.name(&api).await {
        for uri in uris {
            if let Ok(blocking) = social::is_blocking(&api, &me, &uri).await {
                states.push((uri, blocking));
            }
        }
    }
    Event::BlockState(states).into()
}

async fn privacy_read(api: Api, user: User) -> crate::worker::Event {
    let result = async {
        let me = user.name(&api).await?;
        social::privacy(&api, &me).await
    }
    .await
    .map_err(|_| super::privacy::LOAD_FAILED.to_owned());
    Event::Privacy(result).into()
}

async fn privacy_write(
    api: Api,
    user: User,
    field: PrivacyField,
    value: bool,
) -> crate::worker::Event {
    let written = async {
        let me = user.name(&api).await.ok()?;
        social::set_privacy(&api, &me, field, value).await.ok()?;
        Some(social::privacy(&api, &me).await.ok())
    }
    .await;
    Event::PrivacySaved {
        field,
        value,
        saved: written.is_some(),
        now: written.flatten(),
    }
    .into()
}

async fn friend_activity(api: Api) -> crate::worker::Event {
    let (result, slow) = match social::friends(&api).await {
        Ok(friends) => (Ok(friends), false),
        Err(error) => (
            Err(super::friends::LOAD_FAILED.to_owned()),
            status(&error) == Some(reqwest::StatusCode::TOO_MANY_REQUESTS),
        ),
    };
    Event::Friends { result, slow }.into()
}

async fn rename(api: Api, user: User, name: String, editor: Option<u64>) -> crate::worker::Event {
    let done = match user.name(&api).await {
        Ok(me) => social::set_display_name(&api, &me, &name).await.is_ok(),
        Err(_) => false,
    };
    Event::NameSaved { name, done, editor }.into()
}

pub fn block_message(block: bool, done: bool) -> &'static str {
    match (block, done) {
        (true, true) => "You have blocked this account.",
        (false, true) => "You have unblocked this account.",
        (true, false) => "Couldn’t block this account. Try again.",
        (false, false) => "Couldn’t unblock this account. Try again.",
    }
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

pub fn follow_message(follow: bool) -> &'static str {
    if follow {
        "Couldn’t follow this profile. Try again."
    } else {
        "Couldn’t unfollow this profile. Try again."
    }
}

async fn follow_users(api: Api, uris: Vec<String>, follow: bool) -> crate::worker::Event {
    let written = match social::set_following(&api, &uris, follow).await {
        Ok(written) => written,
        Err(_) => {
            return Event::FollowFailed {
                uris,
                message: follow_message(follow).to_owned(),
            }
            .into();
        }
    };
    let unknown: Vec<String> = written
        .iter()
        .filter(|(_, state)| state.is_none())
        .map(|(uri, _)| uri.clone())
        .collect();
    let checked: HashMap<String, bool> = if unknown.is_empty() {
        HashMap::new()
    } else {
        social::is_following(&api, &unknown)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect()
    };
    let states = settle(written, &checked, follow);
    let refused = states.iter().any(|(_, state)| *state != follow);
    Event::Followed {
        states,
        message: refused.then(|| follow_message(follow).to_owned()),
    }
    .into()
}

pub fn settle(
    written: Vec<(String, Option<bool>)>,
    checked: &HashMap<String, bool>,
    follow: bool,
) -> Vec<(String, bool)> {
    written
        .into_iter()
        .map(|(uri, state)| {
            let state = state
                .or_else(|| checked.get(&uri).copied())
                .unwrap_or(follow);
            (uri, state)
        })
        .collect()
}

async fn follow_lookup(api: Api, uris: Vec<String>, asked: Instant) -> crate::worker::Event {
    let mut states = Vec::new();
    for batch in uris.chunks(LOOKUP_BATCH) {
        if let Ok(found) = social::is_following(&api, batch).await {
            states.extend(found);
        }
    }
    Event::FollowState { states, asked }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settles_follow_results() {
        let written = vec![
            ("spotify:user:a".to_owned(), Some(true)),
            ("spotify:user:b".to_owned(), Some(false)),
            ("spotify:user:c".to_owned(), None),
            ("spotify:user:d".to_owned(), None),
        ];
        let checked = HashMap::from([("spotify:user:c".to_owned(), false)]);
        assert_eq!(
            settle(written, &checked, true),
            vec![
                ("spotify:user:a".to_owned(), true),
                ("spotify:user:b".to_owned(), false),
                ("spotify:user:c".to_owned(), false),
                ("spotify:user:d".to_owned(), true),
            ]
        );
    }
}
