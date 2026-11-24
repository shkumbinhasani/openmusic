pub use native::{clear, read, write};

#[cfg(target_os = "macos")]
mod native {
    use security_framework::passwords;

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

    pub fn clear() -> Result<(), security_framework::base::Error> {
        match passwords::delete_generic_password(SERVICE, ACCOUNT) {
            Err(error) if error.code() != ITEM_NOT_FOUND => Err(error),
            _ => Ok(()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub fn read() -> Option<String> {
        None
    }
    pub fn write(_: &str) {}
    pub fn clear() -> Result<(), ()> {
        Ok(())
    }
}
