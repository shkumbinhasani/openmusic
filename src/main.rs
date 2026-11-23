mod app_icon;
mod debug;
mod glass;
mod icons;
mod overlay;
mod spotify;
mod ui;

use std::{collections::HashMap, sync::Arc, time::Duration};

use gpui::{
    App, AppContext, Application, Bounds, Context, Entity, Image, ImageFormat, Menu, MenuItem,
    OsAction, ScrollHandle, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds,
    WindowOptions, actions, point, px, size,
};
use gpui_component::{
    Root,
    input::{Copy, Cut, InputEvent, InputState, Paste, SelectAll},
};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use spotify::{
    Artist, Collection, Command, Discovery, Event, Lyrics, Mix, NowPlaying, Playlist, Track,
};

actions!(openmusic, [Quit, FocusSearch, SignOut]);

fn main() {
    let initial_track = std::env::args().nth(1);
    let runtime = tokio::runtime::Runtime::new().expect("start OpenMusic background runtime");
    let (commands, inbound) = unbounded_channel();
    let (outbound, mut events) = unbounded_channel();
    runtime.spawn(spotify::run(inbound, outbound));

    Application::new()
        .with_assets(icons::Icons)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            app_icon::install();
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([
                gpui::KeyBinding::new("cmd-q", Quit, None),
                gpui::KeyBinding::new("cmd-f", FocusSearch, None),
            ]);
            cx.set_menus(vec![
                Menu {
                    name: "OpenMusic".into(),
                    items: vec![
                        MenuItem::action("Sign Out of Spotify", SignOut),
                        MenuItem::separator(),
                        MenuItem::action("Quit OpenMusic", Quit),
                    ],
                },
                Menu {
                    name: "Edit".into(),
                    items: vec![
                        MenuItem::os_action("Cut", Cut, OsAction::Cut),
                        MenuItem::os_action("Copy", Copy, OsAction::Copy),
                        MenuItem::os_action("Paste", Paste, OsAction::Paste),
                        MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
                        MenuItem::separator(),
                        MenuItem::action("Find Music", FocusSearch),
                    ],
                },
            ]);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let bounds = Bounds::centered(None, size(px(1120.), px(760.)), cx);
            cx.open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("OpenMusic".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(point(px(22.), px(22.))),
                    }),
                    window_background: WindowBackgroundAppearance::Transparent,
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(800.), px(600.))),
                    ..Default::default()
                },
                |window, cx| {
                    if !glass::install(window) {
                        window.set_background_appearance(WindowBackgroundAppearance::Opaque);
                    }
                    ui::apply_theme(window.appearance(), cx);
                    let (controls, mut presses) = unbounded_channel();
                    overlay::set_handler(overlay::handler(controls));
                    let player = cx.new(|cx| OpenMusic::new(commands, initial_track, window, cx));
                    let handle = window.window_handle();
                    let target = player.downgrade();
                    cx.on_action(move |_: &FocusSearch, cx| {
                        let target = target.clone();
                        let _ = handle.update(cx, |_, window, cx| {
                            let _ = target.update(cx, |this, cx| this.focus_search(window, cx));
                        });
                    });
                    let target = player.downgrade();
                    cx.on_action(move |_: &SignOut, cx| {
                        let _ = target.update(cx, |this, _| this.send(Command::SignOut));
                    });
                    player.update(cx, |this, cx| {
                        this.send(Command::Restore);
                        cx.spawn_in(window, async move |view, window| {
                            while let Some(action) = presses.recv().await {
                                if view
                                    .update_in(window, |this, _, cx| {
                                        this.handle_control(action, cx);
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                        cx.spawn_in(window, async move |view, window| {
                            while let Some(event) = events.recv().await {
                                if view
                                    .update_in(window, |this, window, cx| {
                                        this.handle_event(event, window, cx);
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        })
                        .detach();
                        cx.spawn_in(window, async move |view, window| {
                            loop {
                                window
                                    .background_executor()
                                    .timer(Duration::from_millis(250))
                                    .await;
                                if view.update_in(window, |this, _, cx| this.tick(cx)).is_err() {
                                    break;
                                }
                            }
                        })
                        .detach();
                    });
                    if let Some(directory) = debug::directory() {
                        let _ = std::fs::create_dir_all(&directory);
                        player.update(cx, |_, cx| {
                            cx.spawn_in(window, async move |view, window| {
                                let command_file = directory.join("command");
                                loop {
                                    window
                                        .background_executor()
                                        .timer(Duration::from_millis(250))
                                        .await;
                                    let Ok(commands) = std::fs::read_to_string(&command_file)
                                    else {
                                        continue;
                                    };
                                    let _ = std::fs::remove_file(&command_file);
                                    for line in commands
                                        .lines()
                                        .map(str::trim)
                                        .filter(|line| !line.is_empty())
                                    {
                                        let line = line.to_owned();
                                        let directory = directory.clone();
                                        if view
                                            .update_in(window, |this, window, cx| {
                                                this.debug_command(&line, &directory, window, cx)
                                            })
                                            .is_err()
                                        {
                                            return;
                                        }
                                    }
                                }
                            })
                            .detach();
                        });
                    }
                    cx.new(|cx| Root::new(player, window, cx))
                },
            )
            .expect("open the OpenMusic window");
            cx.activate(true);
        });

    runtime.shutdown_timeout(Duration::from_secs(1));
}

#[derive(Clone, PartialEq)]
enum Page {
    Home,
    Search,
    Lyrics,
    Artist(String),
    Collection(String),
}

enum Account {
    SignedOut,
    Connecting,
    Pairing { code: String, url: String },
    Connected,
}

struct OpenMusic {
    commands: UnboundedSender<Command>,
    account: Account,
    initial_track: Option<String>,
    input: Entity<InputState>,
    results: Vec<Track>,
    home: Discovery,
    newest_mix: Option<Mix>,
    last_seed: Option<String>,
    discovery_pending: bool,
    page: Page,
    history: Vec<Page>,
    playlists: Vec<Playlist>,
    playlists_loading: bool,
    artist: Option<(String, Result<Artist, String>)>,
    collection: Option<(String, Result<Collection, String>)>,
    search_query: String,
    covers: HashMap<String, Arc<Image>>,
    now: NowPlaying,
    notice: Option<String>,
    seek_hold: Option<(u32, std::time::Instant)>,
    lyrics: Option<(String, Option<Lyrics>)>,
    lyrics_scroll: ScrollHandle,
    lyrics_line: Option<usize>,
    lyrics_loading: bool,
    search_pending: bool,
    search_error: Option<String>,
    account_error: Option<String>,
}

impl OpenMusic {
    fn new(
        commands: UnboundedSender<Command>,
        initial_track: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.observe_window_appearance(window, |_, window, cx| {
            ui::apply_theme(window.appearance(), cx);
            cx.notify();
        })
        .detach();
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Songs, artists, or links"));
        cx.subscribe_in(&input, window, |this, _, event, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.search(cx);
            }
        })
        .detach();

        Self {
            commands,
            account: Account::Connecting,
            initial_track,
            input,
            results: Vec::new(),
            home: Discovery::default(),
            newest_mix: None,
            last_seed: None,
            discovery_pending: false,
            page: Page::Home,
            history: Vec::new(),
            playlists: Vec::new(),
            playlists_loading: false,
            artist: None,
            collection: None,
            search_query: String::new(),
            covers: HashMap::new(),
            now: NowPlaying::default(),
            notice: None,
            seek_hold: None,
            lyrics: None,
            lyrics_scroll: ScrollHandle::new(),
            lyrics_line: None,
            lyrics_loading: false,
            search_pending: false,
            search_error: None,
            account_error: None,
        }
    }

    fn debug_command(
        &mut self,
        line: &str,
        directory: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        match verb {
            "snapshot" => {
                let name = if rest.is_empty() { "snapshot" } else { rest };
                let png = directory.join(format!("{name}.png"));
                let marker = directory.join(format!("{name}.done"));
                let _ = std::fs::remove_file(&marker);
                window.refresh();
                window.on_next_frame(move |window, _| {
                    window.on_next_frame(move |_, _| {
                        debug::capture(png, move |result| {
                            let _ =
                                std::fs::write(marker, result.err().unwrap_or_else(|| "ok".into()));
                        });
                    });
                    window.refresh();
                });
            }
            "home" => self.navigate(Page::Home),
            "lyrics" => self.show_lyrics(true),
            "back" => self.back(),
            "search" => {
                let query = rest.to_owned();
                self.input
                    .update(cx, |input, cx| input.set_value(query, window, cx));
                self.search_pending = false;
                self.search(cx);
            }
            "artist" => self.navigate(Page::Artist(rest.to_owned())),
            "open" => self.navigate(Page::Collection(rest.to_owned())),
            "play" => {
                let (uri, index) = rest.split_once(' ').unwrap_or((rest, "0"));
                self.send(Command::PlayContext {
                    uri: uri.to_owned(),
                    index: index.trim().parse().unwrap_or(0),
                    track_uri: None,
                });
            }
            "toggle" => self.toggle(cx),
            "size" => {
                let mut parts = rest
                    .split_whitespace()
                    .filter_map(|part| part.parse::<f32>().ok());
                if let (Some(width), Some(height)) = (parts.next(), parts.next()) {
                    window.resize(size(px(width), px(height)));
                }
            }
            "state" => {
                let state = format!(
                    "account={} page={} track={:?} playing={} position={} device={:?} playlists={} notice={:?}",
                    matches!(self.account, Account::Connected),
                    match &self.page {
                        Page::Home => "home".to_owned(),
                        Page::Search => "search".to_owned(),
                        Page::Lyrics => "lyrics".to_owned(),
                        Page::Artist(uri) => format!("artist {uri}"),
                        Page::Collection(uri) => format!("collection {uri}"),
                    },
                    self.now
                        .track
                        .as_ref()
                        .map(|track| format!("{} - {}", track.title, track.artist)),
                    self.now.playing,
                    self.now.position(),
                    self.now.device,
                    self.playlists.len(),
                    self.notice,
                );
                let _ = std::fs::write(directory.join("state.txt"), state);
            }
            _ => {}
        }
        cx.notify();
    }

    fn referenced_covers(&self) -> std::collections::HashSet<String> {
        let tracks = self
            .results
            .iter()
            .chain(&self.home.tracks)
            .chain(self.home.mix.iter().flat_map(|mix| &mix.tracks))
            .chain(self.newest_mix.iter().flat_map(|mix| &mix.tracks))
            .chain(self.now.track.iter());
        let mut urls: std::collections::HashSet<String> =
            tracks.filter_map(|track| track.cover.clone()).collect();
        urls.extend(
            self.playlists
                .iter()
                .filter_map(|playlist| playlist.cover.clone()),
        );
        if let Some((_, Ok(artist))) = &self.artist {
            urls.extend(artist.image.clone());
            urls.extend(
                artist
                    .top_tracks
                    .iter()
                    .filter_map(|track| track.cover.clone()),
            );
            urls.extend(
                artist
                    .releases
                    .iter()
                    .filter_map(|release| release.cover.clone()),
            );
            urls.extend(
                artist
                    .related
                    .iter()
                    .filter_map(|related| related.image.clone()),
            );
        }
        if let Some((_, Ok(collection))) = &self.collection {
            urls.extend(collection.cover.clone());
            urls.extend(
                collection
                    .tracks
                    .iter()
                    .filter_map(|track| track.cover.clone()),
            );
        }
        urls
    }

    fn evict_covers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keep = self.referenced_covers();
        let stale: Vec<String> = self
            .covers
            .keys()
            .filter(|url| !keep.contains(*url))
            .cloned()
            .collect();
        for url in &stale {
            if let Some(image) = self.covers.remove(url) {
                if let Some(render) = image.clone().get_render_image(window, cx) {
                    let _ = window.drop_image(render);
                }
                image.remove_asset(cx);
            }
        }
        if !stale.is_empty() {
            self.send(Command::ForgetCovers(stale));
        }
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.account, Account::Connected) {
            self.input.update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    fn search(&mut self, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value();
        let query = value.trim();
        if query.is_empty() || self.search_pending || !matches!(self.account, Account::Connected) {
            return;
        }
        self.navigate(Page::Search);
        self.search_query = query.to_owned();
        self.search_pending = true;
        self.search_error = None;
        self.results.clear();
        self.send(Command::Search(query.to_owned()));
        cx.notify();
    }

    fn track(&self) -> Option<&Track> {
        self.now.track.as_ref()
    }

    fn duration(&self) -> Option<u32> {
        self.track()
            .map(|track| track.duration_ms)
            .filter(|duration| *duration > 0)
    }

    fn is_current(&self, uri: &str) -> bool {
        self.track().is_some_and(|track| track.uri == *uri)
    }

    fn play(&mut self, tracks: &[Track], index: usize, cx: &mut Context<Self>) {
        let Some(track) = tracks.get(index) else {
            return;
        };
        self.now.track = Some(track.clone());
        self.now.playing = true;
        self.now.position_ms = 0;
        self.now.observed = std::time::Instant::now();
        self.notice = None;
        self.send(Command::PlayTracks {
            tracks: tracks.iter().map(|track| track.uri.clone()).collect(),
            index,
        });
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.track().is_none() {
            return;
        }
        self.now.position_ms = self.now.position();
        self.now.observed = std::time::Instant::now();
        self.now.playing = !self.now.playing;
        self.notice = None;
        self.send(Command::SetPlaying(self.now.playing));
        cx.notify();
    }

    fn tick(&mut self, cx: &mut Context<Self>) {
        if !self.now.playing {
            return;
        }
        overlay::progress(self.now.position(), self.duration().unwrap_or(0));
        if self.follow_lyrics() && self.page == Page::Lyrics {
            cx.notify();
        }
    }

    fn current_lyrics(&self) -> Option<&Lyrics> {
        let (uri, lyrics) = self.lyrics.as_ref()?;
        if !self.is_current(uri) {
            return None;
        }
        lyrics.as_ref()
    }

    fn lyric_line(&self) -> Option<usize> {
        let lyrics = self.current_lyrics().filter(|lyrics| lyrics.synced)?;
        let position = self.now.position();
        lyrics
            .lines
            .iter()
            .rposition(|line| line.start_ms <= position)
    }

    fn follow_lyrics(&mut self) -> bool {
        let line = self.lyric_line();
        if line == self.lyrics_line {
            return false;
        }
        self.lyrics_line = line;
        if self.page == Page::Lyrics
            && let Some(line) = line
        {
            self.lyrics_scroll
                .scroll_to_top_of_item((line + 1).saturating_sub(3));
        }
        true
    }

    fn seek_to(&mut self, position: u32, cx: &mut Context<Self>) {
        self.now.position_ms = position;
        self.now.observed = std::time::Instant::now();
        self.seek_hold = Some((position, std::time::Instant::now()));
        self.send(Command::Seek(position));
        self.follow_lyrics();
        cx.notify();
    }

    fn show_lyrics(&mut self, show: bool) {
        if show {
            self.navigate(Page::Lyrics);
            self.lyrics_line = None;
            self.follow_lyrics();
        } else if self.page == Page::Lyrics {
            self.back();
        }
    }

    fn navigate(&mut self, page: Page) {
        if self.page == page {
            return;
        }
        match &page {
            Page::Artist(uri) if self.artist.as_ref().is_none_or(|(loaded, _)| loaded != uri) => {
                self.artist = None;
                self.send(Command::Artist(uri.clone()));
            }
            Page::Collection(uri)
                if self
                    .collection
                    .as_ref()
                    .is_none_or(|(loaded, _)| loaded != uri) =>
            {
                self.collection = None;
                self.send(Command::Collection(uri.clone()));
            }
            _ => {}
        }
        let previous = std::mem::replace(&mut self.page, page);
        self.history.push(previous);
        if self.history.len() > 30 {
            self.history.remove(0);
        }
    }

    fn back(&mut self) {
        let page = self.history.pop().unwrap_or(Page::Home);
        match &page {
            Page::Artist(uri) if self.artist.as_ref().is_none_or(|(loaded, _)| loaded != uri) => {
                self.send(Command::Artist(uri.clone()));
            }
            Page::Collection(uri)
                if self
                    .collection
                    .as_ref()
                    .is_none_or(|(loaded, _)| loaded != uri) =>
            {
                self.send(Command::Collection(uri.clone()));
            }
            _ => {}
        }
        self.page = page;
    }

    fn play_context(&mut self, uri: &str, tracks: &[Track], index: usize, cx: &mut Context<Self>) {
        if let Some(track) = tracks.get(index) {
            self.now.track = Some(track.clone());
            self.now.playing = true;
            self.now.position_ms = 0;
            self.now.observed = std::time::Instant::now();
        }
        self.notice = None;
        self.send(Command::PlayContext {
            uri: uri.to_owned(),
            index,
            track_uri: tracks.get(index).map(|track| track.uri.clone()),
        });
        cx.notify();
    }

    fn handle_control(&mut self, action: overlay::Action, cx: &mut Context<Self>) {
        match action {
            overlay::Action::Press("previous") => self.send(Command::Previous),
            overlay::Action::Press("next") => self.send(Command::Next),
            overlay::Action::Press("toggle-playback") => self.toggle(cx),
            overlay::Action::Press("play-station") => {
                if let Some(mix) = self.newest_mix.as_ref().or(self.home.mix.as_ref()) {
                    let tracks = mix.tracks.clone();
                    self.play(&tracks, 0, cx);
                }
            }
            overlay::Action::Press("sign-in") => {
                self.account_error = None;
                self.account = Account::Connecting;
                self.send(Command::SignIn);
            }
            overlay::Action::Press("cancel") => self.send(Command::Cancel),
            overlay::Action::Press("copy-code") => {
                if let Account::Pairing { code, .. } = &self.account {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(code.clone()));
                }
            }
            overlay::Action::Press("open-pairing") => {
                if let Account::Pairing { url, .. } = &self.account {
                    cx.open_url(url);
                }
            }
            overlay::Action::Press("play-here") => self.send(Command::PlayHere),
            overlay::Action::Press("lyrics") => self.show_lyrics(self.page != Page::Lyrics),
            overlay::Action::Press("back") => self.back(),
            overlay::Action::Press("play-artist") => {
                if let Some((uri, Ok(artist))) = &self.artist {
                    let (uri, tracks) = (uri.clone(), artist.top_tracks.clone());
                    self.play_context(&uri, &tracks, 0, cx);
                }
            }
            overlay::Action::Press("play-collection") => {
                if let Some((uri, Ok(collection))) = &self.collection {
                    let (uri, tracks) = (uri.clone(), collection.tracks.clone());
                    self.play_context(&uri, &tracks, 0, cx);
                }
            }
            overlay::Action::Press(_) => {}
            overlay::Action::Seek(fraction) => {
                if let Some(duration) = self.duration() {
                    let position = (fraction * duration.saturating_sub(1) as f32) as u32;
                    self.seek_to(position, cx);
                }
            }
            overlay::Action::Volume(fraction) => {
                let volume = (fraction.clamp(0., 1.) * f32::from(u16::MAX)).round() as u16;
                self.now.volume = volume;
                self.send(Command::Volume(volume));
            }
        }
        cx.notify();
    }

    fn handle_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            Event::Connecting => self.account = Account::Connecting,
            Event::Pairing { code, url } => self.account = Account::Pairing { code, url },
            Event::Connected => {
                self.account = Account::Connected;
                self.discovery_pending = true;
                self.send(Command::Discover);
                self.playlists_loading = true;
                self.send(Command::Playlists);
                if let Some(track) = self.initial_track.take() {
                    self.input
                        .update(cx, |input, cx| input.set_value(track, window, cx));
                    self.search(cx);
                }
            }
            Event::SignedOut(message) => {
                self.account = Account::SignedOut;
                self.account_error = message;
                self.search_error = None;
                self.results.clear();
                self.home = Discovery::default();
                self.newest_mix = None;
                self.last_seed = None;
                self.discovery_pending = false;
                self.page = Page::Home;
                self.history.clear();
                self.playlists.clear();
                self.artist = None;
                self.collection = None;
                self.covers.clear();
                self.now = NowPlaying::default();
                self.lyrics = None;
                self.notice = None;
                self.search_pending = false;
            }
            Event::Playlists(playlists) => {
                self.playlists = playlists;
                self.playlists_loading = false;
            }
            Event::Artist { uri, result } => self.artist = Some((uri, result)),
            Event::Collection { uri, result } => self.collection = Some((uri, result)),
            Event::SearchDone(result) => {
                self.search_pending = false;
                match result {
                    Ok(tracks) => self.results = tracks,
                    Err(error) => self.search_error = Some(error),
                }
            }
            Event::DirectTrack(result) => {
                self.search_pending = false;
                match result {
                    Ok(track) => {
                        self.results = vec![track.clone()];
                        self.play(&[track], 0, cx);
                    }
                    Err(error) => self.search_error = Some(error),
                }
            }
            Event::Discovery(home) => {
                self.discovery_pending = false;
                self.home = home;
            }
            Event::Recommendations(mix) => {
                if mix.is_some() {
                    self.newest_mix = mix;
                }
            }
            Event::Cover { url, jpeg } => {
                self.covers
                    .insert(url, Arc::new(Image::from_bytes(image_format(&jpeg), jpeg)));
                if self.covers.len() > COVER_CACHE_LIMIT {
                    self.evict_covers(window, cx);
                }
            }
            Event::NowPlaying(now) => {
                if let Some(track) = &now.track
                    && track.is_track()
                    && self
                        .lyrics
                        .as_ref()
                        .is_none_or(|(uri, _)| *uri != track.uri)
                {
                    self.lyrics = Some((track.uri.clone(), None));
                    self.lyrics_loading = true;
                    self.lyrics_line = None;
                    self.send(Command::Lyrics(track.uri.clone()));
                }
                if let Some(track) = &now.track
                    && track.is_track()
                    && !track.title.is_empty()
                    && self.last_seed.as_ref() != Some(&track.uri)
                {
                    self.last_seed = Some(track.uri.clone());
                    self.send(Command::Recommend(track.clone()));
                }
                let expected = self.now.position();
                let held = self.seek_hold.filter(|(_, at)| {
                    at.elapsed() < Duration::from_secs(3)
                        && now.track.as_ref().map(|track| &track.uri)
                            == self.now.track.as_ref().map(|track| &track.uri)
                        && now.position().abs_diff(expected) > 1500
                });
                let (position, observed) = (self.now.position_ms, self.now.observed);
                self.now = now;
                if held.is_some() {
                    self.now.position_ms = position;
                    self.now.observed = observed;
                } else {
                    self.seek_hold = None;
                }
                self.follow_lyrics();
            }
            Event::Notice(message) => {
                self.notice = Some(message);
                self.seek_hold = None;
            }
            Event::Lyrics { uri, lyrics } => {
                if self
                    .lyrics
                    .as_ref()
                    .is_some_and(|(pending, _)| *pending == uri)
                {
                    self.lyrics = Some((uri, lyrics));
                    self.lyrics_loading = false;
                    self.lyrics_line = None;
                    self.follow_lyrics();
                }
            }
        }
        cx.notify();
    }
}

const COVER_CACHE_LIMIT: usize = 220;

fn image_format(bytes: &[u8]) -> ImageFormat {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        ImageFormat::Png
    } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        ImageFormat::Webp
    } else {
        ImageFormat::Jpeg
    }
}

fn timestamp(ms: u32) -> String {
    let seconds = ms / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
