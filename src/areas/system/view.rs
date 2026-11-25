use gpui::{AnyElement, Context, div, prelude::*, px};
use gpui_component::switch::Switch;
use spotify_client::Track;

use super::{
    login_item::{self, Status},
    mini_player, native_window,
    notify::{self, Authorization},
};
use crate::{
    app::OpenMusic,
    areas::account::view::{setting_row, settings_section},
    overlay::{self, Label},
    ui::{
        Palette,
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

pub const MINI_BUTTON: &str = "mini-player";
pub const MINI_BUTTON_WEIGHT: u32 = 950;
pub const MINI_BUTTON_WINDOW: f32 = 1280.;
pub const NOTIFICATION_SETTINGS: &str = "system-notification-settings";

pub const NOTIFY_TITLE: &str = "Notification on song change";
pub const NOTIFY_TEXT: &str = "Show a desktop notification when the track changes.";
pub const NOTIFY_DENIED_TEXT: &str = "Enable notifications in your system settings to receive desktop notifications when the track changes.";
pub const BROADCAST_TITLE: &str = "Share what’s playing with other apps";
pub const BROADCAST_TEXT: &str = "Announces each song and every play, pause or seek to other apps on this Mac, such as scrobblers and menu bar players. Any app on this Mac can read it.";
pub const LOGIN_TITLE: &str = "Open OpenMusic automatically after you log in";
pub const MINI_TITLE: &str = "Show Mini Player when minimized";
pub const MINI_TEXT: &str =
    "Minimizing the window opens the Mini Player. It closes again when the window comes back.";

impl OpenMusic {
    pub fn startup_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let mini = Switch::new("system-mini-on-minimize")
            .checked(self.settings.system.mini_player_when_minimized)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.set_mini_on_minimize(*checked, cx);
                cx.notify();
            }));
        let mut rows = Vec::new();
        if login_item::available() {
            rows.push(self.login_row(palette, cx));
        }
        rows.push(setting_row(
            MINI_TITLE,
            Some(MINI_TEXT.into()),
            mini,
            palette,
        ));
        Some(settings_section("Startup and window", rows, palette))
    }

    fn login_row(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let status = self.system.login;
        let choice = self.startup();
        let label = choice.label();
        let picker = self
            .control(login_item::CHOICE, Label::Text(label), false, palette, cx)
            .h(px(36.))
            .w(px(choice_width(label)))
            .child(overlay::anchor(login_item::CHOICE));
        let control = div()
            .flex()
            .items_center()
            .gap_3()
            .when(status == Status::NeedsApproval, |control| {
                control.child(self.text_control(
                    login_item::SETTINGS,
                    "Login Items",
                    false,
                    palette,
                    cx,
                ))
            })
            .child(picker);
        let text = choice.describe(status, self.system.login_error.as_deref());
        setting_row(LOGIN_TITLE, Some(text.into()), control, palette)
    }

    pub fn notifications_section(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let mut rows = Vec::new();
        if notify::available() {
            rows.push(self.notify_row(palette, cx));
        }
        let broadcast = Switch::new("system-broadcast-playback")
            .checked(self.settings.system.broadcast_playback)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.set_broadcast(*checked);
                cx.notify();
            }));
        rows.push(setting_row(
            BROADCAST_TITLE,
            Some(BROADCAST_TEXT.into()),
            broadcast,
            palette,
        ));
        Some(settings_section("Notifications", rows, palette))
    }

    fn notify_row(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let denied = notify::status() == Authorization::Denied;
        let switch = Switch::new("system-notify-on-track-change")
            .checked(self.settings.system.notify_on_track_change)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                if *checked != this.settings.system.notify_on_track_change {
                    this.toggle_notifications(cx);
                }
                cx.notify();
            }));
        let control = div()
            .flex()
            .items_center()
            .gap_3()
            .when(denied, |control| {
                control.child(self.text_control(
                    NOTIFICATION_SETTINGS,
                    "Notifications Settings",
                    false,
                    palette,
                    cx,
                ))
            })
            .child(switch);
        let text = if denied {
            NOTIFY_DENIED_TEXT
        } else {
            NOTIFY_TEXT
        };
        setting_row(NOTIFY_TITLE, Some(text.into()), control, palette)
    }
}

fn choice_width(label: &str) -> f32 {
    (32. + 7.6 * label.chars().count() as f32).max(72.)
}

pub fn menu_items(_target: &Target, _app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    Vec::new()
}

pub fn nav_entries(_app: &OpenMusic) -> Vec<NavEntry> {
    Vec::new()
}

pub fn dock_items(
    app: &OpenMusic,
    roomy: bool,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Vec<DockItem> {
    if !roomy || native_window::width() < MINI_BUTTON_WINDOW || !app.mini_allowed() {
        return Vec::new();
    }
    let symbol = if mini_player::is_visible() {
        "pip.exit"
    } else {
        "pip.enter"
    };
    vec![DockItem::new(
        MINI_BUTTON,
        MINI_BUTTON_WEIGHT,
        48.,
        app.dock_control(MINI_BUTTON, Label::Symbol(symbol), false, palette, cx)
            .size(px(48.)),
    )]
}

pub fn row_accessories(
    _app: &OpenMusic,
    _track: &Track,
    _row: Row,
    _options: &RowOptions,
    _palette: Palette,
    _cx: &mut Context<OpenMusic>,
) -> Vec<AnyElement> {
    Vec::new()
}

pub fn row_dimmed(_app: &OpenMusic, _track: &Track, _row: Row, _options: &RowOptions) -> bool {
    false
}
