use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail};
use serde_json::Value;

use super::Playing;
use crate::{details::hide_context, engine::Engine};

const MAX_RUN: u32 = 25;
const ALL_SKIPPED: &str = "The songs left here are hidden or by artists you chose not to play.";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Skips {
    pub hidden: HashMap<String, HashSet<String>>,
    pub artists: HashSet<String>,
}

impl Skips {
    pub(super) fn skips(&self, track: &Value, context: Option<&str>) -> bool {
        let metadata = &track["metadata"];
        let uris: Vec<&str> = [
            &metadata["uri"],
            &track["uri"],
            &metadata["linked_from_uri"],
        ]
        .into_iter()
        .filter_map(Value::as_str)
        .filter(|uri| uri.starts_with("spotify:track:"))
        .collect();
        if uris.is_empty() {
            return false;
        }
        let artist = metadata["authors"]
            .get(0)
            .and_then(|author| author["uri"].as_str())
            .or_else(|| metadata["artist_uri"].as_str());
        if artist.is_some_and(|artist| self.artists.contains(artist)) {
            return true;
        }
        metadata["context_uri"]
            .as_str()
            .filter(|uri| !uri.is_empty())
            .or(context)
            .and_then(hide_context)
            .and_then(|context| self.hidden.get(&context))
            .is_some_and(|tracks| uris.iter().any(|uri| tracks.contains(*uri)))
    }
}

impl Playing {
    pub(super) fn steps_back_to(&self, machine: &Value, index: usize) -> bool {
        let state_id = |machine: &Value, index: usize| {
            machine["states"]
                .get(index)
                .map(|state| state["state_id"].clone())
        };
        self.transition("skip_prev")
            .and_then(|(back, _)| state_id(&self.machine, back))
            .is_some_and(|back| Some(back) == state_id(machine, index))
    }
}

impl Engine {
    pub(super) fn skipped(&self, track: &Value) -> bool {
        let context = self
            .playback_here()
            .and_then(|playback| playback.context_uri.as_deref());
        self.audio.skips.skips(track, context)
    }

    pub(super) async fn skip_hidden(&mut self, playing: Playing, backwards: bool) -> Result<()> {
        self.audio.skip_run += 1;
        if self.audio.skip_run > MAX_RUN {
            self.audio.skip_run = 0;
            bail!(ALL_SKIPPED);
        }
        let heading = if backwards && playing.transition("skip_prev").is_some() {
            "skip_prev"
        } else {
            "advance"
        };
        Box::pin(self.move_past(playing, heading, "skipped")).await
    }

    pub(super) async fn move_past(
        &mut self,
        playing: Playing,
        heading: &str,
        why: &str,
    ) -> Result<()> {
        let next = playing.transition(heading);
        let paused = playing.paused;
        let machine = playing.machine.clone();
        let generation = playing.generation;
        self.audio.note(format!("{why} {}", playing.uri()));
        self.playing = Some(playing);
        self.report("track_data_finalized").await;
        if self.playing.as_ref().map(|playing| playing.generation) != Some(generation) {
            return Ok(());
        }
        match next {
            Some((index, next_paused)) => {
                Box::pin(self.start_state(machine, index, paused || next_paused, None)).await
            }
            None => {
                if !self.local_active() {
                    self.player.stop();
                }
                if let Some(playing) = self.playing.as_mut() {
                    playing.paused = true;
                }
                self.report("pause").await;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const SONG: &str = "spotify:track:0000000000000000000001";
    const OTHER: &str = "spotify:track:0000000000000000000002";
    const PLAYLIST: &str = "spotify:playlist:00000000000000000000p1";
    const ALBUM: &str = "spotify:album:00000000000000000000a1";
    const ARTIST: &str = "spotify:artist:00000000000000000000r1";

    fn skips() -> Skips {
        Skips {
            hidden: HashMap::from([(PLAYLIST.to_owned(), HashSet::from([SONG.to_owned()]))]),
            artists: HashSet::from([ARTIST.to_owned()]),
        }
    }

    fn track(uri: &str, context: Option<&str>, artist: Option<&str>) -> Value {
        let mut metadata = json!({"uri": uri});
        if let Some(context) = context {
            metadata["context_uri"] = json!(context);
        }
        if let Some(artist) = artist {
            metadata["authors"] = json!([{"name": "Someone", "uri": artist}]);
        }
        json!({"metadata": metadata})
    }

    #[test]
    fn skips_songs_hidden_in_their_context() {
        let skips = skips();
        assert!(skips.skips(&track(SONG, Some(PLAYLIST), None), None));
        assert!(skips.skips(
            &track(
                SONG,
                Some("spotify:user:someone:playlist:00000000000000000000p1"),
                None
            ),
            None
        ));
        assert!(skips.skips(&track(SONG, None, None), Some(PLAYLIST)));
        assert!(!skips.skips(&track(SONG, Some(ALBUM), None), Some(PLAYLIST)));
        assert!(!skips.skips(&track(OTHER, Some(PLAYLIST), None), None));
        assert!(!skips.skips(&track(SONG, None, None), None));
        let relinked = json!({"metadata": {
            "uri": OTHER, "linked_from_uri": SONG, "context_uri": PLAYLIST}});
        assert!(skips.skips(&relinked, None));
    }

    #[test]
    fn skips_songs_by_banned_artists_anywhere() {
        let skips = skips();
        assert!(skips.skips(&track(OTHER, Some(ALBUM), Some(ARTIST)), None));
        assert!(skips.skips(&track(OTHER, None, Some(ARTIST)), None));
        let episode = track(
            "spotify:episode:0000000000000000000003",
            Some(PLAYLIST),
            Some(ARTIST),
        );
        assert!(!skips.skips(&episode, None));
        assert!(!Skips::default().skips(&track(SONG, Some(PLAYLIST), Some(ARTIST)), None));
    }

    #[test]
    fn knows_when_a_state_is_one_step_back() {
        let machine = json!({
            "states": [
                {"state_id": "a", "track": 0, "transitions": {"advance": {"state_index": 1}}},
                {"state_id": "b", "track": 1, "transitions": {
                    "advance": {"state_index": 2}, "skip_prev": {"state_index": 0}}},
                {"state_id": "c", "track": 2, "transitions": {"skip_prev": {"state_index": 1}}},
            ],
            "tracks": [{}, {}, {}],
        });
        let at = |index: usize| Playing::new(machine.clone(), index, false, 1, 0);
        assert!(at(2).steps_back_to(&machine, 1));
        assert!(at(1).steps_back_to(&machine, 0));
        assert!(!at(1).steps_back_to(&machine, 2));
        assert!(!at(0).steps_back_to(&machine, 0));
        let fresh = json!({"states": [{"state_id": "z"}], "tracks": [{}]});
        assert!(!at(2).steps_back_to(&fresh, 0));
    }
}
