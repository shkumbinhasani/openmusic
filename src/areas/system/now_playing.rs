use crate::app::intent::Intent;

pub use native::{clear, dump, fire, install, installed, publish, set_state};

pub const DRIFT_MS: i64 = 1500;
pub const REMOTE_SEEK_MS: i32 = 15_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u32,
    pub position_ms: u32,
    pub playing: bool,
    pub speed: f32,
    pub cover: Option<String>,
}

impl Snapshot {
    fn same_item(&self, other: &Snapshot) -> bool {
        self.uri == other.uri
            && self.title == other.title
            && self.artist == other.artist
            && self.album == other.album
            && self.duration_ms == other.duration_ms
            && self.playing == other.playing
            && self.speed == other.speed
            && self.cover == other.cover
    }

    pub fn rate(&self) -> f64 {
        if self.playing {
            f64::from(self.speed)
        } else {
            0.
        }
    }
}

pub fn should_publish(last: Option<&Snapshot>, next: &Snapshot, elapsed_ms: u64) -> bool {
    let Some(last) = last else {
        return true;
    };
    if !last.same_item(next) {
        return true;
    }
    let moved = (elapsed_ms as f64 * last.rate()) as i64;
    let mut expected = i64::from(last.position_ms) + moved;
    if next.duration_ms > 0 {
        expected = expected.min(i64::from(next.duration_ms));
    }
    (i64::from(next.position_ms) - expected).abs() > DRIFT_MS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Done,
    NothingPlaying,
    NotInstalled,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Done => "success",
            Status::NothingPlaying => "no-item",
            Status::NotInstalled => "not-installed",
        }
    }
}

pub fn accepts(intent: &Intent, loaded: bool) -> bool {
    loaded || *intent == Intent::Play
}

pub fn parse_remote(text: &str) -> Option<Intent> {
    let (name, rest) = text.split_once(' ').unwrap_or((text, ""));
    let rest = rest.trim();
    let bare = |intent: Intent| rest.is_empty().then_some(intent);
    match name {
        "play" => bare(Intent::Play),
        "pause" => bare(Intent::Pause),
        "toggle" => bare(Intent::PlayPause),
        "next" => bare(Intent::Next),
        "previous" => bare(Intent::Previous),
        "seek" => rest.parse().ok().map(Intent::SeekTo),
        "seekby" => rest.parse().ok().map(Intent::SeekBy),
        "forward" => bare(Intent::SeekBy(REMOTE_SEEK_MS)),
        "backward" => bare(Intent::SeekBy(-REMOTE_SEEK_MS)),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::{
        cell::{Cell, RefCell},
        ptr::NonNull,
        rc::Rc,
        sync::Arc,
    };

    use block2::RcBlock;
    use gpui::Image;
    use objc2::{AllocAnyThread, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::NSImage;
    use objc2_core_foundation::CGSize;
    use objc2_foundation::{NSData, NSMutableDictionary, NSNumber, NSString};
    use objc2_media_player::{
        MPChangePlaybackPositionCommandEvent, MPMediaItemArtwork, MPMediaItemPropertyAlbumTitle,
        MPMediaItemPropertyArtist, MPMediaItemPropertyArtwork, MPMediaItemPropertyPlaybackDuration,
        MPMediaItemPropertyTitle, MPNowPlayingInfoCenter, MPNowPlayingInfoMediaType,
        MPNowPlayingInfoPropertyDefaultPlaybackRate, MPNowPlayingInfoPropertyElapsedPlaybackTime,
        MPNowPlayingInfoPropertyExternalContentIdentifier, MPNowPlayingInfoPropertyMediaType,
        MPNowPlayingInfoPropertyPlaybackRate, MPNowPlayingPlaybackState, MPRemoteCommandCenter,
        MPRemoteCommandEvent, MPRemoteCommandHandlerStatus, MPSeekCommandEvent,
        MPSeekCommandEventType,
    };
    use serde_json::{Map, Value, json};

    use super::{REMOTE_SEEK_MS, Snapshot, Status, accepts};
    use crate::app::intent::Intent;

    type Handler = Rc<dyn Fn(Intent)>;

    struct Remote {
        handler: Handler,
        _targets: Vec<Retained<AnyObject>>,
    }

    thread_local! {
        static REMOTE: RefCell<Option<Remote>> = RefCell::default();
        static ARTWORK: RefCell<Option<(String, Retained<MPMediaItemArtwork>)>> = RefCell::default();
        static LOADED: Cell<bool> = const { Cell::new(false) };
    }

    fn center() -> Retained<MPNowPlayingInfoCenter> {
        unsafe { MPNowPlayingInfoCenter::defaultCenter() }
    }

    fn number(value: f64) -> Retained<NSNumber> {
        NSNumber::new_f64(value)
    }

    fn state(playing: bool) -> MPNowPlayingPlaybackState {
        if playing {
            MPNowPlayingPlaybackState::Playing
        } else {
            MPNowPlayingPlaybackState::Paused
        }
    }

    fn artwork(url: &str, image: &Arc<Image>) -> Option<Retained<MPMediaItemArtwork>> {
        if let Some(artwork) = ARTWORK.with_borrow(|cached| {
            cached
                .as_ref()
                .filter(|(cached, _)| cached == url)
                .map(|(_, artwork)| artwork.clone())
        }) {
            return Some(artwork);
        }
        let picture = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(image.bytes()))?;
        let size = picture.size();
        let handler =
            RcBlock::new(move |_: CGSize| -> NonNull<NSImage> { NonNull::from(&*picture) });
        let artwork = unsafe {
            MPMediaItemArtwork::initWithBoundsSize_requestHandler(
                MPMediaItemArtwork::alloc(),
                size,
                &handler,
            )
        };
        ARTWORK.set(Some((url.to_owned(), artwork.clone())));
        Some(artwork)
    }

    pub fn publish(snapshot: &Snapshot, cover: Option<&Arc<Image>>) {
        let info: Retained<NSMutableDictionary<NSString, AnyObject>> = NSMutableDictionary::new();
        let text = |value: &str| NSString::from_str(value);
        unsafe {
            info.insert(MPMediaItemPropertyTitle, &*text(&snapshot.title));
            info.insert(MPMediaItemPropertyArtist, &*text(&snapshot.artist));
            if !snapshot.album.is_empty() {
                info.insert(MPMediaItemPropertyAlbumTitle, &*text(&snapshot.album));
            }
            if snapshot.duration_ms > 0 {
                info.insert(
                    MPMediaItemPropertyPlaybackDuration,
                    &*number(f64::from(snapshot.duration_ms) / 1000.),
                );
            }
            info.insert(
                MPNowPlayingInfoPropertyElapsedPlaybackTime,
                &*number(f64::from(snapshot.position_ms) / 1000.),
            );
            info.insert(
                MPNowPlayingInfoPropertyPlaybackRate,
                &*number(snapshot.rate()),
            );
            info.insert(MPNowPlayingInfoPropertyDefaultPlaybackRate, &*number(1.));
            info.insert(
                MPNowPlayingInfoPropertyMediaType,
                &*NSNumber::new_usize(MPNowPlayingInfoMediaType::Audio.0),
            );
            info.insert(
                MPNowPlayingInfoPropertyExternalContentIdentifier,
                &*text(&snapshot.uri),
            );
            match (snapshot.cover.as_deref(), cover) {
                (Some(url), Some(image)) => {
                    if let Some(artwork) = artwork(url, image) {
                        info.insert(MPMediaItemPropertyArtwork, &*artwork);
                    }
                }
                _ => ARTWORK.set(None),
            }
            let center = center();
            center.setNowPlayingInfo(Some(&info));
            center.setPlaybackState(state(snapshot.playing));
            MPRemoteCommandCenter::sharedCommandCenter()
                .changePlaybackPositionCommand()
                .setEnabled(snapshot.duration_ms > 0);
        }
        LOADED.set(true);
    }

    pub fn set_state(playing: bool) {
        unsafe { center().setPlaybackState(state(playing)) };
        LOADED.set(true);
    }

    pub fn clear() {
        let center = center();
        unsafe {
            center.setNowPlayingInfo(None);
            center.setPlaybackState(MPNowPlayingPlaybackState::Stopped);
        }
        ARTWORK.set(None);
        LOADED.set(false);
    }

    pub fn installed() -> bool {
        REMOTE.with_borrow(Option::is_some)
    }

    pub fn fire(intent: Intent) -> Status {
        let Some(handler) = REMOTE.with_borrow(|remote| remote.as_ref().map(|r| r.handler.clone()))
        else {
            return Status::NotInstalled;
        };
        if !accepts(&intent, LOADED.get()) {
            return Status::NothingPlaying;
        }
        handler(intent);
        Status::Done
    }

    fn reply(status: Status) -> MPRemoteCommandHandlerStatus {
        match status {
            Status::Done => MPRemoteCommandHandlerStatus::Success,
            Status::NothingPlaying | Status::NotInstalled => {
                MPRemoteCommandHandlerStatus::NoActionableNowPlayingItem
            }
        }
    }

    fn simple(
        intent: Intent,
    ) -> RcBlock<dyn Fn(NonNull<MPRemoteCommandEvent>) -> MPRemoteCommandHandlerStatus> {
        RcBlock::new(move |_: NonNull<MPRemoteCommandEvent>| reply(fire(intent.clone())))
    }

    pub fn install(handler: impl Fn(Intent) + 'static) {
        if installed() {
            return;
        }
        REMOTE.set(Some(Remote {
            handler: Rc::new(handler),
            _targets: Vec::new(),
        }));
        let center = unsafe { MPRemoteCommandCenter::sharedCommandCenter() };
        let mut targets = Vec::new();
        unsafe {
            targets.push(
                center
                    .playCommand()
                    .addTargetWithHandler(&simple(Intent::Play)),
            );
            targets.push(
                center
                    .pauseCommand()
                    .addTargetWithHandler(&simple(Intent::Pause)),
            );
            targets.push(
                center
                    .togglePlayPauseCommand()
                    .addTargetWithHandler(&simple(Intent::PlayPause)),
            );
            targets.push(
                center
                    .nextTrackCommand()
                    .addTargetWithHandler(&simple(Intent::Next)),
            );
            targets.push(
                center
                    .previousTrackCommand()
                    .addTargetWithHandler(&simple(Intent::Previous)),
            );
            let position = RcBlock::new(|event: NonNull<MPRemoteCommandEvent>| {
                let event = event.cast::<MPChangePlaybackPositionCommandEvent>();
                let seconds = event.as_ref().positionTime();
                reply(fire(Intent::SeekTo((seconds.max(0.) * 1000.) as u32)))
            });
            targets.push(
                center
                    .changePlaybackPositionCommand()
                    .addTargetWithHandler(&position),
            );
            for (command, step) in [
                (center.seekForwardCommand(), REMOTE_SEEK_MS),
                (center.seekBackwardCommand(), -REMOTE_SEEK_MS),
            ] {
                let seek = RcBlock::new(move |event: NonNull<MPRemoteCommandEvent>| {
                    let event = event.cast::<MPSeekCommandEvent>();
                    if event.as_ref().r#type() == MPSeekCommandEventType::BeginSeeking {
                        reply(fire(Intent::SeekBy(step)))
                    } else {
                        MPRemoteCommandHandlerStatus::Success
                    }
                });
                targets.push(command.addTargetWithHandler(&seek));
            }
            for command in [
                center.skipForwardCommand().into_super(),
                center.skipBackwardCommand().into_super(),
                center.likeCommand().into_super(),
                center.dislikeCommand().into_super(),
                center.bookmarkCommand().into_super(),
                center.changeRepeatModeCommand().into_super(),
                center.changeShuffleModeCommand().into_super(),
            ] {
                command.setEnabled(false);
            }
        }
        REMOTE.with_borrow_mut(|remote| {
            if let Some(remote) = remote.as_mut() {
                remote._targets = targets;
            }
        });
    }

    fn value(object: &AnyObject) -> Value {
        if let Some(text) = object.downcast_ref::<NSString>() {
            Value::String(text.to_string())
        } else if let Some(number) = object.downcast_ref::<NSNumber>() {
            json!(number.doubleValue())
        } else if object.downcast_ref::<MPMediaItemArtwork>().is_some() {
            Value::String("artwork".into())
        } else {
            Value::String(object.class().name().to_string_lossy().into_owned())
        }
    }

    pub fn dump() -> Value {
        let center = center();
        let playback = match unsafe { center.playbackState() } {
            MPNowPlayingPlaybackState::Playing => "playing",
            MPNowPlayingPlaybackState::Paused => "paused",
            MPNowPlayingPlaybackState::Stopped => "stopped",
            MPNowPlayingPlaybackState::Interrupted => "interrupted",
            _ => "unknown",
        };
        let mut info = Map::new();
        let mut has_artwork = false;
        if let Some(dictionary) = unsafe { center.nowPlayingInfo() } {
            let (keys, objects) = dictionary.to_vecs();
            for (key, object) in keys.iter().zip(objects.iter()) {
                has_artwork |= object.downcast_ref::<MPMediaItemArtwork>().is_some();
                info.insert(key.to_string(), value(object));
            }
        }
        let read = |key: &NSString| info.get(&key.to_string()).cloned().unwrap_or(Value::Null);
        unsafe {
            json!({
                "state": playback,
                "title": read(MPMediaItemPropertyTitle),
                "artist": read(MPMediaItemPropertyArtist),
                "album": read(MPMediaItemPropertyAlbumTitle),
                "duration": read(MPMediaItemPropertyPlaybackDuration),
                "elapsed": read(MPNowPlayingInfoPropertyElapsedPlaybackTime),
                "rate": read(MPNowPlayingInfoPropertyPlaybackRate),
                "uri": read(MPNowPlayingInfoPropertyExternalContentIdentifier),
                "has_artwork": has_artwork,
                "remote_installed": installed(),
                "seekable": MPRemoteCommandCenter::sharedCommandCenter()
                    .changePlaybackPositionCommand()
                    .isEnabled(),
                "info": Value::Object(info.clone()),
            })
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use std::{cell::RefCell, rc::Rc, sync::Arc};

    use gpui::Image;
    use serde_json::{Value, json};

    use super::{Snapshot, Status};
    use crate::app::intent::Intent;

    thread_local! {
        static HANDLER: RefCell<Option<Rc<dyn Fn(Intent)>>> = RefCell::default();
    }

    pub fn publish(_: &Snapshot, _: Option<&Arc<Image>>) {}

    pub fn set_state(_: bool) {}

    pub fn clear() {}

    pub fn installed() -> bool {
        HANDLER.with_borrow(Option::is_some)
    }

    pub fn install(handler: impl Fn(Intent) + 'static) {
        HANDLER.set(Some(Rc::new(handler)));
    }

    pub fn fire(intent: Intent) -> Status {
        match HANDLER.with_borrow(Clone::clone) {
            Some(handler) => {
                handler(intent);
                Status::Done
            }
            None => Status::NotInstalled,
        }
    }

    pub fn dump() -> Value {
        json!({ "state": "unavailable", "remote_installed": installed() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Snapshot {
        Snapshot {
            uri: "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration_ms: 200_000,
            position_ms: 10_000,
            playing: true,
            speed: 1.,
            cover: None,
        }
    }

    #[test]
    fn publishes_the_first_snapshot() {
        assert!(should_publish(None, &song(), 0));
    }

    #[test]
    fn stays_quiet_while_time_moves_as_expected() {
        let last = song();
        let mut next = song();
        next.position_ms = 11_000;
        assert!(!should_publish(Some(&last), &next, 1_000));
        next.position_ms = 12_400;
        assert!(!should_publish(Some(&last), &next, 1_000));
    }

    #[test]
    fn republishes_after_a_seek_or_drift() {
        let last = song();
        let mut next = song();
        next.position_ms = 60_000;
        assert!(should_publish(Some(&last), &next, 1_000));
        next.position_ms = 10_000;
        assert!(should_publish(Some(&last), &next, 2_000));
    }

    #[test]
    fn paused_time_does_not_move() {
        let mut last = song();
        last.playing = false;
        let next = last.clone();
        assert!(!should_publish(Some(&last), &next, 60_000));
    }

    #[test]
    fn republishes_on_any_item_change() {
        let last = song();
        let changes: [fn(&mut Snapshot); 7] = [
            |s| s.playing = false,
            |s| s.title = "Other".into(),
            |s| s.uri = "spotify:episode:512ojhOuo1ktJprKbVcKyQ".into(),
            |s| s.cover = Some("https://i.scdn.co/image/ab67616d00001e02".into()),
            |s| s.duration_ms = 0,
            |s| s.album = String::new(),
            |s| s.speed = 1.5,
        ];
        for change in changes {
            let mut next = song();
            change(&mut next);
            assert!(should_publish(Some(&last), &next, 0));
        }
    }

    #[test]
    fn faster_speed_moves_the_expected_position() {
        let mut last = song();
        last.speed = 2.;
        let mut next = last.clone();
        next.position_ms = 12_000;
        assert!(!should_publish(Some(&last), &next, 1_000));
        next.position_ms = 11_000;
        assert!(!should_publish(Some(&last), &next, 1_000));
        next.position_ms = 10_000;
        assert!(should_publish(Some(&last), &next, 1_000));
    }

    #[test]
    fn waiting_at_the_end_of_a_track_is_not_drift() {
        let mut last = song();
        last.position_ms = last.duration_ms;
        let next = last.clone();
        assert!(!should_publish(Some(&last), &next, 5_000));
    }

    #[test]
    fn only_play_works_without_a_track() {
        assert!(accepts(&Intent::Play, false));
        assert!(!accepts(&Intent::Pause, false));
        assert!(!accepts(&Intent::Next, false));
        assert!(accepts(&Intent::Next, true));
    }

    #[test]
    fn parses_remote_debug_commands() {
        assert_eq!(parse_remote("toggle"), Some(Intent::PlayPause));
        assert_eq!(parse_remote("seek 42000"), Some(Intent::SeekTo(42_000)));
        assert_eq!(parse_remote("seekby -15000"), Some(Intent::SeekBy(-15_000)));
        assert_eq!(parse_remote("forward"), Some(Intent::SeekBy(15_000)));
        assert_eq!(parse_remote("backward"), Some(Intent::SeekBy(-15_000)));
        assert_eq!(parse_remote("next now"), None);
        assert_eq!(parse_remote("seek soon"), None);
        assert_eq!(parse_remote("like"), None);
    }
}
