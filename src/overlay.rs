use std::sync::Arc;

use gpui::{Image, IntoElement, Styled, canvas};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Press(&'static str),
    Seek(f32),
    Volume(f32),
}

#[derive(Clone, Copy)]
pub enum Label {
    Symbol(&'static str),
    Text(&'static str),
}

#[derive(Clone)]
pub struct NowPlaying {
    pub title: String,
    pub subtitle: String,
    pub alert: bool,
    pub cover: Option<(String, Arc<Image>)>,
    pub position_ms: u32,
    pub duration_ms: u32,
}

pub use native::{available, progress, set_handler};

pub fn button(key: &'static str, label: Label, prominent: bool) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| native::button(key, label, prominent, bounds, window),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub fn now_playing(state: NowPlaying) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| native::now_playing(&state, bounds, window),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub fn volume(fraction: f32, corner: f32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| native::volume(fraction, corner, bounds, window),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub fn finish() -> impl IntoElement {
    canvas(|_, _, _| (), |_, _, _, _| native::finish())
        .absolute()
        .size_0()
}

pub fn handler(sender: UnboundedSender<Action>) -> impl Fn(Action) + 'static {
    move |action| {
        let _ = sender.send(action);
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::{cell::RefCell, collections::HashMap};

    use gpui::{Bounds, Pixels, Window};
    use objc2::{
        AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject},
        sel,
    };
    use objc2_app_kit::{
        NSApplication, NSBezelStyle, NSButton, NSColor, NSControl, NSControlBorderShape,
        NSControlSize, NSEvent, NSEventType, NSFont, NSFontWeightSemibold, NSGlassEffectView,
        NSImage, NSImageScaling, NSImageSymbolConfiguration, NSImageView, NSLineBreakMode,
        NSSlider, NSTextAlignment, NSTextField, NSView, NSWindowOrderingMode,
    };
    use objc2_foundation::{NSData, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

    use super::{Action, Label, NowPlaying};

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OpenMusicControlTarget"]
        struct Target;

        unsafe impl NSObjectProtocol for Target {}

        impl Target {
            #[unsafe(method(activate:))]
            fn activate(&self, sender: &NSControl) {
                if sender.tag() == SEEK && dragging() {
                    preview(sender.doubleValue());
                    return;
                }
                dispatch(sender.tag(), sender.doubleValue());
            }
        }
    );

    impl Target {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    const SEEK: isize = -1;
    const VOLUME: isize = -2;

    struct Dock {
        glass: Retained<NSGlassEffectView>,
        art: Retained<NSImageView>,
        title: Retained<NSTextField>,
        subtitle: Retained<NSTextField>,
        elapsed: Retained<NSTextField>,
        remaining: Retained<NSTextField>,
        slider: Retained<NSSlider>,
        cover: Option<String>,
        duration_ms: u32,
        scrubbing: bool,
        placed: bool,
    }

    struct Volume {
        glass: Retained<NSGlassEffectView>,
        icon: Retained<NSImageView>,
        slider: Retained<NSSlider>,
        placed: bool,
    }

    struct Control {
        button: Retained<NSButton>,
        symbol: (&'static str, u32),
        placed: bool,
    }

    #[derive(Default)]
    struct Overlay {
        target: Option<Retained<Target>>,
        handler: Option<Box<dyn Fn(Action)>>,
        keys: Vec<&'static str>,
        buttons: HashMap<&'static str, Control>,
        dock: Option<Dock>,
        volume: Option<Volume>,
    }

    thread_local! {
        static OVERLAY: RefCell<Overlay> = RefCell::default();
    }

    pub fn available() -> bool {
        crate::glass::available()
    }

    pub fn set_handler(handler: impl Fn(Action) + 'static) {
        OVERLAY.with_borrow_mut(|overlay| overlay.handler = Some(Box::new(handler)));
    }

    fn dragging() -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        NSApplication::sharedApplication(mtm)
            .currentEvent()
            .is_some_and(|event| {
                matches!(
                    event.r#type(),
                    NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged
                )
            })
    }

    fn preview(fraction: f64) {
        OVERLAY.with_borrow_mut(|overlay| {
            if let Some(dock) = overlay.dock.as_mut() {
                dock.scrubbing = true;
                let position = (fraction * f64::from(dock.duration_ms)) as u32;
                dock.elapsed
                    .setStringValue(&string(&crate::timestamp(position)));
            }
        });
    }

    fn dispatch(tag: isize, value: f64) {
        if tag == SEEK {
            OVERLAY.with_borrow_mut(|overlay| {
                if let Some(dock) = overlay.dock.as_mut() {
                    dock.scrubbing = false;
                }
            });
        }
        OVERLAY.with_borrow(|overlay| {
            let action = match tag {
                SEEK => Some(Action::Seek(value as f32)),
                VOLUME => Some(Action::Volume(value as f32)),
                tag => overlay.keys.get(tag as usize).copied().map(Action::Press),
            };
            if let (Some(action), Some(handler)) = (action, &overlay.handler) {
                handler(action);
            }
        });
    }

    fn string(text: &str) -> Retained<NSString> {
        NSString::from_str(text)
    }

    fn symbol(name: &str) -> Option<Retained<NSImage>> {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(&string(name), None)
    }

    fn sized_symbol(name: &str, size: f64) -> Option<Retained<NSImage>> {
        let configuration =
            NSImageSymbolConfiguration::configurationWithPointSize_weight(size, unsafe {
                NSFontWeightSemibold
            });
        symbol(name)?.imageWithSymbolConfiguration(&configuration)
    }

    fn label(text: &str, size: f64, bold: bool, mtm: MainThreadMarker) -> Retained<NSTextField> {
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

    fn frame(container: &NSView, bounds: Bounds<Pixels>) -> NSRect {
        let height = container.bounds().size.height;
        let x = f64::from(f32::from(bounds.origin.x));
        let y = f64::from(f32::from(bounds.origin.y));
        let width = f64::from(f32::from(bounds.size.width));
        let tall = f64::from(f32::from(bounds.size.height));
        NSRect::new(NSPoint::new(x, height - y - tall), NSSize::new(width, tall))
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width.max(0.), height))
    }

    fn visible(bounds: Bounds<Pixels>, window: &Window) -> bool {
        let mask = window.content_mask().bounds;
        mask.contains(&bounds.origin) && mask.contains(&bounds.bottom_right())
    }

    fn attach(view: &NSView) -> Option<Retained<NSView>> {
        let host = crate::glass::host()?;
        let container = unsafe { host.superview() }?;
        container.addSubview_positioned_relativeTo(view, NSWindowOrderingMode::Above, Some(&host));
        Some(container)
    }

    fn container() -> Option<Retained<NSView>> {
        let host = crate::glass::host()?;
        unsafe { host.superview() }
    }

    fn target(overlay: &mut Overlay, mtm: MainThreadMarker) -> Retained<Target> {
        overlay
            .target
            .get_or_insert_with(|| Target::new(mtm))
            .clone()
    }

    fn make_interactive(glass: &NSGlassEffectView) {
        let selector = sel!(setEffectIsInteractive:);
        if glass.respondsToSelector(selector) {
            unsafe {
                let _: () = msg_send![glass, setEffectIsInteractive: true];
            }
        }
    }

    fn mouse_down() -> bool {
        NSEvent::pressedMouseButtons() != 0
    }

    pub fn button(
        key: &'static str,
        label: Label,
        prominent: bool,
        bounds: Bounds<Pixels>,
        window: &Window,
    ) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let shown = visible(bounds, window);
        OVERLAY.with_borrow_mut(|overlay| {
            if !overlay.buttons.contains_key(key) {
                let target = target(overlay, mtm);
                let receiver: &AnyObject = &target;
                let button = unsafe {
                    match label {
                        Label::Symbol(name) => NSButton::buttonWithImage_target_action(
                            &symbol(name).unwrap_or_default(),
                            Some(receiver),
                            Some(sel!(activate:)),
                            mtm,
                        ),
                        Label::Text(title) => NSButton::buttonWithTitle_target_action(
                            &string(title),
                            Some(receiver),
                            Some(sel!(activate:)),
                            mtm,
                        ),
                    }
                };
                button.setBezelStyle(NSBezelStyle::Glass);
                button.setBorderShape(match label {
                    Label::Symbol(_) => NSControlBorderShape::Circle,
                    Label::Text(_) => NSControlBorderShape::Capsule,
                });
                button.setControlSize(NSControlSize::Large);
                button.setImageScaling(NSImageScaling::ScaleNone);
                if prominent {
                    button.setBezelColor(Some(&NSColor::controlAccentColor()));
                }
                button.setTag(overlay.keys.len() as isize);
                overlay.keys.push(key);
                if attach(&button).is_none() {
                    return;
                }
                overlay.buttons.insert(
                    key,
                    Control {
                        button,
                        symbol: ("", 0),
                        placed: false,
                    },
                );
            }
            let Some(container) = container() else {
                return;
            };
            let control = overlay.buttons.get_mut(key).expect("button was created");
            control.placed = shown;
            control.button.setHidden(!shown);
            if !shown {
                return;
            }
            let rect = frame(&container, bounds);
            if let Label::Symbol(name) = label {
                let size = (rect.size.height * 0.34).clamp(13., 22.);
                let key = (name, size.round() as u32);
                if control.symbol != key {
                    control.button.setImage(sized_symbol(name, size).as_deref());
                    control.symbol = key;
                }
            }
            control.button.setFrame(rect);
        });
    }

    pub fn now_playing(state: &NowPlaying, bounds: Bounds<Pixels>, window: &Window) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let shown = visible(bounds, window);
        OVERLAY.with_borrow_mut(|overlay| {
            if overlay.dock.is_none() {
                let target = target(overlay, mtm);
                let glass =
                    NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), NSRect::ZERO);
                make_interactive(&glass);
                let content = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
                let art =
                    NSImageView::imageViewWithImage(&symbol("music.note").unwrap_or_default(), mtm);
                art.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                art.setWantsLayer(true);
                unsafe {
                    let layer: Option<Retained<AnyObject>> = msg_send![&*art, layer];
                    if let Some(layer) = layer {
                        let _: () = msg_send![&*layer, setCornerRadius: 8.0f64];
                        let _: () = msg_send![&*layer, setMasksToBounds: true];
                    }
                }
                let title = label("", 13., true, mtm);
                let subtitle = label("", 12., false, mtm);
                subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
                let elapsed = label("0:00", 11., false, mtm);
                elapsed.setTextColor(Some(&NSColor::secondaryLabelColor()));
                elapsed.setAlignment(NSTextAlignment::Right);
                let remaining = label("0:00", 11., false, mtm);
                remaining.setTextColor(Some(&NSColor::secondaryLabelColor()));
                let receiver: &AnyObject = &target;
                let slider = unsafe {
                    NSSlider::sliderWithValue_minValue_maxValue_target_action(
                        0.,
                        0.,
                        1.,
                        Some(receiver),
                        Some(sel!(activate:)),
                        mtm,
                    )
                };
                slider.setContinuous(true);
                slider.setControlSize(NSControlSize::Small);
                slider.setTag(SEEK);
                for view in [
                    &**art as &NSView,
                    &title,
                    &subtitle,
                    &elapsed,
                    &slider,
                    &remaining,
                ] {
                    content.addSubview(view);
                }
                glass.setContentView(Some(&content));
                if attach(&glass).is_none() {
                    return;
                }
                overlay.dock = Some(Dock {
                    glass,
                    art,
                    title,
                    subtitle,
                    elapsed,
                    remaining,
                    slider,
                    cover: None,
                    duration_ms: 0,
                    scrubbing: false,
                    placed: false,
                });
            }
            let Some(container) = container() else {
                return;
            };
            let dock = overlay.dock.as_mut().expect("dock was created");
            dock.placed = shown;
            dock.glass.setHidden(!shown);
            if !shown {
                return;
            }
            let outer = frame(&container, bounds);
            let (width, height) = (outer.size.width, outer.size.height);
            dock.glass.setFrame(outer);
            dock.glass.setCornerRadius(height / 2.);

            let art = height - 18.;
            dock.art.setFrame(rect(9., 9., art, art));
            let text_x = 9. + art + 12.;
            let text_width = (width * 0.3).clamp(70., 220.);
            dock.title
                .setFrame(rect(text_x, height / 2. + 1., text_width, 17.));
            dock.subtitle
                .setFrame(rect(text_x, height / 2. - 17., text_width, 16.));
            let bar_x = text_x + text_width + 14.;
            let bar_width = width - bar_x - 20.;
            let compact = bar_width < 200.;
            dock.elapsed.setHidden(compact);
            dock.remaining.setHidden(compact);
            if compact {
                dock.slider
                    .setFrame(rect(bar_x, height / 2. - 10., bar_width, 20.));
            } else {
                dock.elapsed
                    .setFrame(rect(bar_x, height / 2. - 8., 38., 16.));
                dock.slider
                    .setFrame(rect(bar_x + 44., height / 2. - 10., bar_width - 88., 20.));
                dock.remaining
                    .setFrame(rect(bar_x + bar_width - 38., height / 2. - 8., 38., 16.));
            }

            dock.title.setStringValue(&string(&state.title));
            dock.subtitle.setStringValue(&string(&state.subtitle));
            let color = if state.alert {
                NSColor::systemRedColor()
            } else {
                NSColor::secondaryLabelColor()
            };
            dock.subtitle.setTextColor(Some(&color));
            dock.duration_ms = state.duration_ms;
            dock.remaining
                .setStringValue(&string(&crate::timestamp(state.duration_ms)));
            dock.slider.setEnabled(state.duration_ms > 0);
            if !dock.scrubbing {
                dock.elapsed
                    .setStringValue(&string(&crate::timestamp(state.position_ms)));
                dock.slider.setDoubleValue(
                    f64::from(state.position_ms) / f64::from(state.duration_ms.max(1)),
                );
            }

            let cover = state.cover.as_ref().map(|(url, _)| url.clone());
            if dock.cover != cover {
                let image = state
                    .cover
                    .as_ref()
                    .and_then(|(_, image)| {
                        NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(image.bytes()))
                    })
                    .or_else(|| symbol("music.note"));
                dock.art.setImage(image.as_deref());
                dock.cover = cover;
            }
        });
    }

    pub fn volume(fraction: f32, corner: f32, bounds: Bounds<Pixels>, window: &Window) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let shown = visible(bounds, window);
        OVERLAY.with_borrow_mut(|overlay| {
            if overlay.volume.is_none() {
                let target = target(overlay, mtm);
                let glass =
                    NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), NSRect::ZERO);
                make_interactive(&glass);
                let content = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
                let icon = NSImageView::imageViewWithImage(
                    &symbol("speaker.wave.2.fill").unwrap_or_default(),
                    mtm,
                );
                icon.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
                let receiver: &AnyObject = &target;
                let slider = unsafe {
                    NSSlider::sliderWithValue_minValue_maxValue_target_action(
                        1.,
                        0.,
                        1.,
                        Some(receiver),
                        Some(sel!(activate:)),
                        mtm,
                    )
                };
                slider.setContinuous(true);
                slider.setControlSize(NSControlSize::Small);
                slider.setTag(VOLUME);
                content.addSubview(&icon);
                content.addSubview(&slider);
                glass.setContentView(Some(&content));
                if attach(&glass).is_none() {
                    return;
                }
                overlay.volume = Some(Volume {
                    glass,
                    icon,
                    slider,
                    placed: false,
                });
            }
            let Some(container) = container() else {
                return;
            };
            let volume = overlay.volume.as_mut().expect("volume was created");
            volume.placed = shown;
            volume.glass.setHidden(!shown);
            if !shown {
                return;
            }
            let outer = frame(&container, bounds);
            let (width, height) = (outer.size.width, outer.size.height);
            volume.glass.setFrame(outer);
            volume
                .glass
                .setCornerRadius((height / 2.).min(f64::from(corner)));
            volume.icon.setFrame(rect(16., height / 2. - 9., 20., 18.));
            volume
                .slider
                .setFrame(rect(44., height / 2. - 10., width - 62., 20.));
            if !mouse_down() {
                volume.slider.setDoubleValue(f64::from(fraction));
            }
        });
    }

    pub fn progress(position_ms: u32, duration_ms: u32) {
        OVERLAY.with_borrow_mut(|overlay| {
            let Some(dock) = overlay.dock.as_mut() else {
                return;
            };
            if dock.scrubbing || !dock.placed && dock.glass.isHidden() {
                return;
            }
            dock.elapsed
                .setStringValue(&string(&crate::timestamp(position_ms)));
            dock.slider
                .setDoubleValue(f64::from(position_ms) / f64::from(duration_ms.max(1)));
        });
    }

    pub fn finish() {
        OVERLAY.with_borrow_mut(|overlay| {
            for control in overlay.buttons.values_mut() {
                if !control.placed {
                    control.button.setHidden(true);
                }
                control.placed = false;
            }
            if let Some(dock) = overlay.dock.as_mut() {
                if !dock.placed {
                    dock.glass.setHidden(true);
                }
                dock.placed = false;
            }
            if let Some(volume) = overlay.volume.as_mut() {
                if !volume.placed {
                    volume.glass.setHidden(true);
                }
                volume.placed = false;
            }
        });
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use gpui::{Bounds, Pixels, Window};

    use super::{Action, Label, NowPlaying};

    pub fn available() -> bool {
        false
    }

    pub fn set_handler(_: impl Fn(Action) + 'static) {}

    pub fn button(_: &'static str, _: Label, _: bool, _: Bounds<Pixels>, _: &Window) {}

    pub fn now_playing(_: &NowPlaying, _: Bounds<Pixels>, _: &Window) {}

    pub fn volume(_: f32, _: f32, _: Bounds<Pixels>, _: &Window) {}

    pub fn finish() {}

    pub fn progress(_: u32, _: u32) {}
}
