use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::tls::TlsConfig;

pub mod broker;
pub mod connect;
pub mod disconnect;
pub mod publish;
pub mod subscribe;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct MqttConfig {
    pub host: String,
    pub port: u16,
    pub client_id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default = "default_keep_alive")]
    pub keep_alive_seconds: u64,
    #[serde(default)]
    pub use_tls: bool,
    #[serde(default)]
    pub tls: TlsConfig,
    /// Broker session retention is independent of the bounded in-memory workflow queues.
    #[serde(default = "default_clean_session")]
    pub clean_session: bool,
    #[serde(default)]
    pub last_will: Option<MqttLastWill>,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_seconds: u64,
}

fn default_connect_timeout() -> u64 {
    10
}

fn default_clean_session() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct MqttLastWill {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: MqttQoS,
    #[serde(default)]
    pub retain: bool,
}

fn default_keep_alive() -> u64 {
    30
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct MqttSession {
    pub ref_id: String,
    pub client_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct MqttBrokerConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub timeout_seconds: u64,
    #[serde(default = "default_broker_max_connections")]
    pub max_connections: u32,
    #[serde(default)]
    pub tls: TlsConfig,
}

fn default_broker_max_connections() -> u32 {
    128
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum MqttQoS {
    AtMostOnce,
    AtLeastOnce,
    ExactlyOnce,
}

#[cfg(feature = "execute")]
mod runtime;
#[cfg(feature = "execute")]
mod topic;
#[cfg(feature = "execute")]
pub use runtime::*;

#[cfg(all(test, feature = "execute"))]
mod mosquitto_tests;
