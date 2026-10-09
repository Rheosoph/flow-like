#![cfg(feature = "execute")]

use flow_like_industrial::cip::{CipClient, CipConfig, CipValue};

fn config() -> CipConfig {
    let endpoint: std::net::SocketAddr = std::env::var("FLOW_LIKE_CIP_E2E_ADDR")
        .expect("Set FLOW_LIKE_CIP_E2E_ADDR to the dedicated libplctag ab_server fixture")
        .parse()
        .expect("FLOW_LIKE_CIP_E2E_ADDR must be an IP address and port");
    CipConfig {
        host: endpoint.ip().to_string(),
        port: endpoint.port(),
        slot: Some(0),
        timeout_ms: 5_000,
    }
}

#[tokio::test]
#[ignore = "requires the Docker libplctag fixture and FLOW_LIKE_CIP_E2E_ADDR"]
async fn libplctag_scalar_and_indexed_tags_round_trip_across_sessions() {
    let writer = CipClient::connect(config()).await.unwrap();
    let observer = CipClient::connect(config()).await.unwrap();
    let values = [
        ("Bool[1]", CipValue::Bool(true)),
        ("Sint[1]", CipValue::Sint(-113)),
        ("Int[1]", CipValue::Int(-30_123)),
        ("Dint[1]", CipValue::Dint(-2_000_000_123)),
        ("Lint[1]", CipValue::Lint(-8_000_000_000_000_123)),
        ("Real[1]", CipValue::Real(-123.75)),
        ("Lreal[1]", CipValue::Lreal(1_234_567.125)),
        ("Text[1]", CipValue::String("Flow-like Logix STRING".into())),
    ];
    for (tag, value) in &values {
        writer
            .write(tag, value.clone())
            .await
            .unwrap_or_else(|error| panic!("write {tag}: {error}"));
        assert_eq!(observer.read(tag).await.unwrap(), *value, "{tag}");
    }
    writer
        .write("Bool[1]", CipValue::Bool(false))
        .await
        .unwrap();
    assert_eq!(
        observer.read("Bool[1]").await.unwrap(),
        CipValue::Bool(false)
    );
    // An adjacent array element must retain its initial value.
    assert_eq!(observer.read("Dint[0]").await.unwrap(), CipValue::Dint(0));
    writer.close().await;
    writer.close().await;
    assert!(writer.read("Dint[1]").await.is_err());
    observer.close().await;

    let reconnected = CipClient::connect(config()).await.unwrap();
    assert_eq!(reconnected.read("Dint[1]").await.unwrap(), values[3].1);
    reconnected.close().await;
}

#[tokio::test]
#[ignore = "requires the Docker libplctag fixture and FLOW_LIKE_CIP_E2E_ADDR"]
async fn libplctag_service_errors_close_session_and_allow_reconnect() {
    for tag in ["MissingTag", "Dint[99]"] {
        let client = CipClient::connect(config()).await.unwrap();
        let error = client.read(tag).await.unwrap_err();
        assert!(!error.to_string().is_empty());
        assert!(
            client
                .read("Dint[0]")
                .await
                .unwrap_err()
                .to_string()
                .contains("session closed")
        );
        client.close().await;
    }
    let client = CipClient::connect(config()).await.unwrap();
    assert_eq!(client.read("Dint[0]").await.unwrap(), CipValue::Dint(0));
    assert!(client.write("Dint[0]", CipValue::Real(1.0)).await.is_err());
    assert!(client.read("Dint[0]").await.is_err());
    client.close().await;
    let observer = CipClient::connect(config()).await.unwrap();
    assert_eq!(observer.read("Dint[0]").await.unwrap(), CipValue::Dint(0));
    observer.close().await;
}
