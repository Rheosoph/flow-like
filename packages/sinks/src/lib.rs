//! Flow-Like Sinks - Event sinks for server-side and desktop event triggers
//!
//! This crate provides shared sink types, traits, and configurations used by both
//! the desktop app and server-side API for handling events from external sources.
//!
//! ## Sink Types
//!
//! | Sink | Server | Desktop | Description |
//! |------|--------|---------|-------------|
//! | HTTP | ✅ | ✅ | REST API endpoints |
//! | Webhook | ✅ | ✅ | Incoming webhooks from external services |
//! | Cron | ✅ | ✅ | Scheduled triggers |
//! | MQTT | ✅ | ⚠️ | IoT messaging |
//! | GitHub | ✅ | ⚠️ | Repository webhooks |
//! | RSS | ✅ | ✅ | Feed polling |
//! | Discord | ⚠️ | ✅ | Bot integration (requires persistent process) |
//! | Slack | ⚠️ | ✅ | Bot integration |
//! | Deeplink | ❌ | ✅ | Desktop app URL scheme |
//! | NFC | ❌ | ✅ | Local hardware |
//! | Geolocation | ❌ | ✅ | Device GPS |
//! | Shortcut | ❌ | ✅ | OS keyboard shortcuts |
//! | File | ❌ | ✅ | Local filesystem watching |

mod config;
mod traits;
mod types;

pub mod http;
pub mod scheduler;

pub use config::{
    CronSinkConfig, HttpSinkConfig, MqttSinkConfig, RssSinkConfig, ScheduledLocal, SinkConfig,
    WebhookSinkConfig,
};
pub use scheduler::{ScheduleInfo, SchedulerBackend, SchedulerError, SchedulerResult};
pub use traits::{Executor, SinkContext, SinkError, SinkResult, SinkTrait, TriggerResponse};
pub use types::{SinkAvailability, SinkExecution, SinkRegistration, SinkType};
