use super::*;
use flow_like::flow_like_storage::{
    databases::vector::lancedb::connect_lance, object_store::memory::InMemory,
};
use flow_like_bot_tools::{Status, ToolImage, ToolOutput};
use flow_like_types::{create_id, tokio};
use serde_json::json;
use std::sync::Mutex;

const WORKSPACE: &str = "ws-bot";
const LOCAL_APP: &str = "app-local";
const ACCOUNT_APP: &str = "app-account";
const STRANGER: &str = "app-stranger";

struct HashEmbedder;

#[async_trait]
impl KnowledgeEmbedder for HashEmbedder {
    fn model_id(&self) -> &str {
        "embed-test"
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
    let mut vector = vec![0.0_f32; 16];
    for word in text.to_lowercase().split_whitespace() {
        let slot = word.bytes().fold(7_usize, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(byte as usize)
        });
        vector[slot % 16] += 1.0;
    }
    vector
}

struct TestHost {
    stores: HashMap<String, FlowLikeStore>,
    connection: Connection,
    secrets: Mutex<HashMap<String, String>>,
    _dir: std::path::PathBuf,
}

impl TestHost {
    async fn new() -> Arc<Self> {
        let dir = std::env::temp_dir().join(format!("bot-tools-{}", create_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let connection = connect_lance(dir.to_str().unwrap())
            .execute()
            .await
            .unwrap();
        let stores = [WORKSPACE, LOCAL_APP, ACCOUNT_APP, STRANGER]
            .into_iter()
            .map(|app| {
                (
                    app.to_string(),
                    FlowLikeStore::Memory(Arc::new(InMemory::new())),
                )
            })
            .collect();
        Arc::new(Self {
            stores,
            connection,
            secrets: Mutex::default(),
            _dir: dir,
        })
    }

    async fn put(&self, app_id: &str, path: &str, content: &str) {
        let store = &self.stores[app_id];
        let location = store.construct_upload(app_id, path).await.unwrap();
        store
            .put(&location, content.as_bytes().to_vec())
            .await
            .unwrap();
    }

    async fn text(&self, app_id: &str, path: &str) -> Option<String> {
        let store = &self.stores[app_id];
        let location = store.construct_upload(app_id, path).await.unwrap();
        let object = store.as_generic().get(&location).await.ok()?;
        Some(String::from_utf8(object.bytes().await.ok()?.to_vec()).unwrap())
    }
}

impl Drop for TestHost {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self._dir);
    }
}

#[async_trait]
impl ToolHost for TestHost {
    async fn store(&self, app_id: &str) -> Result<FlowLikeStore> {
        self.stores
            .get(app_id)
            .cloned()
            .ok_or_else(|| anyhow!("unknown app {app_id}"))
    }

    async fn tables(&self, _app_id: &str) -> Result<(Connection, ColumnReader)> {
        Ok((self.connection.clone(), ColumnReader::Unrestricted))
    }

    async fn model(&self) -> Result<Arc<dyn ModelLogic>> {
        Err(anyhow!("no model in tests"))
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        Ok(Some(Arc::new(HashEmbedder)))
    }

    async fn secret(&self, name: &str) -> Result<Option<String>> {
        Ok(self.secrets.lock().unwrap().get(name).cloned())
    }
}

fn bot(capabilities: &[BotCapability]) -> BotRunContext {
    BotRunContext {
        bot_id: "bot-test".into(),
        capabilities: capabilities.to_vec(),
        selected_app_ids: [LOCAL_APP, ACCOUNT_APP].map(str::to_string).into(),
        profile_app_ids: [STRANGER.to_string()].into(),
        workspace_app_id: Some(WORKSPACE.into()),
        host_tools: HostTools::linked(),
        ..Default::default()
    }
}

fn env_for(host: &Arc<TestHost>, bot: &BotRunContext) -> DesktopToolEnv {
    DesktopToolEnv::new(
        host.clone(),
        scope_of(bot),
        Arc::new(MemorySessions::default()),
        CancellationToken::new(),
        false,
    )
}

async fn call(env: &DesktopToolEnv, tool: &str, args: Value) -> Value {
    let outcome = flow_like_bot_tools::run(tool, args, env).await;
    outcome_value(&outcome).unwrap()
}

fn code(result: &Value) -> &str {
    result["error"]["code"].as_str().unwrap_or("")
}

#[tokio::test]
async fn files_of_local_and_account_apps_resolve_inside_the_scope_only() {
    let host = TestHost::new().await;
    host.put(LOCAL_APP, "notes/a.txt", "from the device").await;
    host.put(ACCOUNT_APP, "notes/b.txt", "from the account")
        .await;
    host.put(STRANGER, "notes/c.txt", "not yours").await;
    let granted = bot(&[BotCapability::Storage, BotCapability::SelectedApps]);
    let env = env_for(&host, &granted);

    let local = call(
        &env,
        "file_read",
        json!({"file": format!("app:{LOCAL_APP}/notes/a.txt")}),
    )
    .await;
    assert_eq!(local["status"], "ok");
    assert_eq!(local["text"], "from the device");
    let account = call(
        &env,
        "file_read",
        json!({"file": format!("app:{ACCOUNT_APP}/notes/b.txt")}),
    )
    .await;
    assert_eq!(account["text"], "from the account");
    let stranger = call(
        &env,
        "file_read",
        json!({"file": format!("app:{STRANGER}/notes/c.txt")}),
    )
    .await;
    assert_eq!(code(&stranger), "scope_refused");
    assert!(env.read_file(STRANGER, "notes/c.txt", 100).await.is_err());
    assert!(env.list_files(STRANGER, "", 10).await.is_err());

    let listed = call(&env, "file_list", json!({"app_id": ACCOUNT_APP})).await;
    assert_eq!(listed["files"][0]["path"], "notes/b.txt");
}

#[tokio::test]
async fn other_apps_files_need_the_storage_capability_and_a_grant() {
    let host = TestHost::new().await;
    host.put(LOCAL_APP, "a.txt", "x").await;
    host.put(STRANGER, "a.txt", "x").await;
    let args = |app: &str| json!({"file": format!("app:{app}/a.txt")});

    let no_storage = bot(&[BotCapability::SelectedApps]);
    let result = call(&env_for(&host, &no_storage), "file_read", args(LOCAL_APP)).await;
    assert_eq!(code(&result), "scope_refused");

    let no_apps = bot(&[BotCapability::Storage]);
    let result = call(&env_for(&host, &no_apps), "file_read", args(LOCAL_APP)).await;
    assert_eq!(code(&result), "scope_refused");

    let with_profile = bot(&[BotCapability::Storage, BotCapability::ProfileApps]);
    let result = call(&env_for(&host, &with_profile), "file_read", args(STRANGER)).await;
    assert_eq!(result["status"], "ok");
    assert!(!scope_of(&no_storage).data_connections);
    assert!(scope_of(&bot(&[BotCapability::DataConnections])).data_connections);
}

#[tokio::test]
async fn workspace_writes_land_in_the_bots_workspace_only() {
    let host = TestHost::new().await;
    let granted = bot(&[BotCapability::Storage, BotCapability::SelectedApps]);
    let env = env_for(&host, &granted);

    let written = call(
        &env,
        "file_write",
        json!({"path": "bot/notes/a.md", "content": "hello"}),
    )
    .await;
    assert_eq!(written["status"], "ok");
    assert_eq!(written["saved"][0]["app_id"], WORKSPACE);
    assert_eq!(
        host.text(WORKSPACE, "bot/notes/a.md").await.as_deref(),
        Some("hello")
    );

    let elsewhere = call(
        &env,
        "file_write",
        json!({"path": "bot/notes/b.md", "content": "no", "app_id": LOCAL_APP}),
    )
    .await;
    assert_eq!(elsewhere["status"], "error");
    assert!(host.text(LOCAL_APP, "bot/notes/b.md").await.is_none());

    let none = BotRunContext {
        workspace_app_id: None,
        ..granted
    };
    let result = call(
        &env_for(&host, &none),
        "file_write",
        json!({"path": "bot/x.md", "content": "x"}),
    )
    .await;
    assert_eq!(result["status"], "error");
}

#[tokio::test]
async fn a_knowledge_index_round_trips_with_the_bots_embedder() {
    let host = TestHost::new().await;
    host.put(
        WORKSPACE,
        "bot/docs/cats.md",
        "Cats purr softly.\n\nDogs bark loudly at night.",
    )
    .await;
    let granted = bot(&[BotCapability::Knowledge, BotCapability::Storage]);
    let env = env_for(&host, &granted);

    assert_eq!(
        call(&env, "knowledge_create", json!({"name": "pets"})).await["status"],
        "ok"
    );
    let added = call(
        &env,
        "knowledge_add",
        json!({"index": "pets", "files": ["bot/docs/cats.md"]}),
    )
    .await;
    assert_eq!(added["status"], "ok", "{added}");
    assert_eq!(added["added"][0]["chunks"], 2);
    let found = call(
        &env,
        "knowledge_search",
        json!({"query": "dogs bark", "indexes": ["pets"], "k": 1}),
    )
    .await;
    assert_eq!(found["status"], "ok", "{found}");
    assert!(
        found["hits"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Dogs bark")
    );
}

#[tokio::test]
async fn app_tables_follow_the_same_scope_and_only_the_workspace_is_writable() {
    let host = TestHost::new().await;
    let granted = bot(&[BotCapability::DataStudio, BotCapability::SelectedApps]);
    let env = env_for(&host, &granted);

    assert!(env.app_tables(WORKSPACE).await.unwrap().writable);
    assert!(!env.app_tables(LOCAL_APP).await.unwrap().writable);
    assert!(env.app_tables(STRANGER).await.is_err());
}

#[tokio::test]
async fn secrets_come_from_the_host_and_missing_ones_are_none() {
    let host = TestHost::new().await;
    host.secrets
        .lock()
        .unwrap()
        .insert("db".into(), "pw".into());
    let env = env_for(&host, &bot(&[BotCapability::DataConnections]));
    assert_eq!(env.secret("db").await.unwrap().as_deref(), Some("pw"));
    assert!(env.secret("other").await.unwrap().is_none());
}

#[tokio::test]
async fn dispatch_returns_the_tool_outcome_output_with_images_in_the_tool_loop_field() {
    let host = TestHost::new().await;
    host.put(WORKSPACE, "bot/a.txt", "hi").await;
    let env = env_for(&host, &bot(&[BotCapability::Storage]));

    let value = run_tool(
        env_for(&host, &bot(&[BotCapability::Storage])),
        "file_list",
        json!({}),
    )
    .await
    .unwrap();
    let direct = flow_like_bot_tools::run("file_list", json!({}), &env).await;
    assert_eq!(
        value,
        serde_json::from_str::<Value>(&tool_outcome_output(&direct)).unwrap()
    );
    assert_eq!(value["status"], "ok");

    let mut output = ToolOutput::new(json!({"pages": 1}));
    output.images.push(ToolImage {
        label: "page 1".into(),
        mime: "image/png".into(),
        data: Bytes::from_static(b"png"),
    });
    let outcome = ToolOutcome::from_output("pdf_render", output, 16 * 1024);
    assert_eq!(outcome.status, Status::Ok);
    let value = outcome_value(&outcome).unwrap();
    assert_eq!(value["_flowpilot_image_urls"][0]["media_type"], "image/png");
}

#[test]
fn only_linked_tools_are_dispatched_here_and_code_follows_the_platform() {
    assert!(supports("file_read"));
    assert!(supports("knowledge_search"));
    assert!(!supports("database_tool"));
    assert!(!supports("list_apps"));
    let tools = host_tools();
    assert!(tools.runs("file_read"));
    assert_eq!(tools.runs("code_run"), !code_languages().is_empty());
}

#[test]
fn the_shell_runs_on_desktops_only_and_each_bot_has_its_own_directory() {
    assert_eq!(host_tools().runs("shell_run"), SHELL_AVAILABLE);
    let projects = std::path::Path::new("/data/projects");
    let one = shell_dir(projects, "bot_1");
    assert_eq!(one, std::path::Path::new("/data/bot-shell/bot_1"));
    assert_ne!(one, shell_dir(projects, "bot_2"));
    assert_eq!(
        shell_dir(projects, "../../etc"),
        std::path::Path::new("/data/bot-shell/______etc")
    );
}

#[tokio::test]
async fn the_browser_and_the_computer_exist_only_where_the_machine_hands_them_out() {
    let host = TestHost::new().await;
    let bot = bot(&[BotCapability::BrowserUse]);
    let bare = env_for(&host, &bot);
    assert!(bare.browser().is_none() && bare.computer().is_none());
    let value = call(&bare, "browser_open", json!({ "url": "https://example.com" })).await;
    assert_eq!(code(&value), "not_yet");

    let tools = host_tools();
    let desktop = cfg!(any(target_os = "macos", windows, target_os = "linux"));
    for tool in ["browser_open", "computer_act", "computer_wait"] {
        assert_eq!(tools.runs(tool), desktop, "{tool}");
    }
}
