# Spotify Web Playback SDK: device protocol

Static analysis of the Web Playback SDK's embedded player script.

- Source: `https://sdk.scdn.co/embedded/index.js` (local copy `/tmp/sdk.j2ta/index.js`, 747,947 bytes). The loader (`player.js`, "Spotify JS-SDK - v1.10.0-5be383f") creates an iframe at `https://sdk.scdn.co/embedded/index.html` and talks to it with `postMessage`.
- The bundled library inside is `@spotify-internal/harmony` **4.67.0-887cfb0c3** (module 168), with `@spotify-internal/transport` and a playback engine.
- Byte offsets are approximate positions in `index.js`. "Module N" is the browserify module id.
- **[seen]** means the code shows it directly. **[inferred]** means it follows from the code but is not spelled out (for example, server response fields that the code reads but never builds).

The SDK does **not** register through `connect-state`. It registers with the older **track-playback** service, which sends the device a full "state machine" (tracks plus transitions) and lets the device drive playback locally. The bundle also contains a `connect-state` client (module 36), but the embedded player never creates it. See section 8.

---

## 0. Big picture

```
                 apresolve.spotify.com  (no auth)
                         |  dealer host
                         v
 SDK iframe  ==wss==>  dealer  (wss://<dealer>/?access_token=T)
     |                    |  first message: hm://pusher/v1/connections/<id>  -> connection id
     |                    |  later: hm://track-playback/v1/... commands (replace_state, set_volume, ping, log_out)
     |
     +--HTTPS (Bearer T)--> https://api.spotify.com/v1/track-playback/v1/devices ...
     |                         POST   /devices                 register (needs connection_id)
     |                         PUT    /devices/{id}/state      state report  -> returns new state_machine
     |                         PUT    /devices/{id}/volume     volume report
     |                         POST   /devices/{id}/state_conflict
     |                         PUT    /devices/{id}            device info update
     |                         DELETE /devices/{id}            deregister
     +--HTTPS (Bearer T)--> https://api.spotify.com/v1/storage-resolve/...  (CDN URLs)
     +--HTTPS (no auth)---> https://seektables.scdn.co/seektable/<file_id>.json
     +--HTTPS (Bearer T)--> https://api.spotify.com/v1/melody/v1/license_url ... then the Widevine license POST
     +--HTTPS (Bearer T)--> https://api.spotify.com/v1/melody/v1/{check_scope,product_state,time,...}
```

---

## 1. Host resolution and auth headers

### 1.1 `@webgate` and the endpoints provider [seen]

The transport rewrites any URL of the form `@name/...` (optionally prefixed by `http://` or `https://`) with the endpoint called `name`.

Module 207, about 602,600:

```js
S=/^(?:https?:\/\/)?@([^/]+)\//, P=/^(ws|http)s?:\/\/.*[^/]$/
...
t.prototype._tryExpandSpecialURL=function(e){var t=this._endpoints;return e.replace(S,(function(e,r){ ... return t[r]}))}
```

`P` adds a trailing `/` to each resolved endpoint (`_connectToEndpoints`, about 608,300). So `"http://@webgate/track-playback/v1/devices"` becomes `<webgate>/track-playback/v1/devices`. The `http://` in the source does not matter.

The endpoints come from apresolve (module 204, about 598,650):

```js
r="".concat("https://apresolve.spotify.com/","?type=").concat(t.dealer,"&type=").concat(t.webgate)   // dealer="dealer-g2", webgate="spclient"
... _={dealer: c[t.dealer][0] ?? "dealer.g2.spotify.com",
       webgate: c[t.webgate][0] ?? s[t.webgate] ?? "spclient.wg.spotify.com",
       webapi:"https://api.spotify.com/"}
... e.dealer="wss://".concat(e.dealer.replace(/:443$/,"")), e.webgate="https://".concat(e.webgate.replace(/:443$/,""))
```

- Request: `GET https://apresolve.spotify.com/?type=dealer-g2&type=spclient`. No auth. `metadata:{noRequestTransform:true}`. Up to 3 retries with exponential backoff on any non-2xx.
- Expected response [inferred from the reader code]: `{"dealer-g2":["host:443", ...], "spclient":["host:443", ...]}`. The SDK takes the first entry of each.

**The embedded player then overrides `webgate`.** Module 234 `_onInit`, about 735,700:

```js
this._transport=(0,s.createTransportForHarmony)({getToken:this._getToken,endpoints:{webgate:"https://api.spotify.com/v1"}})
```

`createTransportForHarmony` (module 163, about 550,600) copies each string in `options.endpoints` over the apresolve result. So, in the SDK:

| alias      | value                                                        |
|------------|--------------------------------------------------------------|
| `@webgate` | `https://api.spotify.com/v1/` (fixed)                        |
| `dealer`   | `wss://<apresolve dealer-g2[0]>/` (default `wss://dealer.g2.spotify.com/`) |
| `webapi`   | `https://api.spotify.com/`                                   |

So the device endpoints are on the public Web API host: `https://api.spotify.com/v1/track-playback/v1/devices`, and the same pattern applies to `melody` and `storage-resolve`. apresolve only supplies the dealer host. In the generic harmony build, without the override, `@webgate` would be the `spclient` host.

### 1.2 Token flow [seen]

- The page's `getOAuthToken(cb)` runs in the loader. The iframe asks with `GET_TOKEN`, and the loader replies with `TOKEN {token, ref}` (module 234 `_getToken` and `_onToken`).
- Each new token is checked once: `GET @webgate/melody/v1/check_scope?scope=web-playback` with `Authorization: Bearer <token>` (about 738,650). A 403 raises "Invalid token scopes." So the token needs the `streaming` scope (the documented SDK scope), which maps to `web-playback` on the server [inferred].
- Token lifetime: the provider returns a bare string, so `_parseProvidedToken` assumes **3540 s** (about 611,500). If the provider returns the same token twice in a row, the transport raises `TRANSPORT_STALE_TOKEN`.
- `ProductStateObserver` (module 203) calls `GET @webgate/melody/v1/product_state`. If the response has `ads === "1"` (a free user), the SDK reports "premium only" and disconnects (about 737,400).

### 1.3 Headers on HTTP requests [seen]

`_setAuthHeader` (module 207, about 618,900):

```js
e.headers.set("Authorization","Bearer ".concat(t))
```

- Any request whose URL starts with `@...` gets `authorize=true` by default, so it carries `Authorization: Bearer <token>`.
- Requests with an absolute URL (apresolve, seektables) carry no auth.
- Track-playback requests add `Content-Type: application/json`.
- The bundle contains **no** `client-token`, `App-Platform`, `Spotify-App-Version` or `User-Agent` header (searched for, none found). The only Spotify-specific header in the bundle is `X-Spotify-Connection-Id`, and only the unused connect-state client sends it (section 8).
- The browser sends requests with `fetch` using `mode:"cors"` and `credentials:"same-origin"` (module 190). No cookies go to `api.spotify.com`.
- A 401 response triggers a token refresh and a retry (`ignoreUnauthorized` is false by default).

---

## 2. Device identity and registration

### 2.1 Descriptor the SDK passes in [seen]

Module 234 `_onInit`, about 736,900:

```js
d.resolve({id:null,name:e.name,brand:"public_js-sdk",randomizeId:!0,
  platform_identifier:(0,l.interpolateUA)("Partner public_js-sdk harmony-{{name}}.{{version}}-{{platform}};tp"),
  capabilities:{lossless_playback:!1}})
```

`e.name` is the `name` option from `new Spotify.Player({name})`, which defaults to `location.hostname`. `e.volume` defaults to 1.

### 2.2 Device id [seen]

Module 164 (about 551,531) and module 165:

```js
DeviceId.get(e,t){ if(e&&"string"==typeof e)return e; if(t)return i();   // i = generateId(40)
  var r=localStorage.getItem("_spharmony_device_id"); ... }
generateId(e){ var t=Math.ceil(e/2); return bytesToHex(generateRandomBytes(t)) }   // crypto.getRandomValues
```

- Because `randomizeId:true`, the SDK makes a **new 40-character lowercase hex id (20 random bytes) on every page load**. It does not persist the id.
- An id you supply must match `/^[a-zA-Z0-9_%:-]{1,40}$/` (module 35, about 30,600).

### 2.3 Final descriptor (DeviceDescriptor.create, module 39, about 85,900) [seen]

```js
model: e.model || interpolateUA("harmony-{{name}}.{{version}}-{{platform}}").toLowerCase(),
name:  e.nameTemplate ? ... : e.name || capitalize(interpolateUA("Spotify ({{name}}/{{platform}})")),
type:  e.type || "computer",
brand: e.brand || "SpotifyHarmonyGeneric",
```

`{{name}}`, `{{version}}` and `{{platform}}` come from parsing the user agent: browser name, browser major version, and one of `mac`, `windows`, `linux`, `ios`, `android`, `cros` or `other`.

Capabilities (`HarmonyClient._createDeviceDescriptor`, module 35, about 30,700):

```js
{ change_volume: true, endsong_snooping: undefined, enable_play_token: true, supports_file_media_type: true,
  play_token_lost_behavior: "pause", disable_connect: false /* !!hidden */, audio_podcasts: true,
  video_playback: undefined, lossless_playback: false,
  volume_steps: undefined /* only if 0 <= n < 65535 */,
  manifest_formats: ["file_ids_mp3","file_urls_mp3","manifest_urls_audio_ad"],
  video_ads: undefined, supports_preferred_media_type: true,
  supports_external_url_playback_speed: undefined, supports_playback_offsets: true, supports_playback_speed: true }
```

After the media engine starts, the streamer appends more `manifest_formats` (module 159, about 547,000):

- `audio_podcasts` on: adds `file_urls_external`.
- FairPlay (Safari): adds `file_ids_mp4_cbcs`.
- Widevine (Chrome and Firefox): adds `file_ids_mp4`, `file_ids_mp4_dual`, `manifest_urls_audio_ad`. If `lossless_playback` is on and a hardware-secure FLAC path exists, it also adds `file_ids_mp4flac`. The SDK sets `lossless_playback` to false.
- Other systems (for example PlayReady): `file_ids_mp4_dual`, `file_ids_mp4`, `manifest_urls_audio_ad`.

Wire form of the device (`toTrackPlaybackDeviceInfo`, about 88,700) [seen]:

```json
{
  "brand": "public_js-sdk",
  "capabilities": { "change_volume": true, "enable_play_token": true, "supports_file_media_type": true,
    "play_token_lost_behavior": "pause", "disable_connect": false, "audio_podcasts": true,
    "lossless_playback": false, "manifest_formats": ["file_ids_mp3","file_urls_mp3","manifest_urls_audio_ad",
    "file_urls_external","file_ids_mp4","file_ids_mp4_dual","manifest_urls_audio_ad"],
    "supports_preferred_media_type": true, "supports_playback_offsets": true, "supports_playback_speed": true },
  "device_id": "<40 hex>",
  "device_type": "computer",
  "metadata": {},
  "model": "harmony-chrome.131-mac",
  "name": "My Web Player",
  "platform_name": undefined,
  "platform_identifier": "Partner public_js-sdk harmony-chrome.131-mac;tp",
  "is_group": false,
  "correlation_id": undefined
}
```

`undefined` fields disappear when serialised to JSON. The values in `model` and `name` are examples.

Device type values (module 137): `unknown`, `computer`, `tablet`, `smartphone`, `speaker`, `tv`, `avr`, `stb`, `audio_dongle`, `game_console`, `cast_video`, `cast_audio`, `automobile`, `smartwatch`, `chromebook`.

### 2.4 Register: `POST {webgate}/track-playback/v1/devices` [seen]

Registration runs when the dealer hands out a connection id (`_onConnectionId`, module 130, about 473,500). It runs again on every new connection id, for example after a reconnect.

```js
r.prototype.register=function(){ ...
  if(!this._connectionId)return Promise.reject(new Error("Need connection-id for device registration"));
  ... return this._sendDevicesRequest("POST",{device:this._deviceInfo,
        outro_endcontent_snooping:"true"===t.env.OUTRO_SNOOPING, connection_id:this._connectionId,
        client_version:this._clientVersion, previous_session_state:e, volume:this._currentGaiaVolume})
```

`_sendDevicesRequest` (about 488,900) sends the request with `Content-Type: application/json` and Bearer auth. On server errors and connection errors it retries up to 8 times.

Request:

```http
POST https://api.spotify.com/v1/track-playback/v1/devices
Authorization: Bearer <token>
Content-Type: application/json

{
  "device": { ...device info from 2.3... },
  "outro_endcontent_snooping": false,
  "connection_id": "<dealer connection id>",
  "client_version": "harmony:4.67.0-887cfb0c3",
  "previous_session_state": null,
  "volume": 65535
}
```

Field notes:

- `client_version` is `HarmonyClient.getSDKId()`, which is `"harmony:" + "4.67.0-887cfb0c3"`.
- `volume` is the initial volume times 65535 (`GAIA_VOLUME = 65535`, module 160).
- `previous_session_state` is a state payload (section 5) only when a context already exists, for example when re-registering after a reconnect. Otherwise it is absent.

Response handling (`_handleRegisterResponse`, about 482,900):

| status | meaning |
|--------|---------|
| 200 with body | registered. The SDK reads `initial_seq_num` (the starting sequence number) and optionally `endsong` or `endsongs` (logged). It then emits `registered`, and the SDK fires `ready {device_id}`. |
| 403 with `{"error_type":"PREMIUM_REQUIRED"}` | not premium |
| 429 | "Max connections reached" (`TP_MAX_SUBSCRIPTIONS_REACHED`) |
| other | `TP_REGISTRATION_FAILED_WITH_STATUS` |

Response example [inferred]: `{"initial_seq_num": 0}`.

### 2.5 Update device info: `PUT {webgate}/track-playback/v1/devices/{device_id}` [seen]

Sent when the descriptor changes, for example on `setName`. Only sent while registered. The body is the bare device info object (2.3), not wrapped in `{device: ...}`. A 204 response means it worked (`_updateDeviceInfo`, about 489,900).

### 2.6 Deregister: `DELETE {webgate}/track-playback/v1/devices/{device_id}` [seen]

Triggers: transport disconnect, `player.disconnect()`, `beforeunload`, and logout. The body is a state payload with `debug_source:"deregister"` and `seq_num` set (`_deregisterFromService`, about 478,000). On server or connection errors it retries twice. 200 or 204 means done. The response may carry `endsong` or `endsongs`.

---

## 3. Realtime channel (dealer websocket)

### 3.1 URL and auth [seen]

Module 197 `authenticate`, about 588,700:

```js
var r="".concat(this._endpoint,"?access_token=").concat(e), n=this._socket=new this._WebSocket(r);
```

The URL is `wss://<dealer host>/?access_token=<token>`. The host has had `:443` removed and a trailing `/` added. The token is **not** URL-encoded here. There are no other query parameters and no subprotocol.

### 3.2 Connection id [seen]

```js
_=/hm:\/\/pusher\/(?:[^]+)?\/connections\/([^]+)/
_prepareConnectionId(e){ ... n=(e.uri.match(_)||[])[1];
  r=e.headers&&e.headers["Spotify-Connection-Id"]?e.headers["Spotify-Connection-Id"]:decodeURIComponent(n);
  this._connectionId=r; ... emit(CONNECTION_ID,{id:r,uri:e.uri}) }
```

The first `type:"message"` frame received after the socket opens is checked against that pattern. If it matches:

1. The connection id is taken from `headers["Spotify-Connection-Id"]`. If that header is absent, the URI-decoded last path segment is used instead.
2. The heartbeat starts with an immediate ping.
3. The frame is **not** passed on to subscribers.

Example frame [inferred from the regex]:

```json
{"type":"message","method":"PUT","uri":"hm://pusher/v1/connections/<urlencoded id>",
 "headers":{"Spotify-Connection-Id":"<id>"}}
```

The connection id is then used **only** as the `connection_id` field of the register body (2.4). Track-playback requests do not send it as a header.

### 3.3 Frame types [seen]

`_parseMessage`, about 586,300:

| `type` (incoming) | handling |
|-------------------|----------|
| `message` | First one: connection id (3.2). After that: emitted to `matchMessages` subscribers, which test a regex against `message.uri`. |
| `pong` | Resolves the pending ping. |
| `request` | If it has a `key`, it is emitted as a push request with a reply function. The reply is sent as `{"type":"reply","key":<key>,"payload":<result>}`. The track-playback client does not register any request handlers, so it never sends a reply. |

Outgoing frames: `{"type":"ping"}` and `{"type":"reply", ...}`.

Frames arrive as text or as binary. Binary frames are decoded as ASCII or Latin-1, then parsed with `JSON.parse`. The dealer layer does **no** gzip or base64 decoding. Subscribers receive the parsed `message` object as is, with `uri`, `headers` and `payloads`. The track-playback client reads `payloads[0]` straight as a JSON object (`_onTrackPlaybackMessage`, about 476,100):

```js
r.prototype._onTrackPlaybackMessage=function(e){var t=e.payloads;Array.isArray(t)&&t.length&&this._performCommand(t[0])}
```

So the server is expected to send track-playback commands as plain JSON objects in `payloads` [inferred]. A native client should still check for a `Transfer-Encoding: gzip` header and for base64 string payloads, because other Spotify clients receive those on other topics. This SDK code would fail on them.

### 3.4 Heartbeat [seen]

```js
_startHeartbeat(e){ ... r=function(){ t.ping().then(onSuccess, onError);
   t._heartbeatTimeoutToken=setTimeout(onError, t._heartbeatTimeout) };   // heartbeatTimeout default 1e4
   e ? r() : this._heartbeatToken=setTimeout(r, 3e4) }
```

- The client sends `{"type":"ping"}` immediately after the connection id arrives, then 30 s after each pong.
- If no pong arrives within **10 s**, the client closes the socket with an internal timeout code and a reconnect follows.

### 3.5 Reconnects [seen]

Module 207:

- Disconnect after a session longer than 5 s: reconnect at once, with backoff of base 1 s, max 15 s, and 3 retries.
- Disconnect within 5 s of the last one: wait 5 to 15 s (exponential, by the number of quick disconnects) before reconnecting.
- Each reconnect makes a new socket, gets a new connection id, and sends a **new `POST /devices`** (2.4) with `previous_session_state`.
- A dealer message with URI `client:logout` makes the transport log out.

### 3.6 Topics the SDK subscribes to [seen]

| URI regex | handler |
|-----------|---------|
| `^hm://track-playback/v1` | `_performCommand(payloads[0])` (section 4) |
| `^hm://play-token/lost` | pause local playback ("play token lost": another device on the same account started playing) |
| `client:logout` | transport logout |
| `spotify:user:attributes:mutated` | after 300 ms debounce: force token refresh, then re-fetch `melody/v1/product_state` |

---

## 4. Incoming commands

`_performCommand` (module 130, about 485,700) handles exactly four command types. There is **no** per-command `play`, `pause`, `seek`, `skip`, `set_queue` or `transfer` in this protocol. All of those arrive as `replace_state`.

```js
switch(e.type){case"set_volume":this._setVolume(e);break;case"log_out":this._logout();break;
  case"replace_state":this._replaceState(e);break;case"ping":this._updateState(p.DebugSource.PING);break;
  default: TP_UNKNOWN_COMMAND }
```

Every command is acknowledged by a later HTTP state or volume report, not by a websocket reply.

### 4.1 `replace_state` [seen; field list from the reader code]

```json
{
  "type": "replace_state",
  "state_machine": { ...see 6.1... },
  "state_ref": { "state_index": 3, "paused": false },
  "prev_state_ref": { "state_machine_id": "...", "state_id": "...", "paused": true },
  "seek_to": "12345",
  "context_metadata": { }
}
```

- `state_ref` can be null, meaning stop.
- `prev_state_ref` is null when nothing is playing.
- `seek_to` is optional, in milliseconds. It may be a string or a number; the code calls `parseInt`.

Algorithm (`_replaceState`, about 486,700):

1. **Stale check.** If `prev_state_ref` does not equal the device's current `{state_machine_id, state_id, paused}` (both null counts as equal), the device does not apply the command. It queues `createStateRef(state_machine, state_ref)` as a rejected ref and sends **`POST /devices/{id}/state_conflict`** (4.4).
2. **`state_ref` is null.** Pause, clear the player, drop the context, and send `PUT /state` with `debug_source:"state_clear"` and `state_ref:null`.
3. **Same `state_id` as the current state** (a change to the playing item). Swap in the new state machine, current state and context metadata, then:
   - `state_ref.paused` true: pause. False: resume.
   - The new state's `options.playback_speed` differs from the current speed: set the speed.
   - `seek_to` is a number and the state does not have `disallow_seeking`: seek with reason `remote`.
   - `preferred_media_type` is set: switch between audio and video mode.
   - If anything changed, the player events that follow produce the state reports. If nothing changed, the device sends `PUT /state` with `debug_source:"modify_current_state"`.

   This one path carries **pause, resume, seek, playback speed, shuffle and repeat changes, and queue edits**. A queue edit is a new state machine with the same current state.
4. **Different state** (play a new context, a skip, a transfer to this device, or set_queue). Build a new list context (`TrackPlayerServiceContext`, URI `spotify:app:jsspeaker`), set the state machine, call `startAtState(state_ref)`, set the initial position from `seek_to`, and `play()` with reason `remote`. The first report after this is `debug_source:"before_track_load"`.

`startAtState` makes a "decoy" current state (index -1) whose `advance` and `skip_next` transitions point to the target. The list player then advances into it.

### 4.2 `set_volume` [seen]

```json
{ "type": "set_volume", "volume": 32768, "command_id": "abc" }
```

`volume` runs from 0 to 65535 and the device divides by 65535. The device sets the local volume. The volume change then produces the acknowledgement:

```http
PUT {webgate}/track-playback/v1/devices/{id}/volume
{"seq_num": 17, "volume": 32768, "command_id": "abc"}
```

That PUT also goes out on local volume changes, with `command_id` `""`. It is debounced: sent 1 s after the last change (Debouncer with `blockInitial:true`, threshold 1000 ms, module 5). It is only sent while registered, and it has no retry options.

If a `set_volume` command has no `command_id`, the device uses `"tpcommand-<Date.now()>"` locally.

### 4.3 `ping` and `log_out` [seen]

- `ping`: send a state report now, with `debug_source:"ping"`. The server can use this to poll the device's state.
- `log_out`: pause, clear the state, deregister (`DELETE`), then emit `logout`.

### 4.4 `POST /devices/{id}/state_conflict` [seen]

`_rejectState`, about 486,000. This runs only while registered and only one conflict request at a time. Rejected refs go out in batches of up to 5.

```json
{
  "seq_num": null, "seq_nums": [18, 19],
  "state_ref": { "state_machine_id": "...", "state_id": "...", "paused": false },
  "sub_state": { ...see 5.1... },
  "previous_position": 1000,
  "rejected_state_refs": [ { "state_machine_id": "...", "state_id": "...", "paused": false } ],
  "debug_source": undefined
}
```

Each rejected ref uses up one sequence number. On 2xx, the device runs every command in the response's `body.commands[]` through `_performCommand`, normally a fresh `replace_state`.

### 4.5 Local controls (SDK API calls in the page) [seen]

`pause`, `resume`, `togglePlay`, `seek`, `nextTrack` and `previousTrack` act on the **local** list player, not on a Web API call (module 129):

- **next**: use the current state's `transitions.skip_next` for reason `fwdbtn`, otherwise `transitions.advance` (end of track uses reason `trackdone`).
- **previous**: use `transitions.skip_prev`.
- **pause, resume, seek**: change the local player, then report the new state (section 5).

So the device moves through the state machine by itself, and the server learns the new position from `PUT /state`. A transition that is null or missing means the action is not allowed (`FORBIDDEN`).

---

## 5. State reporting

### 5.1 Payload [seen]

`_generateStatePayload`, about 482,200:

```js
{seq_num:void 0,seq_nums:void 0,state_ref:e,
 sub_state:{playback_speed: e?.paused ? 0 : (currentPlaybackSpeed ?? 1),
            position:this._rawTrackPosition, duration:this._rawTrackDuration,
            media_type:i.mediaType, bitrate:i.bitrate, audio_quality:i.playbackQuality,
            format: hasFileId ? i.fileFormat : "file_urls_external",
            video_surface:i.videoSurface, is_video_on:i.isVideoVisible&&i.isMediaPlaybackModeVideo},
 previous_position:this._previousRawTrackPosition, playback_stats:void 0, rejected_state_refs:void 0, debug_source:t}
```

Example:

```http
PUT https://api.spotify.com/v1/track-playback/v1/devices/{device_id}/state
Authorization: Bearer <token>
Content-Type: application/json

{
  "seq_num": 12,
  "state_ref": { "state_machine_id": "<id>", "state_id": "<id>", "paused": false },
  "sub_state": {
    "playback_speed": 1,
    "position": 15342,
    "duration": 215000,
    "media_type": "AUDIO",
    "bitrate": 128000,
    "audio_quality": "<from manifest file audio_quality>",
    "format": 10
  },
  "previous_position": 15342,
  "debug_source": "pause"
}
```

Field notes:

- `position`, `duration` and `previous_position` are in milliseconds. They are "raw" values, before any `position_offset` or `duration_override` from the state.
- `media_type` is `"AUDIO"` or `"VIDEO"` (upper case).
- `format` is the numeric `AudioFileFormat` from the manifest entry's `format`: MP3_256=3, MP3_320=4, MP3_160=5, MP3_96=6, MP4_128=10, MP4_256=11, MP4_128_DUAL=12, MP4_256_DUAL=13, MP4_128_CBCS=14, MP4_256_CBCS=15, MP4_FLAC=17. For external URL playback it is the string `"file_urls_external"`.
- `playback_speed` is 0 when paused.
- `playback_stats` is set only on the report sent when a track ends or changes (`track_data_finalized`). Its contents come from the player's tracking data and this document does not reconstruct them.
- `seq_num` goes up by one on each report. It starts from `initial_seq_num` in the register response; the first report sends `initial_seq_num + 1`.
- On server or connection errors the request retries up to 8 times.

### 5.2 Response [seen in reader code; values inferred]

`_handleStateUpdateResponse`, about 483,900:

```json
{ "state_machine": { ...full or updated machine... },
  "updated_state_ref": { "state_index": 4, "paused": false },
  "context_metadata": { },
  "endsong": "...", "endsongs": [ ... ] }
```

- The device applies `state_machine`, `updated_state_ref` and `context_metadata` only if the `state_ref` it sent is still its current one.
- The device needs this response to get more tracks. The server extends the machine (more next tracks, new transitions) as playback moves on.
- A non-200 response or an empty body counts as an error.

### 5.3 When reports are sent [seen]

`_updateState(debug_source)`. The DebugSource enum is module 135.

| debug_source | trigger |
|--------------|---------|
| `before_track_load` | Player is about to load a track (new context, or advance to the next state). Position is the start position. |
| `started_playing` | First progress event with more than 1000 ms played. |
| `played_threshold_reached` | Played time reached the track's `ms_played_until_update`. |
| `progress` | Periodic. After the first `played_threshold_reached`, sent when more than the track's `ms_playing_update_interval` ms have passed since the last report. Progress events come from the player. |
| `pause` / `resume` | Local or remote pause and resume. Skipped when the event's stateId equals `_lastProcessedStateId`, which is set only for the state a `replace_state` just loaded. |
| `position_changed` | Seek. |
| `speed_changed` | Playback speed change. |
| `track_data_finalized` | Track ended or changed. The payload is captured before the change (with the old state_ref and position) and sent once the player finalizes tracking, with `playback_stats` attached. |
| `error` | Playback error that carries a position. |
| `capped` | Playback capped (free-tier limit). The context is cleared. |
| `modify_current_state` | `replace_state` for the current state with nothing to change locally (4.1, step 3). |
| `state_clear` | Context cleared (`replace_state` with null, or `stop()`). |
| `ping` | `ping` command. |
| `register` / `deregister` | Used for the `previous_session_state` and DELETE bodies. |
| video sources | Video only; ignore for audio. |

Reports are deduplicated (`_shouldSendPayload`, about 490,800). A report is skipped unless at least one of these changed: `state_ref` (paused, state_id, state_machine_id), `sub_state` `playback_speed`, `position`, `video_surface`, `is_video_on`, `previous_position`, or `duration` by 51 ms or more.

Only one `PUT /state` is in flight at a time when `_limitStateUpdates` is on. It is off by default, so updates can overlap. When the limit is on, the device queues reports and sends the newest one afterwards.

### 5.4 Track end and errors

- **End of track**: the list player calls `context.next({reason:"trackdone"})`, which follows `transitions.advance`. The device sends `track_data_finalized` for the old track, then `before_track_load` for the new one.
- **No `advance` transition**: the list ends. The next server response or command decides what happens [inferred].
- **Errors**: errors are not reported to track-playback itself, apart from the `error` state report. They are logged to `@webgate/melody/v1/msg/...` (the event logger) [the logger path is inferred from the logger module, about 20,800].

---

## 6. Track list, context and file selection

### 6.1 State machine [inferred from the reader code; field names seen]

Modules 131 and 132, plus the resolver in module 44:

```json
{
  "state_machine_id": "<id>",
  "attributes": { "options": { "shuffling_context": false, "repeating_context": false, "repeating_track": false } },
  "states": [
    {
      "state_id": "<id>",
      "track": 0,
      "track_uid": "...", "track_metadata": { },
      "transitions": {
        "advance":   { "state_index": 1, "paused": false },
        "skip_next": { "state_index": 1, "paused": false },
        "skip_prev": { "state_index": 0, "paused": false },
        "show_next": { "state_index": 1 },
        "show_prev": { "state_index": 0 }
      },
      "restrictions": { "disallow_pausing_reasons": [], "disallow_resuming_reasons": [], "disallow_seeking_reasons": [],
                        "disallow_setting_playback_speed_reasons": [], "...": [] },
      "disallow_seeking": false,
      "options": { "playback_speed": 1, "modes": { }, "shuffling_context": false },
      "playback_offsets": { "start": 0, "end": 30000 },
      "initial_playback_position": 0,
      "duration_override": null, "position_offset": 0,
      "signals": [], "preferred_media_type": "AUDIO", "ad_break_context": null
    }
  ],
  "tracks": [
    {
      "track_type": "AUDIO",
      "ms_played_until_update": 30000,
      "ms_playing_update_interval": 0,
      "metadata": { "uri": "spotify:track:...", "name": "...", "authors": [{"name":"...","uri":"..."}],
                    "group_uri": "spotify:album:...", "group_name": "...",
                    "images": [{"url":"spotify:image:<hex>","width":640,"height":640}],
                    "duration": 215000, "context_uri": "spotify:playlist:...",
                    "linked_from_uri": null, "episode_content_type": null },
      "manifest": {
        "file_ids_mp4": [ { "file_id": "<hex>", "format": "10", "bitrate": 128000,
                            "audio_quality": "...", "hifi_status": "...", "gain_db": -6.1 } ],
        "file_ids_mp4_dual": [ ... ],
        "file_urls_external": [ { "file_url": "https://..." } ]
      }
    }
  ]
}
```

How these fields are used:

- The SDK's `player_state_changed` builds `track_window` from this machine: two `show_next` steps forward and two `show_prev` steps back (module 132).
- Image URLs of the form `spotify:image:<id>` become `https://i.scdn.co/image/<id>`.
- `options.shuffling_context`, `repeating_context` and `repeating_track` give the shuffle and repeat state.

### 6.2 File selection (`ManifestTrackResolver`, module 44, about 126,600) [seen]

1. Walk the device's `manifest_formats` **in order**. For each format, walk `track.manifest[format]` in order.
2. Accept an entry if it has a `file_id`, or if its `file_url` starts with `https://`.
3. **Skip `file_ids_mp3` unless the track is an ad or an interruption.** In practice, music plays from `file_ids_mp4`, which is Widevine CENC AAC.
4. Bitrate: `bitrate ?? (flac ? 1411200 : 128000)`. The first entry whose bitrate is at or below the current bandwidth estimate wins. If none qualifies, the last match is used.
5. Format to media type: `file_*_mp3` maps to MP3 (plain URL or storage-resolve), `file_ids_mp4*` to MP4, `*_cbcs` to MP4_CBCS, `mp4flac` to MP4_FLAC, `*_ad` to AD_MANIFEST.

### 6.3 Fetching the audio [seen]

1. **CDN URL** (module 119, about 413,200):

   ```
   GET @webgate/storage-resolve/v2/files/audio/interactive/{fileFormat}/{fileId}?version=10000000&product=9&platform=39&alt=json
   ```

   - Without a known `fileFormat`, the path is `files/audio/interactive/{fileId}`.
   - Bearer auth.
   - Response: `{"cdnurl":["https://...", ...]}`. The first URL is used and the rest are fallbacks.
   - Retries on 429 and 5xx.
   - The SDK waits at least 1 s between storage-resolve calls.

2. **Seek table**: `GET https://seektables.scdn.co/seektable/{fileId}.json`, no auth. The body holds:
   - `offset` (init segment size);
   - `timescale`;
   - `segments: [[byteSize, duration], ...]`, or the older `references: [{size, duration}]`;
   - `pssh` or `pssh_widevine`, `pssh_playready`, `pssh_fairplay`;
   - `protection` (default `"cenc"`).

   The player then fetches byte ranges from the CDN for each segment into MSE.

3. **License URL**:

   ```
   GET @webgate/melody/v1/license_url?keysystem=com.widevine.alpha&mediatype=audio&sdk_name=harmony&sdk_version=4.67.0
   ```

   - Response: `{"uri":"widevine-license/v1/audio/license?...", "expires": <seconds>}`.
   - The SDK prefixes `@webgate/` to `uri` and caches it until 60 s before expiry.
   - Fallback if this call fails: `@webgate/widevine-license/v1/audio/license`.
   - A 400 with `{"code":"deprecated-version"}` means the SDK is too old.

4. **License**: `POST <license uri>` with the raw Widevine challenge (an arraybuffer) as the body, and Bearer auth (EMEManager, about 220,200).

5. **`file_urls_external`** (podcasts hosted elsewhere): play `file_url` directly.

6. **Video resolve** (`GET @webgate/track-playback/v1/media/{uri}?manifestFileFormat=...`) only runs for music videos and does not matter for audio.

---

## 7. Call sequence (cold start)

1. `GET https://apresolve.spotify.com/?type=dealer-g2&type=spclient` gives the dealer host.
2. The page's `getOAuthToken` gives a token. The SDK checks it with `GET api.spotify.com/v1/melody/v1/check_scope?scope=web-playback`.
3. Open `wss://<dealer>/?access_token=<token>`.
4. Receive `{"type":"message","uri":"hm://pusher/v1/connections/<id>", "headers":{"Spotify-Connection-Id":"<id>"}}`. Store the connection id and send `{"type":"ping"}`. From then on, ping 30 s after each pong, with a 10 s timeout.
5. `POST api.spotify.com/v1/track-playback/v1/devices {device, connection_id, client_version, volume, outro_endcontent_snooping:false}`. Save `initial_seq_num`.
6. The device now appears in Connect. Wait for commands.
7. The user picks the device in another client. The dealer sends `hm://track-playback/v1/...` with payload `{"type":"replace_state", state_machine, state_ref, prev_state_ref:null, seek_to}`.
8. Load the track for `state_ref.state_index` and send `PUT .../state` with `before_track_load`. Resolve the file, then storage-resolve, seektable, license and CDN.
9. While playing, send `PUT .../state` for `started_playing`, then `played_threshold_reached`, then `progress` (if the track sets an interval), and on pause, resume or seek. Apply the `state_machine` and `updated_state_ref` from each response.
10. At track end, send `track_data_finalized`, follow `advance`, then send `before_track_load`.
11. Remote control arrives as more `replace_state` commands (same state: pause, seek or speed; new state: skip or new context), plus `set_volume` (answered with `PUT .../volume`) and `ping` (answered with `PUT .../state`).
12. On shutdown, send `DELETE .../devices/{id}` with a final state body.

---

## 8. What the bundle contains but the SDK does not use

Module 36 is a connect-state client (`PlayerAPIClient`). It is created only through `createControllerForClient` (module 134), which the embedded player never calls. What it would do [seen]:

- **Register as a hidden observer**:

  ```http
  PUT @webgate/connect-state/v1/devices/hobs_<device_id>
  X-Spotify-Connection-Id: <connection id>
  Accept: application/json
  Content-Type: application/json

  {"member_type":"CONNECT_STATE","device":{"device_info":{"capabilities":{"can_be_player":false,"hidden":true,"needs_full_player_state":true}}}}
  ```

  The response is a cluster.

- **Subscribe** to dealer URIs matching `/connect-state\/v1\/cluster$/`. The payload is `{ack_id, cluster}`, and `cluster` holds `timestamp`, `devices{}`, `active_device_id` and `player_state`.

- **Send commands**:
  - `POST @webgate/connect-state/v1/player/command/from/<self>/to/<target>` with body `{"command":{"endpoint":"<name>", ..., "logging_params":{"command_id":"<id>"}}}`. Names: `play`, `pause`, `resume`, `skip_next`, `skip_prev`, `seek_to`, `set_queue`, `add_to_queue`, `play_as_next_in_queue`, `update_context`, `set_shuffling_context`, `set_repeating_context`, `set_repeating_track`, `set_options`, `signal`, `set_sleep_timer`.
  - `connect/transfer` (POST), `connect/volume` (PUT) and `connect/logout` (POST) use the path `@webgate/connect-state/v1/connect/<name>/from/<self>/to/<target>`.

This is how a controller talks to other devices. It is not how a playback device registers in this SDK. The desktop client's xpui bundle contains no `track-playback/v1` or `connect-state/v1/devices` URLs, only a `"track-playback"` enum string, so its device registration lives in native code.

---

## 9. Open questions and risks for a native Rust device

1. **DRM.** On Chrome and Firefox the SDK plays music only from `file_ids_mp4` and `file_ids_mp4_dual` (Widevine CENC), and it skips `file_ids_mp3` for music. A native app has no Widevine CDM. Untested options:
   - Advertise only `file_ids_mp3` and `file_urls_mp3`, and see whether the server then fills `manifest.file_ids_mp3` for music. Even if it does, those files are probably encrypted with keys this SDK never fetches.
   - Use macOS FairPlay through AVFoundation (`file_ids_mp4_cbcs` plus `@webgate/fairplay-license/v1/audio/license`, with `assetId=hex` appended), as Safari does. This looks like the most plausible path on macOS.
2. **Token scope and client.** The Web API host accepts `/v1/track-playback/...` with a Web API OAuth token that has the `streaming` scope. This document has not checked whether a non-SDK client id may register. The server may also check `brand`, `platform_identifier` or the client id.
3. **The `;tp` suffix** in `platform_identifier` (`"Partner public_js-sdk ...;tp"`) probably marks a track-playback partner device. Its exact meaning is unknown.
4. **Server response shapes.** These are reconstructed from reader code, not observed: the register response (only `initial_seq_num` is read), the state machine schema, the `seek_to` type, and `playback_stats` contents. A live capture from a browser running the official SDK would confirm them. That needs your own authenticated session, which this analysis did not use.
5. **Dealer payload encoding** for `hm://track-playback`. The code assumes plain JSON objects in `payloads`. Handle gzip and base64 defensively anyway.
6. **`ms_playing_update_interval`.** Many tracks may set it to 0, meaning no periodic progress reports. The server must then infer position from timestamps.
7. **Quotas.** `POST /devices` returns 429 when an account has too many devices or subscriptions.
