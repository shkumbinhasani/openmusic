use std::collections::HashSet;

use anyhow::Result;
use serde_json::{Value, json};

use super::ShowKind;
use crate::{
    api::Api,
    entity::{self, Card, Kind},
    pathfinder::{self, Operation},
};

pub const SIMILAR_AUDIOBOOKS: Operation = Operation::query(
    "similarAudiobooks",
    "d62da1bbf5859e5a156da1f52ce21a73a642f925db717a179fd89626b27a80b8",
);
pub const SIMILAR_PODCASTS: Operation = Operation::query(
    "internalLinkRecommenderShow",
    "6c369ff272a666b31fef1629c169925a1bd80f372195396c82304142cacd89e8",
);
pub const LIMIT: usize = 20;

pub fn title(kind: ShowKind) -> &'static str {
    match kind {
        ShowKind::Audiobook => "More audiobooks like this",
        ShowKind::Podcast => "More podcasts like this",
    }
}

pub fn request(uri: &str, kind: ShowKind) -> (Operation, Value) {
    let operation = match kind {
        ShowKind::Audiobook => SIMILAR_AUDIOBOOKS,
        ShowKind::Podcast => SIMILAR_PODCASTS,
    };
    (operation, json!({ "uri": uri }))
}

fn wanted(kind: ShowKind) -> Kind {
    match kind {
        ShowKind::Audiobook => Kind::Audiobook,
        ShowKind::Podcast => Kind::Show,
    }
}

pub fn parse(data: &Value, uri: &str, kind: ShowKind) -> Vec<Card> {
    let key = match kind {
        ShowKind::Audiobook => "seoRecommendedAudiobook",
        ShowKind::Podcast => "seoRecommendedPodcast",
    };
    let mut seen = HashSet::from([uri.to_owned()]);
    data[key]["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let inner = if item.get("__typename").is_none() && item["data"].is_object() {
                &item["data"]
            } else {
                item
            };
            entity::card(inner)
        })
        .filter(|card| card.kind == wanted(kind) && seen.insert(card.uri.clone()))
        .take(LIMIT)
        .collect()
}

pub async fn similar(api: &Api, uri: &str, kind: ShowKind) -> Result<Vec<Card>> {
    let (operation, variables) = request(uri, kind);
    let data = pathfinder::query(api, operation, variables).await?;
    Ok(parse(&data, uri, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOK: &str = "spotify:show:0000000000000000000bk1";
    const SHOW: &str = "spotify:show:0000000000000000000ab1";

    fn cover(stem: &str) -> Value {
        json!({"sources": [
            {"url": format!("https://i.scdn.co/image/{stem}-300"), "width": 300, "height": 300},
            {"url": format!("https://i.scdn.co/image/{stem}-640"), "width": 640, "height": 640}
        ]})
    }

    #[test]
    fn builds_the_requests() {
        assert_eq!(
            request(BOOK, ShowKind::Audiobook),
            (SIMILAR_AUDIOBOOKS, json!({"uri": BOOK}))
        );
        assert_eq!(
            request(SHOW, ShowKind::Podcast),
            (SIMILAR_PODCASTS, json!({"uri": SHOW}))
        );
        assert_eq!(title(ShowKind::Audiobook), "More audiobooks like this");
    }

    #[test]
    fn reads_similar_books() {
        let data = json!({"seoRecommendedAudiobook": {"totalCount": 4, "items": [
            {"__typename": "AudiobookResponseWrapper", "data": {
                "__typename": "Audiobook",
                "uri": "spotify:show:0000000000000000000bk2",
                "name": "Second Book",
                "coverArt": cover("bk2"),
                "authorsV2": [{"name": "Some Author", "uri": "spotify:author:0000000000000000000au1"}, {"name": "Co Author"}]
            }},
            {"data": {
                "__typename": "Audiobook",
                "uri": "spotify:show:0000000000000000000bk3",
                "name": "Third Book",
                "coverArt": cover("bk3"),
                "authorsV2": []
            }},
            {"data": {"__typename": "Audiobook", "uri": BOOK, "name": "This Book"}},
            {"data": {"__typename": "NotFound"}},
            {"data": {"__typename": "Podcast", "uri": SHOW, "name": "A Show"}}
        ]}});
        let cards = parse(&data, BOOK, ShowKind::Audiobook);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].uri, "spotify:show:0000000000000000000bk2");
        assert_eq!(cards[0].kind, Kind::Audiobook);
        assert_eq!(cards[0].title, "Second Book");
        assert_eq!(cards[0].subtitle, "Some Author, Co Author");
        assert_eq!(
            cards[0].image.as_deref(),
            Some("https://i.scdn.co/image/bk2-300")
        );
        assert_eq!(cards[1].title, "Third Book");
    }

    #[test]
    fn reads_similar_podcasts() {
        let data = json!({"seoRecommendedPodcast": {"totalCount": 2, "items": [
            {"data": {
                "__typename": "Podcast",
                "uri": "spotify:show:0000000000000000000ab2",
                "name": "Other Show",
                "publisher": {"name": "Some Studio"},
                "mediaType": "AUDIO",
                "coverArt": cover("ab2")
            }},
            {"data": {
                "__typename": "Podcast",
                "uri": "spotify:show:0000000000000000000ab2",
                "name": "Other Show"
            }},
            {"data": {"__typename": "GenericError"}}
        ]}});
        let cards = parse(&data, SHOW, ShowKind::Podcast);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, Kind::Show);
        assert_eq!(cards[0].subtitle, "Some Studio");
        assert!(parse(&json!({}), SHOW, ShowKind::Podcast).is_empty());
    }
}
