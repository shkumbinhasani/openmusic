use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::{
    AllocAnyThread, MainThreadMarker, Message, define_class, msg_send,
    rc::Retained,
    runtime::{Bool, NSObject, ProtocolObject},
};
use objc2_av_foundation::{
    AVAssetResourceLoader, AVAssetResourceLoaderDelegate, AVAssetResourceLoadingRequest,
    AVContentKeyRequest, AVContentKeyResponse, AVContentKeySession, AVContentKeySessionDelegate,
    AVContentKeySystemFairPlayStreaming, AVPlayer, AVPlayerItem, AVPlayerItemStatus, AVURLAsset,
};
use objc2_core_media::CMTime;
use objc2_foundation::{NSData, NSError, NSObjectProtocol, NSString, NSURL};
use tokio::{runtime::Handle, sync::mpsc::UnboundedSender};

const SCHEME: &str = "openmusic-hls";

#[derive(Clone)]
pub struct Media {
    pub file_id: String,
    pub cdn_url: String,
    pub init_size: u64,
    pub segments: Vec<(u64, f64)>,
    pub certificate: Arc<Vec<u8>>,
    pub license_url: String,
    pub token: String,
    pub start_ms: u32,
    pub play: bool,
}

#[derive(Clone, Debug)]
pub enum PlayerEvent {
    Ready {
        generation: u64,
        duration_ms: u32,
    },
    Position {
        generation: u64,
        position_ms: u32,
        playing: bool,
    },
    Ended {
        generation: u64,
    },
    Failed {
        generation: u64,
        message: String,
    },
}

#[derive(Clone)]
pub struct Player {
    shared: Arc<Shared>,
}

struct Shared {
    events: UnboundedSender<PlayerEvent>,
    http: reqwest::Client,
    runtime: Handle,
}

struct Licensing {
    certificate: Arc<Vec<u8>>,
    license_url: String,
    token: String,
}

struct MainState {
    player: Retained<AVPlayer>,
    session: Retained<AVContentKeySession>,
    loader: Retained<LoaderDelegate>,
    _keys: Retained<KeyDelegate>,
    item: Option<Retained<AVPlayerItem>>,
    generation: u64,
    reported_ready: bool,
    reported_end: bool,
    pending_seek: Option<u32>,
}

thread_local! {
    static MAIN: RefCell<Option<MainState>> = const { RefCell::new(None) };
}

static PLAYLISTS: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
static LICENSING: OnceLock<Mutex<Option<Licensing>>> = OnceLock::new();
static SHARED: OnceLock<Arc<Shared>> = OnceLock::new();

fn playlists() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    PLAYLISTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn licensing() -> &'static Mutex<Option<Licensing>> {
    LICENSING.get_or_init(|| Mutex::new(None))
}

struct Sendable<T>(T);
unsafe impl<T> Send for Sendable<T> {}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "OpenMusicPlaylistLoader"]
    struct LoaderDelegate;

    unsafe impl NSObjectProtocol for LoaderDelegate {}

    unsafe impl AVAssetResourceLoaderDelegate for LoaderDelegate {
        #[unsafe(method(resourceLoader:shouldWaitForLoadingOfRequestedResource:))]
        fn should_wait(
            &self,
            _loader: &AVAssetResourceLoader,
            request: &AVAssetResourceLoadingRequest,
        ) -> Bool {
            let key = unsafe { request.request() }
                .URL()
                .and_then(|url| url.host())
                .map(|host| host.to_string())
                .unwrap_or_default();
            let Some(playlist) = playlists()
                .lock()
                .ok()
                .and_then(|map| map.get(&key).cloned())
            else {
                return Bool::NO;
            };
            unsafe {
                if let Some(info) = request.contentInformationRequest() {
                    info.setContentType(Some(&NSString::from_str("public.m3u-playlist")));
                    info.setContentLength(playlist.len() as i64);
                    info.setByteRangeAccessSupported(false);
                }
                if let Some(data) = request.dataRequest() {
                    data.respondWithData(&NSData::with_bytes(&playlist));
                }
                request.finishLoading();
            }
            Bool::YES
        }
    }
);

impl LoaderDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "OpenMusicKeyDelegate"]
    struct KeyDelegate;

    unsafe impl NSObjectProtocol for KeyDelegate {}

    unsafe impl AVContentKeySessionDelegate for KeyDelegate {
        #[unsafe(method(contentKeySession:didProvideContentKeyRequest:))]
        fn did_provide(&self, _session: &AVContentKeySession, request: &AVContentKeyRequest) {
            provide_key(request);
        }

        #[unsafe(method(contentKeySession:contentKeyRequest:didFailWithError:))]
        fn did_fail(
            &self,
            _session: &AVContentKeySession,
            _request: &AVContentKeyRequest,
            error: &NSError,
        ) {
            let message = error.localizedDescription().to_string();
            let generation = MAIN.with_borrow(|state| state.as_ref().map(|state| state.generation));
            if let (Some(shared), Some(generation)) = (SHARED.get(), generation) {
                let _ = shared.events.send(PlayerEvent::Failed {
                    generation,
                    message,
                });
            } else if let Some(shared) = SHARED.get() {
                let _ = shared.events.send(PlayerEvent::Failed {
                    generation: 0,
                    message,
                });
            }
        }
    }
);

impl KeyDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

fn content_id(request: &AVContentKeyRequest) -> String {
    unsafe { request.identifier() }
        .and_then(|identifier| identifier.downcast::<NSString>().ok())
        .map(|identifier| identifier.to_string())
        .map(|identifier| identifier.trim_start_matches("skd://").to_owned())
        .unwrap_or_default()
}

fn provide_key(request: &AVContentKeyRequest) {
    let Some((certificate, license_url, token)) = licensing().lock().ok().and_then(|licensing| {
        licensing.as_ref().map(|licensing| {
            (
                licensing.certificate.clone(),
                licensing.license_url.clone(),
                licensing.token.clone(),
            )
        })
    }) else {
        return;
    };
    let Some(shared) = SHARED.get().cloned() else {
        return;
    };
    let identifier = content_id(request);
    let captured = Sendable(request.retain());
    let handler = RcBlock::new(move |spc: *mut NSData, _error: *mut NSError| {
        let request = Sendable(captured.0.clone());
        let Some(spc) = (unsafe { spc.as_ref() }) else {
            fail_key(
                &request.0,
                &shared,
                "FairPlay could not create a license request.",
            );
            return;
        };
        let spc = spc.to_vec();
        let shared_for_task = shared.clone();
        let license_url = license_url.clone();
        let token = token.clone();
        shared.runtime.spawn(async move {
            let request = request;
            let separator = if license_url.contains('?') { '&' } else { '?' };
            let response = shared_for_task
                .http
                .post(format!("{license_url}{separator}assetId=hex"))
                .bearer_auth(&token)
                .body(spc)
                .send()
                .await;
            match response {
                Ok(response) if response.status().is_success() => match response.bytes().await {
                    Ok(ckc) => unsafe {
                        let response =
                            AVContentKeyResponse::contentKeyResponseWithFairPlayStreamingKeyResponseData(
                                &NSData::with_bytes(&ckc),
                            );
                        request.0.processContentKeyResponse(&response);
                    },
                    Err(_) => fail_key(&request.0, &shared_for_task, "Spotify's license was unreadable."),
                },
                Ok(response) => fail_key(
                    &request.0,
                    &shared_for_task,
                    &format!("Spotify refused the license ({}).", response.status()),
                ),
                Err(_) => fail_key(&request.0, &shared_for_task, "Could not reach Spotify's license service."),
            }
        });
    });
    unsafe {
        request
            .makeStreamingContentKeyRequestDataForApp_contentIdentifier_options_completionHandler(
                &NSData::with_bytes(&certificate),
                Some(&NSData::with_bytes(identifier.as_bytes())),
                None,
                &handler,
            );
    }
}

fn fail_key(request: &AVContentKeyRequest, shared: &Shared, message: &str) {
    let error = NSError::new(-1, &NSString::from_str("OpenMusicLicense"));
    unsafe { request.processContentKeyResponseError(&error) };
    let _ = shared.events.send(PlayerEvent::Failed {
        generation: 0,
        message: message.to_owned(),
    });
}

fn playlist(media: &Media) -> Vec<u8> {
    let target = media
        .segments
        .iter()
        .map(|(_, seconds)| *seconds)
        .fold(0.0f64, f64::max)
        .ceil() as u64
        + 1;
    let mut lines = vec![
        "#EXTM3U".to_owned(),
        "#EXT-X-VERSION:6".to_owned(),
        format!("#EXT-X-TARGETDURATION:{target}"),
        "#EXT-X-MEDIA-SEQUENCE:0".to_owned(),
        "#EXT-X-PLAYLIST-TYPE:VOD".to_owned(),
        format!(
            "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://{}\",KEYFORMATVERSIONS=\"1\",KEYFORMAT=\"com.apple.streamingkeydelivery\"",
            media.file_id
        ),
        format!(
            "#EXT-X-MAP:URI=\"{}\",BYTERANGE=\"{}@0\"",
            media.cdn_url, media.init_size
        ),
    ];
    let mut start = media.init_size;
    for (size, seconds) in &media.segments {
        lines.push(format!("#EXTINF:{seconds:.6},"));
        lines.push(format!("#EXT-X-BYTERANGE:{size}@{start}"));
        lines.push(media.cdn_url.clone());
        start += size;
    }
    lines.push("#EXT-X-ENDLIST".to_owned());
    lines.join("\n").into_bytes()
}

fn on_main(work: impl FnOnce(MainThreadMarker, &mut MainState) + Send + 'static) {
    DispatchQueue::main().exec_async(move || {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        MAIN.with_borrow_mut(|state| {
            if state.is_none() {
                *state = Some(create_main_state(mtm));
            }
            if let Some(state) = state.as_mut() {
                work(mtm, state);
            }
        });
    });
}

fn create_main_state(mtm: MainThreadMarker) -> MainState {
    let player = unsafe { AVPlayer::new(mtm) };
    let session = unsafe {
        AVContentKeySession::contentKeySessionWithKeySystem(AVContentKeySystemFairPlayStreaming)
    };
    let keys = KeyDelegate::new();
    let key_queue = DispatchQueue::new("openmusic.fairplay", None);
    unsafe {
        session.setDelegate_queue(Some(ProtocolObject::from_ref(&*keys)), Some(&key_queue));
    }
    let state = MainState {
        player,
        session,
        loader: LoaderDelegate::new(),
        _keys: keys,
        item: None,
        generation: 0,
        reported_ready: false,
        reported_end: false,
        pending_seek: None,
    };
    install_observer(&state.player);
    state
}

fn install_observer(player: &AVPlayer) {
    let block = RcBlock::new(|_time: CMTime| {
        MAIN.with_borrow_mut(|state| {
            let (Some(state), Some(shared)) = (state.as_mut(), SHARED.get()) else {
                return;
            };
            let Some(item) = state.item.clone() else {
                return;
            };
            let generation = state.generation;
            let status = unsafe { item.status() };
            if status == AVPlayerItemStatus::Failed {
                if !state.reported_end {
                    state.reported_end = true;
                    let message = unsafe { item.error() }
                        .map(|error| error.localizedDescription().to_string())
                        .unwrap_or_else(|| "Playback failed.".into());
                    let _ = shared.events.send(PlayerEvent::Failed {
                        generation,
                        message,
                    });
                }
                return;
            }
            if status != AVPlayerItemStatus::ReadyToPlay {
                return;
            }
            let duration = unsafe { item.duration().seconds() };
            if !state.reported_ready {
                state.reported_ready = true;
                if let Some(seek) = state.pending_seek.take() {
                    seek_player(&state.player, seek);
                }
                let _ = shared.events.send(PlayerEvent::Ready {
                    generation,
                    duration_ms: if duration.is_finite() {
                        (duration * 1000.) as u32
                    } else {
                        0
                    },
                });
            }
            let position = unsafe { state.player.currentTime().seconds() };
            let playing = unsafe { state.player.rate() } > 0.;
            if position.is_finite() {
                let _ = shared.events.send(PlayerEvent::Position {
                    generation,
                    position_ms: (position.max(0.) * 1000.) as u32,
                    playing,
                });
            }
            if duration.is_finite()
                && duration > 0.
                && position.is_finite()
                && position >= duration - 0.35
                && !state.reported_end
            {
                state.reported_end = true;
                let _ = shared.events.send(PlayerEvent::Ended { generation });
            }
        });
    });
    unsafe {
        let observer = player.addPeriodicTimeObserverForInterval_queue_usingBlock(
            CMTime::with_seconds(0.25, 1000),
            Some(DispatchQueue::main()),
            &block,
        );
        std::mem::forget(observer);
    }
}

fn seek_player(player: &AVPlayer, position_ms: u32) {
    unsafe {
        player.seekToTime(CMTime::with_seconds(f64::from(position_ms) / 1000., 1000));
    }
}

impl Player {
    pub fn new(
        events: UnboundedSender<PlayerEvent>,
        http: reqwest::Client,
        runtime: Handle,
    ) -> Self {
        let shared = SHARED
            .get_or_init(|| {
                Arc::new(Shared {
                    events,
                    http,
                    runtime,
                })
            })
            .clone();
        Self { shared }
    }

    pub fn load(&self, generation: u64, media: Media) {
        let _ = &self.shared;
        let key = format!("track{generation}");
        if let Ok(mut map) = playlists().lock() {
            map.clear();
            map.insert(key.clone(), playlist(&media));
        }
        if let Ok(mut licensing) = licensing().lock() {
            *licensing = Some(Licensing {
                certificate: media.certificate.clone(),
                license_url: media.license_url.clone(),
                token: media.token.clone(),
            });
        }
        on_main(move |mtm, state| unsafe {
            let Some(url) = NSURL::URLWithString(&NSString::from_str(&format!(
                "{SCHEME}://{key}/playlist.m3u8"
            ))) else {
                return;
            };
            let asset = AVURLAsset::URLAssetWithURL_options(&url, None);
            let queue = DispatchQueue::new("openmusic.playlist", None);
            asset
                .resourceLoader()
                .setDelegate_queue(Some(ProtocolObject::from_ref(&*state.loader)), Some(&queue));
            state
                .session
                .addContentKeyRecipient(ProtocolObject::from_ref(&*asset));
            let item = AVPlayerItem::playerItemWithAsset(&asset, mtm);
            state.generation = generation;
            state.reported_ready = false;
            state.reported_end = false;
            state.pending_seek = (media.start_ms > 0).then_some(media.start_ms);
            state.player.replaceCurrentItemWithPlayerItem(Some(&item));
            state.item = Some(item);
            if media.play {
                state.player.play();
            } else {
                state.player.pause();
            }
        });
    }

    pub fn update_token(&self, token: String) {
        if let Ok(mut licensing) = licensing().lock()
            && let Some(licensing) = licensing.as_mut()
        {
            licensing.token = token;
        }
    }

    pub fn play(&self) {
        on_main(|_, state| unsafe { state.player.play() });
    }

    pub fn pause(&self) {
        on_main(|_, state| unsafe { state.player.pause() });
    }

    pub fn seek(&self, position_ms: u32) {
        on_main(move |_, state| {
            if state.reported_ready {
                seek_player(&state.player, position_ms);
            } else {
                state.pending_seek = Some(position_ms);
            }
        });
    }

    pub fn set_volume(&self, volume: f32) {
        on_main(move |_, state| unsafe { state.player.setVolume(volume.clamp(0., 1.)) });
    }

    pub fn stop(&self) {
        on_main(|_, state| unsafe {
            state.player.pause();
            state.player.replaceCurrentItemWithPlayerItem(None);
            state.item = None;
        });
    }
}
