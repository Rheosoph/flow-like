use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::{KnowledgeEmbedder, ModelRole, ToolEnv, ToolOutcome, ToolOutput, run};
use flow_like_model_provider::llm::ModelLogic;
use flow_like_storage::arrow_schema::{DataType, Field, Schema};
use flow_like_storage::contracts::column_metadata::READ_ATTRIBUTES_KEY;
use flow_like_storage::databases::vector::VectorStore;
use flow_like_storage::databases::vector::lancedb::{LanceDBVectorStore, connect_lance};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::object_store::memory::InMemory;
use flow_like_types::data_classification::DataClassification;
use flow_like_types::{Result, anyhow};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::*;

const WORKSPACE: &str = "ws-bot";
const CRM: &str = "app-crm";
const DOCS: &str = "app-docs";

struct Access {
    stores: HashMap<String, FlowLikeStore>,
    connections: HashMap<String, Connection>,
    _dirs: Vec<TempDir>,
}

impl Access {
    async fn new(apps: &[&str]) -> Self {
        let mut stores = HashMap::new();
        let mut connections = HashMap::new();
        let mut dirs = Vec::new();
        for app in apps {
            stores.insert(
                app.to_string(),
                FlowLikeStore::Memory(Arc::new(InMemory::new())),
            );
            let dir = tempfile::tempdir().expect("tempdir");
            let connection = connect_lance(dir.path().to_str().expect("path"))
                .execute()
                .await
                .expect("lance");
            connections.insert(app.to_string(), connection);
            dirs.push(dir);
        }
        Self {
            stores,
            connections,
            _dirs: dirs,
        }
    }
}

#[async_trait]
impl AppAccess for Access {
    async fn store(&self, app_id: &str) -> Result<FlowLikeStore> {
        self.stores
            .get(app_id)
            .cloned()
            .ok_or_else(|| anyhow!("no store for {app_id}"))
    }

    async fn connection(&self, app_id: &str) -> Result<Connection> {
        self.connections
            .get(app_id)
            .cloned()
            .ok_or_else(|| anyhow!("no tables for {app_id}"))
    }
}

struct WordEmbedder;

#[async_trait]
impl KnowledgeEmbedder for WordEmbedder {
    fn model_id(&self) -> &str {
        "word-embed"
    }

    fn split(&self, markdown: &str) -> Result<Vec<String>> {
        Ok(markdown
            .split("\n\n")
            .map(str::trim)
            .filter(|chunk| !chunk.is_empty())
            .map(str::to_string)
            .collect())
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|text| vector(text)).collect())
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(vector(text))
    }
}

fn vector(text: &str) -> Vec<f32> {
    let lower = text.to_lowercase();
    ["cat", "dog", "tax", "invoice"]
        .iter()
        .map(|word| lower.matches(word).count() as f32)
        .chain([0.1])
        .collect()
}

struct Models {
    embedder: bool,
}

#[async_trait]
impl ModelSource for Models {
    async fn model(&self, _role: ModelRole) -> Result<Arc<dyn ModelLogic>> {
        Err(anyhow!("no chat model in this test"))
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        Ok(self
            .embedder
            .then(|| Arc::new(WordEmbedder) as Arc<dyn KnowledgeEmbedder>))
    }
}

fn app(id: &str, files: bool, tables: bool, attributes: Option<Vec<&str>>) -> JobApp {
    JobApp {
        app_id: id.to_string(),
        alias: Some(id.trim_start_matches("app-").to_string()),
        files,
        tables,
        column_attributes: attributes.map(|list| list.into_iter().map(str::to_string).collect()),
    }
}

fn scope(storage: bool, apps: Vec<JobApp>) -> JobScope {
    JobScope {
        storage,
        data_connections: false,
        classification: DataClassification::Internal,
        apps,
    }
}

async fn env(storage: bool, apps: Vec<JobApp>) -> (HostEnv, Arc<Access>) {
    let access = Arc::new(Access::new(&[WORKSPACE, CRM, DOCS]).await);
    let env = HostEnv::new(
        WORKSPACE,
        &scope(storage, apps),
        access.clone(),
        Arc::new(Models { embedder: true }),
    );
    (env, access)
}

async fn call(env: &HostEnv, tool: &str, args: Value) -> ToolOutcome {
    run(tool, args, env).await
}

async fn staff_table(access: &Access) {
    let schema = Schema::new(vec![
        Field::new("name", DataType::Utf8, true),
        Field::new("ssn", DataType::Utf8, true)
            .with_metadata([(READ_ATTRIBUTES_KEY.to_string(), "hr".to_string())].into()),
    ]);
    let mut store =
        LanceDBVectorStore::from_connection(access.connections[CRM].clone(), "staff".to_string())
            .await;
    store.create_empty_table(schema, true).await.expect("table");
    store
        .insert(vec![json!({ "name": "Ann", "ssn": "123" })])
        .await
        .expect("rows");
}

#[tokio::test]
async fn workspace_files_round_trip_through_the_tools() {
    let (env, _) = env(false, vec![]).await;
    let written = call(
        &env,
        "file_write",
        json!({ "path": "bot/notes/Übersicht (2)#1.md", "content": "hello server" }),
    )
    .await;
    assert!(written.is_ok(), "{:?}", written.error);
    let listed = call(&env, "file_list", json!({ "prefix": "bot/notes" })).await;
    assert!(listed.is_ok(), "{:?}", listed.error);
    assert_eq!(
        listed.result["files"][0]["path"],
        "bot/notes/Übersicht (2)#1.md"
    );
    let read = call(
        &env,
        "file_read",
        json!({ "file": "bot/notes/Übersicht (2)#1.md" }),
    )
    .await;
    assert!(read.is_ok(), "{:?}", read.error);
    assert_eq!(read.result["text"], "hello server");
}

#[tokio::test]
async fn app_files_need_the_storage_capability_and_the_owners_right() {
    let (env, access) = env(
        true,
        vec![app(DOCS, true, false, None), app(CRM, false, true, None)],
    )
    .await;
    use flow_like_storage::object_store::{ObjectStoreExt, PutPayload};
    let base = access.stores[DOCS]
        .construct_upload(DOCS, "a.txt")
        .await
        .unwrap();
    access.stores[DOCS]
        .as_generic()
        .put(&base, PutPayload::from_static(b"docs text"))
        .await
        .unwrap();
    let readable = call(&env, "file_read", json!({ "file": "app:app-docs/a.txt" })).await;
    assert!(readable.is_ok(), "{:?}", readable.error);
    assert_eq!(readable.result["text"], "docs text");

    let no_right = call(&env, "file_read", json!({ "file": "app:app-crm/a.txt" })).await;
    assert!(!no_right.is_ok());

    let (without_storage, _) = env_without_storage().await;
    let refused = call(
        &without_storage,
        "file_read",
        json!({ "file": "app:app-docs/a.txt" }),
    )
    .await;
    assert!(!refused.is_ok());
    assert_eq!(refused.error.expect("error").code, "scope_refused");
}

async fn env_without_storage() -> (HostEnv, Arc<Access>) {
    env(false, vec![app(DOCS, true, false, None)]).await
}

#[tokio::test]
async fn writes_reach_the_workspace_only() {
    let (env, access) = env(true, vec![app(DOCS, true, true, None)]).await;
    let refused = call(
        &env,
        "file_write",
        json!({ "path": "x.txt", "content": "no", "app_id": DOCS }),
    )
    .await;
    assert!(!refused.is_ok());
    let docs = access.stores[DOCS]
        .construct_upload(DOCS, "x.txt")
        .await
        .unwrap();
    use flow_like_storage::object_store::ObjectStoreExt;
    assert!(access.stores[DOCS].as_generic().head(&docs).await.is_err());
    let saved = call(
        &env,
        "file_write",
        json!({ "path": "x.txt", "content": "yes" }),
    )
    .await;
    assert!(saved.is_ok(), "{:?}", saved.error);
    let ws = access.stores[WORKSPACE]
        .construct_upload(WORKSPACE, "x.txt")
        .await
        .unwrap();
    assert!(
        access.stores[WORKSPACE]
            .as_generic()
            .head(&ws)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn app_tables_follow_the_owners_column_restrictions() {
    let (env, access) = env(false, vec![app(CRM, false, true, Some(vec![]))]).await;
    staff_table(&access).await;
    let allowed = call(
        &env,
        "data_query",
        json!({ "sql": "SELECT name FROM crm.staff" }),
    )
    .await;
    assert!(allowed.is_ok(), "{:?}", allowed.error);
    assert_eq!(allowed.result["rows"][0]["name"], "Ann");
    let hidden = call(
        &env,
        "data_query",
        json!({ "sql": "SELECT ssn FROM crm.staff" }),
    )
    .await;
    assert!(!hidden.is_ok());

    let (hr, access) = env_with_attributes(vec!["hr"]).await;
    staff_table(&access).await;
    let visible = call(
        &hr,
        "data_query",
        json!({ "sql": "SELECT ssn FROM crm.staff" }),
    )
    .await;
    assert!(visible.is_ok(), "{:?}", visible.error);
}

async fn env_with_attributes(attributes: Vec<&str>) -> (HostEnv, Arc<Access>) {
    env(false, vec![app(CRM, false, true, Some(attributes))]).await
}

#[tokio::test]
async fn an_app_without_the_table_right_cannot_be_queried() {
    let (env, access) = env(true, vec![app(CRM, true, false, None)]).await;
    staff_table(&access).await;
    let refused = call(
        &env,
        "data_query",
        json!({ "sql": "SELECT name FROM crm.staff" }),
    )
    .await;
    assert!(!refused.is_ok());
    assert!(env.app_tables(CRM).await.is_err());
}

#[tokio::test]
async fn workspace_tables_are_writable_and_app_tables_are_not() {
    let (env, _) = env(false, vec![app(CRM, false, true, None)]).await;
    assert!(env.app_tables(WORKSPACE).await.expect("workspace").writable);
    assert!(!env.app_tables(CRM).await.expect("app").writable);
}

#[tokio::test]
async fn knowledge_indexes_live_in_the_workspace() {
    let (env, _) = env(false, vec![]).await;
    call(
        &env,
        "file_write",
        json!({ "path": "bot/files/pets.md", "content": "The cat sleeps.\n\nTax invoice due." }),
    )
    .await;
    let created = call(&env, "knowledge_create", json!({ "name": "pets" })).await;
    assert!(created.is_ok(), "{:?}", created.error);
    let added = call(
        &env,
        "knowledge_add",
        json!({ "index": "pets", "files": ["bot/files/pets.md"] }),
    )
    .await;
    assert!(added.is_ok(), "{:?}", added.error);
    let hits = call(
        &env,
        "knowledge_search",
        json!({ "query": "cat", "indexes": ["pets"] }),
    )
    .await;
    assert!(hits.is_ok(), "{:?}", hits.error);
    assert!(
        hits.result["hits"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("cat"))
    );
}

#[tokio::test]
async fn knowledge_without_an_embedding_model_says_so() {
    let access = Arc::new(Access::new(&[WORKSPACE]).await);
    let env = HostEnv::new(
        WORKSPACE,
        &scope(false, vec![]),
        access,
        Arc::new(Models { embedder: false }),
    );
    let created = call(&env, "knowledge_create", json!({ "name": "pets" })).await;
    assert!(!created.is_ok());
    assert_eq!(created.error.expect("error").code, "no_embedder");
}

#[test]
fn the_scope_drops_apps_the_owner_may_not_read() {
    let job = scope(
        true,
        vec![app("a", true, false, None), app("b", false, false, None)],
    );
    let scope = job.to_scope(WORKSPACE);
    assert_eq!(scope.apps.len(), 1);
    assert_eq!(scope.apps[0].app_id, "a");
    assert!(scope.is_workspace(WORKSPACE));
}

#[test]
fn a_job_hash_covers_every_byte_of_the_job() {
    let job = r#"{"tool":"file_read"}"#;
    assert_eq!(job_hash(job), job_hash(job));
    assert_ne!(job_hash(job), job_hash(r#"{"tool":"file_read" }"#));
    assert_ne!(
        job_hash(job),
        blake3::hash(job.as_bytes()).to_hex().to_string()
    );
}

#[test]
fn images_move_into_the_tool_loop_field() {
    let mut output = ToolOutput::new(json!({ "page": 1 }));
    output.images.push(flow_like_bot_tools::ToolImage {
        label: "page".into(),
        mime: "image/png".into(),
        data: flow_like_types::Bytes::from_static(b"png"),
    });
    let outcome = ToolOutcome::from_output("pdf_render", output, 16 * 1024);
    let text = outcome_output(&outcome);
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value[IMAGES_FIELD][0]["media_type"], "image/png");
    assert_eq!(value["status"], "ok");
}

#[tokio::test]
async fn a_host_without_object_writes_refuses_workspace_files_but_keeps_tables() {
    let access = Arc::new(Access::new(&[WORKSPACE]).await);
    let env = HostEnv::new(
        WORKSPACE,
        &scope(false, vec![]),
        access,
        Arc::new(Models { embedder: true }),
    )
    .with_object_writes(false);
    let refused = call(
        &env,
        "file_write",
        json!({ "path": "a.txt", "content": "x" }),
    )
    .await;
    assert!(!refused.is_ok());
    assert!(env.app_tables(WORKSPACE).await.expect("tables").writable);
}

#[tokio::test]
async fn job_secrets_travel_in_the_job_open_in_the_env_and_never_print() {
    let secrets = JobSecrets(
        [("PG_SALES".to_string(), "pa55word".to_string())]
            .into_iter()
            .collect(),
    );
    let printed = format!("{secrets:?}");
    assert!(printed.contains("PG_SALES") && !printed.contains("pa55word"));
    let wire = serde_json::to_value(&secrets).unwrap();
    assert_eq!(wire, json!({ "PG_SALES": "pa55word" }));
    assert_eq!(serde_json::from_value::<JobSecrets>(wire).unwrap(), secrets);

    let (env, _) = env(false, vec![]).await;
    assert_eq!(env.secret("PG_SALES").await.unwrap(), None);
    assert!(env.secrets().await.unwrap().is_empty());
    let env = env.with_secrets(Arc::new(secrets));
    assert_eq!(
        env.secret("PG_SALES").await.unwrap().as_deref(),
        Some("pa55word")
    );
    let listed = call(&env, "secret_list", json!({})).await;
    assert!(listed.is_ok());
    let text = listed.result.to_string();
    assert!(text.contains("PG_SALES") && !text.contains("pa55word"));
}

const TURN: &str = "chat";

/// Files of the conversation `TURN`, as a host's own source serves them.
struct Shared(HashMap<&'static str, &'static str>);

#[async_trait]
impl ConversationFiles for Shared {
    fn serves(&self, conversation_id: &str) -> bool {
        conversation_id == TURN
    }

    async fn read(
        &self,
        _conversation_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<flow_like_bot_tools::FileData>> {
        Ok(self.0.get(path).map(|text| {
            let fits = text.len() as u64 <= max_bytes;
            flow_like_bot_tools::FileData {
                bytes: if fits {
                    text.as_bytes().to_vec().into()
                } else {
                    Default::default()
                },
                size: text.len() as u64,
                mime: Some("text/plain".to_string()),
                classification: None,
            }
        }))
    }

    async fn file_link(
        &self,
        _conversation_id: &str,
        path: &str,
    ) -> Result<Option<flow_like_bot_tools::FileLink>> {
        Ok(self
            .0
            .contains_key(path)
            .then(|| flow_like_bot_tools::FileLink {
                url: format!("https://files.example.com/chat/{path}?sig=1"),
                expires_at_ms: None,
            }))
    }
}

#[tokio::test]
async fn a_turn_file_gets_its_link_from_the_hosts_source() {
    let (env, _) = env(false, vec![]).await;
    let env = env.with_conversation_files(Arc::new(Shared(
        [("notes.txt", "from the chat")].into_iter().collect(),
    )));
    let linked = call(
        &env,
        "file_link",
        json!({ "file": conversation_handle(TURN, "notes.txt") }),
    )
    .await;
    assert!(linked.is_ok(), "{:?}", linked.error);
    assert_eq!(
        linked.result["url"],
        "https://files.example.com/chat/notes.txt?sig=1"
    );
}

fn conversation_handle(conversation_id: &str, path: &str) -> Value {
    json!({
        "app_id": format!("conversation_{conversation_id}"),
        "path": path,
        "bytes": 0,
        "sha256": "",
        "mime": "text/plain",
    })
}

#[tokio::test]
async fn a_conversation_file_comes_from_the_hosts_source_when_it_serves_the_conversation() {
    let (env, _) = env(false, vec![]).await;
    let env = env.with_conversation_files(Arc::new(Shared(
        [("notes.txt", "from the chat")].into_iter().collect(),
    )));

    let saved = call(
        &env,
        "file_save",
        json!({ "file": conversation_handle(TURN, "notes.txt") }),
    )
    .await;
    assert!(saved.is_ok(), "{:?}", saved.error);
    assert_eq!(saved.result["handle"]["app_id"], WORKSPACE);
    assert_eq!(saved.result["handle"]["path"], "bot/files/notes.txt");
    let copy = call(&env, "file_read", json!({ "file": "bot/files/notes.txt" })).await;
    assert_eq!(copy.result["text"], "from the chat");

    let missing = call(
        &env,
        "file_save",
        json!({ "file": conversation_handle(TURN, "other.txt") }),
    )
    .await;
    assert_eq!(
        missing.error.expect("a missing file fails").code,
        flow_like_bot_tools::error::code::NOT_FOUND
    );

    let unserved = call(
        &env,
        "file_save",
        json!({ "file": conversation_handle("room1", "notes.txt") }),
    )
    .await;
    let unserved = unserved
        .error
        .expect("the app store has no such conversation");
    assert!(
        unserved.message.contains("conversation_room1"),
        "{unserved:?}"
    );
}

#[tokio::test]
async fn a_conversation_file_over_the_read_limit_is_refused_by_its_size() {
    let (env, _) = env(false, vec![]).await;
    let env = env
        .with_limits(flow_like_bot_tools::Limits {
            max_read_bytes: 4,
            ..Default::default()
        })
        .with_conversation_files(Arc::new(Shared(
            [("notes.txt", "from the chat")].into_iter().collect(),
        )));
    let refused = call(
        &env,
        "file_save",
        json!({ "file": conversation_handle(TURN, "notes.txt") }),
    )
    .await;
    let refused = refused.error.expect("a file over the limit fails");
    assert_eq!(refused.code, flow_like_bot_tools::error::code::TOO_LARGE);
    assert!(refused.message.contains("13 bytes"), "{refused:?}");
}

/// Answers every request on a local port: `/ok` with `hello`, `/denied` with 403, anything else
/// with 404. Returns the port.
async fn file_server() -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("address").port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let request = String::from_utf8_lossy(&request).to_string();
                let (status, body) = if request.starts_with("GET /ok?") {
                    ("200 OK", "hello")
                } else if request.starts_with("GET /denied?") {
                    ("403 Forbidden", "")
                } else {
                    ("404 Not Found", "")
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn a_jobs_conversation_files_are_read_through_their_signed_links() {
    let port = file_server().await;
    let file = |path: &str, route: &str| JobFile {
        app_id: "conversation_room1".to_string(),
        path: path.to_string(),
        url: format!("http://127.0.0.1:{port}/{route}?X-Amz-Signature=s3cret"),
        mime: Some("text/plain".to_string()),
    };
    let files = LinkedFiles::new(vec![
        file("f1.txt", "ok"),
        file("f2.txt", "gone"),
        file("f3.txt", "denied"),
    ]);
    assert!(files.serves("room1") && files.serves("chat"));

    let read = files.read("room1", "f1.txt", 1024).await.unwrap().unwrap();
    assert_eq!(read.bytes.as_ref(), b"hello");
    assert_eq!(read.size, 5);
    assert_eq!(read.mime.as_deref(), Some("text/plain"));

    let too_large = files.read("room1", "f1.txt", 4).await.unwrap().unwrap();
    assert!(too_large.bytes.is_empty());
    assert_eq!(too_large.size, 5);

    assert!(files.read("room1", "f2.txt", 1024).await.unwrap().is_none());
    assert!(
        files
            .read("room1", "unlisted.txt", 1024)
            .await
            .unwrap()
            .is_none()
    );
    assert!(files.read("room2", "f1.txt", 1024).await.unwrap().is_none());

    let denied = files
        .read("room1", "f3.txt", 1024)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        denied.contains("f3.txt") && denied.contains("HTTP 403"),
        "{denied}"
    );
    assert!(
        !denied.contains("s3cret") && !denied.contains("127.0.0.1"),
        "{denied}"
    );

    let printed = format!("{:?}", file("f1.txt", "ok"));
    assert!(
        printed.contains("f1.txt") && !printed.contains("s3cret"),
        "{printed}"
    );
}

#[tokio::test]
async fn an_executor_job_saves_a_linked_conversation_file_into_the_workspace() {
    let port = file_server().await;
    let job_files = vec![JobFile {
        app_id: "conversation_room1".to_string(),
        path: "f1.txt".to_string(),
        url: format!("http://127.0.0.1:{port}/ok?X-Amz-Signature=s3cret"),
        mime: None,
    }];
    let (env, _) = env(false, vec![]).await;
    let env = env.with_conversation_files(Arc::new(LinkedFiles::new(job_files)));

    let saved = call(
        &env,
        "file_save",
        json!({ "file": conversation_handle("room1", "f1.txt") }),
    )
    .await;
    assert!(saved.is_ok(), "{:?}", saved.error);
    let copy = call(&env, "file_read", json!({ "file": "bot/files/f1.txt" })).await;
    assert_eq!(copy.result["text"], "hello");

    let unlinked = call(
        &env,
        "file_save",
        json!({ "file": conversation_handle("room1", "f9.txt") }),
    )
    .await;
    assert_eq!(
        unlinked.error.expect("a file without a link fails").code,
        flow_like_bot_tools::error::code::NOT_FOUND
    );
}
