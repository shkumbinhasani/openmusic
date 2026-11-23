pub mod api;
pub mod catalog;
pub mod connect;
pub mod dealer;
#[cfg(target_os = "macos")]
pub mod engine;
pub mod oauth;
#[cfg(target_os = "macos")]
pub mod player;

pub use api::Api;
pub use catalog::Track;

pub fn http_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(format!(
            "Spotify/124200290 OSX/0 (openmusic-{})",
            env!("CARGO_PKG_VERSION")
        ))
        .timeout(std::time::Duration::from_secs(20))
        .build()
}
