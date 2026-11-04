//! A `global_chat` run as one of the person's bots: which bot a request names (a bot on this
//! device, an account bot read with the run's sign-in, or Assistant's defaults), the apps it may
//! use, the model it runs on, and the desktop bridge for its tools. Nothing here needs a webview:
//! a run without one (`global_chat_native` with no webview) offers only natively run tools.

use std::{
    collections::{BTreeSet, HashMap},
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use flow_like::{
    flow::copilot::{
        BotRunContext, BotToolHost, BotToolset, FLUX_BOT_ID, FLUX_BOT_NAME,
        GENERAL_BOT_DESCRIPTION, GENERAL_BOT_ID, GENERAL_BOT_NAME, bot_persona_section,
        memory::MemoryScope, platform::PlatformToolBridge, resolve_bot_model,
        resolve_host_completion_model, runnable_completion_model,
    },
    profile::Profile,
    state::FlowLikeState,
};
use flow_like_types::{
    bot::{BotCapability, BotModelPolicy, BotSystemKind},
    data_classification::DataSensitivity,
};
use reqwest::Method;
use serde::Deserialize;
use tauri::{AppHandle, Manager};

use super::super::{
    backend_types::{FlowPilotChatBackend, FlowPilotModelSelection},
    chat::copilot_profile_for,
    platform_bridge::{DesktopPlatformBridge, FrontendPlatformToolSet},
};
use crate::functions::{
    ai::{
        native_platform_tools::{RunWorkspace, segment},
        native_tool_bridge::NativeRunIdentity,
    },
    local_bots::{
        LocalBots, local_bots_apps, local_bots_get, local_built_in, workspace::local_workspace,
    },
};

const MAX_BOT_RESPONSE_BYTES: usize = 1024 * 1024;

/// What a request's `bot_id` asks for, before anything is looked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BotRequest<'a> {
    /// No bot: the FlowPilot path, unchanged.
    FlowPilot,
    /// Flux: its account row with its edits when signed in, otherwise the FlowPilot path.
    Flux,
    /// Assistant without an account row: its defaults.
    General,
    /// A bot of this device or of the signed-in account.
    Bot(&'a str),
}

pub(super) fn bot_request(bot_id: Option<&str>) -> BotRequest<'_> {
    match bot_id.map(str::trim).filter(|bot_id| !bot_id.is_empty()) {
        None => BotRequest::FlowPilot,
        Some(FLUX_BOT_ID) => BotRequest::Flux,
        Some(GENERAL_BOT_ID) => BotRequest::General,
        Some(bot_id) => BotRequest::Bot(bot_id),
    }
}

/// Whose memory a request means, the store the bot's chat uses: FlowPilot's in `profile_id`
/// without a bot, for `flux` and for a Flux row this desktop already ran; otherwise the bot's own
/// store. [`resolved_memory_scope`] also reads a bot this desktop hasn't run.
pub(super) fn memory_scope(profile_id: String, bot_id: Option<&str>) -> MemoryScope {
    match bot_request(bot_id) {
        BotRequest::FlowPilot | BotRequest::Flux => MemoryScope::Profile(profile_id),
        BotRequest::General => MemoryScope::Bot(GENERAL_BOT_ID.to_string()),
        BotRequest::Bot(bot_id) if remembered_as_flux(bot_id) => MemoryScope::Profile(profile_id),
        BotRequest::Bot(bot_id) => MemoryScope::Bot(bot_id.to_string()),
    }
}

/// [`memory_scope`] for the memory commands: a bot this desktop hasn't run yet is read from this
/// device or the signed-in account of `profile_id`, as a chat with it would, so Flux's row reaches
/// FlowPilot's memory there too.
pub(super) async fn resolved_memory_scope(
    app: &AppHandle,
    profile_id: String,
    bot_id: Option<&str>,
) -> Result<MemoryScope, String> {
    let scope = memory_scope(profile_id.clone(), bot_id);
    let MemoryScope::Bot(bot_id) = &scope else {
        return Ok(scope);
    };
    if bot_id == GENERAL_BOT_ID {
        return Ok(scope);
    }
    let system = bot_system(app, &profile_id, bot_id).await?;
    Ok(BotRunContext {
        bot_id: bot_id.clone(),
        system,
        ..Default::default()
    }
    .memory_scope(&profile_id))
}

/// The built-in kind of `bot_id`: from this device's store, else from the signed-in account of
/// `profile_id`. Another person's or an unknown id fails like a chat with it.
async fn bot_system(
    app: &AppHandle,
    profile_id: &str,
    bot_id: &str,
) -> Result<Option<BotSystemKind>, String> {
    if let Some(store) = app.try_state::<LocalBots>()
        && let Some(bot) = local_bots_get(store.clone(), bot_id.to_string())
            .await
            .map_err(|error| error.to_string())?
    {
        return Ok(bot.system);
    }
    let identity = profile_identity(app, profile_id).await?;
    let (_, record) = account_record(identity.as_ref(), bot_id).await?;
    Ok(record.system())
}

/// The run identity of `profile_id`'s signed-in session; `None` signed out.
async fn profile_identity(
    app: &AppHandle,
    profile_id: &str,
) -> Result<Option<NativeRunIdentity>, String> {
    let settings = crate::state::TauriSettingsState::construct(app)
        .await
        .map_err(|error| error.to_string())?;
    let profile = settings
        .lock()
        .await
        .profile_by_id(profile_id)
        .map_err(|error| error.to_string())?;
    let Some(hub) =
        flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
    else {
        return Ok(None);
    };
    let auth = crate::native_auth::SessionBinding::capture(&profile.hub_profile.id, &hub).await?;
    Ok(auth.map(|auth| NativeRunIdentity {
        profile_id: profile.hub_profile.id,
        hub,
        subject: Some(auth.subject.clone()),
        auth: Some(auth),
    }))
}

/// A bot as the local store or `GET /bots/{id}` returns it: the fields a run needs.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct BotRecord {
    pub(super) id: String,
    #[serde(default)]
    pub(super) system: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) description: Option<String>,
    #[serde(default)]
    pub(super) instructions: Option<String>,
    #[serde(default)]
    pub(super) model: BotModelPolicy,
    #[serde(default)]
    pub(super) embedding_model: Option<String>,
    #[serde(default)]
    pub(super) stt_model: Option<String>,
    #[serde(default)]
    pub(super) tts_model: Option<String>,
    #[serde(default)]
    pub(super) image_model: Option<String>,
    #[serde(default)]
    pub(super) vision_model: Option<String>,
    #[serde(default)]
    pub(super) classification: DataSensitivity,
    #[serde(default)]
    pub(super) capabilities: Vec<String>,
    #[serde(default)]
    pub(super) profile_id: Option<String>,
}

impl From<flow_like_local_bots::Bot> for BotRecord {
    fn from(bot: flow_like_local_bots::Bot) -> Self {
        Self {
            id: bot.id,
            system: bot.system.map(|kind| kind.as_str().to_string()),
            name: bot.name,
            description: bot.description,
            instructions: bot.instructions,
            model: bot.model,
            embedding_model: bot.embedding_model,
            stt_model: bot.stt_model,
            tts_model: bot.tts_model,
            image_model: bot.image_model,
            vision_model: bot.vision_model,
            classification: bot.classification,
            capabilities: bot
                .capabilities
                .iter()
                .map(|capability| capability.as_str().to_string())
                .collect(),
            profile_id: bot.profile_id,
        }
    }
}

impl BotRecord {
    fn system(&self) -> Option<BotSystemKind> {
        self.system.as_deref().and_then(BotSystemKind::parse)
    }

    /// The ids this desktop knows, plus what a built-in bot always holds (Flux: `flowpilot`, also
    /// from a hub that predates it).
    fn capabilities(&self) -> Vec<BotCapability> {
        let mut capabilities: Vec<BotCapability> = self
            .capabilities
            .iter()
            .filter_map(|id| BotCapability::parse(id))
            .collect();
        let fixed = self
            .system()
            .map_or(&[][..], BotSystemKind::fixed_capabilities);
        for capability in fixed {
            if !capabilities.contains(capability) {
                capabilities.push(*capability);
            }
        }
        capabilities
    }
}

#[derive(Debug, Default, Deserialize)]
struct BotRecordList {
    #[serde(default)]
    bots: Vec<BotRecord>,
}

/// An app attached to a bot and the owner's current role there; a role-less app is out of scope.
#[derive(Debug, Clone, Deserialize)]
pub(super) struct BotAppRecord {
    pub(super) app_id: String,
    #[serde(default)]
    pub(super) owner_role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BotAppList {
    #[serde(default)]
    apps: Vec<BotAppRecord>,
}

/// What `POST /bots/{id}/workspace` answers.
#[derive(Debug, Deserialize)]
struct WorkspaceRecord {
    #[serde(default)]
    app_id: String,
}

impl WorkspaceRecord {
    fn app_id(self) -> Option<String> {
        Some(self.app_id).filter(|app_id| !app_id.trim().is_empty())
    }
}

/// Flux has no workspace yet, here and on the account; every other bot has one.
fn wants_workspace(record: &BotRecord) -> bool {
    record.system() != Some(BotSystemKind::Flux)
}

/// The bot a run answers as, and the model policy hosts resolve.
#[derive(Debug, Clone)]
pub(super) struct DesktopBot {
    pub(super) context: Arc<BotRunContext>,
    pub(super) model: BotModelPolicy,
}

impl DesktopBot {
    fn general(tool_host: BotToolHost) -> Self {
        Self {
            context: Arc::new(BotRunContext {
                tool_host,
                ..BotRunContext::general()
            }),
            model: BotModelPolicy::default(),
        }
    }

    /// The bot with its workspace app, which its run's tools may always use.
    fn with_workspace(mut self, app_id: Option<String>) -> Self {
        Arc::make_mut(&mut self.context).workspace_app_id = app_id;
        self
    }

    /// Lets the tools of run `run_id` use the bot's workspace, an app no profile lists, until the
    /// guard drops.
    pub(super) fn admit_workspace(&self, run_id: &str) -> Option<RunWorkspace> {
        let app_id = self.context.workspace_app_id.as_deref()?;
        Some(RunWorkspace::admit(run_id, app_id))
    }

    /// Flux and its clones: FlowPilot's toolset on every backend, persona on top.
    pub(super) fn runs_flowpilot(&self) -> bool {
        self.context.toolset == BotToolset::FlowPilot
    }

    /// FlowPilot's context for agent-CLI backends, which build their own prompt: the persona has
    /// to travel with it. The Bits path adds it to the system prompt itself.
    pub(super) fn with_persona(&self, context: String) -> String {
        match bot_persona_section(&self.context) {
            Some(persona) => format!("{context}\n\n{persona}"),
            None => context,
        }
    }
}

/// The run context of `record`. Bots that hold `flowpilot` (Flux and its duplicates) run
/// FlowPilot's toolset with their row's edits on top.
pub(super) fn desktop_bot(
    record: BotRecord,
    apps: Vec<BotAppRecord>,
    profile_app_ids: BTreeSet<String>,
    tool_host: BotToolHost,
) -> DesktopBot {
    let system = record.system();
    let general = system == Some(BotSystemKind::General);
    let text = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let capabilities = record.capabilities();
    let toolset = BotToolset::of(&capabilities);
    let name = text(record.name.clone()).unwrap_or_else(|| match system {
        Some(BotSystemKind::General) => GENERAL_BOT_NAME.to_string(),
        Some(BotSystemKind::Flux) => FLUX_BOT_NAME.to_string(),
        None => record.id.clone(),
    });
    let description = text(record.description.clone())
        .or_else(|| general.then(|| GENERAL_BOT_DESCRIPTION.to_string()));
    let context = BotRunContext {
        bot_id: record.id,
        system,
        toolset,
        name,
        description,
        instructions: text(record.instructions),
        capabilities,
        classification: record.classification,
        selected_app_ids: apps
            .into_iter()
            .filter(|app| app.owner_role.is_some())
            .map(|app| app.app_id)
            .collect(),
        profile_app_ids,
        workspace_app_id: None,
        embedding_model: record.embedding_model,
        stt_model: record.stt_model,
        tts_model: record.tts_model,
        image_model: record.image_model,
        vision_model: record.vision_model,
        tool_host,
        host_tools: super::super::super::native_bot_tools::host_tools(),
        requester: None,
        permissions: Default::default(),
    };
    DesktopBot {
        context: Arc::new(context),
        model: record.model,
    }
}

/// The bot `bot_id` names: `None` without one (today's FlowPilot run), Flux's row for `flux` (the
/// signed-in account's, else this device's when signed out; `None` without one), Assistant's
/// defaults for `general`, a bot on this device, else the signed-in account's bot. Another
/// person's or an unknown id fails. A bot other than Flux comes with its workspace, made on its
/// first run; where that fails the run goes on without one. `tool_host` is `Chat` with a webview
/// attached and `Native` without one.
pub(super) async fn load_bot(
    app: &AppHandle,
    bot_id: Option<&str>,
    identity: Option<&NativeRunIdentity>,
    active_profile: Option<&Profile>,
    tool_host: BotToolHost,
) -> Result<Option<DesktopBot>, String> {
    let signed_in = identity.and_then(|identity| identity.auth.as_ref().map(|_| identity));
    let bot_id = match bot_request(bot_id) {
        BotRequest::FlowPilot => return Ok(None),
        BotRequest::Flux => return Ok(flux_bot(app, signed_in, tool_host).await),
        BotRequest::General => return Ok(Some(DesktopBot::general(tool_host))),
        BotRequest::Bot(bot_id) => bot_id,
    };
    let (record, apps, workspace) = match local_bot(app, bot_id).await {
        Ok(Some((record, apps))) => {
            let workspace = device_workspace(app, &record).await;
            (record, apps, workspace)
        }
        Ok(None) => account_bot_with_workspace(signed_in, bot_id).await?,
        Err(error) if signed_in.is_some() => {
            tracing::warn!(%error, bot_id, "Bots on this device are unavailable; trying the account");
            account_bot_with_workspace(signed_in, bot_id).await?
        }
        Err(error) => return Err(format!("Bot '{bot_id}' can't be loaded: {error}")),
    };
    let profile_app_ids = if record.capabilities().contains(&BotCapability::ProfileApps) {
        profile_app_ids(app, record.profile_id.as_deref(), active_profile).await
    } else {
        BTreeSet::new()
    };
    Ok(Some(
        desktop_bot(record, apps, profile_app_ids, tool_host).with_workspace(workspace),
    ))
}

/// The workspace app of a bot on this device, made on its first run.
async fn device_workspace(app: &AppHandle, record: &BotRecord) -> Option<String> {
    if !wants_workspace(record) {
        return None;
    }
    let bots = app.try_state::<LocalBots>()?;
    match local_workspace(app, &bots, &record.id).await {
        Ok(app_id) => Some(app_id),
        Err(error) => {
            tracing::warn!(%error, bot_id = %record.id, "A local bot's workspace is unavailable; the run goes on without it");
            None
        }
    }
}

/// An account bot with its apps and the workspace the hub keeps for it.
async fn account_bot_with_workspace(
    identity: Option<&NativeRunIdentity>,
    bot_id: &str,
) -> Result<(BotRecord, Vec<BotAppRecord>, Option<String>), String> {
    let (record, apps) = account_bot(identity, bot_id).await?;
    let workspace = match identity {
        Some(identity) if wants_workspace(&record) => account_workspace(identity, bot_id).await,
        _ => None,
    };
    Ok((record, apps, workspace))
}

/// `POST /bots/{id}/workspace`, which makes the workspace on the first call and answers the same
/// app after. A hub without the route (404) or any refusal or failure means no workspace: it never
/// fails the turn.
async fn account_workspace(identity: &NativeRunIdentity, bot_id: &str) -> Option<String> {
    let path = format!("bots/{}/workspace", segment(bot_id));
    match hub_json::<WorkspaceRecord>(identity, Method::POST, &path).await {
        Ok(workspace) => workspace.and_then(WorkspaceRecord::app_id),
        Err(error) => {
            tracing::warn!(%error, bot_id, "The account bot's workspace is unavailable; the run goes on without it");
            None
        }
    }
}

async fn local_bot(
    app: &AppHandle,
    bot_id: &str,
) -> Result<Option<(BotRecord, Vec<BotAppRecord>)>, String> {
    let Some(store) = app.try_state::<LocalBots>() else {
        return Ok(None);
    };
    let Some(bot) = local_bots_get(store.clone(), bot_id.to_string())
        .await
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    let apps = local_bots_apps(store, bot_id.to_string())
        .await
        .map_err(|error| format!("the apps of bot '{bot_id}' can't be read: {error}"))?
        .into_iter()
        .map(|app| BotAppRecord {
            app_id: app.app_id,
            owner_role: app.owner_role,
        })
        .collect();
    Ok(Some((bot.into(), apps)))
}

fn not_yours(bot_id: &str) -> String {
    format!("Bot '{bot_id}' isn't one of yours")
}

/// Flux with its edits: the signed-in account's Flux row, signed out this device's. Without the
/// row, or while the hub can't answer, Flux runs as today's FlowPilot.
async fn flux_bot(
    app: &AppHandle,
    identity: Option<&NativeRunIdentity>,
    tool_host: BotToolHost,
) -> Option<DesktopBot> {
    let Some(identity) = identity else {
        return device_flux(app, tool_host).await;
    };
    match account_flux(identity).await {
        Ok(record) => {
            record.map(|record| desktop_bot(record, Vec::new(), BTreeSet::new(), tool_host))
        }
        Err(error) => {
            tracing::warn!(%error, "Flux's account row is unavailable; Flux runs without its edits");
            None
        }
    }
}

/// Flux's row on this device, edited or not; FlowPilot's own memory stays the bot's memory.
async fn device_flux(app: &AppHandle, tool_host: BotToolHost) -> Option<DesktopBot> {
    let bots = app.try_state::<LocalBots>()?;
    match local_built_in(bots, BotSystemKind::Flux).await {
        Ok(row) => row.map(|row| desktop_bot(row.into(), Vec::new(), BTreeSet::new(), tool_host)),
        Err(error) => {
            tracing::warn!(%error, "Flux's row on this device is unavailable; Flux runs without its edits");
            None
        }
    }
}

/// Flux's row id per signed-in account, so later turns read the row by id like any other bot
/// and its edits apply on the next turn.
static FLUX_ROW_IDS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);

fn flux_session(identity: &NativeRunIdentity) -> Option<String> {
    let auth = identity.auth.as_ref()?;
    Some(format!("{}\n{}\n{}", auth.hub, auth.issuer, auth.subject))
}

fn remembered_flux(session: &str) -> Option<String> {
    FLUX_ROW_IDS.lock().ok()?.get(session).cloned()
}

fn remembered_as_flux(bot_id: &str) -> bool {
    FLUX_ROW_IDS
        .lock()
        .is_ok_and(|ids| ids.values().any(|id| id == bot_id))
}

fn remember_flux(session: String, bot_id: Option<String>) {
    let Ok(mut ids) = FLUX_ROW_IDS.lock() else {
        return;
    };
    match bot_id {
        Some(bot_id) => ids.insert(session, bot_id),
        None => ids.remove(&session),
    };
}

fn pick_flux(bots: Vec<BotRecord>) -> Option<BotRecord> {
    bots.into_iter()
        .find(|bot| bot.system() == Some(BotSystemKind::Flux))
}

/// The signed-in account's Flux row: by its remembered id, else from `GET /bots`, which also
/// sets Flux up for an account that has no row yet.
async fn account_flux(identity: &NativeRunIdentity) -> Result<Option<BotRecord>, String> {
    let session = flux_session(identity).ok_or("the run has no account session")?;
    if let Some(bot_id) = remembered_flux(&session) {
        let row: Option<BotRecord> =
            hub_get(identity, &format!("bots/{}", segment(&bot_id))).await?;
        if let Some(row) = row.filter(|row| row.system() == Some(BotSystemKind::Flux)) {
            return Ok(Some(row));
        }
    }
    let list: BotRecordList = hub_get(identity, "bots").await?.unwrap_or_default();
    let flux = pick_flux(list.bots);
    remember_flux(session, flux.as_ref().map(|row| row.id.clone()));
    Ok(flux)
}

/// The signed-in account's bot `bot_id`; signed out, another person's or an unknown id fails.
async fn account_record<'a>(
    identity: Option<&'a NativeRunIdentity>,
    bot_id: &str,
) -> Result<(&'a NativeRunIdentity, BotRecord), String> {
    let Some(identity) = identity else {
        return Err(format!(
            "{} on this device. Sign in to use the bots of your account.",
            not_yours(bot_id)
        ));
    };
    let record = hub_get(identity, &format!("bots/{}", segment(bot_id)))
        .await
        .map_err(|error| format!("Bot '{bot_id}' can't be loaded from your account: {error}"))?
        .ok_or_else(|| not_yours(bot_id))?;
    Ok((identity, record))
}

async fn account_bot(
    identity: Option<&NativeRunIdentity>,
    bot_id: &str,
) -> Result<(BotRecord, Vec<BotAppRecord>), String> {
    let (identity, record) = account_record(identity, bot_id).await?;
    if !record.capabilities().contains(&BotCapability::SelectedApps) {
        return Ok((record, Vec::new()));
    }
    let apps: BotAppList = hub_get(identity, &format!("bots/{}/apps", segment(bot_id)))
        .await
        .map_err(|error| format!("The apps of bot '{bot_id}' can't be loaded: {error}"))?
        .ok_or_else(|| not_yours(bot_id))?;
    Ok((record, apps.apps))
}

/// `GET /api/v1/{path}` on the run's hub with its sign-in; `None` for 404.
async fn hub_get<T: serde::de::DeserializeOwned>(
    identity: &NativeRunIdentity,
    path: &str,
) -> Result<Option<T>, String> {
    hub_json(identity, Method::GET, path).await
}

/// `method /api/v1/{path}` on the run's hub with its sign-in, without a body; `None` for 404.
async fn hub_json<T: serde::de::DeserializeOwned>(
    identity: &NativeRunIdentity,
    method: Method,
    path: &str,
) -> Result<Option<T>, String> {
    let auth = identity
        .auth
        .as_ref()
        .ok_or("the run has no account session")?;
    let url = format!("{}/api/v1/{path}", identity.hub.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "the hub client could not be created")?;
    let mut token = auth.cached_access_token().await?;
    for attempt in 0..2 {
        let response = client
            .request(method.clone(), &url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|_| "the hub could not be reached")?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
            token = auth.access_token(Some(&token)).await?;
            continue;
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(format!("the hub answered HTTP {}", status.as_u16()));
        }
        let body = response
            .bytes()
            .await
            .map_err(|_| "the hub response was interrupted")?;
        if body.len() > MAX_BOT_RESPONSE_BYTES {
            return Err("the hub response is too large".into());
        }
        return serde_json::from_slice(&body)
            .map(Some)
            .map_err(|_| "the hub returned an unexpected answer".into());
    }
    Err("the hub rejected the refreshed session".into())
}

/// Apps of the bot's own profile, or of the active profile when the bot names none. A profile
/// that isn't on this device contributes no apps.
async fn profile_app_ids(
    app: &AppHandle,
    bot_profile_id: Option<&str>,
    active_profile: Option<&Profile>,
) -> BTreeSet<String> {
    let app_ids = |profile: &Profile| -> BTreeSet<String> {
        profile
            .apps
            .iter()
            .flatten()
            .map(|entry| entry.app_id.clone())
            .collect()
    };
    match (bot_profile_id, active_profile) {
        (None, Some(active)) => app_ids(active),
        (Some(profile_id), Some(active)) if active.id == profile_id => app_ids(active),
        (Some(profile_id), _) => copilot_profile_for(app, profile_id)
            .await
            .map(|profile| app_ids(&profile))
            .unwrap_or_default(),
        (None, None) => BTreeSet::new(),
    }
}

/// The chat's own model choice, when it is a Bits model; agent CLIs never run bots.
pub(super) fn chat_bits_model(selection: &FlowPilotModelSelection) -> Option<String> {
    match selection.backend {
        FlowPilotChatBackend::Bits => selection
            .model_id
            .clone()
            .filter(|model_id| !model_id.trim().is_empty()),
        FlowPilotChatBackend::Agent(_) => None,
    }
}

/// The context an agent-CLI backend runs with: Flux's persona travels in it, since those
/// backends build their own prompt. Only FlowPilot runs reach those backends.
pub(super) fn agent_context(context: String, bot: Option<&DesktopBot>) -> String {
    match bot {
        Some(bot) => bot.with_persona(context),
        None => context,
    }
}

/// The bot's own model on this desktop: its primary or first runnable fallback.
pub(super) async fn own_model(
    state: &Arc<FlowLikeState>,
    profile: &Arc<Profile>,
    bot: &DesktopBot,
) -> Option<String> {
    resolve_bot_model(&bot.model, |model_id| {
        let (state, profile) = (state.clone(), profile.clone());
        async move {
            runnable_completion_model(&state, &profile, &model_id)
                .await
                .is_some()
        }
    })
    .await
}

/// The model a capabilities bot runs on here: its own, else the chat's Bits model, else the
/// profile's best runnable language model.
pub(super) async fn bot_model(
    state: &Arc<FlowLikeState>,
    profile: &Arc<Profile>,
    bot: &DesktopBot,
    chat_model: Option<String>,
) -> Result<String, String> {
    if let Some(model_id) = own_model(state, profile, bot).await.or(chat_model) {
        return Ok(model_id);
    }
    resolve_host_completion_model(state, profile, None)
        .await
        .map(|bit| bit.id)
        .map_err(|error| {
            format!(
                "Bot '{}' can't answer: no language model it can use runs on this desktop ({error}). Add one in Settings → Models or pick one in the bot's settings.",
                bot.context.name
            )
        })
}

/// Which shared spec set authorizes a bot tool on the desktop; every set resolves the same spec
/// as [`flow_like::flow::copilot::find_bot_tool_spec`].
pub(super) fn bot_tool_set(tool_name: &str) -> FrontendPlatformToolSet {
    match tool_name {
        "get_app_detail" => FrontendPlatformToolSet::Scout,
        "database_tool" | "graph_query_tool" => FrontendPlatformToolSet::DataStudio,
        "storage_tool" => FrontendPlatformToolSet::BoardRuntime,
        _ if super::super::super::native_bot_tools::supports(tool_name) => {
            FrontendPlatformToolSet::BotTools
        }
        _ => FrontendPlatformToolSet::Global,
    }
}

/// The global chat's frontend bridge for a bot run. Core has already checked each call against
/// the bot's limits; this resolves its approval and timeout and dispatches it like FlowPilot's.
pub(super) struct BotDesktopBridge(pub(super) DesktopPlatformBridge);

#[async_trait]
impl PlatformToolBridge for BotDesktopBridge {
    async fn call(&self, tool_name: &str, arguments: serde_json::Value) -> String {
        DesktopPlatformBridge {
            bridge: self.0.bridge.clone(),
            tool_set: bot_tool_set(tool_name),
            cancellation: self.0.cancellation.clone(),
            steerable: false,
        }
        .call(tool_name, arguments)
        .await
    }

    async fn drain_steering(&self) -> Vec<String> {
        self.0.drain_steering().await
    }

    async fn is_cancelled(&self) -> bool {
        self.0.is_cancelled().await
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::platform_bridge::frontend_platform_tool_spec;
    use super::*;
    use flow_like::flow::copilot::find_bot_tool_spec;
    use flow_like_types::data_classification::DataClassification;
    use serde_json::json;

    fn record(value: serde_json::Value) -> BotRecord {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn memory_requests_never_cross_between_flowpilot_and_bots() {
        assert_eq!(
            memory_scope("profile-1".into(), None),
            MemoryScope::Profile("profile-1".into())
        );
        assert_eq!(
            memory_scope("profile-1".into(), Some("flux")),
            MemoryScope::Profile("profile-1".into())
        );
        assert_eq!(
            memory_scope("profile-1".into(), Some("general")),
            MemoryScope::Bot("general".into())
        );
        assert_eq!(
            memory_scope("profile-1".into(), Some(" bot_a ")),
            MemoryScope::Bot("bot_a".into())
        );
        assert_ne!(
            memory_scope("profile-1".into(), Some("bot_a")),
            memory_scope("profile-1".into(), Some("bot_b"))
        );
    }

    /// The chat runs Flux on FlowPilot's memory of the profile, so its row id must reach that
    /// store too, not an empty one of its own.
    #[test]
    fn flux_s_row_id_reaches_flowpilot_s_memory() {
        let session = "memory-scope-test-session".to_string();
        remember_flux(session.clone(), Some("bot_flux_row".into()));
        assert_eq!(
            memory_scope("profile-1".into(), Some(" bot_flux_row ")),
            MemoryScope::Profile("profile-1".into())
        );
        assert_eq!(
            memory_scope("profile-1".into(), Some("bot_scout_row")),
            MemoryScope::Bot("bot_scout_row".into())
        );
        remember_flux(session, None);
    }

    #[test]
    fn requests_name_flowpilot_flux_assistant_or_a_bot() {
        assert_eq!(bot_request(None), BotRequest::FlowPilot);
        assert_eq!(bot_request(Some("  ")), BotRequest::FlowPilot);
        assert_eq!(bot_request(Some("flux")), BotRequest::Flux);
        assert_eq!(bot_request(Some(" flux ")), BotRequest::Flux);
        assert_eq!(bot_request(Some("general")), BotRequest::General);
        assert_eq!(bot_request(Some(" bot_abc ")), BotRequest::Bot("bot_abc"));
        assert_eq!(bot_request(Some("bot_flux")), BotRequest::Bot("bot_flux"));
    }

    #[test]
    fn flux_and_its_duplicates_run_flowpilot_because_they_hold_it() {
        let run = |value: serde_json::Value| {
            desktop_bot(
                record(value),
                Vec::new(),
                BTreeSet::new(),
                BotToolHost::Chat,
            )
        };
        let early_flux = run(json!({ "id": "bot_flux", "system": "flux", "capabilities": [] }));
        assert!(early_flux.runs_flowpilot());
        assert_eq!(early_flux.context.capabilities, [BotCapability::Flowpilot]);
        assert_eq!(
            early_flux.context.memory_scope("profile-1"),
            MemoryScope::Profile("profile-1".into())
        );

        let duplicate = run(json!({
            "id": "bot_copy",
            "system": null,
            "name": "Flux copy",
            "capabilities": ["flowpilot"]
        }));
        assert!(duplicate.runs_flowpilot());
        assert_eq!(duplicate.context.name, "Flux copy");
        assert_eq!(
            duplicate.context.memory_scope("profile-1"),
            MemoryScope::Bot("bot_copy".into())
        );

        let scout =
            run(json!({ "id": "bot_scout", "name": "Scout", "capabilities": ["internet"] }));
        assert!(!scout.runs_flowpilot());
        assert_eq!(scout.context.capabilities, [BotCapability::Internet]);
    }

    #[test]
    fn flux_comes_from_the_account_list_and_its_id_is_remembered_per_session() {
        let list: BotRecordList = serde_json::from_value(json!({
            "bots": [
                { "id": "bot_general", "system": "general", "capabilities": ["internet"] },
                { "id": "bot_flux", "system": "flux", "name": "Fluxy", "capabilities": ["flowpilot"] },
                { "id": "bot_scout", "system": null, "name": "Scout" }
            ]
        }))
        .unwrap();
        let flux = pick_flux(list.bots).unwrap();
        assert_eq!(
            (flux.id.as_str(), flux.name.as_deref()),
            ("bot_flux", Some("Fluxy"))
        );
        assert!(pick_flux(vec![record(json!({ "id": "bot_scout" }))]).is_none());

        let signed_out = NativeRunIdentity {
            profile_id: "profile-1".into(),
            hub: "https://hub.test".into(),
            subject: None,
            auth: None,
        };
        assert_eq!(flux_session(&signed_out), None);
        let signed_in = NativeRunIdentity {
            auth: Some(crate::native_auth::SessionBinding::for_test(
                "profile-1",
                "https://hub.test",
                "https://issuer.test",
                "flux-session-test-user",
            )),
            ..signed_out
        };
        let session = flux_session(&signed_in).unwrap();
        assert!(session.contains("flux-session-test-user"), "{session}");
        let other = format!("{session}-other");

        remember_flux(session.clone(), Some("bot_flux".into()));
        assert_eq!(remembered_flux(&session).as_deref(), Some("bot_flux"));
        assert_eq!(remembered_flux(&other), None);
        remember_flux(session.clone(), None);
        assert_eq!(remembered_flux(&session), None);
    }

    #[test]
    fn an_account_bot_maps_to_its_run_context() {
        let bot = desktop_bot(
            record(json!({
                "id": "bot_scout",
                "system": null,
                "name": " Scout ",
                "description": "Finds things",
                "instructions": "",
                "model": { "primary": "model-a", "fallbacks": ["model-b"] },
                "embedding_model": "embed-a",
                "stt_model": "stt-a",
                "tts_model": "tts-a",
                "image_model": "image-a",
                "vision_model": "vision-a",
                "classification": { "classification": "confidential", "pii": true },
                "capabilities": ["internet", "selected_apps", "profile_apps", "telepathy"],
                "profile_id": "profile-1",
                "appearance": { "kind": "pip" },
                "location": "server"
            })),
            vec![
                BotAppRecord {
                    app_id: "app-crm".into(),
                    owner_role: Some("owner".into()),
                },
                BotAppRecord {
                    app_id: "app-left".into(),
                    owner_role: None,
                },
            ],
            ["app-notes".to_string()].into(),
            BotToolHost::Chat,
        );

        let context = &bot.context;
        assert_eq!(context.bot_id, "bot_scout");
        assert_eq!(context.toolset, BotToolset::Capabilities);
        assert_eq!(context.name, "Scout");
        assert_eq!(context.description.as_deref(), Some("Finds things"));
        assert_eq!(context.instructions, None);
        assert_eq!(
            context.capabilities,
            [
                BotCapability::Internet,
                BotCapability::SelectedApps,
                BotCapability::ProfileApps
            ]
        );
        assert_eq!(
            context.classification.classification,
            Some(DataClassification::Confidential)
        );
        assert!(context.classification.pii);
        assert!(context.allows_app("app-crm"));
        assert!(context.allows_app("app-notes"));
        assert!(!context.allows_app("app-left"));
        assert_eq!(context.workspace_app_id, None);
        assert_eq!(bot.model.primary.as_deref(), Some("model-a"));
        assert_eq!(bot.model.fallbacks, ["model-b"]);
        assert_eq!(context.embedding_model.as_deref(), Some("embed-a"));
        assert_eq!(context.stt_model.as_deref(), Some("stt-a"));
        assert_eq!(context.tts_model.as_deref(), Some("tts-a"));
        assert_eq!(context.image_model.as_deref(), Some("image-a"));
        assert_eq!(context.vision_model.as_deref(), Some("vision-a"));
        assert_eq!(
            context.memory_model(Some("embed-chat")),
            None,
            "without `memory` the bot opens no memory"
        );
    }

    #[test]
    fn built_in_bots_keep_their_toolsets_and_defaults() {
        let flux = desktop_bot(
            record(json!({
                "id": "bot_flux",
                "system": "flux",
                "name": null,
                "instructions": "Always add tests.",
                "model": { "primary": "model-a", "fallbacks": [] }
            })),
            Vec::new(),
            BTreeSet::new(),
            BotToolHost::Chat,
        );
        assert!(flux.runs_flowpilot());
        assert_eq!(flux.context.system, Some(BotSystemKind::Flux));
        assert_eq!(flux.context.name, FLUX_BOT_NAME);
        assert_eq!(flux.model.primary.as_deref(), Some("model-a"));
        assert_eq!(
            flux.context.memory_scope("profile-1"),
            MemoryScope::Profile("profile-1".into())
        );
        let context = flux.with_persona("## CONTEXT".into());
        assert!(context.starts_with("## CONTEXT\n\n"));
        assert!(context.contains("Always add tests."));

        let assistant = desktop_bot(
            record(
                json!({ "id": "bot_general", "system": "general", "name": null, "capabilities": ["internet"] }),
            ),
            Vec::new(),
            BTreeSet::new(),
            BotToolHost::Chat,
        );
        assert!(!assistant.runs_flowpilot());
        assert_eq!(assistant.context.bot_id, "bot_general");
        assert_eq!(assistant.context.name, GENERAL_BOT_NAME);
        assert_eq!(
            assistant.context.description.as_deref(),
            Some(GENERAL_BOT_DESCRIPTION)
        );

        let defaults = DesktopBot::general(BotToolHost::Chat);
        assert_eq!(defaults.context.bot_id, GENERAL_BOT_ID);
        assert_eq!(
            defaults.context.capabilities,
            [BotCapability::Internet, BotCapability::Memory]
        );
    }

    #[test]
    fn a_run_without_a_webview_offers_only_native_tools() {
        let scout = || {
            record(json!({
                "id": "bot_scout",
                "name": "Scout",
                "capabilities": ["selected_apps", "storage", "data_studio"]
            }))
        };
        let apps = || {
            vec![BotAppRecord {
                app_id: "app-crm".into(),
                owner_role: Some("owner".into()),
            }]
        };
        let offered = |tool_host| -> Vec<&'static str> {
            desktop_bot(scout(), apps(), BTreeSet::new(), tool_host)
                .context
                .tool_specs(false)
                .iter()
                .map(|spec| spec.name)
                .collect()
        };
        let chat = offered(BotToolHost::Chat);
        let headless = offered(BotToolHost::Native);
        for chat_only in [
            "ask_user",
            "open_app_page",
            "open_app_chat",
            "interact_app_page",
        ] {
            assert!(chat.contains(&chat_only));
            assert!(!headless.contains(&chat_only), "{chat_only}");
        }
        for tool_name in &headless {
            assert!(
                crate::functions::ai::native_tool_bridge::supports(
                    tool_name,
                    &json!({ "operation": "list_tables" })
                ),
                "{tool_name} has no native implementation"
            );
        }
        assert_eq!(
            DesktopBot::general(BotToolHost::Native).context.tool_host,
            BotToolHost::Native
        );
    }

    /// The hub hands Flux's room turns to this desktop because it builds without a window; every
    /// tool such a turn is offered must run here, the build and specialist tools among them.
    #[test]
    fn flux_without_a_window_is_offered_only_tools_this_desktop_runs() {
        let flux = desktop_bot(
            record(json!({ "id": "bot_flux", "system": "flux", "capabilities": [] })),
            Vec::new(),
            BTreeSet::new(),
            BotToolHost::Native,
        );
        let offered: Vec<&str> = flux
            .context
            .tool_specs(false)
            .iter()
            .map(|spec| spec.name)
            .collect();
        for builds in ["create_app", "flowpilot_board", "flowpilot_widget", "upsert_event"] {
            assert!(offered.contains(&builds), "{builds} missing from {offered:?}");
        }
        let core_runs = flow_like::flow::copilot::tool_spec::RESEARCH_AGENT_TOOL;
        for tool_name in offered.iter().filter(|name| **name != core_runs) {
            assert!(
                crate::functions::ai::native_tool_bridge::supports(tool_name, &json!({})),
                "{tool_name} has no native implementation"
            );
        }
    }

    #[test]
    fn a_local_bot_maps_like_an_account_bot() {
        let local: flow_like_local_bots::Bot = flow_like_local_bots::Bot {
            id: "bot_local".into(),
            owner_id: flow_like_local_bots::LOCAL_OWNER_ID.into(),
            system: None,
            preset_id: None,
            name: Some("Clerk".into()),
            description: None,
            instructions: Some("Be brief.".into()),
            appearance: json!({ "kind": "pip" }),
            model: BotModelPolicy::default(),
            embedding_model: None,
            stt_model: None,
            tts_model: None,
            image_model: None,
            vision_model: None,
            classification: DataSensitivity::default(),
            capabilities: vec![BotCapability::SelectedApps, BotCapability::Storage],
            profile_id: None,
            location: flow_like_types::bot::BotLocation::Desktop,
            device_id: None,
            workspace_app_id: None,
            created_at: chrono::Utc::now().fixed_offset(),
            updated_at: chrono::Utc::now().fixed_offset(),
            local: true,
        };
        let bot = desktop_bot(
            local.into(),
            vec![BotAppRecord {
                app_id: "app-crm".into(),
                owner_role: Some("owner".into()),
            }],
            BTreeSet::new(),
            BotToolHost::Chat,
        );
        assert_eq!(bot.context.name, "Clerk");
        assert_eq!(bot.context.instructions.as_deref(), Some("Be brief."));
        assert_eq!(
            bot.context.capabilities,
            [BotCapability::SelectedApps, BotCapability::Storage]
        );
        assert!(bot.context.allows_app("app-crm"));
    }

    #[test]
    fn a_bots_workspace_is_its_own_app_and_its_run_may_use_it_until_the_run_ends() {
        use crate::functions::ai::{
            frontend_tool_bridge::FrontendToolContext, native_platform_tools::visible_to_run,
        };

        let scout = || {
            desktop_bot(
                record(json!({ "id": "bot_scout", "name": "Scout", "capabilities": ["storage"] })),
                Vec::new(),
                BTreeSet::new(),
                BotToolHost::Chat,
            )
        };
        let run = FrontendToolContext {
            run_id: Some("bot-chat-workspace-run".into()),
            ..Default::default()
        };
        let profile = Profile::default();

        let without = scout();
        assert_eq!(without.context.workspace_app_id, None);
        assert!(without.admit_workspace("bot-chat-workspace-run").is_none());
        assert!(!without.context.allows_app("app-workspace"));

        let bot = scout().with_workspace(Some("app-workspace".into()));
        assert_eq!(
            bot.context.workspace_app_id.as_deref(),
            Some("app-workspace")
        );
        assert!(bot.context.allows_app("app-workspace"));
        assert!(!bot.context.allows_app("app-other"));
        assert!(!visible_to_run(Some(&run), &profile, "app-workspace"));

        let admitted = bot.admit_workspace("bot-chat-workspace-run");
        assert!(visible_to_run(Some(&run), &profile, "app-workspace"));
        assert!(!visible_to_run(Some(&run), &profile, "app-other"));
        drop(admitted);
        assert!(!visible_to_run(Some(&run), &profile, "app-workspace"));
    }

    #[test]
    fn flux_has_no_workspace_and_the_hub_answers_one_app() {
        assert!(!wants_workspace(&record(
            json!({ "id": "bot_flux", "system": "flux" })
        )));
        for bot in [
            json!({ "id": "bot_general", "system": "general" }),
            json!({ "id": "bot_scout", "system": null }),
            json!({ "id": "bot_copy", "capabilities": ["flowpilot"] }),
        ] {
            assert!(wants_workspace(&record(bot.clone())), "{bot}");
        }

        let app_id = |answer: serde_json::Value| {
            serde_json::from_value::<WorkspaceRecord>(answer)
                .unwrap()
                .app_id()
        };
        assert_eq!(
            app_id(json!({ "app_id": "app-ws" })).as_deref(),
            Some("app-ws")
        );
        assert_eq!(app_id(json!({})), None);
        assert_eq!(app_id(json!({ "app_id": " " })), None);
    }

    #[test]
    fn only_a_bits_chat_model_backs_a_bot() {
        assert_eq!(
            chat_bits_model(&FlowPilotModelSelection::parse(Some("bit-1".into()))),
            Some("bit-1".into())
        );
        assert_eq!(
            chat_bits_model(&FlowPilotModelSelection::parse(Some("claude:opus".into()))),
            None
        );
        assert_eq!(chat_bits_model(&FlowPilotModelSelection::parse(None)), None);
    }

    #[test]
    fn every_bot_tool_resolves_the_spec_core_names() {
        for tool_name in [
            "ask_user",
            "list_apps",
            "describe_app_interface",
            "get_app_detail",
            "open_app_page",
            "open_app_chat",
            "call_app_event",
            "call_app_chat",
            "interact_app_page",
            "database_tool",
            "graph_query_tool",
            "storage_tool",
        ] {
            let desktop = frontend_platform_tool_spec(bot_tool_set(tool_name), tool_name)
                .unwrap_or_else(|| panic!("the desktop can't resolve {tool_name}"));
            let core = find_bot_tool_spec(tool_name).unwrap();
            assert_eq!(desktop.name, core.name);
            assert_eq!(desktop.timeout_secs, core.timeout_secs);
            assert_eq!(desktop.approval.effect(), core.approval.effect());
            assert_eq!((desktop.schema)(), (core.schema)(), "{tool_name}");
        }
        let insert = json!({ "operation": "insert", "app_id": "app" });
        let desktop =
            frontend_platform_tool_spec(bot_tool_set("database_tool"), "database_tool").unwrap();
        assert_eq!(
            flow_like::flow::copilot::tool_spec::resolve_tool_effect(&desktop, &insert),
            flow_like::flow::copilot::tool_spec::ToolEffect::Mutating
        );
    }
    #[test]
    fn bot_tools_resolve_their_core_specs_and_gated_calls_are_refused_in_headless_runs() {
        use crate::functions::ai::{
            copilot_sdk_tools::approval_from_spec,
            frontend_tool_bridge::FrontendToolContext,
            native_tool_bridge::{interaction_unavailable, unavailable_result},
        };
        for tool_name in flow_like::flow::copilot::linked_tool_names() {
            let desktop = frontend_platform_tool_spec(bot_tool_set(tool_name), tool_name)
                .unwrap_or_else(|| panic!("the desktop can't resolve {tool_name}"));
            let core = find_bot_tool_spec(tool_name).unwrap();
            assert_eq!(desktop.timeout_secs, core.timeout_secs, "{tool_name}");
        }
        let kind = |tool: &str, args: serde_json::Value| {
            let spec = frontend_platform_tool_spec(bot_tool_set(tool), tool).unwrap();
            approval_from_spec(&spec, &args).kind
        };
        assert_eq!(kind("file_read", json!({ "file": "bot/a.md" })), "none");
        assert_eq!(
            kind("knowledge_drop", json!({ "index": "pets" })),
            "mutating"
        );
        assert_eq!(
            kind("code_run", json!({ "language": "python", "code": "1" })),
            "none"
        );
        assert_eq!(
            kind(
                "code_run",
                json!({ "language": "python", "code": "1", "network_hosts": ["example.com"] })
            ),
            "mutating"
        );

        let headless = FrontendToolContext {
            run_id: Some("headless-run-without-handler".into()),
            ..Default::default()
        };
        assert!(interaction_unavailable(Some(&headless)));
        let refusal = unavailable_result("knowledge_drop");
        assert_eq!(refusal["approved"], false);
        assert_eq!(refusal["status"], "unavailable");
    }
}
