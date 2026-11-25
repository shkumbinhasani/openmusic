use std::rc::Rc;

use gpui::{Context, FontWeight, SharedString, Window, div, prelude::*, px, rgba};
use gpui_component::input::Input;

use super::{
    Palette,
    components::{ROW_RADIUS, icon},
    menu::Run,
};
use crate::{
    app::{OpenMusic, Session},
    areas::{account, audio, content, details, discovery, library, playback, playlists, system},
    icons,
};

const SIDEBAR_PADDING: f32 = 10.;

pub struct NavEntry {
    pub key: &'static str,
    pub weight: u32,
    pub icon: &'static str,
    pub label: SharedString,
    pub selected: bool,
    pub run: Run,
}

impl NavEntry {
    pub fn new(
        key: &'static str,
        weight: u32,
        icon: &'static str,
        label: impl Into<SharedString>,
        selected: bool,
        run: impl Fn(&mut OpenMusic, &mut Window, &mut Context<OpenMusic>) + 'static,
    ) -> Self {
        Self {
            key,
            weight,
            icon,
            label: label.into(),
            selected,
            run: Rc::new(run),
        }
    }
}

pub type NavEntries = fn(&OpenMusic) -> Vec<NavEntry>;

pub const NAV_ENTRIES: &[NavEntries] = &[
    account::view::nav_entries,
    audio::view::nav_entries,
    content::view::nav_entries,
    details::view::nav_entries,
    discovery::view::nav_entries,
    library::view::nav_entries,
    playback::view::nav_entries,
    playlists::view::nav_entries,
    system::view::nav_entries,
];

impl OpenMusic {
    pub fn nav_entries(&self) -> Vec<NavEntry> {
        let mut entries: Vec<NavEntry> = NAV_ENTRIES
            .iter()
            .flat_map(|entries| entries(self))
            .collect();
        entries.sort_by_key(|entry| entry.weight);
        entries
    }

    pub fn sidebar(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = matches!(self.session, Session::Connected);

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
                .children(self.search_suggestions(palette, cx))
                .child(div().flex().flex_col().gap_0p5().children(
                    self.nav_entries().into_iter().map(|entry| {
                        let run = entry.run;
                        let selected = entry.selected;
                        div()
                            .id(entry.key)
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
                                entry.icon,
                                16.,
                                if selected {
                                    palette.accent
                                } else {
                                    palette.muted
                                },
                            ))
                            .child(entry.label)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                run(this, window, cx);
                                cx.notify();
                            }))
                    }),
                ))
                .child(self.playlist_list(palette, cx))
                .children(self.account_chip(palette, cx))
                .child(self.device_panel(palette, cx))
            })
    }
}
