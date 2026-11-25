use std::collections::HashMap;

use anyhow::{Result, anyhow};
use reqwest::Method;
use serde_json::Value;

use crate::{
    api::{Api, Body},
    playlist,
};

pub const FORMATS: [&str; 2] = ["blend", "celebrity-blend"];
pub const GENERATE_PATH: &str = "/blend-invitation/v1/generate";
pub const TRACK_ATTRIBUTED: &str = "Listened to this song";
pub const ARTIST_ATTRIBUTED: &str = "Listened to this artist";
pub const USER_ADDED: &str = "Added this song";

const USERINFO: &str = "blend.userinfo-";
const MADE_FOR: &str = "madeFor.username";

pub fn is_blend(format: Option<&str>) -> bool {
    format.is_some_and(|format| FORMATS.contains(&format))
}

pub fn is_blend_list(metadata: &HashMap<String, String>) -> bool {
    is_blend(metadata.get("format_list_type").map(String::as_str))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Member {
    pub username: String,
    pub name: String,
    pub image: Option<String>,
}

impl Member {
    pub fn uri(&self) -> Option<String> {
        (!self.username.is_empty()).then(|| format!("spotify:user:{}", self.username))
    }

    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            &self.username
        } else {
            &self.name
        }
    }
}

fn filled(text: Option<&String>) -> Option<String> {
    text.map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

pub fn members(metadata: &HashMap<String, String>) -> Vec<Member> {
    let mut found: HashMap<&str, Member> = HashMap::new();
    for (key, value) in metadata {
        let Some((id, field)) = key
            .strip_prefix(USERINFO)
            .and_then(|rest| rest.rsplit_once('.'))
        else {
            continue;
        };
        if id.is_empty() {
            continue;
        }
        let member = found.entry(id).or_insert_with(|| Member {
            username: id.to_owned(),
            ..Member::default()
        });
        let value = filled(Some(value));
        match field {
            "username" => {
                if let Some(username) = value {
                    member.username = username;
                }
            }
            "name" => member.name = value.unwrap_or_default(),
            "profileimage" => member.image = value,
            _ => {}
        }
    }
    let me = metadata.get(MADE_FOR).map(String::as_str);
    let mut list: Vec<Member> = found.into_values().collect();
    list.sort_by(|a, b| {
        let mine = |member: &Member| Some(member.username.as_str()) != me;
        mine(a)
            .cmp(&mine(b))
            .then_with(|| a.label().to_lowercase().cmp(&b.label().to_lowercase()))
            .then_with(|| a.username.cmp(&b.username))
    });
    list
}

pub fn creators(members: &[Member]) -> String {
    match members {
        [] => String::new(),
        [only] => only.label().to_owned(),
        [first, second] => format!("{} and {}", first.label(), second.label()),
        [first, rest @ ..] => format!("{} and {} others", first.label(), rest.len()),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attribution {
    pub username: Option<String>,
    pub name: String,
    pub image: Option<String>,
    pub kind: String,
    pub description: Option<String>,
}

impl Attribution {
    pub fn uri(&self) -> Option<String> {
        self.username
            .as_ref()
            .map(|username| format!("spotify:user:{username}"))
    }

    pub fn label(&self) -> &str {
        if self.name.is_empty() {
            self.username.as_deref().unwrap_or_default()
        } else {
            &self.name
        }
    }

    pub fn reason(&self, list: &HashMap<String, String>) -> String {
        if let Some(description) = self.description.as_ref().filter(|text| !text.is_empty()) {
            return description.clone();
        }
        let (key, fallback) = if self.kind.contains("ARTIST") {
            ("artist_attributed_desc", ARTIST_ATTRIBUTED)
        } else if self.kind.contains("ADD") {
            ("user_added_desc", USER_ADDED)
        } else {
            ("track_attributed_desc", TRACK_ATTRIBUTED)
        };
        filled(list.get(key)).unwrap_or_else(|| fallback.to_owned())
    }
}

fn split(attributes: &HashMap<String, String>, key: &str) -> Option<Vec<String>> {
    attributes
        .get(key)
        .filter(|text| !text.is_empty())
        .map(|text| text.split(',').map(|part| part.trim().to_owned()).collect())
}

fn described(attributes: &HashMap<String, String>) -> HashMap<String, String> {
    let Some(metadata) = attributes
        .get("multiUserAttributionMetadata")
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
    else {
        return HashMap::new();
    };
    metadata["attributed_users"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|user| {
            Some((
                user["username"].as_str()?.to_owned(),
                user["description"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

pub fn attribution(
    attributes: &HashMap<String, String>,
    added_by: Option<&str>,
) -> Vec<Attribution> {
    let Some(names) = split(attributes, "multiUserAttributionDisplayNamesV2")
        .or_else(|| split(attributes, "multiUserAttributionDisplayNames"))
    else {
        return Vec::new();
    };
    let usernames = split(attributes, "multiUserAttributionUsernamesV2").unwrap_or_default();
    let images = split(attributes, "multiUserAttributionImages").unwrap_or_default();
    let kinds = split(attributes, "multiUserAttributionType").unwrap_or_default();
    let descriptions = described(attributes);
    let mut list: Vec<Attribution> = names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let username = usernames
                .get(index)
                .filter(|name| !name.is_empty())
                .cloned();
            Attribution {
                description: username
                    .as_ref()
                    .and_then(|username| descriptions.get(username))
                    .cloned(),
                username,
                name,
                image: images.get(index).filter(|url| url.len() > 1).cloned(),
                kind: kinds.get(index).cloned().unwrap_or_default(),
            }
        })
        .collect();
    if let Some(added_by) = added_by {
        list.sort_by_key(|attribution| attribution.username.as_deref() != Some(added_by));
    }
    list
}

pub fn attributions(items: &[playlist::PlaylistItem]) -> HashMap<String, Vec<Attribution>> {
    items
        .iter()
        .filter_map(|item| {
            let found = attribution(&item.attributes, item.added_by.as_deref());
            if found.is_empty() {
                return None;
            }
            Some((item.uid.clone().unwrap_or_else(|| item.uri.clone()), found))
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvitationState {
    Pending,
    ReadyEmpty,
    ReadyExisting,
    Full,
    Joined,
    Deleted,
    Other(String),
}

impl InvitationState {
    pub fn parse(text: &str) -> Self {
        match text {
            "PENDING_INVITATION" => Self::Pending,
            "READY_TO_JOIN_EMPTY_BLEND" => Self::ReadyEmpty,
            "READY_TO_JOIN_ALREADY_CREATED_BLEND" => Self::ReadyExisting,
            "MAX_MEMBERS" => Self::Full,
            "ALREADY_JOINED" => Self::Joined,
            "DELETED" => Self::Deleted,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn joins(&self) -> bool {
        matches!(self, Self::ReadyEmpty | Self::ReadyExisting)
    }

    pub fn invites(&self) -> bool {
        matches!(self, Self::Pending | Self::Full | Self::Deleted)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation {
    pub state: InvitationState,
    pub title: String,
    pub subtitle: String,
    pub footnote: Option<String>,
    pub button: Option<String>,
    pub members: Vec<Member>,
    pub members_title: Option<String>,
    pub recipient: Option<Member>,
    pub playlist_uri: Option<String>,
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn invite_member(value: &Value) -> Option<Member> {
    let username = text(&value["username"]).unwrap_or_default();
    let name = text(&value["name"]).unwrap_or_default();
    if username.is_empty() && name.is_empty() {
        return None;
    }
    Some(Member {
        username,
        name,
        image: text(&value["image_url"]),
    })
}

pub fn parse_invitation(value: &Value) -> Result<Invitation> {
    let state = value["page_type"]
        .as_str()
        .map(InvitationState::parse)
        .ok_or_else(|| anyhow!("Something went wrong."))?;
    Ok(Invitation {
        state,
        title: text(&value["title"]).unwrap_or_default(),
        subtitle: text(&value["subtitle"]).unwrap_or_default(),
        footnote: text(&value["footnote"]),
        button: text(&value["button_text"]),
        members: value["members"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(invite_member)
            .collect(),
        members_title: text(&value["members_title"]),
        recipient: invite_member(&value["recipient"]),
        playlist_uri: text(&value["playlist_uri"]).and_then(|uri| playlist::canonical(&uri)),
    })
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub fn invitation_id(input: &str) -> Option<String> {
    let input = input.trim();
    let path = match url::Url::parse(input) {
        Ok(url) if url.host_str() == Some("open.spotify.com") => url.path().to_owned(),
        Ok(_) => return None,
        Err(_) => input.to_owned(),
    };
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && !part.starts_with("intl-"))
        .collect();
    match parts.as_slice() {
        ["blend", "taste-match", id] if valid_id(id) => Some((*id).to_owned()),
        _ => None,
    }
}

fn component(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

pub fn invitation_path(id: &str, locale: &str) -> String {
    format!(
        "/blend-invitation/v3/view-invitation/{}?locale={}",
        component(id),
        component(locale)
    )
}

pub fn join_path(id: &str) -> String {
    format!("/blend-invitation/v2/join/{}", component(id))
}

pub fn parse_invite(value: &Value) -> Result<String> {
    text(&value["invite"]).ok_or_else(|| anyhow!("unable to generate invite link"))
}

pub fn parse_joined(value: &Value) -> Result<String> {
    text(&value["playlist_uri"])
        .and_then(|uri| playlist::canonical(&uri))
        .ok_or_else(|| anyhow!("unable to join"))
}

pub fn share_text(link: &str, inviter: Option<&str>) -> String {
    match inviter.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => format!(
            "{name} has invited you to join a Blend on Spotify.  Join on the Spotify mobile app. {link}"
        ),
        None => format!(
            "You have been invited to join a Blend on Spotify. Join on the Spotify mobile app. {link}"
        ),
    }
}

fn lenient(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap_or(Value::Null)
}

pub async fn invite_link(api: &Api) -> Result<String> {
    let bytes = api
        .request(Method::POST, GENERATE_PATH, Body::Empty, "application/json")
        .await?;
    parse_invite(&lenient(&bytes))
}

pub async fn invitation(api: &Api, id: &str) -> Result<Invitation> {
    let value: Value = api.get_json(&invitation_path(id, &api.language())).await?;
    parse_invitation(&value)
}

pub async fn join(api: &Api, id: &str) -> Result<String> {
    let bytes = api
        .request(Method::PUT, &join_path(id), Body::Empty, "application/json")
        .await?;
    parse_joined(&lenient(&bytes))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn context_metadata() -> HashMap<String, String> {
        [
            ("format_list_type", "blend"),
            ("context_owner", "spotify:user:spotify"),
            ("blend.userinfo-member-a.username", "member-a"),
            ("blend.userinfo-member-a.name", "Member A"),
            (
                "blend.userinfo-member-a.profileimage",
                "https://i.scdn.co/image/ab67757000000000000000a1",
            ),
            ("blend.userinfo-member-b.username", "member-b"),
            ("blend.userinfo-member-b.name", "Member B"),
            ("blend.userinfo-member-b.profileimage", ""),
            ("track_attributed_desc", "Listened to this song"),
            ("artist_attributed_desc", "Listened to this artist"),
            ("user_added_desc", "Added this song"),
            ("madeFor.username", "member-b"),
            ("madeFor.displayed", "1"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }

    #[test]
    fn finds_blend_members_with_the_listener_first() {
        let metadata = context_metadata();
        assert!(is_blend_list(&metadata));
        assert!(is_blend(Some("celebrity-blend")));
        assert!(!is_blend(Some("daily-mix")) && !is_blend(None));
        let found = members(&metadata);
        assert_eq!(
            found,
            vec![
                Member {
                    username: "member-b".into(),
                    name: "Member B".into(),
                    image: None,
                },
                Member {
                    username: "member-a".into(),
                    name: "Member A".into(),
                    image: Some("https://i.scdn.co/image/ab67757000000000000000a1".into()),
                },
            ]
        );
        assert_eq!(found[0].uri().as_deref(), Some("spotify:user:member-b"));
        assert_eq!(creators(&found), "Member B and Member A");
        let group: Vec<Member> = ["a", "b", "c", "d"]
            .into_iter()
            .map(|name| Member {
                username: name.into(),
                name: name.to_uppercase(),
                image: None,
            })
            .collect();
        assert_eq!(creators(&group), "A and 3 others");
        assert_eq!(creators(&group[..1]), "A");
        let unnamed = HashMap::from([(
            "blend.userinfo-member-c.username".to_owned(),
            "member-c".to_owned(),
        )]);
        assert_eq!(members(&unnamed)[0].label(), "member-c");
        assert!(members(&HashMap::new()).is_empty());
    }

    fn item_attributes() -> HashMap<String, String> {
        [
            ("multiUserAttributionType", "ARTIST_ATTRIBUTION,TRACK_ATTRIBUTION"),
            ("multiUserAttributionUsernamesV2", "member-b,member-a"),
            ("multiUserAttributionDisplayNamesV2", "Member B,Member A"),
            ("multiUserAttributionUsernames", "Member B,Member A"),
            (
                "multiUserAttributionImages",
                ",https://i.scdn.co/image/ab67757000000000000000a1",
            ),
            (
                "multiUserAttributionMetadata",
                r#"{"attributed_users":[{"username":"member-a","display_name":"Member A","image_url":"","description":"Played this on repeat","can_change_attribution":false}]}"#,
            ),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }

    #[test]
    fn reads_who_each_song_comes_from() {
        let list = context_metadata();
        let found = attribution(&item_attributes(), Some("member-a"));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].username.as_deref(), Some("member-a"));
        assert_eq!(
            found[0].image.as_deref(),
            Some("https://i.scdn.co/image/ab67757000000000000000a1")
        );
        assert_eq!(found[0].kind, "TRACK_ATTRIBUTION");
        assert_eq!(found[0].reason(&list), "Played this on repeat");
        assert_eq!(found[1].label(), "Member B");
        assert_eq!(found[1].image, None);
        assert_eq!(found[1].reason(&list), "Listened to this artist");
        assert_eq!(found[1].uri().as_deref(), Some("spotify:user:member-b"));

        let mut older = item_attributes();
        older.remove("multiUserAttributionDisplayNamesV2");
        older.insert(
            "multiUserAttributionDisplayNames".into(),
            "Member B,Member A".into(),
        );
        older.remove("multiUserAttributionMetadata");
        let found = attribution(&older, None);
        assert_eq!(found[0].label(), "Member B");
        assert_eq!(found[1].reason(&HashMap::new()), TRACK_ATTRIBUTED);

        let added = Attribution {
            kind: "USER_ADDED".into(),
            ..Attribution::default()
        };
        assert_eq!(added.reason(&list), "Added this song");
        assert!(attribution(&HashMap::new(), None).is_empty());
    }

    #[test]
    fn keys_attributions_by_row() {
        let items = vec![
            playlist::PlaylistItem {
                uri: "spotify:track:a".into(),
                uid: Some("00000000000000a1".into()),
                attributes: item_attributes(),
                ..playlist::PlaylistItem::default()
            },
            playlist::PlaylistItem {
                uri: "spotify:track:b".into(),
                ..playlist::PlaylistItem::default()
            },
            playlist::PlaylistItem {
                uri: "spotify:track:c".into(),
                attributes: item_attributes(),
                ..playlist::PlaylistItem::default()
            },
        ];
        let found = attributions(&items);
        assert_eq!(found.len(), 2);
        assert_eq!(found["00000000000000a1"].len(), 2);
        assert!(found.contains_key("spotify:track:c"));
    }

    #[test]
    fn reads_invitations_of_every_kind() {
        let value = json!({
            "page_type": "READY_TO_JOIN_ALREADY_CREATED_BLEND",
            "title": "Join this Blend",
            "subtitle": "Member A invited you.",
            "footnote": "",
            "button_text": "Join",
            "members_title": "MEMBERS",
            "members": [
                {"name": "Member A", "username": "member-a", "image_url": "https://i.scdn.co/image/ab67757000000000000000a1", "hash": "x"},
                {"name": "", "username": "", "image_url": null, "hash": ""}
            ],
            "recipient": {"name": "Member B", "username": "member-b", "image_url": null, "hash": "y"},
            "playlist_uri": "spotify:playlist:37i9dQZF1EJRedacted00001"
        });
        let invitation = parse_invitation(&value).unwrap();
        assert_eq!(invitation.state, InvitationState::ReadyExisting);
        assert!(invitation.state.joins() && !invitation.state.invites());
        assert_eq!(invitation.button.as_deref(), Some("Join"));
        assert_eq!(invitation.footnote, None);
        assert_eq!(invitation.members.len(), 1);
        assert_eq!(invitation.recipient.as_ref().unwrap().name, "Member B");
        assert_eq!(
            invitation.playlist_uri.as_deref(),
            Some("spotify:playlist:37i9dQZF1EJRedacted00001")
        );
        for (kind, state) in [
            ("PENDING_INVITATION", InvitationState::Pending),
            ("READY_TO_JOIN_EMPTY_BLEND", InvitationState::ReadyEmpty),
            ("MAX_MEMBERS", InvitationState::Full),
            ("ALREADY_JOINED", InvitationState::Joined),
            ("DELETED", InvitationState::Deleted),
            (
                "SOMETHING_NEW",
                InvitationState::Other("SOMETHING_NEW".into()),
            ),
        ] {
            let parsed = parse_invitation(&json!({"page_type": kind})).unwrap();
            assert_eq!(parsed.state, state);
            assert!(parsed.members.is_empty() && parsed.recipient.is_none());
        }
        assert!(InvitationState::Deleted.invites());
        assert!(parse_invitation(&json!({"title": "x"})).is_err());
    }

    #[test]
    fn finds_invitation_ids_in_links() {
        for input in [
            "https://open.spotify.com/blend/taste-match/AbC-123_x",
            "https://open.spotify.com/intl-de/blend/taste-match/AbC-123_x?si=1",
            "/blend/taste-match/AbC-123_x",
        ] {
            assert_eq!(
                invitation_id(input).as_deref(),
                Some("AbC-123_x"),
                "{input}"
            );
        }
        for input in [
            "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
            "https://example.com/blend/taste-match/abc",
            "https://open.spotify.com/blend/taste-match/bad%20id",
            "AbC-123_x",
            "radiohead",
            "",
        ] {
            assert_eq!(invitation_id(input), None, "{input}");
        }
        assert!(valid_id("AbC-123_x"));
        assert!(!valid_id("two words") && !valid_id("a/b") && !valid_id(""));
    }

    #[test]
    fn builds_the_invitation_requests() {
        assert_eq!(
            invitation_path("a b/c", "en"),
            "/blend-invitation/v3/view-invitation/a%20b%2Fc?locale=en"
        );
        assert_eq!(join_path("abc"), "/blend-invitation/v2/join/abc");
        assert_eq!(
            parse_invite(&json!({"invite": "https://spotify.link/x"})).unwrap(),
            "https://spotify.link/x"
        );
        assert!(parse_invite(&json!({})).is_err());
        assert_eq!(
            parse_joined(&json!({"playlist_uri": "spotify:user:spotify:playlist:x"})).unwrap(),
            "spotify:playlist:x"
        );
        assert!(parse_joined(&json!({"playlist_uri": ""})).is_err());
        assert_eq!(
            share_text("https://x", Some("Member A")),
            "Member A has invited you to join a Blend on Spotify.  Join on the Spotify mobile app. https://x"
        );
        assert_eq!(
            share_text("https://x", Some(" ")),
            "You have been invited to join a Blend on Spotify. Join on the Spotify mobile app. https://x"
        );
    }
}
