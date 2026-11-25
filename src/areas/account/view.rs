use gpui::{
    AnyElement, Context, FontWeight, MouseButton, Pixels, SharedString, div, prelude::*, px, rgb,
    rgba,
};
use spotify_client::Track;

use super::state::{CHIP, CHIP_ROW, ProfileListKind};
use crate::{
    app::{OpenMusic, settings::SECTIONS},
    overlay,
    ui::{
        Palette, Panel,
        components::{ROW_RADIUS, bone},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

const SETTINGS_WIDTH: f32 = 720.;
const ROW_INSET: f32 = 14.;

pub fn settings_section(
    title: impl Into<SharedString>,
    rows: Vec<AnyElement>,
    palette: Palette,
) -> AnyElement {
    let count = rows.len();
    div()
        .flex()
        .flex_col()
        .gap_2p5()
        .child(
            div()
                .px_1()
                .text_size(px(16.))
                .font_weight(FontWeight::BOLD)
                .child(title.into()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(ROW_RADIUS + 4.))
                .bg(rgba(palette.press))
                .children(rows.into_iter().enumerate().flat_map(|(index, row)| {
                    let line = (index + 1 < count).then(|| {
                        div()
                            .h(px(1.))
                            .mx(px(ROW_INSET))
                            .bg(rgba(palette.line))
                            .into_any_element()
                    });
                    std::iter::once(row).chain(line)
                })),
        )
        .into_any_element()
}

pub fn setting_row(
    title: impl IntoElement,
    description: Option<SharedString>,
    control: impl IntoElement,
    palette: Palette,
) -> AnyElement {
    div()
        .min_h(px(52.))
        .flex()
        .items_center()
        .gap_4()
        .px(px(ROW_INSET))
        .py(px(10.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_size(px(13.5))
                        .font_weight(FontWeight::MEDIUM)
                        .child(title),
                )
                .when_some(description, |text, description| {
                    text.child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(palette.muted))
                            .child(description),
                    )
                }),
        )
        .child(div().flex_shrink_0().flex().items_center().child(control))
        .into_any_element()
}

impl OpenMusic {
    pub fn settings_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sections: Vec<AnyElement> = SECTIONS
            .iter()
            .filter_map(|section| section(self, palette, cx))
            .collect();
        div()
            .w_full()
            .max_w(px(SETTINGS_WIDTH))
            .flex()
            .flex_col()
            .gap(px(28.))
            .child(
                div()
                    .text_size(px(30.))
                    .font_weight(FontWeight::BOLD)
                    .child("Settings"),
            )
            .map(|page| {
                if sections.is_empty() {
                    page.child(self.page_status("Nothing to set up yet.", palette))
                } else {
                    page.children(sections)
                }
            })
            .into_any_element()
    }

    pub fn profile_page(
        &self,
        _uri: &str,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn profile_list_page(
        &self,
        _uri: &str,
        _kind: ProfileListKind,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn local_files_page(
        &self,
        _card_side: Pixels,
        palette: Palette,
        _cx: &mut Context<Self>,
    ) -> AnyElement {
        self.page_status("Coming soon", palette).into_any_element()
    }

    pub fn account_chip(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chip = div()
            .id(CHIP)
            .flex_shrink_0()
            .h(px(46.))
            .flex()
            .items_center()
            .gap_2p5()
            .px_1p5()
            .rounded(px(ROW_RADIUS + 2.));
        let Some(info) = self.account() else {
            if !self.account.loading {
                return None;
            }
            return Some(
                chip.child(bone(CHIP, 0, palette).size(px(30.)).rounded_full())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1p5()
                            .child(bone(CHIP, 1, palette).w(px(96.)).h(px(11.)).rounded(px(4.)))
                            .child(bone(CHIP, 2, palette).w(px(56.)).h(px(9.)).rounded(px(4.))),
                    )
                    .into_any_element(),
            );
        };
        Some(
            chip.cursor_pointer()
                .when(self.menu_row(CHIP_ROW), |chip| chip.bg(rgba(palette.press)))
                .hover(move |chip| chip.bg(rgba(palette.press)))
                .child(self.avatar(&info.name, info.avatar.as_deref(), px(30.), palette))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(info.name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(palette.muted))
                                .truncate()
                                .child(info.plan()),
                        ),
                )
                .child(overlay::anchor(CHIP))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_account_menu();
                    cx.notify();
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_account_menu();
                        cx.notify();
                    }),
                )
                .into_any_element(),
        )
    }

    pub fn right_panel(&self, _palette: Palette, _cx: &mut Context<Self>) -> Option<Panel> {
        None
    }

    pub fn account_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rows = match (self.account(), &self.account.error) {
            (Some(info), _) => {
                let mut plan = info.plan().to_owned();
                if info.plan_manager() {
                    plan.push_str(" · Plan manager");
                }
                vec![
                    setting_row(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(self.avatar(
                                &info.name,
                                info.avatar.as_deref(),
                                px(44.),
                                palette,
                            ))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_size(px(15.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(info.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.))
                                            .text_color(rgb(palette.muted))
                                            .child(plan),
                                    ),
                            ),
                        None,
                        self.text_control("account-manage", "Manage Account", false, palette, cx),
                        palette,
                    ),
                    setting_row(
                        "Sign out",
                        Some("Signs out of Spotify on this Mac. Playback here stops.".into()),
                        self.text_control("account-sign-out", "Sign Out", false, palette, cx),
                        palette,
                    ),
                ]
            }
            (None, Some(error)) if !self.account.loading => vec![setting_row(
                error.clone(),
                Some("Check your connection, then try again.".into()),
                self.text_control("account-retry", "Try Again", false, palette, cx),
                palette,
            )],
            (None, _) => vec![
                div()
                    .h(px(68.))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px(px(ROW_INSET))
                    .child(
                        bone("account-section", 0, palette)
                            .size(px(44.))
                            .rounded_full(),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                bone("account-section", 1, palette)
                                    .w(px(160.))
                                    .h(px(13.))
                                    .rounded(px(4.)),
                            )
                            .child(
                                bone("account-section", 2, palette)
                                    .w(px(96.))
                                    .h(px(10.))
                                    .rounded(px(4.)),
                            ),
                    )
                    .into_any_element(),
            ],
        };
        Some(settings_section("Account", rows, palette))
    }

    pub fn content_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn social_section(&self, _palette: Palette, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    pub fn library_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn language_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub fn storage_section(
        &self,
        _palette: Palette,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }
}

pub fn menu_items(_target: &Target, _app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    Vec::new()
}

pub fn nav_entries(_app: &OpenMusic) -> Vec<NavEntry> {
    Vec::new()
}

pub fn dock_items(
    _app: &OpenMusic,
    _roomy: bool,
    _palette: Palette,
    _cx: &mut Context<OpenMusic>,
) -> Vec<DockItem> {
    Vec::new()
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
