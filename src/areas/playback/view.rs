use std::rc::Rc;

use gpui::{
    AnyElement, Context, Div, FontWeight, Pixels, SharedString, Window, div, prelude::*, px, rgb,
    rgba,
};
use spotify_client::{
    Track,
    connect::{Repeat, Shuffle},
};

use super::state::{
    QUEUE_AUTOPLAY, QUEUE_CURRENT, QUEUE_NEXT, QUEUE_QUEUED, QUEUE_RECENT, next_repeat,
    next_shuffle,
};
use crate::{
    app::{OpenMusic, Page},
    icons,
    overlay::{self, ButtonSpec, Label},
    ui::{
        Palette,
        components::{ROW_RADIUS, icon},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Surface, Target},
        sidebar::NavEntry,
        tracks::{ROW_GROUP, RowClick, RowOptions},
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

fn jump(prefix: &'static str) -> RowClick {
    Rc::new(
        move |app: &mut OpenMusic, _: &[Track], index, _: &mut Window, cx| {
            app.jump_in_queue((prefix, index), cx)
        },
    )
}

fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .truncate()
        .text_size(px(19.))
        .font_weight(FontWeight::BOLD)
        .child(text.into())
}

impl OpenMusic {
    fn queue_section(
        &self,
        title: impl Into<SharedString>,
        prefix: &'static str,
        options: &RowOptions,
        trailing: Option<AnyElement>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Div {
        let tracks: Vec<Track> = self
            .playback
            .queue
            .section(prefix)
            .iter()
            .map(|row| row.track.clone())
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .min_h(px(36.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_2()
                    .child(heading(title))
                    .children(trailing),
            )
            .child(self.track_list(&tracks, prefix, 0, options, palette, cx))
    }

    fn queue_empty(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .py_16()
            .child(icon(icons::QUEUE, 40., palette.muted))
            .child(heading("Your queue is empty").flex_none())
            .child(
                div()
                    .id("queue-search")
                    .cursor_pointer()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.accent))
                    .hover(|link| link.underline())
                    .child("Search for something new")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.focus_search(window, cx);
                        cx.notify();
                    })),
            )
    }

    pub fn queue_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let queue = &self.playback.queue;
        let allowed = self.playback_allowed();
        let model = self.playback_model();
        let editable = self.queue_editable();
        let reorder = editable && allowed.reorder_queue;
        let base = RowOptions {
            context: model.and_then(|model| model.context_uri.clone()),
            ..RowOptions::default()
        };
        let page = div().flex().flex_col().gap_8().child(
            div()
                .text_size(px(30.))
                .font_weight(FontWeight::BOLD)
                .child("Queue"),
        );
        let page = page.when(self.in_jam(), |page| {
            page.child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child("You’re in a Jam, so the queue can’t be rearranged here."),
            )
        });
        if queue.current.is_none() && queue.upcoming() == 0 && queue.recent.is_empty() {
            return page.child(self.queue_empty(palette, cx)).into_any_element();
        }
        if model.is_some_and(|model| model.track.is_some()) && !allowed.view_queue {
            return page
                .child(self.page_status("The queue isn’t available for what’s playing.", palette))
                .into_any_element();
        }
        let mut sections: Vec<AnyElement> = Vec::new();
        if queue.current.is_some() {
            let options = RowOptions {
                on_click: Some(jump(QUEUE_CURRENT)),
                ..base.clone()
            };
            sections.push(
                self.queue_section("Now playing", QUEUE_CURRENT, &options, None, palette, cx)
                    .into_any_element(),
            );
        }
        if !queue.queued.is_empty() {
            let options = RowOptions {
                on_click: Some(jump(QUEUE_QUEUED)),
                draggable: reorder,
                editable: reorder,
                ..base.clone()
            };
            let clear = (editable && allowed.remove_from_queue).then(|| {
                self.text_control("queue-clear", "Clear queue", false, palette, cx)
                    .into_any_element()
            });
            sections.push(
                self.queue_section("Next in queue", QUEUE_QUEUED, &options, clear, palette, cx)
                    .into_any_element(),
            );
        }
        if !queue.next.is_empty() {
            let options = RowOptions {
                on_click: Some(jump(QUEUE_NEXT)),
                draggable: reorder,
                editable: reorder,
                ..base.clone()
            };
            let title = match self.next_from_title() {
                Some(name) => format!("Next from: {name}"),
                None => "Next up".to_owned(),
            };
            sections.push(
                self.queue_section(title, QUEUE_NEXT, &options, None, palette, cx)
                    .into_any_element(),
            );
        }
        if !queue.autoplay.is_empty() {
            let options = RowOptions {
                on_click: Some(jump(QUEUE_AUTOPLAY)),
                ..base.clone()
            };
            sections.push(
                self.queue_section(
                    "Next from: Autoplay",
                    QUEUE_AUTOPLAY,
                    &options,
                    None,
                    palette,
                    cx,
                )
                .into_any_element(),
            );
        }
        if queue.current.is_some() && queue.upcoming() == 0 {
            sections.push(
                self.page_status("Nothing else is coming up.", palette)
                    .px_2()
                    .into_any_element(),
            );
        }
        if !queue.recent.is_empty() {
            let options = RowOptions {
                on_click: Some(jump(QUEUE_RECENT)),
                ..base
            };
            sections.push(
                self.queue_section("Recently played", QUEUE_RECENT, &options, None, palette, cx)
                    .into_any_element(),
            );
        }
        page.children(sections).into_any_element()
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

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let mut items = Vec::new();
    let Target::Track(track) = target else {
        return items;
    };
    if app.page != Page::Queue || track.surface != Surface::Row {
        return items;
    }
    let queue = &app.playback.queue;
    let found = track.uid.as_deref().and_then(|uid| {
        queue
            .queued
            .iter()
            .chain(&queue.next)
            .find(|row| row.key.uid == uid)
    });
    if let Some(row) = found
        && app.queue_editable()
        && app.playback_allowed().remove_from_queue
    {
        let key = row.key.clone();
        items.push((
            Group::Queue,
            MenuItem::action("Remove from queue", move |app, _, cx| {
                app.remove_from_queue(vec![key.clone()], cx)
            })
            .symbol("minus.circle"),
        ));
    }
    items
}

pub fn nav_entries(app: &OpenMusic) -> Vec<NavEntry> {
    vec![NavEntry::new(
        "nav-queue",
        400,
        icons::QUEUE,
        "Queue",
        app.page == Page::Queue,
        |app, _, _| app.navigate(Page::Queue),
    )]
}

fn shuffle_button(app: &OpenMusic) -> ButtonSpec {
    let mode = app.shuffle_mode();
    let tooltip = match next_shuffle(mode, true, app.smart_shuffle_allowed()) {
        Shuffle::On => "Enable shuffle",
        Shuffle::Smart => "Enable Smart Shuffle",
        Shuffle::Off => "Disable shuffle",
    };
    ButtonSpec {
        label: Label::Symbol(if mode == Shuffle::Smart {
            "sparkles"
        } else {
            "shuffle"
        }),
        prominent: mode != Shuffle::Off,
        enabled: app.playback_allowed().shuffle,
        tooltip: Some(tooltip.to_owned()),
    }
}

fn repeat_button(app: &OpenMusic) -> ButtonSpec {
    let mode = app.repeat_mode();
    let tooltip = match next_repeat(mode, &app.playback_allowed()) {
        Repeat::Context => "Enable repeat",
        Repeat::Track => "Enable repeat one",
        Repeat::Off => "Disable repeat",
    };
    ButtonSpec {
        label: Label::Symbol(if mode == Repeat::Track {
            "repeat.1"
        } else {
            "repeat"
        }),
        prominent: mode != Repeat::Off,
        enabled: app.repeat_allowed(),
        tooltip: Some(tooltip.to_owned()),
    }
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
            "shuffle",
            100,
            36.,
            app.glass_control("shuffle", shuffle_button(app), true, palette, cx)
                .size(px(36.)),
        ),
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
        DockItem::new(
            "repeat",
            450,
            36.,
            app.glass_control("repeat", repeat_button(app), true, palette, cx)
                .size(px(36.)),
        ),
    ];
    if roomy {
        let queue = ButtonSpec {
            label: Label::Symbol("list.bullet"),
            prominent: app.page == Page::Queue,
            enabled: true,
            tooltip: Some("Queue".to_owned()),
        };
        items.push(DockItem::new(
            "queue",
            800,
            48.,
            app.glass_control("queue", queue, true, palette, cx)
                .size(px(48.)),
        ));
    }
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
    app: &OpenMusic,
    _track: &Track,
    row: Row,
    _options: &RowOptions,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Vec<AnyElement> {
    let Some(queue_row) = app.playback.queue.row(row) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    if queue_row.provider == "enhanced_recommendation" {
        items.push(icon(icons::SHUFFLE_SMART, 15., palette.accent).into_any_element());
    }
    if row.0 == QUEUE_QUEUED && app.queue_editable() && app.playback_allowed().remove_from_queue {
        let key = queue_row.key.clone();
        items.push(
            div()
                .id(("queue-remove", row.1))
                .size(px(28.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .opacity(0.)
                .group_hover(ROW_GROUP, |button| button.opacity(1.))
                .hover(move |button| button.bg(rgba(palette.press)))
                .child(icon(icons::CLOSE, 13., palette.muted))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.remove_from_queue(vec![key.clone()], cx);
                }))
                .into_any_element(),
        );
    }
    items
}

pub fn row_dimmed(_app: &OpenMusic, _track: &Track, _row: Row, _options: &RowOptions) -> bool {
    false
}
