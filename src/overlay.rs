use std::{cell::RefCell, collections::HashMap, sync::Arc};

use gpui::{Bounds, Image, IntoElement, Pixels, Point, Styled, canvas, point, px};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Copy, Debug)]
pub enum Action {
    Press(&'static str),
    Seek(f32),
    Volume(f32),
    Menu(u32),
    MenuClosed,
    BarMenu { generation: u32, id: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Label {
    Symbol(&'static str),
    Text(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ButtonSpec {
    pub label: Label,
    pub prominent: bool,
    pub enabled: bool,
    pub tooltip: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    Item {
        title: String,
        id: u32,
        enabled: bool,
        checked: bool,
        symbol: Option<&'static str>,
        key: Option<&'static str>,
        alternate: Option<(String, u32)>,
        subtitle: Option<String>,
    },
    Note(String),
    Header(String),
    Separator,
    Submenu {
        title: String,
        symbol: Option<&'static str>,
        entries: Vec<Entry>,
        search: Option<&'static str>,
        id: Option<u32>,
    },
    Volume(f32),
}

#[derive(Clone)]
pub struct NowPlaying {
    pub title: String,
    pub subtitle: String,
    pub alert: bool,
    pub cover: Option<(String, Arc<Image>)>,
    pub position_ms: u32,
    pub duration_ms: u32,
    pub tooltip: Option<String>,
}

thread_local! {
    static ANCHORS: RefCell<HashMap<&'static str, Bounds<Pixels>>> = RefCell::default();
}

#[cfg(target_os = "macos")]
pub use native::dock_artwork;
pub use native::{
    available, bar_built, bar_menu, bar_menu_tree, close_menu, drag, filter_menu, menu_bar_tree,
    menu_tree, nudge_window, pick, progress, right_click, set_handler, update_menu,
};

pub fn button(key: &'static str, spec: ButtonSpec) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| native::button(key, &spec, bounds, window),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub fn anchor(key: &'static str) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, _, _| {
            ANCHORS.with_borrow_mut(|anchors| anchors.insert(key, bounds));
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub fn context_menu(entries: Vec<Entry>, at: Point<Pixels>) -> bool {
    native::popup(entries, at)
}

pub fn menu_at(anchor: &'static str, entries: Vec<Entry>) -> bool {
    let Some(bounds) = ANCHORS.with_borrow(|anchors| anchors.get(anchor).copied()) else {
        return false;
    };
    native::popup(entries, point(bounds.origin.x, bounds.bottom() + px(4.)))
}

pub fn menu_above(anchor: &'static str, entries: Vec<Entry>) -> bool {
    let Some(bounds) = ANCHORS.with_borrow(|anchors| anchors.get(anchor).copied()) else {
        return false;
    };
    native::popup_above(entries, point(bounds.origin.x, bounds.origin.y - px(4.)))
}

pub fn key(name: &str) -> Option<&'static str> {
    native::key(name)
        .or_else(|| ANCHORS.with_borrow(|anchors| anchors.keys().copied().find(|key| *key == name)))
}

pub fn describe() -> String {
    let mut lines = native::describe();
    #[cfg(target_os = "macos")]
    if let Some(art) = dock_artwork() {
        let frame = art.frame();
        let sublayers = art
            .layer()
            .and_then(|layer| unsafe { layer.sublayers() })
            .map_or(0, |layers| layers.count());
        lines.push(format!(
            "artwork frame={},{},{},{} image={} sublayers={sublayers}",
            frame.origin.x,
            frame.origin.y,
            frame.size.width,
            frame.size.height,
            art.image().is_some(),
        ));
    }
    ANCHORS.with_borrow(|anchors| {
        let mut anchors: Vec<_> = anchors.iter().collect();
        anchors.sort_by_key(|(key, _)| **key);
        for (key, bounds) in anchors {
            lines.push(format!(
                "anchor {key} x={} y={} w={} h={}",
                f32::from(bounds.origin.x),
                f32::from(bounds.origin.y),
                f32::from(bounds.size.width),
                f32::from(bounds.size.height),
            ));
        }
    });
    lines.join("\n")
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

pub fn volume(
    fraction: f32,
    corner: f32,
    enabled: bool,
    tooltip: Option<String>,
) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            native::volume(
                fraction,
                corner,
                enabled,
                tooltip.as_deref(),
                bounds,
                window,
            )
        },
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

    use gpui::{Bounds, Pixels, Point, Window};
    use objc2::{
        AllocAnyThread, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObject, ProtocolObject, Sel},
        sel,
    };
    use objc2_app_kit::{
        NSApplication, NSAutoresizingMaskOptions, NSBezelStyle, NSButton, NSCellImagePosition,
        NSColor, NSControl, NSControlBorderShape, NSControlSize, NSControlStateValueOn, NSEvent,
        NSEventModifierFlags, NSEventTrackingRunLoopMode, NSEventType, NSGlassEffectView, NSImage,
        NSImageScaling, NSImageView, NSMenu, NSMenuDelegate, NSMenuItem, NSResponder,
        NSSearchField, NSSlider, NSTextAlignment, NSTextField, NSView, NSWindowOrderingMode,
    };
    use objc2_foundation::{NSArray, NSData, NSObjectProtocol, NSPoint, NSRect};

    use super::{Action, ButtonSpec, Entry, Label, NowPlaying};
    use crate::{
        appkit::{
            self, Target, container, dispatch, frame, label, rect, sized_symbol, string, symbol,
        },
        ui::components::timestamp,
    };

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OpenMusicMenuDelegate"]
        struct MenuDelegate;

        unsafe impl NSObjectProtocol for MenuDelegate {}

        unsafe impl NSMenuDelegate for MenuDelegate {
            #[unsafe(method(menuWillOpen:))]
            fn menu_will_open(&self, menu: &NSMenu) {
                will_open(self, menu);
            }

            #[unsafe(method(menuDidClose:))]
            fn menu_did_close(&self, menu: &NSMenu) {
                did_close(menu);
            }

            #[unsafe(method(menuNeedsUpdate:))]
            fn menu_needs_update(&self, menu: &NSMenu) {
                needs_update(menu);
            }
        }

        impl MenuDelegate {
            #[unsafe(method(menuItem:))]
            fn menu_item(&self, sender: &NSMenuItem) {
                dispatch(Action::Menu(sender.tag() as u32));
            }

            #[unsafe(method(barMenuItem:))]
            fn bar_menu_item(&self, sender: &NSMenuItem) {
                if let Some(generation) = OVERLAY.with_borrow(|overlay| overlay.bar_built) {
                    dispatch(Action::BarMenu {
                        generation,
                        id: sender.tag() as u32,
                    });
                }
            }

            #[unsafe(method(showMenu:))]
            fn show_menu(&self, _: Option<&AnyObject>) {
                show_pending();
            }

            #[unsafe(method(filter:))]
            fn filter(&self, sender: &NSSearchField) {
                filter_field(sender);
            }

            #[unsafe(method(focusField:))]
            fn focus_field(&self, field: &NSSearchField) {
                if let Some(window) = field.window() {
                    let responder: &NSResponder = field;
                    window.makeFirstResponder(Some(responder));
                }
            }
        }
    );

    impl MenuDelegate {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), init] }
        }
    }

    const SEEK: isize = -1;
    const VOLUME: isize = -2;
    const NO_MATCH: isize = -1;

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
        tooltip: Option<String>,
    }

    struct Volume {
        glass: Retained<NSGlassEffectView>,
        icon: Retained<NSImageView>,
        slider: Retained<NSSlider>,
        tooltip: Option<String>,
        placed: bool,
    }

    struct Control {
        button: Retained<NSButton>,
        label: Label,
        symbol: (&'static str, u32),
        prominent: bool,
        tooltip: Option<String>,
        placed: bool,
    }

    #[derive(Default)]
    struct Built {
        submenus: HashMap<u32, Retained<NSMenu>>,
        searches: Vec<(Retained<NSMenu>, Retained<NSSearchField>)>,
    }

    #[derive(Default)]
    struct Overlay {
        target: Option<Retained<Target>>,
        delegate: Option<Retained<MenuDelegate>>,
        keys: Vec<&'static str>,
        buttons: HashMap<&'static str, Control>,
        dock: Option<Dock>,
        volume: Option<Volume>,
        pending: Option<(Vec<Entry>, Point<Pixels>, bool)>,
        context: Option<Retained<NSMenu>>,
        tracking: Option<Retained<NSMenu>>,
        built: Built,
        bar_entries: Vec<Entry>,
        bar_generation: u32,
        bar_built: Option<u32>,
        bar: Option<Retained<NSMenu>>,
    }

    thread_local! {
        static OVERLAY: RefCell<Overlay> = RefCell::default();
    }

    pub fn available() -> bool {
        crate::glass::available()
    }

    pub fn set_handler(handler: impl Fn(Action) + 'static) {
        appkit::set_handler(handler);
    }

    pub fn key(name: &str) -> Option<&'static str> {
        OVERLAY.with_borrow(|overlay| overlay.keys.iter().copied().find(|key| *key == name))
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
                dock.elapsed.setStringValue(&string(&timestamp(position)));
            }
        });
    }

    fn activate(sender: &NSControl) {
        if sender.tag() == SEEK && dragging() {
            preview(sender.doubleValue());
            return;
        }
        press(sender.tag(), sender.doubleValue());
    }

    fn press(tag: isize, value: f64) {
        let action = OVERLAY.with_borrow_mut(|overlay| match tag {
            SEEK => {
                if let Some(dock) = overlay.dock.as_mut() {
                    dock.scrubbing = false;
                }
                Some(Action::Seek(value as f32))
            }
            VOLUME => Some(Action::Volume(value as f32)),
            tag => overlay.keys.get(tag as usize).copied().map(Action::Press),
        });
        if let Some(action) = action {
            dispatch(action);
        }
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

    fn target(overlay: &mut Overlay, mtm: MainThreadMarker) -> Retained<Target> {
        overlay
            .target
            .get_or_insert_with(|| Target::new(mtm, activate))
            .clone()
    }

    fn delegate(overlay: &mut Overlay, mtm: MainThreadMarker) -> Retained<MenuDelegate> {
        overlay
            .delegate
            .get_or_insert_with(|| MenuDelegate::new(mtm))
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

    fn show_label(button: &NSButton, label: Label) {
        match label {
            Label::Symbol(_) => {
                button.setTitle(&string(""));
                button.setImagePosition(NSCellImagePosition::ImageOnly);
                button.setBorderShape(NSControlBorderShape::Circle);
            }
            Label::Text(title) => {
                button.setImage(None);
                button.setImagePosition(NSCellImagePosition::NoImage);
                button.setTitle(&string(title));
                button.setBorderShape(NSControlBorderShape::Capsule);
            }
        }
    }

    pub fn button(key: &'static str, spec: &ButtonSpec, bounds: Bounds<Pixels>, window: &Window) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let shown = visible(bounds, window);
        OVERLAY.with_borrow_mut(|overlay| {
            if !overlay.buttons.contains_key(key) {
                let target = target(overlay, mtm);
                let receiver: &AnyObject = &target;
                let button = unsafe {
                    match spec.label {
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
                button.setBorderShape(match spec.label {
                    Label::Symbol(_) => NSControlBorderShape::Circle,
                    Label::Text(_) => NSControlBorderShape::Capsule,
                });
                button.setControlSize(NSControlSize::Large);
                button.setImageScaling(NSImageScaling::ScaleNone);
                if spec.prominent {
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
                        label: spec.label,
                        symbol: ("", 0),
                        prominent: spec.prominent,
                        tooltip: None,
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
            if control.label != spec.label {
                show_label(&control.button, spec.label);
                control.label = spec.label;
                control.symbol = ("", 0);
            }
            if let Label::Symbol(name) = spec.label {
                let size = (rect.size.height * 0.34).clamp(13., 22.);
                let key = (name, size.round() as u32);
                if control.symbol != key {
                    control.button.setImage(sized_symbol(name, size).as_deref());
                    control.symbol = key;
                }
            }
            if control.prominent != spec.prominent {
                let accent = spec.prominent.then(NSColor::controlAccentColor);
                control.button.setBezelColor(accent.as_deref());
                control.prominent = spec.prominent;
            }
            if control.button.isEnabled() != spec.enabled {
                control.button.setEnabled(spec.enabled);
            }
            if control.tooltip != spec.tooltip {
                let tip = spec.tooltip.as_deref().map(string);
                control.button.setToolTip(tip.as_deref());
                control.tooltip = spec.tooltip.clone();
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
                    tooltip: None,
                });
                attach_bar(overlay, mtm);
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
            if dock.tooltip != state.tooltip {
                let tip = state.tooltip.as_deref().map(string);
                for view in [&*dock.art as &NSView, &dock.title, &dock.subtitle] {
                    view.setToolTip(tip.as_deref());
                }
                dock.tooltip = state.tooltip.clone();
            }
            dock.duration_ms = state.duration_ms;
            dock.remaining
                .setStringValue(&string(&timestamp(state.duration_ms)));
            dock.slider.setEnabled(state.duration_ms > 0);
            if !dock.scrubbing {
                dock.elapsed
                    .setStringValue(&string(&timestamp(state.position_ms)));
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

    pub fn dock_artwork() -> Option<Retained<NSImageView>> {
        OVERLAY.with_borrow(|overlay| overlay.dock.as_ref().map(|dock| dock.art.clone()))
    }

    pub fn volume(
        fraction: f32,
        corner: f32,
        enabled: bool,
        tooltip: Option<&str>,
        bounds: Bounds<Pixels>,
        window: &Window,
    ) {
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
                    tooltip: None,
                    placed: false,
                });
            }
            let Some(container) = container() else {
                return;
            };
            let volume = overlay.volume.as_mut().expect("volume was created");
            if volume.slider.isEnabled() != enabled {
                volume.slider.setEnabled(enabled);
                let tint = if enabled {
                    NSColor::secondaryLabelColor()
                } else {
                    NSColor::tertiaryLabelColor()
                };
                volume.icon.setContentTintColor(Some(&tint));
            }
            if volume.tooltip.as_deref() != tooltip {
                let tip = tooltip.map(string);
                volume.glass.setToolTip(tip.as_deref());
                volume.slider.setToolTip(tip.as_deref());
                volume.tooltip = tooltip.map(str::to_owned);
            }
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
                .setStringValue(&string(&timestamp(position_ms)));
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

    pub fn popup(entries: Vec<Entry>, at: Point<Pixels>) -> bool {
        schedule(entries, at, false)
    }

    pub fn popup_above(entries: Vec<Entry>, at: Point<Pixels>) -> bool {
        schedule(entries, at, true)
    }

    fn schedule(entries: Vec<Entry>, at: Point<Pixels>, above: bool) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        if container().is_none() {
            return false;
        }
        let delegate = OVERLAY.with_borrow_mut(|overlay| {
            overlay.pending = Some((entries, at, above));
            delegate(overlay, mtm)
        });
        unsafe {
            let _: () = msg_send![
                &*delegate,
                performSelector: sel!(showMenu:),
                withObject: None::<&AnyObject>,
                afterDelay: 0.0f64
            ];
        }
        true
    }

    fn show_pending() {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some(view) = container() else {
            return;
        };
        let Some((entries, at, above, delegate)) = OVERLAY.with_borrow_mut(|overlay| {
            let (entries, at, above) = overlay.pending.take()?;
            Some((entries, at, above, delegate(overlay, mtm)))
        }) else {
            return;
        };
        let menu = NSMenu::new(mtm);
        menu.setAutoenablesItems(false);
        menu.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let mut built = Built::default();
        populate(
            &menu,
            &entries,
            None,
            &delegate,
            sel!(menuItem:),
            &mut built,
            mtm,
        );
        OVERLAY.with_borrow_mut(|overlay| {
            overlay.context = Some(menu.clone());
            overlay.built = built;
        });
        let lift = if above { menu.size().height } else { 0. };
        let location = NSPoint::new(
            f64::from(f32::from(at.x)),
            view.bounds().size.height - f64::from(f32::from(at.y)) + lift,
        );
        menu.popUpMenuPositioningItem_atLocation_inView(None, location, Some(&view));
        OVERLAY.with_borrow_mut(|overlay| {
            if overlay.context.as_ref() == Some(&menu) {
                overlay.context = None;
                overlay.built = Built::default();
            }
        });
    }

    pub fn update_menu(id: u32, entries: Vec<Entry>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let found = OVERLAY.with_borrow_mut(|overlay| {
            if let Some((pending, ..)) = overlay.pending.as_mut()
                && replace(pending, id, &entries)
            {
                return None;
            }
            let menu = overlay.built.submenus.get(&id)?.clone();
            let search = overlay
                .built
                .searches
                .iter()
                .find(|(owner, _)| *owner == menu)
                .map(|(_, field)| field.clone());
            Some((menu, search, delegate(overlay, mtm)))
        });
        let Some((menu, search, delegate)) = found else {
            return;
        };
        let keep = isize::from(search.is_some());
        while menu.numberOfItems() > keep {
            menu.removeItemAtIndex(keep);
        }
        let mut built = Built::default();
        populate(
            &menu,
            &entries,
            search.as_deref(),
            &delegate,
            sel!(menuItem:),
            &mut built,
            mtm,
        );
        OVERLAY.with_borrow_mut(|overlay| {
            overlay.built.submenus.extend(built.submenus);
            overlay.built.searches.extend(built.searches);
        });
    }

    fn replace(entries: &mut [Entry], id: u32, fresh: &[Entry]) -> bool {
        entries.iter_mut().any(|entry| match entry {
            Entry::Submenu {
                id: Some(found),
                entries,
                ..
            } if *found == id => {
                *entries = fresh.to_vec();
                true
            }
            Entry::Submenu { entries, .. } => replace(entries, id, fresh),
            _ => false,
        })
    }

    pub fn bar_menu(generation: u32, entries: Vec<Entry>) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        OVERLAY.with_borrow_mut(|overlay| {
            overlay.bar_entries = entries;
            overlay.bar_generation = generation;
            attach_bar(overlay, mtm);
        });
    }

    pub fn bar_built() -> Option<u32> {
        OVERLAY.with_borrow(|overlay| overlay.bar_built)
    }

    fn attach_bar(overlay: &mut Overlay, mtm: MainThreadMarker) {
        let delegate = delegate(overlay, mtm);
        let menu = overlay
            .bar
            .get_or_insert_with(|| {
                let menu = NSMenu::new(mtm);
                menu.setAutoenablesItems(false);
                menu.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
                menu
            })
            .clone();
        let Some(dock) = overlay.dock.as_ref() else {
            return;
        };
        let menu = (!overlay.bar_entries.is_empty()).then_some(menu);
        unsafe { dock.glass.setMenu(menu.as_deref()) };
    }

    fn will_open(delegate: &MenuDelegate, menu: &NSMenu) {
        let field = OVERLAY.with_borrow_mut(|overlay| {
            if unsafe { menu.supermenu() }.is_none() {
                overlay.tracking = Some(menu.retain());
            }
            overlay
                .built
                .searches
                .iter()
                .find(|(owner, _)| **owner == *menu)
                .map(|(_, field)| field.clone())
        });
        let Some(field) = field else {
            return;
        };
        let modes = NSArray::from_slice(&[unsafe { NSEventTrackingRunLoopMode }]);
        unsafe {
            let _: () = msg_send![
                delegate,
                performSelector: sel!(focusField:),
                withObject: &*field,
                afterDelay: 0.0f64,
                inModes: &*modes
            ];
        }
    }

    fn did_close(menu: &NSMenu) {
        let closed = OVERLAY.with_borrow_mut(|overlay| {
            if overlay.tracking.as_deref() == Some(menu) {
                overlay.tracking = None;
            }
            overlay.context.as_deref() == Some(menu)
        });
        if closed {
            dispatch(Action::MenuClosed);
        }
    }

    fn needs_update(menu: &NSMenu) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let Some((entries, delegate)) = OVERLAY.with_borrow_mut(|overlay| {
            (overlay.bar.as_deref() == Some(menu)).then(|| {
                overlay.bar_built = Some(overlay.bar_generation);
                (overlay.bar_entries.clone(), delegate(overlay, mtm))
            })
        }) else {
            return;
        };
        menu.removeAllItems();
        let mut built = Built::default();
        populate(
            menu,
            &entries,
            None,
            &delegate,
            sel!(barMenuItem:),
            &mut built,
            mtm,
        );
        OVERLAY.with_borrow_mut(|overlay| overlay.built = built);
    }

    fn key_equivalent(key: &str) -> (String, NSEventModifierFlags) {
        let mut parts: Vec<&str> = key.split('-').collect();
        let name = parts.pop().unwrap_or_default();
        let mut mask = NSEventModifierFlags::empty();
        for modifier in parts {
            mask |= match modifier {
                "alt" => NSEventModifierFlags::Option,
                "ctrl" => NSEventModifierFlags::Control,
                "shift" => NSEventModifierFlags::Shift,
                "cmd" => NSEventModifierFlags::Command,
                _ => NSEventModifierFlags::empty(),
            };
        }
        let name = match name {
            "backspace" => "\u{8}".to_owned(),
            "delete" => "\u{7f}".to_owned(),
            "enter" => "\r".to_owned(),
            "space" => " ".to_owned(),
            name => name.to_lowercase(),
        };
        (name, mask)
    }

    fn menu_item(
        title: &str,
        tag: isize,
        key: Option<&str>,
        delegate: &MenuDelegate,
        action: Sel,
        mtm: MainThreadMarker,
    ) -> (Retained<NSMenuItem>, NSEventModifierFlags) {
        let (equivalent, mask) = key
            .map(key_equivalent)
            .unwrap_or_else(|| (String::new(), NSEventModifierFlags::empty()));
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &string(title),
                Some(action),
                &string(&equivalent),
            )
        };
        item.setKeyEquivalentModifierMask(mask);
        let receiver: &AnyObject = delegate;
        unsafe { item.setTarget(Some(receiver)) };
        item.setTag(tag);
        (item, mask)
    }

    fn note(text: &str, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&string(text));
        item.setEnabled(false);
        item
    }

    fn volume_item(value: f32, mtm: MainThreadMarker) -> Retained<NSMenuItem> {
        let target = OVERLAY.with_borrow_mut(|overlay| target(overlay, mtm));
        let receiver: &AnyObject = &target;
        let holder = NSView::initWithFrame(NSView::alloc(mtm), rect(0., 0., 264., 30.));
        holder.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        let speaker = |name: &str, frame: NSRect| {
            let image = NSImageView::imageViewWithImage(&symbol(name).unwrap_or_default(), mtm);
            image.setFrame(frame);
            image.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
            image
        };
        let quiet = speaker("speaker.fill", rect(14., 7., 16., 16.));
        let loud = speaker("speaker.wave.3.fill", rect(230., 7., 22., 16.));
        loud.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        let slider = unsafe {
            NSSlider::sliderWithValue_minValue_maxValue_target_action(
                f64::from(value.clamp(0., 1.)),
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
        slider.setFrame(rect(36., 5., 188., 20.));
        slider.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        holder.addSubview(&quiet);
        holder.addSubview(&slider);
        holder.addSubview(&loud);
        let item = NSMenuItem::new(mtm);
        item.setView(Some(&holder));
        item.setTag(VOLUME);
        item
    }

    fn search_item(
        placeholder: &str,
        delegate: &MenuDelegate,
        mtm: MainThreadMarker,
    ) -> (Retained<NSMenuItem>, Retained<NSSearchField>) {
        let field =
            NSSearchField::initWithFrame(NSSearchField::alloc(mtm), rect(14., 4., 236., 22.));
        field.setPlaceholderString(Some(&string(placeholder)));
        field.setSendsSearchStringImmediately(true);
        let receiver: &AnyObject = delegate;
        unsafe {
            field.setTarget(Some(receiver));
            field.setAction(Some(sel!(filter:)));
        }
        let holder = NSView::initWithFrame(NSView::alloc(mtm), rect(0., 0., 264., 30.));
        holder.addSubview(&field);
        let item = NSMenuItem::new(mtm);
        item.setView(Some(&holder));
        (item, field)
    }

    fn populate(
        menu: &NSMenu,
        entries: &[Entry],
        search: Option<&NSSearchField>,
        delegate: &MenuDelegate,
        action: Sel,
        built: &mut Built,
        mtm: MainThreadMarker,
    ) {
        for entry in entries {
            match entry {
                Entry::Item {
                    title,
                    id,
                    enabled,
                    checked,
                    symbol: image,
                    key,
                    alternate,
                    subtitle,
                } => {
                    let (item, mask) = menu_item(title, *id as isize, *key, delegate, action, mtm);
                    item.setEnabled(*enabled);
                    if *checked {
                        item.setState(NSControlStateValueOn);
                    }
                    if let Some(name) = image {
                        item.setImage(symbol(name).as_deref());
                    }
                    if let Some(subtitle) = subtitle {
                        item.setSubtitle(Some(&string(subtitle)));
                    }
                    menu.addItem(&item);
                    if let Some((title, id)) = alternate {
                        let (alternate, _) =
                            menu_item(title, *id as isize, *key, delegate, action, mtm);
                        alternate.setKeyEquivalentModifierMask(mask | NSEventModifierFlags::Option);
                        alternate.setAlternate(true);
                        alternate.setEnabled(*enabled);
                        menu.addItem(&alternate);
                    }
                }
                Entry::Note(text) => menu.addItem(&note(text, mtm)),
                Entry::Volume(value) => menu.addItem(&volume_item(*value, mtm)),
                Entry::Header(text) => {
                    menu.addItem(&NSMenuItem::sectionHeaderWithTitle(&string(text), mtm))
                }
                Entry::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
                Entry::Submenu {
                    title,
                    symbol: image,
                    entries,
                    search,
                    id,
                } => {
                    let item = NSMenuItem::new(mtm);
                    item.setTitle(&string(title));
                    if let Some(name) = image {
                        item.setImage(symbol(name).as_deref());
                    }
                    let submenu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &string(title));
                    submenu.setAutoenablesItems(false);
                    submenu.setDelegate(Some(ProtocolObject::from_ref(delegate)));
                    let field = search.map(|placeholder| {
                        let (item, field) = search_item(placeholder, delegate, mtm);
                        submenu.addItem(&item);
                        built.searches.push((submenu.clone(), field.clone()));
                        field
                    });
                    populate(
                        &submenu,
                        entries,
                        field.as_deref(),
                        delegate,
                        action,
                        built,
                        mtm,
                    );
                    item.setSubmenu(Some(&submenu));
                    menu.addItem(&item);
                    if let Some(id) = id {
                        built.submenus.insert(*id, submenu);
                    }
                }
            }
        }
        if let Some(field) = search {
            let empty = note("No matches", mtm);
            empty.setTag(NO_MATCH);
            menu.addItem(&empty);
            apply_filter(menu, &field.stringValue().to_string());
        }
    }

    fn filter_field(field: &NSSearchField) {
        let menu = OVERLAY.with_borrow(|overlay| {
            overlay
                .built
                .searches
                .iter()
                .find(|(_, owned)| **owned == *field)
                .map(|(menu, _)| menu.clone())
        });
        if let Some(menu) = menu {
            apply_filter(&menu, &field.stringValue().to_string());
        }
    }

    fn apply_filter(menu: &NSMenu, text: &str) {
        let text = text.trim().to_lowercase();
        let mut matched = false;
        let mut empty = None;
        for index in 0..menu.numberOfItems() {
            let Some(item) = menu.itemAtIndex(index) else {
                continue;
            };
            if item.view().is_some() {
                continue;
            }
            if item.tag() == NO_MATCH && !item.isEnabled() {
                empty = Some(item);
                continue;
            }
            let show = text.is_empty()
                || !item.isSeparatorItem()
                    && item.title().to_string().to_lowercase().contains(&text);
            item.setHidden(!show);
            matched |= show && !item.isSeparatorItem();
        }
        if let Some(empty) = empty {
            empty.setHidden(matched);
        }
    }

    fn describe_item(
        item: &NSMenuItem,
        searches: &[(Retained<NSMenu>, Retained<NSSearchField>)],
    ) -> String {
        if item.isSeparatorItem() {
            return "──".to_owned();
        }
        if let Some(view) = item.view().filter(|_| item.tag() == VOLUME) {
            let value = view
                .viewWithTag(VOLUME)
                .and_then(|slider| slider.downcast::<NSSlider>().ok())
                .map_or(0., |slider| slider.doubleValue());
            return format!("[volume {value:.2}]");
        }
        if item.view().is_some() {
            let text = searches
                .iter()
                .find(|(menu, _)| unsafe { item.menu() }.as_deref() == Some(&**menu))
                .map(|(_, field)| field.stringValue().to_string())
                .unwrap_or_default();
            return format!("[search “{text}”]");
        }
        let mut line = item.title().to_string();
        let equivalent = item.keyEquivalent().to_string();
        if !equivalent.is_empty() {
            let mask = item.keyEquivalentModifierMask();
            let mut shortcut = String::new();
            for (flag, glyph) in [
                (NSEventModifierFlags::Control, "⌃"),
                (NSEventModifierFlags::Option, "⌥"),
                (NSEventModifierFlags::Shift, "⇧"),
                (NSEventModifierFlags::Command, "⌘"),
            ] {
                if mask.contains(flag) {
                    shortcut.push_str(glyph);
                }
            }
            shortcut.push_str(&match equivalent.as_str() {
                "\u{8}" => "⌫".to_owned(),
                key => key.to_uppercase(),
            });
            line.push_str(&format!("  {shortcut}"));
        }
        if let Some(subtitle) = item.subtitle() {
            line.push_str(&format!("  — {subtitle}"));
        }
        for (flag, mark) in [
            (item.isSectionHeader(), " #header"),
            (item.isAlternate(), " (⌥)"),
            (!item.isEnabled(), " [x]"),
            (item.state() == NSControlStateValueOn, " ✓"),
            (item.isHidden(), " (hidden)"),
            (item.submenu().is_some(), " ▸"),
        ] {
            if flag {
                line.push_str(mark);
            }
        }
        line
    }

    fn tree(menu: &NSMenu, depth: usize, out: &mut Vec<String>) {
        let searches = OVERLAY.with_borrow(|overlay| overlay.built.searches.clone());
        for index in 0..menu.numberOfItems() {
            let Some(item) = menu.itemAtIndex(index) else {
                continue;
            };
            out.push(format!(
                "{}{index}: {}",
                "  ".repeat(depth),
                describe_item(&item, &searches)
            ));
            if let Some(submenu) = item.submenu() {
                tree(&submenu, depth + 1, out);
            }
        }
    }

    pub fn menu_tree() -> Option<String> {
        let menu = OVERLAY.with_borrow(|overlay| overlay.tracking.clone())?;
        let mut lines = Vec::new();
        tree(&menu, 0, &mut lines);
        Some(lines.join("\n"))
    }

    pub fn bar_menu_tree() -> Option<String> {
        let menu = OVERLAY
            .with_borrow(|overlay| overlay.dock.as_ref().and_then(|dock| dock.glass.menu()))?;
        needs_update(&menu);
        let mut lines = Vec::new();
        tree(&menu, 0, &mut lines);
        Some(lines.join("\n"))
    }

    pub fn menu_bar_tree() -> Option<String> {
        let mtm = MainThreadMarker::new()?;
        let menu = NSApplication::sharedApplication(mtm).mainMenu()?;
        let mut lines = Vec::new();
        tree(&menu, 0, &mut lines);
        Some(lines.join("\n"))
    }

    pub fn pick(path: &str) -> bool {
        let Some(mut menu) = OVERLAY.with_borrow(|overlay| overlay.tracking.clone()) else {
            return false;
        };
        let root = menu.clone();
        let indices: Vec<isize> = path
            .split('.')
            .filter_map(|part| part.parse().ok())
            .collect();
        let Some((last, walk)) = indices.split_last() else {
            return false;
        };
        for index in walk {
            let Some(submenu) = menu.itemAtIndex(*index).and_then(|item| item.submenu()) else {
                return false;
            };
            menu = submenu;
        }
        if *last >= menu.numberOfItems() {
            return false;
        }
        menu.performActionForItemAtIndex(*last);
        root.cancelTracking();
        true
    }

    pub fn close_menu() -> bool {
        let Some(menu) = OVERLAY.with_borrow(|overlay| overlay.tracking.clone()) else {
            return false;
        };
        menu.cancelTracking();
        true
    }

    pub fn filter_menu(text: &str) -> bool {
        let Some((menu, field)) =
            OVERLAY.with_borrow(|overlay| overlay.built.searches.first().cloned())
        else {
            return false;
        };
        field.setStringValue(&string(text));
        apply_filter(&menu, text);
        true
    }

    pub fn right_click(x: f64, y: f64) -> bool {
        appkit::right_click(x, y)
    }

    pub fn drag(from: (f64, f64), to: (f64, f64)) -> bool {
        appkit::drag(from, to)
    }

    pub fn nudge_window(dx: f64, dy: f64) -> bool {
        appkit::nudge_window(dx, dy)
    }

    pub fn describe() -> Vec<String> {
        let mut lines = Vec::new();
        OVERLAY.with_borrow(|overlay| {
            let mut keys: Vec<_> = overlay.buttons.keys().copied().collect();
            keys.sort_unstable();
            for key in keys {
                let control = &overlay.buttons[key];
                let frame = control.button.frame();
                lines.push(format!(
                    "button {key} label={:?} prominent={} enabled={} tooltip={:?} hidden={} frame={},{},{},{}",
                    control.label,
                    control.prominent,
                    control.button.isEnabled(),
                    control.tooltip,
                    control.button.isHidden(),
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height,
                ));
            }
            lines.push(format!(
                "bar entries={} attached={}",
                overlay.bar_entries.len(),
                overlay
                    .dock
                    .as_ref()
                    .is_some_and(|dock| dock.glass.menu().is_some())
            ));
            if let Some(dock) = overlay.dock.as_ref() {
                lines.push(format!(
                    "dock elapsed={} slider={:.4}",
                    dock.elapsed.stringValue(),
                    dock.slider.doubleValue()
                ));
            }
            if let Some(volume) = overlay.volume.as_ref() {
                lines.push(format!(
                    "volume value={:.4} enabled={} tooltip={:?} hidden={}",
                    volume.slider.doubleValue(),
                    volume.slider.isEnabled(),
                    volume.tooltip,
                    volume.glass.isHidden(),
                ));
            }
        });
        lines
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use gpui::{Bounds, Pixels, Point, Window};

    use super::{Action, ButtonSpec, Entry, NowPlaying};

    pub fn available() -> bool {
        false
    }

    pub fn set_handler(_: impl Fn(Action) + 'static) {}

    pub fn key(_: &str) -> Option<&'static str> {
        None
    }

    pub fn button(_: &'static str, _: &ButtonSpec, _: Bounds<Pixels>, _: &Window) {}

    pub fn now_playing(_: &NowPlaying, _: Bounds<Pixels>, _: &Window) {}

    pub fn volume(_: f32, _: f32, _: bool, _: Option<&str>, _: Bounds<Pixels>, _: &Window) {}

    pub fn finish() {}

    pub fn progress(_: u32, _: u32) {}

    pub fn popup(_: Vec<Entry>, _: Point<Pixels>) -> bool {
        false
    }

    pub fn popup_above(_: Vec<Entry>, _: Point<Pixels>) -> bool {
        false
    }

    pub fn update_menu(_: u32, _: Vec<Entry>) {}

    pub fn bar_menu(_: u32, _: Vec<Entry>) {}

    pub fn bar_built() -> Option<u32> {
        None
    }

    pub fn menu_tree() -> Option<String> {
        None
    }

    pub fn bar_menu_tree() -> Option<String> {
        None
    }

    pub fn menu_bar_tree() -> Option<String> {
        None
    }

    pub fn pick(_: &str) -> bool {
        false
    }

    pub fn close_menu() -> bool {
        false
    }

    pub fn filter_menu(_: &str) -> bool {
        false
    }

    pub fn right_click(_: f64, _: f64) -> bool {
        false
    }

    pub fn drag(_: (f64, f64), _: (f64, f64)) -> bool {
        false
    }

    pub fn nudge_window(_: f64, _: f64) -> bool {
        false
    }

    pub fn describe() -> Vec<String> {
        Vec::new()
    }
}
