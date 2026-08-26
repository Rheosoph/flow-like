//! Shared cache contract.
//!
//! Only the pieces that cross a process boundary live here: the scope discriminator
//! (used on the wire by the runtime cache nodes) and the maintenance result reported
//! back to whoever triggered an expiry sweep.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Who an entry belongs to.
///
/// `App` entries are shared by every principal that may execute in the app. `User`
/// entries are private to the invoking user and require a resolvable user identity.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheScope {
    #[default]
    App,
    User,
}

impl CacheScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::User => "user",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "app" | "global" => Some(Self::App),
            "user" => Some(Self::User),
            _ => None,
        }
    }

    pub const fn is_user(self) -> bool {
        matches!(self, Self::User)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheCleanupResult {
    /// Entries removed because their TTL had elapsed.
    pub deleted: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scope_wire_format_is_stable() {
        assert_eq!(serde_json::to_value(CacheScope::App).unwrap(), json!("app"));
        assert_eq!(
            serde_json::to_value(CacheScope::User).unwrap(),
            json!("user")
        );
        assert_eq!(
            serde_json::from_value::<CacheScope>(json!("user")).unwrap(),
            CacheScope::User
        );
    }

    #[test]
    fn parse_accepts_the_documented_aliases_only() {
        assert_eq!(CacheScope::parse(" App "), Some(CacheScope::App));
        assert_eq!(CacheScope::parse("global"), Some(CacheScope::App));
        assert_eq!(CacheScope::parse("USER"), Some(CacheScope::User));
        assert_eq!(CacheScope::parse("everyone"), None);
    }
}
