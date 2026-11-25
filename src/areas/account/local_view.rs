use std::{path::Path, rc::Rc};

use gpui::{
    AnyElement, Context, Div, FontWeight, Pixels, Stateful, div, prelude::*, px, rgb, rgba,
};
use gpui_component::{input::Input, switch::Switch};

use super::{
    local::{
        ADD_KEY, Column, ENABLE_KEY, MORE_KEY, PLAY_KEY, PRIVACY_KEY, PRIVACY_URL, RESCAN_KEY, ROW,
        SORT_KEY, count_label, display_path, total_label,
    },
    local_scan::{LocalSources, ScanReport, home},
    view::setting_row,
};
use crate::{
    app::{OpenMusic, Page},
    icons,
    overlay::{self, Label},
    ui::{
        Palette,
        components::{icon, link, scroll_sentinel, skeleton_rows},
        tracks::RowOptions,
    },
};

const COVER_SIDE: f32 = 184.;
const SHOW_DETAIL: &str = "Adds songs from folders on this Mac to Local Files in the sidebar. The files stay on this Mac.";
const ADD_DETAIL: &str =
    "Pick a folder with music in it. OpenMusic reads MP3, AAC, ALAC, FLAC, WAV and AIFF files.";

type SourceChange = fn(&mut LocalSources, bool);

fn source_switch(
    id: &'static str,
    checked: bool,
    change: SourceChange,
    cx: &mut Context<OpenMusic>,
) -> impl IntoElement {
    let app = cx.entity().downgrade();
    Switch::new(id).checked(checked).on_click(move |on, _, cx| {
        let on = *on;
        let _ = app.update(cx, |app, cx| {
            app.set_local_sources(|sources| change(sources, on));
            cx.notify();
        });
    })
}

fn root_note(report: &ScanReport, path: &Path) -> &'static str {
    if report.denied.iter().any(|denied| denied == path) {
        " · No access"
    } else if report.missing.iter().any(|missing| missing == path) {
        " · Not connected"
    } else {
        ""
    }
}

fn local_cover(palette: Palette) -> impl IntoElement {
    div()
        .size(px(COVER_SIDE))
        .flex_shrink_0()
        .rounded(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(palette.raised))
        .shadow_lg()
        .child(icon(icons::LOCAL, 76., palette.accent))
}

fn note_line(text: String, palette: Palette) -> Div {
    div()
        .text_size(px(12.5))
        .text_color(rgb(palette.muted))
        .child(text)
}

impl OpenMusic {
    pub fn local_rows(&self, palette: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let sources = self.local_sources().clone();
        let mut rows = vec![setting_row(
            "Show Local Files",
            Some(SHOW_DETAIL.into()),
            source_switch(
                "settings-local-files",
                sources.enabled,
                |sources, on| sources.enabled = on,
                cx,
            ),
            palette,
        )];
        if !sources.enabled {
            return rows;
        }
        let home = home();
        let report = self.account.local.report.clone().unwrap_or_default();
        let defaults: [(&str, &'static str, bool, &str, SourceChange); 2] = [
            (
                "Downloads",
                "settings-local-downloads",
                sources.downloads,
                "Downloads",
                |sources, on| sources.downloads = on,
            ),
            (
                "Music",
                "settings-local-music",
                sources.music,
                "Music",
                |sources, on| sources.music = on,
            ),
        ];
        for (title, id, checked, folder, change) in defaults {
            let description = home.as_ref().map(|home| {
                let path = home.join(folder);
                format!(
                    "{}{}",
                    display_path(&path, Some(home)),
                    root_note(&report, &path)
                )
                .into()
            });
            rows.push(setting_row(
                title,
                description,
                source_switch(id, checked, change, cx),
                palette,
            ));
        }
        for (index, folder) in sources.folders.iter().enumerate() {
            let name = folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| folder.display().to_string());
            let description = format!(
                "{}{}",
                display_path(folder, home.as_deref()),
                root_note(&report, folder)
            );
            let path = folder.clone();
            let remove = div()
                .id(("local-folder-remove", index))
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(palette.accent))
                .cursor_pointer()
                .hover(|remove| remove.underline())
                .child("Remove")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.remove_local_source(&path);
                    cx.notify();
                }));
            rows.push(setting_row(name, Some(description.into()), remove, palette));
        }
        rows.push(setting_row(
            "Add a source",
            Some(ADD_DETAIL.into()),
            self.text_control(ADD_KEY, "Add Folder…", false, palette, cx),
            palette,
        ));
        rows.push(setting_row(
            "Songs found",
            Some(self.account.local.status().into()),
            self.text_control(RESCAN_KEY, "Rescan", false, palette, cx),
            palette,
        ));
        if !report.denied.is_empty() {
            let names: Vec<String> = report
                .denied
                .iter()
                .map(|path| display_path(path, home.as_deref()))
                .collect();
            rows.push(setting_row(
                "Allow access",
                Some(
                    format!(
                        "macOS keeps OpenMusic out of {}. Allow it in System Settings › Privacy & Security › Files and Folders, then rescan.",
                        names.join(", ")
                    )
                    .into(),
                ),
                self.text_control(PRIVACY_KEY, "Open Settings", false, palette, cx),
                palette,
            ));
        }
        rows
    }

    pub fn local_files_view(
        &self,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let wide = card_side >= px(150.);
        div()
            .flex()
            .flex_col()
            .gap_7()
            .child(div().child(self.back_button(palette, cx)))
            .child(self.local_header(palette, cx))
            .child(self.local_body(wide, palette, cx))
            .into_any_element()
    }

    fn local_header(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        let local = &self.account.local;
        let enabled = self.local_files_on();
        let mut subtitle = vec!["Files from your computer".to_owned()];
        if enabled && local.loaded {
            let mut count = count_label(local.order.len());
            if !local.order.is_empty() {
                count.push_str(&format!(", {}", total_label(local.total_duration())));
            }
            subtitle.push(count);
        }
        let playable = enabled && local.order.iter().any(|&index| !local.files[index].missing);
        let play = if self.local_playing() && self.playback.now.playing {
            "Pause"
        } else {
            "Play"
        };
        div()
            .flex()
            .items_center()
            .gap_6()
            .child(local_cover(palette))
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
                            .child("On this Mac"),
                    )
                    .child(
                        div()
                            .text_size(px(34.))
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .w_full()
                            .child("Local Files"),
                    )
                    .child(
                        div()
                            .text_size(px(13.5))
                            .text_color(rgb(palette.muted))
                            .truncate()
                            .w_full()
                            .child(subtitle.join(" • ")),
                    )
                    .child(
                        div()
                            .mt_3()
                            .w_full()
                            .flex()
                            .items_center()
                            .gap_2()
                            .when(playable, |actions| {
                                actions.child(
                                    self.text_control(PLAY_KEY, play, true, palette, cx)
                                        .w(px(96.)),
                                )
                            })
                            .child(self.menu_button(MORE_KEY, "Local Files", palette, cx)),
                    ),
            )
    }

    fn local_toolbar(&self, palette: Palette, cx: &mut Context<Self>) -> Div {
        div()
            .mb_2()
            .flex()
            .items_center()
            .gap_2()
            .children(self.local_filter(palette))
            .child(div().flex_1())
            .child(self.local_sort_button(palette, cx))
    }

    fn local_filter(&self, palette: Palette) -> Option<impl IntoElement> {
        let input = self.account.local.filter.as_ref()?;
        Some(
            div()
                .h(px(34.))
                .flex_1()
                .max_w(px(260.))
                .min_w(px(120.))
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
                        Input::new(input)
                            .appearance(false)
                            .bordered(false)
                            .focus_bordered(false)
                            .cleanable(true),
                    ),
                ),
        )
    }

    fn local_sort_button(&self, palette: Palette, cx: &mut Context<Self>) -> Stateful<Div> {
        let label = self.account.local.sort.column.label();
        self.control(SORT_KEY, Label::Text(label), false, palette, cx)
            .h(px(34.))
            .w(px(28. + 7.2 * label.chars().count() as f32))
            .child(overlay::anchor(SORT_KEY))
    }

    fn local_message(
        &self,
        title: &'static str,
        detail: &'static str,
        action: (&'static str, &'static str),
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (key, label) = action;
        div()
            .py_8()
            .flex()
            .flex_col()
            .items_start()
            .gap_1()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(rgb(palette.muted))
                    .child(detail),
            )
            .child(
                div()
                    .mt_3()
                    .flex()
                    .items_center()
                    .gap_4()
                    .child(self.text_control(key, label, false, palette, cx))
                    .child(link(
                        "localfiles-settings",
                        "Local Files settings".to_owned(),
                        Some(Page::Settings),
                        13.,
                        palette,
                        cx,
                    )),
            )
            .into_any_element()
    }

    fn local_notes(&self, palette: Palette, cx: &mut Context<Self>) -> Option<Div> {
        let local = &self.account.local;
        let mut notes = Vec::new();
        if local.scanning {
            notes.push(note_line(local.status(), palette).into_any_element());
        }
        let missing = local.missing_count();
        if missing > 0 {
            let (count, verb) = if missing == 1 {
                ("1 song".to_owned(), "isn’t")
            } else {
                (count_label(missing), "aren’t")
            };
            notes.push(
                note_line(
                    format!("{count} {verb} available right now. Connect the drive to play them."),
                    palette,
                )
                .into_any_element(),
            );
        }
        let denied = local
            .report
            .as_ref()
            .map(|report| report.denied.clone())
            .unwrap_or_default();
        if !denied.is_empty() {
            let home = home();
            let names: Vec<String> = denied
                .iter()
                .map(|path| display_path(path, home.as_deref()))
                .collect();
            notes.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(note_line(
                        format!("OpenMusic can’t open {}.", names.join(", ")),
                        palette,
                    ))
                    .child(
                        div()
                            .id("localfiles-allow")
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(palette.accent))
                            .cursor_pointer()
                            .hover(|allow| allow.underline())
                            .child("Allow access")
                            .on_click(cx.listener(|_, _, _, cx| cx.open_url(PRIVACY_URL))),
                    )
                    .into_any_element(),
            );
        }
        (!notes.is_empty()).then(|| div().flex().flex_col().gap_1().mb_3().children(notes))
    }

    fn local_column(
        &self,
        column: Column,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let sort = self.account.local.sort;
        let active = sort.column == column;
        let label = if column == Column::Duration {
            "Time"
        } else {
            column.label()
        };
        let color = if active { palette.text } else { palette.muted };
        div()
            .id(("localfiles-column", column as usize))
            .flex()
            .items_center()
            .gap_1()
            .min_w_0()
            .cursor_pointer()
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(color))
            .hover(move |cell| cell.text_color(rgb(palette.text)))
            .child(div().min_w_0().truncate().child(label))
            .when(active, |cell| {
                cell.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(9.))
                        .text_color(rgb(palette.accent))
                        .child(if sort.descending { "▼" } else { "▲" }),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.sort_local_by(column);
                cx.notify();
            }))
    }

    fn local_columns(&self, wide: bool, palette: Palette, cx: &mut Context<Self>) -> Div {
        div()
            .h(px(32.))
            .flex()
            .items_center()
            .gap_3()
            .px_2()
            .mb_1()
            .border_b_1()
            .border_color(rgba(palette.line))
            .child(div().w(px(40.)).flex_shrink_0())
            .child(self.local_column(Column::Title, palette, cx).flex_1())
            .child(div().w_1_4().flex().min_w_0().child(self.local_column(
                Column::Album,
                palette,
                cx,
            )))
            .when(wide, |columns| {
                columns.child(
                    div()
                        .w(px(112.))
                        .flex_shrink_0()
                        .flex()
                        .child(self.local_column(Column::Added, palette, cx)),
                )
            })
            .child(
                div()
                    .w(px(48.))
                    .flex_shrink_0()
                    .flex()
                    .justify_end()
                    .child(self.local_column(Column::Duration, palette, cx)),
            )
    }

    fn local_body(&self, wide: bool, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let local = &self.account.local;
        if !self.local_files_on() {
            return self.local_message(
                "Local Files is off",
                "Turn it on to see songs from folders on this Mac. Nothing leaves this Mac.",
                (ENABLE_KEY, "Turn On"),
                palette,
                cx,
            );
        }
        if !local.loaded || (local.files.is_empty() && local.scanning) {
            return div()
                .flex()
                .flex_col()
                .children(self.local_notes(palette, cx))
                .child(skeleton_rows("localfiles-rows", 8, false, palette))
                .into_any_element();
        }
        if local.files.is_empty() {
            return self.local_message(
                "Listen to local files",
                "Add a source or turn off local files in Settings.",
                (ADD_KEY, "Add Folder…"),
                palette,
                cx,
            );
        }
        if local.order.is_empty() {
            return div()
                .flex()
                .flex_col()
                .child(self.local_toolbar(palette, cx))
                .child(
                    self.page_status(format!("No songs match “{}”.", local.query.trim()), palette),
                )
                .into_any_element();
        }
        let shown = local.limit.min(local.shown.len());
        let options = RowOptions {
            show_added: wide,
            on_click: Some(Rc::new(|app, _, index, _, cx| {
                app.play_local_row(index);
                cx.notify();
            })),
            ..RowOptions::default()
        };
        div()
            .flex()
            .flex_col()
            .child(self.local_toolbar(palette, cx))
            .children(self.local_notes(palette, cx))
            .child(self.local_columns(wide, palette, cx))
            .child(self.track_list(&local.shown[..shown], ROW, 0, &options, palette, cx))
            .child(scroll_sentinel(&local.sentinel))
            .when(shown < local.shown.len(), |list| {
                list.child(skeleton_rows("localfiles-more", 2, false, palette))
            })
            .into_any_element()
    }
}
