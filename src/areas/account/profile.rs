use std::{collections::HashSet, path::Path};

use gpui::Context;
use spotify_client::social::{ProfileRef, RefKind};

use super::{
    state::ProfileListKind,
    worker::{Command, Listing, ProfileView},
};
use crate::{
    app::{OpenMusic, Page},
    ui::components::Sentinel,
};

pub const OWN_ONLY: &str = "Only you can see your top artists and tracks, on your own profile.";

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
}

impl OpenMusic {
    pub fn load_profile(&mut self, uri: &str) {
        let profiles = &mut self.account.profiles;
        if profiles.loaded(uri).is_none() {
            profiles.profile = None;
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

pub fn on_profile(app: &mut OpenMusic, uri: String, result: Result<Box<ProfileView>, String>) {
    let profiles = &mut app.account.profiles;
    if result.is_err() && profiles.loaded(&uri).is_some() {
        return;
    }
    profiles.profile = Some((uri, result.map(|view| *view)));
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
    match result {
        Ok(Listing::Playlists {
            items: more,
            total: reported,
        }) if !more.is_empty() => {
            let known: HashSet<String> = items.iter().map(|item| item.uri.clone()).collect();
            items.extend(more.into_iter().filter(|item| !known.contains(&item.uri)));
            *total = reported.max(items.len() as u64);
        }
        _ => *total = items.len() as u64,
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

pub fn debug(app: &mut OpenMusic, verb: &str, rest: &str, directory: &Path) -> bool {
    match verb {
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
