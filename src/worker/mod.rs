pub mod cache;
mod keychain;
pub mod saved;
mod tasks;

use std::{collections::HashMap, sync::Arc, time::Duration};

use spotify_client::{
    Api, Track, catalog,
    dealer::Message,
    engine::{self, Control, NowPlaying, Update},
    oauth::{self, Token},
};
use tokio::{
    sync::mpsc::{UnboundedReceiver, UnboundedSender},
    task::JoinSet,
    time::Instant,
};

use crate::{
    app::settings::Settings,
    areas::{account, audio, content, details, discovery, library, playback, playlists, system},
};
pub use tasks::{Mutation, Tasks, Writes};

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
    ForgetCovers(Vec<String>),
    ApplySettings(Settings),
    Saved(saved::Command),
    Account(account::worker::Command),
    Audio(audio::worker::Command),
    Content(content::worker::Command),
    Details(details::worker::Command),
    Discovery(discovery::worker::Command),
    Library(library::worker::Command),
    Playback(playback::worker::Command),
    Playlists(playlists::worker::Command),
    System(system::worker::Command),
}

pub enum Event {
    Connecting,
    Pairing { code: String, url: String },
    Connected,
    Reconnected,
    SignedOut(Option<String>),
    Cover { url: String, jpeg: Vec<u8> },
    NowPlaying(NowPlaying),
    Notice(String),
    MutationFailed { keys: Vec<String>, message: String },
    Saved(saved::Event),
    Account(account::worker::Event),
    Audio(audio::worker::Event),
    Content(content::worker::Event),
    Details(details::worker::Event),
    Discovery(discovery::worker::Event),
    Library(library::worker::Event),
    Playback(playback::worker::Event),
    Playlists(playlists::worker::Event),
    System(system::worker::Event),
}

macro_rules! wrap {
    ($($variant:ident => $command:ty, $event:ty;)*) => {
        $(
            impl From<$command> for Command {
                fn from(command: $command) -> Self {
                    Self::$variant(command)
                }
            }

            impl From<$event> for Event {
                fn from(event: $event) -> Self {
                    Self::$variant(event)
                }
            }
        )*
    };
}

wrap! {
    Saved => saved::Command, saved::Event;
    Account => account::worker::Command, account::worker::Event;
    Audio => audio::worker::Command, audio::worker::Event;
    Content => content::worker::Command, content::worker::Event;
    Details => details::worker::Command, details::worker::Event;
    Discovery => discovery::worker::Command, discovery::worker::Event;
    Library => library::worker::Command, library::worker::Event;
    Playback => playback::worker::Command, playback::worker::Event;
    Playlists => playlists::worker::Command, playlists::worker::Event;
    System => system::worker::Command, system::worker::Event;
}

impl Event {
    pub fn covers(&self) -> Vec<String> {
        match self {
            Self::Account(event) => event.covers(),
            Self::Audio(event) => event.covers(),
            Self::Content(event) => event.covers(),
            Self::Details(event) => event.covers(),
            Self::Discovery(event) => event.covers(),
            Self::Library(event) => event.covers(),
            Self::Playback(event) => event.covers(),
            Self::Playlists(event) => event.covers(),
            Self::System(event) => event.covers(),
            _ => Vec::new(),
        }
    }
}

pub struct Ctx {
    pub api: Api,
    pub http: reqwest::Client,
    pub handle: engine::Handle,
    pub user: User,
    pub events: UnboundedSender<Event>,
    pub tasks: Tasks,
    pub writes: Writes,
    pub known: Known,
    pub covers: Covers,
    pub saved: saved::Lookups,
}

impl Ctx {
    pub fn publish(&mut self, event: impl Into<Event>) {
        let event = event.into();
        self.covers.request(event.covers(), &self.http);
        let _ = self.events.send(event);
    }
}

#[derive(Default)]
pub struct Known {
    pub tracks: HashMap<String, Track>,
    lookups: JoinSet<Option<Track>>,
}

#[derive(Default)]
pub struct Covers {
    fetches: JoinSet<(String, Option<Vec<u8>>)>,
    requested: HashMap<String, Option<Instant>>,
}

impl Covers {
    pub fn request(&mut self, urls: impl IntoIterator<Item = String>, http: &reqwest::Client) {
        for url in urls {
            if !wants_fetch(self.requested.get(&url).copied(), Instant::now()) {
                continue;
            }
            self.requested.insert(url.clone(), None);
            let http = http.clone();
            self.fetches.spawn(async move {
                if let Some(bytes) = cache::read_cover(&url) {
                    return (url, Some(bytes));
                }
                let jpeg = async {
                    if !trusted_image(&url) {
                        return None;
                    }
                    let bytes = http
                        .get(&url)
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

    fn forget(&mut self, urls: Vec<String>) {
        for url in urls {
            self.requested.remove(&url);
        }
    }

    fn failed(&mut self, url: String) {
        let now = Instant::now();
        self.requested
            .retain(|_, failed| failed.is_none_or(|at| now.duration_since(at) < COVER_RETRY));
        self.requested.insert(url, Some(now));
    }
}

fn wants_fetch(requested: Option<Option<Instant>>, now: Instant) -> bool {
    match requested {
        None => true,
        Some(None) => false,
        Some(Some(failed)) => now.duration_since(failed) >= COVER_RETRY,
    }
}

pub async fn run(mut commands: UnboundedReceiver<Command>, events: UnboundedSender<Event>) {
    let Ok(http) = spotify_client::http_client() else {
        let _ = events.send(Event::SignedOut(Some("Could not start networking.".into())));
        return;
    };
    while let Some(command) = commands.recv().await {
        let saved = keychain::read();
        let mut restored = Vec::new();
        let token = match (command, saved) {
            (Command::Restore, Some(refresh)) | (Command::SignIn, Some(refresh)) => {
                match cache::load() {
                    Some(library) => restored = restore(library, &events),
                    None => {
                        let _ = events.send(Event::Connecting);
                    }
                }
                match oauth::refresh(&http, oauth::DESKTOP_CLIENT_ID, &refresh).await {
                    Ok(token) => Some(token),
                    Err(error) if error.downcast_ref::<oauth::Rejected>().is_some() => {
                        restored.clear();
                        let _ = keychain::clear();
                        let _ = events.send(Event::SignedOut(None));
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
        let reason = connected(&http, token, restored, &mut commands, &events).await;
        crate::debug::forget_token();
        let _ = events.send(Event::SignedOut(reason));
    }
}

fn restore(library: cache::Library, events: &UnboundedSender<Event>) -> Vec<String> {
    let cached = [
        Some(Event::Connected),
        Some(library::worker::Event::Playlists(library.playlists).into()),
        library
            .discovery
            .map(|discovery| discovery::worker::Event::Discovered(discovery).into()),
    ];
    let mut covers = Vec::new();
    for event in cached.into_iter().flatten() {
        covers.extend(event.covers());
        let _ = events.send(event);
    }
    covers
}

async fn pair(
    http: &reqwest::Client,
    commands: &mut UnboundedReceiver<Command>,
    events: &UnboundedSender<Event>,
) -> Option<Token> {
    cache::clear();
    library::cache::clear();
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
    restored: Vec<String>,
    commands: &mut UnboundedReceiver<Command>,
    events: &UnboundedSender<Event>,
) -> Option<String> {
    let api = Api::connect(http.clone(), token.access_token.clone()).await;
    let device_name = std::env::var("OPENMUSIC_DEVICE_NAME").unwrap_or_else(|_| "OpenMusic".into());
    let (handle, mut updates) = engine::start(http.clone(), api, token, device_name);
    let mut ctx = Ctx {
        api: handle.api.clone(),
        http: http.clone(),
        handle,
        user: User::default(),
        events: events.clone(),
        tasks: Tasks::default(),
        writes: Writes::start(),
        known: Known::default(),
        covers: Covers::default(),
        saved: saved::Lookups::default(),
    };
    ctx.covers.request(restored, &ctx.http);
    let _ = events.send(Event::Connected);

    let mut dealer_seen = false;
    let mut shown: Option<NowPlaying> = None;
    let mut debug_tasks = JoinSet::new();
    if crate::debug::secrets().is_some() {
        let api = ctx.api.clone();
        debug_tasks.spawn(async move {
            loop {
                crate::debug::write_token(&api.token());
                tokio::time::sleep(Duration::from_secs(300)).await;
            }
        });
    }

    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    ctx.handle.send(Control::SignOut);
                    return None;
                };
                match command {
                    Command::SignOut => {
                        cache::clear();
                        ctx.handle.send(Control::SignOut);
                        return keychain::clear().err().map(|_| {
                            "Signed out. macOS could not remove the saved sign-in from Keychain.".into()
                        });
                    }
                    Command::ForgetCovers(urls) => ctx.covers.forget(urls),
                    Command::ApplySettings(settings) => apply_settings(&settings, &mut ctx),
                    Command::Saved(command) => saved::handle(command, &mut ctx),
                    Command::Account(command) => account::worker::handle(command, &mut ctx),
                    Command::Audio(command) => audio::worker::handle(command, &mut ctx),
                    Command::Content(command) => content::worker::handle(command, &mut ctx),
                    Command::Details(command) => details::worker::handle(command, &mut ctx),
                    Command::Discovery(command) => discovery::worker::handle(command, &mut ctx),
                    Command::Library(command) => library::worker::handle(command, &mut ctx),
                    Command::Playback(command) => playback::worker::handle(command, &mut ctx),
                    Command::Playlists(command) => playlists::worker::handle(command, &mut ctx),
                    Command::System(command) => system::worker::handle(command, &mut ctx),
                    Command::Restore | Command::SignIn | Command::Cancel => {}
                }
            }
            update = updates.recv() => {
                let Some(update) = update else {
                    return Some("Spotify disconnected. Sign in again to reconnect.".into());
                };
                match update {
                    Update::Connected => {
                        if std::mem::replace(&mut dealer_seen, true) {
                            let _ = events.send(Event::Reconnected);
                        }
                    }
                    Update::Ready => {}
                    Update::Now(mut now) => {
                        now_playing(&mut now, &mut ctx);
                        shown = Some((*now).clone());
                        let _ = events.send(Event::NowPlaying(*now));
                    }
                    Update::Playback(model) => playback::worker::on_playback(model, &mut ctx),
                    Update::Push(message) => on_push(&message, &mut ctx),
                    Update::Audio(update) => audio::worker::on_update(update, &mut ctx),
                    Update::Notice(message) => {
                        let _ = events.send(Event::Notice(message));
                    }
                    Update::RefreshToken(refresh) => keychain::write(&refresh),
                    Update::Ended(reason) => return reason,
                }
            }
            Some(Ok(found)) = ctx.known.lookups.join_next(), if !ctx.known.lookups.is_empty() => {
                if let Some(track) = found {
                    ctx.covers.request(track.cover.clone(), &ctx.http);
                    let uri = track.uri.clone();
                    ctx.known.tracks.insert(uri.clone(), track);
                    if let Some(mut now) = shown
                        .clone()
                        .filter(|now| now.track.as_ref().is_some_and(|track| track.uri == uri))
                    {
                        now_playing(&mut now, &mut ctx);
                        let _ = events.send(Event::NowPlaying(now));
                    }
                }
            }
            Some(event) = ctx.tasks.next(), if !ctx.tasks.is_empty() => ctx.publish(event),
            Some(event) = ctx.writes.next() => ctx.publish(event),
            () = ctx.saved.wait(), if ctx.saved.pending() => saved::flush(&mut ctx),
            Some(Ok((url, cover))) = ctx.covers.fetches.join_next(), if !ctx.covers.fetches.is_empty() => {
                match cover {
                    Some(jpeg) => {
                        let _ = events.send(Event::Cover { url, jpeg });
                    }
                    None => ctx.covers.failed(url),
                }
            }
        }
    }
}

fn now_playing(now: &mut NowPlaying, ctx: &mut Ctx) {
    let Some(track) = now.track.as_mut() else {
        return;
    };
    if let Some(full) = ctx.known.tracks.get(&track.uri) {
        let duration = track.duration_ms;
        let uid = track.uid.take();
        *track = full.clone();
        track.uid = uid;
        if duration > 0 {
            track.duration_ms = duration;
        }
    } else if (track.cover.is_none() || track.title.is_empty())
        && (track.uri.starts_with("spotify:track:") || track.uri.starts_with("spotify:episode:"))
    {
        let api = ctx.api.clone();
        let uri = track.uri.clone();
        ctx.known.tracks.insert(uri.clone(), track.clone());
        ctx.known.lookups.spawn(async move {
            catalog::tracks(&api, &[uri])
                .await
                .ok()
                .and_then(|mut found| found.pop())
        });
    }
    ctx.covers.request(track.cover.clone(), &ctx.http);
}

fn on_push(message: &Message, ctx: &mut Ctx) {
    saved::on_push(message, ctx);
    account::worker::on_push(message, ctx);
    audio::worker::on_push(message, ctx);
    content::worker::on_push(message, ctx);
    details::worker::on_push(message, ctx);
    discovery::worker::on_push(message, ctx);
    library::worker::on_push(message, ctx);
    playback::worker::on_push(message, ctx);
    playlists::worker::on_push(message, ctx);
    system::worker::on_push(message, ctx);
}

fn apply_settings(settings: &Settings, ctx: &mut Ctx) {
    account::worker::apply_settings(&settings.account, ctx);
    audio::worker::apply_settings(&settings.audio, ctx);
    content::worker::apply_settings(&settings.content, ctx);
    library::worker::apply_settings(&settings.library, ctx);
    playback::worker::apply_settings(&settings.playback, ctx);
    system::worker::apply_settings(&settings.system, ctx);
}

pub fn trusted_image(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| {
        parsed.scheme() == "https"
            && parsed
                .host_str()
                .is_some_and(|host| host.ends_with(".scdn.co") || host.ends_with(".spotifycdn.com"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_again_only_for_covers_that_failed_a_while_ago() {
        let now = Instant::now();
        assert!(wants_fetch(None, now));
        assert!(!wants_fetch(Some(None), now));
        assert!(!wants_fetch(Some(Some(now)), now + Duration::from_secs(1)));
        assert!(wants_fetch(Some(Some(now)), now + COVER_RETRY));
    }

    #[test]
    fn keeps_loaded_covers_and_drops_old_failures() {
        let mut covers = Covers::default();
        covers.requested.insert("loaded".into(), None);
        let long_ago = Instant::now() - COVER_RETRY - Duration::from_secs(1);
        covers.requested.insert("gave up".into(), Some(long_ago));
        covers.failed("missing".into());
        assert_eq!(covers.requested.len(), 2);
        assert_eq!(covers.requested.get("loaded"), Some(&None));
        assert!(covers.requested.get("missing").is_some_and(Option::is_some));
    }

    #[test]
    fn collects_the_covers_of_cached_home_and_browse_all() {
        let image = |name: &str| Some(format!("https://i.scdn.co/image/{name}"));
        let card = |uri: &str, name: &str| {
            crate::ui::cards::card(
                uri,
                spotify_client::entity::Kind::Album,
                name,
                "",
                image(name),
            )
        };
        let section = spotify_client::home::Section {
            uri: "spotify:section:shortcuts".into(),
            title: String::new(),
            subtitle: String::new(),
            layout: spotify_client::home::Layout::Shortcuts,
            header: None,
            cards: vec![card("spotify:album:a", "shortcut")],
            total: 1,
            next: None,
        };
        let library = cache::Library {
            playlists: Vec::new(),
            discovery: Some(discovery::worker::Discovery {
                home: Some(spotify_client::home::Home {
                    sections: vec![section],
                    ..Default::default()
                }),
                browse: Some(spotify_client::browse::BrowseAll {
                    title: "Browse all".into(),
                    tiles: vec![card("spotify:page:rock", "tile")],
                }),
            }),
        };
        let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
        let covers = restore(library, &events);
        assert_eq!(covers, [image("shortcut").unwrap(), image("tile").unwrap()]);
        assert!(matches!(received.try_recv(), Ok(Event::Connected)));
        assert!(matches!(received.try_recv(), Ok(Event::Library(_))));
        assert!(matches!(received.try_recv(), Ok(Event::Discovery(_))));
        assert!(received.try_recv().is_err());
    }
}
