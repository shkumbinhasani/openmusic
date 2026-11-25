use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, AppContext, ClipboardItem, Context, PromptButton, PromptLevel, SharedString, Window,
};
use spotify_client::{
    catalog::{Capabilities, Collection},
    connect::{self, PlayOptions, Shuffle},
    playlist::{
        self, Details, ItemPosition, Level, Permissions, RootEntry, RootPlaylist, Rootlist,
    },
};

use super::{
    dialogs::{self, Collaborators, Draft, EditDetails},
    image,
    worker::{Command, Cover, Event, Write},
};
use crate::{
    app::{OpenMusic, Page},
    areas::library,
    menus::{self, Handle},
    ui::{
        components::Sentinel,
        menu::Target,
        tracks::{RowDrag, RowDrop},
    },
};

const BASE_PREFETCH: usize = 50;
const BASE_SPACING: u32 = 4;

#[derive(Default)]
pub struct State {
    pub collection: Option<(String, Result<Collection, String>)>,
    pub loading_more: bool,
    pub sentinel: Sentinel,
    pub user: Option<String>,
    pub rootlist: Option<Rootlist>,
    pub permissions: HashMap<String, Permissions>,
    pub bases: HashMap<String, Level>,
    pub base_queue: VecDeque<String>,
    pub base_wait: u32,
    pub edit_after_load: Option<String>,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        if let Some((_, Ok(collection))) = &self.collection {
            urls.extend(collection.cover.clone());
            urls.extend(
                collection
                    .tracks
                    .iter()
                    .filter_map(|track| track.cover.clone()),
            );
            if let Some(permissions) = self.permissions.get(&collection.uri) {
                urls.extend(
                    permissions
                        .members
                        .iter()
                        .filter_map(|member| member.image.clone()),
                );
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistInfo {
    pub uri: String,
    pub name: String,
    pub owner: Option<String>,
    pub owner_name: Option<String>,
    pub owned: bool,
    pub saved: Option<bool>,
    pub on_profile: bool,
    pub capabilities: Capabilities,
    pub base: Option<Level>,
    pub members: usize,
}

pub fn owner_label(owner: &str, name: Option<&str>) -> String {
    match name {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ if owner == "spotify" => "Spotify".to_owned(),
        _ => owner.to_owned(),
    }
}

fn random_index(length: usize) -> usize {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos() as usize);
    nanos % length.max(1)
}

impl OpenMusic {
    pub fn open_playlist(&self, uri: &str) -> Option<&Collection> {
        let uri = playlist::canonical(uri)?;
        match &self.playlists.collection {
            Some((_, Ok(collection))) if collection.uri == uri => Some(collection),
            _ => None,
        }
    }

    pub fn playlist_info(&self, uri: &str) -> Option<PlaylistInfo> {
        let uri = playlist::canonical(uri)?;
        if playlist::is_virtual(&uri) {
            return None;
        }
        let open = self.open_playlist(&uri);
        let rootlist = self.playlists.rootlist.as_ref();
        let root = rootlist.and_then(|list| list.playlist(&uri));
        let permissions = self.playlists.permissions.get(&uri);
        let mut capabilities = open
            .map(|collection| collection.capabilities)
            .or_else(|| root.map(|playlist| playlist.capabilities))
            .unwrap_or_default();
        if let Some(permissions) = permissions {
            capabilities.administrate = permissions.administrate;
            capabilities.cancel_membership = permissions.cancel_membership;
            capabilities.edit_items |= permissions.edit_items;
            capabilities.edit_metadata |= permissions.edit_metadata;
        }
        let owner = open
            .and_then(|collection| collection.owner.clone())
            .or_else(|| root.and_then(|playlist| playlist.owner.clone()));
        let owned = match (&owner, &self.playlists.user) {
            (Some(owner), Some(user)) => owner == user,
            _ => capabilities.delete,
        };
        Some(PlaylistInfo {
            name: open
                .map(|collection| collection.name.clone())
                .or_else(|| root.map(|playlist| playlist.name.clone()))
                .unwrap_or_default(),
            owner_name: permissions
                .and_then(Permissions::owner)
                .map(|member| member.name.clone()),
            owner,
            owned,
            saved: rootlist.map(|list| list.contains(&uri)),
            on_profile: root.is_some_and(|playlist| playlist.public),
            base: self
                .playlists
                .bases
                .get(&uri)
                .copied()
                .or_else(|| permissions.and_then(|permissions| permissions.base)),
            members: permissions.map_or(0, |permissions| permissions.members.len()),
            capabilities,
            uri,
        })
    }

    pub fn refresh_rootlist(&self) {
        self.send(Command::Rootlist);
        self.send(library::worker::Command::Playlists);
    }

    pub fn reload_playlist(&mut self, uri: &str) {
        let Some(canonical) = playlist::canonical(uri) else {
            return;
        };
        if let Some((loaded, Ok(collection))) = &self.playlists.collection
            && collection.uri == canonical
        {
            self.send(Command::Reload {
                uri: loaded.clone(),
                upto: collection.tracks.len(),
            });
        }
        self.send(Command::Permissions(canonical));
    }

    fn write(&self, write: Write, done: Option<String>) {
        self.send(Command::Write { write, done });
    }

    pub fn create_playlist(
        &mut self,
        name: Option<String>,
        folder: Option<String>,
        tracks: Vec<String>,
        edit: bool,
    ) {
        self.write(
            Write::Create {
                name,
                folder,
                tracks,
                edit,
            },
            None,
        );
    }

    pub fn follow_playlist(&mut self, uri: &str, follow: bool) {
        let Some(info) = self.playlist_info(uri) else {
            return;
        };
        let entry = self
            .open_playlist(&info.uri)
            .map(|collection| RootPlaylist {
                uri: info.uri.clone(),
                key: info.uri.clone(),
                name: collection.name.clone(),
                description: collection.description.clone(),
                cover: collection.cover.clone(),
                picture: collection.picture.clone(),
                owner: collection.owner.clone(),
                format: collection.format.clone(),
                length: collection.total as u32,
                capabilities: collection.capabilities,
                ..RootPlaylist::default()
            });
        if let Some(list) = &mut self.playlists.rootlist {
            if follow && !list.contains(&info.uri) {
                list.insert_first(entry.unwrap_or_else(|| RootPlaylist {
                    uri: info.uri.clone(),
                    key: info.uri.clone(),
                    name: info.name.clone(),
                    ..RootPlaylist::default()
                }));
            } else if !follow {
                list.remove(&info.uri);
            }
        }
        let done = if follow {
            "Added to Your Library."
        } else {
            "Removed from Your Library."
        };
        self.write(
            Write::Follow {
                uri: info.uri,
                follow,
            },
            Some(done.to_owned()),
        );
    }

    pub fn remove_playlist(&mut self, uri: &str) {
        let Some(canonical) = playlist::canonical(uri) else {
            return;
        };
        if let Page::Collection(open) = &self.page
            && playlist::canonical(open).as_deref() == Some(canonical.as_str())
        {
            self.navigate(Page::Home);
        }
        self.follow_playlist(&canonical, false);
    }

    pub fn delete_playlist(&mut self, uri: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.playlist_info(uri) else {
            return;
        };
        let detail = format!("This will delete {} from Your Library.", info.name);
        let answer = window.prompt(
            PromptLevel::Warning,
            "Delete from Your Library?",
            Some(&detail),
            &[PromptButton::new("Delete"), PromptButton::cancel("Cancel")],
            cx,
        );
        let uri = info.uri;
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |app, cx| {
                    app.remove_playlist(&uri);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn set_playlist_private(
        &mut self,
        uri: &str,
        private: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(info) = self.playlist_info(uri) else {
            return;
        };
        if !private {
            self.apply_private(info.uri, false);
            return;
        }
        let title = format!("Make '{}' private?", info.name);
        let answer = window.prompt(
            PromptLevel::Info,
            &title,
            Some("Only you and people you invite will be able to view your playlist."),
            &[
                PromptButton::new("Make private"),
                PromptButton::cancel("Cancel"),
            ],
            cx,
        );
        let uri = info.uri;
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |app, cx| {
                    app.apply_private(uri, true);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn apply_private(&mut self, uri: String, private: bool) {
        let level = if private {
            Level::Blocked
        } else {
            Level::Viewer
        };
        self.playlists.bases.insert(uri.clone(), level);
        let done = if private {
            "Playlist has been made private."
        } else {
            "Playlist has been made public."
        };
        self.write(Write::Private { uri, private }, Some(done.to_owned()));
    }

    pub fn set_playlist_on_profile(&mut self, uri: &str, public: bool) {
        let Some(canonical) = playlist::canonical(uri) else {
            return;
        };
        if let Some(entry) = self
            .playlists
            .rootlist
            .as_mut()
            .and_then(|list| list.playlist_mut(&canonical))
        {
            entry.public = public;
        }
        let done = if public {
            "Playlist is now displayed on your profile."
        } else {
            "Playlist is no longer displayed on your profile."
        };
        self.write(
            Write::Profile {
                uri: canonical,
                public,
            },
            Some(done.to_owned()),
        );
    }

    pub fn invite_collaborators(&mut self, uri: String) {
        if let Some(uri) = playlist::canonical(&uri) {
            self.write(Write::Invite { uri }, None);
        }
    }

    pub fn set_playlist_member(&mut self, uri: String, username: String, level: Level) {
        let done = if level == Level::Contributor {
            "User is now a collaborator on this playlist."
        } else {
            "User is now a listener on this playlist."
        };
        self.write(
            Write::Member {
                uri,
                username,
                level,
            },
            Some(done.to_owned()),
        );
    }

    pub fn remove_playlist_member(&mut self, uri: String, username: String) {
        self.write(
            Write::RemoveMember { uri, username },
            Some("You removed the user from this playlist.".to_owned()),
        );
    }

    pub fn leave_playlist(&mut self, uri: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.playlist_info(&uri) else {
            return;
        };
        let contributor = self
            .playlists
            .permissions
            .get(&info.uri)
            .zip(self.playlists.user.as_deref())
            .and_then(|(permissions, user)| permissions.member(user))
            .is_some_and(|member| member.level == Some(Level::Contributor));
        let detail = if info.base == Some(Level::Blocked) {
            "This is a private playlist. If you leave you will no longer have access to it."
        } else if contributor {
            "If you leave this playlist, you won’t be able to add songs to it."
        } else {
            "If this playlist is made private you won’t have access to it."
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            "Are you sure?",
            Some(detail),
            &[
                PromptButton::new("Leave Playlist"),
                PromptButton::cancel("Not now"),
            ],
            cx,
        );
        let uri = info.uri;
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |app, cx| {
                    app.write(
                        Write::Leave { uri },
                        Some("You left the playlist.".to_owned()),
                    );
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn edit_details(&mut self, uri: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.playlist_info(uri) else {
            return;
        };
        if !info.capabilities.edit_metadata {
            return;
        }
        let open = self.open_playlist(&info.uri);
        let root = self
            .playlists
            .rootlist
            .as_ref()
            .and_then(|list| list.playlist(&info.uri));
        let draft = Draft {
            description: open
                .map(|collection| collection.description.clone())
                .or_else(|| root.map(|playlist| playlist.description.clone()))
                .unwrap_or_default(),
            cover: open
                .and_then(|collection| collection.cover.clone())
                .or_else(|| root.and_then(|playlist| playlist.cover.clone())),
            custom: open
                .map(|collection| collection.picture.is_some())
                .or_else(|| root.map(|playlist| playlist.picture.is_some()))
                .unwrap_or_default(),
            name: info.name,
            uri: info.uri,
        };
        let app = cx.weak_entity();
        let edit = cx.new(|cx| EditDetails::new(app, draft, window, cx));
        dialogs::open_edit_details(window, cx, edit.clone());
        edit.update(cx, |edit, cx| edit.focus(window, cx));
    }

    pub fn show_collaborators(&mut self, uri: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(info) = self.playlist_info(uri) else {
            return;
        };
        self.send(Command::Permissions(info.uri.clone()));
        let app = cx.weak_entity();
        let view = cx.new(|_| Collaborators::new(app, info.uri.clone()));
        let title: SharedString = if info.name.is_empty() {
            "Playlist permissions".into()
        } else {
            info.name.into()
        };
        dialogs::open_collaborators(window, cx, title, view);
    }

    pub fn save_details(
        &mut self,
        uri: String,
        details: Details,
        cover: Cover,
        cx: &mut Context<Self>,
    ) {
        if let Some((_, Ok(collection))) = &mut self.playlists.collection
            && collection.uri == uri
        {
            if let Some(name) = &details.name {
                collection.name = name.trim().to_owned();
            }
            if let Some(description) = &details.description {
                collection.description = description.clone().unwrap_or_default();
            }
        }
        if let (Some(name), Some(entry)) = (
            &details.name,
            self.playlists
                .rootlist
                .as_mut()
                .and_then(|list| list.playlist_mut(&uri)),
        ) {
            entry.name = name.trim().to_owned();
        }
        self.write(
            Write::Details {
                uri,
                details,
                cover,
            },
            None,
        );
        cx.notify();
    }

    pub fn shuffle_collection(&mut self, cx: &mut Context<Self>) {
        let Some((uri, Ok(collection))) = &self.playlists.collection else {
            return;
        };
        if collection.tracks.is_empty() {
            return;
        }
        let (uri, tracks) = (uri.clone(), collection.tracks.clone());
        let options = PlayOptions {
            shuffle_override: Some(Shuffle::On),
            ..PlayOptions::default()
        };
        let index = random_index(tracks.len());
        self.play_context_from(&uri, &tracks, index, connect::ORIGIN, options, cx);
    }

    pub fn add_to_playlist(&mut self, uri: &str, uris: Vec<String>, position: ItemPosition) {
        let Some(info) = self.playlist_info(uri) else {
            return;
        };
        if uris.is_empty() {
            return;
        }
        let done = format!("Added to {}", info.name);
        self.write(
            Write::Add {
                uri: info.uri,
                uris,
                position,
            },
            Some(done),
        );
    }

    pub fn remove_from_playlist(&mut self, uri: &str, uids: Vec<String>) {
        let Some(info) = self.playlist_info(uri) else {
            return;
        };
        if uids.is_empty() {
            return;
        }
        if let Some((_, Ok(collection))) = &mut self.playlists.collection
            && collection.uri == info.uri
        {
            let before = collection.tracks.len();
            collection
                .tracks
                .retain(|track| track.uid.as_ref().is_none_or(|uid| !uids.contains(uid)));
            collection.total = collection
                .total
                .saturating_sub(before - collection.tracks.len());
        }
        let done = format!("Removed from {}.", info.name);
        self.write(
            Write::Remove {
                uri: info.uri,
                uids,
            },
            Some(done),
        );
    }

    pub fn move_in_playlist(&mut self, uri: &str, uids: Vec<String>, position: ItemPosition) {
        if let Some(uri) = playlist::canonical(uri)
            && !uids.is_empty()
        {
            self.write(
                Write::Move {
                    uri,
                    uids,
                    position,
                },
                None,
            );
        }
    }
}

fn after_write(app: &mut OpenMusic, uri: Option<String>, rootlist: bool) {
    if rootlist {
        app.refresh_rootlist();
    }
    if let Some(uri) = uri {
        app.reload_playlist(&uri);
    }
}

pub fn load(app: &mut OpenMusic, page: &Page) {
    if let Page::Collection(uri) = page {
        if app
            .playlists
            .collection
            .as_ref()
            .is_none_or(|(loaded, _)| loaded != uri)
        {
            app.playlists.collection = None;
            app.playlists.loading_more = false;
            app.send(Command::Collection(uri.clone()));
        }
        if let Some(canonical) = playlist::canonical(uri)
            && !playlist::is_virtual(&canonical)
        {
            app.send(Command::Permissions(canonical));
        }
    }
}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::Collection { uri, result } => {
            if app.page != Page::Collection(uri.clone()) {
                return;
            }
            app.playlists.loading_more = false;
            let edit = result.is_ok()
                && app.playlists.edit_after_load.is_some()
                && app.playlists.edit_after_load == playlist::canonical(&uri);
            app.playlists.collection = Some((uri.clone(), result.map(|collection| *collection)));
            if edit {
                app.playlists.edit_after_load = None;
                app.edit_details(&uri, window, cx);
            }
        }
        Event::More { uri, from, result } => {
            app.playlists.loading_more = false;
            let Some((loaded, Ok(collection))) = &mut app.playlists.collection else {
                return;
            };
            if *loaded != uri || collection.next != Some(from) {
                return;
            }
            match result {
                Ok((tracks, next)) => {
                    collection.tracks.extend(tracks);
                    collection.next = next;
                }
                Err(message) => {
                    collection.next = None;
                    app.show_toast(message, None);
                }
            }
        }
        Event::Rootlist(None) => {}
        Event::Rootlist(Some((user, rootlist))) => {
            let known = &app.playlists.bases;
            app.playlists.base_queue = rootlist
                .playlists()
                .filter(|entry| {
                    entry.owner.as_deref() == Some(user.as_str())
                        && entry.capabilities.administrate
                        && !known.contains_key(&entry.uri)
                })
                .map(|entry| entry.uri.clone())
                .take(BASE_PREFETCH)
                .collect();
            app.playlists.user = Some(user);
            app.playlists.rootlist = Some(rootlist);
        }
        Event::Permissions { uri, permissions } => {
            if let Some(canonical) = playlist::canonical(&uri) {
                if let Some(base) = permissions.base {
                    app.playlists.bases.insert(canonical.clone(), base);
                }
                app.playlists.permissions.insert(canonical, permissions);
            }
        }
        Event::Base { uri, level } => {
            if let (Some(canonical), Some(level)) = (playlist::canonical(&uri), level) {
                app.playlists.bases.insert(canonical, level);
            }
        }
        Event::Created { uri, edit } => {
            app.refresh_rootlist();
            if edit {
                app.playlists.edit_after_load = Some(uri.clone());
            }
            app.navigate(Page::Collection(uri));
        }
        Event::Invited { link } => {
            cx.write_to_clipboard(ClipboardItem::new_string(link));
            app.show_toast("Link copied to clipboard", None);
        }
        Event::Changed {
            uri,
            rootlist,
            done,
        } => {
            after_write(app, uri, rootlist);
            if let Some(done) = done {
                app.show_toast(done, None);
            }
        }
        Event::Failed {
            uri,
            rootlist,
            message,
        } => {
            if let Some(uri) = &uri
                && let Some(canonical) = playlist::canonical(uri)
            {
                app.playlists.bases.remove(&canonical);
            }
            after_write(app, uri, rootlist);
            app.show_toast(message, None);
        }
    }
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {
    app.send(Command::Rootlist);
}

pub fn signed_out(app: &mut OpenMusic) {
    app.playlists = State::default();
}

fn switch(text: &str) -> Option<bool> {
    match text {
        "on" | "true" | "yes" => Some(true),
        "off" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn item_position(text: &str) -> Option<ItemPosition> {
    match text.split_once(':') {
        None if text == "top" => Some(ItemPosition::Top),
        None if text == "bottom" => Some(ItemPosition::Bottom),
        Some(("before", uid)) if !uid.is_empty() => Some(ItemPosition::BeforeUid(uid.to_owned())),
        Some(("after", uid)) if !uid.is_empty() => Some(ItemPosition::AfterUid(uid.to_owned())),
        _ => None,
    }
}

fn flag(value: bool) -> u8 {
    u8::from(value)
}

pub fn describe_playlist(app: &OpenMusic) -> String {
    let Some((loaded, result)) = &app.playlists.collection else {
        return "collection=none".to_owned();
    };
    let collection = match result {
        Ok(collection) => collection,
        Err(error) => return format!("collection={loaded} error={error}"),
    };
    let info = app.playlist_info(&collection.uri).unwrap_or_default();
    let capabilities = collection.capabilities;
    let mut lines = vec![
        format!(
            "collection={loaded} kind={} name={:?} total={} loaded={} next={:?} owner={:?} revision={:?}",
            collection.kind,
            collection.name,
            collection.total,
            collection.tracks.len(),
            collection.next,
            collection.owner,
            collection.revision,
        ),
        format!(
            "capabilities view={} edit_items={} edit_metadata={} administrate={} publish={} delete={} cancel_membership={}",
            flag(capabilities.view),
            flag(capabilities.edit_items),
            flag(capabilities.edit_metadata),
            flag(capabilities.administrate),
            flag(capabilities.publish),
            flag(capabilities.delete),
            flag(capabilities.cancel_membership),
        ),
        format!(
            "owned={} saved={:?} profile={} base={:?} members={} picture={} description={:?}",
            flag(info.owned),
            info.saved,
            flag(info.on_profile),
            info.base,
            info.members,
            flag(collection.picture.is_some()),
            collection.description,
        ),
    ];
    lines.extend(collection.tracks.iter().enumerate().map(|(index, track)| {
        format!(
            "{index} {} {} {:?}",
            track.uid.as_deref().unwrap_or("-"),
            track.uri,
            track.title
        )
    }));
    lines.join("\n")
}

pub fn describe_rootlist(app: &OpenMusic) -> String {
    let Some(list) = &app.playlists.rootlist else {
        return "rootlist=none".to_owned();
    };
    let mut lines = vec![format!(
        "revision={} entries={} playlists={} writable={} default_name={:?} user={:?}",
        list.revision,
        list.entries.len(),
        list.playlists().count(),
        list.writable_count(),
        list.default_name(),
        app.playlists.user,
    )];
    lines.extend(
        list.entries
            .iter()
            .enumerate()
            .map(|(index, entry)| match entry {
                RootEntry::Playlist(playlist) => format!(
                    "{index} playlist {} public={} edit={} owner={:?} base={:?} name={:?}",
                    playlist.uri,
                    flag(playlist.public),
                    flag(playlist.capabilities.edit_items),
                    playlist.owner,
                    app.playlists.bases.get(&playlist.uri),
                    playlist.name,
                ),
                RootEntry::Start { hash, name } => format!("{index} start {hash} {name:?}"),
                RootEntry::End { hash } => format!("{index} end {hash}"),
                RootEntry::Other { uri } => format!("{index} other {uri}"),
            }),
    );
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
    let words: Vec<&str> = rest.split_whitespace().collect();
    let tail = |skip: usize| {
        words
            .iter()
            .skip(skip)
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let write = |name: &str, text: String| {
        let _ = std::fs::write(directory.join(name), text);
    };
    match (verb, words.as_slice()) {
        ("playlist", []) => write("playlist.txt", describe_playlist(app)),
        ("rootlist", []) => write("rootlist.txt", describe_rootlist(app)),
        ("rootlist", ["reload"]) => app.refresh_rootlist(),
        ("playlist-create", _) => {
            let name = Some(rest.to_owned()).filter(|name| !name.is_empty());
            app.create_playlist(name, None, Vec::new(), false);
        }
        ("playlist-rename", [uri, _, ..]) => {
            let details = Details {
                name: Some(tail(1)),
                ..Details::default()
            };
            app.save_details((*uri).to_owned(), details, Cover::Keep, cx);
        }
        ("playlist-describe", [uri, ..]) => {
            let text = tail(1);
            let details = Details {
                description: Some(Some(text).filter(|text| !text.is_empty())),
                ..Details::default()
            };
            app.save_details((*uri).to_owned(), details, Cover::Keep, cx);
        }
        ("playlist-cover", [uri, "none"]) => {
            app.save_details((*uri).to_owned(), Details::default(), Cover::Remove, cx);
        }
        ("playlist-cover", [uri, _, ..]) => {
            let (uri, path) = ((*uri).to_owned(), std::path::PathBuf::from(tail(1)));
            let report = directory.join("playlist-cover.txt");
            cx.spawn_in(window, async move |this, cx| {
                let jpeg = cx
                    .background_executor()
                    .spawn(async move { image::cover_jpeg(&path) })
                    .await;
                let _ = std::fs::write(
                    &report,
                    match &jpeg {
                        Ok(bytes) => format!("ok {} bytes", bytes.len()),
                        Err(message) => format!("error {message}"),
                    },
                );
                if let Ok(jpeg) = jpeg {
                    let _ = this.update(cx, |app, cx| {
                        app.save_details(uri, Details::default(), Cover::Set(jpeg), cx)
                    });
                }
            })
            .detach();
        }
        ("playlist-private", [uri, value]) => {
            if let Some(private) = switch(value) {
                app.apply_private((*uri).to_owned(), private);
            }
        }
        ("playlist-profile", [uri, value]) => {
            if let Some(public) = switch(value) {
                app.set_playlist_on_profile(uri, public);
            }
        }
        ("playlist-follow", [uri, value]) => {
            if let Some(follow) = switch(value) {
                app.follow_playlist(uri, follow);
            }
        }
        ("playlist-delete", [uri]) => app.remove_playlist(uri),
        ("playlist-invite", [uri]) => app.invite_collaborators((*uri).to_owned()),
        ("playlist-member", [uri, username, level]) => {
            let level = match *level {
                "contributor" | "collaborator" => Level::Contributor,
                _ => Level::Viewer,
            };
            app.set_playlist_member((*uri).to_owned(), (*username).to_owned(), level);
        }
        ("playlist-unmember", [uri, username]) => {
            app.remove_playlist_member((*uri).to_owned(), (*username).to_owned())
        }
        ("playlist-leave", [uri]) => {
            if let Some(uri) = playlist::canonical(uri) {
                app.write(Write::Leave { uri }, Some("You left the playlist.".into()));
            }
        }
        ("playlist-add", [uri, uris @ ..]) if !uris.is_empty() => {
            let uris = uris.iter().map(|uri| (*uri).to_owned()).collect();
            app.add_to_playlist(uri, uris, ItemPosition::Bottom);
        }
        ("playlist-remove", [uri, uids @ ..]) if !uids.is_empty() => {
            let uids = uids.iter().map(|uid| (*uid).to_owned()).collect();
            app.remove_from_playlist(uri, uids);
        }
        ("playlist-move", [uri, uid, position]) => {
            if let Some(position) = item_position(position) {
                app.move_in_playlist(uri, vec![(*uid).to_owned()], position);
            }
        }
        ("playlist-edit", [uri]) => app.edit_details(uri, window, cx),
        ("playlist-members", [uri]) => app.show_collaborators(uri, window, cx),
        ("playlist-shuffle", []) => app.shuffle_collection(cx),
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    let rootlist = app
        .playlists
        .rootlist
        .as_ref()
        .map_or("-".to_owned(), |list| list.playlists().count().to_string());
    match &app.playlists.collection {
        Some((_, Ok(collection))) if collection.kind == "PLAYLIST" => format!(
            "rootlist={rootlist} rows={}/{}",
            collection.tracks.len(),
            collection.total
        ),
        _ => format!("rootlist={rootlist}"),
    }
}

pub fn register_actions(cx: &mut App, handle: &Handle) {
    menus::route(cx, handle, |_: &menus::NewPlaylist, app, _, cx| {
        app.create_playlist(None, None, Vec::new(), true);
        cx.notify();
    });
}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        "play-collection" => {
            if let Some((uri, Ok(collection))) = &app.playlists.collection {
                let (uri, tracks) = (uri.clone(), collection.tracks.clone());
                app.play_context(&uri, &tracks, 0, cx);
            }
        }
        "shuffle-collection" => app.shuffle_collection(cx),
        "follow-collection" => {
            if let Page::Collection(uri) = &app.page
                && let Some(info) = app.playlist_info(&uri.clone())
                && let Some(saved) = info.saved
            {
                app.follow_playlist(&info.uri, !saved);
            }
        }
        "more-collection" => {
            if let Page::Collection(uri) = &app.page
                && let Some(target) = Target::for_uri(uri)
            {
                app.open_menu_under(target, key);
            }
        }
        _ => return false,
    }
    true
}

pub fn row_dropped(
    _app: &mut OpenMusic,
    _drag: &RowDrag,
    _at: RowDrop,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
}

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    if app.playlists.sentinel.take()
        && !app.playlists.loading_more
        && let Some((uri, Ok(collection))) = &app.playlists.collection
        && let Some(from) = collection.next
        && app.page == Page::Collection(uri.clone())
    {
        let uri = uri.clone();
        app.playlists.loading_more = true;
        app.send(Command::More { uri, from });
        cx.notify();
    }
    app.playlists.base_wait = app.playlists.base_wait.saturating_sub(1);
    if app.playlists.base_wait == 0
        && let Some(uri) = app.playlists.base_queue.pop_front()
    {
        if !app.playlists.bases.contains_key(&uri) {
            app.send(Command::Base(uri));
        }
        app.playlists.base_wait = BASE_SPACING;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_owners_by_name() {
        assert_eq!(owner_label("spotify", None), "Spotify");
        assert_eq!(owner_label("someone", Some("Some One")), "Some One");
        assert_eq!(owner_label("someone", Some("")), "someone");
    }

    #[test]
    fn parses_debug_positions() {
        assert_eq!(item_position("top"), Some(ItemPosition::Top));
        assert_eq!(item_position("bottom"), Some(ItemPosition::Bottom));
        assert_eq!(
            item_position("before:abc"),
            Some(ItemPosition::BeforeUid("abc".into()))
        );
        assert_eq!(
            item_position("after:abc"),
            Some(ItemPosition::AfterUid("abc".into()))
        );
        assert_eq!(item_position("after:"), None);
        assert_eq!(item_position("middle"), None);
        assert_eq!(switch("on"), Some(true));
        assert_eq!(switch("off"), Some(false));
        assert_eq!(switch("maybe"), None);
    }

    #[test]
    fn picks_a_shuffle_start_inside_the_list() {
        for length in [1, 2, 7, 100] {
            assert!(random_index(length) < length);
        }
        assert_eq!(random_index(0), 0);
    }
}
