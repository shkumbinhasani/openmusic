use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::{
    join::{File, choose_file},
    quality::{codec_of, quality_label},
    video::{Opened, VideoPrefs},
};
use crate::{
    engine::Engine,
    player::{Media, Source},
};

pub const MANIFEST_FORMATS: [&str; 3] = [
    "file_urls_external",
    "file_ids_mp4_cbcs",
    "manifest_ids_video",
];

const BOOK: &str = "Audiobooks can’t play in OpenMusic. Open this book in the Spotify app.";
const EPISODE: &str =
    "This episode can’t play in OpenMusic. Spotify only offers it in a format macOS can’t decode.";
const TRACK: &str = "Spotify offered no audio OpenMusic can play for this track.";

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Pick {
    FairPlay(File),
    External(String),
    Director(String),
}

impl Pick {
    pub(super) fn format(&self) -> Value {
        match self {
            Self::FairPlay(file) => file.format.clone(),
            Self::External(_) => json!("file_urls_external"),
            Self::Director(_) => Value::Null,
        }
    }

    pub(super) fn bitrate(&self) -> u64 {
        match self {
            Self::FairPlay(file) => file.bitrate,
            Self::External(_) | Self::Director(_) => 0,
        }
    }

    pub(super) fn name(&self) -> &'static str {
        match self {
            Self::FairPlay(_) => "fairplay",
            Self::External(_) => "external",
            Self::Director(_) => "director",
        }
    }

    pub(super) fn codec(&self) -> Option<String> {
        match self {
            Self::FairPlay(_) | Self::Director(_) => Some("AAC".to_owned()),
            Self::External(url) => codec_of(url),
        }
    }

    pub(super) fn reported_quality(&self, bitrate: u64) -> Option<String> {
        match self {
            Self::FairPlay(file) => file.quality.clone(),
            Self::External(_) => None,
            Self::Director(_) => (bitrate > 0).then(|| quality_label(bitrate).to_owned()),
        }
    }
}

pub(super) fn external(manifest: &Value) -> Option<String> {
    manifest["file_urls_external"]
        .as_array()?
        .iter()
        .filter_map(|file| file["file_url"].as_str())
        .find(|url| url.starts_with("https://"))
        .map(str::to_owned)
}

pub(super) fn video_manifest(manifest: &Value) -> Option<String> {
    manifest["manifest_ids_video"]
        .as_array()?
        .iter()
        .filter_map(|file| file["file_id"].as_str())
        .find(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
        .map(str::to_owned)
}

pub(super) fn pick(track: &Value, ceiling: Option<u64>, video: VideoPrefs) -> Option<Pick> {
    if video.wants(uri(track))
        && let Some(id) = video_manifest(&track["manifest"])
    {
        return Some(Pick::Director(id));
    }
    audio_pick(track, ceiling)
}

pub(super) fn audio_pick(track: &Value, ceiling: Option<u64>) -> Option<Pick> {
    let manifest = &track["manifest"];
    if let Some(url) = external(manifest) {
        return Some(Pick::External(url));
    }
    if let Some(file) = choose_file(track, ceiling) {
        return Some(Pick::FairPlay(file));
    }
    video_manifest(manifest).map(Pick::Director)
}

fn uri(track: &Value) -> &str {
    track["metadata"]["uri"].as_str().unwrap_or_default()
}

pub(super) fn is_book(track: &Value) -> bool {
    let book = |value: &Value| {
        value.as_str().is_some_and(|kind| {
            let kind = kind.to_ascii_uppercase();
            kind.contains("AUDIOBOOK") || kind.contains("CHAPTER")
        })
    };
    let metadata = &track["metadata"];
    uri(track).starts_with("spotify:chapter:")
        || book(&metadata["show_content_type"])
        || book(&metadata["episode_content_type"])
        || book(&track["content_type"])
}

pub(super) fn is_video_track(track: &Value) -> bool {
    let video = |value: &Value| value.as_str() == Some("VIDEO");
    video(&track["track_type"]) || video(&track["metadata"]["track_type"])
}

pub(super) fn is_spoken(uri: &str) -> bool {
    uri.starts_with("spotify:episode:") || uri.starts_with("spotify:chapter:")
}

pub(super) fn unplayable(track: &Value) -> &'static str {
    if is_book(track) {
        BOOK
    } else if is_spoken(uri(track)) {
        EPISODE
    } else {
        TRACK
    }
}

pub(super) fn media_path(uri: &str) -> String {
    let formats = MANIFEST_FORMATS
        .iter()
        .map(|format| format!("manifestFileFormat={format}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("/track-playback/v1/media/{uri}?{formats}")
}

pub(super) fn pick_from_media(
    response: &Value,
    uri: &str,
    ceiling: Option<u64>,
    video: VideoPrefs,
) -> Option<Pick> {
    let item = response["media"].get(uri)?.get("item")?;
    if is_book(item) {
        return None;
    }
    pick(item, ceiling, video)
}

impl Engine {
    pub(super) async fn resolve(&self, track: &Value) -> Result<Pick> {
        if is_book(track) {
            bail!(BOOK);
        }
        let ceiling = self.bitrate_ceiling();
        let video = self.audio.video.prefs;
        let uri = uri(track);
        if video.wants(uri)
            && is_video_track(track)
            && video_manifest(&track["manifest"]).is_none()
            && let Ok(response) = self.api.get_json::<Value>(&media_path(uri)).await
            && let Some(pick @ Pick::Director(_)) = pick_from_media(&response, uri, ceiling, video)
        {
            return Ok(pick);
        }
        if let Some(pick) = pick(track, ceiling, video) {
            return Ok(pick);
        }
        if is_spoken(uri)
            && let Ok(response) = self.api.get_json::<Value>(&media_path(uri)).await
            && let Some(pick) = pick_from_media(&response, uri, ceiling, video)
        {
            return Ok(pick);
        }
        bail!(unplayable(track))
    }

    pub(super) async fn open(&mut self, pick: &Pick, uri: &str) -> Result<Opened> {
        match pick {
            Pick::FairPlay(file) => {
                let media = self.media(&file.id, &file.format_number).await?;
                Ok(Opened::audio(media, file.bitrate))
            }
            Pick::External(url) => Ok(Opened::audio(Media::new(Source::Url(url.clone())), 0)),
            Pick::Director(id) => self.open_director(id, uri).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUDIO: VideoPrefs = VideoPrefs {
        music: false,
        podcasts: false,
    };

    fn episode(manifest: Value) -> Value {
        json!({
            "metadata": {"uri": "spotify:episode:0000000000000000000001",
                         "group_uri": "spotify:show:0000000000000000000002",
                         "episode_content_type": "PODCAST", "show_content_type": "PODCAST"},
            "manifest": manifest
        })
    }

    #[test]
    fn prefers_external_audio_for_episodes() {
        let track = episode(json!({
            "file_ids_mp4_dual": [{"file_id": "b1de", "format": "12"}],
            "file_ids_mp4": [{"file_id": "d2d5", "format": "10"}],
            "file_urls_external": [
                {"file_url": "http://insecure.example/a.mp3", "file_id": null},
                {"file_url": "https://dts.podtrac.com/redirect.mp3/example.com/a.mp3", "file_id": null}
            ],
            "manifest_ids_video": [{"file_id": "7cad856b55c54b159b87fbf205abae87"}]
        }));
        let pick = pick(&track, None, AUDIO).unwrap();
        assert_eq!(
            pick,
            Pick::External("https://dts.podtrac.com/redirect.mp3/example.com/a.mp3".into())
        );
        assert_eq!(pick.format(), json!("file_urls_external"));
        assert_eq!(pick.bitrate(), 0);
        assert_eq!(pick.name(), "external");
        assert_eq!(pick.codec().as_deref(), Some("MP3"));
        assert_eq!(pick.reported_quality(0), None);
        assert_eq!(
            super::pick(&track, None, VideoPrefs::default()),
            Some(Pick::Director("7cad856b55c54b159b87fbf205abae87".into()))
        );
        assert_eq!(
            audio_pick(&track, None),
            Some(Pick::External(
                "https://dts.podtrac.com/redirect.mp3/example.com/a.mp3".into()
            ))
        );
    }

    #[test]
    fn falls_back_to_the_video_manifest() {
        let track = episode(json!({
            "file_ids_mp4_dual": [{"file_id": "b1de", "format": "12"}],
            "manifest_ids_video": [{"file_id": "7cad856b55c54b159b87fbf205abae87", "format": null}]
        }));
        let pick = pick(&track, None, AUDIO).unwrap();
        assert_eq!(
            pick,
            Pick::Director("7cad856b55c54b159b87fbf205abae87".into())
        );
        assert_eq!(pick.format(), Value::Null);
        assert_eq!(pick.codec().as_deref(), Some("AAC"));
        assert_eq!(pick.reported_quality(0), None);
        assert_eq!(pick.reported_quality(96_000).as_deref(), Some("NORMAL"));
        assert_eq!(
            video_manifest(&json!({"manifest_ids_video": [{"file_id": "../x"}]})),
            None
        );
    }

    #[test]
    fn keeps_fairplay_for_music() {
        let track = json!({
            "metadata": {"uri": "spotify:track:25jgQBxuUkGDdCG1WGKKN9"},
            "manifest": {
                "file_ids_mp4": [{"file_id": "aa", "format": "10", "bitrate": 128000}],
                "file_ids_mp4_cbcs": [{"file_id": "cc", "format": "15", "bitrate": 256000}],
                "manifest_ids_video": [{"file_id": "7cad856b55c54b159b87fbf205abae87"}]
            }
        });
        let Some(Pick::FairPlay(file)) = pick(&track, None, AUDIO) else {
            panic!("expected FairPlay");
        };
        assert_eq!(file.id, "cc");
        let fairplay = Pick::FairPlay(file);
        assert_eq!(fairplay.name(), "fairplay");
        assert_eq!(fairplay.reported_quality(0).as_deref(), Some("VERY_HIGH"));
        let Some(Pick::FairPlay(only)) = pick(&track, Some(128_000), AUDIO) else {
            panic!("expected FairPlay");
        };
        assert_eq!(only.id, "cc");
        let podcasts_only = VideoPrefs {
            music: false,
            podcasts: true,
        };
        assert!(matches!(
            pick(&track, None, podcasts_only),
            Some(Pick::FairPlay(_))
        ));
    }

    #[test]
    fn plays_music_videos_with_video_when_asked() {
        let video = json!({
            "track_type": "VIDEO",
            "metadata": {"uri": "spotify:track:0000000000000000000004"},
            "manifest": {
                "file_ids_mp4_cbcs": [{"file_id": "cc", "format": "15", "bitrate": 256000}],
                "manifest_ids_video": [{"file_id": "0000eeee1111ffff2222aaaa3333bbbb"}]
            }
        });
        assert!(is_video_track(&video));
        assert_eq!(
            pick(&video, None, VideoPrefs::default()),
            Some(Pick::Director("0000eeee1111ffff2222aaaa3333bbbb".into()))
        );
        assert!(matches!(pick(&video, None, AUDIO), Some(Pick::FairPlay(_))));
        assert!(matches!(audio_pick(&video, None), Some(Pick::FairPlay(_))));
        assert!(!is_video_track(&json!({"track_type": "AUDIO"})));
        assert!(is_video_track(
            &json!({"metadata": {"track_type": "VIDEO"}})
        ));
    }

    #[test]
    fn explains_what_cannot_play() {
        let hosted = episode(json!({
            "file_ids_mp4_dual": [{"file_id": "b1de", "format": "12"}],
            "file_ids_mp4": [{"file_id": "d2d5", "format": "10"}]
        }));
        assert_eq!(pick(&hosted, None, VideoPrefs::default()), None);
        assert_eq!(unplayable(&hosted), EPISODE);

        let mut chapter = hosted.clone();
        chapter["metadata"]["show_content_type"] = json!("AUDIOBOOK");
        assert!(is_book(&chapter));
        assert_eq!(unplayable(&chapter), BOOK);
        assert!(is_book(
            &json!({"metadata": {"uri": "spotify:chapter:0000000000000000000003"}})
        ));

        let track = json!({"metadata": {"uri": "spotify:track:x"}, "manifest": {}});
        assert!(!is_book(&track));
        assert_eq!(unplayable(&track), TRACK);
    }

    #[test]
    fn asks_the_media_service_for_every_format() {
        let uri = "spotify:episode:0000000000000000000001";
        assert_eq!(
            media_path(uri),
            "/track-playback/v1/media/spotify:episode:0000000000000000000001?manifestFileFormat=file_urls_external&manifestFileFormat=file_ids_mp4_cbcs&manifestFileFormat=manifest_ids_video"
        );
        let response = json!({"media": {uri: {"item": episode(json!({
            "file_urls_external": [{"file_url": "https://pdrl.fm/example/a.mp3"}]
        }))}}});
        assert_eq!(
            pick_from_media(&response, uri, None, VideoPrefs::default()),
            Some(Pick::External("https://pdrl.fm/example/a.mp3".into()))
        );
        assert_eq!(
            pick_from_media(&json!({"media": {}}), uri, None, AUDIO),
            None
        );
        let mut book = response.clone();
        book["media"][uri]["item"]["metadata"]["show_content_type"] = json!("AUDIOBOOK");
        assert_eq!(pick_from_media(&book, uri, None, AUDIO), None);
    }
}
