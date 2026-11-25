use std::time::{Duration, Instant};

use serde_json::Value;

use super::{
    AudioSettings, Ending, Playing, ReportPrepared,
    gain::{fetch_loudness, linear},
    mix::blend_at,
    narration::is_narration,
    quality::{choose, reported_quality},
    source::Pick,
    speed::{show_of, starting_speed, state_speed},
    video::{Opened, VideoStatus, offer},
};
use crate::{
    automix::Plan,
    engine::Engine,
    player::{Blend, Join, Media},
};

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
    if track_at(machine, target.index).is_none_or(is_narration) {
        return false;
    }
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
pub(super) struct Wanted {
    pub(super) target: Target,
    pub(super) join: Join,
    pub(super) mix: Option<Plan>,
}

impl Wanted {
    pub(super) fn due_ms(&self, settings: &AudioSettings, duration_ms: u32) -> u32 {
        match &self.mix {
            Some(plan) => plan.at_ms,
            None => duration_ms.saturating_sub(if settings.crossfade {
                settings.crossfade_ms
            } else {
                0
            }),
        }
    }

    pub(super) fn start_ms(&self) -> u32 {
        self.mix.as_ref().map_or(0, |plan| plan.start_b_ms)
    }
}

pub(super) fn blend_for(
    mix: Option<&Plan>,
    overlap_ms: u32,
    position_ms: u32,
    duration_ms: u32,
) -> Option<Blend> {
    match mix {
        Some(plan) => blend_at(plan, position_ms),
        None => {
            crossfade_length(overlap_ms, duration_ms.saturating_sub(position_ms)).map(Blend::fade)
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct File {
    pub(super) id: String,
    pub(super) format: Value,
    pub(super) format_number: String,
    pub(super) bitrate: u64,
    pub(super) quality: Option<String>,
}

pub(super) fn choose_file(track: &Value, ceiling: Option<u64>) -> Option<File> {
    let files = track["manifest"]["file_ids_mp4_cbcs"].as_array()?;
    let file = choose(files, ceiling)?;
    Some(File {
        id: file["file_id"].as_str()?.to_owned(),
        format: file["format"].clone(),
        format_number: file["format"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| file["format"].to_string()),
        bitrate: file["bitrate"].as_u64().unwrap_or(128_000),
        quality: reported_quality(file),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Prepared {
    pub(super) generation: u64,
    target: Target,
    pub(super) join: Join,
    pick: Pick,
    bitrate: u64,
    duration_ms: u32,
    pub(super) ready: bool,
    pub(super) gain_db: Option<f32>,
    speed: f32,
    pub(super) mix: Option<Plan>,
    start_ms: u32,
    video: Option<VideoStatus>,
}

impl Prepared {
    pub(super) fn matches(&self, target: &Target) -> bool {
        !target.paused && self.target.state_id == target.state_id && self.target.uri == target.uri
    }

    fn wanted(&self, wanted: &Wanted) -> bool {
        self.matches(&wanted.target) && self.join == wanted.join && self.mix == wanted.mix
    }

    pub(super) fn index(&self) -> usize {
        self.target.index
    }

    pub(super) fn report(&self) -> ReportPrepared {
        ReportPrepared {
            generation: self.generation,
            uri: self.target.uri.clone(),
            join: self.join,
            ready: self.ready,
            gain_db: self.gain_db,
            mix: self.mix.as_ref().map(Plan::describe),
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

    pub(super) fn wanted_join(&self) -> Option<Wanted> {
        if self.audio.sleep.ends_track() || self.ending("advance") != Ending::Continue {
            return None;
        }
        let playing = self.playing.as_ref()?;
        let target = target(&playing.machine, playing.index, "advance")?;
        if !preparable(&playing.machine, &target) {
            return None;
        }
        let next = track_at(&playing.machine, target.index)?;
        if self.skipped(next) {
            return None;
        }
        let next_ms = next["metadata"]["duration"].as_u64().unwrap_or_default() as u32;
        if let Some(plan) = self.mix_plan(playing, &target, next_ms) {
            return Some(Wanted {
                target,
                join: Join::Crossfade,
                mix: Some(plan),
            });
        }
        let join = join_for(
            &self.audio.settings,
            (playing.uri(), playing.duration_ms),
            (&target.uri, next_ms),
        )?;
        Some(Wanted {
            target,
            join,
            mix: None,
        })
    }

    fn prepared_file_current(&self, prepared: &Prepared) -> bool {
        let Pick::FairPlay(file) = &prepared.pick else {
            return true;
        };
        self.playing
            .as_ref()
            .and_then(|playing| track_at(&playing.machine, prepared.index()))
            .and_then(|track| choose_file(track, self.bitrate_ceiling()))
            .is_some_and(|wanted| wanted.id == file.id)
    }

    pub(super) fn check_prepared(&mut self) {
        let Some(prepared) = &self.audio.joins.prepared else {
            return;
        };
        let keep = self
            .wanted_join()
            .is_some_and(|wanted| prepared.wanted(&wanted))
            && self.prepared_file_current(prepared);
        if !keep {
            self.discard_prepared();
            self.audio.joins.tried = None;
        }
    }

    pub(super) async fn prepare_next(&mut self) {
        self.follow_mix();
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
        let source = playing.generation;
        let Some(wanted) = self.wanted_join() else {
            return;
        };
        let due = wanted.due_ms(&self.audio.settings, playing.duration_ms);
        if due.saturating_sub(playing.position_ms) > PREPARE_LEAD_MS {
            return;
        }
        let start_ms = wanted.start_ms();
        let Wanted { target, join, mix } = wanted;
        let Some(track) = track_at(&playing.machine, target.index).cloned() else {
            self.audio.joins.tried = Some(source);
            return;
        };
        let duration_ms = track["metadata"]["duration"].as_u64().unwrap_or_default() as u32;
        let missing = self.audio.gains.missing(&playing.machine);
        let Ok(pick) = self.resolve(&track).await else {
            self.audio.joins.tried = Some(source);
            return;
        };
        if self.playing.as_ref().map(|playing| playing.generation) != Some(source) {
            return;
        }
        self.generation += 1;
        let generation = self.generation;
        let api = self.api.clone();
        let (found, opened) = tokio::join!(
            fetch_loudness(&api, &missing),
            self.open(&pick, &target.uri)
        );
        self.store_loudness(found);
        let Ok(Opened {
            media,
            bitrate,
            video: carried,
        }) = opened
        else {
            self.audio.joins.tried = Some(source);
            return;
        };
        let Some(playing) = self
            .playing
            .as_ref()
            .filter(|playing| playing.generation == source)
        else {
            return;
        };
        let gain_db = self.gain_at(&playing.machine, target.index);
        let show = show_of(&playing.machine, target.index);
        let speed = starting_speed(
            &target.uri,
            state_speed(&playing.machine, target.index),
            self.known_show_speed(show.as_deref()),
        );
        self.player.prepare(
            generation,
            Media {
                gain: linear(gain_db),
                speed,
                start_ms,
                ..media
            },
            join,
        );
        let video = offer(&target.uri, &track, carried);
        self.audio.joins.prepared = Some(Prepared {
            generation,
            target,
            join,
            bitrate: bitrate.max(pick.bitrate()),
            pick,
            duration_ms,
            ready: false,
            gain_db,
            speed,
            mix,
            start_ms,
            video,
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
        let start = if transition == "advance" {
            prepared.start_ms
        } else {
            0
        };
        let mut next = Playing::new(
            playing.machine.clone(),
            target.index,
            target.paused,
            prepared.generation,
            start,
        );
        next.format = prepared.pick.format();
        next.bitrate = prepared.bitrate;
        next.source = prepared.pick.name();
        next.codec = prepared.pick.codec();
        next.audio_quality = prepared.pick.reported_quality(prepared.bitrate);
        next.duration_ms = prepared.duration_ms;
        next.gain_db = prepared.gain_db;
        next.speed = prepared.speed;
        next.state_speed = state_speed(&next.machine, next.index);
        next.video = prepared.video;
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
        if prepared.join != Join::Crossfade
            || !prepared.ready
            || playing.paused
            || playing.duration_ms == 0
        {
            return;
        }
        let Some(blend) = blend_for(
            prepared.mix.as_ref(),
            self.audio.settings.crossfade_ms,
            playing.position_ms,
            playing.duration_ms,
        ) else {
            return;
        };
        let how = match &prepared.mix {
            Some(plan) => format!("automix {}ms {}", blend.left_ms(), plan.shape.name()),
            None => format!("crossfade {}ms", blend.length_ms),
        };
        if let Taken::Joined(generation) = self.take_prepared("advance", &how, false).await {
            let left = Duration::from_millis(u64::from(blend.left_ms()));
            self.player.crossfade(generation, blend);
            self.audio.joins.fading_until = Some(Instant::now() + left);
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
            pick: Pick::FairPlay(File {
                id: "ba".into(),
                format: json!(14),
                format_number: "14".into(),
                bitrate: 128_000,
                quality: Some("HIGH".into()),
            }),
            bitrate: 128_000,
            duration_ms: 180_000,
            ready: false,
            gain_db: None,
            speed: 1.,
            mix: None,
            start_ms: 0,
            video: None,
        }
    }

    fn plan() -> Plan {
        Plan {
            at_ms: 150_000,
            length_ms: 10_000,
            lead_in_ms: 0,
            start_b_ms: 8_000,
            shape: crate::automix::Shape::fade(),
            shortened: false,
        }
    }

    #[test]
    fn keeps_a_prepared_mix_only_while_its_plan_holds() {
        let machine = machine();
        let next = target(&machine, 0, "advance").unwrap();
        let mut item = prepared(next.clone());
        let plain = Wanted {
            target: next.clone(),
            join: Join::Gapless,
            mix: None,
        };
        assert!(item.wanted(&plain));
        let mixed = Wanted {
            target: next,
            join: Join::Crossfade,
            mix: Some(plan()),
        };
        assert!(!item.wanted(&mixed));
        item.join = Join::Crossfade;
        item.mix = Some(plan());
        assert!(item.wanted(&mixed));
        let moved = Wanted {
            mix: Some(Plan {
                at_ms: 151_000,
                ..plan()
            }),
            ..mixed.clone()
        };
        assert!(!item.wanted(&moved));
        assert_eq!(
            item.report().mix.as_deref(),
            Some("at=150000 length=10000 lead_in=0 b_start=8000 shape=fade/fade")
        );
    }

    #[test]
    fn readies_the_next_song_before_the_mix_point() {
        let machine = machine();
        let next = target(&machine, 0, "advance").unwrap();
        let settings = AudioSettings {
            crossfade: true,
            crossfade_ms: 6000,
            ..AudioSettings::default()
        };
        let plain = Wanted {
            target: next.clone(),
            join: Join::Crossfade,
            mix: None,
        };
        assert_eq!(plain.due_ms(&settings, 200_000), 194_000);
        assert_eq!(plain.due_ms(&AudioSettings::default(), 200_000), 200_000);
        assert_eq!(plain.start_ms(), 0);
        let mixed = Wanted {
            target: next,
            join: Join::Crossfade,
            mix: Some(plan()),
        };
        assert_eq!(mixed.due_ms(&settings, 200_000), 150_000);
        assert_eq!(mixed.start_ms(), 8_000);
    }

    #[test]
    fn blends_at_the_mix_point_or_near_the_end() {
        assert_eq!(blend_for(None, 6000, 193_000, 200_000), None);
        assert_eq!(
            blend_for(None, 6000, 194_200, 200_000),
            Some(Blend::fade(5800))
        );
        assert_eq!(blend_for(None, 0, 199_000, 200_000), None);
        let plan = plan();
        assert_eq!(blend_for(Some(&plan), 6000, 149_000, 200_000), None);
        let blend = blend_for(Some(&plan), 6000, 150_300, 200_000).unwrap();
        assert_eq!((blend.length_ms, blend.elapsed_ms), (10_000, 300));
        assert_eq!(blend_for(Some(&plan), 6000, 156_000, 200_000), None);
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
    fn picks_the_fairplay_file_for_the_chosen_quality() {
        let machine = machine();
        let file = choose_file(track_at(&machine, 0).unwrap(), None).unwrap();
        assert_eq!(file.id, "ab");
        assert_eq!(file.format_number, "15");
        assert_eq!(file.bitrate, 256_000);
        assert_eq!(file.quality.as_deref(), Some("VERY_HIGH"));
        let file = choose_file(track_at(&machine, 0).unwrap(), Some(128_000)).unwrap();
        assert_eq!(file.id, "aa");
        assert_eq!(file.format_number, "14");
        let file = choose_file(track_at(&machine, 1).unwrap(), None).unwrap();
        assert_eq!(file.format_number, "14");
        assert_eq!(file.quality.as_deref(), Some("HIGH"));
        assert_eq!(choose_file(track_at(&machine, 2).unwrap(), None), None);
        assert_eq!(choose_file(track_at(&machine, 3).unwrap(), None), None);
    }

    #[test]
    fn never_prepares_a_voice_clip() {
        let mut machine = machine();
        machine["tracks"][1]["metadata"]["uri"] = json!("spotify:media:0000000000000000000001");
        let next = target(&machine, 0, "advance").unwrap();
        assert!(!preparable(&machine, &next));
        machine["tracks"][1]["metadata"]["uri"] = json!("spotify:track:b");
        machine["tracks"][1]["metadata"]["is_narration"] = json!("true");
        assert!(!preparable(&machine, &next));
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
