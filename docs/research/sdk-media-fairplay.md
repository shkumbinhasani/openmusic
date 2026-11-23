# Web Playback SDK: media fetch and FairPlay path (Safari)

Source: `/tmp/sdk.j2ta/index.js` (747,947 bytes, browserify bundle, internal playback lib version `4.67.0-887cfb0c3` at ~553700). Offsets are byte offsets into that file (`grep -b`). WebKit references are from `WebKit/WebKit` `main` as fetched on 2026-11-23; AVFoundation references are from the local macOS 27.0 SDK headers.

Legend:
- **[seen]** read directly in the bundle, WebKit source or SDK headers.
- **[inferred]** follows from what was seen but was not observed running.
- **[unknown]** the code does not reveal it.

No Spotify endpoint was called while writing this. No seektable, certificate or license response was fetched, so every response shape below comes from the fields the code reads.

---

## 0. Headline finding

**In Safari the SDK does not use MSE for FairPlay audio.** It builds an **HLS media playlist** in the page, with a `SAMPLE-AES` `skd://<fileId>` key line and `EXT-X-BYTERANGE` segments that point at the CDN file. It hands that playlist to a `<source type="application/vnd.apple.mpegurl">`. Safari then plays it through its native HLS stack (AVPlayer/AVURLAsset inside `MediaPlayerPrivateAVFoundationObjC`), and keys come through the **legacy prefixed EME** (`WebKitMediaKeys`). `SourceBufferPrivateAVFObjC` and `MediaSampleAVFObjC` are not involved.

```js
// ~202900 createContent()
return t.keySystemInfo.keySystem===a.KeySystem.FAIRPLAY
  ? new c.AudioManifestHLSContent(R).load(t.measures)
  : new _.AudioManifestMSEContent(R).load(t.measures);
```

So the closest native copy is **AVURLAsset (synthetic HLS playlist) + AVContentKeySession**. The AVSampleBufferAudioRenderer route is possible with public API but is **not** what Safari does for this content (see section 6).

---

## 1. File selection

### 1.1 Key system decides the manifest format [seen]

```js
// ~547700 (player bootstrap)
if(n.keysystem===a.KeySystem.FAIRPLAY)t.appendManifestFormat(i.ManifestFormat.FILE_IDS_CBCS);
else if(n.keysystem!==a.KeySystem.WIDEVINE||r.preferMultiDRMFormat)
  t.appendManifestFormat(i.ManifestFormat.FILE_IDS_MP4_DUAL,i.ManifestFormat.FILE_IDS_MP4,i.ManifestFormat.MANIFEST_URLS_AUDIO_AD);
else{ /* optional FILE_IDS_MP4FLAC if lossless_playback + flac robustness */
  t.appendManifestFormat(i.ManifestFormat.FILE_IDS_MP4,i.ManifestFormat.FILE_IDS_MP4_DUAL,i.ManifestFormat.MANIFEST_URLS_AUDIO_AD)}
```

`FILE_URLS_EXTERNAL` comes first if the device has `audio_podcasts`. On FairPlay, music asks **only** for `file_ids_mp4_cbcs`.

Manifest format names (~518540):
```js
FILE_IDS_MP4="file_ids_mp4", FILE_IDS_MP4_DUAL="file_ids_mp4_dual",
FILE_IDS_CBCS="file_ids_mp4_cbcs", FILE_IDS_MP4FLAC="file_ids_mp4flac", ...
```

Manifest format to MediaFormat (~126580): `FILE_IDS_CBCS → MediaFormat.MP4_CBCS`.

### 1.2 File format enum and MIME strings [seen]

```js
// ~251400 AudioFileFormat
MP3_256=3, MP3_320=4, MP3_160=5, MP3_96=6,
MP4_128=10, MP4_256=11, MP4_128_DUAL=12, MP4_256_DUAL=13,
MP4_128_CBCS=14, MP4_256_CBCS=15, MP4_FLAC=17
// ~250690 AudioFileMimeType: every MP4_* (incl. *_CBCS) -> 'audio/mp4; codecs="mp4a.40.2"', MP4_FLAC -> 'audio/mp4; codecs="flac"'
```

No xHE-AAC or `mp4a.40.42` string appears anywhere. The `MediaCodec` enum (~265440) lists only `mp4a.40.2`, `mp4a.40.5`, flac, opus and video codecs. **CBCS audio is AAC-LC** as far as the SDK knows. [seen]

### 1.3 Capability checks [seen]

- EME configs (~208896) offer `audio/mp4; codecs="mp4a.40.2"` and `"mp4a.40.5"` (plus flac/opus variants) with robustness `SW_SECURE_CRYPTO` or `""`.
- Safari polyfill `_checkConfig` (~243000) strips codecs and calls `WebKitMediaKeys.isTypeSupported("com.apple.fps.1_0", "audio/mp4")`.
- Playability (~277355):
  ```js
  isWebKitMediaKeysPlayable=function(e,t){return"undefined"!=typeof WebKitMediaKeys&&(0===t.indexOf("audio/mp2t")||(0!==t.indexOf("audio/mp4")||-1===t.indexOf("flac"))&&WebKitMediaKeys.isTypeSupported(e,t))}
  ```
  For the URL strategy with FairPlay it uses this, not `MediaSource.isTypeSupported`. `MediaSource.isTypeSupported` appears only on the MSE path (Widevine/PlayReady). FLAC is never offered on FairPlay.

### 1.4 Picking the file id [seen]

Tracks come from the playback state machine (`track-playback`) or from:
```js
// ~553185 resolveTrack
"http://@webgate/track-playback/v1/media/"+uri+"?"+manifestFormats.map(f=>"manifestFileFormat="+f).join("&")
// body.media[uri].item.manifest[<format>][0].file_id ; body.media[uri].item.metadata.uri
```

The state-track resolver (`ManifestTrackResolver._resolveFromStateTrack`, ~126900) walks `track.manifest[format]` in the order of the device's manifest formats. Each entry may carry:

| field | use |
|---|---|
| `file_id` | hex file id → `fileId` |
| `file_url` | direct URL (MP3/external only) |
| `format` | string, `parseInt` → `AudioFileFormat` (e.g. 14/15 for CBCS) → `fileFormat` |
| `bitrate` | default 128000 |
| `audio_quality`, `hifi_status`, `gain_db`, `track_type`, `impression_url(s)` | metadata |

Selection rule: return the **first entry whose `bitrate <= currentBandwidth`**. If none fits, return the last entry seen:
```js
// ~128212
if(h={uri:f,...,fileId:null!==(d=I.file_id)&&void 0!==d?d:"",...,format:C,bitrate:O,fileFormat:null!=A?A:void 0,...},m===o.ManifestFormat.MANIFEST_IDS_VIDEO||L<=t)return h
```
Track metadata itself needs authenticated `@webgate` calls. That is out of scope here.

---

## 2. storage-resolve [seen]

```js
// ~413011 getCDNURL(e=fileId, t=fileFormat)
var r="files/audio/interactive",
    n=t?"v2/".concat(r,"/").concat(t,"/").concat(e):"".concat(r,"/").concat(e),
    i="@webgate/storage-resolve/".concat(n,"?").concat("version=10000000&product=9&platform=39&alt=json");
```

- With a known file format (CBCS always has one): `@webgate/storage-resolve/v2/files/audio/interactive/{format_int}/{file_id_hex}?version=10000000&product=9&platform=39&alt=json`, e.g. `.../v2/files/audio/interactive/14/<hex>?...`.
- Without one: `@webgate/storage-resolve/files/audio/interactive/{file_id_hex}?...`.
- `@webgate` expands with `S=/^(?:https?:\/\/)?@([^/]+)\//` (~602758) to the host from `https://apresolve.spotify.com/?type=dealer-g2&type=spclient` (fallback `spclient.wg.spotify.com`) (~598850). Because the URL matched `@…`, the transport sets `authorize=true` (~613782) and adds `Authorization: Bearer <token>` (~617783).
- Retries on 429, 5xx and connection errors.
- Response handling (~412600): status must be 200, and `body.cdnurl` must be a non-empty array. Result: `{uri: cdnurl[0], uris: cdnurl, protection: "cenc"}`. **Only `cdnurl` is read.** `fileid`, `ttl` and `result` may exist but the SDK ignores them. [unknown]
- On failover the SDK shifts to the next URL in `cdnurl`. It waits at least 1001 ms between resolves (~165500).

---

## 3. Seektable

### 3.1 Fetch [seen]
```js
// ~413513
var t="".concat("https://seektables.scdn.co/seektable","/").concat(e,".json");   // e = fileId (hex)
```
The id is the **same hex file id** used for storage-resolve. The URL is not an `@` URL, so the request goes **without** an Authorization header. It runs in parallel with storage-resolve (`Promise.all([_getCDNURLs, _getManifest])`, ~167800).

### 3.2 Fields read (audio, `AudioManifestContent`, ~168400–167600) [seen]

| field | meaning |
|---|---|
| `pssh_widevine` / `pssh_playready` / **`pssh_fairplay`** / `pssh_invalid` | base64. The key is chosen by `KeySystemSetting[keySystem].pssh_field.audio` (~211401). FairPlay → `pssh_fairplay`. If absent the file is rejected (`FILE_FORMAT_NOT_SUPPORTED`). Legacy `pssh` is used only for Widevine. |
| `protection` | becomes `initDataType`; default `"cenc"`. For CBCS files this is expected to be `"cbcs"` [inferred]. |
| `offset` | byte length of the init segment (ftyp+moov). Init range = `[0, offset-1]`. |
| `timescale` | units for durations. |
| `segments` | **v2**: array of `[byte_size, duration_in_timescale]` pairs. |
| `references` | **v1** fallback: array of `{size, duration}`. |

```js
// ~166386 _calculateSegments
var t=e.offset; this._initSegment={...,byteRanges:{audio:{start:0,end:t-1}}};
for(var r=e.segments,n=e.timescale,...;u<l;u++){var c=r[u];if(c&&c.length){var _=c[0],d=c[1],p=d/n,
  f={index:u,timeStart:a,timeEnd:a+p,byteRanges:{audio:{start:t,end:t+(_-1)}}};o[u]=f,t+=_,s+=d,a+=p,...}}
this._duration=s/n
```

Not read anywhere in the bundle: encoder delay, padding, `pssh_fairplay` contents beyond base64 decode, KID. A grep for `padding`/`encoder_delay` finds nothing. Gapless trimming, if any, is left to the decoder or the container's `elst` [inferred]. The other seektable fields are [unknown].

The video/"director" manifests use a different shape (`contents[0].encryption_infos[{key_system, encryption_data, license_server_endpoint}]`, `base_urls`, `initialization_template`, `segment_template`, `encoding_id`). For FairPlay there, `protection="cbcs"` and the asset id is `encoding_id` (~183640). That path is for video, not track audio.

### 3.3 Turning it into requests

**MSE path (Chrome/Firefox, not Safari)** [seen]: fetch the init byte range, then each segment range from the CDN URL and append to a `SourceBuffer` of the MIME in 1.2.

**Safari/FairPlay path** [seen] (~170900, ~275279): `AudioManifestHLSContent.getURL()` builds the playlist:

```js
// ~171046
i={url:r, byteStart:n.byteRanges.audio.start, byteEnd:n.byteRanges.audio.end},          // init
s=this._contentSegments.map(e=>({url:r,duration:e.timeEnd-e.timeStart,byteStart:...,byteEnd:...})),
l="undefined"==typeof MediaSource?u.createProtectedManifestURL:u.createProtectedManifestDataURL;
return this._hlsURL=l({assetID:this._fileId,targetDuration:10,map:i,segments:s})
```

```js
// ~275000 createProtectedManifest
n=e.assetID?'#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://'+assetID+'",KEYFORMATVERSIONS="1",KEYFORMAT="com.apple.streamingkeydelivery"':"",
i=e.map?'#EXT-X-MAP:URI="'+map.url+'"':"";   if(map.byteEnd&&map.byteStart!==undefined) i+=',BYTERANGE="'+(1+map.byteEnd)+"@"+map.byteStart+'"';
"#EXTM3U\n#EXT-X-VERSION:6\n#EXT-X-TARGETDURATION:"+floor(targetDuration)+"\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n"+ (key+"\n"+map) +"\n"+segments+"\n#EXT-X-ENDLIST"
// each segment: "#EXTINF:<sec>,\n#EXT-X-BYTERANGE:<len>@<start>\n<cdnurl>"
```

Resulting playlist (structure seen, values illustrative):
```
#EXTM3U
#EXT-X-VERSION:6
#EXT-X-TARGETDURATION:10
#EXT-X-MEDIA-SEQUENCE:0
#EXT-X-PLAYLIST-TYPE:VOD
#EXT-X-KEY:METHOD=SAMPLE-AES,URI="skd://<fileId hex>",KEYFORMATVERSIONS="1",KEYFORMAT="com.apple.streamingkeydelivery"
#EXT-X-MAP:URI="<cdnurl>",BYTERANGE="<offset>@0"
#EXTINF:<dur>,
#EXT-X-BYTERANGE:<size0>@<offset>
<cdnurl>
...
#EXT-X-ENDLIST
```
Notes:
- `skd://` carries the **file id**, not the KID or the seektable PSSH.
- `targetDuration` is hard-coded to 10, whatever the real segment lengths are. Safari accepts this.
- The init `BYTERANGE` computes length as `1+byteEnd`. That is correct only because `byteStart` is 0.
- Delivery: a `data:application/vnd.apple.mpegurl;base64,…` URL when `MediaSource` exists (desktop Safari), otherwise a blob URL. The SDK attaches it as `<source type="application/vnd.apple.mpegurl">` (~380900).

---

## 4. FairPlay EME flow

### 4.1 Key system and EME flavour [seen]
- Key system string: `com.apple.fps.1_0` (~264715).
- `KeySystemSetting[FAIRPLAY]` (~212550): `{commonName:"fairplay", licenseServer:"https://@webgate/fairplay-license", withCertificate:true, pssh_field:{audio:"pssh_fairplay", video:"asset_id"}}`.
- Preference order: Widevine, PlayReady variants, then FairPlay. Safari only answers for FairPlay.
- Polyfill choice (~229239): the native EME path is used only if `WebKitMediaKeys` is **undefined**. Safari still exposes `WebKitMediaKeys`, so the SDK installs its **Safari polyfill** (`"Detected Safari EME. Polyfilling."`, ~230284) on top of legacy `WebKitMediaKeys`. [seen for the branch; that current Safari still defines `WebKitMediaKeys` is inferred from WebKit's `LEGACY_ENCRYPTED_MEDIA` code still being present]

### 4.2 Server certificate [seen]
```js
// ~217779 _cacheServerCertificate (at init, precache, unauthenticated)
this._requestServerCertificate(this._unauthServerCertificateBase+"/"+"fairplay"+"-license", false)
//   _unauthServerCertificateBase defaults to "https://spclient.wg.spotify.com"
// ~217200 _trySetServerCertificate (when MediaKeys attach) uses KeySystemSetting.licenseServer = "https://@webgate/fairplay-license" (authorized)
// ~217992 _requestServerCertificate
this._transport.request(e+"/v1/application-certificate",{authorize:t,responseType:"arraybuffer",parseResponseHeaders:!0,retry:{condition:5xx}})
```
- URL: `GET https://spclient.wg.spotify.com/fairplay-license/v1/application-certificate` (precache, no auth), or the same path on the apresolved `@webgate` host with a Bearer token.
- Body is used as raw bytes: an `ArrayBuffer` passed straight to `setServerCertificate`. It is the DER FPS application certificate [inferred].
- Caching: `Cache-Control: max-age=N` → expiry. Note a units mix-up: default `n=3600` but a parsed header sets `n=1000*N` (ms), then `expiry=Date.now()+(n-600)`. Without the header the cache lasts about 3 s. Only one certificate request runs at a time (`_pendingCertificateRequest`).
- `_testMediaKeys` calls `setServerCertificate(new ArrayBuffer(0))` once as a probe (~222517).

### 4.3 Init data (what Safari hands the page) [seen, WebKit]
When AVFoundation reaches the `skd://` key, `MediaPlayerPrivateAVFoundationObjC::shouldWaitForLoadingOfResource` builds legacy init data:
```
[u32 LE: byteLength of URI in UTF-16] [URI "skd://<fileId>" as UTF-16LE]
```
and fires `webkitneedkey`. The SDK's polyfill re-dispatches it as `encrypted` with `fromPolyfill=true` (~228972). The player listens for that event on URL content and calls `_requestLicense(…, {initData: e.data.initData})` (~380643). **The seektable `pssh_fairplay` is not the init data used on Safari.** It only gates `isProtected()` and is passed as a fallback in `getInitParams()`.

### 4.4 Session creation (`_rebuildInitData`, ~239403) [seen]
```js
// checks u32LE(0)+4 === length, decodes UTF-16LE, matches /^skd:\/\/([0-9a-fA-F]+)/
// builds: [original initData][u32LE len(contentId)][contentId as UTF-16LE][u32LE len(cert)][cert bytes]
r._nativeKeySession=r._nativeMediaKeys.createSession("video/mp4",new Uint8Array(i),null)
```
WebKit (`CDMSessionAVFoundationObjC::generateKeyRequest`) parses that blob, takes the `AVAssetResourceLoadingRequest` for the key URI, and calls:
```objc
[m_request streamingContentKeyRequestDataForApp:certificateData contentIdentifier:[keyID dataUsingEncoding:NSUTF8StringEncoding] options:nil error:&err]
```
So **contentIdentifier = UTF-8 bytes of the hex file id**, and **appIdentifier = Spotify's FPS certificate**. The returned SPC arrives as `webkitkeymessage`.

### 4.5 License request [seen]
License URL resolution (`LicenseURLResolver.get`, ~290600):
```js
"@webgate/melody/v1/license_url?keysystem=com.apple.fps.1_0&mediatype=audio&sdk_name=<name>&sdk_version=<ver>"
// 200 → body {uri, expires(seconds)} ; url = "@webgate/"+uri (+ "?sl=<securityLevel>" if configured); cached until expires-60s
// 5xx → fallback "@webgate/fairplay-license/v1/"+t+"/license"   with t = mediaType ("audio" | "video")
// then .replace(/\{contentId\}/, fileId)
```
POST (`_onMessage`, ~219969):
```js
e.keySystem===c.KeySystem.FAIRPLAY&&(i="".concat(i).concat(-1!==i.indexOf("?")?"&":"?","assetId=hex")),
this._transport.request(i,{method:"POST",payload:r,responseType:"arraybuffer",retry:{condition:not 400/402/403 ...}})
```
- **Body: raw SPC bytes** (the `ArrayBuffer` from the key message). No JSON, no base64. No explicit Content-Type. The transport default is [unknown].
- Query: append the literal `assetId=hex`, which tells the server the asset id inside the SPC is the hex string [inferred].
- Headers: `Authorization: Bearer <access token>` (auto, `@webgate`). Nothing else is added in this code. Transport plugins may add `client-token` or similar [unknown].
- Response: **raw CKC bytes** on 200, passed as-is to `keySession.update(body)` → WebKit `respondWithData:` + `finishLoading` on the `AVAssetResourceLoadingRequest`.
- Errors: 402 means "User is capped". Otherwise the body is parsed as JSON `{errorMsg}`. 400/403 mark the endpoint for refresh.

---

## 5. Encryption scheme of the MP4 files

| item | evidence |
|---|---|
| Format family `MP4_128_CBCS` / `MP4_256_CBCS` (14/15) for FairPlay | [seen] |
| HLS `METHOD=SAMPLE-AES` + `EXT-X-MAP` → fMP4 with **cbcs** (`enca`, `sinf/frma=mp4a`, `schm=cbcs`, `tenc`) | [inferred]: Apple's spec for fMP4 SAMPLE-AES requires cbcs |
| Director (video) FairPlay sets `protection="cbcs"` explicitly | [seen] ~183640 |
| Seektable `protection` field feeds initDataType | [seen]; value for CBCS audio [unknown] |
| KID location (`tenc` default_KID, `pssh`) | [unknown]: the SDK never parses MP4 boxes. FairPlay keys by `skd://fileId`, not by KID. |
| Constant IV vs per-sample IV, pattern | [unknown]. For AAC cbcs the usual layout is whole-sample protection (pattern 0:0 or 1:0) with a constant IV in `tenc` [inferred, verify with one init segment]. |
| Codec | AAC-LC `mp4a.40.2` (MIME table). No xHE-AAC seen. |

The first thing to check, from any one CBCS file, is the `offset` bytes of the init segment: dump `moov/trak/mdia/minf/stbl/stsd/enca/sinf` (`frma`, `schm`, `schi/tenc`) and any `pssh`. The CDN URL is signed, so getting one still needs storage-resolve.

---

## 6. Native macOS pipeline

### 6.1 Route A: copy Safari (recommended)

| step | what | API |
|---|---|---|
| 1 | Pick `file_ids_mp4_cbcs` entry (sec. 1.4) → `file_id`, `format` | Spotify metadata (auth) |
| 2 | storage-resolve v2 → `cdnurl[]` | HTTPS, Bearer |
| 3 | seektable → `offset`, `timescale`, `segments` | HTTPS, no auth |
| 4 | Build the HLS playlist exactly as in 3.3 | string |
| 5 | Serve the playlist to `AVURLAsset` through a custom scheme (`openmusic-hls://<fileId>.m3u8`) with `AVAssetResourceLoaderDelegate`, because AVFoundation does not load HLS from `data:` URLs reliably [inferred]. Segment and map URIs stay absolute `https://` CDN URLs with byte ranges, so AVFoundation fetches them itself. | `AVURLAsset.resourceLoader` |
| 6 | `AVContentKeySession(keySystem: .fairPlayStreaming)`, `addContentKeyRecipient(asset)` **before** playback | macOS 10.12.4+ |
| 7 | Delegate `didProvide keyRequest`: `keyRequest.identifier` = `"skd://<fileId>"`. Strip the scheme to get `contentId = fileId.data(using: .utf8)` (matches 4.4). | |
| 8 | Fetch the certificate (4.2) once and cache it. `keyRequest.makeStreamingContentKeyRequestData(forApp: cert, contentIdentifier: contentId, options: nil)` → SPC | macOS 10.12.4+ (completion form) |
| 9 | Resolve the license URL (`melody/v1/license_url`, mediatype=audio). `POST <url>?assetId=hex`, body = SPC, Bearer token → CKC | HTTPS |
| 10 | `keyRequest.processContentKeyResponse(AVContentKeyResponse(fairPlayStreamingKeyResponseData: ckc))`. On failure, `processContentKeyResponseError`. | |
| 11 | `AVPlayer(playerItem: AVPlayerItem(asset:))`. Gapless needs `AVQueuePlayer`. Seeking and buffering come for free. | |

In Rust, all of this is reachable through `objc2` bindings (`objc2-av-foundation`, `block2` for completion handlers). The resource loader and content key delegates must be Objective-C classes declared with `define_class!`, using a serial dispatch queue.

### 6.2 Route B: AVSampleBufferAudioRenderer + AVSampleBufferRenderSynchronizer

This is what WebKit does for **MSE** FairPlay, but WebKit relies on private `AVStreamDataParser`. That class is absent from the public macOS 27.0 SDK headers (checked), and it does the fMP4 parsing and attaches the decryption info. A public-API version would need:

1. Our own fMP4 demuxer (moof/trun/senc/saiz/saio) to cut AAC access units. The payload stays encrypted.
2. A `CMAudioFormatDescription` for the protected track: original format `mp4a` via `kCMFormatDescriptionExtension_ProtectedContentOriginalFormat` (macOS 11.0), plus the `sinf` atom, probably in `SampleDescriptionExtensionAtoms` [inferred, not documented for audio].
3. Per-sample subsample map via `kCMSampleAttachmentKey_CryptorSubsampleAuxiliaryData` (macOS 12.0). The per-sample IV, when needed, has **no documented public attachment key** [unknown]. That is why the constant-IV question in section 5 matters.
4. `AVContentKeySession.processContentKeyRequest(withIdentifier: "skd://<fileId>", initializationData: nil, options: nil)` drives steps 7–10 above with no recipient. Delegate `contentKeySession(_:didProvide contentKey:)` (macOS 11.3) returns an `AVContentKey`.
5. `AVSampleBufferAttachContentKey(sbuf, contentKey, &err)` (macOS 11.3; Swift-deprecated in 27.0 in favour of `CMReadySampleBuffer.attach(contentKey:)`) on every buffer, then `enqueue` on `AVSampleBufferAudioRenderer` (macOS 10.13) added to `AVSampleBufferRenderSynchronizer`. The header says it accepts client-created buffers "for enqueueing with AVSampleBufferDisplayLayer or AVSampleBufferAudioRenderer".

The renderer is **not** an `AVContentKeyRecipient` in the public headers. Only `AVURLAsset` is. Keys reach it only by manual attachment. Route B costs a demuxer and depends on undocumented format-description details, so use it only if Route A cannot meet a requirement (custom DSP between decode and output is impossible either way, because decrypted PCM never reaches the app).

### 6.3 Versions, entitlements, certificate use

- Minimum macOS: Route A 10.12.4 (practically 11+). Route B 12.0 (subsample attachment) and 11.3 (attach key).
- Entitlements: FairPlay Streaming through AVContentKeySession needs **no special entitlement** in a macOS app [inferred from Apple docs and the SDK headers. No entitlement is named for FPS; sandboxed apps need `com.apple.security.network.client`]. The Developer ID, notarization and hardened runtime defaults are fine [inferred].
- Certificate ownership: the FPS application certificate belongs to the content provider's key server (Spotify), not to an app bundle. Safari is itself a "foreign" client using Spotify's certificate, so FairPlay technically allows any client app to build an SPC with it [inferred]. Whether Spotify's license server accepts SPCs from a non-Safari process is **untested**. The SPC carries no bundle id that we know of, but the server may check other things.
- DRM stays intact on both routes: decryption happens inside Apple's FPS module and the CDM. The app only moves opaque SPC and CKC bytes and never sees keys or clear samples.

---

## 7. Step-by-step pipeline (summary)

1. Device announces manifest format `file_ids_mp4_cbcs` (FairPlay).
2. Track manifest → first entry with `bitrate <= bandwidth` → `file_id` (hex), `format` (14 or 15).
3. In parallel:
   - `GET @webgate/storage-resolve/v2/files/audio/interactive/{format}/{file_id}?version=10000000&product=9&platform=39&alt=json` (Bearer) → `cdnurl[]`
   - `GET https://seektables.scdn.co/seektable/{file_id}.json` (no auth) → require `pssh_fairplay`; read `offset`, `timescale`, `segments` (or `references`), `protection`.
4. Init = bytes `[0, offset-1]`. Segment *i* = cumulative byte ranges from `offset`, duration = `segments[i][1]/timescale`.
5. HLS VOD playlist: `EXT-X-KEY SAMPLE-AES skd://{file_id}`, `EXT-X-MAP` init byte range, `EXT-X-BYTERANGE` per segment, all on `cdnurl[0]`.
6. Player reaches the key → key request with identifier `skd://{file_id}`.
7. Certificate: `GET …/fairplay-license/v1/application-certificate` (raw bytes, cache by `max-age`).
8. SPC = streamingContentKeyRequestData(app: cert, contentIdentifier: utf8(file_id)).
9. License URL: `GET @webgate/melody/v1/license_url?keysystem=com.apple.fps.1_0&mediatype=audio&sdk_name=…&sdk_version=…` → `{uri, expires}`. Fallback `@webgate/fairplay-license/v1/audio/license`.
10. `POST <license url>?assetId=hex`, body = raw SPC, Bearer → raw CKC.
11. Hand the CKC to FPS (`processContentKeyResponse`). Playback continues. On CDN errors, rotate through `cdnurl[]` and re-resolve no more than once per second.

---

## 8. Open risks

1. **License server acceptance of a native client.** Untested. The server may check SPC fields, `sdk_name`/`sdk_version` from `melody/v1/license_url` (a 400 `deprecated-version` is handled), `client-token` headers added by transport plugins we did not trace, or token scopes and client ids tied to the Web Playback SDK. This is the main go/no-go question.
2. **Terms of use.** Using the Web Playback SDK's license endpoints from a native app is likely outside Spotify's developer terms, even with DRM intact. Needs a decision outside engineering.
3. **contentIdentifier format.** It comes from WebKit's legacy path (UTF-8 hex `file_id`). If the server expects another form, the `assetId=hex` flag suggests it parses hex from the SPC. Low risk but unverified.
4. **Data-URL / custom-scheme HLS in AVFoundation.** Serving the playlist through `AVAssetResourceLoaderDelegate` is a known pattern. Byte-range segments on signed CDN URLs should work but are unverified, and so is CDN URL expiry during long pauses.
5. **cbcs details** (constant vs per-sample IV, subsample layout). Unknown until one init segment is inspected. They matter only for Route B.
6. **Seektable/storage-resolve fields beyond those the code reads** (ttl, encoder delay/padding) are unknown. Gapless behaviour on Route A depends on what AVPlayer does with `elst`/`iTunSMPB` in the file.
7. **Certificate cache bug** in the SDK (units mix-up) does not affect a native port, but do not copy it.
