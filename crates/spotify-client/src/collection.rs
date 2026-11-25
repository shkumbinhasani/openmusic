use std::collections::HashSet;

use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::{Map, Value, json};

use crate::api::Api;

pub const PROTOBUF: &str = "application/vnd.collection-v2.spotify.proto";
pub const PAGE_LIMIT: u32 = 300;

pub const ARTIST: &str = "artist";
pub const LISTEN_LATER: &str = "listenlater";
pub const IGNORE_IN_RECS: &str = "ignoreinrecs";
pub const PLAY_CONTEXT_BANS: &str = "playcontextbans";
pub const MARKED_AS_FINISHED: &str = "markedasfinished";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub uri: String,
    pub context_uri: Option<String>,
    pub added_at: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<Item>,
    pub next: Option<String>,
    pub sync_token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Write {
    pub uri: String,
    pub context_uri: Option<String>,
    pub removed: bool,
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn base(user: &str, set: &str) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("username".into(), user.into());
    body.insert("set".into(), set.into());
    body
}

fn reference(uri: &str, context_uri: Option<&str>) -> Value {
    let mut item = Map::new();
    item.insert("uri".into(), uri.into());
    if let Some(context) = context_uri {
        item.insert("context_uri".into(), context.into());
    }
    Value::Object(item)
}

pub fn page_body(user: &str, set: &str, token: Option<&str>, limit: u32) -> Value {
    let mut body = base(user, set);
    body.insert("limit".into(), limit.into());
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        body.insert("pagination_token".into(), token.into());
    }
    Value::Object(body)
}

pub fn contains_body(user: &str, set: &str, items: &[(String, Option<String>)]) -> Value {
    let mut body = base(user, set);
    body.insert(
        "items".into(),
        items
            .iter()
            .map(|(uri, context)| reference(uri, context.as_deref()))
            .collect(),
    );
    Value::Object(body)
}

pub fn write_body(user: &str, set: &str, items: &[Write]) -> Value {
    let mut body = base(user, set);
    body.insert(
        "items".into(),
        items
            .iter()
            .map(|item| {
                let mut value = reference(&item.uri, item.context_uri.as_deref());
                if item.removed {
                    value["is_removed"] = json!(true);
                }
                value
            })
            .collect(),
    );
    Value::Object(body)
}

pub fn parse_page(reply: &Value) -> Page {
    let items = reply["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item["is_removed"].as_bool() != Some(true))
        .filter_map(|item| {
            Some(Item {
                uri: text(&item["uri"])?,
                context_uri: text(&item["context_uri"]),
                added_at: item["added_at"]
                    .as_i64()
                    .or_else(|| item["added_at"].as_str()?.parse().ok())
                    .unwrap_or_default(),
            })
        })
        .collect();
    Page {
        items,
        next: text(&reply["next_page_token"]),
        sync_token: text(&reply["sync_token"]),
    }
}

pub fn parse_found(reply: &Value, expected: usize) -> Result<Vec<bool>> {
    let found: Vec<bool> = reply["found"]
        .as_array()
        .context("Spotify did not say which items are saved.")?
        .iter()
        .map(|value| value.as_bool().unwrap_or(false))
        .collect();
    anyhow::ensure!(
        found.len() == expected,
        "Spotify answered for {} of {expected} items.",
        found.len()
    );
    Ok(found)
}

pub async fn page(
    api: &Api,
    user: &str,
    set: &str,
    token: Option<&str>,
    limit: u32,
) -> Result<Page> {
    let reply: Value = api
        .json(
            Method::POST,
            "/collection/v2/paging",
            &page_body(user, set, token, limit),
        )
        .await?;
    Ok(parse_page(&reply))
}

pub async fn all(api: &Api, user: &str, set: &str) -> Result<Vec<Item>> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut token: Option<String> = None;
    loop {
        let page = page(api, user, set, token.as_deref(), PAGE_LIMIT).await?;
        let added = page.items.len();
        items.extend(page.items);
        match page.next {
            Some(next) if added > 0 && seen.insert(next.clone()) => token = Some(next),
            _ => break,
        }
    }
    Ok(items)
}

pub async fn contains(
    api: &Api,
    user: &str,
    set: &str,
    items: &[(String, Option<String>)],
) -> Result<Vec<bool>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let reply: Value = api
        .json(
            Method::POST,
            "/collection/v2/contains",
            &contains_body(user, set, items),
        )
        .await?;
    parse_found(&reply, items.len())
}

pub async fn write(api: &Api, user: &str, set: &str, items: &[Write]) -> Result<()> {
    if items.is_empty() {
        return Ok(());
    }
    let _: Value = api
        .json(
            Method::POST,
            "/collection/v2/write",
            &write_body(user, set, items),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_paging_bodies() {
        assert_eq!(
            page_body("someone", "collection", None, 2),
            json!({"username": "someone", "set": "collection", "limit": 2})
        );
        assert_eq!(
            page_body("someone", ARTIST, Some("next"), PAGE_LIMIT),
            json!({"username": "someone", "set": "artist", "limit": 300, "pagination_token": "next"})
        );
    }

    #[test]
    fn builds_contains_bodies_with_optional_contexts() {
        let body = contains_body(
            "someone",
            PLAY_CONTEXT_BANS,
            &[
                ("spotify:track:70LcF31zb1H0PyJoS1Sx1r".into(), None),
                (
                    "spotify:track:70LcF31zb1H0PyJoS1Sx1r".into(),
                    Some("spotify:playlist:37i9dQZF1E8Redacted001".into()),
                ),
            ],
        );
        assert_eq!(
            body,
            json!({"username": "someone", "set": "playcontextbans", "items": [
                {"uri": "spotify:track:70LcF31zb1H0PyJoS1Sx1r"},
                {"uri": "spotify:track:70LcF31zb1H0PyJoS1Sx1r", "context_uri": "spotify:playlist:37i9dQZF1E8Redacted001"}
            ]})
        );
    }

    #[test]
    fn builds_write_bodies_for_adds_and_removals() {
        let body = write_body(
            "someone",
            IGNORE_IN_RECS,
            &[
                Write {
                    uri: "spotify:playlist:37i9dQZF1E8Redacted001".into(),
                    context_uri: None,
                    removed: false,
                },
                Write {
                    uri: "spotify:track:70LcF31zb1H0PyJoS1Sx1r".into(),
                    context_uri: Some("spotify:album:3gBVdu4a1MMJVMy6vwPEb8".into()),
                    removed: true,
                },
            ],
        );
        assert_eq!(
            body,
            json!({"username": "someone", "set": "ignoreinrecs", "items": [
                {"uri": "spotify:playlist:37i9dQZF1E8Redacted001"},
                {"uri": "spotify:track:70LcF31zb1H0PyJoS1Sx1r", "context_uri": "spotify:album:3gBVdu4a1MMJVMy6vwPEb8", "is_removed": true}
            ]})
        );
    }

    #[test]
    fn reads_pages() {
        let page = parse_page(&json!({
            "items": [
                {"uri": "spotify:album:6dVIqQ8qmQ5GBnJ9shOYGE", "added_at": "1725636253"},
                {"uri": "spotify:track:70LcF31zb1H0PyJoS1Sx1r", "added_at": 1725636000, "context_uri": "spotify:playlist:x"},
                {"uri": "spotify:track:gone", "added_at": 1, "is_removed": true},
                {"added_at": 2}
            ],
            "next_page_token": "token"
        }));
        assert_eq!(
            page,
            Page {
                items: vec![
                    Item {
                        uri: "spotify:album:6dVIqQ8qmQ5GBnJ9shOYGE".into(),
                        context_uri: None,
                        added_at: 1725636253,
                    },
                    Item {
                        uri: "spotify:track:70LcF31zb1H0PyJoS1Sx1r".into(),
                        context_uri: Some("spotify:playlist:x".into()),
                        added_at: 1725636000,
                    },
                ],
                next: Some("token".into()),
                sync_token: None,
            }
        );
        let empty = parse_page(&json!({"sync_token": "sync"}));
        assert!(empty.items.is_empty());
        assert_eq!(empty.next, None);
        assert_eq!(empty.sync_token.as_deref(), Some("sync"));
    }

    #[test]
    fn reads_contains_answers() {
        assert_eq!(
            parse_found(&json!({"found": [true, false, true]}), 3).unwrap(),
            vec![true, false, true]
        );
        assert!(parse_found(&json!({"found": [true]}), 2).is_err());
        assert!(parse_found(&json!({}), 1).is_err());
    }
}
