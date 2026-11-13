//! The tool environment of a device-hosted turn: the hub-backed [`HostEnv`] (files and tables of
//! the bot's workspace through presigned credentials, hosted models through the executor token)
//! plus the machine backends this device gives the bot.

use async_trait::async_trait;
use flow_like::credentials::SharedCredentials;
use flow_like::{
    bit::Bit, flow::execution::ExecutionEnvironment, models::llm::ModelUsageContext,
    profile::Profile, state::FlowLikeState,
};
use flow_like_bot_tools::image::{
    CatalogImages, ImageBackend, ImageBudget, ImageUnavailable, hosted_access,
};
use flow_like_bot_tools::{
    AppTables, Attachment, BrowserBackend, ComputerBackend, Environment, FileData, FileEntry,
    FileMeta, KnowledgeEmbedder, Limits, ModelRole, Scope, SecretInfo, SessionCache, ShellBackend,
    ToolEnv,
};
use flow_like_bot_tools_host::{AppAccess, HostEnv, ImageSource, SecretSource};
use flow_like_model_provider::llm::ModelLogic;
use flow_like_storage::{files::store::FlowLikeStore, lancedb::Connection};
use flow_like_types::{
    Bytes, Result, anyhow,
    bot_message::{CONVERSATION_FILE_APP_PREFIX, WorkspaceHandle},
};
use reqwest::Method;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

use super::{
    backends::{HubImageSignIn, image_bit_for_call},
    hub::TurnCreds,
};

/// The browser and screen a device gives one bot.
#[derive(Clone, Default)]
pub struct MachineBackends {
    pub browser: Option<Arc<dyn BrowserBackend>>,
    pub computer: Option<Arc<dyn ComputerBackend>>,
}

/// Where a turn gets its machine; the playground feature provides the real one.
pub trait Machines: Send + Sync + 'static {
    fn machine(
        &self,
        bot_id: &str,
        conversation_id: Option<&str>,
        browser_use: bool,
        computer_use: bool,
    ) -> MachineBackends;
}

/// The bot's image model: a hosted one through the hub's proxy with the turn's executor token,
/// an own-key one with the sign-in the hub hands out per call. One per turn, so the turn's
/// image allowance holds across tool calls.
pub struct HubImages {
    pub state: Arc<FlowLikeState>,
    pub profile: Arc<Profile>,
    pub bit: Bit,
    pub creds: Arc<TurnCreds>,
    pub usage: ModelUsageContext,
    pub budget: Arc<ImageBudget>,
    pub sign_in: HubImageSignIn,
}

#[async_trait]
impl ImageSource for HubImages {
    async fn images(&self) -> std::result::Result<Arc<dyn ImageBackend>, ImageUnavailable> {
        let bit = image_bit_for_call(&self.bit, &self.sign_in).await?;
        let hosted = hosted_access(&self.state, self.creds.executor_token(), Some(&self.usage));
        let images = CatalogImages::resolve(
            bit,
            &self.profile,
            &self.state,
            ExecutionEnvironment::Desktop,
            Some(hosted),
            self.budget.clone(),
        )
        .await?;
        Ok(Arc::new(images))
    }
}

/// Presigned credentials per app, fetched with the turn's bot token.
pub struct PresignedAccess {
    http: reqwest::Client,
    base_url: String,
    creds: Arc<TurnCreds>,
    cache: Mutex<HashMap<String, Arc<SharedCredentials>>>,
}

impl PresignedAccess {
    pub fn new(http: reqwest::Client, base_url: String, creds: Arc<TurnCreds>) -> Self {
        Self {
            http,
            base_url,
            creds,
            cache: Mutex::default(),
        }
    }

    async fn credentials(&self, app_id: &str) -> Result<Arc<SharedCredentials>> {
        let mut cache = self.cache.lock().await;
        if let Some(found) = cache.get(app_id) {
            return Ok(found.clone());
        }
        let url = format!(
            "{}/apps/{app_id}/invoke/presign",
            self.base_url.trim_end_matches('/')
        );
        let response = self
            .http
            .request(Method::GET, &url)
            .bearer_auth(self.creds.bot_token())
            .send()
            .await
            .map_err(|error| {
                anyhow!("Credentials for app '{app_id}' could not be fetched: {error}")
            })?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(anyhow!(
                "The hub refused credentials for app '{app_id}' with HTTP {status}"
            ));
        }
        let value: Value = serde_json::from_str(&body)?;
        let credentials: Arc<SharedCredentials> =
            Arc::new(serde_json::from_value(value).map_err(|error| {
                anyhow!("The hub sent unusable credentials for '{app_id}': {error}")
            })?);
        cache.insert(app_id.to_owned(), credentials.clone());
        Ok(credentials)
    }
}

#[async_trait]
impl AppAccess for PresignedAccess {
    async fn store(&self, app_id: &str) -> Result<FlowLikeStore> {
        if app_id.starts_with(flow_like_types::bot_message::CONVERSATION_FILE_APP_PREFIX) {
            return Err(anyhow!(
                "Conversation files of '{app_id}' are read through the hub, not through presigned storage credentials"
            ));
        }
        self.credentials(app_id).await?.to_store(false).await
    }

    async fn connection(&self, app_id: &str) -> Result<Connection> {
        let builder = self.credentials(app_id).await?.to_db(app_id).await?;
        Ok(builder.execute().await?)
    }
}

/// The bot's secrets through the hub: a bot token reads a value while its turn runs, but only
/// people list names, so the model sees no secret names on a device.
pub struct HubSecrets {
    pub hub: Arc<dyn flow_like_bot_host::hub::TurnHub>,
    pub bot_id: String,
    pub turn_id: String,
    /// Opens a secret sealed to the bot's own key; without it a sealed secret is refused.
    pub e2e: Option<Arc<flow_like_bot_host::e2e::E2eHost>>,
}

#[async_trait]
impl SecretSource for HubSecrets {
    async fn secret(&self, name: &str) -> Result<Option<String>> {
        flow_like_bot_host::secrets::read_turn_secret(
            self.hub.as_ref(),
            self.e2e.as_deref(),
            &self.bot_id,
            &self.turn_id,
            name,
        )
        .await
    }

    async fn list(&self) -> Result<Vec<SecretInfo>> {
        Ok(Vec::new())
    }
}

const CONVERSATION_FILE_TIMEOUT_SECS: u64 = 120;

fn conversation_file_route(conversation_id: &str, path: &str) -> Result<String> {
    let file_id = path.split_once('.').map_or(path, |(id, _)| id);
    let plain = |id: &str| {
        !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    if !plain(file_id) || !plain(conversation_id) {
        return Err(anyhow!(
            "Conversation file '{path}' of conversation '{conversation_id}' has an unusable id"
        ));
    }
    Ok(format!(
        "inbox/conversations/{conversation_id}/files/{file_id}"
    ))
}

/// Files people attach to a conversation: the hub signs a short-lived download link for the
/// bot's runtime token, and only for the conversation that token was minted for.
pub struct ConversationFiles {
    hub: Arc<dyn flow_like_bot_host::hub::TurnHub>,
    http: reqwest::Client,
    conversation: String,
}

fn ensure_turn_conversation(turn_conversation: &str, requested: &str) -> Result<()> {
    if turn_conversation == requested {
        return Ok(());
    }
    Err(anyhow!(
        "Conversation '{requested}' is not this turn's conversation; a bot reads the files of the conversation it answers in"
    ))
}

struct ConversationDownload {
    url: String,
    bytes: u64,
    mime: Option<String>,
}

impl ConversationFiles {
    pub fn new(hub: Arc<dyn flow_like_bot_host::hub::TurnHub>, conversation: &str) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(
                CONVERSATION_FILE_TIMEOUT_SECS,
            ))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self {
            hub,
            http,
            conversation: conversation.to_string(),
        }
    }

    async fn download(
        &self,
        conversation_id: &str,
        path: &str,
    ) -> Result<Option<ConversationDownload>> {
        ensure_turn_conversation(&self.conversation, conversation_id)?;
        let route = conversation_file_route(conversation_id, path)?;
        let answer = match self.hub.read(&route, Vec::new()).await {
            Ok(answer) => answer,
            Err(failure) if failure.status == Some(404) => return Ok(None),
            Err(failure) => {
                return Err(anyhow!(
                    "Conversation file '{path}' could not be read: {failure}"
                ));
            }
        };
        let url = answer["url"].as_str().ok_or_else(|| {
            anyhow!("The hub sent no download link for conversation file '{path}'")
        })?;
        Ok(Some(ConversationDownload {
            url: url.to_owned(),
            bytes: answer["bytes"].as_u64().unwrap_or(0),
            mime: answer["mime"].as_str().map(str::to_owned),
        }))
    }

    async fn read(
        &self,
        conversation_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        let Some(download) = self.download(conversation_id, path).await? else {
            return Ok(None);
        };
        let limit = usize::try_from(max_bytes).unwrap_or(usize::MAX);
        let mut response = self
            .http
            .get(&download.url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| anyhow!("Conversation file '{path}' could not be fetched: {error}"))?;
        let mut bytes = Vec::new();
        while bytes.len() < limit {
            let Some(chunk) = response.chunk().await? else {
                break;
            };
            let room = limit - bytes.len();
            bytes.extend_from_slice(&chunk[..chunk.len().min(room)]);
        }
        Ok(Some(FileData {
            size: download.bytes.max(bytes.len() as u64),
            bytes: Bytes::from(bytes),
            mime: download.mime,
            classification: None,
        }))
    }

    async fn stat(&self, conversation_id: &str, path: &str) -> Result<Option<FileMeta>> {
        Ok(self
            .download(conversation_id, path)
            .await?
            .map(|download| FileMeta {
                bytes: download.bytes,
                mime: download.mime,
            }))
    }
}

pub struct DeviceToolEnv {
    inner: HostEnv,
    machine: MachineBackends,
    shell: Option<Arc<dyn ShellBackend>>,
    conversation_files: Option<ConversationFiles>,
}

impl DeviceToolEnv {
    pub fn new(inner: HostEnv, machine: MachineBackends) -> Self {
        Self {
            inner,
            machine,
            shell: None,
            conversation_files: None,
        }
    }

    pub fn with_shell(mut self, shell: Arc<dyn ShellBackend>) -> Self {
        self.shell = Some(shell);
        self
    }

    pub fn with_conversation_files(mut self, files: ConversationFiles) -> Self {
        self.conversation_files = Some(files);
        self
    }

    fn conversation_file<'a>(&self, app_id: &'a str) -> Option<(&ConversationFiles, &'a str)> {
        let conversation_id = app_id.strip_prefix(CONVERSATION_FILE_APP_PREFIX)?;
        self.conversation_files
            .as_ref()
            .map(|files| (files, conversation_id))
    }
}

#[async_trait]
impl ToolEnv for DeviceToolEnv {
    fn environment(&self) -> Environment {
        self.inner.environment()
    }

    fn scope(&self) -> &Scope {
        self.inner.scope()
    }

    fn limits(&self) -> &Limits {
        self.inner.limits()
    }

    fn sessions(&self) -> Option<Arc<dyn SessionCache>> {
        self.inner.sessions()
    }

    fn mail(&self) -> Option<Arc<dyn flow_like_bot_tools::email::MailBackend>> {
        self.inner.mail()
    }

    fn connectors(&self) -> Option<Arc<dyn flow_like_bot_tools::services::ConnectorSource>> {
        self.inner.connectors()
    }

    fn scheduler(&self) -> Option<Arc<dyn flow_like_bot_tools::Scheduler>> {
        self.inner.scheduler()
    }

    fn skills(&self) -> Option<Arc<dyn flow_like_bot_tools::SkillHost>> {
        self.inner.skills()
    }

    fn notifier(&self) -> Option<Arc<dyn flow_like_bot_tools::OwnerNotifier>> {
        self.inner.notifier()
    }

    fn user_context(
        &self,
    ) -> Option<Arc<dyn flow_like_bot_tools::user_context::UserContextBackend>> {
        self.inner.user_context()
    }

    fn people(&self) -> Option<Arc<dyn flow_like_bot_tools::PeopleBackend>> {
        self.inner.people()
    }

    fn rooms(&self) -> Option<Arc<dyn flow_like_bot_tools::RoomInvites>> {
        self.inner.rooms()
    }

    fn allow_expensive_documents(&self) -> bool {
        self.inner.allow_expensive_documents()
    }

    fn browser(&self) -> Option<Arc<dyn BrowserBackend>> {
        self.machine.browser.clone()
    }

    fn computer(&self) -> Option<Arc<dyn ComputerBackend>> {
        self.machine.computer.clone()
    }

    fn shell(&self) -> Option<Arc<dyn ShellBackend>> {
        self.shell.clone()
    }

    async fn read_file(
        &self,
        app_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        if let Some((files, conversation_id)) = self.conversation_file(app_id) {
            return files.read(conversation_id, path, max_bytes).await;
        }
        self.inner.read_file(app_id, path, max_bytes).await
    }

    async fn stat_file(&self, app_id: &str, path: &str) -> Result<Option<FileMeta>> {
        if let Some((files, conversation_id)) = self.conversation_file(app_id) {
            return files.stat(conversation_id, path).await;
        }
        self.inner.stat_file(app_id, path).await
    }

    async fn list_files(&self, app_id: &str, prefix: &str, limit: usize) -> Result<Vec<FileEntry>> {
        self.inner.list_files(app_id, prefix, limit).await
    }

    async fn write_workspace(&self, path: &str, data: Bytes, mime: &str) -> Result<()> {
        self.inner.write_workspace(path, data, mime).await
    }

    async fn message_attachment(&self, seq: u64, part: &str) -> Result<Option<Attachment>> {
        self.inner.message_attachment(seq, part).await
    }

    fn handle_granted(&self, handle: &WorkspaceHandle) -> bool {
        self.inner.handle_granted(handle)
    }

    async fn app_tables(&self, app_id: &str) -> Result<AppTables> {
        self.inner.app_tables(app_id).await
    }

    async fn model(&self, role: ModelRole) -> Result<Arc<dyn ModelLogic>> {
        self.inner.model(role).await
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        self.inner.embedder().await
    }

    async fn images(
        &self,
    ) -> std::result::Result<
        Arc<dyn flow_like_bot_tools::image::ImageBackend>,
        flow_like_bot_tools::image::ImageUnavailable,
    > {
        self.inner.images().await
    }

    async fn secret(&self, name: &str) -> Result<Option<String>> {
        self.inner.secret(name).await
    }

    async fn secrets(&self) -> Result<Vec<SecretInfo>> {
        self.inner.secrets().await
    }
}

#[cfg(all(
    feature = "playground",
    any(target_os = "macos", windows, target_os = "linux")
))]
impl Machines for crate::playground::MachineHost {
    fn machine(
        &self,
        bot_id: &str,
        conversation_id: Option<&str>,
        browser_use: bool,
        computer_use: bool,
    ) -> MachineBackends {
        let machine = crate::playground::MachineHost::machine(
            self,
            bot_id,
            conversation_id,
            browser_use,
            computer_use,
        );
        MachineBackends {
            browser: machine.browser,
            computer: machine.computer,
        }
    }
}

#[cfg(test)]
mod conversation_file_tests {
    use super::conversation_file_route;

    #[test]
    fn a_file_id_that_is_not_a_plain_id_is_refused_before_any_call() {
        assert_eq!(
            conversation_file_route("conv-1", "f_9.pdf").unwrap(),
            "inbox/conversations/conv-1/files/f_9"
        );
        for path in ["../secret.pdf", "a/b", "", ".pdf"] {
            let error = conversation_file_route("conv-1", path)
                .unwrap_err()
                .to_string();
            assert!(error.contains("unusable id"), "{path}: {error}");
        }
        assert!(conversation_file_route("a/b", "f_9").is_err());
    }
}

#[cfg(test)]
mod conversation_scope_tests {
    use super::ensure_turn_conversation;

    #[test]
    fn only_the_turns_conversation_is_readable() {
        assert!(ensure_turn_conversation("room-1", "room-1").is_ok());
        let refused = ensure_turn_conversation("room-1", "room-2").unwrap_err();
        assert!(refused.to_string().contains("room-2"));
    }
}
