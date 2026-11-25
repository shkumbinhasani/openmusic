use std::path::Path;

use gpui::{App, Context, Entity, SharedString, WeakEntity, Window, div, prelude::*, px};
use gpui_component::{
    ActiveTheme, WindowExt,
    dialog::{Dialog, DialogButtonProps},
    input::{Input, InputState},
};
use spotify_client::social::display_name;

use super::worker::Command;
use crate::app::{OpenMusic, Session};

pub const MISSING_NAME: &str = "Display name is required.";
pub const SAVE_FAILED: &str = "Failed to save profile changes. Please try again.";
pub const NAME_NOTE: &str = "People see this name on your profile, in Jams and in Friend Activity.";

pub struct NameEditor {
    pub input: Entity<InputState>,
    pub original: String,
    pub error: Option<SharedString>,
    pub saving: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum NameCheck {
    Empty,
    Unchanged,
    Changed(String),
}

pub fn check_name(text: &str, original: &str) -> NameCheck {
    match display_name(text) {
        None => NameCheck::Empty,
        Some(name) if name == original.trim() => NameCheck::Unchanged,
        Some(name) => NameCheck::Changed(name),
    }
}

impl OpenMusic {
    pub fn name_locked(&self) -> bool {
        self.account
            .profiles
            .profile
            .iter()
            .filter_map(|(_, result)| result.as_ref().ok())
            .any(|view| view.profile.is_current_user && view.profile.name_locked)
    }

    pub fn can_edit_name(&self) -> bool {
        matches!(self.session, Session::Connected)
            && self.account().is_some()
            && !self.name_locked()
    }

    pub fn edit_display_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.account.name_editor.is_some() || !self.can_edit_name() {
            return;
        }
        let Some(original) = self.account().map(|account| account.name.clone()) else {
            return;
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Add a display name")
                .default_value(original.clone())
        });
        let editor = cx.new(|_| NameEditor {
            input: input.clone(),
            original,
            error: None,
            saving: false,
        });
        self.account.name_editor = Some(editor.clone());
        let app = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            name_dialog(dialog, &editor, &app, cx)
        });
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    pub fn submit_name(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.account.name_editor.clone() else {
            return true;
        };
        let (original, saving) = {
            let editor = editor.read(cx);
            (editor.original.clone(), editor.saving)
        };
        if saving {
            return false;
        }
        match check_name(text, &original) {
            NameCheck::Empty => {
                editor.update(cx, |editor, cx| {
                    editor.error = Some(MISSING_NAME.into());
                    cx.notify();
                });
                false
            }
            NameCheck::Unchanged => {
                self.account.name_editor = None;
                true
            }
            NameCheck::Changed(name) => {
                editor.update(cx, |editor, cx| {
                    editor.error = None;
                    editor.saving = true;
                    cx.notify();
                });
                self.send(Command::SetName(name));
                cx.notify();
                false
            }
        }
    }

    pub fn close_name_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.account.name_editor.take().is_some() {
            window.close_dialog(cx);
        }
    }

    fn rename_here(&mut self, name: &str) {
        if let Some(info) = self.account.info.as_mut() {
            info.name = name.to_owned();
        }
        let own = self.account.info.as_ref().map(|info| info.uri.clone());
        if let Some((uri, Ok(view))) = self.account.profiles.profile.as_mut()
            && (view.profile.is_current_user || own.as_ref() == Some(uri))
        {
            view.profile.name = name.to_owned();
        }
    }
}

fn name_dialog(
    dialog: Dialog,
    editor: &Entity<NameEditor>,
    app: &WeakEntity<OpenMusic>,
    cx: &mut App,
) -> Dialog {
    let state = editor.read(cx);
    let input = state.input.clone();
    let saving = state.saving;
    let error = state.error.clone();
    let muted = cx.theme().muted_foreground;
    let danger = cx.theme().danger;
    let submit = {
        let (editor, app) = (editor.clone(), app.clone());
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            let text = editor.read(cx).input.read(cx).value().to_string();
            app.update(cx, |app, cx| app.submit_name(&text, cx))
                .unwrap_or(true)
        }
    };
    let forget = {
        let app = app.clone();
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            let _ = app.update(cx, |app, cx| {
                app.account.name_editor = None;
                cx.notify();
            });
            true
        }
    };
    dialog
        .title("Profile details")
        .w(px(420.))
        .bg(cx.theme().popover)
        .confirm()
        .button_props(
            DialogButtonProps::default()
                .ok_text(if saving { "Saving…" } else { "Save" })
                .cancel_text("Cancel"),
        )
        .on_ok(submit)
        .on_cancel(forget)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .pb_1()
                .child(div().text_size(px(12.5)).text_color(muted).child("Name"))
                .child(Input::new(&input).disabled(saving))
                .map(|column| match error {
                    Some(error) => {
                        column.child(div().text_size(px(12.5)).text_color(danger).child(error))
                    }
                    None => {
                        column.child(div().text_size(px(12.)).text_color(muted).child(NAME_NOTE))
                    }
                }),
        )
}

pub fn on_saved(
    app: &mut OpenMusic,
    name: String,
    done: bool,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    if done {
        app.rename_here(&name);
        app.close_name_editor(window, cx);
        app.send(Command::RefreshSoon);
        return;
    }
    match app.account.name_editor.clone() {
        Some(editor) => editor.update(cx, |editor, cx| {
            editor.saving = false;
            editor.error = Some(SAVE_FAILED.into());
            cx.notify();
        }),
        None => app.show_toast(SAVE_FAILED, None),
    }
}

pub fn describe(app: &OpenMusic, cx: &App) -> String {
    let name = app
        .account()
        .map_or("unknown", |account| account.name.as_str());
    let editor = app
        .account
        .name_editor
        .as_ref()
        .map(|editor| editor.read(cx));
    format!(
        "name={name}\nlocked={}\nopen={}\ntyped={}\nsaving={}\nerror={}",
        app.name_locked(),
        editor.is_some(),
        editor.map_or(String::new(), |editor| editor
            .input
            .read(cx)
            .value()
            .to_string()),
        editor.is_some_and(|editor| editor.saving),
        editor
            .and_then(|editor| editor.error.clone())
            .unwrap_or_else(|| "none".into()),
    )
}

pub fn debug(
    app: &mut OpenMusic,
    rest: &str,
    directory: &Path,
    window: &mut Window,
    cx: &mut Context<OpenMusic>,
) {
    let (word, text) = rest.split_once(' ').unwrap_or((rest, ""));
    match word {
        "edit" => app.edit_display_name(window, cx),
        "close" => app.close_name_editor(window, cx),
        "type" => {
            if let Some(editor) = app.account.name_editor.clone() {
                let input = editor.read(cx).input.clone();
                input.update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
            }
        }
        "submit" => {
            if let Some(editor) = app.account.name_editor.clone() {
                let typed = editor.read(cx).input.read(cx).value().to_string();
                if app.submit_name(&typed, cx) {
                    window.close_dialog(cx);
                }
            }
        }
        "set" => {
            if let Some(name) = display_name(text) {
                app.send(Command::SetName(name));
            }
        }
        _ => {}
    }
    let _ = std::fs::write(directory.join("display-name.txt"), describe(app, cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_the_new_name() {
        assert_eq!(check_name("   ", "Someone"), NameCheck::Empty);
        assert_eq!(check_name(" Someone ", "Someone"), NameCheck::Unchanged);
        assert_eq!(
            check_name("Someone Else", "Someone"),
            NameCheck::Changed("Someone Else".into())
        );
        assert_eq!(check_name(" Someone\t", " Someone "), NameCheck::Unchanged);
    }
}
