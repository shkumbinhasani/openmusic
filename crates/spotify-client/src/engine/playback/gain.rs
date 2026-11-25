use std::collections::HashMap;

use anyhow::Result;
use serde_json::Value;

use super::{AudioSettings, join::track_at};
use crate::{
    engine::Engine,
    loudness::{self, TrackLoudness},
};

const CACHE: usize = 512;

pub(super) fn state_options(machine: &Value, index: usize) -> &Value {
    machine["states"]
        .get(index)
        .map(|state| &state["options"])
        .filter(|options| options.is_object())
        .unwrap_or(&machine["attributes"]["options"])
}

pub(super) fn album_order(machine: &Value, index: usize) -> bool {
    let shuffling = state_options(machine, index)["shuffling_context"]
        .as_bool()
        .unwrap_or(false);
    !shuffling
        && track_at(machine, index)
            .and_then(|track| track["metadata"]["context_uri"].as_str())
            .is_some_and(|uri| uri.starts_with("spotify:album:"))
}

pub(super) fn machine_uris(machine: &Value) -> Vec<String> {
    machine["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|track| track["metadata"]["uri"].as_str())
        .filter(|uri| loudness::wanted(uri))
        .map(str::to_owned)
        .collect()
}

pub(super) fn gain_for(
    settings: &AudioSettings,
    known: Option<&TrackLoudness>,
    album: bool,
) -> Option<f32> {
    loudness::gain(
        settings.normalize,
        settings.level,
        known.and_then(|known| known.pick(album)),
    )
}

pub(super) fn linear(gain_db: Option<f32>) -> f32 {
    gain_db.map_or(1., loudness::linear)
}

#[derive(Debug, Default)]
pub(super) struct Gains {
    known: HashMap<String, TrackLoudness>,
}

impl Gains {
    pub(super) fn missing(&self, machine: &Value) -> Vec<String> {
        let mut uris = machine_uris(machine);
        uris.retain(|uri| !self.known.contains_key(uri));
        uris.sort();
        uris.dedup();
        uris
    }

    pub(super) fn store(&mut self, found: HashMap<String, TrackLoudness>) {
        if self.known.len() + found.len() > CACHE {
            self.known.clear();
        }
        self.known.extend(found);
    }

    pub(super) fn get(&self, uri: &str) -> Option<&TrackLoudness> {
        self.known.get(uri)
    }
}

impl Engine {
    pub(super) fn store_loudness(&mut self, found: Result<HashMap<String, TrackLoudness>>) {
        if let Ok(found) = found {
            self.audio.gains.store(found);
        }
    }

    pub(super) fn gain_at(&self, machine: &Value, index: usize) -> Option<f32> {
        let uri = track_at(machine, index)?["metadata"]["uri"].as_str()?;
        gain_for(
            &self.audio.settings,
            self.audio.gains.get(uri),
            album_order(machine, index),
        )
    }

    pub(super) fn refresh_gains(&mut self) {
        let Some(playing) = self.playing.as_ref() else {
            return;
        };
        let current = self.gain_at(&playing.machine, playing.index);
        let generation = playing.generation;
        let prepared = self.audio.joins.prepared.as_ref().map(|prepared| {
            (
                prepared.generation,
                self.gain_at(&playing.machine, prepared.index()),
            )
        });
        if let Some(playing) = self.playing.as_mut()
            && playing.gain_db != current
        {
            playing.gain_db = current;
            self.player.set_gain(generation, linear(current));
        }
        if let Some((generation, gain)) = prepared
            && let Some(prepared) = self.audio.joins.prepared.as_mut()
            && prepared.gain_db != gain
        {
            prepared.gain_db = gain;
            self.player.set_gain(generation, linear(gain));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{loudness::Loudness, playback_settings::LoudnessLevel};

    fn machine(context: &str, shuffling: bool) -> Value {
        json!({
            "attributes": {"options": {"shuffling_context": shuffling}},
            "states": [{"state_id": "s0", "track": 0}, {"state_id": "s1", "track": 1,
                "options": {"shuffling_context": false}}],
            "tracks": [
                {"metadata": {"uri": "spotify:track:a", "context_uri": context}},
                {"metadata": {"uri": "spotify:ad:b", "context_uri": context}}
            ]
        })
    }

    #[test]
    fn uses_album_params_only_for_albums_in_order() {
        assert!(album_order(&machine("spotify:album:x", false), 0));
        assert!(!album_order(&machine("spotify:album:x", true), 0));
        assert!(album_order(&machine("spotify:album:x", true), 1));
        assert!(!album_order(&machine("spotify:playlist:x", false), 0));
        assert!(!album_order(&machine("spotify:album:x", false), 5));
    }

    #[test]
    fn asks_only_for_unknown_tracks() {
        let machine = machine("spotify:album:x", false);
        assert_eq!(machine_uris(&machine), vec!["spotify:track:a".to_owned()]);
        let mut gains = Gains::default();
        assert_eq!(gains.missing(&machine), vec!["spotify:track:a".to_owned()]);
        gains.store(HashMap::from([(
            "spotify:track:a".to_owned(),
            TrackLoudness::default(),
        )]));
        assert!(gains.missing(&machine).is_empty());
        assert_eq!(
            gains.get("spotify:track:a"),
            Some(&TrackLoudness::default())
        );
    }

    #[test]
    fn follows_the_normalize_switch_and_level() {
        let known = TrackLoudness {
            track: Some(Loudness {
                loudness_db: -11.413,
                true_peak_db: -0.33,
            }),
            album: Some(Loudness {
                loudness_db: -8.936,
                true_peak_db: 1.925,
            }),
        };
        let on = AudioSettings::default();
        let track = gain_for(&on, Some(&known), false).unwrap();
        assert!((track + 2.587).abs() < 1e-3);
        let album = gain_for(&on, Some(&known), true).unwrap();
        assert!((album + 5.064).abs() < 1e-3);
        let off = AudioSettings {
            normalize: false,
            ..on
        };
        assert_eq!(gain_for(&off, Some(&known), false), None);
        let quiet = AudioSettings {
            level: LoudnessLevel::Quiet,
            ..on
        };
        assert!(gain_for(&quiet, Some(&known), false).unwrap() < track);
        assert_eq!(gain_for(&on, None, false), None);
        assert_eq!(linear(None), 1.);
        assert!((linear(Some(-2.587)) - 0.742).abs() < 1e-3);
    }
}
