use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use flow_like_model_provider::llm::ModelLogic;
use flow_like_storage::contracts::column_metadata::ColumnReader;
use flow_like_storage::lancedb::Connection;
use flow_like_types::bot_message::WorkspaceHandle;
use flow_like_types::data_classification::DataClassification;
use flow_like_types::{Bytes, Result};
use serde::Serialize;

use crate::browser::BrowserBackend;
use crate::code::SandboxBackend;
use crate::computer::ComputerBackend;
use crate::convert::DocumentConverter;
use crate::data_backend::DataBackend;
use crate::documents::DocumentBackend;
use crate::email::MailBackend;
use crate::embed::KnowledgeEmbedder;
use crate::shell::ShellBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Local,
    Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppGrant {
    pub app_id: String,
    pub access: Access,
    /// The app's schema name in SQL (`<alias>.<table>`); derived from the app id when unset.
    pub alias: Option<String>,
}

/// What the bot may touch. The workspace is always readable and writable.
#[derive(Debug, Clone)]
pub struct Scope {
    pub workspace: Option<String>,
    pub apps: Vec<AppGrant>,
    pub storage: bool,
    /// The `data_connections` capability: attaching network databases.
    pub data_connections: bool,
    pub classification: DataClassification,
}

impl Scope {
    pub fn is_workspace(&self, app_id: &str) -> bool {
        self.workspace.as_deref() == Some(app_id)
    }

    pub fn grant(&self, app_id: &str) -> Option<&AppGrant> {
        self.apps.iter().find(|grant| grant.app_id == app_id)
    }

    /// Tables of the workspace, or of an app the bot was given.
    pub fn can_read_tables(&self, app_id: &str) -> bool {
        self.is_workspace(app_id) || self.grant(app_id).is_some()
    }

    pub fn can_read_storage(&self, app_id: &str) -> bool {
        self.is_workspace(app_id) || (self.storage && self.grant(app_id).is_some())
    }
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub inline_cap_bytes: usize,
    pub max_read_bytes: u64,
    pub max_write_bytes: u64,
    pub max_ingest_bytes: u64,
    pub max_chunks_per_source: usize,
    pub max_files_per_add: usize,
    pub max_files_per_folder: usize,
    pub max_folder_bytes: u64,
    pub folder_scan_limit: usize,
    pub embed_batch: usize,
    pub list_limit: usize,
    pub knowledge_max_k: usize,
    pub data_default_rows: usize,
    pub data_max_rows: usize,
    pub data_memory_bytes: usize,
    pub snapshot_max_rows: usize,
    pub code_wall_time_secs: u64,
    pub code_memory_bytes: usize,
    pub code_max_input_bytes: u64,
    pub code_max_hosts: usize,
    pub shell_wall_time_secs: u64,
    pub shell_max_wall_time_secs: u64,
    pub shell_output_bytes: usize,
    pub shell_sync_file_bytes: u64,
    pub shell_sync_max_files: usize,
    pub shell_pull_max_files: usize,
    pub document_max_pages: usize,
    pub document_max_tokens: u64,
    pub pdf_shown_pages: usize,
    pub pdf_max_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            inline_cap_bytes: 16 * 1024,
            max_read_bytes: 16 * 1024 * 1024,
            max_write_bytes: 16 * 1024 * 1024,
            max_ingest_bytes: 64 * 1024 * 1024,
            max_chunks_per_source: 5000,
            max_files_per_add: 20,
            max_files_per_folder: 100,
            max_folder_bytes: 32 * 1024 * 1024,
            folder_scan_limit: 5000,
            embed_batch: 32,
            list_limit: 200,
            knowledge_max_k: 20,
            data_default_rows: 50,
            data_max_rows: 200,
            data_memory_bytes: 512 * 1024 * 1024,
            snapshot_max_rows: 5_000_000,
            code_wall_time_secs: 120,
            code_memory_bytes: 512 * 1024 * 1024,
            code_max_input_bytes: 64 * 1024 * 1024,
            code_max_hosts: 8,
            shell_wall_time_secs: 60,
            shell_max_wall_time_secs: 170,
            shell_output_bytes: 8 * 1024,
            shell_sync_file_bytes: 8 * 1024 * 1024,
            shell_sync_max_files: 50,
            shell_pull_max_files: 500,
            document_max_pages: 100,
            document_max_tokens: 200_000,
            pdf_shown_pages: 4,
            pdf_max_pixels: 2_000_000,
        }
    }
}

/// A file's content. `bytes` holds at most the requested maximum; `size` is the whole file.
#[derive(Debug, Clone)]
pub struct FileData {
    pub bytes: Bytes,
    pub size: u64,
    pub mime: Option<String>,
    pub classification: Option<DataClassification>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub bytes: u64,
    pub mime: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub bytes: u64,
    pub mime: Option<String>,
    pub modified_ms: Option<i64>,
}

/// A file a message carried, as the message's part `#seq.aN` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub handle: WorkspaceHandle,
}

pub struct AppTables {
    pub connection: Connection,
    pub writable: bool,
    /// Columns this reader may not read are hidden from every query.
    pub reader: ColumnReader,
}

/// Host-owned state that outlives one tool call, already scoped to one bot and conversation.
/// Entries expire after their TTL.
pub trait SessionCache: Send + Sync {
    fn get(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>>;
    fn put(&self, name: &str, value: Arc<dyn Any + Send + Sync>, ttl: Duration);
    fn remove(&self, name: &str);
}

/// A [`SessionCache`] in memory: what a host keeps per bot and conversation.
#[derive(Default)]
pub struct MemorySessions {
    entries: Mutex<HashMap<String, (Arc<dyn Any + Send + Sync>, Instant)>>,
}

impl SessionCache for MemorySessions {
    fn get(&self, name: &str) -> Option<Arc<dyn Any + Send + Sync>> {
        let mut entries = self.entries.lock().ok()?;
        match entries.get(name) {
            Some((value, expires)) if *expires > Instant::now() => Some(value.clone()),
            Some(_) => {
                entries.remove(name);
                None
            }
            None => None,
        }
    }

    fn put(&self, name: &str, value: Arc<dyn Any + Send + Sync>, ttl: Duration) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(name.to_string(), (value, Instant::now() + ttl));
        }
    }

    fn remove(&self, name: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(name);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRole {
    Default,
    Fast,
    /// Reads pages and images; the bot's vision model when it has one.
    Vision,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub message: String,
    pub fraction: Option<f32>,
}

/// The host side of every tool. A host answers for access: `read_file`, `list_files` and
/// `stat_file` read whatever app they are given, and the tools decide beforehand whether the
/// bot may ask. Writes go to the workspace only.
#[async_trait]
pub trait ToolEnv: Send + Sync {
    fn environment(&self) -> Environment;
    fn scope(&self) -> &Scope;
    fn limits(&self) -> &Limits;

    fn is_cancelled(&self) -> bool {
        false
    }

    fn progress(&self, _update: Progress) {}

    /// `None` when the file does not exist. `bytes` is cut at `max_bytes`.
    async fn read_file(&self, app_id: &str, path: &str, max_bytes: u64)
    -> Result<Option<FileData>>;

    async fn stat_file(&self, app_id: &str, path: &str) -> Result<Option<FileMeta>>;

    async fn list_files(&self, app_id: &str, prefix: &str, limit: usize) -> Result<Vec<FileEntry>>;

    async fn write_workspace(&self, path: &str, data: Bytes, mime: &str) -> Result<()>;

    async fn message_attachment(&self, seq: u64, part: &str) -> Result<Option<Attachment>>;

    /// Whether `handle` reached the bot in a message, so its tools may read it whatever its app.
    fn handle_granted(&self, _handle: &WorkspaceHandle) -> bool {
        false
    }

    /// Where tool state lives between calls (data sessions); `None` rebuilds it every call.
    fn sessions(&self) -> Option<Arc<dyn SessionCache>> {
        None
    }

    /// Parsers and database drivers for `data_sources`; the catalog's (feature `connectors`)
    /// when the host supplies none.
    fn data_backend(&self) -> Option<Arc<dyn DataBackend>> {
        crate::data_backend::default_backend()
    }

    async fn app_tables(&self, app_id: &str) -> Result<AppTables>;

    async fn model(&self, role: ModelRole) -> Result<Arc<dyn ModelLogic>>;

    /// The bot's embedding model; `None` when it has none.
    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>>;

    /// Turns documents into text; markitdown (feature `documents`) when the host supplies none.
    fn converter(&self) -> Option<Arc<dyn DocumentConverter>> {
        crate::documents::default_converter()
    }

    /// The services behind `file_summarize`, `file_extract` and `pdf_render`; the catalog's
    /// (feature `documents`) when the host supplies none.
    fn documents(&self) -> Option<Arc<dyn DocumentBackend>> {
        crate::documents::default_backend()
    }

    /// The decoder behind `barcode_read`; the catalog's (feature `barcodes`) when the host
    /// supplies none.
    fn barcodes(&self) -> Option<Arc<dyn crate::barcode::BarcodeReader>> {
        crate::barcode::default_reader()
    }

    /// The code sandbox; the code-interpreter's (feature `code`) when the host supplies none.
    fn sandbox(&self) -> Option<Arc<dyn SandboxBackend>> {
        crate::code::default_backend()
    }

    /// The languages `code_run` can run on this host (empty: do not advertise `code_run`).
    fn code_languages(&self) -> Vec<String> {
        self.sandbox()
            .map(|sandbox| {
                sandbox
                    .languages()
                    .into_iter()
                    .map(|language| language.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The process runner behind `shell_run`; `None` on hosts that cannot run commands.
    fn shell(&self) -> Option<Arc<dyn ShellBackend>> {
        None
    }

    /// The browser behind the `browser_*` tools, one per bot and conversation; `None` on hosts
    /// that cannot drive one.
    fn browser(&self) -> Option<Arc<dyn BrowserBackend>> {
        None
    }

    /// The screen, mouse and keyboard behind the `computer_*` tools; `None` on hosts without
    /// them or in a turn nobody is watching.
    fn computer(&self) -> Option<Arc<dyn ComputerBackend>> {
        None
    }

    /// Reaches the bot's owner for `message_owner`; `None` where the host cannot message the owner.
    fn notifier(&self) -> Option<Arc<dyn crate::notify::OwnerNotifier>> {
        None
    }

    /// Reaches the people the owner chats with for `list_people` and `message_person`; `None`
    /// where the bot is not on an account.
    fn people(&self) -> Option<Arc<dyn crate::people::PeopleBackend>> {
        None
    }

    /// Requests context through the current conversation. The person supplying it consents.
    fn user_context(&self) -> Option<Arc<dyn crate::user_context::UserContextBackend>> {
        None
    }

    /// Reads and updates this bot's own settings after host-enforced owner approval.
    fn configuration(&self) -> Option<Arc<dyn crate::configuration::ConfigurationBackend>> {
        None
    }

    /// The owner's bots for Flux's `list_bots`, `get_bot`, `create_bot` and `update_bot`, after
    /// host-enforced owner approval; `None` where the host can't manage them.
    fn crew(&self) -> Option<Arc<dyn crate::crew::CrewBackend>> {
        None
    }

    /// The bot's scheduled tasks; `None` where the host keeps none.
    fn scheduler(&self) -> Option<Arc<dyn crate::schedule::Scheduler>> {
        None
    }

    /// The bot's skills; `None` where the host keeps none.
    fn skills(&self) -> Option<Arc<dyn crate::skills::SkillHost>> {
        None
    }

    /// The bot's mail service; `None` where the host has none (local bots, other lanes' hosts).
    fn mail(&self) -> Option<Arc<dyn MailBackend>> {
        None
    }

    /// The bot's attached MCP servers and OAuth accounts; `None` where the host reaches none.
    fn connectors(&self) -> Option<Arc<dyn crate::services::ConnectorSource>> {
        None
    }

    /// Whether the owner granted standing approval for document calls above the page and
    /// token limits.
    fn allow_expensive_documents(&self) -> bool {
        false
    }

    /// The bot's image model for `image_generate`: its `image_model`, else the host's default
    /// image-capable Bit.
    async fn images(
        &self,
    ) -> std::result::Result<Arc<dyn crate::image::ImageBackend>, crate::image::ImageUnavailable>
    {
        Err(crate::image::ImageUnavailable::NotOnThisHost)
    }

    async fn secret(&self, name: &str) -> Result<Option<String>>;

    /// The bot's stored secrets by name and description, sorted by name; never their values.
    async fn secrets(&self) -> Result<Vec<crate::secrets::SecretInfo>>;
}
