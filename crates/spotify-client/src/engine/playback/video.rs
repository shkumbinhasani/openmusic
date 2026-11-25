use std::sync::Arc;

use anyhow::{Context, Result};
use reqwest::Method;
use serde_json::Value;

use super::{
    Update,
    gain::linear,
    source::{Pick, audio_pick, video_manifest},
};
use crate::{
    api::Body,
    connect, director,
    engine::Engine,
    loudness,
    player::{Hls, HlsKey, Media, Source},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoKind {
    Podcast,
    Music,
}

impl VideoKind {
    pub fn of(uri: &str) -> Option<Self> {
        if uri.starts_with("spotify:episode:") || uri.starts_with("spotify:chapter:") {
            Some(Self::Podcast)
        } else if uri.starts_with("spotify:track:") {
            Some(Self::Music)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Podcast => "podcast",
            Self::Music => "music",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoPrefs {
    pub music: bool,
    pub podcasts: bool,
}

impl Default for VideoPrefs {
    fn default() -> Self {
        Self {
            music: true,
            podcasts: true,
        }
    }
}

impl VideoPrefs {
    pub fn wants(self, uri: &str) -> bool {
        match VideoKind::of(uri) {
            Some(VideoKind::Podcast) => self.podcasts,
            Some(VideoKind::Music) => self.music,
            None => false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoStatus {
    pub kind: VideoKind,
    pub on: bool,
    pub aspect: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Carried {
    pub(super) on: bool,
    pub(super) aspect: Option<f32>,
}

pub(super) struct Opened {
    pub(super) media: Media,
    pub(super) bitrate: u64,
    pub(super) video: Option<Carried>,
}

impl Opened {
    pub(super) fn audio(media: Media, bitrate: u64) -> Self {
        Self {
            media,
            bitrate,
            video: None,
        }
    }
}

const REFUSED: &str = "This video can’t play right now, so OpenMusic plays its audio.";

#[derive(Default)]
pub(super) struct Video {
    pub(super) prefs: VideoPrefs,
    refused: Option<String>,
}

impl Video {
    pub(super) fn prefs_for(&self, uri: &str) -> VideoPrefs {
        if self.refused.as_deref() == Some(uri) {
            VideoPrefs {
                music: false,
                podcasts: false,
            }
        } else {
            self.prefs
        }
    }
}

pub(super) fn offer(uri: &str, track: &Value, carried: Option<Carried>) -> Option<VideoStatus> {
    let kind = VideoKind::of(uri)?;
    match carried {
        Some(carried) => Some(VideoStatus {
            kind,
            on: carried.on,
            aspect: carried.aspect,
        }),
        None => video_manifest(&track["manifest"]).map(|_| VideoStatus {
            kind,
            on: false,
            aspect: None,
        }),
    }
}

pub(super) fn needs_swap(status: Option<VideoStatus>, prefs: VideoPrefs, uri: &str) -> bool {
    status.is_some_and(|status| status.on != prefs.wants(uri))
}

impl Engine {
    pub(super) async fn certificate(&mut self) -> Result<Arc<Vec<u8>>> {
        if let Some(certificate) = &self.certificate {
            return Ok(certificate.clone());
        }
        let bytes = self
            .api
            .request(
                Method::GET,
                "/fairplay-license/v1/application-certificate",
                Body::Empty,
                "*/*",
            )
            .await
            .context("Could not get Spotify's FairPlay certificate.")?;
        let certificate = Arc::new(bytes);
        self.certificate = Some(certificate.clone());
        Ok(certificate)
    }

    pub(super) async fn open_director(&mut self, id: &str, uri: &str) -> Result<Opened> {
        let manifest = director::manifest(&self.api, id).await?;
        if let Some(measured) = manifest
            .loudness
            .and_then(|(level, peak)| loudness::measured(level, peak))
        {
            self.audio.gains.learn(uri, measured);
        }
        let show = self.audio.video.prefs_for(uri).wants(uri) && manifest.has_video();
        let (playlists, root) = director::playlists(&manifest, show)?;
        let key = match &manifest.protection {
            Some(protection) => Some(HlsKey {
                asset_id: protection.asset_id.clone(),
                certificate: self.certificate().await?,
                license_url: self.api.url(&protection.endpoint),
                token: self.api.token(),
            }),
            None => None,
        };
        let on = root == director::MASTER_PLAYLIST;
        Ok(Opened {
            media: Media::new(Source::Hls(Hls {
                playlists,
                root,
                key,
            })),
            bitrate: manifest.audio.as_ref().map_or(0, |profile| profile.bitrate),
            video: manifest.has_video().then(|| Carried {
                on,
                aspect: manifest.aspect(),
            }),
        })
    }

    pub(super) async fn recover(
        &mut self,
        track: &Value,
        uri: &str,
        pick: Pick,
        opened: Result<Opened>,
    ) -> (Pick, Result<Opened>) {
        let Err(error) = &opened else {
            return (pick, opened);
        };
        if !matches!(pick, Pick::Director(_)) {
            return (pick, opened);
        }
        let Some(audio) = audio_pick(track, self.bitrate_ceiling()).filter(|audio| *audio != pick)
        else {
            return (pick, opened);
        };
        self.audio
            .note(format!("video failed for {uri}, playing audio: {error}"));
        let retry = self.open(&audio, uri).await;
        (audio, retry)
    }

    pub(super) async fn set_video(&mut self, prefs: VideoPrefs) {
        if self.audio.video.prefs == prefs {
            return;
        }
        self.audio.video.prefs = prefs;
        self.audio.video.refused = None;
        self.discard_prepared();
        self.audio.joins.tried = None;
        self.swap_media().await;
    }

    pub(super) fn video_status(&self) -> Option<VideoStatus> {
        if self.audio.local.is_some() || self.audio.unloaded || !self.is_active_here() {
            return None;
        }
        self.playing.as_ref().and_then(|playing| playing.video)
    }

    async fn swap_media(&mut self) {
        let Some(playing) = self.playing.as_ref() else {
            return;
        };
        if self.local_active() || self.audio.unloaded {
            return;
        }
        let uri = playing.uri().to_owned();
        if !needs_swap(playing.video, self.audio.video.prefs_for(&uri), &uri) {
            return;
        }
        let Some(track) = playing.track().cloned() else {
            return;
        };
        let generation = playing.generation;
        let pick = match self.resolve(&track).await {
            Ok(pick) => pick,
            Err(error) => {
                self.audio.note(format!("video switch failed: {error}"));
                return;
            }
        };
        let opened = self.open(&pick, &uri).await;
        let (pick, opened) = self.recover(&track, &uri, pick, opened).await;
        let Opened {
            media,
            bitrate,
            video: carried,
        } = match opened {
            Ok(opened) => opened,
            Err(error) => {
                self.audio.note(format!("video switch failed: {error}"));
                return;
            }
        };
        let Some(playing) = self
            .playing
            .as_mut()
            .filter(|playing| playing.generation == generation)
        else {
            return;
        };
        playing.format = pick.format();
        playing.bitrate = if bitrate > 0 { bitrate } else { pick.bitrate() };
        playing.source = pick.name();
        playing.codec = pick.codec();
        playing.audio_quality = pick.reported_quality(playing.bitrate);
        playing.video = offer(&uri, &track, carried);
        let media = Media {
            start_ms: playing.position_ms,
            play: !playing.paused,
            gain: linear(playing.gain_db),
            speed: playing.speed,
            ..media
        };
        let shown = playing.video.is_some_and(|video| video.on);
        self.discard_prepared();
        self.audio.joins.tried = None;
        self.player.load(generation, media);
        self.audio.note(format!(
            "switched {uri} to {}",
            if shown { "video" } else { "audio" }
        ));
        self.report("modify_current_state").await;
    }

    pub(super) async fn video_failed(&mut self, generation: u64, message: &str) -> bool {
        let Some(playing) = self
            .playing
            .as_ref()
            .filter(|playing| playing.generation == generation)
        else {
            return false;
        };
        if !playing.video.is_some_and(|video| video.on) {
            return false;
        }
        let uri = playing.uri().to_owned();
        if self.audio.video.refused.as_deref() == Some(uri.as_str()) {
            return false;
        }
        self.audio.video.refused = Some(uri.clone());
        self.audio
            .note(format!("video refused for {uri}: {message}"));
        self.audio.updates.push(Update::Notice(REFUSED.to_owned()));
        self.swap_media().await;
        self.playing
            .as_ref()
            .is_some_and(|playing| !playing.video.is_some_and(|video| video.on))
    }

    pub(super) async fn watch(&mut self, uri: String, position_ms: u32) -> Result<()> {
        if VideoKind::of(&uri).is_none() {
            return Ok(());
        }
        let own = self.device_id.clone();
        self.send_command(&own, connect::play_command(&[uri], 0, position_ms))
            .await
            .context("Spotify didn’t start the video.")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const EPISODE: &str = "spotify:episode:0000000000000000000001";
    const TRACK: &str = "spotify:track:0000000000000000000002";

    #[test]
    fn follows_the_setting_for_each_kind() {
        let prefs = VideoPrefs::default();
        assert!(prefs.wants(EPISODE));
        assert!(prefs.wants(TRACK));
        assert!(!prefs.wants("spotify:local:a:b:c:10"));
        let audio_only = VideoPrefs {
            music: false,
            podcasts: true,
        };
        assert!(audio_only.wants(EPISODE));
        assert!(!audio_only.wants(TRACK));
        assert_eq!(VideoKind::of(EPISODE), Some(VideoKind::Podcast));
        assert_eq!(VideoKind::of("spotify:chapter:x"), Some(VideoKind::Podcast));
        assert_eq!(VideoKind::of(TRACK), Some(VideoKind::Music));
        assert_eq!(VideoKind::of("spotify:media:x"), None);
        assert_eq!(VideoKind::Music.name(), "music");
    }

    #[test]
    fn offers_video_from_the_stream_or_the_manifest() {
        let plain = json!({"manifest": {"file_ids_mp4_cbcs": [{"file_id": "cc"}]}});
        assert_eq!(offer(TRACK, &plain, None), None);
        let video = json!({"manifest": {
            "file_ids_mp4_cbcs": [{"file_id": "cc"}],
            "manifest_ids_video": [{"file_id": "0000aaaa1111bbbb2222cccc3333dddd"}]
        }});
        assert_eq!(
            offer(TRACK, &video, None),
            Some(VideoStatus {
                kind: VideoKind::Music,
                on: false,
                aspect: None,
            })
        );
        let carried = Carried {
            on: true,
            aspect: Some(16. / 9.),
        };
        assert_eq!(
            offer(EPISODE, &plain, Some(carried)),
            Some(VideoStatus {
                kind: VideoKind::Podcast,
                on: true,
                aspect: Some(16. / 9.),
            })
        );
        assert_eq!(offer("spotify:local:a:b:c:1", &video, Some(carried)), None);
    }

    #[test]
    fn keeps_a_refused_video_as_audio() {
        let mut video = Video::default();
        assert_eq!(video.prefs_for(TRACK), VideoPrefs::default());
        video.refused = Some(TRACK.into());
        assert!(!video.prefs_for(TRACK).wants(TRACK));
        assert!(video.prefs_for(EPISODE).wants(EPISODE));
    }

    #[test]
    fn swaps_only_when_the_setting_and_the_stream_disagree() {
        let status = VideoStatus {
            kind: VideoKind::Podcast,
            on: true,
            aspect: None,
        };
        let shown = Some(status);
        let hidden = Some(VideoStatus {
            on: false,
            ..status
        });
        let off = VideoPrefs {
            music: true,
            podcasts: false,
        };
        assert!(!needs_swap(shown, VideoPrefs::default(), EPISODE));
        assert!(needs_swap(shown, off, EPISODE));
        assert!(needs_swap(hidden, VideoPrefs::default(), EPISODE));
        assert!(!needs_swap(hidden, off, EPISODE));
        assert!(!needs_swap(None, VideoPrefs::default(), EPISODE));
    }
}
