use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::Context;
use spotify_client::{
    Track,
    details::{Ban, BanProvider, FeedbackSet, SetPush, WriteFormat, hide_context},
    engine::playback::Skips,
    entity,
};

use super::{
    menu::Facts,
    worker::{Command, Snapshot},
};
use crate::{
    app::OpenMusic,
    areas::playback,
    ui::menu::{Group, MenuItem, TrackTarget},
};

pub const RETRY_AFTER: Duration = Duration::from_secs(30);
pub const REFRESH_EVERY: Duration = Duration::from_secs(600);

pub const EXCLUDE: &str = "Exclude from your taste profile";
pub const INCLUDE: &str = "Include in your taste profile";
pub const BAN: &str = "Don't play this artist";
pub const UNBAN: &str = "Allow this artist to play";
pub const BANNED: &str = "Ok, we won’t play music from this artist again.";
pub const ARTIST_REFUSED: &str = "You asked us not to play this artist.";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Load {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed(Instant),
}

impl Load {
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Failed(_) => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Entry {
    Excluded(String),
    Banned(String),
    Hidden(Ban),
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    count: usize,
    settled: bool,
}

#[derive(Default)]
pub struct Sets {
    pub load: Load,
    pub excluded: HashSet<String>,
    pub banned: HashSet<String>,
    pub hidden: HashMap<String, HashSet<String>>,
    pub editable: HashMap<String, Option<bool>>,
    pub last_format: Option<WriteFormat>,
    pub pushes: usize,
    pub revision: u64,
    writing: HashMap<Entry, Pending>,
    touched: HashMap<Entry, Instant>,
    requested: Option<Instant>,
    next: Option<Instant>,
}

impl Sets {
    pub fn ready(&self) -> bool {
        self.load == Load::Ready
    }

    pub fn is_excluded(&self, uri: &str) -> bool {
        self.excluded.contains(uri)
    }

    pub fn is_banned(&self, artist: &str) -> bool {
        self.banned.contains(artist)
    }

    pub fn is_hidden(&self, track: &str, context: &str) -> bool {
        self.hidden
            .get(context)
            .is_some_and(|tracks| tracks.contains(track))
    }

    pub fn hidden_count(&self) -> usize {
        self.hidden.values().map(HashSet::len).sum()
    }

    pub fn skips(&self) -> Skips {
        Skips {
            hidden: self.hidden.clone(),
            artists: self.banned.clone(),
        }
    }

    pub fn contains(&self, entry: &Entry) -> bool {
        match entry {
            Entry::Excluded(uri) => self.is_excluded(uri),
            Entry::Banned(uri) => self.is_banned(uri),
            Entry::Hidden(ban) => self.is_hidden(&ban.track, &ban.context),
        }
    }

    fn mark(&mut self, entry: &Entry, on: bool) {
        self.revision += 1;
        match entry {
            Entry::Excluded(uri) => mark_in(&mut self.excluded, uri, on),
            Entry::Banned(uri) => mark_in(&mut self.banned, uri, on),
            Entry::Hidden(ban) => self.mark_hidden(ban, on),
        }
    }

    fn mark_hidden(&mut self, ban: &Ban, hidden: bool) {
        if hidden {
            self.hidden
                .entry(ban.context.clone())
                .or_default()
                .insert(ban.track.clone());
        } else if let Some(tracks) = self.hidden.get_mut(&ban.context) {
            tracks.remove(&ban.track);
            if tracks.is_empty() {
                self.hidden.remove(&ban.context);
            }
        }
    }

    pub fn loading(&mut self) {
        if self.load != Load::Ready {
            self.load = Load::Loading;
        }
        self.requested = Some(Instant::now());
    }

    pub fn loading_now(&self) -> bool {
        self.requested.is_some()
    }

    fn newer_than_request(&self, entry: &Entry) -> bool {
        self.writing.contains_key(entry)
            || self
                .touched
                .get(entry)
                .is_some_and(|at| self.requested.is_none_or(|requested| *at >= requested))
    }

    pub fn apply(&mut self, result: Result<Snapshot, String>) {
        let now = Instant::now();
        let snapshot = match result {
            Ok(snapshot) => snapshot,
            Err(_) => {
                if self.load != Load::Ready {
                    self.load = Load::Failed(now);
                }
                self.requested = None;
                self.next = Some(now + RETRY_AFTER);
                return;
            }
        };
        let local: Vec<(Entry, bool)> = self
            .writing
            .keys()
            .chain(self.touched.keys())
            .filter(|entry| self.newer_than_request(entry))
            .map(|entry| (entry.clone(), self.contains(entry)))
            .collect();
        self.touched.clear();
        self.requested = None;
        self.next = Some(now + REFRESH_EVERY);
        self.excluded = snapshot.excluded.into_iter().collect();
        self.banned = snapshot.banned.into_iter().collect();
        self.hidden.clear();
        for ban in snapshot.hidden {
            self.mark_hidden(&ban, true);
        }
        for (entry, on) in local {
            self.mark(&entry, on);
        }
        self.revision += 1;
        self.load = Load::Ready;
    }

    pub fn due(&self) -> bool {
        self.requested.is_none() && self.next.is_some_and(|next| Instant::now() >= next)
    }

    pub fn push(&mut self, push: &SetPush) -> bool {
        self.pushes += 1;
        let mut reload = push.unreadable > 0;
        for item in &push.items {
            let entry = match (push.set, &item.context) {
                (FeedbackSet::Excluded, _) => Entry::Excluded(item.uri.clone()),
                (FeedbackSet::Banned, _) => Entry::Banned(item.uri.clone()),
                (FeedbackSet::Hidden, Some(context)) => Entry::Hidden(Ban {
                    track: item.uri.clone(),
                    context: context.clone(),
                }),
                (FeedbackSet::Hidden, None) => {
                    reload = true;
                    continue;
                }
            };
            if self.writing.contains_key(&entry) {
                continue;
            }
            self.mark(&entry, !item.removed);
            self.touched.insert(entry, Instant::now());
        }
        reload && self.load != Load::Idle
    }

    fn begin(&mut self, entry: Entry, on: bool) {
        let settled = self.contains(&entry);
        self.mark(&entry, on);
        self.writing
            .entry(entry)
            .or_insert(Pending { count: 0, settled })
            .count += 1;
    }

    fn written(&mut self, entry: &Entry, on: bool, ok: bool) {
        let mut pending = self.writing.get(entry).copied().unwrap_or(Pending {
            count: 1,
            settled: !on,
        });
        if ok {
            pending.settled = on;
        }
        pending.count = pending.count.saturating_sub(1);
        if pending.count > 0 {
            self.writing.insert(entry.clone(), pending);
            return;
        }
        self.writing.remove(entry);
        self.touched.insert(entry.clone(), Instant::now());
        self.mark(entry, pending.settled);
    }

    pub fn excluded_written(&mut self, uri: &str, excluded: bool, ok: bool) {
        self.written(&Entry::Excluded(uri.to_owned()), excluded, ok);
    }

    pub fn banned_written(&mut self, uri: &str, banned: bool, ok: bool) {
        self.written(&Entry::Banned(uri.to_owned()), banned, ok);
    }

    pub fn hidden_written(
        &mut self,
        ban: &Ban,
        hidden: bool,
        result: &Result<WriteFormat, String>,
    ) {
        if let Ok(format) = result {
            self.last_format = Some(*format);
        }
        self.written(&Entry::Hidden(ban.clone()), hidden, result.is_ok());
    }
}

fn mark_in(set: &mut HashSet<String>, uri: &str, on: bool) {
    if on {
        set.insert(uri.to_owned());
    } else {
        set.remove(uri);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    Artist(String),
    Hidden(Ban),
}

pub fn refusal(sets: &Sets, track: &Track, context: Option<&str>) -> Option<Refusal> {
    if !track.is_track() {
        return None;
    }
    if let Some(artist) = first_artist(track).filter(|artist| sets.is_banned(artist)) {
        return Some(Refusal::Artist(artist.to_owned()));
    }
    context
        .and_then(hide_context)
        .filter(|context| sets.is_hidden(&track.uri, context))
        .map(|context| {
            Refusal::Hidden(Ban {
                track: track.uri.clone(),
                context,
            })
        })
}

pub fn first_artist(track: &Track) -> Option<&str> {
    track
        .artists
        .first()
        .map(|artist| artist.uri.as_str())
        .or(track.artist_uri.as_deref())
        .filter(|uri| uri.starts_with("spotify:artist:"))
}

pub fn exclude_message(uri: &str, excluded: bool) -> &'static str {
    let playlist = !uri.starts_with("spotify:track:");
    match (playlist, excluded) {
        (false, true) => {
            "Listening to this track will have less impact on your taste profile and recommendations."
        }
        (false, false) => {
            "Listening to this track will impact your taste profile and recommendations."
        }
        (true, true) => {
            "Listening to this playlist will have less impact on your taste profile and recommendations."
        }
        (true, false) => {
            "Listening to this playlist will impact your taste profile and recommendations."
        }
    }
}

fn exclude_item(uri: &str, facts: &Facts) -> MenuItem {
    let excluded = facts.sets.is_excluded(uri);
    let uri = uri.to_owned();
    MenuItem::action(
        if excluded { INCLUDE } else { EXCLUDE },
        move |app, _, _| app.set_excluded(uri.clone(), !excluded),
    )
    .key("x")
    .symbol(if excluded {
        "plus.circle"
    } else {
        "minus.circle"
    })
    .enabled(facts.sets.ready())
}

fn hide_title(context: &str, hidden: bool) -> &'static str {
    match (context.starts_with("spotify:album:"), hidden) {
        (false, false) => "Hide in this playlist",
        (false, true) => "Show in this playlist",
        (true, false) => "Hide in this album",
        (true, true) => "Show in this album",
    }
}

fn hide_item(target: &TrackTarget, facts: &Facts) -> Option<MenuItem> {
    let context = target.context.as_deref().and_then(hide_context)?;
    let provider = BanProvider::of(target.provider.as_deref())?;
    let hidden = facts.sets.is_hidden(&target.track.uri, &context);
    if !hidden && facts.editable(&context) == Some(true) {
        return None;
    }
    let title = hide_title(&context, hidden);
    let ban = Ban {
        track: target.track.uri.clone(),
        context,
    };
    Some(
        MenuItem::action(title, move |app, _, _| {
            app.set_hidden(ban.clone(), provider, !hidden)
        })
        .symbol(if hidden { "eye" } else { "eye.slash" })
        .enabled(facts.sets.ready()),
    )
}

pub fn track_items(target: &TrackTarget, facts: &Facts) -> Vec<(Group, MenuItem)> {
    if !target.track.is_track() {
        return Vec::new();
    }
    hide_item(target, facts)
        .into_iter()
        .chain([exclude_item(&target.track.uri, facts)])
        .map(|item| (Group::Feedback, item))
        .collect()
}

pub fn artist_items(uri: &str, facts: &Facts) -> Vec<(Group, MenuItem)> {
    let banned = facts.sets.is_banned(uri);
    let artist = uri.to_owned();
    let item = MenuItem::action(if banned { UNBAN } else { BAN }, move |app, _, _| {
        app.set_artist_banned(artist.clone(), !banned)
    })
    .symbol(if banned { "play.circle" } else { "nosign" })
    .enabled(facts.sets.ready());
    vec![(Group::Feedback, item)]
}

pub fn playlist_items(uri: &str, facts: &Facts) -> Vec<(Group, MenuItem)> {
    if entity::is_liked_songs(uri) {
        return Vec::new();
    }
    let Some(playlist) = hide_context(uri).filter(|uri| uri.starts_with("spotify:playlist:"))
    else {
        return Vec::new();
    };
    vec![(Group::Feedback, exclude_item(&playlist, facts))]
}

impl OpenMusic {
    pub fn context_name(&self, context: &str) -> String {
        let loaded = self
            .playlists
            .collection
            .as_ref()
            .and_then(|(uri, result)| Some((uri, result.as_ref().ok()?)))
            .filter(|(uri, _)| hide_context(uri).as_deref() == Some(context))
            .map(|(_, collection)| collection.name.clone());
        let sidebar = || {
            self.library
                .playlists
                .iter()
                .find(|playlist| playlist.uri == context)
                .map(|playlist| playlist.name.clone())
        };
        loaded.or_else(sidebar).unwrap_or_else(|| {
            if context.starts_with("spotify:album:") {
                "this album".into()
            } else {
                "this playlist".into()
            }
        })
    }

    pub fn set_excluded(&mut self, uri: String, excluded: bool) {
        let sets = &mut self.details.sets;
        if !sets.ready() {
            return;
        }
        sets.begin(Entry::Excluded(uri.clone()), excluded);
        self.show_toast(exclude_message(&uri, excluded), None);
        self.send(Command::Exclude { uri, excluded });
    }

    pub fn set_hidden(&mut self, ban: Ban, provider: BanProvider, hidden: bool) {
        let sets = &mut self.details.sets;
        if !sets.ready() {
            return;
        }
        sets.begin(Entry::Hidden(ban.clone()), hidden);
        if hidden {
            let name = self.context_name(&ban.context);
            let again = ban.clone();
            self.show_toast(
                format!("Hidden in {name}."),
                Some(Rc::new(
                    move |app: &mut OpenMusic, cx: &mut Context<OpenMusic>| {
                        app.set_hidden(again.clone(), provider, false);
                        cx.notify();
                    },
                )),
            );
            if self.playing_in(&ban) {
                self.send(playback::worker::Command::Next);
            }
        }
        self.send(Command::Hide {
            ban,
            provider,
            hidden,
        });
    }

    pub fn set_artist_banned(&mut self, uri: String, banned: bool) {
        let sets = &mut self.details.sets;
        if !sets.ready() || !uri.starts_with("spotify:artist:") {
            return;
        }
        sets.begin(Entry::Banned(uri.clone()), banned);
        if banned {
            self.show_toast(BANNED, None);
            if self.is_saved(&uri) != Some(false) {
                self.set_saved(vec![uri.clone()], false);
            }
            if self.track().and_then(first_artist) == Some(uri.as_str()) {
                self.send(playback::worker::Command::Next);
            }
        }
        self.send(Command::Ban { uri, banned });
    }

    pub fn refuses_skipped(
        &mut self,
        track: Option<&Track>,
        context: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(refused) = track.and_then(|track| refusal(&self.details.sets, track, context))
        else {
            return false;
        };
        match refused {
            Refusal::Artist(artist) => self.show_toast(
                ARTIST_REFUSED,
                Some(Rc::new(
                    move |app: &mut OpenMusic, cx: &mut Context<OpenMusic>| {
                        app.set_artist_banned(artist.clone(), false);
                        cx.notify();
                    },
                )),
            ),
            Refusal::Hidden(ban) => {
                let name = self.context_name(&ban.context);
                self.show_toast(
                    format!("You hid this song in {name}."),
                    Some(Rc::new(
                        move |app: &mut OpenMusic, cx: &mut Context<OpenMusic>| {
                            app.set_hidden(ban.clone(), BanProvider::Context, false);
                            cx.notify();
                        },
                    )),
                );
            }
        }
        cx.notify();
        true
    }

    fn playing_in(&self, ban: &Ban) -> bool {
        let now = &self.playback.now;
        now.track
            .as_ref()
            .is_some_and(|track| track.uri == ban.track)
            && now.context_uri.as_deref().and_then(hide_context).as_deref() == Some(&ban.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SONG: &str = "spotify:track:70LcF31zb1H0PyJoS1Sx1r";
    const RADIO: &str = "spotify:playlist:37i9dQZF1E8Redacted001";
    const RADIOHEAD: &str = "spotify:artist:4Z8W4fKeB5YxbusRsdQVPb";
    const BOWIE: &str = "spotify:artist:0oSGxfWSnnOXhD2fKuz2Gy";

    fn ban() -> Ban {
        Ban {
            track: SONG.into(),
            context: RADIO.into(),
        }
    }

    #[test]
    fn keeps_writes_in_flight_when_a_snapshot_lands() {
        let mut sets = Sets::default();
        sets.apply(Ok(Snapshot::default()));
        assert!(sets.ready());
        sets.begin(Entry::Hidden(ban()), true);
        sets.begin(Entry::Excluded(RADIO.into()), true);
        sets.begin(Entry::Banned(RADIOHEAD.into()), false);
        sets.apply(Ok(Snapshot {
            excluded: vec!["spotify:playlist:other".into()],
            hidden: Vec::new(),
            banned: vec![RADIOHEAD.into(), BOWIE.into()],
        }));
        assert!(sets.is_hidden(SONG, RADIO));
        assert!(sets.is_excluded(RADIO));
        assert!(sets.is_excluded("spotify:playlist:other"));
        assert!(!sets.is_banned(RADIOHEAD));
        assert!(sets.is_banned(BOWIE));
        assert_eq!(sets.hidden_count(), 1);
    }

    #[test]
    fn rolls_back_to_what_the_server_last_took() {
        let mut sets = Sets::default();
        sets.apply(Ok(Snapshot::default()));
        sets.begin(Entry::Hidden(ban()), true);
        sets.begin(Entry::Hidden(ban()), false);
        sets.hidden_written(&ban(), true, &Err("no".into()));
        assert!(!sets.is_hidden(SONG, RADIO));
        sets.hidden_written(&ban(), false, &Err("no".into()));
        assert!(!sets.is_hidden(SONG, RADIO));
        assert!(!sets.writing.contains_key(&Entry::Hidden(ban())));
        sets.begin(Entry::Hidden(ban()), true);
        sets.begin(Entry::Hidden(ban()), false);
        sets.hidden_written(&ban(), true, &Ok(WriteFormat::Protobuf));
        assert!(!sets.is_hidden(SONG, RADIO));
        sets.hidden_written(&ban(), false, &Err("no".into()));
        assert!(sets.is_hidden(SONG, RADIO));
        sets.begin(Entry::Hidden(ban()), false);
        sets.hidden_written(&ban(), false, &Ok(WriteFormat::Protobuf));
        assert!(!sets.is_hidden(SONG, RADIO));
        sets.hidden_written(&ban(), true, &Ok(WriteFormat::Json));
        assert_eq!(sets.last_format, Some(WriteFormat::Json));

        sets.begin(Entry::Excluded(SONG.into()), true);
        sets.excluded_written(SONG, true, false);
        assert!(!sets.is_excluded(SONG));
        sets.begin(Entry::Excluded(SONG.into()), true);
        sets.excluded_written(SONG, true, true);
        assert!(sets.is_excluded(SONG));

        sets.begin(Entry::Banned(RADIOHEAD.into()), true);
        assert!(sets.is_banned(RADIOHEAD));
        sets.banned_written(RADIOHEAD, true, false);
        assert!(!sets.is_banned(RADIOHEAD));
    }

    #[test]
    fn refuses_hidden_songs_and_banned_artists() {
        let mut sets = Sets::default();
        sets.apply(Ok(Snapshot {
            hidden: vec![ban()],
            banned: vec![BOWIE.into()],
            ..Snapshot::default()
        }));
        let song = Track::placeholder(SONG);
        assert_eq!(
            refusal(&sets, &song, Some(RADIO)),
            Some(Refusal::Hidden(ban()))
        );
        assert_eq!(
            refusal(
                &sets,
                &song,
                Some("spotify:user:someone:playlist:37i9dQZF1E8Redacted001")
            ),
            Some(Refusal::Hidden(ban()))
        );
        assert_eq!(refusal(&sets, &song, Some("spotify:playlist:other")), None);
        assert_eq!(refusal(&sets, &song, None), None);
        let by_bowie = Track {
            artist_uri: Some(BOWIE.into()),
            ..Track::placeholder("spotify:track:0000000000000000000002")
        };
        assert_eq!(
            refusal(&sets, &by_bowie, None),
            Some(Refusal::Artist(BOWIE.into()))
        );
        let episode = Track {
            artist_uri: Some(BOWIE.into()),
            ..Track::placeholder("spotify:episode:0000000000000000000003")
        };
        assert_eq!(refusal(&sets, &episode, Some(RADIO)), None);
    }

    #[test]
    fn hands_the_engine_what_to_skip() {
        let mut sets = Sets::default();
        sets.apply(Ok(Snapshot {
            hidden: vec![ban()],
            banned: vec![BOWIE.into()],
            ..Snapshot::default()
        }));
        let loaded = sets.revision;
        let skips = sets.skips();
        assert!(skips.artists.contains(BOWIE));
        assert!(skips.hidden[RADIO].contains(SONG));
        sets.begin(Entry::Banned(RADIOHEAD.into()), true);
        assert!(sets.revision > loaded);
        assert!(sets.skips().artists.contains(RADIOHEAD));
    }

    #[test]
    fn finds_the_first_artist_of_a_song() {
        let mut track = Track::placeholder(SONG);
        assert_eq!(first_artist(&track), None);
        track.artist_uri = Some(BOWIE.into());
        assert_eq!(first_artist(&track), Some(BOWIE));
        track.artists = vec![spotify_client::catalog::Link {
            uri: RADIOHEAD.into(),
            name: "Radiohead".into(),
        }];
        assert_eq!(first_artist(&track), Some(RADIOHEAD));
        track.artists[0].uri = "spotify:show:0123456789abcdefghijkl".into();
        assert_eq!(first_artist(&track), None);
    }

    #[test]
    fn retries_a_failed_load_later() {
        let mut sets = Sets::default();
        assert!(!sets.due());
        sets.loading();
        assert_eq!(sets.load, Load::Loading);
        assert!(sets.loading_now());
        sets.apply(Err("offline".into()));
        assert_eq!(sets.load.name(), "failed");
        assert!(!sets.due());
        let next = sets.next.unwrap();
        assert!(next >= Instant::now() + RETRY_AFTER - Duration::from_secs(1));
        sets.next = Some(Instant::now());
        assert!(sets.due());
        sets.loading();
        assert!(!sets.due());
        sets.apply(Ok(Snapshot::default()));
        assert!(!sets.loading_now());
        assert!(sets.next.unwrap() >= Instant::now() + REFRESH_EVERY - Duration::from_secs(1));
        sets.apply(Err("offline".into()));
        assert!(sets.ready());
        sets.loading();
        assert!(sets.ready());
    }

    #[test]
    fn keeps_writes_that_land_after_a_reload_was_asked_for() {
        let mut sets = Sets::default();
        sets.apply(Ok(Snapshot::default()));
        sets.begin(Entry::Excluded(RADIO.into()), true);
        sets.loading();
        sets.excluded_written(RADIO, true, true);
        sets.apply(Ok(Snapshot::default()));
        assert!(sets.is_excluded(RADIO));
        sets.loading();
        sets.apply(Ok(Snapshot::default()));
        assert!(!sets.is_excluded(RADIO));
    }

    fn pushed(set: FeedbackSet, uri: &str, context: Option<&str>, removed: bool) -> SetPush {
        SetPush {
            set,
            items: vec![spotify_client::details::Pushed {
                uri: uri.into(),
                context: context.map(str::to_owned),
                removed,
            }],
            unreadable: 0,
        }
    }

    #[test]
    fn applies_pushes_one_entry_at_a_time() {
        let mut sets = Sets::default();
        sets.apply(Ok(Snapshot {
            excluded: vec!["spotify:playlist:other".into()],
            ..Snapshot::default()
        }));
        assert!(!sets.push(&pushed(FeedbackSet::Excluded, RADIO, None, false)));
        assert!(sets.is_excluded(RADIO));
        assert!(sets.is_excluded("spotify:playlist:other"));
        assert!(!sets.push(&pushed(FeedbackSet::Banned, RADIOHEAD, None, false)));
        assert!(sets.is_banned(RADIOHEAD));
        assert!(!sets.push(&pushed(FeedbackSet::Banned, RADIOHEAD, None, true)));
        assert!(!sets.is_banned(RADIOHEAD));
        assert!(!sets.push(&pushed(FeedbackSet::Hidden, SONG, Some(RADIO), false)));
        assert!(sets.is_hidden(SONG, RADIO));
        assert!(sets.push(&pushed(FeedbackSet::Hidden, SONG, None, true)));
        assert!(sets.is_hidden(SONG, RADIO));
        assert_eq!(sets.pushes, 5);

        sets.begin(Entry::Banned(BOWIE.into()), true);
        sets.push(&pushed(FeedbackSet::Banned, BOWIE, None, true));
        assert!(sets.is_banned(BOWIE));

        sets.loading();
        sets.push(&pushed(FeedbackSet::Excluded, SONG, None, false));
        sets.apply(Ok(Snapshot::default()));
        assert!(sets.is_excluded(SONG));
        assert!(!sets.is_excluded(RADIO));

        let mut unread = pushed(FeedbackSet::Excluded, RADIO, None, false);
        unread.unreadable = 1;
        assert!(sets.push(&unread));
        assert!(!Sets::default().push(&unread));
    }

    #[test]
    fn words_the_taste_profile_toasts_like_spotify() {
        assert_eq!(
            exclude_message(SONG, true),
            "Listening to this track will have less impact on your taste profile and recommendations."
        );
        assert_eq!(
            exclude_message(RADIO, false),
            "Listening to this playlist will impact your taste profile and recommendations."
        );
    }
}
