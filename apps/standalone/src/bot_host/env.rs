//! The tool environment of a device-hosted turn: the hub-backed [`HostEnv`] (files and tables of
//! the bot's workspace through presigned credentials, hosted models through the executor token)
//! plus the machine backends this device gives the bot.

use async_trait::async_trait;
use flow_like::credentials::SharedCredentials;
use flow_like_bot_tools::{
    AppTables, Attachment, BrowserBackend, ComputerBackend, Environment, FileData, FileEntry,
    FileMeta, KnowledgeEmbedder, Limits, ModelRole, Scope, SecretInfo, SessionCache, ToolEnv,
};
use flow_like_bot_tools_host::{AppAccess, HostEnv, SecretSource};
use flow_like_model_provider::llm::ModelLogic;
use flow_like_storage::{files::store::FlowLikeStore, lancedb::Connection};
use flow_like_types::{Bytes, Result, anyhow, bot_message::WorkspaceHandle};
use reqwest::Method;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

use super::hub::TurnCreds;

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
                "Conversation files of '{app_id}' are not reachable from a device-hosted bot yet"
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
    pub turn_id: String,
}

#[async_trait]
impl SecretSource for HubSecrets {
    async fn secret(&self, name: &str) -> Result<Option<String>> {
        let path = format!("bots/turns/{}/secrets/{name}", self.turn_id);
        match self.hub.write(&path, Vec::new(), Value::Null).await {
            Ok(value) => Ok(value
                .get("value")
                .and_then(Value::as_str)
                .map(str::to_owned)),
            Err(failure) if failure.status == Some(404) => Ok(None),
            Err(failure) => Err(anyhow!("Reading secret '{name}' failed: {failure}")),
        }
    }

    async fn list(&self) -> Result<Vec<SecretInfo>> {
        Ok(Vec::new())
    }
}

pub struct DeviceToolEnv {
    inner: HostEnv,
    machine: MachineBackends,
}

impl DeviceToolEnv {
    pub fn new(inner: HostEnv, machine: MachineBackends) -> Self {
        Self { inner, machine }
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

    fn user_context(&self) -> Option<Arc<dyn flow_like_bot_tools::user_context::UserContextBackend>> {
        self.inner.user_context()
    }

    fn people(&self) -> Option<Arc<dyn flow_like_bot_tools::PeopleBackend>> {
        self.inner.people()
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

    async fn read_file(
        &self,
        app_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        self.inner.read_file(app_id, path, max_bytes).await
    }

    async fn stat_file(&self, app_id: &str, path: &str) -> Result<Option<FileMeta>> {
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
