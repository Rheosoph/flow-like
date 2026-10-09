#![cfg(feature = "execute")]

use flow_like_industrial::ads::{
    AdsAddress, AdsClient, AdsConfig, AdsNotificationConfig, AdsSubscription,
};
use std::time::Duration;

fn config() -> AdsConfig {
    let endpoint: std::net::SocketAddr = std::env::var("FLOW_LIKE_ADS_E2E_ADDR")
        .expect("Set FLOW_LIKE_ADS_E2E_ADDR to the dedicated Beckhoff ADS fixture")
        .parse()
        .expect("FLOW_LIKE_ADS_E2E_ADDR must be an IP address and port");
    AdsConfig {
        host: endpoint.ip().to_string(),
        tcp_port: endpoint.port(),
        target_net_id: [42, 42, 42, 42, 1, 1],
        target_port: 25000,
        source_net_id: Some([10, 11, 12, 13, 1, 1]),
        source_port: 59001,
        timeout_ms: 5_000,
    }
}

fn symbol(name: &str) -> AdsAddress {
    AdsAddress::Symbol {
        name: format!("Globals.{name}"),
    }
}

fn index(offset: u32) -> AdsAddress {
    AdsAddress::Index { group: 2, offset }
}

#[tokio::test]
#[ignore = "requires the Docker Beckhoff fixture and FLOW_LIKE_ADS_E2E_ADDR; run serially"]
async fn beckhoff_symbols_and_index_addresses_share_server_storage() {
    let client = AdsClient::connect(config()).await.unwrap();
    let mut unicode = "Grüße 水"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    unicode.resize(162, 0);
    let cases = [
        ("int1", 0x1001, (-30_123i16).to_le_bytes().to_vec()),
        ("dint1", 0x1003, (-2_000_000_123i32).to_le_bytes().to_vec()),
        ("real1", 0x1007, (-123.75f32).to_le_bytes().to_vec()),
        ("lreal1", 0x100b, (1_234_567.125f64).to_le_bytes().to_vec()),
        ("string1", 0x1013, unicode),
    ];
    for (name, offset, data) in &cases {
        client.write(symbol(name), data.clone()).await.unwrap();
        assert_eq!(
            client
                .read(index(*offset), data.len() as u32)
                .await
                .unwrap(),
            *data,
            "{name}"
        );
        assert_eq!(
            client.read(symbol(name), data.len() as u32).await.unwrap(),
            *data,
            "{name}"
        );
    }
    client
        .write(index(0x1003), 314159i32.to_le_bytes().to_vec())
        .await
        .unwrap();
    assert_eq!(
        client.read(symbol("dint1"), 4).await.unwrap(),
        314159i32.to_le_bytes()
    );
    client.close().await.unwrap();
    client.close().await.unwrap();
    assert!(client.read(symbol("dint1"), 4).await.is_err());
    let reconnected = AdsClient::connect(config()).await.unwrap();
    assert_eq!(
        reconnected.read(symbol("dint1"), 4).await.unwrap(),
        314159i32.to_le_bytes()
    );
    reconnected.close().await.unwrap();
}

async fn receive_value(subscription: &mut AdsSubscription, expected: &[u8]) -> u32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let sample = subscription.receive().await.unwrap();
            assert!(
                sample.timestamp > 0,
                "vendor notification must carry a FILETIME"
            );
            if sample.data == expected {
                return sample.handle;
            }
        }
    })
    .await
    .expect("vendor server did not send the expected wire notification")
}

#[tokio::test]
#[ignore = "requires the Docker Beckhoff fixture and FLOW_LIKE_ADS_E2E_ADDR; run serially"]
async fn beckhoff_wire_notifications_route_and_cleanup_on_the_shared_connection() {
    let client = AdsClient::connect(config()).await.unwrap();
    let first_data = 12345i16.to_le_bytes();
    let second_data = 123456789i32.to_le_bytes();
    client
        .write(symbol("int1"), 0i16.to_le_bytes().to_vec())
        .await
        .unwrap();
    client
        .write(symbol("dint1"), 0i32.to_le_bytes().to_vec())
        .await
        .unwrap();
    let mut first = client
        .subscribe(AdsNotificationConfig {
            address: symbol("int1"),
            length: 2,
            cycle_ms: 20,
            on_change: true,
        })
        .await
        .unwrap();
    let mut second = client
        .subscribe(AdsNotificationConfig {
            address: symbol("dint1"),
            length: 4,
            cycle_ms: 200,
            on_change: false,
        })
        .await
        .unwrap();
    client
        .write(symbol("int1"), first_data.to_vec())
        .await
        .unwrap();
    client
        .write(index(0x1003), second_data.to_vec())
        .await
        .unwrap();
    let first_handle = receive_value(&mut first, &first_data).await;
    let second_handle = receive_value(&mut second, &second_data).await;
    assert_ne!(first_handle, second_handle);
    let (earlier, later) = tokio::time::timeout(Duration::from_secs(5), async {
        (
            second.receive().await.unwrap(),
            second.receive().await.unwrap(),
        )
    })
    .await
    .unwrap();
    assert_eq!(earlier.data, second_data);
    assert_eq!(later.data, second_data);
    assert!(
        later.timestamp.saturating_sub(earlier.timestamp) >= 1_500_000,
        "a 200 ms subscription must not be serialized as 200 wire ticks"
    );
    drop(first);
    let changed = (-7654321i32).to_le_bytes();
    client
        .write(symbol("dint1"), changed.to_vec())
        .await
        .unwrap();
    assert_eq!(receive_value(&mut second, &changed).await, second_handle);
    client.close().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while second.receive().await.is_ok() {}
    })
    .await
    .expect("closing the client must close its notification stream");
}

#[tokio::test]
#[ignore = "requires the Docker Beckhoff fixture and FLOW_LIKE_ADS_E2E_ADDR; run serially"]
async fn beckhoff_service_errors_invalidate_session_and_reconnect() {
    for address in [
        symbol("missing"),
        AdsAddress::Index {
            group: 0xdead,
            offset: 0,
        },
    ] {
        let client = AdsClient::connect(config()).await.unwrap();
        let error = client.read(address, 4).await.unwrap_err();
        assert!(!error.to_string().is_empty());
        assert!(client.read(symbol("int1"), 2).await.is_err());
        let _ = client.close().await;
    }
    let client = AdsClient::connect(config()).await.unwrap();
    assert_eq!(client.read(symbol("int1"), 2).await.unwrap().len(), 2);
    client.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the Docker Beckhoff fixture and FLOW_LIKE_ADS_E2E_ADDR; run serially"]
async fn beckhoff_unsupported_index_notification_fails_and_reconnects() {
    let client = AdsClient::connect(config()).await.unwrap();
    // This vendor symbolic server only accepts notifications through F005 handles.
    let error = match client
        .subscribe(AdsNotificationConfig {
            address: index(0x1001),
            length: 2,
            cycle_ms: 20,
            on_change: true,
        })
        .await
    {
        Ok(_) => panic!("vendor fixture unexpectedly accepted process-image notifications"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("0x701"), "{error}");
    assert!(client.read(symbol("int1"), 2).await.is_err());
    let reconnected = AdsClient::connect(config()).await.unwrap();
    assert_eq!(reconnected.read(symbol("int1"), 2).await.unwrap().len(), 2);
    reconnected.close().await.unwrap();
}
