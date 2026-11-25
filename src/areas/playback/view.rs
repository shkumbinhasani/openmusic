use gpui::{AnyElement, Context, FontWeight, Pixels, div, prelude::*, px, rgb, rgba};
use spotify_client::Track;

use crate::{
    app::OpenMusic,
    icons,
    overlay::{self, Label},
    ui::{
        Palette,
        components::{ROW_RADIUS, icon},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

impl OpenMusic {
    pub fn device_panel(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let remote = self
            .playback
            .now
            .device
            .clone()
            .filter(|_| !self.playback.now.local);
        let (caption, name) = match (
            &remote,
            self.playback.now.local,
            self.playback.now.track.is_some(),
        ) {
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
}

impl OpenMusic {
    pub fn queue_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn jam_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn jam_invite_page(
        &self,
        _uri: &str,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
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
    let toggle = if app.playback.now.playing {
        "pause.fill"
    } else {
        "play.fill"
    };
    let mut items = vec![
        DockItem::new(
            "previous",
            200,
            48.,
            app.dock_control(
                "previous",
                Label::Symbol("backward.fill"),
                false,
                palette,
                cx,
            )
            .size(px(48.)),
        ),
        DockItem::new(
            "toggle-playback",
            300,
            60.,
            app.dock_control("toggle-playback", Label::Symbol(toggle), false, palette, cx)
                .size(px(60.)),
        ),
        DockItem::new(
            "next",
            400,
            48.,
            app.dock_control("next", Label::Symbol("forward.fill"), false, palette, cx)
                .size(px(48.)),
        ),
    ];
    if overlay::available() && roomy {
        let volume = f32::from(app.playback.now.volume) / f32::from(u16::MAX);
        items.push(DockItem::new(
            "volume",
            900,
            168.,
            div()
                .relative()
                .size_full()
                .child(overlay::volume(volume, f32::MAX)),
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
