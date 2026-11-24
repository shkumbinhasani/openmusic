use std::{collections::HashMap, time::Duration};

use spotify_client::{
    Api, catalog,
    engine::{self, Control, Update},
    oauth::{self, Token},
};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task::JoinSet,
    time::Instant,
};

pub use spotify_client::{
    Track,
    catalog::{Artist, Collection, Lyrics, Playlist},
    engine::NowPlaying,
};

const COVER_RETRY: Duration = Duration::from_secs(60);

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct Discovery {
    pub tracks: Vec<Track>,
    pub from_top_tracks: bool,
    pub mix: Option<Mix>,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Mix {
    pub seed: String,
    pub tracks: Vec<Track>,
}

pub enum Command {
    Restore,
    SignIn,
    Cancel,
    SignOut,
    Search(String),
    Discover,
    Recommend(Track),
    Lyrics(String),
    Playlists,
    Artist(String),
    Collection(String),
    ForgetCovers(Vec<String>),
    PlayContext {
        uri: String,
        index: usize,
        track_uri: Option<String>,
    },
    PlayTracks {
        tracks: Vec<String>,
        index: usize,
    },
    SetPlaying(bool),
    Next,
    Previous,
    Seek(u32),
    Volume(u16),
    PlayHere,
}

pub enum Event {
    Connecting,
    Pairing {
        code: String,
        url: String,
    },
    Connected,
    Discovery(Discovery),
    Recommendations(Option<Mix>),
    Cover {
        url: String,
        jpeg: Vec<u8>,
    },
    SignedOut(Option<String>),
    SearchDone(Result<Vec<Track>, String>),
    DirectTrack(Result<Track, String>),
    NowPlaying(NowPlaying),
    Notice(String),
    Lyrics {
        uri: String,
        lyrics: Option<Lyrics>,
    },
    Playlists(Vec<Playlist>),
    Artist {
        uri: String,
        result: Result<Artist, String>,
    },
    Collection {
        uri: String,
        result: Result<Collection, String>,
    },
}

pub async fn run(mut commands: UnboundedReceiver<Command>, events: UnboundedSender<Event>) {
    let Ok(http) = spotify_client::http_client() else {
        let _ = events.send(Event::SignedOut(Some("Could not start networking.".into())));
        return;
    };
    while let Some(command) = commands.recv().await {
        let saved = keychain::read();
        let token = match (command, saved) {
            (Command::Restore, Some(refresh)) | (Command::SignIn, Some(refresh)) => {
                if let Some(library) = cache::load() {
                    let _ = events.send(Event::Connected);
                    let _ = events.send(Event::Playlists(library.playlists));
                    if let Some(discovery) = library.discovery {
                        let _ = events.send(Event::Discovery(discovery));
                    }
                } else {
                    let _ = events.send(Event::Connecting);
                }
                match oauth::refresh(&http, oauth::DESKTOP_CLIENT_ID, &refresh).await {
                    Ok(token) => Some(token),
                    Err(error) if error.downcast_ref::<oauth::Rejected>().is_some() => {
                        let _ = keychain::clear();
                        pair(&http, &mut commands, &events).await
                    }
                    Err(_) => {
                        let _ = events.send(Event::SignedOut(Some(
                            "Could not reach Spotify. Check your connection and try again.".into(),
                        )));
                        None
                    }
                }
            }
            (Command::Restore, None) => {
                let _ = events.send(Event::SignedOut(None));
                None
            }
            (Command::SignIn, None) => pair(&http, &mut commands, &events).await,
            _ => None,
        };
        let Some(token) = token else { continue };
        keychain::write(&token.refresh_token);
        let reason = connected(&http, token, &mut commands, &events).await;
        let _ = events.send(Event::SignedOut(reason));
    }
}

async fn pair(
    http: &reqwest::Client,
    commands: &mut UnboundedReceiver<Command>,
    events: &UnboundedSender<Event>,
) -> Option<Token> {
    let _ = events.send(Event::Connecting);
    let pairing = match oauth::start_pairing(http, oauth::DESKTOP_CLIENT_ID).await {
        Ok(pairing) => pairing,
        Err(error) => {
            let _ = events.send(Event::SignedOut(Some(error.to_string())));
            return None;
        }
    };
    let link = pairing.link().to_owned();
    let trusted = url::Url::parse(&link).is_ok_and(|url| {
        url.scheme() == "https"
            && matches!(
                url.host_str(),
                Some("spotify.com" | "www.spotify.com" | "accounts.spotify.com")
            )
    });
    if !trusted {
        let _ = events.send(Event::SignedOut(Some(
            "Spotify returned an unexpected pairing link.".into(),
        )));
        return None;
    }
    let _ = events.send(Event::Pairing {
        code: pairing.user_code.clone(),
        url: link,
    });
    let waiting = oauth::finish_pairing(http, oauth::DESKTOP_CLIENT_ID, &pairing);
    tokio::pin!(waiting);
    loop {
        tokio::select! {
            result = &mut waiting => {
                return match result {
                    Ok(token) => Some(token),
                    Err(error) => {
                        let _ = events.send(Event::SignedOut(Some(error.to_string())));
                        None
                    }
                };
            }
            command = commands.recv() => match command {
                None => return None,
                Some(Command::Cancel) => {
                    let _ = events.send(Event::SignedOut(None));
                    return None;
                }
                _ => {}
            },
        }
    }
}

async fn connected(
    http: &reqwest::Client,
    token: Token,
    commands: &mut UnboundedReceiver<Command>,
    events: &UnboundedSender<Event>,
) -> Option<String> {
    let api = Api::connect(http.clone(), token.access_token.clone()).await;
    let device_name = std::env::var("OPENMUSIC_DEVICE_NAME").unwrap_or_else(|_| "OpenMusic".into());
    let (handle, mut updates) = engine::start(http.clone(), api, token, device_name);
    let _ = events.send(Event::Connected);

    let mut debug_tasks = JoinSet::new();
    if crate::debug::directory().is_some() {
        let api = handle.api.clone();
        debug_tasks.spawn(async move {
            loop {
                crate::debug::write_token(&api.token());
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
        });
    }

    let mut search = JoinSet::new();
    let mut discovery = JoinSet::new();
    let mut recommendations = JoinSet::new();
    let mut lyrics = JoinSet::new();
    let mut covers = JoinSet::new();
    let mut requested: HashMap<String, Option<Instant>> = HashMap::new();
    let mut known: HashMap<String, Track> = HashMap::new();
    let mut lookups = JoinSet::new();
    let mut pages = JoinSet::new();

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    handle.send(Control::SignOut);
                    return None;
                };
                match command {
                    Command::SignOut => {
                        cache::clear();
                        handle.send(Control::SignOut);
                        return keychain::clear().err().map(|_| {
                            "Signed out. macOS could not remove the saved sign-in from Keychain.".into()
                        });
                    }
                    Command::Search(query) => {
                        search.abort_all();
                        let api = handle.api.clone();
                        search.spawn(async move {
                            match catalog::parse_track_input(&query) {
                                Err(error) => Event::SearchDone(Err(error.to_string())),
                                Ok(Some(uri)) => Event::DirectTrack(
                                    catalog::tracks(&api, &[uri])
                                        .await
                                        .ok()
                                        .and_then(|mut tracks| tracks.pop())
                                        .ok_or_else(|| "Could not load this song from Spotify.".to_owned()),
                                ),
                                Ok(None) => Event::SearchDone(
                                    catalog::search(&api, &query)
                                        .await
                                        .map_err(|_| "Spotify search is unavailable. Try again.".to_owned()),
                                ),
                            }
                        });
                    }
                    Command::Discover => {
                        discovery.abort_all();
                        let api = handle.api.clone();
                        discovery.spawn(async move { Event::Discovery(discover(&api).await) });
                    }
                    Command::Recommend(seed) => {
                        recommendations.abort_all();
                        let api = handle.api.clone();
                        recommendations.spawn(async move { Event::Recommendations(mix(&api, &seed).await) });
                    }
                    Command::Lyrics(uri) => {
                        lyrics.abort_all();
                        let api = handle.api.clone();
                        lyrics.spawn(async move {
                            let found = catalog::lyrics(&api, &uri).await.ok().flatten();
                            Event::Lyrics { uri, lyrics: found }
                        });
                    }
                    Command::ForgetCovers(urls) => {
                        for url in urls {
                            requested.remove(&url);
                        }
                    }
                    Command::Playlists => {
                        let api = handle.api.clone();
                        pages.spawn(async move {
                            Event::Playlists(catalog::playlists(&api).await.unwrap_or_default())
                        });
                    }
                    Command::Artist(uri) => {
                        let api = handle.api.clone();
                        pages.spawn(async move {
                            let result = catalog::artist(&api, &uri)
                                .await
                                .map_err(|_| "Couldn’t load this artist. Try again.".to_owned());
                            Event::Artist { uri, result }
                        });
                    }
                    Command::Collection(uri) => {
                        let api = handle.api.clone();
                        pages.spawn(async move {
                            let result = catalog::collection(&api, &uri)
                                .await
                                .map_err(|_| "Couldn’t load this. Try again.".to_owned());
                            Event::Collection { uri, result }
                        });
                    }
                    Command::PlayContext { uri, index, track_uri } => {
                        handle.send(Control::PlayContext { uri, index, track_uri });
                    }
                    Command::PlayTracks { tracks, index } => {
                        handle.send(Control::PlayTracks { uris: tracks, index });
                    }
                    Command::SetPlaying(playing) => handle.send(Control::SetPlaying(playing)),
                    Command::Next => handle.send(Control::Next),
                    Command::Previous => handle.send(Control::Previous),
                    Command::Seek(position) => handle.send(Control::Seek(position)),
                    Command::Volume(volume) => handle.send(Control::Volume(volume)),
                    Command::PlayHere => handle.send(Control::PlayHere),
                    Command::Restore | Command::SignIn | Command::Cancel => {}
                }
            }
            update = updates.recv() => {
                let Some(update) = update else {
                    return Some("Spotify disconnected. Sign in again to reconnect.".into());
                };
                match update {
                    Update::Ready => {}
                    Update::Now(mut now) => {
                        if let Some(track) = now.track.as_mut() {
                            if let Some(full) = known.get(&track.uri) {
                                let duration = track.duration_ms;
                                *track = full.clone();
                                if duration > 0 {
                                    track.duration_ms = duration;
                                }
                            } else if track.cover.is_none() || track.title.is_empty() {
                                let api = handle.api.clone();
                                let uri = track.uri.clone();
                                known.insert(uri.clone(), track.clone());
                                lookups.spawn(async move {
                                    catalog::tracks(&api, &[uri]).await.ok().and_then(|mut found| found.pop())
                                });
                            }
                            request_covers([&*track], http, &mut covers, &mut requested);
                        }
                        let _ = events.send(Event::NowPlaying(now));
                    }
                    Update::Notice(message) => {
                        let _ = events.send(Event::Notice(message));
                    }
                    Update::RefreshToken(refresh) => keychain::write(&refresh),
                    Update::Ended(reason) => return reason,
                }
            }
            Some(Ok(found)) = lookups.join_next(), if !lookups.is_empty() => {
                if let Some(track) = found {
                    request_covers([&track], http, &mut covers, &mut requested);
                    known.insert(track.uri.clone(), track);
                }
            }
            Some(Ok(event)) = search.join_next(), if !search.is_empty() => {
                publish(event, events, http, &mut covers, &mut requested);
            }
            Some(Ok(event)) = discovery.join_next(), if !discovery.is_empty() => {
                if let Event::Discovery(home) = &event {
                    cache::save_discovery(home);
                }
                publish(event, events, http, &mut covers, &mut requested);
            }
            Some(Ok(event)) = recommendations.join_next(), if !recommendations.is_empty() => {
                publish(event, events, http, &mut covers, &mut requested);
            }
            Some(Ok(event)) = pages.join_next(), if !pages.is_empty() => {
                let urls: Vec<String> = match &event {
                    Event::Playlists(playlists) => playlists
                        .iter()
                        .filter_map(|playlist| playlist.cover.clone())
                        .collect(),
                    Event::Artist { result: Ok(artist), .. } => artist
                        .image
                        .iter()
                        .cloned()
                        .chain(artist.top_tracks.iter().filter_map(|track| track.cover.clone()))
                        .chain(artist.releases.iter().filter_map(|release| release.cover.clone()))
                        .chain(artist.related.iter().filter_map(|related| related.image.clone()))
                        .collect(),
                    Event::Collection { result: Ok(collection), .. } => collection
                        .cover
                        .iter()
                        .cloned()
                        .chain(collection.tracks.iter().filter_map(|track| track.cover.clone()))
                        .collect(),
                    _ => Vec::new(),
                };
                request_urls(urls, http, &mut covers, &mut requested);
                if let Event::Playlists(playlists) = &event {
                    cache::save_playlists(playlists);
                }
                let _ = events.send(event);
            }
            Some(Ok(event)) = lyrics.join_next(), if !lyrics.is_empty() => {
                let _ = events.send(event);
            }
            Some(Ok((url, cover))) = covers.join_next(), if !covers.is_empty() => {
                match cover {
                    Some(jpeg) => {
                        let _ = events.send(Event::Cover { url, jpeg });
                    }
                    None => {
                        requested.insert(url, Some(Instant::now()));
                    }
                }
            }
        }
    }
}

async fn discover(api: &Api) -> Discovery {
    let tracks = catalog::liked(api, 8).await.unwrap_or_default();
    let mix = match tracks.first() {
        Some(seed) => mix(api, seed).await,
        None => None,
    };
    Discovery {
        tracks,
        from_top_tracks: false,
        mix,
    }
}

async fn mix(api: &Api, seed: &Track) -> Option<Mix> {
    let tracks = catalog::station(api, &seed.uri).await.ok()?;
    (!tracks.is_empty()).then(|| Mix {
        seed: seed.title.clone(),
        tracks,
    })
}

fn publish(
    event: Event,
    events: &UnboundedSender<Event>,
    http: &reqwest::Client,
    covers: &mut JoinSet<(String, Option<Vec<u8>>)>,
    requested: &mut HashMap<String, Option<Instant>>,
) {
    let tracks: Vec<&Track> = match &event {
        Event::SearchDone(Ok(tracks)) => tracks.iter().collect(),
        Event::DirectTrack(Ok(track)) => vec![track],
        Event::Discovery(home) => home
            .tracks
            .iter()
            .chain(home.mix.iter().flat_map(|mix| &mix.tracks))
            .collect(),
        Event::Recommendations(Some(mix)) => mix.tracks.iter().collect(),
        _ => Vec::new(),
    };
    request_covers(tracks, http, covers, requested);
    let _ = events.send(event);
}

fn request_covers<'a>(
    tracks: impl IntoIterator<Item = &'a Track>,
    http: &reqwest::Client,
    covers: &mut JoinSet<(String, Option<Vec<u8>>)>,
    requested: &mut HashMap<String, Option<Instant>>,
) {
    let urls = tracks
        .into_iter()
        .filter_map(|track| track.cover.clone())
        .collect();
    request_urls(urls, http, covers, requested);
}

fn trusted_image_host(host: &str) -> bool {
    host.ends_with(".scdn.co") || host.ends_with(".spotifycdn.com")
}

fn request_urls(
    urls: Vec<String>,
    http: &reqwest::Client,
    covers: &mut JoinSet<(String, Option<Vec<u8>>)>,
    requested: &mut HashMap<String, Option<Instant>>,
) {
    if requested.len() > 512 {
        requested.clear();
    }
    for url in &urls {
        let waiting = requested
            .get(url)
            .is_some_and(|failed| failed.is_none_or(|at| at.elapsed() < COVER_RETRY));
        if waiting {
            continue;
        }
        requested.insert(url.clone(), None);
        let http = http.clone();
        let url = url.clone();
        covers.spawn(async move {
            if let Some(bytes) = cache::read_cover(&url) {
                return (url, Some(bytes));
            }
            let jpeg = async {
                let parsed = url::Url::parse(&url).ok()?;
                if parsed.scheme() != "https" || !parsed.host_str().is_some_and(trusted_image_host)
                {
                    return None;
                }
                let bytes = http
                    .get(parsed)
                    .send()
                    .await
                    .ok()?
                    .error_for_status()
                    .ok()?
                    .bytes()
                    .await
                    .ok()?;
                Some(bytes.to_vec())
            }
            .await;
            if let Some(bytes) = &jpeg {
                cache::write_cover(&url, bytes);
            }
            (url, jpeg)
        });
    }
}

#[cfg(target_os = "macos")]
mod keychain {
    use security_framework::passwords;

    const SERVICE: &str = "openmusic.spotify";
    const ACCOUNT: &str = "refresh-token";
    const ITEM_NOT_FOUND: i32 = -25300;

    pub fn read() -> Option<String> {
        String::from_utf8(passwords::get_generic_password(SERVICE, ACCOUNT).ok()?).ok()
    }

    pub fn write(token: &str) {
        if token.is_empty() || read().as_deref() == Some(token) {
            return;
        }
        if passwords::set_generic_password(SERVICE, ACCOUNT, token.as_bytes()).is_err() {
            eprintln!("OpenMusic could not save the Spotify session in Keychain.");
        }
    }

    pub fn clear() -> Result<(), security_framework::base::Error> {
        match passwords::delete_generic_password(SERVICE, ACCOUNT) {
            Err(error) if error.code() != ITEM_NOT_FOUND => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    pub fn read() -> Option<String> {
        None
    }
    pub fn write(_: &str) {}
    pub fn clear() -> Result<(), ()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use spotify_client::catalog::parse_track_input;

    #[test]
    fn distinguishes_tracks_from_search_queries() {
        let id = "4uLU6hMCjMI75M1A2tKUQC";
        let uri = format!("spotify:track:{id}");
        for input in [
            uri.clone(),
            format!("https://open.spotify.com/track/{id}?si=123"),
            format!("https://open.spotify.com/intl-de/track/{id}"),
        ] {
            assert_eq!(parse_track_input(&input).unwrap().unwrap(), uri);
        }
        assert!(parse_track_input("Nina Simone").unwrap().is_none());
        assert!(parse_track_input("spotify:track:broken").is_err());
        assert!(parse_track_input("https://open.spotify.com/album/abc").is_err());
    }
}

mod cache {
    use std::path::PathBuf;

    use serde::{Deserialize, Serialize};

    use super::{Discovery, Playlist};

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
        if discovery.tracks.is_empty() && discovery.mix.is_none() {
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
}
