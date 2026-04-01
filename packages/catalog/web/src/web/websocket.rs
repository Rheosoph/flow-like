use flow_like::flow::execution::context::ExecutionContext;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub mod close;
pub mod connect;
pub mod send;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct WebSocketConfig {
    pub url: String,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub timeout_seconds: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct WebSocketSession {
    pub ref_id: String,
    pub url: String,
}

#[cfg(feature = "execute")]
use flow_like_types::Cacheable;
#[cfg(feature = "execute")]
use std::any::Any;
#[cfg(feature = "execute")]
use std::sync::Arc;
#[cfg(feature = "execute")]
use tokio::sync::Mutex;
#[cfg(feature = "execute")]
use tokio_tungstenite::tungstenite::Message;

#[cfg(feature = "execute")]
type WsSink = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

#[cfg(feature = "execute")]
type WsStream = futures::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

#[cfg(feature = "execute")]
pub struct CachedWebSocketConnection {
    pub sink: Arc<Mutex<WsSink>>,
    pub close_notify: Arc<tokio::sync::Notify>,
    pub reader_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[cfg(feature = "execute")]
impl Cacheable for CachedWebSocketConnection {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(feature = "execute")]
pub async fn get_ws_connection(
    context: &ExecutionContext,
    ref_id: &str,
) -> flow_like_types::Result<Arc<CachedWebSocketConnection>> {
    let cache = context.cache.read().await;
    let conn = cache
        .get(ref_id)
        .ok_or_else(|| {
            flow_like_types::anyhow!("WebSocket connection not found in cache: {}", ref_id)
        })?
        .clone();

    let conn = conn
        .as_any()
        .downcast_ref::<CachedWebSocketConnection>()
        .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast WebSocket connection"))?;

    Ok(Arc::new(CachedWebSocketConnection {
        sink: conn.sink.clone(),
        close_notify: conn.close_notify.clone(),
        reader_handle: Mutex::new(None),
    }))
}
