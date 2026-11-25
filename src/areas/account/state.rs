use std::{collections::HashSet, path::Path};

use gpui::{App, Context, PromptButton, PromptLevel, Window};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use spotify_client::account::{AccountInfo, Explicit};

use super::{
    profile::{self, Profiles},
    profile_view::{FOLLOW_PROFILE, MORE_PROFILE, UNBLOCK_PROFILE},
    worker::{Command, Event},
};
use crate::{
    app::{OpenMusic, Page, Session, settings::Settings},
    areas::{discovery, playlists},
    menus::{self, Handle, OpenSettings, SignOut},
    ui::{
        menu::{MenuItem, Place, Row, Target},
        tracks::{RowDrag, RowDrop},
    },
    worker,
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountPrefs {
    pub language: Option<String>,
}

pub const LANGUAGE_KEY: &str = "settings-language";
pub const FALLBACK_LANGUAGE: &str = "en";

pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("id", "Bahasa Indonesia"),
    ("ms", "Bahasa Melayu"),
    ("cs", "Čeština"),
    ("da", "Dansk"),
    ("de", "Deutsch"),
    ("et", "Eesti"),
    ("es", "Español"),
    ("es-419", "Español de Latinoamérica"),
    ("fil", "Filipino"),
    ("fr", "Français"),
    ("fr-CA", "Français canadien"),
    ("hr", "Hrvatski"),
    ("is", "Íslenska"),
    ("it", "Italiano"),
    ("lv", "Latviešu"),
    ("lt", "Lietuvių"),
    ("hu", "Magyar"),
    ("nl", "Nederlands"),
    ("nb", "Norsk"),
    ("pl", "Polski"),
    ("pt-BR", "Português do Brasil"),
    ("pt-PT", "Português de Portugal"),
    ("ro", "Română"),
    ("sq", "Shqip"),
    ("sk", "Slovenčina"),
    ("sl", "Slovenščina"),
    ("sr", "Srpski"),
    ("fi", "Suomi"),
    ("sv", "Svenska"),
    ("vi", "Tiếng Việt"),
    ("tr", "Türkçe"),
    ("el", "Ελληνικά"),
    ("bg", "Български"),
    ("ru", "Русский"),
    ("uk", "Українська"),
    ("he", "עברית"),
    ("ar", "العربية"),
    ("fa", "فارسی"),
    ("hi", "हिन्दी"),
    ("th", "ไทย"),
    ("ko", "한국어"),
    ("ja", "日本語"),
    ("zh-CN", "简体中文"),
    ("zh-TW", "繁體中文"),
];

pub fn valid_language(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 35
        && !tag.starts_with('-')
        && !tag.ends_with('-')
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

pub fn language_name(tag: &str) -> Option<&'static str> {
    let exact = LANGUAGES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(tag));
    let primary = tag.split('-').next().unwrap_or_default();
    if exact.is_none() && primary.eq_ignore_ascii_case("zh") {
        let traditional = tag.split('-').skip(1).any(|part| {
            ["hant", "tw", "hk", "mo"]
                .iter()
                .any(|traditional| part.eq_ignore_ascii_case(traditional))
        });
        return Some(if traditional {
            "繁體中文"
        } else {
            "简体中文"
        });
    }
    exact
        .or_else(|| {
            LANGUAGES
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(primary))
        })
        .map(|(_, name)| *name)
}

#[cfg(target_os = "macos")]
pub fn system_language() -> String {
    objc2_foundation::NSLocale::preferredLanguages()
        .iter()
        .map(|language| language.to_string())
        .find(|language| valid_language(language))
        .unwrap_or_else(|| FALLBACK_LANGUAGE.to_owned())
}

#[cfg(not(target_os = "macos"))]
pub fn system_language() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|lang| {
            lang.split(['.', '@'])
                .next()
                .map(|tag| tag.replace('_', "-"))
        })
        .filter(|tag| valid_language(tag) && tag != "C" && tag != "POSIX")
        .unwrap_or_else(|| FALLBACK_LANGUAGE.to_owned())
}

pub fn chosen_language(language: Option<&str>, system: impl FnOnce() -> String) -> String {
    match language {
        Some(tag) if valid_language(tag) => tag.to_owned(),
        _ => system(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileListKind {
    Followers,
    Following,
    Playlists,
    RecentArtists,
    TopArtists,
    TopTracks,
}

impl ProfileListKind {
    pub const KINDS: [Self; 6] = [
        Self::Followers,
        Self::Following,
        Self::Playlists,
        Self::RecentArtists,
        Self::TopArtists,
        Self::TopTracks,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Followers => "followers",
            Self::Following => "following",
            Self::Playlists => "playlists",
            Self::RecentArtists => "recentartists",
            Self::TopArtists => "topartists",
            Self::TopTracks => "toptracks",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Followers => "Followers",
            Self::Following => "Following",
            Self::Playlists => "Public Playlists",
            Self::RecentArtists => "Recently played artists",
            Self::TopArtists => "Top artists this month",
            Self::TopTracks => "Top tracks this month",
        }
    }

    pub fn own_only(self) -> bool {
        matches!(self, Self::TopArtists | Self::TopTracks)
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::KINDS.into_iter().find(|kind| kind.name() == name)
    }
}

pub const CHIP: &str = "account-chip";
pub const CLEAR_CACHE_KEY: &str = "settings-clear-cache";
pub const CACHE_DETAIL: &str =
    "Removes saved artwork and the offline copy of your playlists. Your account is not affected.";
pub const CHIP_ROW: Row = (CHIP, 0);
pub const ACCOUNT_URL: &str = "https://www.spotify.com/account/overview/";
pub const FAMILY_URL: &str = "https://www.spotify.com/account/family/";
pub const PREMIUM_URL: &str = "https://www.spotify.com/premium/";
pub const PREMIUM_KEY: &str = "account-premium";
pub const FREE_NOTICE: &str = "Playing on this Mac needs Spotify Premium.";
pub const FREE_MESSAGE: &str = "Playing on this Mac needs Spotify Premium. You can still browse your library and control Spotify on your other devices.";

#[derive(Default)]
pub struct State {
    pub info: Option<AccountInfo>,
    pub loading: bool,
    pub error: Option<String>,
    pub languages_open: bool,
    pub cache: Option<u64>,
    pub cache_busy: bool,
    pub profiles: Profiles,
}

impl State {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        if let Some(info) = &self.info {
            urls.extend(info.avatar.clone());
        }
        self.profiles.covers(urls);
    }
}

impl OpenMusic {
    pub fn account(&self) -> Option<&AccountInfo> {
        self.account.info.as_ref()
    }

    pub fn needs_premium_here(&mut self, resume: bool, cx: &mut Context<Self>) -> bool {
        let elsewhere = self.playback.now.device.is_some() && !self.playback.now.local;
        if !blocks_playback(self.account(), resume, elsewhere) {
            return false;
        }
        self.show_toast(FREE_NOTICE, None);
        cx.notify();
        true
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
        let premium = info.premium();
        let mut items = vec![
            MenuItem::action("Profile", move |app, _, _| app.navigate(profile.clone()))
                .symbol("person.crop.circle"),
            MenuItem::action("Settings…", |app, window, cx| {
                app.open_settings(window, cx)
            })
            .symbol("gearshape")
            .key("cmd-,"),
            MenuItem::action("Manage Account", |_, _, cx| cx.open_url(ACCOUNT_URL))
                .symbol("arrow.up.forward.square"),
        ];
        if !premium {
            items.push(
                MenuItem::action("Get Premium", |_, _, cx| cx.open_url(PREMIUM_URL))
                    .symbol("sparkles")
                    .subtitle(FREE_NOTICE),
            );
        }
        items.extend([
            MenuItem::Separator,
            MenuItem::action("Sign Out", |app, _, _| app.send(worker::Command::SignOut))
                .symbol("rectangle.portrait.and.arrow.right"),
        ]);
        if !self.show_menu(items, Place::Under(CHIP), Some(CHIP_ROW)) {
            self.navigate(Page::Settings);
        }
    }

    pub fn language(&self) -> String {
        chosen_language(self.settings.account.language.as_deref(), system_language)
    }

    pub fn set_language(&mut self, language: Option<String>) {
        let language = language.filter(|tag| valid_language(tag));
        self.account.languages_open = false;
        if self.settings.account.language == language {
            return;
        }
        let before = self.language();
        self.update_settings(|settings| settings.account.language = language);
        if self.language() != before {
            self.drop_page_caches();
        }
    }

    pub fn open_language_menu(&mut self) {
        let current = self.settings.account.language.clone();
        let system = system_language();
        let mut system_item =
            MenuItem::action("System Default", |app, _, _| app.set_language(None))
                .checked(current.is_none());
        system_item = match language_name(&system) {
            Some(name) => system_item.subtitle(format!("{name} ({system})")),
            None => system_item.subtitle(system),
        };
        let mut items = vec![system_item, MenuItem::Separator];
        items.extend(LANGUAGES.iter().map(|(tag, name)| {
            MenuItem::action(*name, move |app, _, _| {
                app.set_language(Some((*tag).into()))
            })
            .checked(current.as_deref() == Some(*tag))
        }));
        if !self.show_menu(items, Place::Under(LANGUAGE_KEY), None) {
            self.account.languages_open = !self.account.languages_open;
        }
    }

    pub fn refresh_cache_size(&mut self) {
        if !self.account.cache_busy {
            self.send(Command::CacheSize);
        }
    }

    pub fn clear_cache(&mut self) {
        if self.account.cache_busy {
            return;
        }
        self.account.cache_busy = true;
        self.send(Command::ClearCache);
    }

    pub fn confirm_clear_cache(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.account.cache_busy {
            return;
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Clear the cache?",
            Some(CACHE_DETAIL),
            &[
                PromptButton::ok("Clear Cache"),
                PromptButton::cancel("Cancel"),
            ],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |app, cx| {
                    app.clear_cache();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn drop_page_caches(&mut self) {
        self.discovery.artist = None;
        self.playlists.collection = None;
        self.discovery.discovery_pending = true;
        self.send(discovery::worker::Command::Discover);
        let page = self.page.clone();
        match page {
            Page::Artist(_) => discovery::state::load(self, &page),
            Page::Collection(_) => playlists::state::load(self, &page),
            _ => {}
        }
    }
}

pub fn blocks_playback(account: Option<&AccountInfo>, resume: bool, elsewhere: bool) -> bool {
    account.is_some_and(|account| !account.premium()) && !(resume && elsewhere)
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
        format!("premium={}", info.premium()),
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

pub fn cache_label(bytes: u64) -> String {
    const KB: f64 = 1_000.;
    const MB: f64 = 1_000_000.;
    const GB: f64 = 1_000_000_000.;
    if bytes == 0 {
        return "Empty".to_owned();
    }
    let size = bytes as f64;
    let kilobytes = (size / KB).round().max(1.);
    if kilobytes < 1000. {
        return format!("{kilobytes} KB");
    }
    let megabytes = size / MB;
    if megabytes < 999.95 {
        return format!("{megabytes:.1} MB");
    }
    format!("{:.2} GB", size / GB)
}

pub fn load(app: &mut OpenMusic, page: &Page) {
    if *page == Page::Settings {
        app.refresh_cache_size();
    }
    profile::load(app, page);
}

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
        Event::Profile { uri, result } => profile::on_profile(app, uri, result),
        Event::ProfileList {
            uri,
            kind,
            offset,
            result,
        } => profile::on_list(app, uri, kind, offset, result),
        Event::Followed { states, message } => profile::on_followed(app, states, message),
        Event::FollowFailed { uris, message } => profile::on_follow_failed(app, uris, message),
        Event::FollowState(states) => app.account.profiles.learn_following(states),
        Event::Blocked { uri, block, state } => profile::on_blocked(app, uri, block, state),
        Event::BlockState(states) => app.account.profiles.learn_blocking(states),
        Event::Cache { bytes, cleared } => {
            app.account.cache = Some(bytes);
            if cleared {
                app.account.cache_busy = false;
                app.show_toast("Cache cleared", None);
            }
        }
    }
}

pub fn connected(app: &mut OpenMusic, _window: &mut Window, _cx: &mut Context<OpenMusic>) {
    app.send(Command::Language(app.settings.account.language.clone()));
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
            match rest {
                "refresh" => app.refresh_account(),
                "free" => {
                    if let Some(info) = app.account.info.as_mut() {
                        info.product = "free".into();
                        info.catalogue = "free".into();
                        info.ads = true;
                    }
                }
                _ => {}
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
        "profile" | "profiles" | "follow-user" | "follows" | "block" | "block-ask" | "blocking" => {
            return profile::debug(app, verb, rest, directory, window, cx);
        }
        "cache" => {
            match rest {
                "refresh" => app.refresh_cache_size(),
                "clear" => app.clear_cache(),
                "ask" => app.confirm_clear_cache(window, cx),
                _ => {}
            }
            let bytes = app
                .account
                .cache
                .map_or("unknown".to_owned(), |bytes| bytes.to_string());
            let label = app.account.cache.map_or("unknown".to_owned(), cache_label);
            let text = format!(
                "bytes={bytes} label={label} busy={}",
                app.account.cache_busy
            );
            let _ = std::fs::write(directory.join("cache.txt"), text);
        }
        "language" => {
            match rest {
                "" => {}
                "system" => app.set_language(None),
                "menu" => app.open_language_menu(),
                tag => app.set_language(Some(tag.to_owned())),
            }
            let setting = app.settings.account.language.as_deref().unwrap_or("system");
            let text = format!(
                "setting={setting} effective={} system={} open={}",
                app.language(),
                system_language(),
                app.account.languages_open
            );
            let _ = std::fs::write(directory.join("language.txt"), text);
        }
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
    let profile = app
        .account
        .profiles
        .profile
        .as_ref()
        .map_or("none", |(uri, _)| uri.as_str());
    format!(
        "plan={plan} user={user} explicit={explicit} language={} profile={profile} follows_pending={} blocks_pending={}",
        app.language(),
        app.account.profiles.pending.len(),
        app.account.profiles.block_pending.len()
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
    app: &mut OpenMusic,
    key: &'static str,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) -> bool {
    match key {
        "account-manage" => cx.open_url(ACCOUNT_URL),
        "account-sign-out" => app.send(worker::Command::SignOut),
        "account-retry" => app.refresh_account(),
        "account-family" => cx.open_url(FAMILY_URL),
        PREMIUM_KEY => cx.open_url(PREMIUM_URL),
        LANGUAGE_KEY => app.open_language_menu(),
        CLEAR_CACHE_KEY => app.confirm_clear_cache(window, cx),
        FOLLOW_PROFILE => {
            if let Page::Profile(uri) = &app.page {
                let uri = uri.clone();
                let following = app.account.profiles.known_following(&uri).unwrap_or(false);
                app.set_following(vec![uri], !following);
            }
        }
        MORE_PROFILE => {
            if let Page::Profile(uri) = &app.page {
                let target = Target::User { uri: uri.clone() };
                app.open_menu_under(target, MORE_PROFILE);
            }
        }
        UNBLOCK_PROFILE => {
            if let Page::Profile(uri) = &app.page {
                app.set_blocking(uri.clone(), false);
            }
        }
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

pub fn tick(app: &mut OpenMusic, cx: &mut Context<OpenMusic>) {
    profile::tick(app, cx);
}

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

    #[test]
    fn sets_the_language_through_its_path() {
        let settings = Settings::default();
        let german = with_setting(&settings, "account.language", "\"de\"").unwrap();
        assert_eq!(german.account.language.as_deref(), Some("de"));
        let bare = with_setting(&settings, "account.language", "pt-BR").unwrap();
        assert_eq!(bare.account.language.as_deref(), Some("pt-BR"));
        let system = with_setting(&german, "account.language", "null").unwrap();
        assert_eq!(system.account.language, None);
        assert!(with_setting(&settings, "account.language", "3").is_err());
    }

    #[test]
    fn names_languages_by_tag() {
        assert_eq!(language_name("de"), Some("Deutsch"));
        assert_eq!(language_name("pt-br"), Some("Português do Brasil"));
        assert_eq!(language_name("en-US"), Some("English"));
        assert_eq!(language_name("sq-XK"), Some("Shqip"));
        assert_eq!(language_name("zh-Hant-TW"), Some("繁體中文"));
        assert_eq!(language_name("zh-HK"), Some("繁體中文"));
        assert_eq!(language_name("zh-Hans-CN"), Some("简体中文"));
        assert_eq!(language_name("zh"), Some("简体中文"));
        assert_eq!(language_name("xx"), None);
        for (tag, _) in LANGUAGES {
            assert!(valid_language(tag), "{tag}");
        }
    }

    #[test]
    fn keeps_free_accounts_from_playing_here() {
        let free = AccountInfo {
            product: "free".into(),
            catalogue: "free".into(),
            ads: true,
            ..AccountInfo::default()
        };
        let premium = AccountInfo {
            product: "premium".into(),
            ..AccountInfo::default()
        };
        assert!(blocks_playback(Some(&free), false, false));
        assert!(blocks_playback(Some(&free), false, true));
        assert!(blocks_playback(Some(&free), true, false));
        assert!(!blocks_playback(Some(&free), true, true));
        assert!(!blocks_playback(Some(&premium), false, false));
        assert!(!blocks_playback(None, false, false));
    }

    #[test]
    fn labels_cache_sizes() {
        assert_eq!(cache_label(0), "Empty");
        assert_eq!(cache_label(12), "1 KB");
        assert_eq!(cache_label(1_500), "2 KB");
        assert_eq!(cache_label(999_999), "1.0 MB");
        assert_eq!(cache_label(999_960_000), "1.00 GB");
        assert_eq!(cache_label(9_800_000), "9.8 MB");
        assert_eq!(cache_label(16_049_000), "16.0 MB");
        assert_eq!(cache_label(1_234_000_000), "1.23 GB");
    }

    #[test]
    fn falls_back_to_the_system_language() {
        let system = || "sq-XK".to_owned();
        assert_eq!(chosen_language(None, system), "sq-XK");
        assert_eq!(chosen_language(Some("de"), system), "de");
        assert_eq!(chosen_language(Some(""), system), "sq-XK");
        assert_eq!(chosen_language(Some("de\r\nX: y"), system), "sq-XK");
        assert_eq!(chosen_language(Some("-de"), system), "sq-XK");
        assert!(valid_language("zh-Hans-CN"));
        assert!(!valid_language("en US"));
        assert!(!valid_language(&"a".repeat(36)));
        assert!(valid_language(&system_language()));
    }
}
