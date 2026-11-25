use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui::{
    AnimationExt, Context, Div, ElementId, FontWeight, Image, ObjectFit, Pixels, Stateful, canvas,
    div, img, prelude::*, px, rgb, rgba, svg,
};
use spotify_client::Track;

use super::Palette;
use crate::{
    app::{OpenMusic, Page},
    icons,
    overlay::{self, Action, ButtonSpec, Label},
};

pub fn icon(path: &'static str, size: f32, color: u32) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(size))
        .flex_shrink_0()
        .text_color(rgb(color))
}

pub fn link(
    id: impl Into<ElementId>,
    text: String,
    target: Option<Page>,
    size: f32,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> impl IntoElement {
    let label = div()
        .id(id)
        .min_w_0()
        .truncate()
        .text_size(px(size))
        .text_color(rgb(palette.muted))
        .child(text);
    match target {
        Some(page) => label
            .cursor_pointer()
            .hover(move |label| label.text_color(rgb(palette.text)).underline())
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.navigate(page.clone());
                cx.notify();
            })),
        None => label,
    }
}

pub fn artwork(image: Option<&Arc<Image>>, side: Pixels, radius: Pixels, palette: Palette) -> Div {
    div()
        .size(side)
        .flex_shrink_0()
        .rounded(radius)
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(palette.raised))
        .map(|cover| match image {
            Some(image) => cover.child(
                img(image.clone())
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded(radius),
            ),
            None => cover.child(icon(icons::NOTE, f32::from(side) * 0.36, palette.muted)),
        })
}

pub const ROW_RADIUS: f32 = 8.;

const SHIMMER_PERIOD: std::time::Duration = std::time::Duration::from_millis(1500);

pub fn bone(key: &'static str, index: usize, palette: Palette) -> Div {
    let glint = if palette.dark { 0xffffff1a } else { 0xffffffb0 };
    let clear = glint & 0xffffff00;
    div()
        .relative()
        .overflow_hidden()
        .bg(rgba(palette.press))
        .child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(gpui::relative(0.6))
                .flex()
                .child(div().flex_1().h_full().bg(gpui::linear_gradient(
                    90.,
                    gpui::linear_color_stop(rgba(clear), 0.),
                    gpui::linear_color_stop(rgba(glint), 1.),
                )))
                .child(div().flex_1().h_full().bg(gpui::linear_gradient(
                    90.,
                    gpui::linear_color_stop(rgba(glint), 0.),
                    gpui::linear_color_stop(rgba(clear), 1.),
                )))
                .with_animation(
                    (key, index),
                    gpui::Animation::new(SHIMMER_PERIOD)
                        .repeat()
                        .with_easing(gpui::ease_in_out),
                    |band, delta| band.left(gpui::relative(-0.6 + delta * 1.6)),
                ),
        )
}

pub fn skeleton_rows(key: &'static str, count: usize, numbered: bool, palette: Palette) -> Div {
    let widths = [0.42, 0.3, 0.5, 0.36, 0.46, 0.28, 0.4, 0.33];
    div()
        .flex()
        .flex_col()
        .children((0..count).map(move |index| {
            let width = widths[index % widths.len()];
            div()
                .h(px(56.))
                .flex()
                .items_center()
                .gap_3()
                .px_2()
                .map(|row| {
                    if numbered {
                        row.child(
                            div().w(px(40.)).flex().justify_center().child(
                                bone(key, index * 8, palette)
                                    .w(px(14.))
                                    .h(px(12.))
                                    .rounded(px(3.)),
                            ),
                        )
                    } else {
                        row.child(
                            bone(key, index * 8, palette)
                                .size(px(40.))
                                .flex_shrink_0()
                                .rounded(px(6.)),
                        )
                    }
                })
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .child(
                            bone(key, index * 8 + 1, palette)
                                .w(gpui::relative(width))
                                .h(px(13.))
                                .rounded(px(4.)),
                        )
                        .child(
                            bone(key, index * 8 + 2, palette)
                                .w(gpui::relative(width * 0.55))
                                .h(px(11.))
                                .rounded(px(4.)),
                        ),
                )
                .child(
                    bone(key, index * 8 + 3, palette)
                        .w(px(34.))
                        .h(px(11.))
                        .rounded(px(4.)),
                )
        }))
}

pub fn skeleton_header(key: &'static str, round: bool, palette: Palette) -> Div {
    div()
        .flex()
        .items_center()
        .gap_6()
        .child(
            bone(key, 0, palette)
                .size(px(176.))
                .flex_shrink_0()
                .rounded(px(if round { 88. } else { 16. })),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(bone(key, 1, palette).w(px(64.)).h(px(11.)).rounded(px(4.)))
                .child(bone(key, 2, palette).w(px(320.)).h(px(34.)).rounded(px(8.)))
                .child(bone(key, 3, palette).w(px(200.)).h(px(14.)).rounded(px(4.)))
                .child(
                    bone(key, 4, palette)
                        .mt_2()
                        .w(px(96.))
                        .h(px(38.))
                        .rounded_full(),
                ),
        )
}

pub fn fallback_icon(symbol: &str) -> &'static str {
    match symbol.trim_end_matches(".fill") {
        "backward" => icons::PREVIOUS,
        "forward" => icons::NEXT,
        "pause" => icons::PAUSE,
        "quote.bubble" => icons::LYRICS,
        "chevron.left" => icons::BACK,
        "house" => icons::HOME,
        "magnifyingglass" => icons::SEARCH,
        "music.note" => icons::NOTE,
        "shuffle" => icons::SHUFFLE,
        "sparkles" => icons::SHUFFLE_SMART,
        "repeat" => icons::REPEAT,
        "repeat.1" => icons::REPEAT_ONE,
        "list.bullet" | "text.line.first.and.arrowtriangle.forward" => icons::QUEUE,
        "hifispeaker.and.homepod" | "laptopcomputer.and.iphone" | "airplay.audio" => icons::DEVICES,
        "laptopcomputer" | "desktopcomputer" => icons::DEVICE,
        "iphone" | "ipad" | "tv" | "gamecontroller" | "car" | "applewatch" => icons::DEVICES,
        "hifispeaker" | "hifispeaker.2" | "hifireceiver" | "speaker.wave.2" | "speaker.wave.3" => {
            icons::SPEAKER
        }
        "heart" if symbol.ends_with(".fill") => icons::HEART_FILL,
        "heart" => icons::HEART,
        "plus" | "plus.circle" => icons::PLUS,
        "checkmark" | "checkmark.circle" => icons::CHECK,
        "folder" => icons::FOLDER,
        "pin" => icons::PIN,
        "lock" => icons::LOCK,
        "ellipsis" | "ellipsis.circle" => icons::MORE,
        "moon" | "moon.zzz" => icons::MOON,
        "gauge.with.dots.needle.67percent" | "speedometer" => icons::SPEED,
        "pip" | "pip.enter" | "pip.exit" => icons::PIP,
        "person" | "person.crop.circle" | "person.2" => icons::PERSON,
        "e.square" => icons::EXPLICIT,
        "doc" | "music.note.house" => icons::LOCAL,
        _ => icons::PLAY,
    }
}

pub fn explicit_badge(palette: Palette) -> impl IntoElement {
    icon(icons::EXPLICIT, 14., palette.muted)
}

#[derive(Clone, Debug, Default)]
pub struct Sentinel(Rc<Cell<bool>>);

impl Sentinel {
    pub fn take(&self) -> bool {
        self.0.replace(false)
    }
}

pub fn scroll_sentinel(sentinel: &Sentinel) -> impl IntoElement {
    let seen = sentinel.0.clone();
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            if window.content_mask().bounds.contains(&bounds.origin) {
                seen.set(true);
            }
        },
    )
    .w_full()
    .h(px(0.))
}

pub fn timestamp(ms: u32) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl OpenMusic {
    pub fn control(
        &self,
        key: &'static str,
        label: Label,
        prominent: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let spec = ButtonSpec {
            label,
            prominent,
            enabled: true,
            tooltip: None,
        };
        self.glass_control(key, spec, !self.modal_open(), palette, cx)
    }

    pub fn dock_control(
        &self,
        key: &'static str,
        label: Label,
        prominent: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let spec = ButtonSpec {
            label,
            prominent,
            enabled: true,
            tooltip: None,
        };
        self.glass_control(key, spec, true, palette, cx)
    }

    pub fn menu_button(
        &self,
        key: &'static str,
        name: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let spec = ButtonSpec {
            label: Label::Symbol("ellipsis"),
            prominent: false,
            enabled: true,
            tooltip: Some(format!("More options for {name}")),
        };
        self.glass_control(key, spec, !self.modal_open(), palette, cx)
            .size(px(36.))
            .child(overlay::anchor(key))
    }

    fn glass_control(
        &self,
        key: &'static str,
        spec: ButtonSpec,
        visible: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let slot = div()
            .id(key)
            .occlude()
            .relative()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center();
        if overlay::available() {
            return slot.when(visible, |slot| slot.child(overlay::button(key, spec)));
        }
        let label = spec.label;
        slot.rounded_full()
            .bg(rgba(if spec.prominent {
                palette.accent << 8 | 0xff
            } else {
                palette.fill
            }))
            .border_1()
            .border_color(rgba(palette.rim))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .when(!spec.enabled, |button| button.opacity(0.5))
            .when(spec.enabled, |button| {
                button
                    .cursor_pointer()
                    .hover(move |button| button.opacity(0.85))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.handle_control(Action::Press(key), window, cx)
                    }))
            })
            .map(|button| match label {
                Label::Symbol(name) => button.child(icon(fallback_icon(name), 20., palette.text)),
                Label::Text(text) => button.child(text),
            })
    }

    pub fn text_control(
        &self,
        key: &'static str,
        text: &'static str,
        prominent: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        self.control(key, Label::Text(text), prominent, palette, cx)
            .h(px(36.))
            .w(px(32. + 7.4 * text.chars().count() as f32))
    }

    pub fn cover_for(&self, track: &Track) -> Option<&Arc<Image>> {
        track.cover.as_ref().and_then(|url| self.covers.get(url))
    }

    pub fn back_button(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        self.control("back", Label::Symbol("chevron.left"), false, palette, cx)
            .size(px(40.))
    }

    pub fn page_status(&self, text: impl Into<gpui::SharedString>, palette: Palette) -> Div {
        div()
            .py_8()
            .text_color(rgb(palette.muted))
            .child(text.into())
    }
}

#[cfg(test)]
mod tests {
    use gpui::AssetSource;
    use spotify_client::connect::DeviceKind;

    use super::fallback_icon;
    use crate::icons::{self, Icons};

    #[test]
    fn every_fallback_icon_has_artwork() {
        let kinds = [
            DeviceKind::Computer,
            DeviceKind::Smartphone,
            DeviceKind::Tablet,
            DeviceKind::Speaker,
            DeviceKind::Tv,
            DeviceKind::Receiver,
            DeviceKind::GameConsole,
            DeviceKind::Car,
            DeviceKind::Watch,
            DeviceKind::Group,
            DeviceKind::Other,
        ];
        for kind in kinds {
            assert_ne!(fallback_icon(kind.symbol()), icons::PLAY, "{kind:?}");
        }
        let symbols = [
            "play.fill",
            "pause.fill",
            "backward.fill",
            "forward.fill",
            "quote.bubble",
            "quote.bubble.fill",
            "chevron.left",
            "ellipsis",
            "shuffle",
            "repeat",
            "repeat.1",
            "heart",
            "heart.fill",
            "e.square",
        ];
        for symbol in symbols.into_iter().chain(kinds.map(DeviceKind::symbol)) {
            let path = fallback_icon(symbol);
            assert!(
                Icons.load(path).is_ok_and(|svg| svg.is_some()),
                "{symbol} → {path}"
            );
        }
        assert_eq!(fallback_icon("heart"), icons::HEART);
        assert_eq!(fallback_icon("heart.fill"), icons::HEART_FILL);
        assert_eq!(fallback_icon("ellipsis"), icons::MORE);
        assert_eq!(fallback_icon("laptopcomputer"), icons::DEVICE);
    }
}
