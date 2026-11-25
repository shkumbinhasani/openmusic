use std::time::{Duration, Instant};

pub use native::{authorize, available, install, post, refresh_status, set_app_active, status};

pub const DELAY: Duration = Duration::from_secs(1);
pub const OPEN: &str = "system-notification-open";
pub const STATUS: &str = "system-notification-status";
pub const IDENTIFIER: &str = "openmusic.now-playing";
pub const THREAD: &str = "now-playing";
pub const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.Notifications-Settings.extension?id=io.openmusic.player";
const CACHED_FILES: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authorization {
    Unknown,
    NotDetermined,
    Denied,
    Authorized,
    Unavailable,
}

impl Authorization {
    pub fn name(self) -> &'static str {
        match self {
            Authorization::Unknown => "unknown",
            Authorization::NotDetermined => "not-determined",
            Authorization::Denied => "denied",
            Authorization::Authorized => "authorized",
            Authorization::Unavailable => "unavailable",
        }
    }

    fn code(self) -> u8 {
        match self {
            Authorization::Unknown => 0,
            Authorization::NotDetermined => 1,
            Authorization::Denied => 2,
            Authorization::Authorized => 3,
            Authorization::Unavailable => 4,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            1 => Authorization::NotDetermined,
            2 => Authorization::Denied,
            3 => Authorization::Authorized,
            4 => Authorization::Unavailable,
            _ => Authorization::Unknown,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackNote {
    pub uri: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub cover: Option<(Vec<u8>, &'static str)>,
}

pub fn notifiable(uri: &str) -> bool {
    ["spotify:track:", "spotify:episode:", "spotify:local:"]
        .iter()
        .any(|prefix| uri.starts_with(prefix))
}

pub fn cover_extension(format: gpui::ImageFormat) -> Option<&'static str> {
    match format {
        gpui::ImageFormat::Jpeg => Some("jpg"),
        gpui::ImageFormat::Png => Some("png"),
        gpui::ImageFormat::Gif => Some("gif"),
        _ => None,
    }
}

pub fn cover_file(index: usize, extension: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join("Library/Caches/OpenMusic")
            .join(format!("notification-{}.{extension}", index % CACHED_FILES)),
    )
}

#[derive(Clone, Copy, Debug)]
pub struct Seen<'a> {
    pub uri: &'a str,
    pub title: &'a str,
    pub playing: bool,
}

#[derive(Debug, Default)]
pub struct Watch {
    last: Option<String>,
    armed: bool,
    due: Option<(String, Instant)>,
}

impl Watch {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn due(&self) -> Option<&str> {
        self.due.as_ref().map(|(uri, _)| uri.as_str())
    }

    pub fn observe(&mut self, seen: Option<Seen>, enabled: bool, now: Instant) -> Option<String> {
        let uri = seen.map(|seen| seen.uri);
        if uri != self.last.as_deref() {
            self.due = match seen {
                Some(seen) if self.armed && enabled && seen.playing && notifiable(seen.uri) => {
                    Some((seen.uri.to_owned(), now + DELAY))
                }
                _ => None,
            };
            self.last = uri.map(str::to_owned);
            self.armed |= seen.is_some();
        }
        let at = self.due.as_ref()?.1;
        let Some(seen) = seen.filter(|seen| enabled && seen.playing) else {
            self.due = None;
            return None;
        };
        if now < at || seen.title.is_empty() {
            return None;
        }
        self.due.take().map(|(uri, _)| uri)
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::{
        cell::RefCell,
        ptr::NonNull,
        sync::{
            OnceLock,
            atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
        },
    };

    use block2::RcBlock;
    use objc2::{
        AllocAnyThread, define_class, msg_send,
        rc::Retained,
        runtime::{Bool, NSObject, ProtocolObject},
    };
    use objc2_foundation::{
        NSArray, NSBundle, NSError, NSObjectProtocol, NSOperationQueue, NSString, NSURL,
    };
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
        UNNotification, UNNotificationAttachment, UNNotificationPresentationOptions,
        UNNotificationRequest, UNNotificationResponse, UNNotificationSettings,
        UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };

    use super::{Authorization, IDENTIFIER, OPEN, STATUS, THREAD, TrackNote, cover_file};
    use crate::{appkit::dispatch, overlay::Action};

    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    static APP_ACTIVE: AtomicBool = AtomicBool::new(true);
    static FORCE: AtomicBool = AtomicBool::new(false);
    static AUTHORIZATION: AtomicU8 = AtomicU8::new(0);
    static FILES: AtomicUsize = AtomicUsize::new(0);

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "OpenMusicNotificationDelegate"]
        struct Delegate;

        unsafe impl NSObjectProtocol for Delegate {}

        unsafe impl UNUserNotificationCenterDelegate for Delegate {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                let forced = FORCE.swap(false, Ordering::Relaxed);
                let options = if APP_ACTIVE.load(Ordering::Relaxed) && !forced {
                    UNNotificationPresentationOptions::empty()
                } else {
                    UNNotificationPresentationOptions::Banner
                        | UNNotificationPresentationOptions::List
                };
                handler.call((options,));
            }

            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                _response: &UNNotificationResponse,
                handler: &block2::DynBlock<dyn Fn()>,
            ) {
                on_main(|| dispatch(Action::Press(OPEN)));
                handler.call(());
            }
        }
    );

    impl Delegate {
        fn new() -> Retained<Self> {
            let this = Self::alloc().set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        static DELEGATE: RefCell<Option<Retained<Delegate>>> = RefCell::default();
    }

    fn on_main(run: impl Fn() + 'static) {
        let block = RcBlock::new(run);
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
    }

    fn center() -> Retained<UNUserNotificationCenter> {
        UNUserNotificationCenter::currentNotificationCenter()
    }

    pub fn available() -> bool {
        *AVAILABLE.get_or_init(|| {
            let bundle = NSBundle::mainBundle();
            bundle.bundleIdentifier().is_some() && bundle.bundlePath().to_string().ends_with(".app")
        })
    }

    pub fn install() {
        if !available() || DELEGATE.with_borrow(Option::is_some) {
            return;
        }
        let delegate = Delegate::new();
        center().setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        DELEGATE.set(Some(delegate));
        refresh_status();
    }

    fn store(status: UNAuthorizationStatus) {
        let status = if status == UNAuthorizationStatus::Denied {
            Authorization::Denied
        } else if status == UNAuthorizationStatus::NotDetermined {
            Authorization::NotDetermined
        } else {
            Authorization::Authorized
        };
        AUTHORIZATION.store(status.code(), Ordering::Relaxed);
    }

    pub fn status() -> Authorization {
        if available() {
            Authorization::from_code(AUTHORIZATION.load(Ordering::Relaxed))
        } else {
            Authorization::Unavailable
        }
    }

    pub fn refresh_status() {
        if !available() {
            return;
        }
        let done = RcBlock::new(|settings: NonNull<UNNotificationSettings>| {
            store(unsafe { settings.as_ref() }.authorizationStatus());
            on_main(|| dispatch(Action::Press(STATUS)));
        });
        center().getNotificationSettingsWithCompletionHandler(&done);
    }

    pub fn authorize() {
        if !available() {
            return;
        }
        let done = RcBlock::new(|_: Bool, _: *mut NSError| refresh_status());
        center().requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert,
            &done,
        );
    }

    pub fn set_app_active(active: bool) {
        APP_ACTIVE.store(active, Ordering::Relaxed);
    }

    fn attachment(bytes: &[u8], extension: &str) -> Option<Retained<UNNotificationAttachment>> {
        let path = cover_file(FILES.fetch_add(1, Ordering::Relaxed), extension)?;
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, bytes).ok()?;
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        unsafe {
            UNNotificationAttachment::attachmentWithIdentifier_URL_options_error(
                &NSString::from_str("cover"),
                &url,
                None,
            )
        }
        .ok()
    }

    pub fn post(note: &TrackNote, force: bool) -> bool {
        if !available() || note.title.is_empty() {
            return false;
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&note.title));
        if !note.artist.is_empty() {
            content.setSubtitle(&NSString::from_str(&note.artist));
        }
        if !note.album.is_empty() {
            content.setBody(&NSString::from_str(&note.album));
        }
        content.setThreadIdentifier(&NSString::from_str(THREAD));
        if let Some(attachment) = note
            .cover
            .as_ref()
            .and_then(|(bytes, extension)| attachment(bytes, extension))
        {
            content.setAttachments(&NSArray::from_retained_slice(&[attachment]));
        }
        let identifier = NSString::from_str(IDENTIFIER);
        let center = center();
        center.removeDeliveredNotificationsWithIdentifiers(&NSArray::from_slice(&[&*identifier]));
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &identifier,
            &content,
            None,
        );
        FORCE.store(force, Ordering::Relaxed);
        center.addNotificationRequest_withCompletionHandler(&request, None);
        true
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::{Authorization, TrackNote};

    pub fn available() -> bool {
        false
    }

    pub fn install() {}

    pub fn status() -> Authorization {
        Authorization::Unavailable
    }

    pub fn refresh_status() {}

    pub fn authorize() {}

    pub fn set_app_active(_: bool) {}

    pub fn post(_: &TrackNote, _: bool) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = "spotify:track:4uLU6hMCjMI75M1A2tKUQC";
    const SECOND: &str = "spotify:track:1b5KESuOMQomF3wntoLAsk";
    const THIRD: &str = "spotify:episode:512ojhOuo1ktJprKbVcKyQ";

    fn seen(uri: &str) -> Option<Seen<'_>> {
        Some(Seen {
            uri,
            title: "Title",
            playing: true,
        })
    }

    fn armed(start: Instant) -> Watch {
        let mut watch = Watch::default();
        assert_eq!(watch.observe(seen(FIRST), true, start), None);
        watch
    }

    #[test]
    fn the_first_track_after_connecting_is_quiet() {
        let start = Instant::now();
        let mut watch = Watch::default();
        assert_eq!(watch.observe(None, true, start), None);
        assert_eq!(watch.observe(seen(FIRST), true, start), None);
        assert_eq!(watch.due(), None);
        assert_eq!(watch.observe(seen(FIRST), true, start + DELAY * 3), None);
    }

    #[test]
    fn a_change_posts_once_after_the_delay() {
        let start = Instant::now();
        let mut watch = armed(start);
        assert_eq!(watch.observe(seen(SECOND), true, start), None);
        assert_eq!(watch.due(), Some(SECOND));
        let half = start + DELAY / 2;
        assert_eq!(watch.observe(seen(SECOND), true, half), None);
        let later = start + DELAY;
        assert_eq!(
            watch.observe(seen(SECOND), true, later),
            Some(SECOND.into())
        );
        assert_eq!(watch.observe(seen(SECOND), true, later + DELAY), None);
    }

    #[test]
    fn skipping_fast_posts_only_the_last_track() {
        let start = Instant::now();
        let mut watch = armed(start);
        let step = Duration::from_millis(300);
        for (index, uri) in [SECOND, THIRD, SECOND, THIRD].into_iter().enumerate() {
            assert_eq!(
                watch.observe(seen(uri), true, start + step * index as u32),
                None
            );
        }
        let end = start + step * 3 + DELAY;
        assert_eq!(watch.observe(seen(THIRD), true, end), Some(THIRD.into()));
    }

    #[test]
    fn waits_for_the_title() {
        let start = Instant::now();
        let mut watch = armed(start);
        let loading = Some(Seen {
            uri: SECOND,
            title: "",
            playing: true,
        });
        assert_eq!(watch.observe(loading, true, start), None);
        assert_eq!(watch.observe(loading, true, start + DELAY * 2), None);
        assert_eq!(
            watch.observe(seen(SECOND), true, start + DELAY * 2),
            Some(SECOND.into())
        );
    }

    #[test]
    fn stays_quiet_when_off_paused_or_an_ad() {
        let start = Instant::now();
        let mut watch = armed(start);
        assert_eq!(watch.observe(seen(SECOND), false, start), None);
        assert_eq!(watch.observe(seen(SECOND), true, start + DELAY), None);

        let mut watch = armed(start);
        let paused = Some(Seen {
            uri: SECOND,
            title: "Title",
            playing: false,
        });
        assert_eq!(watch.observe(paused, true, start), None);
        assert_eq!(watch.observe(paused, true, start + DELAY), None);

        let mut watch = armed(start);
        assert_eq!(watch.observe(seen(SECOND), true, start), None);
        assert_eq!(watch.observe(paused, true, start + DELAY / 2), None);
        assert_eq!(watch.observe(seen(SECOND), true, start + DELAY), None);

        let mut watch = armed(start);
        let ad = "spotify:ad:000000000000000000000000";
        assert_eq!(watch.observe(seen(ad), true, start), None);
        assert_eq!(watch.observe(seen(ad), true, start + DELAY), None);
    }

    #[test]
    fn turning_off_drops_a_pending_post() {
        let start = Instant::now();
        let mut watch = armed(start);
        assert_eq!(watch.observe(seen(SECOND), true, start), None);
        assert_eq!(watch.observe(seen(SECOND), false, start + DELAY), None);
        assert_eq!(watch.due(), None);
    }

    #[test]
    fn reset_quiets_the_next_track() {
        let start = Instant::now();
        let mut watch = armed(start);
        watch.reset();
        assert_eq!(watch.observe(seen(SECOND), true, start), None);
        assert_eq!(watch.due(), None);
    }

    #[test]
    fn authorization_round_trips_and_files_rotate() {
        for status in [
            Authorization::Unknown,
            Authorization::NotDetermined,
            Authorization::Denied,
            Authorization::Authorized,
            Authorization::Unavailable,
        ] {
            assert_eq!(Authorization::from_code(status.code()), status);
        }
        let first = cover_file(0, "jpg").unwrap();
        assert!(first.ends_with("Library/Caches/OpenMusic/notification-0.jpg"));
        assert_eq!(cover_file(4, "jpg"), Some(first));
        assert_eq!(cover_extension(gpui::ImageFormat::Webp), None);
        assert_eq!(cover_extension(gpui::ImageFormat::Jpeg), Some("jpg"));
        assert!(notifiable("spotify:local:Artist:Album:Title:200"));
    }
}
