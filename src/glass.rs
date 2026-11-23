use gpui::{IntoElement, Styled, canvas};

#[cfg(target_os = "macos")]
pub use native::host;
pub use native::{available, install};

pub fn surface(key: impl Into<String>, radius: f32) -> impl IntoElement {
    let key = key.into();
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| native::place(&key, bounds, radius, window),
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

#[cfg(target_os = "macos")]
mod native {
    use std::{cell::RefCell, collections::HashMap};

    use gpui::{Bounds, Pixels, Window};
    use objc2::{MainThreadMarker, MainThreadOnly, Message, rc::Retained, runtime::AnyClass};
    use objc2_app_kit::{
        NSAutoresizingMaskOptions, NSGlassEffectView, NSGlassEffectViewStyle, NSView,
        NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState,
        NSVisualEffectView, NSWindowOrderingMode,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    struct Surface {
        view: Retained<NSGlassEffectView>,
        placed: bool,
    }

    #[derive(Default)]
    struct Glass {
        host: Option<Retained<NSView>>,
        surfaces: HashMap<String, Surface>,
    }

    thread_local! {
        static GLASS: RefCell<Glass> = RefCell::default();
    }

    pub fn available() -> bool {
        GLASS.with_borrow(|glass| glass.host.is_some())
    }

    pub fn host() -> Option<Retained<NSView>> {
        GLASS.with_borrow(|glass| glass.host.clone())
    }

    pub fn install(window: &Window) -> bool {
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
        let Some(container) = (unsafe { view.superview() }) else {
            return false;
        };

        let backdrop =
            NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), container.bounds());
        backdrop.setMaterial(NSVisualEffectMaterial::WindowBackground);
        backdrop.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        backdrop.setState(NSVisualEffectState::Active);
        backdrop.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        container.addSubview_positioned_relativeTo(&backdrop, NSWindowOrderingMode::Below, None);

        if AnyClass::get(c"NSGlassEffectView").is_some() {
            GLASS.with_borrow_mut(|glass| glass.host = Some(view.retain()));
        }
        true
    }

    pub fn place(key: &str, bounds: Bounds<Pixels>, radius: f32, window: &Window) {
        let mask = window.content_mask().bounds;
        let visible = mask.contains(&bounds.origin) && mask.contains(&bounds.bottom_right());
        GLASS.with_borrow_mut(|glass| {
            let Some(host) = glass.host.clone() else {
                return;
            };
            let Some(container) = (unsafe { host.superview() }) else {
                return;
            };
            let Some(mtm) = MainThreadMarker::new() else {
                return;
            };
            let surface = glass.surfaces.entry(key.to_owned()).or_insert_with(|| {
                let view =
                    NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), NSRect::ZERO);
                view.setStyle(NSGlassEffectViewStyle::Regular);
                container.addSubview_positioned_relativeTo(
                    &view,
                    NSWindowOrderingMode::Below,
                    Some(&host),
                );
                Surface {
                    view,
                    placed: false,
                }
            });
            surface.placed = visible;
            if !visible {
                surface.view.setHidden(true);
                return;
            }
            let height = container.bounds().size.height;
            let x = f64::from(f32::from(bounds.origin.x));
            let y = f64::from(f32::from(bounds.origin.y));
            let width = f64::from(f32::from(bounds.size.width));
            let tall = f64::from(f32::from(bounds.size.height));
            surface.view.setFrame(NSRect::new(
                NSPoint::new(x, height - y - tall),
                NSSize::new(width, tall),
            ));
            surface
                .view
                .setCornerRadius(f64::from(radius).min(tall / 2.).min(width / 2.));
            surface.view.setHidden(false);
        });
    }

    pub fn finish() {
        GLASS.with_borrow_mut(|glass| {
            for surface in glass.surfaces.values_mut() {
                if !surface.placed {
                    surface.view.setHidden(true);
                }
                surface.placed = false;
            }
        });
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    use gpui::{Bounds, Pixels, Window};

    pub fn available() -> bool {
        false
    }

    pub fn install(_: &Window) -> bool {
        false
    }

    pub fn place(_: &str, _: Bounds<Pixels>, _: f32, _: &Window) {}

    pub fn finish() {}
}
