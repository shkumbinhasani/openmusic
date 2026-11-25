use std::time::{Duration, SystemTime};

use anyhow::{Result, bail};

use crate::{connect::SleepState, engine::Engine};

pub const FADE_MS: u32 = 3000;
pub const MAX_MINUTES: u32 = 24 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepChoice {
    Off,
    Minutes(u32),
    EndOfTrack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Due {
    Wait,
    Fade(u32),
    Stop,
}

pub(super) fn due(state: SleepState, fading: bool, now: SystemTime) -> Due {
    let SleepState::At(deadline) = state else {
        return Due::Wait;
    };
    match deadline.duration_since(now) {
        Ok(left) if left.is_zero() => Due::Stop,
        Ok(left) if left > Duration::from_millis(u64::from(FADE_MS)) || fading => Due::Wait,
        Ok(left) => Due::Fade(left.as_millis() as u32),
        Err(_) => Due::Stop,
    }
}

pub(super) fn deadline(choice: SleepChoice, now: SystemTime) -> SleepState {
    match choice {
        SleepChoice::Off => SleepState::Off,
        SleepChoice::EndOfTrack => SleepState::EndOfTrack,
        SleepChoice::Minutes(0) => SleepState::Off,
        SleepChoice::Minutes(minutes) => {
            SleepState::At(now + Duration::from_secs(u64::from(minutes.min(MAX_MINUTES)) * 60))
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Sleep {
    state: SleepState,
    fading: bool,
}

impl Sleep {
    pub(super) fn state(&self) -> SleepState {
        self.state
    }

    pub(super) fn ends_track(&self) -> bool {
        self.state == SleepState::EndOfTrack
    }

    pub(super) fn fading(&self) -> bool {
        self.fading
    }

    pub(super) fn set(&mut self, state: SleepState) {
        self.state = state;
        self.fading = false;
    }

    pub(super) fn clear(&mut self) {
        self.set(SleepState::Off);
    }

    pub(super) fn expired(&self, now: SystemTime) -> bool {
        matches!(self.state, SleepState::At(deadline) if deadline <= now)
    }
}

impl Engine {
    pub(super) fn set_sleep(&mut self, choice: SleepChoice) -> Result<()> {
        if choice != SleepChoice::Off && !self.local_active() && self.remote_target().is_some() {
            bail!("The sleep timer works when music plays on this Mac.");
        }
        if self.audio.sleep.fading() {
            self.player.reset_fade();
        }
        self.audio.sleep.set(deadline(choice, SystemTime::now()));
        self.check_prepared();
        Ok(())
    }

    pub(super) fn sleep_due(&mut self, playing: bool) -> bool {
        let now = SystemTime::now();
        if !playing {
            if self.audio.sleep.expired(now) {
                self.end_sleep();
            }
            return false;
        }
        match due(self.audio.sleep.state(), self.audio.sleep.fading(), now) {
            Due::Wait => false,
            Due::Fade(ms) => {
                self.audio.sleep.fading = true;
                self.player.fade_out(ms);
                false
            }
            Due::Stop => {
                self.audio.sleep.clear();
                true
            }
        }
    }

    pub(super) fn end_sleep(&mut self) {
        if self.audio.sleep.fading() {
            self.player.reset_fade();
        }
        self.audio.sleep.clear();
    }

    pub(in crate::engine) fn local_sleep(&self) -> SleepState {
        self.audio.sleep.state()
    }

    pub(super) fn sleep_after_track(&mut self) -> bool {
        if self.audio.sleep.ends_track() {
            self.audio.sleep.clear();
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000 + seconds)
    }

    #[test]
    fn sets_a_wall_clock_deadline() {
        assert_eq!(
            deadline(SleepChoice::Minutes(30), at(0)),
            SleepState::At(at(1800))
        );
        assert_eq!(
            deadline(SleepChoice::Minutes(100_000), at(0)),
            SleepState::At(at(u64::from(MAX_MINUTES) * 60))
        );
        assert_eq!(deadline(SleepChoice::Minutes(0), at(0)), SleepState::Off);
        assert_eq!(deadline(SleepChoice::Off, at(0)), SleepState::Off);
        assert_eq!(
            deadline(SleepChoice::EndOfTrack, at(0)),
            SleepState::EndOfTrack
        );
    }

    #[test]
    fn fades_for_the_last_three_seconds_then_stops() {
        let state = SleepState::At(at(300));
        assert_eq!(due(state, false, at(0)), Due::Wait);
        assert_eq!(due(state, false, at(296)), Due::Wait);
        assert_eq!(due(state, false, at(297)), Due::Fade(3000));
        assert_eq!(due(state, true, at(298)), Due::Wait);
        assert_eq!(due(state, true, at(300)), Due::Stop);
        assert_eq!(due(state, false, at(301)), Due::Stop);
        assert_eq!(due(SleepState::Off, false, at(301)), Due::Wait);
        assert_eq!(due(SleepState::EndOfTrack, false, at(301)), Due::Wait);
    }

    #[test]
    fn survives_clock_jumps() {
        let state = deadline(SleepChoice::Minutes(10), at(0));
        assert_eq!(due(state, false, at(3600)), Due::Stop);
        let backwards = SystemTime::UNIX_EPOCH + Duration::from_secs(1_799_996_400);
        assert_eq!(due(state, false, backwards), Due::Wait);
        let mut sleep = Sleep::default();
        sleep.set(state);
        assert!(!sleep.expired(at(599)));
        assert!(sleep.expired(at(600)));
        assert!(sleep.expired(at(86_400)));
    }

    #[test]
    fn clears_and_ends_the_track_once() {
        let mut sleep = Sleep::default();
        assert_eq!(sleep.state(), SleepState::Off);
        sleep.set(SleepState::EndOfTrack);
        assert!(sleep.ends_track());
        sleep.fading = true;
        sleep.clear();
        assert!(!sleep.ends_track() && !sleep.fading());
        assert!(!sleep.expired(at(0)));
    }
}
