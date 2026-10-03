//! The Telegram and Discord bots of a placement process: the agent's side of
//! `flow_like_bots`. A bot event's settings and token are read before the validation
//! early-out; after Ready each token is registered under its handle in the catalog, so a
//! flow receives the handle and never the token, and the crate connects every bot whose
//! claim gate is open. The scheduler's one claim call decides the gates (`ClaimGates`).

use std::{
    collections::HashMap,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use flow_like_bots::{
    Bot, BotHost, BotProblem, BotSpec, BotToken, BotsState, Connector, Gate, Provider, Revisions,
    RunEnd, StreamEvent, limits::RUN_TIME_LIMIT, runner::Providers, state::MAX_STATE_BYTES,
};
use flow_like_types::intercom::{InterComCallback, InterComEvent};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    config::PlacementConfig,
    hosting::PreparedInvocation,
    run_once::{RunContext, RunRequest},
    schedule::{self, ClaimGates},
};

const STATE_DIRECTORY: &str = ".standalone-bots";
const STATE_FILE: &str = "state.json";
const HANDLE_PREFIX: &str = "device-bot:";

/// The `secret_overrides` key that names the token of the bot event `event_id`.
pub(crate) fn token_key(event_id: &str) -> String {
    crate::event_kind::bot_token_key(event_id)
}

/// What a flow receives in place of the token of the bot event `event_id`.
pub(crate) fn handle(event_id: &str) -> String {
    format!("{HANDLE_PREFIX}{event_id}")
}

/// A bot event of the placement with its settings and, once secrets were read, its token.
pub(crate) struct PreparedBot {
    spec: BotSpec,
    token: Option<BotToken>,
    invocation: PreparedInvocation,
}

impl PreparedBot {
    pub(crate) fn event_id(&self) -> &str {
        &self.spec.event_id
    }
}

/// Reads the settings of a bot event and, with `read_secret`, its token. It runs before the
/// validation early-out, so `validate` and `validate_rollout` refuse what cannot run. No
/// sentence names the token or a part of it; a problem of the settings or the token can be
/// told apart by `downcast_ref::<BotProblem>()` and its `code()`.
pub(crate) fn prepare(
    config: &PlacementConfig,
    invocation: PreparedInvocation,
    read_secret: bool,
) -> Result<PreparedBot> {
    let event = &invocation.event;
    let spec = BotSpec::from_config(&event.id, &event.event_type, &event.config)
        .with_context(|| format!("bot of event {}", event.id))?;
    let token = read_secret
        .then(|| read_token(config, &spec))
        .transpose()
        .with_context(|| format!("bot of event {}", spec.event_id))?;
    Ok(PreparedBot {
        spec,
        token,
        invocation,
    })
}

/// The token from the secret its reserved key names. A secret that is not there is a missing
/// token; any other problem keeps its own sentence, which holds no part of the value.
fn read_token(config: &PlacementConfig, spec: &BotSpec) -> Result<BotToken> {
    let name = config
        .secret_overrides
        .get(&token_key(&spec.event_id))
        .ok_or(BotProblem::TokenMissing)?;
    let secret = crate::config::private_secret_path(config, name)
        .and_then(|path| crate::vault::read_private(&path))
        .map_err(|error| {
            if absent(&error) {
                anyhow::Error::new(BotProblem::TokenMissing)
            } else {
                error.context("Read the bot token")
            }
        })?;
    Ok(BotToken::parse(spec.provider, &secret)?)
}

fn absent(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
    })
}

/// What the bots of one placement process share.
pub(crate) struct BotContext {
    pub(crate) run: Arc<RunContext>,
    /// Decided by the scheduler, which makes the one parent call and the one claim call of the
    /// process for the schedules and the bots together.
    pub(crate) gates: ClaimGates,
    /// `<data root>/.standalone-bots/<placement id>`, where the agent parent reads the bots'
    /// state: [`BotContext::state_directory`].
    pub(crate) state_dir: PathBuf,
    pub(crate) placement_id: String,
    pub(crate) config_revision: u64,
    pub(crate) intent_revision: u64,
}

impl BotContext {
    /// The bots' state directory of `placement_id` below the placement's data root.
    pub(crate) fn state_directory(data_root: &Path, placement_id: &str) -> PathBuf {
        data_root.join(STATE_DIRECTORY).join(placement_id)
    }

    /// The context of the bots that run beside the schedules of `schedule`, in the same data
    /// root, placement and revisions.
    pub(crate) fn beside(
        schedule: &schedule::ScheduleContext,
        run: Arc<RunContext>,
        gates: ClaimGates,
    ) -> Result<Self> {
        let data_root = schedule
            .state_dir
            .parent()
            .and_then(Path::parent)
            .context("The schedule state directory lies outside a placement data root")?;
        Ok(Self {
            run,
            gates,
            state_dir: Self::state_directory(data_root, &schedule.placement_id),
            placement_id: schedule.placement_id.clone(),
            config_revision: schedule.config_revision,
            intent_revision: schedule.intent_revision,
        })
    }

    fn revisions(&self) -> Revisions {
        Revisions {
            config_revision: self.config_revision,
            intent_revision: self.intent_revision,
        }
    }
}

/// Before Ready: the state directory exists and holds this process's document, with
/// `decided: false`. A failure fails the start.
pub(crate) fn prepare_state(bots: &[PreparedBot], context: &BotContext) -> Result<()> {
    if bots.is_empty() {
        return Ok(());
    }
    create_state_directory(&context.state_dir)?;
    let store = StateStore {
        directory: context.state_dir.clone(),
    };
    let state = BotsState::prepare(
        store.load(),
        bots.iter().map(|bot| &bot.spec),
        context.revisions(),
        now(),
    );
    store
        .save(&state)
        .with_context(|| format!("Write bot state in {}", context.state_dir.display()))
}

/// The bots after Ready, until `stop`. Returns `Ok(())` only then: a bot that fails is its
/// state, never the end of this task.
pub(crate) async fn run(
    bots: Vec<PreparedBot>,
    context: BotContext,
    ready: oneshot::Receiver<()>,
    stop: CancellationToken,
) -> Result<()> {
    let gates = bots
        .iter()
        .filter_map(|bot| {
            let gate = context.gates.watch(bot.event_id());
            if gate.is_none() {
                tracing::warn!(
                    event_id = bot.event_id(),
                    "Bot has no claim gate; it does not connect"
                );
            }
            Some((bot.event_id().to_owned(), gate?))
        })
        .collect();
    serve(bots, context, gates, Arc::new(Providers), ready, stop).await
}

/// [`run`] with the gates and provider connections given.
async fn serve(
    bots: Vec<PreparedBot>,
    context: BotContext,
    gates: HashMap<String, watch::Receiver<schedule::Gate>>,
    connector: Arc<dyn Connector>,
    ready: oneshot::Receiver<()>,
    stop: CancellationToken,
) -> Result<()> {
    if bots.is_empty() || !ready_or_stopped(ready, &stop).await {
        stop.cancelled().await;
        return Ok(());
    }
    let (runnable, events) = with_tokens(bots);
    let handles = Handles::register(&runnable);
    tracing::info!(
        placement_id = %context.placement_id,
        bots = runnable.len(),
        "Bots of the service start"
    );
    let host = Arc::new(AgentHost::new(&context, events, gates, &runnable));
    flow_like_bots::run_with(
        runnable,
        context.revisions(),
        host,
        connector,
        std::future::ready(()),
        stop,
    )
    .await;
    drop(handles);
    tracing::info!(placement_id = %context.placement_id, "Bots of the service stopped");
    Ok(())
}

/// The bots that have their token, and the flow each one runs.
fn with_tokens(bots: Vec<PreparedBot>) -> (Vec<Bot>, HashMap<String, PreparedInvocation>) {
    let mut events = HashMap::new();
    let mut runnable = Vec::new();
    for bot in bots {
        let Some(token) = bot.token else {
            tracing::error!(
                event_id = bot.spec.event_id.as_str(),
                "Bot was prepared without its token; it does not connect"
            );
            continue;
        };
        events.insert(bot.spec.event_id.clone(), bot.invocation);
        runnable.push(Bot {
            spec: bot.spec,
            token,
        });
    }
    (runnable, events)
}

async fn ready_or_stopped(ready: oneshot::Receiver<()>, stop: &CancellationToken) -> bool {
    tokio::select! {
        biased;
        _ = stop.cancelled() => false,
        signal = ready => {
            if signal.is_err() {
                stop.cancelled().await;
            }
            signal.is_ok()
        }
    }
}

/// The handles of the process's bots, registered with the catalog while the bots run. A flow
/// node resolves a handle to its token inside this process only.
struct Handles(Vec<(Provider, String)>);

impl Handles {
    fn register(bots: &[Bot]) -> Self {
        Self(
            bots.iter()
                .map(|bot| {
                    let handle = handle(&bot.spec.event_id);
                    let token = bot.token.expose();
                    match bot.spec.provider {
                        Provider::Telegram => {
                            flow_like_catalog::telegram::session::register_bot_credential(
                                &handle, token,
                            )
                        }
                        Provider::Discord => {
                            flow_like_catalog::discord::session::register_bot_credential(
                                &handle, token,
                            )
                        }
                    }
                    (bot.spec.provider, handle)
                })
                .collect(),
        )
    }
}

impl Drop for Handles {
    fn drop(&mut self) {
        for (provider, handle) in &self.0 {
            match provider {
                Provider::Telegram => {
                    flow_like_catalog::telegram::session::forget_bot_credential(handle)
                }
                Provider::Discord => {
                    flow_like_catalog::discord::session::forget_bot_credential(handle)
                }
            }
        }
    }
}

struct AgentHost {
    run: Arc<RunContext>,
    events: HashMap<String, PreparedInvocation>,
    gates: HashMap<String, watch::Receiver<schedule::Gate>>,
    /// Telegram bots, whose updates may answer a flow node that waits for a reply.
    observers: HashMap<String, BotToken>,
    store: StateStore,
}

impl AgentHost {
    fn new(
        context: &BotContext,
        events: HashMap<String, PreparedInvocation>,
        gates: HashMap<String, watch::Receiver<schedule::Gate>>,
        bots: &[Bot],
    ) -> Self {
        Self {
            run: context.run.clone(),
            events,
            gates,
            observers: bots
                .iter()
                .filter(|bot| bot.spec.provider == Provider::Telegram)
                .map(|bot| (bot.spec.event_id.clone(), bot.token.clone()))
                .collect(),
            store: StateStore {
                directory: context.state_dir.clone(),
            },
        }
    }
}

#[async_trait]
impl BotHost for AgentHost {
    async fn run(
        &self,
        event_id: &str,
        payload: Value,
        events: mpsc::Sender<StreamEvent>,
        cancel: CancellationToken,
    ) -> RunEnd {
        let Some(invocation) = self.events.get(event_id) else {
            return RunEnd::Failed;
        };
        let run_id = uuid::Uuid::new_v4().to_string();
        let asked = Arc::new(AtomicBool::new(false));
        let cancel = cancel.child_token();
        let request_bytes = serde_json::to_vec(&payload).map_or(0, |bytes| bytes.len() as u64);
        tracing::info!(event_id, run_id = %run_id, "Bot run's flow started");
        let end = crate::run_once::run_event_once(
            &self.run,
            invocation,
            RunRequest {
                payload: Some(payload),
                callback: forward(events, cancel.clone(), asked.clone()),
                cancel,
                run_id: Some(run_id),
                time_limit: RUN_TIME_LIMIT,
                request_bytes,
            },
        )
        .await;
        if asked.load(Ordering::SeqCst) {
            return RunEnd::Failed;
        }
        match end {
            crate::run_once::RunEnd::Succeeded => RunEnd::Succeeded,
            crate::run_once::RunEnd::Failed => RunEnd::Failed,
            crate::run_once::RunEnd::Cancelled => RunEnd::Cancelled,
            crate::run_once::RunEnd::TimedOut => RunEnd::TimedOut,
        }
    }

    fn gate(&self, event_id: &str) -> watch::Receiver<Gate> {
        match self.gates.get(event_id) {
            Some(source) => follow_gate(source.clone()),
            None => watch::channel(Gate::Undecided).1,
        }
    }

    fn load(&self) -> Option<BotsState> {
        self.store.load()
    }

    fn save(&self, state: &BotsState) -> std::io::Result<()> {
        self.store.save(state)
    }

    fn now(&self) -> i64 {
        now()
    }

    fn handle(&self, event_id: &str) -> String {
        handle(event_id)
    }

    fn observed(&self, event_id: &str, update: &Value) -> bool {
        self.observers.get(event_id).is_some_and(|token| {
            flow_like_catalog::telegram::session::broadcast_update_json(token.expose(), update)
        })
    }
}

/// The run's chat events go to the bot's reply; a partial chunk is dropped while the reply is
/// behind. A question cancels the run at once: nobody can answer it here.
fn forward(
    events: mpsc::Sender<StreamEvent>,
    cancel: CancellationToken,
    asked: Arc<AtomicBool>,
) -> InterComCallback {
    Some(Arc::new(move |event: InterComEvent| {
        let (events, cancel, asked) = (events.clone(), cancel.clone(), asked.clone());
        Box::pin(async move {
            let question = event.event_type == "interaction_request";
            if question {
                asked.store(true, Ordering::SeqCst);
                cancel.cancel();
            } else if !event.event_type.starts_with("chat_") {
                return Ok(());
            }
            let partial = event.event_type == "chat_stream_partial";
            let event = StreamEvent::new(event.event_type, event.payload);
            if question || partial {
                let _ = events.try_send(event);
            } else {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {}
                    _ = events.send(event) => {}
                }
            }
            Ok(())
        })
    }))
}

/// The scheduler's gate in the crate's terms, for as long as both sides listen.
fn follow_gate(mut source: watch::Receiver<schedule::Gate>) -> watch::Receiver<Gate> {
    let (sender, receiver) = watch::channel(bot_gate(*source.borrow_and_update()));
    tokio::spawn(async move {
        loop {
            tokio::select! {
                changed = source.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let gate = bot_gate(*source.borrow_and_update());
                    sender.send_if_modified(|current| std::mem::replace(current, gate) != gate);
                }
                () = sender.closed() => return,
            }
        }
    });
    receiver
}

fn bot_gate(gate: schedule::Gate) -> Gate {
    match gate {
        schedule::Gate::Undecided => Gate::Undecided,
        schedule::Gate::Armed { .. } => Gate::Open,
        schedule::Gate::Held(reason) => Gate::Held(reason.as_str()),
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// `<data root>/.standalone-bots/<placement>`, private to the agent's account.
fn create_state_directory(directory: &Path) -> Result<()> {
    directory
        .parent()
        .context("Bot state directory has no parent")
        .and_then(crate::runtime::private_runtime_directory)
        .and_then(|()| crate::runtime::private_runtime_directory(directory))
        .with_context(|| format!("Create the bot state directory {}", directory.display()))
}

/// The bots' state file (§1.6b). The agent parent reads it as untrusted input.
struct StateStore {
    directory: PathBuf,
}

impl StateStore {
    /// The document an earlier process left, or `None` when there is none this agent can use.
    fn load(&self) -> Option<BotsState> {
        let path = self.directory.join(STATE_FILE);
        let bytes = match crate::diagnostics::small_file(&path, MAX_STATE_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return None,
            Err(_) => {
                tracing::warn!("Bot state is not readable; the bots start without it");
                return None;
            }
        };
        let state = serde_json::from_slice::<BotsState>(&bytes)
            .ok()
            .filter(|_| bytes.len() <= MAX_STATE_BYTES);
        if state.is_none() {
            tracing::warn!("Bot state is not usable by this agent; the bots start without it");
        }
        state
    }

    /// Replaces the file as a whole. File and name are durable on return: a watermark lost to a
    /// power failure could answer a message twice.
    fn save(&self, state: &BotsState) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(state)?;
        if bytes.len() > MAX_STATE_BYTES {
            return Err(std::io::Error::other(format!(
                "Bot state of {} bytes exceeds its {MAX_STATE_BYTES} byte limit",
                bytes.len()
            )));
        }
        let temporary = self.directory.join(format!(".{STATE_FILE}.tmp"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, self.directory.join(STATE_FILE))?;
        std::fs::File::open(&self.directory)?.sync_all()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::config::EventBinding;
    use flow_like_bots::reply::{Answer, ReplySink, ReplyStream, SendError};
    use flow_like_catalog::events::chat_event::{ChatResponse, ChatStreamingResponse};
    use flow_like_runtime::{
        app::{App, AppVisibility},
        flow::{
            board::Board,
            compiled::TemplateCache,
            execution::context::ExecutionContext,
            node::{Node, NodeLogic},
            variable::VariableType,
        },
        flow_like_model_provider::{response::Response, response_chunk::ResponseChunk},
        profile::Profile,
        state::FlowLikeState,
    };
    use flow_like_storage::Path as StorePath;
    use serde_json::json;
    use std::{sync::Mutex, time::Duration};
    use tokio::sync::Notify;

    const EVENT: &str = "helper";
    const TELEGRAM_TOKEN: &str = "123456789:AAH-bot_secrecy-token_0123456789abcdefghij";
    // Synthetic segments exercise local parsing without resembling an issued token.
    const DISCORD_TOKEN: &str = "MTIzNDU2Nzg5MDEyMzQ1Njc4.test-only.not-a-real-discord-token";

    /// The flow of a bot event: it keeps the payload of every run and answers, or asks a
    /// question nobody can answer and waits.
    #[derive(Default)]
    struct BotProbe {
        payloads: Mutex<Vec<Value>>,
        ask: AtomicBool,
        dropped: Notify,
    }

    struct Dropped<'a>(&'a Notify);

    impl Drop for Dropped<'_> {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }

    #[flow_like_types::async_trait]
    impl NodeLogic for BotProbe {
        fn get_node(&self) -> Node {
            let mut node = Node::new("standalone_test_bot", "Bot", "", "Tests");
            node.set_start(true);
            node.add_input_pin("exec_in", "Execute", "", VariableType::Execution);
            node
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let payload = context.get_payload().await?.payload.clone();
            self.payloads
                .lock()
                .unwrap()
                .push(payload.unwrap_or_default());
            if self.ask.load(Ordering::SeqCst) {
                let _dropped = Dropped(&self.dropped);
                context
                    .stream_response("interaction_request", json!({"id": "confirm"}))
                    .await?;
                std::future::pending::<()>().await;
            }
            context
                .stream_response("chat_stream_partial", partial("po"))
                .await?;
            context.stream_response("chat_stream", answer("pong")).await
        }
    }

    fn partial(text: &str) -> ChatStreamingResponse {
        ChatStreamingResponse {
            chunk: Some(ResponseChunk::from_text(text, "probe")),
            actions: vec![],
            attachments: vec![],
            plan: None,
            widgets: vec![],
        }
    }

    fn answer(text: &str) -> ChatResponse {
        ChatResponse {
            response: Response::from_text(text, "probe"),
            local_session: None,
            global_session: None,
            actions: vec![],
            attachments: vec![],
            model_id: None,
            widgets: vec![],
        }
    }

    struct Fixture {
        config: PlacementConfig,
        state: Arc<FlowLikeState>,
        visibility: AppVisibility,
        probe: Arc<BotProbe>,
        invocation: PreparedInvocation,
    }

    /// The runtime fixture's project with one bot event `helper` of `event_type` on a flow of
    /// its own; the placement lists only that event.
    async fn fixture(root: &Path, event_type: &str, settings: Value) -> Fixture {
        let (mut config, state, _, _) = crate::runtime::tests::fixture(root).await;
        let probe = Arc::new(BotProbe::default());
        state.node_registry.write().await.push_node(probe.clone());
        let app = App::load(config.project_id.clone(), state.clone())
            .await
            .unwrap();
        let mut board = Board::new(
            Some("bot-board".into()),
            StorePath::from("apps/project"),
            state.clone(),
        );
        let mut node = probe.get_node();
        node.id = "bot".into();
        board.nodes.insert(node.id.clone(), node);
        board.snapshot_at_version((1, 0, 0), None).await.unwrap();
        let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
        event.id = EVENT.into();
        event.board_id = board.id.clone();
        event.node_id = "bot".into();
        event.event_type = event_type.into();
        event.config = serde_json::to_vec(&settings).unwrap();
        event.save(&app, Some((1, 0, 0))).await.unwrap();
        config.events = vec![EventBinding {
            event_id: EVENT.into(),
            event_version: [1, 0, 0],
            board_version: [1, 0, 0],
        }];
        let template = TemplateCache::default()
            .resolve(&state, &app.id, &board.id, Some((1, 0, 0)), None, "")
            .await
            .unwrap();
        Fixture {
            config,
            state,
            visibility: app.visibility.clone(),
            probe,
            invocation: PreparedInvocation {
                event,
                template,
                action_admission: None,
            },
        }
    }

    /// The placement secret `bot-token` with `value`, named by the bot's reserved key.
    fn store_token(config: &mut PlacementConfig, value: &[u8]) {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        let directory = config.project_path.join(".secrets").join(&config.id);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(directory.join("bot-token.secret"))
            .and_then(|mut file| file.write_all(value))
            .unwrap();
        config
            .secret_overrides
            .insert(token_key(EVENT), "bot-token".into());
    }

    fn json_string(token: &str) -> Vec<u8> {
        serde_json::to_vec(token).unwrap()
    }

    fn context(fixture: &Fixture, data_root: &Path) -> BotContext {
        BotContext {
            run: Arc::new(RunContext {
                project_id: fixture.config.project_id.clone(),
                state: fixture.state.clone(),
                profile: Profile::default(),
                visibility: fixture.visibility.clone(),
                execution_sub: None,
            }),
            gates: ClaimGates::new(vec![EVENT.into()]),
            state_dir: BotContext::state_directory(data_root, &fixture.config.id),
            placement_id: fixture.config.id.clone(),
            config_revision: 1,
            intent_revision: 1,
        }
    }

    fn bot_state(directory: &Path) -> Value {
        std::fs::read(directory.join(STATE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    /// Waits up to a minute: teloxide pauses 10 s after every answer of a server error, and a
    /// run whose partial and final answer both meet one takes 20 s.
    #[cfg(any(feature = "bots-telegram", feature = "bots-discord"))]
    async fn until(what: &str, condition: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(60), async {
            while !condition() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("Timed out waiting until {what}"));
    }

    fn problem(error: &anyhow::Error) -> &str {
        error
            .downcast_ref::<BotProblem>()
            .map_or("", BotProblem::code)
    }

    #[test]
    fn the_reserved_names_follow_the_event_id() {
        assert_eq!(token_key("evt_helper"), "event.evt_helper.bot_token");
        assert_eq!(handle("evt_helper"), "device-bot:evt_helper");
        assert_eq!(
            BotContext::state_directory(Path::new("/data/store"), "placement"),
            Path::new("/data/store/.standalone-bots/placement")
        );
    }

    fn refusal(fixture: &Fixture, read_secret: bool) -> anyhow::Error {
        prepare(&fixture.config, fixture.invocation.clone(), read_secret)
            .err()
            .expect("the bot is refused")
    }

    #[tokio::test]
    async fn prepare_refuses_bad_settings_naming_the_key() {
        let root = tempfile::tempdir().unwrap();
        let mut fixture = fixture(root.path(), "telegram", json!({"chat_whitelist": "42"})).await;
        let error = refusal(&fixture, false);
        assert_eq!(problem(&error), "bot_invalid");
        let sentence = format!("{error:#}");
        assert!(
            sentence.contains("bot of event helper") && sentence.contains("chat_whitelist"),
            "{sentence}"
        );
        fixture.invocation.event.event_type = "discord".into();
        fixture.invocation.event.config =
            serde_json::to_vec(&json!({"intents": ["Guilds", "Telepathy"]})).unwrap();
        assert_eq!(problem(&refusal(&fixture, false)), "bot_invalid");
        fixture.invocation.event.config = serde_json::to_vec(&json!({})).unwrap();
        let bot = prepare(&fixture.config, fixture.invocation.clone(), false).unwrap();
        assert!(bot.token.is_none(), "plain validation reads no secret");
        assert_eq!(bot.event_id(), EVENT);
    }

    #[tokio::test]
    async fn prepare_reads_the_token_with_secrets_and_never_names_its_value() {
        let root = tempfile::tempdir().unwrap();
        let mut fixture = fixture(root.path(), "telegram", json!({})).await;
        assert_eq!(problem(&refusal(&fixture, true)), "bot_token_missing");
        fixture
            .config
            .secret_overrides
            .insert(token_key(EVENT), "bot-token".into());
        assert_eq!(
            problem(&refusal(&fixture, true)),
            "bot_token_missing",
            "a key without its secret file"
        );
        let pasted = "Here is the token: 12345:short";
        store_token(&mut fixture.config, &json_string(pasted));
        let error = refusal(&fixture, true);
        assert_eq!(problem(&error), "bot_token_invalid");
        assert!(!format!("{error:#} {error:?}").contains("12345:short"));
        for stored in [
            json_string(TELEGRAM_TOKEN),
            TELEGRAM_TOKEN.as_bytes().to_vec(),
        ] {
            store_token(&mut fixture.config, &stored);
            let bot = prepare(&fixture.config, fixture.invocation.clone(), true).unwrap();
            let token = bot.token.as_ref().map(BotToken::expose);
            assert_eq!(token, Some(TELEGRAM_TOKEN));
        }
        fixture.invocation.event.event_type = "discord".into();
        assert_eq!(
            problem(&refusal(&fixture, true)),
            "bot_token_invalid",
            "a Telegram token is no Discord token"
        );
        store_token(&mut fixture.config, &json_string(DISCORD_TOKEN));
        assert!(prepare(&fixture.config, fixture.invocation.clone(), true).is_ok());
    }

    #[tokio::test]
    async fn the_state_file_is_private_starts_undecided_and_keeps_the_bots_watermark() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let fixture = fixture(root.path(), "telegram", json!({})).await;
        let bot = prepare(&fixture.config, fixture.invocation.clone(), false).unwrap();
        let mut context = context(&fixture, root.path());
        let directory = context.state_dir.clone();
        prepare_state(std::slice::from_ref(&bot), &context).unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&directory.join(STATE_FILE)), 0o600);
        assert_eq!(mode(&directory), 0o700);
        let state = bot_state(&directory);
        assert_eq!(
            (
                &state["version"],
                &state["decided"],
                &state["config_revision"]
            ),
            (&json!(1), &json!(false), &json!(1))
        );
        assert_eq!(state["bots"][EVENT]["state"], "waiting");

        let store = StateStore {
            directory: directory.clone(),
        };
        let mut earlier = store.load().unwrap();
        let entry = earlier.bots.get_mut(EVENT).unwrap();
        entry.bot_id = Some(123456789);
        entry.watermark = Some(1001);
        entry.watermark_at = Some(now());
        entry.runs = 3;
        earlier.decided = true;
        store.save(&earlier).unwrap();
        context.intent_revision = 2;
        prepare_state(std::slice::from_ref(&bot), &context).unwrap();
        let state = bot_state(&directory);
        assert_eq!(state["decided"], false);
        assert_eq!(state["intent_revision"], 2);
        assert_eq!(state["bots"][EVENT]["watermark"], 1001);
        assert_eq!(
            state["bots"][EVENT]["runs"], 0,
            "counters start again with an intent"
        );

        std::fs::write(directory.join(STATE_FILE), b"{not json").unwrap();
        prepare_state(std::slice::from_ref(&bot), &context).unwrap();
        assert_eq!(
            bot_state(&directory)["bots"][EVENT]["watermark"],
            Value::Null
        );
    }

    #[tokio::test]
    async fn a_run_that_asks_a_question_is_cancelled_at_once_and_failed() {
        let root = tempfile::tempdir().unwrap();
        let fixture = fixture(root.path(), "telegram", json!({})).await;
        fixture.probe.ask.store(true, Ordering::SeqCst);
        let context = context(&fixture, root.path());
        let events = HashMap::from([(EVENT.to_owned(), fixture.invocation.clone())]);
        let host = AgentHost::new(&context, events, HashMap::new(), &[]);
        let (events, mut received) = mpsc::channel(16);
        let started = std::time::Instant::now();
        let end = tokio::time::timeout(
            Duration::from_secs(20),
            host.run(
                EVENT,
                json!({"messages": []}),
                events,
                CancellationToken::new(),
            ),
        )
        .await
        .expect("a question ends the run");
        assert_eq!(end, RunEnd::Failed);
        assert!(started.elapsed() < Duration::from_secs(10));
        tokio::time::timeout(Duration::from_secs(5), fixture.probe.dropped.notified())
            .await
            .expect("the waiting flow was stopped");
        let mut kinds = Vec::new();
        while let Ok(event) = received.try_recv() {
            kinds.push(event.kind);
        }
        assert_eq!(kinds, ["interaction_request"]);
    }

    #[tokio::test]
    async fn the_reply_stream_reads_the_catalogs_own_stream_events() {
        type Shown = Arc<Mutex<Vec<(String, bool)>>>;
        struct Recording(Shown);

        #[async_trait]
        impl ReplySink for Recording {
            async fn show(&mut self, answer: &Answer, done: bool) -> Result<(), SendError> {
                self.0.lock().unwrap().push((answer.text.clone(), done));
                Ok(())
            }

            fn edit_interval(&self, _shown: u32) -> Duration {
                Duration::ZERO
            }
        }

        for last in ["chat_stream", "chat_out"] {
            let shown = Shown::default();
            let mut stream = ReplyStream::new(Box::new(Recording(shown.clone())));
            for text in ["Grüße, ", "Welt 🌍 你好"] {
                let payload = serde_json::to_value(partial(text)).unwrap();
                assert!(stream.take(&StreamEvent::new("chat_stream_partial", payload)));
            }
            assert_eq!(stream.answer().text, "Grüße, Welt 🌍 你好");
            let payload = serde_json::to_value(answer("Fertig: 完成 ✅")).unwrap();
            assert!(stream.take(&StreamEvent::new(last, payload)));
            assert_eq!(stream.answer().text, "Fertig: 完成 ✅", "{last}");
            stream.finish().await;
            assert_eq!(
                *shown.lock().unwrap(),
                [("Fertig: 完成 ✅".to_owned(), true)],
                "{last}"
            );
        }
    }

    /// The bots inside the placement runtime: validation, readiness, the reserved variable id.
    /// The runtime admits a bot event only in a build with its provider.
    #[cfg(any(feature = "bots-telegram", feature = "bots-discord"))]
    mod service {
        use super::*;
        use flow_like_runtime::flow::{pin::ValueType, variable::Variable};

        /// The runtime fixture's project with the bot event `helper` on a flow that starts with the
        /// catalog's own Chat Event node, so a runtime without the test's nodes can read it.
        /// `variable` adds a secret, exposed variable of that id to the flow.
        async fn catalog_fixture(
            root: &Path,
            event_type: &str,
            settings: Value,
            variable: Option<&str>,
        ) -> PlacementConfig {
            let (mut config, state, _, _) = crate::runtime::tests::fixture(root).await;
            let app = App::load(config.project_id.clone(), state.clone())
                .await
                .unwrap();
            let mut board = Board::new(
                Some("chat-board".into()),
                StorePath::from("apps/project"),
                state.clone(),
            );
            let registry = state.node_registry.read().await;
            let definition = registry.get_node("events_chat").unwrap();
            let mut node = registry.instantiate(&definition).unwrap().get_node();
            drop(registry);
            node.id = "chat".into();
            board.nodes.insert(node.id.clone(), node);
            if let Some(id) = variable {
                let mut secret = Variable::new("Token", VariableType::String, ValueType::Normal);
                secret.id = id.into();
                secret.secret = true;
                secret.exposed = true;
                board.variables.insert(secret.id.clone(), secret);
            }
            board.snapshot_at_version((1, 0, 0), None).await.unwrap();
            let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
            event.id = EVENT.into();
            event.board_id = board.id.clone();
            event.node_id = "chat".into();
            event.event_type = event_type.into();
            event.config = serde_json::to_vec(&settings).unwrap();
            event.save(&app, Some((1, 0, 0))).await.unwrap();
            config.events = vec![EventBinding {
                event_id: EVENT.into(),
                event_version: [1, 0, 0],
                board_version: [1, 0, 0],
            }];
            config
        }

        #[cfg(feature = "bots-telegram")]
        #[tokio::test]
        async fn validation_refuses_a_bad_setting_and_with_secrets_a_missing_or_malformed_token() {
            let root = tempfile::tempdir().unwrap();
            let settings = json!({"chat_whitelist": "42"});
            let config = catalog_fixture(root.path(), "telegram", settings, None).await;
            for error in [
                crate::runtime::validate(&config).await,
                crate::runtime::validate_rollout(&config).await,
            ] {
                let sentence = format!("{:#}", error.unwrap_err());
                assert!(
                    sentence.contains("bot of event helper") && sentence.contains("chat_whitelist"),
                    "{sentence}"
                );
            }

            let root = tempfile::tempdir().unwrap();
            let mut config = catalog_fixture(root.path(), "telegram", json!({}), None).await;
            crate::runtime::validate(&config)
                .await
                .expect("Apply reads no secret: the token arrives after it");
            let error = crate::runtime::validate_rollout(&config).await.unwrap_err();
            assert!(
                format!("{error:#}").contains("The bot token is missing"),
                "{error:#}"
            );
            let pasted = "my token is 12345:short";
            store_token(&mut config, &json_string(pasted));
            let sentence = format!(
                "{:#}",
                crate::runtime::validate_rollout(&config).await.unwrap_err()
            );
            assert!(
                sentence.contains("not in the shape of a bot token") && !sentence.contains(pasted),
                "{sentence}"
            );
            store_token(&mut config, &json_string(TELEGRAM_TOKEN));
            crate::runtime::validate_rollout(&config).await.unwrap();
            crate::runtime::validate(&config).await.unwrap();
        }

        #[cfg(feature = "bots-discord")]
        #[tokio::test]
        async fn validation_refuses_an_unknown_discord_intent() {
            let root = tempfile::tempdir().unwrap();
            let settings = json!({"intents": ["Guilds", "Telepathy"]});
            let config = catalog_fixture(root.path(), "discord", settings, None).await;
            let sentence = format!("{:#}", crate::runtime::validate(&config).await.unwrap_err());
            assert!(
                sentence.contains("bot of event helper") && sentence.contains("Telepathy"),
                "{sentence}"
            );
        }

        /// The rule is the runtime's (§1.10): a variable of that id would receive the token.
        #[cfg(feature = "bots-telegram")]
        #[tokio::test]
        async fn a_flow_variable_with_the_token_keys_id_is_refused() {
            let root = tempfile::tempdir().unwrap();
            let reserved = token_key(EVENT);
            let mut config =
                catalog_fixture(root.path(), "telegram", json!({}), Some(&reserved)).await;
            store_token(&mut config, &json_string(TELEGRAM_TOKEN));
            for error in [
                crate::runtime::validate(&config).await,
                crate::runtime::validate_rollout(&config).await,
            ] {
                let sentence = format!("{:#}", error.unwrap_err());
                assert!(
                    sentence.contains("variable id event.helper.bot_token is reserved"),
                    "{sentence}"
                );
                assert!(!sentence.contains(TELEGRAM_TOKEN));
            }
        }

        /// The agent parent: another service of this device runs every bot asked for.
        #[cfg(feature = "bots-telegram")]
        struct OtherService;

        #[cfg(feature = "bots-telegram")]
        #[async_trait]
        impl schedule::ScheduleArbiter for OtherService {
            async fn hold(&self, ids: &[String]) -> Result<Vec<(String, String)>> {
                Ok(ids
                    .iter()
                    .map(|id| (id.clone(), "other-service".to_owned()))
                    .collect())
            }
        }

        #[cfg(feature = "bots-telegram")]
        #[tokio::test]
        async fn a_bot_only_service_is_ready_and_stays_running_while_its_bot_is_held() {
            use crate::runtime::tests::{TestClock, schedule_context, start_supervised};
            let root = tempfile::tempdir().unwrap();
            let mut fixture = fixture(root.path(), "telegram", json!({})).await;
            store_token(&mut fixture.config, &json_string(TELEGRAM_TOKEN));
            let mut context =
                schedule_context(root.path(), TestClock::at("2026-10-03T00:00:00Z"), None);
            context.arbiter = Some(Arc::new(OtherService));
            let (ready, is_ready) = oneshot::channel();
            let (stop, task) = start_supervised(
                fixture.config.clone(),
                fixture.state.clone(),
                context,
                || async move {
                    let _ = ready.send(());
                    Ok(())
                },
            );
            tokio::time::timeout(Duration::from_secs(30), is_ready)
                .await
                .expect("a bot-only service becomes ready")
                .unwrap();
            let directory = BotContext::state_directory(root.path(), &fixture.config.id);
            until("the bot is held", || {
                let state = bot_state(&directory);
                state["decided"] == true && state["bots"][EVENT]["hold"] == "other_service"
            })
            .await;
            assert_eq!(bot_state(&directory)["bots"][EVENT]["state"], "waiting");
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(!task.is_finished(), "a held bot leaves the service running");
            stop.cancel();
            tokio::time::timeout(Duration::from_secs(30), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(fixture.probe.payloads.lock().unwrap().is_empty());
        }
    }

    /// A whole message-to-answer cycle through a provider connection: what the service keeps
    /// and logs never holds the bot's token.
    #[cfg(any(feature = "bots-telegram", feature = "bots-discord"))]
    mod providers {
        use super::*;
        use std::sync::{Weak, atomic::AtomicUsize};

        /// A gate the test moves, standing for the scheduler's.
        fn gate(
            initial: schedule::Gate,
        ) -> (
            watch::Sender<schedule::Gate>,
            HashMap<String, watch::Receiver<schedule::Gate>>,
        ) {
            let (sender, receiver) = watch::channel(initial);
            (sender, HashMap::from([(EVENT.to_owned(), receiver)]))
        }

        /// The parts of a token that must never leave its secret file: the whole token and
        /// what follows the bot's public id.
        fn secret_parts(token: &str) -> [&str; 2] {
            [token, token.rsplit([':', '.']).next().unwrap()]
        }

        fn assert_no_token(token: &str, place: &str, text: &str) {
            for part in secret_parts(token) {
                assert!(!text.contains(part), "{place} holds the bot token");
            }
        }

        /// Every file below `root` that holds a part of `token`, the placement's secrets aside.
        fn files_with_token(root: &Path, token: &str) -> Vec<PathBuf> {
            let holds = |bytes: &[u8]| {
                secret_parts(token).iter().any(|part| {
                    bytes
                        .windows(part.len())
                        .any(|window| window == part.as_bytes())
                })
            };
            let mut found = Vec::new();
            let mut pending = vec![root.to_path_buf()];
            while let Some(directory) = pending.pop() {
                for entry in std::fs::read_dir(&directory)
                    .into_iter()
                    .flatten()
                    .flatten()
                {
                    let Ok(kind) = entry.file_type() else {
                        continue;
                    };
                    let path = entry.path();
                    if kind.is_dir() && entry.file_name() != ".secrets" {
                        pending.push(path);
                    } else if kind.is_file()
                        && std::fs::read(&path).is_ok_and(|bytes| holds(&bytes))
                    {
                        found.push(path);
                    }
                }
            }
            found
        }

        /// What the agent parent makes of a bot state file: the row of a running placement
        /// that lists `helper`, live and as a snapshot.
        fn row_facts(state: &[u8]) -> [Value; 2] {
            use crate::diagnostics::{Detail, Diagnostics, Rows, test_support};
            let parent = tempfile::tempdir().unwrap();
            test_support::write_bot_state(parent.path(), "api", state);
            let record = test_support::running_record(&[EVENT]);
            let diagnostics = Diagnostics::default();
            [
                Rows::new(&diagnostics, parent.path(), false).placement(
                    &record,
                    true,
                    Detail::Full,
                ),
                Rows::snapshot(&diagnostics, parent.path()).placement(&record, true, Detail::Full),
            ]
        }

        /// The log lines of this process while it lives, filtered as a placement process
        /// filters them with `RUST_LOG=trace`.
        struct Logs(Arc<Lines>);

        type Lines = Mutex<Vec<u8>>;

        static CAPTURING: AtomicUsize = AtomicUsize::new(0);

        static LOG_SINKS: Mutex<Vec<Weak<Lines>>> = Mutex::new(Vec::new());

        struct Tee;

        impl std::io::Write for Tee {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                for sink in LOG_SINKS.lock().unwrap().iter().filter_map(Weak::upgrade) {
                    sink.lock().unwrap().extend_from_slice(bytes);
                }
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        fn capture_logs() -> Logs {
            static INSTALL: std::sync::Once = std::sync::Once::new();
            INSTALL.call_once(|| {
                use tracing_subscriber::{Layer, layer::SubscriberExt, util::SubscriberInitExt};
                let capturing =
                    tracing_subscriber::filter::filter_fn(|_| CAPTURING.load(Ordering::SeqCst) > 0);
                let layer = tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_writer(|| Tee)
                    .with_filter(crate::log_filter(Some("trace"), true))
                    .with_filter(capturing);
                tracing_subscriber::registry()
                    .with(layer)
                    .try_init()
                    .expect("the bot tests own the process's log subscriber");
            });
            let buffer = Arc::new(Mutex::new(Vec::new()));
            LOG_SINKS.lock().unwrap().push(Arc::downgrade(&buffer));
            CAPTURING.fetch_add(1, Ordering::SeqCst);
            Logs(buffer)
        }

        impl Logs {
            /// Every captured line holds no part of `token`, and the bots' own lines were
            /// captured.
            fn assert_kept_secret(&self, token: &str) {
                let lines = String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned();
                assert!(
                    lines.contains("Bot run started") && lines.contains("Bot run's flow started"),
                    "the capture saw the runs"
                );
                assert_no_token(token, "a log line", &lines);
            }
        }

        impl Drop for Logs {
            fn drop(&mut self) {
                CAPTURING.fetch_sub(1, Ordering::SeqCst);
            }
        }

        /// The bots of a service with one bot event, prepared as a real start prepares them
        /// and served by a provider connection behind a gate the test moves.
        struct Service {
            root: tempfile::TempDir,
            probe: Arc<BotProbe>,
            directory: PathBuf,
            gate: watch::Sender<schedule::Gate>,
            ready: Option<oneshot::Sender<()>>,
            stop: CancellationToken,
            task: tokio::task::JoinHandle<Result<()>>,
        }

        impl Service {
            /// Before Ready, with the gate at `gate`.
            async fn start(
                event_type: &str,
                token: &str,
                connector: Arc<dyn Connector>,
                gate: schedule::Gate,
            ) -> Self {
                let root = tempfile::tempdir().unwrap();
                let mut fixture = fixture(root.path(), event_type, json!({})).await;
                store_token(&mut fixture.config, &json_string(token));
                let bot = prepare(&fixture.config, fixture.invocation.clone(), true).unwrap();
                let context = context(&fixture, root.path());
                let directory = context.state_dir.clone();
                prepare_state(std::slice::from_ref(&bot), &context).unwrap();
                let (gate, gates) = self::gate(gate);
                let (ready, ready_receiver) = oneshot::channel();
                let stop = CancellationToken::new();
                let bots = vec![bot];
                let served = serve(
                    bots,
                    context,
                    gates,
                    connector,
                    ready_receiver,
                    stop.clone(),
                );
                Self {
                    root,
                    probe: fixture.probe,
                    directory,
                    gate,
                    ready: Some(ready),
                    stop,
                    task: tokio::spawn(served),
                }
            }

            fn ready(&mut self) {
                self.ready.take().unwrap().send(()).unwrap();
            }

            fn state(&self) -> Value {
                bot_state(&self.directory)
            }

            async fn stopped(self) -> Cycle {
                self.stop.cancel();
                tokio::time::timeout(Duration::from_secs(30), self.task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                Cycle {
                    state: std::fs::read(self.directory.join(STATE_FILE)).unwrap(),
                    probe: self.probe,
                    root: self.root,
                }
            }
        }

        /// What one bot's whole cycle left: the service's data root, the flow and its state.
        struct Cycle {
            root: tempfile::TempDir,
            probe: Arc<BotProbe>,
            state: Vec<u8>,
        }

        /// One message-to-answer cycle of a bot event of `event_type` with `token`, served by
        /// `connector` until `answered`; then the service stops.
        async fn cycle(
            event_type: &str,
            token: &str,
            connector: Arc<dyn Connector>,
            answered: impl Fn() -> bool,
        ) -> Cycle {
            let open = schedule::Gate::Armed { since: None };
            let mut service = Service::start(event_type, token, connector, open).await;
            service.ready();
            until("the answer was sent", answered).await;
            until("the run is recorded", || {
                service.state()["bots"][EVENT]["last"]["outcome"] == "succeeded"
            })
            .await;
            service.stopped().await
        }

        /// No state file, row fact, run payload or other file of the service holds `token`.
        fn assert_kept_secret(token: &str, cycle: &Cycle, case: &str) {
            let place = |what: &str| format!("{what} ({case})");
            let text = String::from_utf8_lossy(&cycle.state);
            assert_no_token(token, &place("the state file"), &text);
            let state: Value = serde_json::from_slice(&cycle.state).unwrap();
            assert_eq!(state["bots"][EVENT]["runs"], 1, "{}", place("the runs"));
            for row in row_facts(&cycle.state) {
                assert_eq!(row["bots"][0]["event_id"], EVENT, "{}", place("the row"));
                assert_no_token(token, &place("a row fact"), &row.to_string());
            }
            let payloads = cycle.probe.payloads.lock().unwrap().clone();
            assert_eq!(payloads.len(), 1, "{}", place("the flow's runs"));
            assert_eq!(
                payloads[0]["local_session"]["bot_token"],
                handle(EVENT),
                "{}",
                place("the handle")
            );
            assert_no_token(token, &place("the run payload"), &payloads[0].to_string());
            assert_eq!(
                files_with_token(cycle.root.path(), token),
                Vec::<PathBuf>::new(),
                "{}",
                place("the service's files")
            );
        }

        /// A Telegram Bot API on a local port: one private message, then nothing; answers
        /// are taken with `send_status`.
        #[cfg(feature = "bots-telegram")]
        struct FakeTelegram {
            send_status: u16,
            methods: Mutex<Vec<String>>,
            delivered: AtomicBool,
        }

        #[cfg(feature = "bots-telegram")]
        impl FakeTelegram {
            const UPDATE: i64 = 1001;

            async fn start(send_status: u16) -> (Arc<Self>, String) {
                let fake = Arc::new(Self {
                    send_status,
                    methods: Mutex::new(Vec::new()),
                    delivered: AtomicBool::new(false),
                });
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let address = listener.local_addr().unwrap();
                let api = fake.clone();
                let router = axum::Router::new().fallback(
                    move |uri: axum::http::Uri, body: axum::body::Bytes| {
                        let api = api.clone();
                        async move { api.answer(uri.path(), &body).await }
                    },
                );
                tokio::spawn(async move { axum::serve(listener, router).await });
                (fake, format!("http://{address}/"))
            }

            fn methods(&self) -> Vec<String> {
                self.methods.lock().unwrap().clone()
            }

            fn answered(&self) -> bool {
                self.methods()
                    .iter()
                    .any(|method| method == "sendmessage" || method == "editmessagetext")
            }

            async fn answer(
                &self,
                path: &str,
                body: &[u8],
            ) -> (axum::http::StatusCode, axum::Json<Value>) {
                let method = path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                self.methods.lock().unwrap().push(method.clone());
                let answer = matches!(method.as_str(), "sendmessage" | "editmessagetext");
                if answer && self.send_status != 200 {
                    let status = axum::http::StatusCode::from_u16(self.send_status).unwrap();
                    let description = status.canonical_reason().unwrap_or("Refused");
                    let refusal = json!({"ok": false, "error_code": self.send_status,
                        "description": description});
                    return (status, axum::Json(refusal));
                }
                let result = match method.as_str() {
                    "getme" => Self::me(),
                    "getupdates" => self.updates(body).await,
                    _ if answer => json!({"message_id": 100, "date": now(), "chat": Self::chat(),
                        "from": Self::me(), "text": "pong"}),
                    _ => json!(true),
                };
                let body = json!({"ok": true, "result": result});
                (axum::http::StatusCode::OK, axum::Json(body))
            }

            fn me() -> Value {
                json!({"id": 123456789, "is_bot": true, "first_name": "Helper",
                    "username": "helper_bot", "can_join_groups": true,
                    "can_read_all_group_messages": false, "supports_inline_queries": false,
                    "can_connect_to_business": false, "has_main_web_app": false})
            }

            fn chat() -> Value {
                json!({"id": 42, "type": "private", "first_name": "Ada", "username": "ada"})
            }

            /// The one message at the first poll that asks for it, then a short empty poll.
            async fn updates(&self, request: &[u8]) -> Value {
                let offset = serde_json::from_slice::<Value>(request)
                    .ok()
                    .and_then(|request| request["offset"].as_i64())
                    .unwrap_or(0);
                if offset <= Self::UPDATE && !self.delivered.swap(true, Ordering::SeqCst) {
                    let from =
                        json!({"id": 42, "is_bot": false, "first_name": "Ada", "username": "ada"});
                    return json!([{"update_id": Self::UPDATE, "message": {"message_id": 7,
                        "date": now() - 2, "chat": Self::chat(), "from": from, "text": "ping"}}]);
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
                json!([])
            }
        }

        /// The real Telegram connection, pointed at a fake Bot API with short pauses.
        #[cfg(feature = "bots-telegram")]
        struct FakeApi(String);

        #[cfg(feature = "bots-telegram")]
        impl Connector for FakeApi {
            fn connect(
                &self,
                provider: Provider,
                token: &BotToken,
                specs: &[BotSpec],
            ) -> Option<Arc<dyn flow_like_bots::Connection>> {
                (provider == Provider::Telegram).then(|| {
                    flow_like_bots::telegram::connect_to(
                        Some(&self.0),
                        flow_like_bots::telegram::Timing {
                            long_poll_secs: 1,
                            blocked_pause: Duration::from_secs(1),
                        },
                        token,
                        specs,
                    )
                })
            }
        }

        #[cfg(feature = "bots-telegram")]
        #[tokio::test]
        async fn nothing_connects_before_ready_and_an_open_gate() {
            use schedule::{Gate::Held, HoldReason};
            let (fake, url) = FakeTelegram::start(200).await;
            let api = Arc::new(FakeApi(url));
            let undecided = schedule::Gate::Undecided;
            let mut service = Service::start("telegram", TELEGRAM_TOKEN, api, undecided).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert!(fake.methods().is_empty(), "nothing before Ready");
            assert_eq!(service.state()["decided"], false);

            service.ready();
            tokio::time::sleep(Duration::from_millis(300)).await;
            let silent = fake.methods().is_empty();
            assert!(silent, "nothing while the gate is undecided");

            service.gate.send(Held(HoldReason::NotReleased)).unwrap();
            until("the hold is recorded", || {
                let state = service.state();
                state["decided"] == true && state["bots"][EVENT]["hold"] == "not_released"
            })
            .await;
            assert_eq!(service.state()["bots"][EVENT]["state"], "waiting");
            assert!(fake.methods().is_empty(), "nothing while the bot is held");

            let open = schedule::Gate::Armed { since: Some(1) };
            service.gate.send(open).unwrap();
            until("the bot is connected", || {
                service.state()["bots"][EVENT]["state"] == "connected"
            })
            .await;
            assert!(fake.methods().iter().any(|method| method == "getme"));
            assert_eq!(service.state()["bots"][EVENT]["bot_name"], "helper_bot");

            service.gate.send(Held(HoldReason::RunsElsewhere)).unwrap();
            until("the bot let go", || {
                service.state()["bots"][EVENT]["hold"] == "runs_elsewhere"
            })
            .await;
            let running = !service.task.is_finished();
            assert!(running, "a held bot keeps the service up");
            service.stopped().await;
        }

        /// The fake Bot API takes the answer (200), refuses the bot's token (401) or fails
        /// (500).
        #[cfg(feature = "bots-telegram")]
        #[tokio::test]
        async fn a_telegram_run_keeps_the_token_out_of_everything_it_leaves() {
            let logs = capture_logs();
            for send_status in [200, 401, 500] {
                let (fake, url) = FakeTelegram::start(send_status).await;
                let answered = fake.clone();
                let cycle = cycle(
                    "telegram",
                    TELEGRAM_TOKEN,
                    Arc::new(FakeApi(url)),
                    move || answered.answered(),
                )
                .await;
                assert_kept_secret(TELEGRAM_TOKEN, &cycle, &format!("answer {send_status}"));
            }
            logs.assert_kept_secret(TELEGRAM_TOKEN);
        }

        /// Discord's bot id 123456789012345678, as the first part of `DISCORD_TOKEN` names it.
        #[cfg(feature = "bots-discord")]
        const DISCORD_BOT: u64 = 123_456_789_012_345_678;

        /// A Discord connection without a gateway: it reports the bot and one direct
        /// message, builds the run's payload with the provider's own builder and has its
        /// answers taken or refused.
        #[cfg(feature = "bots-discord")]
        struct DirectMessage {
            refuse: bool,
            shown: Arc<Mutex<Vec<bool>>>,
        }

        #[cfg(feature = "bots-discord")]
        struct Answers {
            refuse: bool,
            shown: Arc<Mutex<Vec<bool>>>,
        }

        #[cfg(feature = "bots-discord")]
        #[async_trait]
        impl ReplySink for Answers {
            async fn show(&mut self, _answer: &Answer, done: bool) -> Result<(), SendError> {
                self.shown.lock().unwrap().push(done);
                if self.refuse {
                    Err(SendError::Failed)
                } else {
                    Ok(())
                }
            }

            fn edit_interval(&self, _shown: u32) -> Duration {
                Duration::ZERO
            }
        }

        #[cfg(feature = "bots-discord")]
        #[async_trait]
        impl flow_like_bots::Connection for DirectMessage {
            async fn serve(
                &self,
                intake: flow_like_bots::Intake,
                stop: CancellationToken,
            ) -> flow_like_bots::End {
                intake.connected(DISCORD_BOT, Some("helper"));
                let message = json!({"id": "1000", "channel_id": "300",
                    "author": {"id": "500", "username": "alice", "discriminator": "0"},
                    "content": "ping", "timestamp": "2026-10-02T10:00:00.000000+00:00",
                    "edited_timestamp": null, "tts": false, "mention_everyone": false,
                    "mentions": [], "mention_roles": [], "attachments": [], "embeds": [],
                    "pinned": false, "type": 0});
                let message = serde_json::from_value(message).expect("a Discord message");
                intake
                    .message(flow_like_bots::discord::facts(message, None))
                    .await;
                stop.cancelled().await;
                flow_like_bots::End::Stopped
            }

            async fn payload(
                &self,
                message: &flow_like_bots::Message,
                handle: &str,
                _images: &mut flow_like_bots::limits::ImageBudget,
            ) -> Value {
                let native = message.native().expect("the Discord message");
                flow_like_bots::discord::chat_payload(native, &[], handle, None)
            }

            fn reply(&self, _message: &flow_like_bots::Message) -> Box<dyn ReplySink> {
                Box::new(Answers {
                    refuse: self.refuse,
                    shown: self.shown.clone(),
                })
            }

            async fn notice(&self, _message: &flow_like_bots::Message, _text: &str) {}
        }

        #[cfg(feature = "bots-discord")]
        struct Gatewayless(Arc<DirectMessage>);

        #[cfg(feature = "bots-discord")]
        impl Connector for Gatewayless {
            fn connect(
                &self,
                provider: Provider,
                _token: &BotToken,
                _specs: &[BotSpec],
            ) -> Option<Arc<dyn flow_like_bots::Connection>> {
                (provider == Provider::Discord)
                    .then(|| self.0.clone() as Arc<dyn flow_like_bots::Connection>)
            }
        }

        /// The answer is taken, or refused by Discord.
        #[cfg(feature = "bots-discord")]
        #[tokio::test]
        async fn a_discord_run_keeps_the_token_out_of_everything_it_leaves() {
            let logs = capture_logs();
            for refuse in [false, true] {
                let connection = Arc::new(DirectMessage {
                    refuse,
                    shown: Arc::default(),
                });
                let shown = connection.shown.clone();
                let cycle = cycle(
                    "discord",
                    DISCORD_TOKEN,
                    Arc::new(Gatewayless(connection)),
                    move || shown.lock().unwrap().contains(&true),
                )
                .await;
                let case = if refuse {
                    "answer refused"
                } else {
                    "answer taken"
                };
                assert_kept_secret(DISCORD_TOKEN, &cycle, case);
            }
            logs.assert_kept_secret(DISCORD_TOKEN);
        }
    }
}
