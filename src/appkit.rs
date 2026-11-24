use std::{cell::RefCell, rc::Rc};

use gpui::{Bounds, Pixels};
use objc2::{
    ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained,
    runtime::{AnyClass, AnyObject, NSObject},
};
use objc2_app_kit::{
    NSApplication, NSControl, NSEvent, NSEventModifierFlags, NSEventType, NSFont,
    NSFontWeightSemibold, NSImage, NSImageSymbolConfiguration, NSLineBreakMode, NSTextField,
    NSView,
};
use objc2_core_graphics::{CGEvent, CGEventField};
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::overlay::Action;

type Activate = Box<dyn Fn(&NSControl)>;

type Handler = Rc<dyn Fn(Action)>;

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OpenMusicControlTarget"]
    #[ivars = Activate]
    pub(crate) struct Target;

    unsafe impl NSObjectProtocol for Target {}

    impl Target {
        #[unsafe(method(activate:))]
        fn activate(&self, sender: &NSControl) {
            (self.ivars())(sender);
        }
    }
);

impl Target {
    pub(crate) fn new(
        mtm: MainThreadMarker,
        activate: impl Fn(&NSControl) + 'static,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Box::new(activate) as Activate);
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = RefCell::default();
}

pub(crate) fn set_handler(handler: impl Fn(Action) + 'static) {
    HANDLER.set(Some(Rc::new(handler)));
}

pub(crate) fn dispatch(action: Action) {
    if let Some(handler) = HANDLER.with_borrow(Clone::clone) {
        handler(action);
    }
}

pub(crate) fn string(text: &str) -> Retained<NSString> {
    NSString::from_str(text)
}

pub(crate) fn symbol(name: &str) -> Option<Retained<NSImage>> {
    NSImage::imageWithSystemSymbolName_accessibilityDescription(&string(name), None)
}

pub(crate) fn sized_symbol(name: &str, size: f64) -> Option<Retained<NSImage>> {
    let configuration =
        NSImageSymbolConfiguration::configurationWithPointSize_weight(size, unsafe {
            NSFontWeightSemibold
        });
    symbol(name)?.imageWithSymbolConfiguration(&configuration)
}

pub(crate) fn label(
    text: &str,
    size: f64,
    bold: bool,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let field = NSTextField::labelWithString(&string(text), mtm);
    let font = if bold {
        NSFont::boldSystemFontOfSize(size)
    } else {
        NSFont::systemFontOfSize(size)
    };
    field.setFont(Some(&font));
    field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    field
}

pub(crate) fn frame(container: &NSView, bounds: Bounds<Pixels>) -> NSRect {
    let height = container.bounds().size.height;
    let x = f64::from(f32::from(bounds.origin.x));
    let y = f64::from(f32::from(bounds.origin.y));
    let width = f64::from(f32::from(bounds.size.width));
    let tall = f64::from(f32::from(bounds.size.height));
    NSRect::new(NSPoint::new(x, height - y - tall), NSSize::new(width, tall))
}

pub(crate) fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width.max(0.), height))
}

pub(crate) fn container() -> Option<Retained<NSView>> {
    let host = crate::glass::host()?;
    unsafe { host.superview() }
}

pub(crate) fn right_click(x: f64, y: f64) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(view) = container() else {
        return false;
    };
    let Some(window) = view.window() else {
        return false;
    };
    let location = NSPoint::new(x, view.bounds().size.height - y);
    let number = window.windowNumber();
    let Some(process) = AnyClass::get(c"NSProcessInfo") else {
        return false;
    };
    let uptime: f64 = unsafe {
        let info: Retained<AnyObject> = msg_send![process, processInfo];
        msg_send![&*info, systemUptime]
    };
    let application = NSApplication::sharedApplication(mtm);
    for kind in [NSEventType::RightMouseDown, NSEventType::RightMouseUp] {
        let event: Option<Retained<NSEvent>> = unsafe {
            msg_send![
                NSEvent::class(),
                mouseEventWithType: kind,
                location: location,
                modifierFlags: NSEventModifierFlags::empty(),
                timestamp: uptime,
                windowNumber: number,
                context: None::<&AnyObject>,
                eventNumber: 0isize,
                clickCount: 1isize,
                pressure: 1.0f32
            ]
        };
        let Some(event) = event.and_then(|event| event.CGEvent()) else {
            return false;
        };
        CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventButtonNumber, 1);
        let Some(event) = NSEvent::eventWithCGEvent(&event) else {
            return false;
        };
        application.postEvent_atStart(&event, false);
    }
    true
}

pub(crate) fn nudge_window(dx: f64, dy: f64) -> bool {
    let Some(window) = container().and_then(|view| view.window()) else {
        return false;
    };
    let origin = window.frame().origin;
    window.setFrameOrigin(NSPoint::new(origin.x + dx, origin.y - dy));
    true
}
