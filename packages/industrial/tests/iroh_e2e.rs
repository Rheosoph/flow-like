#![cfg(feature = "execute")]

use flow_like_industrial::iroh::{
    IrohConfig, IrohEndpoint, IrohPeer, MAX_FRAME_BYTES, allowed_peers,
};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Reference {
    child: Child,
    peer: IrohPeer,
}

impl Reference {
    fn start(mode: &str) -> Self {
        let binary = std::env::var("FLOW_LIKE_IROH_REFERENCE")
            .expect("Set FLOW_LIKE_IROH_REFERENCE to the separate raw-SDK fixture peer binary");
        let mut child = Command::new(binary)
            .arg(mode)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = tx.send(result);
        });
        let ready = rx.recv_timeout(Duration::from_secs(20));
        let peer = match ready {
            Ok(Ok(line)) => serde_json::from_str(&line),
            error => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("reference peer did not become ready: {error:?}");
            }
        };
        Self {
            child,
            peer: peer.unwrap(),
        }
    }

    fn connect_to(&mut self, peer: &IrohPeer) {
        let input = self.child.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *input, peer).unwrap();
        writeln!(input).unwrap();
        input.flush().unwrap();
    }

    async fn finish(&mut self) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    assert!(status.success(), "reference peer failed: {status}");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("reference peer failed to close");
    }
}

impl Drop for Reference {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn endpoint(timeout_ms: u64) -> IrohEndpoint {
    IrohEndpoint::bind(IrohConfig {
        bind_address: "127.0.0.1:0".into(),
        public_relay: false,
        timeout_ms,
        ..Default::default()
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the Docker raw-SDK peer and FLOW_LIKE_IROH_REFERENCE"]
async fn raw_sdk_binary_frames_acknowledgements_and_close() {
    let mut reference = Reference::start("echo");
    let endpoint = endpoint(10_000).await;
    assert!(endpoint.peer().relay_url.is_none());
    assert!(reference.peer.relay_url.is_none());
    let connection = endpoint.connect(reference.peer.clone()).await.unwrap();
    assert_eq!(connection.peer_id(), reference.peer.endpoint_id);
    for length in [0, 256, 1024 * 1024, MAX_FRAME_BYTES] {
        let payload: Vec<u8> = (0..length).map(|n| (n % 251) as u8).collect();
        connection.send(&payload).await.unwrap();
        let received = tokio::time::timeout(Duration::from_secs(15), connection.receive())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received.peer_id, reference.peer.endpoint_id);
        assert_eq!(received.payload, payload);
    }
    assert!(
        connection
            .send(&vec![0; MAX_FRAME_BYTES + 1])
            .await
            .unwrap_err()
            .to_string()
            .contains("16 MiB")
    );
    connection.close();
    connection.close();
    assert!(connection.send(&[42]).await.is_err());
    assert!(connection.receive().await.is_err());
    endpoint.close().await;
    reference.finish().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the Docker raw-SDK peer and FLOW_LIKE_IROH_REFERENCE"]
async fn raw_sdk_malformed_frames_and_acknowledgements_are_rejected() {
    for (mode, expected) in [
        ("oversize", "exceeding 16 MiB"),
        ("truncated", ""),
        ("trailing", "trailing bytes"),
        ("short-header", ""),
        ("bad-ack", "invalid message acknowledgement"),
        ("trailing-ack", "acknowledgement contains trailing bytes"),
        ("silent-ack", "timed out"),
    ] {
        let mut reference = Reference::start(mode);
        let endpoint = endpoint(1_000).await;
        let connection = endpoint.connect(reference.peer.clone()).await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(5), async {
            if mode.ends_with("ack") {
                connection.send(&[0, 128, 255]).await.map(|_| ())
            } else {
                connection.receive().await.map(|_| ())
            }
        })
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.to_string().contains(expected), "{mode}: {error}");
        connection.close();
        endpoint.close().await;
        reference.finish().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the Docker raw-SDK peer and FLOW_LIKE_IROH_REFERENCE"]
async fn raw_sdk_unknown_identity_is_rejected_and_listener_accepts_allowed_peer() {
    let mut denied = Reference::start("client-denied");
    let mut trusted = Reference::start("client-allowed");
    let endpoint = endpoint(5_000).await;
    let allowlist = allowed_peers(&[trusted.peer.endpoint_id.clone()]).unwrap();
    let accept = endpoint.accept(&allowlist);
    tokio::pin!(accept);
    denied.connect_to(&endpoint.peer());
    tokio::select! {
        result = &mut accept => panic!("unallowed peer was accepted: {}", result.is_ok()),
        _ = denied.finish() => {}
    }
    trusted.connect_to(&endpoint.peer());
    let connection = tokio::time::timeout(Duration::from_secs(10), accept)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(connection.peer_id(), trusted.peer.endpoint_id);
    assert_eq!(connection.receive().await.unwrap().payload, b"trusted peer");
    connection.close();
    trusted.finish().await;
    endpoint.close().await;
}
