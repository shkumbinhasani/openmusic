use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use super::Update;
use crate::{
    catalog::{LocalTrack, Track},
    engine::{Control, Engine, GAIA_VOLUME, NowPlaying},
    player::{Media, PlayerEvent, Source},
};

const RESTART_MS: u32 = 3000;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LocalQueue {
    pub(super) tracks: Vec<LocalTrack>,
    pub(super) index: usize,
    pub(super) generation: u64,
    pub(super) paused: bool,
    pub(super) position_ms: u32,
    pub(super) duration_ms: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Step {
    pub(super) index: usize,
    pub(super) play: bool,
}

impl LocalQueue {
    pub(super) fn new(tracks: Vec<LocalTrack>, index: usize) -> Option<Self> {
        (index < tracks.len()).then_some(Self {
            tracks,
            index,
            generation: 0,
            paused: false,
            position_ms: 0,
            duration_ms: 0,
        })
    }

    pub(super) fn current(&self) -> Option<&LocalTrack> {
        self.tracks.get(self.index)
    }

    pub(super) fn at_end(&self) -> bool {
        self.index + 1 >= self.tracks.len()
    }

    pub(super) fn step(&self, forward: bool) -> Step {
        if forward {
            if self.at_end() {
                Step {
                    index: 0,
                    play: false,
                }
            } else {
                Step {
                    index: self.index + 1,
                    play: true,
                }
            }
        } else {
            Step {
                index: self.index.saturating_sub(1),
                play: true,
            }
        }
    }

    pub(super) fn describe(&self) -> String {
        format!(
            "index={}/{} generation={} paused={} position={} duration={} path={}",
            self.index + 1,
            self.tracks.len(),
            self.generation,
            self.paused,
            self.position_ms,
            self.duration_ms,
            self.current()
                .map(|track| track.path.display().to_string())
                .unwrap_or_default()
        )
    }

    pub(super) fn restarts(&self) -> bool {
        self.position_ms > RESTART_MS || self.index == 0
    }

    pub(super) fn track(&self) -> Option<Track> {
        let local = self.current()?;
        Some(Track {
            uri: local.uri.clone(),
            title: local.title.clone(),
            artist: local.artist.clone(),
            album: local.album.clone(),
            duration_ms: if self.duration_ms > 0 {
                self.duration_ms
            } else {
                local.duration_ms
            },
            ..Track::default()
        })
    }
}

impl Engine {
    pub(super) fn local_active(&self) -> bool {
        self.audio.local.is_some()
    }

    pub(super) async fn play_local(&mut self, tracks: Vec<LocalTrack>, index: usize) -> Result<()> {
        let Some(queue) = LocalQueue::new(tracks, index) else {
            bail!("Choose a file to play.");
        };
        self.discard_prepared();
        self.pause_local().await;
        self.pause_elsewhere().await;
        self.audio.unloaded = self.playing.is_some();
        self.audio.local = Some(queue);
        self.load_local(true);
        Ok(())
    }

    pub(super) async fn reload(&mut self) {
        self.end_local();
        let Some(playing) = self.playing.as_ref() else {
            return;
        };
        let (machine, index, position) =
            (playing.machine.clone(), playing.index, playing.position_ms);
        if let Err(error) = self
            .start_state(machine, index, false, Some(position))
            .await
        {
            self.stop_local();
            self.report("error").await;
            self.audio.updates.push(Update::Notice(error.to_string()));
        }
    }

    fn load_local(&mut self, play: bool) {
        self.generation += 1;
        let generation = self.generation;
        let Some(queue) = self.audio.local.as_mut() else {
            return;
        };
        let Some(path) = queue.current().map(|track| track.path.clone()) else {
            return;
        };
        queue.generation = generation;
        queue.paused = !play;
        queue.position_ms = 0;
        queue.duration_ms = 0;
        self.player.set_volume(f32::from(self.volume) / GAIA_VOLUME);
        self.player.load(
            generation,
            Media {
                play,
                ..Media::new(Source::File(path))
            },
        );
    }

    fn go_local(&mut self, step: Step) {
        if let Some(queue) = self.audio.local.as_mut() {
            queue.index = step.index;
        }
        self.load_local(step.play);
    }

    fn finish_local(&mut self) {
        let Some(queue) = self.audio.local.as_ref() else {
            return;
        };
        if queue.at_end() && self.audio.unloaded {
            self.end_local();
            return;
        }
        let step = queue.step(true);
        let play = step.play && !self.sleep_after_track();
        self.go_local(Step { play, ..step });
    }

    pub(super) fn end_local(&mut self) {
        if self.audio.local.take().is_some() {
            self.player.stop();
        }
    }

    pub(in crate::engine) async fn local_control(&mut self, control: Control) -> Option<Control> {
        let Some(queue) = self.audio.local.as_mut() else {
            let resume = matches!(control, Control::SetPlaying(true))
                && self.audio.unloaded
                && self.playing.is_some()
                && self.remote_target().is_none();
            if resume {
                self.reload().await;
                return None;
            }
            return Some(control);
        };
        match control {
            Control::SetPlaying(play) => {
                if queue.paused == play {
                    queue.paused = !play;
                    if play {
                        if self.audio.sleep.expired(std::time::SystemTime::now()) {
                            self.end_sleep();
                        }
                        self.player.play();
                    } else {
                        self.player.pause();
                    }
                }
                None
            }
            Control::Next => {
                let step = queue.step(true);
                self.go_local(step);
                None
            }
            Control::Previous => {
                if queue.restarts() {
                    queue.position_ms = 0;
                    self.player.seek(0);
                } else {
                    let step = queue.step(false);
                    self.go_local(step);
                }
                None
            }
            Control::Seek(position) => {
                queue.position_ms = position;
                self.player.seek(position);
                None
            }
            Control::Volume(volume) => {
                self.volume = volume;
                self.player.set_volume(f32::from(volume) / GAIA_VOLUME);
                self.volume_due = Some(tokio::time::Instant::now() + Duration::from_secs(1));
                None
            }
            Control::PlayTracks { .. } | Control::PlayContext { .. } | Control::PlayHere => {
                self.end_local();
                Some(control)
            }
            control => Some(control),
        }
    }

    pub(super) async fn local_event(&mut self, event: &PlayerEvent) -> bool {
        let Some(queue) = self.audio.local.as_mut() else {
            return false;
        };
        let current = queue.generation;
        match event {
            PlayerEvent::Ready {
                generation,
                duration_ms,
            } if *generation == current => {
                if *duration_ms > 0 {
                    queue.duration_ms = *duration_ms;
                }
            }
            PlayerEvent::Position {
                generation,
                position_ms,
                playing,
                ..
            } if *generation == current => {
                queue.position_ms = *position_ms;
                let sounding = *playing && !queue.paused;
                if self.sleep_due(sounding) {
                    if let Some(queue) = self.audio.local.as_mut() {
                        queue.paused = true;
                    }
                    self.player.pause();
                    self.player.reset_fade();
                }
            }
            PlayerEvent::Ended { generation } if *generation == current => self.finish_local(),
            PlayerEvent::Advanced { from, .. } if *from == current => self.finish_local(),
            PlayerEvent::Failed {
                generation,
                message,
            } if *generation == current => {
                let title = queue
                    .current()
                    .map(|track| track.title.clone())
                    .unwrap_or_default();
                self.audio.updates.push(Update::Notice(format!(
                    "OpenMusic couldn’t play “{title}”. {message}"
                )));
                let step = queue.step(true);
                if step.play {
                    self.go_local(step);
                } else {
                    self.end_local();
                }
            }
            _ => return false,
        }
        true
    }

    pub(in crate::engine) fn local_now(&self) -> Option<NowPlaying> {
        let queue = self.audio.local.as_ref()?;
        Some(NowPlaying {
            track: queue.track(),
            playing: !queue.paused,
            position_ms: queue.position_ms,
            observed: Instant::now(),
            device: Some(self.device_name.clone()),
            local: true,
            volume: self.volume,
            sleep: self.audio.sleep.state(),
            ..NowPlaying::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn tracks() -> Vec<LocalTrack> {
        ["One", "Two", "Three"]
            .into_iter()
            .enumerate()
            .map(|(index, title)| LocalTrack {
                path: PathBuf::from(format!("/tmp/{index}.m4a")),
                uri: format!("spotify:local:Artist:Album:{title}:180"),
                title: title.into(),
                artist: "Artist".into(),
                album: "Album".into(),
                duration_ms: 180_000,
            })
            .collect()
    }

    #[test]
    fn refuses_an_index_past_the_end() {
        assert!(LocalQueue::new(tracks(), 3).is_none());
        assert!(LocalQueue::new(Vec::new(), 0).is_none());
        assert_eq!(LocalQueue::new(tracks(), 2).unwrap().index, 2);
    }

    #[test]
    fn moves_through_the_files_and_wraps_paused() {
        let mut queue = LocalQueue::new(tracks(), 0).unwrap();
        assert!(!queue.at_end());
        assert_eq!(
            queue.step(true),
            Step {
                index: 1,
                play: true
            }
        );
        assert_eq!(
            queue.step(false),
            Step {
                index: 0,
                play: true
            }
        );
        queue.index = 2;
        assert!(queue.at_end());
        assert_eq!(
            queue.step(true),
            Step {
                index: 0,
                play: false
            }
        );
        assert_eq!(
            queue.step(false),
            Step {
                index: 1,
                play: true
            }
        );
    }

    #[test]
    fn restarts_the_file_after_three_seconds() {
        let mut queue = LocalQueue::new(tracks(), 1).unwrap();
        queue.position_ms = 2_000;
        assert!(!queue.restarts());
        queue.position_ms = 3_001;
        assert!(queue.restarts());
        queue.index = 0;
        queue.position_ms = 0;
        assert!(queue.restarts());
    }

    #[test]
    fn describes_the_queue_for_the_report() {
        let queue = LocalQueue::new(tracks(), 1).unwrap();
        assert_eq!(
            queue.describe(),
            "index=2/3 generation=0 paused=false position=0 duration=0 path=/tmp/1.m4a"
        );
    }

    #[test]
    fn shows_the_file_as_a_track() {
        let mut queue = LocalQueue::new(tracks(), 1).unwrap();
        let track = queue.track().unwrap();
        assert_eq!(track.title, "Two");
        assert_eq!(track.uri, "spotify:local:Artist:Album:Two:180");
        assert_eq!(track.duration_ms, 180_000);
        assert!(track.cover.is_none());
        queue.duration_ms = 179_512;
        assert_eq!(queue.track().unwrap().duration_ms, 179_512);
    }
}
