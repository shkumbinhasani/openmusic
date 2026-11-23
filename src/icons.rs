use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

pub const PLAY: &str = "icons/play.svg";
pub const PAUSE: &str = "icons/pause.svg";
pub const NEXT: &str = "icons/next.svg";
pub const PREVIOUS: &str = "icons/previous.svg";
pub const SEARCH: &str = "icons/search.svg";
pub const HOME: &str = "icons/home.svg";
pub const SPEAKER: &str = "icons/speaker.svg";
pub const DEVICE: &str = "icons/device.svg";
pub const NOTE: &str = "icons/note.svg";
pub const LYRICS: &str = "icons/lyrics.svg";
pub const BACK: &str = "icons/back.svg";

const ICONS: &[(&str, &str)] = &[
    (
        BACK,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="M15 5 8 12l7 7"/></svg>"#,
    ),
    (
        LYRICS,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill-rule="evenodd" d="M12 3c5.2 0 9 3.4 9 7.6s-3.8 7.6-9 7.6c-.7 0-1.3-.1-1.9-.2L6 20.6v-4.3c-1.9-1.4-3-3.4-3-5.7C3 6.4 6.8 3 12 3zM8.3 8.6a1.5 1.5 0 0 0 0 3c.2 0 .3 0 .5-.1-.2.6-.6 1-1.2 1.3l.5.8c1.5-.6 2.2-1.9 2.2-3.3 0-1-.8-1.7-2-1.7zm5 0a1.5 1.5 0 0 0 0 3c.2 0 .3 0 .5-.1-.2.6-.6 1-1.2 1.3l.5.8c1.5-.6 2.2-1.9 2.2-3.3 0-1-.8-1.7-2-1.7z"/></svg>"#,
    ),
    (
        PLAY,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M7 4.9v14.2a1.1 1.1 0 0 0 1.68.94l11.3-7.1a1.1 1.1 0 0 0 0-1.88L8.68 3.96A1.1 1.1 0 0 0 7 4.9z"/></svg>"#,
    ),
    (
        PAUSE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="5.5" y="4" width="4.6" height="16" rx="1.4"/><rect x="13.9" y="4" width="4.6" height="16" rx="1.4"/></svg>"#,
    ),
    (
        NEXT,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M2.5 6.4v11.2a.95.95 0 0 0 1.48.79L12 13.1v4.5a.95.95 0 0 0 1.48.79l8.2-5.6a.95.95 0 0 0 0-1.58l-8.2-5.6A.95.95 0 0 0 12 6.4v4.5L3.98 5.6A.95.95 0 0 0 2.5 6.4z"/></svg>"#,
    ),
    (
        PREVIOUS,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M21.5 6.4v11.2a.95.95 0 0 1-1.48.79L12 13.1v4.5a.95.95 0 0 1-1.48.79l-8.2-5.6a.95.95 0 0 1 0-1.58l8.2-5.6A.95.95 0 0 1 12 6.4v4.5l8.02-5.3a.95.95 0 0 1 1.48.8z"/></svg>"#,
    ),
    (
        SEARCH,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.3" stroke-linecap="round"><circle cx="10.5" cy="10.5" r="6.3"/><path d="M15.3 15.3 20.5 20.5"/></svg>"#,
    ),
    (
        HOME,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill-rule="evenodd" d="M12 2.5a9.5 9.5 0 1 1 0 19 9.5 9.5 0 0 1 0-19zm0 1.9a7.6 7.6 0 1 0 0 15.2 7.6 7.6 0 0 0 0-15.2z"/><path d="M10 8.3v7.4a.7.7 0 0 0 1.06.6l5.9-3.7a.7.7 0 0 0 0-1.2l-5.9-3.7A.7.7 0 0 0 10 8.3z"/></svg>"#,
    ),
    (
        SPEAKER,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M3 9.3v5.4c0 .6.5 1.1 1.1 1.1H7l4.3 3.6c.7.6 1.7.1 1.7-.8V5.4c0-.9-1-1.4-1.7-.8L7 8.2H4.1C3.5 8.2 3 8.7 3 9.3z"/><path d="M16 8.5a5 5 0 0 1 0 7" fill="none" stroke="black" stroke-width="1.9" stroke-linecap="round"/><path d="M18.6 5.8a8.8 8.8 0 0 1 0 12.4" fill="none" stroke="black" stroke-width="1.9" stroke-linecap="round"/></svg>"#,
    ),
    (
        DEVICE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.9" stroke-linejoin="round"><rect x="3" y="4.5" width="13" height="10" rx="1.6"/><path d="M1.8 18h15.4" stroke-linecap="round"/><rect x="18.2" y="8" width="4" height="10" rx="1.2"/></svg>"#,
    ),
    (
        NOTE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M19 3.6v11.9a3.3 3.3 0 1 1-1.9-3V7.6L10 9.2v8.3a3.3 3.3 0 1 1-1.9-3V6.4c0-.6.4-1.1 1-1.2l8.6-2c.7-.2 1.3.3 1.3 1z"/></svg>"#,
    ),
];

pub struct Icons;

impl AssetSource for Icons {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, svg)| Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}
