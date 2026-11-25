use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use spotify_client::catalog::Playlist;

use crate::areas::discovery::worker::Discovery;

const COVER_LIMIT_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Default, Serialize, Deserialize)]
pub struct Library {
    pub playlists: Vec<Playlist>,
    pub discovery: Option<Discovery>,
}

fn directory() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let directory = PathBuf::from(home).join("Library/Caches/OpenMusic");
    std::fs::create_dir_all(directory.join("covers")).ok()?;
    Some(directory)
}

fn library_path() -> Option<PathBuf> {
    Some(directory()?.join("library.json"))
}

pub fn load() -> Option<Library> {
    let bytes = std::fs::read(library_path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn store(update: impl FnOnce(&mut Library)) {
    let Some(path) = library_path() else { return };
    let mut library = load().unwrap_or_default();
    update(&mut library);
    if let Ok(bytes) = serde_json::to_vec(&library) {
        let temporary = path.with_extension("tmp");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
}

pub fn save_playlists(playlists: &[Playlist]) {
    if playlists.is_empty() {
        return;
    }
    let playlists = playlists.to_vec();
    store(move |library| library.playlists = playlists);
}

pub fn save_discovery(discovery: &Discovery) {
    if discovery.is_empty() {
        return;
    }
    let Ok(copy) = serde_json::to_value(discovery).and_then(serde_json::from_value) else {
        return;
    };
    store(move |library| library.discovery = Some(copy));
}

pub fn clear() {
    if let Some(path) = library_path() {
        let _ = std::fs::remove_file(path);
    }
}

fn hash(url: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in url.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn cover_path(url: &str) -> Option<PathBuf> {
    Some(
        directory()?
            .join("covers")
            .join(format!("{:016x}", hash(url))),
    )
}

pub fn read_cover(url: &str) -> Option<Vec<u8>> {
    std::fs::read(cover_path(url)?).ok()
}

pub fn write_cover(url: &str, bytes: &[u8]) {
    let Some(path) = cover_path(url) else { return };
    let _ = std::fs::write(&path, bytes);
    if !hash(url).is_multiple_of(64) {
        return;
    }
    let Some(folder) = path.parent() else { return };
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    if total <= COVER_LIMIT_BYTES {
        return;
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in files {
        if total <= COVER_LIMIT_BYTES * 3 / 4 {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            total -= size;
        }
    }
}
