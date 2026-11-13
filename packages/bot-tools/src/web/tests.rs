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
        "/dl/report.pdf?sig=SECRET123" => {
            reply("200 OK", "application/pdf", "", b"%PDF-1.7 \x00\x01\x02")
        }
        "/dl/blob%20one?sig=SECRET123" => reply(
            "200 OK",
            "application/pdf",
            "Content-Disposition: attachment; filename=\"Quarterly Report.pdf\"\r\n",
            b"%PDF-1.7",
        ),
        "/dl/plain?sig=SECRET123" => reply("200 OK", "text/csv", "", b"a,b\n1,2\n"),
        "/dl/moved?sig=SECRET123" => reply(
            "302 Found",
            "text/plain",
            "Location: https://cdn.example.com/x?token=SECRET123\r\n",
            b"",
        ),
        "/dl/gone?sig=SECRET123" => reply("403 Forbidden", "text/plain", "", b"expired"),
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
        "The bot wants to send a GET request to api.github.com/repos/a/b/issues?state=open with your secret GITHUB_TOKEN."
    );
    let write = ask_summary(&json!({
        "url": "https://hooks.example.com/notify",
        "method": "POST",
        "json": { "text": "deploy done" },
    }));
    assert!(write.starts_with("The bot wants to send a POST request to hooks.example.com/notify."));
    assert!(write.contains(r#"Body: {"text":"deploy done"}"#), "{write}");
}

#[test]
fn a_long_query_is_shortened_in_the_approval_and_the_fragment_is_dropped() {
    let long = "x".repeat(500);
    let text = ask_summary(&json!({ "url": format!("https://api.example.com/s?q={long}#frag") }));
    assert!(text.contains("api.example.com/s?q=xxx"), "{text}");
    assert!(text.contains('…') && !text.contains("#frag"), "{text}");
    assert!(text.len() < 400, "{}", text.len());
}

#[test]
fn an_ip_literal_and_localhost_keep_the_plain_scope() {
    assert_eq!(
        grant_scopes(&json!({ "url": "http://192.168.1.4/api" })),
        Some(vec!["web:192.168.1.4:read".to_string()])
    );
    assert_eq!(
        grant_scopes(&json!({ "url": "http://localhost/api" })),
        Some(vec!["web:localhost:read".to_string()])
    );
}

#[test]
fn allow_private_selects_the_private_scope_and_the_summary_says_so() {
    assert_eq!(
        grant_scopes(&json!({ "url": "https://nas.example.com/x", "allow_private": true })),
        Some(vec!["web:nas.example.com:private:read".to_string()])
    );
    assert_eq!(
        grant_scopes(&json!({ "url": "https://nas.example.com/x", "allow_private": false })),
        Some(vec!["web:nas.example.com:read".to_string()])
    );
    let text = ask_summary(&json!({ "url": "https://nas.example.com/x", "allow_private": true }));
    assert!(text.contains("is on a private network"), "{text}");
    let plain = ask_summary(&json!({ "url": "https://nas.example.com/x" }));
    assert!(!plain.contains("private network"), "{plain}");
}

#[test]
fn allow_private_is_in_the_schema_and_parses() {
    let schema = schema_of::<WebRequestArgs>();
    assert!(schema["properties"]["allow_private"].is_object());
    let args: WebRequestArgs =
        serde_json::from_value(json!({ "url": "https://a.example/", "allow_private": true }))
            .unwrap();
    assert_eq!(args.allow_private, Some(true));
}

#[tokio::test]
async fn a_desktop_never_reaches_a_metadata_address() {
    let env = FakeEnv::new().await;
    let result = crate::run(
        WEB_REQUEST,
        json!({ "url": "http://169.254.169.254/latest/meta-data" }),
        &env,
    )
    .await;
    assert!(!result.is_ok());
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

mod file_save_url {
    use super::*;
    use crate::testing::WORKSPACE;

    async fn save(env: &FakeEnv, args: serde_json::Value) -> crate::ToolOutcome {
        crate::run("file_save", args, env).await
    }

    fn assert_no_secret(outcome: &crate::ToolOutcome) {
        let text = format!("{:?} {:?}", outcome.result, outcome.error);
        assert!(!text.contains("SECRET123"), "{text}");
        assert!(!text.contains("sig="), "{text}");
    }

    #[tokio::test]
    async fn a_download_lands_in_the_workspace_without_its_query() {
        let (base, _) = server().await;
        let env = FakeEnv::new().await;
        let url = format!("{base}/dl/report.pdf?sig=SECRET123#frag");
        let saved = save(&env, json!({ "url": url })).await;
        assert!(saved.is_ok(), "{:?}", saved.error);
        assert_eq!(saved.saved[0].path, "bot/files/report.pdf");
        assert_eq!(saved.saved[0].app_id, WORKSPACE);
        assert_eq!(saved.saved[0].mime, "application/pdf");
        assert_eq!(
            env.get(WORKSPACE, "bot/files/report.pdf").unwrap().to_vec(),
            b"%PDF-1.7 \x00\x01\x02"
        );
        assert_eq!(saved.result["source"], format!("{base}/dl/report.pdf"));
        assert_eq!(
            saved.result["markdown_url"],
            format!("storage://apps/{WORKSPACE}/app/bot/files/report.pdf")
        );
        assert_no_secret(&saved);
    }

    #[tokio::test]
    async fn the_content_disposition_name_wins_and_a_type_adds_the_extension() {
        let (base, _) = server().await;
        let env = FakeEnv::new().await;
        let named = save(
            &env,
            json!({ "url": format!("{base}/dl/blob%20one?sig=SECRET123") }),
        )
        .await;
        assert!(named.is_ok(), "{:?}", named.error);
        assert_eq!(named.saved[0].path, "bot/files/Quarterly Report.pdf");

        let typed = save(
            &env,
            json!({ "url": format!("{base}/dl/plain?sig=SECRET123") }),
        )
        .await;
        assert_eq!(typed.saved[0].path, "bot/files/plain.csv");
        assert_eq!(typed.saved[0].mime, "text/csv");

        let explicit = save(
            &env,
            json!({ "url": format!("{base}/dl/plain?sig=SECRET123"), "path": "bot/in/data.csv" }),
        )
        .await;
        assert_eq!(explicit.saved[0].path, "bot/in/data.csv");
    }

    #[tokio::test]
    async fn oversize_failures_and_redirects_are_refused_without_the_query() {
        let (base, _) = server().await;
        let mut env = FakeEnv::new().await;
        env.limits.max_write_bytes = 1000;
        let big = save(&env, json!({ "url": format!("{base}/big") })).await;
        assert_eq!(big.error.as_ref().unwrap().code, code::TOO_LARGE);
        assert_no_secret(&big);

        let gone = save(
            &env,
            json!({ "url": format!("{base}/dl/gone?sig=SECRET123") }),
        )
        .await;
        let error = gone.error.as_ref().unwrap();
        assert_eq!(error.code, code::FAILED);
        assert!(error.message.contains("403"));
        assert_no_secret(&gone);

        let moved = save(
            &env,
            json!({ "url": format!("{base}/dl/moved?sig=SECRET123") }),
        )
        .await;
        let error = moved.error.as_ref().unwrap();
        assert!(error.message.contains("redirects are not followed"));
        assert!(error.message.contains("cdn.example.com"));
        assert_no_secret(&moved);
        assert_eq!(env.write_count(), 0);
    }

    #[tokio::test]
    async fn a_server_refuses_loopback_and_exactly_one_source_is_required() {
        let (base, seen) = server().await;
        let env = FakeEnv::new().await;
        *env.environment.lock().unwrap() = Environment::Server;
        let refused = save(
            &env,
            json!({ "url": format!("{base}/dl/plain?sig=SECRET123") }),
        )
        .await;
        assert_eq!(refused.error.as_ref().unwrap().code, code::SCOPE_REFUSED);
        assert_no_secret(&refused);
        assert!(seen.lock().unwrap().is_empty());

        let env = FakeEnv::new().await;
        for args in [
            json!({}),
            json!({ "file": "bot/files/a.md", "url": format!("{base}/api") }),
            json!({ "url": "ftp://example.com/a" }),
            json!({ "url": "https://user:pw@example.com/a" }),
        ] {
            let failed = save(&env, args.clone()).await;
            assert_eq!(
                failed.error.as_ref().unwrap().code,
                code::INVALID_ARGS,
                "{args}"
            );
        }
        assert!(seen.lock().unwrap().is_empty());
    }
}
