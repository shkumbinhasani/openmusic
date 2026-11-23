use anyhow::Result;
use prost::Message as _;
use reqwest::Method;
use serde_json::{Value, json};
use spotify_proto::connectstate::{
    Capabilities, Cluster, ClusterUpdate, Device, DeviceInfo, MemberType, PutStateReason,
    PutStateRequest,
};

use crate::{
    api::{Api, Body},
    catalog::{Track, image_url},
    dealer::Message,
};

pub const CLUSTER_URI: &str = "hm://connect-state/v1/cluster";

pub fn observer_id(device_id: &str) -> String {
    format!("hobs_{}", &device_id[..device_id.len().min(35)])
}

pub async fn observe(api: &Api, device_id: &str, connection_id: &str) -> Result<Cluster> {
    let observer = observer_id(device_id);
    let request = PutStateRequest {
        member_type: MemberType::ConnectState as i32,
        put_state_reason: PutStateReason::NewDevice as i32,
        device: Some(Device {
            device_info: Some(DeviceInfo {
                name: "OpenMusic".into(),
                device_id: observer.clone(),
                capabilities: Some(Capabilities {
                    hidden: true,
                    is_observable: true,
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    let bytes = api
        .http()
        .put(api.url(&format!("/connect-state/v1/devices/{observer}")))
        .bearer_auth(api.token())
        .header("x-spotify-connection-id", connection_id)
        .header("content-type", "application/x-protobuf")
        .header("accept", "application/x-protobuf")
        .body(request.encode_to_vec())
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    Ok(Cluster::decode(bytes.as_ref())?)
}

pub fn cluster_from(message: &Message) -> Option<Cluster> {
    if !message.uri.starts_with(CLUSTER_URI) {
        return None;
    }
    ClusterUpdate::decode(message.bytes()?.as_slice())
        .ok()?
        .cluster
}

pub struct Remote {
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub volume: Option<u16>,
    pub track: Option<Track>,
    pub playing: bool,
    pub position_ms: u32,
}

pub fn remote_state(cluster: &Cluster) -> Remote {
    let active = (!cluster.active_device_id.is_empty()).then(|| cluster.active_device_id.clone());
    let device = active.as_ref().and_then(|id| cluster.device.get(id));
    let state = cluster.player_state.as_ref();
    let playing =
        active.is_some() && state.is_some_and(|state| state.is_playing && !state.is_paused);
    let position_ms = state
        .map(|state| {
            let elapsed = if playing
                && state.timestamp > 0
                && cluster.server_timestamp_ms > state.timestamp
            {
                cluster.server_timestamp_ms - state.timestamp
            } else {
                0
            };
            (state.position_as_of_timestamp + elapsed).max(0) as u32
        })
        .unwrap_or_default();
    let track = state.and_then(|state| {
        let provided = state.track.as_ref()?;
        if provided.uri.is_empty() {
            return None;
        }
        let field = |key: &str| provided.metadata.get(key).cloned().unwrap_or_default();
        let artist = Some(field("artist_name"))
            .filter(|artist| !artist.is_empty())
            .unwrap_or_else(|| field("album_title"));
        Some(Track {
            uri: provided.uri.clone(),
            title: field("title"),
            artist,
            album: field("album_title"),
            duration_ms: state.duration.max(0) as u32,
            cover: provided
                .metadata
                .get("image_url")
                .map(|image| image_url(image)),
            artist_uri: (!provided.artist_uri.is_empty()).then(|| provided.artist_uri.clone()),
            album_uri: (!provided.album_uri.is_empty()).then(|| provided.album_uri.clone()),
        })
    });
    Remote {
        volume: device.map(|device| device.volume.min(u32::from(u16::MAX)) as u16),
        device_name: device.map(|device| device.name.clone()),
        device_id: active,
        track,
        playing,
        position_ms,
    }
}

pub async fn command(api: &Api, from: &str, to: &str, command: Value) -> Result<()> {
    let mut command = command;
    if command.get("logging_params").is_none() {
        command["logging_params"] = json!({});
    }
    api.request(
        Method::POST,
        &format!("/connect-state/v1/player/command/from/{from}/to/{to}"),
        Body::Json(json!({ "command": command })),
        "application/json",
    )
    .await?;
    Ok(())
}

pub async fn set_volume(api: &Api, from: &str, to: &str, volume: u16) -> Result<()> {
    api.request(
        Method::PUT,
        &format!("/connect-state/v1/connect/volume/from/{from}/to/{to}"),
        Body::Json(json!({ "volume": volume })),
        "application/json",
    )
    .await?;
    Ok(())
}

pub async fn transfer(api: &Api, from: &str, to: &str) -> Result<()> {
    api.request(
        Method::POST,
        &format!("/connect-state/v1/connect/transfer/from/{from}/to/{to}"),
        Body::Json(json!({ "transfer_options": { "restore_paused": "resume" } })),
        "application/json",
    )
    .await?;
    Ok(())
}

pub fn play_command(uris: &[String], index: usize, position_ms: u32) -> Value {
    let tracks: Vec<Value> = uris.iter().map(|uri| json!({ "uri": uri })).collect();
    let mut options = json!({ "skip_to": { "track_index": index, "track_uri": uris.get(index) } });
    if position_ms > 0 {
        options["seek_to"] = json!(position_ms);
    }
    json!({
        "endpoint": "play",
        "context": {
            "uri": format!("spotify:app:openmusic:{}", uris.get(index).map(String::as_str).unwrap_or_default()),
            "url": "",
            "metadata": {},
            "pages": [{ "tracks": tracks }],
        },
        "play_origin": { "feature_identifier": "openmusic" },
        "options": options,
    })
}

pub fn play_context_command(context: &str, index: usize, track_uri: Option<&str>) -> Value {
    let mut skip_to = json!({ "track_index": index });
    if let Some(track_uri) = track_uri {
        skip_to["track_uri"] = json!(track_uri);
    }
    json!({
        "endpoint": "play",
        "context": { "uri": context, "url": format!("context://{context}"), "metadata": {} },
        "play_origin": { "feature_identifier": "openmusic" },
        "options": { "skip_to": skip_to },
    })
}
