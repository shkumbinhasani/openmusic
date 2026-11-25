use gpui::{IntoElement, Styled, canvas};

use super::canvas::Want;

pub const NOW_PLAYING_KEY: &str = "now-playing";
const SETTLE_WITHIN: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Full,
    Dock,
    Hidden,
}

impl Cap {
    pub fn size(self) -> (f64, f64) {
        match self {
            Self::Full => (0., 0.),
            Self::Dock => (640., 360.),
            Self::Hidden => (320., 180.),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    Main,
    Canvas,
}

impl Feed {
    pub fn name(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Canvas => "canvas",
        }
    }
}

pub fn fit(width: f32, height: f32, aspect: f32, widest: f32) -> (f32, f32) {
    if width <= 0. || height <= 0. || !aspect.is_finite() || aspect <= 0. {
        return (0., 0.);
    }
    let width = width.min(widest);
    if width / height > aspect {
        (height * aspect, height)
    } else {
        (width, width / aspect)
    }
}

pub fn finish() -> impl IntoElement {
    canvas(|_, _, _| (), |_, _, _, _| native::screen_finish())
        .absolute()
        .size_0()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Loading,
    Playing,
    Paused,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Playing => "playing",
            Self::Paused => "paused",
        }
    }
}

const VALID: u32 = 1;
const UNBOUNDED: u32 = (1 << 2) | (1 << 3) | (1 << 4);
const LATEST_START: f64 = 0.9;

pub fn start_point(value: i64, timescale: i32, flags: u32, fraction: f64) -> Option<i64> {
    if flags & VALID == 0 || flags & UNBOUNDED != 0 || timescale <= 0 || value <= 0 {
        return None;
    }
    Some((value as f64 * fraction.clamp(0., LATEST_START)) as i64)
}

pub fn fraction() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    f64::from(nanos % 1000) / 1000.
}

pub use native::{
    cap_video, describe, describe_screen, dock_video, full_screen, install_artwork_click,
    keep_full_screen, leave_full_screen, place_screen, reduce_motion, screen_settled, status, sync,
    take_redraw, toggle_full_screen, window_visible,
};

#[cfg(target_os = "macos")]
mod native {
    use std::{
        cell::RefCell,
        time::{Duration, Instant},
    };

    use gpui::{Bounds, Pixels, Window};
    use objc2::{
        MainThreadMarker, MainThreadOnly, define_class, msg_send,
        rc::{Allocated, Retained},
        runtime::{AnyClass, AnyObject, NSObject},
        sel,
    };
    use objc2_app_kit::{
        NSEvent, NSFullScreenModeAllScreens, NSImageView, NSResponder, NSView,
        NSWindowOcclusionState, NSWindowOrderingMode, NSWorkspace,
    };
    use objc2_av_foundation::{
        AVLayerVideoGravity, AVLayerVideoGravityResizeAspect, AVLayerVideoGravityResizeAspectFill,
        AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVPlayerLayer, AVPlayerLooper,
        AVPlayerLooperStatus, AVQueuePlayer,
    };
    use objc2_core_foundation::CGSize;
    use objc2_foundation::{NSDictionary, NSNumber, NSObjectProtocol, NSString, NSURL};
    use objc2_quartz_core::{CAAutoresizingMask, CALayer};
    use spotify_client::player;

    use super::{Cap, Feed, NOW_PLAYING_KEY, SETTLE_WITHIN, Status, Want, fraction, start_point};
    use crate::{
        appkit::{self, dispatch},
        overlay::{self, Action},
    };

    const RETRY_FAILED: Duration = Duration::from_secs(30);

    struct Loop {
        player: Retained<AVQueuePlayer>,
        looper: Retained<AVPlayerLooper>,
        url: String,
        started: bool,
        since: Instant,
    }

    impl Loop {
        fn start(url: &str, random: bool, mtm: MainThreadMarker) -> Option<Self> {
            let address = NSURL::URLWithString(&NSString::from_str(url))?;
            unsafe {
                let item = AVPlayerItem::playerItemWithURL(&address, mtm);
                let player = AVQueuePlayer::new(mtm);
                player.setMuted(true);
                player.setAllowsExternalPlayback(false);
                player.setPreventsDisplaySleepDuringVideoPlayback(false);
                let looper = AVPlayerLooper::playerLooperWithPlayer_templateItem(&player, &item);
                Some(Self {
                    player,
                    looper,
                    url: url.to_owned(),
                    started: !random,
                    since: Instant::now(),
                })
            }
        }

        fn stale(&self, url: &str) -> bool {
            self.url != url
                || (unsafe { self.looper.status() } == AVPlayerLooperStatus::Failed
                    && self.since.elapsed() >= RETRY_FAILED)
        }

        fn run(&mut self, playing: bool) {
            unsafe {
                let moving = self.player.rate() != 0.;
                if playing && !moving {
                    self.player.play();
                } else if !playing && moving {
                    self.player.pause();
                }
            }
            if !self.started {
                self.jump();
            }
        }

        fn jump(&mut self) {
            unsafe {
                let Some(item) = self.player.currentItem() else {
                    return;
                };
                if item.status() != AVPlayerItemStatus::ReadyToPlay {
                    return;
                }
                let mut at = item.duration();
                if let Some(value) = start_point(at.value, at.timescale, at.flags.0, fraction()) {
                    at.value = value;
                    self.player.seekToTime(at);
                }
            }
            self.started = true;
        }

        fn stop(self) {
            unsafe {
                self.looper.disableLooping();
                self.player.pause();
            }
        }
    }

    #[derive(Default)]
    struct Surface {
        layer: Option<Retained<AVPlayerLayer>>,
        current: Option<Loop>,
    }

    impl Surface {
        fn layer(&mut self) -> Retained<AVPlayerLayer> {
            self.layer
                .get_or_insert_with(|| {
                    let layer = unsafe { AVPlayerLayer::new() };
                    if let Some(gravity) = unsafe { AVLayerVideoGravityResizeAspectFill } {
                        unsafe { layer.setVideoGravity(gravity) };
                    }
                    layer.setAutoresizingMask(
                        CAAutoresizingMask::LayerWidthSizable
                            | CAAutoresizingMask::LayerHeightSizable,
                    );
                    layer.setHidden(true);
                    layer
                })
                .clone()
        }

        fn attach(&mut self, art: &NSImageView) -> Option<Retained<AVPlayerLayer>> {
            let host = art.layer()?;
            let layer = self.layer();
            let parent = layer.superlayer();
            if !parent.is_some_and(|parent| std::ptr::eq::<CALayer>(&*parent, &*host)) {
                layer.removeFromSuperlayer();
                host.addSublayer(&layer);
            }
            let bounds = host.bounds();
            if layer.frame() != bounds {
                layer.setFrame(bounds);
            }
            if layer.contentsScale() != host.contentsScale() {
                layer.setContentsScale(host.contentsScale());
            }
            Some(layer)
        }

        fn stop(&mut self) {
            let Some(current) = self.current.take() else {
                return;
            };
            current.stop();
            if let Some(layer) = &self.layer {
                unsafe { layer.setPlayer(None) };
                layer.setHidden(true);
            }
        }

        fn show(&mut self, want: &Want, art: &NSImageView, mtm: MainThreadMarker) {
            let Some(layer) = self.attach(art) else {
                self.stop();
                return;
            };
            if self
                .current
                .as_ref()
                .is_none_or(|current| current.stale(&want.url))
            {
                self.stop();
                self.current = Loop::start(&want.url, want.random, mtm);
                if let Some(current) = &self.current {
                    unsafe { layer.setPlayer(Some(&current.player)) };
                }
            }
            let Some(current) = self.current.as_mut() else {
                return;
            };
            current.run(want.playing);
            let ready = unsafe { layer.isReadyForDisplay() };
            if layer.isHidden() == ready {
                layer.setHidden(!ready);
            }
        }

        fn status(&self) -> Status {
            let (Some(current), Some(layer)) = (&self.current, &self.layer) else {
                return Status::Idle;
            };
            if layer.isHidden() {
                return Status::Loading;
            }
            if unsafe { current.player.rate() } == 0. {
                Status::Paused
            } else {
                Status::Playing
            }
        }
    }

    thread_local! {
        static SURFACE: RefCell<Surface> = RefCell::default();
        static SCREEN: RefCell<Option<Screen>> = const { RefCell::new(None) };
        static DOCK: RefCell<Option<Retained<AVPlayerLayer>>> = const { RefCell::new(None) };
        static CLICK: RefCell<Option<Click>> = const { RefCell::new(None) };
    }

    const ESCAPE: u16 = 53;
    const SPACE: u16 = 49;

    fn canvas_player() -> Option<Retained<AVQueuePlayer>> {
        SURFACE
            .try_with(|surface| {
                surface
                    .try_borrow()
                    .ok()
                    .and_then(|surface| surface.current.as_ref().map(|loop_| loop_.player.clone()))
            })
            .ok()
            .flatten()
    }

    fn feed_player(feed: Feed, mtm: MainThreadMarker) -> Option<Retained<AVQueuePlayer>> {
        match feed {
            Feed::Main => player::main_player(mtm),
            Feed::Canvas => canvas_player(),
        }
    }

    fn same_player(layer: &AVPlayerLayer, player: Option<&AVQueuePlayer>) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return true;
        };
        let current = unsafe { layer.player(mtm) };
        match (current.as_deref(), player) {
            (None, None) => true,
            (Some(current), Some(player)) => std::ptr::eq(current, player as &AVPlayer),
            _ => false,
        }
    }

    fn bind(layer: &AVPlayerLayer, player: Option<&AVQueuePlayer>) {
        if !same_player(layer, player) {
            unsafe { layer.setPlayer(player.map(|player| player as &AVPlayer)) };
        }
    }

    fn gravity(feed: Feed) -> Option<&'static AVLayerVideoGravity> {
        unsafe {
            match feed {
                Feed::Main => AVLayerVideoGravityResizeAspect,
                Feed::Canvas => AVLayerVideoGravityResizeAspectFill,
            }
        }
    }

    define_class!(
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OpenMusicVideoScreen"]
        struct ScreenView;

        unsafe impl NSObjectProtocol for ScreenView {}

        impl ScreenView {
            #[unsafe(method(acceptsFirstResponder))]
            fn accepts_first_responder(&self) -> bool {
                true
            }

            #[unsafe(method(keyDown:))]
            fn key_down(&self, event: &NSEvent) {
                match event.keyCode() {
                    ESCAPE if self.isInFullScreenMode() => leave(self),
                    SPACE => dispatch(Action::Press("toggle-playback")),
                    _ => unsafe {
                        let _: () = msg_send![super(self), keyDown: event];
                    },
                }
            }

            #[unsafe(method(cancelOperation:))]
            fn cancel_operation(&self, _sender: Option<&AnyObject>) {
                if self.isInFullScreenMode() {
                    leave(self);
                }
            }

            #[unsafe(method(mouseDown:))]
            fn mouse_down(&self, event: &NSEvent) {
                if event.clickCount() == 2 {
                    flip(self);
                }
            }

            #[unsafe(method(flipFullScreen:))]
            fn flip_full_screen(&self, _sender: Option<&AnyObject>) {
                flip(self);
            }
        }
    );

    impl ScreenView {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    fn flip(view: &ScreenView) {
        if view.isInFullScreenMode() {
            leave(view);
        } else {
            enter(view);
        }
    }

    fn enter(view: &ScreenView) {
        let Some(window) = view.window() else {
            return;
        };
        let display: Option<Retained<AnyObject>> = unsafe { msg_send![&*window, screen] };
        let all = NSNumber::new_bool(false);
        let key: &NSString = unsafe { NSFullScreenModeAllScreens };
        let options = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[&*all]);
        let entered: bool = unsafe {
            msg_send![view, enterFullScreenMode: display.as_deref(), withOptions: &*options]
        };
        if entered && let Some(window) = view.window() {
            let responder: &NSResponder = view;
            window.makeFirstResponder(Some(responder));
        }
    }

    fn leave(view: &ScreenView) {
        unsafe { view.exitFullScreenModeWithOptions(None) };
        let _ = SCREEN.with(|cell| {
            cell.try_borrow_mut().map(|mut slot| {
                if let Some(screen) = slot.as_mut() {
                    screen.redraw = true;
                }
            })
        });
    }

    struct Screen {
        view: Retained<ScreenView>,
        layer: Retained<AVPlayerLayer>,
        feed: Option<Feed>,
        placed: bool,
        redraw: bool,
        bound_at: Instant,
    }

    impl Screen {
        fn new(mtm: MainThreadMarker) -> Self {
            let view = ScreenView::new(mtm);
            view.setWantsLayer(true);
            let layer = unsafe { AVPlayerLayer::new() };
            layer.setAutoresizingMask(
                CAAutoresizingMask::LayerWidthSizable | CAAutoresizingMask::LayerHeightSizable,
            );
            layer.setHidden(true);
            if let Some(host) = view.layer() {
                host.setMasksToBounds(true);
                host.addSublayer(&layer);
            }
            view.setHidden(true);
            Self {
                view,
                layer,
                feed: None,
                placed: false,
                redraw: false,
                bound_at: Instant::now(),
            }
        }

        fn fill(&self) {
            if let Some(host) = self.view.layer()
                && self.layer.frame() != host.bounds()
            {
                self.layer.setFrame(host.bounds());
                if self.layer.contentsScale() != host.contentsScale() {
                    self.layer.setContentsScale(host.contentsScale());
                }
            }
        }

        fn round(&self, radius: f32) {
            if let Some(host) = self.view.layer()
                && host.cornerRadius() != f64::from(radius)
            {
                host.setCornerRadius(f64::from(radius));
            }
        }

        fn show(&mut self, feed: Feed, mtm: MainThreadMarker) {
            self.fill();
            if self.feed != Some(feed) {
                if let Some(gravity) = gravity(feed) {
                    unsafe { self.layer.setVideoGravity(gravity) };
                }
                self.feed = Some(feed);
            }
            let player = feed_player(feed, mtm);
            if !same_player(&self.layer, player.as_deref()) {
                bind(&self.layer, player.as_deref());
                self.bound_at = Instant::now();
            }
            let ready = unsafe { self.layer.isReadyForDisplay() };
            if self.layer.isHidden() == ready {
                self.layer.setHidden(!ready);
            }
        }

        fn release(&mut self) {
            self.view.setHidden(true);
            self.layer.setHidden(true);
            bind(&self.layer, None);
            self.feed = None;
        }
    }

    fn keep_low(container: &NSView, host: &NSView, view: &NSView) {
        let subviews = container.subviews();
        let position = |target: &NSView| {
            (0..subviews.count())
                .find(|index| std::ptr::eq(&*subviews.objectAtIndex(*index), target))
        };
        let placed = match (position(host), position(view)) {
            (Some(host), Some(view)) => view == host + 1,
            _ => false,
        };
        if !placed {
            container.addSubview_positioned_relativeTo(
                view,
                NSWindowOrderingMode::Above,
                Some(host),
            );
        }
    }

    pub fn place_screen(bounds: Bounds<Pixels>, feed: Feed, radius: f32, window: &Window) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let mask = window.content_mask().bounds;
        let shown = mask.contains(&bounds.origin) && mask.contains(&bounds.bottom_right());
        let (Some(container), Some(host)) = (appkit::container(), crate::glass::host()) else {
            return;
        };
        SCREEN.with(|cell| {
            let Ok(mut slot) = cell.try_borrow_mut() else {
                return;
            };
            let screen = slot.get_or_insert_with(|| Screen::new(mtm));
            screen.placed = true;
            if screen.view.isInFullScreenMode() {
                screen.round(0.);
                screen.show(feed, mtm);
                return;
            }
            if !shown {
                screen.placed = false;
                screen.release();
                return;
            }
            keep_low(&container, &host, &screen.view);
            screen.view.setFrame(appkit::frame(&container, bounds));
            screen.round(radius);
            screen.view.setHidden(false);
            screen.show(feed, mtm);
        });
    }

    pub fn screen_finish() {
        SCREEN.with(|cell| {
            let Ok(mut slot) = cell.try_borrow_mut() else {
                return;
            };
            let Some(screen) = slot.as_mut() else {
                return;
            };
            if !screen.placed && !screen.view.isInFullScreenMode() {
                screen.release();
            }
            screen.placed = false;
        });
    }

    pub fn toggle_full_screen() -> bool {
        SCREEN.with(|cell| {
            let Ok(slot) = cell.try_borrow() else {
                return false;
            };
            let Some(screen) = slot.as_ref() else {
                return false;
            };
            if screen.view.isHidden() && !screen.view.isInFullScreenMode() {
                return false;
            }
            unsafe {
                let _: () = msg_send![
                    &*screen.view,
                    performSelector: sel!(flipFullScreen:),
                    withObject: None::<&AnyObject>,
                    afterDelay: 0.0f64
                ];
            }
            true
        })
    }

    pub fn keep_full_screen() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        SCREEN.with(|cell| {
            let Ok(mut slot) = cell.try_borrow_mut() else {
                return;
            };
            if let Some(screen) = slot.as_mut()
                && screen.view.isInFullScreenMode()
                && let Some(feed) = screen.feed
            {
                screen.round(0.);
                screen.show(feed, mtm);
            }
        });
    }

    pub fn full_screen() -> bool {
        SCREEN.with(|cell| {
            cell.try_borrow().ok().is_some_and(|slot| {
                slot.as_ref()
                    .is_some_and(|screen| screen.view.isInFullScreenMode())
            })
        })
    }

    pub fn leave_full_screen() {
        let view = SCREEN.with(|cell| {
            cell.try_borrow()
                .ok()
                .and_then(|slot| slot.as_ref().map(|screen| screen.view.clone()))
        });
        if let Some(view) = view.filter(|view| view.isInFullScreenMode()) {
            leave(&view);
        }
    }

    pub fn screen_settled() -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return true;
        };
        SCREEN.with(|cell| {
            let Ok(slot) = cell.try_borrow() else {
                return true;
            };
            let Some(screen) = slot.as_ref() else {
                return true;
            };
            let Some(feed) = screen.feed.filter(|_| !screen.view.isHidden()) else {
                return true;
            };
            let player = feed_player(feed, mtm);
            if !same_player(&screen.layer, player.as_deref()) {
                return false;
            }
            player.is_none()
                || unsafe { screen.layer.isReadyForDisplay() }
                || screen.bound_at.elapsed() >= SETTLE_WITHIN
        })
    }

    pub fn take_redraw() -> bool {
        SCREEN.with(|cell| {
            cell.try_borrow_mut().ok().is_some_and(|mut slot| {
                slot.as_mut()
                    .is_some_and(|screen| std::mem::take(&mut screen.redraw))
            })
        })
    }

    pub fn describe_screen() -> String {
        SCREEN.with(|cell| {
            let Ok(slot) = cell.try_borrow() else {
                return "screen busy".to_owned();
            };
            let Some(screen) = slot.as_ref() else {
                return "screen none".to_owned();
            };
            let frame = screen.view.frame();
            format!(
                "screen feed={} hidden={} layer_hidden={} ready={} full={} frame={}x{} player={}",
                screen.feed.map_or("none", Feed::name),
                screen.view.isHidden(),
                screen.layer.isHidden(),
                unsafe { screen.layer.isReadyForDisplay() },
                screen.view.isInFullScreenMode(),
                frame.size.width,
                frame.size.height,
                MainThreadMarker::new()
                    .is_some_and(|mtm| unsafe { screen.layer.player(mtm) }.is_some()),
            )
        })
    }

    pub fn dock_video(on: bool) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let art = on.then(overlay::dock_artwork).flatten();
        DOCK.with(|cell| {
            let Ok(mut slot) = cell.try_borrow_mut() else {
                return;
            };
            let Some(art) = art else {
                if let Some(layer) = slot.as_ref() {
                    layer.setHidden(true);
                    bind(layer, None);
                }
                return;
            };
            let Some(host) = art.layer() else {
                return;
            };
            let layer = slot
                .get_or_insert_with(|| {
                    let layer = unsafe { AVPlayerLayer::new() };
                    if let Some(gravity) = gravity(Feed::Canvas) {
                        unsafe { layer.setVideoGravity(gravity) };
                    }
                    layer.setAutoresizingMask(
                        CAAutoresizingMask::LayerWidthSizable
                            | CAAutoresizingMask::LayerHeightSizable,
                    );
                    layer.setHidden(true);
                    layer
                })
                .clone();
            if !layer
                .superlayer()
                .is_some_and(|parent| std::ptr::eq::<CALayer>(&*parent, &*host))
            {
                layer.removeFromSuperlayer();
                host.addSublayer(&layer);
            }
            if layer.frame() != host.bounds() {
                layer.setFrame(host.bounds());
            }
            if layer.contentsScale() != host.contentsScale() {
                layer.setContentsScale(host.contentsScale());
            }
            bind(&layer, player::main_player(mtm).as_deref());
            let ready = unsafe { layer.isReadyForDisplay() };
            if layer.isHidden() == ready {
                layer.setHidden(!ready);
            }
        });
    }

    pub fn cap_video(cap: Cap) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(item) =
            player::main_player(mtm).and_then(|player| unsafe { player.currentItem() })
        else {
            return;
        };
        let (width, height) = cap.size();
        let size = CGSize::new(width, height);
        unsafe {
            if item.preferredMaximumResolution() != size {
                item.setPreferredMaximumResolution(size);
            }
        }
    }

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OpenMusicArtworkClick"]
        struct ArtworkClick;

        unsafe impl NSObjectProtocol for ArtworkClick {}

        impl ArtworkClick {
            #[unsafe(method(clicked:))]
            fn clicked(&self, _sender: Option<&AnyObject>) {
                dispatch(Action::Press(NOW_PLAYING_KEY));
            }
        }
    );

    impl ArtworkClick {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    struct Click {
        _target: Retained<ArtworkClick>,
        _recognizer: Retained<AnyObject>,
        art: Retained<NSImageView>,
    }

    pub fn install_artwork_click() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(art) = overlay::dock_artwork() else {
            return;
        };
        CLICK.with(|cell| {
            let Ok(mut slot) = cell.try_borrow_mut() else {
                return;
            };
            if slot.as_ref().is_some_and(|click| click.art == art) {
                return;
            }
            let Some(class) = AnyClass::get(c"NSClickGestureRecognizer") else {
                return;
            };
            let target = ArtworkClick::new(mtm);
            let recognizer: Option<Retained<AnyObject>> = unsafe {
                let allocated: Allocated<AnyObject> = msg_send![class, alloc];
                msg_send![allocated, initWithTarget: &*target, action: sel!(clicked:)]
            };
            let Some(recognizer) = recognizer else {
                return;
            };
            unsafe {
                let _: () = msg_send![&*art, addGestureRecognizer: &*recognizer];
            }
            *slot = Some(Click {
                _target: target,
                _recognizer: recognizer,
                art,
            });
        });
    }

    pub fn sync(want: Option<&Want>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let art = want.and_then(|_| overlay::dock_artwork());
        SURFACE.with_borrow_mut(|surface| match (want, art) {
            (Some(want), Some(art)) => surface.show(want, &art, mtm),
            _ => surface.stop(),
        });
    }

    pub fn status() -> Status {
        SURFACE.with_borrow(Surface::status)
    }

    pub fn reduce_motion() -> bool {
        NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
    }

    pub fn window_visible() -> bool {
        let Some(host) = crate::glass::host() else {
            return true;
        };
        host.window().is_some_and(|window| {
            window
                .occlusionState()
                .contains(NSWindowOcclusionState::Visible)
        })
    }

    pub fn describe() -> String {
        SURFACE.with_borrow(|surface| {
            let Some(layer) = &surface.layer else {
                return "video layer=none".to_owned();
            };
            let attached = layer.superlayer().is_some();
            let frame = layer.frame();
            let mut line = format!(
                "video status={} attached={attached} hidden={} ready={} frame={}x{}",
                surface.status().name(),
                layer.isHidden(),
                unsafe { layer.isReadyForDisplay() },
                frame.size.width,
                frame.size.height,
            );
            if let Some(current) = &surface.current {
                let (rate, looper, loops, muted) = unsafe {
                    (
                        current.player.rate(),
                        current.looper.status().0,
                        current.looper.loopCount(),
                        current.player.isMuted(),
                    )
                };
                line.push_str(&format!(
                    " rate={rate} muted={muted} looper={looper} loops={loops} started={} url={}",
                    current.started, current.url
                ));
            }
            line
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use gpui::{Bounds, Pixels, Window};

    use super::{Cap, Feed, Status, Want};

    pub fn sync(_want: Option<&Want>) {}

    pub fn place_screen(_bounds: Bounds<Pixels>, _feed: Feed, _radius: f32, _window: &Window) {}

    pub fn screen_finish() {}

    pub fn toggle_full_screen() -> bool {
        false
    }

    pub fn full_screen() -> bool {
        false
    }

    pub fn leave_full_screen() {}

    pub fn take_redraw() -> bool {
        false
    }

    pub fn screen_settled() -> bool {
        true
    }

    pub fn describe_screen() -> String {
        "screen unsupported".to_owned()
    }

    pub fn dock_video(_on: bool) {}

    pub fn cap_video(_cap: Cap) {}

    pub fn keep_full_screen() {}

    pub fn install_artwork_click() {}

    pub fn status() -> Status {
        Status::Idle
    }

    pub fn reduce_motion() -> bool {
        false
    }

    pub fn window_visible() -> bool {
        true
    }

    pub fn describe() -> String {
        "video unsupported".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_the_picture_by_where_it_shows() {
        assert_eq!(Cap::Full.size(), (0., 0.));
        assert_eq!(Cap::Dock.size(), (640., 360.));
        assert_eq!(Cap::Hidden.size(), (320., 180.));
    }

    #[test]
    fn fits_the_picture_inside_the_space() {
        assert_eq!(fit(1600., 900., 16. / 9., 960.), (960., 540.));
        assert_eq!(fit(800., 300., 16. / 9., 960.), (300. * 16. / 9., 300.));
        let (width, height) = fit(400., 900., 9. / 16., 360.);
        assert!((width - 360.).abs() < 1e-3 && (height - 640.).abs() < 1e-3);
        assert_eq!(fit(500., 500., 1., 2000.), (500., 500.));
        assert_eq!(fit(0., 500., 1., 2000.), (0., 0.));
        assert_eq!(fit(500., 500., f32::NAN, 2000.), (0., 0.));
        assert_eq!(Feed::Main.name(), "main");
        assert_eq!(Feed::Canvas.name(), "canvas");
    }

    #[test]
    fn starts_somewhere_inside_the_first_loop() {
        assert_eq!(start_point(7000, 1000, VALID, 0.5), Some(3500));
        assert_eq!(start_point(7000, 1000, VALID, 0.), Some(0));
        assert_eq!(start_point(7000, 1000, VALID, 1.), Some(6300));
        assert_eq!(start_point(7000, 1000, 0, 0.5), None);
        assert_eq!(start_point(0, 1000, VALID, 0.5), None);
        assert_eq!(start_point(7000, 0, VALID, 0.5), None);
        assert_eq!(start_point(7000, 1000, VALID | (1 << 4), 0.5), None);
        assert!((0. ..1.).contains(&fraction()));
    }
}
