pub use native::{close, hide, install, show, state, width};

pub const REOPEN: &str = "system-reopen";
pub const MINIATURIZED: &str = "system-miniaturized";
pub const RESTORED: &str = "system-restored";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowState {
    pub installed: bool,
    pub visible: bool,
    pub miniaturized: bool,
    pub key: bool,
    pub full_screen: bool,
    pub app_active: bool,
    pub listed: bool,
}

impl WindowState {
    pub fn frontmost(&self) -> bool {
        self.app_active && self.visible && !self.miniaturized
    }

    pub fn hidden(&self) -> bool {
        self.installed && (!self.visible || self.miniaturized)
    }

    pub fn describe(&self) -> String {
        format!(
            "installed={} visible={} miniaturized={} key={} full_screen={} app_active={} frontmost={} listed={}",
            self.installed,
            self.visible,
            self.miniaturized,
            self.key,
            self.full_screen,
            self.app_active,
            self.frontmost(),
            self.listed
        )
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::{
        cell::{Cell, RefCell},
        ptr::NonNull,
    };

    use block2::RcBlock;
    use objc2::{
        MainThreadMarker, MainThreadOnly, define_class, msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject, NSObject, ProtocolObject},
        sel,
    };
    use objc2_app_kit::{
        NSApplication, NSView, NSWindow, NSWindowDidDeminiaturizeNotification,
        NSWindowDidExitFullScreenNotification, NSWindowDidMiniaturizeNotification,
        NSWindowStyleMask,
    };
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSNotificationName, NSObjectProtocol,
        NSOperationQueue,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use super::{MINIATURIZED, REOPEN, RESTORED, WindowState};
    use crate::{appkit::dispatch, overlay::Action};

    const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
    const REOPEN_APPLICATION: u32 = u32::from_be_bytes(*b"rapp");

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OpenMusicReopenHandler"]
        struct Reopen;

        unsafe impl NSObjectProtocol for Reopen {}

        impl Reopen {
            #[unsafe(method(handleReopen:withReplyEvent:))]
            fn handle_reopen(&self, _event: &AnyObject, _reply: &AnyObject) {
                dispatch(Action::Press(REOPEN));
            }
        }
    );

    impl Reopen {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    struct Main {
        window: Retained<NSWindow>,
        _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
        _reopen: Option<Retained<Reopen>>,
    }

    thread_local! {
        static MAIN: RefCell<Option<Main>> = RefCell::default();
        static HIDE_AFTER_FULL_SCREEN: Cell<bool> = const { Cell::new(false) };
    }

    fn window() -> Option<Retained<NSWindow>> {
        MAIN.with_borrow(|main| main.as_ref().map(|main| main.window.clone()))
    }

    fn observe(
        window: &NSWindow,
        name: &NSNotificationName,
        run: impl Fn() + 'static,
    ) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
        let block = RcBlock::new(move |_: NonNull<NSNotification>| run());
        let object: &AnyObject = window;
        unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(name),
                Some(object),
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        }
    }

    fn listen_for_reopen(mtm: MainThreadMarker) -> Option<Retained<Reopen>> {
        let manager_class = AnyClass::get(c"NSAppleEventManager")?;
        let handler = Reopen::new(mtm);
        unsafe {
            let manager: Retained<AnyObject> = msg_send![manager_class, sharedAppleEventManager];
            let receiver: &AnyObject = &handler;
            let _: () = msg_send![
                &*manager,
                setEventHandler: receiver,
                andSelector: sel!(handleReopen:withReplyEvent:),
                forEventClass: CORE_EVENT_CLASS,
                andEventID: REOPEN_APPLICATION
            ];
        }
        Some(handler)
    }

    pub fn install(window: &gpui::Window) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return false;
        };
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return false;
        };
        let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
        let Some(window) = view.window() else {
            return false;
        };
        window.setExcludedFromWindowsMenu(true);
        let observers = unsafe {
            vec![
                observe(&window, NSWindowDidMiniaturizeNotification, || {
                    dispatch(Action::Press(MINIATURIZED))
                }),
                observe(&window, NSWindowDidDeminiaturizeNotification, || {
                    dispatch(Action::Press(RESTORED))
                }),
                observe(&window, NSWindowDidExitFullScreenNotification, || {
                    if HIDE_AFTER_FULL_SCREEN.replace(false)
                        && let Some(window) = self::window()
                    {
                        window.orderOut(None);
                    }
                }),
            ]
        };
        MAIN.set(Some(Main {
            window,
            _observers: observers,
            _reopen: listen_for_reopen(mtm),
        }));
        true
    }

    pub fn hide() {
        let Some(window) = window() else {
            return;
        };
        if window.styleMask().contains(NSWindowStyleMask::FullScreen) {
            HIDE_AFTER_FULL_SCREEN.set(true);
            window.toggleFullScreen(None);
            return;
        }
        window.orderOut(None);
    }

    pub fn close() {
        if let Some(window) = window() {
            window.performClose(None);
        }
    }

    pub fn show() {
        let Some(window) = window() else {
            return;
        };
        HIDE_AFTER_FULL_SCREEN.set(false);
        if window.isMiniaturized() {
            window.deminiaturize(None);
        }
        window.makeKeyAndOrderFront(None);
    }

    pub fn width() -> f32 {
        window().map_or(0., |window| window.frame().size.width as f32)
    }

    pub fn state() -> WindowState {
        let app_active = MainThreadMarker::new()
            .is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive());
        match window() {
            Some(window) => WindowState {
                installed: true,
                visible: window.isVisible(),
                miniaturized: window.isMiniaturized(),
                key: window.isKeyWindow(),
                full_screen: window.styleMask().contains(NSWindowStyleMask::FullScreen),
                app_active,
                listed: !window.isExcludedFromWindowsMenu(),
            },
            None => WindowState {
                app_active,
                ..WindowState::default()
            },
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::WindowState;

    pub fn install(_: &gpui::Window) -> bool {
        false
    }

    pub fn hide() {}

    pub fn close() {}

    pub fn show() {}

    pub fn state() -> WindowState {
        WindowState::default()
    }

    pub fn width() -> f32 {
        0.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmost_needs_an_active_app_and_a_shown_window() {
        let shown = WindowState {
            installed: true,
            visible: true,
            app_active: true,
            ..WindowState::default()
        };
        assert!(shown.frontmost());
        for state in [
            WindowState {
                app_active: false,
                ..shown
            },
            WindowState {
                visible: false,
                ..shown
            },
            WindowState {
                miniaturized: true,
                ..shown
            },
        ] {
            assert!(!state.frontmost(), "{}", state.describe());
        }
    }

    #[test]
    fn hidden_means_closed_or_minimized_but_not_merely_behind() {
        let shown = WindowState {
            installed: true,
            visible: true,
            ..WindowState::default()
        };
        assert!(!shown.hidden());
        assert!(
            !WindowState {
                app_active: true,
                ..shown
            }
            .hidden()
        );
        assert!(
            WindowState {
                visible: false,
                ..shown
            }
            .hidden()
        );
        assert!(
            WindowState {
                miniaturized: true,
                ..shown
            }
            .hidden()
        );
        assert!(!WindowState::default().hidden());
    }
}
