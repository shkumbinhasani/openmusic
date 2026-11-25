use std::time::{Duration, Instant};

use gpui::{AppContext, Context, Entity, Focusable, Window};
use gpui_component::{
    input::{InputEvent, InputState},
    slider::{SliderEvent, SliderState},
};
use spotify_client::{
    engine::playback::{Quality, Stream, StreamKind, device_name},
    playback_settings::{LoudnessLevel, MAX_CROSSFADE_MS},
};

use super::{
    state::{AudioPrefs, base_device_name},
    worker::Command,
};
use crate::{
    app::OpenMusic,
    ui::menu::{MenuItem, Place},
};

pub const QUALITY_KEY: &str = "audio-quality";
pub const LEVEL_KEY: &str = "audio-level";
pub const RETRY_KEY: &str = "audio-settings-retry";
pub const RESET_NAME_KEY: &str = "audio-device-reset";
pub const DEFAULT_CROSSFADE_MS: u32 = 5_000;
const STEP_MS: u32 = 1_000;
const SETTLE: Duration = Duration::from_millis(400);

pub const AUTOPLAY_TITLE: &str = "Autoplay";
pub const AUTOPLAY_TEXT: &str =
    "Enjoy non-stop listening. When your audio ends, we’ll play you something similar.";
pub const GAPLESS_TITLE: &str = "Gapless playback";
pub const GAPLESS_TEXT: &str = "Removes any gaps or pauses that may occur in between tracks.";
pub const CROSSFADE_TITLE: &str = "Crossfade songs";
pub const CROSSFADE_TEXT: &str =
    "Fades each song into the next. Episodes and very short songs don’t fade.";
pub const LENGTH_TITLE: &str = "Crossfade length";
pub const SHARED_NOTE: &str = "Gapless and crossfade are saved to your Spotify account, so they also change in the Spotify app.";
pub const SYNC_FAILED_TITLE: &str = "Couldn’t load your playback settings from Spotify";
pub const SYNC_FAILED_TEXT: &str = "Changes here still apply on this Mac.";
pub const QUALITY_TITLE: &str = "Streaming quality";
pub const LOWERED_NOTE: &str = "Playback stalled, so songs play at 128 kbps for now. They go back to 256 kbps after 15 minutes without a stall.";
pub const NEXT_SONG: &str = "A new choice starts with the next song.";
pub const OTHER_SOURCE: &str = "Streaming quality doesn’t change this source.";
pub const FORMATS_NOTE: &str =
    "Very high and lossless use Spotify’s own encryption, which OpenMusic can’t play.";
pub const NORMALIZE_TITLE: &str = "Normalize volume";
pub const NORMALIZE_TEXT: &str = "Set the same volume level for all songs and podcasts.";
pub const LEVEL_TITLE: &str = "Volume level";
pub const LEVEL_TEXT: &str =
    "Adjust the volume for your environment. OpenMusic can’t boost songs above full volume.";
pub const LEVEL_OFF_TEXT: &str = "Turn on Normalize volume to change your volume level.";
pub const NAME_TITLE: &str = "Device name";
pub const NAME_TEXT: &str = "How this Mac shows up in Spotify Connect on your other devices.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toggle {
    Autoplay,
    Gapless,
    Crossfade,
    Normalize,
}

impl Toggle {
    pub fn id(self) -> &'static str {
        match self {
            Self::Autoplay => "audio-autoplay",
            Self::Gapless => "audio-gapless",
            Self::Crossfade => "audio-crossfade",
            Self::Normalize => "audio-normalize",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Switch {
        toggle: Toggle,
        title: &'static str,
        text: &'static str,
        on: bool,
    },
    Choice {
        key: &'static str,
        title: &'static str,
        text: String,
        label: &'static str,
        enabled: bool,
    },
    Length {
        ms: u32,
    },
    Name {
        renamed: bool,
        error: Option<String>,
    },
    Note(String),
    Retry,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NameChange {
    Same(String),
    Rename(String),
    Invalid(&'static str),
}

#[derive(Default)]
pub struct Controls {
    pub name: Option<Entity<InputState>>,
    pub length: Option<Entity<SliderState>>,
    pub draft: Option<(u32, Instant)>,
    pub name_error: Option<String>,
}

pub fn level_title(level: LoudnessLevel) -> &'static str {
    match level {
        LoudnessLevel::Quiet => "Quiet",
        LoudnessLevel::Normal => "Normal",
        LoudnessLevel::Loud => "Loud",
    }
}

pub fn next_choice<T: Copy + PartialEq>(all: &[T], current: T) -> T {
    let index = all.iter().position(|choice| *choice == current);
    index
        .and_then(|index| all.get((index + 1) % all.len()))
        .or_else(|| all.first())
        .copied()
        .unwrap_or(current)
}

pub fn crossfade_on_ms(current: u32) -> u32 {
    if current == 0 {
        DEFAULT_CROSSFADE_MS
    } else {
        current.min(MAX_CROSSFADE_MS)
    }
}

pub fn slider_ms(value: f32) -> u32 {
    let seconds = value.round().clamp(0., (MAX_CROSSFADE_MS / STEP_MS) as f32);
    seconds as u32 * STEP_MS
}

pub fn slider_value(ms: u32) -> f32 {
    (ms.min(MAX_CROSSFADE_MS) as f32 / STEP_MS as f32).round()
}

pub fn seconds_label(ms: u32) -> String {
    format!("{} sec", slider_value(ms) as u32)
}

pub fn choice_width(label: &str) -> f32 {
    (32. + 7.6 * label.chars().count() as f32).max(72.)
}

pub fn name_change(current: &str, typed: &str) -> NameChange {
    match device_name(typed) {
        Err(message) => NameChange::Invalid(message),
        Ok(name) if name == current => NameChange::Same(name),
        Ok(name) => NameChange::Rename(name),
    }
}

pub fn playback_lines(
    prefs: &AudioPrefs,
    length_ms: u32,
    sync_failed: bool,
    elsewhere: Option<&str>,
) -> Vec<Line> {
    let mut lines = Vec::new();
    if let Some(device) = elsewhere {
        lines.push(Line::Note(format!(
            "Playback is on {device}. These settings take effect when music plays on this Mac."
        )));
    }
    if sync_failed {
        lines.push(Line::Retry);
    }
    lines.push(Line::Switch {
        toggle: Toggle::Autoplay,
        title: AUTOPLAY_TITLE,
        text: AUTOPLAY_TEXT,
        on: prefs.autoplay,
    });
    lines.push(Line::Switch {
        toggle: Toggle::Gapless,
        title: GAPLESS_TITLE,
        text: GAPLESS_TEXT,
        on: prefs.gapless,
    });
    lines.push(Line::Switch {
        toggle: Toggle::Crossfade,
        title: CROSSFADE_TITLE,
        text: CROSSFADE_TEXT,
        on: prefs.crossfade,
    });
    if prefs.crossfade {
        lines.push(Line::Length { ms: length_ms });
    }
    lines.push(Line::Note(SHARED_NOTE.to_owned()));
    lines
}

pub fn now_playing_note(stream: &Stream) -> String {
    let follow = if stream.kind == StreamKind::FairPlay {
        NEXT_SONG
    } else {
        OTHER_SOURCE
    };
    format!("Now playing: {}. {follow}", stream.describe())
}

pub fn quality_lines(quality: Quality, lowered: bool, stream: Option<&Stream>) -> Vec<Line> {
    let mut lines = vec![Line::Choice {
        key: QUALITY_KEY,
        title: QUALITY_TITLE,
        text: quality.detail().to_owned(),
        label: quality.title(),
        enabled: true,
    }];
    if lowered && quality == Quality::Automatic {
        lines.push(Line::Note(LOWERED_NOTE.to_owned()));
    }
    if let Some(stream) = stream {
        lines.push(Line::Note(now_playing_note(stream)));
    }
    lines.push(Line::Note(FORMATS_NOTE.to_owned()));
    lines
}

pub fn volume_lines(prefs: &AudioPrefs) -> Vec<Line> {
    vec![
        Line::Switch {
            toggle: Toggle::Normalize,
            title: NORMALIZE_TITLE,
            text: NORMALIZE_TEXT,
            on: prefs.normalize,
        },
        Line::Choice {
            key: LEVEL_KEY,
            title: LEVEL_TITLE,
            text: if prefs.normalize {
                LEVEL_TEXT
            } else {
                LEVEL_OFF_TEXT
            }
            .to_owned(),
            label: level_title(prefs.level),
            enabled: prefs.normalize,
        },
    ]
}

pub fn device_lines(name: &str, base: &str, error: Option<&str>) -> Vec<Line> {
    vec![Line::Name {
        renamed: name != base,
        error: error.map(str::to_owned),
    }]
}

pub fn describe(title: &str, lines: &[Line], field: Option<&str>) -> String {
    let mut text = format!("section {title}\n");
    for line in lines {
        let entry = match line {
            Line::Switch {
                toggle, title, on, ..
            } => format!(
                "switch {} {} {title}",
                toggle.id(),
                if *on { "on" } else { "off" }
            ),
            Line::Choice {
                key,
                title,
                text,
                label,
                enabled,
            } => format!(
                "choice {key} \"{label}\"{} {title} — {text}",
                if *enabled { "" } else { " [x]" }
            ),
            Line::Length { ms } => format!("length {}", seconds_label(*ms)),
            Line::Name { renamed, error } => format!(
                "name field=\"{}\" renamed={renamed} error={}",
                field.unwrap_or_default(),
                error.as_deref().unwrap_or("none")
            ),
            Line::Note(note) => format!("note {note}"),
            Line::Retry => format!("retry {SYNC_FAILED_TITLE}"),
        };
        text.push_str("  ");
        text.push_str(&entry);
        text.push('\n');
    }
    text
}

impl OpenMusic {
    pub fn playing_elsewhere(&self) -> Option<&str> {
        let now = &self.playback.now;
        if now.local {
            return None;
        }
        now.device.as_deref()
    }

    pub fn crossfade_length(&self) -> u32 {
        self.audio
            .controls
            .draft
            .map_or(self.settings.audio.crossfade_ms, |(ms, _)| ms)
    }

    pub fn playback_lines(&self) -> Vec<Line> {
        playback_lines(
            &self.settings.audio,
            self.crossfade_length(),
            matches!(self.audio.synced, Some(Err(_))),
            self.playing_elsewhere(),
        )
    }

    pub fn quality_lines(&self) -> Vec<Line> {
        quality_lines(
            self.settings.audio.quality,
            self.quality_lowered(),
            self.audio.status.stream.as_ref(),
        )
    }

    pub fn volume_lines(&self) -> Vec<Line> {
        volume_lines(&self.settings.audio)
    }

    pub fn device_lines(&self) -> Vec<Line> {
        device_lines(
            &self.device_name(),
            &base_device_name(),
            self.audio.controls.name_error.as_deref(),
        )
    }

    pub fn flip(&mut self, toggle: Toggle, on: bool) {
        let prefs = &self.settings.audio;
        match toggle {
            Toggle::Autoplay => self.set_autoplay(on),
            Toggle::Gapless => self.set_gapless(on),
            Toggle::Crossfade => {
                let ms = if on {
                    crossfade_on_ms(prefs.crossfade_ms)
                } else {
                    prefs.crossfade_ms
                };
                self.set_crossfade(on, ms);
            }
            Toggle::Normalize => self.set_normalize(on),
        }
    }

    pub fn quality_items(&self) -> Vec<MenuItem> {
        let current = self.settings.audio.quality;
        Quality::ALL
            .into_iter()
            .map(|quality| {
                MenuItem::action(quality.title(), move |app, _, cx| {
                    app.set_quality(quality);
                    cx.notify();
                })
                .checked(quality == current)
                .subtitle(quality.detail())
            })
            .collect()
    }

    pub fn level_items(&self) -> Vec<MenuItem> {
        let current = self.settings.audio.level;
        LoudnessLevel::ALL
            .into_iter()
            .map(|level| {
                MenuItem::action(level_title(level), move |app, _, cx| {
                    app.set_loudness_level(level);
                    cx.notify();
                })
                .checked(level == current)
            })
            .collect()
    }

    pub fn open_quality_menu(&mut self) {
        let items = self.quality_items();
        if !self.show_menu(items, Place::Under(QUALITY_KEY), None) {
            let next = next_choice(&Quality::ALL, self.settings.audio.quality);
            self.set_quality(next);
        }
    }

    pub fn open_level_menu(&mut self) {
        if !self.settings.audio.normalize {
            return;
        }
        let items = self.level_items();
        if !self.show_menu(items, Place::Under(LEVEL_KEY), None) {
            let next = next_choice(&LoudnessLevel::ALL, self.settings.audio.level);
            self.set_loudness_level(next);
        }
    }

    pub fn retry_audio_sync(&mut self) {
        self.audio.synced = None;
        self.send(Command::Sync);
    }

    pub fn reset_device_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let base = base_device_name();
        match self.rename_device(&base) {
            Ok(name) => {
                self.audio.controls.name_error = None;
                self.show_name(&name, window, cx);
                self.show_toast(format!("Renamed this Mac to “{name}”."), None);
            }
            Err(message) => self.show_toast(message, None),
        }
    }

    pub fn name_field(&self, cx: &Context<Self>) -> Option<String> {
        self.audio
            .controls
            .name
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
    }

    pub fn type_device_name(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.show_name(text, window, cx);
        cx.defer_in(window, |app, window, cx| {
            app.commit_device_name(false, window, cx);
            cx.notify();
        });
    }

    fn show_name(&self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = &self.audio.controls.name {
            let text = text.to_owned();
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
    }

    fn commit_device_name(
        &mut self,
        leaving: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(typed) = self.name_field(cx) else {
            return false;
        };
        let current = self.device_name();
        match name_change(&current, &typed) {
            NameChange::Same(name) => {
                if typed != name {
                    self.show_name(&name, window, cx);
                }
                self.audio.controls.name_error = None;
                true
            }
            NameChange::Rename(name) => match self.rename_device(&name) {
                Ok(name) => {
                    self.audio.controls.name_error = None;
                    if typed != name {
                        self.show_name(&name, window, cx);
                    }
                    self.show_toast(format!("Renamed this Mac to “{name}”."), None);
                    true
                }
                Err(message) => {
                    self.audio.controls.name_error = Some(message);
                    false
                }
            },
            NameChange::Invalid(message) => {
                if leaving {
                    self.audio.controls.name_error = None;
                    self.show_name(&current, window, cx);
                    self.show_toast(message, None);
                } else {
                    self.audio.controls.name_error = Some(message.to_owned());
                }
                false
            }
        }
    }
}

pub fn create_controls(app: &mut OpenMusic, window: &mut Window, cx: &mut Context<OpenMusic>) {
    if app.audio.controls.name.is_none() {
        let name = app.device_name();
        let placeholder = base_device_name();
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(name)
        });
        cx.subscribe_in(&input, window, on_name_event).detach();
        app.audio.controls.name = Some(input);
    }
    if app.audio.controls.length.is_none() {
        let value = slider_value(app.settings.audio.crossfade_ms);
        let slider = cx.new(|_| {
            SliderState::new()
                .min(0.)
                .max(slider_value(MAX_CROSSFADE_MS))
                .step(1.)
                .default_value(value)
        });
        cx.subscribe(&slider, |app, _, event: &SliderEvent, cx| {
            let SliderEvent::Change(value) = event;
            app.audio.controls.draft = Some((slider_ms(value.end()), Instant::now()));
            cx.notify();
        })
        .detach();
        app.audio.controls.length = Some(slider);
    }
}

fn on_name_event(
    app: &mut OpenMusic,
    _: &Entity<InputState>,
    event: &InputEvent,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    match event {
        InputEvent::Change => {
            if app.audio.controls.name_error.take().is_none() {
                return;
            }
        }
        InputEvent::PressEnter { .. } => {
            if app.commit_device_name(false, window, cx) {
                window.focus(&app.focus);
            }
        }
        InputEvent::Blur => {
            app.commit_device_name(true, window, cx);
            if window.focused(cx).is_none() {
                window.focus(&app.focus);
            }
        }
        InputEvent::Focus => return,
    }
    cx.notify();
}

pub fn sync_controls(app: &mut OpenMusic, window: &mut Window, cx: &mut Context<OpenMusic>) {
    if let Some(input) = app.audio.controls.name.clone() {
        let name = app.device_name();
        let (focused, text) = {
            let input = input.read(cx);
            (
                input.focus_handle(cx).is_focused(window),
                input.value().to_string(),
            )
        };
        if !focused && text != name {
            app.audio.controls.name_error = None;
            input.update(cx, |input, cx| input.set_value(name, window, cx));
        }
    }
    if app.audio.controls.draft.is_none()
        && let Some(slider) = app.audio.controls.length.clone()
    {
        let value = slider_value(app.settings.audio.crossfade_ms);
        if (slider.read(cx).value().end() - value).abs() > 0.01 {
            slider.update(cx, |slider, cx| slider.set_value(value, window, cx));
        }
    }
}

pub fn settle(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    let Some((ms, at)) = app.audio.controls.draft else {
        return;
    };
    if at.elapsed() < SETTLE {
        return;
    }
    app.audio.controls.draft = None;
    let prefs = &app.settings.audio;
    if !prefs.crossfade || prefs.crossfade_ms != ms {
        app.set_crossfade(true, ms);
    }
    cx.notify();
}

pub fn dump(app: &OpenMusic, cx: &Context<OpenMusic>) -> String {
    let field = app.name_field(cx);
    [
        describe("Playback", &app.playback_lines(), None),
        describe("Audio quality", &app.quality_lines(), None),
        describe("Volume", &app.volume_lines(), None),
        describe("Device", &app.device_lines(), field.as_deref()),
    ]
    .concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn switches(lines: &[Line]) -> Vec<(Toggle, bool)> {
        lines
            .iter()
            .filter_map(|line| match line {
                Line::Switch { toggle, on, .. } => Some((*toggle, *on)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn lists_the_playback_rows_like_the_desktop() {
        let prefs = AudioPrefs::default();
        let lines = playback_lines(&prefs, 0, false, None);
        assert_eq!(
            switches(&lines),
            vec![
                (Toggle::Autoplay, true),
                (Toggle::Gapless, true),
                (Toggle::Crossfade, false)
            ]
        );
        assert!(!lines.iter().any(|line| matches!(line, Line::Length { .. })));
        assert_eq!(lines.last(), Some(&Line::Note(SHARED_NOTE.to_owned())));

        let prefs = AudioPrefs {
            crossfade: true,
            crossfade_ms: 2000,
            autoplay: false,
            ..AudioPrefs::default()
        };
        let lines = playback_lines(&prefs, 7000, true, Some("Living Room"));
        assert_eq!(
            lines[0],
            Line::Note(
                "Playback is on Living Room. These settings take effect when music plays on this Mac."
                    .to_owned()
            )
        );
        assert_eq!(lines[1], Line::Retry);
        assert_eq!(
            switches(&lines),
            vec![
                (Toggle::Autoplay, false),
                (Toggle::Gapless, true),
                (Toggle::Crossfade, true)
            ]
        );
        assert!(lines.contains(&Line::Length { ms: 7000 }));
    }

    #[test]
    fn explains_the_quality_choice() {
        let lines = quality_lines(Quality::Automatic, false, None);
        assert_eq!(
            lines[0],
            Line::Choice {
                key: QUALITY_KEY,
                title: QUALITY_TITLE,
                text: "256 kbps, or 128 kbps while your connection is slow".to_owned(),
                label: "Automatic",
                enabled: true,
            }
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1], Line::Note(FORMATS_NOTE.to_owned()));

        let fairplay = Stream {
            kind: StreamKind::FairPlay,
            codec: Some("AAC".to_owned()),
            kbps: Some(128),
        };
        let lines = quality_lines(Quality::Automatic, true, Some(&fairplay));
        assert_eq!(lines[1], Line::Note(LOWERED_NOTE.to_owned()));
        assert_eq!(
            lines[2],
            Line::Note(
                "Now playing: AAC 128 kbps, FairPlay. A new choice starts with the next song."
                    .to_owned()
            )
        );
        let host = Stream {
            kind: StreamKind::Host,
            codec: Some("MP3".to_owned()),
            kbps: None,
        };
        assert_eq!(
            now_playing_note(&host),
            "Now playing: MP3 from the podcast’s host. Streaming quality doesn’t change this source."
        );

        let lines = quality_lines(Quality::High, true, None);
        assert!(matches!(
            &lines[0],
            Line::Choice { label: "High", text, .. } if text == "256 kbps"
        ));
        assert!(!lines.contains(&Line::Note(LOWERED_NOTE.to_owned())));
    }

    #[test]
    fn locks_the_level_while_normalization_is_off() {
        let on = volume_lines(&AudioPrefs {
            level: LoudnessLevel::Loud,
            ..AudioPrefs::default()
        });
        assert_eq!(switches(&on), vec![(Toggle::Normalize, true)]);
        assert_eq!(
            on[1],
            Line::Choice {
                key: LEVEL_KEY,
                title: LEVEL_TITLE,
                text: LEVEL_TEXT.to_owned(),
                label: "Loud",
                enabled: true,
            }
        );
        let off = volume_lines(&AudioPrefs {
            normalize: false,
            ..AudioPrefs::default()
        });
        assert_eq!(switches(&off), vec![(Toggle::Normalize, false)]);
        assert!(matches!(
            &off[1],
            Line::Choice { enabled: false, label: "Normal", text, .. } if text == LEVEL_OFF_TEXT
        ));
    }

    #[test]
    fn checks_a_typed_device_name() {
        assert_eq!(
            name_change("OpenMusic", "  OpenMusic "),
            NameChange::Same("OpenMusic".to_owned())
        );
        assert_eq!(
            name_change("OpenMusic", "Desk   Mac"),
            NameChange::Rename("Desk Mac".to_owned())
        );
        assert_eq!(
            name_change("OpenMusic", "   "),
            NameChange::Invalid("Enter a name for this Mac.")
        );
        assert_eq!(
            name_change("OpenMusic", &"x".repeat(65)),
            NameChange::Invalid("Use 64 characters or fewer.")
        );
        assert_eq!(
            device_lines("Desk Mac", "OpenMusic", Some("Use 64 characters or fewer.")),
            vec![Line::Name {
                renamed: true,
                error: Some("Use 64 characters or fewer.".to_owned())
            }]
        );
        assert_eq!(
            device_lines("OpenMusic", "OpenMusic", None),
            vec![Line::Name {
                renamed: false,
                error: None
            }]
        );
    }

    #[test]
    fn moves_the_crossfade_length_in_whole_seconds() {
        assert_eq!(slider_ms(0.), 0);
        assert_eq!(slider_ms(4.6), 5000);
        assert_eq!(slider_ms(30.), 12_000);
        assert_eq!(slider_ms(-1.), 0);
        assert_eq!(slider_value(2000), 2.);
        assert_eq!(slider_value(2600), 3.);
        assert_eq!(slider_value(60_000), 12.);
        assert_eq!(seconds_label(0), "0 sec");
        assert_eq!(seconds_label(12_000), "12 sec");
        assert_eq!(crossfade_on_ms(0), DEFAULT_CROSSFADE_MS);
        assert_eq!(crossfade_on_ms(2000), 2000);
        assert_eq!(crossfade_on_ms(90_000), MAX_CROSSFADE_MS);
    }

    #[test]
    fn cycles_choices_where_menus_cannot_open() {
        assert_eq!(
            next_choice(&Quality::ALL, Quality::Automatic),
            Quality::Normal
        );
        assert_eq!(
            next_choice(&Quality::ALL, Quality::High),
            Quality::Automatic
        );
        assert_eq!(
            next_choice(&LoudnessLevel::ALL, LoudnessLevel::Loud),
            LoudnessLevel::Quiet
        );
        assert_eq!(choice_width("High"), 72.);
        assert!(choice_width("Automatic") > 72.);
        assert_eq!(
            LoudnessLevel::ALL.map(level_title),
            ["Quiet", "Normal", "Loud"]
        );
    }

    #[test]
    fn dumps_the_rows_for_live_checks() {
        let prefs = AudioPrefs {
            crossfade: true,
            crossfade_ms: 6000,
            normalize: false,
            ..AudioPrefs::default()
        };
        let text = [
            describe("Playback", &playback_lines(&prefs, 6000, false, None), None),
            describe("Volume", &volume_lines(&prefs), None),
            describe(
                "Device",
                &device_lines("OpenMusic", "OpenMusic", None),
                Some("OpenMusic"),
            ),
        ]
        .concat();
        assert_eq!(
            text,
            format!(
                "section Playback\n  switch audio-autoplay on Autoplay\n  switch audio-gapless on Gapless playback\n  switch audio-crossfade on Crossfade songs\n  length 6 sec\n  note {SHARED_NOTE}\nsection Volume\n  switch audio-normalize off Normalize volume\n  choice audio-level \"Normal\" [x] Volume level — {LEVEL_OFF_TEXT}\nsection Device\n  name field=\"OpenMusic\" renamed=false error=none\n"
            )
        );
    }
}
