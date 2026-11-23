use std::sync::Arc;

use gpui::{
    AnimationExt, App, Context, Div, FontWeight, Image, ObjectFit, Pixels, Stateful, Window,
    WindowAppearance, div, img, prelude::*, px, rgb, rgba, svg,
};
use gpui_component::{Theme, ThemeMode, input::Input};

use crate::{
    Account, OpenMusic, Page, glass, icons,
    overlay::{self, Action, Label, NowPlaying as NowPlayingBar},
    spotify::Track,
    timestamp,
};

#[derive(Clone, Copy)]
struct Palette {
    dark: bool,
    canvas: u32,
    raised: u32,
    text: u32,
    muted: u32,
    accent: u32,
    line: u32,
    fill: u32,
    press: u32,
    rim: u32,
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

fn icon(path: &'static str, size: f32, color: u32) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(size))
        .flex_shrink_0()
        .text_color(rgb(color))
}

fn link(
    id: (&'static str, usize),
    text: String,
    target: Option<Page>,
    size: f32,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> impl IntoElement {
    let label = div()
        .id(gpui::ElementId::NamedInteger(
            format!("{}-link", id.0).into(),
            id.1 as u64,
        ))
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

fn artwork(image: Option<&Arc<Image>>, side: Pixels, radius: Pixels, palette: Palette) -> Div {
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

const WINDOW_INSET: f32 = 8.;
const SIDEBAR_PADDING: f32 = 10.;
const ROW_RADIUS: f32 = 8.;

const SHIMMER_PERIOD: std::time::Duration = std::time::Duration::from_millis(1500);

fn bone(key: &'static str, index: usize, palette: Palette) -> Div {
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

fn skeleton_rows(key: &'static str, count: usize, numbered: bool, palette: Palette) -> Div {
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

fn skeleton_header(key: &'static str, round: bool, palette: Palette) -> Div {
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

fn fallback_icon(symbol: &str) -> &'static str {
    match symbol {
        "backward.fill" => icons::PREVIOUS,
        "forward.fill" => icons::NEXT,
        "pause.fill" => icons::PAUSE,
        "quote.bubble" | "quote.bubble.fill" => icons::LYRICS,
        "chevron.left" => icons::BACK,
        _ => icons::PLAY,
    }
}

impl OpenMusic {
    fn control(
        &self,
        key: &'static str,
        label: Label,
        prominent: bool,
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
            return slot.child(overlay::button(key, label, prominent));
        }
        slot.rounded_full()
            .bg(rgba(if prominent {
                palette.accent << 8 | 0xff
            } else {
                palette.fill
            }))
            .border_1()
            .border_color(rgba(palette.rim))
            .cursor_pointer()
            .hover(move |button| button.opacity(0.85))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .on_click(
                cx.listener(move |this, _, _, cx| this.handle_control(Action::Press(key), cx)),
            )
            .map(|button| match label {
                Label::Symbol(name) => button.child(icon(fallback_icon(name), 20., palette.text)),
                Label::Text(text) => button.child(text),
            })
    }

    fn text_control(
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

    fn cover_for(&self, track: &Track) -> Option<&Arc<Image>> {
        track.cover.as_ref().and_then(|url| self.covers.get(url))
    }

    fn sidebar(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = matches!(self.account, Account::Connected);
        let nav = |id: &'static str, path: &'static str, label: &'static str, selected: bool| {
            div()
                .id(id)
                .h(px(34.))
                .flex()
                .items_center()
                .gap_2p5()
                .px_2p5()
                .rounded(px(ROW_RADIUS))
                .cursor_pointer()
                .text_size(px(13.5))
                .font_weight(if selected {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .when(selected, |item| item.bg(rgba(palette.press)))
                .hover(move |item| item.bg(rgba(palette.press)))
                .child(icon(
                    path,
                    16.,
                    if selected {
                        palette.accent
                    } else {
                        palette.muted
                    },
                ))
                .child(label)
        };

        palette
            .glass("sidebar", 0.)
            .w(px(232.))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_4()
            .pt(px(48.))
            .pb(px(SIDEBAR_PADDING))
            .px(px(SIDEBAR_PADDING))
            .when(connected, |bar| {
                bar.child(
                    div()
                        .h(px(34.))
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .pl_3()
                        .pr_1()
                        .rounded_full()
                        .bg(rgba(palette.press))
                        .child(icon(icons::SEARCH, 14., palette.muted))
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&self.input)
                                    .appearance(false)
                                    .bordered(false)
                                    .focus_bordered(false),
                            ),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(
                            nav(
                                "nav-home",
                                icons::HOME,
                                "Listen Now",
                                self.page == Page::Home,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.navigate(Page::Home);
                                cx.notify();
                            })),
                        )
                        .child(
                            nav(
                                "nav-search",
                                icons::SEARCH,
                                "Search",
                                self.page == Page::Search,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    if this.search_query.is_empty() {
                                        this.input.update(cx, |input, cx| input.focus(window, cx));
                                    } else {
                                        this.navigate(Page::Search);
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .child(
                            nav(
                                "nav-lyrics",
                                icons::LYRICS,
                                "Lyrics",
                                self.page == Page::Lyrics,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_lyrics(true);
                                cx.notify();
                            })),
                        ),
                )
                .child(self.playlist_list(palette, cx))
                .child(self.device_panel(palette, cx))
            })
    }

    fn device_panel(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let remote = self.now.device.clone().filter(|_| !self.now.local);
        let (caption, name) = match (&remote, self.now.local, self.now.track.is_some()) {
            (Some(name), _, _) => ("Playing on", name.clone()),
            (None, true, _) => ("Playing on", "This Mac".to_owned()),
            (None, false, _) => ("Spotify Connect", "No device is active".to_owned()),
        };
        let color = if remote.is_some() {
            palette.accent
        } else {
            palette.muted
        };
        div()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .p(px(10.))
            .rounded(px(ROW_RADIUS + 4.))
            .bg(rgba(palette.press))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .child(icon(icons::DEVICE, 18., color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(palette.muted))
                                    .child(caption),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(if remote.is_some() {
                                        palette.accent
                                    } else {
                                        palette.text
                                    }))
                                    .truncate()
                                    .child(name),
                            ),
                    ),
            )
            .when(remote.is_some(), |panel| {
                panel.child(
                    self.text_control("play-here", "Play on This Mac", true, palette, cx)
                        .h(px(30.))
                        .w_full(),
                )
            })
    }

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

    fn rows(
        &self,
        tracks: &[Track],
        prefix: &'static str,
        start: usize,
        context: Option<&str>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        self.track_list(tracks, prefix, start, context, false, palette, cx)
    }

    #[allow(clippy::too_many_arguments)]
    fn track_list(
        &self,
        tracks: &[Track],
        prefix: &'static str,
        start: usize,
        context: Option<&str>,
        numbered: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let queue: Arc<[Track]> = tracks.into();
        let context: Option<Arc<str>> = context.map(Into::into);
        div()
            .flex()
            .flex_col()
            .children(tracks.iter().enumerate().skip(start).map(|(index, track)| {
                let queue = queue.clone();
                let context = context.clone();
                let selected = self.is_current(&track.uri);
                let artist_uri = track.artist_uri.clone();
                let album_uri = track.album_uri.clone();
                div()
                    .id((prefix, index))
                    .h(px(56.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_2()
                    .rounded(px(10.))
                    .cursor_pointer()
                    .when(selected, |row| row.bg(rgba(palette.press)))
                    .hover(move |row| row.bg(rgba(palette.press)))
                    .on_click(cx.listener(move |this, _, _, cx| match &context {
                        Some(context) => this.play_context(context, &queue, index, cx),
                        None => this.play(&queue, index, cx),
                    }))
                    .map(|row| {
                        if numbered {
                            row.child(
                                div()
                                    .w(px(40.))
                                    .flex_shrink_0()
                                    .text_center()
                                    .text_size(px(13.))
                                    .text_color(rgb(if selected {
                                        palette.accent
                                    } else {
                                        palette.muted
                                    }))
                                    .child(if selected {
                                        "♪".to_owned()
                                    } else {
                                        (index + 1).to_string()
                                    }),
                            )
                        } else {
                            row.child(artwork(self.cover_for(track), px(40.), px(6.), palette))
                        }
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::MEDIUM)
                                    .when(selected, |title| title.text_color(rgb(palette.accent)))
                                    .truncate()
                                    .child(track.title.clone()),
                            )
                            .child(div().flex().min_w_0().child(link(
                                (prefix, index * 2),
                                track.artist.clone(),
                                artist_uri.map(Page::Artist),
                                12.,
                                palette,
                                cx,
                            ))),
                    )
                    .when(!numbered, |row| {
                        row.child(div().w_1_4().flex().min_w_0().child(link(
                            (prefix, index * 2 + 1),
                            track.album.clone(),
                            album_uri.map(Page::Collection),
                            12.5,
                            palette,
                            cx,
                        )))
                    })
                    .child(
                        div()
                            .w(px(48.))
                            .text_size(px(12.5))
                            .text_color(rgb(palette.muted))
                            .text_right()
                            .child(timestamp(track.duration_ms)),
                    )
            }))
    }

    fn discover(
        &self,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mix = self.newest_mix.as_ref().or(self.home.mix.as_ref());
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child("Listen Now"),
            )
            .when_some(mix, |page, mix| {
                let featured = &mix.tracks[0];
                let hero = div()
                    .flex()
                    .items_center()
                    .gap_6()
                    .child(
                        artwork(self.cover_for(featured), px(176.), px(16.), palette).shadow_lg(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap_1p5()
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(palette.accent))
                                    .truncate()
                                    .w_full()
                                    .child(format!(
                                        "STATION · INSPIRED BY {}",
                                        mix.seed.to_uppercase()
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(28.))
                                    .font_weight(FontWeight::BOLD)
                                    .truncate()
                                    .w_full()
                                    .child(featured.title.clone()),
                            )
                            .child(
                                div()
                                    .text_color(rgb(palette.muted))
                                    .truncate()
                                    .w_full()
                                    .child(featured.artist.clone()),
                            )
                            .child(
                                div().mt_3().child(
                                    self.text_control("play-station", "Play", true, palette, cx)
                                        .w(px(96.)),
                                ),
                            ),
                    );
                let page = page.child(hero);
                if mix.tracks.len() > 1 {
                    page.child(self.mix_tracks(&mix.tracks, card_side, palette, cx))
                } else {
                    page
                }
            })
            .when(!self.home.tracks.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child(if self.home.from_top_tracks {
                                    "In Your Rotation"
                                } else {
                                    "From Your Liked Songs"
                                }),
                        )
                        .child(self.rows(
                            &self.home.tracks,
                            "home-track",
                            0,
                            Some("spotify:collection:tracks"),
                            palette,
                            cx,
                        )),
                )
            })
            .when(mix.is_none() && self.home.tracks.is_empty(), |page| {
                if self.discovery_pending {
                    page.child(skeleton_header("home-hero", false, palette))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .child(
                                    bone("home-title", 0, palette)
                                        .w(px(220.))
                                        .h(px(20.))
                                        .rounded(px(5.)),
                                )
                                .child(div().flex().gap_4().children((0..4).map(|index| {
                                    div()
                                        .flex_1()
                                        .flex()
                                        .flex_col()
                                        .gap_2()
                                        .child(
                                            bone("home-card", index * 3, palette)
                                                .size(card_side)
                                                .rounded(px(12.)),
                                        )
                                        .child(
                                            bone("home-card", index * 3 + 1, palette)
                                                .w(gpui::relative(0.7))
                                                .h(px(12.))
                                                .rounded(px(4.)),
                                        )
                                        .child(
                                            bone("home-card", index * 3 + 2, palette)
                                                .w(gpui::relative(0.45))
                                                .h(px(11.))
                                                .rounded(px(4.)),
                                        )
                                }))),
                        )
                        .child(skeleton_rows("home-rows", 4, false, palette))
                } else {
                    page.child(
                        div()
                            .py_4()
                            .text_color(rgb(palette.muted))
                            .child("Play a song to get recommendations from Spotify."),
                    )
                }
            })
    }

    fn mix_tracks(
        &self,
        tracks: &[Track],
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let queue: Arc<[Track]> = tracks.into();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(19.))
                    .font_weight(FontWeight::BOLD)
                    .child("Up Next on This Station"),
            )
            .child(
                div()
                    .flex()
                    .gap_4()
                    .children(
                        tracks
                            .iter()
                            .enumerate()
                            .skip(1)
                            .take(4)
                            .map(|(index, track)| {
                                let queue = queue.clone();
                                div()
                                    .id(("mix-card", index))
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_2()
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.play(&queue, index, cx)
                                    }))
                                    .child(
                                        artwork(self.cover_for(track), card_side, px(12.), palette)
                                            .shadow_md(),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .truncate()
                                                    .child(track.title.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(12.))
                                                    .text_color(rgb(palette.muted))
                                                    .truncate()
                                                    .child(track.artist.clone()),
                                            ),
                                    )
                            }),
                    ),
            )
            .when(tracks.len() > 5, |section| {
                section.child(self.rows(tracks, "mix-track", 5, None, palette, cx))
            })
    }

    fn search_results(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .child("Search"),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette.muted))
                            .truncate()
                            .child(format!("Results for “{}”", self.search_query)),
                    ),
            )
            .when(self.results.is_empty(), |page| {
                if self.search_pending {
                    return page.child(skeleton_rows("search-rows", 9, false, palette));
                }
                page.child(
                    div().py_4().text_color(rgb(palette.muted)).child(
                        self.search_error
                            .clone()
                            .unwrap_or_else(|| "No songs found.".into()),
                    ),
                )
            })
            .child(self.rows(&self.results, "search-track", 0, None, palette, cx))
    }

    fn dock(&self, roomy: bool, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let track = self.track();
        let title = track
            .map(|track| track.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Not playing".into());
        let subtitle = self
            .notice
            .clone()
            .or_else(|| track.map(|track| track.artist.clone()))
            .unwrap_or_default();
        let toggle = if self.now.playing {
            "pause.fill"
        } else {
            "play.fill"
        };
        let state = NowPlayingBar {
            title: title.clone(),
            subtitle: subtitle.clone(),
            alert: self.notice.is_some(),
            cover: track.and_then(|track| {
                let url = track.cover.clone()?;
                let image = self.covers.get(&url)?.clone();
                Some((url, image))
            }),
            position_ms: self.now.position(),
            duration_ms: self.duration().unwrap_or(0),
        };
        let volume = f32::from(self.now.volume) / f32::from(u16::MAX);

        div()
            .id("dock")
            .occlude()
            .absolute()
            .left(px(WINDOW_INSET))
            .right(px(WINDOW_INSET))
            .bottom(px(WINDOW_INSET))
            .h(px(60.))
            .flex()
            .items_center()
            .gap_2p5()
            .child(
                self.control(
                    "previous",
                    Label::Symbol("backward.fill"),
                    false,
                    palette,
                    cx,
                )
                .size(px(48.)),
            )
            .child(
                self.control("toggle-playback", Label::Symbol(toggle), false, palette, cx)
                    .size(px(60.)),
            )
            .child(
                self.control("next", Label::Symbol("forward.fill"), false, palette, cx)
                    .size(px(48.)),
            )
            .child(div().relative().flex_1().min_w_0().h_full().map(|bar| {
                if overlay::available() {
                    bar.child(overlay::now_playing(state))
                } else {
                    bar.flex()
                        .flex_col()
                        .justify_center()
                        .px_5()
                        .rounded_full()
                        .bg(rgba(palette.fill))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(rgb(palette.muted))
                                .truncate()
                                .child(subtitle),
                        )
                }
            }))
            .child(
                self.control(
                    "lyrics",
                    Label::Symbol(if self.page == Page::Lyrics {
                        "quote.bubble.fill"
                    } else {
                        "quote.bubble"
                    }),
                    false,
                    palette,
                    cx,
                )
                .size(px(48.)),
            )
            .when(overlay::available() && roomy, |dock| {
                dock.child(
                    div()
                        .relative()
                        .w(px(168.))
                        .h_full()
                        .flex_shrink_0()
                        .child(overlay::volume(volume, f32::MAX)),
                )
            })
    }
}

impl OpenMusic {
    fn lyrics_page(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let track = self.track();
        let lyrics = self.current_lyrics();
        let synced = lyrics.is_some_and(|lyrics| lyrics.synced);
        let lines = lyrics
            .map(|lyrics| lyrics.lines.as_slice())
            .unwrap_or_default();
        let active = self.lyrics_line;
        let status = match (track, lyrics) {
            (None, _) => Some("Play a song to see its lyrics."),
            (Some(_), None) if self.lyrics_loading => None,
            (Some(_), None) => Some("Lyrics aren’t available for this song."),
            (Some(_), Some(_)) => None,
        };
        let heading = track
            .map(|track| {
                if track.artist.is_empty() {
                    track.title.clone()
                } else {
                    format!("{} · {}", track.title, track.artist)
                }
            })
            .unwrap_or_default();

        div()
            .id("lyrics")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.lyrics_scroll)
            .px(px(48.))
            .pt(px(40.))
            .pb(px(320.))
            .child(
                div()
                    .pb_8()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::BOLD)
                            .child("Lyrics"),
                    )
                    .child(
                        div()
                            .text_color(rgb(palette.muted))
                            .truncate()
                            .child(heading),
                    )
                    .when_some(status, |header, status| {
                        header.child(
                            div()
                                .pt_6()
                                .text_size(px(17.))
                                .text_color(rgb(palette.muted))
                                .child(status),
                        )
                    }),
            )
            .when(
                self.lyrics_loading && lines.is_empty() && track.is_some(),
                |page| {
                    page.child(
                        div().flex().flex_col().gap(px(22.)).px_3().children(
                            [0.62, 0.48, 0.7, 0.4, 0.56, 0.66, 0.36, 0.52]
                                .into_iter()
                                .enumerate()
                                .map(|(index, width)| {
                                    bone("lyric-line", index, palette)
                                        .w(gpui::relative(width * 0.85))
                                        .max_w(px(640.))
                                        .h(px(24.))
                                        .rounded(px(6.))
                                }),
                        ),
                    )
                },
            )
            .children(lines.iter().enumerate().map(|(index, line)| {
                let current = active == Some(index);
                let start = line.start_ms;
                let pause = line.words.is_empty();
                let (size, height) = match (pause, current) {
                    (true, _) => (18., 26.),
                    (false, true) => (30., 38.),
                    (false, false) => (26., 34.),
                };
                div()
                    .id(("lyric", index))
                    .max_w(px(760.))
                    .px_3()
                    .py(px(if pause { 4. } else { 8. }))
                    .rounded(px(12.))
                    .text_size(px(size))
                    .line_height(px(height))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(if current || !synced {
                        palette.text
                    } else {
                        palette.muted
                    }))
                    .when(synced && !current, |line| line.opacity(0.55))
                    .when(synced, |line| {
                        line.cursor_pointer()
                            .hover(move |line| line.bg(rgba(palette.press)).opacity(1.))
                            .on_click(cx.listener(move |this, _, _, cx| this.seek_to(start, cx)))
                    })
                    .child(if pause {
                        "• • •".to_owned()
                    } else {
                        line.words.clone()
                    })
            }))
            .when_some(
                lyrics.filter(|lyrics| !lyrics.provider.is_empty()),
                |page, lyrics| {
                    page.child(
                        div()
                            .px_3()
                            .pt_8()
                            .text_size(px(12.))
                            .text_color(rgb(palette.muted))
                            .child(format!("Lyrics provided by {}", lyrics.provider)),
                    )
                },
            )
    }
}

impl OpenMusic {
    fn playlist_list(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                div()
                    .px_2p5()
                    .pb_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.muted))
                    .child("Playlists"),
            )
            .child(
                div()
                    .id("sidebar-playlists")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(
                        self.playlists.is_empty() && self.playlists_loading,
                        |list| {
                            list.children((0..7).map(|index| {
                                div()
                                    .h(px(36.))
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .px_1p5()
                                    .child(
                                        bone("sidebar-playlist", index * 2, palette)
                                            .size(px(26.))
                                            .rounded(px(5.)),
                                    )
                                    .child(
                                        bone("sidebar-playlist", index * 2 + 1, palette)
                                            .w(px([120., 90., 140., 104., 128., 84., 112.][index]))
                                            .h(px(11.))
                                            .rounded(px(4.)),
                                    )
                            }))
                        },
                    )
                    .children(self.playlists.iter().enumerate().map(|(index, playlist)| {
                        let page = Page::Collection(playlist.uri.clone());
                        let selected = self.page == page;
                        let image = playlist.cover.as_ref().and_then(|url| self.covers.get(url));
                        div()
                            .id(("playlist", index))
                            .h(px(36.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .px_1p5()
                            .rounded(px(ROW_RADIUS))
                            .cursor_pointer()
                            .when(selected, |item| {
                                item.bg(rgba(palette.press))
                                    .font_weight(FontWeight::SEMIBOLD)
                            })
                            .hover(move |item| item.bg(rgba(palette.press)))
                            .child(artwork(image, px(26.), px(5.), palette))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .child(playlist.name.clone()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.navigate(page.clone());
                                cx.notify();
                            }))
                    })),
            )
    }

    fn back_button(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        self.control("back", Label::Symbol("chevron.left"), false, palette, cx)
            .size(px(40.))
    }

    fn page_status(&self, text: impl Into<gpui::SharedString>, palette: Palette) -> Div {
        div()
            .py_8()
            .text_color(rgb(palette.muted))
            .child(text.into())
    }

    fn artist_page(
        &self,
        uri: &str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)));
        let Some((loaded, result)) = self.artist.as_ref().filter(|(loaded, _)| loaded == uri)
        else {
            return page
                .child(skeleton_header("artist-header", true, palette))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            bone("artist-title", 0, palette)
                                .w(px(110.))
                                .h(px(20.))
                                .rounded(px(5.)),
                        )
                        .child(skeleton_rows("artist-rows", 6, false, palette)),
                );
        };
        let artist = match result {
            Ok(artist) => artist,
            Err(error) => return page.child(self.page_status(error.clone(), palette)),
        };
        let image = artist.image.as_ref().and_then(|url| self.covers.get(url));
        let header = div()
            .flex()
            .items_center()
            .gap_6()
            .child(
                div()
                    .size(px(176.))
                    .flex_shrink_0()
                    .rounded_full()
                    .overflow_hidden()
                    .shadow_lg()
                    .child(artwork(image, px(176.), px(88.), palette)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1p5()
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.accent))
                            .child("ARTIST"),
                    )
                    .child(
                        div()
                            .text_size(px(40.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .w_full()
                            .child(artist.name.clone()),
                    )
                    .child(
                        div().mt_3().child(
                            self.text_control("play-artist", "Play", true, palette, cx)
                                .w(px(96.)),
                        ),
                    ),
            );
        let releases = artist.releases.clone();
        let related = artist.related.clone();
        page.child(header)
            .when(!artist.top_tracks.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child("Popular"),
                        )
                        .child(self.rows(
                            &artist.top_tracks,
                            "artist-track",
                            0,
                            Some(loaded),
                            palette,
                            cx,
                        )),
                )
            })
            .when(!releases.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child("Releases"),
                        )
                        .child(
                            self.grid(
                                releases
                                    .iter()
                                    .map(|release| {
                                        (
                                            release.uri.clone(),
                                            release.name.clone(),
                                            [
                                                release.year.map(|year| year.to_string()),
                                                Some(release.kind.clone()),
                                            ]
                                            .into_iter()
                                            .flatten()
                                            .collect::<Vec<_>>()
                                            .join(" · "),
                                            release.cover.clone(),
                                            Page::Collection(release.uri.clone()),
                                        )
                                    })
                                    .collect(),
                                "release",
                                card_side,
                                false,
                                palette,
                                cx,
                            ),
                        ),
                )
            })
            .when_some(artist.biography.clone(), |page, biography| {
                let short: String = biography.chars().take(900).collect();
                let short = if short.len() < biography.len() {
                    format!("{short}…")
                } else {
                    short
                };
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child("About"),
                        )
                        .child(
                            div()
                                .max_w(px(760.))
                                .text_size(px(14.))
                                .line_height(px(22.))
                                .text_color(rgb(palette.muted))
                                .child(short),
                        ),
                )
            })
            .when(!related.is_empty(), |page| {
                page.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(19.))
                                .font_weight(FontWeight::BOLD)
                                .child("Fans Also Like"),
                        )
                        .child(
                            self.grid(
                                related
                                    .iter()
                                    .map(|related| {
                                        (
                                            related.uri.clone(),
                                            related.name.clone(),
                                            "Artist".to_owned(),
                                            related.image.clone(),
                                            Page::Artist(related.uri.clone()),
                                        )
                                    })
                                    .collect(),
                                "related",
                                card_side,
                                true,
                                palette,
                                cx,
                            ),
                        ),
                )
            })
    }

    #[allow(clippy::type_complexity)]
    fn grid(
        &self,
        items: Vec<(String, String, String, Option<String>, Page)>,
        prefix: &'static str,
        card_side: Pixels,
        round: bool,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_wrap()
            .gap_4()
            .children(items.into_iter().enumerate().map(
                move |(index, (_, title, subtitle, cover, page))| {
                    let image = cover.as_ref().and_then(|url| self.covers.get(url));
                    let radius = if round { card_side / 2. } else { px(12.) };
                    div()
                        .id((prefix, index))
                        .w(card_side)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .cursor_pointer()
                        .child(artwork(image, card_side, radius, palette).shadow_md())
                        .child(
                            div()
                                .w_full()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .truncate()
                                        .child(title),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(rgb(palette.muted))
                                        .truncate()
                                        .child(subtitle),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.navigate(page.clone());
                            cx.notify();
                        }))
                },
            ))
    }

    fn collection_page(
        &self,
        uri: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)));
        let Some((loaded, result)) = self.collection.as_ref().filter(|(loaded, _)| loaded == uri)
        else {
            let numbered = uri.starts_with("spotify:album:");
            return page
                .child(skeleton_header("collection-header", false, palette))
                .child(skeleton_rows("collection-rows", 8, numbered, palette));
        };
        let collection = match result {
            Ok(collection) => collection,
            Err(error) => return page.child(self.page_status(error.clone(), palette)),
        };
        let image = collection
            .cover
            .as_ref()
            .and_then(|url| self.covers.get(url));
        let artist_page = collection.artist_uri.clone().map(Page::Artist);
        page.child(
            div()
                .flex()
                .items_center()
                .gap_6()
                .child(artwork(image, px(184.), px(16.), palette).shadow_lg())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_1p5()
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(palette.accent))
                                .child(collection.kind.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(34.))
                                .font_weight(FontWeight::BOLD)
                                .truncate()
                                .w_full()
                                .child(collection.name.clone()),
                        )
                        .child(div().w_full().flex().min_w_0().child(link(
                            ("collection-subtitle", 0),
                            collection.subtitle.clone(),
                            artist_page,
                            13.5,
                            palette,
                            cx,
                        )))
                        .child(
                            div().mt_3().child(
                                self.text_control("play-collection", "Play", true, palette, cx)
                                    .w(px(96.)),
                            ),
                        ),
                ),
        )
        .child(if collection.tracks.is_empty() {
            self.page_status("This is empty.", palette)
        } else {
            self.track_list(
                &collection.tracks,
                "collection-track",
                0,
                Some(loaded),
                collection.kind == "ALBUM",
                palette,
                cx,
            )
        })
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
                Page::Search => format!("search-{}", self.search_query),
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
