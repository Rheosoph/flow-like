use super::{Inbox, MAX_WAKE_BYTES};
use crate::gcp::{
    auth::FirebaseAuth, database_root, json_url, path_segments, redirect_policy, validate_key,
};
use flow_like_types::{Result, anyhow, bail, realtime::RealtimeTransport};
use futures_util::StreamExt;
use reqwest::{Response, Url};
use std::convert::Infallible;
use std::time::Duration;
use tokio::sync::watch;

/// Room for a reply of [`MAX_WAKE_BYTES`] even if the database escapes every character it sends.
const MAX_REPLY_FRAME_BYTES: usize = MAX_WAKE_BYTES * 4;

/// A bounded SSE decoder. The provider stores one wakeup value at this exact path.
struct Decoder {
    buffered: Vec<u8>,
    limit: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::bounded(MAX_WAKE_BYTES * 2)
    }
}

impl Decoder {
    fn bounded(limit: usize) -> Self {
        Self {
            buffered: Vec::new(),
            limit,
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<Vec<(String, String)>> {
        let mut events = Vec::new();
        for byte in chunk {
            self.buffered.push(*byte);
            if self.buffered.len() > self.limit {
                bail!("Realtime Firebase frame exceeds the size limit");
            }
            let end = self.buffered.ends_with(b"\n\n") || self.buffered.ends_with(b"\r\n\r\n");
            if !end {
                continue;
            }
            let text = std::str::from_utf8(&self.buffered)
                .map_err(|_| anyhow!("Invalid realtime Firebase encoding"))?;
            let mut kind = String::new();
            let mut data = String::new();
            for line in text.lines() {
                if let Some(value) = line.strip_prefix("event:") {
                    kind = value.trim_start().to_owned();
                }
                if let Some(value) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(value.trim_start());
                }
            }
            if !kind.is_empty() {
                events.push((kind, data));
            }
            self.buffered.clear();
        }
        Ok(events)
    }
}

/// The value a write to the watched node itself stored; `None` for every other event.
fn written(kind: &str, text: &str) -> Option<Vec<u8>> {
    if !matches!(kind, "put" | "patch") {
        return None;
    }
    let event = serde_json::from_str::<serde_json::Value>(text).ok()?;
    if event["path"] != "/" {
        return None;
    }
    serde_json::to_vec(&event["data"]).ok()
}

fn node_url(root: &Url, path: &str) -> Result<Url> {
    let segments = path_segments(path);
    for segment in &segments {
        validate_key("realtime path", segment)?;
    }
    json_url(root, &segments)
}

async fn subscribe(client: &reqwest::Client, mut url: Url, token: &str) -> Result<Response> {
    url.query_pairs_mut().append_pair("auth", token);
    let response = tokio::time::timeout(
        Duration::from_secs(15),
        client.get(url).header("Accept", "text/event-stream").send(),
    )
    .await
    .map_err(|_| anyhow!("Realtime Firebase connection timed out"))?
    .map_err(|_| anyhow!("Realtime Firebase connection failed"))?;
    if !response.status().is_success() {
        bail!("Realtime Firebase stream was denied");
    }
    Ok(response)
}

async fn next_chunk<S, B>(stream: &mut S) -> Result<B>
where
    S: futures_util::Stream<Item = reqwest::Result<B>> + Unpin,
{
    tokio::time::timeout(Duration::from_secs(90), stream.next())
        .await
        .map_err(|_| anyhow!("Realtime Firebase stream became idle"))?
        .ok_or_else(|| anyhow!("Realtime Firebase stream closed"))?
        .map_err(|_| anyhow!("Realtime Firebase stream failed"))
}

/// Hands each value written to the node to `deliver` until the stream ends.
async fn follow(response: Response, mut decoder: Decoder, deliver: impl Fn(&[u8])) -> Result<()> {
    let mut stream = response.bytes_stream();
    loop {
        let chunk = next_chunk(&mut stream).await?;
        for (kind, data) in decoder.push(&chunk)? {
            if let Some(payload) = handed_on(&kind, &data)? {
                deliver(&payload);
            }
        }
    }
}

/// The value an event hands on, or the end of the stream when access was withdrawn.
fn handed_on(kind: &str, data: &str) -> Result<Option<Vec<u8>>> {
    if matches!(kind, "cancel" | "auth_revoked") {
        bail!("Realtime Firebase stream was revoked");
    }
    Ok(written(kind, data))
}

/// The reply node is best effort: a deployment whose rules predate it refuses the read, and
/// the wakeups go on without it until the next connection tries again.
async fn follow_replies(
    client: &reqwest::Client,
    url: Option<Url>,
    token: &str,
    inbox: &Inbox,
) -> Infallible {
    if let Some(url) = url {
        let decoder = Decoder::bounded(MAX_REPLY_FRAME_BYTES);
        let followed = match subscribe(client, url, token).await {
            Ok(response) => follow(response, decoder, |payload| inbox.reply(payload)).await,
            Err(error) => Err(error),
        };
        if let Err(error) = followed {
            tracing::debug!(%error, "Realtime Firebase replies stopped; wakeups continue");
        }
    }
    std::future::pending().await
}

fn realtime_root(database_url: &str) -> Result<Url> {
    let root = database_root(database_url)?;
    if !root.username().is_empty()
        || root.password().is_some()
        || root.query().is_some()
        || root.fragment().is_some()
    {
        bail!("Invalid realtime Firebase URL");
    }
    Ok(root)
}

pub(super) async fn listen(
    transport: &RealtimeTransport,
    inbox: &Inbox,
    healthy: &watch::Sender<bool>,
) -> Result<()> {
    let RealtimeTransport::GcpFirebaseRtdb {
        database_url,
        api_key,
        custom_token,
        path,
        reply_path,
        ..
    } = transport
    else {
        bail!("Realtime transport is not a Firebase grant");
    };
    let root = realtime_root(database_url)?;
    let url = node_url(&root, path)?;
    let reply_url = reply_path
        .as_deref()
        .filter(|_| inbox.replies.is_some())
        .and_then(|reply_path| node_url(&root, reply_path).ok());
    let auth_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15))
        .build()?;
    let auth = FirebaseAuth::new(auth_client, api_key.to_owned(), custom_token.to_owned());
    let token = auth
        .id_token()
        .await
        .map_err(|_| anyhow!("Realtime Firebase sign-in failed"))?;
    let client = reqwest::Client::builder()
        .redirect(redirect_policy(root.host_str().unwrap_or_default()))
        .connect_timeout(Duration::from_secs(10))
        .build()?;
    let wakeups = subscribe(&client, url, &token).await?;
    inbox.ready(healthy);
    follow_nodes(&client, wakeups, reply_url, &token, inbox).await
}

/// Follows the wakeup node until it ends, and the reply node beside it.
async fn follow_nodes(
    client: &reqwest::Client,
    wakeups: Response,
    reply_url: Option<Url>,
    token: &str,
    inbox: &Inbox,
) -> Result<()> {
    tokio::select! {
        ended = follow(wakeups, Decoder::default(), |payload| inbox.receive(payload)) => ended,
        never = follow_replies(client, reply_url, token, inbox) => match never {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcp::allows_redirect;

    fn wakeup(kind: &str, text: &str) -> bool {
        written(kind, text).is_some_and(|payload| crate::realtime::valid_wakeup(&payload))
    }

    #[test]
    fn fragmented_frames_reassemble_and_ignore_other_paths() {
        let mut decoder = Decoder::default();
        assert!(decoder.push(b"event: put\r\nda").unwrap().is_empty());
        let events = decoder.push(b"ta: {\"path\":\"/\",\"data\":{\"protocol_version\":1,\"kind\":\"invalidate\",\"nonce\":\"n\"}}\r\n\r\n").unwrap();
        assert_eq!(events.len(), 1);
        assert!(wakeup(&events[0].0, &events[0].1));
        assert!(!wakeup("put", &events[0].1.replace("\"/\"", "\"/other\"")));
        assert!(decoder.push(&vec![b'x'; MAX_WAKE_BYTES * 2 + 1]).is_err());
    }

    const EVENT_STREAM: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n";
    const REFUSED: &str = "HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\n\r\n";

    fn put(data: serde_json::Value) -> String {
        format!(
            "event: put\ndata: {}\n\n",
            serde_json::json!({ "path": "/", "data": data })
        )
    }

    fn reply_value() -> serde_json::Value {
        serde_json::json!({
            "protocol_version": 1, "kind": "reply", "conversation_id": "c-1",
            "message_id": "re-1", "reply_to": "m-1", "sender_id": "bot-1",
            "offset": 0, "text": "Hello", "state": "writing"
        })
    }

    /// A database whose `reply` node answers with `reply_node`; its `wake` node writes one
    /// wakeup once the reply node was asked for. Both stay open.
    async fn database(reply_node: String) -> Url {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let root = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let asked = std::sync::Arc::new(tokio::sync::Notify::new());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (asked, reply_node) = (asked.clone(), reply_node.clone());
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n")
                        && socket.read(&mut byte).await.unwrap_or(0) == 1
                    {
                        head.push(byte[0]);
                    }
                    if String::from_utf8_lossy(&head).contains("/reply.json") {
                        let _ = socket.write_all(reply_node.as_bytes()).await;
                        asked.notify_one();
                    } else {
                        let _ = socket.write_all(EVENT_STREAM.as_bytes()).await;
                        asked.notified().await;
                        let wakeup = put(serde_json::json!({
                            "protocol_version": 1, "kind": "invalidate", "nonce": "n"
                        }));
                        let _ = socket.write_all(wakeup.as_bytes()).await;
                    }
                    std::future::pending::<()>().await;
                });
            }
        });
        root
    }

    #[tokio::test]
    async fn replies_follow_their_own_node_and_a_refused_one_leaves_wakeups_alone() {
        let wakeup_shaped = put(serde_json::json!({
            "protocol_version": 1, "kind": "invalidate", "nonce": "on-the-reply-node"
        }));
        let readable = format!("{EVENT_STREAM}{wakeup_shaped}{}", put(reply_value()));
        for (reply_node, expected) in [(readable, Some("Hello")), (REFUSED.to_owned(), None)] {
            let root = database(reply_node).await;
            let (sender, mut received) = tokio::sync::mpsc::unbounded_channel();
            let inbox = Inbox {
                wake: std::sync::Arc::new(tokio::sync::Notify::new()),
                replies: Some(std::sync::Arc::new(
                    move |reply: flow_like_types::realtime::RealtimeReply| {
                        let _ = sender.send(reply.text);
                    },
                )),
            };
            let client = reqwest::Client::new();
            let wake_url = node_url(&root, "/realtime/scope/wake").unwrap();
            let wakeups = subscribe(&client, wake_url, "token").await.unwrap();
            let reply_url = node_url(&root, "/realtime/scope/reply").ok();
            let settled = async {
                inbox.wake.notified().await;
                match expected {
                    Some(_) => received.recv().await,
                    None => received.try_recv().ok(),
                }
            };
            let outcome = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::select! {
                    ended = follow_nodes(&client, wakeups, reply_url, "token", &inbox) => {
                        panic!("the wakeup node ended: {ended:?}")
                    }
                    reply = settled => reply,
                }
            })
            .await
            .expect("the wakeup arrives");
            assert_eq!(outcome.as_deref(), expected);
            assert!(received.try_recv().is_err(), "one reply, delivered once");
        }
    }

    #[test]
    fn a_removed_reply_node_delivers_nothing() {
        let mut decoder = Decoder::bounded(MAX_REPLY_FRAME_BYTES);
        let removed = put(serde_json::Value::Null);
        let events = decoder.push(removed.as_bytes()).unwrap();
        let payload = written(&events[0].0, &events[0].1).unwrap();
        assert!(crate::realtime::valid_reply(&payload).is_none());
        let long = put(serde_json::json!({ "text": "\\u3042".repeat(MAX_WAKE_BYTES / 3) }));
        assert!(Decoder::default().push(long.as_bytes()).is_err());
        assert!(decoder.push(long.as_bytes()).is_ok());
    }

    const LEGACY: &str = "project.firebaseio.com";
    const REGIONAL: &str = "demo.europe-west1.firebasedatabase.app";

    fn allows(original: &str, target: &str) -> bool {
        allows_redirect(original, &target.parse().unwrap())
    }

    #[test]
    fn redirects_follow_shards_of_the_configured_database_domain() {
        for target in [
            "https://s-usc1a-nss-2001.firebaseio.com/path.json?ns=project",
            "https://s-gke-usc1-nssi3-16.firebaseio.com/path.json",
            "https://project.firebaseio.com/other.json",
        ] {
            assert!(allows(LEGACY, target), "{target}");
        }
        for target in [
            "https://s-gke-euw1-nssi1-1.europe-west1.firebasedatabase.app/path.json",
            "https://s-euw1b-nss-206.europe-west1.firebasedatabase.app/path.json",
        ] {
            assert!(allows(REGIONAL, target), "{target}");
        }
    }

    #[test]
    fn redirects_refuse_other_hosts_schemes_credentials_and_ports() {
        for target in [
            "https://other-project.firebaseio.com/path.json",
            "https://s-usc1a-nss-1.firebaseio.com.attacker.test/path.json",
            "https://s-usc1a-nss-1.evilfirebaseio.com/path.json",
            "https://s-a.b.firebaseio.com/path.json",
            "http://s-usc1a-nss-1.firebaseio.com/path.json",
            "https://user:secret@s-usc1a-nss-1.firebaseio.com/path.json",
            "https://s-usc1a-nss-1.firebaseio.com:8443/path.json",
            "https://s-usc1a-nss-1.firebaseio.com:443@attacker.test/path.json",
            "https://s-gke-euw1-nssi1-1.europe-west1.firebasedatabase.app/path.json",
        ] {
            assert!(!allows(LEGACY, target), "{target}");
        }
        for target in [
            "https://s-gke-asse1-nssi1-1.asia-southeast1.firebasedatabase.app/path.json",
            "https://s-usc1a-nss-1.firebaseio.com/path.json",
            "https://other.europe-west1.firebasedatabase.app/path.json",
            "https://s-x.firebasedatabase.app/path.json",
        ] {
            assert!(!allows(REGIONAL, target), "{target}");
        }
        assert!(!allows(
            "emulator.test",
            "https://s-usc1a-nss-1.firebaseio.com/path.json"
        ));
    }
}
