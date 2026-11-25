use anyhow::{Result, anyhow, bail};
use reqwest::Method;

use crate::{api::Body, engine::Engine};

pub const MAX_NAME: usize = 64;
const NOT_RENAMED: &str =
    "Spotify didn’t take the new name. OpenMusic uses it the next time it connects.";

pub fn device_name(text: &str) -> Result<String, &'static str> {
    let cleaned: String = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let name = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Enter a name for this Mac.");
    }
    if name.chars().count() > MAX_NAME {
        return Err("Use 64 characters or fewer.");
    }
    Ok(name)
}

pub(super) fn device_path(device_id: &str) -> String {
    format!("/track-playback/v1/devices/{device_id}")
}

impl Engine {
    pub(super) async fn rename(&mut self, name: &str) -> Result<()> {
        let name = device_name(name).map_err(|message| anyhow!(message))?;
        if name == self.device_name {
            return Ok(());
        }
        self.device_name = name;
        if !self.registered {
            return Ok(());
        }
        let (status, _) = self
            .api
            .send(
                Method::PUT,
                &device_path(&self.device_id),
                Body::Json(self.device_info()),
                "application/json",
            )
            .await
            .map_err(|_| anyhow!(NOT_RENAMED))?;
        if !status.is_success() {
            bail!(NOT_RENAMED);
        }
        self.audio.note(format!("renamed {}", self.device_name));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidies_and_checks_names() {
        assert_eq!(device_name("  Kitchen   Mac ").unwrap(), "Kitchen Mac");
        assert_eq!(device_name("Desk\nMac\t2").unwrap(), "Desk Mac 2");
        assert_eq!(device_name("Café ☕").unwrap(), "Café ☕");
        assert!(device_name("   ").is_err());
        assert!(device_name("\n").is_err());
        assert_eq!(device_name(&"é".repeat(64)).unwrap().chars().count(), 64);
        assert!(device_name(&"é".repeat(65)).is_err());
    }

    #[test]
    fn renames_the_registered_device() {
        assert_eq!(
            device_path("0123456789abcdef0123456789abcdef01234567"),
            "/track-playback/v1/devices/0123456789abcdef0123456789abcdef01234567"
        );
    }
}
