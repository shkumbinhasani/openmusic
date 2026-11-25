use anyhow::{Context, Result};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    api::{Api, Body},
    catalog, entity,
    pathfinder::{self, Operation},
};

pub const PROFILE_ATTRIBUTES: Operation = Operation::query(
    "profileAttributes",
    "08ffb4730af3746e04a8301396f20875dbbce10c75243803091a9274eacc8ac0",
);
pub const ACCOUNT_ATTRIBUTES: Operation = Operation::query(
    "accountAttributes",
    "be4ce359daee01463ddb8e77536220e108e3d28105e12199f244d7a5954e9d14",
);
pub const PRODUCT_STATE: &str = "/melody/v1/product_state";
pub const AVATAR_SIZE: u32 = 300;
const PROTOBUF: &str = "application/x-protobuf";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccountInfo {
    pub uri: String,
    pub username: String,
    pub name: String,
    pub avatar: Option<String>,
    pub product: String,
    pub catalogue: String,
    pub ads: bool,
    pub explicit_filtered: bool,
    pub explicit_locked: bool,
    pub family_role: Option<String>,
    pub country: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Explicit {
    Allowed,
    Filtered,
    FilteredByManager,
}

impl AccountInfo {
    pub fn premium(&self) -> bool {
        self.product == "premium" || (self.catalogue == "premium" && !self.ads)
    }

    pub fn plan(&self) -> &'static str {
        if !self.premium() {
            return "Spotify Free";
        }
        let role = self.family_role.as_deref().unwrap_or_default();
        if role.contains("DUO") {
            "Premium Duo"
        } else if role.contains("FAMILY") || role.contains("CHILD") || role.contains("KID") {
            "Premium Family"
        } else {
            "Premium"
        }
    }

    pub fn plan_manager(&self) -> bool {
        self.family_role
            .as_deref()
            .is_some_and(|role| role.ends_with("MANAGER"))
    }

    pub fn explicit(&self) -> Explicit {
        match (self.explicit_filtered, self.explicit_locked) {
            (true, true) => Explicit::FilteredByManager,
            (true, false) => Explicit::Filtered,
            (false, _) => Explicit::Allowed,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profile {
    pub uri: String,
    pub username: String,
    pub name: String,
    pub avatar: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub product: String,
    pub catalogue: String,
    pub ads: bool,
    pub explicit_filtered: bool,
    pub explicit_locked: bool,
    pub family_role: Option<String>,
    pub country: String,
}

impl Profile {
    pub fn from_graphql(data: &Value) -> Option<Self> {
        let profile = &data["me"]["profile"];
        let username = text(&profile["username"])?;
        Some(Self {
            uri: text(&profile["uri"]).unwrap_or_else(|| format!("spotify:user:{username}")),
            name: text(&profile["name"]).unwrap_or_else(|| username.clone()),
            avatar: entity::pick_image(&profile["avatar"]["sources"], AVATAR_SIZE),
            username,
        })
    }

    pub fn from_identity(username: &str, profile: &IdentityProfile) -> Self {
        let name = profile
            .name
            .as_ref()
            .map(|name| name.value.trim())
            .filter(|name| !name.is_empty())
            .unwrap_or(username);
        let avatar = profile
            .images
            .iter()
            .filter(|image| !image.url.is_empty())
            .min_by_key(|image| image.max_width.abs_diff(AVATAR_SIZE as i32))
            .map(|image| image.url.clone());
        Self {
            uri: format!("spotify:user:{username}"),
            username: username.to_owned(),
            name: name.to_owned(),
            avatar,
        }
    }
}

impl Plan {
    pub fn from_graphql(data: &Value) -> Option<Self> {
        let account = &data["me"]["account"];
        let attributes = account.get("attributes")?;
        let flag = |key: &str| truthy(&attributes[key]);
        Some(Self {
            product: text(&account["product"])
                .unwrap_or_default()
                .to_ascii_lowercase(),
            catalogue: text(&attributes["catalogue"])
                .unwrap_or_default()
                .to_ascii_lowercase(),
            ads: flag("ads"),
            explicit_filtered: flag("filterExplicitContent"),
            explicit_locked: flag("lockFilterExplicitContent"),
            family_role: text(&attributes["multiUserPlanMemberType"]),
            country: text(&account["country"]).unwrap_or_default(),
        })
    }

    pub fn from_product_state(state: &Value) -> Self {
        let flag = |key: &str| truthy(&state[key]);
        Self {
            product: text(&state["product"])
                .unwrap_or_default()
                .to_ascii_lowercase(),
            catalogue: text(&state["catalogue"])
                .unwrap_or_default()
                .to_ascii_lowercase(),
            ads: flag("ads"),
            explicit_filtered: flag("filter-explicit-content"),
            explicit_locked: flag("lock-filter-explicit-content"),
            family_role: text(&state["multiuserplan-member-type"]),
            country: text(&state["country"]).unwrap_or_default(),
        }
    }
}

pub fn combine(profile: Profile, plan: Plan) -> AccountInfo {
    AccountInfo {
        uri: profile.uri,
        username: profile.username,
        name: profile.name,
        avatar: profile.avatar,
        product: plan.product,
        catalogue: plan.catalogue,
        ads: plan.ads,
        explicit_filtered: plan.explicit_filtered,
        explicit_locked: plan.explicit_locked,
        family_role: plan.family_role,
        country: plan.country,
    }
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Wrapped {
    #[prost(string, tag = "1")]
    pub value: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct IdentityImage {
    #[prost(int32, tag = "1")]
    pub max_width: i32,
    #[prost(int32, tag = "2")]
    pub max_height: i32,
    #[prost(string, tag = "3")]
    pub url: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct IdentityProfile {
    #[prost(message, optional, tag = "1")]
    pub username: Option<Wrapped>,
    #[prost(message, optional, tag = "2")]
    pub name: Option<Wrapped>,
    #[prost(message, repeated, tag = "3")]
    pub images: Vec<IdentityImage>,
}

pub fn identity_path(username: &str) -> String {
    let escaped: String = username
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("/identity/v3/user/username/{escaped}")
}

pub async fn identity(api: &Api, username: &str) -> Result<IdentityProfile> {
    let bytes = api
        .request(Method::GET, &identity_path(username), Body::Empty, PROTOBUF)
        .await?;
    <IdentityProfile as prost::Message>::decode(bytes.as_slice())
        .context("Unreadable profile from Spotify.")
}

pub async fn account(api: &Api) -> Result<AccountInfo> {
    let (profile, plan) = tokio::join!(
        pathfinder::query(api, PROFILE_ATTRIBUTES, json!({})),
        pathfinder::query(api, ACCOUNT_ATTRIBUTES, json!({})),
    );
    let plan = match plan.ok().as_ref().and_then(Plan::from_graphql) {
        Some(plan) => plan,
        None => Plan::from_product_state(&api.get_json::<Value>(PRODUCT_STATE).await?),
    };
    let profile = match profile.ok().as_ref().and_then(Profile::from_graphql) {
        Some(profile) => profile,
        None => {
            let username = catalog::username(api).await?;
            match identity(api, &username).await {
                Ok(found) => Profile::from_identity(&username, &found),
                Err(_) => Profile::from_identity(&username, &IdentityProfile::default()),
            }
        }
    };
    Ok(combine(profile, plan))
}

pub fn attributes_changed(uri: &str) -> bool {
    uri.contains("spotify:user:attributes:mutated") || uri.starts_with("ap://product-state-update")
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_i64() == Some(1),
        Value::String(text) => matches!(text.trim(), "1" | "true"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use prost::Message;

    use super::*;

    fn profile_attributes() -> Value {
        json!({"me": {"profile": {
            "accountId": "abcdefghij",
            "avatar": {"sources": [
                {"height": 64, "url": "https://i.scdn.co/image/small", "width": 64},
                {"height": 300, "url": "https://i.scdn.co/image/large", "width": 300}
            ]},
            "avatarBackgroundColor": 13139285,
            "name": "Someone Example",
            "socialHandle": null,
            "uri": "spotify:user:someone",
            "username": "someone"
        }}})
    }

    fn account_attributes(attributes: Value, product: &str) -> Value {
        json!({"me": {"account": {
            "attributes": attributes,
            "country": "XK",
            "features": {"personalizedRecommendations": "PERMITTED"},
            "product": product
        }}})
    }

    #[test]
    fn reads_the_profile_attributes() {
        let profile = Profile::from_graphql(&profile_attributes()).unwrap();
        assert_eq!(
            profile,
            Profile {
                uri: "spotify:user:someone".into(),
                username: "someone".into(),
                name: "Someone Example".into(),
                avatar: Some("https://i.scdn.co/image/large".into()),
            }
        );
    }

    #[test]
    fn fills_absent_profile_fields() {
        let bare =
            json!({"me": {"profile": {"username": "someone", "name": null, "avatar": null}}});
        let profile = Profile::from_graphql(&bare).unwrap();
        assert_eq!(profile.uri, "spotify:user:someone");
        assert_eq!(profile.name, "someone");
        assert_eq!(profile.avatar, None);
        assert_eq!(
            Profile::from_graphql(&json!({"me": {"profile": null}})),
            None
        );
        assert_eq!(Profile::from_graphql(&json!({})), None);
    }

    #[test]
    fn reads_a_family_manager_plan() {
        let data = account_attributes(
            json!({
                "ads": false, "catalogue": "premium", "filterExplicitContent": null,
                "multiUserPlanCurrentSize": 6, "multiUserPlanMemberType": "FAMILY_MANAGER",
                "onDemand": true
            }),
            "PREMIUM",
        );
        let plan = Plan::from_graphql(&data).unwrap();
        assert_eq!(
            plan,
            Plan {
                product: "premium".into(),
                catalogue: "premium".into(),
                ads: false,
                explicit_filtered: false,
                explicit_locked: false,
                family_role: Some("FAMILY_MANAGER".into()),
                country: "XK".into(),
            }
        );
        let info = combine(Profile::from_graphql(&profile_attributes()).unwrap(), plan);
        assert!(info.premium());
        assert_eq!(info.plan(), "Premium Family");
        assert!(info.plan_manager());
        assert_eq!(info.explicit(), Explicit::Allowed);
    }

    #[test]
    fn reads_the_explicit_filter_and_its_lock() {
        let filtered = account_attributes(
            json!({"ads": false, "catalogue": "premium", "filterExplicitContent": true,
                   "lockFilterExplicitContent": true, "multiUserPlanMemberType": "CHILD"}),
            "PREMIUM",
        );
        let plan = Plan::from_graphql(&filtered).unwrap();
        assert!(plan.explicit_filtered);
        assert!(plan.explicit_locked);
        let info = combine(Profile::default(), plan);
        assert_eq!(info.explicit(), Explicit::FilteredByManager);
        assert_eq!(info.plan(), "Premium Family");
        assert!(!info.plan_manager());

        let own = account_attributes(
            json!({"filterExplicitContent": true, "lockFilterExplicitContent": false}),
            "PREMIUM",
        );
        let info = combine(Profile::default(), Plan::from_graphql(&own).unwrap());
        assert_eq!(info.explicit(), Explicit::Filtered);
        assert_eq!(info.plan(), "Premium");
    }

    #[test]
    fn recognises_free_accounts() {
        let free = account_attributes(json!({"ads": true, "catalogue": "free"}), "FREE");
        let info = combine(Profile::default(), Plan::from_graphql(&free).unwrap());
        assert!(!info.premium());
        assert_eq!(info.plan(), "Spotify Free");
        let open = account_attributes(json!({"ads": true, "catalogue": "free"}), "OPEN");
        assert!(!combine(Profile::default(), Plan::from_graphql(&open).unwrap()).premium());
        assert_eq!(
            Plan::from_graphql(&json!({"me": {"account": {"product": "PREMIUM"}}})),
            None
        );
    }

    #[test]
    fn reads_the_product_state_fallback() {
        let state = json!({
            "ads": "0", "catalogue": "premium", "country": "XK", "is-standalone-audiobooks": "0",
            "multiuserplan-current-size": "6", "multiuserplan-member-type": "FAMILY_MANAGER",
            "on-demand": "1", "preferred-locale": "en", "product": "premium", "selected-language": ""
        });
        let plan = Plan::from_product_state(&state);
        assert_eq!(
            plan,
            Plan {
                product: "premium".into(),
                catalogue: "premium".into(),
                ads: false,
                explicit_filtered: false,
                explicit_locked: false,
                family_role: Some("FAMILY_MANAGER".into()),
                country: "XK".into(),
            }
        );
        let filtered = Plan::from_product_state(&json!({
            "ads": "1", "catalogue": "free", "product": "free",
            "filter-explicit-content": "1", "lock-filter-explicit-content": "0"
        }));
        assert!(filtered.ads);
        assert!(filtered.explicit_filtered);
        assert!(!filtered.explicit_locked);
        assert_eq!(filtered.family_role, None);
    }

    #[test]
    fn reads_an_identity_profile() {
        let bytes = IdentityProfile {
            username: Some(Wrapped {
                value: "someone".into(),
            }),
            name: Some(Wrapped {
                value: "Someone Example".into(),
            }),
            images: vec![
                IdentityImage {
                    max_width: 64,
                    max_height: 64,
                    url: "https://i.scdn.co/image/small".into(),
                },
                IdentityImage {
                    max_width: 300,
                    max_height: 300,
                    url: "https://i.scdn.co/image/large".into(),
                },
            ],
        }
        .encode_to_vec();
        let decoded = IdentityProfile::decode(bytes.as_slice()).unwrap();
        let profile = Profile::from_identity("someone", &decoded);
        assert_eq!(profile.name, "Someone Example");
        assert_eq!(
            profile.avatar.as_deref(),
            Some("https://i.scdn.co/image/large")
        );
        assert_eq!(profile.uri, "spotify:user:someone");
        let bare = Profile::from_identity("someone", &IdentityProfile::default());
        assert_eq!(bare.name, "someone");
        assert_eq!(bare.avatar, None);
    }

    #[test]
    fn skips_unknown_identity_fields() {
        let mut bytes = IdentityProfile {
            username: Some(Wrapped {
                value: "someone".into(),
            }),
            name: None,
            images: Vec::new(),
        }
        .encode_to_vec();
        bytes.extend([0xc2, 0x01, 0x02, 0x08, 0x01]);
        let decoded = IdentityProfile::decode(bytes.as_slice()).unwrap();
        assert_eq!(decoded.username.unwrap().value, "someone");
    }

    #[test]
    fn escapes_the_identity_path() {
        assert_eq!(
            identity_path("someone"),
            "/identity/v3/user/username/someone"
        );
        assert_eq!(
            identity_path("a b/c"),
            "/identity/v3/user/username/a%20b%2Fc"
        );
    }

    #[test]
    fn spots_attribute_pushes() {
        assert!(attributes_changed("spotify:user:attributes:mutated"));
        assert!(attributes_changed(
            "hm://notifications/spotify:user:attributes:mutated"
        ));
        assert!(attributes_changed("ap://product-state-update"));
        assert!(!attributes_changed(
            "hm://collection/collection/someone/json"
        ));
    }

    #[test]
    fn keeps_old_saved_accounts_readable() {
        let old: AccountInfo = serde_json::from_str(
            r#"{"uri":"spotify:user:someone","username":"someone","name":"Someone","avatar":null,
                "product":"premium","catalogue":"premium","ads":false,"explicit_filtered":false,
                "explicit_locked":false,"family_role":null}"#,
        )
        .unwrap();
        assert_eq!(old.country, "");
        assert!(old.premium());
    }
}
