use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Who a layer's cached results belong to. Mirrors the scopes the flow cache backends
/// understand; kept here so the layer settings do not have to depend on the catalog.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LayerCacheScope {
    /// Shared by everyone who can execute in the app.
    #[default]
    App,
    /// Private to the user who triggered the run.
    User,
}

impl LayerCacheScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::User => "user",
        }
    }
}

/// Result caching for a layer invoked as a function.
///
/// A hit replaces the whole call: the function body never runs, so its side effects do
/// not happen either. Only turn this on for layers whose outputs are a function of their
/// inputs.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq, Eq)]
pub struct LayerCache {
    #[serde(default)]
    pub enabled: bool,
    /// Namespace every entry for this layer is written under, so one layer's cache can be
    /// invalidated without touching the rest of the app's.
    #[serde(default)]
    pub prefix: String,
    /// Lifetime of an entry in seconds. `None` or `0` keeps it until it is invalidated.
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    #[serde(default)]
    pub scope: LayerCacheScope,
}

impl LayerCache {
    pub fn is_active(&self) -> bool {
        self.enabled
    }

    /// The TTL handed to the cache backend. Layer settings define both omission and `0` as
    /// "never expires", so normalize both to explicit `Some(0)`. At the remote cache boundary,
    /// `None` instead means "use the deployment default" and would violate that layer contract.
    pub fn ttl(&self) -> Option<u64> {
        Some(self.ttl_seconds.unwrap_or(0))
    }
}
