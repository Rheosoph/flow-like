use super::{MAX_WAKE_BYTES, ready, valid_wakeup};
use crate::gcp::{
    auth::FirebaseAuth, database_root, json_url, path_segments, redirect_policy, validate_key,
};
use flow_like_types::{Result, anyhow, bail};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::sync::{Notify, watch};

/// A bounded SSE decoder. The provider stores one wakeup value at this exact path.
#[derive(Default)]
struct Decoder {
    buffered: Vec<u8>,
}

impl Decoder {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<(String, String)>> {
        let mut events = Vec::new();
        for byte in chunk {
            self.buffered.push(*byte);
            if self.buffered.len() > MAX_WAKE_BYTES * 2 {
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

fn wakeup(kind: &str, text: &str) -> bool {
    if !matches!(kind, "put" | "patch") {
        return false;
    }
    let Ok(event) = serde_json::from_str::<serde_json::Value>(text) else {
        return false;
    };
    event["path"] == "/" && serde_json::to_vec(&event["data"]).is_ok_and(|data| valid_wakeup(&data))
}

pub(super) async fn listen(
    database_url: &str,
    api_key: &str,
    custom_token: &str,
    path: &str,
    wake: &Notify,
    healthy: &watch::Sender<bool>,
) -> Result<()> {
    let root = database_root(database_url)?;
    if !root.username().is_empty()
        || root.password().is_some()
        || root.query().is_some()
        || root.fragment().is_some()
    {
        bail!("Invalid realtime Firebase URL");
    }
    let segments = path_segments(path);
    for segment in &segments {
        validate_key("realtime path", segment)?;
    }
    let mut url = json_url(&root, &segments)?;
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
    url.query_pairs_mut().append_pair("auth", &token);
    let client = reqwest::Client::builder()
        .redirect(redirect_policy(root.host_str().unwrap_or_default()))
        .connect_timeout(Duration::from_secs(10))
        .build()?;
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
    let mut stream = response.bytes_stream();
    let mut decoder = Decoder::default();
    ready(healthy, wake);
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(90), stream.next())
            .await
            .map_err(|_| anyhow!("Realtime Firebase stream became idle"))?
            .ok_or_else(|| anyhow!("Realtime Firebase stream closed"))?
            .map_err(|_| anyhow!("Realtime Firebase stream failed"))?;
        for (kind, data) in decoder.push(&chunk)? {
            if matches!(kind.as_str(), "cancel" | "auth_revoked") {
                bail!("Realtime Firebase stream was revoked");
            }
            if wakeup(&kind, &data) {
                wake.notify_one();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gcp::allows_redirect;

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
