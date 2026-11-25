use std::path::{Path, PathBuf};

pub fn read() -> Option<String> {
    let Some(path) = debug_file() else {
        return native::read();
    };
    read_debug(&path, native::read)
}

pub fn write(token: &str) {
    match debug_file() {
        Some(path) => write_shared(&path, token, native::read, native::write),
        None => native::write(token),
    }
}

pub fn clear() -> Result<(), native::Error> {
    match debug_file() {
        Some(path) => {
            crate::debug::write_private(&path, "");
            Ok(())
        }
        None => native::clear(),
    }
}

fn debug_file() -> Option<PathBuf> {
    crate::debug::secrets().map(|directory| directory.join("refresh-token"))
}

fn read_debug(path: &Path, keychain: impl FnOnce() -> Option<String>) -> Option<String> {
    if path.exists() {
        return saved(path);
    }
    let token = keychain()?;
    crate::debug::write_private(path, &token);
    Some(token)
}

fn saved(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
}

fn write_shared(
    path: &Path,
    token: &str,
    keychain: impl FnOnce() -> Option<String>,
    store: impl FnOnce(&str),
) {
    if token.is_empty() {
        return;
    }
    if saved(path).is_some_and(|current| keychain().as_deref() == Some(current.as_str())) {
        store(token);
    }
    crate::debug::write_private(path, token);
}

#[cfg(target_os = "macos")]
mod native {
    use security_framework::passwords;

    pub type Error = security_framework::base::Error;

    const SERVICE: &str = "openmusic.spotify";
    const ACCOUNT: &str = "refresh-token";
    const ITEM_NOT_FOUND: i32 = -25300;

    pub fn read() -> Option<String> {
        String::from_utf8(passwords::get_generic_password(SERVICE, ACCOUNT).ok()?).ok()
    }

    pub fn write(token: &str) {
        if token.is_empty() || read().as_deref() == Some(token) {
            return;
        }
        if passwords::set_generic_password(SERVICE, ACCOUNT, token.as_bytes()).is_err() {
            eprintln!("OpenMusic could not save the Spotify session in Keychain.");
        }
    }

    pub fn clear() -> Result<(), Error> {
        match passwords::delete_generic_password(SERVICE, ACCOUNT) {
            Err(error) if error.code() != ITEM_NOT_FOUND => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub type Error = ();

    pub fn read() -> Option<String> {
        None
    }
    pub fn write(_: &str) {}
    pub fn clear() -> Result<(), Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("openmusic-keychain-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        directory.join("refresh-token")
    }

    #[test]
    fn a_rotated_token_reaches_keychain_while_the_session_is_shared() {
        let path = scratch("shared");
        let stored = RefCell::new(None);
        let store = |token: &str| *stored.borrow_mut() = Some(token.to_owned());
        std::fs::write(&path, "first\n").unwrap();
        write_shared(&path, "second", || Some("first".into()), store);
        assert_eq!(stored.take().as_deref(), Some("second"));
        assert_eq!(saved(&path).as_deref(), Some("second"));
        write_shared(&path, "third", || Some("someone else".into()), store);
        assert_eq!(stored.take(), None);
        assert_eq!(saved(&path).as_deref(), Some("third"));
        write_shared(&path, "", || Some("third".into()), store);
        assert_eq!(stored.take(), None);
        assert_eq!(saved(&path).as_deref(), Some("third"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_debug_build_borrows_keychain_once_and_stays_signed_out() {
        let path = scratch("borrow");
        assert_eq!(
            read_debug(&path, || Some("installed".into())).as_deref(),
            Some("installed")
        );
        assert_eq!(saved(&path).as_deref(), Some("installed"));
        assert_eq!(
            read_debug(&path, || panic!("read Keychain twice")).as_deref(),
            Some("installed")
        );
        crate::debug::write_private(&path, "");
        assert_eq!(read_debug(&path, || Some("installed".into())), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
