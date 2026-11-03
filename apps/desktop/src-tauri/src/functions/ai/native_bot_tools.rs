//! The shared bot tools (files, knowledge, documents, data, Excel, code) on this device, for
//! interactive chats and headless turns alike. `DesktopToolEnv` is the host side of
//! `flow-like-bot-tools`; it checks scope again for everything it hands out.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use dashmap::DashMap;
use flow_like::{
    credentials::SharedCredentials,
    flow::copilot::{
        BotRunContext, HostTools, linked_tool_names, resolve_host_completion_model,
        tool_outcome_output,
    },
    flow_like_model_provider::llm::ModelLogic,
    flow_like_storage::{
        Path as ObjectPath,
        contracts::column_metadata::ColumnReader,
        display_object_path,
        files::store::FlowLikeStore,
        lancedb::Connection,
        object_store::{Error as ObjectStoreError, ObjectStoreExt},
    },
    models::llm::ModelUsageContext,
    profile::Profile,
    state::FlowLikeState,
};
use flow_like_bot_tools::{
    AppGrant, AppTables, Attachment, Environment, FileData, FileEntry, FileMeta, KnowledgeEmbedder,
    Limits, LocalShell, MemorySessions, ModelEmbedder, ModelRole, Scope, ShellBackend, ToolEnv,
    ToolOutcome, env::Access, files::guess_mime,
};
use flow_like_types::{
    Bytes, Result, anyhow, bot::BotCapability, bot_message::WorkspaceHandle,
    data_classification::DataClassification, tokio_util::sync::CancellationToken,
};
use futures::StreamExt;
use reqwest::Method;
use serde_json::Value;
use tauri::{AppHandle, Manager};

use super::{
    bot_tools::LocalMachine,
    frontend_tool_bridge::FrontendToolContext,
    native_platform_tools::{NativePlatformSession, segment},
};
use crate::{functions::local_bots::LocalBots, state::TauriFlowLikeState};

const SHELL_AVAILABLE: bool = cfg!(not(any(target_os = "ios", target_os = "android")));
const IDLE_SESSION: Duration = Duration::from_secs(15 * 60);
const EMBED_CHUNK_TOKENS: usize = 512;
const EMBED_CHUNK_OVERLAP: usize = 64;

/// What a bot turn gives its tools: the bot, and the model state the run was built on.
pub(crate) struct BotToolRuntime {
    pub(crate) context: Arc<BotRunContext>,
    pub(crate) state: Arc<FlowLikeState>,
    pub(crate) profile: Option<Arc<Profile>>,
    pub(crate) token: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) embedding_model_id: Option<String>,
    pub(crate) run_id: Option<String>,
}

impl std::fmt::Debug for BotToolRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BotToolRuntime")
            .field("bot_id", &self.context.bot_id)
            .field("run_id", &self.run_id)
            .finish()
    }
}

#[cfg(feature = "code-sandbox")]
fn code_languages() -> Vec<String> {
    use flow_like_bot_tools::{SandboxBackend, code::CatalogSandbox};
    CatalogSandbox::shared()
        .languages()
        .into_iter()
        .map(|language| language.id)
        .collect()
}

#[cfg(not(feature = "code-sandbox"))]
fn code_languages() -> Vec<String> {
    Vec::new()
}

/// The bot-tools tools this build links and this platform can run: no code where there is no
/// interpreter (iOS).
pub(crate) fn host_tools() -> HostTools {
    let tools = HostTools::linked()
        .with_code_languages(&code_languages())
        .with_shell(SHELL_AVAILABLE);
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    let tools = tools.with_local_machine();
    tools
}

pub(crate) fn supports(tool: &str) -> bool {
    linked_tool_names().contains(&tool)
}

pub(crate) async fn execute(
    app: &AppHandle,
    context: Option<&FrontendToolContext>,
    tool: &str,
    args: &Value,
) -> std::result::Result<Value, String> {
    let runtime = context
        .and_then(|context| context.bot.clone())
        .ok_or_else(|| format!("{tool} is a bot tool and runs only inside a bot's turn."))?;
    if !runtime.context.host_tools.runs(tool) {
        return Err(format!("This device does not run the bot tool {tool}."));
    }
    let session = NativePlatformSession::new(app, context).await?;
    session.assert_current_profile().await?;
    let options = super::native_tool_bridge::run_options(context);
    let gate = options.approval_gate.clone();
    let auto_approved = match &gate {
        Some(gate) => {
            options.auto_approve || gate.expensive_documents_allowed(tool, args).await
        }
        None => options.auto_approve,
    };
    let host = Arc::new(LiveHost::new(session, runtime.clone()));
    let scheduler =
        super::bot_tools::schedule::scheduler_for(app, &runtime.context.bot_id, host.clone()).await;
    let env = DesktopToolEnv::new(
        host.clone(),
        scope_of(&runtime.context),
        sessions_for(&runtime.context, context),
        super::native_tool_bridge::execution_cancellation(),
        auto_approved,
    )
    .with_scheduler(scheduler)
    .with_mail(
        super::bot_tools::mail::mail_for(app, &runtime.context.bot_id, host.clone()).await,
    )
    .with_shell(shell_backend(app, &runtime.context.bot_id).await)
    .with_notifier(crate::bot_host::notifier_for(
        app,
        &runtime.context.bot_id,
        &runtime.context.name,
    ))
    .with_machine(super::bot_tools::local_machine(
        app,
        &runtime.context.bot_id,
        context.and_then(|context| context.conversation_id.as_deref()),
        runtime.context.has(BotCapability::BrowserUse),
        runtime.context.has(BotCapability::ComputerUse),
    ));
    let mut value = run_tool(env, tool, args.clone()).await?;
    host.session.assert_current_profile().await?;
    if let (Some(gate), false) = (&gate, auto_approved) {
        value = gate.park_for_cost(tool, args, value);
    }
    Ok(value)
}

/// Runs `tool` on its own task, so a panic in a heavy backend fails the call and not the turn.
async fn run_tool(
    env: DesktopToolEnv,
    tool: &str,
    args: Value,
) -> std::result::Result<Value, String> {
    let name = tool.to_string();
    let outcome = tokio::spawn(async move { flow_like_bot_tools::run(&name, args, &env).await })
        .await
        .map_err(|error| format!("The bot tool {tool} stopped unexpectedly: {error}"))?;
    outcome_value(&outcome)
}

/// The tool result the model reads: `tool_outcome_output`, parsed so the bridge serializes it back
/// unchanged and the tool loop finds the image field.
pub(crate) fn outcome_value(outcome: &ToolOutcome) -> std::result::Result<Value, String> {
    serde_json::from_str(&tool_outcome_output(outcome))
        .map_err(|error| format!("The bot tool result is not valid JSON: {error}"))
}

pub(crate) fn scope_of(bot: &BotRunContext) -> Scope {
    let mut apps: Vec<&String> = Vec::new();
    if bot.has(BotCapability::SelectedApps) {
        apps.extend(bot.selected_app_ids.iter());
    }
    if bot.has(BotCapability::ProfileApps) {
        apps.extend(bot.profile_app_ids.iter());
    }
    apps.sort();
    apps.dedup();
    Scope {
        workspace: bot.workspace_app_id.clone(),
        apps: apps
            .into_iter()
            .filter(|app_id| bot.workspace_app_id.as_deref() != Some(app_id.as_str()))
            .map(|app_id| AppGrant {
                app_id: app_id.clone(),
                access: Access::Read,
                alias: None,
            })
            .collect(),
        storage: bot.has(BotCapability::Storage),
        data_connections: bot.has(BotCapability::DataConnections),
        classification: bot
            .classification
            .classification
            .unwrap_or(DataClassification::Internal),
    }
}

fn desktop_limits() -> Limits {
    Limits {
        data_memory_bytes: 1024 * 1024 * 1024,
        code_memory_bytes: 1024 * 1024 * 1024,
        ..Limits::default()
    }
}

static SESSIONS: LazyLock<DashMap<String, (Arc<MemorySessions>, Instant)>> =
    LazyLock::new(DashMap::new);

/// One cache per bot, conversation and set of grants, so a changed grant starts clean.
fn sessions_for(bot: &BotRunContext, context: Option<&FrontendToolContext>) -> Arc<MemorySessions> {
    let now = Instant::now();
    SESSIONS.retain(|_, (_, used)| now.duration_since(*used) < IDLE_SESSION);
    let scope = scope_of(bot);
    let grants: Vec<&str> = scope
        .apps
        .iter()
        .map(|grant| grant.app_id.as_str())
        .collect();
    let key = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        bot.bot_id,
        context
            .and_then(|context| context.conversation_id.as_deref())
            .unwrap_or(""),
        scope.workspace.as_deref().unwrap_or(""),
        scope.data_connections,
        grants.join(",")
    );
    let mut entry = SESSIONS
        .entry(key)
        .or_insert_with(|| (Arc::new(MemorySessions::default()), now));
    entry.1 = now;
    entry.0.clone()
}

/// Everything the env needs from this device, so the scope rules above it can be tested alone.
#[async_trait]
pub(crate) trait ToolHost: Send + Sync {
    async fn store(&self, app_id: &str) -> Result<FlowLikeStore>;
    async fn tables(&self, app_id: &str) -> Result<(Connection, ColumnReader)>;
    async fn model(&self) -> Result<Arc<dyn ModelLogic>>;
    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>>;
    async fn secret(&self, name: &str) -> Result<Option<String>>;
}

pub(crate) struct DesktopToolEnv {
    host: Arc<dyn ToolHost>,
    scope: Scope,
    limits: Limits,
    sessions: Arc<MemorySessions>,
    cancellation: CancellationToken,
    allow_expensive_documents: bool,
    shell: Option<Arc<dyn ShellBackend>>,
    machine: LocalMachine,
    scheduler: Option<Arc<dyn flow_like_bot_tools::Scheduler>>,
    notifier: Option<Arc<dyn flow_like_bot_tools::OwnerNotifier>>,
    mail: Option<Arc<dyn flow_like_bot_tools::email::MailBackend>>,
}

/// One working directory per bot next to the project directory, on local and account bots alike:
/// the workspace's storage may be an object store, so its files are mirrored here for the run.
fn shell_dir(project_dir: &std::path::Path, bot_id: &str) -> std::path::PathBuf {
    let name: String = bot_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    project_dir
        .parent()
        .unwrap_or(project_dir)
        .join("bot-shell")
        .join(name)
}

async fn shell_backend(app: &AppHandle, bot_id: &str) -> Option<Arc<dyn ShellBackend>> {
    if !SHELL_AVAILABLE {
        return None;
    }
    let settings = app.try_state::<crate::state::TauriSettingsState>()?;
    let project_dir = settings.0.lock().await.project_dir.clone();
    Some(Arc::new(LocalShell::new(shell_dir(&project_dir, bot_id))))
}

impl DesktopToolEnv {
    pub(crate) fn new(
        host: Arc<dyn ToolHost>,
        scope: Scope,
        sessions: Arc<MemorySessions>,
        cancellation: CancellationToken,
        allow_expensive_documents: bool,
    ) -> Self {
        Self {
            host,
            scope,
            limits: desktop_limits(),
            sessions,
            cancellation,
            allow_expensive_documents,
            shell: None,
            machine: LocalMachine::default(),
            scheduler: None,
            notifier: None,
            mail: None,
        }
    }

    pub(crate) fn with_notifier(
        mut self,
        notifier: Arc<dyn flow_like_bot_tools::OwnerNotifier>,
    ) -> Self {
        self.notifier = Some(notifier);
        self
    }

    pub(crate) fn with_mail(
        mut self,
        mail: Option<Arc<dyn flow_like_bot_tools::email::MailBackend>>,
    ) -> Self {
        self.mail = mail;
        self
    }

    pub(crate) fn with_machine(mut self, machine: LocalMachine) -> Self {
        self.machine = machine;
        self
    }

    pub(crate) fn with_scheduler(
        mut self,
        scheduler: Arc<dyn flow_like_bot_tools::Scheduler>,
    ) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    pub(crate) fn with_shell(mut self, shell: Option<Arc<dyn ShellBackend>>) -> Self {
        self.shell = shell;
        self
    }

    fn require_files(&self, app_id: &str) -> Result<()> {
        if self.scope.can_read_storage(app_id) {
            Ok(())
        } else {
            Err(anyhow!(
                "The bot may not read the files of app '{app_id}': it is neither its workspace nor an app it was given with file access"
            ))
        }
    }

    async fn locate(&self, app_id: &str, path: &str) -> Result<(FlowLikeStore, ObjectPath)> {
        let store = self.host.store(app_id).await?;
        let location = store.construct_upload(app_id, path).await?;
        let base = store.construct_upload(app_id, "").await?;
        if !location.prefix_matches(&base) {
            return Err(anyhow!("Path '{path}' is outside app '{app_id}'"));
        }
        Ok((store, location))
    }
}

fn relative_to(base: &ObjectPath, location: &ObjectPath) -> String {
    let base = display_object_path(base);
    let full = display_object_path(location);
    full.strip_prefix(&base)
        .map(|rest| rest.trim_start_matches('/').to_string())
        .unwrap_or(full)
}

#[async_trait]
impl ToolEnv for DesktopToolEnv {
    fn environment(&self) -> Environment {
        Environment::Local
    }

    fn scope(&self) -> &Scope {
        &self.scope
    }

    fn limits(&self) -> &Limits {
        &self.limits
    }

    fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    async fn read_file(
        &self,
        app_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        self.require_files(app_id)?;
        let (store, location) = self.locate(app_id, path).await?;
        let store = store.as_generic();
        let meta = match store.head(&location).await {
            Ok(meta) => meta,
            Err(ObjectStoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let end = meta.size.min(max_bytes);
        let bytes = if end == 0 {
            Bytes::new()
        } else {
            store.get_range(&location, 0..end).await?
        };
        Ok(Some(FileData {
            bytes,
            size: meta.size,
            mime: Some(guess_mime(path)),
            classification: None,
        }))
    }

    async fn stat_file(&self, app_id: &str, path: &str) -> Result<Option<FileMeta>> {
        self.require_files(app_id)?;
        let (store, location) = self.locate(app_id, path).await?;
        match store.as_generic().head(&location).await {
            Ok(meta) => Ok(Some(FileMeta {
                bytes: meta.size,
                mime: Some(guess_mime(path)),
            })),
            Err(ObjectStoreError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn list_files(&self, app_id: &str, prefix: &str, limit: usize) -> Result<Vec<FileEntry>> {
        self.require_files(app_id)?;
        let (store, location) = self.locate(app_id, prefix).await?;
        let base = store.construct_upload(app_id, "").await?;
        let mut stream = store.as_generic().list(Some(&location));
        let mut files = Vec::new();
        while let Some(item) = stream.next().await {
            let meta = item?;
            let path = relative_to(&base, &meta.location);
            files.push(FileEntry {
                mime: Some(guess_mime(&path)),
                path,
                bytes: meta.size,
                modified_ms: Some(meta.last_modified.timestamp_millis()),
            });
            if files.len() >= limit {
                break;
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    async fn write_workspace(&self, path: &str, data: Bytes, _mime: &str) -> Result<()> {
        let workspace = self
            .scope
            .workspace
            .clone()
            .ok_or_else(|| anyhow!("The bot has no workspace to write to"))?;
        let (store, location) = self.locate(&workspace, path).await?;
        store.put(&location, data).await
    }

    async fn message_attachment(&self, _seq: u64, _part: &str) -> Result<Option<Attachment>> {
        Ok(None)
    }

    fn handle_granted(&self, _handle: &WorkspaceHandle) -> bool {
        false
    }

    fn sessions(&self) -> Option<Arc<dyn flow_like_bot_tools::SessionCache>> {
        Some(self.sessions.clone())
    }

    async fn app_tables(&self, app_id: &str) -> Result<AppTables> {
        if !self.scope.can_read_tables(app_id) {
            return Err(anyhow!(
                "The bot may not read the tables of app '{app_id}': it is neither its workspace nor an app it was given"
            ));
        }
        let (connection, reader) = self.host.tables(app_id).await?;
        Ok(AppTables {
            connection,
            writable: self.scope.is_workspace(app_id),
            reader,
        })
    }

    async fn model(&self, _role: ModelRole) -> Result<Arc<dyn ModelLogic>> {
        self.host.model().await
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        self.host.embedder().await
    }

    fn allow_expensive_documents(&self) -> bool {
        self.allow_expensive_documents
    }

    fn mail(&self) -> Option<Arc<dyn flow_like_bot_tools::email::MailBackend>> {
        self.mail.clone()
    }

    fn scheduler(&self) -> Option<Arc<dyn flow_like_bot_tools::Scheduler>> {
        self.scheduler.clone()
    }

    fn shell(&self) -> Option<Arc<dyn ShellBackend>> {
        self.shell.clone()
    }

    fn notifier(&self) -> Option<Arc<dyn flow_like_bot_tools::OwnerNotifier>> {
        self.notifier.clone()
    }

    fn browser(&self) -> Option<Arc<dyn flow_like_bot_tools::BrowserBackend>> {
        self.machine.browser.clone()
    }

    fn computer(&self) -> Option<Arc<dyn flow_like_bot_tools::ComputerBackend>> {
        self.machine.computer.clone()
    }

    async fn secret(&self, name: &str) -> Result<Option<String>> {
        self.host.secret(name).await
    }
}

/// This device: local apps through the object store and the device databases, account apps
/// through the credentials the hub hands this signed-in user.
struct LiveHost {
    session: NativePlatformSession,
    runtime: Arc<BotToolRuntime>,
    credentials: tokio::sync::Mutex<HashMap<String, Arc<SharedCredentials>>>,
}

#[async_trait]
impl super::bot_tools::schedule::TaskApi for LiveHost {
    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> std::result::Result<Value, String> {
        self.session.request(method, path, body).await
    }
}

impl LiveHost {
    fn new(session: NativePlatformSession, runtime: Arc<BotToolRuntime>) -> Self {
        Self {
            session,
            runtime,
            credentials: Default::default(),
        }
    }

    async fn credentials(&self, app_id: &str) -> Result<Arc<SharedCredentials>> {
        let mut cached = self.credentials.lock().await;
        if let Some(credentials) = cached.get(app_id) {
            return Ok(credentials.clone());
        }
        let value = self
            .session
            .request(
                Method::GET,
                &format!("apps/{}/invoke/presign", segment(app_id)),
                None,
            )
            .await
            .map_err(|error| anyhow!("Credentials for app '{app_id}': {error}"))?;
        let credentials: Arc<SharedCredentials> =
            Arc::new(serde_json::from_value(value).map_err(|error| {
                anyhow!("The hub sent unusable credentials for '{app_id}': {error}")
            })?);
        cached.insert(app_id.to_string(), credentials.clone());
        Ok(credentials)
    }

    async fn is_local(&self, app_id: &str) -> Result<bool> {
        self.session
            .is_local(app_id)
            .await
            .map_err(|error| anyhow!(error))
    }

    fn usage_context(&self) -> ModelUsageContext {
        ModelUsageContext {
            app_id: self.runtime.context.workspace_app_id.clone(),
            run_id: self.runtime.run_id.clone(),
            api_base_url: None,
        }
    }

    fn profile(&self) -> Result<&Arc<Profile>> {
        self.runtime
            .profile
            .as_ref()
            .ok_or_else(|| anyhow!("This run has no profile to resolve models from"))
    }
}

#[async_trait]
impl ToolHost for LiveHost {
    async fn store(&self, app_id: &str) -> Result<FlowLikeStore> {
        if self.is_local(app_id).await? {
            let state = TauriFlowLikeState::construct(&self.session.app_handle).await?;
            let store = state.config.read().await.stores.app_storage_store.clone();
            return store.ok_or_else(|| anyhow!("This device has no app storage"));
        }
        self.credentials(app_id).await?.to_store(false).await
    }

    async fn tables(&self, app_id: &str) -> Result<(Connection, ColumnReader)> {
        if self.is_local(app_id).await? {
            let connection = crate::functions::app::tables::device_connection(
                &self.session.app_handle,
                app_id,
                None,
                false,
                None,
            )
            .await?;
            return Ok((connection, ColumnReader::Unrestricted));
        }
        let credentials = self.credentials(app_id).await?;
        let connection = crate::functions::app::tables::device_connection(
            &self.session.app_handle,
            app_id,
            Some(credentials),
            false,
            None,
        )
        .await?;
        Ok((
            connection,
            ColumnReader::with_attributes(Vec::<String>::new()),
        ))
    }

    async fn model(&self) -> Result<Arc<dyn ModelLogic>> {
        let state = &self.runtime.state;
        let bit =
            resolve_host_completion_model(state, self.profile()?, self.runtime.model_id.as_deref())
                .await?;
        state
            .model_factory
            .build(
                &bit,
                state.clone(),
                self.runtime.token.clone(),
                Some(self.usage_context()),
            )
            .await
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        let Some(model_id) = self.runtime.embedding_model_id.as_deref() else {
            return Ok(None);
        };
        let state = &self.runtime.state;
        let bit = self
            .profile()?
            .find_bit(model_id, state.http_client.clone())
            .await?;
        let logic = state
            .embedding_factory
            .build_text_routed(
                &bit,
                state.clone(),
                self.runtime.token.clone(),
                Some(self.usage_context()),
            )
            .await?;
        let embedder = ModelEmbedder::new(
            bit.id,
            logic,
            Some(EMBED_CHUNK_TOKENS),
            Some(EMBED_CHUNK_OVERLAP),
        )
        .await?;
        Ok(Some(Arc::new(embedder)))
    }

    async fn secret(&self, name: &str) -> Result<Option<String>> {
        let bot_id = self.runtime.context.bot_id.clone();
        let Some(bots) = self.session.app_handle.try_state::<LocalBots>() else {
            return Err(anyhow!("Bots on this device are unavailable"));
        };
        let lookup = bot_id.clone();
        let is_local = crate::functions::local_bots::on_store(&bots, move |store| {
            store.get_bot(&lookup).map(|bot| bot.is_some())
        })
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        if !is_local {
            return Err(anyhow!(
                "Secrets of account bots are not available on this desktop yet; the bot's secret '{name}' cannot be read here"
            ));
        }
        let opened =
            crate::functions::local_bots::on_store(&bots, move |store| store.open_secrets(&bot_id))
                .await
                .map_err(|error| anyhow!(error.to_string()))?;
        if opened
            .unreadable
            .iter()
            .any(|unreadable| unreadable == name)
        {
            return Err(anyhow!(
                "The bot's secret '{name}' was sealed with another device's key and cannot be opened here"
            ));
        }
        Ok(opened
            .readable
            .into_iter()
            .find(|secret| secret.name == name)
            .map(|secret| secret.value.to_string()))
    }
}

#[cfg(test)]
#[path = "native_bot_tools_tests.rs"]
mod tests;
