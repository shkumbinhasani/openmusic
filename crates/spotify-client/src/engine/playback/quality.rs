use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::Engine;

pub const NORMAL_BITRATE: u64 = 128_000;
const STALL_LIMIT: usize = 2;
const STALL_WINDOW: Duration = Duration::from_secs(10 * 60);
const RECOVERY: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Quality {
    #[default]
    Automatic,
    Normal,
    High,
}

impl Quality {
    pub const ALL: [Self; 3] = [Self::Automatic, Self::Normal, Self::High];

    pub fn name(self) -> &'static str {
        match self {
            Self::Automatic => "auto",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Automatic => "Automatic",
            Self::Normal => "Normal",
            Self::High => "High",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            Self::Automatic => "256 kbps, or 128 kbps while your connection is slow",
            Self::Normal => "128 kbps",
            Self::High => "256 kbps",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("automatic") {
            return Some(Self::Automatic);
        }
        Self::ALL.into_iter().find(|quality| {
            quality.name().eq_ignore_ascii_case(text) || quality.title().eq_ignore_ascii_case(text)
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StreamKind {
    #[default]
    FairPlay,
    Host,
    Video,
    MusicVideo,
    File,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stream {
    pub kind: StreamKind,
    pub codec: Option<String>,
    pub kbps: Option<u32>,
}

impl Stream {
    pub fn describe(&self) -> String {
        let format = match (&self.codec, self.kbps) {
            (Some(codec), Some(kbps)) => format!("{codec} {kbps} kbps"),
            (Some(codec), None) => codec.clone(),
            (None, Some(kbps)) => format!("{kbps} kbps"),
            (None, None) => "Audio".to_owned(),
        };
        match self.kind {
            StreamKind::FairPlay => format!("{format}, FairPlay"),
            StreamKind::Host => format!("{format} from the podcast’s host"),
            StreamKind::Video => format!("{format} from the video podcast"),
            StreamKind::MusicVideo => format!("{format} from the music video, FairPlay"),
            StreamKind::File => format!("{format} file on this Mac"),
        }
    }
}

pub fn codec_of(path: &str) -> Option<String> {
    let path = path.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?;
    let (_, extension) = name.rsplit_once('.')?;
    let codec = match extension.to_ascii_lowercase().as_str() {
        "mp3" => "MP3",
        "aac" => "AAC",
        "m4a" | "m4b" => "M4A",
        "mp4" => "MP4",
        "flac" => "FLAC",
        "wav" => "WAV",
        "aif" | "aiff" => "AIFF",
        "caf" => "CAF",
        "ogg" | "oga" => "Ogg",
        "opus" => "Opus",
        _ => return None,
    };
    Some(codec.to_owned())
}

pub fn quality_label(bitrate: u64) -> &'static str {
    match bitrate {
        256_000.. => "VERY_HIGH",
        128_000.. => "HIGH",
        96_000.. => "NORMAL",
        _ => "LOW",
    }
}

pub(super) fn ceiling(quality: Quality, lowered: bool) -> Option<u64> {
    match quality {
        Quality::Normal => Some(NORMAL_BITRATE),
        Quality::Automatic if lowered => Some(NORMAL_BITRATE),
        Quality::Automatic | Quality::High => None,
    }
}

pub(super) fn bitrate(file: &Value) -> u64 {
    file["bitrate"].as_u64().unwrap_or(NORMAL_BITRATE)
}

pub(super) fn choose(files: &[Value], ceiling: Option<u64>) -> Option<&Value> {
    let usable = files
        .iter()
        .filter(|file| file["file_id"].as_str().is_some_and(|id| !id.is_empty()));
    let Some(limit) = ceiling else {
        return usable.max_by_key(|file| bitrate(file));
    };
    let (within, above): (Vec<&Value>, Vec<&Value>) =
        usable.partition(|file| bitrate(file) <= limit);
    within
        .into_iter()
        .max_by_key(|file| bitrate(file))
        .or_else(|| above.into_iter().min_by_key(|file| bitrate(file)))
}

pub(super) fn reported_quality(file: &Value) -> Option<String> {
    file["audio_quality"]
        .as_str()
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .or_else(|| Some(quality_label(bitrate(file)).to_owned()))
}

#[derive(Debug, Default)]
pub(super) struct Automatic {
    stalls: Vec<Instant>,
    lowered_until: Option<Instant>,
}

impl Automatic {
    pub(super) fn lowered(&self, now: Instant) -> bool {
        self.lowered_until.is_some_and(|until| until > now)
    }

    pub(super) fn stalled(&mut self, now: Instant) -> bool {
        let before = self.lowered(now);
        self.stalls
            .retain(|at| now.saturating_duration_since(*at) < STALL_WINDOW);
        self.stalls.push(now);
        if before || self.stalls.len() >= STALL_LIMIT {
            self.lowered_until = Some(now + RECOVERY);
        }
        !before && self.lowered(now)
    }
}

impl Engine {
    pub(super) fn bitrate_ceiling(&self) -> Option<u64> {
        ceiling(
            self.audio.settings.quality,
            self.audio.automatic.lowered(Instant::now()),
        )
    }

    pub(super) fn lowered_quality(&self) -> bool {
        self.audio.settings.quality == Quality::Automatic
            && self.audio.automatic.lowered(Instant::now())
    }

    pub(super) fn playback_stalled(&mut self, generation: u64) {
        let Some(playing) = self
            .playing
            .as_ref()
            .filter(|playing| playing.generation == generation && playing.source == "fairplay")
        else {
            return;
        };
        let bitrate = playing.bitrate;
        if self.audio.settings.quality != Quality::Automatic || bitrate <= NORMAL_BITRATE {
            self.audio.note(format!("stalled {generation}"));
            return;
        }
        if self.audio.automatic.stalled(Instant::now()) {
            self.audio
                .note(format!("stalled {generation}, next songs at 128 kbps"));
            self.check_prepared();
        } else {
            self.audio.note(format!("stalled {generation}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn files() -> Vec<Value> {
        vec![
            json!({"file_id": "aa", "format": "14", "bitrate": 128000, "audio_quality": "HIGH"}),
            json!({"file_id": "bb", "format": "15", "bitrate": 256000}),
            json!({"file_id": null, "format": "15", "bitrate": 320000}),
        ]
    }

    #[test]
    fn names_and_parses_each_choice() {
        for quality in Quality::ALL {
            assert_eq!(Quality::parse(quality.name()), Some(quality));
            assert_eq!(Quality::parse(quality.title()), Some(quality));
        }
        assert_eq!(Quality::parse(" Automatic "), Some(Quality::Automatic));
        assert_eq!(Quality::parse("very high"), None);
        assert_eq!(Quality::default(), Quality::Automatic);
        assert_eq!(
            serde_json::to_string(&Quality::High).unwrap(),
            "\"High\"".to_owned()
        );
    }

    #[test]
    fn picks_the_file_under_the_ceiling() {
        let files = files();
        assert_eq!(choose(&files, None).unwrap()["file_id"], "bb");
        assert_eq!(
            choose(&files, Some(NORMAL_BITRATE)).unwrap()["file_id"],
            "aa"
        );
        let only_high = vec![files[1].clone()];
        assert_eq!(
            choose(&only_high, Some(NORMAL_BITRATE)).unwrap()["file_id"],
            "bb"
        );
        let unknown = vec![json!({"file_id": "cc"})];
        assert_eq!(bitrate(&unknown[0]), NORMAL_BITRATE);
        assert_eq!(
            choose(&unknown, Some(NORMAL_BITRATE)).unwrap()["file_id"],
            "cc"
        );
        assert!(choose(&[], None).is_none());
        assert!(choose(&[json!({"file_id": ""})], None).is_none());
    }

    #[test]
    fn caps_the_bitrate_by_choice() {
        assert_eq!(ceiling(Quality::High, true), None);
        assert_eq!(ceiling(Quality::Normal, false), Some(NORMAL_BITRATE));
        assert_eq!(ceiling(Quality::Automatic, false), None);
        assert_eq!(ceiling(Quality::Automatic, true), Some(NORMAL_BITRATE));
    }

    #[test]
    fn reports_the_label_spotify_gave_the_file() {
        let files = files();
        assert_eq!(reported_quality(&files[0]).as_deref(), Some("HIGH"));
        assert_eq!(reported_quality(&files[1]).as_deref(), Some("VERY_HIGH"));
        assert_eq!(quality_label(96_000), "NORMAL");
        assert_eq!(quality_label(24_000), "LOW");
        assert_eq!(quality_label(160_000), "HIGH");
    }

    #[test]
    fn steps_down_after_two_stalls_and_recovers() {
        let start = Instant::now();
        let mut automatic = Automatic::default();
        assert!(!automatic.stalled(start));
        assert!(!automatic.lowered(start));
        assert!(automatic.stalled(start + Duration::from_secs(60)));
        assert!(automatic.lowered(start + Duration::from_secs(61)));
        assert!(!automatic.stalled(start + Duration::from_secs(120)));
        assert!(automatic.lowered(start + Duration::from_secs(120) + RECOVERY / 2));
        assert!(!automatic.lowered(start + Duration::from_secs(121) + RECOVERY));

        let mut spaced = Automatic::default();
        assert!(!spaced.stalled(start));
        assert!(!spaced.stalled(start + STALL_WINDOW + Duration::from_secs(1)));
        assert!(!spaced.lowered(start + STALL_WINDOW + Duration::from_secs(2)));
    }

    #[test]
    fn describes_the_stream() {
        let fairplay = Stream {
            kind: StreamKind::FairPlay,
            codec: Some("AAC".into()),
            kbps: Some(256),
        };
        assert_eq!(fairplay.describe(), "AAC 256 kbps, FairPlay");
        let host = Stream {
            kind: StreamKind::Host,
            codec: codec_of("https://dts.podtrac.com/redirect.mp3/example.com/a.mp3?x=1"),
            kbps: None,
        };
        assert_eq!(host.describe(), "MP3 from the podcast’s host");
        let unknown = Stream {
            kind: StreamKind::Host,
            codec: codec_of("https://example.com/episode"),
            kbps: None,
        };
        assert_eq!(unknown.describe(), "Audio from the podcast’s host");
        let video = Stream {
            kind: StreamKind::Video,
            codec: Some("AAC".into()),
            kbps: Some(96),
        };
        assert_eq!(video.describe(), "AAC 96 kbps from the video podcast");
        let music_video = Stream {
            kind: StreamKind::MusicVideo,
            codec: Some("AAC".into()),
            kbps: Some(160),
        };
        assert_eq!(
            music_video.describe(),
            "AAC 160 kbps from the music video, FairPlay"
        );
        let file = Stream {
            kind: StreamKind::File,
            codec: codec_of("/Users/someone/Music/Song.FLAC"),
            kbps: None,
        };
        assert_eq!(file.describe(), "FLAC file on this Mac");
        assert_eq!(codec_of("/tmp/no-extension"), None);
        assert_eq!(codec_of("/tmp/a.m4a").as_deref(), Some("M4A"));
    }
}
