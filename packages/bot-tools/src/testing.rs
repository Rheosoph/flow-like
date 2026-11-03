use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use flow_like_model_provider::llm::{ModelConstructor, ModelLogic};
use flow_like_storage::contracts::column_metadata::ColumnReader;
use flow_like_storage::databases::vector::lancedb::connect_lance;
use flow_like_types::bot_message::WorkspaceHandle;
use flow_like_types::data_classification::DataClassification;
use flow_like_types::{Bytes, Result, anyhow};
use tempfile::TempDir;

use crate::code::{LanguageSupport, RunRequest, RunResponse, RunStatus, SandboxBackend};
use crate::convert::{ConvertInput, Converted, DocumentConverter};
use crate::data_backend::{DataBackend, NetworkTable, SourceTable};
use crate::documents::{
    DocumentBackend, ExcelRequest, Extracted, ModelTokens, RenderedPng, Summary, SummaryRequest,
};
use crate::embed::KnowledgeEmbedder;
use crate::env::{
    Access, AppGrant, AppTables, Attachment, Environment, FileData, FileEntry, FileMeta, Limits,
    MemorySessions, ModelRole, Scope, SessionCache, ToolEnv,
};

pub const WORKSPACE: &str = "ws-bot";
pub const OTHER_APP: &str = "app-other";

type Files = BTreeMap<(String, String), (Bytes, String, Option<DataClassification>)>;

pub struct FakeEnv {
    pub scope: Scope,
    pub limits: Limits,
    pub files: Mutex<Files>,
    pub modified: Mutex<HashMap<(String, String), i64>>,
    pub reads: std::sync::atomic::AtomicUsize,
    pub writes: Mutex<Vec<String>>,
    pub attachments: Mutex<HashMap<(u64, String), Attachment>>,
    pub granted: Mutex<Vec<WorkspaceHandle>>,
    pub embedder: Mutex<Option<Arc<dyn KnowledgeEmbedder>>>,
    pub converter: Mutex<Option<Arc<dyn DocumentConverter>>>,
    pub cancelled: AtomicBool,
    pub backend: Mutex<Option<Arc<FakeBackend>>>,
    pub data: Arc<FakeDataBackend>,
    pub sandbox: Mutex<Option<Arc<FakeSandbox>>>,
    pub real_sandbox: Mutex<Option<Arc<dyn SandboxBackend>>>,
    pub shell: Mutex<Option<Arc<dyn crate::shell::ShellBackend>>>,
    pub mail: Mutex<Option<Arc<dyn crate::email::MailBackend>>>,
    pub sessions: Arc<MemorySessions>,
    pub use_sessions: AtomicBool,
    pub table_opens: std::sync::atomic::AtomicUsize,
    pub environment: Mutex<Environment>,
    pub secrets: Mutex<HashMap<String, String>>,
    pub notifier: Mutex<Option<Arc<dyn crate::notify::OwnerNotifier>>>,
    pub scheduler: Mutex<Option<Arc<dyn crate::schedule::Scheduler>>>,
    app_dbs: Mutex<
        HashMap<
            String,
            (
                flow_like_storage::lancedb::Connection,
                ColumnReader,
                TempDir,
            ),
        >,
    >,
    pub expensive_ok: AtomicBool,
    pub writable: AtomicBool,
    connection: flow_like_storage::lancedb::Connection,
    _dir: TempDir,
}

impl FakeEnv {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let connection = connect_lance(dir.path().to_str().expect("utf8 path"))
            .execute()
            .await
            .expect("lance connection");
        Self {
            scope: Scope {
                workspace: Some(WORKSPACE.to_string()),
                apps: Vec::new(),
                storage: false,
                data_connections: false,
                classification: DataClassification::Internal,
            },
            limits: Limits::default(),
            files: Mutex::default(),
            modified: Mutex::default(),
            reads: std::sync::atomic::AtomicUsize::new(0),
            writes: Mutex::default(),
            attachments: Mutex::default(),
            granted: Mutex::default(),
            embedder: Mutex::new(Some(Arc::new(HashEmbedder::new("embed-a", 16)))),
            converter: Mutex::new(Some(Arc::new(PageConverter))),
            cancelled: AtomicBool::new(false),
            backend: Mutex::new(Some(Arc::new(FakeBackend::default()))),
            data: Arc::new(FakeDataBackend::default()),
            sandbox: Mutex::new(Some(Arc::new(FakeSandbox::default()))),
            real_sandbox: Mutex::new(None),
            shell: Mutex::new(None),
            mail: Mutex::new(None),
            sessions: Arc::new(MemorySessions::default()),
            use_sessions: AtomicBool::new(true),
            table_opens: std::sync::atomic::AtomicUsize::new(0),
            environment: Mutex::new(Environment::Local),
            secrets: Mutex::default(),
            notifier: Mutex::new(None),
            scheduler: Mutex::new(None),
            app_dbs: Mutex::default(),
            expensive_ok: AtomicBool::new(false),
            writable: AtomicBool::new(true),
            connection,
            _dir: dir,
        }
    }

    pub fn with_storage(mut self, app_id: &str) -> Self {
        self.scope.storage = true;
        self.scope.apps.push(AppGrant {
            app_id: app_id.to_string(),
            access: Access::Read,
            alias: None,
        });
        self
    }

    /// Gives the bot read access to an app's tables, without the storage capability.
    pub async fn with_app_tables(mut self, app_id: &str, alias: Option<&str>) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let connection = connect_lance(dir.path().to_str().expect("utf8 path"))
            .execute()
            .await
            .expect("lance connection");
        self.app_dbs.lock().expect("apps").insert(
            app_id.to_string(),
            (connection, ColumnReader::Unrestricted, dir),
        );
        self.scope.apps.push(AppGrant {
            app_id: app_id.to_string(),
            access: Access::Read,
            alias: alias.map(str::to_string),
        });
        self
    }

    pub fn app_connection(&self, app_id: &str) -> flow_like_storage::lancedb::Connection {
        if app_id == WORKSPACE {
            return self.connection.clone();
        }
        self.app_dbs
            .lock()
            .expect("apps")
            .get(app_id)
            .expect("app db")
            .0
            .clone()
    }

    pub fn restrict_app(&self, app_id: &str, reader: ColumnReader) {
        if let Some(entry) = self.app_dbs.lock().expect("apps").get_mut(app_id) {
            entry.1 = reader;
        }
    }

    pub fn set_secret(&self, name: &str, value: &str) {
        self.secrets
            .lock()
            .expect("secrets")
            .insert(name.to_string(), value.to_string());
    }

    pub fn sandbox(&self) -> Arc<FakeSandbox> {
        self.sandbox
            .lock()
            .expect("sandbox")
            .clone()
            .expect("sandbox present")
    }

    pub fn data(&self) -> Arc<FakeDataBackend> {
        self.data.clone()
    }

    pub fn put(&self, app: &str, path: &str, bytes: &[u8], mime: &str) {
        self.files.lock().expect("files").insert(
            (app.to_string(), path.to_string()),
            (Bytes::copy_from_slice(bytes), mime.to_string(), None),
        );
    }

    pub fn touch(&self, app: &str, path: &str, modified_ms: i64) {
        self.modified
            .lock()
            .expect("modified")
            .insert((app.to_string(), path.to_string()), modified_ms);
    }

    pub fn classify(&self, app: &str, path: &str, level: DataClassification) {
        if let Some(entry) = self
            .files
            .lock()
            .expect("files")
            .get_mut(&(app.to_string(), path.to_string()))
        {
            entry.2 = Some(level);
        }
    }

    pub fn get(&self, app: &str, path: &str) -> Option<Bytes> {
        self.files
            .lock()
            .expect("files")
            .get(&(app.to_string(), path.to_string()))
            .map(|entry| entry.0.clone())
    }

    pub fn attach(&self, seq: u64, part: &str, name: &str, handle: WorkspaceHandle) {
        self.attachments.lock().expect("attachments").insert(
            (seq, part.to_string()),
            Attachment {
                name: name.to_string(),
                handle,
            },
        );
    }

    pub fn backend(&self) -> Arc<FakeBackend> {
        self.backend
            .lock()
            .expect("backend")
            .clone()
            .expect("backend present")
    }

    pub fn write_count(&self) -> usize {
        self.writes.lock().expect("writes").len()
    }

    pub fn use_embedder(&self, embedder: Option<Arc<dyn KnowledgeEmbedder>>) {
        *self.embedder.lock().expect("embedder") = embedder;
    }
}

#[async_trait]
impl ToolEnv for FakeEnv {
    fn environment(&self) -> Environment {
        *self.environment.lock().expect("environment")
    }

    fn sessions(&self) -> Option<Arc<dyn SessionCache>> {
        self.use_sessions
            .load(Ordering::SeqCst)
            .then(|| self.sessions.clone() as Arc<dyn SessionCache>)
    }

    fn scheduler(&self) -> Option<Arc<dyn crate::schedule::Scheduler>> {
        self.scheduler.lock().expect("scheduler").clone()
    }

    fn notifier(&self) -> Option<Arc<dyn crate::notify::OwnerNotifier>> {
        self.notifier.lock().expect("notifier").clone()
    }

    fn sandbox(&self) -> Option<Arc<dyn SandboxBackend>> {
        if let Some(real) = self.real_sandbox.lock().expect("real").clone() {
            return Some(real);
        }
        self.sandbox
            .lock()
            .expect("sandbox")
            .clone()
            .map(|sandbox| sandbox as Arc<dyn SandboxBackend>)
    }

    fn mail(&self) -> Option<Arc<dyn crate::email::MailBackend>> {
        self.mail.lock().expect("mail").clone()
    }

    fn shell(&self) -> Option<Arc<dyn crate::shell::ShellBackend>> {
        self.shell.lock().expect("shell").clone()
    }

    fn data_backend(&self) -> Option<Arc<dyn DataBackend>> {
        Some(self.data.clone())
    }

    fn scope(&self) -> &Scope {
        &self.scope
    }

    fn limits(&self) -> &Limits {
        &self.limits
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    async fn read_file(
        &self,
        app_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let files = self.files.lock().expect("files");
        Ok(files.get(&(app_id.to_string(), path.to_string())).map(
            |(bytes, mime, classification)| FileData {
                bytes: bytes.slice(..bytes.len().min(max_bytes as usize)),
                size: bytes.len() as u64,
                mime: Some(mime.clone()),
                classification: *classification,
            },
        ))
    }

    async fn stat_file(&self, app_id: &str, path: &str) -> Result<Option<FileMeta>> {
        let files = self.files.lock().expect("files");
        Ok(files
            .get(&(app_id.to_string(), path.to_string()))
            .map(|(bytes, mime, _)| FileMeta {
                bytes: bytes.len() as u64,
                mime: Some(mime.clone()),
            }))
    }

    async fn list_files(&self, app_id: &str, prefix: &str, limit: usize) -> Result<Vec<FileEntry>> {
        let files = self.files.lock().expect("files");
        let modified = self.modified.lock().expect("modified");
        Ok(files
            .iter()
            .filter(|((app, path), _)| app == app_id && path.starts_with(prefix))
            .take(limit)
            .map(|((app, path), (bytes, mime, _))| FileEntry {
                path: path.clone(),
                bytes: bytes.len() as u64,
                mime: Some(mime.clone()),
                modified_ms: modified.get(&(app.clone(), path.clone())).copied(),
            })
            .collect())
    }

    async fn write_workspace(&self, path: &str, data: Bytes, mime: &str) -> Result<()> {
        self.writes.lock().expect("writes").push(path.to_string());
        self.put(WORKSPACE, path, &data, mime);
        Ok(())
    }

    async fn message_attachment(&self, seq: u64, part: &str) -> Result<Option<Attachment>> {
        Ok(self
            .attachments
            .lock()
            .expect("attachments")
            .get(&(seq, part.to_string()))
            .cloned())
    }

    fn handle_granted(&self, handle: &WorkspaceHandle) -> bool {
        self.granted.lock().expect("granted").contains(handle)
    }

    async fn app_tables(&self, app_id: &str) -> Result<AppTables> {
        self.table_opens.fetch_add(1, Ordering::SeqCst);
        if app_id == WORKSPACE {
            return Ok(AppTables {
                connection: self.connection.clone(),
                writable: self.writable.load(Ordering::SeqCst),
                reader: ColumnReader::Unrestricted,
            });
        }
        let apps = self.app_dbs.lock().expect("apps");
        let (connection, reader, _) = apps
            .get(app_id)
            .ok_or_else(|| anyhow!("The fake has no tables for app '{app_id}'"))?;
        Ok(AppTables {
            connection: connection.clone(),
            writable: false,
            reader: reader.clone(),
        })
    }

    async fn model(&self, _role: ModelRole) -> Result<Arc<dyn ModelLogic>> {
        Ok(Arc::new(StubModel))
    }

    fn documents(&self) -> Option<Arc<dyn DocumentBackend>> {
        self.backend
            .lock()
            .expect("backend")
            .clone()
            .map(|backend| backend as Arc<dyn DocumentBackend>)
    }

    fn allow_expensive_documents(&self) -> bool {
        self.expensive_ok.load(Ordering::SeqCst)
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        Ok(self.embedder.lock().expect("embedder").clone())
    }

    fn converter(&self) -> Option<Arc<dyn DocumentConverter>> {
        self.converter.lock().expect("converter").clone()
    }

    async fn secret(&self, name: &str) -> Result<Option<String>> {
        Ok(self.secrets.lock().expect("secrets").get(name).cloned())
    }
}

/// Chunks on blank lines; vectors are hashed bags of words unless `fixed` names a text.
pub struct HashEmbedder {
    model: String,
    dimension: usize,
    fixed: Vec<(String, Vec<f32>)>,
}

impl HashEmbedder {
    pub fn new(model: &str, dimension: usize) -> Self {
        Self {
            model: model.to_string(),
            dimension,
            fixed: Vec::new(),
        }
    }

    pub fn fixed(mut self, text: &str, vector: Vec<f32>) -> Self {
        self.fixed.push((text.to_string(), vector));
        self
    }

    fn vector(&self, text: &str) -> Vec<f32> {
        if let Some((_, vector)) = self.fixed.iter().find(|(known, _)| known == text) {
            return vector.clone();
        }
        let mut vector = vec![0.0_f32; self.dimension];
        for word in text.to_lowercase().split(|c: char| !c.is_alphanumeric()) {
            if word.is_empty() {
                continue;
            }
            let bucket = word.bytes().fold(7_usize, |hash, b| {
                hash.wrapping_mul(31).wrapping_add(b as usize)
            });
            vector[bucket % self.dimension] += 1.0;
        }
        if vector.iter().all(|v| *v == 0.0) {
            vector[0] = 1.0;
        }
        vector
    }
}

#[async_trait]
impl KnowledgeEmbedder for HashEmbedder {
    fn model_id(&self) -> &str {
        &self.model
    }

    fn split(&self, markdown: &str) -> Result<Vec<String>> {
        Ok(markdown
            .split("\n\n")
            .map(str::trim)
            .filter(|piece| !piece.is_empty())
            .map(str::to_string)
            .collect())
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|text| self.vector(text)).collect())
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.vector(text))
    }
}

/// Treats a form feed as a page break, so converted documents carry page starts.
pub struct PageConverter;

#[async_trait]
impl DocumentConverter for PageConverter {
    async fn convert(&self, input: ConvertInput<'_>) -> Result<Converted> {
        if input.mime != "application/x-paged" {
            return Err(anyhow!("No converter for {}", input.mime));
        }
        let raw = String::from_utf8_lossy(input.bytes).into_owned();
        let mut text = String::new();
        let mut page_starts = Vec::new();
        for page in raw.split('\u{c}') {
            page_starts.push(text.len());
            text.push_str(page);
            text.push_str("\n\n");
        }
        Ok(Converted { text, page_starts })
    }
}

pub struct StubModel;

#[async_trait]
impl ModelLogic for StubModel {
    async fn provider(&self) -> Result<ModelConstructor> {
        Err(anyhow!("The stub model answers nothing"))
    }

    async fn default_model(&self) -> Option<String> {
        Some("stub".to_string())
    }
}

/// Reads "PAGES:n" in a PDF's bytes for its page count; records what the tools asked for.
#[derive(Default)]
pub struct FakeBackend {
    pub goals: Mutex<Vec<String>>,
    pub extractions: Mutex<Vec<String>>,
    pub rendered: Mutex<Vec<Vec<usize>>>,
    pub failed_pages: Mutex<Vec<u32>>,
    pub excel_requests: Mutex<Vec<String>>,
}

impl FakeBackend {
    pub fn calls(&self) -> usize {
        self.goals.lock().expect("goals").len()
            + self.extractions.lock().expect("extractions").len()
            + self.rendered.lock().expect("rendered").len()
    }
}

fn pages_in(bytes: &[u8]) -> usize {
    let text = String::from_utf8_lossy(bytes);
    text.split("PAGES:")
        .nth(1)
        .and_then(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(1)
}

#[async_trait]
impl DocumentBackend for FakeBackend {
    async fn summarize(
        &self,
        _model: Arc<dyn ModelLogic>,
        request: SummaryRequest<'_>,
    ) -> Result<Summary> {
        self.goals
            .lock()
            .expect("goals")
            .push(request.goal.to_string());
        Ok(Summary {
            summary: format!("About {}: {} bytes read", request.goal, request.bytes.len()),
            key_points: vec!["first point (p.1)".to_string()],
            failed_pages: self.failed_pages.lock().expect("failed").clone(),
            markdown: "## Summary\n\nnotes".to_string(),
            tokens: ModelTokens {
                input: 1200,
                output: 80,
            },
            warnings: Vec::new(),
        })
    }

    async fn extract(
        &self,
        _model: Arc<dyn ModelLogic>,
        text: &str,
        _schema: &str,
        hint: &str,
    ) -> Result<Extracted> {
        if text.contains("BROKEN") {
            return Err(anyhow!("the model returned nothing usable"));
        }
        self.extractions
            .lock()
            .expect("extractions")
            .push(hint.to_string());
        Ok(Extracted {
            value: serde_json::json!({ "first_word": text.split_whitespace().next() }),
            tokens: ModelTokens {
                input: 300,
                output: 20,
            },
            warnings: Vec::new(),
        })
    }

    async fn convert(&self, bytes: Bytes, _extension: Option<&str>) -> Result<Converted> {
        Ok(Converted::plain(
            String::from_utf8_lossy(&bytes).into_owned(),
        ))
    }

    async fn excel(&self, _bytes: Bytes, request: ExcelRequest) -> Result<String> {
        self.excel_requests.lock().expect("excel").push(format!(
            "{:?}|{:?}|{:?}|{:?}",
            request.operation, request.sheet, request.range, request.style.bold
        ));
        Ok("sheet text".to_string())
    }

    fn page_count(&self, pdf: Vec<u8>) -> Result<usize> {
        Ok(pages_in(&pdf))
    }

    fn render_pages(
        &self,
        _pdf: Vec<u8>,
        pages: &[usize],
        _max_pixels: u64,
    ) -> Result<Vec<RenderedPng>> {
        self.rendered.lock().expect("rendered").push(pages.to_vec());
        Ok(pages
            .iter()
            .map(|page| RenderedPng {
                page: *page,
                png: Bytes::from(format!("png-{page}")),
                width: 10,
                height: 20,
            })
            .collect())
    }
}

fn string_batch(
    columns: &[&str],
    rows: &[Vec<&str>],
) -> flow_like_storage::arrow_array::RecordBatch {
    use flow_like_storage::arrow_array::{ArrayRef, RecordBatch, StringArray};
    use flow_like_storage::arrow_schema::{DataType, Field, Schema};
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|name| Field::new(*name, DataType::Utf8, true))
            .collect::<Vec<_>>(),
    ));
    let arrays: Vec<ArrayRef> = (0..columns.len())
        .map(|index| {
            Arc::new(StringArray::from(
                rows.iter().map(|row| Some(row[index])).collect::<Vec<_>>(),
            )) as ArrayRef
        })
        .collect();
    RecordBatch::try_new(schema, arrays).expect("batch")
}

/// Parses CSV naively; "workbooks" hold two fixed sheets; databases answer one row per table
/// unless the host is "bad.example", which fails with the password in the message.
#[derive(Default)]
pub struct FakeDataBackend {
    pub network_calls: Mutex<Vec<(String, bool, bool, String)>>,
}

#[async_trait]
impl DataBackend for FakeDataBackend {
    async fn csv(&self, bytes: Bytes, delimiter: u8, has_header: bool) -> Result<SourceTable> {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let mut lines = text
            .lines()
            .map(|line| line.split(delimiter as char).collect::<Vec<_>>());
        let header: Vec<String> = if has_header {
            lines
                .next()
                .unwrap_or_default()
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        };
        let rows: Vec<Vec<&str>> = lines.collect();
        let width = rows.first().map_or(header.len(), Vec::len);
        let names: Vec<String> = if header.is_empty() {
            (1..=width).map(|i| format!("column_{i}")).collect()
        } else {
            header
        };
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        Ok(SourceTable {
            name: String::new(),
            batches: vec![string_batch(&refs, &rows)],
        })
    }

    async fn workbook(
        &self,
        _bytes: Bytes,
        sheet: Option<&str>,
        _detect_tables: bool,
    ) -> Result<(Vec<SourceTable>, Vec<String>)> {
        let all = [
            ("Sales", vec![vec!["north", "10"], vec!["south", "20"]]),
            ("Cost Centers", vec![vec!["hq", "5"]]),
        ];
        let tables = all
            .iter()
            .filter(|(name, _)| sheet.is_none_or(|wanted| wanted == *name))
            .map(|(name, rows)| SourceTable {
                name: name.to_string(),
                batches: vec![string_batch(&["label", "amount"], rows)],
            })
            .collect();
        Ok((tables, vec!["merged cells were flattened".to_string()]))
    }

    async fn network(
        &self,
        source: &NetworkTable,
        server: bool,
        public_only: bool,
        _max_rows: usize,
    ) -> Result<SourceTable> {
        self.network_calls.lock().expect("calls").push((
            source.host.clone(),
            server,
            public_only,
            source.password.clone(),
        ));
        if source.host == "bad.example" {
            return Err(anyhow!("login failed for password {}", source.password));
        }
        Ok(SourceTable {
            name: source.table.clone(),
            batches: vec![string_batch(
                &["id", "name"],
                &[vec!["1", "acme"], vec!["2", "globex"]],
            )],
        })
    }
}

/// Answers every run with `reply` (or a small successful default) and records the requests.
pub struct FakeSandbox {
    pub requests: Mutex<Vec<RunRequest>>,
    pub reply: Mutex<Option<RunResponse>>,
}

impl Default for FakeSandbox {
    fn default() -> Self {
        Self {
            requests: Mutex::default(),
            reply: Mutex::default(),
        }
    }
}

impl FakeSandbox {
    pub fn respond(&self, status: RunStatus, stdout: &str, error: Option<&str>) {
        *self.reply.lock().expect("reply") = Some(RunResponse {
            status,
            outputs: serde_json::json!({}),
            stdout: stdout.to_string(),
            stderr: "trace".to_string(),
            error: error.map(str::to_string),
            files: Vec::new(),
            truncated: false,
            duration_ms: 7,
        });
    }

    pub fn last(&self) -> RunRequest {
        self.requests
            .lock()
            .expect("requests")
            .last()
            .cloned()
            .expect("a run")
    }
}

#[async_trait]
impl SandboxBackend for FakeSandbox {
    fn languages(&self) -> Vec<LanguageSupport> {
        vec![LanguageSupport {
            id: "python".to_string(),
            installed: true,
            download_bytes: 0,
            packages: vec!["numpy".to_string(), "pandas".to_string()],
            ..LanguageSupport::default()
        }]
    }

    async fn run(&self, request: RunRequest) -> RunResponse {
        self.requests.lock().expect("requests").push(request);
        if let Some(reply) = self.reply.lock().expect("reply").clone() {
            return reply;
        }
        RunResponse {
            status: RunStatus::Ok,
            outputs: serde_json::json!({ "x": 1 }),
            stdout: "hello\n".to_string(),
            stderr: String::new(),
            error: None,
            files: vec![
                (
                    "out/result.csv".to_string(),
                    Bytes::from_static(b"a,b\n1,2\n"),
                ),
                ("in/data.csv".to_string(), Bytes::from_static(b"changed")),
            ],
            truncated: false,
            duration_ms: 12,
        }
    }
}
