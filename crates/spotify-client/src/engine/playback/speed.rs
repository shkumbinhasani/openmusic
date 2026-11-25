use std::collections::HashMap;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use super::{Playing, gain::state_options, join::track_at, source::is_spoken};
use crate::{
    api::Api,
    engine::Engine,
    playback_settings::{self, round_speed, valid_speed},
};

const SHOW_CACHE: usize = 256;

pub(super) fn state_speed(machine: &Value, index: usize) -> Option<f32> {
    state_options(machine, index)["playback_speed"]
        .as_f64()
        .and_then(|speed| valid_speed(speed as f32))
}

pub(super) fn show_of(machine: &Value, index: usize) -> Option<String> {
    let metadata = &track_at(machine, index)?["metadata"];
    let uri = metadata["uri"].as_str()?;
    if !is_spoken(uri) {
        return None;
    }
    metadata["group_uri"]
        .as_str()
        .filter(|show| show.starts_with("spotify:show:"))
        .map(str::to_owned)
}

pub(super) fn starting_speed(uri: &str, state: Option<f32>, show: Option<f32>) -> f32 {
    if !is_spoken(uri) {
        return 1.;
    }
    match (state, show) {
        (Some(state), _) if state != 1. => state,
        (_, Some(show)) => show,
        (Some(state), None) => state,
        (None, None) => 1.,
    }
}

pub(super) fn chosen(speed: f32) -> Result<f32> {
    match valid_speed(round_speed(speed)) {
        Some(speed) => Ok(speed),
        None => bail!("Choose a speed between 0.5× and 3.5×."),
    }
}

#[derive(Debug, Default)]
pub(super) struct Shows {
    known: HashMap<String, Option<f32>>,
}

impl Shows {
    pub(super) fn get(&self, show: &str) -> Option<Option<f32>> {
        self.known.get(show).copied()
    }

    pub(super) fn set(&mut self, show: String, speed: Option<f32>) {
        if self.known.len() >= SHOW_CACHE && !self.known.contains_key(&show) {
            self.known.clear();
        }
        self.known.insert(show, speed.and_then(valid_speed));
    }
}

pub(super) async fn read_show_speed(
    api: &Api,
    show: Option<String>,
) -> Option<(String, Option<f32>)> {
    let show = show?;
    let remote = playback_settings::read(api, None, std::slice::from_ref(&show))
        .await
        .ok()?;
    let speed = remote.speeds.get(&show).copied();
    Some((show, speed))
}

impl Playing {
    pub(super) fn spoken(&self) -> bool {
        is_spoken(self.uri())
    }
}

impl Engine {
    pub(super) fn known_show_speed(&self, show: Option<&str>) -> Option<f32> {
        self.audio.shows.get(show?).flatten()
    }

    pub(super) fn unknown_show(&self, show: Option<&String>) -> Option<String> {
        show.filter(|show| self.audio.shows.get(show).is_none())
            .cloned()
    }

    pub(super) fn store_show_speed(&mut self, read: Option<(String, Option<f32>)>) {
        if let Some((show, speed)) = read {
            self.audio.shows.set(show, speed);
        }
    }

    pub(super) async fn change_speed(&mut self, speed: f32) -> Result<()> {
        let speed = chosen(speed)?;
        if self.local_active() {
            bail!("Speed works for podcasts from Spotify only.");
        }
        if let Some(target) = self.remote_target() {
            let episode = self
                .playback
                .as_ref()
                .and_then(|playback| playback.track.as_ref())
                .is_some_and(|track| is_spoken(&track.uri));
            if !episode {
                bail!("Speed works for podcasts only.");
            }
            self.send_command(
                &target,
                json!({"endpoint": "set_options", "playback_speed": speed}),
            )
            .await
            .map_err(|_| anyhow::anyhow!("Spotify didn’t change the speed on that device."))?;
            return Ok(());
        }
        let Some(playing) = self.playing.as_mut() else {
            bail!("Nothing is playing.");
        };
        if !playing.spoken() {
            bail!("Speed works for podcasts only.");
        }
        if let Some(show) = show_of(&playing.machine, playing.index) {
            self.audio.shows.set(show, Some(speed));
        }
        let Some(playing) = self.playing.as_mut() else {
            return Ok(());
        };
        if playing.speed != speed {
            playing.speed = speed;
            self.player.set_speed(playing.generation, speed);
            self.report("speed_changed").await;
        }
        Ok(())
    }

    pub(super) fn follow_state_speed(&mut self) -> bool {
        let Some(playing) = self.playing.as_mut() else {
            return false;
        };
        let incoming = state_speed(&playing.machine, playing.index);
        if incoming == playing.state_speed {
            return false;
        }
        playing.state_speed = incoming;
        let Some(speed) = incoming.filter(|speed| *speed != playing.speed && playing.spoken())
        else {
            return false;
        };
        playing.speed = speed;
        self.player.set_speed(playing.generation, speed);
        if let Some(show) = show_of(&playing.machine, playing.index) {
            self.audio.shows.set(show, Some(speed));
        }
        true
    }

    pub(super) fn learn_show_speed(&mut self, show: String, speed: f32) {
        self.audio.shows.set(show, Some(speed));
    }

    pub(super) fn speed_allowed(&self) -> bool {
        if self.local_active() {
            return false;
        }
        if let Some(playing) = self.playing.as_ref().filter(|_| self.is_active_here()) {
            return playing.spoken()
                && self.playback_here().is_none_or(|playback| {
                    playback.allowed.set_speed
                        || playback
                            .track
                            .as_ref()
                            .is_none_or(|track| track.uri != playing.uri())
                });
        }
        self.playback.as_ref().is_some_and(|playback| {
            playback.allowed.set_speed
                && playback
                    .track
                    .as_ref()
                    .is_some_and(|track| is_spoken(&track.uri))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Value {
        json!({
            "attributes": {"options": {"playback_speed": 1.0}},
            "states": [
                {"state_id": "s0", "track": 0},
                {"state_id": "s1", "track": 1, "options": {"playback_speed": 1.5}},
                {"state_id": "s2", "track": 2, "options": {"playback_speed": 9.0}}
            ],
            "tracks": [
                {"metadata": {"uri": "spotify:episode:0000000000000000000001",
                              "group_uri": "spotify:show:0000000000000000000002"}},
                {"metadata": {"uri": "spotify:track:0000000000000000000003",
                              "group_uri": "spotify:album:0000000000000000000004"}},
                {"metadata": {"uri": "spotify:episode:0000000000000000000005"}}
            ]
        })
    }

    #[test]
    fn reads_the_speed_and_show_from_the_state() {
        let machine = machine();
        assert_eq!(state_speed(&machine, 0), Some(1.));
        assert_eq!(state_speed(&machine, 1), Some(1.5));
        assert_eq!(state_speed(&machine, 2), None);
        assert_eq!(
            show_of(&machine, 0).as_deref(),
            Some("spotify:show:0000000000000000000002")
        );
        assert_eq!(show_of(&machine, 1), None);
        assert_eq!(show_of(&machine, 2), None);
    }

    #[test]
    fn starts_episodes_at_the_show_speed_unless_asked_otherwise() {
        let episode = "spotify:episode:x";
        assert_eq!(starting_speed(episode, Some(1.), Some(1.5)), 1.5);
        assert_eq!(starting_speed(episode, None, Some(1.5)), 1.5);
        assert_eq!(starting_speed(episode, Some(2.), Some(1.5)), 2.);
        assert_eq!(starting_speed(episode, Some(1.), None), 1.);
        assert_eq!(starting_speed(episode, None, None), 1.);
        assert_eq!(starting_speed("spotify:track:x", Some(2.), Some(1.5)), 1.);
    }

    #[test]
    fn rounds_and_checks_chosen_speeds() {
        assert_eq!(chosen(1.23).unwrap(), 1.25);
        assert_eq!(chosen(0.5).unwrap(), 0.5);
        assert_eq!(chosen(3.5).unwrap(), 3.5);
        assert!(chosen(0.4).is_err());
        assert!(chosen(4.).is_err());
        assert!(chosen(f32::NAN).is_err());
    }

    #[test]
    fn remembers_show_speeds() {
        let mut shows = Shows::default();
        assert_eq!(shows.get("spotify:show:a"), None);
        shows.set("spotify:show:a".into(), None);
        assert_eq!(shows.get("spotify:show:a"), Some(None));
        shows.set("spotify:show:a".into(), Some(1.5));
        assert_eq!(shows.get("spotify:show:a"), Some(Some(1.5)));
        shows.set("spotify:show:b".into(), Some(12.));
        assert_eq!(shows.get("spotify:show:b"), Some(None));
    }
}
