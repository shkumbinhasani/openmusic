use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    ptr::NonNull,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchTime};
use objc2::{
    AllocAnyThread, MainThreadMarker, Message, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, NSObject, ProtocolObject},
};
use objc2_av_foundation::{
    AVAssetResourceLoader, AVAssetResourceLoaderDelegate, AVAssetResourceLoadingRequest,
    AVContentKeyRequest, AVContentKeyResponse, AVContentKeySession, AVContentKeySessionDelegate,
    AVContentKeySystemFairPlayStreaming, AVPlayerItem, AVPlayerItemDidPlayToEndTimeNotification,
    AVPlayerItemFailedToPlayToEndTimeNotification, AVPlayerItemStatus, AVQueuePlayer, AVURLAsset,
};
use objc2_core_media::CMTime;
use objc2_foundation::{
    NSData, NSError, NSNotification, NSNotificationCenter, NSObjectProtocol, NSString, NSURL,
};
use tokio::{runtime::Handle, sync::mpsc::UnboundedSender};

const SCHEME: &str = "openmusic-hls";
const TICK: Duration = Duration::from_millis(250);
const FAST_TICK: Duration = Duration::from_millis(50);
const RAMP_STEP: Duration = Duration::from_millis(20);

#[derive(Clone)]
pub struct FairPlay {
    pub file_id: String,
    pub cdn_url: String,
    pub init_size: u64,
    pub segments: Vec<(u64, f64)>,
    pub certificate: Arc<Vec<u8>>,
    pub license_url: String,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hls {
    pub playlists: Vec<(String, Vec<u8>)>,
    pub root: String,
}

#[derive(Clone)]
pub enum Source {
    FairPlay(FairPlay),
    Url(String),
    Hls(Hls),
    File(PathBuf),
}

impl Source {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::FairPlay(_) => "fairplay",
            Self::Url(_) => "url",
            Self::Hls(_) => "hls",
            Self::File(_) => "file",
        }
    }
}

#[derive(Clone)]
pub struct Media {
    pub source: Source,
    pub start_ms: u32,
    pub play: bool,
    pub gain: f32,
    pub speed: f32,
}

impl Media {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            start_ms: 0,
            play: true,
            gain: 1.,
            speed: 1.,
        }
    }
}

pub fn valid_rate(rate: f32) -> f32 {
    if rate.is_finite() && rate > 0. {
        rate.clamp(0.25, 4.)
    } else {
        1.
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    Gapless,
    Crossfade,
}

impl Join {
    pub fn name(self) -> &'static str {
        match self {
            Self::Gapless => "gapless",
            Self::Crossfade => "crossfade",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
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
    Advanced {
        from: u64,
        to: u64,
    },
    Ended {
        generation: u64,
    },
    Failed {
        generation: u64,
        message: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DeckSnapshot {
    pub items: Vec<u64>,
    pub current: Option<u64>,
    pub fade: f32,
    pub volume: f32,
    pub rate: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub decks: [DeckSnapshot; 2],
    pub active: usize,
    pub master: f32,
    pub playing: bool,
    pub ramp: Option<f32>,
    pub quiet: f32,
}

pub fn fade_levels(progress: f32) -> (f32, f32) {
    let angle = progress.clamp(0., 1.) * std::f32::consts::FRAC_PI_2;
    (angle.cos().clamp(0., 1.), angle.sin().clamp(0., 1.))
}

pub fn deck_volume(master: f32, gain: f32, fade: f32) -> f32 {
    (master * gain * fade).clamp(0., 1.)
}

#[derive(Clone)]
pub struct Player {
    _shared: Arc<Shared>,
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

struct Item {
    generation: u64,
    item: Retained<AVPlayerItem>,
    asset: Retained<AVURLAsset>,
    key: String,
    keyed: bool,
    file_id: Option<String>,
    gain: f32,
    rate: f32,
    ready: bool,
    failed: bool,
    pending_seek: Option<u32>,
}

struct Deck {
    player: Retained<AVQueuePlayer>,
    items: Vec<Item>,
    fade: f32,
}

struct Ramp {
    from: usize,
    duration: Duration,
    elapsed: Duration,
    resumed: Option<Instant>,
}

impl Ramp {
    fn progress(&self) -> f32 {
        let elapsed = self.elapsed + self.resumed.map_or(Duration::ZERO, |at| at.elapsed());
        if self.duration.is_zero() {
            1.
        } else {
            (elapsed.as_secs_f32() / self.duration.as_secs_f32()).clamp(0., 1.)
        }
    }
}

struct Fade {
    started: Instant,
    duration: Duration,
}

impl Fade {
    fn progress(&self) -> f32 {
        if self.duration.is_zero() {
            1.
        } else {
            (self.started.elapsed().as_secs_f32() / self.duration.as_secs_f32()).clamp(0., 1.)
        }
    }
}

struct MainState {
    decks: [Deck; 2],
    active: usize,
    session: Retained<AVContentKeySession>,
    loader: Retained<LoaderDelegate>,
    _keys: Retained<KeyDelegate>,
    _observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    master: f32,
    quiet: f32,
    fade_out: Option<Fade>,
    playing: bool,
    ramp: Option<Ramp>,
    ticking: bool,
    stepping: bool,
}

thread_local! {
    static MAIN: RefCell<Option<MainState>> = const { RefCell::new(None) };
}

static PLAYLISTS: OnceLock<Mutex<HashMap<String, Vec<u8>>>> = OnceLock::new();
static LICENSING: OnceLock<Mutex<Option<Licensing>>> = OnceLock::new();
static FILES: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
static SHARED: Mutex<Option<Arc<Shared>>> = Mutex::new(None);
static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);

fn playlists() -> &'static Mutex<HashMap<String, Vec<u8>>> {
    PLAYLISTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn licensing() -> &'static Mutex<Option<Licensing>> {
    LICENSING.get_or_init(|| Mutex::new(None))
}

fn files() -> &'static Mutex<HashMap<String, u64>> {
    FILES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shared() -> Option<Arc<Shared>> {
    SHARED.lock().ok().and_then(|shared| shared.clone())
}

fn emit(event: PlayerEvent) {
    if let Some(shared) = shared() {
        let _ = shared.events.send(event);
    }
}

fn playlist_key(host: &str, path: &str) -> String {
    format!("{host}/{}", path.trim_start_matches('/'))
}

fn generation_for(file_id: &str) -> u64 {
    files()
        .lock()
        .ok()
        .and_then(|files| files.get(file_id).copied())
        .unwrap_or(0)
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
                .and_then(|url| {
                    let host = url.host()?.to_string();
                    let path = url.path().map(|path| path.to_string()).unwrap_or_default();
                    Some(playlist_key(&host, &path))
                })
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
            request: &AVContentKeyRequest,
            error: &NSError,
        ) {
            emit(PlayerEvent::Failed {
                generation: generation_for(&content_id(request)),
                message: error.localizedDescription().to_string(),
            });
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
    let Some(shared) = shared() else {
        return;
    };
    let identifier = content_id(request);
    let captured = Sendable(request.retain());
    let handler = RcBlock::new(move |spc: *mut NSData, _error: *mut NSError| {
        let request = Sendable(captured.0.clone());
        let Some(spc) = (unsafe { spc.as_ref() }) else {
            fail_key(&request.0, "FairPlay could not create a license request.");
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
                    Err(_) => fail_key(&request.0, "Spotify's license was unreadable."),
                },
                Ok(response) => fail_key(
                    &request.0,
                    &format!("Spotify refused the license ({}).", response.status()),
                ),
                Err(_) => fail_key(&request.0, "Could not reach Spotify's license service."),
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

fn fail_key(request: &AVContentKeyRequest, message: &str) {
    let error = NSError::new(-1, &NSString::from_str("OpenMusicLicense"));
    unsafe { request.processContentKeyResponseError(&error) };
    emit(PlayerEvent::Failed {
        generation: generation_for(&content_id(request)),
        message: message.to_owned(),
    });
}

fn playlist(media: &FairPlay) -> Vec<u8> {
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
                state.update_snapshot();
            }
        });
    });
}

fn with_main(
    work: impl FnOnce(&mut MainState) -> Option<Duration>,
) -> Result<Option<Duration>, ()> {
    MAIN.with(|cell| match cell.try_borrow_mut() {
        Ok(mut state) => Ok(state.as_mut().and_then(work)),
        Err(_) => Err(()),
    })
}

fn deck(mtm: MainThreadMarker) -> Deck {
    Deck {
        player: unsafe { AVQueuePlayer::new(mtm) },
        items: Vec::new(),
        fade: 0.,
    }
}

fn create_main_state(mtm: MainThreadMarker) -> MainState {
    let session = unsafe {
        AVContentKeySession::contentKeySessionWithKeySystem(AVContentKeySystemFairPlayStreaming)
    };
    let keys = KeyDelegate::new();
    let key_queue = DispatchQueue::new("openmusic.fairplay", None);
    unsafe {
        session.setDelegate_queue(Some(ProtocolObject::from_ref(&*keys)), Some(&key_queue));
    }
    let mut decks = [deck(mtm), deck(mtm)];
    decks[0].fade = 1.;
    MainState {
        decks,
        active: 0,
        session,
        loader: LoaderDelegate::new(),
        _keys: keys,
        _observers: vec![
            observe_items(unsafe { AVPlayerItemDidPlayToEndTimeNotification }, false),
            observe_items(
                unsafe { AVPlayerItemFailedToPlayToEndTimeNotification },
                true,
            ),
        ],
        master: 1.,
        quiet: 1.,
        fade_out: None,
        playing: false,
        ramp: None,
        ticking: false,
        stepping: false,
    }
}

fn observe_items(
    name: &objc2_foundation::NSNotificationName,
    failed: bool,
) -> Retained<ProtocolObject<dyn NSObjectProtocol>> {
    let block = RcBlock::new(move |notification: NonNull<NSNotification>| {
        let notification = unsafe { notification.as_ref() };
        let Some(object) = notification.object() else {
            return;
        };
        let object = Sendable(object);
        DispatchQueue::main().exec_async(move || {
            let object = object;
            item_finished(object, failed);
        });
    });
    unsafe {
        NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
            Some(name),
            None,
            None,
            &block,
        )
    }
}

fn item_finished(object: Sendable<Retained<AnyObject>>, failed: bool) {
    let handled = with_main(|state| {
        state.item_finished(&object.0, failed);
        state.update_snapshot();
        None
    });
    if handled.is_err() {
        DispatchQueue::main().exec_async(move || {
            let object = object;
            item_finished(object, failed);
        });
    }
}

fn schedule(delay: Duration, work: fn()) {
    if let Ok(when) = DispatchTime::try_from(delay) {
        let _ = DispatchQueue::main().after(when, work);
    }
}

fn tick() {
    match with_main(MainState::poll) {
        Ok(Some(delay)) => schedule(delay, tick),
        Ok(None) => {}
        Err(()) => schedule(FAST_TICK, tick),
    }
}

fn step() {
    match with_main(MainState::step) {
        Ok(Some(delay)) => schedule(delay, step),
        Ok(None) => {}
        Err(()) => schedule(RAMP_STEP, step),
    }
}

fn seek_player(player: &AVQueuePlayer, position_ms: u32) {
    unsafe {
        player.seekToTime(CMTime::with_seconds(f64::from(position_ms) / 1000., 1000));
    }
}

fn milliseconds(seconds: f64) -> Option<u32> {
    seconds
        .is_finite()
        .then(|| (seconds.max(0.) * 1000.) as u32)
}

fn is_current(player: &AVQueuePlayer, item: &Item) -> bool {
    unsafe { player.currentItem() }.is_some_and(|current| current == item.item)
}

impl MainState {
    fn idle(&self) -> usize {
        1 - self.active
    }

    fn find(&self, generation: u64) -> Option<(usize, usize)> {
        self.decks.iter().enumerate().find_map(|(deck, contents)| {
            contents
                .items
                .iter()
                .position(|item| item.generation == generation)
                .map(|index| (deck, index))
        })
    }

    fn find_object(&self, object: &AnyObject) -> Option<(usize, usize)> {
        let pointer = object as *const AnyObject;
        self.decks.iter().enumerate().find_map(|(deck, contents)| {
            contents
                .items
                .iter()
                .position(|item| Retained::as_ptr(&item.item).cast::<AnyObject>() == pointer)
                .map(|index| (deck, index))
        })
    }

    fn make_item(&mut self, mtm: MainThreadMarker, generation: u64, media: &Media) -> Option<Item> {
        let key = format!("track{generation}");
        let store = |name: &str, bytes: Vec<u8>| {
            if let Ok(mut map) = playlists().lock() {
                map.insert(playlist_key(&key, name), bytes);
            }
        };
        let (url, file_id, loaded, keyed) = match &media.source {
            Source::FairPlay(fairplay) => {
                store("playlist.m3u8", playlist(fairplay));
                if let Ok(mut licensing) = licensing().lock() {
                    *licensing = Some(Licensing {
                        certificate: fairplay.certificate.clone(),
                        license_url: fairplay.license_url.clone(),
                        token: fairplay.token.clone(),
                    });
                }
                if let Ok(mut files) = files().lock() {
                    files.insert(fairplay.file_id.clone(), generation);
                }
                (
                    NSURL::URLWithString(&NSString::from_str(&format!(
                        "{SCHEME}://{key}/playlist.m3u8"
                    ))),
                    Some(fairplay.file_id.clone()),
                    true,
                    true,
                )
            }
            Source::Hls(hls) => {
                for (name, bytes) in &hls.playlists {
                    store(name, bytes.clone());
                }
                (
                    NSURL::URLWithString(&NSString::from_str(&format!(
                        "{SCHEME}://{key}/{}",
                        hls.root.trim_start_matches('/')
                    ))),
                    None,
                    true,
                    false,
                )
            }
            Source::Url(url) => (
                NSURL::URLWithString(&NSString::from_str(url)),
                None,
                false,
                false,
            ),
            Source::File(path) => (
                Some(NSURL::fileURLWithPath(&NSString::from_str(
                    &path.to_string_lossy(),
                ))),
                None,
                false,
                false,
            ),
        };
        let Some(url) = url else {
            self.forget(&key, file_id.as_deref(), generation);
            return None;
        };
        let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
        if loaded {
            let queue = DispatchQueue::new("openmusic.playlist", None);
            unsafe {
                asset
                    .resourceLoader()
                    .setDelegate_queue(Some(ProtocolObject::from_ref(&*self.loader)), Some(&queue));
            }
        }
        if keyed {
            unsafe {
                self.session
                    .addContentKeyRecipient(ProtocolObject::from_ref(&*asset));
            }
        }
        let item = unsafe { AVPlayerItem::playerItemWithAsset(&asset, mtm) };
        Some(Item {
            generation,
            item,
            asset,
            key,
            keyed,
            file_id,
            gain: media.gain.max(0.),
            rate: valid_rate(media.speed),
            ready: false,
            failed: false,
            pending_seek: (media.start_ms > 0).then_some(media.start_ms),
        })
    }

    fn forget(&self, key: &str, file_id: Option<&str>, generation: u64) {
        let prefix = format!("{key}/");
        if let Ok(mut map) = playlists().lock() {
            map.retain(|name, _| !name.starts_with(&prefix));
        }
        if let (Some(file_id), Ok(mut files)) = (file_id, files().lock())
            && files.get(file_id) == Some(&generation)
        {
            files.remove(file_id);
        }
    }

    fn release(&self, item: Item) {
        self.forget(&item.key, item.file_id.as_deref(), item.generation);
        if item.keyed {
            unsafe {
                self.session
                    .removeContentKeyRecipient(ProtocolObject::from_ref(&*item.asset));
            }
        }
    }

    fn prefetch(&self, item: &Item) {
        let Some(file_id) = &item.file_id else {
            return;
        };
        let identifier = NSString::from_str(&format!("skd://{file_id}"));
        let identifier: &AnyObject = &identifier;
        unsafe {
            self.session
                .processContentKeyRequestWithIdentifier_initializationData_options(
                    Some(identifier),
                    None,
                    None,
                );
        }
    }

    fn clear_deck(&mut self, index: usize) {
        let items = {
            let deck = &mut self.decks[index];
            unsafe {
                deck.player.pause();
                deck.player.removeAllItems();
            }
            deck.fade = 0.;
            std::mem::take(&mut deck.items)
        };
        for item in items {
            self.release(item);
        }
    }

    fn drop_queued(&mut self, index: usize) {
        while self.decks[index].items.len() > 1 {
            if let Some(item) = self.decks[index].items.pop() {
                unsafe { self.decks[index].player.removeItem(&item.item) };
                self.release(item);
            }
        }
    }

    fn apply_volumes(&self) {
        self.apply_rates();
        for deck in &self.decks {
            let gain = deck.items.first().map_or(1., |item| item.gain);
            unsafe {
                deck.player
                    .setVolume(deck_volume(self.master * self.quiet, gain, deck.fade))
            };
        }
    }

    fn apply_rates(&self) {
        for deck in &self.decks {
            let rate = deck.items.first().map_or(1., |item| item.rate);
            unsafe {
                if deck.player.defaultRate() != rate {
                    deck.player.setDefaultRate(rate);
                }
                let current = deck.player.rate();
                if current > 0. && current != rate {
                    deck.player.setRate(rate);
                }
            }
        }
    }

    fn set_rate(&mut self, generation: u64, rate: f32) {
        if let Some((deck, index)) = self.find(generation) {
            self.decks[deck].items[index].rate = valid_rate(rate);
            self.apply_rates();
        }
    }

    fn start_active(&self) {
        let deck = &self.decks[self.active];
        if self.playing
            && deck
                .items
                .first()
                .is_some_and(|item| item.pending_seek.is_none())
        {
            unsafe { deck.player.play() };
        }
    }

    fn ensure_ticking(&mut self) {
        if !self.ticking {
            self.ticking = true;
            schedule(FAST_TICK, tick);
        }
    }

    fn ensure_stepping(&mut self) {
        if !self.stepping {
            self.stepping = true;
            schedule(RAMP_STEP, step);
        }
    }

    fn finish_ramp(&mut self) {
        if let Some(ramp) = self.ramp.take() {
            self.clear_deck(ramp.from);
            self.decks[self.active].fade = 1.;
            self.apply_volumes();
        }
    }

    fn switch_to(&mut self, index: usize) {
        self.finish_ramp();
        if index != self.active {
            self.clear_deck(self.active);
            self.active = index;
        }
        self.decks[index].fade = 1.;
        self.apply_volumes();
        self.start_active();
    }

    fn load(&mut self, mtm: MainThreadMarker, generation: u64, media: Media) {
        self.ramp = None;
        self.clear_deck(0);
        self.clear_deck(1);
        self.playing = media.play;
        let Some(item) = self.make_item(mtm, generation, &media) else {
            emit(PlayerEvent::Failed {
                generation,
                message: "OpenMusic could not open this audio.".into(),
            });
            return;
        };
        let deck = &mut self.decks[self.active];
        unsafe {
            deck.player.pause();
            deck.player.insertItem_afterItem(&item.item, None);
        }
        deck.fade = 1.;
        deck.items.push(item);
        self.apply_volumes();
        self.start_active();
        self.ensure_ticking();
    }

    fn prepare(&mut self, mtm: MainThreadMarker, generation: u64, media: Media, join: Join) {
        let deck = match join {
            Join::Gapless => {
                let active = self.active;
                let current_is_head = self.decks[active]
                    .items
                    .first()
                    .is_some_and(|head| is_current(&self.decks[active].player, head));
                if !current_is_head {
                    emit(PlayerEvent::Failed {
                        generation,
                        message: "Nothing is playing to join the next track to.".into(),
                    });
                    return;
                }
                self.drop_queued(active);
                active
            }
            Join::Crossfade => {
                self.finish_ramp();
                let idle = self.idle();
                self.clear_deck(idle);
                idle
            }
        };
        let Some(item) = self.make_item(mtm, generation, &media) else {
            emit(PlayerEvent::Failed {
                generation,
                message: "OpenMusic could not open the next track.".into(),
            });
            return;
        };
        let player = self.decks[deck].player.clone();
        let after = self.decks[deck].items.first().map(|head| head.item.clone());
        let fits = unsafe { player.canInsertItem_afterItem(&item.item, after.as_deref()) };
        if !fits {
            self.release(item);
            emit(PlayerEvent::Failed {
                generation,
                message: "The next track could not be queued.".into(),
            });
            return;
        }
        unsafe { player.insertItem_afterItem(&item.item, after.as_deref()) };
        if join == Join::Crossfade {
            unsafe { player.pause() };
            self.decks[deck].fade = 0.;
        }
        self.prefetch(&item);
        self.decks[deck].items.push(item);
        self.apply_volumes();
        self.ensure_ticking();
    }

    fn discard(&mut self, generation: u64) {
        let Some((deck, index)) = self.find(generation) else {
            return;
        };
        let outgoing = self.ramp.as_ref().is_some_and(|ramp| ramp.from == deck);
        if (deck == self.active && index == 0) || outgoing {
            return;
        }
        let item = self.decks[deck].items.remove(index);
        unsafe { self.decks[deck].player.removeItem(&item.item) };
        self.release(item);
        self.apply_volumes();
    }

    fn promote(&mut self, generation: u64) {
        self.finish_ramp();
        let Some((deck, index)) = self.find(generation) else {
            emit(PlayerEvent::Failed {
                generation,
                message: "The next track was not loaded.".into(),
            });
            return;
        };
        for _ in 0..index {
            let item = self.decks[deck].items.remove(0);
            unsafe { self.decks[deck].player.advanceToNextItem() };
            self.release(item);
        }
        self.switch_to(deck);
        self.ensure_ticking();
    }

    fn crossfade(&mut self, generation: u64, duration: Duration) {
        self.finish_ramp();
        let idle = self.idle();
        let waiting = self.decks[idle]
            .items
            .first()
            .is_some_and(|item| item.generation == generation);
        if !waiting {
            self.promote(generation);
            return;
        }
        if !self.playing || duration.is_zero() {
            self.switch_to(idle);
            return;
        }
        let from = self.active;
        self.active = idle;
        self.decks[idle].fade = 0.;
        self.ramp = Some(Ramp {
            from,
            duration,
            elapsed: Duration::ZERO,
            resumed: Some(Instant::now()),
        });
        self.apply_volumes();
        self.start_active();
        self.ensure_stepping();
        self.ensure_ticking();
    }

    fn step_ramp(&mut self) -> bool {
        let Some(ramp) = self.ramp.as_ref() else {
            return false;
        };
        if ramp.resumed.is_none() {
            return false;
        }
        let progress = ramp.progress();
        let (outgoing, incoming) = fade_levels(progress);
        let from = ramp.from;
        self.decks[from].fade = outgoing;
        self.decks[self.active].fade = incoming;
        if progress >= 1. {
            self.finish_ramp();
            return false;
        }
        true
    }

    fn step_fade(&mut self) -> bool {
        let Some(fade) = self.fade_out.as_ref() else {
            return false;
        };
        let progress = fade.progress();
        self.quiet = fade_levels(progress).0;
        progress < 1.
    }

    fn step(&mut self) -> Option<Duration> {
        let ramping = self.step_ramp();
        let fading = self.step_fade();
        self.apply_volumes();
        self.update_snapshot();
        if ramping || fading {
            Some(RAMP_STEP)
        } else {
            self.stepping = false;
            None
        }
    }

    fn fade_out(&mut self, duration: Duration) {
        self.fade_out = Some(Fade {
            started: Instant::now(),
            duration,
        });
        self.ensure_stepping();
    }

    fn reset_fade(&mut self) {
        self.fade_out = None;
        self.quiet = 1.;
        self.apply_volumes();
    }

    fn pause(&mut self) {
        self.playing = false;
        for deck in &self.decks {
            unsafe { deck.player.pause() };
        }
        if let Some(ramp) = self.ramp.as_mut()
            && let Some(resumed) = ramp.resumed.take()
        {
            ramp.elapsed += resumed.elapsed();
        }
    }

    fn play(&mut self) {
        self.playing = true;
        self.start_active();
        if let Some(ramp) = self.ramp.as_mut() {
            unsafe { self.decks[ramp.from].player.play() };
            if ramp.resumed.is_none() {
                ramp.resumed = Some(Instant::now());
            }
            self.ensure_stepping();
        }
    }

    fn seek(&mut self, position_ms: u32) {
        self.finish_ramp();
        let deck = &mut self.decks[self.active];
        let Some(head) = deck.items.first_mut() else {
            return;
        };
        if head.ready {
            seek_player(&deck.player, position_ms);
        } else {
            head.pending_seek = Some(position_ms);
            unsafe { deck.player.pause() };
        }
        self.ensure_ticking();
    }

    fn stop(&mut self) {
        self.ramp = None;
        self.playing = false;
        self.clear_deck(0);
        self.clear_deck(1);
        self.decks[self.active].fade = 1.;
    }

    fn item_finished(&mut self, object: &AnyObject, failed: bool) {
        let Some((deck, index)) = self.find_object(object) else {
            return;
        };
        if failed {
            let item = &mut self.decks[deck].items[index];
            if !item.failed {
                item.failed = true;
                let message = unsafe { item.item.error() }
                    .map(|error| error.localizedDescription().to_string())
                    .unwrap_or_else(|| "The audio stream stopped.".into());
                emit(PlayerEvent::Failed {
                    generation: item.generation,
                    message,
                });
            }
            return;
        }
        let item = self.decks[deck].items.remove(index);
        let generation = item.generation;
        self.release(item);
        if deck != self.active || index != 0 {
            return;
        }
        match self.decks[deck].items.first() {
            Some(next) => {
                emit(PlayerEvent::Advanced {
                    from: generation,
                    to: next.generation,
                });
                self.apply_volumes();
            }
            None => emit(PlayerEvent::Ended { generation }),
        }
    }

    fn follow_queue(&mut self) {
        let active = self.active;
        let deck = &self.decks[active];
        let Some(head) = deck.items.first() else {
            return;
        };
        let advanced = deck
            .items
            .get(1)
            .is_some_and(|next| is_current(&deck.player, next));
        let ended = unsafe { deck.player.currentItem() }.is_none() && head.ready && !head.failed;
        if !advanced && !ended {
            return;
        }
        let item = self.decks[active].items.remove(0);
        let generation = item.generation;
        self.release(item);
        match self.decks[active].items.first() {
            Some(next) if advanced => {
                emit(PlayerEvent::Advanced {
                    from: generation,
                    to: next.generation,
                });
                self.apply_volumes();
            }
            _ => emit(PlayerEvent::Ended { generation }),
        }
    }

    fn poll(&mut self) -> Option<Duration> {
        let mut waiting = false;
        for index in 0..self.decks.len() {
            let active = index == self.active;
            let deck = &mut self.decks[index];
            for item in &mut deck.items {
                let status = unsafe { item.item.status() };
                if status == AVPlayerItemStatus::Failed && !item.failed {
                    item.failed = true;
                    let message = unsafe { item.item.error() }
                        .map(|error| error.localizedDescription().to_string())
                        .unwrap_or_else(|| "Playback failed.".into());
                    emit(PlayerEvent::Failed {
                        generation: item.generation,
                        message,
                    });
                } else if status == AVPlayerItemStatus::ReadyToPlay && !item.ready {
                    item.ready = true;
                    let duration = unsafe { item.item.duration().seconds() };
                    emit(PlayerEvent::Ready {
                        generation: item.generation,
                        duration_ms: milliseconds(duration).unwrap_or(0),
                    });
                } else if status == AVPlayerItemStatus::Unknown {
                    waiting = true;
                }
            }
            let player = deck.player.clone();
            if let Some(head) = deck.items.first_mut()
                && head.ready
                && is_current(&player, head)
                && let Some(seek) = head.pending_seek.take()
            {
                seek_player(&player, seek);
                if active && self.playing {
                    unsafe { player.play() };
                }
            }
        }
        self.follow_queue();
        self.report_position();
        self.update_snapshot();
        if self.decks.iter().all(|deck| deck.items.is_empty()) {
            self.ticking = false;
            return None;
        }
        Some(if waiting { FAST_TICK } else { TICK })
    }

    fn report_position(&self) {
        let deck = &self.decks[self.active];
        let Some(head) = deck.items.first() else {
            return;
        };
        if !head.ready || head.pending_seek.is_some() || !is_current(&deck.player, head) {
            return;
        }
        let Some(position_ms) = milliseconds(unsafe { head.item.currentTime().seconds() }) else {
            return;
        };
        emit(PlayerEvent::Position {
            generation: head.generation,
            position_ms,
            playing: unsafe { deck.player.rate() } > 0.,
        });
    }

    fn update_snapshot(&self) {
        let snapshot = Snapshot {
            decks: [0, 1].map(|index| {
                let deck = &self.decks[index];
                let current = unsafe { deck.player.currentItem() };
                DeckSnapshot {
                    items: deck.items.iter().map(|item| item.generation).collect(),
                    current: deck
                        .items
                        .iter()
                        .find(|item| current.as_ref() == Some(&item.item))
                        .map(|item| item.generation),
                    fade: deck.fade,
                    volume: unsafe { deck.player.volume() },
                    rate: unsafe { deck.player.rate() },
                }
            }),
            active: self.active,
            master: self.master,
            playing: self.playing,
            ramp: self.ramp.as_ref().map(Ramp::progress),
            quiet: self.quiet,
        };
        if let Ok(mut slot) = SNAPSHOT.lock() {
            *slot = Some(snapshot);
        }
    }
}

impl Player {
    pub fn new(
        events: UnboundedSender<PlayerEvent>,
        http: reqwest::Client,
        runtime: Handle,
    ) -> Self {
        let shared = Arc::new(Shared {
            events,
            http,
            runtime,
        });
        if let Ok(mut slot) = SHARED.lock() {
            *slot = Some(shared.clone());
        }
        Self { _shared: shared }
    }

    pub fn snapshot(&self) -> Snapshot {
        SNAPSHOT
            .lock()
            .ok()
            .and_then(|snapshot| snapshot.clone())
            .unwrap_or_default()
    }

    pub fn load(&self, generation: u64, media: Media) {
        on_main(move |mtm, state| state.load(mtm, generation, media));
    }

    pub fn prepare(&self, generation: u64, media: Media, join: Join) {
        on_main(move |mtm, state| state.prepare(mtm, generation, media, join));
    }

    pub fn discard(&self, generation: u64) {
        on_main(move |_, state| state.discard(generation));
    }

    pub fn promote(&self, generation: u64) {
        on_main(move |_, state| state.promote(generation));
    }

    pub fn crossfade(&self, generation: u64, duration_ms: u32) {
        let duration = Duration::from_millis(u64::from(duration_ms));
        on_main(move |_, state| state.crossfade(generation, duration));
    }

    pub fn fade_out(&self, duration_ms: u32) {
        let duration = Duration::from_millis(u64::from(duration_ms));
        on_main(move |_, state| state.fade_out(duration));
    }

    pub fn reset_fade(&self) {
        on_main(|_, state| state.reset_fade());
    }

    pub fn set_speed(&self, generation: u64, speed: f32) {
        on_main(move |_, state| state.set_rate(generation, speed));
    }

    pub fn set_gain(&self, generation: u64, gain: f32) {
        on_main(move |_, state| {
            if let Some((deck, index)) = state.find(generation) {
                state.decks[deck].items[index].gain = gain.max(0.);
                state.apply_volumes();
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
        on_main(|_, state| state.play());
    }

    pub fn pause(&self) {
        on_main(|_, state| state.pause());
    }

    pub fn seek(&self, position_ms: u32) {
        on_main(move |_, state| state.seek(position_ms));
    }

    pub fn set_volume(&self, volume: f32) {
        on_main(move |_, state| {
            state.master = volume.clamp(0., 1.);
            state.apply_volumes();
        });
    }

    pub fn stop(&self) {
        on_main(|_, state| state.stop());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossfades_with_equal_power() {
        let (out, incoming) = fade_levels(0.);
        assert!((out - 1.).abs() < 1e-6 && incoming.abs() < 1e-6);
        let (out, incoming) = fade_levels(1.);
        assert!(out.abs() < 1e-6 && (incoming - 1.).abs() < 1e-6);
        for step in 0..=20 {
            let (out, incoming) = fade_levels(step as f32 / 20.);
            assert!((out * out + incoming * incoming - 1.).abs() < 1e-5);
            assert!((0. ..=1.).contains(&out) && (0. ..=1.).contains(&incoming));
        }
        let (out, incoming) = fade_levels(0.5);
        assert!((out - incoming).abs() < 1e-6);
        assert_eq!(fade_levels(-1.), fade_levels(0.));
        assert_eq!(fade_levels(2.), fade_levels(1.));
    }

    #[test]
    fn clamps_deck_volume() {
        assert_eq!(deck_volume(1., 1., 1.), 1.);
        assert_eq!(deck_volume(0.5, 1.5, 1.), 0.75);
        assert_eq!(deck_volume(1., 1.5, 1.), 1.);
        assert_eq!(deck_volume(0.8, 0.5, 0.), 0.);
        assert_eq!(deck_volume(-1., 1., 1.), 0.);
    }

    #[test]
    fn fades_out_from_full_to_silent() {
        let mut fade = Fade {
            started: Instant::now(),
            duration: Duration::ZERO,
        };
        assert_eq!(fade.progress(), 1.);
        assert!(fade_levels(fade.progress()).0 < 1e-6);
        fade.duration = Duration::from_secs(3600);
        assert!(fade.progress() < 0.01);
        assert!(fade_levels(fade.progress()).0 > 0.99);
    }

    #[test]
    fn ramps_pause_and_resume() {
        let mut ramp = Ramp {
            from: 0,
            duration: Duration::from_secs(4),
            elapsed: Duration::from_secs(1),
            resumed: None,
        };
        assert_eq!(ramp.progress(), 0.25);
        ramp.elapsed = Duration::from_secs(9);
        assert_eq!(ramp.progress(), 1.);
        ramp.duration = Duration::ZERO;
        assert_eq!(ramp.progress(), 1.);
        ramp.duration = Duration::from_secs(3600);
        ramp.elapsed = Duration::ZERO;
        ramp.resumed = Some(Instant::now());
        assert!(ramp.progress() < 0.01);
    }

    #[test]
    fn keeps_rates_in_a_safe_range() {
        assert_eq!(valid_rate(1.5), 1.5);
        assert_eq!(valid_rate(0.), 1.);
        assert_eq!(valid_rate(-2.), 1.);
        assert_eq!(valid_rate(f32::NAN), 1.);
        assert_eq!(valid_rate(9.), 4.);
        assert_eq!(Media::new(Source::Url(String::new())).speed, 1.);
    }

    #[test]
    fn keys_playlists_by_host_and_path() {
        assert_eq!(
            playlist_key("track7", "/playlist.m3u8"),
            "track7/playlist.m3u8"
        );
        assert_eq!(playlist_key("track7", "audio.m3u8"), "track7/audio.m3u8");
        assert_ne!(
            playlist_key("track7", "/master.m3u8"),
            playlist_key("track7", "/audio.m3u8")
        );
        assert_eq!(
            Source::Url("https://example.com/a.mp3".into()).kind(),
            "url"
        );
        assert_eq!(Source::File(PathBuf::from("/tmp/a.m4a")).kind(), "file");
    }

    #[test]
    fn builds_a_byte_range_playlist_per_file() {
        let media = FairPlay {
            file_id: "6f5cc32c".into(),
            cdn_url: "https://audio-fa.scdn.co/audio/6f5cc32c".into(),
            init_size: 1336,
            segments: vec![(163_155, 10.007), (58_121, 3.559)],
            certificate: Arc::new(Vec::new()),
            license_url: String::new(),
            token: String::new(),
        };
        let text = String::from_utf8(playlist(&media)).unwrap();
        assert!(text.contains("#EXT-X-TARGETDURATION:12"));
        assert!(text.contains("URI=\"skd://6f5cc32c\""));
        assert!(text.contains(
            "#EXT-X-MAP:URI=\"https://audio-fa.scdn.co/audio/6f5cc32c\",BYTERANGE=\"1336@0\""
        ));
        assert!(text.contains("#EXT-X-BYTERANGE:163155@1336"));
        assert!(text.contains("#EXT-X-BYTERANGE:58121@164491"));
        assert!(text.ends_with("#EXT-X-ENDLIST"));
    }
}
