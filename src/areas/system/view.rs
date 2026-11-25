use gpui::{AnyElement, Context, FontWeight, SharedString, div, prelude::*, px, rgb, rgba};
use gpui_component::switch::Switch;
use spotify_client::Track;

use super::{
    mini_player, native_window,
    notify::{self, Authorization},
};
use crate::{
    app::OpenMusic,
    overlay::Label,
    ui::{
        Palette,
        components::ROW_RADIUS,
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

const ROW_INSET: f32 = 14.;

pub const NOTIFY_TITLE: &str = "Notification on song change";
pub const NOTIFY_TEXT: &str = "Show a desktop notification when the track changes.";
pub const NOTIFY_DENIED_TEXT: &str = "Enable notifications in your system settings to receive desktop notifications when the track changes.";
pub const MINI_TITLE: &str = "Show Mini Player when minimized";
pub const MINI_TEXT: &str =
    "Minimizing the window opens the Mini Player. It closes again when the window comes back.";

pub fn settings_section(
    title: impl Into<SharedString>,
    rows: Vec<AnyElement>,
    palette: Palette,
) -> AnyElement {
    let count = rows.len();
    div()
        .flex()
        .flex_col()
        .gap_2p5()
        .child(
            div()
                .px_1()
                .text_size(px(16.))
                .font_weight(FontWeight::BOLD)
                .child(title.into()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(ROW_RADIUS + 4.))
                .bg(rgba(palette.press))
                .children(rows.into_iter().enumerate().flat_map(|(index, row)| {
                    let line = (index + 1 < count).then(|| {
                        div()
                            .h(px(1.))
                            .mx(px(ROW_INSET))
                            .bg(rgba(palette.line))
                            .into_any_element()
                    });
                    std::iter::once(row).chain(line)
                })),
        )
        .into_any_element()
}

pub fn setting_row(
    title: impl IntoElement,
    description: Option<SharedString>,
    control: impl IntoElement,
    palette: Palette,
) -> AnyElement {
    div()
        .min_h(px(52.))
        .flex()
        .items_center()
        .gap_4()
        .px(px(ROW_INSET))
        .py(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .when_some(description, |text, description| {
                    text.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette.muted))
                            .child(description),
                    )
                }),
        )
        .child(div().flex_shrink_0().flex().items_center().child(control))
        .into_any_element()
}

impl OpenMusic {
    pub fn startup_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let mini = Switch::new("system-mini-on-minimize")
            .checked(self.settings.system.mini_player_when_minimized)
            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                this.set_mini_on_minimize(*checked, cx);
                cx.notify();
            }));
        let rows = vec![setting_row(
            MINI_TITLE,
            Some(MINI_TEXT.into()),
            mini,
            palette,
        )];
        Some(settings_section("Startup and window", rows, palette))
    }

    pub fn notifications_section(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !notify::available() {
            return None;
        }
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
        let rows = vec![setting_row(
            NOTIFY_TITLE,
            Some(text.into()),
            control,
            palette,
        )];
        Some(settings_section("Notifications", rows, palette))
    }
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
