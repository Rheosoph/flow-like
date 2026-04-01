use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
pub enum MqttQoS {
    AtMostOnce,
    AtLeastOnce,
    ExactlyOnce,
}

#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_types::Cacheable;
#[cfg(feature = "execute")]
use std::any::Any;
#[cfg(feature = "execute")]
use std::sync::Arc;
#[cfg(feature = "execute")]
use tokio::sync::Mutex;

#[cfg(feature = "execute")]
pub struct CachedMqttConnection {
    pub client: Arc<Mutex<rumqttc::AsyncClient>>,
    pub event_loop: Arc<Mutex<rumqttc::EventLoop>>,
    pub close_notify: Arc<tokio::sync::Notify>,
}

#[cfg(feature = "execute")]
impl Cacheable for CachedMqttConnection {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(feature = "execute")]
pub async fn get_mqtt_connection(
    context: &ExecutionContext,
    ref_id: &str,
) -> flow_like_types::Result<Arc<CachedMqttConnection>> {
    let cache = context.cache.read().await;
    let conn: Arc<dyn Cacheable> = cache
        .get(ref_id)
        .ok_or_else(|| flow_like_types::anyhow!("MQTT connection not found in cache: {}", ref_id))?
        .clone();

    let conn = conn
        .as_any()
        .downcast_ref::<CachedMqttConnection>()
        .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast MQTT connection"))?;

    Ok(Arc::new(CachedMqttConnection {
        client: conn.client.clone(),
        event_loop: conn.event_loop.clone(),
        close_notify: conn.close_notify.clone(),
    }))
}

#[cfg(feature = "execute")]
pub fn to_rumqttc_qos(qos: &MqttQoS) -> rumqttc::QoS {
    match qos {
        MqttQoS::AtMostOnce => rumqttc::QoS::AtMostOnce,
        MqttQoS::AtLeastOnce => rumqttc::QoS::AtLeastOnce,
        MqttQoS::ExactlyOnce => rumqttc::QoS::ExactlyOnce,
    }
}
