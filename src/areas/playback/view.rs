use std::rc::Rc;

use gpui::{
    AnyElement, Context, Div, FontWeight, Pixels, SharedString, Window, div, prelude::*, px, rgb,
    rgba,
};
use spotify_client::{
    Track,
    connect::{RECOMMENDED, Repeat, Shuffle},
};

use super::{
    jam::{JAM_SYMBOL, PlayAfter, jam_rows},
    state::{
        DEVICE_PANEL, DEVICES_BUTTON, DJ_JUMP, PickerAction, PickerRow, QUEUE_AUTOPLAY,
        QUEUE_CURRENT, QUEUE_NEXT, QUEUE_QUEUED, QUEUE_RECENT, QueueSource, device_label,
        next_repeat, next_shuffle, queue_source,
    },
};
use crate::{
    app::{OpenMusic, Page},
    icons,
    overlay::{self, ButtonSpec, Label},
    ui::{
        Palette,
        components::{ROW_RADIUS, fallback_icon, icon},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Surface, Target},
        sidebar::NavEntry,
        tracks::{ROW_GROUP, RowClick, RowOptions},
    },
};

impl OpenMusic {
    pub fn device_panel(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let remote = self
            .remote_device()
            .map(device_label)
            .or_else(|| self.playback.now.device.clone())
            .filter(|_| !self.playback.now.local);
        let remote = remote.or_else(|| {
            self.jam()
                .filter(|session| session.redirect_commands && !self.playback.now.local)
                .map(|session| {
                    if session.host_device_name.is_empty() {
                        format!("{}’s speaker", session.host_name())
                    } else {
                        session.host_device_name.clone()
                    }
                })
        });
        let pending = self.playback.transfer.as_ref();
        let (caption, name) = match (pending, &remote, self.playback.now.local) {
            (Some(pending), _, _) => ("Connecting to", pending.name.clone()),
            (None, Some(name), _) => ("Playing on", name.clone()),
            (None, None, true) => ("Playing on", "This Mac".to_owned()),
            (None, None, false) => ("Spotify Connect", "Connect to a device".to_owned()),
        };
        let highlight = remote.is_some() || pending.is_some();
        let color = if highlight {
            palette.accent
        } else {
            palette.muted
        };
        let symbol = self
            .remote_device()
            .map_or("laptopcomputer", |device| device.kind.symbol());
        div()
            .id(DEVICE_PANEL)
            .relative()
            .flex_shrink_0()
            .flex()
            .flex_col()
            .gap_2()
            .p(px(10.))
            .rounded(px(ROW_RADIUS + 4.))
            .bg(rgba(palette.press))
            .cursor_pointer()
            .hover(move |panel| panel.bg(rgba(palette.fill)))
            .on_click(cx.listener(|this, _, _, cx| {
                if this.show_devices(Some(DEVICE_PANEL)) {
                    cx.notify();
                }
            }))
            .child(overlay::anchor(DEVICE_PANEL))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .child(icon(fallback_icon(symbol), 18., color))
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
                                    .text_color(rgb(if highlight {
                                        palette.accent
                                    } else {
                                        palette.text
                                    }))
                                    .truncate()
                                    .child(name),
                            ),
                    ),
            )
            .when(
                remote.is_some() && !self.dj_narrating() && !self.jam_redirected(),
                |panel| {
                    panel.child(
                        self.text_control("play-here", "Play on This Mac", true, palette, cx)
                            .h(px(30.))
                            .w_full(),
                    )
                },
            )
            .children(self.jam_panel(palette, cx))
    }

    pub fn device_menu(&self) -> Vec<MenuItem> {
        let mut rows = self.picker_rows();
        if !self.volume_allowed() {
            rows.retain(|row| !matches!(row, PickerRow::Volume(_)));
        }
        rows.extend(jam_rows(self.jam()));
        rows.into_iter()
            .map(|row| match row {
                PickerRow::Header(text) => MenuItem::header(text),
                PickerRow::Note(text) => MenuItem::note(text),
                PickerRow::Volume(value) => MenuItem::Volume(value),
                PickerRow::Separator => MenuItem::Separator,
                PickerRow::Action {
                    action,
                    title,
                    subtitle,
                    symbol,
                } => {
                    let item = MenuItem::action(title, move |app, window, cx| match action {
                        PickerAction::StartJam => app.start_jam(None, cx),
                        PickerAction::OpenJam => app.navigate(Page::Jam),
                        PickerAction::CopyJamLink => app.copy_jam_link(cx),
                        PickerAction::LeaveJam => app.confirm_leave_jam(window, cx),
                        PickerAction::EndJam => app.confirm_end_jam(window, cx),
                    })
                    .symbol(symbol)
                    .enabled(!self.jam_busy());
                    match subtitle {
                        Some(text) => item.subtitle(text),
                        None => item,
                    }
                }
                PickerRow::Device {
                    id,
                    title,
                    symbol,
                    current,
                    enabled,
                    subtitle,
                } => {
                    let item = MenuItem::action(title, move |app, _, cx| {
                        if !current {
                            app.transfer_to(&id, cx);
                        }
                    })
                    .symbol(symbol)
                    .checked(current)
                    .enabled(enabled);
                    match subtitle {
                        Some(text) => item.subtitle(text),
                        None => item,
                    }
                }
            })
            .collect()
    }
}

pub(super) fn jump(prefix: &'static str) -> RowClick {
    Rc::new(
        move |app: &mut OpenMusic, _: &[Track], index, _: &mut Window, cx| {
            app.jump_in_queue((prefix, index), cx)
        },
    )
}

pub(super) fn heading(text: impl Into<SharedString>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(px(19.))
        .font_weight(FontWeight::BOLD)
        .child(text.into())
}

impl OpenMusic {
    pub(super) fn queue_section(
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
        let page = match self.jam_queue_note(palette, cx) {
            Some(note) => page.child(note),
            None => page.when(self.in_jam(), |page| {
                page.child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(palette.muted))
                        .child("You’re in a Jam, so the queue can’t be rearranged here."),
                )
            }),
        };
        let dj = self.dj_context();
        if queue.current.is_none() && queue.upcoming() == 0 && queue.recent.is_empty() {
            return page.child(self.queue_empty(palette, cx)).into_any_element();
        }
        if model.is_some_and(|model| model.track.is_some()) && !allowed.view_queue {
            if dj {
                return page.child(dj_queue_note(palette)).into_any_element();
            }
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
        if dj {
            sections.push(dj_queue_note(palette).into_any_element());
        }
        if !dj && !queue.next.is_empty() {
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
        if !dj && !queue.autoplay.is_empty() {
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
        if !dj && queue.current.is_some() && queue.upcoming() == 0 {
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
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.jam_page_body(palette, cx)
    }

    pub fn jam_invite_page(
        &self,
        uri: &str,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap_4()
            .child(self.jam_invite_body(uri, palette, cx))
            .into_any_element()
    }
}

fn dj_queue_note(palette: Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .px_2()
        .child(heading("The DJ doesn’t have a queue"))
        .child(
            div()
                .text_size(px(13.))
                .text_color(rgb(palette.muted))
                .child(
                    "Your DJ picks each song for you in the moment, so you never know what’s coming next.",
                ),
        )
}

fn play_after(target: &Target) -> Option<PlayAfter> {
    match target {
        Target::Track(track) if !track.track.uri.starts_with("spotify:local:") => {
            Some(PlayAfter::Track(Box::new(track.track.clone())))
        }
        Target::Album { uri } | Target::Playlist { uri } => Some(PlayAfter::Context(uri.clone())),
        _ => None,
    }
}

fn jam_actions(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    if let Target::User { uri } = target {
        let username = uri.rsplit(':').next().unwrap_or_default();
        let Some(member) = app
            .jam()
            .filter(|session| session.can_remove)
            .and_then(|session| {
                session
                    .member(username)
                    .filter(|member| member.id != session.owner_id && !member.current_user)
            })
        else {
            return Vec::new();
        };
        let id = member.id.clone();
        return vec![(
            Group::Edit,
            MenuItem::action("Remove from Jam", move |app, window, cx| {
                app.confirm_remove_from_jam(&id, window, cx)
            })
            .symbol("person.crop.circle.badge.xmark")
            .enabled(!app.jam_busy()),
        )];
    }
    if app.jam().is_some() || !matches!(app.session, crate::app::Session::Connected) {
        return Vec::new();
    }
    let Some(play) = play_after(target) else {
        return Vec::new();
    };
    vec![(
        Group::Queue,
        MenuItem::action("Start a Jam", move |app, _, cx| {
            app.start_jam(Some(play.clone()), cx)
        })
        .key("j")
        .symbol(JAM_SYMBOL)
        .enabled(!app.jam_busy()),
    )]
}

fn context_empty(app: &OpenMusic, source: &QueueSource) -> bool {
    let QueueSource::Context(uri) = source else {
        return false;
    };
    matches!(
        &app.playlists.collection,
        Some((loaded, Ok(collection))) if loaded == uri && collection.tracks.is_empty()
    )
}

fn queue_actions(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let Some(source) = queue_source(target) else {
        return Vec::new();
    };
    let empty = context_empty(app, &source);
    let later = source.clone();
    vec![
        (
            Group::Queue,
            MenuItem::action("Add to queue", move |app, _, cx| {
                app.queue_from(later.clone(), false, cx)
            })
            .key("q")
            .symbol("text.line.last.and.arrowtriangle.forward")
            .enabled(!empty && app.queue_allowed(false)),
        ),
        (
            Group::Queue,
            MenuItem::action("Play next", move |app, _, cx| {
                app.queue_from(source.clone(), true, cx)
            })
            .symbol("text.line.first.and.arrowtriangle.forward")
            .enabled(!empty && app.queue_allowed(true)),
        ),
    ]
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let mut items = queue_actions(target, app);
    items.extend(jam_actions(target, app));
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
            .key("alt-q")
            .symbol("minus.circle"),
        ));
    }
    items
}

pub fn nav_entries(app: &OpenMusic) -> Vec<NavEntry> {
    let mut entries = vec![NavEntry::new(
        "nav-queue",
        400,
        icons::QUEUE,
        "Queue",
        app.page == Page::Queue,
        |app, _, _| app.navigate(Page::Queue),
    )];
    if app.jam().is_some() {
        entries.push(NavEntry::new(
            "nav-jam",
            410,
            icons::PERSON,
            "Jam",
            app.page == Page::Jam,
            |app, _, _| app.navigate(Page::Jam),
        ));
    }
    if let Some(dj) = &app.playback.dj.available {
        entries.push(NavEntry::new(
            "nav-dj",
            450,
            icons::SHUFFLE_SMART,
            dj.name.clone(),
            app.dj_context(),
            |app, _, cx| app.play_dj(cx),
        ));
    }
    entries
}

fn shuffle_button(app: &OpenMusic) -> ButtonSpec {
    shuffle_spec(
        app.shuffle_mode(),
        app.playback_allowed().shuffle,
        app.smart_shuffle_allowed(),
    )
}

pub fn shuffle_spec(mode: Shuffle, enabled: bool, smart: bool) -> ButtonSpec {
    let tooltip = match next_shuffle(mode, true, smart) {
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
        enabled,
        tooltip: Some(tooltip.to_owned()),
    }
}

fn repeat_button(app: &OpenMusic) -> ButtonSpec {
    let mode = app.repeat_mode();
    let tooltip = match next_repeat(mode, &app.playback_allowed()) {
        Repeat::Context => "Enable repeat",
        Repeat::Track => "Enable repeat one",
        Repeat::Off if mode == Repeat::Off => "Enable repeat",
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

fn devices_button(app: &OpenMusic) -> ButtonSpec {
    let remote = app.remote_device();
    let tooltip = match &app.playback.transfer {
        Some(pending) => format!("Connecting to {}…", pending.name),
        None => "Connect to a device".to_owned(),
    };
    ButtonSpec {
        label: Label::Symbol(
            remote.map_or("laptopcomputer.and.iphone", |device| device.kind.symbol()),
        ),
        prominent: remote.is_some() || app.playback.transfer.is_some(),
        enabled: app.playback_model().is_some(),
        tooltip: Some(tooltip),
    }
}

pub fn dock_items(
    app: &OpenMusic,
    roomy: bool,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Vec<DockItem> {
    app.playback.dock_devices.set(roomy);
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
    if app.dj_context() && app.dj_jump_available() {
        let jump = ButtonSpec {
            label: Label::Symbol("waveform"),
            prominent: false,
            enabled: true,
            tooltip: Some("Get some different DJ picks".to_owned()),
        };
        items.push(DockItem::new(
            DJ_JUMP,
            640,
            48.,
            app.glass_control(DJ_JUMP, jump, true, palette, cx)
                .size(px(48.)),
        ));
    }
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
        items.push(DockItem::new(
            DEVICES_BUTTON,
            850,
            48.,
            app.glass_control(DEVICES_BUTTON, devices_button(app), true, palette, cx)
                .size(px(48.))
                .child(overlay::anchor(DEVICES_BUTTON)),
        ));
    }
    if overlay::available() && roomy {
        let volume = f32::from(app.playback.now.volume) / f32::from(u16::MAX);
        let (enabled, tooltip) = app.volume_state();
        items.push(DockItem::new(
            "volume",
            900,
            168.,
            div()
                .relative()
                .size_full()
                .child(overlay::volume(volume, f32::MAX, enabled, tooltip)),
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
    if let Some(name) = queue_row
        .queued_by
        .as_deref()
        .filter(|_| row.0 != QUEUE_RECENT && app.in_jam())
    {
        items.push(app.added_by_badge(name, palette));
    }
    if queue_row.provider == RECOMMENDED {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_the_shuffle_mode_and_what_a_press_does() {
        let off = shuffle_spec(Shuffle::Off, true, true);
        assert_eq!(off.label, Label::Symbol("shuffle"));
        assert!(!off.prominent && off.enabled);
        assert_eq!(off.tooltip.as_deref(), Some("Enable shuffle"));
        let on = shuffle_spec(Shuffle::On, false, true);
        assert!(on.prominent && !on.enabled);
        assert_eq!(on.tooltip.as_deref(), Some("Enable Smart Shuffle"));
        assert_eq!(
            shuffle_spec(Shuffle::On, true, false).tooltip.as_deref(),
            Some("Disable shuffle")
        );
        let smart = shuffle_spec(Shuffle::Smart, true, true);
        assert_eq!(smart.label, Label::Symbol("sparkles"));
        assert_eq!(smart.tooltip.as_deref(), Some("Disable shuffle"));
    }
}
