use std::time::{Duration, Instant};

use anyhow::Result;
use spotify_client::{
    Api, catalog, connect,
    engine::{self, Control, Update},
    oauth,
};

fn main() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.spawn(async {
        if let Err(error) = scenario().await {
            println!("scenario failed: {error:#}");
        }
        std::process::exit(0);
    });
    loop {
        objc2_foundation::NSRunLoop::mainRunLoop()
            .runUntilDate(&objc2_foundation::NSDate::dateWithTimeIntervalSinceNow(0.1));
    }
}

async fn scenario() -> Result<()> {
    let http = spotify_client::http_client()?;
    let access = std::fs::read_to_string(std::env::var("TOKEN_FILE")?)?
        .trim()
        .to_owned();
    let token = oauth::Token {
        access_token: access,
        refresh_token: String::new(),
        expires_at: Instant::now() + Duration::from_secs(1200),
    };
    let api = Api::connect(http.clone(), token.access_token.clone()).await;

    let found = catalog::search(&api, "nina simone feeling good").await?;
    println!(
        "search: {} tracks, first = {:?}",
        found.len(),
        found.first().map(|track| (&track.title, &track.artist))
    );
    let station = catalog::station(&api, &found[0].uri).await?;
    println!("station: {} tracks", station.len());
    let lyrics = catalog::lyrics(&api, &found[0].uri).await?;
    println!(
        "lyrics: {:?}",
        lyrics
            .as_ref()
            .map(|lyrics| (lyrics.synced, lyrics.lines.len()))
    );

    let (handle, mut updates) = engine::start(http, api, token, "OpenMusic (test)".into());
    let started = Instant::now();
    let mut stage = 0;
    let mut last_line = String::new();
    while started.elapsed() < Duration::from_secs(45) {
        let Ok(Some(update)) =
            tokio::time::timeout(Duration::from_millis(500), updates.recv()).await
        else {
            continue;
        };
        match update {
            Update::Ready => {
                println!(
                    "[{:>4.1}s] device registered",
                    started.elapsed().as_secs_f32()
                );
                if stage == 0 {
                    stage = 1;
                    if let Ok(context) = std::env::var("CONTEXT") {
                        handle.send(Control::PlayContext {
                            uri: context,
                            index: 2,
                            track_uri: None,
                            origin: connect::ORIGIN,
                            options: connect::PlayOptions::default(),
                        });
                    } else {
                        let uris: Vec<String> = found
                            .iter()
                            .take(3)
                            .map(|track| track.uri.clone())
                            .collect();
                        handle.send(Control::PlayTracks { uris, index: 0 });
                    }
                }
            }
            Update::Now(now) => {
                let line = format!(
                    "{} | playing={} local={} device={:?}",
                    now.track
                        .as_ref()
                        .map(|track| format!(
                            "{} - {} cover={:?}",
                            track.title, track.artist, track.cover
                        ))
                        .unwrap_or_default(),
                    now.playing,
                    now.local,
                    now.device,
                );
                if line != last_line {
                    println!(
                        "[{:>4.1}s] {line} pos={}ms",
                        started.elapsed().as_secs_f32(),
                        now.position()
                    );
                    last_line = line;
                }
                let elapsed = started.elapsed();
                if stage == 1 && now.local && now.playing && now.position() > 6000 {
                    stage = 2;
                    println!(
                        "[{:>4.1}s] at {}ms -> next",
                        elapsed.as_secs_f32(),
                        now.position()
                    );
                    handle.send(Control::Next);
                } else if stage == 2 && now.local && now.playing && now.position() > 5000 {
                    stage = 3;
                    println!(
                        "[{:>4.1}s] at {}ms -> seek 60s",
                        elapsed.as_secs_f32(),
                        now.position()
                    );
                    handle.send(Control::Seek(60_000));
                } else if stage == 3 && now.local && now.position() > 63_000 {
                    stage = 4;
                    println!(
                        "[{:>4.1}s] at {}ms -> pause",
                        elapsed.as_secs_f32(),
                        now.position()
                    );
                    handle.send(Control::SetPlaying(false));
                } else if stage == 4 && !now.playing {
                    println!(
                        "[{:>4.1}s] paused at {}ms",
                        elapsed.as_secs_f32(),
                        now.position()
                    );
                    stage = 5;
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    handle.send(Control::SignOut);
                }
            }
            Update::Playback(playback) => println!(
                "[{:>4.1}s] playback: active={:?} context={:?} shuffle={:?} repeat={:?} devices={}",
                started.elapsed().as_secs_f32(),
                playback.active_device_name,
                playback.context_uri,
                playback.shuffle,
                playback.repeat,
                playback.devices.len()
            ),
            Update::Push(message) => println!(
                "[{:>4.1}s] push: {}",
                started.elapsed().as_secs_f32(),
                message.uri
            ),
            Update::Audio(update) => match update {},
            Update::Notice(message) => println!(
                "[{:>4.1}s] notice: {message}",
                started.elapsed().as_secs_f32()
            ),
            Update::RefreshToken(_) => println!("token rotated"),
            Update::Ended(reason) => {
                println!(
                    "[{:>4.1}s] engine ended: {reason:?}",
                    started.elapsed().as_secs_f32()
                );
                break;
            }
        }
    }
    Ok(())
}
