use std::path::PathBuf;

use gpui::{AnyElement, Context};
use serde::{Deserialize, Serialize};

use crate::{
    app::OpenMusic,
    areas::{
        account::state::AccountPrefs, audio::state::AudioPrefs, library::state::LibraryPrefs,
        playback::state::PlaybackPrefs, system::state::SystemPrefs,
    },
    ui::Palette,
    worker::Command,
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub audio: AudioPrefs,
    pub playback: PlaybackPrefs,
    pub system: SystemPrefs,
    pub account: AccountPrefs,
    pub library: LibraryPrefs,
}

pub type Section = fn(&OpenMusic, Palette, &mut Context<OpenMusic>) -> Option<AnyElement>;

pub const SECTIONS: &[Section] = &[
    OpenMusic::account_section,
    OpenMusic::playback_section,
    OpenMusic::audio_quality_section,
    OpenMusic::volume_section,
    OpenMusic::content_section,
    OpenMusic::social_section,
    OpenMusic::library_section,
    OpenMusic::language_section,
    OpenMusic::startup_section,
    OpenMusic::notifications_section,
    OpenMusic::device_section,
    OpenMusic::storage_section,
];

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/OpenMusic/settings.json"))
}

pub fn load() -> Settings {
    path()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save(settings: &Settings) {
    let Some(path) = path() else { return };
    let Some(folder) = path.parent() else { return };
    if std::fs::create_dir_all(folder).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec_pretty(settings) else {
        return;
    };
    let temporary = path.with_extension("tmp");
    if std::fs::write(&temporary, bytes).is_ok() {
        let _ = std::fs::rename(temporary, path);
    }
}

impl OpenMusic {
    pub fn update_settings(&mut self, change: impl FnOnce(&mut Settings)) {
        let before = self.settings.clone();
        change(&mut self.settings);
        if self.settings != before {
            save(&self.settings);
            self.send(Command::ApplySettings(self.settings.clone()));
        }
    }
}
