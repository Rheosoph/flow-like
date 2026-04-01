use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod close;
pub mod connect;
pub mod listen;
pub mod send;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct TcpConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub timeout_seconds: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct TcpListenConfig {
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub timeout_seconds: u64,
    #[serde(default = "default_backlog")]
    pub max_connections: u32,
}

fn default_backlog() -> u32 {
    128
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct TcpSession {
    pub ref_id: String,
    pub remote_addr: String,
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
pub struct CachedTcpConnection {
    pub reader: Arc<Mutex<tokio::io::ReadHalf<tokio::net::TcpStream>>>,
    pub writer: Arc<Mutex<tokio::io::WriteHalf<tokio::net::TcpStream>>>,
    pub close_notify: Arc<tokio::sync::Notify>,
}

#[cfg(feature = "execute")]
impl Cacheable for CachedTcpConnection {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(feature = "execute")]
pub async fn get_tcp_connection(
    context: &flow_like::flow::execution::context::ExecutionContext,
    ref_id: &str,
) -> flow_like_types::Result<Arc<CachedTcpConnection>> {
    let cache = context.cache.read().await;
    let conn = cache
        .get(ref_id)
        .ok_or_else(|| flow_like_types::anyhow!("TCP connection not found in cache: {}", ref_id))?
        .clone();

    let conn = conn
        .as_any()
        .downcast_ref::<CachedTcpConnection>()
        .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast TCP connection"))?;

    Ok(Arc::new(CachedTcpConnection {
        reader: conn.reader.clone(),
        writer: conn.writer.clone(),
        close_notify: conn.close_notify.clone(),
    }))
}
