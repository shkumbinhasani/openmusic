use std::{
    collections::{HashMap, HashSet, hash_map::RandomState},
    hash::BuildHasher,
    path::{Path, PathBuf},
    time::SystemTime,
};

use gpui::Context;
use spotify_client::Track;

use super::{
    local::{LocalLibrary, MISSING_FILE, ROW, display_path},
    local_scan::{LOCAL_PREFIX, LocalFile, home, is_local},
    worker::Command,
};
use crate::{
    app::{OpenMusic, Page, Session},
    ui::menu::Row,
};

pub const TOLERANCE_SECONDS: u32 = 2;
pub const LOCAL_OFF: &str = "Turn on Local Files in Settings to play files from this Mac.";
pub const STILL_CHECKING: &str =
    "OpenMusic is still looking through your folders. Try again in a moment.";
pub const NOT_ON_THIS_MAC: &str =
    "This file isn’t on this Mac. Add the folder it’s in to Local Files in Settings.";
pub const NONE_ON_THIS_MAC: &str =
    "None of these files are on this Mac. Add the folders they’re in to Local Files in Settings.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalRef {
    pub artist: String,
    pub album: String,
    pub title: String,
    pub seconds: Option<u32>,
}

impl LocalRef {
    pub fn parse(uri: &str) -> Option<Self> {
        let parts: Vec<&str> = uri.strip_prefix(LOCAL_PREFIX)?.split(':').collect();
        let [artist, album, title, seconds] = parts.as_slice() else {
            return None;
        };
        Some(Self {
            artist: decode(artist),
            album: decode(album),
            title: decode(title),
            seconds: seconds.parse().ok(),
        })
    }

    pub fn key(&self) -> Key {
        Key::new(&self.artist, &self.album, &self.title)
    }

    pub fn gap(&self, duration_ms: u32) -> Option<u32> {
        let Some(seconds) = self.seconds else {
            return Some(0);
        };
        let gap = (duration_ms / 1000).abs_diff(seconds);
        (gap <= TOLERANCE_SECONDS).then_some(gap)
    }
}

pub fn decode(part: &str) -> String {
    let bytes = part.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escaped = (bytes[at] == b'%')
            .then(|| Some(hex(*bytes.get(at + 1)?)? << 4 | hex(*bytes.get(at + 2)?)?))
            .flatten();
        match (bytes[at], escaped) {
            (_, Some(byte)) => {
                decoded.push(byte);
                at += 3;
            }
            (b'+', None) => {
                decoded.push(b' ');
                at += 1;
            }
            (byte, None) => {
                decoded.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex(byte: u8) -> Option<u8> {
    char::from(byte).to_digit(16).map(|digit| digit as u8)
}

pub fn fold(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key(String);

impl Key {
    pub fn new(artist: &str, album: &str, title: &str) -> Self {
        Self(format!(
            "{}\u{1f}{}\u{1f}{}",
            fold(artist),
            fold(album),
            fold(title)
        ))
    }

    fn of(file: &LocalFile) -> Self {
        Self::new(&file.artist, &file.album, &file.title)
    }
}

#[derive(Debug, Default)]
pub struct LocalIndex {
    by_uri: HashMap<String, usize>,
    by_key: HashMap<Key, Vec<usize>>,
}

impl LocalIndex {
    pub fn build(files: &[LocalFile]) -> Self {
        let mut index = Self::default();
        for (position, file) in files.iter().enumerate() {
            let kept = index.by_uri.entry(file.uri.clone()).or_insert(position);
            if files[*kept].missing && !file.missing {
                *kept = position;
            }
            index
                .by_key
                .entry(Key::of(file))
                .or_default()
                .push(position);
        }
        index
    }

    pub fn exact(&self, uri: &str) -> Option<usize> {
        self.by_uri.get(uri).copied()
    }

    pub fn find(&self, files: &[LocalFile], uri: &str) -> Option<usize> {
        if let Some(position) = self.exact(uri)
            && files.get(position).is_some_and(|file| !file.missing)
        {
            return Some(position);
        }
        let reference = LocalRef::parse(uri)?;
        self.by_key
            .get(&reference.key())?
            .iter()
            .filter_map(|&position| {
                let file = files.get(position)?;
                let gap = reference.gap(file.duration_ms)?;
                Some((file.missing, gap, position))
            })
            .min()
            .map(|(_, _, position)| position)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unplayable {
    Off,
    Checking,
    NotFound,
    Missing,
    NoneFound,
}

impl Unplayable {
    pub fn message(self) -> &'static str {
        match self {
            Self::Off => LOCAL_OFF,
            Self::Checking => STILL_CHECKING,
            Self::NotFound => NOT_ON_THIS_MAC,
            Self::Missing => MISSING_FILE,
            Self::NoneFound => NONE_ON_THIS_MAC,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Checking => "checking",
            Self::NotFound => "not-found",
            Self::Missing => "missing",
            Self::NoneFound => "none-found",
        }
    }
}

pub type Queue = (Vec<LocalFile>, usize);

pub fn row_file(local: &LocalLibrary, enabled: bool, uri: &str) -> Result<usize, Unplayable> {
    if !enabled {
        return Err(Unplayable::Off);
    }
    match local.index.find(&local.files, uri) {
        Some(position) if local.files[position].missing => Err(Unplayable::Missing),
        Some(position) => Ok(position),
        None if !local.loaded || local.scanning => Err(Unplayable::Checking),
        None => Err(Unplayable::NotFound),
    }
}

pub fn row_queue(
    local: &LocalLibrary,
    enabled: bool,
    tracks: &[Track],
    index: usize,
) -> Result<Queue, Unplayable> {
    let clicked = tracks.get(index).ok_or(Unplayable::NotFound)?;
    row_file(local, enabled, &clicked.uri)?;
    let mut files = Vec::new();
    let mut start = 0;
    for (row, track) in tracks.iter().enumerate() {
        if !is_local(&track.uri) {
            continue;
        }
        let Ok(position) = row_file(local, enabled, &track.uri) else {
            continue;
        };
        if row == index {
            start = files.len();
        }
        files.push(LocalFile {
            uri: track.uri.clone(),
            ..local.files[position].clone()
        });
    }
    Ok((files, start))
}

pub fn first_queue(
    local: &LocalLibrary,
    enabled: bool,
    tracks: &[Track],
    from: usize,
) -> Result<Queue, Unplayable> {
    if !enabled {
        return Err(Unplayable::Off);
    }
    let count = tracks.len();
    let first = (from..count).chain(0..from.min(count)).find(|&row| {
        is_local(&tracks[row].uri) && row_file(local, enabled, &tracks[row].uri).is_ok()
    });
    match first {
        Some(row) => row_queue(local, enabled, tracks, row),
        None if !local.loaded || local.scanning => Err(Unplayable::Checking),
        None => Err(Unplayable::NoneFound),
    }
}

pub fn shuffle<T>(mut items: Vec<T>, seed: u64) -> Vec<T> {
    let mut state = seed | 1;
    for last in (1..items.len()).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let pick = (state % (last as u64 + 1)) as usize;
        items.swap(last, pick);
    }
    items
}

fn seed() -> u64 {
    RandomState::new().hash_one(SystemTime::now())
}

pub fn first_streamable(tracks: &[Track], from: usize) -> Option<usize> {
    let count = tracks.len();
    (from..count)
        .chain(0..from.min(count))
        .find(|&row| !is_local(&tracks[row].uri))
}

pub fn row_art(local: &LocalLibrary, enabled: bool, uris: &[String]) -> Vec<(String, PathBuf)> {
    if !enabled {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    uris.iter()
        .filter_map(|uri| {
            let file = local.matching(uri)?;
            let key = file.artwork.clone()?;
            seen.insert(key.clone()).then(|| (key, file.path.clone()))
        })
        .collect()
}

#[derive(Debug, Default)]
pub struct MatchedRows {
    seen: Option<(u64, bool, Vec<String>)>,
    pub art: Vec<(String, PathBuf)>,
}

impl MatchedRows {
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.art.iter().map(|(key, _)| key)
    }
}

fn open_rows(app: &OpenMusic) -> Option<impl Iterator<Item = &String> + Clone> {
    if !matches!(app.page, Page::Collection(_)) {
        return None;
    }
    let (_, Ok(collection)) = app.playlists.collection.as_ref()? else {
        return None;
    };
    Some(
        collection
            .tracks
            .iter()
            .map(|track| &track.uri)
            .filter(|uri| is_local(uri)),
    )
}

fn open_tracks(app: &OpenMusic) -> Vec<Track> {
    match &app.playlists.collection {
        Some((_, Ok(collection))) => collection.tracks.clone(),
        _ => Vec::new(),
    }
}

pub fn tick(app: &mut OpenMusic) {
    if !matches!(app.session, Session::Connected) {
        return;
    }
    let enabled = app.local_files_on();
    let version = app.account.local.version;
    let open = open_rows(app).map(|rows| {
        let unchanged = app.account.local.rows.seen.as_ref().is_some_and(
            |(seen_version, seen_enabled, uris)| {
                *seen_version == version && *seen_enabled == enabled && rows.clone().eq(uris)
            },
        );
        (!unchanged).then(|| rows.cloned().collect::<Vec<String>>())
    });
    let local = &mut app.account.local;
    match open {
        None => {
            if local.rows.seen.is_some() {
                local.rows = MatchedRows::default();
            }
            return;
        }
        Some(Some(uris)) => {
            let art = row_art(local, enabled, &uris);
            local.rows = MatchedRows {
                seen: Some((version, enabled, uris)),
                art,
            };
        }
        Some(None) => {}
    }
    let wanted: Vec<(String, PathBuf)> = local
        .rows
        .art
        .iter()
        .filter(|(key, _)| !app.covers.contains_key(key) && !local.art_requested.contains(key))
        .cloned()
        .collect();
    if wanted.is_empty() {
        return;
    }
    local
        .art_requested
        .extend(wanted.iter().map(|(key, _)| key.clone()));
    app.send(Command::LocalArt(wanted));
}

pub fn row_dimmed(app: &OpenMusic, track: &Track, row: Row) -> bool {
    row.0 != ROW && is_local(&track.uri) && app.local_row(&track.uri).is_err()
}

impl OpenMusic {
    pub fn local_row(&self, uri: &str) -> Result<usize, Unplayable> {
        row_file(&self.account.local, self.local_files_on(), uri)
    }

    pub fn local_match(&self, uri: &str) -> Option<&LocalFile> {
        if !is_local(uri) || !self.local_files_on() {
            return None;
        }
        self.account.local.matching(uri)
    }

    pub fn local_artwork(&self, track: &Track) -> Option<&String> {
        self.local_match(&track.uri)?.artwork.as_ref()
    }

    pub fn plays_local_row(
        &mut self,
        tracks: &[Track],
        index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        if !tracks.get(index).is_some_and(|track| is_local(&track.uri)) {
            return false;
        }
        let queue = row_queue(&self.account.local, self.local_files_on(), tracks, index);
        self.start_local_queue(queue);
        cx.notify();
        true
    }

    pub fn streamable_start(
        &mut self,
        tracks: &[Track],
        index: usize,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        if !tracks.get(index).is_some_and(|track| is_local(&track.uri)) {
            return Some(index);
        }
        if let Some(start) = first_streamable(tracks, index) {
            return Some(start);
        }
        let queue = first_queue(&self.account.local, self.local_files_on(), tracks, index);
        self.start_local_queue(queue);
        cx.notify();
        None
    }

    pub fn shuffles_local_rows(&mut self, tracks: &[Track], cx: &mut Context<Self>) -> bool {
        if tracks.is_empty() || first_streamable(tracks, 0).is_some() {
            return false;
        }
        let queue = first_queue(&self.account.local, self.local_files_on(), tracks, 0)
            .map(|(files, _)| (shuffle(files, seed()), 0));
        self.start_local_queue(queue);
        cx.notify();
        true
    }

    fn start_local_queue(&mut self, queue: Result<Queue, Unplayable>) {
        match queue {
            Ok((files, index)) => self.send(Command::PlayLocal { files, index }),
            Err(reason) => self.show_toast(reason.message(), None),
        }
    }
}

fn describe_row(app: &OpenMusic, uri: &str) -> String {
    let home = home();
    let path = |file: &LocalFile| display_path(&file.path, home.as_deref());
    match app.local_row(uri) {
        Ok(position) => format!("playable {}", path(&app.account.local.files[position])),
        Err(Unplayable::Missing) => match app.account.local.matching(uri) {
            Some(file) => format!("missing {}", path(file)),
            None => "missing".to_owned(),
        },
        Err(reason) => reason.name().to_owned(),
    }
}

pub fn describe_rows(app: &OpenMusic) -> String {
    let local = &app.account.local;
    let tracks = open_tracks(app);
    let loaded = match &app.playlists.collection {
        Some((uri, Ok(_))) => uri.as_str(),
        _ => "none",
    };
    let rows: Vec<String> = tracks
        .iter()
        .enumerate()
        .filter(|(_, track)| is_local(&track.uri))
        .map(|(row, track)| format!("{row} | {} | {}", track.uri, describe_row(app, &track.uri)))
        .collect();
    let playable = tracks
        .iter()
        .filter(|track| is_local(&track.uri) && app.local_row(&track.uri).is_ok())
        .count();
    let mut lines = vec![format!(
        "collection={loaded} page={} enabled={} loaded={} scanning={} version={} rows={} local={} playable={} art={} first_streamable={}",
        app.page.describe(),
        app.local_files_on(),
        local.loaded,
        local.scanning,
        local.version,
        tracks.len(),
        rows.len(),
        playable,
        local.rows.art.len(),
        first_streamable(&tracks, 0).map_or("none".to_owned(), |row| row.to_string()),
    )];
    lines.extend(rows);
    lines.join("\n")
}

pub fn debug(
    app: &mut OpenMusic,
    word: &str,
    argument: &str,
    directory: &Path,
    cx: &mut Context<OpenMusic>,
) {
    let text = match word {
        "match" => argument
            .split_whitespace()
            .map(|uri| format!("{uri} | {}", describe_row(app, uri)))
            .collect::<Vec<_>>()
            .join("\n"),
        "playrow" => {
            if let Ok(row) = argument.parse::<usize>() {
                let tracks = open_tracks(app);
                if !app.plays_local_row(&tracks, row, cx) {
                    app.show_toast("That row isn’t a local file.", None);
                }
            }
            describe_rows(app)
        }
        _ => describe_rows(app),
    };
    let _ = std::fs::write(directory.join("localmatch.txt"), text);
}

#[cfg(test)]
mod tests {
    use super::super::local_scan::local_uri;
    use super::*;

    fn file(path: &str, artist: &str, album: &str, title: &str, duration_ms: u32) -> LocalFile {
        LocalFile {
            path: PathBuf::from(path),
            uri: local_uri(artist, album, title, duration_ms),
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            duration_ms,
            ..LocalFile::default()
        }
    }

    fn library(files: Vec<LocalFile>) -> LocalLibrary {
        let mut local = LocalLibrary {
            files,
            loaded: true,
            ..LocalLibrary::default()
        };
        local.reindex();
        local
    }

    fn row(uri: &str) -> Track {
        Track {
            uri: uri.into(),
            ..Track::default()
        }
    }

    #[test]
    fn reads_the_parts_of_a_local_uri() {
        assert_eq!(
            LocalRef::parse(
                "spotify:local:Some+Band:The+Album%3A+Live:Song+%28Edit%29+%2B+More:215"
            ),
            Some(LocalRef {
                artist: "Some Band".into(),
                album: "The Album: Live".into(),
                title: "Song (Edit) + More".into(),
                seconds: Some(215),
            })
        );
        assert_eq!(
            LocalRef::parse("spotify:local:::Only+Title:x"),
            Some(LocalRef {
                artist: String::new(),
                album: String::new(),
                title: "Only Title".into(),
                seconds: None,
            })
        );
        assert_eq!(
            LocalRef::parse("spotify:local:Caf%C3%A9:%E2%80%9CQuoted%E2%80%9D:100%25+Sure:9"),
            Some(LocalRef {
                artist: "Café".into(),
                album: "“Quoted”".into(),
                title: "100% Sure".into(),
                seconds: Some(9),
            })
        );
        assert!(LocalRef::parse("spotify:local:Artist:Album").is_none());
        assert!(LocalRef::parse("spotify:local:a:b:c:d:1").is_none());
        assert!(LocalRef::parse("spotify:track:4uLU6hMCjMI75M1A2tKUQC").is_none());
        assert_eq!(decode("50%+off%2"), "50% off%2");
        assert_eq!(decode("%zz%4"), "%zz%4");
        assert_eq!(decode("%FF"), "\u{fffd}");
    }

    #[test]
    fn reads_back_what_the_scanner_writes() {
        for (artist, album, title) in [
            ("A & B", "Album: Part 1", "Track #1 (Live) + Bonus"),
            ("Ünïcode", "日本語", "Ça va?"),
            ("", "", "untitled"),
        ] {
            let uri = local_uri(artist, album, title, 61_999);
            assert_eq!(
                LocalRef::parse(&uri),
                Some(LocalRef {
                    artist: artist.into(),
                    album: album.into(),
                    title: title.into(),
                    seconds: Some(61),
                })
            );
        }
    }

    #[test]
    fn matches_on_tags_and_a_close_duration() {
        let local = library(vec![
            file("/m/one.mp3", "Some Band", "First Album", "Opening", 200_400),
            file("/m/two.m4a", "Other", "Second", "Closing  Time", 180_000),
        ]);
        let find = |uri: &str| local.index.find(&local.files, uri);
        assert_eq!(find(&local.files[0].uri), Some(0));
        assert_eq!(
            find("spotify:local:some+band:FIRST+ALBUM:opening:200"),
            Some(0)
        );
        assert_eq!(
            find("spotify:local:Some+Band:First+Album:Opening:202"),
            Some(0)
        );
        assert_eq!(
            find("spotify:local:Some+Band:First+Album:Opening:198"),
            Some(0)
        );
        assert_eq!(
            find("spotify:local:Some+Band:First+Album:Opening:203"),
            None
        );
        assert_eq!(
            find("spotify:local:Some+Band:First+Album:Opening:197"),
            None
        );
        assert_eq!(
            find("spotify:local:Some+Band:First+Album:Opening:soon"),
            Some(0)
        );
        assert_eq!(
            find("spotify:local:Other:Second:+Closing+Time+:181"),
            Some(1)
        );
        assert_eq!(
            find("spotify:local:Some+Band:Other+Album:Opening:200"),
            None
        );
        assert_eq!(find("spotify:local:Some+Band::Opening:200"), None);
        assert_eq!(find("spotify:local:broken"), None);
    }

    #[test]
    fn prefers_a_present_file_with_the_closest_length() {
        let mut away = file("/Volumes/Drive/song.mp3", "Band", "Album", "Song", 200_000);
        away.missing = true;
        let local = library(vec![
            away,
            file("/m/far.mp3", "Band", "Album", "Song", 202_000),
            file("/m/near.mp3", "Band", "Album", "Song", 201_000),
        ]);
        let found = local
            .index
            .find(&local.files, "spotify:local:Band:Album:Song:200")
            .unwrap();
        assert_eq!(local.files[found].path, PathBuf::from("/m/near.mp3"));
        let gone = library(vec![local.files[0].clone()]);
        assert_eq!(
            row_file(&gone, true, "spotify:local:Band:Album:Song:200"),
            Err(Unplayable::Missing)
        );
        assert_eq!(
            row_file(&local, false, "spotify:local:Band:Album:Song:200"),
            Err(Unplayable::Off)
        );
        assert_eq!(
            row_file(&local, true, "spotify:local:Band:Album:Other:200"),
            Err(Unplayable::NotFound)
        );
        let scanning = LocalLibrary {
            scanning: true,
            ..library(Vec::new())
        };
        assert_eq!(
            row_file(&scanning, true, "spotify:local:Band:Album:Song:200"),
            Err(Unplayable::Checking)
        );
    }

    #[test]
    fn queues_the_playable_local_rows_of_a_list() {
        let local = library(vec![
            file("/m/a.mp3", "Band", "Album", "A", 100_000),
            file("/m/b.mp3", "Band", "Album", "B", 100_000),
            LocalFile {
                missing: true,
                ..file("/x/c.mp3", "Band", "Album", "C", 100_000)
            },
        ]);
        let tracks = vec![
            row("spotify:local:Band:Album:A:101"),
            row("spotify:track:4uLU6hMCjMI75M1A2tKUQC"),
            row("spotify:local:Band:Album:C:100"),
            row("spotify:local:Band:Album:Gone:100"),
            row("spotify:local:band:album:b:99"),
            row("spotify:local:Band:Album:A:101"),
        ];
        let (files, start) = row_queue(&local, true, &tracks, 4).unwrap();
        let uris: Vec<&str> = files.iter().map(|file| file.uri.as_str()).collect();
        assert_eq!(
            uris,
            [
                "spotify:local:Band:Album:A:101",
                "spotify:local:band:album:b:99",
                "spotify:local:Band:Album:A:101",
            ]
        );
        assert_eq!(files[1].path, PathBuf::from("/m/b.mp3"));
        assert_eq!(files[1].title, "B");
        assert_eq!(start, 1);
        assert_eq!(row_queue(&local, true, &tracks, 5).unwrap().1, 2);
        assert_eq!(
            row_queue(&local, true, &tracks, 2),
            Err(Unplayable::Missing)
        );
        assert_eq!(
            row_queue(&local, true, &tracks, 3),
            Err(Unplayable::NotFound)
        );
        assert_eq!(row_queue(&local, false, &tracks, 0), Err(Unplayable::Off));
        assert_eq!(first_queue(&local, true, &tracks, 2).unwrap().1, 1);
        let none = vec![row("spotify:local:Band:Album:Gone:100")];
        assert_eq!(
            first_queue(&local, true, &none, 0),
            Err(Unplayable::NoneFound)
        );
        assert_eq!(first_queue(&local, false, &none, 0), Err(Unplayable::Off));
    }

    #[test]
    fn starts_spotify_playback_on_a_song_it_can_stream() {
        let tracks = vec![
            row("spotify:local:Band:Album:A:100"),
            row("spotify:track:4uLU6hMCjMI75M1A2tKUQC"),
            row("spotify:local:Band:Album:B:100"),
        ];
        assert_eq!(first_streamable(&tracks, 0), Some(1));
        assert_eq!(first_streamable(&tracks, 2), Some(1));
        assert_eq!(first_streamable(&tracks, 1), Some(1));
        assert_eq!(first_streamable(&tracks[2..], 0), None);
        assert_eq!(first_streamable(&tracks, 9), Some(1));
        assert_eq!(first_streamable(&[], 0), None);
    }

    #[test]
    fn shuffles_every_file_into_the_queue_once() {
        let items: Vec<u32> = (0..12).collect();
        let first = shuffle(items.clone(), 7);
        assert_eq!(first, shuffle(items.clone(), 7));
        assert_ne!(first, items);
        assert_ne!(first, shuffle(items.clone(), 8));
        let mut sorted = first.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, items);
        assert_eq!(shuffle(vec![1], 3), vec![1]);
        assert!(shuffle(Vec::<u32>::new(), 3).is_empty());
    }

    #[test]
    fn lists_each_piece_of_row_artwork_once() {
        let mut files = vec![
            file("/m/a.mp3", "Band", "Album", "A", 100_000),
            file("/m/b.mp3", "Band", "Album", "B", 100_000),
            file("/m/c.mp3", "Band", "Album", "C", 100_000),
        ];
        files[0].artwork = Some("local-art:1".into());
        files[1].artwork = Some("local-art:1".into());
        let local = library(files);
        let uris = vec![
            "spotify:local:Band:Album:A:100".to_owned(),
            "spotify:local:Band:Album:B:100".to_owned(),
            "spotify:local:Band:Album:C:100".to_owned(),
            "spotify:local:Band:Album:D:100".to_owned(),
        ];
        assert_eq!(
            row_art(&local, true, &uris),
            vec![("local-art:1".to_owned(), PathBuf::from("/m/a.mp3"))]
        );
        assert!(row_art(&local, false, &uris).is_empty());
    }
}
