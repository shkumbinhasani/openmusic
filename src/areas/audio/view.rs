use std::sync::Mutex;

use gpui::{AnyElement, Context, FontWeight, SharedString, div, prelude::*, px, rgb, rgba};
use gpui_component::{input::Input, slider::Slider, switch::Switch};
use spotify_client::{
    Track,
    playback_settings::{MAX_SPEED, MIN_SPEED, round_speed},
};

use super::{
    settings::{
        LENGTH_TITLE, Line, NAME_TEXT, NAME_TITLE, RESET_NAME_KEY, RETRY_KEY, SYNC_FAILED_TEXT,
        SYNC_FAILED_TITLE, choice_width, seconds_label,
    },
    state::{SLEEP_KEY, SPEED_KEY, base_device_name, speed_text},
};
use crate::{
    app::OpenMusic,
    overlay::{self, ButtonSpec, Label},
    ui::{
        Palette,
        components::ROW_RADIUS,
        dock::DockItem,
        menu::{Group, MenuItem, Row, Surface, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

const SPEED_WEIGHT: u32 = 620;
const SLEEP_WEIGHT: u32 = 660;
const ROW_INSET: f32 = 14.;
const NAME_WIDTH: f32 = 180.;
const LENGTH_WIDTH: f32 = 160.;

fn settings_section(
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

fn setting_row(
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

fn note_row(text: String, palette: Palette) -> AnyElement {
    div()
        .px(px(ROW_INSET))
        .py(px(10.))
        .text_size(px(12.))
        .text_color(rgb(palette.muted))
        .child(text)
        .into_any_element()
}

fn speed_label(speed: f32) -> &'static str {
    static LABELS: Mutex<Vec<(u32, &'static str)>> = Mutex::new(Vec::new());
    let speed = round_speed(speed.clamp(MIN_SPEED, MAX_SPEED));
    let key = (speed * 100.).round() as u32;
    let Ok(mut labels) = LABELS.lock() else {
        return "1×";
    };
    if let Some((_, label)) = labels.iter().find(|(known, _)| *known == key) {
        return label;
    }
    let label: &'static str = Box::leak(speed_text(speed).into_boxed_str());
    labels.push((key, label));
    label
}

impl OpenMusic {
    pub fn playback_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rows = self.setting_rows(self.playback_lines(), palette, cx);
        Some(settings_section("Playback", rows, palette))
    }

    pub fn audio_quality_section(
        &self,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let rows = self.setting_rows(self.quality_lines(), palette, cx);
        Some(settings_section("Audio quality", rows, palette))
    }

    pub fn volume_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rows = self.setting_rows(self.volume_lines(), palette, cx);
        Some(settings_section("Volume", rows, palette))
    }

    pub fn device_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rows = self.setting_rows(self.device_lines(), palette, cx);
        Some(settings_section("Device", rows, palette))
    }

    fn setting_rows(
        &self,
        lines: Vec<Line>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        lines
            .into_iter()
            .map(|line| self.setting_line(line, palette, cx))
            .collect()
    }

    fn setting_line(&self, line: Line, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        match line {
            Line::Switch {
                toggle,
                title,
                text,
                on,
            } => {
                let switch = Switch::new(toggle.id()).checked(on).on_click(cx.listener(
                    move |this, checked: &bool, _, cx| {
                        this.flip(toggle, *checked);
                        cx.notify();
                    },
                ));
                setting_row(title, Some(text.into()), switch, palette)
            }
            Line::Choice {
                key,
                title,
                text,
                label,
                enabled,
            } => {
                let spec = ButtonSpec {
                    label: Label::Text(label),
                    prominent: false,
                    enabled,
                    tooltip: None,
                };
                let picker = self
                    .glass_control(key, spec, !self.modal_open(), palette, cx)
                    .h(px(36.))
                    .w(px(choice_width(label)))
                    .child(overlay::anchor(key));
                setting_row(title, Some(text.into()), picker, palette)
            }
            Line::Length { ms } => {
                let slider = self.audio.controls.length.as_ref().map(|state| {
                    div().w(px(LENGTH_WIDTH)).child(
                        Slider::new(state)
                            .bg(rgb(palette.accent))
                            .text_color(rgb(0xffffff)),
                    )
                });
                let control = div().flex().items_center().gap_3().children(slider).child(
                    div()
                        .w(px(44.))
                        .text_size(px(12.5))
                        .text_color(rgb(palette.muted))
                        .child(seconds_label(ms)),
                );
                setting_row(LENGTH_TITLE, None, control, palette)
            }
            Line::Name { renamed, error } => self.name_row(renamed, error, palette, cx),
            Line::Note(text) => note_row(text, palette),
            Line::Retry => setting_row(
                SYNC_FAILED_TITLE,
                Some(SYNC_FAILED_TEXT.into()),
                self.text_control(RETRY_KEY, "Try Again", false, palette, cx),
                palette,
            ),
        }
    }

    fn name_row(
        &self,
        renamed: bool,
        error: Option<String>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field = div()
            .w(px(NAME_WIDTH))
            .h(px(32.))
            .flex()
            .items_center()
            .rounded(px(7.))
            .bg(rgba(palette.press))
            .border_1()
            .border_color(rgba(if error.is_some() {
                palette.accent << 8 | 0xff
            } else {
                palette.line
            }))
            .children(self.audio.controls.name.as_ref().map(|state| {
                Input::new(state)
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false)
            }));
        let control = div()
            .flex()
            .items_center()
            .gap_2()
            .child(field)
            .when(renamed, |control| {
                let spec = ButtonSpec {
                    label: Label::Text("Reset"),
                    prominent: false,
                    enabled: true,
                    tooltip: Some(format!("Use “{}” again", base_device_name())),
                };
                control.child(
                    self.glass_control(RESET_NAME_KEY, spec, !self.modal_open(), palette, cx)
                        .h(px(36.))
                        .w(px(choice_width("Reset"))),
                )
            });
        let row = setting_row(NAME_TITLE, Some(NAME_TEXT.into()), control, palette);
        match error {
            None => row,
            Some(error) => div()
                .flex()
                .flex_col()
                .child(row)
                .child(
                    div()
                        .px(px(ROW_INSET))
                        .pb(px(10.))
                        .mt(px(-4.))
                        .text_size(px(12.))
                        .text_color(rgb(palette.accent))
                        .child(error),
                )
                .into_any_element(),
        }
    }

    fn speed_control(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let speed = self.current_speed();
        let enabled = self.can_set_speed();
        let spec = ButtonSpec {
            label: Label::Text(speed_label(speed)),
            prominent: (speed - 1.).abs() > 0.01,
            enabled,
            tooltip: Some(if enabled {
                "Playback speed".to_owned()
            } else {
                "Speed can’t change for this episode".to_owned()
            }),
        };
        self.glass_control(SPEED_KEY, spec, true, palette, cx)
            .h(px(36.))
            .w(px(56.))
            .child(overlay::anchor(SPEED_KEY))
    }

    fn sleep_control(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let spec = ButtonSpec {
            label: Label::Symbol("moon.zzz.fill"),
            prominent: false,
            enabled: true,
            tooltip: Some(format!(
                "Sleep timer: {}",
                self.sleep_label().unwrap_or_default()
            )),
        };
        self.glass_control(SLEEP_KEY, spec, true, palette, cx)
            .size(px(48.))
            .child(overlay::anchor(SLEEP_KEY))
    }
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let Target::Track(track) = target else {
        return Vec::new();
    };
    let current = app.playback.now.track.as_ref();
    if track.surface != Surface::NowPlaying
        || current.is_none_or(|current| current.uri != track.track.uri)
    {
        return Vec::new();
    }
    let mut items = Vec::new();
    if app.playback.now.episode {
        items.push((
            Group::Queue,
            MenuItem::submenu(
                format!("Playback speed ({})", speed_text(app.current_speed())),
                app.speed_items(),
            )
            .symbol("speedometer"),
        ));
    }
    items.push((
        Group::Queue,
        MenuItem::submenu("Sleep timer", app.sleep_items()).symbol("moon.zzz"),
    ));
    if let Some(stream) = app.stream_text() {
        items.push((Group::Info, MenuItem::note(stream)));
    }
    items
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
    let mut items = Vec::new();
    if app.playback.now.episode && app.playback.now.track.is_some() {
        items.push(DockItem::new(
            SPEED_KEY,
            SPEED_WEIGHT,
            60.,
            app.speed_control(palette, cx),
        ));
    }
    if roomy && app.sleep_label().is_some() {
        items.push(DockItem::new(
            SLEEP_KEY,
            SLEEP_WEIGHT,
            48.,
            app.sleep_control(palette, cx),
        ));
    }
    items
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reuses_one_label_per_speed() {
        let first = speed_label(1.5);
        assert_eq!(first, "1.5×");
        assert!(std::ptr::eq(first, speed_label(1.49)));
        assert_eq!(speed_label(9.), "3.5×");
        assert_eq!(speed_label(0.), "0.5×");
    }
}
