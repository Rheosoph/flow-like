#[path = "support/header_proxy.rs"]
mod header_proxy;
#[path = "support/tls.rs"]
mod tls_support;

use std::sync::Arc;
use std::time::Duration;

use flow_like_browser::transport::ws::{ConnectOptions, WsTransport};
use flow_like_browser::transport::{Inbound, Outbound, Transport, WriteTicket};
use rustls::crypto::CryptoProvider;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_connects_without_installing_a_process_crypto_provider() {
    assert!(
        CryptoProvider::get_default().is_none(),
        "nothing may install a process default before this test"
    );
    let ca = tls_support::test_ca();
    let upstream = header_proxy::start_upstream().await;
    let proxy =
        header_proxy::start(upstream.url, None, Some(tls_support::server_config(&ca))).await;
    let mut options = ConnectOptions::new(&proxy.url);
    options.extra_root_certificates = vec![ca.ca_der.clone()];

    let transport = WsTransport::connect(&options)
        .await
        .unwrap_or_else(|error| panic!("wss connect with ring and aws-lc-rs linked: {error}"));
    let mut channels = Box::new(transport).start();
    let sent = channels.outbound.send(Outbound {
        text: "guard".to_owned(),
        method: Arc::from("Test.guard"),
        deadline: None,
        ticket: WriteTicket::default(),
    });
    assert!(sent.is_ok(), "the transport writer is running");
    let reply = tokio::time::timeout(Duration::from_secs(10), channels.inbound.recv())
        .await
        .expect("an echo within 10 s");
    assert!(matches!(reply, Some(Inbound::Text(text)) if text == "guard"));
    channels.control.close().await;

    assert!(
        CryptoProvider::get_default().is_none(),
        "WsTransport must not install a process-level CryptoProvider"
    );
}
