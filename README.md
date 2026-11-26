<p align="center">
  <img src="assets/icon/openmusic-512.png" width="128" height="128" alt="OpenMusic icon">
</p>

<h1 align="center">OpenMusic</h1>

<p align="center">
  A native macOS player for Spotify, written in Rust.<br>
  One small app. No web view, no Electron, no librespot.
</p>

<p align="center">
  <a href="https://github.com/shkumbinhasani/openmusic/actions/workflows/ci.yml"><img src="https://github.com/shkumbinhasani/openmusic/actions/workflows/ci.yml/badge.svg" alt="CI status"></a>
  <a href="https://github.com/shkumbinhasani/openmusic/releases/latest"><img src="https://img.shields.io/github/v/release/shkumbinhasani/openmusic?label=release" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/macOS-13%2B-black?logo=apple" alt="macOS 13 or later">
</p>

<p align="center">
  <img src="docs/images/openmusic.png" alt="OpenMusic showing the EnteTainment artist page while playing “JBB Finale vs. NEO”" width="880">
</p>

OpenMusic signs in to your Spotify account and joins it as a Spotify Connect device. It plays music and podcasts itself, so you can control it from Spotify on your phone, and it shows whatever your account plays on other devices. The interface uses the native macOS glass controls.

## Benchmarks

OpenMusic against the official Spotify app on the same Mac, measured with `scripts/bench.sh`.

| | OpenMusic | Spotify 1.3.10 | Difference |
|---|---:|---:|---:|
| App size | **13 MB** | 423 MB | 32× smaller |
| Launch to window | **220 ms** | 394 ms | 1.8× faster |
| Processes | **1** | 7 | |
| Memory, idle | **240 MB** | 2,054 MB | 8.6× less |
| Memory, playing | **266 MB** | 1,877 MB | 7.1× less |
| CPU, idle | **0.4%** | 3.0% | 7.5× less |
| CPU, playing | **4.0%** | 11.4% | 2.9× less |

<sub>MacBook Pro with Apple M4 Pro and 24 GB of memory, macOS 27.0, 23 November 2026. Launch time is the median of five launches, from <code>open</code> until the first window is on screen. Memory is the physical footprint of every process the app runs, added together; this is the figure Activity Monitor shows. CPU is the share of one core over 30 seconds. Each app was measured alone, signed in, 20 seconds after launch and again 20 seconds after it started playing the same album.</sub>

These figures come from before this release added podcasts, Canvas, Jam, local files and the other features below, so the current app may differ. Run the benchmark yourself after building the app. It needs OpenMusic signed in, and Spotify installed if you want the comparison. It plays music in both apps for about a minute. The script drives OpenMusic through a command folder that only exists in builds with the `debug-dir` feature, so build the app with it.

```sh
FEATURES=debug-dir ./scripts/bundle-macos.sh
./scripts/bench.sh
```

## Install

### Homebrew

```sh
brew install --cask shkumbinhasani/tap/openmusic
```

Installing by the full name also trusts this one cask, which Homebrew requires for third-party taps. `brew upgrade` installs new versions; the release workflow updates the tap each time a version is published. `brew uninstall --zap --cask openmusic` also removes the cache, the settings, the local music index, the device ID, the client token and the Keychain sign-in.

### Download

Download `OpenMusic-<version>-arm64.zip` for Apple silicon or `OpenMusic-<version>-x86_64.zip` for Intel from the [latest release](https://github.com/shkumbinhasani/openmusic/releases/latest), unzip it, and move OpenMusic to Applications.

### Gatekeeper

The release workflow signs each release with a Developer ID certificate, and Apple notarizes it. On the first launch macOS checks the app, as it does any downloaded app, and asks you once to confirm that you want to open it. The Homebrew cask no longer removes the quarantine flag, so macOS runs the same check for Homebrew installs.

### First launch

Choose **Sign in with Spotify** and approve the pairing in your browser. Playing on this Mac needs Spotify Premium. OpenMusic keeps the sign-in in your macOS Keychain. Choose **OpenMusic → Sign Out of Spotify** to remove it.

## What it does

- **Spotify Connect.** OpenMusic appears as a device named “OpenMusic”. Pick it from any Spotify app, or choose **Play on This Mac** to move playback here. The device picker lists every device on your account, moves playback between them and sets their volume.
- **Playback.** Shuffle, smart shuffle and repeat, and a queue you can reorder and clear. Gapless playback, crossfade, Automix, volume normalization (it can only turn the volume down), a sleep timer, and speed control for episodes that a podcast hosts outside Spotify. Spotify’s DJ shows up when your account has it; OpenMusic skips the DJ’s voice.
- **Home and search.** Spotify’s Home feed with Music and Podcasts filters, and Browse all with its genre pages. Search finds songs, artists, albums, playlists, podcasts, episodes, audiobooks and profiles, suggests as you type and keeps your recent searches. Paste any Spotify link or URI to open it.
- **Your Library.** The sidebar lists your playlists, albums, artists, podcasts and audiobooks, with filters, sorting, pins and folders. Like songs, save albums, follow artists, playlists and people. Create, rename and delete playlists and folders; add, remove and reorder songs; change a playlist’s name, description and cover; invite collaborators.
- **Artists, albums and songs.** Artist pages show popular songs, the discography, music videos, the biography and related artists. Each song has a page with its credits. Right-click any song, album, artist, playlist, show or episode for Spotify’s menu: add to the queue or a playlist, go to the album or artist, song radio, share, embed, hide a song in a playlist or album, stop an artist from playing, or exclude it from your taste profile.
- **Podcasts.** Show and episode pages, Your Episodes, resume where you stopped, mark as finished, and transcripts. Episodes that a podcast hosts outside Spotify stream from that host, so the host can see what you play, as with Spotify’s own apps.
- **Canvas.** Canvas loops in the dock artwork and on the Now Playing page. Artist pages list the artist’s music videos.
- **Lyrics.** Time-synced lyrics highlight and scroll with the song. Click a line to jump to it.
- **Jam and Blend.** Start a Jam, invite people with its link, or join one from a link. Create a Blend and see who is in it.
- **Local files.** Add folders in Settings to play music files from your Mac. Local songs inside your Spotify playlists play too.
- **People.** Friend Activity shows what the people you follow are playing. Profile pages show their public playlists, followers and who they follow.
- **Mac integration.** Media keys, AirPods and Control Center control playback, and the song shows in Now Playing. Spotify’s keyboard shortcuts work; press ? to list them. Closing the window keeps the music playing; the Dock icon brings it back, and its menu holds the song, playback controls and Recently Played. Option-Command-M opens a floating mini player. Settings can turn on song-change notifications, Open at Login, and a broadcast of what’s playing for scrobblers and other apps.
- **Native controls.** On macOS 26 and later the playback buttons, now-playing bar and volume are AppKit glass controls placed over the GPUI content, so songs scroll underneath them. Older systems get plain controls, without seeking and volume.

## What it does not do

- **Offline downloads, 320 kbps and lossless.** Spotify protects these files with its own encryption rather than FairPlay, and OpenMusic does not break it. Songs stream at up to 256 kbps.
- **Audiobooks.** You can browse and save them, but they do not play. OpenMusic offers to open them in Spotify.
- **Video.** Spotify sends OpenMusic audio only, so video podcasts play their sound and music videos play the song.
- **Speed on Spotify’s own podcast episodes.** Spotify refuses speed changes on this kind of device for the episodes it hosts itself.
- **Private session.** Spotify offers no way for OpenMusic to start one.
- **Free accounts.** You can sign in, browse and control Spotify on your other devices, but playing on this Mac needs Premium.

## Build from source

Install Rust and Xcode. GPUI compiles Metal shaders during the build, so Xcode’s Metal Toolchain must be installed:

```sh
xcodebuild -downloadComponent MetalToolchain
cargo run --locked
```

`./scripts/bundle-macos.sh` builds an optimized `target/OpenMusic.app`. Set `TARGET=x86_64-apple-darwin` to build for Intel. The script signs with your Apple Development or Developer ID identity when one is in your keychain, which keeps the Keychain approval across rebuilds.

## Releases

Pushes and pull requests run `.github/workflows/ci.yml`: formatting, Clippy, tests and a bundle build.

To publish a version, set `version` in `Cargo.toml`, commit, then push a matching tag:

```sh
git tag v0.2.0
git push origin v0.2.0
```

`.github/workflows/release.yml` then:

1. checks that the tag matches `Cargo.toml`;
2. builds `OpenMusic.app` for Apple silicon and Intel;
3. signs it with the certificate in `MACOS_CERTIFICATE`, and notarizes it when that is a Developer ID;
4. publishes both zips and a `SHA256SUMS` file as a GitHub release;
5. writes the new version and checksums into the Homebrew tap.

| Secret | Purpose |
|---|---|
| `MACOS_CERTIFICATE` | Developer ID Application certificate and key, as a base64 `.p12`. An Apple Development certificate works until you have one, but those builds are not notarized |
| `MACOS_CERTIFICATE_PASSWORD` | Password for that `.p12` |
| `APPLE_API_KEY` | App Store Connect API key for notarization, as a base64 `.p8` |
| `APPLE_API_KEY_ID` | ID of that key |
| `APPLE_API_ISSUER` | Issuer ID of that key |
| `TAP_DEPLOY_KEY` | Private half of a deploy key with write access to the tap repository |

Without a certificate the workflow still publishes, but signs ad hoc. macOS then treats each version as a new app, so the Keychain asks again for access to the sign-in after every upgrade. Without the deploy key it skips the Homebrew step. The cask goes to `Casks/openmusic.rb` in `<owner>/homebrew-tap`, next to your other casks; set the `HOMEBREW_TAP` repository variable to use another tap. The cask template is `packaging/homebrew/openmusic.rb`; the workflow fills in the version and checksums, and drops the quarantine step when the build is notarized.

## How playback works

OpenMusic registers with Spotify the way Spotify’s Web Playback SDK does, then plays audio natively:

1. Sign-in uses Spotify’s OAuth device flow (`oauth.rs`).
2. A websocket to Spotify’s dealer service delivers commands (`dealer.rs`).
3. OpenMusic registers a track-playback device and follows the state machine Spotify sends, reporting playback state back (`engine/`).
4. Audio is Spotify’s FairPlay-protected AAC. OpenMusic builds an HLS playlist from the file’s segment table and plays it with AVFoundation. macOS FairPlay decrypts it with a license from Spotify, so the content protection stays intact (`player.rs`).
5. Episodes hosted outside Spotify and local files play as plain files through the same player.
6. Metadata, Home, search, your library and lyrics come from Spotify’s internal APIs (`catalog.rs`, `pathfinder.rs` and the modules beside them). Other devices are mirrored through a Connect observer (`connect.rs`).

The protobuf schemas in `crates/spotify-proto/proto` were extracted from the Spotify macOS app; see `docs/research/proto-extraction.md`. The rest of the protocol research is in `docs/research/`.

## Disclaimer

OpenMusic is not affiliated with or endorsed by Spotify. It uses Spotify’s private interfaces, which is likely outside Spotify’s developer terms, and Spotify can change those interfaces at any time.
