use bytes::Bytes;
use flow_like_platform::{
    Auth, Client, DatabaseAction, DatabaseSelector, Error, InvokeBoardRequest, InvokeEventRequest,
    Method, PollOptions, RequestOptions, SignedFile, Value, WaitOptions, json,
};
use futures_util::StreamExt;
use std::{collections::BTreeMap, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    chunks: Vec<Vec<u8>>,
    delay: Duration,
    declared_length: Option<usize>,
}
impl Reply {
    fn json(body: Value) -> Self {
        Self::bytes(200, "application/json", serde_json::to_vec(&body).unwrap())
    }
    fn bytes(status: u16, content_type: &str, bytes: Vec<u8>) -> Self {
        Self {
            status,
            headers: vec![("Content-Type".into(), content_type.into())],
            chunks: vec![bytes],
            delay: Duration::ZERO,
            declared_length: None,
        }
    }
}

#[derive(Debug)]
struct Request {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
struct Server {
    url: String,
    requests: mpsc::Receiver<Request>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut data = Vec::new();
                let end = loop {
                    let mut chunk = [0u8; 4096];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(read, 0);
                    data.extend_from_slice(&chunk[..read]);
                    if let Some(end) = data.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let text = String::from_utf8(data[..end].to_vec()).unwrap();
                let mut lines = text.split("\r\n");
                let first = lines.next().unwrap().split_whitespace().collect::<Vec<_>>();
                let headers = lines
                    .filter_map(|line| line.split_once(':'))
                    .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
                    .collect::<BTreeMap<_, _>>();
                let length = headers
                    .get("content-length")
                    .map(|value| value.parse::<usize>().unwrap())
                    .unwrap_or(0);
                while data.len() < end + length {
                    let mut chunk = [0u8; 4096];
                    let read = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(read, 0);
                    data.extend_from_slice(&chunk[..read]);
                }
                sender
                    .send(Request {
                        method: first[0].into(),
                        target: first[1].into(),
                        headers,
                        body: data[end..end + length].to_vec(),
                    })
                    .await
                    .unwrap();
                tokio::time::sleep(reply.delay).await;
                let size: usize = reply
                    .declared_length
                    .unwrap_or_else(|| reply.chunks.iter().map(Vec::len).sum());
                let mut head = format!(
                    "HTTP/1.1 {} Test\r\nContent-Length: {size}\r\nConnection: close\r\n",
                    reply.status
                );
                for (key, value) in reply.headers {
                    head.push_str(&format!("{key}: {value}\r\n"));
                }
                head.push_str("\r\n");
                if socket.write_all(head.as_bytes()).await.is_err() {
                    continue;
                }
                for chunk in reply.chunks {
                    if socket.write_all(&chunk).await.is_err() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
    fn client(&self) -> Client {
        Client::new(&self.url, Auth::ApiKey("flk_secret".into())).unwrap()
    }
    async fn take(&mut self) -> Request {
        tokio::time::timeout(Duration::from_secs(2), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }
}

#[tokio::test]
async fn paths_keep_proxy_prefix_encode_ids_and_preserve_queries() {
    let mut server = Server::start(vec![
        Reply::json(json!({"id":"board","geometry":{"type":"Point","coordinates":[1.0,2.0]}})),
        Reply::json(Value::Null),
    ])
    .await;
    let client = Client::new(
        format!("{}/proxy/api/v1/", server.url),
        Auth::ApiKey("flk_secret".into()),
    )
    .unwrap();
    let board = client
        .get_board(
            "app/id",
            "b ?",
            &RequestOptions::new().query("version", "1_2_3"),
        )
        .await
        .unwrap();
    assert_eq!(
        board["geometry"],
        json!({"type":"Point","coordinates":[1.0,2.0]})
    );
    let request = server.take().await;
    assert_eq!(
        request.target,
        "/proxy/api/v1/apps/app%2Fid/board/b%20%3F?version=1_2_3"
    );
    assert_eq!(request.headers["x-api-key"], "flk_secret");
    assert_eq!(request.headers["x-flow-like-board-format"], "2");
    assert!(!request.headers.contains_key("authorization"));
    assert!(client.get_app("..", &RequestOptions::new()).await.is_err());
    client
        .create_app(&RequestOptions::new().json(json!({"name":"Demo"})))
        .await
        .unwrap();
    let request = server.take().await;
    assert_eq!(request.method, "PUT");
    assert_eq!(request.target, "/proxy/api/v1/apps/new");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap()["name"],
        "Demo"
    );
}

#[tokio::test]
async fn pat_headers_empty_success_and_structured_errors_match_contract() {
    let mut error = Reply::bytes(
        423,
        "application/json",
        br#"{"message":"Board busy","code":"BOARD_LOCKED"}"#.to_vec(),
    );
    error.headers.extend([
        ("X-Request-Id".into(), "req-1".into()),
        ("Retry-After".into(), "2".into()),
    ]);
    let mut server = Server::start(vec![
        Reply::bytes(204, "application/json", vec![]),
        error,
        Reply::bytes(403, "text/plain", b"denied".to_vec()),
    ])
    .await;
    let auth = Auth::PersonalAccessToken("pat_secret".into());
    assert!(!format!("{auth:?}").contains("pat_secret"));
    let client = Client::new(&server.url, auth).unwrap();
    assert_eq!(
        client
            .delete_app("app", &RequestOptions::new())
            .await
            .unwrap(),
        Value::Null
    );
    let request = server.take().await;
    assert_eq!(request.headers["authorization"], "pat_secret");
    assert!(!request.headers.contains_key("x-api-key"));
    let error = client
        .publish_board_if_changed("app", "b", &RequestOptions::new())
        .await
        .unwrap_err();
    match error {
        Error::Api {
            status,
            body,
            request_id,
            retry_after,
            ..
        } => {
            assert_eq!(status, 423);
            assert_eq!(body["code"], "BOARD_LOCKED");
            assert_eq!(request_id.as_deref(), Some("req-1"));
            assert_eq!(retry_after.as_deref(), Some("2"));
        }
        error => panic!("{error}"),
    }
    let error = client.health(&RequestOptions::new()).await.unwrap_err();
    assert!(error.is_auth());
    assert!(error.to_string().contains("denied"));
}

fn poll(status: &str, sequences: std::ops::Range<i32>) -> Value {
    json!({"run_id":"r","status":status,"progress":100,"current_step":null,"error":if status == "FAILED" {Some("broken")}else{None},"started_at":null,"completed_at":null,
        "events":sequences.map(|sequence|json!({"sequence":sequence,"event_type":"output","payload":{"value":sequence},"created_at":"now"})).collect::<Vec<_>>()})
}

#[tokio::test]
async fn async_invocation_and_polling_use_distinct_authentication() {
    let mut server = Server::start(vec![
        Reply::json(
            json!({"run_id":"r","status":"PENDING","poll_token":"poll-secret","backend":"queue"}),
        ),
        Reply::json(poll("COMPLETED", 0..0)),
        Reply::json(json!({"run_id":"r","status":"Cancelled","cancelled":true})),
    ])
    .await;
    let client = server.client();
    let invoke = client
        .trigger_event_async(
            "a",
            "e",
            &InvokeEventRequest {
                payload: Some(json!({"text":"hello"})),
                ..Default::default()
            },
            &RequestOptions::new().query("__variant", "canary"),
        )
        .await
        .unwrap();
    assert_eq!(invoke.run_id, "r");
    let request = server.take().await;
    assert_eq!(
        request.target,
        "/api/v1/apps/a/events/e/invoke/async?__variant=canary"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"payload":{"text":"hello"}})
    );
    client
        .poll_execution(&invoke.poll_token, &PollOptions::default())
        .await
        .unwrap();
    let request = server.take().await;
    assert_eq!(request.headers["authorization"], "Bearer poll-secret");
    assert!(!request.headers.contains_key("x-api-key"));
    assert_eq!(
        request.target,
        "/api/v1/execution/poll?after_sequence=-1&timeout=10"
    );
    assert!(client.cancel_run("r").await.unwrap().cancelled);
    assert_eq!(server.take().await.method, "DELETE");
}

#[tokio::test]
async fn board_invocation_preserves_node_version_and_runtime_options() {
    let mut server = Server::start(vec![Reply::json(
        json!({"run_id":"r","status":"PENDING","poll_token":"poll-secret","backend":"queue"}),
    )])
    .await;
    server
        .client()
        .trigger_workflow_async(
            "a",
            "b",
            &InvokeBoardRequest {
                node_id: "entry".into(),
                version: Some((1, 2, 3)),
                payload: Some(json!({"text":"hello"})),
                profile_id: Some("profile".into()),
                ..Default::default()
            },
            &RequestOptions::new(),
        )
        .await
        .unwrap();
    let request = server.take().await;
    assert_eq!(request.target, "/api/v1/apps/a/board/b/invoke/async");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"node_id":"entry","version":[1,2,3],"payload":{"text":"hello"},"profile_id":"profile"})
    );
}

#[tokio::test]
async fn terminal_polling_drains_remaining_pages_and_surfaces_failure() {
    let mut server = Server::start(vec![
        Reply::json(poll("COMPLETED", 0..100)),
        Reply::json(poll("COMPLETED", 100..102)),
        Reply::json(poll("COMPLETED", 0..0)),
        Reply::json(poll("FAILED", 0..0)),
    ])
    .await;
    let client = server.client();
    let completed = client
        .wait_for_execution("poll-secret", &WaitOptions::default())
        .await
        .unwrap();
    assert_eq!(completed.events.len(), 102);
    assert_eq!(completed.last_sequence(), Some(101));
    assert!(server.take().await.target.contains("after_sequence=-1"));
    assert!(server.take().await.target.contains("after_sequence=99"));
    assert!(server.take().await.target.contains("after_sequence=101"));
    let error = client
        .wait_for_execution("poll-secret", &WaitOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Execution { status, message, .. } if status == "FAILED" && message == "broken")
    );
}

#[tokio::test]
async fn wait_deadline_cancels_a_pending_poll() {
    let mut reply = Reply::json(poll("RUNNING", 0..0));
    reply.delay = Duration::from_secs(1);
    let server = Server::start(vec![reply]).await;
    let result = server
        .client()
        .wait_for_execution(
            "poll-secret",
            &WaitOptions {
                timeout: Duration::from_millis(25),
                poll_timeout: 10,
            },
        )
        .await;
    assert!(matches!(result, Err(Error::Timeout)));
}

#[tokio::test]
async fn sse_handles_split_utf8_crlf_multiline_and_empty_data() {
    let source =
        ": keepalive\r\nid: 7\r\nevent: output\r\ndata: héllo \r\ndata: world\r\n\r\ndata:\r\n\r\n"
            .as_bytes();
    let mut reply = Reply::bytes(200, "text/event-stream", Vec::new());
    reply.chunks = source.chunks(2).map(<[u8]>::to_vec).collect();
    let server = Server::start(vec![reply, Reply::json(json!({"run_id":"r"}))]).await;
    let client = server.client();
    let mut stream = client
        .trigger_event(
            "a",
            "e",
            &InvokeEventRequest::default(),
            &RequestOptions::new(),
        )
        .await
        .unwrap();
    let event = stream.next().await.unwrap().unwrap();
    assert_eq!(event.data, "héllo \nworld");
    assert_eq!(event.id.as_deref(), Some("7"));
    assert_eq!(event.event.as_deref(), Some("output"));
    let empty = stream.next().await.unwrap().unwrap();
    assert_eq!(empty.data, "");
    assert_eq!(empty.id.as_deref(), Some("7"));
    assert!(stream.next().await.is_none());
    assert!(matches!(
        client
            .trigger_event(
                "a",
                "e",
                &InvokeEventRequest::default(),
                &RequestOptions::new()
            )
            .await,
        Err(Error::Stream(_))
    ));
}

#[tokio::test]
async fn sse_accepts_bare_carriage_return_separators() {
    let mut reply = Reply::bytes(200, "text/event-stream", vec![]);
    reply.chunks = b"data: one\r\rdata: two\r\rdata:\r\r"
        .chunks(1)
        .map(<[u8]>::to_vec)
        .collect();
    let server = Server::start(vec![reply]).await;
    let mut stream = server
        .client()
        .trigger_event(
            "a",
            "e",
            &InvokeEventRequest::default(),
            &RequestOptions::new(),
        )
        .await
        .unwrap();
    let mut data = Vec::new();
    while let Some(event) = stream.next().await {
        data.push(event.unwrap().data);
    }
    assert_eq!(data, vec!["one", "two", ""]);
}

#[tokio::test]
async fn signed_url_is_redacted_when_success_or_error_body_is_truncated() {
    let replies = [200, 503]
        .into_iter()
        .map(|status| {
            let mut reply = Reply::bytes(status, "text/plain", b"short".to_vec());
            reply.declared_length = Some(100);
            reply
        })
        .collect();
    let server = Server::start(replies).await;
    for _ in 0..2 {
        let error = server
            .client()
            .download_signed(&SignedFile {
                url: Some(format!("{}/signed?secret=private", server.url)),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert!(matches!(&error,Error::Transport(error) if error.url().is_none()));
        assert!(!error.to_string().contains("private"));
    }
}

#[tokio::test]
async fn signed_put_and_post_uploads_never_receive_platform_credentials() {
    let mut storage = Server::start(vec![
        Reply::bytes(204, "text/plain", vec![]),
        Reply::bytes(204, "text/plain", vec![]),
        Reply::bytes(200, "text/plain", b"download".to_vec()),
    ])
    .await;
    let mut api = Server::start(vec![Reply::json(
        json!([{"prefix":"folder/a.txt","url":format!("{}/put?signed=secret",storage.url)}]),
    )])
    .await;
    let client = api.client();
    client
        .upload_file(
            "a",
            "folder/a.txt",
            Bytes::from_static(b"hello"),
            "text/plain",
            false,
        )
        .await
        .unwrap();
    let requested = api.take().await;
    assert_eq!(requested.method, "PUT");
    assert_eq!(
        serde_json::from_slice::<Value>(&requested.body).unwrap(),
        json!({"prefixes":["folder/a.txt"],"sizes":[5]})
    );
    let mut grant = SignedFile {
        prefix: "a.txt".into(),
        url: Some(format!("{}/post", storage.url)),
        method: Some("POST".into()),
        fields: BTreeMap::from([
            ("key".into(), "a.txt".into()),
            ("policy".into(), "signed-policy".into()),
        ]),
        ..Default::default()
    };
    client
        .upload_signed(&grant, Bytes::from_static(b"content"), "text/plain")
        .await
        .unwrap();
    grant.url = Some(format!("{}/get", storage.url));
    assert_eq!(
        client.download_signed(&grant).await.unwrap(),
        Bytes::from_static(b"download")
    );
    for index in 0..3 {
        let request = storage.take().await;
        assert!(!request.headers.contains_key("authorization"));
        assert!(!request.headers.contains_key("x-api-key"));
        if index == 1 {
            assert_eq!(request.method, "POST");
            assert!(request.headers["content-type"].starts_with("multipart/form-data; boundary="));
            let body = String::from_utf8(request.body).unwrap();
            assert!(body.contains("signed-policy"));
            assert!(body.contains("name=\"file\""));
        }
    }
}

#[tokio::test]
async fn redirects_do_not_forward_platform_or_signed_requests() {
    let mut reply = Reply::bytes(302, "text/plain", vec![]);
    reply
        .headers
        .push(("Location".into(), "http://127.0.0.1:1/leak".into()));
    let mut signed_reply = Reply::bytes(302, "text/plain", vec![]);
    signed_reply.headers = reply.headers.clone();
    let mut server = Server::start(vec![reply, signed_reply]).await;
    let error = server
        .client()
        .health(&RequestOptions::new())
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(302));
    assert_eq!(server.take().await.target, "/api/v1/health");
    let error = server
        .client()
        .download_signed(&SignedFile {
            url: Some(format!("{}/signed", server.url)),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(error.status(), Some(302));
    let request = server.take().await;
    assert_eq!(request.target, "/signed");
    assert!(!request.headers.contains_key("authorization"));
    assert!(!request.headers.contains_key("x-api-key"));
}

#[tokio::test]
async fn project_database_and_ui_routes_preserve_wire_bodies_and_selectors() {
    let mut server = Server::start((0..5).map(|_| Reply::json(json!({}))).collect()).await;
    let client = server.client();
    client
        .get_run_logs(
            "a",
            "b",
            &RequestOptions::new()
                .json(json!({"run_id":"r","query":{"levels":[3]},"offset":10,"limit":20})),
        )
        .await
        .unwrap();
    client
        .apply_table_action(
            "a",
            "table/one",
            &DatabaseSelector {
                scope: Some("user".into()),
                branch: Some("dev".into()),
                ..Default::default()
            },
            &DatabaseAction::CreateTag {
                name: "release".into(),
            },
        )
        .await
        .unwrap();
    client
        .create_route(
            "a",
            &RequestOptions::new().json(json!({"path":"/","eventId":"e","isDefault":true})),
        )
        .await
        .unwrap();
    client
        .add_package(
            "a",
            &RequestOptions::new().json(json!({"packageId":"p","autoUpdate":true})),
        )
        .await
        .unwrap();
    client
        .get_device_access("d", &RequestOptions::new())
        .await
        .unwrap();
    let request = server.take().await;
    assert_eq!(request.target, "/api/v1/apps/a/board/b/logs/query");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap()["run_id"],
        "r"
    );
    let request = server.take().await;
    assert_eq!(
        request.target,
        "/api/v1/apps/a/db/table%2Fone/references?scope=user&branch=dev"
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"action":"create_tag","name":"release"})
    );
    let request = server.take().await;
    assert_eq!(request.target, "/api/v1/apps/a/routes");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap()["eventId"],
        "e"
    );
    let request = server.take().await;
    assert_eq!(request.target, "/api/v1/apps/a/packages");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap()["packageId"],
        "p"
    );
    assert_eq!(
        server.take().await.target,
        "/api/v1/devices/d/management/my-access"
    );
}

#[test]
fn invalid_configuration_is_rejected_before_network_access() {
    for url in [
        "ftp://example.com",
        "https://user:secret@example.com",
        "https://example.com?token=secret",
    ] {
        assert!(Client::new(url, Auth::Anonymous).is_err());
    }
    assert!(Auth::from_token("secret").is_err());
    assert!(
        Client::builder("http://localhost", Auth::Anonymous)
            .board_format_version(0)
            .build()
            .is_err()
    );
}

#[tokio::test]
async fn request_options_cannot_replace_authentication() {
    let client = Client::new("http://127.0.0.1:1", Auth::ApiKey("flk_original".into())).unwrap();
    let mut options = RequestOptions::new();
    options
        .headers
        .insert("x-api-key", "flk_override".parse().unwrap());
    assert!(matches!(
        client
            .request::<Value>(Method::GET, &["health"], &options)
            .await,
        Err(Error::Configuration(_))
    ));
}

#[tokio::test]
async fn model_discovery_filters_local_bits_and_preserves_provider_metadata() {
    let mut server=Server::start(vec![Reply::json(json!([
        {"id":"local","type":"Embedding","parameters":{"vector_length":384},"meta":{}},
        {"id":"remote","type":"Embedding","parameters":{"vector_length":512,"remote":{"implementation":"OpenAI","model_id":"embed"},"provider":{"model_id":"embed","provider_name":"Hosted"}},"meta":{"en":{"name":"Remote embedding","description":"Text vectors","tags":["text"]}}}
    ]))]).await;
    let models = server
        .client()
        .list_embedding_models(Some("embed"), 50)
        .await
        .unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].bit_id, "remote");
    assert_eq!(models[0].vector_length, Some(512));
    assert_eq!(models[0].name, "Remote embedding");
    let request = server.take().await;
    assert_eq!(request.target, "/api/v1/bit");
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        json!({"bit_types":["Embedding"],"search":"embed","limit":50})
    );
}
