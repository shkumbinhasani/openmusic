use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::page::iso_ms;
use crate::{
    api::Api,
    catalog::Track,
    entity,
    pathfinder::{self, Operation},
};

pub const FETCH_LIBRARY_TRACKS: Operation = Operation::query(
    "fetchLibraryTracks",
    "087278b20b743578a6262c2b0b4bcd20d879c503cc359a2285baf083ef944240",
);

pub const LIKED_PAGE_SIZE: u32 = 50;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LikedPage {
    pub tracks: Vec<Track>,
    pub unplayable: Vec<String>,
    pub total: u32,
    pub offset: u32,
    pub rows: u32,
}

pub fn liked_variables(offset: u32, limit: u32) -> Value {
    json!({ "offset": offset, "limit": limit })
}

pub async fn liked(api: &Api, offset: u32, limit: u32) -> Result<LikedPage> {
    let data = pathfinder::query(api, FETCH_LIBRARY_TRACKS, liked_variables(offset, limit)).await?;
    parse_liked(&data, offset)
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

pub fn parse_liked(data: &Value, offset: u32) -> Result<LikedPage> {
    let page = &data["me"]["library"]["tracks"];
    if !page.is_object()
        || page["__typename"]
            .as_str()
            .is_some_and(|typename| typename != "UserLibraryTrackPage")
    {
        bail!("Spotify sent an unexpected Liked Songs page.");
    }
    let rows = page["items"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut tracks = Vec::with_capacity(rows.len());
    let mut unplayable = Vec::new();
    for row in rows {
        let wrapper = &row["track"];
        let data = &wrapper["data"];
        if data["__typename"] != "Track" {
            continue;
        }
        let Some(uri) = text(&wrapper["_uri"]).or_else(|| text(&data["uri"])) else {
            continue;
        };
        let wrapped = json!({ "__typename": "TrackResponseWrapper", "_uri": uri, "data": data });
        let Some(mut track) = entity::track(&wrapped) else {
            continue;
        };
        track.added_at = text(&row["addedAt"]["isoString"]).and_then(iso_ms);
        if !entity::playable(data) {
            unplayable.push(track.uri.clone());
        }
        tracks.push(track);
    }
    let count = rows.len() as u32;
    Ok(LikedPage {
        total: page["totalCount"]
            .as_u64()
            .map_or(offset + count, |total| total as u32),
        offset: page["pagingInfo"]["offset"]
            .as_u64()
            .map_or(offset, |found| found as u32),
        rows: count,
        tracks,
        unplayable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(uri: &str, name: &str, added: &str, playable: bool) -> Value {
        json!({
            "__typename": "UserLibraryTrackResponse",
            "addedAt": {"isoString": added},
            "track": {"_uri": uri, "data": {
                "__typename": "Track",
                "name": name,
                "discNumber": 1,
                "trackNumber": 1,
                "duration": {"totalMilliseconds": 172858},
                "playability": {"playable": playable},
                "contentRating": {"label": "NONE"},
                "mediaType": "AUDIO",
                "artists": {"items": [
                    {"profile": {"name": "Artist A"}, "uri": "spotify:artist:0000000000000000000001"},
                    {"profile": {"name": "Artist B"}, "uri": "spotify:artist:0000000000000000000002"}
                ]},
                "albumOfTrack": {
                    "name": "Album",
                    "uri": "spotify:album:0000000000000000000003",
                    "coverArt": {"sources": [
                        {"height": 64, "url": "https://i.scdn.co/image/small", "width": 64},
                        {"height": 300, "url": "https://i.scdn.co/image/medium", "width": 300}
                    ]},
                    "artists": {"items": []}
                }
            }}
        })
    }

    fn sample() -> Value {
        json!({"me": {"library": {"tracks": {
            "__typename": "UserLibraryTrackPage",
            "pagingInfo": {"limit": 50, "offset": 50},
            "totalCount": 64,
            "items": [
                row("spotify:track:0000000000000000000010", "First", "2026-11-21T15:30:43Z", true),
                {"__typename": "UserLibraryTrackResponse", "addedAt": {"isoString": "2026-11-20T10:00:00Z"},
                 "track": {"_uri": "spotify:track:0000000000000000000011", "data": {"__typename": "NotFound"}}},
                row("spotify:track:0000000000000000000012", "Greyed", "2021-07-01T14:33:26Z", false)
            ]
        }}}})
    }

    #[test]
    fn asks_for_a_page_of_liked_songs() {
        assert_eq!(FETCH_LIBRARY_TRACKS.name, "fetchLibraryTracks");
        assert_eq!(
            liked_variables(100, 50),
            json!({"offset": 100, "limit": 50})
        );
    }

    #[test]
    fn reads_liked_songs_with_their_dates() {
        let page = parse_liked(&sample(), 50).unwrap();
        assert_eq!(page.total, 64);
        assert_eq!(page.offset, 50);
        assert_eq!(page.rows, 3);
        assert_eq!(page.tracks.len(), 2);
        let first = &page.tracks[0];
        assert_eq!(first.uri, "spotify:track:0000000000000000000010");
        assert_eq!(first.title, "First");
        assert_eq!(first.artist, "Artist A, Artist B");
        assert_eq!(first.artists.len(), 2);
        assert_eq!(first.album, "Album");
        assert_eq!(
            first.album_uri.as_deref(),
            Some("spotify:album:0000000000000000000003")
        );
        assert_eq!(
            first.cover.as_deref(),
            Some("https://i.scdn.co/image/medium")
        );
        assert_eq!(first.duration_ms, 172_858);
        assert_eq!(first.added_at, Some(1_795_275_043_000));
        assert_eq!(
            page.unplayable,
            vec!["spotify:track:0000000000000000000012"]
        );
    }

    #[test]
    fn refuses_other_replies() {
        assert!(parse_liked(&json!({"me": {"library": {}}}), 0).is_err());
        assert!(
            parse_liked(
                &json!({"me": {"library": {"tracks": {"__typename": "GenericError"}}}}),
                0
            )
            .is_err()
        );
        let empty = parse_liked(
            &json!({"me": {"library": {"tracks": {"__typename": "UserLibraryTrackPage", "items": []}}}}),
            0,
        )
        .unwrap();
        assert_eq!((empty.total, empty.rows), (0, 0));
    }
}
