use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod bind;
pub mod close;
pub mod receive;
pub mod send_to;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct UdpConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct UdpSession {
    pub ref_id: String,
    pub local_addr: String,
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
pub struct CachedUdpSocket {
    pub socket: Arc<tokio::net::UdpSocket>,
    pub close_notify: Arc<tokio::sync::Notify>,
}

#[cfg(feature = "execute")]
impl Cacheable for CachedUdpSocket {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(feature = "execute")]
pub async fn get_udp_socket(
    context: &ExecutionContext,
    ref_id: &str,
) -> flow_like_types::Result<Arc<CachedUdpSocket>> {
    let cache = context.cache.read().await;
    let conn = cache
        .get(ref_id)
        .ok_or_else(|| flow_like_types::anyhow!("UDP socket not found in cache: {}", ref_id))?
        .clone();

    let conn = conn
        .as_any()
        .downcast_ref::<CachedUdpSocket>()
        .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast UDP socket"))?;

    Ok(Arc::new(CachedUdpSocket {
        socket: conn.socket.clone(),
        close_notify: conn.close_notify.clone(),
    }))
}
