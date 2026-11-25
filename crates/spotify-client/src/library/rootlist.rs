use std::collections::HashSet;

use anyhow::Result;
use serde_json::Value;

use crate::{
    api::Api,
    catalog::{self, Playlist},
};

pub const ROOT_HASH: &str = "0000000000000000";
const DECORATE: &str = "revision,length,attributes,timestamp,owner,capabilities";
const PAGE_LENGTH: usize = 200;
const MAX_PAGES: usize = 100;

#[derive(Clone, Debug)]
pub enum RootEntry {
    Playlist(Playlist),
    Unavailable(String),
    FolderStart { hash: String, name: String },
    FolderEnd { hash: String },
    Other(String),
}

#[derive(Clone, Debug)]
pub struct RootItem {
    pub entry: RootEntry,
    pub added_at: Option<i64>,
}

#[derive(Clone, Debug, Default)]
pub struct Rootlist {
    pub user: String,
    pub revision: Option<String>,
    pub items: Vec<RootItem>,
}

#[derive(Clone, Debug)]
pub enum RootNode {
    Playlist {
        playlist: Playlist,
        added_at: Option<i64>,
        index: usize,
    },
    Unavailable {
        uri: String,
        index: usize,
    },
    Folder(RootFolder),
}

#[derive(Clone, Debug)]
pub struct RootFolder {
    pub uri: String,
    pub hash: String,
    pub name: String,
    pub added_at: Option<i64>,
    pub start: usize,
    pub raw_count: usize,
    pub children: Vec<RootNode>,
}

impl RootFolder {
    pub fn playlist_count(&self) -> usize {
        self.children
            .iter()
            .filter(|node| !matches!(node, RootNode::Folder(_)))
            .count()
    }

    pub fn folder_count(&self) -> usize {
        self.children.len() - self.playlist_count()
    }
}

#[derive(Clone, Debug)]
pub struct RootPage {
    pub revision: Option<String>,
    pub length: usize,
    pub pos: usize,
    pub truncated: bool,
    pub items: Vec<RootItem>,
}

pub fn folder_uri(user: &str, hash: &str) -> String {
    format!("spotify:user:{user}:folder:{hash}")
}

pub fn folder_hash(uri: &str) -> Option<&str> {
    let mut parts = uri.split(':');
    match (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) {
        (Some("spotify"), Some("user"), Some(_), Some("folder"), Some(hash), None)
            if !hash.is_empty() =>
        {
            Some(hash)
        }
        _ => None,
    }
}

fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

pub fn decode_name(encoded: &str) -> String {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| {
                Some((
                    hex_digit(*bytes.get(index + 1)?)?,
                    hex_digit(*bytes.get(index + 2)?)?,
                ))
            })
            .flatten();
        match (bytes[index], escaped) {
            (_, Some((high, low))) => {
                decoded.push(high << 4 | low);
                index += 2;
            }
            (b'+', _) => decoded.push(b' '),
            (byte, _) => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn playlist_uri(uri: &str) -> Option<String> {
    let parts: Vec<&str> = uri.split(':').collect();
    match parts.as_slice() {
        ["spotify", "playlist", id] | ["spotify", "user", _, "playlist", id] if !id.is_empty() => {
            Some(format!("spotify:playlist:{id}"))
        }
        _ => None,
    }
}

fn timestamp(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

pub fn root_item(item: &Value, meta: &Value) -> RootItem {
    let uri = item["uri"].as_str().unwrap_or_default();
    let added_at = timestamp(&item["attributes"]["timestamp"]);
    let entry = if let Some(rest) = uri.strip_prefix("spotify:start-group:") {
        match rest.split_once(':') {
            Some((hash, name)) if !hash.is_empty() => RootEntry::FolderStart {
                hash: hash.to_owned(),
                name: decode_name(name),
            },
            _ => RootEntry::Other(uri.to_owned()),
        }
    } else if let Some(hash) = uri.strip_prefix("spotify:end-group:") {
        if hash.is_empty() || hash.contains(':') {
            RootEntry::Other(uri.to_owned())
        } else {
            RootEntry::FolderEnd {
                hash: hash.to_owned(),
            }
        }
    } else if let Some(uri) = playlist_uri(uri) {
        let attributes = &meta["attributes"];
        match meta["ownerUsername"].as_str() {
            Some(owner) if attributes.is_object() => RootEntry::Playlist(Playlist {
                uri,
                name: attributes["name"].as_str().unwrap_or("Untitled").to_owned(),
                description: catalog::strip_tags(
                    attributes["description"].as_str().unwrap_or_default(),
                ),
                cover: catalog::playlist_cover(attributes),
                length: meta["length"].as_u64().unwrap_or_default() as u32,
                format: catalog::filled(&attributes["format"]),
                owner: Some(owner.to_owned()).filter(|owner| !owner.is_empty()),
            }),
            _ => RootEntry::Unavailable(uri),
        }
    } else {
        RootEntry::Other(uri.to_owned())
    };
    RootItem { entry, added_at }
}

pub fn parse_page(value: &Value) -> RootPage {
    let contents = &value["contents"];
    let meta = contents["metaItems"].as_array();
    let items = contents["items"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, item)| {
            let meta = meta
                .and_then(|meta| meta.get(index))
                .unwrap_or(&Value::Null);
            root_item(item, meta)
        })
        .collect();
    RootPage {
        revision: catalog::filled(&value["revision"]),
        length: value["length"].as_u64().unwrap_or_default() as usize,
        pos: contents["pos"].as_u64().unwrap_or_default() as usize,
        truncated: contents["truncated"].as_bool().unwrap_or(false),
        items,
    }
}

pub fn path(user: &str, from: Option<usize>) -> String {
    let mut path = format!("/playlist/v2/user/{user}/rootlist?decorate={DECORATE}");
    if let Some(from) = from {
        path.push_str(&format!("&from={from}&length={PAGE_LENGTH}"));
    }
    path
}

pub async fn rootlist(api: &Api, user: &str) -> Result<Rootlist> {
    let mut restarted = false;
    'fetch: loop {
        let first = parse_page(&api.get_json::<Value>(&path(user, None)).await?);
        let revision = first.revision.clone();
        let mut truncated = first.truncated;
        let mut items = first.items;
        let mut pages = 1;
        while truncated && pages < MAX_PAGES {
            let from = items.len();
            let page = parse_page(&api.get_json::<Value>(&path(user, Some(from))).await?);
            if page.revision != revision && !restarted {
                restarted = true;
                continue 'fetch;
            }
            if page.items.is_empty() || page.pos != from {
                break;
            }
            truncated = page.truncated;
            items.extend(page.items);
            pages += 1;
        }
        return Ok(Rootlist {
            user: user.to_owned(),
            revision,
            items,
        });
    }
}

fn attach(stack: &mut [RootFolder], root: &mut Vec<RootNode>, node: RootNode) {
    match stack.last_mut() {
        Some(folder) => folder.children.push(node),
        None => root.push(node),
    }
}

fn close(stack: &mut Vec<RootFolder>, root: &mut Vec<RootNode>, keep: usize) {
    while stack.len() > keep {
        let Some(folder) = stack.pop() else { break };
        attach(stack, root, RootNode::Folder(folder));
    }
}

fn find<'a>(nodes: &'a [RootNode], uri: &str) -> Option<&'a RootFolder> {
    nodes.iter().find_map(|node| match node {
        RootNode::Folder(folder) if folder.uri == uri => Some(folder),
        RootNode::Folder(folder) => find(&folder.children, uri),
        _ => None,
    })
}

impl Rootlist {
    pub fn playlists(&self) -> Vec<Playlist> {
        let mut seen = HashSet::new();
        self.items
            .iter()
            .filter_map(|item| match &item.entry {
                RootEntry::Playlist(playlist) if seen.insert(playlist.uri.clone()) => {
                    Some(playlist.clone())
                }
                _ => None,
            })
            .collect()
    }

    pub fn tree(&self) -> Vec<RootNode> {
        let mut root = Vec::new();
        let mut stack: Vec<RootFolder> = Vec::new();
        let mut seen = HashSet::new();
        for (index, item) in self.items.iter().enumerate() {
            for folder in &mut stack {
                folder.raw_count += 1;
            }
            match &item.entry {
                RootEntry::Playlist(playlist) => attach(
                    &mut stack,
                    &mut root,
                    RootNode::Playlist {
                        playlist: playlist.clone(),
                        added_at: item.added_at,
                        index,
                    },
                ),
                RootEntry::Unavailable(uri) => attach(
                    &mut stack,
                    &mut root,
                    RootNode::Unavailable {
                        uri: uri.clone(),
                        index,
                    },
                ),
                RootEntry::FolderStart { hash, name } => {
                    if seen.insert(hash.clone()) {
                        stack.push(RootFolder {
                            uri: folder_uri(&self.user, hash),
                            hash: hash.clone(),
                            name: name.clone(),
                            added_at: item.added_at,
                            start: index,
                            raw_count: 1,
                            children: Vec::new(),
                        });
                    }
                }
                RootEntry::FolderEnd { hash } => {
                    if let Some(position) = stack.iter().position(|folder| folder.hash == *hash) {
                        close(&mut stack, &mut root, position);
                    }
                }
                RootEntry::Other(_) => {}
            }
        }
        close(&mut stack, &mut root, 0);
        root
    }

    pub fn folder(&self, uri: &str) -> Option<RootFolder> {
        let hash = folder_hash(uri)?;
        let tree = self.tree();
        find(&tree, uri)
            .or_else(|| find(&tree, &folder_uri(&self.user, hash)))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn playlist(id: &str, name: &str) -> (Value, Value) {
        (
            json!({"uri": format!("spotify:playlist:{id}"), "attributes": {"timestamp": "1700000000000"}}),
            json!({"attributes": {"name": name}, "length": 3, "ownerUsername": "someone"}),
        )
    }

    fn marker(uri: &str) -> (Value, Value) {
        (json!({"uri": uri}), json!({}))
    }

    fn rootlist(entries: Vec<(Value, Value)>) -> Rootlist {
        let (items, meta): (Vec<Value>, Vec<Value>) = entries.into_iter().unzip();
        let page = parse_page(&json!({
            "revision": "AAAAAQ==",
            "length": items.len(),
            "contents": {"pos": 0, "truncated": false, "items": items, "metaItems": meta}
        }));
        Rootlist {
            user: "someone".into(),
            revision: page.revision,
            items: page.items,
        }
    }

    fn outline(nodes: &[RootNode], depth: usize, lines: &mut Vec<String>) {
        for node in nodes {
            let indent = "  ".repeat(depth);
            match node {
                RootNode::Playlist {
                    playlist, index, ..
                } => lines.push(format!("{indent}{} @{index}", playlist.name)),
                RootNode::Unavailable { uri, index } => {
                    lines.push(format!("{indent}unavailable {uri} @{index}"))
                }
                RootNode::Folder(folder) => {
                    lines.push(format!(
                        "{indent}[{}] start={} raw={}",
                        folder.name, folder.start, folder.raw_count
                    ));
                    outline(&folder.children, depth + 1, lines);
                }
            }
        }
    }

    fn lines(list: &Rootlist) -> Vec<String> {
        let mut lines = Vec::new();
        outline(&list.tree(), 0, &mut lines);
        lines
    }

    #[test]
    fn reads_playlists_markers_and_placeholders_in_step_with_meta_items() {
        let list = rootlist(vec![
            marker("spotify:start-group:1a2b3c4d5e6f7081:Road+Trip%20%F0%9F%9A%97"),
            playlist("0000000000000000000001", "First"),
            marker("spotify:end-group:1a2b3c4d5e6f7081"),
            (
                json!({"uri": "spotify:user:someone:playlist:0000000000000000000002"}),
                json!({"attributes": {"name": "Old style"}, "ownerUsername": "someone"}),
            ),
            (
                json!({"uri": "spotify:playlist:0000000000000000000003"}),
                json!({}),
            ),
            marker("spotify:start-group:abcdef"),
        ]);
        assert_eq!(list.revision.as_deref(), Some("AAAAAQ=="));
        assert!(matches!(
            &list.items[0].entry,
            RootEntry::FolderStart { hash, name } if hash == "1a2b3c4d5e6f7081" && name == "Road Trip 🚗"
        ));
        let playlists = list.playlists();
        assert_eq!(playlists.len(), 2);
        assert_eq!(playlists[0].name, "First");
        assert_eq!(playlists[0].length, 3);
        assert_eq!(playlists[0].owner.as_deref(), Some("someone"));
        assert_eq!(list.items[1].added_at, Some(1_700_000_000_000));
        assert_eq!(playlists[1].uri, "spotify:playlist:0000000000000000000002");
        assert!(matches!(&list.items[4].entry, RootEntry::Unavailable(uri) if uri.ends_with("3")));
        assert!(matches!(&list.items[5].entry, RootEntry::Other(_)));
    }

    #[test]
    fn decodes_folder_names() {
        assert_eq!(decode_name("New+Folder"), "New Folder");
        assert_eq!(decode_name("Rock%2BRoll"), "Rock+Roll");
        assert_eq!(decode_name("100%25"), "100%");
        assert_eq!(decode_name("bad%zzname%"), "bad%zzname%");
        assert_eq!(decode_name("%C3%A9t%C3%A9"), "été");
    }

    #[test]
    fn builds_the_folder_tree_like_the_desktop_app() {
        let list = rootlist(vec![
            playlist("0000000000000000000001", "Top"),
            marker("spotify:start-group:aaaaaaaaaaaaaaaa:Outer"),
            playlist("0000000000000000000002", "In outer"),
            marker("spotify:start-group:bbbbbbbbbbbbbbbb:Inner"),
            playlist("0000000000000000000003", "In inner"),
            marker("spotify:end-group:bbbbbbbbbbbbbbbb"),
            marker("spotify:end-group:aaaaaaaaaaaaaaaa"),
            playlist("0000000000000000000004", "After"),
        ]);
        assert_eq!(
            lines(&list),
            vec![
                "Top @0",
                "[Outer] start=1 raw=6",
                "  In outer @2",
                "  [Inner] start=3 raw=3",
                "    In inner @4",
                "After @7",
            ]
        );
        let tree = list.tree();
        let RootNode::Folder(outer) = &tree[1] else {
            panic!("second node is the folder");
        };
        assert_eq!(outer.uri, "spotify:user:someone:folder:aaaaaaaaaaaaaaaa");
        assert_eq!((outer.playlist_count(), outer.folder_count()), (1, 1));
        let inner = list
            .folder("spotify:user:someone:folder:bbbbbbbbbbbbbbbb")
            .unwrap();
        assert_eq!(inner.name, "Inner");
        assert!(list.folder("spotify:user:someone:folder:cccc").is_none());
    }

    #[test]
    fn tolerates_unbalanced_markers() {
        let list = rootlist(vec![
            marker("spotify:end-group:ffffffffffffffff"),
            marker("spotify:start-group:aaaaaaaaaaaaaaaa:Outer"),
            marker("spotify:start-group:bbbbbbbbbbbbbbbb:Inner"),
            playlist("0000000000000000000001", "Deep"),
            marker("spotify:end-group:aaaaaaaaaaaaaaaa"),
            marker("spotify:start-group:aaaaaaaaaaaaaaaa:Again"),
            playlist("0000000000000000000002", "Root"),
            marker("spotify:start-group:cccccccccccccccc:Open"),
            playlist("0000000000000000000003", "Never closed"),
        ]);
        assert_eq!(
            lines(&list),
            vec![
                "[Outer] start=1 raw=4",
                "  [Inner] start=2 raw=3",
                "    Deep @3",
                "Root @6",
                "[Open] start=7 raw=2",
                "  Never closed @8",
            ]
        );
    }

    #[test]
    fn reads_folder_hashes_from_uris() {
        assert_eq!(
            folder_hash("spotify:user:someone:folder:1a2b3c4d5e6f7081"),
            Some("1a2b3c4d5e6f7081")
        );
        assert_eq!(folder_hash("spotify:user:someone:folder:"), None);
        assert_eq!(folder_hash("spotify:playlist:1a2b"), None);
    }

    #[test]
    fn asks_for_every_item_and_pages_when_truncated() {
        assert_eq!(
            path("someone", None),
            "/playlist/v2/user/someone/rootlist?decorate=revision,length,attributes,timestamp,owner,capabilities"
        );
        assert!(path("someone", Some(200)).ends_with("&from=200&length=200"));
        let page = parse_page(&json!({
            "revision": "r",
            "length": 450,
            "contents": {"pos": 200, "truncated": true, "items": [{"uri": "spotify:playlist:x"}], "metaItems": []}
        }));
        assert_eq!((page.pos, page.length, page.truncated), (200, 450, true));
        assert!(matches!(&page.items[0].entry, RootEntry::Unavailable(_)));
    }
}
