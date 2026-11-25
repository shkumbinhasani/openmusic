use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use spotify_client::library::{LibraryPage, LibraryQuery};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cached {
    pub query: LibraryQuery,
    pub page: LibraryPage,
}

fn path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let directory = PathBuf::from(home).join("Library/Caches/OpenMusic");
    std::fs::create_dir_all(&directory).ok()?;
    Some(directory.join("your-library.json"))
}

pub fn load() -> Option<Cached> {
    let bytes = std::fs::read(path()?).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn keeps(query: &LibraryQuery, page: &LibraryPage) -> bool {
    query.text.is_none() && query.folder.is_none() && !page.fallback && !page.items.is_empty()
}

pub fn save(query: &LibraryQuery, page: &LibraryPage) {
    if !keeps(query, page) {
        return;
    }
    let Some(path) = path() else { return };
    let cached = Cached {
        query: query.clone(),
        page: page.clone(),
    };
    if let Ok(bytes) = serde_json::to_vec(&cached) {
        let temporary = path.with_extension("tmp");
        if std::fs::write(&temporary, bytes).is_ok() {
            let _ = std::fs::rename(temporary, path);
        }
    }
}

pub fn clear() {
    if let Some(path) = path() {
        let _ = std::fs::remove_file(path);
    }
}
