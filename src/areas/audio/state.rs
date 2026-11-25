use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

use gpui::{App, Context, Window};
use serde::{Deserialize, Serialize};
use spotify_client::{
    catalog::LocalTrack,
    connect::SleepState,
    engine::playback::{AudioSettings, AudioStatus, SleepChoice},
    playback_settings::{
        LoudnessLevel, MAX_CROSSFADE_MS, Remote, Setting, round_speed, valid_speed,
    },
};

use super::worker::{Command, Event};
use crate::{
    app::OpenMusic,
    menus::{self, Handle, SleepAtEndOfTrack, SleepTimer},
    ui::{
        menu::{MenuItem, Place},
        tracks::{RowDrag, RowDrop},
    },
};

pub const SPEED_KEY: &str = "audio-speed";
pub const SLEEP_KEY: &str = "audio-sleep";
pub const SLEEP_MINUTES: [(u32, &str); 6] = [
    (5, "5 minutes"),
    (10, "10 minutes"),
    (15, "15 minutes"),
    (30, "30 minutes"),
    (45, "45 minutes"),
    (60, "1 hour"),
];
pub const SPEEDS: [f32; 10] = [0.5, 0.8, 1., 1.2, 1.5, 1.8, 2., 2.5, 3., 3.5];
const PENDING: Duration = Duration::from_secs(3);

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
    pub status: AudioStatus,
    pending_speed: Option<(f32, Instant)>,
    sleep_label: Option<String>,
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

    pub fn speed_show(&self) -> Option<String> {
        let now = &self.playback.now;
        if !now.local || !now.episode {
            return None;
        }
        now.track
            .as_ref()
            .and_then(|track| track.album_uri.clone())
            .filter(|uri| uri.starts_with("spotify:show:"))
    }

    pub fn current_speed(&self) -> f32 {
        match self.audio.pending_speed {
            Some((speed, at)) if at.elapsed() < PENDING => speed,
            _ => self.playback.now.speed,
        }
    }

    pub fn can_set_speed(&self) -> bool {
        self.audio.status.speed_allowed && self.playback.now.episode
    }

    pub fn set_speed(&mut self, speed: f32) {
        let Some(speed) = valid_speed(round_speed(speed)) else {
            self.show_toast("Choose a speed between 0.5× and 3.5×.", None);
            return;
        };
        let show = self.speed_show();
        if let Some(show) = &show {
            self.audio.speeds.insert(show.clone(), speed);
        }
        self.audio.pending_speed = Some((speed, Instant::now()));
        self.send(Command::Speed { speed, show });
    }

    pub fn speed_items(&self) -> Vec<MenuItem> {
        let current = self.current_speed();
        let enabled = self.can_set_speed();
        let mut items = vec![MenuItem::header("Playback speed")];
        items.extend(SPEEDS.into_iter().map(|speed| {
            MenuItem::action(speed_text(speed), move |app, _, cx| {
                app.set_speed(speed);
                cx.notify();
            })
            .checked((current - speed).abs() < 0.01)
            .enabled(enabled)
        }));
        if !SPEEDS.iter().any(|speed| (current - speed).abs() < 0.01) {
            items.push(MenuItem::Separator);
            items.push(MenuItem::note(format!("Now {}", speed_text(current))));
        }
        if self.speed_show().is_some() {
            items.push(MenuItem::Separator);
            items.push(MenuItem::note("Saved for this show"));
        }
        items
    }

    pub fn open_speed_menu(&mut self) -> bool {
        let items = self.speed_items();
        self.show_menu(items, Place::Under(SPEED_KEY), None)
    }

    pub fn sleep_state(&self) -> SleepState {
        self.audio.status.sleep
    }

    pub fn sleep_label(&self) -> Option<String> {
        sleep_left(
            self.sleep_state(),
            SystemTime::now(),
            self.playback.now.episode,
        )
    }

    pub fn sleep_available(&self) -> bool {
        let now = &self.playback.now;
        now.local || now.device.is_none()
    }

    pub fn set_sleep(&mut self, choice: SleepChoice) {
        let set = choice != SleepChoice::Off && choice != SleepChoice::Minutes(0);
        if set && !self.sleep_available() {
            self.show_toast("The sleep timer works when music plays on this Mac.", None);
            return;
        }
        if !set && self.sleep_state() == SleepState::Off {
            return;
        }
        self.audio.status.sleep = match choice {
            SleepChoice::EndOfTrack => SleepState::EndOfTrack,
            SleepChoice::Minutes(minutes) if minutes > 0 => {
                SleepState::At(SystemTime::now() + Duration::from_secs(u64::from(minutes) * 60))
            }
            _ => SleepState::Off,
        };
        self.send(Command::Sleep(choice));
        self.show_toast(
            if set {
                "OK, your sleep timer is set."
            } else {
                "Sleep timer turned off."
            },
            None,
        );
    }

    pub fn sleep_items(&self) -> Vec<MenuItem> {
        let state = self.sleep_state();
        let enabled = self.sleep_available();
        let mut items = vec![MenuItem::header("Sleep timer")];
        if let Some(label) = self.sleep_label() {
            items.push(MenuItem::note(label));
        }
        items.extend(SLEEP_MINUTES.into_iter().map(|(minutes, title)| {
            MenuItem::action(title, move |app, _, cx| {
                app.set_sleep(SleepChoice::Minutes(minutes));
                cx.notify();
            })
            .enabled(enabled)
        }));
        let end = if self.playback.now.episode {
            "End of episode"
        } else {
            "End of track"
        };
        items.push(
            MenuItem::action(end, |app, _, cx| {
                app.set_sleep(SleepChoice::EndOfTrack);
                cx.notify();
            })
            .checked(state == SleepState::EndOfTrack)
            .enabled(enabled),
        );
        if state != SleepState::Off {
            items.push(MenuItem::Separator);
            items.push(MenuItem::action("Turn off timer", |app, _, cx| {
                app.set_sleep(SleepChoice::Off);
                cx.notify();
            }));
        }
        if !enabled {
            items.push(MenuItem::Separator);
            items.push(MenuItem::note("Works when music plays on this Mac"));
        }
        items
    }

    pub fn open_sleep_menu(&mut self) -> bool {
        let items = self.sleep_items();
        self.show_menu(items, Place::Under(SLEEP_KEY), None)
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
        lines.push(format!(
            "status speed_allowed={} speed={} episode={} sleep={}",
            self.audio.status.speed_allowed,
            self.current_speed(),
            self.playback.now.episode,
            self.sleep_label().unwrap_or_else(|| "off".to_owned())
        ));
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
        Event::Status(status) => app.audio.status = status,
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
        "sleep" => match rest.trim() {
            "off" | "0" => app.set_sleep(SleepChoice::Off),
            "end" => app.set_sleep(SleepChoice::EndOfTrack),
            minutes => {
                if let Ok(minutes) = minutes.parse::<u32>() {
                    app.set_sleep(SleepChoice::Minutes(minutes));
                }
            }
        },
        "speed" => {
            if let Ok(speed) = rest.trim().trim_end_matches(['x', '×']).parse::<f32>() {
                app.set_speed(speed);
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
        "gapless={} crossfade={} crossfade_ms={} normalize={} level={} speed={} sleep={}",
        prefs.gapless,
        prefs.crossfade,
        prefs.crossfade_ms,
        prefs.normalize,
        prefs.level.name(),
        app.current_speed(),
        match app.sleep_state() {
            SleepState::Off => "off".to_owned(),
            SleepState::EndOfTrack => "end".to_owned(),
            SleepState::At(deadline) => format!(
                "{}s",
                deadline
                    .duration_since(SystemTime::now())
                    .map_or(0, |left| left.as_secs())
            ),
        }
    )
}

pub fn register_actions(cx: &mut App, handle: &Handle) {
    menus::route(cx, handle, |action: &SleepTimer, app, _, cx| {
        app.set_sleep(match action.minutes {
            0 => SleepChoice::Off,
            minutes => SleepChoice::Minutes(minutes),
        });
        cx.notify();
    });
    menus::route(cx, handle, |_: &SleepAtEndOfTrack, app, _, cx| {
        app.set_sleep(SleepChoice::EndOfTrack);
        cx.notify();
    });
}

pub fn on_press(
    app: &mut OpenMusic,
    key: &'static str,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        SPEED_KEY => {
            app.open_speed_menu();
            true
        }
        SLEEP_KEY => {
            app.open_sleep_menu();
            true
        }
        _ => false,
    }
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

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    if let Some((_, at)) = app.audio.pending_speed
        && at.elapsed() >= PENDING
    {
        app.audio.pending_speed = None;
        cx.notify();
    }
    if let SleepState::At(deadline) = app.sleep_state()
        && !app.playback.now.playing
        && deadline + PENDING < SystemTime::now()
    {
        app.audio.status.sleep = SleepState::Off;
    }
    let label = app.sleep_label();
    if label != app.audio.sleep_label {
        app.audio.sleep_label = label;
        cx.notify();
    }
}

pub fn sleep_left(state: SleepState, now: SystemTime, episode: bool) -> Option<String> {
    match state {
        SleepState::Off => None,
        SleepState::EndOfTrack => Some(
            if episode {
                "Stops at the end of this episode"
            } else {
                "Stops at the end of this track"
            }
            .to_owned(),
        ),
        SleepState::At(deadline) => {
            let seconds = deadline
                .duration_since(now)
                .map_or(0, |left| left.as_secs());
            let minutes = seconds.div_ceil(60);
            Some(match minutes {
                0 => "Stopping now".to_owned(),
                1 if seconds < 60 => "Less than a minute left".to_owned(),
                minutes if minutes < 60 => format!("{minutes} min left"),
                minutes if minutes % 60 == 0 => format!("{} hr left", minutes / 60),
                minutes => format!("{} hr {} min left", minutes / 60, minutes % 60),
            })
        }
    }
}

pub fn speed_text(speed: f32) -> String {
    let text = format!("{:.2}", round_speed(speed));
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text}×")
}

#[cfg(test)]
mod tests {
    use spotify_client::playback_settings::{DeviceAudio, UserAudio};

    use super::*;
    use crate::app::settings::Settings;

    #[test]
    fn says_how_long_the_sleep_timer_has_left() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        let at = |seconds: u64| SleepState::At(now + Duration::from_secs(seconds));
        assert_eq!(sleep_left(SleepState::Off, now, false), None);
        assert_eq!(
            sleep_left(SleepState::EndOfTrack, now, false).as_deref(),
            Some("Stops at the end of this track")
        );
        assert_eq!(
            sleep_left(SleepState::EndOfTrack, now, true).as_deref(),
            Some("Stops at the end of this episode")
        );
        assert_eq!(
            sleep_left(at(0), now, false).as_deref(),
            Some("Stopping now")
        );
        assert_eq!(
            sleep_left(at(59), now, false).as_deref(),
            Some("Less than a minute left")
        );
        assert_eq!(
            sleep_left(at(60), now, false).as_deref(),
            Some("1 min left")
        );
        assert_eq!(
            sleep_left(at(61), now, false).as_deref(),
            Some("2 min left")
        );
        assert_eq!(
            sleep_left(at(1800), now, false).as_deref(),
            Some("30 min left")
        );
        assert_eq!(
            sleep_left(at(3600), now, false).as_deref(),
            Some("1 hr left")
        );
        assert_eq!(
            sleep_left(at(3900), now, false).as_deref(),
            Some("1 hr 5 min left")
        );
        let past = SleepState::At(now - Duration::from_secs(30));
        assert_eq!(
            sleep_left(past, now, false).as_deref(),
            Some("Stopping now")
        );
    }

    #[test]
    fn writes_speeds_the_way_the_menu_shows_them() {
        assert_eq!(speed_text(1.), "1×");
        assert_eq!(speed_text(1.5), "1.5×");
        assert_eq!(speed_text(0.8), "0.8×");
        assert_eq!(speed_text(1.25), "1.25×");
        assert_eq!(speed_text(1.23), "1.25×");
        assert_eq!(speed_text(3.5), "3.5×");
        assert_eq!(
            SPEEDS.map(speed_text).join(" "),
            "0.5× 0.8× 1× 1.2× 1.5× 1.8× 2× 2.5× 3× 3.5×"
        );
    }

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
