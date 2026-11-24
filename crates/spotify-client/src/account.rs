use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
}
