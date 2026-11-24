pub mod components;
mod dock;
mod sidebar;

use gpui::{
    App, Context, Div, FontWeight, Window, WindowAppearance, div, prelude::*, px, rgb, rgba,
};
use gpui_component::{Theme, ThemeMode};

use crate::{
    app::{Account, OpenMusic, Page},
    glass, icons, overlay,
};
use components::icon;

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub canvas: u32,
    pub raised: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub line: u32,
    pub fill: u32,
    pub press: u32,
    pub rim: u32,
}

impl Palette {
    fn for_window(appearance: WindowAppearance) -> Self {
        let solid = reduce_transparency();
        if matches!(
            appearance,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ) {
            Self {
                dark: true,
                canvas: 0x212329,
                raised: 0x2a2930,
                text: 0xf5f5f7,
                muted: 0xa1a1aa,
                accent: system_accent().unwrap_or(0x3d8bff),
                line: 0xffffff1c,
                fill: if solid { 0x3a3940ff } else { 0xffffff17 },
                press: 0xffffff1f,
                rim: if solid { 0xffffff33 } else { 0xffffff2b },
            }
        } else {
            Self {
                dark: false,
                canvas: 0xececec,
                raised: 0xe9e8ee,
                text: 0x1d1d1f,
                muted: 0x6e6e73,
                accent: system_accent().unwrap_or(0x0a6cff),
                line: 0x0000001a,
                fill: if solid { 0xe8e8edff } else { 0xffffff8c },
                press: 0x00000012,
                rim: if solid { 0x00000026 } else { 0xffffffe6 },
            }
        }
    }

    fn glass(self, key: impl Into<String>, radius: f32) -> Div {
        let pane = div().relative().rounded(px(radius));
        if glass::available() {
            pane.child(glass::surface(key, radius))
        } else {
            pane.bg(rgba(self.fill))
                .border_1()
                .border_color(rgba(self.rim))
        }
    }
}

#[cfg(target_os = "macos")]
fn system_accent() -> Option<u32> {
    use objc2_app_kit::{NSColor, NSColorSpace};
    let color =
        NSColor::controlAccentColor().colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    let channel = |value: f64| (value.clamp(0., 1.) * 255.).round() as u32;
    Some(
        channel(color.redComponent()) << 16
            | channel(color.greenComponent()) << 8
            | channel(color.blueComponent()),
    )
}

#[cfg(not(target_os = "macos"))]
fn system_accent() -> Option<u32> {
    None
}

#[cfg(target_os = "macos")]
fn reduce_transparency() -> bool {
    let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
    workspace.accessibilityDisplayShouldReduceTransparency()
        || workspace.accessibilityDisplayShouldIncreaseContrast()
}

#[cfg(not(target_os = "macos"))]
fn reduce_transparency() -> bool {
    true
}

pub(super) fn apply_theme(appearance: WindowAppearance, cx: &mut App) {
    let palette = Palette::for_window(appearance);
    Theme::change(
        if palette.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    let theme = Theme::global_mut(cx);
    theme.background = rgba(0).into();
    theme.foreground = rgb(palette.text).into();
    theme.muted_foreground = rgb(palette.muted).into();
    theme.border = rgba(palette.line).into();
    theme.input = rgba(palette.line).into();
    theme.primary = rgb(palette.accent).into();
    theme.primary_hover = rgb(palette.accent).into();
    theme.primary_active = rgb(palette.accent).into();
    theme.slider_bar = rgb(palette.text).into();
    theme.slider_thumb = rgb(palette.text).into();
}

const WINDOW_INSET: f32 = 8.;

impl OpenMusic {
    fn sign_in(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let (headline, description) = match self.account {
            Account::Connecting => ("Connecting to Spotify", "This can take a few seconds."),
            Account::Pairing { .. } => (
                "Pair your account",
                "Open Spotify and enter this code if prompted.",
            ),
            _ => (
                "Listen with OpenMusic",
                "Connect your Spotify Premium account to get started.",
            ),
        };
        let controls = match &self.account {
            Account::Pairing { code, .. } => div()
                .flex()
                .flex_col()
                .items_center()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .font_family("Menlo")
                                .text_size(px(20.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(code.clone()),
                        )
                        .child(self.text_control("copy-code", "Copy", false, palette, cx)),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(self.text_control(
                            "open-pairing",
                            "Open Spotify in browser",
                            true,
                            palette,
                            cx,
                        ))
                        .child(self.text_control("cancel", "Cancel", false, palette, cx)),
                ),
            Account::Connecting => {
                div().child(self.text_control("cancel", "Cancel", false, palette, cx))
            }
            _ => {
                div().child(self.text_control("sign-in", "Sign in with Spotify", true, palette, cx))
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .gap_3()
            .px_8()
            .child(icon(icons::NOTE, 44., palette.accent))
            .child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child(headline),
            )
            .child(div().text_color(rgb(palette.muted)).child(description))
            .when_some(self.account_error.clone(), |page, error| {
                page.child(div().text_sm().text_color(rgb(palette.accent)).child(error))
            })
            .child(div().mt_2().child(controls))
    }
}

impl Render for OpenMusic {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::for_window(window.appearance());
        let content_width = window.viewport_size().width - px(248.);
        let card_side =
            ((content_width.min(px(1060.)) - px(80.) - px(48.)) / 4.).clamp(px(110.), px(200.));
        let connected = matches!(self.account, Account::Connected);
        let roomy = window.viewport_size().width >= px(1060.);

        let content = div().flex_1().min_h_0().map(|content| {
            if !connected {
                return content.child(self.sign_in(palette, cx));
            }
            if self.page == Page::Lyrics {
                return content.child(self.lyrics_page(palette, cx));
            }
            let scroll_id = match &self.page {
                Page::Artist(uri) | Page::Collection(uri) => format!("page-{uri}"),
                Page::Search => format!("search-{}", self.discovery.search_query),
                _ => "home".to_owned(),
            };
            content.child(
                div()
                    .id(gpui::ElementId::Name(scroll_id.into()))
                    .size_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .max_w(px(1060.))
                            .mx_auto()
                            .px(px(32.))
                            .pt(px(40.))
                            .pb(px(96.))
                            .map(|page| match self.page.clone() {
                                Page::Search => page.child(self.search_results(palette, cx)),
                                Page::Artist(uri) => {
                                    page.child(self.artist_page(&uri, card_side, palette, cx))
                                }
                                Page::Collection(uri) => {
                                    page.child(self.collection_page(&uri, palette, cx))
                                }
                                _ => page.child(self.discover(card_side, palette, cx)),
                            }),
                    ),
            )
        });

        div()
            .size_full()
            .relative()
            .flex()
            .when(!glass::available(), |root| {
                root.bg(rgb(if palette.dark { 0x121216 } else { 0xe9e8ee }))
            })
            .text_color(rgb(palette.text))
            .child(self.sidebar(palette, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .relative()
                    .flex()
                    .flex_col()
                    .pr(px(WINDOW_INSET))
                    .child(content)
                    .when(connected, |column| {
                        column
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .bottom_0()
                                    .h(px(132.))
                                    .bg(gpui::linear_gradient(
                                        180.,
                                        gpui::linear_color_stop(rgba(palette.canvas << 8), 0.),
                                        gpui::linear_color_stop(
                                            rgba(palette.canvas << 8 | 0xf0),
                                            0.62,
                                        ),
                                    )),
                            )
                            .child(self.dock(roomy, palette, cx))
                    }),
            )
            .child(glass::finish())
            .child(overlay::finish())
    }
}
