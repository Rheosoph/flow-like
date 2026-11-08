//! Lightweight, runtime-independent contracts shared across Flow-Like processes.

use std::any::Any;

pub mod authorization;
pub mod egress;
pub mod interaction;

#[cfg(feature = "storage-paths")]
pub mod storage;

#[cfg(feature = "data-classification")]
pub mod data_classification;

#[cfg(feature = "bot")]
pub mod bot;
#[cfg(feature = "bot")]
pub mod bot_connector;
#[cfg(feature = "bot")]
pub mod bot_effective;
#[cfg(feature = "bot")]
pub mod bot_skill;
#[cfg(feature = "bot")]
pub mod bot_restrictions;

#[cfg(feature = "inbox")]
pub mod bot_message;
#[cfg(feature = "inbox")]
pub mod bot_notify;
#[cfg(feature = "bot-task")]
pub mod bot_task;
#[cfg(feature = "inbox")]
pub mod bot_turn;
#[cfg(feature = "inbox")]
pub mod bot_user_context;
#[cfg(feature = "inbox")]
pub mod user_availability;
#[cfg(feature = "device-bot")]
pub mod device_bot;
#[cfg(feature = "inbox")]
pub mod inbox;
#[cfg(feature = "inbox")]
pub mod tool_grant;

#[cfg(feature = "geometry")]
pub mod geometry;

#[cfg(feature = "cache")]
pub mod cache;
#[cfg(feature = "channel")]
pub mod channel;
#[cfg(feature = "dispatch")]
pub mod dispatch;
#[cfg(feature = "maintenance")]
pub mod maintenance;
#[cfg(feature = "realtime")]
pub mod realtime;

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
