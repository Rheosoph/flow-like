//! One run of a prepared event that no request started: a scheduled time, a person's "Run
//! now", a bot's message. It makes the calls a hosted request makes
//! (`hosting::invoke_inner`) without the request's own parts.
// Its callers are the parts a build may leave out.
#![cfg_attr(not(any(feature = "on-demand", feature = "bots")), allow(dead_code))]

use std::{sync::Arc, time::Duration};

use flow_like_runtime::{
    app::AppVisibility,
    flow::execution::{InternalRun, LogLevel, RunPayload, RunStatus},
    profile::Profile,
    state::FlowLikeState,
};
use flow_like_types::{
    channel::{Channel, InProcessChannel},
    intercom::{InterComCallback, InterComEvent},
};
use futures_util::FutureExt;
use serde_json::Value;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::hosting::{CloseChannelOnDrop, PreparedInvocation};

/// A run that was told to end and does not end by itself is dropped after this long.
const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// What every run of a placement process shares.
pub(crate) struct RunContext {
    pub(crate) project_id: String,
    pub(crate) state: Arc<FlowLikeState>,
    pub(crate) profile: Profile,
    pub(crate) visibility: AppVisibility,
    /// The approval's delegating user of an online app; `None` runs as the local user.
    pub(crate) execution_sub: Option<String>,
}

pub(crate) struct RunRequest {
    pub(crate) payload: Option<Value>,
    /// Gets `run_initiated` first, then every event of the run.
    pub(crate) callback: InterComCallback,
    pub(crate) cancel: CancellationToken,
    /// A new id when `None`.
    pub(crate) run_id: Option<String>,
    /// Counted from the call, preparation included.
    pub(crate) time_limit: Duration,
    pub(crate) request_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunEnd {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

/// Runs `prepared` once. Usage is counted here, so a caller does not count the run itself.
/// Nothing of the run's input or error text is logged: the run's own log has it.
pub(crate) async fn run_event_once(
    context: &RunContext,
    prepared: &PreparedInvocation,
    request: RunRequest,
) -> RunEnd {
    let invocation = crate::usage::begin(request.request_bytes);
    let deadline = Instant::now() + request.time_limit;
    let cancel = request.cancel.child_token();
    let run_id = request
        .run_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let event_id = prepared.event.id.as_str();
    let started = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(RunEnd::Cancelled),
        _ = tokio::time::sleep_until(deadline) => Err(RunEnd::TimedOut),
        started = start(context, prepared, &run_id, request.payload, request.callback, request.time_limit) => {
            started.ok_or_else(|| {
                tracing::warn!(event_id, run_id = %run_id, "Run could not be prepared");
                RunEnd::Failed
            })
        }
    };
    let end = match started {
        Ok((mut run, _close_channel)) => execute(context, &mut run, &cancel, deadline).await,
        Err(end) => end,
    };
    invocation.finish(match end {
        RunEnd::Succeeded => crate::usage::Outcome::Succeeded,
        RunEnd::Failed => crate::usage::Outcome::Failed,
        RunEnd::Cancelled | RunEnd::TimedOut => crate::usage::Outcome::Cancelled,
    });
    end
}

/// The run with its identity and policies, after `run_initiated` reached the callback.
async fn start(
    context: &RunContext,
    prepared: &PreparedInvocation,
    run_id: &str,
    payload: Option<Value>,
    callback: InterComCallback,
    time_limit: Duration,
) -> Option<(InternalRun, CloseChannelOnDrop)> {
    let channel = InProcessChannel::register(run_id.to_owned(), time_limit).await;
    let close_channel = CloseChannelOnDrop(channel.clone());
    let callback = counted(callback);
    if let Some(callback) = &callback {
        callback(InterComEvent::with_type(
            "run_initiated",
            serde_json::json!({"run_id": run_id, "channel": channel.handle()}),
        ))
        .await
        .ok()?;
    }
    let mut run = InternalRun::from_template(
        &context.project_id,
        prepared.template.clone(),
        Some(prepared.event.clone()),
        &context.state,
        &context.profile,
        &RunPayload {
            id: prepared.event.node_id.clone(),
            payload,
            runtime_variables: None,
            filter_secrets: Some(false),
        },
        false,
        callback,
        None,
        None,
        Default::default(),
        Some(run_id.to_owned()),
        Some(channel),
    )
    .await
    .ok()?;
    identify(context, &mut run).await;
    run.set_cancellation_log("Standalone run stopped", LogLevel::Info);
    run.set_log_flush_policy(Duration::from_secs(5), 500)
        .await
        .ok()?;
    Some((run, close_channel))
}

/// Every run of a placement has its identity: the approval's delegating user of an online
/// app, the local user otherwise; hosted models are attributed to the app.
async fn identify(context: &RunContext, run: &mut InternalRun) {
    run.set_usage_attribution_from_visibility(&context.visibility)
        .await;
    match &context.execution_sub {
        Some(sub) => {
            run.set_execution_sub(sub.clone()).await;
            run.set_unresolved_user_context().await;
        }
        None => run.set_offline_user_context(),
    }
}

async fn execute(
    context: &RunContext,
    run: &mut InternalRun,
    cancel: &CancellationToken,
    deadline: Instant,
) -> RunEnd {
    run.set_cancellation_token(cancel.clone());
    let ended = {
        let execution = run.execute(context.state.clone());
        tokio::pin!(execution);
        let ended = tokio::select! {
            biased;
            _ = &mut execution => None,
            _ = cancel.cancelled() => Some(RunEnd::Cancelled),
            _ = tokio::time::sleep_until(deadline) => Some(RunEnd::TimedOut),
        };
        if ended.is_some() {
            cancel.cancel();
            let _ = tokio::time::timeout(CANCEL_GRACE, &mut execution).await;
        }
        ended
    };
    match ended {
        Some(end) => end,
        None if matches!(run.get_status().await, RunStatus::Success) => RunEnd::Succeeded,
        None if cancel.is_cancelled() => RunEnd::Cancelled,
        None => RunEnd::Failed,
    }
}

/// Every event of a run counts as a runtime message of the service, as for a request.
fn counted(callback: InterComCallback) -> InterComCallback {
    Some(Arc::new(move |event: InterComEvent| {
        crate::usage::runtime_message();
        match &callback {
            Some(callback) => callback(event),
            None => futures_util::future::ready(Ok(())).boxed(),
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_runtime::{
        app::App,
        flow::{
            board::Board,
            compiled::TemplateCache,
            execution::context::ExecutionContext,
            node::{Node, NodeLogic},
            variable::VariableType,
        },
    };
    use flow_like_storage::Path as StorePath;
    use std::sync::Mutex;
    use tokio::sync::Notify;

    #[derive(Default)]
    struct Probe {
        entered: Notify,
    }

    #[flow_like_types::async_trait]
    impl NodeLogic for Probe {
        fn get_node(&self) -> Node {
            let mut node = Node::new("standalone_test_run_once", "Run once", "", "Tests");
            node.set_start(true);
            node.add_input_pin("exec_in", "Execute", "", VariableType::Execution);
            node
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            let payload = context.get_payload().await?.payload.clone();
            self.entered.notify_one();
            match payload
                .as_ref()
                .and_then(|payload| payload["mode"].as_str())
            {
                Some("fail") => Err(flow_like_types::anyhow!("probe failed")),
                Some("hang") => std::future::pending().await,
                _ => Ok(()),
            }
        }
    }

    async fn prepared(root: &std::path::Path) -> (RunContext, PreparedInvocation, Arc<Probe>) {
        let (config, state, _, _) = crate::runtime::tests::fixture(root).await;
        let probe = Arc::new(Probe::default());
        state.node_registry.write().await.push_node(probe.clone());
        let app = App::load(config.project_id.clone(), state.clone())
            .await
            .unwrap();
        let mut board = Board::new(
            Some("once-board".into()),
            StorePath::from("apps/project"),
            state.clone(),
        );
        let mut node = probe.get_node();
        node.id = "once".into();
        board.nodes.insert(node.id.clone(), node);
        board.snapshot_at_version((1, 0, 0), None).await.unwrap();
        let mut event = app.get_event("event", Some((1, 0, 0))).await.unwrap();
        event.id = "once".into();
        event.board_id = board.id.clone();
        event.node_id = "once".into();
        event.event_type = "quick_action".into();
        let template = TemplateCache::default()
            .resolve(&state, &app.id, &board.id, Some((1, 0, 0)), None, "")
            .await
            .unwrap();
        let context = RunContext {
            project_id: app.id.clone(),
            state,
            profile: Profile::default(),
            visibility: app.visibility.clone(),
            execution_sub: None,
        };
        let prepared = PreparedInvocation {
            event,
            template,
            action_admission: None,
        };
        (context, prepared, probe)
    }

    type Recorded = Arc<Mutex<Vec<InterComEvent>>>;

    fn recording() -> (InterComCallback, Recorded) {
        let events = Recorded::default();
        let sink = events.clone();
        let callback: InterComCallback = Some(Arc::new(move |event: InterComEvent| {
            sink.lock().unwrap().push(event);
            futures_util::future::ready(Ok(())).boxed()
        }));
        (callback, events)
    }

    fn asking(mode: &str, callback: InterComCallback, cancel: CancellationToken) -> RunRequest {
        RunRequest {
            payload: Some(serde_json::json!({"mode": mode})),
            callback,
            cancel,
            run_id: Some(uuid::Uuid::new_v4().to_string()),
            time_limit: Duration::from_secs(30),
            request_bytes: 0,
        }
    }

    async fn closed(run_id: &str) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while InProcessChannel::lookup(run_id).await.is_some() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("the channel of run {run_id} stayed registered"));
    }

    #[tokio::test]
    async fn a_run_announces_itself_first_and_ends_with_its_flow() {
        let directory = tempfile::tempdir().unwrap();
        let (context, prepared, _) = prepared(directory.path()).await;
        for (mode, end) in [("succeed", RunEnd::Succeeded), ("fail", RunEnd::Failed)] {
            let (callback, events) = recording();
            let request = asking(mode, callback, CancellationToken::new());
            let run_id = request.run_id.clone().unwrap();
            assert_eq!(run_event_once(&context, &prepared, request).await, end);
            let events = events.lock().unwrap().clone();
            assert_eq!(events[0].event_type, "run_initiated", "{mode}");
            assert_eq!(events[0].payload["run_id"], run_id.as_str());
            assert_eq!(events[0].payload["channel"]["channel_id"], run_id.as_str());
            closed(&run_id).await;
        }
    }

    #[tokio::test]
    async fn a_run_without_a_callback_or_an_id_still_runs() {
        let directory = tempfile::tempdir().unwrap();
        let (context, prepared, _) = prepared(directory.path()).await;
        let mut request = asking("succeed", None, CancellationToken::new());
        request.run_id = None;
        assert_eq!(
            run_event_once(&context, &prepared, request).await,
            RunEnd::Succeeded
        );
    }

    #[tokio::test]
    async fn a_cancelled_run_ends_as_cancelled_and_closes_its_channel() {
        let directory = tempfile::tempdir().unwrap();
        let (context, prepared, probe) = prepared(directory.path()).await;
        let cancel = CancellationToken::new();
        let request = asking("hang", None, cancel.clone());
        let run_id = request.run_id.clone().unwrap();
        let entered = probe.entered.notified();
        let run = run_event_once(&context, &prepared, request);
        tokio::pin!(run);
        tokio::select! {
            _ = &mut run => panic!("a hanging flow ended by itself"),
            _ = entered => {}
        }
        cancel.cancel();
        let end = tokio::time::timeout(CANCEL_GRACE + Duration::from_secs(5), run)
            .await
            .expect("a cancelled run ends");
        assert_eq!(end, RunEnd::Cancelled);
        closed(&run_id).await;

        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let request = asking("succeed", None, cancelled);
        let run_id = request.run_id.clone().unwrap();
        assert_eq!(
            run_event_once(&context, &prepared, request).await,
            RunEnd::Cancelled
        );
        closed(&run_id).await;
    }

    #[tokio::test]
    async fn a_run_past_its_time_limit_is_cancelled_as_timed_out() {
        let directory = tempfile::tempdir().unwrap();
        let (context, prepared, _) = prepared(directory.path()).await;
        let cancel = CancellationToken::new();
        let mut request = asking("hang", None, cancel.clone());
        request.time_limit = Duration::from_millis(300);
        let run_id = request.run_id.clone().unwrap();
        let started = std::time::Instant::now();
        assert_eq!(
            run_event_once(&context, &prepared, request).await,
            RunEnd::TimedOut
        );
        assert!(
            started.elapsed() < Duration::from_millis(300) + CANCEL_GRACE + Duration::from_secs(3)
        );
        assert!(!cancel.is_cancelled(), "the caller's token stays its own");
        closed(&run_id).await;
    }

    #[tokio::test]
    async fn a_run_is_counted_once() {
        const MARK: u64 = 1 << 40;
        let directory = tempfile::tempdir().unwrap();
        let (context, prepared, _) = prepared(directory.path()).await;
        let before = crate::usage::snapshot();
        let mut request = asking("succeed", None, CancellationToken::new());
        request.request_bytes = MARK;
        assert_eq!(
            run_event_once(&context, &prepared, request).await,
            RunEnd::Succeeded
        );
        let after = crate::usage::snapshot();
        assert_eq!(
            (after.request_payload_bytes - before.request_payload_bytes) / MARK,
            1
        );
        assert!(after.invocations_succeeded > before.invocations_succeeded);
        assert!(after.runtime_messages > before.runtime_messages);
    }
}
