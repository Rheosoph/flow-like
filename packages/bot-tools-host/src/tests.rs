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
