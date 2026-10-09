#![cfg(feature = "execute")]

use flow_like_industrial::zenoh::{ZenohClient, ZenohConfig, ZenohMode};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Fixture {
    router: String,
    oracle: String,
    http: reqwest::Client,
}

impl Fixture {
    async fn new() -> Self {
        let fixture = Self {
            router: std::env::var("FLOW_LIKE_ZENOH_ROUTER")
                .expect("Set FLOW_LIKE_ZENOH_ROUTER to the dedicated Zenoh router"),
            oracle: std::env::var("FLOW_LIKE_ZENOH_ORACLE_URL")
                .expect("Set FLOW_LIKE_ZENOH_ORACLE_URL to the Python reference peer"),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        };
        assert_eq!(fixture.get("/health", &[]).await["ready"], true);
        fixture
    }

    async fn connect(&self) -> ZenohClient {
        ZenohClient::connect(ZenohConfig {
            mode: ZenohMode::Client,
            connect: vec![self.router.clone()],
            multicast_discovery: false,
            timeout_ms: 5_000,
            ..Default::default()
        })
        .await
        .unwrap()
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Value {
        self.http
            .get(format!("{}{path}", self.oracle))
            .query(query)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn publish(&self, path: &str, body: Value) {
        self.http
            .post(format!("{}{path}", self.oracle))
            .json(&body)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }

    async fn observe(&self, key: &str, deleted: bool) -> Value {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let samples = self.get("/samples", &[("key", key)]).await;
                if let Some(sample) = samples
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|sample| sample["deleted"] == deleted)
                {
                    return sample.clone();
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("reference peer must observe the routed sample")
    }
}

fn unique() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the official Zenoh router and Python reference peer fixtures"]
async fn zenoh_router_routes_binary_put_delete_and_reconnected_subscriptions() {
    let fixture = Fixture::new().await;
    let client = fixture.connect().await;
    let id = unique();
    let received_key = format!("flow-like/e2e/peer/{id}/温度");
    let subscription = client
        .subscribe(&format!("flow-like/e2e/peer/{id}/*"))
        .await
        .unwrap();
    fixture
        .publish(
            "/publish",
            json!({"key":received_key,"payload":[0,255,128,42],"encoding":"application/octet-stream"}),
        )
        .await;
    let sample = tokio::time::timeout(Duration::from_secs(5), subscription.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sample.key, received_key);
    assert_eq!(sample.payload, [0, 255, 128, 42]);
    assert_eq!(sample.encoding, "application/octet-stream");
    assert!(!sample.deleted);
    assert!(sample.timestamp.is_some());
    fixture
        .publish("/delete", json!({"key":received_key}))
        .await;
    let deleted = tokio::time::timeout(Duration::from_secs(5), subscription.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(deleted.key, received_key);
    assert!(deleted.deleted);

    let sent_key = format!("flow-like/e2e/adapter/{id}/measurement");
    client
        .publish(&sent_key, vec![0, 128, 255], "application/octet-stream")
        .await
        .unwrap();
    let observed = fixture.observe(&sent_key, false).await;
    assert_eq!(observed["payload"], json!([0, 128, 255]));
    assert_eq!(observed["encoding"], "application/octet-stream");
    assert!(observed["timestamp"].is_string());
    client.delete(&sent_key).await.unwrap();
    assert_eq!(fixture.observe(&sent_key, true).await["deleted"], true);

    client.close().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), subscription.receive())
            .await
            .unwrap()
            .is_err()
    );
    assert!(
        client
            .publish(&sent_key, vec![1], "text/plain")
            .await
            .is_err()
    );
    drop(subscription);

    let reconnected = fixture.connect().await;
    // A new key excludes any old subscription still being withdrawn by the router.
    let received_key = format!("flow-like/e2e/peer/{id}-reconnected/温度");
    let subscription = reconnected.subscribe(&received_key).await.unwrap();
    fixture
        .publish(
            "/publish",
            json!({"key":received_key,"payload":[114,101,97,100,121],"encoding":"text/plain"}),
        )
        .await;
    let sample = tokio::time::timeout(Duration::from_secs(5), subscription.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sample.payload, b"ready");
    assert_eq!(sample.encoding, "text/plain");
    reconnected.close().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the official Zenoh router and Python reference peer fixtures"]
async fn zenoh_router_storage_and_reference_queries_preserve_values_and_errors() {
    let fixture = Fixture::new().await;
    let client = fixture.connect().await;
    let key = format!("flow-like/e2e/storage/{}/value", unique());
    client
        .publish(&key, vec![0, 255, 17], "application/octet-stream")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let values = fixture.get("/query", &[("selector", &key)]).await;
            if !values.as_array().unwrap().is_empty() {
                assert_eq!(values[0]["payload"], json!([0, 255, 17]));
                assert_eq!(values[0]["encoding"], "application/octet-stream");
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("official memory storage must receive the routed publication");
    let values = client.query(&key, 2_000, 8).await.unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].payload, [0, 255, 17]);
    assert!(values[0].timestamp.is_some());

    let mut values = client
        .query("flow-like/e2e/oracle/known/**", 2_000, 8)
        .await
        .unwrap();
    values.sort_by(|a, b| a.key.cmp(&b.key));
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].payload, [0, 255, 128]);
    assert_eq!(values[0].encoding, "application/octet-stream");
    assert_eq!(values[1].payload, br#"{"temperature":21.5}"#);
    assert_eq!(values[1].encoding, "application/json");
    assert_eq!(
        client
            .query("flow-like/e2e/oracle/many/**", 2_000, 2)
            .await
            .unwrap()
            .len(),
        2
    );
    let error = client
        .query("flow-like/e2e/oracle/error", 2_000, 8)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("fixture denied query"),
        "{error}"
    );
    let delayed = tokio::time::timeout(
        Duration::from_secs(1),
        client.query("flow-like/e2e/oracle/slow", 100, 8),
    )
    .await
    .expect("remote query must obey its deadline")
    .unwrap_err();
    assert!(delayed.to_string().contains("Timeout"), "{delayed}");

    client.delete(&key).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if fixture
                .get("/query", &[("selector", &key)])
                .await
                .as_array()
                .unwrap()
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("router storage must remove deleted values");
    assert!(client.query(&key, 2_000, 8).await.unwrap().is_empty());
    client.close().await.unwrap();
}
