use std::collections::HashSet;

use anyhow::{Result, anyhow, bail};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::{
    api::{Api, Body, HttpStatus},
    catalog,
    connect::{self, CommandError},
    dealer::Message,
    entity,
    pathfinder::{self, Operation},
};

pub const UPDATE_SUFFIX: &str = "social-connect/v2/session_update";
pub const INVITE_URL: &str = "https://open.spotify.com/socialsession/";
pub const JOIN_URI: &str = "spotify:socialsession:";
pub const DECORATE_QUEUED_BY_USERS: Operation = Operation::query(
    "decorateQueuedByUsers",
    "33fe7085b4a9dde404955eeee4f8f93aa7a8e55028058354bc707727d8a7890d",
);
pub const ENDING: [&str; 4] = [
    "SESSION_DELETED",
    "YOU_LEFT",
    "YOU_WERE_KICKED",
    "YOU_WERE_REMOVED_UNSUPPORTED_CONTENT",
];
pub const JOINING: [&str; 2] = ["YOU_JOINED", "NEW_SESSION"];
const REASONS: [&str; 21] = [
    "UNKNOWN_UPDATE_TYPE",
    "NEW_SESSION",
    "USER_JOINED",
    "USER_LEFT",
    "SESSION_DELETED",
    "YOU_LEFT",
    "YOU_WERE_KICKED",
    "YOU_JOINED",
    "",
    "",
    "USER_KICKED",
    "VOLUME_CONTROL_PERMISSIONS_CHANGED",
    "QUEUE_ONLY_MODE_CONTROL_CHANGED",
    "WIFI_BROADCAST_CHANGED",
    "ACTIVE_DEVICE_CHANGED",
    "",
    "SESSION_MEMBER_UPDATED",
    "SESSION_ACTIVATED",
    "SESSION_MEMBER_PAUSE_CHANGED",
    "JAM_CAPPING_STATE_CHANGED",
    "YOU_WERE_REMOVED_UNSUPPORTED_CONTENT",
];
const SESSIONS: &str = "/social-connect/v2/sessions";
const SESSIONS_V3: &str = "/social-connect/v3/sessions";
const LEAVE_REASON: &str = "user_left";
const AVATAR_WIDTH: u32 = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VolumeControl {
    Enabled,
    Disabled,
    #[default]
    Unavailable,
}

impl VolumeControl {
    pub fn parse(text: &str) -> Self {
        match text.to_ascii_uppercase().as_str() {
            "ENABLED" => Self::Enabled,
            "DISABLED" => Self::Disabled,
            _ => Self::Unavailable,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    pub username: String,
    pub name: String,
    pub image: Option<String>,
    pub current_user: bool,
    pub listening_along: bool,
    pub anonymous: bool,
}

impl Member {
    pub fn display_name(&self) -> String {
        if self.anonymous {
            return "Guest".to_owned();
        }
        [&self.name, &self.username]
            .into_iter()
            .find(|name| !name.is_empty())
            .cloned()
            .unwrap_or_else(|| "Guest".to_owned())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Session {
    pub id: String,
    pub timestamp: i64,
    pub owner_id: String,
    pub is_owner: bool,
    pub active: bool,
    pub join_token: String,
    pub join_uri: String,
    pub members: Vec<Member>,
    pub queue_only: bool,
    pub volume_control: VolumeControl,
    pub redirect_commands: bool,
    pub paused: bool,
    pub jam_paused: bool,
    pub host_device_id: String,
    pub host_device_name: String,
    pub can_invite: bool,
    pub can_remove: bool,
    pub can_manage: bool,
}

impl Session {
    pub fn host(&self) -> Option<&Member> {
        self.members
            .iter()
            .find(|member| member.id == self.owner_id)
    }

    pub fn host_name(&self) -> String {
        self.host()
            .map(Member::display_name)
            .unwrap_or_else(|| "The host".to_owned())
    }

    pub fn me(&self) -> Option<&Member> {
        self.members.iter().find(|member| member.current_user)
    }

    pub fn guests(&self) -> impl Iterator<Item = &Member> {
        self.members
            .iter()
            .filter(|member| member.id != self.owner_id)
    }

    pub fn member(&self, username: &str) -> Option<&Member> {
        self.members
            .iter()
            .find(|member| !member.username.is_empty() && member.username == username)
    }

    pub fn anonymous_count(&self) -> usize {
        self.members
            .iter()
            .filter(|member| member.anonymous)
            .count()
    }

    pub fn invite_url(&self) -> Option<String> {
        token_from_uri(&self.join_uri)
            .or_else(|| (!self.join_token.is_empty()).then(|| self.join_token.clone()))
            .map(|token| format!("{INVITE_URL}{token}"))
    }
}

fn field<'a>(value: &'a Value, names: &[&str]) -> &'a Value {
    names
        .iter()
        .map(|name| &value[*name])
        .find(|found| !found.is_null())
        .unwrap_or(&Value::Null)
}

fn text(value: &Value, names: &[&str]) -> String {
    match field(value, names) {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => String::new(),
    }
}

fn flag(value: &Value, names: &[&str]) -> bool {
    match field(value, names) {
        Value::Bool(on) => *on,
        Value::String(text) => text == "true",
        _ => false,
    }
}

fn number(value: &Value, names: &[&str]) -> i64 {
    match field(value, names) {
        Value::Number(number) => number.as_i64().unwrap_or_default(),
        Value::String(text) => text.trim().parse().unwrap_or_default(),
        _ => 0,
    }
}

fn filled(text: String) -> Option<String> {
    (!text.is_empty()).then_some(text)
}

fn member(value: &Value) -> Member {
    Member {
        id: text(value, &["id"]),
        username: text(value, &["username"]),
        name: text(value, &["display_name", "displayName"]),
        image: filled(text(value, &["image_url", "imageUrl"]))
            .or_else(|| filled(text(value, &["large_image_url", "largeImageUrl"]))),
        current_user: flag(value, &["is_current_user", "isCurrentUser"]),
        listening_along: flag(value, &["listen_mode", "listenMode"]),
        anonymous: flag(value, &["is_anonymous", "isAnonymous"]),
    }
}

pub fn parse(value: &Value) -> Option<Session> {
    let id = text(value, &["session_id", "sessionId"]);
    if id.is_empty() {
        return None;
    }
    let is_owner = flag(value, &["is_session_owner", "isSessionOwner"]);
    let capabilities = field(value, &["user_capabilities", "userCapabilities"]);
    let (can_invite, can_remove, can_manage) = if capabilities.is_object() {
        (
            flag(capabilities, &["invite"]),
            flag(capabilities, &["remove_participants", "removeParticipants"]),
            flag(
                capabilities,
                &["manage_participant_settings", "manageParticipantSettings"],
            ),
        )
    } else {
        (true, is_owner, is_owner)
    };
    let host = field(value, &["host_device_info", "hostDeviceInfo"]);
    let members = field(value, &["session_members", "sessionMembers"])
        .as_array()
        .into_iter()
        .flatten()
        .map(member)
        .collect();
    Some(Session {
        id,
        timestamp: number(value, &["timestamp"]),
        owner_id: text(value, &["session_owner_id", "sessionOwnerId"]),
        is_owner,
        active: flag(value, &["active"]),
        join_token: text(value, &["join_session_token", "joinSessionToken"]),
        join_uri: text(value, &["join_session_uri", "joinSessionUri"]),
        members,
        queue_only: flag(value, &["queue_only_mode", "queueOnlyMode"]),
        volume_control: VolumeControl::parse(&text(
            value,
            &["participantVolumeControl", "participant_volume_control"],
        )),
        redirect_commands: flag(value, &["redirectCommands", "redirect_commands"]),
        paused: flag(value, &["is_paused", "isPaused"]),
        jam_paused: flag(value, &["is_jam_paused", "isJamPaused"]),
        host_device_id: text(value, &["hostActiveDeviceId", "host_active_device_id"]),
        host_device_name: text(host, &["device_name", "deviceName"]),
        can_invite,
        can_remove,
        can_manage,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionUpdate {
    pub reason: String,
    pub session_id: String,
    pub timestamp: i64,
    pub session: Option<Session>,
}

impl SessionUpdate {
    pub fn ends(&self) -> bool {
        self.session.is_none()
    }
}

fn reason(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number
            .as_u64()
            .and_then(|index| REASONS.get(index as usize))
            .filter(|name| !name.is_empty())
            .unwrap_or(&REASONS[0])
            .to_string(),
        _ => REASONS[0].to_owned(),
    }
}

pub fn parse_update(payload: &Value) -> Option<SessionUpdate> {
    if !payload.is_object() {
        return None;
    }
    let raw = &payload["session"];
    let reason = reason(&payload["reason"]);
    let ending = ENDING.contains(&reason.as_str());
    Some(SessionUpdate {
        session_id: text(raw, &["session_id", "sessionId"]),
        timestamp: number(raw, &["timestamp"]),
        session: if ending { None } else { parse(raw) },
        reason,
    })
}

pub fn update(message: &Message) -> Option<SessionUpdate> {
    if !message.uri.ends_with(UPDATE_SUFFIX) {
        return None;
    }
    let payload = match message.json() {
        Some(payload) => payload.clone(),
        None => serde_json::from_slice(&message.bytes()?).ok()?,
    };
    parse_update(&payload)
}

pub fn accepts(current: Option<&Session>, update: &SessionUpdate, departed: Option<i64>) -> bool {
    if let Some(left_at) = departed {
        let rejoined = JOINING.contains(&update.reason.as_str()) && update.timestamp > left_at;
        if !rejoined {
            return false;
        }
    }
    match current {
        Some(current) if current.id == update.session_id => {
            update.timestamp <= 0 || update.timestamp >= current.timestamp
        }
        Some(_) => update.session.is_some() && JOINING.contains(&update.reason.as_str()),
        None => update.session.is_some(),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Started,
    Joined(Vec<String>),
    Left(Vec<String>),
    Ended { host: String },
    YouLeft,
    Kicked,
    Removed,
    QueueOnly(bool),
    VolumeControl(VolumeControl),
}

fn names(members: &[&Member]) -> Vec<String> {
    members.iter().map(|member| member.display_name()).collect()
}

pub fn changes(old: Option<&Session>, new: Option<&Session>, reason: &str) -> Vec<Change> {
    match (old, new) {
        (None, None) => Vec::new(),
        (Some(old), None) => vec![match reason {
            "YOU_WERE_KICKED" => Change::Kicked,
            "YOU_WERE_REMOVED_UNSUPPORTED_CONTENT" => Change::Removed,
            "SESSION_DELETED" => Change::Ended {
                host: old.host_name(),
            },
            _ => Change::YouLeft,
        }],
        (None, Some(_)) => vec![Change::Started],
        (Some(old), Some(new)) if old.id != new.id => vec![Change::Started],
        (Some(old), Some(new)) => {
            let known: HashSet<&str> = old
                .members
                .iter()
                .map(|member| member.id.as_str())
                .collect();
            let current: HashSet<&str> = new
                .members
                .iter()
                .map(|member| member.id.as_str())
                .collect();
            let joined: Vec<&Member> = new
                .members
                .iter()
                .filter(|member| !member.current_user && !known.contains(member.id.as_str()))
                .collect();
            let left: Vec<&Member> = old
                .members
                .iter()
                .filter(|member| !member.current_user && !current.contains(member.id.as_str()))
                .collect();
            let mut changes = Vec::new();
            if !joined.is_empty() {
                changes.push(Change::Joined(names(&joined)));
            }
            if !left.is_empty() {
                changes.push(Change::Left(names(&left)));
            }
            if old.queue_only != new.queue_only {
                changes.push(Change::QueueOnly(new.queue_only));
            }
            if old.volume_control != new.volume_control {
                changes.push(Change::VolumeControl(new.volume_control));
            }
            changes
        }
    }
}

fn group(names: &[String], two: &str, many: &str) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} + {second}{two}"),
        [first, rest @ ..] => format!("{first} + {} others{many}", rest.len()),
    }
}

impl Change {
    pub fn message(&self, owner: bool) -> Option<String> {
        Some(match self {
            Self::Started | Self::YouLeft => return None,
            Self::Joined(names) if names.is_empty() => return None,
            Self::Joined(names) => format!("{} joined your Jam", group(names, "", "")),
            Self::Left(names) if names.is_empty() => return None,
            Self::Left(names) if names.len() > 2 => {
                format!("{} left your Jam", group(names, "", ""))
            }
            Self::Left(names) => format!("{} left the Jam", group(names, "", "")),
            Self::Ended { .. } if owner => return None,
            Self::Ended { host } => format!("{host} ended the Jam"),
            Self::Kicked => "The host removed you from the Jam.".to_owned(),
            Self::Removed => {
                "You left the Jam because it’s playing something you can’t play.".to_owned()
            }
            Self::QueueOnly(true) if owner => {
                "Only you can control what’s playing now. Guests can still add to the queue."
                    .to_owned()
            }
            Self::QueueOnly(true) => {
                "Only the host can control what’s playing now. You can still add to the queue."
                    .to_owned()
            }
            Self::QueueOnly(false) if owner => {
                "Everyone can now control what’s playing and add to the queue.".to_owned()
            }
            Self::QueueOnly(false) => {
                "You can now control what’s playing and add to the queue.".to_owned()
            }
            Self::VolumeControl(VolumeControl::Enabled) => {
                "Everyone on this speaker can now change its volume.".to_owned()
            }
            Self::VolumeControl(VolumeControl::Disabled) if owner => {
                "Only you can change the speaker volume for this session.".to_owned()
            }
            Self::VolumeControl(VolumeControl::Disabled) => {
                "Only the host can change the speaker volume.".to_owned()
            }
            Self::VolumeControl(VolumeControl::Unavailable) => return None,
        })
    }
}

pub fn token_from_uri(uri: &str) -> Option<String> {
    uri.strip_prefix(JOIN_URI)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
}

pub fn token(input: &str) -> Option<String> {
    catalog::parse_link(input)
        .ok()
        .flatten()
        .as_deref()
        .and_then(token_from_uri)
}

pub fn invite_uri(token: &str) -> String {
    format!("{JOIN_URI}{token}")
}

fn segment(text: &str) -> String {
    text.bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum JoinMode {
    #[default]
    Remote,
    InPerson,
}

impl JoinMode {
    pub fn playback_control(self) -> &'static str {
        match self {
            Self::Remote => "listen_and_control",
            Self::InPerson => "control",
        }
    }
}

pub fn current_path() -> String {
    format!("{SESSIONS}/current")
}

pub fn info_path(token: &str) -> String {
    format!("{SESSIONS}/info/{}", segment(token))
}

pub fn start_path(device_id: &str) -> String {
    format!(
        "{SESSIONS}/current_or_new?activate=true&local_device_id={}",
        segment(device_id)
    )
}

pub fn join_path(token: &str, mode: JoinMode, device_id: &str) -> String {
    format!(
        "{SESSIONS}/join/{}?join_type=deeplinking&playback_control={}&local_device_id={}",
        segment(token),
        mode.playback_control(),
        segment(device_id)
    )
}

pub fn leave_path(session_id: &str) -> String {
    format!(
        "{SESSIONS}/{}/leave?reason={LEAVE_REASON}",
        segment(session_id)
    )
}

pub fn end_path(session_id: &str) -> String {
    format!("{SESSIONS_V3}/{}", segment(session_id))
}

pub fn remove_path(session_id: &str, member_id: &str) -> String {
    format!(
        "{SESSIONS_V3}/{}/member/{}/kick",
        segment(session_id),
        segment(member_id)
    )
}

pub fn queue_only_path(on: bool) -> String {
    format!("{SESSIONS}/current/queue_only_mode/{on}")
}

pub fn volume_control_path(on: bool) -> String {
    format!("{SESSIONS}/current/volume_control/{on}")
}

pub fn redirect_path(session_id: &str, from: &str) -> String {
    format!(
        "{SESSIONS}/{}/commands/from/{}",
        segment(session_id),
        segment(from)
    )
}

pub fn redirect_body(command: Value) -> Value {
    connect::envelope(command, &connect::command_id(), None)
}

fn gone(status: StatusCode) -> bool {
    status == StatusCode::NOT_FOUND || status == StatusCode::GONE
}

pub fn session_reply(status: StatusCode, body: &[u8]) -> Result<Option<Session>> {
    if gone(status) {
        return Ok(None);
    }
    if !status.is_success() {
        bail!(HttpStatus(status));
    }
    Ok(serde_json::from_slice::<Value>(body)
        .ok()
        .as_ref()
        .and_then(parse))
}

pub fn done_reply(status: StatusCode) -> Result<bool> {
    if gone(status) {
        return Ok(false);
    }
    if !status.is_success() {
        bail!(HttpStatus(status));
    }
    Ok(true)
}

pub fn redirect_reply(status: StatusCode, body: &[u8]) -> Result<bool> {
    if status == StatusCode::FORBIDDEN {
        bail!(CommandError::Refused(
            String::from_utf8_lossy(body).into_owned()
        ));
    }
    done_reply(status)
}

async fn call(api: &Api, method: Method, path: &str) -> Result<(StatusCode, Vec<u8>)> {
    api.send(method, path, Body::Empty, "application/json")
        .await
}

pub async fn current(api: &Api) -> Result<Option<Session>> {
    let (status, body) = call(api, Method::GET, &current_path()).await?;
    session_reply(status, &body)
}

pub async fn info(api: &Api, token: &str) -> Result<Option<Session>> {
    let (status, body) = call(api, Method::GET, &info_path(token)).await?;
    session_reply(status, &body)
}

pub async fn start(api: &Api, device_id: &str) -> Result<Session> {
    let path = start_path(device_id);
    let (mut status, mut body) = call(api, Method::GET, &path).await?;
    if status == StatusCode::METHOD_NOT_ALLOWED {
        (status, body) = call(api, Method::POST, &path).await?;
    }
    session_reply(status, &body)?.ok_or_else(|| anyhow!("Spotify didn’t start the Jam."))
}

pub async fn join(api: &Api, token: &str, mode: JoinMode, device_id: &str) -> Result<Session> {
    let (status, body) = call(api, Method::POST, &join_path(token, mode, device_id)).await?;
    session_reply(status, &body)?.ok_or_else(|| anyhow!("This Jam has ended."))
}

pub async fn leave(api: &Api, session_id: &str) -> Result<bool> {
    let (status, _) = call(api, Method::POST, &leave_path(session_id)).await?;
    done_reply(status)
}

pub async fn end(api: &Api, session_id: &str) -> Result<bool> {
    let (status, _) = call(api, Method::DELETE, &end_path(session_id)).await?;
    done_reply(status)
}

pub async fn remove(api: &Api, session_id: &str, member_id: &str) -> Result<Option<Session>> {
    let (status, body) = call(api, Method::POST, &remove_path(session_id, member_id)).await?;
    session_reply(status, &body)
}

pub async fn set_queue_only(api: &Api, on: bool) -> Result<Option<Session>> {
    let (status, body) = call(api, Method::PUT, &queue_only_path(on)).await?;
    session_reply(status, &body)
}

pub async fn set_volume_control(api: &Api, on: bool) -> Result<Option<Session>> {
    let (status, body) = call(api, Method::PUT, &volume_control_path(on)).await?;
    session_reply(status, &body)
}

pub async fn redirect(api: &Api, session_id: &str, from: &str, command: Value) -> Result<bool> {
    let (status, body) = api
        .send(
            Method::POST,
            &redirect_path(session_id, from),
            Body::Json(redirect_body(command)),
            "application/json",
        )
        .await?;
    redirect_reply(status, &body)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    pub username: String,
    pub name: String,
    pub image: Option<String>,
}

pub fn user_uri(username: &str) -> String {
    format!("spotify:user:{username}")
}

pub fn people_variables(usernames: &[String]) -> Value {
    json!({ "userUris": usernames.iter().map(|username| user_uri(username)).collect::<Vec<_>>() })
}

pub fn parse_people(data: &Value) -> Vec<Person> {
    data["users"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|user| user["__typename"] == "User")
        .filter_map(|user| {
            let username = text(user, &["username"]);
            if username.is_empty() {
                return None;
            }
            Some(Person {
                name: filled(text(user, &["name"])).unwrap_or_else(|| username.clone()),
                image: entity::pick_image(&user["avatar"]["sources"], AVATAR_WIDTH),
                username,
            })
        })
        .collect()
}

pub async fn people(api: &Api, usernames: &[String]) -> Result<Vec<Person>> {
    if usernames.is_empty() {
        return Ok(Vec::new());
    }
    let data =
        pathfinder::query(api, DECORATE_QUEUED_BY_USERS, people_variables(usernames)).await?;
    Ok(parse_people(&data))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use base64::Engine as _;

    use super::*;

    fn fixture() -> Value {
        json!({
            "timestamp": "1764000000000",
            "session_id": "session-one",
            "join_session_token": "jamtoken123",
            "join_session_uri": "spotify:socialsession:jamtoken123",
            "session_owner_id": "owner-id",
            "is_session_owner": true,
            "active": true,
            "queue_only_mode": false,
            "participantVolumeControl": "ENABLED",
            "wifi_broadcast": false,
            "is_paused": false,
            "is_jam_paused": false,
            "hostActiveDeviceId": "aaaa0000aaaa0000aaaa0000aaaa0000aaaa0000",
            "host_device_info": {"device_name": "Kitchen", "device_type": "SPEAKER"},
            "redirectCommands": false,
            "user_capabilities": {"invite": true, "remove_participants": true, "manage_participant_settings": true, "ask_to_upgrade": false},
            "session_members": [
                {"id": "owner-id", "username": "someone", "display_name": "Host Person", "image_url": "https://i.scdn.co/image/ab67757000003b82host", "is_current_user": true, "listen_mode": false, "playback_control": "LISTEN_AND_CONTROL"},
                {"id": "guest-id", "username": "friend", "display_name": "Guest Person", "image_url": "", "large_image_url": "https://i.scdn.co/image/ab6775700000ee85guest", "is_current_user": false, "listen_mode": true},
                {"id": "anon-id", "is_anonymous": true}
            ]
        })
    }

    #[test]
    fn parses_a_session() {
        let session = parse(&fixture()).unwrap();
        assert_eq!(session.id, "session-one");
        assert_eq!(session.timestamp, 1_764_000_000_000);
        assert!(session.is_owner && session.active);
        assert_eq!(session.volume_control, VolumeControl::Enabled);
        assert_eq!(session.host_device_name, "Kitchen");
        assert!(session.can_invite && session.can_remove && session.can_manage);
        assert!(!session.redirect_commands);
        assert_eq!(session.members.len(), 3);
        assert_eq!(session.host_name(), "Host Person");
        assert_eq!(session.me().map(|me| me.id.as_str()), Some("owner-id"));
        let guest = &session.members[1];
        assert!(guest.listening_along);
        assert_eq!(
            guest.image.as_deref(),
            Some("https://i.scdn.co/image/ab6775700000ee85guest")
        );
        assert_eq!(session.members[2].display_name(), "Guest");
        assert_eq!(session.anonymous_count(), 1);
        assert_eq!(session.guests().count(), 2);
        assert_eq!(
            session.member("friend").map(|member| member.id.as_str()),
            Some("guest-id")
        );
        assert_eq!(
            session.invite_url().as_deref(),
            Some("https://open.spotify.com/socialsession/jamtoken123")
        );
    }

    #[test]
    fn tolerates_missing_fields() {
        assert_eq!(parse(&json!({"timestamp": 1})), None);
        let info = parse(&json!({
            "session_id": "s",
            "session_owner_id": "o",
            "session_members": [{"id": "o", "username": "host"}],
            "is_session_owner": false,
            "timestamp": 12,
            "redirect_commands": true
        }))
        .unwrap();
        assert!(!info.active);
        assert!(info.can_invite && !info.can_remove && !info.can_manage);
        assert_eq!(info.volume_control, VolumeControl::Unavailable);
        assert_eq!(info.host_name(), "host");
        assert_eq!(info.timestamp, 12);
        assert!(info.redirect_commands);
        assert_eq!(info.invite_url(), None);
        let camel = parse(&json!({
            "sessionId": "s",
            "isSessionOwner": true,
            "joinSessionToken": "tok",
            "userCapabilities": {"invite": false}
        }))
        .unwrap();
        assert!(camel.is_owner && !camel.can_invite && !camel.can_manage);
        assert_eq!(
            camel.invite_url().as_deref(),
            Some("https://open.spotify.com/socialsession/tok")
        );
    }

    fn message(uri: &str, payload: Value) -> Message {
        Message {
            uri: uri.into(),
            headers: HashMap::new(),
            payloads: vec![payload],
        }
    }

    #[test]
    fn reads_session_updates() {
        let pushed = message(
            "hm://social-connect/v2/session_update",
            json!({"reason": "USER_JOINED", "session": fixture()}),
        );
        let update = update(&pushed).unwrap();
        assert_eq!(update.reason, "USER_JOINED");
        assert_eq!(update.session_id, "session-one");
        assert!(!update.ends());
        let encoded = base64::engine::general_purpose::STANDARD.encode(
            serde_json::to_vec(
                &json!({"reason": 4, "session": {"session_id": "session-one", "timestamp": "9"}}),
            )
            .unwrap(),
        );
        let deleted = super::update(&message(
            "hm://social-connect/v2/session_update",
            Value::String(encoded),
        ))
        .unwrap();
        assert_eq!(deleted.reason, "SESSION_DELETED");
        assert!(deleted.ends());
        assert_eq!(deleted.timestamp, 9);
        assert_eq!(
            super::update(&message(
                "hm://collection/collection/someone/json",
                json!({})
            )),
            None
        );
        let bare = parse_update(&json!({"reason": "USER_LEFT"})).unwrap();
        assert!(bare.ends());
    }

    fn update_for(id: &str, timestamp: i64, reason: &str, live: bool) -> SessionUpdate {
        SessionUpdate {
            reason: reason.into(),
            session_id: id.into(),
            timestamp,
            session: live.then(|| Session {
                id: id.into(),
                timestamp,
                ..Session::default()
            }),
        }
    }

    #[test]
    fn orders_updates_by_timestamp_and_session() {
        let current = Session {
            id: "a".into(),
            timestamp: 100,
            ..Session::default()
        };
        assert!(accepts(
            Some(&current),
            &update_for("a", 101, "USER_JOINED", true),
            None
        ));
        assert!(accepts(
            Some(&current),
            &update_for("a", 100, "USER_JOINED", true),
            None
        ));
        assert!(!accepts(
            Some(&current),
            &update_for("a", 99, "USER_LEFT", true),
            None
        ));
        assert!(accepts(
            Some(&current),
            &update_for("a", 0, "SESSION_DELETED", false),
            None
        ));
        assert!(!accepts(
            Some(&current),
            &update_for("b", 200, "USER_JOINED", true),
            None
        ));
        assert!(!accepts(
            Some(&current),
            &update_for("b", 200, "SESSION_DELETED", false),
            None
        ));
        assert!(accepts(
            Some(&current),
            &update_for("b", 200, "YOU_JOINED", true),
            None
        ));
        assert!(accepts(
            None,
            &update_for("b", 1, "NEW_SESSION", true),
            None
        ));
        assert!(!accepts(None, &update_for("b", 1, "YOU_LEFT", false), None));
        assert!(!accepts(
            None,
            &update_for("a", 150, "USER_LEFT", true),
            Some(120)
        ));
        assert!(!accepts(
            None,
            &update_for("a", 110, "YOU_JOINED", true),
            Some(120)
        ));
        assert!(accepts(
            None,
            &update_for("a", 130, "YOU_JOINED", true),
            Some(120)
        ));
    }

    fn person(id: &str, name: &str) -> Member {
        Member {
            id: id.into(),
            name: name.into(),
            ..Member::default()
        }
    }

    #[test]
    fn diffs_members_into_changes() {
        let me = Member {
            current_user: true,
            ..person("me", "Me")
        };
        let old = Session {
            id: "s".into(),
            owner_id: "me".into(),
            members: vec![me.clone(), person("a", "Ann")],
            ..Session::default()
        };
        let new = Session {
            members: vec![
                me.clone(),
                person("b", "Ben"),
                person("c", "Cat"),
                person("d", "Dan"),
            ],
            queue_only: true,
            volume_control: VolumeControl::Disabled,
            ..old.clone()
        };
        let found = changes(Some(&old), Some(&new), "USER_JOINED");
        assert_eq!(
            found,
            vec![
                Change::Joined(vec!["Ben".into(), "Cat".into(), "Dan".into()]),
                Change::Left(vec!["Ann".into()]),
                Change::QueueOnly(true),
                Change::VolumeControl(VolumeControl::Disabled),
            ]
        );
        assert_eq!(
            found[0].message(true).as_deref(),
            Some("Ben + 2 others joined your Jam")
        );
        assert_eq!(found[1].message(true).as_deref(), Some("Ann left the Jam"));
        assert_eq!(
            Change::Joined(vec!["Ben".into(), "Cat".into()])
                .message(true)
                .as_deref(),
            Some("Ben + Cat joined your Jam")
        );
        assert_eq!(
            Change::Left(vec!["Ben".into(), "Cat".into(), "Dan".into()])
                .message(true)
                .as_deref(),
            Some("Ben + 2 others left your Jam")
        );
        assert!(
            found[2]
                .message(false)
                .unwrap()
                .starts_with("Only the host")
        );
        assert!(found[3].message(true).unwrap().starts_with("Only you"));
        assert_eq!(
            changes(Some(&old), None, "SESSION_DELETED"),
            vec![Change::Ended { host: "Me".into() }]
        );
        assert_eq!(
            Change::Ended { host: "Me".into() }
                .message(false)
                .as_deref(),
            Some("Me ended the Jam")
        );
        assert_eq!(Change::Ended { host: "Me".into() }.message(true), None);
        assert_eq!(
            changes(Some(&old), None, "YOU_WERE_KICKED"),
            vec![Change::Kicked]
        );
        assert_eq!(changes(Some(&old), None, "YOU_LEFT"), vec![Change::YouLeft]);
        assert_eq!(
            changes(None, Some(&new), "NEW_SESSION"),
            vec![Change::Started]
        );
        assert_eq!(changes(None, None, "YOU_LEFT"), Vec::<Change>::new());
        assert_eq!(changes(Some(&old), Some(&old), "USER_LEFT"), Vec::new());
    }

    #[test]
    fn builds_paths_and_bodies() {
        assert_eq!(current_path(), "/social-connect/v2/sessions/current");
        assert_eq!(
            info_path("tok en"),
            "/social-connect/v2/sessions/info/tok%20en"
        );
        assert_eq!(
            start_path("abc"),
            "/social-connect/v2/sessions/current_or_new?activate=true&local_device_id=abc"
        );
        assert_eq!(
            join_path("tok", JoinMode::Remote, "abc"),
            "/social-connect/v2/sessions/join/tok?join_type=deeplinking&playback_control=listen_and_control&local_device_id=abc"
        );
        assert!(join_path("tok", JoinMode::InPerson, "abc").contains("playback_control=control&"));
        assert_eq!(
            leave_path("s1"),
            "/social-connect/v2/sessions/s1/leave?reason=user_left"
        );
        assert_eq!(end_path("s1"), "/social-connect/v3/sessions/s1");
        assert_eq!(
            remove_path("s1", "m1"),
            "/social-connect/v3/sessions/s1/member/m1/kick"
        );
        assert_eq!(
            queue_only_path(true),
            "/social-connect/v2/sessions/current/queue_only_mode/true"
        );
        assert_eq!(
            volume_control_path(false),
            "/social-connect/v2/sessions/current/volume_control/false"
        );
        assert_eq!(
            redirect_path("s1", "dev"),
            "/social-connect/v2/sessions/s1/commands/from/dev"
        );
        let body = redirect_body(json!({"endpoint": "skip_next"}));
        assert_eq!(body["command"]["endpoint"], "skip_next");
        assert_eq!(
            body["command"]["logging_params"]["command_id"]
                .as_str()
                .map(str::len),
            Some(32)
        );
        assert!(body.get("target_alias_id").is_none());
    }

    #[test]
    fn maps_replies() {
        let body = serde_json::to_vec(&fixture()).unwrap();
        assert_eq!(
            session_reply(StatusCode::OK, &body)
                .unwrap()
                .map(|session| session.id),
            Some("session-one".into())
        );
        assert_eq!(session_reply(StatusCode::NOT_FOUND, b"").unwrap(), None);
        assert_eq!(session_reply(StatusCode::GONE, b"").unwrap(), None);
        assert_eq!(session_reply(StatusCode::OK, b"").unwrap(), None);
        assert!(session_reply(StatusCode::BAD_GATEWAY, b"").is_err());
        assert!(done_reply(StatusCode::NO_CONTENT).unwrap());
        assert!(!done_reply(StatusCode::GONE).unwrap());
        assert!(done_reply(StatusCode::INTERNAL_SERVER_ERROR).is_err());
        assert!(redirect_reply(StatusCode::OK, b"").unwrap());
        assert!(!redirect_reply(StatusCode::NOT_FOUND, b"").unwrap());
        let refused = redirect_reply(StatusCode::FORBIDDEN, b"no").unwrap_err();
        assert_eq!(
            refused.downcast_ref::<CommandError>(),
            Some(&CommandError::Refused("no".into()))
        );
    }

    #[test]
    fn reads_jam_links() {
        assert_eq!(
            token("https://open.spotify.com/socialsession/jamtoken123?si=x").as_deref(),
            Some("jamtoken123")
        );
        assert_eq!(
            token("spotify:socialsession:jamtoken123").as_deref(),
            Some("jamtoken123")
        );
        assert_eq!(token("spotify:track:4uLU6hMCjMI75M1A2tKUQC"), None);
        assert_eq!(token_from_uri("spotify:socialsession:"), None);
        assert_eq!(invite_uri("t"), "spotify:socialsession:t");
    }

    #[test]
    fn reads_people_for_added_by() {
        let data = json!({"users": [
            {"__typename": "User", "id": "x", "uri": "spotify:user:friend", "username": "friend", "name": "Guest Person", "socialHandle": null,
             "avatar": {"extractedColors": {"colorDark": {"hex": "#247BBC"}}, "sources": [
                {"url": "https://i.scdn.co/image/small", "width": 64, "height": 64},
                {"url": "https://i.scdn.co/image/large", "width": 300, "height": 300}]}},
            {"__typename": "User", "username": "plain", "name": "", "avatar": null},
            {"__typename": "NotFound"}
        ]});
        assert_eq!(
            parse_people(&data),
            vec![
                Person {
                    username: "friend".into(),
                    name: "Guest Person".into(),
                    image: Some("https://i.scdn.co/image/small".into()),
                },
                Person {
                    username: "plain".into(),
                    name: "plain".into(),
                    image: None,
                },
            ]
        );
        assert_eq!(
            people_variables(&["friend".into()]),
            json!({"userUris": ["spotify:user:friend"]})
        );
    }
}
