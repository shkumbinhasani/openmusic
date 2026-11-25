pub mod cards;
pub mod components;
pub mod dock;
pub mod gallery;
pub mod menu;
pub mod sidebar;
pub mod tracks;

use std::path::Path;

use gpui::{
    AnyElement, App, Context, Div, FontWeight, Pixels, Window, WindowAppearance, div, prelude::*,
    px, rgb, rgba,
};
use gpui_component::{Root, Theme, ThemeMode, WindowExt};

use crate::{
    app::{OpenMusic, Page, Session},
    glass, icons, overlay,
};
use components::icon;
use dock::BAR_WEIGHT;

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

    pub fn glass(self, key: impl Into<String>, radius: f32) -> Div {
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
const DOCK_HEIGHT: f32 = 60.;
const SIDEBAR_SPACE: f32 = 248.;
pub const ROOMY_WIDTH: f32 = 1060.;

pub struct Panel {
    pub width: f32,
    pub element: AnyElement,
}

struct Layout {
    roomy: bool,
    dock_roomy: bool,
    card_side: Pixels,
    panel: Option<Panel>,
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
        "layout" => {
            let palette = Palette::for_window(window.appearance());
            let width = window.viewport_size().width;
            let layout = app.layout(width, palette, cx);
            let mut lines = vec![format!(
                "width={} roomy={} dock_roomy={} card_side={}",
                f32::from(width),
                layout.roomy,
                layout.dock_roomy,
                f32::from(layout.card_side),
            )];
            for entry in app.nav_entries() {
                let selected = if entry.selected { " selected" } else { "" };
                lines.push(format!(
                    "nav {} {} {}{selected}",
                    entry.key, entry.weight, entry.label
                ));
            }
            let mut bar = false;
            for item in app.dock_items(layout.dock_roomy, palette, cx) {
                if !bar && item.weight >= BAR_WEIGHT {
                    lines.push(format!("bar {BAR_WEIGHT}"));
                    bar = true;
                }
                lines.push(format!("dock {} {} {}", item.key, item.weight, item.width));
            }
            if !bar {
                lines.push(format!("bar {BAR_WEIGHT}"));
            }
            lines.push(match &layout.panel {
                Some(panel) => format!("panel {}", panel.width),
                None => "panel none".to_owned(),
            });
            let _ = std::fs::write(directory.join("layout.txt"), lines.join("\n"));
        }
        "drag" => {
            let numbers: Vec<f64> = rest
                .split_whitespace()
                .filter_map(|part| part.parse().ok())
                .collect();
            if let [x1, y1, x2, y2] = numbers[..] {
                overlay::drag((x1, y1), (x2, y2));
            }
        }
        "gallery" => gallery::debug(app, rest, directory),
        _ => return false,
    }
    true
}

impl OpenMusic {
    fn sign_in(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let (headline, description) = match self.session {
            Session::Connecting => ("Connecting to Spotify", "This can take a few seconds."),
            Session::Pairing { .. } => (
                "Pair your account",
                "Open Spotify and enter this code if prompted.",
            ),
            _ => (
                "Listen with OpenMusic",
                "Connect your Spotify Premium account to get started.",
            ),
        };
        let controls = match &self.session {
            Session::Pairing { code, .. } => div()
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
            Session::Connecting => {
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
            .when_some(self.session_error.clone(), |page, error| {
                page.child(div().text_sm().text_color(rgb(palette.accent)).child(error))
            })
            .child(div().mt_2().child(controls))
    }

    fn layout(&self, width: Pixels, palette: Palette, cx: &mut Context<Self>) -> Layout {
        let roomy = width >= px(ROOMY_WIDTH);
        let connected = matches!(self.session, Session::Connected);
        let panel = if roomy && connected {
            self.right_panel(palette, cx)
                .or_else(|| gallery::panel(self, palette))
        } else {
            None
        };
        let panel_width = px(panel.as_ref().map_or(0., |panel| panel.width));
        let content_width = width - px(SIDEBAR_SPACE) - panel_width;
        let card_side =
            ((content_width.min(px(1060.)) - px(80.) - px(48.)) / 4.).clamp(px(110.), px(200.));
        Layout {
            roomy,
            dock_roomy: width - panel_width >= px(ROOMY_WIDTH),
            card_side,
            panel,
        }
    }

    fn page_body(&self, card_side: Pixels, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        if self.gallery.is_some() {
            return gallery::page(self, card_side, palette, cx);
        }
        match self.page.clone() {
            Page::Home => self.discover(card_side, palette, cx).into_any_element(),
            Page::Search(tab) => self
                .search_results(tab, card_side, palette, cx)
                .into_any_element(),
            Page::Browse(uri) => self.browse_page(&uri, card_side, palette, cx),
            Page::Section { uri, source } => {
                self.section_page(&uri, source, card_side, palette, cx)
            }
            Page::Artist(uri) => self
                .artist_page(&uri, card_side, palette, cx)
                .into_any_element(),
            Page::Collection(uri) => self.collection_page(&uri, palette, cx).into_any_element(),
            Page::BlendInvitation(id) => self.blend_invitation_page(&id, card_side, palette, cx),
            Page::Track(uri) => self.track_page(&uri, card_side, palette, cx),
            Page::Queue => self.queue_page(card_side, palette, cx),
            Page::Jam => self.jam_page(card_side, palette, cx),
            Page::JamInvite(uri) => self.jam_invite_page(&uri, card_side, palette, cx),
            Page::Liked => self.liked_page(card_side, palette, cx),
            Page::Recent => self.recent_page(card_side, palette, cx),
            Page::Folder(uri) => self.folder_page(&uri, card_side, palette, cx),
            Page::Library(shelf) => self.library_page(shelf, card_side, palette, cx),
            Page::Lyrics => self.lyrics_page(palette, cx).into_any_element(),
            Page::Show(uri) => self.show_page(&uri, card_side, palette, cx),
            Page::Episode(uri) => self.episode_page(&uri, card_side, palette, cx),
            Page::YourEpisodes => self.your_episodes_page(card_side, palette, cx),
            Page::NowPlaying => self.now_playing_page(card_side, palette, cx),
            Page::Settings => self.settings_page(card_side, palette, cx),
            Page::Profile(uri) => self.profile_page(&uri, card_side, palette, cx),
            Page::ProfileList(uri, kind) => {
                self.profile_list_page(&uri, kind, card_side, palette, cx)
            }
            Page::LocalFiles => self.local_files_page(card_side, palette, cx),
        }
    }

    fn toast_layer(&self, palette: Palette, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let toast = self.toast.as_ref()?;
        Some(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(WINDOW_INSET + DOCK_HEIGHT + 14.))
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("toast")
                        .occlude()
                        .h(px(36.))
                        .max_w(px(520.))
                        .flex()
                        .items_center()
                        .gap_3()
                        .px_4()
                        .rounded_full()
                        .bg(rgb(palette.raised))
                        .border_1()
                        .border_color(rgba(palette.line))
                        .shadow_lg()
                        .text_size(px(13.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(div().min_w_0().truncate().child(toast.text.clone()))
                        .when(toast.undo.is_some(), |pill| {
                            pill.child(
                                div()
                                    .id("toast-undo")
                                    .flex_shrink_0()
                                    .cursor_pointer()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.accent))
                                    .child("Undo")
                                    .on_click(cx.listener(|this, _, _, cx| this.undo_toast(cx))),
                            )
                        }),
                ),
        )
    }
}

impl Render for OpenMusic {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.modal = window.has_active_dialog(cx) || window.has_active_sheet(cx);
        self.refresh_bar_menu();
        let palette = Palette::for_window(window.appearance());
        let Layout {
            dock_roomy,
            card_side,
            panel,
            ..
        } = self.layout(window.viewport_size().width, palette, cx);
        let connected = matches!(self.session, Session::Connected);

        let content = div().flex_1().min_h_0().map(|content| {
            if !connected {
                return content.child(self.sign_in(palette, cx));
            }
            if self.page.fills_content() && self.gallery.is_none() {
                return content.child(self.page_body(card_side, palette, cx));
            }
            let scroll_id = match &self.page {
                _ if self.gallery.is_some() => "gallery".to_owned(),
                Page::Home => "home".to_owned(),
                Page::Search(_) => format!("search-{}", self.discovery.search_query),
                Page::Artist(uri) | Page::Collection(uri) => format!("page-{uri}"),
                page => format!("page-{}", page.describe()),
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
                            .child(self.page_body(card_side, palette, cx)),
                    ),
            )
        });

        div()
            .size_full()
            .relative()
            .flex()
            .key_context("OpenMusic")
            .track_focus(&self.focus)
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
                            .child(self.dock(dock_roomy, palette, cx))
                            .children(self.toast_layer(palette, cx))
                    }),
            )
            .children(panel.map(|panel| {
                div()
                    .w(px(panel.width))
                    .h_full()
                    .flex_shrink_0()
                    .child(panel.element)
            }))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
            .child(glass::finish())
            .child(overlay::finish())
    }
}
