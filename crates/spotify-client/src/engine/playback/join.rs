use std::time::{Duration, Instant};

use serde_json::Value;

use super::{AudioSettings, Playing, ReportPrepared};
use crate::{engine::Engine, player::Join};

pub(super) const PREPARE_LEAD_MS: u32 = 30_000;
const JOIN_LOG: usize = 12;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Target {
    pub(super) index: usize,
    pub(super) paused: bool,
    state_id: Value,
    pub(super) uri: String,
}

pub(super) fn track_at(machine: &Value, state_index: usize) -> Option<&Value> {
    let state = machine["states"].get(state_index)?;
    machine["tracks"].get(state["track"].as_u64()? as usize)
}

pub(super) fn target(machine: &Value, index: usize, transition: &str) -> Option<Target> {
    let step = machine["states"]
        .get(index)?
        .get("transitions")?
        .get(transition)?;
    let next = step["state_index"].as_u64()? as usize;
    let state = machine["states"].get(next)?;
    Some(Target {
        index: next,
        paused: step["paused"].as_bool().unwrap_or(false),
        state_id: state["state_id"].clone(),
        uri: track_at(machine, next)?["metadata"]["uri"]
            .as_str()?
            .to_owned(),
    })
}

pub(super) fn preparable(machine: &Value, target: &Target) -> bool {
    let Some(state) = machine["states"].get(target.index) else {
        return false;
    };
    let unset = |key: &str| state.get(key).is_none_or(Value::is_null);
    let zero = |key: &str| state[key].as_u64().unwrap_or(0) == 0;
    !target.paused
        && unset("playback_offsets")
        && unset("duration_override")
        && unset("ad_break_context")
        && zero("initial_playback_position")
        && zero("position_offset")
        && (target.uri.starts_with("spotify:track:") || target.uri.starts_with("spotify:episode:"))
}

pub(super) fn join_for(
    settings: &AudioSettings,
    from: (&str, u32),
    to: (&str, u32),
) -> Option<Join> {
    let music = |uri: &str| uri.starts_with("spotify:track:");
    let overlap = settings.crossfade_ms;
    if settings.crossfade && overlap == 0 {
        return Some(Join::Gapless);
    }
    let shortest = overlap.saturating_mul(2).saturating_add(1000);
    if settings.crossfade && music(from.0) && music(to.0) && from.1 >= shortest && to.1 >= shortest
    {
        return Some(Join::Crossfade);
    }
    settings.gapless.then_some(Join::Gapless)
}

pub(super) fn crossfade_length(overlap_ms: u32, remaining_ms: u32) -> Option<u32> {
    (overlap_ms > 0
        && remaining_ms > 0
        && remaining_ms <= overlap_ms
        && remaining_ms >= overlap_ms / 2)
        .then_some(remaining_ms)
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct File {
    pub(super) id: String,
    pub(super) format: Value,
    pub(super) format_number: String,
    pub(super) bitrate: u64,
}

pub(super) fn choose_file(track: &Value) -> Option<File> {
    let file = track["manifest"]["file_ids_mp4_cbcs"]
        .as_array()?
        .iter()
        .filter(|file| file["file_id"].is_string())
        .max_by_key(|file| file["bitrate"].as_u64().unwrap_or_default())?;
    Some(File {
        id: file["file_id"].as_str()?.to_owned(),
        format: file["format"].clone(),
        format_number: file["format"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| file["format"].to_string()),
        bitrate: file["bitrate"].as_u64().unwrap_or(128_000),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Prepared {
    pub(super) generation: u64,
    target: Target,
    pub(super) join: Join,
    file: File,
    duration_ms: u32,
    pub(super) ready: bool,
}

impl Prepared {
    pub(super) fn matches(&self, target: &Target) -> bool {
        !target.paused && self.target.state_id == target.state_id && self.target.uri == target.uri
    }

    pub(super) fn report(&self) -> ReportPrepared {
        ReportPrepared {
            generation: self.generation,
            uri: self.target.uri.clone(),
            join: self.join,
            ready: self.ready,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Joins {
    pub(super) prepared: Option<Prepared>,
    pub(super) tried: Option<u64>,
    pub(super) fading_until: Option<Instant>,
    pub(super) log: Vec<String>,
}

impl Joins {
    pub(super) fn fading(&self) -> bool {
        self.fading_until
            .is_some_and(|until| until > Instant::now())
    }

    pub(super) fn record(&mut self, line: String) {
        self.log.push(line);
        if self.log.len() > JOIN_LOG {
            self.log.remove(0);
        }
    }

    pub(super) fn reset(&mut self) {
        self.prepared = None;
        self.tried = None;
        self.fading_until = None;
    }
}

pub(super) enum Taken {
    Joined(u64),
    Missing,
    Superseded,
}

impl Engine {
    pub(super) fn discard_prepared(&mut self) {
        if let Some(prepared) = self.audio.joins.prepared.take() {
            self.player.discard(prepared.generation);
        }
    }

    pub(super) fn wanted_join(&self) -> Option<(Target, Join)> {
        let playing = self.playing.as_ref()?;
        let target = target(&playing.machine, playing.index, "advance")?;
        if !preparable(&playing.machine, &target) {
            return None;
        }
        let next_ms = track_at(&playing.machine, target.index)?["metadata"]["duration"]
            .as_u64()
            .unwrap_or_default() as u32;
        let join = join_for(
            &self.audio.settings,
            (playing.uri(), playing.duration_ms),
            (&target.uri, next_ms),
        )?;
        Some((target, join))
    }

    pub(super) fn check_prepared(&mut self) {
        let Some(prepared) = &self.audio.joins.prepared else {
            return;
        };
        let keep = self
            .wanted_join()
            .is_some_and(|(target, join)| prepared.matches(&target) && prepared.join == join);
        if !keep {
            self.discard_prepared();
            self.audio.joins.tried = None;
        }
    }

    pub(super) async fn prepare_next(&mut self) {
        let Some(playing) = self.playing.as_ref() else {
            return;
        };
        let joins = &self.audio.joins;
        if joins.prepared.is_some()
            || playing.paused
            || joins.tried == Some(playing.generation)
            || joins.fading()
            || self.player.snapshot().ramp.is_some()
            || playing.duration_ms == 0
        {
            return;
        }
        let settings = &self.audio.settings;
        let lead = PREPARE_LEAD_MS
            + if settings.crossfade {
                settings.crossfade_ms
            } else {
                0
            };
        if playing.duration_ms.saturating_sub(playing.position_ms) > lead {
            return;
        }
        let source = playing.generation;
        let Some((target, join)) = self.wanted_join() else {
            return;
        };
        let Some(file) = track_at(&playing.machine, target.index).and_then(choose_file) else {
            self.audio.joins.tried = Some(source);
            return;
        };
        let duration_ms = track_at(&playing.machine, target.index)
            .and_then(|track| track["metadata"]["duration"].as_u64())
            .unwrap_or_default() as u32;
        self.generation += 1;
        let generation = self.generation;
        let media = match self.media(&file.id, &file.format_number).await {
            Ok(media) => media,
            Err(_) => {
                self.audio.joins.tried = Some(source);
                return;
            }
        };
        if self.playing.as_ref().map(|playing| playing.generation) != Some(source) {
            return;
        }
        self.player.prepare(generation, media, join);
        self.audio.joins.prepared = Some(Prepared {
            generation,
            target,
            join,
            file,
            duration_ms,
            ready: false,
        });
    }

    pub(super) fn prepared_ready(&mut self, generation: u64, duration_ms: u32) -> bool {
        let Some(prepared) = self
            .audio
            .joins
            .prepared
            .as_mut()
            .filter(|prepared| prepared.generation == generation)
        else {
            return false;
        };
        prepared.ready = true;
        if duration_ms > 0 {
            prepared.duration_ms = duration_ms;
        }
        true
    }

    pub(super) fn prepared_failed(&mut self, generation: u64) -> bool {
        if self
            .audio
            .joins
            .prepared
            .as_ref()
            .is_none_or(|prepared| prepared.generation != generation)
        {
            return false;
        }
        self.discard_prepared();
        self.audio.joins.tried = self.playing.as_ref().map(|playing| playing.generation);
        true
    }

    fn prepared_target(&self, transition: &str) -> Option<Target> {
        let playing = self.playing.as_ref()?;
        let prepared = self.audio.joins.prepared.as_ref()?;
        target(&playing.machine, playing.index, transition)
            .filter(|target| prepared.matches(target))
    }

    pub(super) async fn take_prepared(
        &mut self,
        transition: &str,
        how: &str,
        finished: bool,
    ) -> Taken {
        let Some(old) = self.playing.as_ref().map(|playing| playing.generation) else {
            return Taken::Missing;
        };
        if self.audio.joins.prepared.is_none() {
            return Taken::Missing;
        }
        if self.prepared_target(transition).is_none() {
            self.discard_prepared();
            return Taken::Missing;
        }
        if finished && let Some(playing) = self.playing.as_mut() {
            playing.position_ms = playing.duration_ms.max(playing.position_ms);
        }
        self.report("track_data_finalized").await;
        if self.playing.as_ref().map(|playing| playing.generation) != Some(old) {
            self.discard_prepared();
            return Taken::Superseded;
        }
        let Some(target) = self.prepared_target(transition) else {
            self.discard_prepared();
            return Taken::Missing;
        };
        let (Some(prepared), Some(playing)) =
            (self.audio.joins.prepared.take(), self.playing.as_ref())
        else {
            return Taken::Missing;
        };
        let mut next = Playing::new(
            playing.machine.clone(),
            target.index,
            false,
            prepared.generation,
            0,
        );
        next.format = prepared.file.format.clone();
        next.bitrate = prepared.file.bitrate;
        next.duration_ms = prepared.duration_ms;
        self.playing = Some(next);
        self.audio.joins.tried = None;
        self.audio
            .joins
            .record(format!("{how} {old}->{}", prepared.generation));
        self.report("before_track_load").await;
        Taken::Joined(prepared.generation)
    }

    pub(super) async fn joined(&mut self, from: u64, to: u64) {
        if self
            .playing
            .as_ref()
            .is_none_or(|playing| playing.generation != from)
        {
            return;
        }
        let expected = self
            .audio
            .joins
            .prepared
            .as_ref()
            .is_some_and(|prepared| prepared.generation == to);
        if expected {
            match self
                .take_prepared("advance", Join::Gapless.name(), true)
                .await
            {
                Taken::Joined(_) | Taken::Superseded => return,
                Taken::Missing => {}
            }
        }
        self.advance_cold("advance").await;
    }

    pub(super) async fn crossfade_due(&mut self) {
        let (Some(playing), Some(prepared)) =
            (self.playing.as_ref(), self.audio.joins.prepared.as_ref())
        else {
            return;
        };
        if prepared.join != Join::Crossfade || !prepared.ready || playing.paused {
            return;
        }
        let remaining = playing.duration_ms.saturating_sub(playing.position_ms);
        let Some(length) = crossfade_length(self.audio.settings.crossfade_ms, remaining)
            .filter(|_| playing.duration_ms > 0)
        else {
            return;
        };
        let how = format!("crossfade {length}ms");
        if let Taken::Joined(generation) = self.take_prepared("advance", &how, false).await {
            self.player.crossfade(generation, length);
            self.audio.joins.fading_until =
                Some(Instant::now() + Duration::from_millis(u64::from(length)));
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn machine() -> Value {
        json!({
            "state_machine_id": "m1",
            "states": [
                {"state_id": "s0", "track": 0, "transitions": {
                    "advance": {"state_index": 1, "paused": false},
                    "skip_next": {"state_index": 1, "paused": false},
                    "skip_prev": {"state_index": 0, "paused": false}}},
                {"state_id": "s1", "track": 1, "transitions": {
                    "advance": {"state_index": 2, "paused": true},
                    "skip_prev": {"state_index": 0}}},
                {"state_id": "s2", "track": 2, "initial_playback_position": 61000},
                {"state_id": "s3", "track": 3, "playback_offsets": {"start": 0, "end": 30000}},
                {"state_id": "s4", "track": 4, "ad_break_context": {"id": "x"}}
            ],
            "tracks": [
                {"metadata": {"uri": "spotify:track:a", "duration": 200000},
                 "manifest": {"file_ids_mp4_cbcs": [
                    {"file_id": "aa", "format": "14", "bitrate": 128000},
                    {"file_id": "ab", "format": "15", "bitrate": 256000}]}},
                {"metadata": {"uri": "spotify:track:b", "duration": 180000},
                 "manifest": {"file_ids_mp4_cbcs": [{"file_id": "ba", "format": 14, "bitrate": 128000}]}},
                {"metadata": {"uri": "spotify:episode:c", "duration": 3600000}, "manifest": {}},
                {"metadata": {"uri": "spotify:track:d", "duration": 30000}},
                {"metadata": {"uri": "spotify:ad:e", "duration": 15000}}
            ]
        })
    }

    fn prepared(target: Target) -> Prepared {
        Prepared {
            generation: 7,
            target,
            join: Join::Gapless,
            file: File {
                id: "ba".into(),
                format: json!(14),
                format_number: "14".into(),
                bitrate: 128_000,
            },
            duration_ms: 180_000,
            ready: false,
        }
    }

    #[test]
    fn follows_transitions_to_the_next_state() {
        let machine = machine();
        let next = target(&machine, 0, "advance").unwrap();
        assert_eq!(next.index, 1);
        assert!(!next.paused);
        assert_eq!(next.uri, "spotify:track:b");
        assert_eq!(target(&machine, 0, "skip_next"), Some(next.clone()));
        assert_eq!(
            target(&machine, 0, "skip_prev").unwrap().uri,
            "spotify:track:a"
        );
        assert!(target(&machine, 1, "advance").unwrap().paused);
        assert_eq!(target(&machine, 2, "advance"), None);
        assert_eq!(target(&machine, 9, "advance"), None);
    }

    #[test]
    fn prepares_only_plain_next_items() {
        let machine = machine();
        assert!(preparable(
            &machine,
            &target(&machine, 0, "advance").unwrap()
        ));
        assert!(!preparable(
            &machine,
            &target(&machine, 1, "advance").unwrap()
        ));
        let at = |index: usize, uri: &str| Target {
            index,
            paused: false,
            state_id: json!(format!("s{index}")),
            uri: uri.into(),
        };
        assert!(!preparable(&machine, &at(2, "spotify:episode:c")));
        assert!(!preparable(&machine, &at(3, "spotify:track:d")));
        assert!(!preparable(&machine, &at(4, "spotify:ad:e")));
        assert!(!preparable(&machine, &at(9, "spotify:track:z")));
    }

    #[test]
    fn keeps_a_prepared_item_only_while_the_target_holds() {
        let machine = machine();
        let next = target(&machine, 0, "advance").unwrap();
        let item = prepared(next.clone());
        assert!(item.matches(&next));
        assert!(item.matches(&target(&machine, 0, "skip_next").unwrap()));
        assert!(!item.matches(&target(&machine, 0, "skip_prev").unwrap()));

        let mut swapped = machine.clone();
        swapped["states"][1]["state_id"] = json!("s1-new");
        assert!(!item.matches(&target(&swapped, 0, "advance").unwrap()));

        let mut paused = machine.clone();
        paused["states"][0]["transitions"]["advance"]["paused"] = json!(true);
        assert!(!item.matches(&target(&paused, 0, "advance").unwrap()));

        let mut relinked = machine.clone();
        relinked["tracks"][1]["metadata"]["uri"] = json!("spotify:track:other");
        assert!(!item.matches(&target(&relinked, 0, "advance").unwrap()));

        let mut extended = machine;
        extended["states"][1]["transitions"]["advance"] = json!({"state_index": 0});
        assert!(item.matches(&target(&extended, 0, "advance").unwrap()));
    }

    #[test]
    fn picks_the_highest_fairplay_file() {
        let machine = machine();
        let file = choose_file(track_at(&machine, 0).unwrap()).unwrap();
        assert_eq!(file.id, "ab");
        assert_eq!(file.format_number, "15");
        assert_eq!(file.bitrate, 256_000);
        let file = choose_file(track_at(&machine, 1).unwrap()).unwrap();
        assert_eq!(file.format_number, "14");
        assert_eq!(choose_file(track_at(&machine, 2).unwrap()), None);
        assert_eq!(choose_file(track_at(&machine, 3).unwrap()), None);
    }

    #[test]
    fn picks_the_join_from_the_settings_and_the_items() {
        let gapless = AudioSettings::default();
        let neither = AudioSettings {
            gapless: false,
            ..AudioSettings::default()
        };
        let fade = AudioSettings {
            gapless: true,
            crossfade: true,
            crossfade_ms: 6000,
            ..AudioSettings::default()
        };
        let fade_only = AudioSettings {
            gapless: false,
            ..fade
        };
        let song = ("spotify:track:a", 200_000);
        let next = ("spotify:track:b", 180_000);
        let short = ("spotify:track:c", 12_999);
        let episode = ("spotify:episode:d", 3_600_000);

        assert_eq!(join_for(&gapless, song, next), Some(Join::Gapless));
        assert_eq!(join_for(&neither, song, next), None);
        assert_eq!(join_for(&gapless, episode, next), Some(Join::Gapless));

        assert_eq!(join_for(&fade, song, next), Some(Join::Crossfade));
        assert_eq!(
            join_for(&fade, song, ("spotify:track:c", 13_000)),
            Some(Join::Crossfade)
        );
        assert_eq!(join_for(&fade, song, short), Some(Join::Gapless));
        assert_eq!(join_for(&fade, short, next), Some(Join::Gapless));
        assert_eq!(join_for(&fade, episode, next), Some(Join::Gapless));
        assert_eq!(join_for(&fade, song, episode), Some(Join::Gapless));
        assert_eq!(join_for(&fade_only, song, next), Some(Join::Crossfade));
        assert_eq!(join_for(&fade_only, song, episode), None);
        assert_eq!(join_for(&fade_only, short, next), None);

        let no_overlap = AudioSettings {
            crossfade_ms: 0,
            ..fade_only
        };
        assert_eq!(join_for(&no_overlap, song, next), Some(Join::Gapless));
        assert_eq!(join_for(&no_overlap, episode, next), Some(Join::Gapless));
    }

    #[test]
    fn starts_the_fade_inside_the_window_only() {
        assert_eq!(crossfade_length(6000, 7000), None);
        assert_eq!(crossfade_length(6000, 6000), Some(6000));
        assert_eq!(crossfade_length(6000, 5800), Some(5800));
        assert_eq!(crossfade_length(6000, 3000), Some(3000));
        assert_eq!(crossfade_length(6000, 2999), None);
        assert_eq!(crossfade_length(6000, 0), None);
        assert_eq!(crossfade_length(0, 0), None);
        assert_eq!(crossfade_length(0, 100), None);
    }
}
