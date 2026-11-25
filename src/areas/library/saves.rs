use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use gpui::{AnyElement, Context, div, prelude::*, px};
use gpui_component::tooltip::Tooltip;
use spotify_client::{
    Track,
    library::{self, LIKED_SONGS, count_label},
};

use crate::{
    app::{OpenMusic, Page, Session},
    icons,
    overlay::{ButtonSpec, Label},
    ui::{Palette, components::icon, dock::DockItem, tracks::ROW_GROUP},
};

pub const SAVE_KEY: &str = "library-save";
pub const FOLLOW_KEY: &str = "library-follow";
pub const LIKE_CURRENT_KEY: &str = "like-current";
const LOOKUP_RETRY: Duration = Duration::from_secs(30);
const TOGGLE_WAIT: Duration = Duration::from_secs(5);
const ASKED_LIMIT: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saves {
    Like,
    Library,
    Episodes,
    Follow,
}

pub fn saves(uri: &str) -> Option<Saves> {
    if !library::savable(uri) {
        return None;
    }
    match uri.split(':').nth(1)? {
        "track" => Some(Saves::Like),
        "album" | "author" => Some(Saves::Library),
        "episode" => Some(Saves::Episodes),
        "artist" | "show" => Some(Saves::Follow),
        _ => None,
    }
}

pub fn playing_saves(uri: &str) -> Option<Saves> {
    saves(uri).filter(|kind| matches!(kind, Saves::Like | Saves::Episodes))
}

pub fn save_label(uri: &str, saved: bool) -> Option<&'static str> {
    Some(match (saves(uri)?, saved) {
        (Saves::Like, false) => "Save to your Liked Songs",
        (Saves::Like, true) => "Remove from your Liked Songs",
        (Saves::Library, false) => "Save to Your Library",
        (Saves::Library, true) => "Remove from Your Library",
        (Saves::Episodes, false) => "Save to Your Episodes",
        (Saves::Episodes, true) => "Remove from Your Episodes",
        (Saves::Follow, false) => "Follow",
        (Saves::Follow, true) => "Unfollow",
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveMenu {
    pub title: &'static str,
    pub key: Option<&'static str>,
    pub symbol: &'static str,
    pub save: bool,
}

pub fn save_menu(uri: &str, saved: bool, audiobook: bool) -> Option<SaveMenu> {
    let kind = saves(uri)?;
    let library = kind == Saves::Library || (kind == Saves::Follow && audiobook);
    let title = match (library, saved) {
        (true, false) => "Add to Your Library",
        (true, true) => "Remove from Your Library",
        (false, _) => save_label(uri, saved)?,
    };
    let key = match kind {
        Saves::Like => Some("l"),
        _ if library => Some("s"),
        Saves::Follow => Some("f"),
        _ => None,
    };
    let symbol = match (kind, saved) {
        (Saves::Like, false) => "heart",
        (Saves::Like, true) => "heart.slash",
        (Saves::Follow, false) if !library => "person.badge.plus",
        (Saves::Follow, true) if !library => "person.badge.minus",
        (_, false) => "plus.circle",
        (_, true) => "minus.circle",
    };
    Some(SaveMenu {
        title,
        key,
        symbol,
        save: !saved,
    })
}

pub fn saved_toast(uri: &str, saved: bool) -> Option<&'static str> {
    Some(match (saves(uri)?, saved) {
        (Saves::Like, true) => "Added to Liked Songs.",
        (Saves::Like, false) => "Removed from Liked Songs.",
        (Saves::Library, true) => "Added to Your Library.",
        (Saves::Library, false) => "Removed from Your Library.",
        (Saves::Episodes, true) => "Added to Your Episodes.",
        (Saves::Episodes, false) => "Removed from Your Episodes.",
        (Saves::Follow, _) => return None,
    })
}

pub fn header_uri(page: &Page) -> Option<&str> {
    match page {
        Page::Collection(uri)
        | Page::Artist(uri)
        | Page::Show(uri)
        | Page::Episode(uri)
        | Page::Track(uri) => Some(uri),
        _ => None,
    }
}

fn liked_count(detail: &str) -> Option<i64> {
    detail.split_whitespace().next()?.parse().ok()
}

fn toggled(known: Option<bool>) -> bool {
    known != Some(true)
}

fn drop_toast(added: usize) -> String {
    match added {
        0 => "Already in Liked Songs.".to_owned(),
        1 => "Added to Liked Songs.".to_owned(),
        count => format!("Added {count} songs to Liked Songs."),
    }
}

fn settle_unknown(
    waiting: &mut HashMap<String, (bool, Instant)>,
    known: impl Fn(&str) -> bool,
    now: Instant,
) -> (Vec<(String, bool)>, Vec<bool>) {
    let mut ready = Vec::new();
    let mut lost = Vec::new();
    waiting.retain(|uri, (saved, at)| {
        if known(uri) {
            ready.push((uri.clone(), *saved));
        } else if now.duration_since(*at) >= TOGGLE_WAIT {
            lost.push(*saved);
        } else {
            return true;
        }
        false
    });
    (ready, lost)
}

fn lost_toast(saved: bool) -> &'static str {
    if saved {
        "Couldn’t add this to your library. Try again."
    } else {
        "Couldn’t remove this from your library. Try again."
    }
}

fn settle_likes(
    writes: &mut HashMap<String, bool>,
    pending: &HashMap<String, Option<bool>>,
    known: &HashMap<String, bool>,
) -> bool {
    let mut failed = false;
    writes.retain(|uri, saved| {
        if pending.contains_key(uri) {
            return true;
        }
        failed |= known.get(uri) != Some(saved);
        false
    });
    failed
}

impl OpenMusic {
    pub fn toggle_saved(&mut self, uri: &str) {
        self.make_saved(uri, toggled(self.is_saved(uri)));
    }

    pub fn make_saved(&mut self, uri: &str, saved: bool) {
        if saves(uri).is_none() || self.saved.pending.contains_key(uri) {
            return;
        }
        if self.is_saved(uri).is_some() {
            self.apply_save(uri, saved);
        } else {
            self.library
                .when_known
                .insert(uri.to_owned(), (saved, Instant::now()));
            self.lookup_saved([uri.to_owned()]);
        }
    }

    fn apply_save(&mut self, uri: &str, saved: bool) {
        if self.write_saved(vec![uri.to_owned()], saved) == 0 {
            return;
        }
        if let Some(text) = saved_toast(uri, saved) {
            self.show_toast(text, None);
        }
    }

    pub fn write_saved(&mut self, uris: Vec<String>, saved: bool) -> usize {
        let changes: Vec<String> = uris
            .into_iter()
            .filter(|uri| {
                saves(uri).is_some()
                    && !self.saved.pending.contains_key(uri)
                    && self.is_saved(uri) != Some(saved)
            })
            .collect();
        let likes = changes
            .iter()
            .filter(|uri| saves(uri) == Some(Saves::Like))
            .count();
        if likes > 0 {
            self.bump_liked_count(if saved { likes as i64 } else { -(likes as i64) });
            if saved {
                self.library.liked.stale = true;
            }
            self.library.liked_writes.extend(
                changes
                    .iter()
                    .filter(|uri| saves(uri) == Some(Saves::Like))
                    .map(|uri| (uri.clone(), saved)),
            );
        }
        if likes < changes.len() {
            self.library.resync = true;
            if !saved {
                let gone: Vec<&String> = changes
                    .iter()
                    .filter(|uri| saves(uri) != Some(Saves::Like))
                    .collect();
                self.library.forget_items(&gone);
            }
        }
        let written = changes.len();
        self.set_saved(changes, saved);
        written
    }

    pub fn like_dropped(&mut self, songs: Vec<String>) {
        let unknown: Vec<String> = songs
            .iter()
            .filter(|uri| self.is_saved(uri).is_none())
            .cloned()
            .collect();
        if unknown.is_empty() {
            let added = self.write_saved(songs, true);
            self.show_toast(drop_toast(added), None);
            return;
        }
        self.lookup_saved(unknown);
        match &mut self.library.dropped {
            Some((waiting, _)) => {
                for song in songs {
                    if !waiting.contains(&song) {
                        waiting.push(song);
                    }
                }
            }
            None => self.library.dropped = Some((songs, Instant::now())),
        }
    }

    fn bump_liked_count(&mut self, delta: i64) {
        for item in self.library.items_mut(LIKED_SONGS) {
            if let Some(count) = liked_count(&item.detail) {
                item.detail = count_label((count + delta).max(0) as u64, "song", "songs");
            }
        }
    }

    pub fn like_current(&mut self) {
        if let Some(uri) = self.track().map(|track| track.uri.clone()) {
            self.toggle_saved(&uri);
        }
    }

    pub fn want_saved(&self, uri: &str) {
        if saves(uri).is_none() {
            return;
        }
        if self.is_saved(uri).is_none() {
            self.library.wanted.borrow_mut().insert(uri.to_owned());
        } else {
            self.library.sync.saw(uri);
        }
    }

    pub fn library_button(
        &self,
        uri: &str,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let kind = saves(uri)?;
        self.want_saved(uri);
        let known = self.is_saved(uri);
        let saved = known == Some(true);
        if kind == Saves::Follow {
            let text = if saved { "Following" } else { "Follow" };
            let spec = ButtonSpec {
                label: Label::Text(text),
                prominent: false,
                enabled: known.is_some(),
                tooltip: None,
            };
            return Some(
                self.glass_control(FOLLOW_KEY, spec, !self.modal_open(), palette, cx)
                    .h(px(36.))
                    .w(px(32. + 7.4 * text.chars().count() as f32))
                    .into_any_element(),
            );
        }
        let symbol = match (kind, saved) {
            (Saves::Like, false) => "heart",
            (Saves::Like, true) => "heart.fill",
            (_, false) => "plus",
            (_, true) => "checkmark",
        };
        let spec = ButtonSpec {
            label: Label::Symbol(symbol),
            prominent: saved,
            enabled: known.is_some(),
            tooltip: save_label(uri, saved).map(str::to_owned),
        };
        Some(
            self.glass_control(SAVE_KEY, spec, !self.modal_open(), palette, cx)
                .size(px(36.))
                .into_any_element(),
        )
    }
}

pub fn row_heart(
    app: &OpenMusic,
    track: &Track,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Option<AnyElement> {
    if saves(&track.uri) != Some(Saves::Like) {
        return None;
    }
    app.want_saved(&track.uri);
    let saved = app.is_saved(&track.uri) == Some(true);
    let tip = save_label(&track.uri, saved).unwrap_or_default();
    let uri = track.uri.clone();
    Some(
        div()
            .id("library-heart")
            .size(px(28.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(!saved, |heart| {
                heart
                    .opacity(0.)
                    .group_hover(ROW_GROUP, |style| style.opacity(1.))
            })
            .child(icon(
                if saved {
                    icons::HEART_FILL
                } else {
                    icons::HEART
                },
                15.,
                if saved { palette.accent } else { palette.muted },
            ))
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.toggle_saved(&uri);
                cx.notify();
            }))
            .into_any_element(),
    )
}

pub fn dock_heart(
    app: &OpenMusic,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
) -> Option<DockItem> {
    let uri = &app.track()?.uri;
    let (off, on) = match playing_saves(uri)? {
        Saves::Episodes => ("plus", "checkmark"),
        _ => ("heart", "heart.fill"),
    };
    app.want_saved(uri);
    let saved = app.is_saved(uri) == Some(true);
    let symbol = if saved { on } else { off };
    Some(DockItem::new(
        LIKE_CURRENT_KEY,
        600,
        48.,
        app.dock_control(LIKE_CURRENT_KEY, Label::Symbol(symbol), saved, palette, cx)
            .size(px(48.)),
    ))
}

pub fn tick(app: &mut OpenMusic) -> bool {
    if !matches!(app.session, Session::Connected) {
        app.library.wanted.borrow_mut().clear();
        return false;
    }
    let wanted = std::mem::take(&mut *app.library.wanted.borrow_mut());
    if !wanted.is_empty() {
        let now = Instant::now();
        let asked = &mut app.library.asked;
        if asked.len() > ASKED_LIMIT {
            asked.retain(|_, at| now.duration_since(*at) < LOOKUP_RETRY);
        }
        let fresh: Vec<String> = wanted
            .into_iter()
            .filter(|uri| {
                asked
                    .get(uri)
                    .is_none_or(|at| now.duration_since(*at) >= LOOKUP_RETRY)
            })
            .collect();
        for uri in &fresh {
            asked.insert(uri.clone(), now);
        }
        app.lookup_saved(fresh);
    }
    let (ready, lost) = settle_unknown(
        &mut app.library.when_known,
        |uri| app.saved.known.contains_key(uri),
        Instant::now(),
    );
    let mut changed = !ready.is_empty() || !lost.is_empty();
    for (uri, saved) in ready {
        app.make_saved(&uri, saved);
    }
    if let Some(&saved) = lost.last() {
        app.show_toast(lost_toast(saved), None);
    }
    if let Some((songs, at)) = app.library.dropped.clone() {
        if songs.iter().all(|uri| app.is_saved(uri).is_some()) {
            app.library.dropped = None;
            app.like_dropped(songs);
            changed = true;
        } else if at.elapsed() >= TOGGLE_WAIT {
            app.library.dropped = None;
            app.show_toast("Couldn’t add this to your library. Try again.", None);
            changed = true;
        }
    }
    if settle_likes(
        &mut app.library.liked_writes,
        &app.saved.pending,
        &app.saved.known,
    ) {
        app.library.resync = true;
    }
    if app.library.resync && app.saved.pending.is_empty() {
        app.library.resync = false;
        app.reload_library_views();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_each_kind_of_save() {
        assert_eq!(saves("spotify:track:x"), Some(Saves::Like));
        assert_eq!(saves("spotify:album:x"), Some(Saves::Library));
        assert_eq!(saves("spotify:author:x"), Some(Saves::Library));
        assert_eq!(saves("spotify:episode:x"), Some(Saves::Episodes));
        assert_eq!(saves("spotify:artist:x"), Some(Saves::Follow));
        assert_eq!(saves("spotify:show:x"), Some(Saves::Follow));
        assert_eq!(saves("spotify:playlist:x"), None);
        assert_eq!(saves("spotify:local:a:b:c:1"), None);
        assert_eq!(
            save_label("spotify:track:x", false),
            Some("Save to your Liked Songs")
        );
        assert_eq!(
            save_label("spotify:album:x", true),
            Some("Remove from Your Library")
        );
        assert_eq!(save_label("spotify:artist:x", true), Some("Unfollow"));
        assert_eq!(
            saved_toast("spotify:track:x", true),
            Some("Added to Liked Songs.")
        );
        assert_eq!(
            saved_toast("spotify:album:x", false),
            Some("Removed from Your Library.")
        );
        assert_eq!(saved_toast("spotify:artist:x", true), None);
    }

    #[test]
    fn only_songs_and_episodes_can_be_saved_while_playing() {
        assert_eq!(playing_saves("spotify:track:x"), Some(Saves::Like));
        assert_eq!(playing_saves("spotify:episode:x"), Some(Saves::Episodes));
        assert_eq!(playing_saves("spotify:local:a:b:c:1"), None);
        assert_eq!(playing_saves("spotify:album:x"), None);
        assert_eq!(playing_saves("spotify:show:x"), None);
        assert_eq!(playing_saves(""), None);
    }

    #[test]
    fn builds_the_library_menu_items() {
        let menu = |uri, saved, audiobook| save_menu(uri, saved, audiobook).unwrap();
        assert_eq!(
            menu("spotify:track:x", false, false),
            SaveMenu {
                title: "Save to your Liked Songs",
                key: Some("l"),
                symbol: "heart",
                save: true
            }
        );
        assert_eq!(
            menu("spotify:track:x", true, false).title,
            "Remove from your Liked Songs"
        );
        let album = menu("spotify:album:x", false, false);
        assert_eq!(
            (album.title, album.key, album.symbol),
            ("Add to Your Library", Some("s"), "plus.circle")
        );
        assert_eq!(
            menu("spotify:album:x", true, false).title,
            "Remove from Your Library"
        );
        let artist = menu("spotify:artist:x", true, false);
        assert_eq!(
            (artist.title, artist.key, artist.symbol, artist.save),
            ("Unfollow", Some("f"), "person.badge.minus", false)
        );
        assert_eq!(menu("spotify:show:x", false, false).title, "Follow");
        let book = menu("spotify:show:x", true, true);
        assert_eq!(
            (book.title, book.key),
            ("Remove from Your Library", Some("s"))
        );
        let episode = menu("spotify:episode:x", false, false);
        assert_eq!(
            (episode.title, episode.key),
            ("Save to Your Episodes", None)
        );
        assert_eq!(save_menu("spotify:playlist:x", false, false), None);
        assert_eq!(save_menu("spotify:local:a:b:c:1", false, false), None);
    }

    #[test]
    fn finds_the_header_item_of_a_page() {
        assert_eq!(
            header_uri(&Page::Collection("spotify:album:x".into())),
            Some("spotify:album:x")
        );
        assert_eq!(
            header_uri(&Page::Artist("spotify:artist:x".into())),
            Some("spotify:artist:x")
        );
        assert_eq!(header_uri(&Page::Liked), None);
        assert_eq!(liked_count("64 songs"), Some(64));
        assert_eq!(liked_count(""), None);
        assert_eq!(drop_toast(0), "Already in Liked Songs.");
        assert_eq!(drop_toast(1), "Added to Liked Songs.");
        assert_eq!(drop_toast(3), "Added 3 songs to Liked Songs.");
    }

    #[test]
    fn resyncs_only_once_a_like_write_has_failed() {
        let like = |id: &str| format!("spotify:track:{id}");
        let mut writes: HashMap<String, bool> = [
            (like("kept"), true),
            (like("lost"), true),
            (like("gone"), false),
            (like("late"), true),
        ]
        .into_iter()
        .collect();
        let mut pending = HashMap::from([(like("late"), None)]);
        let mut known = HashMap::from([
            (like("kept"), true),
            (like("lost"), false),
            (like("gone"), false),
            (like("late"), true),
        ]);
        assert!(settle_likes(&mut writes, &pending, &known));
        assert_eq!(writes.keys().collect::<Vec<_>>(), [&like("late")]);
        pending.clear();
        assert!(!settle_likes(&mut writes, &pending, &known));
        assert!(writes.is_empty());
        writes.insert(like("late"), true);
        known.remove(&like("late"));
        assert!(settle_likes(&mut writes, &pending, &known));
    }

    #[test]
    fn every_click_on_an_unknown_item_is_kept_until_spotify_answers() {
        let start = Instant::now();
        let mut waiting = HashMap::from([
            ("spotify:track:a".to_owned(), (true, start)),
            ("spotify:album:b".to_owned(), (false, start)),
        ]);
        let (ready, lost) = settle_unknown(&mut waiting, |_| false, start);
        assert!(ready.is_empty() && lost.is_empty());
        assert_eq!(waiting.len(), 2);
        let (ready, lost) = settle_unknown(
            &mut waiting,
            |uri| uri == "spotify:track:a",
            start + Duration::from_secs(1),
        );
        assert_eq!(ready, [("spotify:track:a".to_owned(), true)]);
        assert!(lost.is_empty());
        let (ready, lost) = settle_unknown(&mut waiting, |_| false, start + TOGGLE_WAIT);
        assert!(ready.is_empty());
        assert_eq!(lost, [false]);
        assert!(waiting.is_empty());
        assert_eq!(
            lost_toast(false),
            "Couldn’t remove this from your library. Try again."
        );
    }

    #[test]
    fn a_click_on_an_unknown_state_does_what_the_control_shows() {
        assert!(toggled(None));
        assert!(toggled(Some(false)));
        assert!(!toggled(Some(true)));
    }
}
