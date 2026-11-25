use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, ensure};
use spotify_client::Api;

const MIN_GAP: Duration = Duration::from_millis(400);

fn pace_file() -> PathBuf {
    std::env::var_os("PACE_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("openmusic-examples.last-call"))
}

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
    let path = pace_file();
    let elapsed = std::fs::metadata(&path)
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
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, now.to_string()));
    if written.is_err() {
        tokio::time::sleep(MIN_GAP).await;
    }
}

pub async fn api() -> Result<Api> {
    let token = token()?;
    throttle().await;
    Ok(Api::connect(spotify_client::http_client()?, token).await)
}
