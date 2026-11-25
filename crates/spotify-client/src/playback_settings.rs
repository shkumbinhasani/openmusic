use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use prost::Message as _;
use serde::{Deserialize, Serialize};
pub use spotify_proto::playbacksettings::DeviceDescriptor;
use spotify_proto::playbacksettings::{
    Field, FieldId, FieldValue, GetSettingsDeviceSelectionRequest,
    GetSettingsDeviceSelectionResponse, WriteContentValueRequest, WriteDeviceValueRequest,
    WriteUserSettingValueRequest,
    field_value::Value,
    pubsub::{
        ContentSettingsChangedEvent, DeviceSettingsFieldsChangedEvent, UserSettingsChangedEvent,
    },
};

use crate::{api::Api, dealer::Message, oauth};

pub const SERVICE: &str = "/playback-settings/spotify.playbacksettings.PlaybackSettingsService";
pub const DEVICE_MODEL: &str = "harmony-safari.18-mac";
pub const MAX_CROSSFADE_MS: u32 = 12_000;
pub const MIN_SPEED: f32 = 0.5;
pub const MAX_SPEED: f32 = 3.5;
pub const REQUESTED: [FieldId; 6] = [
    FieldId::PlaybackSpeed,
    FieldId::Gapless,
    FieldId::Crossfade,
    FieldId::CrossfadeDurationMs,
    FieldId::Normalize,
    FieldId::NormalizeLevel,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LoudnessLevel {
    Quiet,
    #[default]
    Normal,
    Loud,
}

impl LoudnessLevel {
    pub const ALL: [Self; 3] = [Self::Quiet, Self::Normal, Self::Loud];

    pub fn from_index(index: i32) -> Self {
        match index {
            0 => Self::Quiet,
            2 => Self::Loud,
            _ => Self::Normal,
        }
    }

    pub fn index(self) -> i32 {
        match self {
            Self::Quiet => 0,
            Self::Normal => 1,
            Self::Loud => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Quiet => "quiet",
            Self::Normal => "normal",
            Self::Loud => "loud",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|level| level.name().eq_ignore_ascii_case(text.trim()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserAudio {
    pub gapless: bool,
    pub crossfade: bool,
    pub crossfade_ms: u32,
}

impl Default for UserAudio {
    fn default() -> Self {
        Self {
            gapless: true,
            crossfade: false,
            crossfade_ms: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceAudio {
    pub normalize: bool,
    pub level: LoudnessLevel,
}

impl Default for DeviceAudio {
    fn default() -> Self {
        Self {
            normalize: true,
            level: LoudnessLevel::Normal,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Remote {
    pub user: Option<UserAudio>,
    pub device: Option<DeviceAudio>,
    pub speeds: HashMap<String, f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    Device,
    Content,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setting {
    Gapless(bool),
    Crossfade(bool),
    CrossfadeMs(u32),
    Normalize(bool),
    Level(LoudnessLevel),
    Speed(f32),
}

impl Setting {
    pub fn id(self) -> FieldId {
        match self {
            Self::Gapless(_) => FieldId::Gapless,
            Self::Crossfade(_) => FieldId::Crossfade,
            Self::CrossfadeMs(_) => FieldId::CrossfadeDurationMs,
            Self::Normalize(_) => FieldId::Normalize,
            Self::Level(_) => FieldId::NormalizeLevel,
            Self::Speed(_) => FieldId::PlaybackSpeed,
        }
    }

    pub fn scope(self) -> Scope {
        match self {
            Self::Gapless(_) | Self::Crossfade(_) | Self::CrossfadeMs(_) => Scope::User,
            Self::Normalize(_) | Self::Level(_) => Scope::Device,
            Self::Speed(_) => Scope::Content,
        }
    }

    pub fn field(self) -> Field {
        let value = match self {
            Self::Gapless(on) | Self::Crossfade(on) | Self::Normalize(on) => Value::BoolValue(on),
            Self::CrossfadeMs(ms) => Value::IntValue(ms.min(MAX_CROSSFADE_MS) as i32),
            Self::Level(level) => Value::IntValue(level.index()),
            Self::Speed(speed) => Value::FloatValue(speed),
        };
        Field {
            id: self.id() as i32,
            value: Some(FieldValue { value: Some(value) }),
        }
    }

    pub fn from_field(field: &Field) -> Option<Self> {
        let value = field.value.as_ref()?.value.as_ref()?;
        let id = FieldId::try_from(field.id).ok()?;
        match (id, value) {
            (FieldId::Gapless, Value::BoolValue(on)) => Some(Self::Gapless(*on)),
            (FieldId::Crossfade, Value::BoolValue(on)) => Some(Self::Crossfade(*on)),
            (FieldId::CrossfadeDurationMs, Value::IntValue(ms)) => {
                Some(Self::CrossfadeMs(crossfade_ms(*ms)))
            }
            (FieldId::Normalize, Value::BoolValue(on)) => Some(Self::Normalize(*on)),
            (FieldId::NormalizeLevel, Value::IntValue(level)) => {
                Some(Self::Level(LoudnessLevel::from_index(*level)))
            }
            (FieldId::PlaybackSpeed, Value::FloatValue(speed)) => {
                valid_speed(*speed).map(Self::Speed)
            }
            _ => None,
        }
    }
}

pub fn crossfade_ms(ms: i32) -> u32 {
    ms.clamp(0, MAX_CROSSFADE_MS as i32) as u32
}

pub fn valid_speed(speed: f32) -> Option<f32> {
    (speed.is_finite() && (MIN_SPEED..=MAX_SPEED).contains(&speed)).then_some(speed)
}

pub fn round_speed(speed: f32) -> f32 {
    (speed * 20.).round() / 20.
}

pub fn descriptor(device_id: &str, device_name: &str) -> DeviceDescriptor {
    DeviceDescriptor {
        device_id: device_id.to_owned(),
        device_name: device_name.to_owned(),
        client_id: oauth::DESKTOP_CLIENT_ID.to_owned(),
        device_model: DEVICE_MODEL.to_owned(),
    }
}

pub fn selection_request(
    device: Option<&DeviceDescriptor>,
    shows: &[String],
) -> GetSettingsDeviceSelectionRequest {
    GetSettingsDeviceSelectionRequest {
        device_capabilities: None,
        requested_setting_fields: REQUESTED.iter().map(|id| *id as i32).collect(),
        device_descriptor: device.cloned(),
        content_identifiers: shows.to_vec(),
        include_user_settings: Some(true),
    }
}

pub fn parse_selection(response: &GetSettingsDeviceSelectionResponse) -> Remote {
    Remote {
        user: response.user_settings.as_ref().map(|user| UserAudio {
            gapless: user.gapless,
            crossfade: user.crossfade,
            crossfade_ms: crossfade_ms(user.crossfade_duration_ms),
        }),
        device: response.device_settings.as_ref().map(|device| DeviceAudio {
            normalize: device.normalize,
            level: LoudnessLevel::from_index(device.normalize_level),
        }),
        speeds: response
            .content_settings
            .iter()
            .filter_map(|(uri, content)| Some((uri.clone(), valid_speed(content.playback_speed)?)))
            .collect(),
    }
}

pub async fn read(
    api: &Api,
    device: Option<&DeviceDescriptor>,
    shows: &[String],
) -> Result<Remote> {
    let body = selection_request(device, shows).encode_to_vec();
    let reply = api
        .grpc(&format!("{SERVICE}/GetSettingsDeviceSelection"), body)
        .await
        .context("Spotify did not send your playback settings.")?;
    let response = GetSettingsDeviceSelectionResponse::decode(reply.as_slice())
        .context("Spotify sent unreadable playback settings.")?;
    Ok(parse_selection(&response))
}

fn fields(settings: &[Setting], scope: Scope) -> Result<Vec<Field>> {
    if let Some(setting) = settings.iter().find(|setting| setting.scope() != scope) {
        bail!("{setting:?} is not a {scope:?} setting.");
    }
    if settings.is_empty() {
        bail!("No settings to write.");
    }
    Ok(settings.iter().map(|setting| setting.field()).collect())
}

pub fn user_request(settings: &[Setting]) -> Result<WriteUserSettingValueRequest> {
    Ok(WriteUserSettingValueRequest {
        user_settings: fields(settings, Scope::User)?,
    })
}

pub fn device_request(
    device: &DeviceDescriptor,
    settings: &[Setting],
) -> Result<WriteDeviceValueRequest> {
    Ok(WriteDeviceValueRequest {
        device_descriptor: Some(device.clone()),
        field: fields(settings, Scope::Device)?,
    })
}

pub fn content_request(content_id: &str, settings: &[Setting]) -> Result<WriteContentValueRequest> {
    if content_id.is_empty() {
        bail!("Content settings need a show or audiobook.");
    }
    Ok(WriteContentValueRequest {
        content_identifier: content_id.to_owned(),
        field: fields(settings, Scope::Content)?,
    })
}

async fn write(api: &Api, method: &str, body: Vec<u8>) -> Result<()> {
    api.grpc(&format!("{SERVICE}/{method}"), body)
        .await
        .context("Spotify did not save the playback setting.")?;
    Ok(())
}

pub async fn write_user(api: &Api, settings: &[Setting]) -> Result<()> {
    let body = user_request(settings)?.encode_to_vec();
    write(api, "WriteUserSettingValue", body).await
}

pub async fn write_device(
    api: &Api,
    device: &DeviceDescriptor,
    settings: &[Setting],
) -> Result<()> {
    let body = device_request(device, settings)?.encode_to_vec();
    write(api, "WriteDeviceValue", body).await
}

pub async fn write_content(api: &Api, content_id: &str, settings: &[Setting]) -> Result<()> {
    let body = content_request(content_id, settings)?.encode_to_vec();
    write(api, "WriteContentValue", body).await
}

pub async fn write_show_speed(api: &Api, show: &str, speed: f32) -> Result<()> {
    let speed = valid_speed(round_speed(speed)).context("Speed must be between 0.5× and 3.5×.")?;
    write_content(api, show, &[Setting::Speed(speed)]).await
}

pub fn by_scope(settings: &[Setting]) -> [(Scope, Vec<Setting>); 3] {
    [Scope::User, Scope::Device, Scope::Content].map(|scope| {
        (
            scope,
            settings
                .iter()
                .copied()
                .filter(|setting| setting.scope() == scope)
                .collect(),
        )
    })
}

pub async fn write_all(
    api: &Api,
    device: &DeviceDescriptor,
    content_id: Option<&str>,
    settings: &[Setting],
) -> Result<()> {
    for (scope, settings) in by_scope(settings) {
        if settings.is_empty() {
            continue;
        }
        match scope {
            Scope::User => write_user(api, &settings).await?,
            Scope::Device => write_device(api, device, &settings).await?,
            Scope::Content => {
                let content_id =
                    content_id.context("Content settings need a show or audiobook.")?;
                write_content(api, content_id, &settings).await?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    User {
        settings: Vec<Setting>,
        complete: bool,
    },
    Device {
        device_id: String,
        settings: Vec<Setting>,
        complete: bool,
    },
    Content {
        content_id: String,
        settings: Vec<Setting>,
        complete: bool,
    },
    Unknown,
}

impl Change {
    pub fn concerns_device(&self, device_id: &str) -> bool {
        match self {
            Self::Device { device_id: id, .. } => id.is_empty() || id == device_id,
            _ => true,
        }
    }
}

fn changed(fields: &[Field]) -> (Vec<Setting>, bool) {
    let mut complete = true;
    let mut settings = Vec::new();
    for field in fields {
        let tracked = REQUESTED.iter().any(|id| *id as i32 == field.id);
        match Setting::from_field(field) {
            Some(setting) => settings.push(setting),
            None if tracked => complete = false,
            None => {}
        }
    }
    (settings, complete)
}

pub fn is_settings_push(uri: &str) -> bool {
    uri.contains("playback-settings/")
}

pub fn change_from(message: &Message) -> Option<Change> {
    if !is_settings_push(&message.uri) {
        return None;
    }
    let Some(bytes) = message.bytes() else {
        return Some(Change::Unknown);
    };
    let change = if message.uri.contains("user-settings-update") {
        UserSettingsChangedEvent::decode(bytes.as_slice())
            .ok()
            .map(|event| {
                let (settings, complete) = changed(&event.changed_fields);
                Change::User { settings, complete }
            })
    } else if message.uri.contains("device-settings-fields-update") {
        DeviceSettingsFieldsChangedEvent::decode(bytes.as_slice())
            .ok()
            .map(|event| {
                let (settings, complete) = changed(&event.changed_fields);
                Change::Device {
                    device_id: event.device_id,
                    settings,
                    complete,
                }
            })
    } else if message.uri.contains("content-settings-update") {
        ContentSettingsChangedEvent::decode(bytes.as_slice())
            .ok()
            .map(|event| {
                let (settings, complete) = changed(&event.changed_fields);
                Change::Content {
                    content_id: event.content_id,
                    settings,
                    complete,
                }
            })
    } else {
        None
    };
    Some(change.unwrap_or(Change::Unknown))
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use spotify_proto::playbacksettings::{ContentSettings, DeviceSettings, UserSettings};

    use super::*;

    const SHOW: &str = "spotify:show:0000000000000000000005";

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    fn unhex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
            .collect()
    }

    fn push(uri: &str, bytes: &[u8]) -> Message {
        Message {
            uri: uri.into(),
            headers: HashMap::new(),
            payloads: vec![serde_json::Value::String(
                base64::engine::general_purpose::STANDARD.encode(bytes),
            )],
        }
    }

    #[test]
    fn asks_for_the_audio_fields_and_user_settings() {
        let request = selection_request(None, &[]);
        assert_eq!(hex(&request.encode_to_vec()), "120603090a0b0c0d2801");
        let device = descriptor("6b95c493", "OpenMusic Dev");
        let request = selection_request(Some(&device), &[SHOW.into()]);
        let decoded =
            GetSettingsDeviceSelectionRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        let sent = decoded.device_descriptor.unwrap();
        assert_eq!(sent.device_id, "6b95c493");
        assert_eq!(sent.device_name, "OpenMusic Dev");
        assert_eq!(sent.client_id, oauth::DESKTOP_CLIENT_ID);
        assert_eq!(sent.device_model, DEVICE_MODEL);
        assert_eq!(decoded.content_identifiers, vec![SHOW.to_owned()]);
        assert_eq!(decoded.include_user_settings, Some(true));
    }

    #[test]
    fn reads_the_recorded_user_settings() {
        let response =
            GetSettingsDeviceSelectionResponse::decode(unhex("1a074801500158d00f").as_slice())
                .unwrap();
        let remote = parse_selection(&response);
        assert_eq!(
            remote.user,
            Some(UserAudio {
                gapless: true,
                crossfade: true,
                crossfade_ms: 2000
            })
        );
        assert_eq!(remote.device, None);
        assert!(remote.speeds.is_empty());
    }

    #[test]
    fn reads_device_and_show_settings() {
        let response = GetSettingsDeviceSelectionResponse {
            device_settings: Some(DeviceSettings {
                social_discoverable: true,
                normalize: true,
                normalize_level: 1,
                ..DeviceSettings::default()
            }),
            content_settings: HashMap::from([
                (
                    SHOW.to_owned(),
                    ContentSettings {
                        playback_speed: 1.5,
                        ..ContentSettings::default()
                    },
                ),
                (
                    "spotify:show:0000000000000000000000".to_owned(),
                    ContentSettings {
                        playback_speed: 9.,
                        ..ContentSettings::default()
                    },
                ),
            ]),
            user_settings: Some(UserSettings {
                gapless: false,
                crossfade: false,
                crossfade_duration_ms: 40_000,
                ..UserSettings::default()
            }),
        };
        let bytes = response.encode_to_vec();
        let remote =
            parse_selection(&GetSettingsDeviceSelectionResponse::decode(bytes.as_slice()).unwrap());
        assert_eq!(
            remote.device,
            Some(DeviceAudio {
                normalize: true,
                level: LoudnessLevel::Normal
            })
        );
        assert_eq!(remote.user.unwrap().crossfade_ms, MAX_CROSSFADE_MS);
        assert!(!remote.user.unwrap().gapless);
        assert_eq!(remote.speeds, HashMap::from([(SHOW.to_owned(), 1.5)]));
    }

    #[test]
    fn writes_user_settings_like_the_desktop() {
        let encode = |settings: &[Setting]| hex(&user_request(settings).unwrap().encode_to_vec());
        assert_eq!(encode(&[Setting::Crossfade(false)]), "0a06080a12021000");
        assert_eq!(encode(&[Setting::Crossfade(true)]), "0a06080a12021001");
        assert_eq!(encode(&[Setting::CrossfadeMs(6000)]), "0a07080b120318f02e");
        assert_eq!(encode(&[Setting::Gapless(true)]), "0a06080912021001");
        assert_eq!(
            encode(&[Setting::Crossfade(true), Setting::CrossfadeMs(6000)]),
            "0a06080a120210010a07080b120318f02e"
        );
        assert!(user_request(&[Setting::Normalize(true)]).is_err());
        assert!(user_request(&[]).is_err());
        let [user, device, content] = by_scope(&[
            Setting::Gapless(true),
            Setting::Level(LoudnessLevel::Loud),
            Setting::CrossfadeMs(1000),
            Setting::Speed(1.5),
        ]);
        assert_eq!(
            user,
            (
                Scope::User,
                vec![Setting::Gapless(true), Setting::CrossfadeMs(1000)]
            )
        );
        assert_eq!(
            device,
            (Scope::Device, vec![Setting::Level(LoudnessLevel::Loud)])
        );
        assert_eq!(content, (Scope::Content, vec![Setting::Speed(1.5)]));
    }

    #[test]
    fn writes_show_speed_and_device_values() {
        let request = content_request(SHOW, &[Setting::Speed(1.5)]).unwrap();
        let expected = format!("0a23{}120908031205250000c03f", hex(SHOW.as_bytes()));
        assert_eq!(hex(&request.encode_to_vec()), expected);
        assert!(content_request("", &[Setting::Speed(1.5)]).is_err());
        assert!(content_request(SHOW, &[Setting::Gapless(true)]).is_err());

        let device = descriptor("6b95c493", "OpenMusic Dev");
        let request = device_request(
            &device,
            &[
                Setting::Normalize(false),
                Setting::Level(LoudnessLevel::Loud),
            ],
        )
        .unwrap();
        let decoded = WriteDeviceValueRequest::decode(request.encode_to_vec().as_slice()).unwrap();
        assert_eq!(decoded.device_descriptor, Some(device));
        assert_eq!(
            decoded
                .field
                .iter()
                .filter_map(Setting::from_field)
                .collect::<Vec<_>>(),
            vec![
                Setting::Normalize(false),
                Setting::Level(LoudnessLevel::Loud)
            ]
        );
        assert_eq!(
            hex(&Setting::Level(LoudnessLevel::Quiet).field().encode_to_vec()),
            "080d12021800"
        );
    }

    #[test]
    fn maps_loudness_levels_and_speeds() {
        for level in LoudnessLevel::ALL {
            assert_eq!(LoudnessLevel::from_index(level.index()), level);
            assert_eq!(LoudnessLevel::parse(level.name()), Some(level));
        }
        assert_eq!(LoudnessLevel::from_index(7), LoudnessLevel::Normal);
        assert_eq!(LoudnessLevel::parse("LOUD"), Some(LoudnessLevel::Loud));
        assert_eq!(LoudnessLevel::parse("max"), None);
        assert_eq!(valid_speed(0.5), Some(0.5));
        assert_eq!(valid_speed(3.5), Some(3.5));
        assert_eq!(valid_speed(3.6), None);
        assert_eq!(valid_speed(f32::NAN), None);
        assert_eq!(round_speed(1.26), 1.25);
        assert_eq!(round_speed(1.0), 1.0);
    }

    #[test]
    fn decodes_settings_pushes() {
        let user = UserSettingsChangedEvent {
            changed_fields: vec![
                Setting::Crossfade(true).field(),
                Setting::CrossfadeMs(6000).field(),
                Field {
                    id: FieldId::DisableSmartShuffle as i32,
                    value: None,
                },
            ],
        };
        assert_eq!(
            change_from(&push(
                "hm://playback-settings/user-settings-update",
                &user.encode_to_vec()
            )),
            Some(Change::User {
                settings: vec![Setting::Crossfade(true), Setting::CrossfadeMs(6000)],
                complete: true
            })
        );

        let partial = UserSettingsChangedEvent {
            changed_fields: vec![Field {
                id: FieldId::Gapless as i32,
                value: None,
            }],
        };
        assert_eq!(
            change_from(&push(
                "playback-settings/user-settings-update",
                &partial.encode_to_vec()
            )),
            Some(Change::User {
                settings: Vec::new(),
                complete: false
            })
        );

        let device = DeviceSettingsFieldsChangedEvent {
            device_id: "6b95c493".into(),
            device_name: "OpenMusic Dev".into(),
            changed_fields: vec![Setting::Level(LoudnessLevel::Quiet).field()],
        };
        let change = change_from(&push(
            "hm://playback-settings/device-settings-fields-update",
            &device.encode_to_vec(),
        ))
        .unwrap();
        assert_eq!(
            change,
            Change::Device {
                device_id: "6b95c493".into(),
                settings: vec![Setting::Level(LoudnessLevel::Quiet)],
                complete: true
            }
        );
        assert!(change.concerns_device("6b95c493"));
        assert!(!change.concerns_device("ffffffff"));

        let content = ContentSettingsChangedEvent {
            content_id: SHOW.into(),
            changed_fields: vec![Setting::Speed(2.).field()],
        };
        assert_eq!(
            change_from(&push(
                "hm://playback-settings/content-settings-update",
                &content.encode_to_vec()
            )),
            Some(Change::Content {
                content_id: SHOW.into(),
                settings: vec![Setting::Speed(2.)],
                complete: true
            })
        );

        assert_eq!(
            change_from(&push("hm://playback-settings/something-new", &[1, 2])),
            Some(Change::Unknown)
        );
        assert_eq!(
            change_from(&push(
                "hm://playback-settings/user-settings-update",
                &[0xff, 0xff]
            )),
            Some(Change::Unknown)
        );
        assert_eq!(
            change_from(&push("hm://collection/collection/someone/json", &[])),
            None
        );
    }
}
