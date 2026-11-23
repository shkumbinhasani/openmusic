use std::fmt;

use anyhow::{Result, bail};

const BASE62: &[u8; 62] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Track,
    Episode,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpotifyId {
    pub kind: Kind,
    pub value: u128,
}

impl SpotifyId {
    pub fn from_base62(kind: Kind, text: &str) -> Result<Self> {
        if text.len() != 22 {
            bail!("A Spotify ID has 22 characters.");
        }
        let mut value: u128 = 0;
        for byte in text.bytes() {
            let Some(digit) = BASE62.iter().position(|candidate| *candidate == byte) else {
                bail!("A Spotify ID uses only letters and digits.");
            };
            value = value
                .checked_mul(62)
                .and_then(|value| value.checked_add(digit as u128))
                .ok_or_else(|| anyhow::anyhow!("The Spotify ID is out of range."))?;
        }
        Ok(Self { kind, value })
    }

    pub fn from_uri(uri: &str) -> Result<Self> {
        match uri.split(':').collect::<Vec<_>>().as_slice() {
            ["spotify", "track", id] => Self::from_base62(Kind::Track, id),
            ["spotify", "episode", id] => Self::from_base62(Kind::Episode, id),
            _ => bail!("Only Spotify tracks and episodes are supported."),
        }
    }

    pub fn from_raw(kind: Kind, bytes: &[u8]) -> Result<Self> {
        let Ok(bytes) = <[u8; 16]>::try_from(bytes) else {
            bail!("A raw Spotify ID has 16 bytes.");
        };
        Ok(Self {
            kind,
            value: u128::from_be_bytes(bytes),
        })
    }

    pub fn to_base62(self) -> String {
        let mut digits = [b'0'; 22];
        let mut value = self.value;
        for slot in digits.iter_mut().rev() {
            *slot = BASE62[(value % 62) as usize];
            value /= 62;
        }
        String::from_utf8_lossy(&digits).into_owned()
    }

    pub fn to_hex(self) -> String {
        format!("{:032x}", self.value)
    }

    pub fn to_raw(self) -> [u8; 16] {
        self.value.to_be_bytes()
    }

    pub fn to_uri(self) -> String {
        let kind = match self.kind {
            Kind::Track => "track",
            Kind::Episode => "episode",
        };
        format!("spotify:{kind}:{}", self.to_base62())
    }
}

impl fmt::Debug for SpotifyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_uri())
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, SpotifyId};

    #[test]
    fn round_trips_base62_and_hex() {
        let id = SpotifyId::from_base62(Kind::Track, "4uLU6hMCjMI75M1A2tKUQC").unwrap();
        assert_eq!(id.to_base62(), "4uLU6hMCjMI75M1A2tKUQC");
        assert_eq!(id.to_hex(), "93bc414a606747b2b612491ef83d5a3e");
        assert_eq!(SpotifyId::from_raw(Kind::Track, &id.to_raw()).unwrap(), id);
        assert_eq!(SpotifyId::from_uri(&id.to_uri()).unwrap(), id);
        assert!(SpotifyId::from_base62(Kind::Track, "short").is_err());
    }
}
