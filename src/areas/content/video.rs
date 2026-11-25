use super::canvas::Want;

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

pub use native::{describe, reduce_motion, status, sync, window_visible};

#[cfg(target_os = "macos")]
mod native {
    use std::{
        cell::RefCell,
        time::{Duration, Instant},
    };

    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{NSImageView, NSWindowOcclusionState, NSWorkspace};
    use objc2_av_foundation::{
        AVLayerVideoGravityResizeAspectFill, AVPlayerItem, AVPlayerItemStatus, AVPlayerLayer,
        AVPlayerLooper, AVPlayerLooperStatus, AVQueuePlayer,
    };
    use objc2_foundation::{NSString, NSURL};
    use objc2_quartz_core::{CAAutoresizingMask, CALayer};

    use super::{Status, Want, fraction, start_point};
    use crate::overlay;

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
    use super::{Status, Want};

    pub fn sync(_want: Option<&Want>) {}

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
