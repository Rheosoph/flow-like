use flow_like_types::{
    Result, anyhow, bail,
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64},
};
use futures::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
    tungstenite::{
        Message, client::IntoClientRequest, http::HeaderValue, protocol::WebSocketConfig,
    },
};

use super::{access::Access, crypto::Envelope};

const PROTOCOL: &str = "flowlike.realtime.v1";
const TOKEN_PROTOCOL_PREFIX: &str = "flowlike.jwt.";
const ORIGIN: &str = "tauri://localhost";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FRAME: usize = 256 * 1024;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(super) enum Frame {
    Publish(Value),
    Pong,
    Other,
}

pub(super) struct Writer {
    sink: SplitSink<Socket, Message>,
    topic: String,
}

pub(super) struct Reader {
    stream: SplitStream<Socket>,
}

pub(super) fn digest(topic: &str) -> String {
    B64.encode(&Sha256::digest(topic.as_bytes())[..24])
}

pub(super) fn socket_url(server: &str, topic: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(server)
        .map_err(|error| anyhow!("Signaling server {server} is not a URL: {error}"))?;
    if url.scheme() != "wss"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Signaling server {server} must be a wss URL without credentials, query or fragment");
    }
    if !matches!(url.path().trim_end_matches('/'), "" | "/ws") {
        bail!("Signaling server {server} must have the path / or /ws");
    }
    url.set_path(&format!("/ws/session/{}", digest(topic)));
    Ok(url)
}

fn upgrade_request(
    url: &reqwest::Url,
    jwt: &str,
) -> Result<tokio_tungstenite::tungstenite::handshake::client::Request> {
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| anyhow!("Invalid signaling request: {error}"))?;
    let protocols = format!("{PROTOCOL}, {TOKEN_PROTOCOL_PREFIX}{jwt}");
    let headers = request.headers_mut();
    headers.insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(&protocols)
            .map_err(|_| anyhow!("The bot screen token is not a valid header value"))?,
    );
    headers.insert("Origin", HeaderValue::from_static(ORIGIN));
    Ok(request)
}

pub(super) async fn connect(access: &Access) -> Result<(Writer, Reader)> {
    let url = socket_url(&access.server, &access.topic)?;
    let request = upgrade_request(&url, &access.jwt)?;
    let connector = Connector::Rustls(
        flow_like_device_client::tls_client_config()
            .map_err(|error| anyhow!("Prepare TLS for the signaling server: {error}"))?,
    );
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_FRAME))
        .max_frame_size(Some(MAX_FRAME));
    let (socket, response) = tokio::time::timeout(
        CONNECT_TIMEOUT,
        connect_async_tls_with_config(request, Some(config), true, Some(connector)),
    )
    .await
    .map_err(|_| {
        anyhow!(
            "The signaling server {} did not answer in {CONNECT_TIMEOUT:?}",
            access.server
        )
    })?
    .map_err(|error| anyhow!("Connect to the signaling server {}: {error}", access.server))?;
    let protocol = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|value| value.to_str().ok());
    if protocol != Some(PROTOCOL) {
        bail!("The signaling server answered subprotocol {protocol:?} instead of {PROTOCOL}");
    }
    let (sink, stream) = socket.split();
    let mut writer = Writer {
        sink,
        topic: access.topic.clone(),
    };
    writer
        .send(json!({ "type": "subscribe", "topics": [access.topic] }))
        .await?;
    Ok((writer, Reader { stream }))
}

impl Writer {
    async fn send(&mut self, frame: Value) -> Result<()> {
        self.sink
            .send(Message::text(frame.to_string()))
            .await
            .map_err(|error| anyhow!("Send to the signaling server: {error}"))
    }

    pub async fn ping(&mut self) -> Result<()> {
        self.send(json!({ "type": "ping" })).await
    }

    pub async fn publish(&mut self, envelope: &Envelope) -> Result<()> {
        let frame = json!({ "type": "publish", "topic": self.topic, "data": envelope });
        self.send(frame).await
    }
}

impl Reader {
    pub async fn next(&mut self) -> Result<Frame> {
        loop {
            let message = self
                .stream
                .next()
                .await
                .ok_or_else(|| anyhow!("The signaling server closed the connection"))?
                .map_err(|error| anyhow!("Read from the signaling server: {error}"))?;
            match message {
                Message::Text(text) => return Ok(parse_frame(text.as_str())),
                Message::Close(_) => bail!("The signaling server closed the connection"),
                _ => {}
            }
        }
    }
}

fn parse_frame(text: &str) -> Frame {
    let Ok(frame) = serde_json::from_str::<Value>(text) else {
        return Frame::Other;
    };
    match frame.get("type").and_then(Value::as_str) {
        Some("publish") => frame
            .get("data")
            .cloned()
            .map_or(Frame::Other, Frame::Publish),
        Some("pong") => Frame::Pong,
        _ => Frame::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOPIC: &str = "bot-screens:0123456789abcdef0123456789abcdef:v1";

    #[test]
    fn the_digest_is_the_first_24_sha256_bytes_in_base64url() {
        assert_eq!(digest(TOPIC), "UXX9ZX4zk9ljg2sdaJl5ODTj8ifuDVyG");
    }

    #[test]
    fn the_socket_url_replaces_the_path_with_the_digest() {
        for server in [
            "wss://signal.example",
            "wss://signal.example/",
            "wss://signal.example/ws",
        ] {
            assert_eq!(
                socket_url(server, TOPIC).unwrap().as_str(),
                "wss://signal.example/ws/session/UXX9ZX4zk9ljg2sdaJl5ODTj8ifuDVyG"
            );
        }
    }

    #[test]
    fn other_servers_are_refused() {
        for server in [
            "ws://signal.example",
            "wss://user:pw@signal.example",
            "wss://signal.example/other",
            "wss://signal.example/?a=1",
            "not a url",
        ] {
            assert!(socket_url(server, TOPIC).is_err(), "{server}");
        }
    }

    #[test]
    fn the_upgrade_carries_both_subprotocols_and_the_tauri_origin() {
        let url = socket_url("wss://signal.example", TOPIC).unwrap();
        let request = upgrade_request(&url, "jwt.value.sig").unwrap();
        assert_eq!(
            request.headers()["Sec-WebSocket-Protocol"],
            "flowlike.realtime.v1, flowlike.jwt.jwt.value.sig"
        );
        assert_eq!(request.headers()["Origin"], "tauri://localhost");
    }

    #[test]
    fn frames_parse_by_type() {
        assert!(matches!(parse_frame(r#"{"type":"pong"}"#), Frame::Pong));
        assert!(matches!(
            parse_frame(r#"{"type":"publish","topic":"t","data":{"v":1},"clients":2}"#),
            Frame::Publish(data) if data["v"] == 1
        ));
        assert!(matches!(parse_frame(r#"{"type":"publish"}"#), Frame::Other));
        assert!(matches!(parse_frame("nope"), Frame::Other));
    }
}
