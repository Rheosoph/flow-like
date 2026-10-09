#![cfg(feature = "opcua")]

use flow_like_industrial::opcua::{
    OpcUaBrowseRequest, OpcUaClient, OpcUaConnectConfig, OpcUaCredentials, OpcUaReadRequest,
    OpcUaSecurityMode, OpcUaSubscribeRequest, OpcUaValue, OpcUaWriteItem, OpcUaWriteValue,
};
use opcua::crypto::{CertificateStore, X509};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

// The Compose fixture runs Python asyncua, independently of our Rust OPC UA SDK.
// These tests require explicit fixture settings so an unavailable server fails the run.
struct Fixture {
    pki: PathBuf,
    config: OpcUaConnectConfig,
    server_certificate: Vec<u8>,
    server_certificate_name: String,
}

impl Fixture {
    fn new(secure: bool, trust_server: bool) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let endpoint = std::env::var("INDUSTRIAL_OPCUA_ENDPOINT")
            .expect("set INDUSTRIAL_OPCUA_ENDPOINT to the asyncua Compose fixture");
        let certificates = PathBuf::from(
            std::env::var("INDUSTRIAL_OPCUA_CERT_DIR")
                .expect("copy opcua:/fixture-pki and set INDUSTRIAL_OPCUA_CERT_DIR"),
        );
        let pki = std::env::temp_dir().join(format!(
            "flow-like-opcua-e2e-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed),
        ));
        for directory in ["own", "private", "trusted", "rejected"] {
            fs::create_dir_all(pki.join(directory)).unwrap();
        }
        fs::copy(certificates.join("client.der"), pki.join("own/cert.der")).unwrap();
        fs::copy(
            certificates.join("client.pem"),
            pki.join("private/private.pem"),
        )
        .unwrap();
        let server_certificate = fs::read(certificates.join("server.der")).unwrap();
        let server_certificate_name =
            CertificateStore::cert_file_name(&X509::from_der(&server_certificate).unwrap());
        if trust_server {
            fs::write(
                pki.join("trusted").join(&server_certificate_name),
                &server_certificate,
            )
            .unwrap();
        }
        let config = OpcUaConnectConfig {
            endpoint,
            pki_dir: pki.to_str().unwrap().to_owned(),
            security_policy: if secure { "Basic256Sha256" } else { "None" }.into(),
            security_mode: if secure {
                OpcUaSecurityMode::SignAndEncrypt
            } else {
                OpcUaSecurityMode::None
            },
            timeout_ms: 10_000,
        };
        Self {
            pki,
            config,
            server_certificate,
            server_certificate_name,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.pki);
    }
}

fn credentials() -> OpcUaCredentials {
    OpcUaCredentials {
        username: "flow-like-test".into(),
        password: "fixture-password".into(),
    }
}

fn node(group: &str, name: &str) -> String {
    format!("ns=2;s={group}.{name}")
}

fn scalar_cases() -> Vec<(&'static str, OpcUaWriteValue, OpcUaValue)> {
    vec![
        (
            "Boolean",
            OpcUaWriteValue::Boolean(true),
            OpcUaValue::Boolean(true),
        ),
        (
            "Byte",
            OpcUaWriteValue::Byte(255),
            OpcUaValue::Unsigned(255),
        ),
        (
            "SByte",
            OpcUaWriteValue::SByte(-128),
            OpcUaValue::Signed(-128),
        ),
        (
            "Int16",
            OpcUaWriteValue::Int16(i16::MIN),
            OpcUaValue::Signed(i16::MIN.into()),
        ),
        (
            "UInt16",
            OpcUaWriteValue::UInt16(u16::MAX),
            OpcUaValue::Unsigned(u16::MAX.into()),
        ),
        (
            "Int32",
            OpcUaWriteValue::Int32(i32::MIN),
            OpcUaValue::Signed(i32::MIN.into()),
        ),
        (
            "UInt32",
            OpcUaWriteValue::UInt32(u32::MAX),
            OpcUaValue::Unsigned(u32::MAX.into()),
        ),
        (
            "Int64",
            OpcUaWriteValue::Int64(i64::MIN),
            OpcUaValue::Signed(i64::MIN),
        ),
        (
            "UInt64",
            OpcUaWriteValue::UInt64(u64::MAX),
            OpcUaValue::Unsigned(u64::MAX),
        ),
        (
            "Float",
            OpcUaWriteValue::Float(-12.5),
            OpcUaValue::Float(-12.5),
        ),
        (
            "Double",
            OpcUaWriteValue::Double(1234.125),
            OpcUaValue::Float(1234.125),
        ),
        (
            "String",
            OpcUaWriteValue::String("温度 °C = 21.5".into()),
            OpcUaValue::Text("温度 °C = 21.5".into()),
        ),
    ]
}

async fn exercise_services(secure: bool) {
    let fixture = Fixture::new(secure, true);
    let credentials = secure.then(credentials);
    let group = if secure { "Secure" } else { "Anonymous" };
    let client = OpcUaClient::connect(&fixture.config, credentials.as_ref())
        .await
        .expect("connect to the independent asyncua server");

    let cases = scalar_cases();
    let browse = client
        .browse(&OpcUaBrowseRequest {
            node_id: format!("ns=2;s={group}"),
            max_references: 64,
        })
        .await
        .unwrap();
    assert!(!browse.truncated);
    for (name, _, _) in &cases {
        assert!(
            browse.references.iter().any(|reference| {
                reference.node_id == node(group, name)
                    && reference.display_name == *name
                    && reference.node_class == "Variable"
            }),
            "missing browsed scalar {name}: {:?}",
            browse.references,
        );
    }

    let writes = cases
        .iter()
        .map(|(name, value, _)| OpcUaWriteItem {
            node_id: node(group, name),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    let statuses = client.write(&writes).await.unwrap();
    assert_eq!(statuses.len(), writes.len());
    for (status, write) in statuses.iter().zip(&writes) {
        assert_eq!(status.node_id, write.node_id);
        assert!(status.good, "{status:?}");
    }
    let read_request = OpcUaReadRequest {
        node_ids: cases
            .iter()
            .map(|(name, _, _)| {
                browse
                    .references
                    .iter()
                    .find(|reference| reference.display_name == *name)
                    .unwrap()
                    .node_id
                    .clone()
            })
            .collect(),
        max_age_ms: 0.0,
    };
    let readings = client.read(&read_request).await.unwrap();
    assert_eq!(readings.len(), cases.len());
    for (reading, (name, _, expected)) in readings.iter().zip(&cases) {
        assert_eq!(reading.node_id, node(group, name));
        assert!(reading.quality_good, "{reading:?}");
        assert_eq!(reading.value.as_ref(), Some(expected), "{name}");
    }

    let mixed = client
        .read(&OpcUaReadRequest {
            node_ids: vec![node(group, "ReadOnly"), node(group, "Missing")],
            max_age_ms: 0.0,
        })
        .await
        .unwrap();
    assert_eq!(mixed[0].value, Some(OpcUaValue::Signed(42)));
    assert!(mixed[0].quality_good);
    assert!(!mixed[1].quality_good);
    assert!(
        mixed[1].status.contains("BadNodeIdUnknown"),
        "{:?}",
        mixed[1]
    );
    assert_eq!(mixed[1].value, None);

    let invalid = client
        .write(&[
            OpcUaWriteItem {
                node_id: node(group, "Int32"),
                value: OpcUaWriteValue::String("wrong type".into()),
            },
            OpcUaWriteItem {
                node_id: node(group, "ReadOnly"),
                value: OpcUaWriteValue::Int32(0),
            },
        ])
        .await
        .unwrap();
    assert!(
        !invalid[0].good && invalid[0].status.contains("BadTypeMismatch"),
        "{invalid:?}"
    );
    assert!(
        !invalid[1].good && invalid[1].status.contains("BadUserAccessDenied"),
        "{invalid:?}"
    );
    assert!(
        client
            .browse(&OpcUaBrowseRequest {
                node_id: node(group, "Missing"),
                max_references: 64,
            })
            .await
            .is_err()
    );
    match client
        .subscribe(&OpcUaSubscribeRequest {
            node_ids: vec![node(group, "Missing")],
            interval_ms: 50,
            queue_capacity: 16,
        })
        .await
    {
        Ok(_) => panic!("monitoring an unknown node must fail"),
        Err(error) => assert!(error.to_string().contains("BadNodeIdUnknown"), "{error}"),
    }

    let mut subscription = client
        .subscribe(&OpcUaSubscribeRequest {
            node_ids: vec![node(group, "String")],
            interval_ms: 50,
            queue_capacity: 16,
        })
        .await
        .unwrap();
    let initial = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("server should publish an initial data change")
        .unwrap();
    assert_eq!(initial.node_id, node(group, "String"));
    assert_eq!(
        initial.value,
        Some(OpcUaValue::Text("温度 °C = 21.5".into()))
    );

    let changed = "line/2: pump ✓";
    let status = client
        .write(&[OpcUaWriteItem {
            node_id: node(group, "String"),
            value: OpcUaWriteValue::String(changed.into()),
        }])
        .await
        .unwrap();
    assert!(status[0].good);
    let notification = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("server should publish the write without a client read")
        .unwrap();
    assert_eq!(notification.node_id, node(group, "String"));
    assert!(notification.quality_good);
    assert_eq!(notification.value, Some(OpcUaValue::Text(changed.into())));
    subscription.close().await.unwrap();
    drop(subscription);
    client.disconnect().await.unwrap();
    assert!(client.read(&read_request).await.is_err());
    drop(client);

    let reconnected = OpcUaClient::connect(&fixture.config, credentials.as_ref())
        .await
        .expect("a new session should connect after explicit disconnect");
    let persisted = reconnected
        .read(&OpcUaReadRequest {
            node_ids: vec![node(group, "String")],
            max_age_ms: 0.0,
        })
        .await
        .unwrap();
    assert_eq!(persisted[0].value, Some(OpcUaValue::Text(changed.into())));
    reconnected.disconnect().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the asyncua Compose fixture and INDUSTRIAL_OPCUA_* settings"]
async fn opcua_anonymous_browse_scalars_statuses_subscription_and_reconnect() {
    exercise_services(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the asyncua Compose fixture and INDUSTRIAL_OPCUA_* settings"]
async fn opcua_encrypted_authenticated_browse_scalars_statuses_subscription_and_reconnect() {
    exercise_services(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the asyncua Compose fixture and INDUSTRIAL_OPCUA_* settings"]
async fn opcua_rejects_untrusted_server_certificate() {
    let fixture = Fixture::new(true, false);
    let result = OpcUaClient::connect(&fixture.config, Some(&credentials())).await;
    assert!(
        result.is_err(),
        "an untrusted server must not establish a session"
    );
    assert_eq!(
        fs::read(
            fixture
                .pki
                .join("rejected")
                .join(&fixture.server_certificate_name)
        )
        .expect("the actual server certificate must be quarantined"),
        fixture.server_certificate,
    );
    assert_eq!(
        fs::read_dir(fixture.pki.join("trusted")).unwrap().count(),
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the asyncua Compose fixture and INDUSTRIAL_OPCUA_* settings"]
async fn opcua_rejects_wrong_password_then_accepts_correct_credentials() {
    let fixture = Fixture::new(true, true);
    let mut wrong = credentials();
    wrong.password = "incorrect-fixture-password".into();
    let result = OpcUaClient::connect(&fixture.config, Some(&wrong)).await;
    assert!(result.is_err(), "an incorrect password must be rejected");
    drop(result);

    let client = OpcUaClient::connect(&fixture.config, Some(&credentials()))
        .await
        .expect("correct credentials must work against the same server");
    let readings = client
        .read(&OpcUaReadRequest {
            node_ids: vec![node("Secure", "ReadOnly")],
            max_age_ms: 0.0,
        })
        .await
        .unwrap();
    assert_eq!(readings[0].value, Some(OpcUaValue::Signed(42)));
    client.disconnect().await.unwrap();
}
