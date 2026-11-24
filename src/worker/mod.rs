mod cache;
mod keychain;

use std::{collections::HashMap, sync::Arc, time::Duration};

use spotify_client::{
    Api, Track, catalog,
    catalog::{Artist, Collection, Lyrics, Playlist},
    connect::{self, PlayOptions},
    engine::{self, Control, NowPlaying, Update},
    oauth::{self, Token},
};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task::JoinSet,
    time::Instant,
};

use crate::areas::{
    content, discovery,
    discovery::worker::{Discovery, Mix},
    library, playlists,
};

const COVER_RETRY: Duration = Duration::from_secs(60);

#[derive(Clone, Default)]
pub struct User(Arc<tokio::sync::OnceCell<String>>);

impl User {
    pub async fn name(&self, api: &Api) -> anyhow::Result<String> {
        self.0
            .get_or_try_init(|| catalog::username(api))
            .await
            .cloned()
    }
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
    let user = User::default();
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
                        search.spawn(discovery::worker::search(handle.api.clone(), query));
                    }
                    Command::Discover => {
                        discovery.abort_all();
                        discovery.spawn(discovery::worker::discover(handle.api.clone()));
                    }
                    Command::Recommend(seed) => {
                        recommendations.abort_all();
                        recommendations.spawn(discovery::worker::recommend(handle.api.clone(), seed));
                    }
                    Command::Lyrics(uri) => {
                        lyrics.abort_all();
                        lyrics.spawn(content::worker::lyrics(handle.api.clone(), uri));
                    }
                    Command::ForgetCovers(urls) => {
                        for url in urls {
                            requested.remove(&url);
                        }
                    }
                    Command::Playlists => {
                        pages.spawn(library::worker::playlists(handle.api.clone(), user.clone()));
                    }
                    Command::Artist(uri) => {
                        pages.spawn(discovery::worker::artist(handle.api.clone(), uri));
                    }
                    Command::Collection(uri) => {
                        pages.spawn(playlists::worker::collection(handle.api.clone(), uri));
                    }
                    Command::PlayContext { uri, index, track_uri } => {
                        handle.send(Control::PlayContext {
                            uri,
                            index,
                            track_uri,
                            origin: connect::ORIGIN,
                            options: PlayOptions::default(),
                        });
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
                                let uid = track.uid.take();
                                *track = full.clone();
                                track.uid = uid;
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
                        let _ = events.send(Event::NowPlaying(*now));
                    }
                    Update::Playback(_) | Update::Push(_) => {}
                    Update::Audio(update) => match update {},
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
