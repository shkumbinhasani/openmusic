use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const DESKTOP_CLIENT_ID: &str = "65b708073fc0480ea92a077233ca87bd";
pub const SCOPES: &str = "streaming user-read-email user-read-private user-read-playback-state user-modify-playback-state user-library-read user-top-read";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const DEVICE_URL: &str = "https://accounts.spotify.com/oauth2/device/authorize";

#[derive(Clone, Debug)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: Instant,
}

impl Token {
    pub fn refresh_due(&self) -> Instant {
        self.expires_at
            .checked_sub(Duration::from_secs(120))
            .unwrap_or(self.expires_at)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: u64,
}

#[derive(Deserialize)]
struct ErrorResponse {
    #[serde(default)]
    error: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Pairing {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_expiry")]
    pub expires_in: u64,
}

fn default_interval() -> u64 {
    5
}

fn default_expiry() -> u64 {
    600
}

impl Pairing {
    pub fn link(&self) -> &str {
        self.verification_uri_complete
            .as_deref()
            .unwrap_or(&self.verification_uri)
    }
}

fn token_from(response: TokenResponse, previous_refresh: &str) -> Token {
    Token {
        access_token: response.access_token,
        refresh_token: response
            .refresh_token
            .filter(|token| !token.is_empty())
            .unwrap_or_else(|| previous_refresh.to_owned()),
        expires_at: Instant::now() + Duration::from_secs(response.expires_in.max(60)),
    }
}

pub async fn refresh(
    http: &reqwest::Client,
    client_id: &str,
    refresh_token: &str,
) -> Result<Token> {
    let response = http
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
        ])
        .send()
        .await
        .context("Could not reach Spotify's sign-in service.")?;
    if response.status().is_client_error() {
        bail!(Rejected);
    }
    let response: TokenResponse = response
        .error_for_status()
        .context("Spotify's sign-in service is unavailable.")?
        .json()
        .await
        .context("Spotify sent an unreadable sign-in response.")?;
    Ok(token_from(response, refresh_token))
}

#[derive(Debug)]
pub struct Rejected;

impl std::fmt::Display for Rejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Spotify rejected the saved sign-in.")
    }
}

impl std::error::Error for Rejected {}

pub async fn start_pairing(http: &reqwest::Client, client_id: &str) -> Result<Pairing> {
    http.post(DEVICE_URL)
        .form(&[("client_id", client_id), ("scope", SCOPES)])
        .send()
        .await
        .context("Could not reach Spotify's sign-in service.")?
        .error_for_status()
        .context("Spotify did not start the pairing.")?
        .json()
        .await
        .context("Spotify sent an unreadable pairing response.")
}

pub async fn finish_pairing(
    http: &reqwest::Client,
    client_id: &str,
    pairing: &Pairing,
) -> Result<Token> {
    let deadline = Instant::now() + Duration::from_secs(pairing.expires_in);
    let mut interval = Duration::from_secs(pairing.interval.max(1));
    loop {
        if Instant::now() > deadline {
            bail!("The pairing code expired. Try again.");
        }
        tokio::time::sleep(interval).await;
        let response = http
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", pairing.device_code.as_str()),
                ("client_id", client_id),
            ])
            .send()
            .await;
        let Ok(response) = response else { continue };
        if response.status().is_success() {
            let token: TokenResponse = response
                .json()
                .await
                .context("Spotify sent an unreadable sign-in response.")?;
            return Ok(token_from(token, ""));
        }
        let error = response
            .json::<ErrorResponse>()
            .await
            .map(|error| error.error)
            .unwrap_or_default();
        match error.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval += Duration::from_secs(5),
            "access_denied" => bail!("Spotify sign-in was declined."),
            "expired_token" => bail!("The pairing code expired. Try again."),
            _ => bail!("Spotify sign-in failed. Try again."),
        }
    }
}
