use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    sync::Arc,
    time::Instant,
};

use gpui::{
    AnyElement, Bounds, Context, Corners, FontWeight, Hsla, Image, MouseButton, ObjectFit, Pixels,
    canvas, div, fill, point, prelude::*, px, rgb, rgba, size,
};
use spotify_client::{
    catalog::Track,
    engine::playback::{VideoKind, VideoStatus},
    music_video::{MusicVideo, Song, VideoPage},
    podcast::Episode,
};

use super::{
    ask::Asks,
    video::{self, Cap, Feed, fit},
    view::{heading, kind_label},
    worker::Command,
};
use crate::{
    app::{OpenMusic, Page, page_for},
    ui::{
        Palette,
        components::link,
        drawn,
        menu::{Surface, Target},
    },
};

pub const VIDEO_KEY: &str = "npv-video";
pub const FULL_KEY: &str = "npv-full-screen";
pub const ALBUM_KEY: &str = "npv-album";

const WIDEST_VIDEO: f32 = 960.;
const WIDEST_CANVAS: f32 = 360.;
const WIDEST_COVER: f32 = 520.;
const RADIUS: f32 = 14.;
const DOCK_CLEARANCE: f32 = 112.;
const KEEP: usize = 200;
const COLLAPSED_VIDEOS: usize = 3;
const CARD_GAP: f32 = 8.;
const TITLE_LINE: f32 = 20.;
const SUBTITLE_LINE: f32 = 18.;
const TO_VIDEO: &str = "We’ll play video when available, from now on.";
const TO_AUDIO: &str = "We’ll only play audio from now on.";
const ELSEWHERE: &str = "Video shows when this plays on this Mac.";

const LARGE_COVERS: [(&str, &str); 4] = [
    ("ab67616d00001e02", "ab67616d0000b273"),
    ("ab67616d00004851", "ab67616d0000b273"),
    ("ab67656300005f1f", "ab6765630000ba8a"),
    ("ab6765630000f68d", "ab6765630000ba8a"),
];

pub fn large_cover(url: &str) -> Option<String> {
    let (base, id) = url.rsplit_once('/')?;
    LARGE_COVERS.iter().find_map(|(small, large)| {
        id.strip_prefix(small)
            .map(|rest| format!("{base}/{large}{rest}"))
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Visual {
    Video(f32),
    Canvas,
    Cover,
}

impl Visual {
    fn shape(self) -> (f32, f32) {
        match self {
            Self::Video(aspect) => (aspect, WIDEST_VIDEO),
            Self::Canvas => (9. / 16., WIDEST_CANVAS),
            Self::Cover => (1., WIDEST_COVER),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Video(_) => "video",
            Self::Canvas => "canvas",
            Self::Cover => "cover",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Switch {
    ToAudio(VideoKind),
    ToVideo(VideoKind),
    Watch(String),
}

impl Switch {
    fn label(&self) -> &'static str {
        match self {
            Self::ToAudio(_) => "Switch to audio",
            Self::ToVideo(_) | Self::Watch(_) => "Switch to video",
        }
    }
}

pub fn switch_for(
    status: Option<VideoStatus>,
    wanted: impl Fn(VideoKind) -> bool,
    local: bool,
    track: &Track,
    known_video: Option<&str>,
) -> Option<Switch> {
    if !local {
        return None;
    }
    match status {
        Some(status) if status.on => Some(Switch::ToAudio(status.kind)),
        Some(status) if wanted(status.kind) => None,
        Some(status) => Some(Switch::ToVideo(status.kind)),
        None if track.is_track() => known_video
            .filter(|video| *video != track.uri)
            .map(|video| Switch::Watch(video.to_owned())),
        None => None,
    }
}

pub fn visual_for(status: Option<VideoStatus>, local: bool, canvas: bool) -> Visual {
    match status {
        Some(status) if local && status.on => Visual::Video(status.aspect.unwrap_or(16. / 9.)),
        _ if canvas => Visual::Canvas,
        _ => Visual::Cover,
    }
}

pub fn kind_text(track: &Track, status: Option<VideoStatus>, episode_video: bool) -> &'static str {
    match status.map(|status| status.kind) {
        Some(VideoKind::Music) => "MUSIC VIDEO",
        Some(VideoKind::Podcast) => "VIDEO PODCAST",
        None if track.uri.starts_with("spotify:episode:") && episode_video => "VIDEO PODCAST",
        None if track.uri.starts_with("spotify:episode:") => "PODCAST EPISODE",
        None => "NOW PLAYING",
    }
}

pub struct ArtistVideos {
    pub uri: String,
    pub page: Option<Result<VideoPage, String>>,
    pub all: bool,
    pub loading: bool,
}

#[derive(Default)]
pub struct Screen {
    pub videos: HashMap<String, Option<String>>,
    pub songs: HashMap<String, Option<Song>>,
    pub episode: Option<(String, Episode)>,
    video_asks: Asks,
    song_asks: Asks,
    episode_asks: Asks,
    pub artist: Option<ArtistVideos>,
    pub large: Option<String>,
}

impl Screen {
    pub fn covers(&self, urls: &mut HashSet<String>) {
        urls.extend(self.large.clone());
        if let Some((_, episode)) = &self.episode {
            urls.extend(episode.large_cover.clone());
        }
        if let Some(ArtistVideos {
            page: Some(Ok(page)),
            ..
        }) = &self.artist
        {
            urls.extend(page.videos.iter().filter_map(|video| video.still.clone()));
        }
    }

    fn trim(&mut self) {
        if self.videos.len() > KEEP {
            self.videos.clear();
        }
        if self.songs.len() > KEEP {
            self.songs.clear();
        }
    }

    pub fn took_video(&mut self, uri: String, result: Result<Option<String>, String>) {
        match result {
            Ok(video) => {
                self.video_asks.answered(&uri);
                self.videos.insert(uri, video);
            }
            Err(_) => self.video_asks.fail(uri, Instant::now()),
        }
        self.trim();
    }

    pub fn took_song(&mut self, uri: String, result: Result<Option<Song>, String>) {
        match result {
            Ok(song) => {
                self.song_asks.answered(&uri);
                self.songs.insert(uri, song);
            }
            Err(_) => self.song_asks.fail(uri, Instant::now()),
        }
        self.trim();
    }

    pub fn took_episode(&mut self, uri: String, result: Result<Episode, String>) {
        match result {
            Ok(episode) => {
                self.episode_asks.answered(&uri);
                self.episode = Some((uri, episode));
            }
            Err(_) => self.episode_asks.fail(uri, Instant::now()),
        }
    }

    pub fn took_artist_videos(
        &mut self,
        uri: &str,
        offset: u32,
        result: Result<VideoPage, String>,
    ) {
        let Some(artist) = self.artist.as_mut().filter(|artist| artist.uri == uri) else {
            return;
        };
        artist.loading = false;
        match (&mut artist.page, result) {
            (Some(Ok(page)), Ok(more)) if offset > 0 => {
                let known: HashSet<String> = page
                    .videos
                    .iter()
                    .map(|video| video.track.uri.clone())
                    .collect();
                page.videos.extend(
                    more.videos
                        .into_iter()
                        .filter(|video| !known.contains(&video.track.uri)),
                );
                page.next = more.next.filter(|next| *next > offset);
                page.total = more.total;
            }
            (_, result) if offset == 0 => artist.page = Some(result),
            _ => {}
        }
    }
}

impl OpenMusic {
    pub fn video_status(&self) -> Option<VideoStatus> {
        self.playback
            .now
            .local
            .then_some(self.audio.status.video)
            .flatten()
            .filter(|_| {
                self.track()
                    .is_some_and(|track| VideoKind::of(&track.uri).is_some())
            })
    }

    pub fn video_on(&self) -> bool {
        self.video_status().is_some_and(|status| status.on)
    }

    fn video_switch(&self) -> Option<Switch> {
        let track = self.track()?;
        let known = self
            .content
            .screen
            .videos
            .get(&track.uri)
            .cloned()
            .flatten();
        let prefs = &self.settings.content;
        switch_for(
            self.video_status(),
            |kind| match kind {
                VideoKind::Music => prefs.music_videos,
                VideoKind::Podcast => prefs.video_podcasts,
            },
            self.playback.now.local,
            track,
            known.as_deref(),
        )
    }

    fn set_video_setting(&mut self, kind: VideoKind, on: bool) {
        self.update_settings(|settings| match kind {
            VideoKind::Music => settings.content.music_videos = on,
            VideoKind::Podcast => settings.content.video_podcasts = on,
        });
    }

    pub fn switch_video(&mut self, cx: &mut Context<Self>) {
        let Some(switch) = self.video_switch() else {
            return;
        };
        match switch {
            Switch::ToAudio(kind) => {
                self.set_video_setting(kind, false);
                self.show_toast(TO_AUDIO, None);
            }
            Switch::ToVideo(kind) => {
                self.set_video_setting(kind, true);
                self.show_toast(TO_VIDEO, None);
            }
            Switch::Watch(uri) => {
                if self.needs_premium_here(false, cx) {
                    return;
                }
                if !self.settings.content.music_videos {
                    self.set_video_setting(VideoKind::Music, true);
                    self.show_toast(TO_VIDEO, None);
                }
                let position_ms = self.playback.now.position();
                self.playback.notice = None;
                self.send(Command::Watch { uri, position_ms });
            }
        }
        cx.notify();
    }

    pub fn toggle_now_playing(&mut self) {
        if self.page == Page::NowPlaying {
            self.back();
        } else {
            self.navigate(Page::NowPlaying);
        }
    }

    pub fn play_music_videos(
        &mut self,
        videos: &[MusicVideo],
        index: usize,
        cx: &mut Context<Self>,
    ) {
        let tracks: Vec<Track> = videos.iter().map(|video| video.track.clone()).collect();
        self.play(&tracks, index, cx);
        if self.page != Page::NowPlaying {
            self.navigate(Page::NowPlaying);
        }
    }

    pub fn go_to_video_album(&mut self) {
        if let Some(album) = self.video_album() {
            self.navigate(Page::Collection(album));
        }
    }

    fn video_album(&self) -> Option<String> {
        let track = self.track()?;
        self.content
            .screen
            .songs
            .get(&track.uri)?
            .as_ref()?
            .album
            .clone()
    }

    fn screen_episode(&self, uri: &str) -> Option<&Episode> {
        self.content.loaded_episode(uri).or_else(|| {
            self.content
                .screen
                .episode
                .as_ref()
                .filter(|(loaded, _)| loaded == uri)
                .map(|(_, episode)| episode)
        })
    }

    fn request_screen(&mut self) {
        let Some(track) = self.track().cloned() else {
            return;
        };
        if let Some(large) = track.cover.as_deref().and_then(large_cover)
            && self.content.screen.large.as_ref() != Some(&large)
        {
            self.content.screen.large = Some(large.clone());
            if !self.covers.contains_key(&large) {
                self.send(Command::Artwork(large));
            }
        }
        let status = self.video_status();
        let now = Instant::now();
        let song = track.is_track();
        let uri = track.uri;
        if uri.starts_with("spotify:episode:") {
            if self.screen_episode(&uri).is_none()
                && self.content.screen.episode_asks.ask(&uri, now)
            {
                self.send(Command::NowEpisode(uri));
            }
        } else if status.is_some_and(|status| status.kind == VideoKind::Music) {
            let screen = &mut self.content.screen;
            if !screen.songs.contains_key(&uri) && screen.song_asks.ask(&uri, now) {
                self.send(Command::SongOf(uri));
            }
        } else if song && self.playback.now.local {
            let screen = &mut self.content.screen;
            if !screen.videos.contains_key(&uri) && screen.video_asks.ask(&uri, now) {
                self.send(Command::VideoOf(uri));
            }
        }
    }

    fn request_artist_videos(&mut self) {
        let Page::Artist(uri) = &self.page else {
            return;
        };
        if self
            .content
            .screen
            .artist
            .as_ref()
            .is_some_and(|artist| artist.uri == *uri)
        {
            return;
        }
        let uri = uri.clone();
        self.content.screen.artist = Some(ArtistVideos {
            uri: uri.clone(),
            page: None,
            all: false,
            loading: true,
        });
        self.send(Command::ArtistVideos { uri, offset: 0 });
    }

    pub fn more_artist_videos(&mut self) {
        let Some(artist) = self.content.screen.artist.as_mut() else {
            return;
        };
        let Some(Ok(page)) = &artist.page else {
            return;
        };
        let Some(offset) = page.next.filter(|_| !artist.loading) else {
            return;
        };
        artist.loading = true;
        let uri = artist.uri.clone();
        self.send(Command::ArtistVideos { uri, offset });
    }

    pub fn screen_tick(&mut self, cx: &mut Context<Self>) {
        video::install_artwork_click();
        let on = self.video_on();
        video::dock_video(on);
        let showing = self.page == Page::NowPlaying;
        let full = video::full_screen();
        if on {
            video::keep_full_screen();
            video::cap_video(if full || (showing && !self.content.canvas.covered) {
                Cap::Full
            } else if self.content.canvas.covered {
                Cap::Hidden
            } else {
                Cap::Dock
            });
        } else if full {
            video::leave_full_screen();
        }
        if showing {
            self.request_screen();
        }
        self.request_artist_videos();
        if video::take_redraw() || (showing && !video::screen_settled()) {
            cx.notify();
        }
    }

    pub fn screen_page(&self, palette: Palette, cx: &mut Context<Self>) -> AnyElement {
        let page = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .px(px(48.))
            .pt(px(40.))
            .pb(px(DOCK_CLEARANCE));
        let Some(track) = self.track().cloned() else {
            return page
                .child(div().child(self.back_button(palette, cx)))
                .child(kind_label("NOW PLAYING", palette))
                .child(
                    div()
                        .text_size(px(26.))
                        .font_weight(FontWeight::BOLD)
                        .child("Nothing is playing"),
                )
                .child(
                    div()
                        .text_color(rgb(palette.muted))
                        .child("Play something to see it here."),
                )
                .into_any_element();
        };
        let status = self.video_status();
        let local = self.playback.now.local;
        let episode = self.screen_episode(&track.uri).cloned();
        let episode_video = episode.as_ref().is_some_and(|episode| episode.video);
        let visual = visual_for(status, local, self.canvas_want().is_some());
        let switch = self.video_switch();
        let image = track
            .cover
            .as_deref()
            .and_then(large_cover)
            .and_then(|url| self.covers.get(&url))
            .or_else(|| self.cover_for(&track))
            .or_else(|| {
                episode
                    .as_ref()
                    .and_then(|episode| episode.large_cover.as_ref())
                    .and_then(|url| self.covers.get(url))
            })
            .cloned();
        let elsewhere = (!local && episode_video).then_some(ELSEWHERE);
        let device = (!local).then(|| self.playback.now.device.clone()).flatten();
        let album = status
            .is_some_and(|status| status.kind == VideoKind::Music)
            .then(|| self.video_album())
            .flatten();
        let about = episode
            .as_ref()
            .map(Episode::summary)
            .filter(|summary| !summary.is_empty());
        page.child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(self.back_button(palette, cx))
                .child(kind_label(
                    kind_text(&track, status, episode_video),
                    palette,
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .on_mouse_down(
                    MouseButton::Right,
                    self.on_menu(None, |app| app.now_playing_target(Surface::Header), cx),
                )
                .child(
                    div()
                        .w_full()
                        .text_size(px(26.))
                        .line_height(px(32.))
                        .font_weight(FontWeight::BOLD)
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(track.title.clone()),
                )
                .child(self.screen_byline(&track, device, palette, cx)),
        )
        .child(
            div()
                .flex_1()
                .min_h(px(120.))
                .w_full()
                .relative()
                .child(screen_visual(visual, image, self.modal_open(), palette)),
        )
        .when(
            switch.is_some() || album.is_some() || elsewhere.is_some(),
            |page| {
                page.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .when_some(switch.as_ref(), |row, switch| {
                            row.child(self.text_control(
                                VIDEO_KEY,
                                switch.label(),
                                false,
                                palette,
                                cx,
                            ))
                        })
                        .when(matches!(visual, Visual::Video(_)), |row| {
                            row.child(self.text_control(
                                FULL_KEY,
                                "Full screen",
                                false,
                                palette,
                                cx,
                            ))
                        })
                        .when(album.is_some(), |row| {
                            row.child(self.text_control(
                                ALBUM_KEY,
                                "Go to album",
                                false,
                                palette,
                                cx,
                            ))
                        })
                        .when_some(elsewhere, |row, note| {
                            row.child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(rgb(palette.muted))
                                    .child(note),
                            )
                        }),
                )
            },
        )
        .when_some(about, |page, about| {
            page.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .max_w(px(760.))
                    .child(heading("About the episode"))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .line_height(px(20.))
                            .text_color(rgb(palette.muted))
                            .line_clamp(3)
                            .text_ellipsis()
                            .child(about),
                    ),
            )
        })
        .into_any_element()
    }

    fn screen_byline(
        &self,
        track: &Track,
        device: Option<String>,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let links: Vec<(String, String)> = if track.artists.is_empty() {
            (!track.artist.is_empty())
                .then(|| {
                    (
                        track.artist.clone(),
                        track.artist_uri.clone().unwrap_or_default(),
                    )
                })
                .into_iter()
                .collect()
        } else {
            track
                .artists
                .iter()
                .map(|artist| (artist.name.clone(), artist.uri.clone()))
                .collect()
        };
        let count = links.len();
        div()
            .flex()
            .items_center()
            .min_w_0()
            .text_color(rgb(palette.muted))
            .children(links.into_iter().enumerate().map(|(index, (name, uri))| {
                div()
                    .flex()
                    .min_w_0()
                    .child(link(
                        ("npv-artist", index),
                        name,
                        page_for(&uri),
                        15.,
                        palette,
                        cx,
                    ))
                    .when(index + 1 < count, |part| part.child(", "))
            }))
            .when_some(device, |line, device| {
                line.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(15.))
                        .child(format!(" · Playing on {device}")),
                )
            })
    }

    pub fn artist_videos_section(
        &self,
        uri: &str,
        card_side: Pixels,
        palette: Palette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let artist = self
            .content
            .screen
            .artist
            .as_ref()
            .filter(|artist| artist.uri == uri)?;
        let Some(Ok(page)) = &artist.page else {
            return None;
        };
        if page.videos.is_empty() {
            return None;
        }
        let width = (f32::from(card_side) * 1.6).clamp(200., 320.);
        let still = width * 9. / 16.;
        let all = artist.all;
        let expandable = page.videos.len() > COLLAPSED_VIDEOS || page.next.is_some();
        let more = all && page.next.is_some();
        let videos = page.videos.clone();
        let cards = videos.iter().enumerate().map(|(index, video)| {
            let image = video
                .still
                .as_ref()
                .and_then(|url| self.covers.get(url))
                .cloned();
            let target = Target::track(video.track.clone(), None, Surface::Card);
            let list = videos.clone();
            div()
                .id(("artist-video", index))
                .w(px(width))
                .flex()
                .flex_col()
                .gap(px(CARD_GAP))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.play_music_videos(&list, index, cx);
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    self.on_menu(None, move |_| Some(target.clone()), cx),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(still))
                        .rounded(px(10.))
                        .overflow_hidden()
                        .bg(rgb(palette.raised))
                        .hover(|card| card.opacity(0.85))
                        .when_some(image, |card, image| {
                            card.child(drawn::cover(image).size_full().object_fit(ObjectFit::Cover))
                        }),
                )
                .child(
                    div()
                        .text_size(px(14.))
                        .line_height(px(TITLE_LINE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(video.track.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .line_height(px(SUBTITLE_LINE))
                        .text_color(rgb(palette.muted))
                        .truncate()
                        .child(format!("Music video • {}", video.track.artist)),
                )
        });
        let grid = div().flex().flex_wrap().gap_4().children(cards);
        Some(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_4()
                        .child(heading("Music videos"))
                        .when(expandable, |row| {
                            row.child(screen_link(
                                "artist-videos-all",
                                if all { "Show less" } else { "Show all" },
                                palette,
                                cx,
                                |app| {
                                    if let Some(artist) = app.content.screen.artist.as_mut() {
                                        artist.all = !artist.all;
                                    }
                                },
                            ))
                        }),
                )
                .child(if all {
                    grid
                } else {
                    grid.max_h(px(still + CARD_GAP * 2. + TITLE_LINE + SUBTITLE_LINE))
                        .overflow_hidden()
                })
                .when(more, |section| {
                    section.child(screen_link(
                        "artist-videos-more",
                        "Show more",
                        palette,
                        cx,
                        OpenMusic::more_artist_videos,
                    ))
                })
                .into_any_element(),
        )
    }

    pub fn describe_screen(&self, lines: &mut String) {
        let status = self.video_status();
        let _ = writeln!(
            lines,
            "settings music_videos={} video_podcasts={}",
            self.settings.content.music_videos, self.settings.content.video_podcasts
        );
        match self.track() {
            Some(track) => {
                let _ = writeln!(
                    lines,
                    "track {} local={} kind={}",
                    track.uri,
                    self.playback.now.local,
                    kind_text(
                        track,
                        status,
                        self.screen_episode(&track.uri)
                            .is_some_and(|episode| episode.video)
                    ),
                );
            }
            None => lines.push_str("track none\n"),
        }
        match self.audio.status.video {
            Some(video) => {
                let _ = writeln!(
                    lines,
                    "engine kind={} on={} aspect={:?} counted={}",
                    video.kind.name(),
                    video.on,
                    video.aspect,
                    status.is_some()
                );
            }
            None => lines.push_str("engine none\n"),
        }
        let visual = visual_for(
            status,
            self.playback.now.local,
            self.canvas_want().is_some(),
        );
        let _ = writeln!(
            lines,
            "visual {} switch={:?} album={:?}",
            visual.name(),
            self.video_switch(),
            self.video_album()
        );
        if let Some(track) = self.track() {
            if let Some(video) = self.content.screen.videos.get(&track.uri) {
                let _ = writeln!(lines, "video_of {} {video:?}", track.uri);
            }
            if let Some(song) = self.content.screen.songs.get(&track.uri) {
                let _ = writeln!(lines, "song_of {} {song:?}", track.uri);
            }
        }
        let _ = writeln!(lines, "{}", video::describe_screen());
        let _ = writeln!(
            lines,
            "full_screen={} settled={}",
            video::full_screen(),
            video::screen_settled()
        );
    }

    pub fn describe_artist_videos(&self, lines: &mut String) {
        let Some(artist) = &self.content.screen.artist else {
            lines.push_str("artist none\n");
            return;
        };
        match &artist.page {
            None => {
                let _ = writeln!(lines, "artist {} loading", artist.uri);
            }
            Some(Err(error)) => {
                let _ = writeln!(lines, "artist {} error={error:?}", artist.uri);
            }
            Some(Ok(page)) => {
                let _ = writeln!(
                    lines,
                    "artist {} videos={} total={} next={:?} all={} loading={}",
                    artist.uri,
                    page.videos.len(),
                    page.total,
                    page.next,
                    artist.all,
                    artist.loading
                );
                for video in &page.videos {
                    let _ = writeln!(
                        lines,
                        "  {} {:?} audio={:?} still={}",
                        video.track.uri,
                        video.track.title,
                        video.audio,
                        video.still.is_some()
                    );
                }
            }
        }
    }
}

fn screen_link(
    id: &'static str,
    text: &'static str,
    palette: Palette,
    cx: &mut Context<OpenMusic>,
    run: impl Fn(&mut OpenMusic) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .flex_shrink_0()
        .cursor_pointer()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(palette.muted))
        .hover(|text| text.text_color(rgb(palette.text)))
        .child(text)
        .on_click(cx.listener(move |this, _, _, cx| {
            run(this);
            cx.notify();
        }))
}

fn centred(bounds: Bounds<Pixels>, width: f32, height: f32) -> Bounds<Pixels> {
    let (width, height) = (px(width), px(height));
    Bounds {
        origin: point(
            bounds.origin.x + (bounds.size.width - width) / 2.,
            bounds.origin.y + (bounds.size.height - height) / 2.,
        ),
        size: size(width, height),
    }
}

fn screen_visual(
    visual: Visual,
    image: Option<Arc<Image>>,
    modal: bool,
    palette: Palette,
) -> impl IntoElement {
    let backdrop: Hsla = match visual {
        Visual::Video(_) => rgb(0x000000).into(),
        Visual::Canvas | Visual::Cover => rgb(palette.raised).into(),
    };
    let rim: Hsla = rgba(palette.line).into();
    canvas(
        move |_, window, cx| image.and_then(|image| image.use_render_image(window, cx)),
        move |bounds, image, window, _| {
            let (aspect, widest) = visual.shape();
            let (width, height) = fit(
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
                aspect,
                widest,
            );
            if width < 1. || height < 1. {
                return;
            }
            let frame = centred(bounds, width, height);
            window.paint_quad(
                fill(frame, backdrop)
                    .corner_radii(px(RADIUS))
                    .border_widths(px(1.))
                    .border_color(rim),
            );
            if let Some(image) = image {
                let side = width.min(height);
                let poster = match visual {
                    Visual::Cover => frame,
                    Visual::Video(_) | Visual::Canvas => centred(bounds, side, side),
                };
                let _ = window.paint_image(poster, Corners::all(px(RADIUS)), image, 0, false);
            }
            match visual {
                _ if modal => {}
                Visual::Video(_) => video::place_screen(frame, Feed::Main, RADIUS, window),
                Visual::Canvas => video::place_screen(frame, Feed::Canvas, RADIUS, window),
                Visual::Cover => {}
            }
        },
    )
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SONG: &str = "spotify:track:00000000000000000000s1";
    const VIDEO: &str = "spotify:track:00000000000000000000v1";
    const EPISODE: &str = "spotify:episode:0000000000000000000e1";

    fn track(uri: &str) -> Track {
        Track {
            uri: uri.into(),
            ..Track::default()
        }
    }

    fn status(kind: VideoKind, on: bool) -> Option<VideoStatus> {
        Some(VideoStatus {
            kind,
            on,
            aspect: Some(1.5),
        })
    }

    #[test]
    fn offers_the_switch_that_fits_what_plays() {
        let off = |_: VideoKind| false;
        let on = |_: VideoKind| true;
        assert_eq!(
            switch_for(
                status(VideoKind::Podcast, true),
                on,
                true,
                &track(EPISODE),
                None
            ),
            Some(Switch::ToAudio(VideoKind::Podcast))
        );
        assert_eq!(
            switch_for(
                status(VideoKind::Music, false),
                off,
                true,
                &track(VIDEO),
                None
            ),
            Some(Switch::ToVideo(VideoKind::Music))
        );
        assert_eq!(
            switch_for(
                status(VideoKind::Music, false),
                on,
                true,
                &track(VIDEO),
                None
            ),
            None
        );
        assert_eq!(
            switch_for(None, off, true, &track(SONG), Some(VIDEO)),
            Some(Switch::Watch(VIDEO.into()))
        );
        assert_eq!(switch_for(None, on, true, &track(SONG), Some(SONG)), None);
        assert_eq!(switch_for(None, on, true, &track(SONG), None), None);
        assert_eq!(
            switch_for(None, on, true, &track(EPISODE), Some(VIDEO)),
            None
        );
        assert_eq!(
            switch_for(
                status(VideoKind::Podcast, true),
                on,
                false,
                &track(EPISODE),
                None
            ),
            None
        );
        assert_eq!(Switch::ToAudio(VideoKind::Music).label(), "Switch to audio");
        assert_eq!(Switch::Watch(VIDEO.into()).label(), "Switch to video");
    }

    #[test]
    fn shows_video_then_canvas_then_the_cover() {
        assert_eq!(
            visual_for(status(VideoKind::Music, true), true, true),
            Visual::Video(1.5)
        );
        assert_eq!(
            visual_for(
                Some(VideoStatus {
                    kind: VideoKind::Podcast,
                    on: true,
                    aspect: None,
                }),
                true,
                false
            ),
            Visual::Video(16. / 9.)
        );
        assert_eq!(
            visual_for(status(VideoKind::Music, false), true, true),
            Visual::Canvas
        );
        assert_eq!(
            visual_for(status(VideoKind::Music, true), false, false),
            Visual::Cover
        );
        assert_eq!(visual_for(None, true, false), Visual::Cover);
        assert_eq!(Visual::Canvas.shape(), (9. / 16., WIDEST_CANVAS));
        assert_eq!(Visual::Cover.name(), "cover");
    }

    #[test]
    fn labels_what_plays() {
        assert_eq!(kind_text(&track(SONG), None, false), "NOW PLAYING");
        assert_eq!(
            kind_text(&track(VIDEO), status(VideoKind::Music, false), false),
            "MUSIC VIDEO"
        );
        assert_eq!(kind_text(&track(EPISODE), None, false), "PODCAST EPISODE");
        assert_eq!(kind_text(&track(EPISODE), None, true), "VIDEO PODCAST");
    }

    #[test]
    fn asks_for_the_large_cover() {
        assert_eq!(
            large_cover("https://i.scdn.co/image/ab67616d00001e02aaaabbbbccccdddd").as_deref(),
            Some("https://i.scdn.co/image/ab67616d0000b273aaaabbbbccccdddd")
        );
        assert_eq!(
            large_cover("https://i.scdn.co/image/ab67656300005f1faaaabbbb").as_deref(),
            Some("https://i.scdn.co/image/ab6765630000ba8aaaaabbbb")
        );
        assert_eq!(
            large_cover("https://i.scdn.co/image/ab67616d0000b273aaaa"),
            None
        );
        assert_eq!(large_cover("no-slash"), None);
    }

    fn video(uri: &str) -> MusicVideo {
        MusicVideo {
            track: track(uri),
            still: None,
            audio: None,
        }
    }

    #[test]
    fn pages_through_an_artists_videos() {
        let mut screen = Screen {
            artist: Some(ArtistVideos {
                uri: "spotify:artist:a".into(),
                page: None,
                all: false,
                loading: true,
            }),
            ..Screen::default()
        };
        screen.took_artist_videos(
            "spotify:artist:a",
            0,
            Ok(VideoPage {
                videos: vec![video(VIDEO), video(SONG)],
                total: 3,
                next: Some(2),
            }),
        );
        screen.took_artist_videos("spotify:artist:b", 0, Err("elsewhere".into()));
        screen.took_artist_videos(
            "spotify:artist:a",
            2,
            Ok(VideoPage {
                videos: vec![video(SONG), video("spotify:track:00000000000000000000v3")],
                total: 3,
                next: None,
            }),
        );
        let artist = screen.artist.as_ref().unwrap();
        assert!(!artist.loading);
        let Some(Ok(page)) = &artist.page else {
            panic!("expected a page");
        };
        assert_eq!(page.videos.len(), 3);
        assert_eq!(page.next, None);
    }

    #[test]
    fn remembers_lookups_and_retries_failures_later() {
        let mut screen = Screen::default();
        let now = Instant::now();
        assert!(screen.video_asks.ask(SONG, now));
        screen.took_video(SONG.into(), Ok(Some(VIDEO.into())));
        assert_eq!(screen.videos.get(SONG), Some(&Some(VIDEO.into())));
        assert!(!screen.video_asks.asking(SONG, now));

        assert!(screen.video_asks.ask(VIDEO, now));
        screen.took_video(VIDEO.into(), Err("offline".into()));
        assert!(!screen.videos.contains_key(VIDEO));
        assert!(!screen.video_asks.ask(VIDEO, Instant::now()));
        assert!(
            screen
                .video_asks
                .ask(VIDEO, Instant::now() + super::super::ask::RETRY_AFTER)
        );

        assert!(screen.song_asks.ask(VIDEO, now));
        screen.took_song(VIDEO.into(), Err("offline".into()));
        assert!(!screen.songs.contains_key(VIDEO));
        screen.took_song(VIDEO.into(), Ok(None));
        assert_eq!(screen.songs.get(VIDEO), Some(&None));

        assert!(screen.episode_asks.ask(EPISODE, now));
        screen.took_episode(EPISODE.into(), Err("offline".into()));
        assert!(screen.episode.is_none());
        assert!(!screen.episode_asks.ask(EPISODE, Instant::now()));
        screen.took_episode(EPISODE.into(), Ok(Episode::default()));
        assert!(
            screen
                .episode
                .as_ref()
                .is_some_and(|(uri, _)| uri == EPISODE)
        );
    }

    #[test]
    fn asks_again_for_a_lookup_a_newer_track_cancelled() {
        let mut screen = Screen::default();
        let now = Instant::now();
        assert!(screen.video_asks.ask(SONG, now));
        assert!(screen.video_asks.ask(VIDEO, now));
        screen.took_video(VIDEO.into(), Ok(None));
        assert!(!screen.videos.contains_key(SONG));
        assert!(screen.video_asks.ask(SONG, now));

        assert!(screen.episode_asks.ask(EPISODE, now));
        assert!(
            screen
                .episode_asks
                .ask("spotify:episode:0000000000000000000e2", now)
        );
        assert!(screen.episode_asks.ask(EPISODE, now));
    }
}
