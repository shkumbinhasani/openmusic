use std::rc::Rc;

use gpui::{
    App, Div, ElementId, FontWeight, SharedString, Stateful, Window, div, prelude::*, px, rgb, rgba,
};
use gpui_component::{WindowExt, dialog::Dialog};

use crate::ui::Palette;

pub type Close = Rc<dyn Fn(&mut Window, &mut App)>;

const DOCK_CLEARANCE: f32 = 84.;
const CHROME: f32 = 116.;

pub fn top(window: &Window) -> f32 {
    (f32::from(window.viewport_size().height) * 0.1).clamp(36., 96.)
}

pub fn body_height(window: &Window) -> f32 {
    let height = f32::from(window.viewport_size().height);
    (height - top(window) - DOCK_CLEARANCE - CHROME).max(120.)
}

pub fn frame(
    dialog: Dialog,
    window: &Window,
    width: f32,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    close: Close,
) -> Dialog {
    let palette = Palette::for_window(window.appearance());
    let on_close = close.clone();
    dialog
        .w(px(width))
        .margin_top(px(top(window)))
        .close_button(false)
        .bg(rgb(palette.raised))
        .border_color(rgba(palette.line))
        .rounded(px(14.))
        .on_close(move |_, window, cx| on_close(window, cx))
        .title(header(title.into(), subtitle, palette, close))
}

fn header(
    title: SharedString,
    subtitle: Option<SharedString>,
    palette: Palette,
    close: Close,
) -> impl IntoElement {
    div()
        .w_full()
        .flex()
        .items_start()
        .gap_4()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_size(px(20.))
                        .line_height(px(26.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(palette.text))
                        .child(title),
                )
                .children(subtitle.map(|subtitle| {
                    div()
                        .truncate()
                        .text_size(px(14.))
                        .line_height(px(20.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(palette.muted))
                        .child(subtitle)
                })),
        )
        .child(
            div()
                .id("dialog-close")
                .size(px(28.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(rgb(palette.muted))
                .hover(move |button| button.bg(rgba(palette.press)).text_color(rgb(palette.text)))
                .child("✕")
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    close(window, cx);
                }),
        )
}

pub fn pill(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    prominent: bool,
    palette: Palette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px_3()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_full()
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .map(|pill| {
            if prominent {
                pill.bg(rgb(palette.accent))
                    .text_color(rgb(0xffffff))
                    .hover(|pill| pill.opacity(0.88))
            } else {
                pill.border_1()
                    .border_color(rgba(outline(palette)))
                    .text_color(rgb(palette.text))
                    .hover(move |pill| pill.bg(rgba(palette.press)))
            }
        })
        .child(label.into())
}

pub fn outline(palette: Palette) -> u32 {
    if palette.dark {
        palette.rim
    } else {
        0x00000029
    }
}

pub fn section_title(text: impl Into<SharedString>, palette: Palette) -> Div {
    div()
        .text_size(px(15.))
        .line_height(px(20.))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(palette.text))
        .child(text.into())
}

pub fn open_url(url: &str, cx: &mut App) {
    if url.starts_with("https://") || url.starts_with("http://") {
        cx.open_url(url);
    }
}
