use std::{
    collections::{HashMap, VecDeque},
    fmt::Write as _,
    time::{Duration, Instant},
};

use spotify_client::{
    canvas::{self, Canvas},
    engine::NowPlaying,
};

use super::{video, worker::Command};
use crate::app::OpenMusic;

const KEEP: usize = 100;
const RETRY_AFTER: Duration = Duration::from_secs(60);
const ANSWER_WITHIN: Duration = Duration::from_secs(20);

#[derive(Default)]
pub struct Canvases {
    found: HashMap<String, Option<Canvas>>,
    order: VecDeque<String>,
    failed: Option<(String, Instant)>,
    pending: Option<(String, Instant)>,
    pub preview: Option<String>,
    pub reduce_motion: bool,
    pub covered: bool,
}

impl Canvases {
    pub fn ask(&mut self, uri: String, at: Instant) {
        self.pending = Some((uri, at));
    }

    pub fn asking(&self, uri: &str, now: Instant) -> bool {
        self.pending.as_ref().is_some_and(|(pending, at)| {
            pending == uri && now.saturating_duration_since(*at) < ANSWER_WITHIN
        })
    }

    pub fn answered(&mut self, uri: &str) {
        if self
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| pending == uri)
        {
            self.pending = None;
        }
    }

    pub fn known(&self, uri: &str) -> Option<Option<&Canvas>> {
        self.found.get(uri).map(Option::as_ref)
    }

    pub fn store(&mut self, uri: String, canvas: Option<Canvas>) {
        if self.found.insert(uri.clone(), canvas).is_none() {
            self.order.push_back(uri);
        }
        while self.order.len() > KEEP {
            if let Some(oldest) = self.order.pop_front() {
                self.found.remove(&oldest);
            }
        }
    }

    pub fn fail(&mut self, uri: String, at: Instant) {
        self.failed = Some((uri, at));
    }

    pub fn should_ask(&self, uri: &str, now: Instant) -> bool {
        !self.found.contains_key(uri)
            && !self.asking(uri, now)
            && !self.failed.as_ref().is_some_and(|(failed, at)| {
                failed == uri && now.saturating_duration_since(*at) < RETRY_AFTER
            })
    }

    pub fn len(&self) -> usize {
        self.found.len()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conditions {
    pub enabled: bool,
    pub reduce_motion: bool,
    pub visible: bool,
    pub explicit_filtered: bool,
    pub screen: bool,
}

impl Conditions {
    pub fn moving(self) -> bool {
        self.enabled && !self.reduce_motion
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    pub track: String,
    pub url: String,
    pub random: bool,
    pub playing: bool,
}

pub fn want(
    track: &str,
    playing: bool,
    canvas: Option<&Canvas>,
    conditions: Conditions,
) -> Option<Want> {
    if !conditions.moving() || conditions.screen {
        return None;
    }
    let canvas = canvas.filter(|canvas| {
        canvas.track_uri == track
            && canvas.kind.moving()
            && !(canvas.explicit && conditions.explicit_filtered)
    })?;
    Some(Want {
        track: track.to_owned(),
        url: canvas.url.clone(),
        random: canvas.kind.random_start(),
        playing: playing && conditions.visible,
    })
}

impl OpenMusic {
    fn canvas_subject(&self) -> Option<(String, bool)> {
        if let Some(uri) = &self.content.canvas.preview {
            return Some((uri.clone(), true));
        }
        self.track()
            .filter(|track| canvas::is_track(&track.uri))
            .map(|track| (track.uri.clone(), self.playback.now.playing))
    }

    fn canvas_conditions(&self) -> Conditions {
        Conditions {
            enabled: self.settings.content.canvas,
            reduce_motion: self.content.canvas.reduce_motion,
            visible: !self.content.canvas.covered,
            explicit_filtered: self
                .account()
                .is_some_and(|account| account.explicit_filtered),
            screen: self.video_on(),
        }
    }

    fn apply_canvas(&mut self, subject: Option<(String, bool)>) {
        let conditions = self.canvas_conditions();
        let Some((track, playing)) = subject else {
            video::sync(None);
            return;
        };
        let now = Instant::now();
        if conditions.moving() && self.content.canvas.should_ask(&track, now) {
            self.content.canvas.ask(track.clone(), now);
            self.send(Command::Canvas(track.clone()));
        }
        let canvas = self.content.canvas.known(&track).flatten();
        video::sync(want(&track, playing, canvas, conditions).as_ref());
    }

    pub fn canvas_want(&self) -> Option<Want> {
        let (track, playing) = self.canvas_subject()?;
        let canvas = self.content.canvas.known(&track).flatten();
        want(&track, playing, canvas, self.canvas_conditions())
    }

    pub fn canvas_tick(&mut self) {
        self.content.canvas.reduce_motion = video::reduce_motion();
        self.content.canvas.covered = !video::window_visible();
        self.apply_canvas(self.canvas_subject());
    }

    pub fn canvas_now(&mut self, now: &NowPlaying) {
        if self.content.canvas.preview.is_some() {
            return;
        }
        let subject = now
            .track
            .as_ref()
            .filter(|track| canvas::is_track(&track.uri))
            .map(|track| (track.uri.clone(), now.playing));
        self.apply_canvas(subject);
    }

    pub fn took_canvas(&mut self, uri: String, result: Result<Option<Canvas>, String>) {
        self.content.canvas.answered(&uri);
        match result {
            Ok(canvas) => self.content.canvas.store(uri, canvas),
            Err(_) => self.content.canvas.fail(uri, Instant::now()),
        }
        self.apply_canvas(self.canvas_subject());
    }

    pub fn set_canvas(&mut self, enabled: bool) {
        self.update_settings(|settings| settings.content.canvas = enabled);
        self.apply_canvas(self.canvas_subject());
    }

    pub fn preview_canvas(&mut self, uri: Option<String>) {
        self.content.canvas.preview = uri;
        self.apply_canvas(self.canvas_subject());
    }

    pub fn canvas_status(&self) -> Option<&'static str> {
        let (track, _) = self.canvas_subject()?;
        let conditions = self.canvas_conditions();
        if !conditions.enabled {
            return Some("off");
        }
        if conditions.reduce_motion {
            return Some("still");
        }
        if conditions.screen {
            return Some("video");
        }
        let canvases = &self.content.canvas;
        let now = Instant::now();
        Some(match canvases.known(&track) {
            None if canvases.asking(&track, now) => "asking",
            None if canvases.should_ask(&track, now) => "waiting",
            None => "failed",
            Some(None) => "none",
            Some(Some(canvas)) if !canvas.kind.moving() => "image",
            Some(Some(canvas)) if canvas.explicit && conditions.explicit_filtered => "filtered",
            Some(Some(_)) => video::status().name(),
        })
    }

    pub fn describe_canvas(&self, lines: &mut String) {
        let conditions = self.canvas_conditions();
        let canvases = &self.content.canvas;
        let _ = writeln!(
            lines,
            "setting={} reduce_motion={} visible={} explicit_filtered={} screen={} cached={}",
            conditions.enabled,
            conditions.reduce_motion,
            conditions.visible,
            conditions.explicit_filtered,
            conditions.screen,
            canvases.len(),
        );
        match self.canvas_subject() {
            Some((track, playing)) => {
                let _ = writeln!(
                    lines,
                    "track {track} playing={playing} preview={} status={}",
                    canvases.preview.is_some(),
                    self.canvas_status().unwrap_or("-"),
                );
                match canvases.known(&track) {
                    Some(Some(canvas)) => {
                        let _ = writeln!(
                            lines,
                            "canvas {} explicit={} uri={:?} artist={:?} url={}",
                            canvas.kind.name(),
                            canvas.explicit,
                            canvas.uri,
                            canvas.artist,
                            canvas.url,
                        );
                    }
                    Some(None) => lines.push_str("canvas none\n"),
                    None => {
                        let _ = writeln!(
                            lines,
                            "canvas unknown asking={}",
                            canvases.asking(&track, Instant::now())
                        );
                    }
                }
                let canvas = canvases.known(&track).flatten();
                match want(&track, playing, canvas, conditions) {
                    Some(want) => {
                        let _ = writeln!(
                            lines,
                            "want random={} playing={} url={}",
                            want.random, want.playing, want.url
                        );
                    }
                    None => lines.push_str("want none\n"),
                }
            }
            None => lines.push_str("track none\n"),
        }
        lines.push_str(&video::describe());
        lines.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use spotify_client::canvas::Kind;

    use super::*;

    const TRACK: &str = "spotify:track:00000000000000000000t1";
    const URL: &str = "https://canvaz.scdn.co/upload/artist/0000/video/0000.cnvs.mp4";

    fn canvas(kind: Kind, explicit: bool) -> Canvas {
        Canvas {
            track_uri: TRACK.into(),
            url: URL.into(),
            kind,
            explicit,
            uri: None,
            artist: None,
        }
    }

    fn open() -> Conditions {
        Conditions {
            enabled: true,
            reduce_motion: false,
            visible: true,
            explicit_filtered: false,
            screen: false,
        }
    }

    #[test]
    fn loops_moving_canvases_while_music_plays() {
        let looping = canvas(Kind::LoopingRandom, false);
        assert_eq!(
            want(TRACK, true, Some(&looping), open()),
            Some(Want {
                track: TRACK.into(),
                url: URL.into(),
                random: true,
                playing: true,
            })
        );
        let paused = want(TRACK, false, Some(&looping), open()).unwrap();
        assert!(!paused.playing);
        let covered = Conditions {
            visible: false,
            ..open()
        };
        assert!(!want(TRACK, true, Some(&looping), covered).unwrap().playing);
        assert!(
            !want(TRACK, true, Some(&canvas(Kind::Video, false)), open())
                .unwrap()
                .random
        );
    }

    #[test]
    fn keeps_the_cover_when_it_should() {
        let looping = canvas(Kind::Looping, false);
        let off = Conditions {
            enabled: false,
            ..open()
        };
        let still = Conditions {
            reduce_motion: true,
            ..open()
        };
        assert_eq!(want(TRACK, true, Some(&looping), off), None);
        assert_eq!(want(TRACK, true, Some(&looping), still), None);
        assert_eq!(want(TRACK, true, None, open()), None);
        assert_eq!(
            want(TRACK, true, Some(&canvas(Kind::Image, false)), open()),
            None
        );
        assert_eq!(
            want(TRACK, true, Some(&canvas(Kind::Gif, false)), open()),
            None
        );
        assert_eq!(
            want(
                "spotify:track:00000000000000000000t2",
                true,
                Some(&looping),
                open()
            ),
            None
        );
        let explicit = canvas(Kind::Looping, true);
        assert!(want(TRACK, true, Some(&explicit), open()).is_some());
        let filtered = Conditions {
            explicit_filtered: true,
            ..open()
        };
        assert_eq!(want(TRACK, true, Some(&explicit), filtered), None);
        assert!(want(TRACK, true, Some(&looping), filtered).is_some());
        let video = Conditions {
            screen: true,
            ..open()
        };
        assert_eq!(want(TRACK, true, Some(&looping), video), None);
    }

    #[test]
    fn asks_once_per_track_and_retries_failures_later() {
        let mut canvases = Canvases::default();
        let now = Instant::now();
        assert!(canvases.should_ask(TRACK, now));
        canvases.ask(TRACK.into(), now);
        assert!(canvases.asking(TRACK, now));
        assert!(!canvases.should_ask(TRACK, now));
        assert!(canvases.should_ask(TRACK, now + ANSWER_WITHIN));
        canvases.answered("spotify:track:00000000000000000000t2");
        assert!(canvases.asking(TRACK, now));
        canvases.answered(TRACK);
        assert!(!canvases.asking(TRACK, now));
        canvases.fail(TRACK.into(), now);
        assert!(!canvases.should_ask(TRACK, now + Duration::from_secs(5)));
        assert!(canvases.should_ask(TRACK, now + RETRY_AFTER));
        canvases.store(TRACK.into(), None);
        assert!(!canvases.should_ask(TRACK, now + RETRY_AFTER));
        assert_eq!(canvases.known(TRACK), Some(None));
    }

    #[test]
    fn forgets_the_oldest_tracks() {
        let mut canvases = Canvases::default();
        for index in 0..KEEP + 5 {
            canvases.store(format!("spotify:track:{index}"), None);
        }
        assert_eq!(canvases.len(), KEEP);
        assert_eq!(canvases.known("spotify:track:0"), None);
        assert_eq!(
            canvases.known(&format!("spotify:track:{}", KEEP + 4)),
            Some(None)
        );
        canvases.store(format!("spotify:track:{}", KEEP + 4), None);
        assert_eq!(canvases.len(), KEEP);
    }
}
