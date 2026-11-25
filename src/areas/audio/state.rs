use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use gpui::{App, Context, Window};
use serde::{Deserialize, Serialize};
use spotify_client::{
    catalog::LocalTrack,
    engine::playback::AudioSettings,
    playback_settings::{LoudnessLevel, MAX_CROSSFADE_MS, Remote, Setting},
};

use super::worker::{Command, Event};
use crate::{
    app::OpenMusic,
    menus::Handle,
    ui::tracks::{RowDrag, RowDrop},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioPrefs {
    pub gapless: bool,
    pub crossfade: bool,
    pub crossfade_ms: u32,
    pub normalize: bool,
    pub level: LoudnessLevel,
}

impl Default for AudioPrefs {
    fn default() -> Self {
        let settings = AudioSettings::default();
        Self {
            gapless: settings.gapless,
            crossfade: settings.crossfade,
            crossfade_ms: settings.crossfade_ms,
            normalize: settings.normalize,
            level: settings.level,
        }
    }
}

impl AudioPrefs {
    pub fn engine(&self) -> AudioSettings {
        AudioSettings {
            gapless: self.gapless,
            crossfade: self.crossfade,
            crossfade_ms: self.crossfade_ms.min(MAX_CROSSFADE_MS),
            normalize: self.normalize,
            level: self.level,
        }
    }

    pub fn apply(&mut self, setting: Setting) {
        match setting {
            Setting::Gapless(on) => self.gapless = on,
            Setting::Crossfade(on) => self.crossfade = on,
            Setting::CrossfadeMs(ms) => self.crossfade_ms = ms.min(MAX_CROSSFADE_MS),
            Setting::Normalize(on) => self.normalize = on,
            Setting::Level(level) => self.level = level,
            Setting::Speed(_) => {}
        }
    }

    pub fn merge(&mut self, remote: &Remote) {
        if let Some(user) = remote.user {
            self.gapless = user.gapless;
            self.crossfade = user.crossfade;
            self.crossfade_ms = user.crossfade_ms;
        }
        if let Some(device) = remote.device {
            self.normalize = device.normalize;
            self.level = device.level;
        }
    }
}

#[derive(Default)]
pub struct State {
    pub synced: Option<Result<(), String>>,
    pub speeds: HashMap<String, f32>,
    pub report_to: Option<PathBuf>,
}

impl State {
    pub fn covers(&self, _urls: &mut HashSet<String>) {}
}

impl OpenMusic {
    pub fn change_audio(&mut self, settings: Vec<Setting>) {
        self.update_settings(|current| {
            for setting in &settings {
                current.audio.apply(*setting);
            }
        });
        self.send(Command::Write(settings));
    }

    pub fn play_local(&mut self, tracks: Vec<LocalTrack>, index: usize) {
        self.send(Command::PlayLocal { tracks, index });
    }

    pub fn set_gapless(&mut self, on: bool) {
        self.change_audio(vec![Setting::Gapless(on)]);
    }

    pub fn set_normalize(&mut self, on: bool) {
        self.change_audio(vec![Setting::Normalize(on)]);
    }

    pub fn set_loudness_level(&mut self, level: LoudnessLevel) {
        self.change_audio(vec![Setting::Normalize(true), Setting::Level(level)]);
    }

    pub fn set_crossfade(&mut self, on: bool, ms: u32) {
        self.change_audio(vec![
            Setting::Crossfade(on),
            Setting::CrossfadeMs(ms.min(MAX_CROSSFADE_MS)),
        ]);
    }

    fn apply_audio(&mut self, settings: &[Setting]) {
        self.update_settings(|current| {
            for setting in settings {
                current.audio.apply(*setting);
            }
        });
    }

    fn audio_lines(&self) -> Vec<String> {
        let prefs = &self.settings.audio;
        let mut lines = vec![
            format!(
                "prefs gapless={} crossfade={} crossfade_ms={} normalize={} level={}",
                prefs.gapless,
                prefs.crossfade,
                prefs.crossfade_ms,
                prefs.normalize,
                prefs.level.name()
            ),
            format!(
                "synced {}",
                match &self.audio.synced {
                    None => "pending".to_owned(),
                    Some(Ok(())) => "ok".to_owned(),
                    Some(Err(message)) => format!("failed {message}"),
                }
            ),
        ];
        let mut speeds: Vec<_> = self.audio.speeds.iter().collect();
        speeds.sort_by(|a, b| a.0.cmp(b.0));
        lines.extend(
            speeds
                .into_iter()
                .map(|(show, speed)| format!("speed {show} {speed}")),
        );
        lines
    }
}

pub fn on_event(
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::Synced(Ok(remote)) => {
            app.audio.synced = Some(Ok(()));
            app.audio.speeds.extend(remote.speeds.clone());
            app.update_settings(|current| current.audio.merge(&remote));
        }
        Event::Synced(Err(message)) => app.audio.synced = Some(Err(message)),
        Event::Changed(settings) | Event::Written(settings) => app.apply_audio(&settings),
        Event::Content {
            content_id,
            settings,
        } => {
            for setting in settings {
                if let Setting::Speed(speed) = setting {
                    app.audio.speeds.insert(content_id.clone(), speed);
                }
            }
        }
        Event::WriteFailed { message, remote } => {
            if let Some(remote) = remote {
                app.update_settings(|current| current.audio.merge(&remote));
            }
            app.show_toast(message, None);
        }
        Event::Report(report) => {
            if let Some(directory) = app.audio.report_to.take() {
                let mut text = report.to_string();
                for line in app.audio_lines() {
                    text.push_str(&line);
                    text.push('\n');
                }
                let _ = std::fs::write(directory.join("audio.txt"), text);
            }
        }
    }
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {
    app.send(Command::Sync);
}

pub fn signed_out(app: &mut OpenMusic) {
    app.audio = State::default();
}

fn debug_local_track(path: &str) -> LocalTrack {
    let path = PathBuf::from(path.trim());
    let title = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part = title.replace(':', " ").replace(' ', "+");
    LocalTrack {
        uri: format!("spotify:local:::{part}:0"),
        path,
        title,
        ..LocalTrack::default()
    }
}

fn switch(text: &str) -> Option<bool> {
    match text {
        "on" | "true" => Some(true),
        "off" | "false" => Some(false),
        _ => None,
    }
}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    match verb {
        "audio" if rest == "sync" => app.send(Command::Sync),
        "audio" => {
            app.audio.report_to = Some(directory.to_owned());
            app.send(Command::Report);
        }
        "gapless" => {
            if let Some(on) = switch(rest) {
                app.set_gapless(on);
            }
        }
        "crossfade" => {
            let ms = app.settings.audio.crossfade_ms;
            match (switch(rest), rest.parse::<u32>()) {
                (Some(on), _) => app.set_crossfade(on, ms),
                (None, Ok(ms)) => app.set_crossfade(true, ms),
                _ => {}
            }
        }
        "playlocal" => {
            let tracks: Vec<LocalTrack> = rest
                .split('|')
                .filter(|path| !path.trim().is_empty())
                .map(debug_local_track)
                .collect();
            if !tracks.is_empty() {
                app.play_local(tracks, 0);
            }
        }
        "normalize" => match (switch(rest), LoudnessLevel::parse(rest)) {
            (Some(on), _) => app.set_normalize(on),
            (None, Some(level)) => app.set_loudness_level(level),
            _ => {}
        },
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    let prefs = &app.settings.audio;
    format!(
        "gapless={} crossfade={} crossfade_ms={} normalize={} level={}",
        prefs.gapless,
        prefs.crossfade,
        prefs.crossfade_ms,
        prefs.normalize,
        prefs.level.name()
    )
}

pub fn register_actions(_cx: &mut App, _handle: &Handle) {}

pub fn on_press(
    _app: &mut OpenMusic,
    _key: &'static str,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
}

pub fn row_dropped(
    _app: &mut OpenMusic,
    _drag: &RowDrag,
    _at: RowDrop,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    false
}

pub fn tick(_app: &mut OpenMusic, _cx: &mut Context<OpenMusic>) {}

#[cfg(test)]
mod tests {
    use spotify_client::playback_settings::{DeviceAudio, UserAudio};

    use super::*;
    use crate::app::settings::Settings;

    #[test]
    fn names_debug_files_after_their_path() {
        let track = debug_local_track(" /tmp/Some Song: Live.m4a ");
        assert_eq!(track.path, PathBuf::from("/tmp/Some Song: Live.m4a"));
        assert_eq!(track.title, "Some Song: Live");
        assert_eq!(track.uri, "spotify:local:::Some+Song++Live:0");
    }

    #[test]
    fn starts_from_the_desktop_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"audio":{}}"#).unwrap();
        assert_eq!(settings.audio, AudioPrefs::default());
        assert!(settings.audio.gapless);
        assert!(!settings.audio.crossfade);
        assert!(settings.audio.normalize);
        assert_eq!(settings.audio.level, LoudnessLevel::Normal);
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.audio, AudioPrefs::default());
    }

    #[test]
    fn takes_values_from_spotify_and_from_changes() {
        let mut prefs = AudioPrefs::default();
        prefs.merge(&Remote {
            user: Some(UserAudio {
                gapless: false,
                crossfade: true,
                crossfade_ms: 2000,
            }),
            device: None,
            speeds: HashMap::new(),
        });
        assert!(!prefs.gapless);
        assert!(prefs.crossfade);
        assert_eq!(prefs.crossfade_ms, 2000);
        assert!(prefs.normalize);
        prefs.merge(&Remote {
            user: None,
            device: Some(DeviceAudio {
                normalize: false,
                level: LoudnessLevel::Quiet,
            }),
            speeds: HashMap::new(),
        });
        assert_eq!(prefs.crossfade_ms, 2000);
        assert!(!prefs.normalize);
        assert_eq!(prefs.level, LoudnessLevel::Quiet);
        prefs.apply(Setting::CrossfadeMs(60_000));
        prefs.apply(Setting::Gapless(true));
        assert_eq!(prefs.crossfade_ms, MAX_CROSSFADE_MS);
        let engine = prefs.engine();
        assert!(engine.gapless);
        assert!(engine.crossfade);
        assert_eq!(engine.crossfade_ms, MAX_CROSSFADE_MS);
        assert_eq!(engine.level, LoudnessLevel::Quiet);
    }
}
