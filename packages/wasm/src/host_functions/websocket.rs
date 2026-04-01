//! WebSocket host functions
//!
//! Provides WebSocket connection capabilities for WASM modules.
//! The host acts as a proxy — the WASM guest never gets direct socket access.

use futures::stream::{SplitSink, SplitStream};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type WsStream = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

/// An active WebSocket connection managed by the host on behalf of a WASM guest.
pub struct WsConnection {
    pub sink: WsSink,
    pub stream: WsStream,
}

impl std::fmt::Debug for WsConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WsConnection").finish()
    }
}
