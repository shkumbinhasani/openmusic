mod liked;
mod page;
mod recents;
mod rootlist;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::Api,
    catalog,
    dealer::Message,
    pathfinder::{self, Operation},
};
pub use liked::{
    FETCH_LIBRARY_TRACKS, LIKED_PAGE_SIZE, LikedPage, liked, liked_variables, parse_liked,
};
pub use page::{
    ALPHABETICAL, CREATOR, CUSTOM_ORDER, Chip, FEATURES, ItemKind, LIBRARY_V3, LIKED_SONGS,
    LibraryError, LibraryItem, LibraryPage, LibraryQuery, PAGE_SIZE, PLAYLISTS, RECENTLY_ADDED,
    RECENTS, YOUR_EPISODES, count_label, fallback_page, folder_detail, iso_ms, now_ms, page,
    parse_page, variables, wants_fallback,
};
pub use recents::{
    Activity, Content, DAY_MS, FETCH_ENTITIES_FOR_RECENTLY_PLAYED, PlayedContext,
    RECENTLY_PLAYED_LIMIT, RECENTS_LIST, RECENTS_PAGE, RECENTS_PAGE_SIZE, RecentDay, RecentEntry,
    RecentGroup, RecentKind, RecentsPage, context_uri, day_label, day_of, decorate, decorated,
    fallback_recents, group_count, is_your_episodes, lookup_entry, merge_played, parse_played,
    parse_recents, recent_contexts, recent_days, recently_played, recently_played_path, recents,
    recents_variables,
};
pub use rootlist::{
    ROOT_HASH, RootEntry, RootFolder, RootItem, RootNode, RootPage, Rootlist, decode_name,
    folder_hash, folder_uri, rootlist,
};

pub const PIN_LIBRARY_ITEM: Operation = Operation::mutation(
    "pinLibraryItem",
    "896ebcb47815681340860d121cb5d494e157e2a78d3950385cd54e0393c67148",
);
pub const UNPIN_LIBRARY_ITEM: Operation = Operation::mutation(
    "unpinLibraryItem",
    "896ebcb47815681340860d121cb5d494e157e2a78d3950385cd54e0393c67148",
);
pub const DEFAULT_PIN_LIMIT: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinResult {
    Done,
    LimitReached(u32),
    InFolder,
    NotSupported,
    NotInLibrary,
    Refused,
}

pub fn pinnable_uri(uri: &str, user: &str) -> String {
    match uri {
        "spotify:collection:tracks" | "spotify:collection" => {
            format!("spotify:user:{user}:collection")
        }
        "spotify:collection:your-episodes" => {
            format!("spotify:user:{user}:collection:your-episodes")
        }
        uri => uri.to_owned(),
    }
}

pub fn pin_request(uri: &str, user: &str, pinned: bool) -> (Operation, Value) {
    let operation = if pinned {
        PIN_LIBRARY_ITEM
    } else {
        UNPIN_LIBRARY_ITEM
    };
    (
        operation,
        json!({ "pinnableItemUri": pinnable_uri(uri, user) }),
    )
}

pub fn parse_pin(data: &Value) -> PinResult {
    let result = &data["pinItemInLibrary"];
    match result["pinResult"].as_str() {
        Some("SUCCESSFUL") => PinResult::Done,
        Some("FAILED_ITEM_LIMIT_REACHED") => PinResult::LimitReached(
            result["pinLimit"]
                .as_u64()
                .map_or(DEFAULT_PIN_LIMIT, |limit| limit as u32),
        ),
        Some("FAILED_ITEM_IN_FOLDER") => PinResult::InFolder,
        Some("FAILED_ITEM_NOT_SUPPORTED") => PinResult::NotSupported,
        Some("FAILED_NOT_IN_YOUR_LIBRARY") => PinResult::NotInLibrary,
        _ => PinResult::Refused,
    }
}

pub async fn pin(api: &Api, user: &str, uri: &str, pinned: bool) -> Result<PinResult> {
    let (operation, variables) = pin_request(uri, user, pinned);
    let data = pathfinder::mutate(api, operation, variables).await?;
    Ok(if pinned {
        parse_pin(&data)
    } else {
        PinResult::Done
    })
}

pub const ARE_ENTITIES_IN_LIBRARY: Operation = Operation::query(
    "areEntitiesInLibrary",
    "134337999233cc6fdd6b1e6dbf94841409f04a946c5c7b744b09ba0dfe5a85ed",
);
pub const ADD_TO_LIBRARY: Operation = Operation::mutation(
    "addToLibrary",
    "896ebcb47815681340860d121cb5d494e157e2a78d3950385cd54e0393c67148",
);
pub const REMOVE_FROM_LIBRARY: Operation = Operation::mutation(
    "removeFromLibrary",
    "896ebcb47815681340860d121cb5d494e157e2a78d3950385cd54e0393c67148",
);

pub const BATCH: usize = 50;

pub const SAVED_SETS: &[&str] = &["collection", "artist", "show", "listenlater", "author"];

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    pub uri: String,
    pub name: String,
}

pub fn savable(uri: &str) -> bool {
    let mut parts = uri.split(':');
    parts.next() == Some("spotify")
        && matches!(
            parts.next(),
            Some("track" | "album" | "artist" | "show" | "episode" | "author")
        )
        && parts.next().is_some_and(|id| !id.is_empty())
        && parts.next().is_none()
}

pub async fn contains(api: &Api, uris: &[String]) -> Result<Vec<Option<bool>>> {
    let mut found = Vec::with_capacity(uris.len());
    for chunk in uris.chunks(BATCH) {
        let data =
            pathfinder::query(api, ARE_ENTITIES_IN_LIBRARY, json!({ "uris": chunk })).await?;
        found.extend(parse_lookup(&data, chunk.len())?);
    }
    Ok(found)
}

pub fn parse_lookup(data: &Value, expected: usize) -> Result<Vec<Option<bool>>> {
    let Some(lookup) = data.get("lookup").and_then(Value::as_array) else {
        bail!("Spotify did not say what is in your library.");
    };
    if lookup.len() != expected {
        bail!(
            "Spotify answered for {} items instead of {expected}.",
            lookup.len()
        );
    }
    Ok(lookup
        .iter()
        .map(|entry| entry.pointer("/data/saved").and_then(Value::as_bool))
        .collect())
}

pub async fn save(api: &Api, uris: &[String], saved: bool) -> Result<()> {
    if let Some(uri) = uris.iter().find(|uri| !savable(uri)) {
        bail!("OpenMusic can’t save {uri} to your library.");
    }
    for chunk in uris.chunks(BATCH) {
        let (operation, variables) = save_request(chunk, saved);
        pathfinder::mutate(api, operation, variables).await?;
    }
    Ok(())
}

pub fn save_request(uris: &[String], saved: bool) -> (Operation, Value) {
    if saved {
        (
            ADD_TO_LIBRARY,
            json!({ "libraryItemUris": uris, "interactionId": null }),
        )
    } else {
        (REMOVE_FROM_LIBRARY, json!({ "libraryItemUris": uris }))
    }
}

pub fn collection_push(message: &Message) -> Option<Vec<(String, bool)>> {
    let set = message
        .uri
        .strip_prefix("hm://collection/")?
        .split('/')
        .next()?;
    if !SAVED_SETS.contains(&set) {
        return None;
    }
    let decoded;
    let payload = match message.json() {
        Some(payload) => payload,
        None => {
            decoded = serde_json::from_slice::<Value>(&message.bytes()?).ok()?;
            &decoded
        }
    };
    let items = payload.get("items")?.as_array()?;
    Some(
        items
            .iter()
            .filter_map(|item| {
                let kind = item.get("type")?.as_str()?;
                let id = item.get("identifier")?.as_str()?;
                let uri = item_uri(kind, id).filter(|uri| savable(uri))?;
                let removed = item
                    .get("removed")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                Some((uri, !removed))
            })
            .collect(),
    )
}

fn item_uri(kind: &str, id: &str) -> Option<String> {
    if id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let gid = (0..16)
            .map(|index| u8::from_str_radix(&id[index * 2..index * 2 + 2], 16).ok())
            .collect::<Option<Vec<u8>>>()?;
        return catalog::uri_from_gid(kind, &gid);
    }
    (id.len() == 22 && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .then(|| format!("spotify:{kind}:{id}"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use base64::Engine;

    use super::*;

    fn message(uri: &str, payload: Value) -> Message {
        Message {
            uri: uri.to_owned(),
            headers: HashMap::new(),
            payloads: vec![payload],
        }
    }

    #[test]
    fn knows_which_uris_the_library_accepts() {
        assert!(savable("spotify:track:4uLU6hMCjMI75M1A2tKUQC"));
        assert!(savable("spotify:album:1b5KESuOMQomF3wntoLAsk"));
        assert!(savable("spotify:show:5CfCWKI5pZ28U0uOzXkDHe"));
        assert!(!savable("spotify:playlist:37i9dQZF1DXcBWIGoYBM5M"));
        assert!(!savable("spotify:user:someone"));
        assert!(!savable("spotify:local:a:b:c:1"));
        assert!(!savable("spotify:track:"));
        assert!(!savable("track:4uLU6hMCjMI75M1A2tKUQC"));
    }

    #[test]
    fn reads_saved_flags_in_order() {
        let data = json!({"lookup": [
            {"__typename": "TrackResponseWrapper", "data": {"__typename": "Track", "saved": true}},
            {"__typename": "PlaylistResponseWrapper"},
            {"__typename": "EpisodeOrChapterResponseWrapper", "data": {"__typename": "RestrictedContent"}},
            {"__typename": "ArtistResponseWrapper", "data": {"__typename": "Artist", "saved": false}}
        ]});
        assert_eq!(
            parse_lookup(&data, 4).unwrap(),
            vec![Some(true), None, None, Some(false)]
        );
        assert!(parse_lookup(&data, 3).is_err());
        assert!(parse_lookup(&json!({}), 0).is_err());
    }

    #[test]
    fn builds_add_and_remove_variables() {
        let uris = vec!["spotify:track:4uLU6hMCjMI75M1A2tKUQC".to_owned()];
        let (operation, variables) = save_request(&uris, true);
        assert_eq!(operation, ADD_TO_LIBRARY);
        assert_eq!(
            variables,
            json!({"libraryItemUris": ["spotify:track:4uLU6hMCjMI75M1A2tKUQC"], "interactionId": null})
        );
        let (operation, variables) = save_request(&uris, false);
        assert_eq!(operation, REMOVE_FROM_LIBRARY);
        assert_eq!(
            variables,
            json!({"libraryItemUris": ["spotify:track:4uLU6hMCjMI75M1A2tKUQC"]})
        );
    }

    #[test]
    fn reads_collection_pushes() {
        let body = json!({"items": [
            {"type": "track", "identifier": "4uLU6hMCjMI75M1A2tKUQC", "removed": false},
            {"type": "album", "identifier": "00000000000000000000000000000001", "removed": true},
            {"type": "podcast_chapter", "identifier": "4uLU6hMCjMI75M1A2tKUQC"},
            {"type": "track", "identifier": "short"}
        ]});
        let encoded = base64::engine::general_purpose::STANDARD.encode(body.to_string());
        let pushed = collection_push(&message(
            "hm://collection/collection/someone/json",
            Value::String(encoded),
        ))
        .unwrap();
        assert_eq!(
            pushed,
            vec![
                ("spotify:track:4uLU6hMCjMI75M1A2tKUQC".to_owned(), true),
                ("spotify:album:0000000000000000000001".to_owned(), false),
            ]
        );
        let plain = collection_push(&message("hm://collection/artist/someone/json", body)).unwrap();
        assert_eq!(plain.len(), 2);
    }

    #[test]
    fn pins_with_the_web_player_uris() {
        assert_eq!(
            pinnable_uri("spotify:collection:tracks", "someone"),
            "spotify:user:someone:collection"
        );
        assert_eq!(
            pinnable_uri("spotify:collection:your-episodes", "someone"),
            "spotify:user:someone:collection:your-episodes"
        );
        assert_eq!(
            pinnable_uri("spotify:playlist:37i9dQZF1FgnTBfUlzkeKt", "someone"),
            "spotify:playlist:37i9dQZF1FgnTBfUlzkeKt"
        );
        let (operation, variables) = pin_request("spotify:collection:tracks", "someone", true);
        assert_eq!(operation, PIN_LIBRARY_ITEM);
        assert_eq!(
            variables,
            json!({"pinnableItemUri": "spotify:user:someone:collection"})
        );
        let (operation, variables) =
            pin_request("spotify:user:someone:folder:1a2b", "someone", false);
        assert_eq!(operation, UNPIN_LIBRARY_ITEM);
        assert_eq!(
            variables,
            json!({"pinnableItemUri": "spotify:user:someone:folder:1a2b"})
        );
    }

    #[test]
    fn reads_pin_results() {
        let result = |value: Value| parse_pin(&json!({ "pinItemInLibrary": value }));
        assert_eq!(
            result(json!({"pinResult": "SUCCESSFUL", "pinLimit": 4})),
            PinResult::Done
        );
        assert_eq!(
            result(json!({"pinResult": "FAILED_ITEM_LIMIT_REACHED", "pinLimit": 5})),
            PinResult::LimitReached(5)
        );
        assert_eq!(
            result(json!({"pinResult": "FAILED_ITEM_LIMIT_REACHED"})),
            PinResult::LimitReached(4)
        );
        assert_eq!(
            result(json!({"pinResult": "FAILED_ITEM_IN_FOLDER"})),
            PinResult::InFolder
        );
        assert_eq!(
            result(json!({"pinResult": "FAILED_ITEM_NOT_SUPPORTED"})),
            PinResult::NotSupported
        );
        assert_eq!(
            result(json!({"pinResult": "FAILED_NOT_IN_YOUR_LIBRARY"})),
            PinResult::NotInLibrary
        );
        assert_eq!(result(Value::Null), PinResult::Refused);
    }

    #[test]
    fn ignores_other_collection_sets() {
        let body = json!({"items": [{"type": "track", "identifier": "4uLU6hMCjMI75M1A2tKUQC"}]});
        assert!(
            collection_push(&message(
                "hm://collection/ignoreinrecs/someone/json",
                body.clone()
            ))
            .is_none()
        );
        assert!(
            collection_push(&message("hm://playlist/v2/user/someone/rootlist", body)).is_none()
        );
    }
}
