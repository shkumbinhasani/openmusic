use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{api::APP_VERSION, oauth::DESKTOP_CLIENT_ID};

pub const URL: &str = "https://clienttoken.spotify.com/v1/clienttoken";
const FILE_NAME: &str = "client-token.json";
const RETRY_AFTER_FAILURE: u64 = 600;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub token: String,
    pub refresh_at: u64,
    pub expires_at: u64,
    #[serde(default)]
    pub device_id: String,
}

impl Grant {
    pub fn usable(&self, now: u64) -> bool {
        !self.token.is_empty() && now < self.expires_at
    }

    pub fn due(&self, now: u64) -> bool {
        now >= self.refresh_at
    }
}

#[derive(Clone)]
pub struct ClientToken {
    shared: Arc<Shared>,
}

struct Shared {
    path: Option<PathBuf>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    loaded: bool,
    grant: Option<Grant>,
    renewing: bool,
    retry_at: u64,
}

pub fn default_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Application Support/OpenMusic")
            .join(FILE_NAME),
    )
}

pub fn allowed(url: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    if url.scheme() != "https" {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    ["spotify.com", "spotify.net"]
        .iter()
        .any(|domain| host == *domain || host.ends_with(&format!(".{domain}")))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

impl ClientToken {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            shared: Arc::new(Shared {
                path,
                state: Mutex::new(State::default()),
            }),
        }
    }

    pub async fn get(&self, http: &reqwest::Client) -> Option<String> {
        let now = now();
        let (token, renew) = {
            let mut state = self.shared.state.lock().ok()?;
            if !state.loaded {
                state.loaded = true;
                state.grant = self.shared.path.as_deref().and_then(load);
            }
            let token = state
                .grant
                .as_ref()
                .filter(|grant| grant.usable(now))
                .map(|grant| grant.token.clone());
            let due = state.grant.as_ref().is_none_or(|grant| grant.due(now));
            let renew = due && !state.renewing && now >= state.retry_at;
            if renew {
                state.renewing = true;
            }
            let device_id = state
                .grant
                .as_ref()
                .map(|grant| grant.device_id.clone())
                .filter(|id| !id.is_empty());
            (token, renew.then_some(device_id))
        };
        if let Some(device_id) = renew {
            match tokio::runtime::Handle::try_current() {
                Ok(runtime) => {
                    let this = self.clone();
                    let http = http.clone();
                    runtime.spawn(async move { this.renew(&http, device_id).await });
                }
                Err(_) => self.finish(None),
            }
        }
        token
    }

    async fn renew(&self, http: &reqwest::Client, device_id: Option<String>) {
        let device_id = device_id.unwrap_or_else(new_device_id);
        let system = tokio::task::spawn_blocking(System::read)
            .await
            .unwrap_or_default();
        let grant = fetch(http, &request_body(&device_id, &system), &device_id)
            .await
            .ok();
        if let (Some(grant), Some(path)) = (&grant, &self.shared.path) {
            let _ = save(path, grant);
        }
        self.finish(grant);
    }

    fn finish(&self, grant: Option<Grant>) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        state.renewing = false;
        match grant {
            Some(grant) => state.grant = Some(grant),
            None => state.retry_at = now() + RETRY_AFTER_FAILURE,
        }
    }
}

fn new_device_id() -> String {
    (0..16)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct System {
    pub version: String,
    pub model: String,
    pub cpu_type: i64,
    pub cpu_subtype: i64,
}

impl Default for System {
    fn default() -> Self {
        Self {
            version: "15.0".into(),
            model: "Mac".into(),
            cpu_type: 0,
            cpu_subtype: 0,
        }
    }
}

impl System {
    pub fn read() -> Self {
        let fallback = Self::default();
        let output = std::process::Command::new("/usr/sbin/sysctl")
            .args([
                "-n",
                "kern.osproductversion",
                "hw.model",
                "hw.cputype",
                "hw.cpusubtype",
            ])
            .output();
        let Ok(output) = output else {
            return fallback;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let mut lines = text.lines().map(str::trim);
        let mut field = || lines.next().filter(|line| !line.is_empty());
        Self {
            version: field().map_or(fallback.version, str::to_owned),
            model: field().map_or(fallback.model, str::to_owned),
            cpu_type: field().and_then(|line| line.parse().ok()).unwrap_or(0),
            cpu_subtype: field().and_then(|line| line.parse().ok()).unwrap_or(0),
        }
    }

    fn version_part(&self, index: usize) -> i64 {
        self.version
            .split('.')
            .nth(index)
            .and_then(|part| part.parse().ok())
            .unwrap_or(0)
    }
}

pub fn request_body(device_id: &str, system: &System) -> Value {
    json!({
        "client_data": {
            "client_version": APP_VERSION,
            "client_id": DESKTOP_CLIENT_ID,
            "connectivity_sdk_data": {
                "platform_specific_data": {
                    "mac": {
                        "system_version": format!("Version {}", system.version),
                        "system_version_major": system.version_part(0),
                        "system_version_minor": system.version_part(1),
                        "system_version_patch": system.version_part(2),
                        "hw_model": system.model,
                        "cpu_type": system.cpu_type,
                        "cpu_subtype": system.cpu_subtype,
                        "compiled_cpu_type": std::env::consts::ARCH,
                    }
                },
                "device_id": device_id,
            }
        }
    })
}

pub fn parse_grant(reply: &Value, device_id: &str, now: u64) -> Result<Grant> {
    let kind = reply["response_type"].as_str().unwrap_or_default();
    if kind != "RESPONSE_GRANTED_TOKEN_RESPONSE" {
        bail!("Spotify did not grant a client token ({kind}).");
    }
    let granted = &reply["granted_token"];
    let token = granted["token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .context("The client token reply had no token.")?;
    let seconds = |key: &str| {
        granted[key]
            .as_u64()
            .or_else(|| granted[key].as_str().and_then(|text| text.parse().ok()))
            .unwrap_or(0)
    };
    let expires = seconds("expires_after_seconds");
    let refresh = seconds("refresh_after_seconds").min(expires);
    Ok(Grant {
        token: token.to_owned(),
        refresh_at: now + refresh,
        expires_at: now + expires,
        device_id: device_id.to_owned(),
    })
}

async fn fetch(http: &reqwest::Client, body: &Value, device_id: &str) -> Result<Grant> {
    let response = http
        .post(URL)
        .header("accept", "application/json")
        .timeout(REQUEST_TIMEOUT)
        .json(body)
        .send()
        .await?;
    if !response.status().is_success() {
        bail!(crate::api::HttpStatus(response.status()));
    }
    let reply: Value = response.json().await?;
    parse_grant(&reply, device_id, now())
}

fn load(path: &Path) -> Option<Grant> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

fn save(path: &Path, grant: &Grant) -> Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let temporary = path.with_extension("tmp");
    write_private(&temporary, &serde_json::to_vec(grant)?)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(unix)]
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::{
        io::Write,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
    };

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_the_token_only_to_spotify_hosts() {
        for url in [
            "https://api-partner.spotify.com/pathfinder/v2/query",
            "https://spclient.wg.spotify.com/collection/v2/paging",
            "https://gew4-spclient.spotify.com/x",
            "https://spotify.com/",
            "https://example.spotify.net/path",
            "https://API-PARTNER.SPOTIFY.COM/x",
        ] {
            assert!(allowed(url), "{url}");
        }
        for url in [
            "http://api-partner.spotify.com/x",
            "https://i.scdn.co/image/abc",
            "https://notspotify.com/",
            "https://spotify.com.evil.example/",
            "https://platform-lookaside.fbsbx.com/x",
            "https://evil.example/?spotify.com",
            "not a url",
        ] {
            assert!(!allowed(url), "{url}");
        }
    }

    #[test]
    fn builds_the_desktop_request() {
        let system = System {
            version: "26.1".into(),
            model: "Mac16,8".into(),
            cpu_type: 16777228,
            cpu_subtype: 2,
        };
        let body = request_body("00112233445566778899aabbccddeeff", &system);
        let data = &body["client_data"];
        assert_eq!(data["client_version"], "1.3.10.2586.g81a118a22460");
        assert_eq!(data["client_id"], DESKTOP_CLIENT_ID);
        let sdk = &data["connectivity_sdk_data"];
        assert_eq!(sdk["device_id"], "00112233445566778899aabbccddeeff");
        let mac = &sdk["platform_specific_data"]["mac"];
        assert_eq!(mac["system_version"], "Version 26.1");
        assert_eq!(mac["system_version_major"], 26);
        assert_eq!(mac["system_version_minor"], 1);
        assert_eq!(mac["system_version_patch"], 0);
        assert_eq!(mac["hw_model"], "Mac16,8");
        assert_eq!(mac["cpu_type"], 16777228);
        assert_eq!(mac["cpu_subtype"], 2);
        assert_eq!(mac["compiled_cpu_type"], std::env::consts::ARCH);
    }

    #[test]
    fn reads_a_granted_token() {
        let reply = json!({
            "response_type": "RESPONSE_GRANTED_TOKEN_RESPONSE",
            "granted_token": {
                "token": "AAD-token",
                "expires_after_seconds": 1216800,
                "refresh_after_seconds": 1209600,
                "domains": [{"domain": "spotify.com"}, {"domain": "spotify.net"}]
            }
        });
        let grant = parse_grant(&reply, "device", 1000).unwrap();
        assert_eq!(
            grant,
            Grant {
                token: "AAD-token".into(),
                refresh_at: 1000 + 1209600,
                expires_at: 1000 + 1216800,
                device_id: "device".into(),
            }
        );
        assert!(grant.usable(1001));
        assert!(!grant.due(1001));
        assert!(grant.due(1000 + 1209600));
        assert!(!grant.usable(1000 + 1216800));
    }

    #[test]
    fn refuses_challenges() {
        let reply = json!({
            "response_type": "RESPONSE_CHALLENGES_RESPONSE",
            "challenges": {"state": "abc", "challenges": [{"type": "CHALLENGE_HASH_CASH"}]}
        });
        assert!(parse_grant(&reply, "device", 0).is_err());
    }

    #[test]
    fn keeps_the_cache_private() {
        let directory = std::env::temp_dir().join(format!(
            "openmusic-client-token-{}-{}",
            std::process::id(),
            new_device_id()
        ));
        let path = directory.join(FILE_NAME);
        let grant = Grant {
            token: "token".into(),
            refresh_at: 10,
            expires_at: 20,
            device_id: "device".into(),
        };
        save(&path, &grant).unwrap();
        assert_eq!(load(&path), Some(grant));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn serves_a_cached_token_without_renewing() {
        let client = ClientToken::new(None);
        {
            let mut state = client.shared.state.lock().unwrap();
            state.loaded = true;
            state.grant = Some(Grant {
                token: "cached".into(),
                refresh_at: now() + 3600,
                expires_at: now() + 7200,
                device_id: "device".into(),
            });
        }
        let http = reqwest::Client::new();
        assert_eq!(client.get(&http).await.as_deref(), Some("cached"));
        assert!(!client.shared.state.lock().unwrap().renewing);
    }

    #[tokio::test]
    async fn sends_nothing_while_a_failed_renewal_waits() {
        let client = ClientToken::new(None);
        {
            let mut state = client.shared.state.lock().unwrap();
            state.loaded = true;
            state.retry_at = now() + 600;
        }
        let http = reqwest::Client::new();
        assert_eq!(client.get(&http).await, None);
        assert!(!client.shared.state.lock().unwrap().renewing);
    }
}
