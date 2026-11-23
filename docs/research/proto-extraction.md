# Protobuf schema extraction from the Spotify macOS app

Source: `/Applications/Spotify.app/Contents/MacOS/Spotify`, Spotify 1.3.10, arm64 Mach-O, 43.4 MB. Read only; nothing under `/Applications` was changed.

Output: `crates/spotify-proto/proto/` (950 `.proto` files, same relative paths as the names inside the binary).

## Result in brief

- The binary mixes two protobuf builds. Most messages use the full C++ runtime and carry a serialized `FileDescriptorProto` each. A smaller set (login5, some auth and storage internals) uses protobuf-lite and carries no descriptor.
- 961 descriptor blobs found, all with unique names and no conflicting copies. 11 are Google's own well-known types (`google/protobuf/*.proto`); I left those out because `protoc` and `prost-build` ship them. That leaves **950 Spotify files**: 604 proto3, 346 proto2, about 2,900 messages, 74 services, 326 RPCs.
- All 950 files compile together with `protoc` 34.1 and every import resolves (`protoc -I proto -o /dev/null $(find proto -name '*.proto')`; the only warnings are 7 unused imports that exist in Spotify's own sources).
- **Not recovered:** the AP handshake and AP authentication messages (no trace of them in this binary) and login5 (lite build: type names only). See Gaps.

## How extraction works

1. Scan the file for the bytes `0x0a <varint len> <name>.proto` where the length matches the name. That is field 1 (`name`) of a `FileDescriptorProto`.
2. From each hit, hand-decode the wire format against a built-in table of the `descriptor.proto` schema (file, message, field, enum, service, method, oneof, ranges, options). Parsing of the top-level message stops at the first tag that is not a valid `FileDescriptorProto` field, which marks the end of the blob. Nested messages must parse exactly to their length or the hit is rejected.
3. Render `.proto` text: syntax, package, imports (incl. `public`/`weak`), file options, messages with nested types and enums, labels (proto2 `optional/required/repeated`, proto3 implicit or `optional`), `oneof` blocks (proto3 synthetic oneofs are folded back into `optional`), `map<K,V>` (from `map_entry` nested types), groups, defaults, `packed`/`deprecated`/`json_name`, extension ranges, `reserved`, `extend` blocks, and services.
4. Custom options are stored as unknown fields on the options messages. The extractor resolves them against the `extend` declarations found in the same set and prints them by name, e.g. `option (spotify.esperanto.direct_call) = true;` and `[(spotify.list.v1.model.expose_list_metadata_v2_ext) = true]`.

Checks:

- Compile the whole set with `protoc` and dump a descriptor set.
- Round-trip: parse protoc's output with the same decoder and compare it, field by field, to the original blobs (ignoring `json_name`). 950 of 961 match exactly. The rest differ only in ways that change nothing on the wire: map-entry nested types listed in a different order (3 files), empty `MethodOptions` messages in the original (4 files), and the skipped Google files.

The extractor is stdlib-only Python (`extract.py`, plus `roundtrip.py` and `lite.py` for the checks). It lives outside the repo in the scratch area used for this work; rerun with `python3 -I extract.py <binary> <outdir> <index.json>`.

## Required schemas

| Area | Message | File |
|---|---|---|
| AP key exchange | ClientHello, APResponseMessage, ClientResponsePlaintext, LoginCryptoDiffieHellman* | **not present** |
| AP authentication | ClientResponseEncrypted, LoginCredentials, APWelcome, APLoginFailed | **not present** |
| login5 | LoginRequest, LoginResponse, StoredCredential, ClientInfo | **lite only, not recoverable** (see Gaps) |
| clienttoken | ClientTokenRequest, ClientTokenResponse | `spotify/client_token/v0/client_token_http.proto` (`spotify.clienttoken.http.v0`) |
| clienttoken | ConnectivitySdkData, PlatformSpecificData, NativeDesktopMacOSData | `connectivity.proto` (`spotify.clienttoken.data.v0`) |
| clienttoken | stored token | `client_token.proto` |
| metadata | Track, Album, Artist, AudioFile, Episode | `metadata.proto` (`spotify.metadata`) |
| extended metadata | BatchedEntityRequest, EntityRequest, ExtensionQuery, BatchedExtensionResponse | `extended_metadata.proto` (`spotify.extendedmetadata`) |
| extended metadata | ExtensionKind | `extension_kind.proto` |
| extended metadata | config service | `spotify/extendedmetadata/v3/extension_config.proto`, `.../extension_config_service.proto` |
| storage-resolve | StorageResolveResponse | `storage-resolve.proto` (`spotify.download.proto`) |
| Connect | Cluster, ClusterUpdate, PutStateRequest, Device, DeviceInfo, Capabilities | `connect.proto` (`spotify.connectstate`) |
| Connect | PlayerState, ProvidedTrack | `player.proto` (`spotify.connectstate`) |
| context | Context, ContextPage, ContextTrack | `context.proto`, `context_page.proto`, `context_track.proto` (`spotify.player.proto`) |

Several names also exist in other packages (for example `es_context.proto`, `es_provided_track.proto` and `state_restore/provided_track.proto` for the local player and its saved state). Use the package to pick the right one.

### Playback

No file is named `track-playback`. Playback-related files found:

- Core player: `playback.proto`, `playback_state.proto`, `playback_esperanto.proto`, `playback_observer.proto`, `playback_segments.proto`, `playback_stack.proto`, `playback_stack_v2.proto`, `player_license.proto`, `play_origin.proto`, `play_reason.proto`, `prepare_play_options.proto`, `context_player_options.proto`, `play_queue_node.proto`, `progress_playback_esperanto.proto`
- Settings: `playback-settings.proto`, `playback-settings-pubsub.proto`, `playback-settings-storage.proto`, `playback_settings.proto`, `playback_settings_esperanto_service.proto`
- Traits: `playback_trait.proto`, `playback_trait_v2.proto`, `playback_trait_v2_transition_map.proto`, `playback_experience_trait.proto`
- New playback platform (about 60 files): `spotify/playback_platform/{common,context,timeline,media,transition,external}/v1/...`
- Saved player state: `state_restore/*.proto`, `player_sequence/player/state/*.proto`
- Event reports: `include/PlaybackError.proto`, `include/PlaybackRetry.proto`, `include/BoomboxPlaybackSession.proto`, `include/RawCoreStream.proto`, and the rest of `include/*.proto`

## Gaps

### AP handshake and AP authentication

No descriptor, name table, or string for `ClientHello`, `APResponseMessage`, `APWelcome`, `LoginCredentials`, `keyexchange` or `authentication.proto` exists in this binary. The app still resolves access points (`apresolve`, `accesspoint_resolve`, `ap.spotify.com`), but whatever it speaks there is not built from those messages in this version, or the code lives elsewhere. Those schemas cannot come from this source.

### login5 (lite runtime)

`spotify.login5.v3` and `spotify.login5.v4` (LoginRequest, LoginResponse, LoginOk, ClientInfo, Challenges, Hashcash/Code challenges, the credential types including `StoredCredential`, identifiers) are built with protobuf-lite. What the binary keeps for each message is a name table: the full type name followed by the names of its **string** fields only (used in UTF-8 error messages), for example `spotify.login5.v4.ClientInfo` + `client_id`, `device_id`. Field numbers, non-string fields, types and nesting are not in readable form. They sit in the generated parse tables (`TcParseTable`) in code and data sections; decoding those would need a separate disassembly-based tool and was not attempted. Full login5 schemas cannot be recovered by descriptor extraction.

The same holds for about 100 smaller lite packages, among them `spotify.authentication.login5.impl.proto`, `spotify.authenticated_sessions.*` and `spotify.desktop.proto.storage`. None of the other required schemas depend on them.

### Other notes

- Comments and source layout are gone: descriptors in the binary carry no `source_code_info`. Field order, numbering, names and types are exact.
- `json_name` is printed only when it differs from the default.
- For prost: `metadata.proto` and 345 other files are proto2, and 30 of them use `required` fields; prost handles both. No file uses groups. Some files share a package across many files, so generate with all files in one `prost_build::compile_protos` call.
