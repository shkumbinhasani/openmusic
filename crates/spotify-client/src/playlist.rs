use std::{
    collections::{HashMap, HashSet},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Method, StatusCode};
use serde_json::{Map, Value, json};

use crate::{
    api::{Api, Body, HttpStatus},
    catalog::{self, Capabilities, Collection, Track},
    entity,
    pathfinder::{self, Operation},
};

const ITEMS_HASH: &str = "b907ad6b088d5bf7680fa81198996d58398c2d55dfcdf59fcc959e0be98a418a";

pub const ADD_TO_PLAYLIST: Operation = Operation::mutation("addToPlaylist", ITEMS_HASH);
pub const REMOVE_FROM_PLAYLIST: Operation = Operation::mutation("removeFromPlaylist", ITEMS_HASH);
pub const MOVE_ITEMS_IN_PLAYLIST: Operation =
    Operation::mutation("moveItemsInPlaylist", ITEMS_HASH);
pub const PLAYLIST_PERMISSIONS: Operation = Operation::query(
    "playlistPermissions",
    "e43d1d35f231cf289c23c9d9c489f4a4f502e4eda09839c530608f107b6556b8",
);

pub const PAGE: usize = 100;
pub const ITEM_BATCH: usize = 100;
pub const IMAGE_UPLOAD_URL: &str = "https://image-upload.spotify.com/v4/playlist";
pub const INVITE_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub const DEFAULT_FOLDER_NAME: &str = "New Folder";
pub const VIRTUAL_PLAYLISTS: [&str; 3] = [
    "spotify:playlist:37i9dQZF1FgnTBfUlzkeKt",
    "spotify:playlist:37i9dQZF1F5p3rmiWPIYgZ",
    "spotify:playlist:37i9dQZF1CIl0ks0ynmzpD",
];
const DECORATE: &str = "revision,length,attributes,timestamp,owner,capabilities";
const START_GROUP: &str = "spotify:start-group:";
const END_GROUP: &str = "spotify:end-group:";

pub fn playlist_id(uri: &str) -> Option<&str> {
    let parts: Vec<&str> = uri.split(':').collect();
    match parts.as_slice() {
        ["spotify", "playlist", id] | ["spotify", "user", _, "playlist", id] if !id.is_empty() => {
            Some(id)
        }
        _ => None,
    }
}

pub fn canonical(uri: &str) -> Option<String> {
    playlist_id(uri).map(|id| format!("spotify:playlist:{id}"))
}

pub fn is_virtual(uri: &str) -> bool {
    canonical(uri).is_some_and(|uri| VIRTUAL_PLAYLISTS.contains(&uri.as_str()))
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn number(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn component(text: &str, space: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => encoded.push(byte as char),
            b' ' => encoded.push_str(space),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn segment(text: &str) -> String {
    component(text, "%20")
}

fn decode_plus(text: &str) -> String {
    url::form_urlencoded::parse(text.as_bytes())
        .next()
        .map(|(key, value)| {
            if value.is_empty() {
                key.into_owned()
            } else {
                format!("{key}={value}")
            }
        })
        .unwrap_or_default()
}

pub fn encode_folder_name(name: &str) -> String {
    let collapsed = name.split_whitespace().collect::<Vec<_>>().join(" ");
    component(&collapsed, "+")
}

pub fn decode_folder_name(encoded: &str) -> String {
    decode_plus(encoded)
}

pub fn random_hash() -> String {
    rand::random::<[u8; 8]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn folder_uri(user: &str, hash: &str) -> String {
    format!("spotify:user:{user}:folder:{hash}")
}

pub fn folder_hash(uri: &str) -> Option<&str> {
    let parts: Vec<&str> = uri.split(':').collect();
    match parts.as_slice() {
        ["spotify", "user", _, "folder", hash] if !hash.is_empty() => Some(hash),
        _ => None,
    }
}

pub fn start_key(hash: &str) -> String {
    format!("{START_GROUP}{hash}")
}

pub fn start_marker(hash: &str, name: &str) -> String {
    format!("{START_GROUP}{hash}:{}", encode_folder_name(name))
}

pub fn end_marker(hash: &str) -> String {
    format!("{END_GROUP}{hash}")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub uri: String,
    pub attributes: Option<Map<String, Value>>,
}

impl Item {
    pub fn key(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            attributes: None,
        }
    }

    pub fn bare(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            attributes: Some(Map::new()),
        }
    }

    pub fn stamped(uri: impl Into<String>, now_ms: i64) -> Self {
        let mut attributes = Map::new();
        attributes.insert("timestamp".into(), json!(now_ms.to_string()));
        Self {
            uri: uri.into(),
            attributes: Some(attributes),
        }
    }

    pub fn json(&self) -> Value {
        let mut item = Map::new();
        item.insert("uri".into(), json!(self.uri));
        if let Some(attributes) = &self.attributes {
            item.insert("attributes".into(), Value::Object(attributes.clone()));
        }
        Value::Object(item)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Position {
    Start,
    End,
    Before(Item),
    After(Item),
}

impl Position {
    pub fn in_folder(hash: &str) -> Self {
        Self::After(Item::key(start_key(hash)))
    }

    fn apply(&self, op: &mut Map<String, Value>) {
        match self {
            Self::Start => op.insert("addFirst".into(), json!(true)),
            Self::End => op.insert("addLast".into(), json!(true)),
            Self::Before(item) => op.insert("addBeforeItem".into(), item.json()),
            Self::After(item) => op.insert("addAfterItem".into(), item.json()),
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListAttribute {
    Name,
    Description,
    Picture,
}

impl ListAttribute {
    pub fn kind(self) -> &'static str {
        match self {
            Self::Name => "LIST_NAME",
            Self::Description => "LIST_DESCRIPTION",
            Self::Picture => "LIST_PICTURE",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes {
    ops: Vec<Value>,
}

impl Changes {
    pub fn new() -> Self {
        Self::default()
    }

    fn items(items: &[Item]) -> Value {
        Value::Array(items.iter().map(Item::json).collect())
    }

    pub fn add(mut self, items: &[Item], position: &Position) -> Self {
        let mut add = Map::new();
        add.insert("items".into(), Self::items(items));
        position.apply(&mut add);
        self.ops.push(json!({ "kind": "ADD", "add": add }));
        self
    }

    pub fn remove_keys(mut self, items: &[Item]) -> Self {
        self.ops.push(json!({
            "kind": "REM",
            "rem": { "items": Self::items(items), "itemsAsKey": true },
        }));
        self
    }

    pub fn remove_range(mut self, from: usize, length: usize) -> Self {
        let mut rem = Map::new();
        rem.insert("fromIndex".into(), json!(from));
        if length > 0 {
            rem.insert("length".into(), json!(length));
        }
        self.ops.push(json!({ "kind": "REM", "rem": rem }));
        self
    }

    pub fn move_items(mut self, items: &[Item], position: &Position) -> Self {
        let mut mov = Map::new();
        mov.insert("items".into(), Self::items(items));
        position.apply(&mut mov);
        self.ops.push(json!({ "kind": "MOV", "mov": mov }));
        self
    }

    pub fn move_range(mut self, from: usize, to: usize, length: usize) -> Self {
        let mut mov = Map::new();
        mov.insert("fromIndex".into(), json!(from));
        if length > 0 {
            mov.insert("length".into(), json!(length));
        }
        mov.insert("toIndex".into(), json!(to));
        self.ops.push(json!({ "kind": "MOV", "mov": mov }));
        self
    }

    pub fn update_list(mut self, values: Map<String, Value>, no_value: &[ListAttribute]) -> Self {
        let mut attributes = Map::new();
        attributes.insert("values".into(), Value::Object(values));
        if !no_value.is_empty() {
            attributes.insert(
                "noValue".into(),
                Value::Array(no_value.iter().map(|kind| json!(kind.kind())).collect()),
            );
        }
        self.ops.push(json!({
            "kind": "UPDATE_LIST_ATTRIBUTES",
            "updateListAttributes": { "newAttributes": attributes },
        }));
        self
    }

    pub fn update_item(mut self, item: &Item, values: Map<String, Value>) -> Self {
        self.ops.push(json!({
            "kind": "UPDATE_ITEM_ATTRIBUTES",
            "updateItemAttributes": {
                "newAttributes": { "values": values },
                "item": item.json(),
            },
        }));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn ops(&self) -> &[Value] {
        &self.ops
    }

    pub fn delta(&self) -> Value {
        json!({ "ops": self.ops, "info": { "source": { "client": "WEBPLAYER" } } })
    }

    pub fn body(&self, base_revision: Option<&str>) -> Value {
        let mut body = Map::new();
        if let Some(revision) = base_revision {
            body.insert("baseRevision".into(), json!(revision));
        }
        body.insert("deltas".into(), json!([self.delta()]));
        Value::Object(body)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RootPlaylist {
    pub uri: String,
    pub key: String,
    pub name: String,
    pub description: String,
    pub cover: Option<String>,
    pub picture: Option<String>,
    pub owner: Option<String>,
    pub format: Option<String>,
    pub length: u32,
    pub public: bool,
    pub added_at: Option<i64>,
    pub capabilities: Capabilities,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RootEntry {
    Playlist(RootPlaylist),
    Start { hash: String, name: String },
    End { hash: String },
    Other { uri: String },
}

impl RootEntry {
    pub fn uri(&self) -> String {
        match self {
            Self::Playlist(playlist) => playlist.key.clone(),
            Self::Start { hash, name } => start_marker(hash, name),
            Self::End { hash } => end_marker(hash),
            Self::Other { uri } => uri.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RootNode {
    Playlist { uri: String, index: usize },
    Folder(Folder),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Folder {
    pub uri: String,
    pub hash: String,
    pub name: String,
    pub children: Vec<RootNode>,
    pub start: usize,
    pub length: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub length: usize,
    pub closed: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rootlist {
    pub revision: String,
    pub entries: Vec<RootEntry>,
}

fn root_entry(item: &Value, meta: &Value) -> RootEntry {
    let uri = item["uri"].as_str().unwrap_or_default();
    if let Some(rest) = uri.strip_prefix(START_GROUP) {
        let (hash, name) = rest.split_once(':').unwrap_or((rest, ""));
        return RootEntry::Start {
            hash: hash.to_owned(),
            name: decode_folder_name(name),
        };
    }
    if let Some(hash) = uri.strip_prefix(END_GROUP) {
        return RootEntry::End {
            hash: hash.to_owned(),
        };
    }
    let Some(canonical) = canonical(uri) else {
        return RootEntry::Other {
            uri: uri.to_owned(),
        };
    };
    let attributes = &meta["attributes"];
    RootEntry::Playlist(RootPlaylist {
        uri: canonical,
        key: uri.to_owned(),
        name: attributes["name"].as_str().unwrap_or_default().to_owned(),
        description: catalog::strip_tags(attributes["description"].as_str().unwrap_or_default()),
        cover: catalog::playlist_cover(attributes),
        picture: text(&attributes["picture"]),
        owner: text(&meta["ownerUsername"]),
        format: text(&attributes["format"]),
        length: meta["length"].as_u64().unwrap_or_default() as u32,
        public: item["attributes"]["public"].as_bool().unwrap_or_default(),
        added_at: number(&item["attributes"]["timestamp"]),
        capabilities: Capabilities::from_playlist(&meta["capabilities"]),
    })
}

impl Rootlist {
    pub fn from_page(value: &Value) -> (Self, Option<usize>) {
        let contents = &value["contents"];
        let items = contents["items"].as_array().cloned().unwrap_or_default();
        let meta = contents["metaItems"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let entries: Vec<RootEntry> = items
            .iter()
            .enumerate()
            .map(|(index, item)| root_entry(item, meta.get(index).unwrap_or(&Value::Null)))
            .collect();
        let pos = contents["pos"].as_u64().unwrap_or_default() as usize;
        let end = pos + entries.len();
        let next = (contents["truncated"].as_bool().unwrap_or_default() && !entries.is_empty())
            .then_some(end);
        let list = Self {
            revision: text(&value["revision"]).unwrap_or_default(),
            entries,
        };
        (list, next)
    }

    pub fn playlists(&self) -> impl Iterator<Item = &RootPlaylist> {
        self.entries.iter().filter_map(|entry| match entry {
            RootEntry::Playlist(playlist) => Some(playlist),
            _ => None,
        })
    }

    pub fn playlist(&self, uri: &str) -> Option<&RootPlaylist> {
        let uri = canonical(uri)?;
        self.playlists().find(|playlist| playlist.uri == uri)
    }

    pub fn playlist_mut(&mut self, uri: &str) -> Option<&mut RootPlaylist> {
        let uri = canonical(uri)?;
        self.entries.iter_mut().find_map(|entry| match entry {
            RootEntry::Playlist(playlist) if playlist.uri == uri => Some(playlist),
            _ => None,
        })
    }

    pub fn contains(&self, uri: &str) -> bool {
        self.playlist(uri).is_some()
    }

    pub fn insert_first(&mut self, playlist: RootPlaylist) {
        self.entries.insert(0, RootEntry::Playlist(playlist));
    }

    pub fn remove(&mut self, uri: &str) -> Option<RootPlaylist> {
        let uri = canonical(uri)?;
        let index = self.entries.iter().position(
            |entry| matches!(entry, RootEntry::Playlist(playlist) if playlist.uri == uri),
        )?;
        match self.entries.remove(index) {
            RootEntry::Playlist(playlist) => Some(playlist),
            _ => None,
        }
    }

    pub fn span(&self, hash: &str) -> Option<Span> {
        let start = self.entries.iter().position(
            |entry| matches!(entry, RootEntry::Start { hash: found, .. } if found == hash),
        )?;
        let mut open: Vec<&str> = Vec::new();
        for (index, entry) in self.entries.iter().enumerate().skip(start) {
            match entry {
                RootEntry::Start { hash, .. } => open.push(hash),
                RootEntry::End { hash } if open.last() == Some(&hash.as_str()) => {
                    open.pop();
                    if open.is_empty() {
                        return Some(Span {
                            start,
                            length: index - start + 1,
                            closed: true,
                        });
                    }
                }
                _ => {}
            }
        }
        Some(Span {
            start,
            length: self.entries.len() - start,
            closed: false,
        })
    }

    pub fn tree(&self, user: &str) -> Vec<RootNode> {
        struct Open {
            hash: String,
            name: String,
            start: usize,
            children: Vec<RootNode>,
        }
        let close = |open: Open, end: usize| {
            RootNode::Folder(Folder {
                uri: folder_uri(user, &open.hash),
                hash: open.hash,
                name: open.name,
                children: open.children,
                start: open.start,
                length: end - open.start,
            })
        };
        let mut roots: Vec<RootNode> = Vec::new();
        let mut stack: Vec<Open> = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            match entry {
                RootEntry::Start { hash, name } => stack.push(Open {
                    hash: hash.clone(),
                    name: name.clone(),
                    start: index,
                    children: Vec::new(),
                }),
                RootEntry::End { hash } => {
                    if let Some(open) = stack.pop_if(|open| open.hash == *hash) {
                        let node = close(open, index + 1);
                        match stack.last_mut() {
                            Some(parent) => parent.children.push(node),
                            None => roots.push(node),
                        }
                    }
                }
                RootEntry::Playlist(playlist) => {
                    let node = RootNode::Playlist {
                        uri: playlist.uri.clone(),
                        index,
                    };
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(node),
                        None => roots.push(node),
                    }
                }
                RootEntry::Other { .. } => {}
            }
        }
        while let Some(open) = stack.pop() {
            let node = close(open, self.entries.len());
            match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => roots.push(node),
            }
        }
        roots
    }

    pub fn writable_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| match entry {
                RootEntry::Playlist(playlist) => playlist.capabilities.edit_items,
                RootEntry::Start { .. } => true,
                _ => false,
            })
            .count()
    }

    pub fn default_name(&self) -> String {
        format!("My Playlist #{}", self.writable_count() + 1)
    }
}

pub fn follow_changes(key: &str, follow: bool, now: i64) -> Changes {
    if follow {
        Changes::new().add(&[Item::stamped(key, now)], &Position::Start)
    } else {
        Changes::new().remove_keys(&[Item::key(key)])
    }
}

pub fn profile_changes(key: &str, public: bool) -> Changes {
    let mut values = Map::new();
    values.insert("public".into(), json!(public));
    Changes::new().update_item(&Item::key(key), values)
}

pub fn create_folder_changes(hash: &str, name: &str, position: &Position, now: i64) -> Changes {
    Changes::new().add(
        &[
            Item::stamped(start_marker(hash, name), now),
            Item::stamped(end_marker(hash), now),
        ],
        position,
    )
}

pub fn rename_folder_changes(hash: &str, temporary: &str, name: &str) -> Changes {
    let temporary_marker = start_marker(temporary, name);
    Changes::new()
        .add(
            &[Item::key(&temporary_marker)],
            &Position::Before(Item::key(start_key(hash))),
        )
        .remove_keys(&[Item::key(start_key(hash))])
        .add(
            &[Item::key(start_marker(hash, name))],
            &Position::Before(Item::key(&temporary_marker)),
        )
        .remove_keys(&[Item::key(&temporary_marker)])
}

fn closed_span(list: &Rootlist, hash: &str) -> Result<Span> {
    let span = list
        .span(hash)
        .ok_or_else(|| anyhow!("This folder is no longer in Your Library."))?;
    let first = list.entries.get(span.start);
    let last = list.entries.get(span.start + span.length - 1);
    let starts = matches!(first, Some(RootEntry::Start { hash: found, .. }) if found == hash);
    let ends = matches!(last, Some(RootEntry::End { hash: found }) if found == hash);
    if !span.closed || !starts || !ends {
        bail!("This folder looks damaged, so OpenMusic won’t change it.");
    }
    Ok(span)
}

pub fn delete_folder_changes(list: &Rootlist, hash: &str) -> Result<Changes> {
    let span = closed_span(list, hash)?;
    Ok(Changes::new().remove_range(span.start, span.length))
}

pub fn move_folder_changes(list: &Rootlist, hash: &str, to: usize) -> Result<Changes> {
    let span = closed_span(list, hash)?;
    if to > span.start && to < span.start + span.length {
        bail!("A folder can’t go inside itself.");
    }
    if to > list.entries.len() {
        bail!("That place is outside Your Library.");
    }
    Ok(Changes::new().move_range(span.start, to, span.length))
}

pub fn move_playlists_changes(keys: &[String], position: &Position) -> Changes {
    let items: Vec<Item> = keys.iter().map(Item::bare).collect();
    Changes::new().move_items(&items, position)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Details {
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub picture: Option<Option<String>>,
}

pub const NAME_LIMIT: usize = 100;
pub const DESCRIPTION_LIMIT: usize = 300;

impl Details {
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.description.is_none() && self.picture.is_none()
    }

    pub fn changes(&self) -> Result<Changes> {
        let mut values = Map::new();
        let mut none = Vec::new();
        if let Some(name) = &self.name {
            let name = name.trim();
            if name.is_empty() {
                bail!("Playlist name is required.");
            }
            if name.chars().count() > NAME_LIMIT {
                bail!("Keep the name under {NAME_LIMIT} characters.");
            }
            values.insert("name".into(), json!(name));
        }
        match &self.description {
            Some(Some(description)) if !description.trim().is_empty() => {
                let description = description.trim();
                if description.contains('\n') {
                    bail!("Line breaks aren't supported in the description.");
                }
                if description.contains(['<', '>']) {
                    bail!("HTML isn't supported in playlist description.");
                }
                if description.chars().count() > DESCRIPTION_LIMIT {
                    bail!("Keep the description under {DESCRIPTION_LIMIT} characters.");
                }
                values.insert("description".into(), json!(description));
            }
            Some(_) => none.push(ListAttribute::Description),
            None => {}
        }
        match &self.picture {
            Some(Some(picture)) => {
                values.insert("picture".into(), json!(picture));
            }
            Some(None) => none.push(ListAttribute::Picture),
            None => {}
        }
        Ok(Changes::new().update_list(values, &none))
    }
}

pub fn create_body(name: &str) -> Value {
    let mut values = Map::new();
    values.insert("name".into(), json!(name));
    Changes::new().update_list(values, &[]).delta()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ItemPosition {
    Top,
    Bottom,
    BeforeUid(String),
    AfterUid(String),
}

impl ItemPosition {
    pub fn json(&self) -> Value {
        match self {
            Self::Top => json!({ "moveType": "TOP_OF_PLAYLIST", "fromUid": null }),
            Self::Bottom => json!({ "moveType": "BOTTOM_OF_PLAYLIST", "fromUid": null }),
            Self::BeforeUid(uid) => json!({ "moveType": "BEFORE_UID", "fromUid": uid }),
            Self::AfterUid(uid) => json!({ "moveType": "AFTER_UID", "fromUid": uid }),
        }
    }

    fn reverses_batches(&self) -> bool {
        matches!(self, Self::Top | Self::AfterUid(_))
    }
}

pub fn add_variables(playlist: &str, uris: &[String], position: &ItemPosition) -> Value {
    json!({
        "playlistUri": playlist,
        "playlistItemUris": uris,
        "newPosition": position.json(),
        "interactionId": null,
    })
}

pub fn remove_variables(playlist: &str, uids: &[String]) -> Value {
    json!({ "playlistUri": playlist, "uids": uids })
}

pub fn move_variables(playlist: &str, uids: &[String], position: &ItemPosition) -> Value {
    json!({ "playlistUri": playlist, "uids": uids, "newPosition": position.json() })
}

pub fn add_batches<'a>(uris: &'a [String], position: &ItemPosition) -> Vec<&'a [String]> {
    let mut batches: Vec<&[String]> = uris.chunks(ITEM_BATCH).collect();
    if position.reverses_batches() {
        batches.reverse();
    }
    batches
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    Blocked,
    Viewer,
    Contributor,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Self::Blocked => "BLOCKED",
            Self::Viewer => "VIEWER",
            Self::Contributor => "CONTRIBUTOR",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "BLOCKED" => Some(Self::Blocked),
            "VIEWER" => Some(Self::Viewer),
            "CONTRIBUTOR" => Some(Self::Contributor),
            _ => None,
        }
    }

    pub fn of(value: &Value) -> Option<Self> {
        value.as_str().and_then(Self::parse)
    }
}

pub fn base_body(current: &Value, level: Level) -> Value {
    let mut body = match current {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    body.insert("permissionLevel".into(), json!(level.name()));
    Value::Object(body)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Member {
    pub username: String,
    pub name: String,
    pub uri: String,
    pub image: Option<String>,
    pub owner: bool,
    pub level: Option<Level>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Permissions {
    pub base: Option<Level>,
    pub administrate: bool,
    pub cancel_membership: bool,
    pub edit_items: bool,
    pub edit_metadata: bool,
    pub grantable: Vec<Level>,
    pub members: Vec<Member>,
}

impl Permissions {
    pub fn owner(&self) -> Option<&Member> {
        self.members.iter().find(|member| member.owner)
    }

    pub fn member(&self, username: &str) -> Option<&Member> {
        self.members
            .iter()
            .find(|member| member.username == username)
    }
}

pub fn parse_permissions(data: &Value) -> Result<Permissions> {
    let playlist = &data["playlistV2"];
    if playlist["__typename"] != "Playlist" {
        bail!("Spotify has no permissions for this playlist.");
    }
    let capabilities = &playlist["currentUserCapabilities"];
    let flag = |key: &str| capabilities[key].as_bool().unwrap_or_default();
    let members = playlist["members"]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|member| {
            let user = &member["user"]["data"];
            let username = text(&user["username"]).or_else(|| {
                user["uri"]
                    .as_str()
                    .and_then(|uri| uri.strip_prefix("spotify:user:"))
                    .map(str::to_owned)
            })?;
            Some(Member {
                name: text(&user["name"]).unwrap_or_else(|| username.clone()),
                uri: text(&user["uri"]).unwrap_or_else(|| format!("spotify:user:{username}")),
                image: entity::pick_image(&user["avatar"]["sources"], 64),
                owner: member["isOwner"].as_bool().unwrap_or_default(),
                level: Level::of(&member["permissionLevel"]),
                username,
            })
        })
        .collect();
    Ok(Permissions {
        base: Level::of(&playlist["basePermission"]),
        administrate: flag("canAdministratePermissions"),
        cancel_membership: flag("canCancelMembership"),
        edit_items: flag("canEditItems"),
        edit_metadata: flag("canEditMetadata"),
        grantable: capabilities["grantableLevel"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Level::of)
            .collect(),
        members,
    })
}

pub fn member_revision(members: &Value, username: &str) -> Option<String> {
    text(&members["memberPermissions"][username]["revision"])
}

pub fn invite_body() -> Value {
    json!({ "permission": { "permissionLevel": "CONTRIBUTOR" }, "ttlMs": INVITE_TTL_MS })
}

pub fn invite_url(id: &str, token: &str) -> String {
    format!(
        "https://open.spotify.com/playlist/{id}?pt={}",
        component(token, "%20")
    )
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaylistItem {
    pub uri: String,
    pub uid: Option<String>,
    pub added_at: Option<i64>,
    pub added_by: Option<String>,
    pub attributes: HashMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contents {
    pub revision: Option<String>,
    pub length: usize,
    pub items: Vec<PlaylistItem>,
    pub next: Option<usize>,
}

pub fn item_uid(item_id: &str) -> Option<String> {
    let bytes = STANDARD.decode(item_id).ok()?;
    (!bytes.is_empty()).then(|| bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn format_attributes(list: &Value) -> HashMap<String, String> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|pair| Some((text(&pair["key"])?, pair["value"].as_str()?.to_owned())))
        .collect()
}

fn playlist_item(item: &Value) -> Option<PlaylistItem> {
    let uri = text(&item["uri"])?;
    let attributes = &item["attributes"];
    Some(PlaylistItem {
        uri,
        uid: attributes["itemId"].as_str().and_then(item_uid),
        added_at: number(&attributes["timestamp"]),
        added_by: text(&attributes["addedBy"]),
        attributes: format_attributes(&attributes["formatAttributes"]),
    })
}

pub fn parse_contents(value: &Value) -> Contents {
    let contents = &value["contents"];
    let raw = contents["items"].as_array().cloned().unwrap_or_default();
    let pos = contents["pos"].as_u64().unwrap_or_default() as usize;
    let end = pos + raw.len();
    let length = value["length"]
        .as_u64()
        .map_or(end, |length| length as usize);
    Contents {
        revision: text(&value["revision"]),
        length,
        items: raw.iter().filter_map(playlist_item).collect(),
        next: (end < length && !raw.is_empty()).then_some(end),
    }
}

pub fn local_track(uri: &str) -> Option<Track> {
    let rest = uri.strip_prefix("spotify:local:")?;
    let parts: Vec<String> = rest.split(':').map(decode_plus).collect();
    let [artist, album, title, seconds] = parts.as_slice() else {
        return None;
    };
    Some(Track {
        uri: uri.to_owned(),
        title: if title.is_empty() {
            "Local file".to_owned()
        } else {
            title.clone()
        },
        artist: artist.clone(),
        album: album.clone(),
        duration_ms: seconds
            .parse::<u32>()
            .unwrap_or_default()
            .saturating_mul(1000),
        ..Track::default()
    })
}

pub fn rows(items: &[PlaylistItem], found: &HashMap<String, Track>) -> Vec<Track> {
    items
        .iter()
        .filter_map(|item| {
            let mut track = match found.get(&item.uri) {
                Some(track) => track.clone(),
                None => local_track(&item.uri)?,
            };
            track.uid = item.uid.clone();
            track.added_at = item.added_at;
            Some(track)
        })
        .collect()
}

pub async fn tracks_for(api: &Api, items: &[PlaylistItem]) -> Result<Vec<Track>> {
    let mut seen = HashSet::new();
    let uris: Vec<String> = items
        .iter()
        .filter(|item| {
            item.uri.starts_with("spotify:track:") || item.uri.starts_with("spotify:episode:")
        })
        .filter(|item| seen.insert(item.uri.clone()))
        .map(|item| item.uri.clone())
        .collect();
    let found: HashMap<String, Track> = catalog::tracks(api, &uris)
        .await?
        .into_iter()
        .map(|track| (track.uri.clone(), track))
        .collect();
    Ok(rows(items, &found))
}

async fn fetch(api: &Api, id: &str, from: usize, length: usize) -> Result<Value> {
    api.get_json(&format!(
        "/playlist/v2/playlist/{id}?decorate={DECORATE}&from={from}&length={length}"
    ))
    .await
}

fn id_of(uri: &str) -> Result<&str> {
    playlist_id(uri).ok_or_else(|| anyhow!("This is not a playlist."))
}

pub async fn collection(api: &Api, uri: &str, upto: usize) -> Result<Collection> {
    let id = id_of(uri)?;
    let value = fetch(api, id, 0, PAGE).await?;
    let mut contents = parse_contents(&value);
    while contents.items.len() < upto
        && let Some(from) = contents.next
    {
        let more = parse_contents(&fetch(api, id, from, PAGE).await?);
        contents.items.extend(more.items);
        contents.next = more.next.filter(|next| *next > from);
    }
    let tracks = tracks_for(api, &contents.items).await?;
    Ok(build_collection(uri, &value, &contents, tracks))
}

pub fn build_collection(
    uri: &str,
    value: &Value,
    contents: &Contents,
    tracks: Vec<Track>,
) -> Collection {
    let attributes = &value["attributes"];
    let description = catalog::strip_tags(attributes["description"].as_str().unwrap_or_default());
    let count = contents.length;
    let subtitle = if description.is_empty() {
        format!("{count} songs")
    } else {
        format!("{description} · {count} songs")
    };
    let owner = text(&value["ownerUsername"]);
    let format = text(&attributes["format"]);
    let mut metadata = format_attributes(&attributes["formatAttributes"]);
    if let Some(format) = &format {
        metadata.insert("format_list_type".into(), format.clone());
    }
    if let Some(owner) = &owner {
        metadata.insert("context_owner".into(), format!("spotify:user:{owner}"));
    }
    Collection {
        uri: canonical(uri).unwrap_or_else(|| uri.to_owned()),
        kind: "PLAYLIST".into(),
        name: attributes["name"].as_str().unwrap_or("Playlist").to_owned(),
        subtitle,
        cover: catalog::playlist_cover(attributes),
        artist_uri: None,
        tracks,
        owner,
        description,
        revision: contents.revision.clone(),
        format,
        capabilities: Capabilities::from_playlist(&value["capabilities"]),
        metadata,
        total: count,
        next: contents.next,
        picture: text(&attributes["picture"]),
    }
}

pub async fn more(api: &Api, uri: &str, from: usize) -> Result<(Vec<Track>, Option<usize>)> {
    let id = id_of(uri)?;
    let contents = parse_contents(&fetch(api, id, from, PAGE).await?);
    let tracks = tracks_for(api, &contents.items).await?;
    Ok((tracks, contents.next.filter(|next| *next > from)))
}

pub async fn rootlist(api: &Api, user: &str) -> Result<Rootlist> {
    let path = format!(
        "/playlist/v2/user/{}/rootlist?decorate={DECORATE}",
        segment(user)
    );
    let first: Value = api.get_json(&path).await?;
    let (mut list, mut next) = Rootlist::from_page(&first);
    while let Some(from) = next {
        let page: Value = api.get_json(&format!("{path}&from={from}")).await?;
        let (more, after) = Rootlist::from_page(&page);
        list.entries.extend(more.entries);
        next = after.filter(|after| *after > from);
    }
    Ok(list)
}

pub fn lenient(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}

async fn post(api: &Api, path: &str, body: Value) -> Result<Value> {
    let bytes = api
        .request(Method::POST, path, Body::Json(body), "application/json")
        .await?;
    Ok(lenient(&bytes))
}

pub async fn rootlist_apply(
    api: &Api,
    user: &str,
    changes: &Changes,
    base_revision: Option<&str>,
) -> Result<String> {
    if changes.is_empty() {
        bail!("Nothing to change.");
    }
    let reply = post(
        api,
        &format!("/playlist/v2/user/{}/rootlist/changes", segment(user)),
        changes.body(base_revision),
    )
    .await?;
    Ok(text(&reply["revision"]).unwrap_or_default())
}

pub async fn playlist_apply(
    api: &Api,
    uri: &str,
    changes: &Changes,
    base_revision: Option<&str>,
) -> Result<String> {
    if changes.is_empty() {
        bail!("Nothing to change.");
    }
    let id = id_of(uri)?;
    let reply = post(
        api,
        &format!("/playlist/v2/playlist/{id}/changes"),
        changes.body(base_revision),
    )
    .await?;
    Ok(text(&reply["revision"]).unwrap_or_default())
}

fn conflict(error: &anyhow::Error) -> bool {
    error.downcast_ref::<HttpStatus>().is_some_and(|status| {
        status.0 == StatusCode::CONFLICT || status.0 == StatusCode::BAD_REQUEST
    })
}

async fn apply_indexed(
    api: &Api,
    user: &str,
    build: impl Fn(&Rootlist) -> Result<Changes>,
) -> Result<String> {
    let mut attempt = 0;
    loop {
        let list = rootlist(api, user).await?;
        let changes = build(&list)?;
        match rootlist_apply(api, user, &changes, Some(&list.revision)).await {
            Err(error) if attempt == 0 && conflict(&error) => attempt += 1,
            result => return result,
        }
    }
}

pub async fn create(api: &Api, user: &str, name: &str, position: &Position) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("Playlist name is required.");
    }
    let reply = post(api, "/playlist/v2/playlist", create_body(name)).await?;
    let uri = reply["uri"]
        .as_str()
        .filter(|uri| playlist_id(uri).is_some())
        .ok_or_else(|| anyhow!("Spotify did not create the playlist."))?
        .to_owned();
    let changes = Changes::new().add(&[Item::stamped(&uri, now_ms())], position);
    rootlist_apply(api, user, &changes, None).await?;
    Ok(canonical(&uri).unwrap_or(uri))
}

pub async fn update_details(api: &Api, uri: &str, details: &Details) -> Result<Option<String>> {
    if details.is_empty() {
        return Ok(None);
    }
    let changes = details.changes()?;
    playlist_apply(api, uri, &changes, None).await.map(Some)
}

pub async fn upload_cover(api: &Api, jpeg: Vec<u8>) -> Result<String> {
    let bytes = api
        .request(
            Method::POST,
            IMAGE_UPLOAD_URL,
            Body::Bytes(jpeg, "image/jpeg"),
            "application/json",
        )
        .await?;
    text(&lenient(&bytes)["uploadToken"])
        .ok_or_else(|| anyhow!("Spotify did not accept the image."))
}

pub async fn register_cover(api: &Api, uri: &str, token: &str) -> Result<String> {
    let id = id_of(uri)?;
    let reply = post(
        api,
        &format!("/playlist/v2/playlist/{id}/register-image"),
        json!({ "uploadToken": token }),
    )
    .await?;
    text(&reply["picture"]).ok_or_else(|| anyhow!("Spotify did not register the image."))
}

pub async fn cover_picture(api: &Api, uri: &str, jpeg: Vec<u8>) -> Result<String> {
    let token = upload_cover(api, jpeg).await?;
    register_cover(api, uri, &token).await
}

pub async fn add_items(
    api: &Api,
    uri: &str,
    uris: &[String],
    position: &ItemPosition,
) -> Result<()> {
    let playlist = canonical(uri).ok_or_else(|| anyhow!("This is not a playlist."))?;
    for batch in add_batches(uris, position) {
        pathfinder::mutate(
            api,
            ADD_TO_PLAYLIST,
            add_variables(&playlist, batch, position),
        )
        .await?;
    }
    Ok(())
}

pub async fn remove_items(api: &Api, uri: &str, uids: &[String]) -> Result<()> {
    let playlist = canonical(uri).ok_or_else(|| anyhow!("This is not a playlist."))?;
    for batch in uids.chunks(ITEM_BATCH) {
        pathfinder::mutate(
            api,
            REMOVE_FROM_PLAYLIST,
            remove_variables(&playlist, batch),
        )
        .await?;
    }
    Ok(())
}

pub async fn move_items(
    api: &Api,
    uri: &str,
    uids: &[String],
    position: &ItemPosition,
) -> Result<()> {
    let playlist = canonical(uri).ok_or_else(|| anyhow!("This is not a playlist."))?;
    pathfinder::mutate(
        api,
        MOVE_ITEMS_IN_PLAYLIST,
        move_variables(&playlist, uids, position),
    )
    .await?;
    Ok(())
}

pub async fn follow(api: &Api, user: &str, uri: &str, follow: bool) -> Result<Option<String>> {
    let list = rootlist(api, user).await?;
    let changes = match (list.playlist(uri), follow) {
        (Some(_), true) | (None, false) => return Ok(None),
        (Some(found), false) => follow_changes(&found.key, false, now_ms()),
        (None, true) => {
            let key = canonical(uri).ok_or_else(|| anyhow!("This is not a playlist."))?;
            follow_changes(&key, true, now_ms())
        }
    };
    rootlist_apply(api, user, &changes, None).await.map(Some)
}

pub async fn set_on_profile(api: &Api, user: &str, uri: &str, public: bool) -> Result<String> {
    let list = rootlist(api, user).await?;
    let found = list
        .playlist(uri)
        .ok_or_else(|| anyhow!("Add this playlist to Your Library first."))?;
    rootlist_apply(api, user, &profile_changes(&found.key, public), None).await
}

pub async fn create_folder(
    api: &Api,
    user: &str,
    name: &str,
    position: &Position,
) -> Result<String> {
    let name = match name.trim() {
        "" => DEFAULT_FOLDER_NAME,
        name => name,
    };
    let hash = random_hash();
    rootlist_apply(
        api,
        user,
        &create_folder_changes(&hash, name, position, now_ms()),
        None,
    )
    .await?;
    Ok(folder_uri(user, &hash))
}

pub async fn rename_folder(api: &Api, user: &str, hash: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("A folder needs a name.");
    }
    let changes = rename_folder_changes(hash, &random_hash(), name);
    rootlist_apply(api, user, &changes, None).await
}

pub async fn delete_folder(api: &Api, user: &str, hash: &str) -> Result<String> {
    apply_indexed(api, user, |list| delete_folder_changes(list, hash)).await
}

pub async fn move_folder(api: &Api, user: &str, hash: &str, to: usize) -> Result<String> {
    apply_indexed(api, user, |list| move_folder_changes(list, hash, to)).await
}

pub async fn move_playlists(
    api: &Api,
    user: &str,
    uris: &[String],
    position: &Position,
) -> Result<String> {
    let list = rootlist(api, user).await?;
    let keys: Vec<String> = uris
        .iter()
        .filter_map(|uri| list.playlist(uri).map(|found| found.key.clone()))
        .collect();
    if keys.is_empty() {
        bail!("These playlists are not in Your Library.");
    }
    rootlist_apply(api, user, &move_playlists_changes(&keys, position), None).await
}

fn permission_path(uri: &str, rest: &str) -> Result<String> {
    Ok(format!(
        "/playlist-permission/v1/playlist/{}/{rest}",
        id_of(uri)?
    ))
}

pub async fn base_permission(api: &Api, uri: &str) -> Result<Option<Level>> {
    let value: Value = api
        .get_json(&permission_path(uri, "permission/base")?)
        .await?;
    Ok(Level::of(&value["permissionLevel"]))
}

pub async fn set_private(api: &Api, uri: &str, private: bool) -> Result<Level> {
    let path = permission_path(uri, "permission/base")?;
    let current: Value = api.get_json(&path).await?;
    let wanted = if private {
        Level::Blocked
    } else {
        Level::Viewer
    };
    if Level::of(&current["permissionLevel"]) == Some(wanted) {
        return Ok(wanted);
    }
    post(api, &path, base_body(&current, wanted)).await?;
    let after: Value = api.get_json(&path).await?;
    if Level::of(&after["permissionLevel"]) != Some(wanted) {
        bail!("Spotify did not change who can see this playlist.");
    }
    Ok(wanted)
}

pub async fn permissions(api: &Api, uri: &str) -> Result<Permissions> {
    let playlist = canonical(uri).ok_or_else(|| anyhow!("This is not a playlist."))?;
    let data = pathfinder::query(api, PLAYLIST_PERMISSIONS, json!({ "uri": playlist })).await?;
    parse_permissions(&data)
}

pub async fn invite_link(api: &Api, uri: &str) -> Result<String> {
    let id = id_of(uri)?;
    let reply = post(
        api,
        &permission_path(uri, "permission-grant")?,
        invite_body(),
    )
    .await?;
    let token =
        text(&reply["token"]).ok_or_else(|| anyhow!("Spotify did not create an invite link."))?;
    Ok(invite_url(id, &token))
}

pub async fn set_member(api: &Api, uri: &str, username: &str, level: Level) -> Result<()> {
    let members: Value = api
        .get_json(&permission_path(uri, "permission/members")?)
        .await?;
    let revision = member_revision(&members, username)
        .ok_or_else(|| anyhow!("This person is no longer on the playlist."))?;
    post(
        api,
        &permission_path(
            uri,
            &format!("permission/member/user/{}", segment(username)),
        )?,
        json!({ "revision": revision, "permissionLevel": level.name() }),
    )
    .await?;
    Ok(())
}

pub async fn remove_member(api: &Api, uri: &str, username: &str) -> Result<()> {
    api.request(
        Method::DELETE,
        &permission_path(
            uri,
            &format!("permission/member/user/{}", segment(username)),
        )?,
        Body::Empty,
        "application/json",
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "4uLU6hMCjMI75M1A2tKUQC";
    const NOW: i64 = 1_795_500_000_000;

    fn playlist_entry(uri: &str, editable: bool) -> RootEntry {
        RootEntry::Playlist(RootPlaylist {
            uri: uri.into(),
            key: uri.into(),
            capabilities: Capabilities {
                edit_items: editable,
                ..Capabilities::default()
            },
            ..RootPlaylist::default()
        })
    }

    fn start(hash: &str, name: &str) -> RootEntry {
        RootEntry::Start {
            hash: hash.into(),
            name: name.into(),
        }
    }

    fn end(hash: &str) -> RootEntry {
        RootEntry::End { hash: hash.into() }
    }

    #[test]
    fn recognises_playlist_uris() {
        assert_eq!(playlist_id(&format!("spotify:playlist:{ID}")), Some(ID));
        assert_eq!(
            canonical(&format!("spotify:user:someone:playlist:{ID}")),
            Some(format!("spotify:playlist:{ID}"))
        );
        assert_eq!(playlist_id("spotify:album:x"), None);
        assert_eq!(playlist_id("spotify:playlist:"), None);
        assert!(is_virtual("spotify:playlist:37i9dQZF1FgnTBfUlzkeKt"));
        assert!(!is_virtual(&format!("spotify:playlist:{ID}")));
    }

    #[test]
    fn follows_and_unfollows_like_the_web_player() {
        let uri = format!("spotify:playlist:{ID}");
        assert_eq!(
            follow_changes(&uri, true, NOW).body(None),
            json!({"deltas":[{"ops":[{"kind":"ADD","add":{"items":[{"uri":uri,"attributes":{"timestamp":"1795500000000"}}],"addFirst":true}}],
                "info":{"source":{"client":"WEBPLAYER"}}}]})
        );
        assert_eq!(
            follow_changes(&uri, false, NOW).body(None),
            json!({"deltas":[{"ops":[{"kind":"REM","rem":{"items":[{"uri":uri}],"itemsAsKey":true}}],
                "info":{"source":{"client":"WEBPLAYER"}}}]})
        );
    }

    #[test]
    fn renames_and_describes_with_one_update() {
        let details = Details {
            name: Some("  New name ".into()),
            description: Some(Some("New description".into())),
            picture: None,
        };
        assert_eq!(
            details.changes().unwrap().body(None),
            json!({"deltas":[{"ops":[{"kind":"UPDATE_LIST_ATTRIBUTES","updateListAttributes":{"newAttributes":{
                "values":{"name":"New name","description":"New description"}}}}],
                "info":{"source":{"client":"WEBPLAYER"}}}]})
        );
        let clear = Details {
            description: Some(None),
            picture: Some(None),
            ..Details::default()
        };
        assert_eq!(
            clear.changes().unwrap().ops()[0],
            json!({"kind":"UPDATE_LIST_ATTRIBUTES","updateListAttributes":{"newAttributes":{
                "values":{},"noValue":["LIST_DESCRIPTION","LIST_PICTURE"]}}})
        );
        let blank = Details {
            description: Some(Some("   ".into())),
            ..Details::default()
        };
        assert_eq!(
            blank.changes().unwrap().ops()[0]["updateListAttributes"]["newAttributes"]["noValue"],
            json!(["LIST_DESCRIPTION"])
        );
        let picture = Details {
            picture: Some(Some("q83aZXsDs5Q=".into())),
            ..Details::default()
        };
        assert_eq!(
            picture.changes().unwrap().ops()[0]["updateListAttributes"]["newAttributes"],
            json!({"values":{"picture":"q83aZXsDs5Q="}})
        );
        assert!(Details::default().is_empty());
        let empty = Details {
            name: Some("   ".into()),
            ..Details::default()
        };
        assert_eq!(
            empty.changes().unwrap_err().to_string(),
            "Playlist name is required."
        );
        let breaks = Details {
            description: Some(Some("one\ntwo".into())),
            ..Details::default()
        };
        assert!(breaks.changes().is_err());
        let html = Details {
            description: Some(Some("<b>bold</b>".into())),
            ..Details::default()
        };
        assert_eq!(
            html.changes().unwrap_err().to_string(),
            "HTML isn't supported in playlist description."
        );
    }

    #[test]
    fn creates_with_the_name_only() {
        assert_eq!(
            create_body("My Playlist #25"),
            json!({"ops":[{"kind":"UPDATE_LIST_ATTRIBUTES","updateListAttributes":{"newAttributes":{"values":{"name":"My Playlist #25"}}}}],
                "info":{"source":{"client":"WEBPLAYER"}}})
        );
    }

    #[test]
    fn publishes_to_the_profile_by_item_attribute() {
        let uri = format!("spotify:playlist:{ID}");
        assert_eq!(
            profile_changes(&uri, true).ops()[0],
            json!({"kind":"UPDATE_ITEM_ATTRIBUTES","updateItemAttributes":{
                "newAttributes":{"values":{"public":true}},"item":{"uri":uri}}})
        );
    }

    #[test]
    fn writes_index_ops_with_zero_indexes() {
        let changes = Changes::new().remove_range(0, 3).move_range(0, 7, 2);
        assert_eq!(
            changes.body(Some("AAAAAQ==")),
            json!({"baseRevision":"AAAAAQ==","deltas":[{"ops":[
                {"kind":"REM","rem":{"fromIndex":0,"length":3}},
                {"kind":"MOV","mov":{"fromIndex":0,"length":2,"toIndex":7}}],
                "info":{"source":{"client":"WEBPLAYER"}}}]})
        );
    }

    #[test]
    fn encodes_folder_names_like_encode_uri_component() {
        assert_eq!(encode_folder_name("New Folder"), "New+Folder");
        assert_eq!(encode_folder_name("  Rock \t & Roll  "), "Rock+%26+Roll");
        assert_eq!(encode_folder_name("100% (live)"), "100%25+(live)");
        assert_eq!(encode_folder_name("Café 🎵"), "Caf%C3%A9+%F0%9F%8E%B5");
        for name in ["New Folder", "Rock & Roll", "a+b", "100% (live)", "Café 🎵"] {
            assert_eq!(decode_folder_name(&encode_folder_name(name)), name);
        }
        assert_eq!(decode_folder_name("New+Folder"), "New Folder");
    }

    #[test]
    fn creates_and_renames_folders_with_markers() {
        let changes =
            create_folder_changes("1a2b3c4d5e6f7081", "New Folder", &Position::Start, NOW);
        assert_eq!(
            changes.ops()[0],
            json!({"kind":"ADD","add":{"items":[
                {"uri":"spotify:start-group:1a2b3c4d5e6f7081:New+Folder","attributes":{"timestamp":"1795500000000"}},
                {"uri":"spotify:end-group:1a2b3c4d5e6f7081","attributes":{"timestamp":"1795500000000"}}],"addFirst":true}})
        );
        let inside = create_folder_changes("aa", "Sub", &Position::in_folder("bb"), NOW);
        assert_eq!(
            inside.ops()[0]["add"]["addAfterItem"],
            json!({"uri":"spotify:start-group:bb"})
        );
        let rename = rename_folder_changes("hhhh", "tttt", "Road Trip");
        assert_eq!(
            rename.ops(),
            [
                json!({"kind":"ADD","add":{"items":[{"uri":"spotify:start-group:tttt:Road+Trip"}],"addBeforeItem":{"uri":"spotify:start-group:hhhh"}}}),
                json!({"kind":"REM","rem":{"items":[{"uri":"spotify:start-group:hhhh"}],"itemsAsKey":true}}),
                json!({"kind":"ADD","add":{"items":[{"uri":"spotify:start-group:hhhh:Road+Trip"}],"addBeforeItem":{"uri":"spotify:start-group:tttt:Road+Trip"}}}),
                json!({"kind":"REM","rem":{"items":[{"uri":"spotify:start-group:tttt:Road+Trip"}],"itemsAsKey":true}}),
            ]
        );
        assert_eq!(random_hash().len(), 16);
    }

    fn sample() -> Rootlist {
        Rootlist {
            revision: "rev".into(),
            entries: vec![
                playlist_entry("spotify:playlist:a", true),
                start("f1", "Outer"),
                playlist_entry("spotify:playlist:b", false),
                start("f2", "Inner"),
                playlist_entry("spotify:playlist:c", true),
                end("f2"),
                end("f1"),
                playlist_entry("spotify:playlist:d", true),
            ],
        }
    }

    #[test]
    fn builds_the_folder_tree_and_spans() {
        let list = sample();
        let tree = list.tree("someone");
        assert_eq!(tree.len(), 3);
        let RootNode::Folder(outer) = &tree[1] else {
            panic!("expected a folder")
        };
        assert_eq!(outer.uri, "spotify:user:someone:folder:f1");
        assert_eq!((outer.start, outer.length), (1, 6));
        assert_eq!(outer.children.len(), 2);
        let RootNode::Folder(inner) = &outer.children[1] else {
            panic!("expected a nested folder")
        };
        assert_eq!(
            (inner.name.as_str(), inner.start, inner.length),
            ("Inner", 3, 3)
        );
        assert_eq!(
            list.span("f1"),
            Some(Span {
                start: 1,
                length: 6,
                closed: true
            })
        );
        assert_eq!(list.writable_count(), 5);
        assert_eq!(list.default_name(), "My Playlist #6");
    }

    #[test]
    fn tolerates_unbalanced_markers() {
        let list = Rootlist {
            revision: String::new(),
            entries: vec![
                end("stray"),
                start("open", "Open"),
                playlist_entry("spotify:playlist:a", true),
                end("other"),
            ],
        };
        let tree = list.tree("u");
        assert_eq!(tree.len(), 1);
        let RootNode::Folder(folder) = &tree[0] else {
            panic!("expected a folder")
        };
        assert_eq!(
            (folder.start, folder.length, folder.children.len()),
            (1, 3, 1)
        );
        assert!(!list.span("open").unwrap().closed);
        assert!(delete_folder_changes(&list, "open").is_err());
    }

    #[test]
    fn deletes_and_moves_folders_by_index() {
        let list = sample();
        assert_eq!(
            delete_folder_changes(&list, "f2").unwrap().ops()[0],
            json!({"kind":"REM","rem":{"fromIndex":3,"length":3}})
        );
        assert_eq!(
            move_folder_changes(&list, "f2", 0).unwrap().ops()[0],
            json!({"kind":"MOV","mov":{"fromIndex":3,"length":3,"toIndex":0}})
        );
        assert!(move_folder_changes(&list, "f1", 3).is_err());
        assert!(move_folder_changes(&list, "f1", 99).is_err());
        assert!(delete_folder_changes(&list, "missing").is_err());
        assert_eq!(
            move_playlists_changes(&["spotify:playlist:d".into()], &Position::in_folder("f1"))
                .ops()[0],
            json!({"kind":"MOV","mov":{"items":[{"uri":"spotify:playlist:d","attributes":{}}],
                "addAfterItem":{"uri":"spotify:start-group:f1"}}})
        );
    }

    #[test]
    fn parses_rootlist_pages() {
        let value = json!({
            "revision": "AAAAKg==", "length": 4,
            "contents": {"pos": 0, "truncated": true,
                "items": [
                    {"uri": "spotify:start-group:abc:Road+Trip", "attributes": {"timestamp": "1"}},
                    {"uri": "spotify:playlist:p1", "attributes": {"timestamp": "1625150017000", "public": true}},
                    {"uri": "spotify:end-group:abc"}
                ],
                "metaItems": [
                    {},
                    {"attributes": {"name": "Mix", "description": "<a href=\"x\">Hi</a>", "format": "blend",
                        "picture": "q83aZXsDs5Q="},
                     "length": 10, "ownerUsername": "someone",
                     "capabilities": {"canEditItems": true, "canDelete": true, "canCancelMembership": false}},
                    {}
                ]}
        });
        let (list, next) = Rootlist::from_page(&value);
        assert_eq!(next, Some(3));
        assert_eq!(list.revision, "AAAAKg==");
        assert_eq!(list.entries[0], start("abc", "Road Trip"));
        assert_eq!(list.entries[2], end("abc"));
        let playlist = list.playlist("spotify:user:someone:playlist:p1").unwrap();
        assert_eq!(playlist.name, "Mix");
        assert_eq!(playlist.description, "Hi");
        assert!(playlist.public);
        assert_eq!(playlist.added_at, Some(1_625_150_017_000));
        assert_eq!(playlist.format.as_deref(), Some("blend"));
        assert_eq!(playlist.owner.as_deref(), Some("someone"));
        assert_eq!(playlist.picture.as_deref(), Some("q83aZXsDs5Q="));
        assert!(playlist.capabilities.edit_items && playlist.capabilities.delete);
        assert_eq!(
            playlist.cover.as_deref(),
            Some("https://i.scdn.co/image/abcdda657b03b394")
        );
        assert_eq!(list.entries[0].uri(), "spotify:start-group:abc:Road+Trip");
    }

    #[test]
    fn edits_the_rootlist_in_place() {
        let mut list = sample();
        assert!(list.contains("spotify:playlist:b"));
        list.remove("spotify:playlist:b");
        assert!(!list.contains("spotify:playlist:b"));
        list.insert_first(RootPlaylist {
            uri: "spotify:playlist:z".into(),
            ..RootPlaylist::default()
        });
        assert!(
            matches!(&list.entries[0], RootEntry::Playlist(p) if p.uri == "spotify:playlist:z")
        );
        list.playlist_mut("spotify:playlist:z").unwrap().public = true;
        assert!(list.playlist("spotify:playlist:z").unwrap().public);
    }

    #[test]
    fn builds_item_mutations() {
        let playlist = format!("spotify:playlist:{ID}");
        let uris = vec!["spotify:track:a".to_owned()];
        assert_eq!(
            pathfinder::body(
                ADD_TO_PLAYLIST,
                add_variables(&playlist, &uris, &ItemPosition::Bottom)
            ),
            json!({"variables":{"playlistUri":playlist,"playlistItemUris":["spotify:track:a"],
                "newPosition":{"moveType":"BOTTOM_OF_PLAYLIST","fromUid":null},"interactionId":null},
                "operationName":"addToPlaylist",
                "extensions":{"persistedQuery":{"version":1,"sha256Hash":ITEMS_HASH}}})
        );
        assert_eq!(
            remove_variables(&playlist, &["f09953d989bf2f37".into()]),
            json!({"playlistUri":playlist,"uids":["f09953d989bf2f37"]})
        );
        assert_eq!(
            move_variables(
                &playlist,
                &["u1".into()],
                &ItemPosition::BeforeUid("u2".into())
            ),
            json!({"playlistUri":playlist,"uids":["u1"],"newPosition":{"moveType":"BEFORE_UID","fromUid":"u2"}})
        );
        assert_eq!(
            ItemPosition::AfterUid("x".into()).json(),
            json!({"moveType":"AFTER_UID","fromUid":"x"})
        );
        assert_eq!(
            ItemPosition::Top.json()["moveType"],
            json!("TOP_OF_PLAYLIST")
        );
        assert_eq!(ADD_TO_PLAYLIST.kind, pathfinder::Kind::Mutation);
        assert_eq!(PLAYLIST_PERMISSIONS.kind, pathfinder::Kind::Query);
    }

    #[test]
    fn keeps_added_order_across_batches() {
        let uris: Vec<String> = (0..250).map(|n| format!("spotify:track:{n}")).collect();
        let bottom = add_batches(&uris, &ItemPosition::Bottom);
        assert_eq!(bottom.len(), 3);
        assert_eq!(bottom[0][0], "spotify:track:0");
        let top = add_batches(&uris, &ItemPosition::Top);
        assert_eq!(top[0][0], "spotify:track:200");
        let after = add_batches(&uris, &ItemPosition::AfterUid("u".into()));
        assert_eq!(after[2][0], "spotify:track:0");
        let before = add_batches(&uris, &ItemPosition::BeforeUid("u".into()));
        assert_eq!(before[0][0], "spotify:track:0");
    }

    #[test]
    fn reads_permissions() {
        let data = json!({"playlistV2":{"__typename":"Playlist","basePermission":"VIEWER",
        "currentUserCapabilities":{"canAdministratePermissions":true,"canCancelMembership":false,
            "canEditItems":true,"canEditMetadata":true,"canView":true,
            "grantableLevel":["BLOCKED","VIEWER","CONTRIBUTOR"]},
        "members":{"items":[
            {"isOwner":true,"permissionLevel":"CONTRIBUTOR","user":{"data":{"__typename":"User",
                "name":"Owner Name","uri":"spotify:user:owner","username":"owner"}}},
            {"isOwner":false,"permissionLevel":"VIEWER","user":{"data":{"__typename":"User",
                "uri":"spotify:user:guest",
                "avatar":{"sources":[{"url":"https://i.scdn.co/image/small","width":64},{"url":"https://i.scdn.co/image/big","width":640}]}}}}
        ]}}});
        let permissions = parse_permissions(&data).unwrap();
        assert_eq!(permissions.base, Some(Level::Viewer));
        assert!(permissions.administrate && !permissions.cancel_membership);
        assert_eq!(
            permissions.grantable,
            [Level::Blocked, Level::Viewer, Level::Contributor]
        );
        assert_eq!(permissions.owner().unwrap().name, "Owner Name");
        let guest = permissions.member("guest").unwrap();
        assert_eq!(guest.name, "guest");
        assert_eq!(guest.level, Some(Level::Viewer));
        assert_eq!(
            guest.image.as_deref(),
            Some("https://i.scdn.co/image/small")
        );
        assert!(parse_permissions(&json!({"playlistV2":{"__typename":"NotFound"}})).is_err());
    }

    #[test]
    fn builds_permission_bodies() {
        let current = json!({"revision":"ZGVmYXVsdA==","permissionLevel":"VIEWER"});
        assert_eq!(
            base_body(&current, Level::Blocked),
            json!({"revision":"ZGVmYXVsdA==","permissionLevel":"BLOCKED"})
        );
        assert_eq!(
            invite_body(),
            json!({"permission":{"permissionLevel":"CONTRIBUTOR"},"ttlMs":604800000u64})
        );
        assert_eq!(
            invite_url(ID, "a+b/c="),
            format!("https://open.spotify.com/playlist/{ID}?pt=a%2Bb%2Fc%3D")
        );
        let members =
            json!({"memberPermissions":{"guest":{"revision":"cmV2","permissionLevel":"VIEWER"}}});
        assert_eq!(member_revision(&members, "guest").as_deref(), Some("cmV2"));
        assert_eq!(member_revision(&json!({}), "guest"), None);
        assert_eq!(lenient(b"{\"revision\":\"AAAA\"}")["revision"], "AAAA");
        assert_eq!(lenient(b"\x08\x01"), Value::Null);
        assert_eq!(lenient(b""), Value::Null);
        assert_eq!(
            Level::parse(Level::Contributor.name()),
            Some(Level::Contributor)
        );
    }

    #[test]
    fn pages_playlist_contents_with_row_uids() {
        let value = json!({
            "revision": "AAAAAw==", "length": 5,
            "attributes": {"name": "Road", "description": "<b>Drive</b>", "format": "blend",
                "formatAttributes": [{"key": "blend.userinfo-a.name", "value": "A"}],
                "picture": "q83aZXsDs5Q="},
            "ownerUsername": "someone",
            "capabilities": {"canEditItems": true, "canEditMetadata": true, "canDelete": true},
            "contents": {"pos": 0, "truncated": true, "items": [
                {"uri": "spotify:track:a", "attributes": {"addedBy": "someone", "timestamp": "1625150017000",
                    "itemId": "8JlT2Ym/Lzc=",
                    "formatAttributes": [{"key": "multiUserAttributionUsernamesV2", "value": "a,b"}]}},
                {"uri": "spotify:track:a", "attributes": {"itemId": "AAAAAAAAAAE="}},
                {"uri": "spotify:local:Artist+Name:Album:Song+Title:215", "attributes": {}},
                {"uri": "spotify:track:gone", "attributes": {}}
            ]}
        });
        let contents = parse_contents(&value);
        assert_eq!((contents.length, contents.next), (5, Some(4)));
        assert_eq!(contents.items[0].uid.as_deref(), Some("f09953d989bf2f37"));
        assert_eq!(contents.items[0].added_at, Some(1_625_150_017_000));
        assert_eq!(contents.items[0].added_by.as_deref(), Some("someone"));
        assert_eq!(
            contents.items[0].attributes["multiUserAttributionUsernamesV2"],
            "a,b"
        );
        assert_eq!(contents.items[1].uid.as_deref(), Some("0000000000000001"));

        let found = HashMap::from([(
            "spotify:track:a".to_owned(),
            Track {
                uri: "spotify:track:a".into(),
                title: "Song".into(),
                ..Track::default()
            },
        )]);
        let tracks = rows(&contents.items, &found);
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].uid.as_deref(), Some("f09953d989bf2f37"));
        assert_eq!(tracks[1].uid.as_deref(), Some("0000000000000001"));
        assert_eq!(tracks[1].title, "Song");
        assert_eq!(
            (
                tracks[2].title.as_str(),
                tracks[2].artist.as_str(),
                tracks[2].duration_ms
            ),
            ("Song Title", "Artist Name", 215_000)
        );

        let collection =
            build_collection("spotify:user:someone:playlist:x", &value, &contents, tracks);
        assert_eq!(collection.uri, "spotify:playlist:x");
        assert_eq!(collection.description, "Drive");
        assert_eq!((collection.total, collection.next), (5, Some(4)));
        assert_eq!(collection.picture.as_deref(), Some("q83aZXsDs5Q="));
        assert!(collection.capabilities.delete);
        assert_eq!(collection.metadata["format_list_type"], "blend");
        assert_eq!(collection.metadata["blend.userinfo-a.name"], "A");
        assert_eq!(collection.metadata["context_owner"], "spotify:user:someone");

        let last = parse_contents(
            &json!({"length": 2, "contents": {"pos": 1, "items": [{"uri": "spotify:track:z"}]}}),
        );
        assert_eq!(last.next, None);
        assert_eq!(item_uid(""), None);
        assert_eq!(
            local_track("spotify:local:::Only:x").unwrap().duration_ms,
            0
        );
        assert!(local_track("spotify:local:broken").is_none());
    }
}
