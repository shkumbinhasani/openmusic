use std::rc::Rc;

use gpui::{Action, App, Context, Div, FontWeight, Window, div, prelude::*, px};
use gpui_component::{ActiveTheme, WindowExt, h_flex, v_flex};

use crate::{app::OpenMusic, menus};

pub struct Row {
    pub label: &'static str,
    pub keys: Vec<String>,
}

pub struct Group {
    pub title: &'static str,
    pub rows: Vec<Row>,
}

pub fn groups(mut available: impl FnMut(&dyn Action) -> bool) -> Vec<Group> {
    menus::sheet()
        .into_iter()
        .filter_map(|(category, rows)| {
            let rows: Vec<Row> = rows
                .into_iter()
                .filter(|row| available((row.action)().as_ref()))
                .map(|row| Row {
                    label: row.label,
                    keys: row.keys.iter().map(|keys| menus::key_label(keys)).collect(),
                })
                .collect();
            (!rows.is_empty()).then_some(Group {
                title: category.title(),
                rows,
            })
        })
        .collect()
}

pub fn describe(groups: &[Group]) -> String {
    let mut lines = Vec::new();
    for group in groups {
        lines.push(format!("# {}", group.title));
        for row in &group.rows {
            lines.push(format!("{}  {}", row.label, row.keys.join(" ")));
        }
    }
    lines.join("\n")
}

fn keycap(label: &str, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .min_w(px(24.))
        .px(px(6.))
        .py(px(2.))
        .rounded(px(5.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .text_xs()
        .text_center()
        .text_color(theme.foreground)
        .child(label.to_owned())
}

fn body(groups: &[Group], cx: &App) -> Div {
    let theme = cx.theme();
    v_flex().gap_5().pb_1().children(groups.iter().map(|group| {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .pb_1()
                    .child(group.title),
            )
            .children(group.rows.iter().map(|row| {
                h_flex()
                    .justify_between()
                    .gap_4()
                    .py(px(5.))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(row.label),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_shrink_0()
                            .children(row.keys.iter().map(|key| keycap(key, cx))),
                    )
            }))
    }))
}

impl OpenMusic {
    pub fn shortcut_groups(&self, cx: &mut Context<Self>) -> Vec<Group> {
        groups(|action| cx.is_action_available(action))
    }

    pub fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.system.shortcuts && window.has_active_dialog(cx) {
            window.close_dialog(cx);
            self.system.shortcuts = false;
        } else {
            self.open_shortcuts(window, cx);
        }
    }

    pub fn open_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.system.shortcuts && window.has_active_dialog(cx) {
            return;
        }
        let groups = Rc::new(self.shortcut_groups(cx));
        let app = cx.entity().downgrade();
        self.system.shortcuts = true;
        window.open_dialog(cx, move |dialog, window, cx| {
            let tallest = (window.viewport_size().height * 0.9 - px(110.)).max(px(240.));
            let app = app.clone();
            dialog
                .title("Keyboard Shortcuts")
                .w(px(520.))
                .max_h(tallest)
                .bg(cx.theme().popover)
                .on_close(move |_, _, cx| {
                    let _ = app.update(cx, |app, cx| {
                        app.system.shortcuts = false;
                        cx.notify();
                    });
                })
                .child(body(&groups, cx))
        });
    }

    pub fn close_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.system.shortcuts && window.has_active_dialog(cx) {
            window.close_dialog(cx);
        }
        self.system.shortcuts = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::{Next, Quit, ShowShortcuts, TogglePlayback};

    #[test]
    fn lists_only_working_shortcuts() {
        let working = |action: &dyn Action| {
            action.partial_eq(&Quit)
                || action.partial_eq(&TogglePlayback)
                || action.partial_eq(&Next)
                || action.partial_eq(&ShowShortcuts)
        };
        let groups = groups(working);
        assert_eq!(
            describe(&groups),
            "# Basic\nKeyboard shortcuts  ⌘/ ?\nQuit OpenMusic  ⌘Q\n# Playback\nPlay or pause  Space\nNext  ⌘→"
        );
    }
}
