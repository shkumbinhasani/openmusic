use std::{
    collections::{HashMap, HashSet},
    fs::{FileType, ReadDir},
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use image::codecs::jpeg::JpegEncoder;
use lofty::{
    config::ParseOptions,
    file::{AudioFile, TaggedFile, TaggedFileExt},
    picture::PictureType,
    probe::Probe,
    tag::{Accessor, ItemKey, Tag},
};
use serde::{Deserialize, Serialize};
use spotify_client::{Track, catalog::LocalTrack};

use crate::worker::cache;

pub const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "m4a", "aac", "alac", "wav", "aif", "aiff", "flac", "caf",
];
pub const ART_PREFIX: &str = "local-art:";
pub const ART_SIDE: u32 = 300;
pub const MAX_DEPTH: usize = 16;
pub const MAX_FILES: usize = 50_000;
pub const BATCH: usize = 500;
pub const LOCAL_PREFIX: &str = "spotify:local:";

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png"];
const ART_NAMES: &[&str] = &["cover", "folder", "front", "album", "albumart", "artwork"];
const PACKAGES: &[&str] = &[
    "app",
    "band",
    "bundle",
    "component",
    "fcpbundle",
    "framework",
    "imovielibrary",
    "logicx",
    "musiclibrary",
    "photoslibrary",
    "plugin",
    "tvlibrary",
    "vst",
    "vst3",
    "xcarchive",
];
const SKIPPED_FOLDERS: &[&str] = &["audio music apps"];
const INDEX_VERSION: u32 = 1;
const JPEG_QUALITY: u8 = 85;

static LATEST: AtomicU64 = AtomicU64::new(0);

pub fn supersede(generation: u64) {
    LATEST.store(generation, Ordering::SeqCst);
}

pub fn live(generation: u64) -> bool {
    LATEST.load(Ordering::SeqCst) == generation
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalSources {
    pub enabled: bool,
    pub downloads: bool,
    pub music: bool,
    pub folders: Vec<PathBuf>,
}

impl Default for LocalSources {
    fn default() -> Self {
        Self {
            enabled: false,
            downloads: true,
            music: true,
            folders: Vec::new(),
        }
    }
}

impl LocalSources {
    pub fn roots(&self, home: Option<&Path>) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(home) = home {
            if self.downloads {
                roots.push(home.join("Downloads"));
            }
            if self.music {
                roots.push(home.join("Music"));
            }
        }
        roots.extend(self.folders.iter().cloned());
        outermost(roots)
    }
}

fn outermost(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::new();
    for root in roots {
        if kept.iter().any(|other| root.starts_with(other)) {
            continue;
        }
        kept.retain(|other| !other.starts_with(&root));
        kept.push(root);
    }
    kept
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalFile {
    pub path: PathBuf,
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration_ms: u32,
    pub artwork: Option<String>,
    pub modified_ms: i64,
    pub size: u64,
    pub added_ms: i64,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
}

impl LocalFile {
    pub fn track(&self) -> Track {
        Track {
            uri: self.uri.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_ms: self.duration_ms,
            cover: self.artwork.clone(),
            added_at: (self.added_ms > 0).then_some(self.added_ms),
            ..Track::default()
        }
    }

    pub fn local_track(&self) -> LocalTrack {
        LocalTrack {
            path: self.path.clone(),
            uri: self.uri.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            album: self.album.clone(),
            duration_ms: self.duration_ms,
        }
    }
}

pub fn is_local(uri: &str) -> bool {
    uri.starts_with(LOCAL_PREFIX)
}

fn encode_part(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

pub fn local_uri(artist: &str, album: &str, title: &str, duration_ms: u32) -> String {
    format!(
        "{LOCAL_PREFIX}{}:{}:{}:{}",
        encode_part(artist),
        encode_part(album),
        encode_part(title),
        duration_ms / 1000
    )
}

pub fn audio_file(name: &str) -> bool {
    extension(name).is_some_and(|extension| AUDIO_EXTENSIONS.contains(&extension.as_str()))
}

pub fn package(name: &str) -> bool {
    extension(name).is_some_and(|extension| PACKAGES.contains(&extension.as_str()))
}

pub fn skipped_folder(name: &str) -> bool {
    package(name) || SKIPPED_FOLDERS.contains(&name.to_lowercase().as_str())
}

fn extension(name: &str) -> Option<String> {
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty()).then(|| extension.to_ascii_lowercase())
}

#[derive(Serialize)]
struct IndexOut<'a> {
    version: u32,
    files: &'a [LocalFile],
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct IndexIn {
    files: Vec<LocalFile>,
}

pub fn index_path() -> Option<PathBuf> {
    Some(home()?.join("Library/Application Support/OpenMusic/local-files.json"))
}

pub fn read_index(path: &Path) -> Vec<LocalFile> {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<IndexIn>(&bytes).ok())
        .map(|index| index.files)
        .unwrap_or_default()
}

pub fn write_index(path: &Path, files: &[LocalFile]) -> bool {
    let Some(folder) = path.parent() else {
        return false;
    };
    if std::fs::create_dir_all(folder).is_err() {
        return false;
    }
    let index = IndexOut {
        version: INDEX_VERSION,
        files,
    };
    let Ok(bytes) = serde_json::to_vec(&index) else {
        return false;
    };
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes).is_ok() && std::fs::rename(temporary, path).is_ok()
}

pub trait ArtStore {
    fn has(&self, key: &str) -> bool;
    fn put(&self, key: &str, jpeg: &[u8]);
}

pub struct CoverCache;

impl ArtStore for CoverCache {
    fn has(&self, key: &str) -> bool {
        cache::read_cover(key).is_some()
    }

    fn put(&self, key: &str, jpeg: &[u8]) {
        cache::write_cover(key, jpeg);
    }
}

pub fn art_key(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{ART_PREFIX}{hash:016x}")
}

pub fn thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let picture = image::load_from_memory(bytes).ok()?;
    let picture = if picture.width() > ART_SIDE || picture.height() > ART_SIDE {
        picture.thumbnail(ART_SIDE, ART_SIDE)
    } else {
        picture
    };
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode_image(&picture.to_rgb8())
        .ok()?;
    Some(jpeg)
}

fn read_tags(path: &Path, cover_art: bool) -> Option<TaggedFile> {
    Probe::open(path)
        .ok()?
        .options(ParseOptions::new().read_cover_art(cover_art))
        .guess_file_type()
        .ok()?
        .read()
        .ok()
}

fn embedded_picture(tagged: &TaggedFile) -> Option<Vec<u8>> {
    let pictures = || tagged.tags().iter().flat_map(Tag::pictures);
    pictures()
        .find(|picture| picture.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures().next())
        .map(|picture| picture.data().to_vec())
        .filter(|data| !data.is_empty())
}

fn folder_art(folder: &Path) -> Option<Vec<u8>> {
    let mut candidates: Vec<(usize, PathBuf)> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?.to_ascii_lowercase();
            let extension = path.extension()?.to_str()?.to_ascii_lowercase();
            let rank = ART_NAMES.iter().position(|name| *name == stem)?;
            IMAGE_EXTENSIONS
                .contains(&extension.as_str())
                .then_some((rank, path))
        })
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .find_map(|(_, path)| std::fs::read(path).ok())
}

#[derive(Default)]
pub struct ArtCache {
    stored: HashSet<String>,
    folders: HashMap<PathBuf, Option<String>>,
}

impl ArtCache {
    fn store(&mut self, bytes: &[u8], store: &dyn ArtStore) -> Option<String> {
        let key = art_key(bytes);
        if !self.stored.contains(&key) {
            if !store.has(&key) {
                store.put(&key, &thumbnail(bytes)?);
            }
            self.stored.insert(key.clone());
        }
        Some(key)
    }

    fn folder(&mut self, folder: &Path, store: &dyn ArtStore) -> Option<String> {
        if let Some(found) = self.folders.get(folder) {
            return found.clone();
        }
        let found = folder_art(folder).and_then(|bytes| self.store(&bytes, store));
        self.folders.insert(folder.to_path_buf(), found.clone());
        found
    }
}

pub fn ensure_art(key: &str, path: &Path, store: &dyn ArtStore) -> bool {
    if store.has(key) {
        return true;
    }
    let picture = read_tags(path, true)
        .as_ref()
        .and_then(embedded_picture)
        .or_else(|| path.parent().and_then(folder_art));
    let Some(jpeg) = picture.as_deref().and_then(thumbnail) else {
        return false;
    };
    store.put(key, &jpeg);
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub modified_ms: i64,
    pub size: u64,
    pub added_ms: i64,
}

fn unix_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |elapsed| {
        i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
    })
}

fn stat(path: &Path) -> Option<Stat> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let modified_ms = metadata.modified().map_or(0, unix_ms);
    Some(Stat {
        modified_ms,
        size: metadata.len(),
        added_ms: metadata.created().map_or(modified_ms, unix_ms),
    })
}

fn clean(text: &str) -> Option<String> {
    let text = text.trim_matches(|glyph: char| glyph == '\0' || glyph.is_whitespace());
    (!text.is_empty()).then(|| text.to_owned())
}

fn first_text(tags: &[&Tag], read: impl Fn(&Tag) -> Option<String>) -> Option<String> {
    tags.iter()
        .find_map(|tag| read(tag).as_deref().and_then(clean))
}

pub fn read_file(path: &Path, stat: Stat, art: &mut ArtCache, store: &dyn ArtStore) -> LocalFile {
    let mut file = LocalFile {
        path: path.to_path_buf(),
        title: path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default(),
        modified_ms: stat.modified_ms,
        size: stat.size,
        added_ms: stat.added_ms,
        ..LocalFile::default()
    };
    if let Some(tagged) = read_tags(path, true) {
        file.duration_ms =
            u32::try_from(tagged.properties().duration().as_millis()).unwrap_or(u32::MAX);
        let tags: Vec<&Tag> = tagged
            .primary_tag()
            .into_iter()
            .chain(tagged.tags().iter())
            .collect();
        if let Some(title) = first_text(&tags, |tag| tag.title().map(Into::into)) {
            file.title = title;
        }
        file.artist = first_text(&tags, |tag| tag.artist().map(Into::into)).unwrap_or_default();
        file.album = first_text(&tags, |tag| tag.album().map(Into::into)).unwrap_or_default();
        file.album_artist = first_text(&tags, |tag| {
            tag.get_string(ItemKey::AlbumArtist).map(str::to_owned)
        })
        .unwrap_or_default();
        file.track_number = tags.iter().find_map(|tag| tag.track());
        file.disc_number = tags.iter().find_map(|tag| tag.disk());
        file.artwork = embedded_picture(&tagged).and_then(|bytes| art.store(&bytes, store));
    }
    if file.artwork.is_none() {
        file.artwork = path.parent().and_then(|folder| art.folder(folder, store));
    }
    file.uri = local_uri(&file.artist, &file.album, &file.title, file.duration_ms);
    file
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanReport {
    pub denied: Vec<PathBuf>,
    pub missing: Vec<PathBuf>,
    pub truncated: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanUpdate {
    pub generation: u64,
    pub files: Option<Vec<LocalFile>>,
    pub seen: usize,
    pub read: usize,
    pub finished: Option<ScanReport>,
}

struct Walk<'a> {
    generation: u64,
    roots: Vec<PathBuf>,
    previous: HashMap<PathBuf, LocalFile>,
    found: Vec<LocalFile>,
    visited: HashSet<PathBuf>,
    seen: usize,
    read: usize,
    unsent: usize,
    truncated: bool,
    stopped: bool,
    art: ArtCache,
    store: &'a dyn ArtStore,
    alive: &'a dyn Fn() -> bool,
    report: &'a mut dyn FnMut(ScanUpdate),
}

enum Entry {
    Audio(PathBuf),
    Folder(PathBuf),
}

fn classify(name: &str, kind: FileType, path: PathBuf) -> Option<Entry> {
    if name.starts_with('.') {
        return None;
    }
    if kind.is_dir() {
        return (!skipped_folder(name)).then_some(Entry::Folder(path));
    }
    ((kind.is_file() || kind.is_symlink()) && audio_file(name)).then_some(Entry::Audio(path))
}

impl Walk<'_> {
    fn directory(&mut self, entries: ReadDir, depth: usize) {
        let mut files = Vec::new();
        let mut folders = Vec::new();
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            match classify(&name.to_string_lossy(), kind, entry.path()) {
                Some(Entry::Audio(path)) => files.push(path),
                Some(Entry::Folder(path)) => folders.push(path),
                None => {}
            }
        }
        files.sort();
        folders.sort();
        for path in files {
            if self.stopped {
                return;
            }
            self.file(path);
        }
        if depth >= MAX_DEPTH {
            return;
        }
        for folder in folders {
            if self.stopped {
                return;
            }
            if let Ok(entries) = std::fs::read_dir(&folder) {
                self.directory(entries, depth + 1);
            }
        }
    }

    fn file(&mut self, path: PathBuf) {
        if !(self.alive)() {
            self.stopped = true;
            return;
        }
        if self.visited.contains(&path) {
            return;
        }
        let Some(stat) = stat(&path) else {
            return;
        };
        self.visited.insert(path.clone());
        self.seen += 1;
        let unchanged = self
            .previous
            .get(&path)
            .filter(|old| old.modified_ms == stat.modified_ms && old.size == stat.size);
        let row = match unchanged {
            Some(old) => LocalFile {
                missing: false,
                ..old.clone()
            },
            None => {
                self.read += 1;
                self.unsent += 1;
                read_file(&path, stat, &mut self.art, self.store)
            }
        };
        self.found.push(row);
        if self.found.len() >= MAX_FILES {
            self.truncated = true;
            self.stopped = true;
        }
        if self.seen.is_multiple_of(BATCH) {
            self.progress();
        }
    }

    fn keep_missing(&mut self, root: &Path) {
        let kept: Vec<LocalFile> = self
            .previous
            .values()
            .filter(|old| old.path.starts_with(root) && !self.visited.contains(&old.path))
            .map(|old| LocalFile {
                missing: true,
                ..old.clone()
            })
            .collect();
        for row in kept {
            self.visited.insert(row.path.clone());
            self.found.push(row);
        }
    }

    fn partial(&self) -> Vec<LocalFile> {
        let mut files = self.found.clone();
        files.extend(
            self.previous
                .values()
                .filter(|old| {
                    !self.visited.contains(&old.path)
                        && self.roots.iter().any(|root| old.path.starts_with(root))
                })
                .cloned(),
        );
        files
    }

    fn progress(&mut self) {
        let files = (self.unsent > 0).then(|| self.partial());
        self.unsent = 0;
        (self.report)(ScanUpdate {
            generation: self.generation,
            files,
            seen: self.seen,
            read: self.read,
            finished: None,
        });
    }
}

pub fn scan(
    sources: &LocalSources,
    home: Option<&Path>,
    previous: Vec<LocalFile>,
    generation: u64,
    store: &dyn ArtStore,
    alive: &dyn Fn() -> bool,
    report: &mut dyn FnMut(ScanUpdate),
) -> Option<ScanUpdate> {
    let roots = sources.roots(home);
    let mut walk = Walk {
        generation,
        roots: roots.clone(),
        previous: previous
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect(),
        found: Vec::new(),
        visited: HashSet::new(),
        seen: 0,
        read: 0,
        unsent: 0,
        truncated: false,
        stopped: false,
        art: ArtCache::default(),
        store,
        alive,
        report,
    };
    let mut outcome = ScanReport::default();
    for root in &roots {
        if walk.stopped {
            break;
        }
        match std::fs::read_dir(root) {
            Ok(entries) => walk.directory(entries, 0),
            Err(error) => {
                if error.kind() == ErrorKind::PermissionDenied {
                    outcome.denied.push(root.clone());
                } else {
                    outcome.missing.push(root.clone());
                }
                walk.keep_missing(root);
            }
        }
    }
    if !alive() {
        return None;
    }
    outcome.truncated = walk.truncated;
    Some(ScanUpdate {
        generation,
        seen: walk.seen,
        read: walk.read,
        files: Some(walk.found),
        finished: Some(outcome),
    })
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, io::Cursor, sync::atomic::AtomicUsize};

    use image::{ImageBuffer, ImageFormat, Rgb};
    use lofty::{
        config::WriteOptions,
        picture::{MimeType, Picture},
        tag::{TagExt, TagType},
    };

    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        saved: RefCell<HashMap<String, Vec<u8>>>,
    }

    impl ArtStore for MemoryStore {
        fn has(&self, key: &str) -> bool {
            self.saved.borrow().contains_key(key)
        }

        fn put(&self, key: &str, jpeg: &[u8]) {
            self.saved
                .borrow_mut()
                .insert(key.to_owned(), jpeg.to_vec());
        }
    }

    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            static COUNT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "openmusic-local-{name}-{}-{}",
                std::process::id(),
                COUNT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn join(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn wav(path: &Path, seconds: u32) {
        let rate: u32 = 8000;
        let samples = rate * seconds;
        let data = samples * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data.to_le_bytes());
        bytes.resize(bytes.len() + data as usize, 0);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
    }

    fn png(side: u32) -> Vec<u8> {
        let picture = ImageBuffer::from_pixel(side, side, Rgb([200u8, 40, 90]));
        let mut bytes = Cursor::new(Vec::new());
        picture.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn tag(path: &Path, title: &str, artist: &str, album: &str, picture: Option<Vec<u8>>) {
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title(title.into());
        tag.set_artist(artist.into());
        tag.set_album(album.into());
        tag.set_track(3);
        tag.set_disk(1);
        tag.insert_text(ItemKey::AlbumArtist, "Various".into());
        if let Some(data) = picture {
            tag.push_picture(
                Picture::unchecked(data)
                    .pic_type(PictureType::CoverFront)
                    .mime_type(MimeType::Png)
                    .build(),
            );
        }
        tag.save_to_path(path, WriteOptions::default()).unwrap();
    }

    fn sources(folder: &Folder) -> LocalSources {
        LocalSources {
            enabled: true,
            downloads: false,
            music: false,
            folders: vec![folder.0.clone()],
        }
    }

    fn run(sources: &LocalSources, previous: Vec<LocalFile>, store: &MemoryStore) -> ScanUpdate {
        let alive = || true;
        let mut updates = Vec::new();
        let mut report = |update: ScanUpdate| updates.push(update);
        scan(sources, None, previous, 7, store, &alive, &mut report).unwrap()
    }

    fn by_title(update: &ScanUpdate) -> Vec<LocalFile> {
        let mut files = update.files.clone().unwrap();
        files.sort_by(|a, b| a.title.cmp(&b.title));
        files
    }

    #[test]
    fn encodes_local_uris_like_spotify() {
        assert_eq!(
            local_uri("The Artist", "Live: Tokyo", "Song (Remix)", 241_900),
            "spotify:local:The+Artist:Live%3A+Tokyo:Song+%28Remix%29:241"
        );
        assert_eq!(
            local_uri("", "", "Untitled", 999),
            "spotify:local:::Untitled:0"
        );
        assert_eq!(
            local_uri("Björk", "Homogenic", "Jóga", 305_000),
            "spotify:local:Bj%C3%B6rk:Homogenic:J%C3%B3ga:305"
        );
    }

    #[test]
    fn picks_audio_files_and_skips_packages() {
        assert!(audio_file("song.MP3"));
        assert!(audio_file("a.b.flac"));
        assert!(audio_file("take.aiff"));
        assert!(!audio_file("cover.jpg"));
        assert!(!audio_file("voice.ogg"));
        assert!(!audio_file(".mp3"));
        assert!(!audio_file("mp3"));
        assert!(package("GarageBand.band"));
        assert!(package("Music Library.musiclibrary"));
        assert!(!package("Albums"));
        assert!(!package("Live.2019"));
        assert!(skipped_folder("Audio Music Apps"));
        assert!(skipped_folder("Song.logicx"));
        assert!(!skipped_folder("Music"));
    }

    #[test]
    fn keeps_only_the_outermost_roots() {
        let home = Path::new("/Users/someone");
        let sources = LocalSources {
            enabled: true,
            downloads: true,
            music: true,
            folders: vec![
                PathBuf::from("/Users/someone/Music/Albums"),
                PathBuf::from("/Volumes/Drive/Music"),
                PathBuf::from("/Volumes/Drive"),
            ],
        };
        assert_eq!(
            sources.roots(Some(home)),
            vec![
                PathBuf::from("/Users/someone/Downloads"),
                PathBuf::from("/Users/someone/Music"),
                PathBuf::from("/Volumes/Drive"),
            ]
        );
        let none = LocalSources {
            downloads: false,
            music: false,
            ..LocalSources::default()
        };
        assert!(none.roots(Some(home)).is_empty());
        assert!(LocalSources::default().roots(None).is_empty());
    }

    #[test]
    fn maps_tags_of_generated_files() {
        let folder = Folder::new("tags");
        let tagged = folder.join("Album/01 first.wav");
        wav(&tagged, 2);
        tag(
            &tagged,
            "First Song",
            "Some Artist",
            "Some Album",
            Some(png(640)),
        );
        let plain = folder.join("Album/plain take.wav");
        wav(&plain, 1);
        wav(&folder.join(".hidden.wav"), 1);
        wav(&folder.join("Project.band/inside.wav"), 1);
        std::fs::write(folder.join("notes.txt"), "not audio").unwrap();

        let store = MemoryStore::default();
        let update = run(&sources(&folder), Vec::new(), &store);
        let files = by_title(&update);
        assert_eq!(files.len(), 2, "{files:?}");
        assert_eq!(update.read, 2);
        assert_eq!(update.finished, Some(ScanReport::default()));

        let first = &files[0];
        assert_eq!(first.title, "First Song");
        assert_eq!(first.artist, "Some Artist");
        assert_eq!(first.album, "Some Album");
        assert_eq!(first.album_artist, "Various");
        assert_eq!(first.track_number, Some(3));
        assert_eq!(first.disc_number, Some(1));
        assert!(
            (1900..=2100).contains(&first.duration_ms),
            "{}",
            first.duration_ms
        );
        assert_eq!(
            first.uri,
            local_uri("Some Artist", "Some Album", "First Song", first.duration_ms)
        );
        let key = first.artwork.clone().expect("embedded art");
        assert_eq!(key, art_key(&png(640)));
        let jpeg = store.saved.borrow().get(&key).cloned().unwrap();
        let small = image::load_from_memory(&jpeg).unwrap();
        assert_eq!((small.width(), small.height()), (ART_SIDE, ART_SIDE));

        let second = &files[1];
        assert_eq!(second.title, "plain take");
        assert_eq!(second.artist, "");
        assert_eq!(second.artwork, None);
        assert!(second.uri.starts_with("spotify:local:::plain+take:"));
        assert!(second.added_ms > 0 && second.modified_ms > 0);

        let track = first.track();
        assert_eq!(track.cover, Some(key));
        assert_eq!(track.uri, first.uri);
        let local = first.local_track();
        assert_eq!(local.path, tagged);
        assert_eq!(local.duration_ms, first.duration_ms);
    }

    #[test]
    fn uses_folder_art_when_a_file_has_none() {
        let folder = Folder::new("folder-art");
        wav(&folder.join("Record/side a.wav"), 1);
        std::fs::write(folder.join("Record/Cover.PNG"), png(64)).unwrap();
        let store = MemoryStore::default();
        let update = run(&sources(&folder), Vec::new(), &store);
        let files = update.files.unwrap();
        assert_eq!(files[0].artwork, Some(art_key(&png(64))));

        let other = MemoryStore::default();
        let key = files[0].artwork.clone().unwrap();
        assert!(ensure_art(&key, &files[0].path, &other));
        assert!(other.has(&key));
        assert!(!ensure_art(
            "local-art:none",
            &folder.join("gone.wav"),
            &MemoryStore::default()
        ));
    }

    #[test]
    fn reuses_unchanged_rows_and_drops_deleted_files() {
        let folder = Folder::new("reuse");
        wav(&folder.join("one.wav"), 1);
        wav(&folder.join("two.wav"), 1);
        let store = MemoryStore::default();
        let first = run(&sources(&folder), Vec::new(), &store);
        assert_eq!(first.read, 2);

        let mut previous = first.files.clone().unwrap();
        previous[0].title = "kept from the index".into();
        std::fs::remove_file(folder.join("two.wav")).unwrap();
        let second = run(&sources(&folder), previous, &store);
        assert_eq!(second.read, 0);
        assert_eq!(second.seen, 1);
        let files = second.files.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].title, "kept from the index");

        wav(&folder.join("one.wav"), 3);
        let third = run(&sources(&folder), files, &store);
        assert_eq!(third.read, 1);
        assert_eq!(third.files.unwrap()[0].title, "one");
    }

    #[test]
    fn keeps_rows_of_unplugged_folders_as_missing() {
        let folder = Folder::new("unplugged");
        wav(&folder.join("song.wav"), 1);
        let store = MemoryStore::default();
        let sources = sources(&folder);
        let before = run(&sources, Vec::new(), &store).files.unwrap();
        let gone = folder.0.clone();
        drop(folder);

        let after = run(&sources, before.clone(), &store);
        let report = after.finished.clone().unwrap();
        assert_eq!(report.missing, vec![gone]);
        let files = after.files.unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].missing);
        assert_eq!(files[0].uri, before[0].uri);

        let elsewhere = LocalSources {
            folders: vec![PathBuf::from("/nowhere/else")],
            ..sources
        };
        assert!(run(&elsewhere, before, &store).files.unwrap().is_empty());
    }

    #[test]
    fn stops_when_a_newer_scan_starts() {
        let folder = Folder::new("cancel");
        wav(&folder.join("a.wav"), 1);
        wav(&folder.join("b.wav"), 1);
        let store = MemoryStore::default();
        let alive = || false;
        let mut report = |_: ScanUpdate| {};
        assert_eq!(
            scan(
                &sources(&folder),
                None,
                Vec::new(),
                1,
                &store,
                &alive,
                &mut report
            ),
            None
        );
        supersede(41);
        assert!(live(41));
        assert!(!live(40));
    }

    #[test]
    fn sends_partial_lists_with_rows_not_reached_yet() {
        let folder = Folder::new("partial");
        for index in 0..BATCH {
            let path = folder.join(&format!("{index:04}.wav"));
            std::fs::write(&path, b"").unwrap();
        }
        let earlier = LocalFile {
            path: folder.join("zz/later.wav"),
            uri: "spotify:local:::later:1".into(),
            ..LocalFile::default()
        };
        let store = MemoryStore::default();
        let alive = || true;
        let mut updates = Vec::new();
        let mut report = |update: ScanUpdate| updates.push(update);
        let done = scan(
            &sources(&folder),
            None,
            vec![earlier.clone()],
            3,
            &store,
            &alive,
            &mut report,
        )
        .unwrap();
        assert_eq!(updates.len(), 1);
        let partial = updates[0].files.as_ref().unwrap();
        assert_eq!(partial.len(), BATCH + 1);
        assert!(partial.contains(&earlier));
        assert_eq!(updates[0].generation, 3);
        assert!(updates[0].finished.is_none());
        assert_eq!(done.files.unwrap().len(), BATCH);
    }

    #[test]
    fn writes_and_reads_the_index() {
        let folder = Folder::new("index");
        let path = folder.join("deep/local-files.json");
        let files = vec![LocalFile {
            path: PathBuf::from("/tmp/a.mp3"),
            uri: local_uri("A", "B", "C", 61_000),
            title: "C".into(),
            missing: true,
            ..LocalFile::default()
        }];
        assert!(write_index(&path, &files));
        assert_eq!(read_index(&path), files);
        std::fs::write(&path, b"{\"files\":[{\"title\":\"old\"}]}").unwrap();
        assert_eq!(read_index(&path)[0].title, "old");
        std::fs::write(&path, b"nonsense").unwrap();
        assert!(read_index(&path).is_empty());
        assert!(read_index(&folder.join("absent.json")).is_empty());
    }
}
