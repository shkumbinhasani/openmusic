use std::{
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use spotify_client::Api;

const PACE_FILE: &str = "/tmp/openmusic-research/.last-call";
const MIN_GAP: Duration = Duration::from_millis(400);

pub fn token() -> Result<String> {
    let path = std::env::var("TOKEN_FILE").context("Set TOKEN_FILE to the access token file.")?;
    let token = std::fs::read_to_string(&path)
        .with_context(|| format!("Could not read {path}."))?
        .trim()
        .to_owned();
    ensure!(!token.is_empty(), "{path} is empty.");
    Ok(token)
}

pub async fn throttle() {
    let path = Path::new(PACE_FILE);
    let elapsed = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok());
    if let Some(wait) = elapsed.and_then(|elapsed| MIN_GAP.checked_sub(elapsed)) {
        tokio::time::sleep(wait).await;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let _ = std::fs::write(path, now.to_string());
}

pub async fn api() -> Result<Api> {
    let token = token()?;
    throttle().await;
    Ok(Api::connect(spotify_client::http_client()?, token).await)
}
