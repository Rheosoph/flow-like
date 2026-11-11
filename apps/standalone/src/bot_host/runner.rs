//! Runs a device-hosted turn: the claim's run spec becomes a `BotRunContext`, a profile that
//! holds the hub-resolved Bits, a tool bridge over this device's [`DeviceToolEnv`] and the
//! core bot run. Models are reached through the hub with the turn's executor token.

use super::{
    backends::{
        BotRoutes, HubConnectors, HubMail, HubNotifier, HubPeople, HubScheduler, HubSkills,
        HubUserContext,
    },
    env::{
        ConversationFiles, DeviceToolEnv, HubImages, HubSecrets, MachineBackends, Machines,
        PresignedAccess,
    },
    hub::{DeviceRunSpec, TurnCreds},
};
use async_trait::async_trait;
use flow_like::{
    bit::Bit,
    flow::copilot::{
        BotRunContext, BotToolHost, BotToolset, ChatMessage, HostTools, PlatformContextInput,
        bot_platform_context, bot_thinking,
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
use flow_like_bot_tools::{LocalShell, MemorySessions, SandboxBackend, code::CatalogSandbox};
use flow_like_bot_tools_host::{HostEnv, JobApp, JobScope, StateModels};
use flow_like_types::{
    bot::BotCapability, data_classification::DataClassification, device_bot::DeviceTurnGrant,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::Notify;

pub const CODE_RUN_FAILED: &str = "BOT_TURN_FAILED";
pub const CODE_SETUP: &str = "BOT_TURN_SETUP";

const TOOLS_NEEDING_UNLINKED_SERVICES: [&str; 7] = [
    // Device runtime tokens have no endpoint for an approved settings change or for managing
    // the owner's bots.
    "bot_configuration",
    "bot_models",
    "bot_update_configuration",
    "list_bots",
    "get_bot",
    "create_bot",
    "update_bot",
];

const IMAGE_GENERATE: &str = "image_generate";

const IDLE_SESSION: Duration = Duration::from_secs(60 * 60);

const SESSION_POLL: Duration = Duration::from_secs(15);

struct RunControl {
    stopped: Arc<AtomicBool>,
    wake: Notify,
}

pub struct DeviceRunner {
    state: Arc<FlowLikeState>,
    base_url: String,
    machines: Option<Arc<dyn Machines>>,
    shell_root: PathBuf,
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

async fn saved_reply(
    recovery: Option<&Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>>,
    delivery_only: bool,
) -> Result<Option<String>, RunError> {
    let answer = match recovery {
        Some(recovery) => recovery
            .load()
            .await
            .map_err(|error| setup(error.to_string()))?
            .as_ref()
            .map(flow_like::flow::copilot::run_recovery::finished_reply)
            .transpose()
            .map_err(|error| setup(error.to_string()))?
            .flatten(),
        None => None,
    };
    if delivery_only && answer.is_none() {
        return Err(setup(
            "The completed checkpoint required by this delivery claim is unavailable",
        ));
    }
    Ok(answer)
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use flow_like::flow::copilot::run_recovery::BotRunRecovery;
    use flow_like_types::bot_turn::BotRunCheckpoint;

    #[derive(Debug)]
    struct EmptyRecovery;

    #[async_trait]
    impl BotRunRecovery for EmptyRecovery {
        async fn load(&self) -> flow_like_types::Result<Option<BotRunCheckpoint>> {
            Ok(None)
        }
        async fn save(&self, _: &BotRunCheckpoint) -> flow_like_types::Result<()> {
            panic!("Delivery must never create new execution state")
        }
    }

    #[tokio::test]
    async fn delivery_claims_require_a_finished_checkpoint_before_execution() {
        assert!(saved_reply(None, true).await.is_err());
        let empty: Arc<dyn BotRunRecovery> = Arc::new(EmptyRecovery);
        assert!(saved_reply(Some(&empty), true).await.is_err());
        assert_eq!(saved_reply(Some(&empty), false).await.unwrap(), None);
    }
}

fn profile_of(grant: &DeviceTurnGrant) -> Result<(Profile, Bit, Option<Bit>), RunError> {
    if grant.delivery_only {
        return Err(setup("This claim only permits publishing its saved answer"));
    }
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
    bits.extend(
        grant
            .models
            .decision
            .as_ref()
            .and_then(|value| bit_of(value, "decision").ok())
            .filter(|bit| {
                bit.try_to_systemone().is_some_and(|parameters| {
                    parameters.context_length > 0
                        && flow_like::flow_like_model_provider::provider::is_hosted_provider_name(
                            &parameters.provider.provider_name,
                        )
                })
            }),
    );
    let profile = Profile {
        name: grant.bot.name.clone(),
        bits: bits.iter().map(|bit| bit.id.clone()).collect(),
        custom_bits: bits.into_iter().map(ProfileCustomBit).collect(),
        ..Profile::default()
    };
    Ok((profile, completion, embedding))
}

fn code_languages() -> Vec<String> {
    CatalogSandbox::shared()
        .languages()
        .into_iter()
        .map(|language| language.id)
        .collect()
}

/// One working directory per bot under the device's state directory; the workspace's files are
/// mirrored into it for the run.
fn shell_dir(state_dir: &Path, bot_id: &str) -> PathBuf {
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
    state_dir.join("bot-shell").join(name)
}

pub fn host_tools(machine: &MachineBackends) -> HostTools {
    let mut tools = HostTools::linked()
        .with_code_languages(&code_languages())
        .with_shell(true);
    if machine.browser.is_some() || machine.computer.is_some() {
        tools = tools.with_local_machine();
    }
    TOOLS_NEEDING_UNLINKED_SERVICES
        .iter()
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
        decision_model: grant
            .models
            .decision
            .as_ref()
            .and_then(|value| value.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        decision_hosted_only: true,
        stt_model: spec.stt_model.clone(),
        tts_model: spec.tts_model.clone(),
        image_model: spec.image_model.clone(),
        vision_model: spec.vision_model.clone(),
        subtask_model: spec.subtask_model.clone(),
        tool_host: BotToolHost::CoreOnly,
        host_tools: tools,
        skills: spec.skills.clone(),
        skills_ranked: false,
        requester,
        acknowledgement: job.acknowledgement,
        live_call: job.live_call,
        spoken_drafts: false,
        tool_gate: job.tool_gate.clone(),
        ephemeral_user_context: job.ephemeral_user_context,
        seed_taint: job.seed_taint,
        suppress_owner_notifications: job.suppress_owner_notifications,
        permissions: Default::default(),
        connectors: spec.connectors.clone(),
        compute: Default::default(),
        recovery: job.recovery.clone().map(Into::into),
    }
}

impl DeviceRunner {
    pub fn new(
        state: Arc<FlowLikeState>,
        base_url: String,
        machines: Option<Arc<dyn Machines>>,
        state_dir: &Path,
    ) -> Self {
        Self {
            state,
            base_url,
            machines,
            shell_root: state_dir.to_path_buf(),
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
        profile: &Arc<Profile>,
        machine: MachineBackends,
    ) -> Result<EnvFactory, RunError> {
        let workspace = grant
            .bot
            .workspace_app_id
            .clone()
            .or_else(|| {
                grant
                    .bot
                    .capabilities
                    .contains(&BotCapability::UserContext)
                    .then(String::new)
            })
            .ok_or_else(|| setup("The bot has no workspace app yet"))?;
        let models = Arc::new(StateModels {
            state: self.state.clone(),
            chat: Some(completion.clone()),
            embedding: embedding.clone(),
            token: creds.executor_token(),
            usage: ModelUsageContext {
                app_id: (!workspace.is_empty()).then(|| workspace.clone()),
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
        let images = grant
            .models
            .image
            .as_ref()
            .map(|bit| bit_of(bit, "image"))
            .transpose()?
            .map(|bit| {
                Arc::new(HubImages {
                    state: self.state.clone(),
                    profile: profile.clone(),
                    bit,
                    creds: creds.clone(),
                    usage: models.usage.clone(),
                    budget: Arc::default(),
                })
            });
        let connectors = Arc::new(HubConnectors::new(routes.clone()));
        let shell_dir = shell_dir(&self.shell_root, &grant.bot.bot_id);
        let files_hub = routes.hub.clone();
        let files_conversation = grant.conversation.id.clone();
        Ok(Arc::new(move |expensive_documents| {
            let mut env = HostEnv::new(&workspace, &scope, access.clone(), models.clone())
                .with_expensive_documents(expensive_documents)
                .with_secrets(secrets.clone())
                .with_sessions(sessions.clone())
                .with_notifier(Arc::new(HubNotifier(routes.clone())))
                .with_scheduler(Arc::new(HubScheduler(routes.clone())))
                .with_people(Arc::new(HubPeople(routes.clone())))
                .with_skills(Arc::new(HubSkills(routes.clone())))
                .with_user_context(Arc::new(HubUserContext(routes.clone())))
                .with_mail(Arc::new(HubMail(routes.clone())))
                .with_connectors(connectors.clone());
            if let Some(images) = &images {
                env = env.with_images(images.clone());
            }
            Arc::new(
                DeviceToolEnv::new(env, machine.clone())
                    .with_shell(Arc::new(LocalShell::new(shell_dir.clone())))
                    .with_conversation_files(ConversationFiles::new(
                        files_hub.clone(),
                        &files_conversation,
                    )),
            )
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
    async fn classify(
        &self,
        job: &TurnJob,
        request: &flow_like::flow_like_model_provider::systemone::SystemOneRequest,
    ) -> Result<Option<flow_like::flow_like_model_provider::systemone::SystemOneResponse>, RunError>
    {
        use flow_like::flow::copilot::decision;
        let spec = job
            .claim_as::<DeviceRunSpec>()
            .ok_or_else(|| setup("The claim carried no device run spec"))?;
        if spec.grant.delivery_only {
            return Ok(None);
        }
        let Some(value) = &spec.grant.models.decision else {
            return Ok(None);
        };
        let bit = bit_of(value, "decision")?;
        let parameters = bit
            .try_to_systemone()
            .ok_or_else(|| setup("Expected a SystemOne decision model"))?;
        if !flow_like::flow_like_model_provider::provider::is_hosted_provider_name(
            &parameters.provider.provider_name,
        ) {
            return Err(setup(
                "Device bot decision models must use the hosted proxy",
            ));
        }
        let mut request = request.clone();
        request.state["context"]["bot_name"] = serde_json::json!(spec.grant.bot.name);
        request.state["context"]["instructions"] = serde_json::json!(decision::excerpt(
            spec.grant.bot.instructions.as_deref().unwrap_or_default(),
            2_000,
        ));
        let model = self
            .state
            .model_factory
            .build_systemone(
                &bit,
                self.state.clone(),
                spec.creds.executor_token(),
                Some(ModelUsageContext {
                    app_id: spec.grant.bot.workspace_app_id.clone(),
                    run_id: Some(job.run_id.clone()),
                    api_base_url: Some(self.base_url.clone()),
                }),
            )
            .await
            .map_err(|error| setup(error.to_string()))?;
        let response = model
            .invoke(&request)
            .await
            .map_err(|error| setup(error.to_string()))?;
        response
            .validate_for(&request)
            .map_err(|error| setup(error.to_string()))?;
        Ok(Some(response))
    }

    async fn run(&self, mut job: TurnJob) -> Result<String, RunError> {
        let spec = job
            .claim_as::<DeviceRunSpec>()
            .ok_or_else(|| setup("The claim carried no device run spec"))?;
        let grant = &spec.grant;
        job.recovery = job.recovery.map(|recovery| {
            Arc::new(flow_like_bot_host::recovery::CappedRecovery::new(
                recovery,
                grant.deadline.saturating_mul(1000),
            )) as Arc<dyn flow_like::flow::copilot::run_recovery::BotRunRecovery>
        });
        if let Some(answer) = saved_reply(job.recovery.as_ref(), grant.delivery_only).await? {
            return Ok(answer);
        }
        let (profile, completion, embedding) = profile_of(grant)?;
        let profile = Arc::new(profile);
        let machine = self.machine_for(grant);
        let mut tools = host_tools(&machine);
        if grant.models.image.is_none() {
            tools = tools.without(IMAGE_GENERATE);
        }
        if let Some(trail) = &job.trail {
            tools = tools.with_trail(trail.clone());
        }
        if let Some(files) = &job.reply_files {
            tools = tools.with_reply_files(files.clone());
        }
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
            &profile,
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
            if !job.ephemeral_user_context {
                history = memory.history();
                prompt = memory.prompt().to_string();
            }
            extra = format!("\n\n{}", memory.context());
            if let Some(shared) = job
                .shared_context
                .as_ref()
                .filter(|_| !job.ephemeral_user_context)
            {
                extra.push_str(&format!("\n\n{shared}"));
            }
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
            Some(bot_thinking(job.live_call)),
            token,
            bridge,
            None,
            None::<fn(String)>,
            None,
        );
        tokio::pin!(chat);
        let mut poll = tokio::time::interval(SESSION_POLL);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let outcome = loop {
            tokio::select! {
                outcome = &mut chat => break Some(outcome),
                () = control.wake.notified() => break None,
                _ = poll.tick() => {
                    if spec.creds.session_lapsed() {
                        control.stopped.store(true, Ordering::SeqCst);
                        self.untrack(&job.run_id);
                        return Err(if mutating.load(Ordering::SeqCst) {
                            RunError::Failed {
                                code: CODE_RUN_FAILED.into(),
                                message: "The device's session with the hub lapsed during the turn".into(),
                            }
                        } else {
                            RunError::SessionChanged
                        });
                    }
                }
            }
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
        if !context.remembers() {
            return None;
        }
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

#[cfg(test)]
mod tool_tests {
    use super::*;

    fn offered() -> Vec<&'static str> {
        host_tools(&MachineBackends::default()).names().collect()
    }

    #[test]
    fn people_tools_are_left_to_the_core_owner_filter() {
        let names = offered();
        assert!(names.contains(&"list_people"));
        assert!(names.contains(&"message_person"));
    }

    #[test]
    fn the_shell_is_offered_and_tools_without_a_device_route_are_not() {
        let names = offered();
        assert!(names.contains(&"shell_run"));
        for removed in TOOLS_NEEDING_UNLINKED_SERVICES {
            assert!(!names.contains(&removed), "{removed}");
        }
    }

    #[test]
    fn shell_directories_are_one_confined_name_per_bot() {
        assert_eq!(
            shell_dir(Path::new("/state"), "a/../b"),
            Path::new("/state/bot-shell/a____b")
        );
    }
}
