use std::{collections::HashSet, path::Path};

use gpui::{App, Context, Window};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spotify_client::account::{AccountInfo, Explicit};

use super::worker::{Command, Event};
use crate::{
    app::{OpenMusic, Page, Session, settings::Settings},
    menus::{self, Handle, OpenSettings, SignOut},
    ui::{
        menu::{MenuItem, Place, Row},
        tracks::{RowDrag, RowDrop},
    },
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

pub const CHIP: &str = "account-chip";
pub const CHIP_ROW: Row = (CHIP, 0);
pub const ACCOUNT_URL: &str = "https://www.spotify.com/account/overview/";
pub const FAMILY_URL: &str = "https://www.spotify.com/account/family/";

#[derive(Default)]
pub struct State {
    pub info: Option<AccountInfo>,
    pub loading: bool,
    pub error: Option<String>,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        if let Some(info) = &self.info {
            urls.extend(info.avatar.clone());
        }
    }
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

    pub fn refresh_account(&mut self) {
        self.account.loading = true;
        self.send(Command::Refresh);
    }

    pub fn open_account_menu(&mut self) {
        let Some(info) = self.account() else {
            self.navigate(Page::Settings);
            return;
        };
        let profile = Page::Profile(info.uri.clone());
        let items = vec![
            MenuItem::action("Profile", move |app, _, _| app.navigate(profile.clone()))
                .symbol("person.crop.circle"),
            MenuItem::action("Settings…", |app, window, cx| {
                app.open_settings(window, cx)
            })
            .symbol("gearshape")
            .key("cmd-,"),
            MenuItem::action("Manage Account", |_, _, cx| cx.open_url(ACCOUNT_URL))
                .symbol("arrow.up.forward.square"),
            MenuItem::Separator,
            MenuItem::action("Sign Out", |app, _, _| app.send(worker::Command::SignOut))
                .symbol("rectangle.portrait.and.arrow.right"),
        ];
        if !self.show_menu(items, Place::Under(CHIP), Some(CHIP_ROW)) {
            self.navigate(Page::Settings);
        }
    }
}

pub fn describe_account(info: &AccountInfo) -> String {
    [
        format!("username={}", info.username),
        format!("name={}", info.name),
        format!("uri={}", info.uri),
        format!("product={}", info.product),
        format!("plan={}", info.plan()),
        format!("catalogue={}", info.catalogue),
        format!("country={}", info.country),
        format!("ads={}", info.ads),
        format!("explicit_filtered={}", info.explicit_filtered),
        format!("explicit_locked={}", info.explicit_locked),
        format!(
            "family_role={}",
            info.family_role.as_deref().unwrap_or("none")
        ),
        format!("avatar={}", info.avatar.is_some()),
    ]
    .join("\n")
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
    app: &mut OpenMusic,
    event: Event,
    _window: &mut Window,
    _cx: &mut Context<OpenMusic>,
) {
    match event {
        Event::Account(result) => {
            app.account.loading = false;
            match result {
                Ok(info) => {
                    app.account.info = Some(info);
                    app.account.error = None;
                }
                Err(message) => app.account.error = Some(message),
            }
        }
    }
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {
    app.refresh_account();
}

pub fn signed_out(app: &mut OpenMusic) {
    app.account = State::default();
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
        "account" => {
            if rest == "refresh" {
                app.refresh_account();
            }
            let text = match (&app.account.info, &app.account.error) {
                (Some(info), error) => {
                    let mut text = describe_account(info);
                    if let Some(error) = error {
                        text.push_str(&format!("\nerror={error}"));
                    }
                    text
                }
                (None, Some(error)) => format!("error={error}"),
                (None, None) => format!("loading={}", app.account.loading),
            };
            let _ = std::fs::write(directory.join("account.txt"), text);
        }
        "account-menu" => app.open_account_menu(),
        _ => return false,
    }
    true
}

pub fn state_fragment(app: &OpenMusic) -> String {
    let plan = app
        .account()
        .map(|account| account.product.as_str())
        .filter(|product| !product.is_empty())
        .unwrap_or("unknown");
    let user = app
        .account()
        .map_or("none", |account| account.username.as_str());
    let explicit = match app.account().map(AccountInfo::explicit) {
        Some(Explicit::Allowed) => "allowed",
        Some(Explicit::Filtered) => "filtered",
        Some(Explicit::FilteredByManager) => "locked",
        None => "unknown",
    };
    format!("plan={plan} user={user} explicit={explicit}")
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
    app: &mut OpenMusic,
    key: &'static str,
    _window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        "account-manage" => cx.open_url(ACCOUNT_URL),
        "account-sign-out" => app.send(worker::Command::SignOut),
        "account-retry" => app.refresh_account(),
        "account-family" => cx.open_url(FAMILY_URL),
        _ => return false,
    }
    true
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
