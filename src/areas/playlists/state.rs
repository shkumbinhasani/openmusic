use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use gpui::{
    App, AppContext, ClipboardItem, Context, PromptButton, PromptLevel, SharedString, Subscription,
    WeakEntity, Window, point,
};
use gpui_component::WindowExt;
use spotify_client::{
    Track,
    blend::{self, Invitation, InvitationState},
    catalog::{Capabilities, Collection},
    connect::{self, PlayOptions, Shuffle},
    playlist::{
        self, Details, EditTarget, Folder, ItemPosition, Level, Permissions, Push, RootEntry,
        RootNode, RootPlace, RootPlaylist, Rootlist, TargetKind,
    },
};

use super::{
    dialogs::{self, BlendMembers, Collaborators, Draft, EditDetails, FolderName, SaveTo},
    image,
    view::{ADD_MENU, AddSource, LibraryDrag, TRACK_ROW, add_menu, create_menu},
    worker::{Command, Cover, Event, Ready, Write, blend_faces},
};
use crate::{
    app::{OpenMusic, Page},
    areas::library,
    menus::{self, Handle},
    ui::{
        components::Sentinel,
        menu::{Place, Target},
        tracks::{RowDrag, RowDrop},
    },
};

const BASE_PREFETCH: usize = 50;
const BASE_SPACING: u32 = 4;
const EDITABLE_FRESH: Duration = Duration::from_secs(60);
const EDITABLE_WAIT: Duration = Duration::from_secs(30);
const EDITABLE_KEEP: usize = 16;
const PUSH_DELAY: Duration = Duration::from_millis(500);
const FOCUS_REFRESH: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlendWork {
    Inviting,
    Joining(String),
}

#[derive(Default)]
pub struct Pushes {
    pub due: Option<Instant>,
    pub rootlist: bool,
    pub playlists: HashSet<String>,
    pub seen: usize,
}

impl Pushes {
    fn waiting(&self) -> bool {
        self.rootlist || !self.playlists.is_empty()
    }
}

pub fn outdated(pushed: Option<&str>, current: Option<&str>) -> bool {
    pushed.is_none() || pushed != current
}

pub struct Editable {
    pub at: Instant,
    pub result: Option<Result<Vec<EditTarget>, String>>,
}

impl Editable {
    fn fresh(&self) -> bool {
        let limit = match self.result {
            None => EDITABLE_WAIT,
            Some(_) => EDITABLE_FRESH,
        };
        self.at.elapsed() < limit
    }
}

pub struct Snapshot {
    pending: usize,
    tracks: Vec<Track>,
    total: usize,
}

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
    pub editable: HashMap<Vec<String>, Editable>,
    pub wanted: RefCell<Option<AddSource>>,
    pub menu_source: Option<AddSource>,
    pub snapshots: HashMap<String, Snapshot>,
    pub save_to: Option<WeakEntity<SaveTo>>,
    pub invitation: Option<(String, Result<Invitation, String>)>,
    pub blend_work: Option<BlendWork>,
    pub pushes: Pushes,
    pub activation: Option<Subscription>,
    pub refreshed: Option<Instant>,
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
            urls.extend(blend_faces(collection));
        }
        if let Some((_, Ok(invitation))) = &self.invitation {
            urls.extend(
                invitation
                    .members
                    .iter()
                    .chain(invitation.recipient.iter())
                    .filter_map(|member| member.image.clone()),
            );
        }
        if let Some(sheet) = &self.save_to
            && sheet.upgrade().is_some()
        {
            urls.extend(
                self.editable
                    .values()
                    .filter_map(|editable| editable.result.as_ref()?.as_ref().ok())
                    .flatten()
                    .filter_map(|target| target.cover.clone()),
            );
        }
    }

    pub fn editable_for(&self, uris: &[String]) -> Option<&Result<Vec<EditTarget>, String>> {
        self.editable.get(uris)?.result.as_ref()
    }

    pub fn curates(&self, uris: &[String]) -> HashMap<String, bool> {
        match self.editable_for(uris) {
            Some(Ok(targets)) => targets
                .iter()
                .filter(|target| target.kind != TargetKind::Folder)
                .map(|target| (target.uri.clone(), target.curates))
                .collect(),
            _ => HashMap::new(),
        }
    }

    pub fn folder(&self, uri: &str) -> Option<Folder> {
        fn find(nodes: Vec<RootNode>, hash: &str) -> Option<Folder> {
            nodes.into_iter().find_map(|node| match node {
                RootNode::Folder(folder) if folder.hash == hash => Some(folder),
                RootNode::Folder(folder) => find(folder.children, hash),
                RootNode::Playlist { .. } => None,
            })
        }
        let hash = playlist::folder_hash(uri)?;
        find(self.rootlist.as_ref()?.tree(self.user.as_deref()?), hash)
    }

    pub fn folders(&self) -> Vec<(usize, Folder)> {
        fn walk(nodes: Vec<RootNode>, depth: usize, out: &mut Vec<(usize, Folder)>) {
            for node in nodes {
                if let RootNode::Folder(mut folder) = node {
                    let children = std::mem::take(&mut folder.children);
                    out.push((depth, folder));
                    walk(children, depth + 1, out);
                }
            }
        }
        let mut out = Vec::new();
        if let (Some(list), Some(user)) = (&self.rootlist, &self.user) {
            walk(list.tree(user), 0, &mut out);
        }
        out
    }

    pub fn parent_folder(&self, uri: &str) -> Option<String> {
        let hash = self.rootlist.as_ref()?.parent_of(uri)?;
        Some(playlist::folder_uri(self.user.as_deref()?, &hash))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct DuplicateQuestion {
    pub title: &'static str,
    pub detail: String,
    pub answers: Vec<&'static str>,
    pub cancel: &'static str,
    pub some: bool,
}

impl DuplicateQuestion {
    pub fn new(name: &str, total: usize, duplicates: usize) -> Self {
        let some = duplicates < total;
        let (title, detail) = if total == 1 {
            (
                "Already added",
                format!("This is already in your '{name}' playlist."),
            )
        } else if some {
            (
                "Some already added",
                format!("Some of these are already in your '{name}' playlist."),
            )
        } else {
            (
                "Already added",
                format!("These are already in your '{name}' playlist."),
            )
        };
        let (answers, cancel) = if some {
            (vec!["Add new ones", "Add all"], "Cancel")
        } else {
            (vec!["Add anyway"], "Don't add")
        };
        Self {
            title,
            detail,
            answers,
            cancel,
            some,
        }
    }
}

fn row_uid(track: &Track) -> Option<&str> {
    track.uid.as_deref()
}

pub fn source_name(app: &OpenMusic, uris: &[String]) -> Option<String> {
    let [uri] = uris else {
        return None;
    };
    if let Some(track) = app
        .playlists
        .collection
        .as_ref()
        .and_then(|(_, result)| result.as_ref().ok())
        .and_then(|collection| {
            if collection.uri == *uri {
                return Some(collection.name.clone());
            }
            collection
                .tracks
                .iter()
                .find(|track| track.uri == *uri)
                .map(|track| track.title.clone())
        })
    {
        return Some(track);
    }
    if let Some(track) = app.track().filter(|track| track.uri == *uri) {
        return Some(track.title.clone());
    }
    app.playlists
        .rootlist
        .as_ref()
        .and_then(|list| list.playlist(uri))
        .map(|playlist| playlist.name.clone())
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
                open: true,
                edit,
            },
            None,
        );
    }

    pub fn create_playlist_from(
        &mut self,
        sources: Vec<String>,
        name: Option<String>,
        folder: Option<String>,
    ) {
        let name = name.or_else(|| source_name(self, &sources));
        self.write(
            Write::Create {
                name,
                folder,
                tracks: sources,
                open: false,
                edit: false,
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

    pub fn add_to_playlist(&mut self, uri: &str, sources: Vec<String>, position: ItemPosition) {
        let Some(info) = self.playlist_info(uri) else {
            return;
        };
        if sources.is_empty() {
            return;
        }
        let known = self
            .open_playlist(&info.uri)
            .filter(|collection| collection.next.is_none())
            .map(|collection| {
                collection
                    .tracks
                    .iter()
                    .map(|track| track.uri.clone())
                    .collect()
            });
        self.send(Command::PrepareAdd {
            target: info.uri,
            name: info.name,
            sources,
            position,
            known,
        });
    }

    fn write_add(
        &mut self,
        target: String,
        name: String,
        uris: Vec<String>,
        position: ItemPosition,
    ) {
        if uris.is_empty() {
            return;
        }
        self.playlists.editable.clear();
        self.write(
            Write::Add {
                uri: target,
                uris,
                position,
            },
            Some(format!("Added to {name}")),
        );
    }

    fn confirm_duplicates(&mut self, ready: Ready, window: &mut Window, cx: &mut Context<Self>) {
        let Ready {
            target,
            name,
            uris,
            duplicates,
            position,
        } = ready;
        let question = DuplicateQuestion::new(&name, uris.len(), duplicates.len());
        let some = question.some;
        let mut buttons: Vec<PromptButton> = question
            .answers
            .iter()
            .map(|answer| PromptButton::new(*answer))
            .collect();
        buttons.push(PromptButton::cancel(question.cancel));
        let answer = window.prompt(
            PromptLevel::Info,
            question.title,
            Some(&question.detail),
            &buttons,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = answer.await else {
                return;
            };
            let chosen = match (some, answer) {
                (true, 0) => {
                    let duplicates: HashSet<&String> = duplicates.iter().collect();
                    uris.iter()
                        .filter(|uri| !duplicates.contains(uri))
                        .cloned()
                        .collect()
                }
                (true, 1) | (false, 0) => uris,
                _ => return,
            };
            let _ = this.update(cx, |app, cx| {
                app.write_add(target, name, chosen, position);
                cx.notify();
            });
        })
        .detach();
    }

    fn snapshot_rows(&mut self, uri: &str) {
        let Some((_, Ok(collection))) = &self.playlists.collection else {
            return;
        };
        if collection.uri != uri {
            return;
        }
        let (tracks, total) = (collection.tracks.clone(), collection.total);
        self.playlists
            .snapshots
            .entry(uri.to_owned())
            .or_insert(Snapshot {
                pending: 0,
                tracks,
                total,
            })
            .pending += 1;
    }

    fn settle_rows(&mut self, uri: &str, failed: bool) {
        if failed {
            let Some(snapshot) = self.playlists.snapshots.remove(uri) else {
                return;
            };
            if let Some((_, Ok(collection))) = &mut self.playlists.collection
                && collection.uri == uri
            {
                collection.tracks = snapshot.tracks;
                collection.total = snapshot.total;
            }
            return;
        }
        if let Some(snapshot) = self.playlists.snapshots.get_mut(uri) {
            snapshot.pending = snapshot.pending.saturating_sub(1);
            if snapshot.pending == 0 {
                self.playlists.snapshots.remove(uri);
            }
        }
    }

    pub fn remove_from_playlist(&mut self, uri: &str, uids: Vec<String>) {
        let Some(info) = self.playlist_info(uri) else {
            return;
        };
        if uids.is_empty() {
            return;
        }
        self.snapshot_rows(&info.uri);
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
        self.playlists.editable.clear();
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
        let Some(uri) = playlist::canonical(uri) else {
            return;
        };
        if uids.is_empty() {
            return;
        }
        self.snapshot_rows(&uri);
        if let Some((_, Ok(collection))) = &mut self.playlists.collection
            && collection.uri == uri
        {
            if position == ItemPosition::Bottom && collection.next.is_some() {
                collection
                    .tracks
                    .retain(|track| track.uid.as_ref().is_none_or(|uid| !uids.contains(uid)));
            } else {
                playlist::reorder(&mut collection.tracks, row_uid, &uids, &position);
            }
        }
        self.write(
            Write::Move {
                uri,
                uids,
                position,
            },
            None,
        );
    }

    pub fn request_editable(&mut self, uris: &[String]) {
        if uris.is_empty()
            || self
                .playlists
                .editable
                .get(uris)
                .is_some_and(Editable::fresh)
        {
            return;
        }
        if self.playlists.editable.len() >= EDITABLE_KEEP
            && let Some(oldest) = self
                .playlists
                .editable
                .iter()
                .min_by_key(|(_, editable)| editable.at)
                .map(|(key, _)| key.clone())
        {
            self.playlists.editable.remove(&oldest);
        }
        self.playlists.editable.insert(
            uris.to_vec(),
            Editable {
                at: Instant::now(),
                result: None,
            },
        );
        self.send(Command::Editable(uris.to_vec()));
    }

    pub fn open_save_to(
        &mut self,
        uris: Vec<String>,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let uris: Vec<String> = uris
            .into_iter()
            .filter(|uri| playlist::addable(uri))
            .collect();
        if uris.is_empty() {
            return;
        }
        if let Some(stale) = self.playlists.editable.get(&uris)
            && stale.result.is_some()
            && !stale.fresh()
        {
            self.playlists.editable.remove(&uris);
        }
        self.request_editable(&uris);
        let app = cx.weak_entity();
        let name = name.or_else(|| source_name(self, &uris));
        let sheet = cx.new(|cx| SaveTo::new(app, uris, name, window, cx));
        self.playlists.save_to = Some(sheet.downgrade());
        dialogs::open_save_to(window, cx, sheet.clone());
        sheet.update(cx, |sheet, cx| sheet.focus(window, cx));
    }

    pub fn apply_curations(&mut self, items: Vec<String>, changes: Vec<(String, bool)>) {
        if items.is_empty() || changes.is_empty() {
            return;
        }
        let name = |uri: &str| -> Option<String> {
            if uri == playlist::LIKED_SONGS {
                return Some("Liked Songs".to_owned());
            }
            self.playlist_info(uri).map(|info| info.name)
        };
        let done = match changes.as_slice() {
            [(uri, true)] => name(uri).map(|name| format!("Added to {name}.")),
            [(uri, false)] => name(uri).map(|name| format!("Removed from {name}.")),
            _ => None,
        }
        .unwrap_or_else(|| "Changes saved.".to_owned());
        self.playlists.editable.clear();
        self.write(Write::Curate { items, changes }, Some(done));
    }

    pub fn create_folder(&mut self, parent: Option<String>, playlists: Vec<String>, rename: bool) {
        self.write(
            Write::CreateFolder {
                name: None,
                parent,
                playlists,
                rename,
            },
            None,
        );
    }

    pub fn rename_folder(
        &mut self,
        uri: &str,
        name: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = name.or_else(|| self.playlists.folder(uri).map(|folder| folder.name))
        else {
            return;
        };
        let app = cx.weak_entity();
        let uri = uri.to_owned();
        let view = cx.new(|cx| FolderName::new(app, uri, name, window, cx));
        dialogs::open_folder_name(window, cx, view.clone());
        view.update(cx, |view, cx| view.focus(window, cx));
    }

    pub fn save_folder_name(&mut self, uri: String, name: String) {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            return;
        }
        if let (Some(list), Some(hash)) = (
            self.playlists.rootlist.as_mut(),
            playlist::folder_hash(&uri),
        ) {
            for entry in &mut list.entries {
                if let RootEntry::Start {
                    hash: found,
                    name: old,
                } = entry
                    && found == hash
                {
                    *old = name.clone();
                }
            }
        }
        self.write(Write::RenameFolder { uri, name }, None);
    }

    pub fn delete_folder(&mut self, uri: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.playlists.folder(uri).is_none() {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Do you really want to delete this folder and all playlists inside?",
            None,
            &[PromptButton::new("Delete"), PromptButton::cancel("Cancel")],
            cx,
        );
        let uri = uri.to_owned();
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |app, cx| {
                    app.remove_folder(uri);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub fn remove_folder(&mut self, uri: String) {
        if let Page::Folder(open) = &self.page
            && playlist::folder_hash(open) == playlist::folder_hash(&uri)
        {
            self.navigate(Page::Home);
        }
        self.write(Write::DeleteFolder { uri }, None);
    }

    pub fn move_to(&mut self, uri: &str, to: RootPlace) {
        if playlist::folder_hash(uri).is_some() {
            self.write(
                Write::MoveFolder {
                    uri: uri.to_owned(),
                    to,
                },
                None,
            );
        } else if let Some(uri) = playlist::canonical(uri) {
            self.write(
                Write::MovePlaylists {
                    uris: vec![uri],
                    to,
                },
                None,
            );
        }
    }

    pub fn drop_library(&mut self, drag: &LibraryDrag, target: &str, after: bool) {
        if drag.uri == target {
            return;
        }
        let place = match (playlist::folder_hash(target).is_some(), after) {
            (true, true) => RootPlace::Into(target.to_owned()),
            (false, true) => RootPlace::After(target.to_owned()),
            (_, false) => RootPlace::Before(target.to_owned()),
        };
        self.move_to(&drag.uri, place);
    }

    fn drop_on_rows(&mut self, drag: &RowDrag, at: RowDrop) {
        let Some((_, Ok(collection))) = &self.playlists.collection else {
            return;
        };
        let uri = collection.uri.clone();
        if !self
            .playlist_info(&uri)
            .is_some_and(|info| info.capabilities.edit_items)
        {
            return;
        }
        let Some(anchor) = collection
            .tracks
            .get(at.row.1)
            .and_then(|track| track.uid.clone())
        else {
            return;
        };
        let position = if at.after {
            ItemPosition::AfterUid(anchor.clone())
        } else {
            ItemPosition::BeforeUid(anchor.clone())
        };
        let same = drag.source.0 == TRACK_ROW
            && drag
                .context
                .as_deref()
                .and_then(playlist::canonical)
                .as_deref()
                == Some(uri.as_str());
        if !same {
            let uris = drag
                .uris
                .iter()
                .filter(|uri| playlist::addable(uri))
                .cloned()
                .collect();
            self.add_to_playlist(&uri, uris, position);
            return;
        }
        let uids: Vec<String> = drag.uids.iter().flatten().cloned().collect();
        let mut order: Vec<&str> = collection
            .tracks
            .iter()
            .filter_map(|track| track.uid.as_deref())
            .collect();
        let before = order.clone();
        fn same_uid<'a>(uid: &'a &str) -> Option<&'a str> {
            Some(uid)
        }
        if uids.is_empty()
            || !playlist::reorder(&mut order, same_uid, &uids, &position)
            || order == before
        {
            return;
        }
        self.move_in_playlist(&uri, uids, position);
    }

    fn drop_on_sidebar(&mut self, drag: &RowDrag, index: usize) {
        let Some(target) = self
            .library
            .playlists
            .get(index)
            .map(|playlist| playlist.uri.clone())
        else {
            return;
        };
        let uris: Vec<String> = drag
            .uris
            .iter()
            .filter(|uri| playlist::addable(uri))
            .cloned()
            .collect();
        let editable = self
            .playlist_info(&target)
            .is_some_and(|info| info.capabilities.edit_items);
        if editable && !uris.is_empty() {
            self.add_to_playlist(&target, uris, ItemPosition::Bottom);
        }
    }
}

impl OpenMusic {
    pub fn on_playlist_push(&mut self, push: Push) {
        self.playlists.pushes.seen += 1;
        self.playlists.editable.clear();
        match push {
            Push::Rootlist { user, revision } => {
                if self.playlists.user.as_deref().is_some_and(|me| me != user) {
                    return;
                }
                let current = self
                    .playlists
                    .rootlist
                    .as_ref()
                    .map(|list| list.revision.as_str());
                if outdated(revision.as_deref(), current) {
                    self.playlists.pushes.rootlist = true;
                }
            }
            Push::Playlist {
                uri,
                revision,
                listed,
            } => {
                if listed
                    && self
                        .playlists
                        .rootlist
                        .as_ref()
                        .is_some_and(|list| list.contains(&uri))
                {
                    self.playlists.pushes.rootlist = true;
                }
                let stale = self.open_playlist(&uri).is_some_and(|collection| {
                    outdated(revision.as_deref(), collection.revision.as_deref())
                });
                if stale {
                    self.playlists.pushes.playlists.insert(uri);
                }
            }
        }
        if self.playlists.pushes.waiting() {
            self.playlists.pushes.due = Some(Instant::now() + PUSH_DELAY);
        }
    }

    fn apply_pushes(&mut self) {
        let pushes = &mut self.playlists.pushes;
        if pushes.due.is_none_or(|due| Instant::now() < due) {
            return;
        }
        pushes.due = None;
        let rootlist = std::mem::take(&mut pushes.rootlist);
        let playlists = std::mem::take(&mut pushes.playlists);
        if rootlist {
            self.refresh_rootlist();
        }
        for uri in playlists {
            if !self.playlists.snapshots.contains_key(&uri) {
                self.reload_playlist(&uri);
            }
        }
    }

    pub fn refresh_on_focus(&mut self) {
        if self.playlists.user.is_none()
            || self
                .playlists
                .refreshed
                .is_some_and(|at| at.elapsed() < FOCUS_REFRESH)
        {
            return;
        }
        self.playlists.refreshed = Some(Instant::now());
        self.playlists.editable.clear();
        self.refresh_rootlist();
        if let Some((_, Ok(collection))) = &self.playlists.collection
            && collection.kind == "PLAYLIST"
            && !self.playlists.snapshots.contains_key(&collection.uri)
        {
            self.send(Command::Check {
                uri: collection.uri.clone(),
            });
        }
    }

    pub fn create_blend(&mut self) {
        self.navigate(Page::BlendInvitation(String::new()));
    }

    pub fn open_blend_invitation(&mut self, input: &str) -> bool {
        match blend::invitation_id(input) {
            Some(id) => {
                self.navigate(Page::BlendInvitation(id));
                true
            }
            None => false,
        }
    }

    pub fn blend_invite(&mut self) {
        if self.playlists.blend_work.is_some() {
            return;
        }
        self.playlists.blend_work = Some(BlendWork::Inviting);
        self.write(Write::BlendInvite, None);
    }

    pub fn join_blend(&mut self, id: String) {
        if self.playlists.blend_work.is_some() || id.is_empty() {
            return;
        }
        self.playlists.blend_work = Some(BlendWork::Joining(id.clone()));
        self.write(Write::JoinBlend { id }, None);
    }

    pub fn blend_action(&mut self) {
        let Page::BlendInvitation(id) = &self.page else {
            return;
        };
        if id.is_empty() {
            self.blend_invite();
            return;
        }
        let state = match &self.playlists.invitation {
            Some((loaded, Ok(invitation))) if loaded == id => invitation.state.clone(),
            _ => return,
        };
        if state.joins() {
            self.join_blend(id.clone());
        } else if state.invites() {
            self.blend_invite();
        }
    }

    pub fn inviter_name(&self) -> Option<String> {
        self.account()
            .map(|account| account.name.clone())
            .filter(|name| !name.trim().is_empty())
    }

    fn replace_page(&mut self, page: Page) {
        let current = self.page.clone();
        self.navigate(page);
        if self.history.last() == Some(&current) {
            self.history.pop();
        }
    }

    pub fn show_blend_members(&mut self, uri: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(collection) = self.open_playlist(uri) else {
            return;
        };
        let members = blend::members(&collection.metadata);
        if members.is_empty() {
            return;
        }
        let title: SharedString = collection.name.clone().into();
        let app = cx.weak_entity();
        let view = cx.new(|_| BlendMembers::new(app, members));
        dialogs::open_blend_members(window, cx, title, view);
    }

    pub fn open_create_menu(&mut self, window: &mut Window) -> bool {
        self.show_menu(create_menu(), Place::Point(window.mouse_position()), None)
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
    if let Page::BlendInvitation(id) = page {
        app.playlists.invitation = None;
        if !id.is_empty() {
            app.send(Command::BlendInvitation(id.clone()));
        }
        return;
    }
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
                Ok(more) => {
                    collection.tracks.extend(more.tracks);
                    collection.attributions.extend(more.attributions);
                    collection.next = more.next;
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
        Event::Created {
            uri,
            name,
            open,
            edit,
        } => {
            app.refresh_rootlist();
            if !open {
                app.show_toast(format!("Added to {name}"), None);
                return;
            }
            if edit {
                app.playlists.edit_after_load = Some(uri.clone());
            }
            app.navigate(Page::Collection(uri));
        }
        Event::Editable { uris, result } => {
            app.playlists.editable.insert(
                uris.clone(),
                Editable {
                    at: Instant::now(),
                    result: Some(result),
                },
            );
            let newer = app
                .playlists
                .wanted
                .borrow()
                .as_ref()
                .is_some_and(|wanted| wanted.uris != uris);
            if let Some(source) = app.playlists.menu_source.clone()
                && source.uris == uris
                && !newer
            {
                let items = add_menu(app, &source);
                app.update_submenu(ADD_MENU, items);
            }
        }
        Event::AddReady(ready) => {
            if ready.duplicates.is_empty() {
                app.write_add(ready.target, ready.name, ready.uris, ready.position);
            } else {
                app.confirm_duplicates(ready, window, cx);
            }
        }
        Event::Curated {
            items,
            changes,
            done,
        } => {
            let open = app
                .playlists
                .collection
                .as_ref()
                .and_then(|(_, result)| result.as_ref().ok())
                .map(|collection| collection.uri.clone());
            if let Some(open) = open
                && changes.iter().any(|(uri, _)| *uri == open)
            {
                app.reload_playlist(&open);
            }
            for (uri, curate) in &changes {
                if uri == playlist::LIKED_SONGS {
                    for item in &items {
                        app.saved.known.insert(item.clone(), *curate);
                    }
                }
            }
            app.playlists.editable.retain(|uris, _| *uris != items);
            if let Some(done) = done {
                app.show_toast(done, None);
            }
        }
        Event::FolderCreated { uri, name, rename } => {
            app.refresh_rootlist();
            if rename {
                app.rename_folder(&uri, Some(name), window, cx);
            }
        }
        Event::Invited { link } => {
            cx.write_to_clipboard(ClipboardItem::new_string(link));
            app.show_toast("Link copied to clipboard", None);
        }
        Event::Pushed(push) => app.on_playlist_push(push),
        Event::Checked { uri, revision } => {
            let stale = revision.is_some()
                && app
                    .open_playlist(&uri)
                    .is_some_and(|collection| collection.revision != revision);
            if stale {
                app.reload_playlist(&uri);
            }
        }
        Event::Invitation { id, result } => {
            if app.page != Page::BlendInvitation(id.clone()) {
                return;
            }
            if let Ok(invitation) = &result
                && invitation.state == InvitationState::Joined
                && let Some(uri) = invitation.playlist_uri.clone()
            {
                app.replace_page(Page::Collection(uri));
                return;
            }
            app.playlists.invitation = Some((id, result.map(|invitation| *invitation)));
        }
        Event::BlendInvited { link } => {
            app.playlists.blend_work = None;
            let text = blend::share_text(&link, app.inviter_name().as_deref());
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            app.show_toast("Link copied to clipboard", None);
        }
        Event::BlendJoined { id, uri } => {
            app.playlists.blend_work = None;
            app.refresh_rootlist();
            if app.page == Page::BlendInvitation(id) {
                app.replace_page(Page::Collection(uri));
            }
        }
        Event::BlendFailed { message } => {
            app.playlists.blend_work = None;
            app.show_toast(message, None);
        }
        Event::Changed {
            uri,
            rootlist,
            rows,
            done,
        } => {
            if rows && let Some(uri) = &uri {
                app.settle_rows(uri, false);
            }
            after_write(app, uri, rootlist);
            if let Some(done) = done {
                app.show_toast(done, None);
            }
        }
        Event::Failed {
            uri,
            rootlist,
            rows,
            message,
        } => {
            if let Some(uri) = &uri
                && let Some(canonical) = playlist::canonical(uri)
            {
                app.playlists.bases.remove(&canonical);
                if rows {
                    app.settle_rows(&canonical, true);
                }
            }
            after_write(app, uri, rootlist);
            app.show_toast(message, None);
        }
    }
}

pub fn connected(app: &mut OpenMusic, window: &mut Window, cx: &mut Context<OpenMusic>) {
    app.send(Command::Rootlist);
    app.playlists.refreshed = Some(Instant::now());
    app.playlists.activation = Some(cx.observe_window_activation(window, |app, window, _| {
        if window.is_window_active() {
            app.refresh_on_focus();
        }
    }));
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

fn root_place(text: &str) -> Option<RootPlace> {
    match text.split_once(':') {
        None if text == "start" => Some(RootPlace::Start),
        None if text == "end" => Some(RootPlace::End),
        Some(("before", uri)) if !uri.is_empty() => Some(RootPlace::Before(uri.to_owned())),
        Some(("after", uri)) if !uri.is_empty() => Some(RootPlace::After(uri.to_owned())),
        Some(("into", uri)) if !uri.is_empty() => Some(RootPlace::Into(uri.to_owned())),
        _ => None,
    }
}

pub fn describe_editable(app: &OpenMusic, uris: &[String]) -> String {
    let mut lines = vec![format!("items={}", uris.join(","))];
    match app.playlists.editable_for(uris) {
        None => lines.push("loading".to_owned()),
        Some(Err(error)) => lines.push(format!("error {error}")),
        Some(Ok(targets)) => lines.extend(playlist::editable_tree(targets).into_iter().map(
            |(depth, target)| {
                format!(
                    "{}{:?} curates={} pinned={} {} {:?}",
                    "  ".repeat(depth),
                    target.kind,
                    flag(target.curates),
                    flag(target.pinned),
                    target.uri,
                    target.name
                )
            },
        )),
    }
    lines.join("\n")
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
        format!(
            "folder={:?} pending_rows={}",
            app.playlists.parent_folder(&collection.uri),
            app.playlists
                .snapshots
                .get(&collection.uri)
                .map_or(0, |snapshot| snapshot.pending),
        ),
    ];
    lines.push(format!(
        "blend={} attributed_rows={} pushes={} push_due={}",
        flag(blend::is_blend_list(&collection.metadata)),
        collection.attributions.len(),
        app.playlists.pushes.seen,
        flag(app.playlists.pushes.due.is_some()),
    ));
    lines.extend(collection.tracks.iter().enumerate().map(|(index, track)| {
        let by = attribution_line(collection, track);
        format!(
            "{index} {} {} {:?}{}",
            track.uid.as_deref().unwrap_or("-"),
            track.uri,
            track.title,
            if by.is_empty() {
                String::new()
            } else {
                format!(" by={by}")
            }
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
        ("curates", [uri]) => {
            let uris = vec![(*uri).to_owned()];
            app.request_editable(&uris);
            write("curates.txt", describe_editable(app, &uris));
        }
        ("save-to", ["dump"]) => {
            let text = app
                .playlists
                .save_to
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .map_or("closed".to_owned(), |sheet| sheet.read(cx).describe(cx));
            write("save-to.txt", text);
        }
        ("save-to", ["toggle", uri]) => {
            if let Some(sheet) = app.playlists.save_to.as_ref().and_then(WeakEntity::upgrade) {
                sheet.update(cx, |sheet, cx| sheet.toggle(uri, cx));
            }
        }
        ("save-to", ["done"]) => {
            if let Some(sheet) = app.playlists.save_to.as_ref().and_then(WeakEntity::upgrade) {
                sheet.update(cx, |sheet, cx| sheet.done(cx));
                window.close_dialog(cx);
            }
        }
        ("save-to", [uri]) => app.open_save_to(vec![(*uri).to_owned()], None, window, cx),
        ("folder-create", _) => app.write(
            Write::CreateFolder {
                name: Some(rest.to_owned()).filter(|name| !name.is_empty()),
                parent: None,
                playlists: Vec::new(),
                rename: false,
            },
            None,
        ),
        ("folder-rename", [uri, _, ..]) => app.save_folder_name((*uri).to_owned(), tail(1)),
        ("folder-edit", [uri]) => app.rename_folder(uri, None, window, cx),
        ("folder-delete", [uri]) => app.remove_folder((*uri).to_owned()),
        ("library-move", [uri, place]) => {
            if let Some(place) = root_place(place) {
                app.move_to(uri, place);
            }
        }
        ("sidebar-drop", [uri, target, side]) => {
            let drag = LibraryDrag {
                uri: (*uri).to_owned(),
                name: String::new(),
            };
            app.drop_library(&drag, target, *side == "after");
        }
        ("sidebar-add", [index, uris @ ..]) if !uris.is_empty() => {
            if let Ok(index) = index.parse() {
                let drag = RowDrag {
                    uris: uris.iter().map(|uri| (*uri).to_owned()).collect(),
                    uids: vec![None; uris.len()],
                    source: ("debug", 0),
                    context: None,
                };
                let at = RowDrop {
                    row: (library::view::PLAYLIST_ROW, index),
                    after: false,
                };
                row_dropped(app, &drag, at, window, cx);
            }
        }
        ("playlist-drop", [from, to, side]) => {
            let (Ok(from), Ok(to)) = (from.parse::<usize>(), to.parse::<usize>()) else {
                return true;
            };
            let Some((loaded, Ok(collection))) = &app.playlists.collection else {
                return true;
            };
            let Some(track) = collection.tracks.get(from) else {
                return true;
            };
            let drag = RowDrag {
                uris: vec![track.uri.clone()],
                uids: vec![track.uid.clone()],
                source: (TRACK_ROW, from),
                context: Some(loaded.clone()),
            };
            let at = RowDrop {
                row: (TRACK_ROW, to),
                after: *side == "after",
            };
            row_dropped(app, &drag, at, window, cx);
        }
        ("playlist-members", [uri]) => app.show_collaborators(uri, window, cx),
        ("playlist-shuffle", []) => app.shuffle_collection(cx),
        ("blend", []) => write("blend.txt", describe_blend(app)),
        ("blend", [input]) => {
            if app.open_blend_invitation(input) {
                return true;
            }
            if blend::valid_id(input) {
                app.navigate(Page::BlendInvitation((*input).to_owned()));
            } else {
                write("blend.txt", format!("invalid {input}"));
            }
        }
        ("blend-create", []) => app.create_blend(),
        ("blend-action", []) => app.blend_action(),
        ("blend-invite", []) => app.blend_invite(),
        ("blend-join", [id]) => app.join_blend((*id).to_owned()),
        ("blend-members", [uri]) => app.show_blend_members(uri, window, cx),
        ("push", ["rootlist", revision @ ..]) => {
            let user = app.playlists.user.clone().unwrap_or_default();
            app.on_playlist_push(Push::Rootlist {
                user,
                revision: revision.first().map(|revision| (*revision).to_owned()),
            });
        }
        ("push", ["playlist", uri, rest @ ..]) => {
            if let Some(uri) = playlist::canonical(uri) {
                app.on_playlist_push(Push::Playlist {
                    uri,
                    revision: rest
                        .iter()
                        .find(|word| **word != "listed")
                        .map(|revision| (*revision).to_owned()),
                    listed: rest.contains(&"listed"),
                });
            }
        }
        ("focus-refresh", []) => {
            app.playlists.refreshed = None;
            app.refresh_on_focus();
        }
        ("create-menu", []) => {
            let size = window.viewport_size();
            let center = point(size.width / 2., size.height / 2.);
            app.show_menu(create_menu(), Place::Point(center), None);
        }
        _ => return false,
    }
    true
}

pub fn describe_blend(app: &OpenMusic) -> String {
    let mut lines = vec![format!(
        "page={} work={:?} pushes={} push_due={}",
        app.page.describe(),
        app.playlists.blend_work,
        app.playlists.pushes.seen,
        flag(app.playlists.pushes.due.is_some()),
    )];
    match &app.playlists.invitation {
        None => lines.push("invitation=none".to_owned()),
        Some((id, Err(error))) => lines.push(format!("invitation={id} error={error}")),
        Some((id, Ok(invitation))) => {
            lines.push(format!(
                "invitation={id} state={:?} joins={} invites={} playlist={:?}",
                invitation.state,
                flag(invitation.state.joins()),
                flag(invitation.state.invites()),
                invitation.playlist_uri,
            ));
            lines.push(format!(
                "title={:?} subtitle={:?} button={:?} footnote={:?} members_title={:?}",
                invitation.title,
                invitation.subtitle,
                invitation.button,
                invitation.footnote,
                invitation.members_title,
            ));
            lines.extend(invitation.members.iter().map(|member| {
                format!(
                    "member {} {:?} image={}",
                    member.username,
                    member.name,
                    flag(member.image.is_some())
                )
            }));
            if let Some(recipient) = &invitation.recipient {
                lines.push(format!(
                    "recipient {} {:?} image={}",
                    recipient.username,
                    recipient.name,
                    flag(recipient.image.is_some())
                ));
            }
        }
    }
    if let Some((_, Ok(collection))) = &app.playlists.collection
        && blend::is_blend_list(&collection.metadata)
    {
        let members = blend::members(&collection.metadata);
        lines.push(format!(
            "blend={} creators={:?}",
            collection.uri,
            blend::creators(&members)
        ));
        lines.extend(members.iter().map(|member| {
            format!(
                "blend-member {} {:?} image={}",
                member.username,
                member.name,
                flag(member.image.is_some())
            )
        }));
    }
    lines.join("\n")
}

pub fn attribution_line(collection: &Collection, track: &Track) -> String {
    let key = track.uid.as_ref().unwrap_or(&track.uri);
    collection
        .attributions
        .get(key)
        .map(|found| {
            found
                .iter()
                .map(|attribution| {
                    format!(
                        "{}:{}",
                        attribution.username.as_deref().unwrap_or("-"),
                        attribution.reason(&collection.metadata)
                    )
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
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
    menus::route(cx, handle, |_: &menus::NewFolder, app, _, cx| {
        app.create_folder(None, Vec::new(), true);
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
        "blend-action" => app.blend_action(),
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
    app: &mut OpenMusic,
    drag: &RowDrag,
    at: RowDrop,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    match at.row.0 {
        TRACK_ROW => app.drop_on_rows(drag, at),
        library::view::PLAYLIST_ROW => app.drop_on_sidebar(drag, at.row.1),
        _ => return false,
    }
    true
}

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    app.apply_pushes();
    if let Some(source) = app.playlists.wanted.take() {
        app.request_editable(&source.uris);
        app.playlists.menu_source = Some(source);
    }
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
    fn asks_about_duplicates_with_the_desktop_wording() {
        let one = DuplicateQuestion::new("Road", 1, 1);
        assert_eq!(one.title, "Already added");
        assert_eq!(one.detail, "This is already in your 'Road' playlist.");
        assert_eq!(
            (one.answers, one.cancel, one.some),
            (vec!["Add anyway"], "Don't add", false)
        );
        let all = DuplicateQuestion::new("Road", 3, 3);
        assert_eq!(all.detail, "These are already in your 'Road' playlist.");
        assert!(!all.some);
        let some = DuplicateQuestion::new("Road", 3, 1);
        assert_eq!(some.title, "Some already added");
        assert_eq!(
            some.detail,
            "Some of these are already in your 'Road' playlist."
        );
        assert_eq!(
            (some.answers, some.cancel, some.some),
            (vec!["Add new ones", "Add all"], "Cancel", true)
        );
    }

    #[test]
    fn parses_library_places() {
        assert_eq!(root_place("start"), Some(RootPlace::Start));
        assert_eq!(root_place("end"), Some(RootPlace::End));
        assert_eq!(
            root_place("before:spotify:playlist:x"),
            Some(RootPlace::Before("spotify:playlist:x".into()))
        );
        assert_eq!(
            root_place("into:spotify:user:someone:folder:00000000000000f1"),
            Some(RootPlace::Into(
                "spotify:user:someone:folder:00000000000000f1".into()
            ))
        );
        assert_eq!(root_place("after:"), None);
        assert_eq!(root_place("middle"), None);
    }

    #[test]
    fn reloads_only_what_a_push_changed() {
        assert!(outdated(None, Some("AAAAKg==")));
        assert!(outdated(Some("AAAAKw=="), Some("AAAAKg==")));
        assert!(outdated(Some("AAAAKg=="), None));
        assert!(!outdated(Some("AAAAKg=="), Some("AAAAKg==")));
        let mut pushes = Pushes::default();
        assert!(!pushes.waiting());
        pushes.playlists.insert("spotify:playlist:x".into());
        assert!(pushes.waiting());
        let rootlist = Pushes {
            rootlist: true,
            ..Pushes::default()
        };
        assert!(rootlist.waiting());
    }

    #[test]
    fn picks_a_shuffle_start_inside_the_list() {
        for length in [1, 2, 7, 100] {
            assert!(random_index(length) < length);
        }
        assert_eq!(random_index(0), 0);
    }
}
