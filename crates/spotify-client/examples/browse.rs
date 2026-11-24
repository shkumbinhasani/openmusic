use anyhow::Result;
use spotify_client::{Api, catalog};

#[tokio::main]
async fn main() -> Result<()> {
    let token = std::fs::read_to_string(std::env::var("TOKEN_FILE")?)?
        .trim()
        .to_owned();
    let api = Api::connect(spotify_client::http_client()?, token).await;

    let artist = catalog::artist(&api, "spotify:artist:7G1GBhoKtEPnP86X2PvEYO").await?;
    println!(
        "artist: {} image={} bio={} chars top={} releases={} related={}",
        artist.name,
        artist.image.is_some(),
        artist.biography.as_ref().map_or(0, String::len),
        artist.top_tracks.len(),
        artist.releases.len(),
        artist.related.len()
    );
    for release in artist.releases.iter().take(3) {
        println!(
            "  release: {} ({:?}, {}) cover={}",
            release.name,
            release.year,
            release.kind,
            release.cover.is_some()
        );
    }
    println!(
        "  first top track: {:?}",
        artist
            .top_tracks
            .first()
            .map(|track| (&track.title, &track.artist_uri))
    );

    let user = catalog::username(&api).await?;
    let playlists = catalog::playlists(&api, &user).await?;
    let with_cover = playlists
        .iter()
        .filter(|playlist| playlist.cover.is_some())
        .count();
    println!(
        "playlists: {} ({} with covers)",
        playlists.len(),
        with_cover
    );

    if let Some(playlist) = playlists.iter().find(|playlist| playlist.length > 0) {
        let collection = catalog::collection(&api, &playlist.uri).await?;
        println!(
            "playlist page: {} | {} | {} tracks",
            collection.name,
            collection.subtitle,
            collection.tracks.len()
        );
    }
    if let Some(release) = artist.releases.first() {
        println!("release uri: {}", release.uri);
        let album = catalog::collection(&api, &release.uri).await?;
        println!(
            "album page: {} | {} | {} tracks | artist link={}",
            album.name,
            album.subtitle,
            album.tracks.len(),
            album.artist_uri.is_some()
        );
    }
    Ok(())
}
