use anyhow::Result;
use prost::Message;
use serde_json::{Value, json};
use spotify_proto::{
    bumblebee::video_associations::v1::VideoAssociations, extendedmetadata::ExtensionKind,
};

use crate::{
    api::Api,
    catalog::{self, Track},
    entity,
    pathfinder::{self, Operation},
};

pub const ARTIST_VIDEOS: Operation = Operation::query(
    "queryArtistRelatedVideos",
    "7c391a047143eff8a698121b8b55e92cdc2c862f92870f10899bd3ed7d1a9f01",
);
pub const VIDEO_TRACK_ALBUM: Operation = Operation::query(
    "getVideoTrackAssociatedAlbum",
    "e9ecdc49f7777062fc841415262d47c3927e09ab6e6845b420a373caec602812",
);

pub const PAGE_SIZE: u32 = 20;
pub const STILL_WIDTH: u32 = 640;

#[derive(Clone, Debug, PartialEq)]
pub struct MusicVideo {
    pub track: Track,
    pub still: Option<String>,
    pub audio: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VideoPage {
    pub videos: Vec<MusicVideo>,
    pub total: u32,
    pub next: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Song {
    pub track: String,
    pub album: Option<String>,
}

fn is_track(uri: &str) -> bool {
    matches!(
        uri.split(':').collect::<Vec<_>>().as_slice(),
        ["spotify", "track", id] if !id.is_empty()
    )
}

pub fn artist_variables(uri: &str, offset: u32, limit: u32) -> Value {
    json!({
        "uri": uri,
        "showMapped": true,
        "showUnmapped": false,
        "limit": limit,
        "offset": offset,
    })
}

fn audio_of(data: &Value) -> Option<String> {
    let audio = &data["associationsV3"]["audioAssociations"]["items"][0]["trackAudio"];
    audio["_uri"]
        .as_str()
        .or_else(|| audio["data"]["uri"].as_str())
        .filter(|uri| is_track(uri))
        .map(str::to_owned)
}

fn video(item: &Value) -> Option<MusicVideo> {
    let data = &item["data"];
    let track = entity::track(data).filter(|track| is_track(&track.uri))?;
    Some(MusicVideo {
        still: entity::pick_image(&data["albumOfTrack"]["coverArt"]["sources"], STILL_WIDTH),
        audio: audio_of(data),
        track,
    })
}

pub fn parse_artist(data: &Value) -> VideoPage {
    let page = &data["artistUnion"]["relatedMusicVideos"];
    let videos: Vec<MusicVideo> = page["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(video)
        .collect();
    VideoPage {
        total: page["totalCount"]
            .as_u64()
            .map_or(videos.len() as u32, |total| total as u32),
        next: page["pagingInfo"]["nextOffset"]
            .as_u64()
            .map(|next| next as u32),
        videos,
    }
}

pub async fn artist_videos(api: &Api, uri: &str, offset: u32) -> Result<VideoPage> {
    let data =
        pathfinder::query(api, ARTIST_VIDEOS, artist_variables(uri, offset, PAGE_SIZE)).await?;
    Ok(parse_artist(&data))
}

pub fn decode_associations(bytes: &[u8]) -> Option<String> {
    VideoAssociations::decode(bytes)
        .ok()?
        .video_association
        .into_iter()
        .map(|association| association.track_uri)
        .find(|uri| is_track(uri))
}

pub async fn video_for(api: &Api, track_uri: &str) -> Result<Option<String>> {
    if !is_track(track_uri) {
        return Ok(None);
    }
    let found = catalog::extended(
        api,
        &[(track_uri.to_owned(), ExtensionKind::VideoAssociations)],
    )
    .await?;
    Ok(found
        .iter()
        .find(|(uri, _)| uri == track_uri)
        .and_then(|(_, bytes)| decode_associations(bytes))
        .filter(|video| video != track_uri))
}

pub fn parse_song(data: &Value) -> Option<Song> {
    let audio =
        &data["trackUnion"]["associationsV3"]["audioAssociations"]["items"][0]["trackAudio"];
    let track = audio["_uri"]
        .as_str()
        .or_else(|| audio["data"]["uri"].as_str())
        .filter(|uri| is_track(uri))?
        .to_owned();
    Some(Song {
        track,
        album: audio["data"]["albumOfTrack"]["uri"]
            .as_str()
            .filter(|uri| uri.starts_with("spotify:album:"))
            .map(str::to_owned),
    })
}

pub async fn song_for(api: &Api, video_uri: &str) -> Result<Option<Song>> {
    let data = pathfinder::query(api, VIDEO_TRACK_ALBUM, json!({ "trackUri": video_uri })).await?;
    Ok(parse_song(&data))
}

#[cfg(test)]
mod tests {
    use spotify_proto::bumblebee::video_associations::v1::{Image, ImageGroup, VideoAssociation};

    use super::*;

    const ARTIST: &str = "spotify:artist:000000000000000000art1";
    const VIDEO: &str = "spotify:track:0000000000000000000vid1";
    const SONG: &str = "spotify:track:0000000000000000000aud1";
    const ALBUM: &str = "spotify:album:0000000000000000000alb1";

    fn item(uri: &str, name: &str, explicit: bool) -> Value {
        json!({
            "_uri": uri,
            "data": {
                "__typename": "Track",
                "name": name,
                "uri": uri,
                "albumOfTrack": {
                    "uri": ALBUM,
                    "coverArt": {"sources": [
                        {"height": 720, "width": 1280, "url": "https://i.scdn.co/image/still-1280"},
                        {"height": 360, "width": 640, "url": "https://i.scdn.co/image/still-640"},
                        {"height": 180, "width": 320, "url": "https://i.scdn.co/image/still-320"}
                    ]}
                },
                "artists": {"items": [
                    {"profile": {"name": "Some Singer"}, "uri": ARTIST},
                    {"profile": {"name": "A Guest"}, "uri": "spotify:artist:000000000000000000art2"}
                ]},
                "associationsV3": {"audioAssociations": {"items": [
                    {"trackAudio": {"_uri": SONG, "data": {"__typename": "Track"}}}
                ]}},
                "contentRating": {"label": if explicit { "EXPLICIT" } else { "NONE" }}
            }
        })
    }

    #[test]
    fn reads_an_artists_music_videos() {
        let data = json!({"artistUnion": {
            "__typename": "Artist",
            "relatedMusicVideos": {
                "__typename": "MusicVideosPage",
                "items": [
                    item(VIDEO, "First video", false),
                    item("spotify:track:0000000000000000000vid2", "Second video", true),
                    {"_uri": "spotify:track:0000000000000000000vid3", "data": {"__typename": "RestrictedContent"}},
                    item("spotify:episode:0000000000000000000ep01", "Not a video", false)
                ],
                "pagingInfo": {"nextOffset": 5},
                "totalCount": 30
            }
        }});
        let page = parse_artist(&data);
        assert_eq!(page.total, 30);
        assert_eq!(page.next, Some(5));
        assert_eq!(page.videos.len(), 2);
        let first = &page.videos[0];
        assert_eq!(first.track.uri, VIDEO);
        assert_eq!(first.track.title, "First video");
        assert_eq!(first.track.artist, "Some Singer, A Guest");
        assert_eq!(first.track.artists.len(), 2);
        assert_eq!(first.track.album_uri.as_deref(), Some(ALBUM));
        assert_eq!(
            first.still.as_deref(),
            Some("https://i.scdn.co/image/still-640")
        );
        assert_eq!(first.audio.as_deref(), Some(SONG));
        assert!(!first.track.explicit);
        assert!(page.videos[1].track.explicit);

        let empty = parse_artist(&json!({"artistUnion": {"relatedMusicVideos": {"items": []}}}));
        assert_eq!(empty, VideoPage::default());
        assert_eq!(parse_artist(&json!({})), VideoPage::default());
    }

    #[test]
    fn asks_for_mapped_videos_a_page_at_a_time() {
        assert_eq!(
            artist_variables(ARTIST, 20, PAGE_SIZE),
            json!({"uri": ARTIST, "showMapped": true, "showUnmapped": false, "limit": 20, "offset": 20})
        );
        assert_eq!(ARTIST_VIDEOS.name, "queryArtistRelatedVideos");
        assert_eq!(VIDEO_TRACK_ALBUM.name, "getVideoTrackAssociatedAlbum");
    }

    #[test]
    fn finds_the_video_of_a_song() {
        let bytes = VideoAssociations {
            video_association: vec![
                VideoAssociation {
                    track_uri: String::new(),
                    images: None,
                },
                VideoAssociation {
                    track_uri: VIDEO.into(),
                    images: Some(ImageGroup {
                        image: vec![Image {
                            file_id: Some(vec![0xab, 0x67]),
                            size: Some(0),
                            width: Some(2560),
                            height: Some(1440),
                        }],
                    }),
                },
            ],
        }
        .encode_to_vec();
        assert_eq!(decode_associations(&bytes).as_deref(), Some(VIDEO));
        assert_eq!(
            decode_associations(&VideoAssociations::default().encode_to_vec()),
            None
        );
        assert_eq!(decode_associations(&[0xff, 0xff, 0xff]), None);
    }

    #[test]
    fn finds_the_song_of_a_video() {
        let data = json!({"trackUnion": {
            "__typename": "Track",
            "mediaType": "VIDEO",
            "uri": VIDEO,
            "associationsV3": {"audioAssociations": {"items": [
                {"trackAudio": {"data": {"__typename": "Track", "uri": SONG, "albumOfTrack": {"uri": ALBUM}}}}
            ]}}
        }});
        assert_eq!(
            parse_song(&data),
            Some(Song {
                track: SONG.into(),
                album: Some(ALBUM.into()),
            })
        );
        let bare = json!({"trackUnion": {"associationsV3": {"audioAssociations": {"items": [
            {"trackAudio": {"_uri": SONG}}
        ]}}}});
        assert_eq!(
            parse_song(&bare),
            Some(Song {
                track: SONG.into(),
                album: None,
            })
        );
        assert_eq!(
            parse_song(
                &json!({"trackUnion": {"associationsV3": {"audioAssociations": {"items": []}}}})
            ),
            None
        );
    }
}
