//! Runs a device-hosted turn: the claim's run spec becomes a `BotRunContext`, a profile that
//! holds the hub-resolved Bits, a tool bridge over this device's [`DeviceToolEnv`] and the
//! core bot run. Models are reached through the hub with the turn's executor token.

use super::{
    backends::{BotRoutes, HubMail, HubNotifier, HubPeople, HubScheduler},
    env::{DeviceToolEnv, HubSecrets, MachineBackends, Machines, PresignedAccess},
    hub::{DeviceRunSpec, TurnCreds},
};
use async_trait::async_trait;
use flow_like::{
    bit::Bit,
    copilot::flowpilot_thinking,
    flow::copilot::{
        BotRunContext, BotToolHost, BotToolset, ChatMessage, HostTools, PlatformContextInput,
        bot_platform_context,
        memory_open::{MemoryModels, open_headless_memory},
        platform::{PlatformToolBridge, RoomUpdatesBridge},
        run_bot_chat_with_talk,
    },
    models::llm::ModelUsageContext,
    profile::{Profile, ProfileCustomBit},
    state::FlowLikeState,
};
use flow_like_bot_host::{
    bridge::{EnvFactory, ToolEnvBridge},
    host::{RunError, TurnJob, TurnRunner},
    hub::TurnHub,
};
use flow_like_bot_tools::MemorySessions;
use flow_like_bot_tools_host::{HostEnv, JobApp, JobScope, StateModels};
use flow_like_types::{
    bot::BotCapability, data_classification::DataClassification, device_bot::DeviceTurnGrant,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;

pub const CODE_RUN_FAILED: &str = "BOT_TURN_FAILED";
pub const CODE_SETUP: &str = "BOT_TURN_SETUP";

const TOOLS_NEEDING_UNLINKED_SERVICES: [&str; 6] = [
    "skill_load",
    "skill_propose",
    "connector_tools",
    "connector_call",
    "connector_request",
    "image_generate",
];

const IDLE_SESSION: Duration = Duration::from_secs(60 * 60);

const OWNER_ONLY_PEOPLE_TOOLS: [&str; 2] = ["list_people", "message_person"];

struct RunControl {
    stopped: Arc<AtomicBool>,
    wake: Notify,
}

pub struct DeviceRunner {
    state: Arc<FlowLikeState>,
    base_url: String,
    machines: Option<Arc<dyn Machines>>,
    running: Mutex<HashMap<String, Arc<RunControl>>>,
    sessions: Mutex<HashMap<String, (Arc<MemorySessions>, Instant)>>,
}

fn setup(message: impl Into<String>) -> RunError {
    RunError::Failed {
        code: CODE_SETUP.into(),
        message: message.into(),
    }
}

fn bit_of(value: &serde_json::Value, role: &str) -> Result<Bit, RunError> {
    serde_json::from_value(value.clone())
        .map_err(|error| setup(format!("The hub sent an unreadable {role} model: {error}")))
}

fn profile_of(grant: &DeviceTurnGrant) -> Result<(Profile, Bit, Option<Bit>), RunError> {
    let completion = bit_of(&grant.models.completion, "completion")?;
    let embedding = grant
        .models
        .embedding
        .as_ref()
        .map(|bit| bit_of(bit, "embedding"))
        .transpose()?;
    let mut bits = vec![completion.clone()];
    for fallback in &grant.models.fallbacks {
        bits.push(bit_of(fallback, "fallback")?);
    }
    bits.extend(embedding.clone());
    let profile = Profile {
        name: grant.bot.name.clone(),
        bits: bits.iter().map(|bit| bit.id.clone()).collect(),
        custom_bits: bits.into_iter().map(ProfileCustomBit).collect(),
        ..Profile::default()
    };
    Ok((profile, completion, embedding))
}

pub fn host_tools(machine: &MachineBackends, room_turn: bool) -> HostTools {
    let mut tools = HostTools::linked();
    if machine.browser.is_some() || machine.computer.is_some() {
        tools = tools.with_local_machine();
    }
    let people: &[&str] = if room_turn {
        &OWNER_ONLY_PEOPLE_TOOLS
    } else {
        &[]
    };
    TOOLS_NEEDING_UNLINKED_SERVICES
        .iter()
        .chain(people)
        .fold(tools, |tools, name| tools.without(name))
}

/// Files of the bot's selected and profile apps, read-only through presigned credentials; the
/// hub decides at presign what the bot holds. Their tables stay closed: column restrictions
/// cannot be enforced behind raw database credentials.
fn job_scope(grant: &DeviceTurnGrant, workspace: &str) -> JobScope {
    let spec = &grant.bot;
    let has = |capability| spec.capabilities.contains(&capability);
    let apps = spec
        .selected_app_ids
        .iter()
        .filter(|_| has(BotCapability::SelectedApps))
        .chain(
            spec.profile_app_ids
                .iter()
                .filter(|_| has(BotCapability::ProfileApps)),
        )
        .filter(|app_id| app_id.as_str() != workspace)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(|app_id| JobApp {
            app_id: app_id.clone(),
            alias: None,
            files: true,
            tables: false,
            column_attributes: None,
        })
        .collect();
    JobScope {
        storage: has(BotCapability::Storage),
        data_connections: has(BotCapability::DataConnections),
        classification: spec
            .classification
            .classification
            .unwrap_or(DataClassification::Internal),
        apps,
    }
}

fn context_of(grant: &DeviceTurnGrant, job: &TurnJob, tools: HostTools) -> BotRunContext {
    let spec = &grant.bot;
    let mut requester = job.requester.clone();
    if let (Some(requester), Some(from_hub)) = (requester.as_mut(), &grant.requester) {
        requester.room = from_hub.room.clone();
        requester.owner_name = from_hub.owner_name.clone();
        requester.requester_name = requester
            .requester_name
            .clone()
            .or_else(|| from_hub.requester_name.clone());
    }
    BotRunContext {
        bot_id: spec.bot_id.clone(),
        system: spec.system,
        toolset: BotToolset::of(&spec.capabilities),
        name: spec.name.clone(),
        description: spec.description.clone(),
        instructions: spec.instructions.clone(),
        capabilities: spec.capabilities.clone(),
        classification: spec.classification,
        selected_app_ids: spec.selected_app_ids.clone(),
        profile_app_ids: spec.profile_app_ids.clone(),
        workspace_app_id: spec.workspace_app_id.clone(),
        embedding_model: spec.embedding_model.clone(),
        stt_model: spec.stt_model.clone(),
        tts_model: spec.tts_model.clone(),
        image_model: spec.image_model.clone(),
        vision_model: spec.vision_model.clone(),
        subtask_model: spec.subtask_model.clone(),
        tool_host: BotToolHost::CoreOnly,
        host_tools: tools,
        skills: spec.skills.clone(),
        requester,
        permissions: Default::default(),
        connectors: spec.connectors.clone(),
        compute: Default::default(),
    }
}

impl DeviceRunner {
    pub fn new(
        state: Arc<FlowLikeState>,
        base_url: String,
        machines: Option<Arc<dyn Machines>>,
    ) -> Self {
        Self {
            state,
            base_url,
            machines,
            running: Mutex::default(),
            sessions: Mutex::default(),
        }
    }

    /// What the tools remember between calls (the browser's approved sites) lives per bot and
    /// conversation, so a later turn finds the page the browser is still on.
    fn sessions_for(&self, grant: &DeviceTurnGrant) -> Arc<MemorySessions> {
        let now = Instant::now();
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        sessions.retain(|_, (_, used)| now.duration_since(*used) < IDLE_SESSION);
        let key = format!("{}/{}", grant.bot.bot_id, grant.conversation.id);
        let entry = sessions
            .entry(key)
            .or_insert_with(|| (Arc::new(MemorySessions::default()), now));
        entry.1 = now;
        entry.0.clone()
    }

    fn machine_for(&self, grant: &DeviceTurnGrant) -> MachineBackends {
        let Some(machines) = &self.machines else {
            return MachineBackends::default();
        };
        let has = |capability| grant.bot.capabilities.contains(&capability);
        machines.machine(
            &grant.bot.bot_id,
            Some(&grant.conversation.id),
            has(BotCapability::BrowserUse),
            has(BotCapability::ComputerUse),
        )
    }

    fn env_factory(
        &self,
        grant: &DeviceTurnGrant,
        creds: &Arc<TurnCreds>,
        bot_hub: Arc<dyn TurnHub>,
        access: Arc<PresignedAccess>,
        completion: &Bit,
        embedding: &Option<Bit>,
        machine: MachineBackends,
    ) -> Result<EnvFactory, RunError> {
        let workspace = grant
            .bot
            .workspace_app_id
            .clone()
            .ok_or_else(|| setup("The bot has no workspace app yet"))?;
        let models = Arc::new(StateModels {
            state: self.state.clone(),
            chat: Some(completion.clone()),
            embedding: embedding.clone(),
            token: creds.executor_token(),
            usage: ModelUsageContext {
                app_id: Some(workspace.clone()),
                run_id: Some(grant.turn.id.clone()),
                api_base_url: None,
            },
        });
        let secrets = Arc::new(HubSecrets {
            hub: bot_hub.clone(),
            turn_id: grant.turn.id.clone(),
        });
        let scope = job_scope(grant, &workspace);
        let sessions = self.sessions_for(grant);
        let routes = BotRoutes {
            hub: bot_hub,
            bot_id: grant.bot.bot_id.clone(),
            conversation_id: Some(grant.conversation.id.clone()),
        };
        Ok(Arc::new(move |expensive_documents| {
            let env = HostEnv::new(&workspace, &scope, access.clone(), models.clone())
                .with_expensive_documents(expensive_documents)
                .with_secrets(secrets.clone())
                .with_sessions(sessions.clone())
                .with_notifier(Arc::new(HubNotifier(routes.clone())))
                .with_scheduler(Arc::new(HubScheduler(routes.clone())))
                .with_people(Arc::new(HubPeople(routes.clone())))
                .with_mail(Arc::new(HubMail(routes.clone())));
            Arc::new(DeviceToolEnv::new(env, machine.clone()))
        }))
    }

    fn track(&self, run_id: &str) -> Arc<RunControl> {
        let control = Arc::new(RunControl {
            stopped: Arc::new(AtomicBool::new(false)),
            wake: Notify::new(),
        });
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run_id.to_owned(), control.clone());
        control
    }

    fn untrack(&self, run_id: &str) {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(run_id);
    }
}

fn history_of(job: &TurnJob) -> Vec<ChatMessage> {
    job.history
        .iter()
        .map(|message| ChatMessage {
            role: message.role.clone(),
            content: message.content.clone(),
            images: message.images.clone(),
        })
        .collect()
}

#[async_trait]
impl TurnRunner for DeviceRunner {
    async fn run(&self, job: TurnJob) -> Result<String, RunError> {
        let spec = job
            .claim_as::<DeviceRunSpec>()
            .ok_or_else(|| setup("The claim carried no device run spec"))?;
        let grant = &spec.grant;
        let (profile, completion, embedding) = profile_of(grant)?;
        let profile = Arc::new(profile);
        let machine = self.machine_for(grant);
        let tools = host_tools(&machine, grant.conversation.kind != "direct");
        let mut context = context_of(grant, &job, tools);
        let access = Arc::new(PresignedAccess::new(
            reqwest::Client::new(),
            self.base_url.clone(),
            spec.creds.clone(),
        ));
        let factory = self.env_factory(
            grant,
            &spec.creds,
            spec.bot_hub.clone(),
            access.clone(),
            &completion,
            &embedding,
            machine,
        )?;
        let mutating = Arc::new(AtomicBool::new(false));
        let control = self.track(&job.run_id);
        let token = spec.creds.executor_token();
        let memory = self
            .open_memory(
                &job,
                &profile,
                &completion,
                &embedding,
                &mut context,
                &access,
                &token,
                &mutating,
            )
            .await;
        let mut history = history_of(&job);
        let mut prompt = job.prompt.clone();
        let mut extra = String::new();
        if let Some(memory) = &memory {
            history = memory.history();
            prompt = memory.prompt().to_string();
            extra = format!("\n\n{}", memory.context());
        }
        let platform = bot_platform_context(
            &context,
            PlatformContextInput {
                user_context: None,
                active_profile: None,
                switchable_profiles: &[],
                open_board: None,
                open_data_studio: None,
                attachments: &[],
            },
        );
        let bridge: Arc<dyn PlatformToolBridge> = Arc::new(ToolEnvBridge::new(
            factory,
            context.host_tools.clone(),
            job.gate.clone(),
            mutating.clone(),
            control.stopped.clone(),
        ));
        let bridge: Arc<dyn PlatformToolBridge> = match &job.room_updates {
            Some(updates) => Arc::new(RoomUpdatesBridge::new(bridge, updates.clone())),
            None => bridge,
        };
        let chat = run_bot_chat_with_talk(
            self.state.clone(),
            Some(profile),
            Arc::new(context),
            format!("{platform}{extra}"),
            prompt,
            None,
            history,
            Some(grant.models.model_id.clone()),
            Some(flowpilot_thinking(None)),
            token,
            bridge,
            None,
            None::<fn(String)>,
            None,
        );
        let outcome = tokio::select! {
            outcome = chat => Some(outcome),
            () = control.wake.notified() => None,
        };
        self.untrack(&job.run_id);
        match outcome {
            Some(Ok(text)) => Ok(text),
            Some(Err(error)) if mutating.load(Ordering::SeqCst) => Err(RunError::Failed {
                code: CODE_RUN_FAILED.into(),
                message: format!("{error:#}"),
            }),
            Some(Err(error)) => Err(RunError::Retry {
                code: CODE_RUN_FAILED.into(),
                message: format!("{error:#}"),
            }),
            None => Err(RunError::Failed {
                code: flow_like_bot_host::host::CODE_CANCELLED.into(),
                message: "The turn was stopped".into(),
            }),
        }
    }

    fn cancel(&self, run_id: &str) {
        let control = self
            .running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(run_id)
            .cloned();
        if let Some(control) = control {
            control.stopped.store(true, Ordering::SeqCst);
            control.wake.notify_one();
        }
    }
}

impl DeviceRunner {
    #[allow(clippy::too_many_arguments)]
    async fn open_memory(
        &self,
        job: &TurnJob,
        profile: &Arc<Profile>,
        completion: &Bit,
        embedding: &Option<Bit>,
        context: &mut BotRunContext,
        access: &Arc<PresignedAccess>,
        token: &Option<String>,
        mutating: &Arc<AtomicBool>,
    ) -> Option<flow_like::flow::copilot::HeadlessMemory> {
        let source = job.memory.clone()?.0;
        let app_id = context.workspace_app_id.clone()?;
        let connection = flow_like_bot_tools_host::AppAccess::connection(access.as_ref(), &app_id)
            .await
            .map_err(
                |error| tracing::warn!(%error, "Bot memory: the workspace database is unreachable"),
            )
            .ok()?;
        let models = MemoryModels {
            state: &self.state,
            profile,
            completion,
            chat_embedding: embedding.as_ref().map(|bit| bit.id.as_str()),
            token: token.clone(),
            run_id: Some(&job.run_id),
            connection: Some(connection),
        };
        let memory = open_headless_memory(&models, context, source)
            .await?
            .with_write_flag(mutating.clone());
        context.host_tools = memory.host_tools(std::mem::take(&mut context.host_tools));
        Some(memory)
    }
}
