#![cfg(feature = "execute")]

use flow_like_industrial::hart::{Connection, HartAddress, HartCommand, HartConfig, HartTransport};
use std::time::Duration;

fn config() -> HartConfig {
    let endpoint: std::net::SocketAddr = std::env::var("FLOW_LIKE_HART_E2E_ADDR")
        .expect("Set FLOW_LIKE_HART_E2E_ADDR to the dedicated FieldComm HART-IP fixture")
        .parse()
        .expect("FLOW_LIKE_HART_E2E_ADDR must be an IP address and port");
    HartConfig {
        transport: HartTransport::HartIpV1 {
            host: endpoint.ip().to_string(),
            port: endpoint.port(),
            inactivity_timeout_ms: 30_000,
        },
        timeout_ms: 3_000,
        queue_capacity: 8,
        preambles: 5,
    }
}

fn command(number: u16, data: Vec<u8>) -> HartCommand {
    HartCommand {
        address: HartAddress::Unique {
            expanded_device_type: 0x1234,
            device_id: 0x010203,
        },
        secondary_master: false,
        command: number,
        data,
    }
}

#[tokio::test]
#[ignore = "requires the Docker FieldComm fixture and FLOW_LIKE_HART_E2E_ADDR; run serially"]
async fn fieldcomm_identity_measurement_expanded_command_and_binary_payload() {
    let connection = Connection::connect(&config()).await.unwrap();
    let identity = connection
        .command(&HartCommand {
            address: HartAddress::Polling { node: 0 },
            ..command(0, vec![])
        })
        .await
        .unwrap();
    assert_eq!(identity.command, 0);
    assert_eq!(identity.response_code, 0);
    assert_eq!(identity.device_status, 0);
    assert_eq!(&identity.data[..3], &[254, 0x12, 0x34]);
    assert_eq!(&identity.data[9..12], &[1, 2, 3]);
    assert!(!identity.burst);
    assert_eq!(
        identity.response_preambles, 0,
        "HART-IP has no serial preambles"
    );

    let measurement = connection.command(&command(1, vec![])).await.unwrap();
    assert_eq!(measurement.command, 1);
    assert_eq!(measurement.response_code, 0);
    assert_eq!(measurement.data, [32, 0x41, 0xac, 0, 0]);
    assert_eq!(
        f32::from_be_bytes(measurement.data[1..].try_into().unwrap()),
        21.5
    );

    let tag = connection.command(&command(520, vec![])).await.unwrap();
    assert_eq!(tag.command, 520);
    assert_eq!(tag.response_code, 0);
    let mut expected = b"flow-like-fixture".to_vec();
    expected.resize(32, 0);
    assert_eq!(tag.data, expected);

    for payload in [vec![], vec![0, 255, 2, 128, 31], (0..=127).collect()] {
        let echo = connection
            .command(&command(128, payload.clone()))
            .await
            .unwrap();
        assert_eq!(echo.command, 128);
        assert_eq!(echo.response_code, 0);
        assert_eq!(echo.data, payload);
    }
    connection.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the Docker FieldComm fixture and FLOW_LIKE_HART_E2E_ADDR; run serially"]
async fn fieldcomm_command_errors_idle_close_and_reconnect() {
    let connection = Connection::connect(&config()).await.unwrap();
    let unsupported = connection.command(&command(255, vec![])).await.unwrap();
    assert_eq!(unsupported.command, 255);
    assert_eq!(unsupported.response_code, 64);
    assert_eq!(unsupported.device_status, 0);
    assert!(unsupported.data.is_empty());

    // The link runner must remain usable while no HART-IP response is pending.
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        connection
            .command(&command(1, vec![]))
            .await
            .unwrap()
            .response_code,
        0
    );
    connection.close().await.unwrap();
    connection.close().await.unwrap();
    assert!(connection.command(&command(1, vec![])).await.is_err());

    let reconnected = Connection::connect(&config()).await.unwrap();
    assert_eq!(
        reconnected.command(&command(1, vec![])).await.unwrap().data,
        [32, 0x41, 0xac, 0, 0]
    );
    reconnected.close().await.unwrap();
}
