use std::{collections::HashMap, path::Path};

use gpui::{AnyElement, Context, SharedString, div, prelude::*, px, rgb};
use gpui_component::{Disableable, switch::Switch};
use spotify_client::social::{Privacy, PrivacyField};

use super::{view::setting_row, worker::Command};
use crate::{app::OpenMusic, ui::Palette};

pub const RETRY_KEY: &str = "settings-privacy-retry";
pub const LOAD_FAILED: &str = "Couldn’t load what others see on your profile.";
pub const SAVE_FAILED: &str = "Couldn’t change what others see on your profile. Try again.";

pub const ROWS: [(PrivacyField, &str, &str); 2] = [
    (
        PrivacyField::ShowFollows,
        "Followers and following",
        "On your profile, people can see who’s following you and who you’re following.",
    ),
    (
        PrivacyField::ShowPlaylists,
        "Public playlists",
        "On your profile, people can see the playlists you’ve made public.",
    ),
];

#[derive(Default)]
pub struct PrivacyState {
    pub value: Option<Privacy>,
    pub error: Option<String>,
    pub loading: bool,
    pub pending: HashMap<PrivacyField, bool>,
}

impl PrivacyState {
    pub fn begin(&mut self, field: PrivacyField, value: bool) -> bool {
        let Some(current) = self.value.as_mut() else {
            return false;
        };
        if self.pending.contains_key(&field) || field.get(current) == value {
            return false;
        }
        self.pending.insert(field, field.get(current));
        field.set(current, value);
        true
    }

    pub fn learn(&mut self, fresh: Privacy) {
        let mut merged = fresh;
        if let Some(current) = self.value {
            for field in self.pending.keys() {
                field.set(&mut merged, field.get(&current));
            }
        }
        self.value = Some(merged);
        self.error = None;
    }

    pub fn finish(
        &mut self,
        field: PrivacyField,
        value: bool,
        saved: bool,
        now: Option<Privacy>,
    ) -> bool {
        let before = self.pending.remove(&field);
        match (saved, now) {
            (true, Some(now)) => {
                self.learn(now);
                field.get(&now) == value
            }
            (true, None) => true,
            (false, _) => {
                if let (Some(before), Some(current)) = (before, self.value.as_mut()) {
                    field.set(current, before);
                }
                false
            }
        }
    }

    pub fn describe(&self) -> String {
        let flag = |field: PrivacyField| {
            self.value
                .map_or("unknown".to_owned(), |value| field.get(&value).to_string())
        };
        let mut pending: Vec<&str> = self.pending.keys().map(|field| field.path()).collect();
        pending.sort_unstable();
        format!(
            "allow_follows={} show_follows={} show_playlists={} loading={} pending={} error={}",
            flag(PrivacyField::AllowFollows),
            flag(PrivacyField::ShowFollows),
            flag(PrivacyField::ShowPlaylists),
            self.loading,
            pending.join(","),
            self.error.as_deref().unwrap_or("none"),
        )
    }
}

pub fn field_named(name: &str) -> Option<PrivacyField> {
    match name {
        "follows" | "show_follows" => Some(PrivacyField::ShowFollows),
        "playlists" | "show_published_playlists" => Some(PrivacyField::ShowPlaylists),
        "allow" | "allow_follows" => Some(PrivacyField::AllowFollows),
        _ => None,
    }
}

impl OpenMusic {
    pub fn refresh_privacy(&mut self) {
        let privacy = &mut self.account.privacy;
        if !privacy.loading {
            privacy.loading = true;
            self.send(Command::Privacy);
        }
    }

    pub fn set_privacy(&mut self, field: PrivacyField, value: bool) {
        if self.account.privacy.begin(field, value) {
            self.send(Command::SetPrivacy { field, value });
        }
    }

    pub fn privacy_rows(&self, palette: Palette, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let state = &self.account.privacy;
        if let (None, Some(error), false) = (&state.value, &state.error, state.loading) {
            return vec![setting_row(
                error.clone(),
                Some("Check your connection, then try again.".into()),
                self.text_control(RETRY_KEY, "Try Again", false, palette, cx),
                palette,
            )];
        }
        ROWS.iter()
            .map(|&(field, title, description)| {
                let control = match state.value {
                    Some(value) => privacy_switch(
                        field,
                        field.get(&value),
                        state.pending.contains_key(&field),
                        cx,
                    )
                    .into_any_element(),
                    None => div()
                        .text_size(px(13.))
                        .text_color(rgb(palette.muted))
                        .child("Checking…")
                        .into_any_element(),
                };
                setting_row(title, Some(description.into()), control, palette)
            })
            .collect()
    }
}

fn privacy_switch(
    field: PrivacyField,
    checked: bool,
    pending: bool,
    cx: &mut Context<OpenMusic>,
) -> impl IntoElement {
    let app = cx.entity().downgrade();
    Switch::new(SharedString::from(format!("privacy-{}", field.path())))
        .checked(checked)
        .disabled(pending)
        .on_click(move |value, _, cx| {
            let _ = app.update(cx, |app, cx| {
                app.set_privacy(field, *value);
                cx.notify();
            });
        })
}

pub fn on_privacy(app: &mut OpenMusic, result: Result<Privacy, String>) {
    let privacy = &mut app.account.privacy;
    privacy.loading = false;
    match result {
        Ok(fresh) => privacy.learn(fresh),
        Err(message) => privacy.error = Some(message),
    }
}

pub fn on_saved(
    app: &mut OpenMusic,
    field: PrivacyField,
    value: bool,
    saved: bool,
    now: Option<Privacy>,
) {
    if !app.account.privacy.finish(field, value, saved, now) {
        app.show_toast(SAVE_FAILED, None);
    }
}

pub fn debug(app: &mut OpenMusic, rest: &str, directory: &Path) {
    let words: Vec<&str> = rest.split_whitespace().collect();
    match words.as_slice() {
        ["refresh"] => app.refresh_privacy(),
        [name, state] => {
            if let Some(field) = field_named(name) {
                app.set_privacy(field, *state != "off");
            }
        }
        _ => {}
    }
    let _ = std::fs::write(
        directory.join("privacy.txt"),
        app.account.privacy.describe(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open() -> Privacy {
        Privacy {
            allow_follows: true,
            show_follows: true,
            show_playlists: true,
        }
    }

    #[test]
    fn flips_a_switch_and_keeps_the_server_answer() {
        let mut state = PrivacyState::default();
        assert!(!state.begin(PrivacyField::ShowFollows, false));
        state.learn(open());
        assert!(!state.begin(PrivacyField::ShowFollows, true));
        assert!(state.begin(PrivacyField::ShowFollows, false));
        assert!(!state.begin(PrivacyField::ShowFollows, true));
        assert!(!state.value.unwrap().show_follows);

        state.learn(open());
        assert!(!state.value.unwrap().show_follows);

        let saved = Privacy {
            show_follows: false,
            ..open()
        };
        assert!(state.finish(PrivacyField::ShowFollows, false, true, Some(saved)));
        assert!(state.pending.is_empty());
        assert_eq!(state.value, Some(saved));
    }

    #[test]
    fn rolls_back_a_refused_or_failed_switch() {
        let mut state = PrivacyState::default();
        state.learn(open());
        state.begin(PrivacyField::ShowPlaylists, false);
        assert!(!state.finish(PrivacyField::ShowPlaylists, false, false, None));
        assert_eq!(state.value, Some(open()));

        state.begin(PrivacyField::ShowPlaylists, false);
        assert!(!state.finish(PrivacyField::ShowPlaylists, false, true, Some(open())));
        assert_eq!(state.value, Some(open()));

        state.begin(PrivacyField::ShowPlaylists, false);
        assert!(state.finish(PrivacyField::ShowPlaylists, false, true, None));
        assert!(!state.value.unwrap().show_playlists);
    }

    #[test]
    fn describes_and_names_privacy_fields() {
        let mut state = PrivacyState::default();
        assert!(state.describe().starts_with("allow_follows=unknown"));
        state.learn(open());
        state.begin(PrivacyField::ShowFollows, false);
        assert_eq!(
            state.describe(),
            "allow_follows=true show_follows=false show_playlists=true loading=false pending=show_follows error=none"
        );
        assert_eq!(field_named("follows"), Some(PrivacyField::ShowFollows));
        assert_eq!(field_named("playlists"), Some(PrivacyField::ShowPlaylists));
        assert_eq!(field_named("allow"), Some(PrivacyField::AllowFollows));
        assert_eq!(field_named("other"), None);
    }
}
