use super::{MAX_WAKE_BYTES, ready, valid_wakeup};
use flow_like_types::{Result, anyhow, bail};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{ClientRequestBuilder, Message, protocol::WebSocketConfig},
};

const SUBPROTOCOL: &str = "json.webpubsub.azure.v1";

pub(super) fn wakeup(frame: &Value, group: &str) -> bool {
    if frame["type"] != "message" {
        return false;
    }
    // REST sends use the server envelope without a group field. Client-origin
    // group frames must match the only group granted to this connection.
    let source = match frame["from"].as_str() {
        Some("server") => frame.get("group").is_none_or(|value| value == group),
        Some("group") => frame["group"] == group,
        _ => false,
    };
    if !source {
        return false;
    }
    match frame["dataType"].as_str() {
        Some("text") => frame["data"]
            .as_str()
            .is_some_and(|text| valid_wakeup(text.as_bytes())),
        Some("json") => serde_json::to_vec(&frame["data"]).is_ok_and(|data| valid_wakeup(&data)),
        _ => false,
    }
}

pub(super) async fn listen(
    url: &str,
    group: &str,
    wake: &Notify,
    healthy: &watch::Sender<bool>,
) -> Result<()> {
    let mut parsed = reqwest::Url::parse(url).map_err(|_| anyhow!("Invalid realtime Azure URL"))?;
    if !matches!(parsed.scheme(), "wss" | "ws")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || group.is_empty()
    {
        bail!("Invalid realtime Azure grant");
    }
    let token = parsed
        .query_pairs()
        .find(|(key, _)| key == "access_token")
        .map(|(_, value)| value.into_owned());
    let query: Vec<_> = parsed
        .query_pairs()
        .filter(|(key, _)| key != "access_token")
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    parsed.set_query(None);
    if !query.is_empty() {
        parsed.query_pairs_mut().extend_pairs(query);
    }
    let uri = parsed
        .as_str()
        .parse()
        .map_err(|_| anyhow!("Invalid realtime Azure URI"))?;
    let mut request = ClientRequestBuilder::new(uri).with_sub_protocol(SUBPROTOCOL);
    if let Some(token) = token {
        request = request.with_header("Authorization", format!("Bearer {token}"));
    }
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_WAKE_BYTES * 2);
    config.max_frame_size = Some(MAX_WAKE_BYTES * 2);
    let (mut socket, response) = tokio::time::timeout(
        Duration::from_secs(15),
        connect_async_tls_with_config(
            request,
            Some(config),
            false,
            Some(tokio_tungstenite::Connector::Rustls(
                crate::tls::client_config()?,
            )),
        ),
    )
    .await
    .map_err(|_| anyhow!("Realtime Azure connection timed out"))?
    .map_err(|_| anyhow!("Realtime Azure connection failed"))?;
    if response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        != Some(SUBPROTOCOL)
    {
        bail!("Realtime Azure subprotocol was not negotiated");
    }
    // The API token joins this exact group through webpubsub.group. It deliberately
    // grants no joinLeaveGroup or sendToGroup role to the receiving client.
    let mut joined = false;
    let join_deadline = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(join_deadline);
    let mut ping = tokio::time::interval(Duration::from_secs(20));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut last_received = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = &mut join_deadline, if !joined => bail!("Realtime Azure join timed out"),
            _ = ping.tick() => {
                if last_received.elapsed() > Duration::from_secs(60) { bail!("Realtime Azure connection became idle"); }
                socket.send(Message::Text("{\"type\":\"ping\"}".into())).await
                    .map_err(|_| anyhow!("Realtime Azure ping failed"))?;
            }
            message = socket.next() => {
                let message = message.ok_or_else(|| anyhow!("Realtime Azure connection closed"))?
                    .map_err(|_| anyhow!("Realtime Azure frame failed"))?;
                last_received = tokio::time::Instant::now();
                match message {
                    Message::Text(text) => {
                        let frame: Value = serde_json::from_str(&text).map_err(|_| anyhow!("Invalid realtime Azure frame"))?;
                        if frame["type"] == "system" && frame["event"] == "connected" {
                            if !joined { joined = true; ready(healthy, wake); }
                        } else if frame["type"] == "system" && frame["event"] == "disconnected" {
                            bail!("Realtime Azure disconnected");
                        } else if wakeup(&frame, group) { wake.notify_one(); }
                    }
                    Message::Ping(payload) => { socket.send(Message::Pong(payload)).await.map_err(|_| anyhow!("Realtime Azure pong failed"))?; }
                    Message::Close(_) => bail!("Realtime Azure connection closed"),
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_the_granted_group_can_wake_replay() {
        let mut frame = json!({"type":"message","from":"group","group":"user-a","dataType":"json","data":{"protocol_version":1,"kind":"invalidate","nonce":"n"}});
        assert!(wakeup(&frame, "user-a"));
        assert!(!wakeup(&frame, "user-b"));
        frame["from"] = "server".into();
        frame.as_object_mut().unwrap().remove("group");
        assert!(wakeup(&frame, "user-a"));
        frame["dataType"] = "text".into();
        frame["data"] = frame["data"].to_string().into();
        assert!(wakeup(&frame, "user-a"));
        frame["data"] = "forged payload".into();
        assert!(!wakeup(&frame, "user-a"));
    }
}
