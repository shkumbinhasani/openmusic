use std::{collections::HashSet, path::Path};

use gpui::{App, Context, Window};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spotify_client::account::AccountInfo;

use super::worker::Event;
use crate::{
    app::{OpenMusic, Page, Session, settings::Settings},
    menus::{self, Handle, OpenSettings, SignOut},
    ui::tracks::{RowDrag, RowDrop},
    worker,
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountPrefs {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileListKind {
    Followers,
    Following,
    Playlists,
    RecentArtists,
}

impl ProfileListKind {
    pub const KINDS: [Self; 4] = [
        Self::Followers,
        Self::Following,
        Self::Playlists,
        Self::RecentArtists,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Followers => "followers",
            Self::Following => "following",
            Self::Playlists => "playlists",
            Self::RecentArtists => "recentartists",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::KINDS.into_iter().find(|kind| kind.name() == name)
    }
}

#[derive(Default)]
pub struct State {
    pub info: Option<AccountInfo>,
}

impl State {
    pub fn covers(&self, _urls: &mut HashSet<String>) {}
}

impl OpenMusic {
    pub fn account(&self) -> Option<&AccountInfo> {
        self.account.info.as_ref()
    }

    pub fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_main_window(window, cx);
        if matches!(self.session, Session::Connected) {
            self.navigate(Page::Settings);
        }
    }
}

pub fn with_setting(settings: &Settings, path: &str, value: &str) -> Result<Settings, String> {
    let text = value.trim();
    let value: Value =
        serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()));
    let mut tree = serde_json::to_value(settings).map_err(|error| error.to_string())?;
    let mut slot = &mut tree;
    for key in path.split('.') {
        slot = slot
            .as_object_mut()
            .and_then(|object| object.get_mut(key))
            .filter(|_| !key.is_empty())
            .ok_or_else(|| format!("unknown setting {path}"))?;
    }
    *slot = value.clone();
    let changed: Settings = serde_json::from_value(tree)
        .map_err(|error| format!("{path} cannot be {text}: {error}"))?;
    let check = serde_json::to_value(&changed).map_err(|error| error.to_string())?;
    let stored = path.split('.').fold(&check, |node, key| &node[key]);
    if !value.is_object() && *stored != value {
        return Err(format!("{path} cannot be {text}"));
    }
    Ok(changed)
}

pub fn load(_app: &mut OpenMusic, _page: &Page) {}

pub fn on_event(
    _app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {}
}

pub fn connected(_app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {}

pub fn signed_out(app: &mut OpenMusic) {
    app.account.info = None;
}

pub fn debug(
    app: &mut OpenMusic,
    verb: &str,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match verb {
        "setting" => {
            let (path, value) = rest.split_once(' ').unwrap_or((rest, "null"));
            let result = with_setting(&app.settings, path, value)
                .map(|settings| app.update_settings(|current| *current = settings));
            let line = result.err().unwrap_or_else(|| "ok".to_owned());
            let _ = std::fs::write(directory.join("setting.txt"), line);
            if let Ok(text) = serde_json::to_string_pretty(&app.settings) {
                let _ = std::fs::write(directory.join("settings.txt"), text);
            }
        }
        "open-settings" => app.open_settings(window, cx),
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    format!(
        "plan={}",
        app.account()
            .map(|account| account.product.as_str())
            .filter(|product| !product.is_empty())
            .unwrap_or("unknown")
    )
}

pub fn register_actions(cx: &mut App, handle: &Handle) {
    menus::route(cx, handle, |_: &SignOut, app, _, _| {
        app.send(worker::Command::SignOut)
    });
    menus::route(cx, handle, |_: &OpenSettings, app, window, cx| {
        app.open_settings(window, cx);
        cx.notify();
    });
}

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
    use super::*;

    #[test]
    fn changes_one_setting_by_its_path() {
        let settings = Settings::default();
        assert_eq!(
            with_setting(&settings, "account", "{}"),
            Ok(settings.clone())
        );
        assert_eq!(
            with_setting(&settings, "account.nothing", "true"),
            Err("unknown setting account.nothing".into())
        );
        assert_eq!(
            with_setting(&settings, "nowhere", "1"),
            Err("unknown setting nowhere".into())
        );
        assert_eq!(
            with_setting(&settings, "", "1"),
            Err("unknown setting ".into())
        );
        assert!(with_setting(&settings, "account", "7").is_err());
    }
}
