#![cfg(feature = "execute")]

// Prism validates requests and returns examples from the official JSON 2.0.0
// contract. These tests do not establish physical master interoperability.
use flow_like_industrial::iolink::{
    IolinkClient, IolinkConfig, IolinkData, IolinkParameter, IolinkProcessData,
};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::json;

fn origin() -> String {
    std::env::var("FLOW_LIKE_IOLINK_CONTRACT_URL")
        .expect("Set FLOW_LIKE_IOLINK_CONTRACT_URL to the dedicated Docker contract fixture")
}

fn client(http: reqwest::Client) -> IolinkClient {
    IolinkClient::with_client(
        IolinkConfig {
            origin: origin(),
            timeout_ms: 5_000,
            ..Default::default()
        },
        http,
    )
    .unwrap()
}

#[tokio::test]
#[ignore = "requires the official IO-Link v2 Docker contract fixture"]
async fn official_examples_decode_devices_process_data_and_isdu() {
    let connection = client(reqwest::Client::new());
    let devices = connection.devices().await.unwrap();
    assert_eq!(devices.len(), 4);
    assert_eq!(devices[0].device_alias, "DT35");
    assert_eq!((devices[0].master_number, devices[0].port_number), (1, 1));
    assert_eq!(
        devices[3].iodd_file_name.as_deref(),
        Some("vendorname-devicename-20231016-IODD1.1.xml")
    );
    let process = connection.read_process_data("DT35").await.unwrap();
    let data = process.iolink.unwrap();
    assert!(data.valid);
    assert_eq!(data.value, [12, 22, 216]);
    assert_eq!(process.iq_value, Some(true));
    for sub_index in [None, Some(0), Some(1)] {
        assert_eq!(
            connection
                .read_parameter(
                    "DT35",
                    &IolinkParameter {
                        index: 16,
                        sub_index
                    }
                )
                .await
                .unwrap(),
            [0, 156, 125, 25]
        );
    }
    connection.close();
    connection.close();
    assert!(connection.devices().await.is_err());
}

#[tokio::test]
#[ignore = "requires the official IO-Link v2 Docker contract fixture"]
async fn official_schema_accepts_adapter_writes_and_rejects_invalid_wire_values() {
    let http = reqwest::Client::new();
    let connection = client(http.clone());
    for sub_index in [None, Some(0), Some(1)] {
        connection
            .write_parameter(
                "DT35",
                &IolinkParameter {
                    index: 16,
                    sub_index,
                },
                vec![0, 1, 128, 255],
            )
            .await
            .unwrap();
    }
    connection
        .write_process_data(
            "DT35",
            IolinkProcessData {
                iolink: Some(IolinkData {
                    valid: true,
                    value: vec![0, 128, 255],
                }),
                iq_value: Some(true),
                cq_value: None,
            },
        )
        .await
        .unwrap();
    connection
        .write_process_data(
            "DT35",
            IolinkProcessData {
                cq_value: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // Negative controls demonstrate that the independent schema validator is
    // active. A permissive HTTP stub would incorrectly accept these bodies.
    let url = format!("{}/iolink/v2/devices/DT35/parameters/16/value", origin());
    for invalid in [json!({"value": [256]}), json!([0, 255])] {
        let response = http.post(&url).json(&invalid).send().await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::BAD_REQUEST);
    }
    connection.close();
}

#[tokio::test]
#[ignore = "requires the official IO-Link v2 Docker contract fixture"]
async fn official_error_body_survives_adapter_mapping() {
    let mut headers = HeaderMap::new();
    headers.insert("Prefer", HeaderValue::from_static("code=404"));
    let connection = client(
        reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap(),
    );
    let error = connection
        .read_parameter(
            "DT35",
            &IolinkParameter {
                index: 16,
                sub_index: None,
            },
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("HTTP 404"), "{error}");
    assert!(
        error.contains("code 103: Operation not supported"),
        "{error}"
    );
    connection.close();
}
