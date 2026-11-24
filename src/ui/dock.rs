use gpui::{Context, FontWeight, div, prelude::*, px, rgb, rgba};

use super::{DOCK_HEIGHT, Palette, WINDOW_INSET};
use crate::{
    app::{OpenMusic, Page},
    overlay::{self, Label, NowPlaying as NowPlayingBar},
};

impl OpenMusic {
    pub fn dock(&self, roomy: bool, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let track = self.track();
        let title = track
            .map(|track| track.title.clone())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Not playing".into());
        let subtitle = self
            .playback
            .notice
            .clone()
            .or_else(|| track.map(|track| track.artist.clone()))
            .unwrap_or_default();
        let toggle = if self.playback.now.playing {
            "pause.fill"
        } else {
            "play.fill"
        };
        let state = NowPlayingBar {
            title: title.clone(),
            subtitle: subtitle.clone(),
            alert: self.playback.notice.is_some(),
            cover: track.and_then(|track| {
                let url = track.cover.clone()?;
                let image = self.covers.get(&url)?.clone();
                Some((url, image))
            }),
            position_ms: self.playback.now.position(),
            duration_ms: self.duration().unwrap_or(0),
        };
        let volume = f32::from(self.playback.now.volume) / f32::from(u16::MAX);

        div()
            .id("dock")
            .occlude()
            .absolute()
            .left(px(WINDOW_INSET))
            .right(px(WINDOW_INSET))
            .bottom(px(WINDOW_INSET))
            .h(px(DOCK_HEIGHT))
            .flex()
            .items_center()
            .gap_2p5()
            .child(
                self.dock_control(
                    "previous",
                    Label::Symbol("backward.fill"),
                    false,
                    palette,
                    cx,
                )
                .size(px(48.)),
            )
            .child(
                self.dock_control("toggle-playback", Label::Symbol(toggle), false, palette, cx)
                    .size(px(60.)),
            )
            .child(
                self.dock_control("next", Label::Symbol("forward.fill"), false, palette, cx)
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
                self.dock_control(
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
