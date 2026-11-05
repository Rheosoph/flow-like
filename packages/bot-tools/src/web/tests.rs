use std::sync::{Arc, Mutex};

use flow_like_types::tokio::io::{AsyncReadExt, AsyncWriteExt};
use flow_like_types::tokio::net::{TcpListener, TcpStream};
use serde_json::json;

use super::*;
use crate::env::Environment;
use crate::error::code;
use crate::testing::FakeEnv;

async fn read_request(stream: &mut TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buffer[..end]).to_string();
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            while buffer.len() < end + 4 + length {
                let read = stream.read(&mut chunk).await.unwrap_or(0);
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            return String::from_utf8_lossy(&buffer).to_string();
        }
        let read = stream.read(&mut chunk).await.unwrap_or(0);
        if read == 0 {
            return String::from_utf8_lossy(&buffer).to_string();
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
}

fn reply(status: &str, content_type: &str, extra: &str, body: &[u8]) -> Vec<u8> {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    response
}

async fn serve(mut stream: TcpStream, seen: Arc<Mutex<Vec<String>>>) {
    let request = read_request(&mut stream).await;
    let path = request.split_whitespace().nth(1).unwrap_or("/").to_string();
    seen.lock().unwrap().push(request);
    let response = match path.as_str() {
        "/feed.xml" => reply(
            "200 OK",
            "application/rss+xml",
            "",
            b"<rss><channel><item><title>CVE-2026-0001</title></item></channel></rss>",
        ),
        "/api" => reply("200 OK", "application/json", "", br#"{"items":[1,2]}"#),
        "/missing" => reply(
            "404 Not Found",
            "application/json",
            "",
            br#"{"error":"nope"}"#,
        ),
        "/moved" => reply(
            "302 Found",
            "text/plain",
            "Location: https://example.com/new\r\nSet-Cookie: a=b\r\n",
            b"",
        ),
        "/big" => reply("200 OK", "text/plain", "", "x".repeat(200_000).as_bytes()),
        "/report.pdf" => reply("200 OK", "application/pdf", "", b"%PDF-1.7 \x00\x01\x02"),
        _ => reply(
            "201 Created",
            "application/json",
            "",
            br#"{"created":true}"#,
        ),
    };
    let _ = stream.write_all(&response).await;
}

async fn server() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let shared = seen.clone();
    flow_like_types::tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            flow_like_types::tokio::spawn(serve(stream, shared.clone()));
        }
    });
    (base, seen)
}

#[test]
fn reads_and_writes_of_a_site_are_granted_apart_and_secrets_per_site() {
    assert_eq!(
        grant_scopes(&json!({ "url": "https://API.example.com/v1/items?q=1" })),
        Some(vec!["web:api.example.com:read".to_string()])
    );
    assert_eq!(
        grant_scopes(&json!({ "url": "https://api.example.com/v1", "method": "post" })),
        Some(vec!["web:api.example.com:write".to_string()])
    );
    assert_eq!(
        grant_scopes(&json!({
            "url": "http://localhost:8123/api/states",
            "method": "HEAD",
            "headers": { "Authorization": "Bearer {{secret:HA_TOKEN}}" },
        })),
        Some(vec![
            "web:localhost:8123:read".to_string(),
            "web:localhost:8123:secret:HA_TOKEN".to_string(),
        ])
    );
    for args in [
        json!({}),
        json!({ "url": "ftp://example.com/file" }),
        json!({ "url": "https://user:pw@example.com/" }),
        json!({ "url": "not a url" }),
    ] {
        assert_eq!(grant_scopes(&args), None, "{args}");
    }
}

#[test]
fn the_owner_reads_method_site_path_secrets_and_the_start_of_a_write() {
    let read = ask_summary(&json!({
        "url": "https://api.github.com/repos/a/b/issues?state=open",
        "headers": { "Authorization": "Bearer {{secret:GITHUB_TOKEN}}" },
    }));
    assert_eq!(
        read,
        "The bot wants to send a GET request to api.github.com/repos/a/b/issues with your secret GITHUB_TOKEN."
    );
    let write = ask_summary(&json!({
        "url": "https://hooks.example.com/notify",
        "method": "POST",
        "json": { "text": "deploy done" },
    }));
    assert!(write.starts_with("The bot wants to send a POST request to hooks.example.com/notify."));
    assert!(write.contains(r#"Body: {"text":"deploy done"}"#), "{write}");
}

#[tokio::test]
async fn a_feed_and_an_api_answer_inline() {
    let (base, _) = server().await;
    let env = FakeEnv::new().await;

    let feed = crate::run(
        WEB_REQUEST,
        json!({ "url": format!("{base}/feed.xml") }),
        &env,
    )
    .await;
    assert!(feed.is_ok(), "{:?}", feed.error);
    assert_eq!(feed.result["http_status"], 200);
    assert_eq!(feed.result["untrusted"], true);
    assert!(
        feed.result["text"]
            .as_str()
            .unwrap()
            .contains("CVE-2026-0001")
    );

    let api = crate::run(WEB_REQUEST, json!({ "url": format!("{base}/api") }), &env).await;
    assert_eq!(api.result["json"], json!({ "items": [1, 2] }));
    assert_eq!(api.result["http_ok"], true);

    let missing = crate::run(
        WEB_REQUEST,
        json!({ "url": format!("{base}/missing") }),
        &env,
    )
    .await;
    assert!(
        missing.is_ok(),
        "an error status is an answer, not a failed call"
    );
    assert_eq!(missing.result["http_status"], 404);
    assert_eq!(missing.result["http_ok"], false);
    assert_eq!(missing.result["json"], json!({ "error": "nope" }));
}

#[tokio::test]
async fn a_write_sends_its_json_body() {
    let (base, seen) = server().await;
    let env = FakeEnv::new().await;
    let created = crate::run(
        WEB_REQUEST,
        json!({ "url": format!("{base}/items"), "method": "post", "json": { "name": "x" } }),
        &env,
    )
    .await;
    assert!(created.is_ok(), "{:?}", created.error);
    assert_eq!(created.result["http_status"], 201);
    let request = seen.lock().unwrap().last().cloned().unwrap();
    assert!(request.starts_with("POST /items "), "{request}");
    assert!(
        request
            .to_ascii_lowercase()
            .contains("content-type: application/json")
    );
    assert!(request.ends_with(r#"{"name":"x"}"#), "{request}");
}

#[tokio::test]
async fn redirects_come_back_unfollowed_without_cookies() {
    let (base, seen) = server().await;
    let env = FakeEnv::new().await;
    let moved = crate::run(WEB_REQUEST, json!({ "url": format!("{base}/moved") }), &env).await;
    assert!(moved.is_ok(), "{:?}", moved.error);
    assert_eq!(moved.result["http_status"], 302);
    assert_eq!(
        moved.result["headers"]["location"],
        "https://example.com/new"
    );
    assert!(moved.result["headers"].get("set-cookie").is_none());
    assert!(moved.warnings.iter().any(|w| w.contains("Location")));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn large_and_binary_responses_land_in_the_workspace() {
    let (base, _) = server().await;
    let env = FakeEnv::new().await;

    let big = crate::run(WEB_REQUEST, json!({ "url": format!("{base}/big") }), &env).await;
    assert!(big.is_ok(), "{:?}", big.error);
    assert_eq!(big.result["preview_only"], true);
    assert!(
        big.result["file"]["path"]
            .as_str()
            .unwrap()
            .starts_with("bot/web/")
    );

    let pdf = crate::run(
        WEB_REQUEST,
        json!({ "url": format!("{base}/report.pdf") }),
        &env,
    )
    .await;
    assert!(pdf.is_ok(), "{:?}", pdf.error);
    let path = pdf.result["file"]["path"].as_str().unwrap();
    assert!(
        path.starts_with("bot/web/") && path.ends_with("-report.pdf"),
        "{path}"
    );
    assert!(pdf.result.get("text").is_none());
}

#[tokio::test]
async fn secrets_never_go_over_plain_http() {
    let (base, seen) = server().await;
    let env = FakeEnv::new().await;
    env.set_secret("API_KEY", "k-123456789");
    for args in [
        json!({ "url": format!("{base}/api"), "headers": { "X-Key": "{{secret:API_KEY}}" } }),
        json!({ "url": format!("{base}/api?key={{{{secret:API_KEY}}}}") }),
        json!({ "url": format!("{base}/items"), "method": "POST", "json": { "k": "{{secret:API_KEY}}" } }),
    ] {
        let refused = crate::run(WEB_REQUEST, args.clone(), &env).await;
        assert_eq!(refused.error.unwrap().code, code::SCOPE_REFUSED, "{args}");
    }
    let header = crate::run(
        WEB_REQUEST,
        json!({ "url": format!("{base}/api"), "headers": { "Authorization": "Bearer plain" } }),
        &env,
    )
    .await;
    assert_eq!(header.error.unwrap().code, code::SCOPE_REFUSED);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn secrets_are_filled_into_headers_and_json_and_scrubbed_after() {
    let env = FakeEnv::new().await;
    env.set_secret("API_KEY", "k-123456789");
    let mut filler = Filler {
        env: &env,
        scrubber: Scrubber::default(),
        used: BTreeSet::new(),
    };
    let headers = BTreeMap::from([(
        "Authorization".to_string(),
        "Bearer {{secret:API_KEY}}".to_string(),
    )]);
    let map = request_headers(&mut filler, &headers, true).await.unwrap();
    assert_eq!(map["authorization"], "Bearer k-123456789");
    let body = filler
        .json(&json!({ "auth": { "key": "{{secret:API_KEY}}" }, "n": 1 }))
        .await
        .unwrap();
    assert_eq!(body, json!({ "auth": { "key": "k-123456789" }, "n": 1 }));
    assert_eq!(
        filler.scrubber.scrub("echo: k-123456789"),
        "echo: {{secret:API_KEY}}"
    );

    let missing = filler.text("{{secret:NOPE}}").await.unwrap_err();
    assert_eq!(missing.code, code::NOT_FOUND);
}

#[tokio::test]
async fn a_server_reaches_public_hosts_only() {
    let (base, seen) = server().await;
    let env = FakeEnv::new().await;
    *env.environment.lock().unwrap() = Environment::Server;
    let refused = crate::run(WEB_REQUEST, json!({ "url": format!("{base}/api") }), &env).await;
    assert_eq!(refused.error.unwrap().code, code::SCOPE_REFUSED);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bad_requests_fail_before_anything_is_sent() {
    let (base, seen) = server().await;
    let env = FakeEnv::new().await;
    for args in [
        json!({ "url": format!("{base}/api"), "method": "TRACE" }),
        json!({ "url": format!("{base}/api"), "body": "x" }),
        json!({ "url": format!("{base}/api"), "method": "POST", "body": "x", "json": {} }),
        json!({ "url": format!("{base}/api"), "headers": { "Host": "evil.com" } }),
        json!({ "url": "file:///etc/passwd" }),
    ] {
        let failed = crate::run(WEB_REQUEST, args.clone(), &env).await;
        assert_eq!(failed.error.unwrap().code, code::INVALID_ARGS, "{args}");
    }
    assert!(seen.lock().unwrap().is_empty());
}
