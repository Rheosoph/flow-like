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
    Limits, LocalShell, MemorySessions, ModelEmbedder, ModelRole, Scope, SecretInfo, ShellBackend,
    ToolEnv, ToolOutcome, env::Access, files::guess_mime,
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
    /// The bot turn this run answers; an account bot reads its secrets for it.
    pub(crate) turn_id: Option<String>,
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

/// Only a headless turn (it holds an approval gate) may write to the owner; a foreground chat is
/// read live.
fn turn_site(
    headless: bool,
    conversation_id: Option<String>,
) -> Option<flow_like_bot_tools::TurnSite> {
    if !headless {
        return Some(flow_like_bot_tools::TurnSite::OwnerLive);
    }
    conversation_id.map(flow_like_bot_tools::TurnSite::Conversation)
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
    .with_skills(
        super::bot_tools::skills::skills_for(
            app,
            &runtime.context.bot_id,
            host.clone(),
            package_runner(app, &runtime, &host).await,
        )
        .await,
    )
    .with_mail(
        super::bot_tools::mail::mail_for(app, &runtime.context.bot_id, host.clone()).await,
    )
    .with_people(
        super::bot_tools::people::people_for(app, &runtime.context.bot_id, host.clone()).await,
    )
    .with_images(Arc::new(super::bot_tools::images::DesktopImages::new(
        runtime.clone(),
        context.and_then(|context| context.conversation_id.as_deref()),
    )))
    .with_connectors(Arc::new(
        super::bot_tools::connectors::DesktopConnectors::new(
            app,
            host.session.auth.clone(),
            runtime.context.workspace_app_id.as_deref().unwrap_or_default(),
            &runtime.context.bot_id,
            host.clone(),
            Arc::new(HostSecrets(host.clone())),
        )
        .await,
    ))
    .with_shell(shell_backend(app, &runtime.context.bot_id).await)
    .with_notifier(crate::bot_host::notifier_for(
        app,
        &runtime.context.bot_id,
        &runtime.context.name,
        turn_site(
            gate.is_some(),
            context.and_then(|context| context.conversation_id.clone()),
        ),
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

/// What a package skill's tool runs with: the bot's workspace store and model access.
async fn package_runner(
    app: &AppHandle,
    runtime: &Arc<BotToolRuntime>,
    host: &Arc<LiveHost>,
) -> super::bot_tools::skills::PackageRunner {
    let workspace = runtime.context.workspace_app_id.clone();
    let workspace_store = match &workspace {
        Some(workspace) => host.store(workspace).await.ok(),
        None => None,
    };
    super::bot_tools::skills::PackageRunner {
        app: app.clone(),
        state: runtime.state.clone(),
        token: runtime.token.clone(),
        workspace,
        workspace_store,
        turn_id: runtime.run_id.clone().unwrap_or_default(),
        user_id: format!("bot:{}", runtime.context.bot_id),
        usage: host.usage_context(),
    }
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
    async fn secrets(&self) -> Result<Vec<SecretInfo>>;

    /// A file members put into a conversation, read for the bot of this run.
    async fn conversation_file(
        &self,
        _conversation_id: &str,
        _path: &str,
        _max_bytes: u64,
    ) -> Result<Option<FileData>> {
        Ok(None)
    }
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
    skills: Option<Arc<dyn flow_like_bot_tools::SkillHost>>,
    notifier: Option<Arc<dyn flow_like_bot_tools::OwnerNotifier>>,
    mail: Option<Arc<dyn flow_like_bot_tools::email::MailBackend>>,
    people: Option<Arc<dyn flow_like_bot_tools::PeopleBackend>>,
    images: Option<Arc<super::bot_tools::images::DesktopImages>>,
    connectors: Option<Arc<dyn flow_like_bot_tools::services::ConnectorSource>>,
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
            skills: None,
            notifier: None,
            mail: None,
            people: None,
            images: None,
            connectors: None,
        }
    }

    pub(crate) fn with_connectors(
        mut self,
        connectors: Arc<dyn flow_like_bot_tools::services::ConnectorSource>,
    ) -> Self {
        self.connectors = Some(connectors);
        self
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

    pub(crate) fn with_people(
        mut self,
        people: Option<Arc<dyn flow_like_bot_tools::PeopleBackend>>,
    ) -> Self {
        self.people = people;
        self
    }

    pub(crate) fn with_images(
        mut self,
        images: Arc<super::bot_tools::images::DesktopImages>,
    ) -> Self {
        self.images = Some(images);
        self
    }

    pub(crate) fn with_machine(mut self, machine: LocalMachine) -> Self {
        self.machine = machine;
        self
    }

    pub(crate) fn with_skills(mut self, skills: Arc<dyn flow_like_bot_tools::SkillHost>) -> Self {
        self.skills = Some(skills);
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

fn conversation_of_app(app_id: &str) -> Option<&str> {
    app_id.strip_prefix(flow_like_types::bot_message::CONVERSATION_FILE_APP_PREFIX)
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
        if let Some(conversation_id) = conversation_of_app(app_id) {
            return self
                .host
                .conversation_file(conversation_id, path, max_bytes)
                .await;
        }
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
        if let Some(conversation_id) = conversation_of_app(app_id) {
            return Ok(self
                .host
                .conversation_file(conversation_id, path, 0)
                .await?
                .map(|file| FileMeta {
                    bytes: file.size,
                    mime: file.mime,
                }));
        }
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

    fn handle_granted(&self, handle: &WorkspaceHandle) -> bool {
        handle.is_conversation_file()
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

    fn connectors(&self) -> Option<Arc<dyn flow_like_bot_tools::services::ConnectorSource>> {
        self.connectors.clone()
    }

    fn skills(&self) -> Option<Arc<dyn flow_like_bot_tools::SkillHost>> {
        self.skills.clone()
    }

    fn shell(&self) -> Option<Arc<dyn ShellBackend>> {
        self.shell.clone()
    }

    fn notifier(&self) -> Option<Arc<dyn flow_like_bot_tools::OwnerNotifier>> {
        self.notifier.clone()
    }

    fn people(&self) -> Option<Arc<dyn flow_like_bot_tools::PeopleBackend>> {
        self.people.clone()
    }

    async fn images(
        &self,
    ) -> std::result::Result<
        Arc<dyn flow_like_bot_tools::image::ImageBackend>,
        flow_like_bot_tools::image::ImageUnavailable,
    > {
        match &self.images {
            Some(images) => images.backend().await,
            None => Err(flow_like_bot_tools::image::ImageUnavailable::NotOnThisHost),
        }
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

    async fn secrets(&self) -> Result<Vec<SecretInfo>> {
        self.host.secrets().await
    }
}

/// The bot's secrets for a connector's sign-in.
struct HostSecrets(Arc<LiveHost>);

#[async_trait]
impl super::bot_tools::connectors::SecretReader for HostSecrets {
    async fn secret(&self, name: &str) -> Result<Option<String>> {
        ToolHost::secret(self.0.as_ref(), name).await
    }
}

/// This device: local apps through the object store and the device databases, account apps
/// through the credentials the hub hands this signed-in user.
struct LiveHost {
    session: NativePlatformSession,
    runtime: Arc<BotToolRuntime>,
    credentials: tokio::sync::Mutex<HashMap<String, Arc<SharedCredentials>>>,
    bot_token: tokio::sync::Mutex<Option<String>>,
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
            bot_token: Default::default(),
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

    async fn conversation_file(
        &self,
        conversation_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        let bot_id = self.runtime.context.bot_id.clone();
        if let Some(bots) = self.session.app_handle.try_state::<LocalBots>() {
            let (conversation, file, reader) = (
                conversation_id.to_string(),
                path.to_string(),
                bot_id.clone(),
            );
            let local = crate::functions::local_bots::on_store(&bots, move |store| {
                store.read_conversation_file(&conversation, &reader, &file)
            })
            .await;
            if let Ok(found) = local {
                return Ok(found.map(|(meta, bytes)| {
                    let size = bytes.len() as u64;
                    let end = size.min(max_bytes) as usize;
                    FileData {
                        bytes: Bytes::from(bytes[..end].to_vec()),
                        size,
                        mime: Some(meta.mime),
                        classification: None,
                    }
                }));
            }
        }
        let file_id = path.split_once('.').map_or(path, |(id, _)| id);
        let download = self
            .session
            .request(
                Method::GET,
                &format!(
                    "inbox/conversations/{}/files/{}?as={}",
                    segment(conversation_id),
                    segment(file_id),
                    segment(&bot_id)
                ),
                None,
            )
            .await
            .map_err(|error| anyhow!("Conversation file '{path}': {error}"))?;
        let url = download["url"]
            .as_str()
            .ok_or_else(|| anyhow!("The hub sent no download link for conversation file '{path}'"))?;
        let bytes = reqwest::get(url)
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        let size = bytes.len() as u64;
        let end = size.min(max_bytes) as usize;
        Ok(Some(FileData {
            bytes: bytes.slice(..end),
            size,
            mime: download["mime"].as_str().map(str::to_string),
            classification: None,
        }))
    }

    async fn secret(&self, name: &str) -> Result<Option<String>> {
        let Some(bots) = self.local_bots().await? else {
            return self.account_secret(name).await;
        };
        let bot_id = self.runtime.context.bot_id.clone();
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

    async fn secrets(&self) -> Result<Vec<SecretInfo>> {
        let bot_id = self.runtime.context.bot_id.clone();
        if let Some(bots) = self.local_bots().await? {
            let stored =
                crate::functions::local_bots::on_store(&bots, move |store| store.secrets(&bot_id))
                    .await
                    .map_err(|error| anyhow!(error.to_string()))?;
            return Ok(stored
                .into_iter()
                .map(|secret| SecretInfo {
                    name: secret.name,
                    description: secret.description,
                })
                .collect());
        }
        let listed = self
            .session
            .request(
                Method::GET,
                &format!("bots/{}/secrets", segment(&bot_id)),
                None,
            )
            .await
            .map_err(|error| anyhow!("Listing the secrets of bot '{bot_id}': {error}"))?;
        Ok(listed["secrets"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|secret| {
                Some(SecretInfo {
                    name: secret["name"].as_str()?.to_string(),
                    description: secret["description"].as_str().map(str::to_string),
                })
            })
            .collect())
    }
}

impl LiveHost {
    /// This device's bot store when the bot is a local one; `None` for an account bot.
    async fn local_bots(&self) -> Result<Option<tauri::State<'_, LocalBots>>> {
        let Some(bots) = self.session.app_handle.try_state::<LocalBots>() else {
            return Err(anyhow!("Bots on this device are unavailable"));
        };
        let lookup = self.runtime.context.bot_id.clone();
        let is_local = crate::functions::local_bots::on_store(&bots, move |store| {
            store.get_bot(&lookup).map(|bot| bot.is_some())
        })
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
        Ok(is_local.then_some(bots))
    }

    /// The bot's own token for the turn this run answers; minted once per tool call.
    async fn bot_token(&self, turn_id: &str) -> Result<String> {
        let mut cached = self.bot_token.lock().await;
        if let Some(token) = cached.as_ref() {
            return Ok(token.clone());
        }
        let bot_id = &self.runtime.context.bot_id;
        let minted = self
            .session
            .request(
                Method::POST,
                &format!("bots/{}/runtime-token", segment(bot_id)),
                Some(&serde_json::json!({ "turn_id": turn_id })),
            )
            .await
            .map_err(|error| {
                anyhow!("Minting bot '{bot_id}' a token for turn '{turn_id}': {error}")
            })?;
        let token = minted["token"]
            .as_str()
            .ok_or_else(|| anyhow!("The hub minted bot '{bot_id}' no token for turn '{turn_id}'"))?
            .to_string();
        *cached = Some(token.clone());
        Ok(token)
    }

    /// An account bot's secret, read with the bot's own token while its turn runs; never cached.
    async fn account_secret(&self, name: &str) -> Result<Option<String>> {
        let turn_id = self.runtime.turn_id.as_deref().ok_or_else(|| {
            anyhow!(
                "An account bot reads its secrets only inside one of its turns; the secret '{name}' cannot be read here"
            )
        })?;
        let token = self.bot_token(turn_id).await?;
        let read = self
            .session
            .request_as_bot(
                &token,
                Method::POST,
                &format!("bots/turns/{}/secrets/{}", segment(turn_id), segment(name)),
            )
            .await
            .map_err(|error| anyhow!("Reading the bot's secret '{name}': {error}"))?;
        Ok(read.and_then(|read| read["value"].as_str().map(str::to_string)))
    }
}

#[cfg(test)]
#[path = "native_bot_tools_tests.rs"]
mod tests;

#[cfg(test)]
mod turn_site_tests {
    use super::turn_site;
    use flow_like_bot_tools::TurnSite;

    #[test]
    fn a_foreground_chat_is_read_live_and_a_headless_turn_keeps_its_conversation() {
        assert_eq!(
            turn_site(false, Some("c1".into())),
            Some(TurnSite::OwnerLive)
        );
        assert_eq!(turn_site(false, None), Some(TurnSite::OwnerLive));
        assert_eq!(
            turn_site(true, Some("c1".into())),
            Some(TurnSite::Conversation("c1".into()))
        );
        assert_eq!(turn_site(true, None), None);
    }
}
