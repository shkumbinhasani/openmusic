use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::api::Api;

pub const AUDIO_PLAYLIST: &str = "audio.m3u8";
pub const MASTER_PLAYLIST: &str = "master.m3u8";

const PROTECTED: &str =
    "This stream is protected in a way macOS can’t play, so OpenMusic can’t open it.";

pub type Playlists = Vec<(String, Vec<u8>)>;

#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub id: i64,
    pub file_type: String,
    pub codec: String,
    pub bandwidth: u64,
    pub bitrate: u64,
    pub width: u32,
    pub height: u32,
    pub encryption: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Protection {
    pub index: i64,
    pub endpoint: String,
    pub asset_id: String,
}

impl Protection {
    pub fn key_line(&self) -> String {
        format!(
            "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://{}\",KEYFORMATVERSIONS=\"1\",KEYFORMAT=\"com.apple.streamingkeydelivery\"",
            self.asset_id
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Director {
    pub encoding_id: String,
    pub segment_seconds: u32,
    pub start_ms: u64,
    pub end_ms: u64,
    pub bases: Vec<String>,
    pub init: String,
    pub segment: String,
    pub audio: Option<Profile>,
    pub video: Vec<Profile>,
    pub encrypted: bool,
    pub protection: Option<Protection>,
    pub loudness: Option<(f32, f32)>,
}

impl Director {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms.saturating_sub(self.start_ms)
    }

    pub fn base(&self) -> Option<&str> {
        self.bases.first().map(String::as_str)
    }

    pub fn has_video(&self) -> bool {
        self.audio.is_some() && !self.video.is_empty()
    }

    pub fn aspect(&self) -> Option<f32> {
        self.video
            .iter()
            .filter(|profile| profile.width > 0 && profile.height > 0)
            .max_by_key(|profile| profile.height)
            .map(|profile| profile.width as f32 / profile.height as f32)
    }
}

pub fn manifest_path(id: &str) -> String {
    format!("/manifests/v9/json/sources/{id}/options/supports_drm")
}

pub async fn manifest(api: &Api, id: &str) -> Result<Director> {
    let json: Value = api
        .get_json(&manifest_path(id))
        .await
        .context("Spotify did not send the episode's stream description.")?;
    parse(&json)
}

fn profile(value: &Value) -> Option<Profile> {
    let codec = value["audio_codec"]
        .as_str()
        .or_else(|| value["video_codec"].as_str())?
        .to_owned();
    Some(Profile {
        id: value["id"].as_i64()?,
        file_type: value["file_type"].as_str()?.to_owned(),
        codec,
        bandwidth: value["max_bitrate"].as_u64().unwrap_or_default(),
        bitrate: value["audio_bitrate"]
            .as_u64()
            .or_else(|| value["video_bitrate"].as_u64())
            .unwrap_or_default(),
        width: value["video_width"].as_u64().unwrap_or_default() as u32,
        height: value["video_height"].as_u64().unwrap_or_default() as u32,
        encryption: value["encryption_indices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
            .chain(
                value["encryption_index"]
                    .as_i64()
                    .filter(|_| value["encryption_indices"].is_null()),
            )
            .collect(),
    })
}

fn protection(infos: &[Value], encoding_id: &str) -> Option<Protection> {
    infos.iter().enumerate().find_map(|(index, info)| {
        if info["key_system"].as_str() != Some("fairplay") {
            return None;
        }
        let endpoint = info["license_server_endpoint"].as_str()?;
        if !endpoint.starts_with('/') || encoding_id.is_empty() {
            return None;
        }
        Some(Protection {
            index: index as i64,
            endpoint: endpoint.to_owned(),
            asset_id: encoding_id.to_owned(),
        })
    })
}

fn usable(profile: &Profile, encrypted: bool, protection: Option<&Protection>) -> bool {
    match protection {
        Some(protection) => {
            profile.file_type == "ts" && profile.encryption.contains(&protection.index)
        }
        None => !encrypted && profile.file_type == "mp4" && profile.encryption.is_empty(),
    }
}

pub fn parse(json: &Value) -> Result<Director> {
    let content = json["contents"]
        .as_array()
        .and_then(|contents| contents.first())
        .context("The stream description had no content.")?;
    let profiles: Vec<Profile> = content["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(profile)
        .collect();
    let encoding_id = content["encoding_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let infos = content["encryption_infos"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let encrypted = !infos.is_empty();
    let protection = protection(&infos, &encoding_id);
    let fits = |profile: &&Profile| usable(profile, encrypted, protection.as_ref());
    let audio = profiles
        .iter()
        .filter(fits)
        .filter(|profile| profile.codec.starts_with("mp4a.40"))
        .max_by_key(|profile| profile.bitrate)
        .cloned();
    let mut video: Vec<Profile> = profiles
        .iter()
        .filter(fits)
        .filter(|profile| profile.codec.starts_with("avc1"))
        .cloned()
        .collect();
    video.sort_by_key(|profile| (profile.height, profile.bandwidth));
    let bases = json["base_urls"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|url| url.starts_with("https://"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if bases.is_empty() {
        bail!("The stream description had no servers.");
    }
    let start_ms = content["start_time_millis"]
        .as_u64()
        .or_else(|| json["start_time_millis"].as_u64())
        .unwrap_or_default();
    let end_ms = content["end_time_millis"]
        .as_u64()
        .or_else(|| json["end_time_millis"].as_u64())
        .context("The stream description had no length.")?;
    if end_ms <= start_ms {
        bail!("The stream description had no length.");
    }
    let loudness = &content["original_loudness"];
    Ok(Director {
        encoding_id,
        segment_seconds: content["segment_length"]
            .as_u64()
            .filter(|seconds| *seconds > 0)
            .unwrap_or(4) as u32,
        start_ms,
        end_ms,
        bases,
        init: json["initialization_template"]
            .as_str()
            .context("The stream description had no init template.")?
            .to_owned(),
        segment: json["segment_template"]
            .as_str()
            .context("The stream description had no segment template.")?
            .to_owned(),
        audio,
        video,
        encrypted,
        protection,
        loudness: loudness["loudness"]
            .as_f64()
            .zip(loudness["true_peak"].as_f64())
            .map(|(loudness, peak)| (loudness as f32, peak as f32)),
    })
}

fn fill(template: &str, profile: &Profile, timestamp: Option<u64>) -> String {
    let filled = template
        .replace("{{profile_id}}", &profile.id.to_string())
        .replace("{{file_type}}", &profile.file_type);
    match timestamp {
        Some(timestamp) => filled.replace("{{segment_timestamp}}", &timestamp.to_string()),
        None => filled,
    }
}

pub fn segments(director: &Director) -> Vec<(u64, f64)> {
    let step = u64::from(director.segment_seconds);
    let end = director.end_ms;
    let mut segments = Vec::new();
    let mut second = director.start_ms / 1000;
    while second * 1000 < end {
        let left = (end - second * 1000) as f64 / 1000.;
        segments.push((second, left.min(step as f64)));
        second += step;
    }
    segments
}

pub fn media_playlist(director: &Director, profile: &Profile) -> Option<Vec<u8>> {
    let base = director.base()?;
    let target = u64::from(director.segment_seconds) + 1;
    let mut lines = vec![
        "#EXTM3U".to_owned(),
        "#EXT-X-VERSION:7".to_owned(),
        format!("#EXT-X-TARGETDURATION:{target}"),
        "#EXT-X-MEDIA-SEQUENCE:0".to_owned(),
        "#EXT-X-PLAYLIST-TYPE:VOD".to_owned(),
        "#EXT-X-INDEPENDENT-SEGMENTS".to_owned(),
    ];
    match &director.protection {
        Some(protection) => lines.push(protection.key_line()),
        None => lines.push(format!(
            "#EXT-X-MAP:URI=\"{base}{}\"",
            fill(&director.init, profile, None)
        )),
    }
    for (timestamp, seconds) in segments(director) {
        lines.push(format!("#EXTINF:{seconds:.3},"));
        lines.push(format!(
            "{base}{}",
            fill(&director.segment, profile, Some(timestamp))
        ));
    }
    lines.push("#EXT-X-ENDLIST".to_owned());
    Some(lines.join("\n").into_bytes())
}

fn video_name(profile: &Profile) -> String {
    format!("video-{}.m3u8", profile.id)
}

pub fn master_playlist(director: &Director) -> Option<Vec<u8>> {
    let audio = director.audio.as_ref()?;
    if director.video.is_empty() {
        return None;
    }
    let mut lines = vec![
        "#EXTM3U".to_owned(),
        "#EXT-X-VERSION:7".to_owned(),
        "#EXT-X-INDEPENDENT-SEGMENTS".to_owned(),
        format!(
            "#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"Audio\",DEFAULT=YES,AUTOSELECT=YES,URI=\"{AUDIO_PLAYLIST}\""
        ),
    ];
    for video in &director.video {
        lines.push(format!(
            "#EXT-X-STREAM-INF:BANDWIDTH={},RESOLUTION={}x{},CODECS=\"{},{}\",AUDIO=\"audio\"",
            audio.bandwidth + video.bandwidth,
            video.width,
            video.height,
            audio.codec,
            video.codec
        ));
        lines.push(video_name(video));
    }
    Some(lines.join("\n").into_bytes())
}

pub fn playlists(director: &Director, with_video: bool) -> Result<(Playlists, String)> {
    if director.encrypted && director.protection.is_none() {
        bail!(PROTECTED);
    }
    let audio = director
        .audio
        .as_ref()
        .context("The stream has no audio OpenMusic can play.")?;
    let mut playlists = vec![(
        AUDIO_PLAYLIST.to_owned(),
        media_playlist(director, audio).context("The stream had no servers.")?,
    )];
    if !with_video {
        return Ok((playlists, AUDIO_PLAYLIST.to_owned()));
    }
    let Some(master) = master_playlist(director) else {
        return Ok((playlists, AUDIO_PLAYLIST.to_owned()));
    };
    for video in &director.video {
        if let Some(bytes) = media_playlist(director, video) {
            playlists.push((video_name(video), bytes));
        }
    }
    playlists.push((MASTER_PLAYLIST.to_owned(), master));
    Ok((playlists, MASTER_PLAYLIST.to_owned()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fixture() -> Value {
        json!({
            "contents": [{
                "encoding_id": "0000aaaa1111bbbb2222cccc3333dddd",
                "segment_length": 4,
                "start_time_millis": 0,
                "end_time_millis": 10_500,
                "profiles": [
                    {"id": 13, "audio_bitrate": 96000, "audio_codec": "mp4a.40.2", "file_type": "mp4",
                     "max_bitrate": 99966, "mime_type": "audio/mp4"},
                    {"id": 5, "file_type": "mp4", "mime_type": "video/mp4", "video_codec": "avc1.4d400d",
                     "video_width": 320, "video_height": 180, "video_bitrate": 78402, "max_bitrate": 204096},
                    {"id": 1, "file_type": "mp4", "mime_type": "video/mp4", "video_codec": "avc1.4d4020",
                     "video_width": 1280, "video_height": 720, "video_bitrate": 900000, "max_bitrate": 1500000},
                    {"id": 12, "file_type": "mp4", "video_codec": "av01.0.00M.10", "video_width": 1920,
                     "video_height": 1080, "max_bitrate": 2000000},
                    {"id": 18, "audio_codec": "opus", "file_type": "webm", "audio_bitrate": 128000},
                    {"id": 14, "video_codec": "vp9", "file_type": "webm", "video_height": 720}
                ],
                "encryption_infos": [],
                "original_loudness": {"loudness": -16.165052, "true_peak": 0.0875}
            }],
            "base_urls": ["https://video-cf.spotifycdn.com/segments/", "https://video-fa.scdn.co/segments/"],
            "initialization_template": "v1/sources/src/encodings/enc/profiles/{{profile_id}}/inits/{{file_type}}?fauth=TOKEN",
            "segment_template": "v1/sources/src/encodings/enc/profiles/{{profile_id}}/{{segment_timestamp}}.{{file_type}}?fauth=TOKEN",
            "start_time_millis": 0,
            "end_time_millis": 10_500
        })
    }

    #[test]
    fn reads_the_manifest() {
        let director = parse(&fixture()).unwrap();
        assert_eq!(director.segment_seconds, 4);
        assert_eq!(director.duration_ms(), 10_500);
        assert_eq!(director.audio.as_ref().unwrap().id, 13);
        assert_eq!(
            director
                .video
                .iter()
                .map(|video| video.id)
                .collect::<Vec<_>>(),
            vec![5, 1]
        );
        assert!(!director.encrypted);
        assert_eq!(
            director.base(),
            Some("https://video-cf.spotifycdn.com/segments/")
        );
        let (loudness, peak) = director.loudness.unwrap();
        assert!((loudness + 16.165).abs() < 1e-3 && (peak - 0.0875).abs() < 1e-4);
        assert_eq!(
            manifest_path("7cad"),
            "/manifests/v9/json/sources/7cad/options/supports_drm"
        );
    }

    #[test]
    fn splits_the_stream_into_segments() {
        let director = parse(&fixture()).unwrap();
        assert_eq!(segments(&director), vec![(0, 4.), (4, 4.), (8, 2.5)]);
    }

    #[test]
    fn builds_an_audio_playlist() {
        let director = parse(&fixture()).unwrap();
        let (playlists, root) = playlists(&director, false).unwrap();
        assert_eq!(root, AUDIO_PLAYLIST);
        assert_eq!(playlists.len(), 1);
        let text = String::from_utf8(playlists[0].1.clone()).unwrap();
        assert!(text.starts_with("#EXTM3U\n"));
        assert!(text.contains("#EXT-X-TARGETDURATION:5"));
        assert!(text.contains(
            "#EXT-X-MAP:URI=\"https://video-cf.spotifycdn.com/segments/v1/sources/src/encodings/enc/profiles/13/inits/mp4?fauth=TOKEN\""
        ));
        assert!(text.contains(
            "#EXTINF:4.000,\nhttps://video-cf.spotifycdn.com/segments/v1/sources/src/encodings/enc/profiles/13/4.mp4?fauth=TOKEN"
        ));
        assert!(text.contains("#EXTINF:2.500,\nhttps://video-cf.spotifycdn.com/segments/v1/sources/src/encodings/enc/profiles/13/8.mp4"));
        assert!(!text.contains("{{"));
        assert!(!text.contains("EXT-X-KEY"));
        assert!(text.ends_with("#EXT-X-ENDLIST"));
    }

    #[test]
    fn builds_a_master_playlist_with_avc1_video() {
        let director = parse(&fixture()).unwrap();
        let (playlists, root) = playlists(&director, true).unwrap();
        assert_eq!(root, MASTER_PLAYLIST);
        let names: Vec<&str> = playlists.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                AUDIO_PLAYLIST,
                "video-5.m3u8",
                "video-1.m3u8",
                MASTER_PLAYLIST
            ]
        );
        let master = String::from_utf8(playlists[3].1.clone()).unwrap();
        assert!(master.contains("GROUP-ID=\"audio\""));
        assert!(master.contains(
            "BANDWIDTH=304062,RESOLUTION=320x180,CODECS=\"mp4a.40.2,avc1.4d400d\",AUDIO=\"audio\"\nvideo-5.m3u8"
        ));
        assert!(!master.contains("av01"));
        assert!(!master.contains("vp9"));
    }

    fn music_video() -> Value {
        json!({
            "contents": [{
                "encoding_id": "0000eeee1111ffff2222aaaa3333bbbb",
                "segment_length": 4,
                "start_time_millis": 0,
                "end_time_millis": 9_000,
                "profiles": [
                    {"id": 15, "audio_bitrate": 160000, "audio_codec": "mp4a.40.2", "file_type": "mp4",
                     "encryption_index": 0, "encryption_indices": [0, 1], "max_bitrate": 167566},
                    {"id": 5, "file_type": "mp4", "video_codec": "avc1.4d400d", "video_width": 270,
                     "video_height": 180, "encryption_indices": [0, 1], "max_bitrate": 309168},
                    {"id": 28, "audio_bitrate": 160000, "audio_codec": "mp4a.40.2", "file_type": "ts",
                     "encryption_index": 2, "encryption_indices": [2], "max_bitrate": 218080},
                    {"id": 26, "file_type": "ts", "video_codec": "avc1.4d400d", "video_width": 270,
                     "video_height": 180, "encryption_indices": [2], "max_bitrate": 332008},
                    {"id": 21, "file_type": "ts", "video_codec": "avc1.4d4029", "video_width": 1620,
                     "video_height": 1080, "encryption_indices": [2], "max_bitrate": 8835624},
                    {"id": 9, "file_type": "mp4", "video_codec": "av01.0.08M.10", "video_width": 1620,
                     "video_height": 1080, "encryption_indices": [0, 1]},
                    {"id": 20, "audio_codec": "opus", "file_type": "webm", "encryption_indices": [0, 1]}
                ],
                "encryption_infos": [
                    {"key_system": "widevine", "encryption_scheme": "cenc/ctr",
                     "license_server_endpoint": "/widevine-license/v1/video/license"},
                    {"key_system": "playready", "encryption_scheme": "cenc/ctr",
                     "license_server_endpoint": "/playready-license/v1/video/license"},
                    {"key_system": "fairplay", "encryption_scheme": "cenc/cbcs",
                     "license_server_endpoint": "/fairplay-license/v2/video/license/0000eeee1111ffff2222aaaa3333bbbb?mpegTs=true",
                     "key_format": "com.apple.streamingkeydelivery", "key_method": "SAMPLE-AES"}
                ]
            }],
            "base_urls": ["https://video-cf.spotifycdn.com/segments/"],
            "initialization_template": "v1/sources/src/encodings/enc/profiles/{{profile_id}}/inits/{{file_type}}?fauth=TOKEN",
            "segment_template": "v1/sources/src/encodings/enc/profiles/{{profile_id}}/{{segment_timestamp}}.{{file_type}}?fauth=TOKEN"
        })
    }

    #[test]
    fn reads_fairplay_music_videos() {
        let director = parse(&music_video()).unwrap();
        assert!(director.encrypted);
        let protection = director.protection.clone().unwrap();
        assert_eq!(protection.index, 2);
        assert_eq!(protection.asset_id, "0000eeee1111ffff2222aaaa3333bbbb");
        assert_eq!(
            protection.endpoint,
            "/fairplay-license/v2/video/license/0000eeee1111ffff2222aaaa3333bbbb?mpegTs=true"
        );
        assert_eq!(director.audio.as_ref().unwrap().id, 28);
        assert_eq!(
            director
                .video
                .iter()
                .map(|video| video.id)
                .collect::<Vec<_>>(),
            vec![26, 21]
        );
        assert!(director.has_video());
        assert_eq!(director.aspect(), Some(1.5));

        let (built, root) = playlists(&director, true).unwrap();
        assert_eq!(root, MASTER_PLAYLIST);
        let audio = String::from_utf8(built[0].1.clone()).unwrap();
        assert!(audio.contains(
            "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://0000eeee1111ffff2222aaaa3333bbbb\",KEYFORMATVERSIONS=\"1\",KEYFORMAT=\"com.apple.streamingkeydelivery\""
        ));
        assert!(!audio.contains("EXT-X-MAP"));
        assert!(audio.contains(
            "#EXTINF:4.000,\nhttps://video-cf.spotifycdn.com/segments/v1/sources/src/encodings/enc/profiles/28/4.ts?fauth=TOKEN"
        ));
        let master = String::from_utf8(built.last().unwrap().1.clone()).unwrap();
        assert!(master.contains("RESOLUTION=1620x1080,CODECS=\"mp4a.40.2,avc1.4d4029\""));
        assert!(!master.contains("av01"));

        let (only_audio, root) = playlists(&director, false).unwrap();
        assert_eq!(root, AUDIO_PLAYLIST);
        assert_eq!(only_audio.len(), 1);
    }

    #[test]
    fn leaves_out_streams_only_other_systems_unlock() {
        let mut other = music_video();
        other["contents"][0]["encryption_infos"] = json!([
            {"key_system": "widevine", "license_server_endpoint": "/widevine-license/v1/video/license"}
        ]);
        let director = parse(&other).unwrap();
        assert!(director.protection.is_none());
        assert!(director.audio.is_none());
        assert!(director.video.is_empty());
        assert!(!director.has_video());
        assert!(playlists(&director, true).is_err());

        let clear = parse(&fixture()).unwrap();
        assert!(clear.protection.is_none());
        assert_eq!(clear.aspect(), Some(1280. / 720.));
        assert!(clear.has_video());
    }

    #[test]
    fn refuses_protected_or_empty_streams() {
        let mut protected = fixture();
        protected["contents"][0]["encryption_infos"] = json!([{"key_system": "fairplay"}]);
        assert!(playlists(&parse(&protected).unwrap(), false).is_err());

        let mut silent = fixture();
        silent["contents"][0]["profiles"] = json!([]);
        assert!(playlists(&parse(&silent).unwrap(), false).is_err());

        let mut no_servers = fixture();
        no_servers["base_urls"] = json!(["http://insecure.example/"]);
        assert!(parse(&no_servers).is_err());

        assert!(parse(&json!({})).is_err());
    }
}
