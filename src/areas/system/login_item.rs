pub use native::{
    available, hide_at_launch, launched_at_login, open_settings, record_launch, set, status,
};

pub const CHOICE: &str = "system-login-choice";
pub const SETTINGS: &str = "system-login-settings";

const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const OPEN_APPLICATION: u32 = u32::from_be_bytes(*b"oapp");
const PROPERTY_DATA: u32 = u32::from_be_bytes(*b"prdt");
const LAUNCHED_AS_LOGIN_ITEM: u32 = u32::from_be_bytes(*b"lgit");
const JOB_NOT_FOUND: isize = 6;
const ALREADY_REGISTERED: isize = 12;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Unavailable,
    Off,
    On,
    NeedsApproval,
    Missing,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Unavailable => "unavailable",
            Status::Off => "off",
            Status::On => "on",
            Status::NeedsApproval => "needs-approval",
            Status::Missing => "missing",
        }
    }

    pub fn registered(self) -> bool {
        matches!(self, Status::On | Status::NeedsApproval)
    }

    pub fn from_code(code: isize) -> Self {
        match code {
            0 => Status::Off,
            1 => Status::On,
            2 => Status::NeedsApproval,
            3 => Status::Missing,
            _ => Status::Unavailable,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Startup {
    No,
    Yes,
    Minimized,
}

pub const STARTUPS: [Startup; 3] = [Startup::No, Startup::Yes, Startup::Minimized];

impl Startup {
    pub fn label(self) -> &'static str {
        match self {
            Startup::No => "No",
            Startup::Yes => "Yes",
            Startup::Minimized => "Minimized",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        STARTUPS
            .into_iter()
            .find(|choice| choice.label().eq_ignore_ascii_case(text))
    }

    pub fn current(status: Status, minimized: bool) -> Self {
        match (status.registered(), minimized) {
            (false, _) => Startup::No,
            (true, false) => Startup::Yes,
            (true, true) => Startup::Minimized,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Startup::No => Startup::Yes,
            Startup::Yes => Startup::Minimized,
            Startup::Minimized => Startup::No,
        }
    }

    pub fn registers(self) -> bool {
        self != Startup::No
    }

    pub fn minimized(self) -> bool {
        self == Startup::Minimized
    }

    pub fn describe(self, status: Status, error: Option<&str>) -> String {
        if let Some(error) = error {
            return format!("macOS didn’t accept the change: {error}");
        }
        match (self, status) {
            (_, Status::NeedsApproval) => {
                "Allow OpenMusic in System Settings › General › Login Items so it can open when you log in.".to_owned()
            }
            (Startup::No, _) => "OpenMusic stays closed until you open it.".to_owned(),
            (Startup::Yes, _) => "OpenMusic opens with its window when you log in.".to_owned(),
            (Startup::Minimized, _) => {
                "OpenMusic opens in the background when you log in, without its window. Click its Dock icon to show it."
                    .to_owned()
            }
        }
    }
}

pub fn outcome(
    enable: bool,
    after: Status,
    failure: Option<(isize, String)>,
) -> Result<Status, String> {
    let Some((code, message)) = failure else {
        return Ok(after);
    };
    let settled = if enable {
        after.registered() || code == ALREADY_REGISTERED
    } else {
        !after.registered() || code == JOB_NOT_FOUND
    };
    if settled { Ok(after) } else { Err(message) }
}

pub fn launched_as_login_item(class: u32, id: u32, property: Option<u32>) -> bool {
    class == CORE_EVENT_CLASS && id == OPEN_APPLICATION && property == Some(LAUNCHED_AS_LOGIN_ITEM)
}

#[cfg(target_os = "macos")]
mod native {
    use std::sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    };

    use block2::RcBlock;
    use objc2::{
        MainThreadMarker, msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_app_kit::NSApplication;
    use objc2_foundation::{NSBundle, NSOperationQueue};
    use objc2_service_management::SMAppService;

    use super::{PROPERTY_DATA, Status, launched_as_login_item, outcome};
    use crate::areas::system::native_window;

    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    static AT_LOGIN: AtomicBool = AtomicBool::new(false);

    pub fn available() -> bool {
        *AVAILABLE.get_or_init(|| {
            let bundle = NSBundle::mainBundle();
            bundle.bundleIdentifier().is_some()
                && bundle.bundlePath().to_string().ends_with(".app")
                && AnyClass::get(c"SMAppService").is_some()
        })
    }

    fn service() -> Retained<SMAppService> {
        unsafe { SMAppService::mainAppService() }
    }

    pub fn status() -> Status {
        if !available() {
            return Status::Unavailable;
        }
        Status::from_code(unsafe { service().status() }.0)
    }

    pub fn set(enable: bool) -> Result<Status, String> {
        if !available() {
            return Err("Open at Login works only in the installed OpenMusic app.".to_owned());
        }
        let service = service();
        let result = unsafe {
            if enable {
                service.registerAndReturnError()
            } else {
                service.unregisterAndReturnError()
            }
        };
        let failure = result
            .err()
            .map(|error| (error.code(), error.localizedDescription().to_string()));
        outcome(enable, status(), failure)
    }

    pub fn open_settings() {
        if available() {
            unsafe { SMAppService::openSystemSettingsLoginItems() };
        }
    }

    pub fn record_launch() {
        let Some(manager_class) = AnyClass::get(c"NSAppleEventManager") else {
            return;
        };
        let at_login = unsafe {
            let manager: Retained<AnyObject> = msg_send![manager_class, sharedAppleEventManager];
            let event: Option<Retained<AnyObject>> = msg_send![&*manager, currentAppleEvent];
            event.is_some_and(|event| {
                let class: u32 = msg_send![&*event, eventClass];
                let id: u32 = msg_send![&*event, eventID];
                let property: Option<Retained<AnyObject>> =
                    msg_send![&*event, paramDescriptorForKeyword: PROPERTY_DATA];
                let code = property.map(|descriptor| {
                    let code: u32 = msg_send![&*descriptor, enumCodeValue];
                    code
                });
                launched_as_login_item(class, id, code)
            })
        };
        AT_LOGIN.store(at_login, Ordering::Relaxed);
    }

    pub fn launched_at_login() -> bool {
        AT_LOGIN.load(Ordering::Relaxed)
    }

    pub fn hide_at_launch() {
        let block = RcBlock::new(|| {
            native_window::hide();
            if let Some(mtm) = MainThreadMarker::new() {
                NSApplication::sharedApplication(mtm).hide(None);
            }
        });
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::Status;

    pub fn available() -> bool {
        false
    }

    pub fn status() -> Status {
        Status::Unavailable
    }

    pub fn set(_: bool) -> Result<Status, String> {
        Err("Open at Login works only on macOS.".to_owned())
    }

    pub fn open_settings() {}

    pub fn record_launch() {}

    pub fn launched_at_login() -> bool {
        false
    }

    pub fn hide_at_launch() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_codes_follow_service_management() {
        assert_eq!(Status::from_code(0), Status::Off);
        assert_eq!(Status::from_code(1), Status::On);
        assert_eq!(Status::from_code(2), Status::NeedsApproval);
        assert_eq!(Status::from_code(3), Status::Missing);
        assert_eq!(Status::from_code(9), Status::Unavailable);
        assert!(Status::On.registered());
        assert!(Status::NeedsApproval.registered());
        assert!(!Status::Missing.registered());
    }

    #[test]
    fn the_choice_comes_from_the_service_and_the_minimized_setting() {
        assert_eq!(Startup::current(Status::Off, true), Startup::No);
        assert_eq!(Startup::current(Status::Missing, false), Startup::No);
        assert_eq!(Startup::current(Status::On, false), Startup::Yes);
        assert_eq!(Startup::current(Status::On, true), Startup::Minimized);
        assert_eq!(
            Startup::current(Status::NeedsApproval, true),
            Startup::Minimized
        );
        for choice in STARTUPS {
            assert_eq!(Startup::parse(&choice.label().to_lowercase()), Some(choice));
            assert_ne!(choice.next(), choice);
        }
        assert_eq!(Startup::Minimized.next(), Startup::No);
        assert_eq!(Startup::parse("sometimes"), None);
    }

    #[test]
    fn errors_count_only_when_the_service_did_not_change() {
        assert_eq!(outcome(true, Status::On, None), Ok(Status::On));
        assert_eq!(
            outcome(
                true,
                Status::On,
                Some((ALREADY_REGISTERED, "already".into()))
            ),
            Ok(Status::On)
        );
        assert_eq!(
            outcome(
                true,
                Status::NeedsApproval,
                Some((1, "Operation not permitted".into()))
            ),
            Ok(Status::NeedsApproval)
        );
        assert_eq!(
            outcome(
                true,
                Status::Off,
                Some((1, "Operation not permitted".into()))
            ),
            Err("Operation not permitted".into())
        );
        assert_eq!(
            outcome(false, Status::Off, Some((JOB_NOT_FOUND, "missing".into()))),
            Ok(Status::Off)
        );
        assert_eq!(
            outcome(false, Status::On, Some((JOB_NOT_FOUND, "missing".into()))),
            Ok(Status::On)
        );
        assert_eq!(
            outcome(false, Status::On, Some((4, "Not allowed".into()))),
            Err("Not allowed".into())
        );
    }

    #[test]
    fn only_a_login_launch_counts() {
        let class = u32::from_be_bytes(*b"aevt");
        let open = u32::from_be_bytes(*b"oapp");
        let login = u32::from_be_bytes(*b"lgit");
        assert!(launched_as_login_item(class, open, Some(login)));
        assert!(!launched_as_login_item(class, open, None));
        assert!(!launched_as_login_item(
            class,
            u32::from_be_bytes(*b"rapp"),
            Some(login)
        ));
        assert!(!launched_as_login_item(
            class,
            open,
            Some(u32::from_be_bytes(*b"null"))
        ));
    }

    #[test]
    fn every_choice_explains_itself() {
        for choice in STARTUPS {
            assert!(!choice.describe(Status::On, None).is_empty());
        }
        assert!(
            Startup::Yes
                .describe(Status::NeedsApproval, None)
                .contains("Login Items")
        );
        assert!(
            Startup::No
                .describe(Status::Off, Some("Operation not permitted"))
                .ends_with("Operation not permitted")
        );
    }
}
