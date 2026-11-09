use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use flow_like_bot_tools::{
    AppTables, Attachment, Environment, FileData, FileEntry, FileMeta, KnowledgeEmbedder, Limits,
    ModelRole, Scope, SecretInfo, SessionCache, ToolEnv,
};
use flow_like_model_provider::llm::ModelLogic;
use flow_like_storage::Path;
use flow_like_storage::contracts::column_metadata::ColumnReader;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::object_store::{ObjectStoreExt, PutPayload};
use flow_like_storage::{display_object_path, join_object_path};
use flow_like_types::bot_message::{CONVERSATION_FILE_APP_PREFIX, WorkspaceHandle};
use flow_like_types::{Bytes, Result, anyhow};
use futures::TryStreamExt;

use crate::models::ModelSource;
use crate::wire::{JobApp, JobScope};

/// How a host reaches one app's content store and Lance database. Credentials are the host's
/// business; the env decides what the bot may ask for.
#[async_trait]
pub trait AppAccess: Send + Sync {
    async fn store(&self, app_id: &str) -> Result<FlowLikeStore>;
    async fn connection(&self, app_id: &str) -> Result<Connection>;
}

/// The bot's secret store as a host reaches it: values for the tools that fill them in, names
/// and descriptions for the model.
#[async_trait]
pub trait SecretSource: Send + Sync {
    async fn secret(&self, name: &str) -> Result<Option<String>>;
    async fn list(&self) -> Result<Vec<SecretInfo>>;
}

/// Where a host gets the bot's image model for `image_generate`; one per turn, so the turn's
/// image allowance holds across calls.
#[async_trait]
pub trait ImageSource: Send + Sync {
    async fn images(
        &self,
    ) -> std::result::Result<
        Arc<dyn flow_like_bot_tools::image::ImageBackend>,
        flow_like_bot_tools::image::ImageUnavailable,
    >;
}

#[derive(Debug, Clone)]
struct Rights {
    files: bool,
    tables: bool,
    reader: ColumnReader,
}

impl From<&JobApp> for Rights {
    fn from(app: &JobApp) -> Self {
        Self {
            files: app.files,
            tables: app.tables,
            reader: match &app.column_attributes {
                None => ColumnReader::Unrestricted,
                Some(attributes) => ColumnReader::with_attributes(attributes),
            },
        }
    }
}

/// The server's [`ToolEnv`]: files and tables through [`AppAccess`], models through
/// [`ModelSource`]. Reads follow the per-app rights of the bot's owner; writes reach the
/// workspace only.
pub struct HostEnv {
    scope: Scope,
    limits: Limits,
    workspace: String,
    rights: BTreeMap<String, Rights>,
    access: Arc<dyn AppAccess>,
    models: Arc<dyn ModelSource>,
    sessions: Option<Arc<dyn SessionCache>>,
    allow_expensive_documents: bool,
    object_writes: bool,
    notifier: Option<Arc<dyn flow_like_bot_tools::OwnerNotifier>>,
    people: Option<Arc<dyn flow_like_bot_tools::PeopleBackend>>,
    user_context: Option<Arc<dyn flow_like_bot_tools::user_context::UserContextBackend>>,
    configuration: Option<Arc<dyn flow_like_bot_tools::configuration::ConfigurationBackend>>,
    crew: Option<Arc<dyn flow_like_bot_tools::crew::CrewBackend>>,
    scheduler: Option<Arc<dyn flow_like_bot_tools::Scheduler>>,
    skills: Option<Arc<dyn flow_like_bot_tools::SkillHost>>,
    mail: Option<Arc<dyn flow_like_bot_tools::email::MailBackend>>,
    secrets: Option<Arc<dyn SecretSource>>,
    images: Option<Arc<dyn ImageSource>>,
    connectors: Option<Arc<dyn flow_like_bot_tools::services::ConnectorSource>>,
}

fn conversation_of_app(app_id: &str) -> Option<&str> {
    app_id.strip_prefix(CONVERSATION_FILE_APP_PREFIX)
}

fn is_conversation_app(app_id: &str) -> bool {
    conversation_of_app(app_id).is_some()
}

impl HostEnv {
    pub fn new(
        workspace: &str,
        job_scope: &JobScope,
        access: Arc<dyn AppAccess>,
        models: Arc<dyn ModelSource>,
    ) -> Self {
        Self {
            scope: job_scope.to_scope(workspace),
            limits: Limits::default(),
            workspace: workspace.to_string(),
            rights: job_scope
                .apps
                .iter()
                .map(|app| (app.app_id.clone(), Rights::from(app)))
                .collect(),
            access,
            models,
            sessions: None,
            allow_expensive_documents: false,
            object_writes: true,
            notifier: None,
            people: None,
            user_context: None,
            configuration: None,
            crew: None,
            scheduler: None,
            skills: None,
            mail: None,
            secrets: None,
            images: None,
            connectors: None,
        }
    }

    pub fn with_connectors(
        mut self,
        connectors: Arc<dyn flow_like_bot_tools::services::ConnectorSource>,
    ) -> Self {
        self.connectors = Some(connectors);
        self
    }

    /// Without a source this host generates no images.
    pub fn with_images(mut self, images: Arc<dyn ImageSource>) -> Self {
        self.images = Some(images);
        self
    }

    /// Without a source the bot has no secrets here.
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretSource>) -> Self {
        self.secrets = Some(secrets);
        self
    }

    pub fn with_sessions(mut self, sessions: Arc<dyn SessionCache>) -> Self {
        self.sessions = Some(sessions);
        self
    }

    pub fn with_expensive_documents(mut self, allowed: bool) -> Self {
        self.allow_expensive_documents = allowed;
        self
    }

    /// Whether `write_workspace` stores objects. A host that keeps object writes out of its
    /// process turns it off; the tools that need it fail with a plain error.
    pub fn with_object_writes(mut self, allowed: bool) -> Self {
        self.object_writes = allowed;
        self
    }

    pub fn with_people(mut self, people: Arc<dyn flow_like_bot_tools::PeopleBackend>) -> Self {
        self.people = Some(people);
        self
    }

    pub fn with_user_context(
        mut self,
        user_context: Arc<dyn flow_like_bot_tools::user_context::UserContextBackend>,
    ) -> Self {
        self.user_context = Some(user_context);
        self
    }

    pub fn with_configuration(
        mut self,
        configuration: Arc<dyn flow_like_bot_tools::configuration::ConfigurationBackend>,
    ) -> Self {
        self.configuration = Some(configuration);
        self
    }

    pub fn with_crew(mut self, crew: Arc<dyn flow_like_bot_tools::crew::CrewBackend>) -> Self {
        self.crew = Some(crew);
        self
    }

    pub fn with_notifier(mut self, notifier: Arc<dyn flow_like_bot_tools::OwnerNotifier>) -> Self {
        self.notifier = Some(notifier);
        self
    }

    pub fn with_scheduler(mut self, scheduler: Arc<dyn flow_like_bot_tools::Scheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    pub fn with_skills(mut self, skills: Arc<dyn flow_like_bot_tools::SkillHost>) -> Self {
        self.skills = Some(skills);
        self
    }

    pub fn with_mail(mut self, mail: Arc<dyn flow_like_bot_tools::email::MailBackend>) -> Self {
        self.mail = Some(mail);
        self
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    fn is_workspace(&self, app_id: &str) -> bool {
        self.workspace == app_id
    }

    fn files_allowed(&self, app_id: &str) -> Result<()> {
        if self.is_workspace(app_id)
            || is_conversation_app(app_id)
            || self.rights.get(app_id).is_some_and(|rights| rights.files)
        {
            return Ok(());
        }
        Err(anyhow!(
            "The owner may not read the files of app '{app_id}'"
        ))
    }

    fn tables_rights(&self, app_id: &str) -> Result<Rights> {
        if self.is_workspace(app_id) {
            return Ok(Rights {
                files: true,
                tables: true,
                reader: ColumnReader::Unrestricted,
            });
        }
        self.rights
            .get(app_id)
            .filter(|rights| rights.tables)
            .cloned()
            .ok_or_else(|| anyhow!("The owner may not read the tables of app '{app_id}'"))
    }

    async fn base(&self, app_id: &str) -> Result<(FlowLikeStore, Path)> {
        self.files_allowed(app_id)?;
        let store = self.access.store(app_id).await?;
        let base = match conversation_of_app(app_id) {
            Some(conversation_id) => Path::from(format!("conversations/{conversation_id}")),
            None => store.construct_upload(app_id, "").await?,
        };
        Ok((store, base))
    }
}

#[async_trait]
impl ToolEnv for HostEnv {
    fn environment(&self) -> Environment {
        Environment::Server
    }

    fn scope(&self) -> &Scope {
        &self.scope
    }

    fn limits(&self) -> &Limits {
        &self.limits
    }

    fn sessions(&self) -> Option<Arc<dyn SessionCache>> {
        self.sessions.clone()
    }

    fn mail(&self) -> Option<Arc<dyn flow_like_bot_tools::email::MailBackend>> {
        self.mail.clone()
    }

    fn connectors(&self) -> Option<Arc<dyn flow_like_bot_tools::services::ConnectorSource>> {
        self.connectors.clone()
    }

    fn scheduler(&self) -> Option<Arc<dyn flow_like_bot_tools::Scheduler>> {
        self.scheduler.clone()
    }

    fn skills(&self) -> Option<Arc<dyn flow_like_bot_tools::SkillHost>> {
        self.skills.clone()
    }

    fn notifier(&self) -> Option<Arc<dyn flow_like_bot_tools::OwnerNotifier>> {
        self.notifier.clone()
    }

    fn people(&self) -> Option<Arc<dyn flow_like_bot_tools::PeopleBackend>> {
        self.people.clone()
    }

    fn user_context(
        &self,
    ) -> Option<Arc<dyn flow_like_bot_tools::user_context::UserContextBackend>> {
        self.user_context.clone()
    }

    fn configuration(
        &self,
    ) -> Option<Arc<dyn flow_like_bot_tools::configuration::ConfigurationBackend>> {
        self.configuration.clone()
    }

    fn crew(&self) -> Option<Arc<dyn flow_like_bot_tools::crew::CrewBackend>> {
        self.crew.clone()
    }

    fn allow_expensive_documents(&self) -> bool {
        self.allow_expensive_documents
    }

    async fn read_file(
        &self,
        app_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        let (store, base) = self.base(app_id).await?;
        let location = join_object_path(&base, path);
        let generic = store.as_generic();
        let meta = match generic.head(&location).await {
            Ok(meta) => meta,
            Err(flow_like_storage::object_store::Error::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let wanted = meta.size.min(max_bytes);
        let bytes = if wanted == 0 {
            Bytes::new()
        } else {
            generic.get_range(&location, 0..wanted).await?
        };
        Ok(Some(FileData {
            bytes,
            size: meta.size,
            mime: Some(flow_like_bot_tools::files::guess_mime(path)),
            classification: None,
        }))
    }

    async fn stat_file(&self, app_id: &str, path: &str) -> Result<Option<FileMeta>> {
        let (store, base) = self.base(app_id).await?;
        let location = join_object_path(&base, path);
        match store.as_generic().head(&location).await {
            Ok(meta) => Ok(Some(FileMeta {
                bytes: meta.size,
                mime: Some(flow_like_bot_tools::files::guess_mime(path)),
            })),
            Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn list_files(&self, app_id: &str, prefix: &str, limit: usize) -> Result<Vec<FileEntry>> {
        let (store, base) = self.base(app_id).await?;
        let start = join_object_path(&base, prefix);
        let mut stream = store.as_generic().list(Some(&start));
        let mut entries = Vec::new();
        while entries.len() < limit {
            let Some(meta) = stream.try_next().await? else {
                break;
            };
            let Some(relative) = relative_to(&base, &meta.location) else {
                continue;
            };
            entries.push(FileEntry {
                mime: Some(flow_like_bot_tools::files::guess_mime(&relative)),
                path: relative,
                bytes: meta.size,
                modified_ms: Some(meta.last_modified.timestamp_millis()),
            });
        }
        Ok(entries)
    }

    async fn write_workspace(&self, path: &str, data: Bytes, _mime: &str) -> Result<()> {
        if !self.object_writes {
            return Err(anyhow!(
                "This host does not store files; the call belongs on the executor"
            ));
        }
        let (store, base) = self.base(&self.workspace).await?;
        let location = join_object_path(&base, path);
        store
            .as_generic()
            .put(&location, PutPayload::from_bytes(data))
            .await?;
        Ok(())
    }

    async fn message_attachment(&self, _seq: u64, _part: &str) -> Result<Option<Attachment>> {
        Ok(None)
    }

    fn handle_granted(&self, handle: &WorkspaceHandle) -> bool {
        handle.is_conversation_file()
    }

    async fn app_tables(&self, app_id: &str) -> Result<AppTables> {
        let rights = self.tables_rights(app_id)?;
        Ok(AppTables {
            connection: self.access.connection(app_id).await?,
            writable: self.is_workspace(app_id),
            reader: rights.reader,
        })
    }

    async fn model(&self, role: ModelRole) -> Result<Arc<dyn ModelLogic>> {
        self.models.model(role).await
    }

    async fn embedder(&self) -> Result<Option<Arc<dyn KnowledgeEmbedder>>> {
        self.models.embedder().await
    }

    async fn images(
        &self,
    ) -> std::result::Result<
        Arc<dyn flow_like_bot_tools::image::ImageBackend>,
        flow_like_bot_tools::image::ImageUnavailable,
    > {
        match &self.images {
            Some(source) => source.images().await,
            None => Err(flow_like_bot_tools::image::ImageUnavailable::NotOnThisHost),
        }
    }

    async fn secret(&self, name: &str) -> Result<Option<String>> {
        match &self.secrets {
            Some(source) => source.secret(name).await,
            None => Ok(None),
        }
    }

    async fn secrets(&self) -> Result<Vec<SecretInfo>> {
        match &self.secrets {
            Some(source) => source.list().await,
            None => Ok(Vec::new()),
        }
    }
}

fn relative_to(base: &Path, location: &Path) -> Option<String> {
    let base_parts: Vec<_> = base.parts().collect();
    let parts: Vec<_> = location.parts().collect();
    if parts.len() <= base_parts.len() || parts[..base_parts.len()] != base_parts[..] {
        return None;
    }
    let relative: Path = parts[base_parts.len()..].iter().cloned().collect();
    Some(display_object_path(&relative))
}
