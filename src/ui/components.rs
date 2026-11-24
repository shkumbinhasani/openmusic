use std::sync::Arc;

use gpui::{
    AnimationExt, Context, Div, FontWeight, Image, ObjectFit, Pixels, Stateful, div, img,
    prelude::*, px, rgb, rgba, svg,
};
use spotify_client::Track;

use super::Palette;
use crate::{
    app::{OpenMusic, Page},
    icons,
    overlay::{self, Action, Label},
};

pub fn icon(path: &'static str, size: f32, color: u32) -> impl IntoElement {
    svg()
        .path(path)
        .size(px(size))
        .flex_shrink_0()
        .text_color(rgb(color))
}

pub fn link(
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

    pub fn rows(
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
    pub fn track_list(
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

    #[allow(clippy::type_complexity)]
    pub fn grid(
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
}
