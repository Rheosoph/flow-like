use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use flow_like_types::Result;
use flow_like_types::bot_connector::{
    CallPolicy, ConnectorKind, ConnectorPolicy, ConnectorRule, ConnectorToolInfo,
};
use serde_json::{Value, json};

use super::*;
use crate::error::code;
use crate::testing::FakeEnv;
use crate::{ToolOutcome, run};

#[derive(Default)]
struct FakeSource {
    entries: Vec<ConnectorEntry>,
    credential: Mutex<Option<String>>,
    audits: Mutex<Vec<CallAudit>>,
    budget: CallBudget,
}

#[async_trait]
impl ConnectorSource for FakeSource {
    async fn entries(&self) -> Result<Vec<ConnectorEntry>> {
        Ok(self.entries.clone())
    }

    async fn credential(&self, _connector_id: &str) -> Result<Credential> {
        let secret = self.credential.lock().expect("credential").clone();
        Ok(secret.map_or_else(Credential::none, |value| Credential::from_secret(&value)))
    }

    fn budget(&self) -> &CallBudget {
        &self.budget
    }

    fn audit(&self, entry: &CallAudit) {
        self.audits.lock().expect("audits").push(entry.clone());
    }
}

fn info(name: &str, read_only: Option<bool>, destructive: Option<bool>) -> ConnectorToolInfo {
    ConnectorToolInfo {
        name: name.to_string(),
        description: format!("{name} does a thing"),
        read_only_hint: read_only,
        destructive_hint: destructive,
        schema_hash: String::new(),
    }
}

fn mcp_entry(url: &str, policies: &[(&str, ConnectorPolicy)]) -> ConnectorEntry {
    ConnectorEntry {
        rule: ConnectorRule {
            id: "c-docs".into(),
            label: "Docs".into(),
            kind: Some(ConnectorKind::Mcp),
            default_policy: None,
            tool_policies: policies
                .iter()
                .map(|(name, policy)| (name.to_string(), *policy))
                .collect::<BTreeMap<_, _>>(),
            tools: vec![
                info("search", Some(true), None),
                info("secret_stuff", None, None),
                info("drop", None, Some(true)),
            ],
        },
        kind: ConnectorKind::Mcp,
        url: Some(url.into()),
        provider_id: None,
        api_hosts: Vec::new(),
    }
}

fn account_entry() -> ConnectorEntry {
    ConnectorEntry {
        rule: ConnectorRule {
            id: "c-gh".into(),
            label: "GitHub".into(),
            kind: Some(ConnectorKind::Oauth),
            default_policy: Some(ConnectorPolicy::Ask),
            tool_policies: BTreeMap::new(),
            tools: Vec::new(),
        },
        kind: ConnectorKind::Oauth,
        url: None,
        provider_id: Some("github".into()),
        api_hosts: vec!["api.github.com".into()],
    }
}

async fn env_with(entries: Vec<ConnectorEntry>) -> (FakeEnv, Arc<FakeSource>) {
    let env = FakeEnv::new().await;
    let source = Arc::new(FakeSource {
        entries,
        ..FakeSource::default()
    });
    *env.connectors.lock().expect("connectors") = Some(source.clone());
    (env, source)
}

fn error_code(outcome: &ToolOutcome) -> &'static str {
    outcome.error.as_ref().expect("an error").code
}

#[tokio::test]
async fn a_host_without_connectors_says_so() {
    let env = FakeEnv::new().await;
    let outcome = run(CONNECTOR_TOOLS, json!({}), &env).await;
    assert_eq!(error_code(&outcome), code::NOT_YET);
}

#[tokio::test]
async fn an_account_lists_its_hosts_and_what_asks() {
    let (env, _) = env_with(vec![account_entry()]).await;
    let outcome = run(CONNECTOR_TOOLS, json!({}), &env).await;
    assert!(outcome.is_ok(), "{:?}", outcome.error);
    let listed = &outcome.result["connectors"][0];
    assert_eq!(listed["kind"], "oauth");
    assert_eq!(listed["api_hosts"], json!(["api.github.com"]));
    assert_eq!(listed["access"]["GET_HEAD"], "runs_directly");
    assert_eq!(listed["access"]["other_methods"], "asks_owner");
}

#[tokio::test]
async fn a_tool_the_owner_switched_off_is_refused() {
    let (env, source) = env_with(vec![mcp_entry(
        "http://127.0.0.1:9/mcp",
        &[("secret_stuff", ConnectorPolicy::Never)],
    )])
    .await;
    let outcome = run(
        CONNECTOR_CALL,
        json!({ "connector": "Docs", "tool": "secret_stuff" }),
        &env,
    )
    .await;
    assert_eq!(error_code(&outcome), code::SCOPE_REFUSED);
    assert!(source.audits.lock().unwrap().is_empty());
    assert_eq!(source.budget.0.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_switched_off_method_is_refused_before_any_request() {
    let mut entry = account_entry();
    entry
        .rule
        .tool_policies
        .insert("write".into(), ConnectorPolicy::Never);
    let (env, _) = env_with(vec![entry]).await;
    let outcome = run(
        CONNECTOR_REQUEST,
        json!({ "connector": "c-gh", "method": "DELETE", "path": "/repos/a/b" }),
        &env,
    )
    .await;
    assert_eq!(error_code(&outcome), code::SCOPE_REFUSED);
}

#[tokio::test]
async fn a_request_to_a_host_off_the_allowlist_never_reads_the_credential() {
    let (env, source) = env_with(vec![account_entry()]).await;
    *source.credential.lock().unwrap() = Some("gho_secret_token".into());
    let outcome = run(
        CONNECTOR_REQUEST,
        json!({ "connector": "GitHub", "method": "GET", "path": "https://evil.example/steal" }),
        &env,
    )
    .await;
    assert_eq!(error_code(&outcome), code::INVALID_ARGS);
    assert!(!format!("{:?}", outcome.error).contains("gho_secret_token"));
    assert!(source.audits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_unknown_connector_names_the_attached_ones() {
    let (env, _) = env_with(vec![account_entry()]).await;
    let outcome = run(
        CONNECTOR_REQUEST,
        json!({ "connector": "Slack", "method": "GET", "path": "/x" }),
        &env,
    )
    .await;
    assert_eq!(error_code(&outcome), code::NOT_FOUND);
    assert!(outcome.error.unwrap().message.contains("GitHub"));
}

#[test]
fn a_turn_makes_at_most_twenty_calls() {
    let budget = CallBudget::default();
    assert!((0..MAX_CALLS_PER_TURN).all(|_| budget.take()));
    assert!(!budget.take());
}

#[test]
fn policy_arguments_replace_what_the_model_sent() {
    let mut args = json!({ "connector": "c", "policy": "run", "connector_label": "Evil" });
    policy_arguments(CallPolicy::Run, "Docs", &mut args);
    assert!(args.get(POLICY_FIELD).is_none());
    assert_eq!(args[LABEL_FIELD], "Docs");
    policy_arguments(CallPolicy::AskEach, "Docs", &mut args);
    assert_eq!(args[POLICY_FIELD], POLICY_ASK_EACH);
    policy_arguments(CallPolicy::Ask, "Docs", &mut args);
    assert_eq!(args[POLICY_FIELD], POLICY_ASK);
}

#[test]
fn a_credential_never_prints() {
    let credential = Credential::from_secret("sk-very-secret-value");
    assert!(!format!("{credential:?}").contains("secret"));
    assert_eq!(credential.header(), Some("Bearer sk-very-secret-value"));
    assert_eq!(
        Credential::from_secret("Basic abc123def456").header(),
        Some("Basic abc123def456")
    );
    let mut text = "the token sk-very-secret-value leaked".to_string();
    credential.scrub(&mut text);
    assert_eq!(text, "the token [redacted] leaked");
}

#[cfg(feature = "mcp")]
mod fake_server {
    use flow_like_types::tokio::io::{AsyncReadExt, AsyncWriteExt};
    use flow_like_types::tokio::net::{TcpListener, TcpStream};

    use super::*;

    #[derive(Default)]
    pub struct Seen {
        pub authorization: Mutex<Vec<String>>,
        pub calls: Mutex<Vec<Value>>,
    }

    async fn read_request(stream: &mut TcpStream) -> Option<(String, Vec<u8>)> {
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 4096];
        let header_end = loop {
            if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                break at + 4;
            }
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
        };
        let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while buffer.len() < header_end + length {
            let read = stream.read(&mut chunk).await.ok()?;
            if read == 0 {
                return None;
            }
            buffer.extend_from_slice(&chunk[..read]);
        }
        Some((head, buffer[header_end..header_end + length].to_vec()))
    }

    fn answer(request: &Value) -> Option<Value> {
        let id = request.get("id")?.clone();
        let result = match request["method"].as_str()? {
            "initialize" => json!({
                "protocolVersion": request["params"]["protocolVersion"],
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "fake", "version": "1" },
            }),
            "tools/list" => json!({ "tools": [
                { "name": "search", "description": "Find docs",
                  "inputSchema": { "type": "object", "properties": { "q": { "type": "string" } } },
                  "annotations": { "readOnlyHint": true } },
                { "name": "drop", "description": "Delete docs",
                  "inputSchema": { "type": "object" },
                  "annotations": { "destructiveHint": true } },
                { "name": "big", "inputSchema": { "type": "object" } },
            ]}),
            "tools/call" => {
                let tool = request["params"]["name"].as_str().unwrap_or_default();
                let text = match tool {
                    "big" => "x".repeat(40_000),
                    "search" => format!("found {}", request["params"]["arguments"]["q"]),
                    other => format!("ran {other}"),
                };
                json!({ "content": [{ "type": "text", "text": text }], "isError": tool == "drop" })
            }
            _ => json!({}),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    async fn serve(mut stream: TcpStream, seen: Arc<Seen>) {
        let Some((head, body)) = read_request(&mut stream).await else {
            return;
        };
        if let Some(line) = head
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
        {
            seen.authorization
                .lock()
                .unwrap()
                .push(line.split_once(':').unwrap().1.trim().to_string());
        }
        let response = if head.starts_with("POST") {
            let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            if request["method"] == "tools/call" {
                seen.calls.lock().unwrap().push(request["params"].clone());
            }
            match answer(&request) {
                Some(reply) => {
                    let text = reply.to_string();
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                        text.len()
                    )
                }
                None => "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_string(),
            }
        } else {
            "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_string()
        };
        let _ = stream.write_all(response.as_bytes()).await;
    }

    pub async fn start() -> (String, Arc<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let seen = Arc::new(Seen::default());
        let shared = seen.clone();
        flow_like_types::tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                flow_like_types::tokio::spawn(serve(stream, shared.clone()));
            }
        });
        (url, seen)
    }
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn an_mcp_server_is_listed_called_and_audited() {
    let (url, seen) = fake_server::start().await;
    let (env, source) = env_with(vec![mcp_entry(&url, &[])]).await;
    *source.credential.lock().unwrap() = Some("tok-12345678".into());

    let listed = run(CONNECTOR_TOOLS, json!({ "names": ["search"] }), &env).await;
    assert!(listed.is_ok(), "{:?}", listed.error);
    let tools = listed.result["connectors"][0]["tools"].as_array().unwrap();
    let search = tools.iter().find(|t| t["name"] == "search").unwrap();
    assert_eq!(search["access"], "runs_directly");
    assert!(search["input_schema"]["properties"]["q"].is_object());
    let drop = tools.iter().find(|t| t["name"] == "drop").unwrap();
    assert_eq!(drop["access"], "asks_owner");
    assert!(drop.get("input_schema").is_none());

    let called = run(
        CONNECTOR_CALL,
        json!({ "connector": "c-docs", "tool": "search", "arguments": { "q": "cats" } }),
        &env,
    )
    .await;
    assert!(called.is_ok(), "{:?}", called.error);
    assert_eq!(called.result["text"], "found \"cats\"");
    assert_eq!(called.result["untrusted"], true);
    assert!(
        seen.authorization
            .lock()
            .unwrap()
            .iter()
            .all(|header| header == "Bearer tok-12345678")
    );
    assert_eq!(seen.calls.lock().unwrap()[0]["arguments"]["q"], "cats");
    let audits = source.audits.lock().unwrap();
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].operation, "search");
    assert_eq!(audits[0].effect, "read");
    assert!(audits[0].ok);
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn a_long_result_goes_to_the_workspace_and_a_tool_error_is_an_error() {
    let (url, _) = fake_server::start().await;
    let (env, _) = env_with(vec![mcp_entry(&url, &[])]).await;
    let big = run(
        CONNECTOR_CALL,
        json!({ "connector": "Docs", "tool": "big" }),
        &env,
    )
    .await;
    assert!(big.is_ok(), "{:?}", big.error);
    assert_eq!(big.result["preview_only"], true);
    assert_eq!(big.saved.len(), 1);
    assert!(big.saved[0].path.starts_with("bot/connectors/"));
    assert_eq!(big.saved[0].bytes, 40_000);
    let failed = run(
        CONNECTOR_CALL,
        json!({ "connector": "Docs", "tool": "drop" }),
        &env,
    )
    .await;
    assert_eq!(error_code(&failed), code::FAILED);
    assert!(failed.error.unwrap().message.contains("ran drop"));
}

#[cfg(feature = "mcp")]
#[tokio::test]
async fn a_server_does_not_reach_a_private_mcp_url() {
    let (url, _) = fake_server::start().await;
    let url = url.replacen("http://", "https://", 1);
    let (env, source) = env_with(vec![mcp_entry(&url, &[])]).await;
    *env.environment.lock().unwrap() = crate::env::Environment::Server;
    let outcome = run(
        CONNECTOR_CALL,
        json!({ "connector": "Docs", "tool": "search", "arguments": {} }),
        &env,
    )
    .await;
    assert_eq!(error_code(&outcome), code::FAILED);
    assert!(outcome.error.unwrap().message.contains("public"));
    assert!(!source.audits.lock().unwrap().is_empty());
}

#[test]
fn grants_name_the_connector_and_the_tool_and_skip_destructive_calls() {
    let call = json!({ "connector": "c-docs", "tool": "search" });
    assert_eq!(
        grant_scope(CONNECTOR_CALL, &call).as_deref(),
        Some("connector:c-docs:search")
    );
    let request = json!({ "connector": "c-gh", "method": "post", "path": "/x" });
    assert_eq!(
        grant_scope(CONNECTOR_REQUEST, &request).as_deref(),
        Some("connector:c-gh:http:POST")
    );
    let destructive = json!({ "connector": "c-docs", "tool": "drop", "policy": POLICY_ASK_EACH });
    assert_eq!(grant_scope(CONNECTOR_CALL, &destructive), None);
    assert_eq!(grant_scope(CONNECTOR_CALL, &json!({ "tool": "x" })), None);
}
