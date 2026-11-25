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
pub const SHUFFLE: &str = "icons/shuffle.svg";
pub const SHUFFLE_SMART: &str = "icons/shuffle-smart.svg";
pub const REPEAT: &str = "icons/repeat.svg";
pub const REPEAT_ONE: &str = "icons/repeat-one.svg";
pub const QUEUE: &str = "icons/queue.svg";
pub const DEVICES: &str = "icons/devices.svg";
pub const HEART: &str = "icons/heart.svg";
pub const HEART_FILL: &str = "icons/heart-fill.svg";
pub const PLUS: &str = "icons/plus.svg";
pub const CHECK: &str = "icons/check.svg";
pub const FOLDER: &str = "icons/folder.svg";
pub const PIN: &str = "icons/pin.svg";
pub const LOCK: &str = "icons/lock.svg";
pub const MORE: &str = "icons/more.svg";
pub const MOON: &str = "icons/moon.svg";
pub const SPEED: &str = "icons/speed.svg";
pub const PIP: &str = "icons/pip.svg";
pub const PERSON: &str = "icons/person.svg";
pub const EXPLICIT: &str = "icons/explicit.svg";
pub const LOCAL: &str = "icons/local.svg";
pub const CLOSE: &str = "icons/close.svg";

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
    (
        SHUFFLE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M3 7h3.2c2 0 3.3.9 4.4 2.7l2.8 4.6c1.1 1.8 2.4 2.7 4.4 2.7H21"/><path d="M3 17h3.2c1.6 0 2.7-.6 3.6-1.7"/><path d="M14.2 8.7c.9-1.1 2-1.7 3.6-1.7H21"/><path d="m18 4 3 3-3 3"/><path d="m18 14 3 3-3 3"/></svg>"#,
    ),
    (
        SHUFFLE_SMART,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M10 2.5c.4 3.9 2.1 5.9 6.5 6.5-4.4.6-6.1 2.6-6.5 6.5-.4-3.9-2.1-5.9-6.5-6.5 4.4-.6 6.1-2.6 6.5-6.5z"/><path d="M18 13c.3 2.3 1.2 3.3 3.5 3.6-2.3.3-3.2 1.3-3.5 3.6-.3-2.3-1.2-3.3-3.5-3.6 2.3-.3 3.2-1.3 3.5-3.6z"/></svg>"#,
    ),
    (
        REPEAT,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M4 11.5V9.5A3.5 3.5 0 0 1 7.5 6H19"/><path d="m16 3 3 3-3 3"/><path d="M20 12.5v2a3.5 3.5 0 0 1-3.5 3.5H5"/><path d="m8 21-3-3 3-3"/></svg>"#,
    ),
    (
        REPEAT_ONE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M4 11.5V9.5A3.5 3.5 0 0 1 7.5 6H19"/><path d="m16 3 3 3-3 3"/><path d="M20 12.5v2a3.5 3.5 0 0 1-3.5 3.5H5"/><path d="m8 21-3-3 3-3"/><path d="M11 10.6 12.4 9.6v5" stroke-width="1.7"/></svg>"#,
    ),
    (
        QUEUE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M4 6h16M4 12h9M4 18h9"/><path d="M16.5 14.2v6.6l4.6-3.3z" fill="black"/></svg>"#,
    ),
    (
        DEVICES,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round" stroke-width="1.9"><rect x="2.5" y="4" width="8" height="15" rx="1.8"/><rect x="13" y="3" width="8.5" height="18" rx="2"/><circle cx="17.25" cy="14.6" r="2.6"/><circle cx="17.25" cy="7.4" r=".5" fill="black"/></svg>"#,
    ),
    (
        HEART,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M12 20.3s-7.4-4.5-9.2-9.2C1.7 8 3.5 4.6 7 4.6c2 0 3.8 1.2 5 3.1 1.2-1.9 3-3.1 5-3.1 3.5 0 5.3 3.4 4.2 6.5-1.8 4.7-9.2 9.2-9.2 9.2z"/></svg>"#,
    ),
    (
        HEART_FILL,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 20.3s-7.4-4.5-9.2-9.2C1.7 8 3.5 4.6 7 4.6c2 0 3.8 1.2 5 3.1 1.2-1.9 3-3.1 5-3.1 3.5 0 5.3 3.4 4.2 6.5-1.8 4.7-9.2 9.2-9.2 9.2z"/></svg>"#,
    ),
    (
        PLUS,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round" stroke-width="2.4"><path d="M12 5v14M5 12h14"/></svg>"#,
    ),
    (
        CHECK,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round" stroke-width="2.4"><path d="m5 12.5 4.5 4.5L19 7.5"/></svg>"#,
    ),
    (
        FOLDER,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M3 7.2C3 6 4 5 5.2 5h4.3c.6 0 1.1.2 1.5.6L12.4 7h6.4C20 7 21 8 21 9.2v8.6c0 1.2-1 2.2-2.2 2.2H5.2C4 20 3 19 3 17.8z"/></svg>"#,
    ),
    (
        PIN,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M9 3h6l-1 6 3.5 3.5V14H13v6l-1 1.5-1-1.5v-6H6.5v-1.5L10 9z"/></svg>"#,
    ),
    (
        LOCK,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="5" y="10.5" width="14" height="10" rx="2"/><path d="M8 10.5V8a4 4 0 0 1 8 0v2.5" fill="none" stroke="black" stroke-width="2"/></svg>"#,
    ),
    (
        MORE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="5.5" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="18.5" cy="12" r="2"/></svg>"#,
    ),
    (
        MOON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M20.5 13.3A8.5 8.5 0 1 1 10.7 3.5a6.6 6.6 0 0 0 9.8 9.8z"/></svg>"#,
    ),
    (
        SPEED,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-linecap="round" stroke-linejoin="round" stroke-width="2"><path d="M4.3 17.5a8.5 8.5 0 1 1 15.4 0"/><path d="m12 14 4-4.8"/><circle cx="12" cy="14" r="1.7" fill="black"/></svg>"#,
    ),
    (
        PIP,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="2.5" y="4.5" width="19" height="15" rx="2.2" fill="none" stroke="black" stroke-width="1.9"/><rect x="11.5" y="11" width="7.5" height="6" rx="1.2"/></svg>"#,
    ),
    (
        PERSON,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><circle cx="12" cy="8" r="4"/><path d="M4 20.2C4 16.4 7.6 14 12 14s8 2.4 8 6.2c0 .5-.4.8-.8.8H4.8c-.4 0-.8-.3-.8-.8z"/></svg>"#,
    ),
    (
        EXPLICIT,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path fill-rule="evenodd" d="M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2zm4.5 4v10H15v-2h-3.5v-2h3.2v-2h-3.2V9H15V7z"/></svg>"#,
    ),
    (
        LOCAL,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M14 2.8H7A1.8 1.8 0 0 0 5.2 4.6v14.8A1.8 1.8 0 0 0 7 21.2h10a1.8 1.8 0 0 0 1.8-1.8V7.6z" fill="none" stroke="black" stroke-width="1.8" stroke-linejoin="round"/><path d="M14 2.8v4.8h4.8" fill="none" stroke="black" stroke-width="1.8" stroke-linejoin="round"/><circle cx="10.2" cy="16.2" r="1.9"/><path d="M11.3 16.2v-5.6l3.4-.9v2l-2 .5v4z"/></svg>"#,
    ),
    (
        CLOSE,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.4" stroke-linecap="round"><path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/></svg>"#,
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
