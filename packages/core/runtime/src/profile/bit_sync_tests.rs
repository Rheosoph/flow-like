use super::{Profile, ProfileCustomBit};
use crate::{bit::Bit, hub::hub_api_url, utils::http::HTTPClient};
use flow_like_types::{json::json, tokio};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};

struct TestHub {
    url: String,
    responses: Arc<std::sync::Mutex<HashMap<String, (u16, String)>>>,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
    task: JoinHandle<()>,
}

impl TestHub {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let responses = Arc::new(std::sync::Mutex::new(HashMap::from([(
            "/api/v1".to_owned(),
            (200, json!({"domain": url, "hubs": []}).to_string()),
        )])));
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let routes = responses.clone();
        let hits = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut buffer = [0; 4096];
                    let length = socket.read(&mut buffer).await.unwrap();
                    if length == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..length]);
                }
                let request = String::from_utf8(request).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_owned();
                hits.lock().unwrap().push(path.clone());
                let (status, body) = routes
                    .lock()
                    .unwrap()
                    .get(&path)
                    .cloned()
                    .unwrap_or((404, "{}".into()));
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            url,
            responses,
            requests,
            task,
        }
    }

    fn response(&self, path: &str, status: u16, body: String) {
        self.responses
            .lock()
            .unwrap()
            .insert(path.into(), (status, body));
    }

    fn dependencies(&self, hubs: &[&str]) {
        self.response(
            "/api/v1",
            200,
            json!({"domain": self.url, "hubs": hubs}).to_string(),
        );
    }

    fn bit(&self, id: &str) {
        self.response(
            &format!("/api/v1/bit/{id}"),
            200,
            flow_like_types::json::to_string(&Bit {
                id: id.into(),
                ..Bit::default()
            })
            .unwrap(),
        );
    }
}

impl Drop for TestHub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn profile(hub: &TestHub, bits: &[&str]) -> Profile {
    Profile {
        hub: hub.url.clone(),
        bits: bits.iter().map(|id| (*id).into()).collect(),
        ..Profile::default()
    }
}

#[test]
fn sync_future_can_be_sent_between_threads() {
    fn assert_send(_: impl Send) {}

    let profile = Profile::default();
    assert_send(profile.missing_bit_references(Arc::new(HTTPClient::new_without_refetch())));
}

#[tokio::test]
async fn sync_removes_only_confirmed_missing_bits_even_when_a_success_was_cached() {
    let hub = TestHub::new().await;
    hub.bit("live");
    let qualified = format!("{}:deleted", hub.url);
    let mut profile = profile(&hub, &["gone", "live", &qualified]);
    let http = Arc::new(HTTPClient::new_without_refetch());
    let request = http
        .client()
        .get(hub_api_url(&hub.url, true, &["bit", "gone"]).unwrap())
        .build()
        .unwrap();
    http.cache.insert(
        http.quick_hash(&request),
        flow_like_types::json::to_value(Bit {
            id: "gone".into(),
            ..Bit::default()
        })
        .unwrap(),
    );

    let missing = profile.missing_bit_references(http.clone()).await;
    assert_eq!(missing, vec!["gone", &qualified]);
    profile
        .bits
        .retain(|reference| !missing.contains(reference));
    hub.requests.lock().unwrap().clear();
    assert!(profile.missing_bit_references(http).await.is_empty());
    assert_eq!(profile.bits, ["live"]);
    assert_eq!(*hub.requests.lock().unwrap(), ["/api/v1/bit/live"]);
}

#[tokio::test]
async fn sync_keeps_custom_bits_without_querying_the_public_catalog() {
    let hub = TestHub::new().await;
    let qualified = format!("{}:private", hub.url);
    let mut profile = profile(&hub, &["private", &qualified]);
    profile.custom_bits.push(ProfileCustomBit(Bit {
        id: "private".into(),
        ..Bit::default()
    }));

    assert!(
        profile
            .missing_bit_references(Arc::new(HTTPClient::new_without_refetch()))
            .await
            .is_empty()
    );
    assert!(hub.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn sync_keeps_bits_on_auth_rate_limit_server_and_invalid_response_errors() {
    let hub = TestHub::new().await;
    let mut profile = profile(&hub, &[]);
    for status in [401, 403, 408, 429, 500, 503] {
        let id = format!("status-{status}");
        hub.response(&format!("/api/v1/bit/{id}"), status, "{}".into());
        profile.bits.push(id);
    }
    hub.response("/api/v1/bit/invalid", 200, "not JSON".into());
    hub.response(
        "/api/v1/bit/wrong",
        200,
        flow_like_types::json::to_string(&Bit {
            id: "other".into(),
            ..Bit::default()
        })
        .unwrap(),
    );
    profile.bits.extend(["invalid".into(), "wrong".into()]);
    assert!(
        profile
            .missing_bit_references(Arc::new(HTTPClient::new_without_refetch()))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn sync_requires_valid_dependency_metadata_after_a_bit_404() {
    let hub = TestHub::new().await;
    let profile = profile(&hub, &["gone"]);
    for (status, body) in [(404, "{}"), (503, "{}"), (200, "{}"), (200, "invalid")] {
        hub.response("/api/v1", status, body.into());
        assert!(
            profile
                .missing_bit_references(Arc::new(HTTPClient::new_without_refetch()))
                .await
                .is_empty(),
            "metadata response {status}: {body} cannot confirm absence"
        );
    }
}

#[tokio::test]
async fn sync_searches_alternate_hubs_for_bare_ids_and_dependencies_for_qualified_ids() {
    let first = TestHub::new().await;
    let second = TestHub::new().await;
    let dependency = TestHub::new().await;
    second.bit("bare");
    dependency.bit("qualified");
    first.dependencies(&[&dependency.url]);
    let qualified = format!("{}:qualified", first.url);
    let mut profile = profile(&first, &["bare", &qualified]);
    profile.hubs.push(second.url.clone());
    assert!(
        profile
            .missing_bit_references(Arc::new(HTTPClient::new_without_refetch()))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn sync_preserves_bits_served_by_the_hubs_canonical_domain() {
    let hub = TestHub::new().await;
    let canonical = TestHub::new().await;
    canonical.bit("model");
    hub.response(
        "/api/v1",
        200,
        json!({"domain": canonical.url, "hubs": []}).to_string(),
    );
    assert!(
        profile(&hub, &["model"])
            .missing_bit_references(Arc::new(HTTPClient::new_without_refetch()))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn sync_handles_dependency_cycles_and_retains_bits_when_any_source_is_unreachable() {
    let hub = TestHub::new().await;
    let dependency = TestHub::new().await;
    hub.dependencies(&[&dependency.url]);
    dependency.dependencies(&[&hub.url]);
    let mut profile = profile(&hub, &["gone"]);
    let http = Arc::new(HTTPClient::new_without_refetch());
    assert_eq!(profile.missing_bit_references(http.clone()).await, ["gone"]);

    let offline = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let offline_url = format!("http://{}", offline.local_addr().unwrap());
    drop(offline);
    dependency.dependencies(&[&hub.url, &offline_url]);
    assert!(
        profile
            .missing_bit_references(http.clone())
            .await
            .is_empty()
    );

    hub.dependencies(&[]);
    profile.hubs.push(offline_url);
    assert!(profile.missing_bit_references(http).await.is_empty());
}

#[tokio::test]
async fn sync_cannot_confirm_missing_bits_without_a_hub() {
    let profile = Profile {
        bits: vec!["unknown".into(), "".into()],
        ..Profile::default()
    };
    assert!(
        profile
            .missing_bit_references(Arc::new(HTTPClient::new_without_refetch()))
            .await
            .is_empty()
    );
}
