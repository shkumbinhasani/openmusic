pub mod account;
pub mod api;
pub mod automix;
pub mod blend;
pub mod browse;
pub mod canvas;
pub mod catalog;
pub mod clienttoken;
pub mod collection;
pub mod connect;
pub mod dealer;
pub mod details;
pub mod director;
pub mod dj;
#[cfg(target_os = "macos")]
pub mod engine;
pub mod entity;
pub mod home;
pub mod jam;
pub mod library;
pub mod loudness;
pub mod oauth;
pub mod pathfinder;
pub mod playback_settings;
#[cfg(target_os = "macos")]
pub mod player;
pub mod playlist;
pub mod podcast;
pub mod search;
pub mod share;
pub mod social;

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
