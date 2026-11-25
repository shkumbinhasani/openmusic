pub use native::{describe, hide, is_visible, progress, resize, set_hover, show, update};

pub const AUTOSAVE: &str = "OpenMusic Mini Player";
pub const DEFAULT_SIDE: f64 = 300.;
pub const MIN_WIDTH: f64 = 240.;
pub const MIN_HEIGHT: f64 = 64.;
pub const STRIP_MAX: f64 = 132.;
pub const MEDIUM_MAX: f64 = 170.;

pub const PREVIOUS: &str = "mini-previous";
pub const TOGGLE: &str = "mini-toggle";
pub const NEXT: &str = "mini-next";
pub const LIKE: &str = "mini-like";
pub const CLOSE: &str = "mini-close";
pub const EXPAND: &str = "mini-expand";
pub const KEYS: [&str; 6] = [PREVIOUS, TOGGLE, NEXT, LIKE, CLOSE, EXPAND];

const CHROME: f64 = 22.;
const CHROME_INSET: f64 = 8.;
const GAP: f64 = 4.;
const EDGE: f64 = 3.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Strip,
    Medium,
    Artwork,
}

impl Layout {
    pub fn for_height(height: f64) -> Self {
        if height <= STRIP_MAX {
            Layout::Strip
        } else if height <= MEDIUM_MAX {
            Layout::Medium
        } else {
            Layout::Artwork
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Layout::Strip => "strip",
            Layout::Medium => "medium",
            Layout::Artwork => "artwork",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MiniState {
    pub title: String,
    pub artist: String,
    pub device: Option<String>,
    pub playing: bool,
    pub saved: Option<bool>,
    pub likable: bool,
    pub cover: Option<String>,
    pub duration_ms: u32,
}

impl MiniState {
    pub fn toggle_symbol(&self) -> &'static str {
        if self.playing {
            "pause.fill"
        } else {
            "play.fill"
        }
    }

    pub fn like_symbol(&self) -> &'static str {
        if self.saved == Some(true) {
            "heart.fill"
        } else {
            "heart"
        }
    }

    pub fn like_tooltip(&self) -> &'static str {
        if self.saved == Some(true) {
            "Remove from Liked Songs"
        } else {
            "Save to Liked Songs"
        }
    }

    pub fn device_line(&self) -> Option<String> {
        self.device
            .as_deref()
            .filter(|device| !device.is_empty())
            .map(|device| format!("Playing on {device}"))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn top(&self) -> f64 {
        self.y + self.h
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.top().max(other.top()) - y,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Frames {
    pub layout: Layout,
    pub art: Rect,
    pub art_radius: f64,
    pub card: Option<Rect>,
    pub title: Rect,
    pub subtitle: Rect,
    pub device: Option<Rect>,
    pub previous: Rect,
    pub toggle: Rect,
    pub next: Rect,
    pub like: Option<Rect>,
    pub button: f64,
    pub slider: Option<Rect>,
    pub edge: Option<Rect>,
    pub close: Rect,
    pub expand: Rect,
    pub hover_only: bool,
}

impl Frames {
    pub fn text(&self) -> Rect {
        let text = self.title.union(&self.subtitle);
        self.device.map_or(text, |device| text.union(&device))
    }
}

fn chrome(height: f64) -> (Rect, Rect) {
    let y = height - CHROME_INSET - CHROME;
    (
        Rect::new(CHROME_INSET, y, CHROME, CHROME),
        Rect::new(CHROME_INSET + CHROME + 6., y, CHROME, CHROME),
    )
}

fn strip(width: f64, height: f64, remote: bool) -> Frames {
    let pad = 8.;
    let side = (height - 2. * pad).min(width * 0.2).max(32.);
    let button = (height * 0.45).clamp(26., 34.).min(width * 0.12);
    let right = width - 10.;
    let text_x = pad + side + 10.;
    let cluster = |count: f64| count * button + (count - 1.) * GAP;
    let with_like = right - cluster(4.) - 10. - text_x >= 72.;
    let cluster_x = right - cluster(if with_like { 4. } else { 3. });
    let text_w = (cluster_x - 10. - text_x).max(0.);
    let y = (height - button) / 2.;
    let slot = |index: f64| Rect::new(cluster_x + index * (button + GAP), y, button, button);
    let first = if with_like { 1. } else { 0. };
    let three = remote && height >= 96.;
    let middle = height / 2.;
    let (title, subtitle, device) = if three {
        (
            Rect::new(text_x, middle + 8., text_w, 17.),
            Rect::new(text_x, middle - 9., text_w, 16.),
            Some(Rect::new(text_x, middle - 25., text_w, 15.)),
        )
    } else {
        (
            Rect::new(text_x, middle + 1., text_w, 17.),
            Rect::new(text_x, middle - 17., text_w, 16.),
            None,
        )
    };
    let (close, expand) = chrome(height);
    Frames {
        layout: Layout::Strip,
        art: Rect::new(pad, (height - side) / 2., side, side),
        art_radius: 6.,
        card: None,
        title,
        subtitle,
        device,
        previous: slot(first),
        toggle: slot(first + 1.),
        next: slot(first + 2.),
        like: with_like.then(|| slot(0.)),
        button,
        slider: None,
        edge: Some(Rect::new(0., 0., width, EDGE)),
        close,
        expand,
        hover_only: false,
    }
}

fn medium(width: f64, height: f64, remote: bool) -> Frames {
    let pad = 12.;
    let side = (height - 2. * pad).min(width * 0.4);
    let column = pad + side + 12.;
    let column_w = (width - column - pad).max(0.);
    let button = ((column_w - 2. * GAP) / 3.).min(30.);
    let controls_y = pad + 20. + 6.;
    let slot = |index: f64| Rect::new(column + index * (button + GAP), controls_y, button, button);
    let with_like = column_w >= 4. * button + 3. * GAP + 6.;
    let title = Rect::new(column, height - pad - 17., column_w, 17.);
    let subtitle = Rect::new(column, title.y - 17., column_w, 16.);
    let device = remote.then(|| Rect::new(column, subtitle.y - 16., column_w, 15.));
    let (close, expand) = chrome(height);
    Frames {
        layout: Layout::Medium,
        art: Rect::new(pad, height - pad - side, side, side),
        art_radius: 8.,
        card: None,
        title,
        subtitle,
        device,
        previous: slot(0.),
        toggle: slot(1.),
        next: slot(2.),
        like: with_like.then(|| Rect::new(column + column_w - button, controls_y, button, button)),
        button,
        slider: Some(Rect::new(column, pad, column_w, 20.)),
        edge: None,
        close,
        expand,
        hover_only: false,
    }
}

fn artwork(width: f64, height: f64, remote: bool) -> Frames {
    let side = width.max(height);
    let margin = 8.;
    let inner = 12.;
    let card_h = 112. + if remote { 16. } else { 0. };
    let card = Rect::new(margin, margin, width - 2. * margin, card_h);
    let text_x = margin + inner;
    let text_w = (width - 2. * (margin + inner)).max(0.);
    let title = Rect::new(text_x, card.top() - 10. - 17., text_w, 17.);
    let subtitle = Rect::new(text_x, title.y - 17., text_w, 16.);
    let device = remote.then(|| Rect::new(text_x, subtitle.y - 16., text_w, 15.));
    let button = 32.;
    let gap = 6.;
    let transport = 3. * button + 2. * gap;
    let start = (width - transport) / 2.;
    let controls_y = margin + 32.;
    let slot = |index: f64| Rect::new(start + index * (button + gap), controls_y, button, button);
    let like_x = card.right() - inner - button;
    let (close, expand) = chrome(height);
    Frames {
        layout: Layout::Artwork,
        art: Rect::new((width - side) / 2., (height - side) / 2., side, side),
        art_radius: 0.,
        card: Some(card),
        title,
        subtitle,
        device,
        previous: slot(0.),
        toggle: slot(1.),
        next: slot(2.),
        like: (like_x >= start + transport + gap)
            .then(|| Rect::new(like_x, controls_y, button, button)),
        button,
        slider: Some(Rect::new(text_x, margin + 8., text_w, 20.)),
        edge: Some(Rect::new(0., 0., width, EDGE)),
        close,
        expand,
        hover_only: true,
    }
}

pub fn frames(width: f64, height: f64, remote: bool) -> Frames {
    let width = width.max(MIN_WIDTH);
    let height = height.max(MIN_HEIGHT);
    match Layout::for_height(height) {
        Layout::Strip => strip(width, height, remote),
        Layout::Medium => medium(width, height, remote),
        Layout::Artwork => artwork(width, height, remote),
    }
}

pub fn symbol_size(button: f64) -> f64 {
    (button * 0.42).clamp(12., 17.)
}

#[cfg(target_os = "macos")]
mod native {
    use std::{cell::RefCell, ptr::NonNull, sync::Arc};

    use block2::RcBlock;
    use gpui::Image;
    use objc2::{
        AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject, NSObject, ProtocolObject},
        sel,
    };
    use objc2_app_kit::{
        NSApplication, NSAutoresizingMaskOptions, NSBackingStoreType, NSBezelStyle, NSButton,
        NSCellImagePosition, NSColor, NSControl, NSControlBorderShape, NSControlSize, NSEvent,
        NSEventType, NSFloatingWindowLevel, NSGlassEffectView, NSImage, NSImageScaling,
        NSImageView, NSPanel, NSResponder, NSSlider, NSTextField, NSTrackingArea,
        NSTrackingAreaOptions, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindowButton, NSWindowCollectionBehavior,
        NSWindowDidResizeNotification, NSWindowStyleMask, NSWindowTitleVisibility,
    };
    use objc2_foundation::{
        NSArray, NSData, NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue,
        NSPoint, NSRect, NSSize,
    };

    use super::{
        AUTOSAVE, CLOSE, DEFAULT_SIDE, EXPAND, Frames, KEYS, LIKE, Layout, MIN_HEIGHT, MIN_WIDTH,
        MiniState, NEXT, PREVIOUS, Rect, TOGGLE, frames, symbol_size,
    };
    use crate::{
        appkit::{Target, dispatch, label, sized_symbol, string},
        overlay::Action,
    };

    const SEEK: isize = -1;

    define_class!(
        #[unsafe(super(NSView, NSResponder, NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "OpenMusicMiniPlayerView"]
        struct HoverView;

        unsafe impl NSObjectProtocol for HoverView {}

        impl HoverView {
            #[unsafe(method(mouseEntered:))]
            fn mouse_entered(&self, _event: &NSEvent) {
                set_hover(true);
            }

            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _event: &NSEvent) {
                set_hover(false);
            }
        }
    );

    impl HoverView {
        fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            unsafe { msg_send![super(this), initWithFrame: frame] }
        }
    }

    struct Button {
        key: &'static str,
        view: Retained<NSButton>,
        symbol: (&'static str, u32),
    }

    struct Mini {
        panel: Retained<NSPanel>,
        art: Retained<NSImageView>,
        card: Retained<NSView>,
        title: Retained<NSTextField>,
        subtitle: Retained<NSTextField>,
        device: Retained<NSTextField>,
        title_button: Retained<NSButton>,
        buttons: Vec<Button>,
        slider: Retained<NSSlider>,
        edge: Retained<NSView>,
        _target: Retained<Target>,
        _root: Retained<HoverView>,
        _tracking: Retained<NSTrackingArea>,
        _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
        glass: bool,
        state: MiniState,
        shown_cover: Option<String>,
        frames: Option<Frames>,
        position_ms: u32,
        hover: bool,
        scrubbing: bool,
    }

    thread_local! {
        static MINI: RefCell<Option<Mini>> = RefCell::default();
    }

    fn with_mini<R>(run: impl FnOnce(&mut Mini) -> R) -> Option<R> {
        MINI.with(|cell| {
            let mut mini = cell.try_borrow_mut().ok()?;
            mini.as_mut().map(run)
        })
    }

    fn ns(rect: Rect) -> NSRect {
        NSRect::new(
            NSPoint::new(rect.x, rect.y),
            NSSize::new(rect.w.max(0.), rect.h),
        )
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

    fn activate(sender: &NSControl) {
        let tag = sender.tag();
        if tag == SEEK {
            let scrubbing = dragging();
            with_mini(|mini| mini.scrubbing = scrubbing);
            if !scrubbing {
                dispatch(Action::Seek(sender.doubleValue() as f32));
            }
            return;
        }
        if let Some(key) = usize::try_from(tag).ok().and_then(|tag| KEYS.get(tag)) {
            dispatch(Action::Press(key));
        }
    }

    fn glass_available() -> bool {
        AnyClass::get(c"NSGlassEffectView").is_some()
    }

    fn round(view: &NSView, radius: f64) {
        view.setWantsLayer(true);
        if let Some(layer) = view.layer() {
            layer.setCornerRadius(radius);
            layer.setMasksToBounds(radius > 0.);
        }
    }

    fn button(
        key: &'static str,
        tooltip: &str,
        target: &Target,
        glass: bool,
        mtm: MainThreadMarker,
    ) -> Button {
        let receiver: &AnyObject = target;
        let view = unsafe {
            NSButton::buttonWithImage_target_action(
                &NSImage::new(),
                Some(receiver),
                Some(sel!(activate:)),
                mtm,
            )
        };
        if glass {
            view.setBezelStyle(NSBezelStyle::Glass);
            view.setBorderShape(NSControlBorderShape::Circle);
            view.setControlSize(NSControlSize::Regular);
        } else {
            view.setBordered(false);
        }
        view.setImagePosition(NSCellImagePosition::ImageOnly);
        view.setImageScaling(NSImageScaling::ScaleNone);
        let tag = KEYS.iter().position(|known| *known == key).unwrap_or(0);
        view.setTag(tag as isize);
        view.setToolTip(Some(&string(tooltip)));
        Button {
            key,
            view,
            symbol: ("", 0),
        }
    }

    fn observe_resize(panel: &NSPanel) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
        let block = RcBlock::new(|_: NonNull<NSNotification>| {
            with_mini(layout);
        });
        let object: &AnyObject = panel;
        unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSWindowDidResizeNotification),
                Some(object),
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        }
    }

    fn build(mtm: MainThreadMarker) -> Mini {
        let glass = glass_available();
        let start = NSRect::new(NSPoint::ZERO, NSSize::new(DEFAULT_SIDE, DEFAULT_SIDE));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            start,
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Resizable
                | NSWindowStyleMask::FullSizeContentView
                | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        panel.setFloatingPanel(true);
        panel.setBecomesKeyOnlyIfNeeded(true);
        panel.setLevel(NSFloatingWindowLevel);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        panel.setHidesOnDeactivate(false);
        panel.setMovableByWindowBackground(true);
        panel.setTitlebarAppearsTransparent(true);
        panel.setTitleVisibility(NSWindowTitleVisibility::Hidden);
        panel.setTitle(&string(AUTOSAVE));
        panel.setContentMinSize(NSSize::new(MIN_WIDTH, MIN_HEIGHT));
        unsafe { panel.setReleasedWhenClosed(false) };
        for kind in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(control) = panel.standardWindowButton(kind) {
                control.setHidden(true);
            }
        }

        let root = HoverView::new(mtm, start);
        panel.setContentView(Some(&root));
        let fill = NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewHeightSizable;
        let backdrop = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), start);
        backdrop.setMaterial(NSVisualEffectMaterial::WindowBackground);
        backdrop.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        backdrop.setState(NSVisualEffectState::Active);
        backdrop.setAutoresizingMask(fill);
        root.addSubview(&backdrop);

        let art = NSImageView::imageViewWithImage(&NSImage::new(), mtm);
        art.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
        art.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        round(&art, 8.);

        let card: Retained<NSView> = if glass {
            let card =
                NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), NSRect::ZERO);
            card.setCornerRadius(16.);
            Retained::into_super(card)
        } else {
            let card =
                NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), NSRect::ZERO);
            card.setMaterial(NSVisualEffectMaterial::HUDWindow);
            card.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
            card.setState(NSVisualEffectState::Active);
            round(&card, 16.);
            Retained::into_super(card)
        };

        let title = label("", 13., true, mtm);
        let subtitle = label("", 12., false, mtm);
        subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let device = label("", 11., false, mtm);
        device.setTextColor(Some(&NSColor::controlAccentColor()));

        let target = Target::new(mtm, activate);
        let receiver: &AnyObject = &target;
        let title_button = unsafe {
            NSButton::buttonWithTitle_target_action(
                &string(""),
                Some(receiver),
                Some(sel!(activate:)),
                mtm,
            )
        };
        title_button.setBordered(false);
        title_button.setTransparent(true);
        title_button.setTag(KEYS.iter().position(|key| *key == EXPAND).unwrap_or(0) as isize);
        title_button.setToolTip(Some(&string("Show OpenMusic")));

        let buttons = vec![
            button(PREVIOUS, "Previous", &target, glass, mtm),
            button(TOGGLE, "Play", &target, glass, mtm),
            button(NEXT, "Next", &target, glass, mtm),
            button(LIKE, "Save to Liked Songs", &target, glass, mtm),
            button(CLOSE, "Close Mini Player", &target, glass, mtm),
            button(EXPAND, "Show OpenMusic", &target, glass, mtm),
        ];

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

        let edge = NSView::initWithFrame(NSView::alloc(mtm), NSRect::ZERO);
        edge.setWantsLayer(true);
        if let Some(layer) = edge.layer() {
            layer.setBackgroundColor(Some(&NSColor::controlAccentColor().CGColor()));
        }

        for view in [
            &*art as &NSView,
            &card,
            &title,
            &subtitle,
            &device,
            &title_button,
        ] {
            root.addSubview(view);
        }
        for button in &buttons {
            root.addSubview(&button.view);
        }
        root.addSubview(&slider);
        root.addSubview(&edge);

        let tracking = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&root),
                None,
            )
        };
        root.addTrackingArea(&tracking);
        let observers = vec![observe_resize(&panel)];

        let name = string(AUTOSAVE);
        if !panel.setFrameUsingName(&name) {
            panel.center();
        }
        panel.setFrameAutosaveName(&name);

        Mini {
            panel,
            art,
            card,
            title,
            subtitle,
            device,
            title_button,
            buttons,
            slider,
            edge,
            _target: target,
            _root: root,
            _tracking: tracking,
            _observers: observers,
            glass,
            state: MiniState::default(),
            shown_cover: None,
            frames: None,
            position_ms: 0,
            hover: false,
            scrubbing: false,
        }
    }

    fn screens() -> Vec<NSRect> {
        let Some(class) = AnyClass::get(c"NSScreen") else {
            return Vec::new();
        };
        let screens: Option<Retained<NSArray<AnyObject>>> = unsafe { msg_send![class, screens] };
        screens.map_or_else(Vec::new, |screens| {
            screens
                .iter()
                .map(|screen| unsafe { msg_send![&*screen, visibleFrame] })
                .collect()
        })
    }

    fn intersects(a: NSRect, b: NSRect) -> bool {
        a.origin.x < b.origin.x + b.size.width
            && b.origin.x < a.origin.x + a.size.width
            && a.origin.y < b.origin.y + b.size.height
            && b.origin.y < a.origin.y + a.size.height
    }

    fn keep_on_screen(panel: &NSPanel) {
        let frame = panel.frame();
        let screens = screens();
        if !screens.is_empty() && !screens.iter().any(|screen| intersects(frame, *screen)) {
            panel.center();
        }
    }

    fn set_symbol(button: &mut Button, name: &'static str, size: f64) {
        let key = (name, size.round() as u32);
        if button.symbol != key {
            button.view.setImage(sized_symbol(name, size).as_deref());
            button.symbol = key;
        }
    }

    fn content_size(mini: &Mini) -> (f64, f64) {
        let Some(view) = mini.panel.contentView() else {
            return (DEFAULT_SIDE, DEFAULT_SIDE);
        };
        let size = view.bounds().size;
        (size.width, size.height)
    }

    fn place(view: &NSView, rect: Option<Rect>, shown: bool) {
        match rect {
            Some(rect) if shown => {
                view.setFrame(ns(rect));
                view.setHidden(false);
            }
            _ => view.setHidden(true),
        }
    }

    fn fraction(mini: &Mini) -> f64 {
        f64::from(mini.position_ms) / f64::from(mini.state.duration_ms.max(1))
    }

    fn place_edge(mini: &Mini) {
        let Some(frames) = mini.frames.as_ref() else {
            return;
        };
        let shown = frames.edge.is_some() && !(frames.hover_only && mini.hover);
        let edge = frames.edge.map(|edge| Rect {
            w: edge.w * fraction(mini).clamp(0., 1.),
            ..edge
        });
        place(&mini.edge, edge, shown && mini.state.duration_ms > 0);
    }

    fn layout(mini: &mut Mini) {
        let (width, height) = content_size(mini);
        let remote = mini.state.device_line().is_some();
        let frames = frames(width, height, remote);
        let body = !frames.hover_only || mini.hover;
        mini.art.setFrame(ns(frames.art));
        round(&mini.art, frames.art_radius);
        place(&mini.card, frames.card, body);
        if mini.card.isHidden() {
            mini.card.setFrame(NSRect::ZERO);
        }
        place(&mini.title, Some(frames.title), body);
        place(&mini.subtitle, Some(frames.subtitle), body);
        place(&mini.device, frames.device, body && remote);
        place(&mini.title_button, Some(frames.text()), body);
        place(&mini.slider, frames.slider, body);
        let size = symbol_size(frames.button);
        let state = mini.state.clone();
        for button in &mut mini.buttons {
            let (rect, shown, name, size) = match button.key {
                PREVIOUS => (Some(frames.previous), body, "backward.fill", size),
                TOGGLE => (Some(frames.toggle), body, state.toggle_symbol(), size + 2.),
                NEXT => (Some(frames.next), body, "forward.fill", size),
                LIKE => (frames.like, body, state.like_symbol(), size),
                CLOSE => (Some(frames.close), mini.hover, "xmark", 10.),
                _ => (
                    Some(frames.expand),
                    mini.hover,
                    "arrow.up.left.and.arrow.down.right",
                    10.,
                ),
            };
            set_symbol(button, name, size);
            place(&button.view, rect, shown);
        }
        mini.frames = Some(frames);
        place_edge(mini);
    }

    fn apply(mini: &mut Mini, state: &MiniState, cover: Option<&Arc<Image>>) {
        if mini.state.title != state.title {
            mini.title.setStringValue(&string(&state.title));
        }
        if mini.state.artist != state.artist {
            mini.subtitle.setStringValue(&string(&state.artist));
        }
        let line = state.device_line().unwrap_or_default();
        if mini.state.device_line().unwrap_or_default() != line {
            mini.device.setStringValue(&string(&line));
        }
        let loaded = state.cover.clone().filter(|_| cover.is_some());
        if mini.shown_cover != loaded || mini.frames.is_none() {
            let picture = cover.and_then(|image| {
                NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(image.bytes()))
            });
            match picture {
                Some(picture) => {
                    mini.art
                        .setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                    mini.art.setImage(Some(&picture));
                }
                None => {
                    mini.art.setImageScaling(NSImageScaling::ScaleNone);
                    mini.art
                        .setImage(sized_symbol("music.note", 28.).as_deref());
                }
            }
            mini.shown_cover = loaded;
        }
        for button in &mini.buttons {
            match button.key {
                TOGGLE => {
                    let tip = if state.playing { "Pause" } else { "Play" };
                    button.view.setToolTip(Some(&string(tip)));
                }
                LIKE => {
                    button.view.setEnabled(state.likable);
                    button.view.setToolTip(Some(&string(state.like_tooltip())));
                    let tint = (state.saved == Some(true)).then(NSColor::controlAccentColor);
                    button.view.setContentTintColor(tint.as_deref());
                }
                PREVIOUS | NEXT => button.view.setEnabled(state.duration_ms > 0),
                _ => {}
            }
        }
        mini.slider.setEnabled(state.duration_ms > 0);
        mini.state = state.clone();
    }

    fn set_progress(mini: &mut Mini, position_ms: u32) {
        mini.position_ms = position_ms;
        if !mini.scrubbing {
            mini.slider.setDoubleValue(fraction(mini).clamp(0., 1.));
        }
        place_edge(mini);
    }

    pub fn show(state: &MiniState, cover: Option<&Arc<Image>>, position_ms: u32) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let missing = MINI.with(|cell| cell.try_borrow().is_ok_and(|mini| mini.is_none()));
        if missing {
            let built = build(mtm);
            MINI.with(|cell| {
                if let Ok(mut mini) = cell.try_borrow_mut() {
                    *mini = Some(built);
                }
            });
        }
        with_mini(|mini| {
            apply(mini, state, cover);
            keep_on_screen(&mini.panel);
            let pointer = NSEvent::mouseLocation();
            mini.hover = intersects(
                mini.panel.frame(),
                NSRect::new(pointer, NSSize::new(1., 1.)),
            );
            layout(mini);
            set_progress(mini, position_ms);
            mini.panel.orderFrontRegardless();
        })
        .is_some()
    }

    pub fn hide() {
        with_mini(|mini| {
            mini.panel.orderOut(None);
            mini.hover = false;
            mini.scrubbing = false;
        });
    }

    pub fn is_visible() -> bool {
        MINI.with(|cell| {
            cell.try_borrow()
                .is_ok_and(|mini| mini.as_ref().is_some_and(|mini| mini.panel.isVisible()))
        })
    }

    pub fn update(state: &MiniState, cover: Option<&Arc<Image>>) {
        with_mini(|mini| {
            let relayout = mini.state.device_line().is_some() != state.device_line().is_some()
                || mini.state.playing != state.playing
                || mini.state.saved != state.saved;
            apply(mini, state, cover);
            if relayout {
                layout(mini);
            }
        });
    }

    pub fn progress(position_ms: u32) {
        with_mini(|mini| {
            if mini.panel.isVisible() {
                set_progress(mini, position_ms);
            }
        });
    }

    pub fn set_hover(hover: bool) {
        with_mini(|mini| {
            if mini.hover != hover {
                mini.hover = hover;
                layout(mini);
            }
        });
    }

    pub fn resize(width: f64, height: f64) {
        with_mini(|mini| {
            mini.panel.setContentSize(NSSize::new(width, height));
            layout(mini);
        });
    }

    pub fn describe() -> String {
        with_mini(|mini| {
            let frame = mini.panel.frame();
            let (width, height) = content_size(mini);
            let mut lines = vec![format!(
                "visible={} layout={} size={}x{} origin={},{} hover={} glass={} scrubbing={}",
                mini.panel.isVisible(),
                Layout::for_height(height).name(),
                width,
                height,
                frame.origin.x,
                frame.origin.y,
                mini.hover,
                mini.glass,
                mini.scrubbing,
            )];
            lines.push(format!(
                "title={:?} subtitle={:?} device={:?} playing={} saved={:?} likable={} cover={} slider={:.3}",
                mini.title.stringValue().to_string(),
                mini.subtitle.stringValue().to_string(),
                mini.state.device_line(),
                mini.state.playing,
                mini.state.saved,
                mini.state.likable,
                mini.shown_cover.is_some(),
                mini.slider.doubleValue(),
            ));
            let mut views: Vec<(&str, &NSView)> = vec![
                ("art", &mini.art),
                ("card", &mini.card),
                ("title", &mini.title),
                ("subtitle", &mini.subtitle),
                ("device", &mini.device),
                ("title-button", &mini.title_button),
                ("slider", &mini.slider),
                ("edge", &mini.edge),
            ];
            views.extend(mini.buttons.iter().map(|button| (button.key, &*button.view as &NSView)));
            for (name, view) in views {
                let rect = view.frame();
                lines.push(format!(
                    "{name} hidden={} frame={},{},{},{}",
                    view.isHidden(),
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height
                ));
            }
            lines.join("\n")
        })
        .unwrap_or_else(|| "visible=false built=false".to_owned())
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use std::sync::Arc;

    use gpui::Image;

    use super::MiniState;

    pub fn show(_: &MiniState, _: Option<&Arc<Image>>, _: u32) -> bool {
        false
    }

    pub fn hide() {}

    pub fn is_visible() -> bool {
        false
    }

    pub fn update(_: &MiniState, _: Option<&Arc<Image>>) {}

    pub fn progress(_: u32) {}

    pub fn set_hover(_: bool) {}

    pub fn resize(_: f64, _: f64) {}

    pub fn describe() -> String {
        "visible=false built=false".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Rect {
        fn inside(&self, width: f64, height: f64) -> bool {
            self.x >= 0.
                && self.y >= 0.
                && self.right() <= width + 0.5
                && self.top() <= height + 0.5
        }

        fn overlaps(&self, other: &Rect) -> bool {
            self.x < other.right()
                && other.x < self.right()
                && self.y < other.top()
                && other.y < self.top()
        }
    }

    impl Frames {
        fn controls(&self) -> Vec<Rect> {
            let mut controls = vec![self.previous, self.toggle, self.next];
            controls.extend(self.like);
            controls
        }
    }

    const SIZES: [(f64, f64); 12] = [
        (240., 64.),
        (300., 64.),
        (420., 90.),
        (240., 132.),
        (520., 120.),
        (240., 133.),
        (300., 150.),
        (480., 170.),
        (240., 171.),
        (300., 300.),
        (420., 260.),
        (640., 640.),
    ];

    #[test]
    fn picks_the_layout_by_height() {
        assert_eq!(Layout::for_height(64.), Layout::Strip);
        assert_eq!(Layout::for_height(132.), Layout::Strip);
        assert_eq!(Layout::for_height(133.), Layout::Medium);
        assert_eq!(Layout::for_height(170.), Layout::Medium);
        assert_eq!(Layout::for_height(171.), Layout::Artwork);
        assert_eq!(Layout::for_height(DEFAULT_SIDE), Layout::Artwork);
    }

    #[test]
    fn every_layout_keeps_controls_inside_and_apart() {
        for (width, height) in SIZES {
            for remote in [false, true] {
                let frames = frames(width, height, remote);
                let controls = frames.controls();
                for control in &controls {
                    assert!(
                        control.inside(width, height),
                        "{width}x{height} {control:?}"
                    );
                    assert!(control.w >= 22., "{width}x{height} buttons too small");
                    assert!(
                        !control.overlaps(&frames.text()),
                        "{width}x{height} {control:?}"
                    );
                }
                for (index, control) in controls.iter().enumerate() {
                    for other in &controls[index + 1..] {
                        assert!(!control.overlaps(other), "{width}x{height}");
                    }
                }
                let text = frames.text();
                assert!(text.inside(width, height), "{width}x{height} {text:?}");
                assert!(frames.title.w >= 50., "{width}x{height} title too narrow");
                if let Some(slider) = frames.slider {
                    assert!(slider.inside(width, height));
                    assert!(controls.iter().all(|control| !control.overlaps(&slider)));
                    assert!(!slider.overlaps(&text));
                }
                if let Some(card) = frames.card {
                    assert!(card.inside(width, height));
                    assert!(
                        controls
                            .iter()
                            .all(|control| control.inside(card.right(), card.top()))
                    );
                }
                assert!(frames.close.inside(width, height));
                assert!(!frames.close.overlaps(&frames.expand));
            }
        }
    }

    #[test]
    fn strip_puts_art_left_and_an_edge_bar_below() {
        let frames = frames(300., 80., false);
        assert_eq!(frames.layout, Layout::Strip);
        assert_eq!(frames.art, Rect::new(8., 10., 60., 60.));
        assert_eq!(frames.edge, Some(Rect::new(0., 0., 300., 3.)));
        assert!(frames.slider.is_none());
        assert!(frames.art.right() < frames.title.x);
        assert!(frames.title.right() < frames.controls()[0].x);
    }

    #[test]
    fn narrow_strip_drops_the_heart_first() {
        assert!(frames(240., 64., false).like.is_none());
        assert!(frames(420., 64., false).like.is_some());
    }

    #[test]
    fn medium_adds_a_slider_under_the_controls() {
        let frames = frames(300., 150., true);
        assert_eq!(frames.layout, Layout::Medium);
        let slider = frames.slider.unwrap();
        assert!(slider.top() <= frames.toggle.y);
        assert!(frames.device.is_some());
        assert!(frames.art.right() < slider.x);
        assert!(!frames.hover_only);
    }

    #[test]
    fn artwork_fills_the_panel_and_hides_the_card_until_hover() {
        let frames = frames(300., 300., false);
        assert_eq!(frames.layout, Layout::Artwork);
        assert_eq!(frames.art, Rect::new(0., 0., 300., 300.));
        assert!(frames.hover_only);
        assert!(frames.card.is_some());
        let wide = super::frames(420., 260., false);
        assert_eq!(wide.art.w, 420.);
        assert!(wide.art.y < 0.);
    }

    #[test]
    fn small_panels_grow_to_the_minimum() {
        assert_eq!(
            frames(100., 20., false),
            frames(MIN_WIDTH, MIN_HEIGHT, false)
        );
    }

    #[test]
    fn state_symbols_follow_playback_and_likes() {
        let mut state = MiniState::default();
        assert_eq!(state.toggle_symbol(), "play.fill");
        assert_eq!(state.like_symbol(), "heart");
        state.playing = true;
        state.saved = Some(true);
        assert_eq!(state.toggle_symbol(), "pause.fill");
        assert_eq!(state.like_symbol(), "heart.fill");
        assert_eq!(state.like_tooltip(), "Remove from Liked Songs");
        assert_eq!(state.device_line(), None);
        state.device = Some("Kitchen".into());
        assert_eq!(state.device_line().as_deref(), Some("Playing on Kitchen"));
    }

    #[test]
    fn every_key_is_known() {
        for key in [PREVIOUS, TOGGLE, NEXT, LIKE, CLOSE, EXPAND] {
            assert!(KEYS.contains(&key));
            assert!(key.starts_with("mini-"));
        }
    }
}
