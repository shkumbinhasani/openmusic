use std::{
    collections::{HashMap, HashSet},
    path::Path,
    rc::Rc,
    time::Instant,
};

use gpui::{Context, PromptButton, PromptLevel, Window};
use spotify_client::social::{ProfileRef, RefKind, username};

use super::{
    state::ProfileListKind,
    worker::{Command, Listing, ProfileView, block_message},
};
use crate::{
    app::{OpenMusic, Page},
    ui::{
        components::Sentinel,
        menu::{Group, MenuItem, Target},
    },
};

pub const OWN_ONLY: &str = "Only you can see your top artists and tracks, on your own profile.";
pub const BLOCKED_NOTE: &str =
    "Blocked. They can’t see your profile, follow you or see your listening activity.";

type Undo = Rc<dyn Fn(&mut OpenMusic, &mut Context<OpenMusic>)>;

pub fn block_detail(name: &str) -> String {
    format!(
        "{name} will no longer be able to see your profile, follow you, or see your listening activity."
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FollowFilter {
    #[default]
    All,
    Artists,
    Profiles,
}

impl FollowFilter {
    pub const FILTERS: [Self; 3] = [Self::All, Self::Artists, Self::Profiles];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Artists => "Artists",
            Self::Profiles => "Profiles",
        }
    }

    pub fn keeps(self, person: &ProfileRef) -> bool {
        match self {
            Self::All => true,
            Self::Artists => person.kind() == RefKind::Artist,
            Self::Profiles => person.kind() == RefKind::User,
        }
    }
}

pub struct ProfileList {
    pub uri: String,
    pub kind: ProfileListKind,
    pub result: Option<Result<Listing, String>>,
    pub loading_more: bool,
}

#[derive(Default)]
pub struct Profiles {
    pub profile: Option<(String, Result<ProfileView, String>)>,
    pub list: Option<ProfileList>,
    pub filter: FollowFilter,
    pub sentinel: Sentinel,
    pub following: HashMap<String, bool>,
    pub pending: HashMap<String, PendingFollow>,
    settled: HashMap<String, Instant>,
    pub blocking: HashMap<String, bool>,
    pub block_pending: HashSet<String>,
    loads: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingFollow {
    pub before: Option<bool>,
    pub counted_in: Option<u64>,
}

impl Profiles {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        if let Some((_, Ok(view))) = &self.profile {
            urls.extend(view.images());
        }
        if let Some(ProfileList {
            result: Some(Ok(listing)),
            ..
        }) = &self.list
        {
            urls.extend(listing.images());
        }
    }

    fn show(&mut self, profile: Option<(String, Result<ProfileView, String>)>) {
        self.profile = profile;
        self.loads += 1;
    }

    fn still_counted(&self, pending: PendingFollow) -> bool {
        pending.counted_in == Some(self.loads)
    }

    pub fn loaded(&self, uri: &str) -> Option<&ProfileView> {
        match &self.profile {
            Some((loaded, Ok(view))) if loaded == uri => Some(view),
            _ => None,
        }
    }

    pub fn list_for(&self, uri: &str, kind: ProfileListKind) -> Option<&ProfileList> {
        self.list
            .as_ref()
            .filter(|list| list.uri == uri && list.kind == kind)
    }

    pub fn known_following(&self, uri: &str) -> Option<bool> {
        self.following.get(uri).copied().or_else(|| {
            self.loaded(uri)
                .filter(|view| !view.profile.is_current_user)
                .map(|view| view.profile.is_following)
        })
    }

    pub fn unknown_users<'a>(
        &self,
        people: impl IntoIterator<Item = &'a ProfileRef>,
    ) -> Vec<String> {
        let mut seen = HashSet::new();
        people
            .into_iter()
            .filter(|person| person.kind() == RefKind::User)
            .filter(|person| !self.following.contains_key(&person.uri))
            .filter(|person| !self.pending.contains_key(&person.uri))
            .filter(|person| seen.insert(person.uri.clone()))
            .map(|person| person.uri.clone())
            .collect()
    }

    fn adjust(&mut self, uri: &str, before: Option<bool>, after: bool) -> bool {
        let Some((_, Ok(view))) = self.profile.as_mut() else {
            return false;
        };
        let profile = &mut view.profile;
        let viewed = profile.uri == uri;
        let before = before.or_else(|| viewed.then_some(profile.is_following));
        if before.is_none_or(|before| before == after) {
            return false;
        }
        let step = |count: &mut u64| {
            *count = if after {
                count.saturating_add(1)
            } else {
                count.saturating_sub(1)
            }
        };
        if viewed && !profile.is_current_user {
            step(&mut profile.followers);
        }
        if profile.is_current_user && !viewed {
            step(&mut profile.following);
        }
        true
    }

    pub fn begin_follow(
        &mut self,
        uris: &[String],
        follow: bool,
        own: Option<&str>,
    ) -> Vec<String> {
        let mut sent = Vec::new();
        for uri in uris {
            if self.pending.contains_key(uri)
                || username(uri).is_none()
                || own == Some(uri.as_str())
                || sent.contains(uri)
            {
                continue;
            }
            let before = self.known_following(uri);
            if before == Some(follow) {
                continue;
            }
            self.following.insert(uri.clone(), follow);
            let counted_in = self.adjust(uri, before, follow).then_some(self.loads);
            self.pending
                .insert(uri.clone(), PendingFollow { before, counted_in });
            sent.push(uri.clone());
        }
        sent
    }

    pub fn finish_follow(&mut self, states: Vec<(String, bool)>) {
        for (uri, state) in states {
            let pending = self.pending.remove(&uri);
            self.settled.insert(uri.clone(), Instant::now());
            let shown = self.following.insert(uri.clone(), state);
            if pending.is_some_and(|pending| self.still_counted(pending)) && shown != Some(state) {
                self.adjust(&uri, shown, state);
            }
        }
    }

    pub fn fail_follow(&mut self, uris: &[String]) {
        for uri in uris {
            let Some(pending) = self.pending.remove(uri) else {
                continue;
            };
            let shown = match pending.before {
                Some(before) => self.following.insert(uri.clone(), before),
                None => self.following.remove(uri),
            };
            if let Some(shown) = shown.filter(|_| self.still_counted(pending)) {
                self.adjust(uri, Some(shown), !shown);
            }
        }
    }

    pub fn forget_pending(&mut self) {
        let follows: Vec<String> = self.pending.keys().cloned().collect();
        self.fail_follow(&follows);
        self.block_pending.clear();
    }

    pub fn learn_following(&mut self, states: Vec<(String, bool)>, asked: Option<Instant>) {
        for (uri, state) in states {
            let overtaken =
                asked.is_some_and(|asked| self.settled.get(&uri).is_some_and(|at| *at > asked));
            if !overtaken && !self.pending.contains_key(&uri) {
                self.following.insert(uri, state);
            }
        }
    }

    pub fn known_blocking(&self, uri: &str) -> Option<bool> {
        self.blocking
            .get(uri)
            .copied()
            .or_else(|| self.loaded(uri).and_then(|view| view.blocking))
    }

    pub fn begin_block(&mut self, uri: &str, own: Option<&str>) -> bool {
        if self.block_pending.contains(uri) || username(uri).is_none() || own == Some(uri) {
            return false;
        }
        self.block_pending.insert(uri.to_owned())
    }

    pub fn finish_block(&mut self, uri: &str, block: bool, state: Option<bool>) -> bool {
        self.block_pending.remove(uri);
        if let Some(state) = state {
            self.blocking.insert(uri.to_owned(), state);
        }
        state == Some(block)
    }

    pub fn learn_blocking(&mut self, states: Vec<(String, bool)>) {
        for (uri, state) in states {
            if !self.block_pending.contains(&uri) {
                self.blocking.insert(uri, state);
            }
        }
    }

    pub fn person_name(&self, uri: &str) -> Option<String> {
        if let Some(view) = self.loaded(uri) {
            return Some(view.profile.name.clone());
        }
        let viewed = self
            .profile
            .iter()
            .filter_map(|(_, result)| result.as_ref().ok())
            .flat_map(|view| view.followers.iter().chain(&view.following));
        let listed = self
            .list
            .iter()
            .filter_map(|list| match &list.result {
                Some(Ok(Listing::People(people))) => Some(people.iter()),
                _ => None,
            })
            .flatten();
        viewed
            .chain(listed)
            .find(|person| person.uri == uri)
            .map(|person| person.name.clone())
    }
}

impl OpenMusic {
    pub fn is_own_profile(&self, uri: &str) -> bool {
        self.account().is_some_and(|account| account.uri == uri)
            || self
                .account
                .profiles
                .loaded(uri)
                .is_some_and(|view| view.profile.is_current_user)
    }

    pub fn set_following(&mut self, uris: Vec<String>, follow: bool) {
        let own = self.account().map(|account| account.uri.clone());
        let sent = self
            .account
            .profiles
            .begin_follow(&uris, follow, own.as_deref());
        if !sent.is_empty() {
            self.send(Command::Follow { uris: sent, follow });
        }
    }

    fn look_up_following(&mut self, uris: Vec<String>) {
        if !uris.is_empty() {
            self.send(Command::FollowLookup(uris));
        }
    }

    pub fn person_name(&self, uri: &str) -> String {
        self.account
            .profiles
            .person_name(uri)
            .or_else(|| {
                self.account
                    .friends
                    .items
                    .iter()
                    .find(|friend| friend.user.uri == uri)
                    .map(|friend| friend.user.name.clone())
            })
            .or_else(|| username(uri))
            .unwrap_or_else(|| uri.to_owned())
    }

    pub fn set_blocking(&mut self, uri: String, block: bool) {
        let own = self.account().map(|account| account.uri.clone());
        if self.account.profiles.begin_block(&uri, own.as_deref()) {
            self.send(Command::Block { uri, block });
        }
    }

    pub fn confirm_block(&mut self, uri: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.account.profiles.block_pending.contains(&uri) || self.is_own_profile(&uri) {
            return;
        }
        let name = self.person_name(&uri);
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Block {name}?"),
            Some(&block_detail(&name)),
            &[PromptButton::ok("Block"), PromptButton::cancel("Cancel")],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |app, cx| {
                    app.set_blocking(uri, true);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn reload_profile_page(&mut self) {
        let page = self.page.clone();
        if matches!(page, Page::Profile(_) | Page::ProfileList(..)) {
            load(self, &page);
        }
    }

    pub fn load_profile(&mut self, uri: &str) {
        let profiles = &mut self.account.profiles;
        if profiles.loaded(uri).is_none() {
            profiles.show(None);
        }
        self.send(Command::Profile(uri.to_owned()));
    }

    pub fn load_profile_list(&mut self, uri: &str, kind: ProfileListKind) {
        let someone_else = self.account().is_some_and(|account| account.uri != uri);
        let profiles = &mut self.account.profiles;
        if kind.own_only() && someone_else {
            profiles.list = Some(ProfileList {
                uri: uri.to_owned(),
                kind,
                result: Some(Err(OWN_ONLY.to_owned())),
                loading_more: false,
            });
            return;
        }
        let loaded = profiles
            .list_for(uri, kind)
            .is_some_and(|list| matches!(list.result, Some(Ok(_))));
        if !loaded {
            profiles.filter = FollowFilter::All;
            profiles.list = Some(ProfileList {
                uri: uri.to_owned(),
                kind,
                result: None,
                loading_more: false,
            });
        } else if let Some(list) = profiles.list.as_mut() {
            list.loading_more = false;
        }
        self.send(Command::ProfileList {
            uri: uri.to_owned(),
            kind,
            offset: 0,
        });
    }
}

pub fn load(app: &mut OpenMusic, page: &Page) {
    match page {
        Page::Profile(uri) => app.load_profile(uri),
        Page::ProfileList(uri, kind) => app.load_profile_list(uri, *kind),
        _ => {}
    }
}

pub fn on_profile(
    app: &mut OpenMusic,
    uri: String,
    result: Result<Box<ProfileView>, String>,
    asked: Instant,
) {
    let profiles = &mut app.account.profiles;
    if result.is_err() && profiles.loaded(&uri).is_some() {
        return;
    }
    let mut unknown = Vec::new();
    if let Ok(view) = &result {
        if !view.profile.is_current_user {
            profiles.learn_following(vec![(uri.clone(), view.profile.is_following)], Some(asked));
        }
        if let Some(blocking) = view.blocking {
            profiles.learn_blocking(vec![(uri.clone(), blocking)]);
        }
        unknown = profiles.unknown_users(view.followers.iter().chain(&view.following));
    }
    profiles.show(Some((uri, result.map(|view| *view))));
    app.look_up_following(unknown);
}

pub fn on_list(
    app: &mut OpenMusic,
    uri: String,
    kind: ProfileListKind,
    offset: usize,
    result: Result<Listing, String>,
) {
    if let Some(list) = app
        .account
        .profiles
        .list
        .as_mut()
        .filter(|list| list.uri == uri && list.kind == kind)
    {
        merge(list, offset, result);
    }
    let profiles = &app.account.profiles;
    let unknown = match profiles.list_for(&uri, kind) {
        Some(ProfileList {
            result: Some(Ok(Listing::People(people))),
            ..
        }) => profiles.unknown_users(people),
        _ => Vec::new(),
    };
    app.look_up_following(unknown);
}

pub fn on_followed(app: &mut OpenMusic, states: Vec<(String, bool)>, message: Option<String>) {
    app.account.profiles.finish_follow(states);
    if let Some(message) = message {
        app.show_toast(message, None);
    }
}

pub fn on_follow_failed(app: &mut OpenMusic, uris: Vec<String>, message: String) {
    app.account.profiles.fail_follow(&uris);
    app.show_toast(message, None);
}

pub fn on_blocked(app: &mut OpenMusic, uri: String, block: bool, state: Option<bool>) {
    let done = app.account.profiles.finish_block(&uri, block, state);
    let undo = (done && block).then(|| {
        let uri = uri.clone();
        Rc::new(move |app: &mut OpenMusic, _: &mut Context<OpenMusic>| {
            app.set_blocking(uri.clone(), false)
        }) as Undo
    });
    app.show_toast(block_message(block, done), undo);
    if done {
        app.reload_profile_page();
    }
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let Target::User { uri } = target else {
        return Vec::new();
    };
    if username(uri).is_none() {
        return Vec::new();
    }
    if app.is_own_profile(uri) {
        let edit = MenuItem::action("Edit profile", |app, window, cx| {
            app.edit_display_name(window, cx)
        })
        .symbol("pencil")
        .enabled(app.can_edit_name());
        return vec![(Group::Edit, edit)];
    }
    let profiles = &app.account.profiles;
    let blocked = profiles.known_blocking(uri) == Some(true);
    let mut items = Vec::new();
    if let Some(item) = follow_item(uri, profiles, blocked) {
        items.push((Group::Library, item));
    }
    items.push((
        Group::Feedback,
        block_item(uri, blocked, profiles.block_pending.contains(uri)),
    ));
    items
}

fn follow_item(uri: &str, profiles: &Profiles, blocked: bool) -> Option<MenuItem> {
    let following = profiles.known_following(uri)?;
    let closed = blocked
        || profiles
            .loaded(uri)
            .is_some_and(|view| !view.profile.allow_follows);
    if closed && !following {
        return None;
    }
    let pending = profiles.pending.contains_key(uri);
    let uri = uri.to_owned();
    let item = if following {
        MenuItem::action("Unfollow", move |app, _, _| {
            app.set_following(vec![uri.clone()], false)
        })
        .symbol("person.badge.minus")
    } else {
        MenuItem::action("Follow", move |app, _, _| {
            app.set_following(vec![uri.clone()], true)
        })
        .symbol("person.badge.plus")
    };
    Some(item.enabled(!pending))
}

fn block_item(uri: &str, blocked: bool, pending: bool) -> MenuItem {
    let uri = uri.to_owned();
    let item = if blocked {
        MenuItem::action("Unblock", move |app, _, _| {
            app.set_blocking(uri.clone(), false)
        })
        .symbol("hand.raised.slash")
    } else {
        MenuItem::action("Block", move |app, window, cx| {
            app.confirm_block(uri.clone(), window, cx)
        })
        .symbol("hand.raised")
    };
    item.enabled(!pending)
}

pub fn describe_blocking(profiles: &Profiles, uris: &[String]) -> String {
    uris.iter()
        .map(|uri| {
            let state = match profiles.known_blocking(uri) {
                Some(true) => "blocked",
                Some(false) => "not-blocked",
                None => "unknown",
            };
            let pending = if profiles.block_pending.contains(uri) {
                " pending"
            } else {
                ""
            };
            format!("{uri} {state}{pending}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn describe_following(profiles: &Profiles, uris: &[String]) -> String {
    uris.iter()
        .map(|uri| {
            let state = match profiles.known_following(uri) {
                Some(true) => "following",
                Some(false) => "not-following",
                None => "unknown",
            };
            let pending = if profiles.pending.contains_key(uri) {
                " pending"
            } else {
                ""
            };
            format!("{uri} {state}{pending}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn merge(list: &mut ProfileList, offset: usize, result: Result<Listing, String>) {
    list.loading_more = false;
    if offset == 0 {
        if result.is_ok() || !matches!(list.result, Some(Ok(_))) {
            list.result = Some(result);
        }
        return;
    }
    let Some(Ok(Listing::Playlists { items, total })) = list.result.as_mut() else {
        return;
    };
    let before = items.len();
    if let Ok(Listing::Playlists {
        items: more,
        total: reported,
    }) = result
    {
        let known: HashSet<String> = items.iter().map(|item| item.uri.clone()).collect();
        items.extend(more.into_iter().filter(|item| !known.contains(&item.uri)));
        *total = reported;
    }
    if items.len() == before || *total < items.len() as u64 {
        *total = items.len() as u64;
    }
}

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    let profiles = &mut app.account.profiles;
    if !profiles.sentinel.take() {
        return;
    }
    let Some(list) = profiles.list.as_mut().filter(|list| !list.loading_more) else {
        return;
    };
    let Some(Ok(Listing::Playlists { items, total })) = &list.result else {
        return;
    };
    if items.len() as u64 >= *total {
        return;
    }
    let command = Command::ProfileList {
        uri: list.uri.clone(),
        kind: list.kind,
        offset: items.len(),
    };
    list.loading_more = true;
    app.send(command);
    cx.notify();
}

pub fn describe_profile(view: &ProfileView) -> String {
    let profile = &view.profile;
    let top = view.top.as_ref();
    [
        format!("uri={}", profile.uri),
        format!("name={}", profile.name),
        format!("current={}", profile.is_current_user),
        format!("following={}", profile.is_following),
        format!("followers_count={}", profile.followers),
        format!("following_count={}", profile.following),
        format!("allow_follows={}", profile.allow_follows),
        format!("show_follows={}", profile.show_follows),
        format!(
            "playlists={}/{}",
            profile.playlists.len(),
            profile.total_playlists
        ),
        format!("recent_artists={}", profile.recent_artists.len()),
        format!("followers_shown={}", view.followers.len()),
        format!("following_shown={}", view.following.len()),
        format!(
            "blocking={}",
            view.blocking
                .map_or("unknown".to_owned(), |blocking| blocking.to_string())
        ),
        format!(
            "top_artists={}",
            top.map_or("none".to_owned(), |top| format!(
                "{}/{}",
                top.artists.len(),
                top.artists_total
            ))
        ),
        format!(
            "top_tracks={}",
            top.map_or("none".to_owned(), |top| format!(
                "{}/{}",
                top.tracks.len(),
                top.tracks_total
            ))
        ),
        format!("image={}", profile.image.is_some()),
    ]
    .join("\n")
}

pub fn describe_list(list: &ProfileList) -> String {
    let mut lines = vec![format!(
        "kind={} uri={} loading_more={}",
        list.kind.name(),
        list.uri,
        list.loading_more
    )];
    match &list.result {
        None => lines.push("loading".to_owned()),
        Some(Err(error)) => lines.push(format!("error={error}")),
        Some(Ok(listing)) => {
            lines.push(format!("count={}", listing.count()));
            match listing {
                Listing::People(people) => lines.extend(
                    people
                        .iter()
                        .map(|person| format!("{} {}", person.uri, person.name)),
                ),
                Listing::Playlists { items, total } => {
                    lines.push(format!("total={total}"));
                    lines.extend(
                        items
                            .iter()
                            .map(|item| format!("{} {}", item.uri, item.name)),
                    );
                }
                Listing::Tracks(tracks) => lines.extend(
                    tracks
                        .iter()
                        .map(|track| format!("{} {}", track.uri, track.title)),
                ),
            }
        }
    }
    lines.join("\n")
}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match verb {
        "block" => {
            let (uri, state) = rest.split_once(' ').unwrap_or((rest, "on"));
            if !uri.is_empty() {
                app.set_blocking(uri.to_owned(), state.trim() != "off");
            }
            let text = describe_blocking(&app.account.profiles, &[uri.to_owned()]);
            let _ = std::fs::write(directory.join("block.txt"), text);
        }
        "block-ask" => {
            if !rest.is_empty() {
                app.confirm_block(rest.trim().to_owned(), window, cx);
            }
        }
        "blocking" => {
            let uris: Vec<String> = rest.split_whitespace().map(str::to_owned).collect();
            let text = describe_blocking(&app.account.profiles, &uris);
            let _ = std::fs::write(directory.join("blocking.txt"), text);
            let ready: Vec<String> = uris
                .into_iter()
                .filter(|uri| !app.account.profiles.block_pending.contains(uri))
                .collect();
            if !ready.is_empty() {
                app.send(Command::BlockLookup(ready));
            }
        }
        "profile" => {
            if !rest.is_empty() {
                app.navigate(Page::Profile(rest.to_owned()));
            }
            let text = match &app.account.profiles.profile {
                None => "loading".to_owned(),
                Some((_, Ok(view))) => describe_profile(view),
                Some((uri, Err(error))) => format!("uri={uri}\nerror={error}"),
            };
            let _ = std::fs::write(directory.join("profile.txt"), text);
        }
        "profiles" => {
            let mut words = rest.split_whitespace();
            if let (Some(kind), Some(uri)) = (
                words.next().and_then(ProfileListKind::from_name),
                words.next(),
            ) {
                app.navigate(Page::ProfileList(uri.to_owned(), kind));
            }
            let text = app
                .account
                .profiles
                .list
                .as_ref()
                .map_or("none".to_owned(), describe_list);
            let _ = std::fs::write(directory.join("profiles.txt"), text);
        }
        "follow-user" => {
            let (uri, state) = rest.split_once(' ').unwrap_or((rest, "on"));
            if !uri.is_empty() {
                app.set_following(vec![uri.to_owned()], state.trim() != "off");
            }
            let text = describe_following(&app.account.profiles, &[uri.to_owned()]);
            let _ = std::fs::write(directory.join("follow.txt"), text);
        }
        "follows" => {
            let uris: Vec<String> = rest.split_whitespace().map(str::to_owned).collect();
            let text = describe_following(&app.account.profiles, &uris);
            let _ = std::fs::write(directory.join("follows.txt"), text);
            let ready: Vec<String> = uris
                .into_iter()
                .filter(|uri| !app.account.profiles.pending.contains_key(uri))
                .collect();
            app.look_up_following(ready);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use spotify_client::social::{PlaylistRef, Profile};

    use super::*;

    fn person(uri: &str) -> ProfileRef {
        ProfileRef {
            uri: uri.into(),
            name: "Name".into(),
            ..ProfileRef::default()
        }
    }

    fn playlist(id: usize) -> PlaylistRef {
        PlaylistRef {
            uri: format!("spotify:playlist:{id:022}"),
            name: format!("Mix {id}"),
            ..PlaylistRef::default()
        }
    }

    fn viewing(uri: &str, current: bool, following: bool, followers: u64) -> Profiles {
        let view = ProfileView {
            profile: Profile {
                uri: uri.into(),
                name: "Name".into(),
                is_current_user: current,
                is_following: following,
                allow_follows: true,
                followers,
                following: 20,
                ..Profile::default()
            },
            top: None,
            followers: Vec::new(),
            following: Vec::new(),
            blocking: None,
        };
        Profiles {
            profile: Some((uri.into(), Ok(view))),
            ..Profiles::default()
        }
    }

    fn counts(profiles: &Profiles) -> (u64, u64) {
        let Some((_, Ok(view))) = &profiles.profile else {
            panic!("no profile");
        };
        (view.profile.followers, view.profile.following)
    }

    #[test]
    fn follows_a_profile_and_keeps_the_count() {
        let friend = "spotify:user:friend".to_owned();
        let mut profiles = viewing(&friend, false, false, 2);
        assert_eq!(profiles.known_following(&friend), Some(false));
        let sent = profiles.begin_follow(&[friend.clone(), friend.clone()], true, None);
        assert_eq!(sent, vec![friend.clone()]);
        assert_eq!(profiles.known_following(&friend), Some(true));
        assert_eq!(counts(&profiles), (3, 20));
        assert!(
            profiles
                .begin_follow(std::slice::from_ref(&friend), false, None)
                .is_empty()
        );
        profiles.finish_follow(vec![(friend.clone(), true)]);
        assert!(profiles.pending.is_empty());
        assert_eq!(counts(&profiles), (3, 20));
        assert!(
            profiles
                .begin_follow(std::slice::from_ref(&friend), true, None)
                .is_empty()
        );
    }

    #[test]
    fn rolls_back_a_failed_follow() {
        let friend = "spotify:user:friend".to_owned();
        let mut profiles = viewing(&friend, false, true, 5);
        profiles.begin_follow(std::slice::from_ref(&friend), false, None);
        assert_eq!(counts(&profiles), (4, 20));
        profiles.fail_follow(std::slice::from_ref(&friend));
        assert_eq!(profiles.known_following(&friend), Some(true));
        assert_eq!(counts(&profiles), (5, 20));
        assert!(profiles.pending.is_empty());

        profiles.begin_follow(std::slice::from_ref(&friend), false, None);
        profiles.finish_follow(vec![(friend.clone(), true)]);
        assert_eq!(profiles.known_following(&friend), Some(true));
        assert_eq!(counts(&profiles), (5, 20));
    }

    #[test]
    fn rolls_back_only_the_profile_it_counted_on() {
        let own = "spotify:user:someone".to_owned();
        let friend = "spotify:user:friend".to_owned();
        let mut profiles = viewing(&friend, false, false, 5);
        profiles.begin_follow(std::slice::from_ref(&friend), true, Some(&own));
        assert_eq!(counts(&profiles), (6, 20));
        let mine = viewing(&own, true, false, 1).profile;
        profiles.show(mine);
        profiles.fail_follow(std::slice::from_ref(&friend));
        assert_eq!(counts(&profiles), (1, 20));
        assert_eq!(profiles.known_following(&friend), Some(false));

        profiles.show(viewing(&friend, false, false, 5).profile);
        profiles.begin_follow(std::slice::from_ref(&friend), true, Some(&own));
        assert_eq!(counts(&profiles), (6, 20));
        profiles.show(viewing(&friend, false, false, 5).profile);
        profiles.finish_follow(vec![(friend.clone(), false)]);
        assert_eq!(counts(&profiles), (5, 20));
        assert_eq!(profiles.known_following(&friend), Some(false));
    }

    #[test]
    fn forgets_answers_that_will_not_come() {
        let own = "spotify:user:someone".to_owned();
        let friend = "spotify:user:friend".to_owned();
        let stranger = "spotify:user:stranger".to_owned();
        let mut profiles = viewing(&friend, false, false, 5);
        profiles.begin_follow(&[friend.clone(), stranger.clone()], true, Some(&own));
        profiles.begin_block(&stranger, Some(&own));
        assert_eq!(counts(&profiles), (6, 20));
        profiles.forget_pending();
        assert!(profiles.pending.is_empty() && profiles.block_pending.is_empty());
        assert_eq!(counts(&profiles), (5, 20));
        assert_eq!(profiles.known_following(&friend), Some(false));
        assert_eq!(profiles.known_following(&stranger), None);
        assert_eq!(
            profiles.begin_follow(std::slice::from_ref(&friend), true, Some(&own)),
            vec![friend.clone()]
        );
        profiles.finish_follow(vec![(friend.clone(), true)]);
        assert_eq!(counts(&profiles), (6, 20));
    }

    #[test]
    fn counts_who_you_follow_on_your_own_profile() {
        let own = "spotify:user:someone".to_owned();
        let friend = "spotify:user:friend".to_owned();
        let stranger = "spotify:user:stranger".to_owned();
        let mut profiles = viewing(&own, true, false, 1);
        assert_eq!(profiles.known_following(&own), None);
        assert!(
            profiles
                .begin_follow(std::slice::from_ref(&own), true, Some(&own))
                .is_empty()
        );
        profiles.learn_following(vec![(friend.clone(), true)], None);
        profiles.begin_follow(std::slice::from_ref(&friend), false, Some(&own));
        assert_eq!(counts(&profiles), (1, 19));
        profiles.begin_follow(std::slice::from_ref(&stranger), true, Some(&own));
        assert_eq!(counts(&profiles), (1, 19));
        assert_eq!(profiles.known_following(&stranger), Some(true));
        profiles.fail_follow(std::slice::from_ref(&stranger));
        assert_eq!(profiles.known_following(&stranger), None);
        assert_eq!(counts(&profiles), (1, 19));
        profiles.fail_follow(std::slice::from_ref(&friend));
        assert_eq!(profiles.known_following(&friend), Some(true));
        assert_eq!(counts(&profiles), (1, 20));
        assert!(
            profiles
                .begin_follow(&["spotify:artist:x".to_owned()], true, Some(&own))
                .is_empty()
        );
    }

    #[test]
    fn a_profile_read_before_a_follow_settled_does_not_undo_it() {
        let friend = "spotify:user:friend".to_owned();
        let mut profiles = Profiles::default();
        let before_follow = Instant::now() - std::time::Duration::from_millis(5);
        profiles.begin_follow(std::slice::from_ref(&friend), true, None);
        profiles.finish_follow(vec![(friend.clone(), true)]);
        profiles.learn_following(vec![(friend.clone(), false)], Some(before_follow));
        assert_eq!(profiles.known_following(&friend), Some(true));
        let later = Instant::now() + std::time::Duration::from_millis(5);
        profiles.learn_following(vec![(friend.clone(), false)], Some(later));
        assert_eq!(profiles.known_following(&friend), Some(false));
    }

    #[test]
    fn learns_follow_states_without_touching_pending_ones() {
        let friend = "spotify:user:friend".to_owned();
        let other = "spotify:user:other".to_owned();
        let mut profiles = Profiles::default();
        profiles.begin_follow(std::slice::from_ref(&friend), true, None);
        profiles.learn_following(vec![(friend.clone(), false), (other.clone(), false)], None);
        assert_eq!(profiles.known_following(&friend), Some(true));
        assert_eq!(profiles.known_following(&other), Some(false));
        let people = vec![
            person("spotify:user:friend"),
            person("spotify:user:other"),
            person("spotify:user:new"),
            person("spotify:user:new"),
            person("spotify:artist:0000000000000000000001"),
        ];
        assert_eq!(profiles.unknown_users(&people), vec!["spotify:user:new"]);
        let text = describe_following(&profiles, &[friend, other, "spotify:user:new".into()]);
        assert_eq!(
            text,
            "spotify:user:friend following pending\nspotify:user:other not-following\nspotify:user:new unknown"
        );
    }

    #[test]
    fn blocks_and_unblocks_with_the_server_answer() {
        let own = "spotify:user:someone".to_owned();
        let friend = "spotify:user:friend".to_owned();
        let mut profiles = viewing(&friend, false, true, 5);
        assert_eq!(profiles.known_blocking(&friend), None);
        if let Some((_, Ok(view))) = profiles.profile.as_mut() {
            view.blocking = Some(false);
        }
        assert_eq!(profiles.known_blocking(&friend), Some(false));
        assert!(!profiles.begin_block(&own, Some(&own)));
        assert!(!profiles.begin_block("spotify:artist:x", Some(&own)));
        assert!(profiles.begin_block(&friend, Some(&own)));
        assert!(!profiles.begin_block(&friend, Some(&own)));
        profiles.learn_blocking(vec![(friend.clone(), true)]);
        assert_eq!(profiles.known_blocking(&friend), Some(false));
        assert!(profiles.finish_block(&friend, true, Some(true)));
        assert!(profiles.block_pending.is_empty());
        assert_eq!(profiles.known_blocking(&friend), Some(true));
        assert!(describe_blocking(&profiles, std::slice::from_ref(&friend)).ends_with(" blocked"));

        profiles.begin_block(&friend, Some(&own));
        assert!(!profiles.finish_block(&friend, false, None));
        assert_eq!(profiles.known_blocking(&friend), Some(true));
        profiles.begin_block(&friend, Some(&own));
        assert!(!profiles.finish_block(&friend, false, Some(true)));
        assert_eq!(profiles.known_blocking(&friend), Some(true));
        profiles.begin_block(&friend, Some(&own));
        assert!(profiles.finish_block(&friend, false, Some(false)));
        assert_eq!(
            describe_blocking(&profiles, &[friend, "spotify:user:new".into()]),
            "spotify:user:friend not-blocked\nspotify:user:new unknown"
        );
    }

    #[test]
    fn names_people_from_loaded_lists() {
        let mut profiles = viewing("spotify:user:friend", false, false, 1);
        assert_eq!(
            profiles.person_name("spotify:user:friend").as_deref(),
            Some("Name")
        );
        if let Some((_, Ok(view))) = profiles.profile.as_mut() {
            view.followers.push(ProfileRef {
                uri: "spotify:user:fan".into(),
                name: "A Fan".into(),
                ..ProfileRef::default()
            });
        }
        profiles.list = Some(ProfileList {
            uri: "spotify:user:friend".into(),
            kind: ProfileListKind::Following,
            result: Some(Ok(Listing::People(vec![ProfileRef {
                uri: "spotify:user:pal".into(),
                name: "A Pal".into(),
                ..ProfileRef::default()
            }]))),
            loading_more: false,
        });
        assert_eq!(
            profiles.person_name("spotify:user:fan").as_deref(),
            Some("A Fan")
        );
        assert_eq!(
            profiles.person_name("spotify:user:pal").as_deref(),
            Some("A Pal")
        );
        assert_eq!(profiles.person_name("spotify:user:nobody"), None);
        assert!(block_detail("A Pal").starts_with("A Pal will no longer"));
    }

    #[test]
    fn filters_the_following_list() {
        let artist = person("spotify:artist:0000000000000000000001");
        let user = person("spotify:user:friend");
        assert!(FollowFilter::All.keeps(&artist) && FollowFilter::All.keeps(&user));
        assert!(FollowFilter::Artists.keeps(&artist));
        assert!(!FollowFilter::Artists.keeps(&user));
        assert!(FollowFilter::Profiles.keeps(&user));
        assert!(!FollowFilter::Profiles.keeps(&artist));
    }

    #[test]
    fn describes_a_loaded_profile() {
        let view = ProfileView {
            profile: Profile {
                uri: "spotify:user:someone".into(),
                name: "Someone".into(),
                total_playlists: 24,
                playlists: vec![playlist(1)],
                ..Profile::default()
            },
            top: None,
            followers: vec![person("spotify:user:friend")],
            following: Vec::new(),
            blocking: None,
        };
        let text = describe_profile(&view);
        assert!(text.contains("playlists=1/24"));
        assert!(text.contains("followers_shown=1"));
        assert!(text.contains("top_artists=none"));
        assert_eq!(view.images(), Vec::<String>::new());
    }

    fn playlists(list: &ProfileList) -> (Vec<String>, u64) {
        match &list.result {
            Some(Ok(Listing::Playlists { items, total })) => {
                (items.iter().map(|item| item.name.clone()).collect(), *total)
            }
            _ => panic!("expected playlists"),
        }
    }

    #[test]
    fn merges_playlist_pages() {
        let mut list = ProfileList {
            uri: "spotify:user:someone".into(),
            kind: ProfileListKind::Playlists,
            result: None,
            loading_more: false,
        };
        merge(
            &mut list,
            0,
            Ok(Listing::Playlists {
                items: vec![playlist(1), playlist(2)],
                total: 5,
            }),
        );
        assert_eq!(playlists(&list), (vec!["Mix 1".into(), "Mix 2".into()], 5));
        list.loading_more = true;
        merge(
            &mut list,
            2,
            Ok(Listing::Playlists {
                items: vec![playlist(2), playlist(3)],
                total: 5,
            }),
        );
        assert!(!list.loading_more);
        assert_eq!(playlists(&list).0, vec!["Mix 1", "Mix 2", "Mix 3"]);
        merge(&mut list, 3, Err("offline".into()));
        assert_eq!(playlists(&list).1, 3);
        merge(&mut list, 0, Err("offline".into()));
        assert_eq!(playlists(&list).0.len(), 3);
        merge(
            &mut list,
            3,
            Ok(Listing::Playlists {
                items: Vec::new(),
                total: 9,
            }),
        );
        assert_eq!(playlists(&list).1, 3);
    }

    #[test]
    fn stops_paging_when_a_page_adds_nothing_new() {
        let mut list = ProfileList {
            uri: "spotify:user:someone".into(),
            kind: ProfileListKind::Playlists,
            result: None,
            loading_more: false,
        };
        merge(
            &mut list,
            0,
            Ok(Listing::Playlists {
                items: vec![playlist(1), playlist(2)],
                total: 4,
            }),
        );
        merge(
            &mut list,
            2,
            Ok(Listing::Playlists {
                items: vec![playlist(1), playlist(2)],
                total: 4,
            }),
        );
        assert_eq!(playlists(&list), (vec!["Mix 1".into(), "Mix 2".into()], 2));
    }

    #[test]
    fn describes_playlist_pages() {
        let list = ProfileList {
            uri: "spotify:user:someone".into(),
            kind: ProfileListKind::Playlists,
            result: Some(Ok(Listing::Playlists {
                items: vec![playlist(1), playlist(2)],
                total: 3,
            })),
            loading_more: true,
        };
        let text = describe_list(&list);
        assert!(text.starts_with("kind=playlists uri=spotify:user:someone loading_more=true"));
        assert!(text.contains("count=2\ntotal=3"));
    }
}
