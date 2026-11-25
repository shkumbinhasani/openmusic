use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub fn directory() -> Option<PathBuf> {
    if !cfg!(any(debug_assertions, feature = "debug-dir")) {
        return None;
    }
    static DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIRECTORY
        .get_or_init(|| {
            let directory = PathBuf::from(std::env::var_os("OPENMUSIC_DEBUG_DIR")?);
            match claim(&directory) {
                Ok(()) => Some(directory),
                Err(reason) => {
                    eprintln!(
                        "OpenMusic ignores OPENMUSIC_DEBUG_DIR {}: {reason}",
                        directory.display()
                    );
                    None
                }
            }
        })
        .clone()
}

#[cfg(unix)]
fn claim(directory: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)
        .map_err(|error| error.to_string())?;
    let metadata = std::fs::symlink_metadata(directory).map_err(|error| error.to_string())?;
    if !metadata.is_dir() {
        return Err("it is not a folder".into());
    }
    if metadata.uid() != unsafe { libc::geteuid() } {
        return Err("another user owns it".into());
    }
    if metadata.mode() & 0o022 != 0 {
        return Err("other users can write to it".into());
    }
    if metadata.mode() & 0o077 != 0 {
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn claim(directory: &Path) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())
}

pub fn secrets() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    directory()
}

pub fn write_token(token: &str) {
    if let Some(directory) = secrets() {
        write_private(&directory.join("token"), token);
    }
}

pub fn forget_token() {
    if let Some(directory) = secrets() {
        let _ = std::fs::remove_file(directory.join("token"));
    }
}

#[cfg(unix)]
pub fn write_private(path: &Path, contents: &str) {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};

    let Some(name) = path.file_name() else {
        return;
    };
    let mut temporary = std::ffi::OsString::from(".");
    temporary.push(name);
    temporary.push(format!(".{}", std::process::id()));
    let temporary = path.with_file_name(temporary);
    let _ = std::fs::remove_file(&temporary);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .and_then(|mut file| file.write_all(contents.as_bytes()))
        .and_then(|()| std::fs::rename(&temporary, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

#[cfg(not(unix))]
pub fn write_private(path: &Path, contents: &str) {
    let _ = std::fs::write(path, contents);
}

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

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("openmusic-debug-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    fn mode(path: &Path) -> u32 {
        std::fs::symlink_metadata(path).unwrap().mode() & 0o777
    }

    #[test]
    fn claims_only_a_private_folder_of_its_own() {
        let root = scratch("claim");
        let fresh = root.join("fresh");
        claim(&fresh).unwrap();
        assert_eq!(mode(&fresh), 0o700);
        let open = root.join("open");
        std::fs::create_dir(&open).unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
        claim(&open).unwrap();
        assert_eq!(mode(&open), 0o700);
        let shared = root.join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(claim(&shared).is_err());
        let link = root.join("link");
        std::os::unix::fs::symlink(&fresh, &link).unwrap();
        assert!(claim(&link).is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn writes_secrets_privately_without_following_links() {
        let root = scratch("write");
        claim(&root).unwrap();
        let elsewhere = root.join("elsewhere");
        std::fs::write(&elsewhere, "untouched").unwrap();
        std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o666)).unwrap();
        let token = root.join("token");
        std::os::unix::fs::symlink(&elsewhere, &token).unwrap();
        write_private(&token, "secret");
        assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "untouched");
        assert_eq!(std::fs::read_to_string(&token).unwrap(), "secret");
        assert!(!std::fs::symlink_metadata(&token).unwrap().is_symlink());
        assert_eq!(mode(&token), 0o600);
        write_private(&token, "");
        assert_eq!(std::fs::read_to_string(&token).unwrap(), "");
        let leftovers = std::fs::read_dir(&root).unwrap().count();
        assert_eq!(leftovers, 2);
        let _ = std::fs::remove_dir_all(root);
    }
}
