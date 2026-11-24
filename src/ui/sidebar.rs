use gpui::{Context, FontWeight, div, prelude::*, px, rgba};
use gpui_component::input::Input;

use super::{
    Palette,
    components::{ROW_RADIUS, icon},
};
use crate::{
    app::{OpenMusic, Page, Session},
    areas::discovery::state::SearchTab,
    icons,
};

const SIDEBAR_PADDING: f32 = 10.;

impl OpenMusic {
    pub fn sidebar(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = matches!(self.session, Session::Connected);
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
                                matches!(self.page, Page::Search(_)),
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    if this.discovery.search_query.is_empty() {
                                        this.input.update(cx, |input, cx| input.focus(window, cx));
                                    } else {
                                        this.navigate(Page::Search(SearchTab::All));
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
}
