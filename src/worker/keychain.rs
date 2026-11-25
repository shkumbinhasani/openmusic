use std::path::PathBuf;

pub fn read() -> Option<String> {
    let Some(path) = debug_file() else {
        return native::read();
    };
    let saved = std::fs::read_to_string(&path)
        .ok()
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty());
    saved.or_else(|| {
        let token = native::read()?;
        crate::debug::write_private(&path, &token);
        Some(token)
    })
}

pub fn write(token: &str) {
    match debug_file() {
        Some(path) if !token.is_empty() => crate::debug::write_private(&path, token),
        Some(_) => {}
        None => native::write(token),
    }
}

pub fn clear() -> Result<(), native::Error> {
    match debug_file() {
        Some(path) => {
            let _ = std::fs::remove_file(path);
            Ok(())
        }
        None => native::clear(),
    }
}

fn debug_file() -> Option<PathBuf> {
    crate::debug::directory().map(|directory| directory.join("refresh-token"))
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
