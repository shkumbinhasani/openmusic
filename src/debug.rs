use std::path::PathBuf;

pub fn directory() -> Option<PathBuf> {
    std::env::var_os("OPENMUSIC_DEBUG_DIR").map(PathBuf::from)
}

#[cfg(unix)]
pub fn write_token(token: &str) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};

    let Some(path) = directory().map(|directory| directory.join("token")) else {
        return;
    };
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path);
    if let Ok(mut file) = file {
        let _ = file.write_all(token.as_bytes());
    }
}

#[cfg(not(unix))]
pub fn write_token(_: &str) {}

#[cfg(target_os = "macos")]
pub fn capture(path: PathBuf, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    use std::sync::{Arc, Mutex};

    use block2::RcBlock;
    use objc2::AllocAnyThread;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    use objc2_core_graphics::CGImage;
    use objc2_foundation::{NSDictionary, NSError};
    use objc2_screen_capture_kit::{
        SCContentFilter, SCScreenshotManager, SCShareableContent, SCStreamConfiguration,
    };

    type Done = Box<dyn FnOnce(Result<(), String>) + Send>;
    let done: Arc<Mutex<Option<Done>>> = Arc::new(Mutex::new(Some(Box::new(done))));
    let finish = move |done: &Arc<Mutex<Option<Done>>>, result: Result<(), String>| {
        if let Some(done) = done.lock().ok().and_then(|mut done| done.take()) {
            done(result);
        }
    };

    let content_done = done.clone();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, _error: *mut NSError| {
            let Some(content) = (unsafe { content.as_ref() }) else {
                finish(&content_done, Err("No shareable content.".into()));
                return;
            };
            let pid = std::process::id() as i32;
            let windows = unsafe { content.windows() };
            let window = windows
                .iter()
                .filter(|window| unsafe {
                    window
                        .owningApplication()
                        .is_some_and(|app| app.processID() == pid)
                        && window.isOnScreen()
                })
                .max_by(|a, b| unsafe {
                    let area =
                        |frame: objc2_core_foundation::CGRect| frame.size.width * frame.size.height;
                    area(a.frame()).total_cmp(&area(b.frame()))
                });
            let Some(window) = window else {
                finish(
                    &content_done,
                    Err("OpenMusic has no window on screen.".into()),
                );
                return;
            };
            let frame = unsafe { window.frame() };
            let filter = unsafe {
                SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window)
            };
            let config = unsafe { SCStreamConfiguration::new() };
            unsafe {
                config.setWidth((frame.size.width * 2.) as usize);
                config.setHeight((frame.size.height * 2.) as usize);
                config.setShowsCursor(false);
            }
            let image_done = content_done.clone();
            let path = path.clone();
            let image_handler = RcBlock::new(move |image: *mut CGImage, _error: *mut NSError| {
                let Some(image) = (unsafe { image.as_ref() }) else {
                    finish(&image_done, Err("Capture failed.".into()));
                    return;
                };
                let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
                let data = unsafe {
                    bitmap.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                };
                let result = match data {
                    Some(data) => {
                        std::fs::write(&path, data.to_vec()).map_err(|error| error.to_string())
                    }
                    None => Err("PNG encoding failed.".into()),
                };
                finish(&image_done, result);
            });
            unsafe {
                SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                    &filter,
                    &config,
                    Some(&image_handler),
                );
            }
        },
    );
    unsafe { SCShareableContent::getCurrentProcessShareableContentWithCompletionHandler(&handler) };
}

#[cfg(not(target_os = "macos"))]
pub fn capture(_: PathBuf, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    done(Err("Capture is only available on macOS.".into()));
}
