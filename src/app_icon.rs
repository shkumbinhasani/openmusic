#[cfg(target_os = "macos")]
pub fn install() {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let bundled = std::env::current_exe().is_ok_and(|path| {
        path.components()
            .any(|part| part.as_os_str().to_string_lossy().ends_with(".app"))
    });
    if bundled {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../assets/icon/openmusic-512.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}
