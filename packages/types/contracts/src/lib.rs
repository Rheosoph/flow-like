//! Lightweight, runtime-independent contracts shared across Flow-Like processes.

use std::any::Any;

#[cfg(feature = "cache")]
pub mod cache;
#[cfg(feature = "channel")]
pub mod channel;
#[cfg(feature = "dispatch")]
pub mod dispatch;
#[cfg(feature = "maintenance")]
pub mod maintenance;

/// Header carrying registration-level credentials on app-connection proxy requests.
pub const PROXY_EVENT_AUTHORIZATION_HEADER: &str = "x-flow-like-event-authorization";

pub trait Cacheable: Any + Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl dyn Cacheable {
    pub fn downcast_ref<T: Cacheable>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    pub fn downcast_mut<T: Cacheable>(&mut self) -> Option<&mut T> {
        self.as_any_mut().downcast_mut::<T>()
    }
}

/// OAuth token input for execution requests.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OAuthTokenInput {
    pub access_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}
