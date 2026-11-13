//! The owner-facing services of a device-hosted turn (notify, tasks, people, mail): thin
//! clients of the routes the hub opens to the turn's bot token. The hub enforces rights, rate
//! limits and mutes; these only shape requests and read the receipts.

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like::{bit::Bit, models::llm::ModelUsageContext, state::FlowLikeState};
use flow_like_bot_host::hub::TurnHub;
use flow_like_bot_tools::image::{
    ImageUnavailable, is_local_image_bit, is_own_key_image_bit, with_provider_params,
};
use flow_like_bot_tools::services::{
    CallAudit, CallBudget, ConnectorEntry, ConnectorSource, Credential,
};
use flow_like_bot_tools::{
    Contact, Delivery, NoticeReceipt, OwnerNotice, OwnerNotifier, PeopleBackend, PersonMessage,
    Scheduler, SkillHost, SkillToolSpec,
    email::{MailBackend, MailFailure, MailFile, OutgoingMail, ReplyMail, SentMail, failure},
};
use flow_like_bot_tools_host::AppAccess;
use flow_like_types::{
    Result, anyhow,
    bot_skill::{BotSkill, CreateBotSkill, SkillSource, SkillStatus},
    bot_task::{BotTask, CreateBotTask, UpdateBotTask},
};
use flow_like_wasm::{HeadlessRun, PackagePermissions, WasmCapabilities, WasmNodeLogic};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};

/// The turn's bot-token hub and the bot it acts as.
#[derive(Clone)]
pub struct BotRoutes {
    pub hub: Arc<dyn TurnHub>,
    pub bot_id: String,
    pub conversation_id: Option<String>,
}

impl BotRoutes {
    fn path(&self, tail: &str) -> String {
        format!("bots/{}/{tail}", self.bot_id)
    }
}

pub struct HubNotifier(pub BotRoutes);

#[async_trait]
impl OwnerNotifier for HubNotifier {
    async fn sent_since(&self, _since_ms: i64) -> Result<usize> {
        Ok(0)
    }

    async fn deliver(&self, notice: OwnerNotice) -> Result<NoticeReceipt> {
        let body = json!({
            "text": notice.text,
            "title": notice.title,
            "conversation_id": self.0.conversation_id,
        });
        let answer = self
            .0
            .hub
            .write(&self.0.path("notify"), Vec::new(), body)
            .await
            .map_err(|failure| anyhow!("Messaging the owner failed: {failure}"))?;
        Ok(NoticeReceipt {
            id: answer["id"].as_str().unwrap_or_default().to_owned(),
            muted: answer["muted"].as_bool().unwrap_or(false),
            posted: answer["posted"].as_bool().unwrap_or(false),
            in_owner_chat: answer["in_owner_chat"].as_bool().unwrap_or(false),
        })
    }
}

pub struct HubScheduler(pub BotRoutes);

#[async_trait]
impl Scheduler for HubScheduler {
    async fn create(&self, request: CreateBotTask) -> Result<BotTask> {
        let body = json!({
            "kind": request.kind,
            "at": request.at,
            "cron": request.cron,
            "tz": request.tz,
            "instruction": request.instruction,
            "title": request.title,
            "monitoring": request.monitoring,
        });
        let created = self
            .0
            .hub
            .write(&self.0.path("tasks"), Vec::new(), body)
            .await
            .map_err(|failure| anyhow!("Creating the task failed: {failure}"))?;
        serde_json::from_value(created)
            .map_err(|error| anyhow!("The hub sent an unusable task: {error}"))
    }

    async fn list(&self) -> Result<Vec<BotTask>> {
        let listed = self
            .0
            .hub
            .read(&self.0.path("tasks"), Vec::new())
            .await
            .map_err(|failure| anyhow!("Listing the tasks failed: {failure}"))?;
        serde_json::from_value(
            listed
                .get("tasks")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        )
        .map_err(|error| anyhow!("The hub sent unusable tasks: {error}"))
    }

    async fn update(&self, task_id: &str, request: UpdateBotTask) -> Result<Option<BotTask>> {
        if !self.list().await?.iter().any(|task| task.id == task_id) {
            return Ok(None);
        }
        let body = serde_json::to_value(&request)
            .map_err(|error| anyhow!("The task change could not be sent: {error}"))?;
        let updated = self
            .0
            .hub
            .patch(&self.0.path(&format!("tasks/{task_id}")), Vec::new(), body)
            .await
            .map_err(|failure| anyhow!("Changing task '{task_id}' failed: {failure}"))?;
        serde_json::from_value(updated)
            .map(Some)
            .map_err(|error| anyhow!("The hub sent an unusable task: {error}"))
    }

    async fn cancel(&self, task_id: &str) -> Result<bool> {
        if !self.list().await?.iter().any(|task| task.id == task_id) {
            return Ok(false);
        }
        self.0
            .hub
            .delete(&self.0.path(&format!("tasks/{task_id}")), Vec::new())
            .await
            .map(|_| true)
            .map_err(|failure| anyhow!("Cancelling task '{task_id}' failed: {failure}"))
    }
}

pub struct HubSkills(pub BotRoutes);

fn skill_of(bot_id: &str, found: &Value) -> BotSkill {
    let text = |key: &str| found[key].as_str().unwrap_or_default().to_string();
    BotSkill {
        id: text("id"),
        bot_id: bot_id.to_string(),
        name: text("name"),
        description: text("description"),
        body: text("body"),
        source: SkillSource::parse(found["source"].as_str().unwrap_or_default())
            .unwrap_or(SkillSource::Import),
        status: SkillStatus::parse(found["status"].as_str().unwrap_or_default())
            .unwrap_or(SkillStatus::Disabled),
        tools: serde_json::from_value(found["tools"].clone()).unwrap_or_default(),
        updated_at: text("updated_at"),
    }
}

#[async_trait]
impl SkillHost for HubSkills {
    async fn get(&self, name: &str) -> Result<Option<BotSkill>> {
        let listed = self
            .0
            .hub
            .read(&self.0.path("skills"), Vec::new())
            .await
            .map_err(|failure| anyhow!("Listing the skills failed: {failure}"))?;
        let skills = listed
            .get("skills")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("The hub sent unusable skills"))?;
        Ok(skills
            .iter()
            .find(|skill| skill["name"] == name)
            .map(|found| skill_of(&self.0.bot_id, found)))
    }

    async fn propose(&self, request: CreateBotSkill) -> Result<BotSkill> {
        let body = json!({
            "name": request.name,
            "description": request.description,
            "body": request.body,
        });
        let created = self
            .0
            .hub
            .write(&self.0.path("skills/propose"), Vec::new(), body)
            .await
            .map_err(|failure| {
                anyhow!("Proposing the skill '{}' failed: {failure}", request.name)
            })?;
        Ok(skill_of(&self.0.bot_id, &created))
    }
}

/// The sign-in of the bot's own-key image model: the hub hands out the model's provider
/// `params` for one call while the turn runs; they stay in memory for that call.
pub struct HubImageSignIn(pub BotRoutes);

impl HubImageSignIn {
    async fn params(&self) -> Result<Value> {
        let answer = self
            .0
            .hub
            .write(
                &self.0.path("image-model/credential"),
                Vec::new(),
                json!({}),
            )
            .await
            .map_err(|failure| {
                anyhow!("The sign-in for the bot's image model failed: {failure}")
            })?;
        answer
            .get("params")
            .filter(|params| params.is_object())
            .cloned()
            .ok_or_else(|| anyhow!("The hub sent no sign-in for the bot's image model"))
    }
}

/// The image Bit one `image_generate` call runs with on this device: a hosted model as the
/// claim holds it, an own-key model with its sign-in from the hub, never a local model.
pub async fn image_bit_for_call(
    bit: &Bit,
    sign_in: &HubImageSignIn,
) -> std::result::Result<Bit, ImageUnavailable> {
    if is_local_image_bit(bit) {
        return Err(ImageUnavailable::Unusable(format!(
            "'{}' is a local stable-diffusion model that runs on its owner's desktop, and a bot on a device runs no local image models. The owner can pick a hosted image model, or one signed in with their own key, in the bot's settings",
            bit.id
        )));
    }
    if !is_own_key_image_bit(bit) {
        return Ok(bit.clone());
    }
    let params = sign_in
        .params()
        .await
        .map_err(|error| ImageUnavailable::Unusable(format!("{error:#}")))?;
    Ok(with_provider_params(bit.clone(), params))
}

const MAX_PACKAGE_BYTES: usize = 128 * 1024 * 1024;

/// What a package skill's tool runs with on this device: the bot's workspace as its storage and
/// the turn's executor token for models.
pub struct PackageRun {
    pub state: Arc<FlowLikeState>,
    /// The turn's executor token as of the call; the hub renews it while the turn runs.
    pub token: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    pub workspace: String,
    pub access: Arc<dyn AppAccess>,
    pub turn_id: String,
    pub owner_id: String,
    pub usage: ModelUsageContext,
}

/// The package one skill names, as the hub signs it for this turn.
#[derive(Debug, Deserialize)]
struct SkillPackage {
    package_id: String,
    version: String,
    wasm_url: String,
    wasm_hash: String,
    #[serde(default)]
    permissions: Option<Value>,
}

/// The bot's skills with their package tools run on this device. The hub names the version the
/// bot's workspace pins and signs a download of its portable WASM; the bytes must match the
/// hub's hash, and every tool runs in the sandbox this device runs flow packages in, with the
/// manifest's permissions. A tool that reaches the network or writes asks the owner first, as
/// on the desktop.
pub struct DeviceSkills {
    hub: HubSkills,
    run: PackageRun,
    http: reqwest::Client,
    loaded: tokio::sync::Mutex<HashMap<String, Arc<Vec<WasmNodeLogic>>>>,
}

impl DeviceSkills {
    pub fn new(hub: HubSkills, run: PackageRun) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self {
            hub,
            run,
            http,
            loaded: tokio::sync::Mutex::default(),
        }
    }

    async fn nodes(&self, skill: &BotSkill) -> Result<Arc<Vec<WasmNodeLogic>>> {
        if skill.id.is_empty()
            || !skill
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(anyhow!("Skill '{}' has an unusable id", skill.name));
        }
        let mut loaded = self.loaded.lock().await;
        if let Some(nodes) = loaded.get(&skill.id) {
            return Ok(nodes.clone());
        }
        let routes = &self.hub.0;
        let answer = routes
            .hub
            .read(
                &routes.path(&format!("skills/{}/package", skill.id)),
                Vec::new(),
            )
            .await
            .map_err(|failure| {
                anyhow!(
                    "The package of skill '{}' is unavailable: {failure}",
                    skill.name
                )
            })?;
        let package: SkillPackage = serde_json::from_value(answer)
            .map_err(|error| anyhow!("The hub sent an unusable skill package: {error}"))?;
        let bytes = self.download(&package).await?;
        let nodes = Arc::new(package_nodes(&package, &bytes).await?);
        tracing::info!(
            bot_id = %routes.bot_id,
            skill = %skill.name,
            package_id = %package.package_id,
            version = %package.version,
            "[bot_host] loaded a skill package"
        );
        loaded.insert(skill.id.clone(), nodes.clone());
        Ok(nodes)
    }

    async fn download(&self, package: &SkillPackage) -> Result<Vec<u8>> {
        let label = format!("'{}' v{}", package.package_id, package.version);
        let failed = |error: reqwest::Error| anyhow!("Downloading package {label} failed: {error}");
        let mut response = self
            .http
            .get(&package.wasm_url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| failed(error.without_url()))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| failed(error.without_url()))?
        {
            if bytes.len() + chunk.len() > MAX_PACKAGE_BYTES {
                return Err(anyhow!(
                    "Package {label} is larger than {MAX_PACKAGE_BYTES} bytes"
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        let actual = blake3::hash(&bytes).to_hex();
        if !actual.as_str().eq_ignore_ascii_case(&package.wasm_hash) {
            return Err(anyhow!(
                "Package {label} does not match the hash the hub named ({} instead of {})",
                actual,
                package.wasm_hash
            ));
        }
        Ok(bytes)
    }
}

async fn tool_node<'a>(
    nodes: &'a [WasmNodeLogic],
    skill: &BotSkill,
    tool: &str,
) -> Result<Option<&'a WasmNodeLogic>> {
    if !skill.tools.iter().any(|known| known == tool) {
        return Ok(None);
    }
    for node in nodes {
        let entry = node
            .tool_entry()
            .await
            .map_err(|error| anyhow!("{error}"))?;
        if entry.name == tool {
            return Ok(Some(node));
        }
    }
    Ok(None)
}

/// The package's nodes in this device's package sandbox.
async fn package_nodes(package: &SkillPackage, bytes: &[u8]) -> Result<Vec<WasmNodeLogic>> {
    let permissions: PackagePermissions = match &package.permissions {
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            anyhow!(
                "Package '{}' v{} has unreadable permissions: {error}",
                package.package_id,
                package.version
            )
        })?,
        None => PackagePermissions::default(),
    };
    let security = crate::dependencies::package_security(&permissions)?;
    let engine = skill_engine()?;
    let (loaded, definitions) =
        crate::dependencies::package_definitions(&engine, bytes, &security).await?;
    Ok(definitions
        .into_iter()
        .map(|definition| {
            crate::dependencies::package_node(
                &loaded,
                &engine,
                &security,
                definition,
                &package.package_id,
            )
        })
        .collect())
}

fn skill_engine() -> Result<Arc<flow_like_wasm::WasmEngine>> {
    static ENGINE: std::sync::Mutex<Option<Arc<flow_like_wasm::WasmEngine>>> =
        std::sync::Mutex::new(None);
    let mut engine = ENGINE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(engine) = engine.as_ref() {
        return Ok(engine.clone());
    }
    let made = crate::dependencies::package_engine()?;
    *engine = Some(made.clone());
    Ok(made)
}

/// Whether a node reaches the network or writes: those tools ask the owner first.
fn gating(capabilities: WasmCapabilities) -> (bool, bool) {
    (
        capabilities.intersects(WasmCapabilities::NETWORK_ALL),
        capabilities.intersects(
            WasmCapabilities::STORAGE_WRITE
                | WasmCapabilities::STORAGE_DELETE
                | WasmCapabilities::DATABASE_WRITE
                | WasmCapabilities::HTTP_WRITE,
        ),
    )
}

fn has_storage(capabilities: WasmCapabilities) -> bool {
    capabilities.intersects(
        WasmCapabilities::STORAGE_READ
            | WasmCapabilities::STORAGE_WRITE
            | WasmCapabilities::STORAGE_DELETE,
    )
}

#[async_trait]
impl SkillHost for DeviceSkills {
    async fn get(&self, name: &str) -> Result<Option<BotSkill>> {
        self.hub.get(name).await
    }

    async fn propose(&self, request: CreateBotSkill) -> Result<BotSkill> {
        self.hub.propose(request).await
    }

    async fn tool_specs(&self, skill: &BotSkill) -> Result<Vec<SkillToolSpec>> {
        if skill.source.package_id().is_none() {
            return Ok(Vec::new());
        }
        let nodes = self.nodes(skill).await?;
        let mut specs = Vec::new();
        for node in nodes.iter() {
            let entry = node
                .tool_entry()
                .await
                .map_err(|error| anyhow!("{error}"))?;
            if !skill.tools.contains(&entry.name) {
                continue;
            }
            let info = flow_like_wasm::skill_call::tool_info(&entry);
            let (network, writes) = gating(node.security().capabilities);
            specs.push(SkillToolSpec {
                name: info.name,
                description: info.description,
                input_schema: info.input_schema,
                network,
                writes,
            });
        }
        Ok(specs)
    }

    async fn call(&self, skill: &BotSkill, tool: &str, arguments: Value) -> Result<Value> {
        if self.run.workspace.is_empty() {
            return Err(anyhow!("The bot has no workspace for '{tool}' to use"));
        }
        let nodes = self.nodes(skill).await?;
        let node = tool_node(&nodes, skill, tool)
            .await?
            .ok_or_else(|| anyhow!("The package has no tool '{tool}'"))?;
        let workspace_store = if has_storage(node.security().capabilities) {
            Some(
                self.run
                    .access
                    .store(&self.run.workspace)
                    .await
                    .map_err(|error| {
                        anyhow!("The bot's workspace is unavailable to '{tool}': {error}")
                    })?,
            )
        } else {
            None
        };
        node.run_headless(
            HeadlessRun {
                app_state: self.run.state.clone(),
                token: (self.run.token)(),
                workspace: self.run.workspace.clone(),
                workspace_store,
                turn_id: self.run.turn_id.clone(),
                user_id: self.run.owner_id.clone(),
                usage: Some(self.run.usage.clone()),
            },
            &arguments,
        )
        .await
        .map_err(|error| anyhow!(error))
    }
}

/// The bot's connectors: the hub lists them and signs each call, this device makes the calls.
/// The hub hands out an access token or the opened secret for one call, never a refresh token.
pub struct HubConnectors {
    routes: BotRoutes,
    budget: CallBudget,
}

impl HubConnectors {
    pub fn new(routes: BotRoutes) -> Self {
        Self {
            routes,
            budget: CallBudget::default(),
        }
    }
}

#[async_trait]
impl ConnectorSource for HubConnectors {
    async fn entries(&self) -> Result<Vec<ConnectorEntry>> {
        let listed = self
            .routes
            .hub
            .read(&self.routes.path("connectors/entries"), Vec::new())
            .await
            .map_err(|failure| anyhow!("Listing the connectors failed: {failure}"))?;
        serde_json::from_value(
            listed
                .get("entries")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        )
        .map_err(|error| anyhow!("The hub sent unusable connectors: {error}"))
    }

    async fn credential(&self, connector_id: &str) -> Result<Credential> {
        let answer = self
            .routes
            .hub
            .write(
                &self
                    .routes
                    .path(&format!("connectors/{connector_id}/credential")),
                Vec::new(),
                json!({}),
            )
            .await
            .map_err(|failure| {
                anyhow!("The sign-in for connector '{connector_id}' failed: {failure}")
            })?;
        Ok(answer["authorization"]
            .as_str()
            .map_or_else(Credential::none, Credential::authorization))
    }

    fn budget(&self) -> &CallBudget {
        &self.budget
    }

    fn audit(&self, entry: &CallAudit) {
        tracing::info!(
            bot_id = %self.routes.bot_id,
            connector = %entry.connector_id,
            operation = %entry.operation,
            effect = entry.effect,
            ok = entry.ok,
            "[bot_host] connector call"
        );
    }
}

pub struct HubPeople(pub BotRoutes);

pub struct HubUserContext(pub BotRoutes);

/// `room_invite` through the hub, into the conversation the turn runs in.
pub struct HubRooms(pub BotRoutes);

#[async_trait]
impl flow_like_bot_tools::RoomInvites for HubRooms {
    async fn invite(
        &self,
        person: &flow_like_bot_tools::InviteTarget,
    ) -> Result<flow_like_bot_tools::InviteOutcome> {
        let room = self
            .0
            .conversation_id
            .as_ref()
            .ok_or_else(|| anyhow!("Inviting people needs the group chat the turn runs in"))?;
        let path = self.0.path(&format!("rooms/{room}/invitations"));
        match self
            .0
            .hub
            .write(&path, Vec::new(), json!({ "person": person.value() }))
            .await
        {
            Ok(reply) => flow_like_bot_tools::rooms::hub_reply(&reply),
            Err(failure) => failure
                .code
                .as_deref()
                .and_then(|code| flow_like_bot_tools::rooms::hub_refusal(code, &failure.message))
                .ok_or_else(|| anyhow!("Inviting '{}' failed: {failure}", person.value())),
        }
    }
}

#[async_trait]
impl flow_like_bot_tools::user_context::UserContextBackend for HubUserContext {
    async fn available(
        &self,
    ) -> Result<flow_like_bot_tools::user_context::UserContextAvailability> {
        let room = self
            .0
            .conversation_id
            .as_ref()
            .ok_or_else(|| anyhow!("User context needs a conversation"))?;
        let answer = self
            .0
            .hub
            .read(
                &self.0.path("user-context/available"),
                vec![("conversation_id".into(), room.clone())],
            )
            .await
            .map_err(|error| anyhow!("Could not list context participants: {error}"))?;
        Ok(serde_json::from_value(answer)?)
    }

    async fn request(
        &self,
        args: flow_like_bot_tools::user_context::UserContextRequestArgs,
    ) -> Result<flow_like_bot_tools::user_context::UserContextRequestReceipt> {
        let room = self
            .0
            .conversation_id
            .as_ref()
            .ok_or_else(|| anyhow!("User context needs a conversation"))?;
        let mut body = serde_json::to_value(args)?;
        body["conversation_id"] = json!(room);
        let answer = self
            .0
            .hub
            .write(&self.0.path("user-context/request"), Vec::new(), body)
            .await
            .map_err(|error| anyhow!("Could not request user context: {error}"))?;
        Ok(serde_json::from_value(answer)?)
    }
}

fn contacts_of(value: &Value) -> Vec<Contact> {
    value
        .get("people")
        .and_then(Value::as_array)
        .map(|people| {
            people
                .iter()
                .filter_map(|person| {
                    Some(Contact {
                        id: person.get("id")?.as_str()?.to_owned(),
                        name: person.get("name")?.as_str()?.to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn message_of(value: &Value) -> Result<PersonMessage> {
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(PersonMessage {
        conversation_id: text("conversation_id")
            .ok_or_else(|| anyhow!("The hub sent a message receipt without a conversation"))?,
        message_id: text("message_id").unwrap_or_default(),
        name: text("name").unwrap_or_default(),
        notified: value
            .get("notified")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

impl HubPeople {
    async fn name_of(&self, person_id: &str) -> String {
        self.contacts(None)
            .await
            .ok()
            .and_then(|people| people.into_iter().find(|person| person.id == person_id))
            .map(|person| person.name)
            .unwrap_or_else(|| "This person".to_owned())
    }
}

#[async_trait]
impl PeopleBackend for HubPeople {
    async fn contacts(&self, query: Option<&str>) -> Result<Vec<Contact>> {
        let query = query.map(|q| vec![("q", q.to_owned())]).unwrap_or_default();
        let reply = self
            .0
            .hub
            .read(&self.0.path("people"), query)
            .await
            .map_err(|failure| anyhow!("Listing the owner's people failed: {failure}"))?;
        Ok(contacts_of(&reply))
    }

    async fn sent_since(&self, _since_ms: i64) -> Result<usize> {
        Ok(0)
    }

    async fn message(&self, person_id: &str, text: &str) -> Result<Delivery> {
        let body = json!({ "person": person_id, "text": text });
        match self
            .0
            .hub
            .write(&self.0.path("people/message"), Vec::new(), body)
            .await
        {
            Ok(reply) => Ok(Delivery::Sent(message_of(&reply)?)),
            Err(failure) => match failure.status {
                Some(404) => Ok(Delivery::NotAContact),
                Some(409) => Ok(Delivery::Left {
                    name: self.name_of(person_id).await,
                }),
                Some(429) => Err(anyhow!(
                    "This bot already sent {} messages to people in the last hour; tell your owner what is left instead",
                    flow_like_bot_tools::people::MAX_PER_WINDOW
                )),
                _ => Err(anyhow!("Messaging '{person_id}' failed: {failure}")),
            },
        }
    }
}

pub struct HubMail(pub BotRoutes);

fn attachments_json(files: &[MailFile]) -> Value {
    Value::Array(
        files
            .iter()
            .map(|file| {
                json!({
                    "filename": file.filename,
                    "content_type": file.content_type,
                    "content": STANDARD.encode(&file.bytes),
                })
            })
            .collect(),
    )
}

fn failure_of(message: &str) -> MailFailure {
    let lowered = message.to_ascii_lowercase();
    let code =
        if lowered.contains("429") || lowered.contains("limit") || lowered.contains("too many") {
            failure::LIMIT
        } else if lowered.contains("404") || lowered.contains("no mail thread") {
            failure::NO_THREAD
        } else if lowered.contains("no email address") || lowered.contains("no address") {
            failure::NO_ADDRESS
        } else if lowered.contains("400") || lowered.contains("403") {
            failure::REFUSED
        } else {
            failure::UNAVAILABLE
        };
    MailFailure::new(code, message)
}

fn sent_of(value: &Value) -> std::result::Result<SentMail, MailFailure> {
    let unusable = || {
        MailFailure::new(
            failure::UNAVAILABLE,
            "The hub sent an unusable mail receipt",
        )
    };
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(SentMail {
        message_id: text("message_id").ok_or_else(unusable)?,
        thread: text("thread").ok_or_else(unusable)?,
        from: text("from").ok_or_else(unusable)?,
        to: value
            .get("to")
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        subject: text("subject").unwrap_or_default(),
    })
}

impl HubMail {
    async fn post(&self, tail: &str, body: Value) -> std::result::Result<SentMail, MailFailure> {
        let reply = self
            .0
            .hub
            .write(&self.0.path(&format!("mail/{tail}")), Vec::new(), body)
            .await
            .map_err(|failure| failure_of(&failure.to_string()))?;
        sent_of(&reply)
    }
}

#[async_trait]
impl MailBackend for HubMail {
    async fn send(&self, mail: OutgoingMail) -> std::result::Result<SentMail, MailFailure> {
        self.post(
            "send",
            json!({
                "to": mail.to, "cc": mail.cc, "subject": mail.subject,
                "text": mail.text, "html": mail.html,
                "attachments": attachments_json(&mail.attachments),
            }),
        )
        .await
    }

    async fn reply(&self, mail: ReplyMail) -> std::result::Result<SentMail, MailFailure> {
        self.post(
            "reply",
            json!({
                "thread": mail.thread, "text": mail.text, "html": mail.html,
                "attachments": attachments_json(&mail.attachments),
            }),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_bot_host::hub::HubFailure;
    use flow_like_types::bot_turn::BotTurn;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Calls(
        Mutex<Vec<(String, String, Value)>>,
        Mutex<Vec<Result<Value, HubFailure>>>,
    );

    impl Calls {
        fn answering(answers: Vec<Result<Value, HubFailure>>) -> Arc<Self> {
            Arc::new(Self(Mutex::default(), Mutex::new(answers)))
        }

        fn next(&self) -> Result<Value, HubFailure> {
            self.1.lock().unwrap().remove(0)
        }
    }

    #[async_trait]
    impl TurnHub for Calls {
        async fn list(&self) -> Result<Vec<BotTurn>, HubFailure> {
            Ok(Vec::new())
        }
        async fn claim(&self, _: &str, _: &str) -> Result<BotTurn, HubFailure> {
            Err(HubFailure::offline("unused"))
        }
        async fn heartbeat(&self, _: &str, _: &str) -> Result<BotTurn, HubFailure> {
            Err(HubFailure::offline("unused"))
        }
        async fn entries(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<flow_like_local_bots::InboxEntry>, HubFailure> {
            Ok(Vec::new())
        }
        async fn send(
            &self,
            _: &str,
            _: &str,
            _: Value,
            _: &str,
            _: &str,
        ) -> Result<(), HubFailure> {
            Ok(())
        }
        async fn desktop_bots(&self) -> Result<Vec<String>, HubFailure> {
            Ok(Vec::new())
        }
        async fn report_host(&self, _: &str, _: &[String]) -> Result<Vec<String>, HubFailure> {
            Ok(Vec::new())
        }
        async fn delete_host(&self, _: &str) -> Result<(), HubFailure> {
            Ok(())
        }
        async fn cancel_turn(&self, _: &str, _: &str) -> Result<(), HubFailure> {
            Ok(())
        }
        async fn read(
            &self,
            path: &str,
            query: Vec<(&'static str, String)>,
        ) -> Result<Value, HubFailure> {
            self.0
                .lock()
                .unwrap()
                .push(("GET".into(), path.into(), json!(query)));
            self.next()
        }
        async fn write(
            &self,
            path: &str,
            _: Vec<(&'static str, String)>,
            body: Value,
        ) -> Result<Value, HubFailure> {
            self.0
                .lock()
                .unwrap()
                .push(("POST".into(), path.into(), body));
            self.next()
        }
        async fn patch(
            &self,
            path: &str,
            _: Vec<(&'static str, String)>,
            body: Value,
        ) -> Result<Value, HubFailure> {
            self.0
                .lock()
                .unwrap()
                .push(("PATCH".into(), path.into(), body));
            self.next()
        }
    }

    fn routes(calls: &Arc<Calls>) -> BotRoutes {
        BotRoutes {
            hub: calls.clone(),
            bot_id: "bot1".into(),
            conversation_id: Some("conv".into()),
        }
    }

    #[tokio::test]
    async fn context_requests_keep_the_turn_room_and_wait_for_the_person() {
        use flow_like_bot_tools::user_context::{UserContextBackend, UserContextRequestArgs};
        let calls = Calls::answering(vec![
            Ok(
                json!({"participants":[{"id":"person","name":"Person"}],"fields":["location"],"device_support":"unknown_until_shared"}),
            ),
            Ok(
                json!({"request_id":"request","conversation_id":"conv","subject_id":"person","expires_at":12345}),
            ),
        ]);
        let backend = HubUserContext(routes(&calls));
        assert_eq!(
            backend.available().await.unwrap().participants[0].id,
            "person"
        );
        let args: UserContextRequestArgs = serde_json::from_value(json!({
            "subject_id":"person", "fields":["location"], "purpose":"Find a nearby cafe",
        }))
        .unwrap();
        assert_eq!(backend.request(args).await.unwrap().request_id, "request");
        let calls = calls.0.lock().unwrap();
        assert_eq!(calls[0].1, "bots/bot1/user-context/available");
        assert_eq!(calls[0].2, json!([["conversation_id", "conv"]]));
        assert_eq!(calls[1].1, "bots/bot1/user-context/request");
        assert_eq!(calls[1].2["conversation_id"], "conv");
        assert_eq!(calls[1].2["precision"], "approximate");
        assert!(calls[1].2.get("values").is_none());
    }

    #[tokio::test]
    async fn a_notice_goes_to_the_bots_notify_route_with_its_conversation() {
        let calls = Calls::answering(vec![Ok(json!({ "id": "m1", "posted": true }))]);
        let receipt = HubNotifier(routes(&calls))
            .deliver(OwnerNotice {
                title: Some("T".into()),
                text: "hi".into(),
            })
            .await
            .unwrap();
        assert!(receipt.posted && !receipt.muted);
        let (method, path, body) = calls.0.lock().unwrap()[0].clone();
        assert_eq!(
            (method.as_str(), path.as_str()),
            ("POST", "bots/bot1/notify")
        );
        assert_eq!(body["conversation_id"], "conv");
        assert_eq!(body["title"], "T");
    }

    #[tokio::test]
    async fn scheduled_monitoring_intent_reaches_the_hub_and_roundtrips() {
        for monitoring in [false, true] {
            let calls = Calls::answering(vec![Ok(json!({
                "id":"task1", "bot_id":"bot1", "kind":"cron", "cron":"0 9 * * *",
                "instruction":"Check the report", "title":"Report", "enabled":true,
                "monitoring":monitoring, "created_by":"bot", "created_at":"2026-11-08T12:00:00Z"
            }))]);
            let task = HubScheduler(routes(&calls))
                .create(CreateBotTask {
                    kind: Some(flow_like_types::bot_task::TaskKind::Cron),
                    cron: Some("0 9 * * *".into()),
                    instruction: "Check the report".into(),
                    monitoring,
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(task.monitoring, monitoring);
            let (method, path, body) = calls.0.lock().unwrap()[0].clone();
            assert_eq!(
                (method.as_str(), path.as_str()),
                ("POST", "bots/bot1/tasks")
            );
            assert_eq!(body["monitoring"], monitoring);
        }
    }

    #[tokio::test]
    async fn a_task_change_is_a_patch_of_only_what_was_given() {
        let task = json!({
            "id":"task1", "bot_id":"bot1", "kind":"once", "instruction":"New",
            "title":"Report", "enabled":false, "created_by":"bot",
            "created_at":"2026-11-08T12:00:00Z"
        });
        let calls = Calls::answering(vec![Ok(json!({ "tasks": [task.clone()] })), Ok(task)]);
        let changed = HubScheduler(routes(&calls))
            .update(
                "task1",
                flow_like_types::bot_task::UpdateBotTask {
                    instruction: Some("New".into()),
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .expect("the task exists");
        assert!(!changed.enabled);
        let calls = calls.0.lock().unwrap();
        assert_eq!(
            (calls[1].0.as_str(), calls[1].1.as_str()),
            ("PATCH", "bots/bot1/tasks/task1")
        );
        assert_eq!(calls[1].2["instruction"], "New");
        assert_eq!(calls[1].2["enabled"], false);
    }

    #[tokio::test]
    async fn people_refusals_become_deliveries() {
        let calls = Calls::answering(vec![Err(HubFailure::answered(404, None, "no"))]);
        let delivery = HubPeople(routes(&calls)).message("p", "hi").await.unwrap();
        assert!(matches!(delivery, Delivery::NotAContact));
        let limited = Calls::answering(vec![Err(HubFailure::answered(429, None, "slow"))]);
        assert!(
            HubPeople(routes(&limited))
                .message("p", "hi")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn contacts_pass_the_query_as_a_parameter() {
        let calls = Calls::answering(vec![Ok(
            json!({ "people": [{ "id": "p1", "name": "Ada" }] }),
        )]);
        let people = HubPeople(routes(&calls))
            .contacts(Some("a b"))
            .await
            .unwrap();
        assert_eq!(people.len(), 1);
        assert_eq!(calls.0.lock().unwrap()[0].2, json!([["q", "a b"]]));
    }

    #[tokio::test]
    async fn skills_are_read_from_the_bots_list_and_proposals_go_to_propose() {
        let skill = json!({
            "id": "s1", "name": "invoices", "description": "d", "body": "b",
            "source": "import", "status": "active", "tools": [], "updated_at": "t"
        });
        let calls = Calls::answering(vec![Ok(json!({ "skills": [skill.clone()] })), Ok(skill)]);
        let skills = HubSkills(routes(&calls));
        let found = skills.get("invoices").await.unwrap().expect("listed");
        assert_eq!(found.status, SkillStatus::Active);
        let proposed = skills
            .propose(CreateBotSkill {
                name: "invoices".into(),
                description: "d".into(),
                body: "b".into(),
                tools: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(proposed.name, "invoices");
        let calls = calls.0.lock().unwrap();
        assert_eq!(calls[0].1, "bots/bot1/skills");
        assert_eq!(calls[1].1, "bots/bot1/skills/propose");
    }

    #[tokio::test]
    async fn a_connector_sign_in_comes_from_the_hub_per_call() {
        let calls = Calls::answering(vec![
            Ok(json!({ "authorization": "Bearer abc" })),
            Ok(json!({ "authorization": null })),
        ]);
        let connectors = HubConnectors::new(routes(&calls));
        let signed = connectors.credential("c1").await.unwrap();
        assert_eq!(signed.header(), Some("Bearer abc"));
        assert_eq!(connectors.credential("c2").await.unwrap().header(), None);
        let calls = calls.0.lock().unwrap();
        assert_eq!(
            (calls[0].0.as_str(), calls[0].1.as_str()),
            ("POST", "bots/bot1/connectors/c1/credential")
        );
    }

    fn skill_module() -> Vec<u8> {
        let pin = |name: &str, pin_type: &str| {
            json!({
                "name": name, "friendly_name": name, "description": "",
                "pin_type": pin_type, "data_type": "String",
            })
        };
        let node = |name: &str, permissions: Value| {
            json!({
                "name": name, "friendly_name": name, "description": "A test tool",
                "category": "Tests", "permissions": permissions,
                "pins": [pin("text", "Input"), pin("echo", "Output")],
            })
        };
        let definitions = json!([
            node("skill_echo", json!([])),
            node("skill_fetch", json!(["network:http"])),
            node("skill_hidden", json!([])),
        ])
        .to_string();
        let result = json!({ "outputs": { "echo": "hi" } }).to_string();
        let escape = |text: &str| {
            text.bytes()
                .map(|byte| format!("\\{byte:02x}"))
                .collect::<String>()
        };
        let at: u64 = 8192;
        wat::parse_str(format!(
            r#"(module
            (memory (export "memory") 1)
            (data (i32.const 0) "{}")
            (data (i32.const {at}) "{}")
            (func (export "get_nodes") (result i64) i64.const {})
            (func (export "run") (param i32 i32) (result i64) i64.const {}))"#,
            escape(&definitions),
            escape(&result),
            definitions.len(),
            (at << 32) | result.len() as u64,
        ))
        .unwrap()
    }

    struct NoStorage;

    #[async_trait]
    impl AppAccess for NoStorage {
        async fn store(
            &self,
            app_id: &str,
        ) -> Result<flow_like_storage::files::store::FlowLikeStore> {
            Err(anyhow!("No storage for '{app_id}' in this test"))
        }

        async fn connection(&self, app_id: &str) -> Result<flow_like_storage::lancedb::Connection> {
            Err(anyhow!("No database for '{app_id}' in this test"))
        }
    }

    async fn serve(bytes: Vec<u8>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = axum::Router::new().route(
            "/package.wasm",
            axum::routing::get(move || {
                let bytes = bytes.clone();
                async move { bytes }
            }),
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{address}/package.wasm")
    }

    fn package_skill() -> BotSkill {
        skill_of(
            "bot1",
            &json!({
                "id": "s1", "name": "tools", "description": "d", "body": "b",
                "source": "package:acme.tools@1.0.0", "status": "active",
                "tools": ["skill_echo", "skill_fetch"], "updated_at": "t"
            }),
        )
    }

    async fn device_skills(calls: &Arc<Calls>, root: &std::path::Path) -> DeviceSkills {
        let state = crate::runtime::offline_state(root, None).await.unwrap();
        DeviceSkills::new(
            HubSkills(routes(calls)),
            PackageRun {
                state,
                token: Arc::new(|| None),
                workspace: "ws".into(),
                access: Arc::new(NoStorage),
                turn_id: "turn".into(),
                owner_id: "owner".into(),
                usage: ModelUsageContext {
                    app_id: Some("ws".into()),
                    run_id: Some("turn".into()),
                    api_base_url: None,
                },
            },
        )
    }

    #[tokio::test]
    async fn package_skill_tools_run_in_the_device_sandbox_from_the_hubs_signed_package() {
        let bytes = skill_module();
        let url = serve(bytes.clone()).await;
        let calls = Calls::answering(vec![Ok(json!({
            "package_id": "acme.tools", "version": "1.0.0", "wasm_url": url,
            "wasm_hash": blake3::hash(&bytes).to_hex().to_string(),
            "permissions": { "memory": "light" },
        }))]);
        let root = tempfile::tempdir().unwrap();
        let skills = device_skills(&calls, root.path()).await;
        let skill = package_skill();
        let specs = skills.tool_specs(&skill).await.unwrap();
        let gated: Vec<(&str, bool, bool)> = specs
            .iter()
            .map(|spec| (spec.name.as_str(), spec.network, spec.writes))
            .collect();
        assert_eq!(
            gated,
            [("skill_echo", false, false), ("skill_fetch", true, true)]
        );
        let output = skills
            .call(&skill, "skill_echo", json!({ "text": "hello" }))
            .await
            .unwrap();
        assert_eq!(output["echo"], "hi");
        let hidden = skills
            .call(&skill, "skill_hidden", json!({}))
            .await
            .unwrap_err();
        assert!(hidden.to_string().contains("no tool 'skill_hidden'"));
        let calls = calls.0.lock().unwrap();
        assert_eq!(calls.len(), 1, "one package fetch per turn");
        assert_eq!(calls[0].1, "bots/bot1/skills/s1/package");
    }

    #[tokio::test]
    async fn a_package_that_differs_from_the_hubs_hash_never_loads() {
        let url = serve(skill_module()).await;
        let calls = Calls::answering(vec![Ok(json!({
            "package_id": "acme.tools", "version": "1.0.0", "wasm_url": url,
            "wasm_hash": "0".repeat(64),
        }))]);
        let root = tempfile::tempdir().unwrap();
        let skills = device_skills(&calls, root.path()).await;
        let error = skills
            .tool_specs(&package_skill())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("does not match the hash"), "{error}");
    }

    #[tokio::test]
    async fn an_instruction_skill_needs_no_package() {
        let calls = Calls::answering(Vec::new());
        let root = tempfile::tempdir().unwrap();
        let skills = device_skills(&calls, root.path()).await;
        let mut skill = package_skill();
        skill.source = SkillSource::Import;
        assert!(skills.tool_specs(&skill).await.unwrap().is_empty());
        assert!(calls.0.lock().unwrap().is_empty());
    }

    fn image_bit(provider_name: &str) -> Bit {
        Bit {
            id: "img".into(),
            bit_type: flow_like::bit::BitTypes::ImageGeneration,
            parameters: json!({ "provider": { "provider_name": provider_name, "model_id": "m" } }),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn an_own_key_image_model_gets_its_sign_in_from_the_hub_per_call() {
        let calls = Calls::answering(vec![
            Ok(json!({ "bit_id": "img", "params": { "api_key": "k1" } })),
            Ok(json!({ "bit_id": "img", "params": { "api_key": "k2" } })),
        ]);
        let sign_in = HubImageSignIn(routes(&calls));
        let held = image_bit("openai");
        for key in ["k1", "k2"] {
            let signed = image_bit_for_call(&held, &sign_in).await.unwrap();
            assert_eq!(signed.parameters["provider"]["params"]["api_key"], key);
        }
        assert!(held.parameters["provider"].get("params").is_none());
        let calls = calls.0.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            (calls[0].0.as_str(), calls[0].1.as_str()),
            ("POST", "bots/bot1/image-model/credential")
        );
    }

    #[tokio::test]
    async fn hosted_models_need_no_sign_in_and_local_models_are_refused_with_a_reason() {
        let calls = Calls::answering(Vec::new());
        let sign_in = HubImageSignIn(routes(&calls));
        let hosted = image_bit("hosted:openrouter");
        assert_eq!(
            image_bit_for_call(&hosted, &sign_in).await.unwrap().id,
            "img"
        );
        let local = image_bit(flow_like::flow_like_model_provider::stablediffusion::PROVIDER_NAME);
        let ImageUnavailable::Unusable(why) =
            image_bit_for_call(&local, &sign_in).await.unwrap_err()
        else {
            panic!("a local model is unusable on a device");
        };
        assert!(why.contains("runs no local image models"), "{why}");
        assert!(calls.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_refused_sign_in_makes_the_image_model_unusable() {
        let calls = Calls::answering(vec![Err(HubFailure::answered(
            403,
            None,
            "not on a device",
        ))]);
        let refused = image_bit_for_call(&image_bit("openai"), &HubImageSignIn(routes(&calls)))
            .await
            .unwrap_err();
        assert!(matches!(refused, ImageUnavailable::Unusable(_)));
    }

    #[test]
    fn mail_failures_keep_their_meaning() {
        assert_eq!(failure_of("429 limit").code, failure::LIMIT);
        assert_eq!(failure_of("404 no mail thread").code, failure::NO_THREAD);
        assert_eq!(failure_of("boom").code, failure::UNAVAILABLE);
    }
}
