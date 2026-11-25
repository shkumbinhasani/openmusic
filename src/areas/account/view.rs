use gpui::{
    AnyElement, Context, FontWeight, MouseButton, Pixels, SharedString, div, prelude::*, px, rgb,
    rgba,
};
use gpui_component::switch::Switch;
use spotify_client::{
    Track,
    account::{AccountInfo, Explicit},
};

use super::state::{
    CHIP, CHIP_ROW, CLEAR_CACHE_KEY, FREE_MESSAGE, FREE_NOTICE, LANGUAGE_KEY, LANGUAGES,
    PREMIUM_KEY, PREMIUM_URL, cache_label, language_name,
};
use super::{local, local_match, profile};
use crate::{
    app::{OpenMusic, settings::SECTIONS},
    icons,
    overlay::{self, Label},
    ui::{
        Palette, Panel,
        components::{ROW_RADIUS, bone, icon},
        dock::DockItem,
        menu::{Group, MenuItem, Row, Target},
        sidebar::NavEntry,
        tracks::RowOptions,
    },
};

const SETTINGS_WIDTH: f32 = 720.;
const FRIENDS_DETAIL: &str = "Shows Friend Activity on the right when the window is wide enough. View › Friend Activity turns it on and off too.";
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

    pub fn local_files_page(
        &self,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.local_files_view(card_side, palette, cx)
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
        let chip = chip
            .cursor_pointer()
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
            );
        if info.premium() {
            return Some(chip.into_any_element());
        }
        Some(
            div()
                .flex_shrink_0()
                .flex()
                .flex_col()
                .gap_2()
                .child(self.free_note(palette, cx))
                .child(chip)
                .into_any_element(),
        )
    }

    fn free_note(&self, palette: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2p5()
            .rounded(px(ROW_RADIUS + 4.))
            .bg(rgba(palette.press))
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(FREE_NOTICE),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(palette.muted))
                    .child("You can still browse your library and control Spotify on your other devices."),
            )
            .child(
                div()
                    .id("account-free-premium")
                    .mt_0p5()
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette.accent))
                    .cursor_pointer()
                    .hover(|link| link.underline())
                    .child("Get Premium")
                    .on_click(cx.listener(|_, _, _, cx| cx.open_url(PREMIUM_URL))),
            )
    }

    pub fn right_panel(&self, palette: Palette, cx: &mut Context<Self>) -> Option<Panel> {
        self.friends_panel(palette, cx)
    }

    pub fn account_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let rows = match (self.account(), &self.account.error) {
            (Some(info), _) => {
                let mut plan = info.plan().to_owned();
                if info.plan_manager() {
                    plan.push_str(" · Plan manager");
                }
                let mut rows = vec![setting_row(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(self.avatar(&info.name, info.avatar.as_deref(), px(44.), palette))
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
                )];
                if !info.premium() {
                    rows.push(setting_row(
                        "Playback on this Mac",
                        Some(FREE_MESSAGE.into()),
                        self.text_control(PREMIUM_KEY, "Get Premium", true, palette, cx),
                        palette,
                    ));
                }
                rows.push(setting_row(
                    "Sign out",
                    Some("Signs out of Spotify on this Mac. Playback here stops.".into()),
                    self.text_control("account-sign-out", "Sign Out", false, palette, cx),
                    palette,
                ));
                rows
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

    pub fn content_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let info = self.account()?;
        let explicit = info.explicit();
        let (status, description) = match explicit {
            Explicit::Allowed => (
                "Allowed",
                "Songs and episodes rated explicit play as usual.",
            ),
            Explicit::Filtered => (
                "Filtered",
                "Spotify skips songs and episodes rated explicit. They show dimmed here.",
            ),
            Explicit::FilteredByManager => {
                ("Off", "Your plan manager has turned off explicit content.")
            }
        };
        let status = div()
            .flex()
            .items_center()
            .gap_1p5()
            .text_size(px(13.))
            .text_color(rgb(palette.muted))
            .when(explicit == Explicit::FilteredByManager, |status| {
                status.child(icon(icons::LOCK, 13., palette.muted))
            })
            .child(status);
        let description = if explicit == Explicit::FilteredByManager {
            description.to_owned()
        } else {
            format!("{description} Change it in Spotify on your phone or computer.")
        };
        let mut rows = vec![setting_row(
            "Allow explicit content",
            Some(description.into()),
            status,
            palette,
        )];
        if info.plan_manager() {
            rows.push(setting_row(
                "Members of your plan",
                Some("Choose who can play explicit content on spotify.com.".into()),
                self.text_control("account-family", "Manage Plan", false, palette, cx),
                palette,
            ));
        }
        Some(settings_section("Explicit content", rows, palette))
    }

    pub fn social_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let mut rows = self.privacy_rows(palette, cx);
        let app = cx.entity().downgrade();
        let friends = Switch::new("settings-friend-activity")
            .checked(self.friends_on())
            .on_click(move |show, _, cx| {
                let _ = app.update(cx, |app, cx| {
                    app.set_friend_activity(*show);
                    cx.notify();
                });
            });
        rows.push(setting_row(
            "See what your friends are playing",
            Some(FRIENDS_DETAIL.into()),
            friends,
            palette,
        ));
        Some(settings_section("Social", rows, palette))
    }

    pub fn library_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        Some(settings_section(
            "Local Files",
            self.local_rows(palette, cx),
            palette,
        ))
    }

    pub fn language_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let chosen = self.settings.account.language.as_deref();
        let label = match chosen {
            None => "System Default",
            Some(tag) => language_name(tag).unwrap_or("Other"),
        };
        let effective = self.language();
        let description = match chosen {
            None => format!(
                "Uses {} from macOS. Spotify picks this language for titles, podcasts and search results.",
                language_name(&effective)
                    .map(|name| format!("{name} ({effective})"))
                    .unwrap_or(effective.clone())
            ),
            Some(_) => format!(
                "Spotify picks this language ({effective}) for titles, podcasts and search results. OpenMusic’s own labels stay in English."
            ),
        };
        let picker = self
            .control(LANGUAGE_KEY, Label::Text(label), false, palette, cx)
            .h(px(36.))
            .w(px(label_width(label)))
            .child(overlay::anchor(LANGUAGE_KEY));
        let mut rows = vec![setting_row(
            "Language",
            Some(description.into()),
            picker,
            palette,
        )];
        if self.account.languages_open {
            let choices = std::iter::once((None, "System Default"))
                .chain(LANGUAGES.iter().map(|(tag, name)| (Some(*tag), *name)));
            rows.extend(choices.enumerate().map(|(index, (tag, name))| {
                let selected = chosen == tag;
                div()
                    .id(("language-choice", index))
                    .h(px(36.))
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .px(px(ROW_INSET))
                    .cursor_pointer()
                    .hover(move |row| row.bg(rgba(palette.press)))
                    .text_size(px(13.))
                    .child(div().w(px(14.)).when(selected, |mark| {
                        mark.child(icon(icons::CHECK, 14., palette.accent))
                    }))
                    .child(name)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_language(tag.map(str::to_owned));
                        cx.notify();
                    }))
                    .into_any_element()
            }));
        }
        Some(settings_section("Language", rows, palette))
    }

    pub fn storage_section(&self, palette: Palette, cx: &mut Context<Self>) -> Option<AnyElement> {
        let size = match (self.account.cache, self.account.cache_busy) {
            (_, true) => "Clearing…".to_owned(),
            (Some(bytes), false) => cache_label(bytes),
            (None, false) => "Checking…".to_owned(),
        };
        let control = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child(size),
            )
            .child(self.text_control(CLEAR_CACHE_KEY, "Clear Cache", false, palette, cx));
        Some(settings_section(
            "Storage",
            vec![setting_row(
                "Cache",
                Some(
                    "Artwork and the offline copy of your playlists, kept so pages open quickly. Clearing it doesn’t affect your account."
                        .into(),
                ),
                control,
                palette,
            )],
            palette,
        ))
    }
}

pub fn menu_items(target: &Target, app: &OpenMusic) -> Vec<(Group, MenuItem)> {
    let mut items = profile::menu_items(target, app);
    items.extend(local::menu_items(target, app));
    items
}

pub fn nav_entries(app: &OpenMusic) -> Vec<NavEntry> {
    local::nav_entry(app).into_iter().collect()
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

fn label_width(text: &str) -> f32 {
    let glyphs: f32 = text
        .chars()
        .map(|glyph| if u32::from(glyph) >= 0x2e80 { 14. } else { 7.6 })
        .sum();
    (32. + glyphs).max(72.)
}

pub fn row_dimmed(app: &OpenMusic, track: &Track, row: Row, _options: &RowOptions) -> bool {
    filtered(app.account(), track)
        || local::row_dimmed(app, row)
        || local_match::row_dimmed(app, track, row)
}

pub fn filtered(account: Option<&AccountInfo>, track: &Track) -> bool {
    track.explicit && account.is_some_and(|account| account.explicit_filtered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dims_explicit_rows_only_when_the_account_filters_them() {
        let explicit = Track {
            explicit: true,
            ..Track::placeholder("spotify:track:a")
        };
        let clean = Track::placeholder("spotify:track:b");
        let filtering = AccountInfo {
            explicit_filtered: true,
            ..AccountInfo::default()
        };
        let open = AccountInfo::default();
        assert!(filtered(Some(&filtering), &explicit));
        assert!(!filtered(Some(&filtering), &clean));
        assert!(!filtered(Some(&open), &explicit));
        assert!(!filtered(None, &explicit));
    }
}
