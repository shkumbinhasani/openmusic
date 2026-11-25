use std::collections::HashMap;

use anyhow::Result;
use prost::Message;
use spotify_proto::extendedmetadata::{
    ExtensionKind,
    audiofiles::{AudioFilesExtensionResponse, NormalizationParams},
};

use crate::{api::Api, catalog, playback_settings::LoudnessLevel};

pub const QUIET_LUFS: f32 = -19.;
pub const NORMAL_LUFS: f32 = -14.;
pub const LOUD_LUFS: f32 = -11.;
pub const PEAK_CEILING_DB: f32 = -1.;
pub const BATCH: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loudness {
    pub loudness_db: f32,
    pub true_peak_db: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TrackLoudness {
    pub track: Option<Loudness>,
    pub album: Option<Loudness>,
}

impl TrackLoudness {
    pub fn pick(&self, album: bool) -> Option<Loudness> {
        if album {
            self.album.or(self.track)
        } else {
            self.track.or(self.album)
        }
    }
}

pub fn target(level: LoudnessLevel) -> f32 {
    match level {
        LoudnessLevel::Quiet => QUIET_LUFS,
        LoudnessLevel::Normal => NORMAL_LUFS,
        LoudnessLevel::Loud => LOUD_LUFS,
    }
}

pub fn gain_db(level: LoudnessLevel, loudness: Loudness) -> f32 {
    let gain = target(level) - loudness.loudness_db;
    if gain <= 0. {
        gain
    } else {
        gain.min((PEAK_CEILING_DB - loudness.true_peak_db).max(0.))
    }
}

pub fn linear(db: f32) -> f32 {
    10f32.powf(db / 20.)
}

pub fn gain(normalize: bool, level: LoudnessLevel, loudness: Option<Loudness>) -> Option<f32> {
    normalize
        .then_some(loudness)
        .flatten()
        .map(|loudness| gain_db(level, loudness))
}

pub fn measured(loudness_db: f32, true_peak_db: f32) -> Option<Loudness> {
    (loudness_db.is_finite() && true_peak_db.is_finite() && loudness_db < 0.).then_some(Loudness {
        loudness_db,
        true_peak_db,
    })
}

fn params(params: Option<&NormalizationParams>) -> Option<Loudness> {
    params.and_then(|params| measured(params.loudness_db, params.true_peak_db))
}

pub fn parse(bytes: &[u8]) -> Option<TrackLoudness> {
    let response = AudioFilesExtensionResponse::decode(bytes).ok()?;
    Some(TrackLoudness {
        track: params(response.default_file_normalization_params.as_ref()),
        album: params(response.default_album_normalization_params.as_ref()),
    })
}

pub fn wanted(uri: &str) -> bool {
    uri.starts_with("spotify:track:") || uri.starts_with("spotify:episode:")
}

pub async fn fetch(api: &Api, uris: &[String]) -> Result<HashMap<String, TrackLoudness>> {
    let mut found = HashMap::new();
    let uris: Vec<&String> = uris.iter().filter(|uri| wanted(uri)).collect();
    for chunk in uris.chunks(BATCH) {
        let entries: Vec<(String, ExtensionKind)> = chunk
            .iter()
            .map(|uri| ((*uri).clone(), ExtensionKind::AudioFiles))
            .collect();
        let data = catalog::extended(api, &entries).await?;
        for (uri, bytes) in data {
            if let Some(loudness) = parse(&bytes) {
                found.insert(uri, loudness);
            }
        }
        for uri in chunk {
            found.entry((*uri).clone()).or_default();
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn computes_the_gain_for_each_level() {
        let remaster = Loudness {
            loudness_db: -11.413,
            true_peak_db: -0.33,
        };
        assert!(close(gain_db(LoudnessLevel::Normal, remaster), -2.587));
        assert!(close(gain_db(LoudnessLevel::Quiet, remaster), -7.587));
        assert!(close(gain_db(LoudnessLevel::Loud, remaster), 0.));
        assert!(close(linear(-2.587), 0.742));

        let quiet_master = Loudness {
            loudness_db: -20.,
            true_peak_db: -4.,
        };
        assert!(close(gain_db(LoudnessLevel::Normal, quiet_master), 3.));
        assert!(close(gain_db(LoudnessLevel::Loud, quiet_master), 3.));
        assert!(close(gain_db(LoudnessLevel::Quiet, quiet_master), 1.));

        let hot_peaks = Loudness {
            loudness_db: -16.,
            true_peak_db: 0.5,
        };
        assert!(close(gain_db(LoudnessLevel::Normal, hot_peaks), 0.));
        assert!(close(gain_db(LoudnessLevel::Quiet, hot_peaks), -3.));

        let loud = Loudness {
            loudness_db: -7.226,
            true_peak_db: 0.697,
        };
        assert!(close(gain_db(LoudnessLevel::Loud, loud), -3.774));
        assert!(
            gain_db(LoudnessLevel::Quiet, loud) < gain_db(LoudnessLevel::Normal, loud)
                && gain_db(LoudnessLevel::Normal, loud) < gain_db(LoudnessLevel::Loud, loud)
        );
    }

    #[test]
    fn applies_gain_only_when_on_and_known() {
        let known = Some(Loudness {
            loudness_db: -11.413,
            true_peak_db: -0.33,
        });
        assert!(gain(true, LoudnessLevel::Normal, known).is_some());
        assert_eq!(gain(false, LoudnessLevel::Normal, known), None);
        assert_eq!(gain(true, LoudnessLevel::Normal, None), None);
        assert_eq!(linear(0.), 1.);
    }

    #[test]
    fn picks_album_or_track_params() {
        let both = TrackLoudness {
            track: Some(Loudness {
                loudness_db: -11.413,
                true_peak_db: -0.33,
            }),
            album: Some(Loudness {
                loudness_db: -8.936,
                true_peak_db: 1.925,
            }),
        };
        assert_eq!(both.pick(false), both.track);
        assert_eq!(both.pick(true), both.album);
        let track_only = TrackLoudness {
            album: None,
            ..both
        };
        assert_eq!(track_only.pick(true), both.track);
        assert_eq!(TrackLoudness::default().pick(false), None);
    }

    #[test]
    fn reads_the_audio_files_extension() {
        let response = AudioFilesExtensionResponse {
            files: Vec::new(),
            default_file_normalization_params: Some(NormalizationParams {
                loudness_db: -15.992,
                true_peak_db: -1.348,
            }),
            default_album_normalization_params: Some(NormalizationParams {
                loudness_db: -15.774,
                true_peak_db: 0.108,
            }),
            audio_id: vec![0x29, 0x30],
        };
        let parsed = parse(&response.encode_to_vec()).unwrap();
        assert_eq!(
            parsed.track,
            Some(Loudness {
                loudness_db: -15.992,
                true_peak_db: -1.348
            })
        );
        assert_eq!(parsed.album.unwrap().true_peak_db, 0.108);

        let episode = AudioFilesExtensionResponse {
            default_file_normalization_params: Some(NormalizationParams {
                loudness_db: -20.091,
                true_peak_db: -0.168,
            }),
            default_album_normalization_params: None,
            ..response.clone()
        };
        let parsed = parse(&episode.encode_to_vec()).unwrap();
        assert!(parsed.track.is_some());
        assert_eq!(parsed.album, None);

        let empty = AudioFilesExtensionResponse {
            default_file_normalization_params: Some(NormalizationParams::default()),
            default_album_normalization_params: None,
            ..response
        };
        assert_eq!(
            parse(&empty.encode_to_vec()),
            Some(TrackLoudness::default())
        );
        assert_eq!(parse(&[0xff, 0xff, 0xff]), None);
    }

    #[test]
    fn asks_only_for_tracks_and_episodes() {
        assert!(wanted("spotify:track:0yNttAVwMr39qyODHNIkrY"));
        assert!(wanted("spotify:episode:1qp98JhxYmyQCXDZPuiOOu"));
        assert!(!wanted("spotify:ad:abc"));
        assert!(!wanted("spotify:local:a:b:c:1"));
        assert!(!wanted("spotify:media:abc"));
        assert_eq!(measured(f32::NAN, 0.), None);
        assert_eq!(measured(3., 0.), None);
    }
}
