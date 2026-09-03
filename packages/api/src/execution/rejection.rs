//! Durable record for triggers that never produced an execution.
//!
//! Every invoke, sink trigger and inbound call funnels into one
//! `execution_run` insert. Anything that fails before that insert — a payload
//! that does not match the event contract, a cron whose PAT expired, a REST
//! path with no registration — used to exist only as a `tracing::warn!` and an
//! HTTP status the caller may never look at.
//!
//! [`record`] gives those attempts the same two homes a real run has:
//! an `ExecutionRun` row (so they show up in `GET /apps/{id}/board/{id}/runs`)
//! and a LanceDB per-run table holding one `Fatal` log message with the reason
//! (so `GET /apps/{id}/board/{id}/logs` explains what happened). It is
//! best-effort by construction: a failure to record a rejection must never
//! change the response the caller already earned.

use super::{format_run_version, normalize_run_version_label};
use crate::credentials::CredentialsAccess;
use crate::entity::execution_run;
use crate::entity::sea_orm_active_enums::{RunMode, RunStatus, RunVariant};
use crate::state::AppState;
use flow_like::flow::execution::rejection::{RejectedRun, record_rejection};

pub use flow_like::flow::execution::rejection::RejectionStage;
use flow_like_types::create_id;
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
};
use serde_json::Value;

/// `LogLevel::Fatal` — a rejected trigger never reached a node, so it is worse
/// than a run that failed inside one.
const FATAL_LOG_LEVEL: i32 = 4;

/// Placeholder actor for rejections that never resolved one.
const SYSTEM_CREDENTIAL_SUBJECT: &str = "system";

/// An orphaned cron schedule can fire every minute forever, and a broken
/// webhook client retries just as hard. Collapse identical rejections of the
/// same event onto one run for this long instead of minting a Lance table per
/// attempt.
const DEDUP_WINDOW_MINUTES: i64 = 15;

/// Identical rejections collapse, but a public REST endpoint can be hit with
/// endlessly *different* bad paths, and each distinct reason would otherwise be
/// a new row and a new Lance table. Past this many rejections of one event
/// inside the window, stop recording and let the log line carry it.
const MAX_REJECTIONS_PER_EVENT: u64 = 20;

/// Everything known about a trigger at the moment it was refused.
#[derive(Debug, Clone)]
pub struct RejectedRunContext {
    pub run_id: String,
    pub app_id: String,
    pub board_id: Option<String>,
    pub event_id: Option<String>,
    pub node_id: Option<String>,
    /// Board version, as stored on the run row.
    pub version: Option<String>,
    pub event_version: Option<String>,
    pub mode: RunMode,
    pub stage: RejectionStage,
    pub reason: String,
    pub payload: Option<Value>,
    pub user_id: Option<String>,
    pub technical_user_id: Option<String>,
    /// Subject the log-store credentials are scoped to. Defaults to `user_id`;
    /// sink triggers pass their `sink:{id}` executor subject here.
    pub credential_subject: Option<String>,
    /// True once a caller supplied the run id, which means the rejection has to
    /// land on that exact run rather than being collapsed onto an earlier one.
    reuses_run_id: bool,
}

impl RejectedRunContext {
    pub fn new(
        app_id: impl Into<String>,
        stage: RejectionStage,
        reason: impl Into<String>,
    ) -> Self {
        RejectedRunContext {
            run_id: create_id(),
            app_id: app_id.into(),
            board_id: None,
            event_id: None,
            node_id: None,
            version: None,
            event_version: None,
            mode: RunMode::Http,
            stage,
            reason: reason.into(),
            payload: None,
            user_id: None,
            technical_user_id: None,
            credential_subject: None,
            reuses_run_id: false,
        }
    }

    /// Reuse an id the caller already minted, so the rejection lands on the
    /// same run the caller was told about.
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self.reuses_run_id = true;
        self
    }

    pub fn with_board(mut self, board_id: impl Into<String>, version: Option<String>) -> Self {
        let board_id = board_id.into();
        self.board_id = (!board_id.is_empty()).then_some(board_id);
        // Event rows carry the dotted board version; the run row's label is
        // canonically `v{major}-{minor}-{patch}`.
        self.version = version.as_deref().map(normalize_run_version_label);
        self
    }

    pub fn with_event(mut self, event_id: impl Into<String>, node_id: Option<String>) -> Self {
        self.event_id = Some(event_id.into());
        self.node_id = node_id;
        self
    }

    pub fn with_event_version(mut self, event_version: Option<String>) -> Self {
        self.event_version = event_version;
        self
    }

    /// Take board, start node, and both versions from the event definition the
    /// caller was trying to trigger.
    pub fn with_event_definition(mut self, event: &flow_like::flow::event::Event) -> Self {
        let (major, minor, patch) = event.event_version;
        self.board_id = (!event.board_id.is_empty()).then(|| event.board_id.clone());
        self.node_id = Some(event.node_id.clone());
        self.event_id = Some(event.id.clone());
        self.event_version = Some(format!("{}.{}.{}", major, minor, patch));
        self.version = event.board_version.map(format_run_version);
        self
    }

    pub fn with_mode(mut self, mode: RunMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn with_actor(
        mut self,
        user_id: Option<String>,
        technical_user_id: Option<String>,
    ) -> Self {
        self.user_id = user_id;
        self.technical_user_id = technical_user_id;
        self
    }

    pub fn with_credential_subject(mut self, subject: impl Into<String>) -> Self {
        self.credential_subject = Some(subject.into());
        self
    }

    pub fn with_payload(mut self, payload: Option<Value>) -> Self {
        self.payload = payload;
        self
    }

    fn payload_len(&self) -> i64 {
        self.payload
            .as_ref()
            .and_then(|value| serde_json::to_string(value).ok())
            .map(|serialized| serialized.len() as i64)
            .unwrap_or(0)
    }
}

/// Persist a refused trigger. Returns the run id it was recorded under so the
/// caller can hand it back to the client; errors are logged, never propagated.
pub async fn record(state: &AppState, context: RejectedRunContext) -> String {
    let run_id = context.run_id.clone();

    match suppression(state, &context).await {
        Ok(Suppression::Fold(existing)) => {
            tracing::debug!(
                run_id = %existing,
                app_id = %context.app_id,
                stage = context.stage.as_str(),
                "Trigger rejected again for the same reason; folding into the existing run"
            );
            return existing;
        }
        Ok(Suppression::Throttled(latest)) => {
            tracing::warn!(
                app_id = %context.app_id,
                event_id = context.event_id.as_deref().unwrap_or(""),
                stage = context.stage.as_str(),
                reason = %context.reason,
                cap = MAX_REJECTIONS_PER_EVENT,
                window_minutes = DEDUP_WINDOW_MINUTES,
                "Event has hit its rejection cap for the window; not recording another run"
            );
            return latest;
        }
        Ok(Suppression::Record) => {}
        Err(error) => tracing::warn!(
            error = %error,
            app_id = %context.app_id,
            "Could not check for a repeated rejection; recording a new run"
        ),
    }

    if let Err(error) = record_run_row(state, &context).await {
        tracing::error!(
            error = %error,
            run_id = %run_id,
            app_id = %context.app_id,
            stage = context.stage.as_str(),
            "Failed to persist rejected run"
        );
    }

    if let Err(error) = record_run_logs(state, &context).await {
        tracing::warn!(
            error = %error,
            run_id = %run_id,
            app_id = %context.app_id,
            "Failed to persist the reason for a rejected run"
        );
    }

    tracing::warn!(
        run_id = %run_id,
        app_id = %context.app_id,
        board_id = context.board_id.as_deref().unwrap_or(""),
        event_id = context.event_id.as_deref().unwrap_or(""),
        stage = context.stage.as_str(),
        reason = %context.reason,
        "Trigger rejected before execution"
    );

    run_id
}

enum Suppression {
    /// Nothing comparable recently; write the run.
    Record,
    /// The same rejection is already on record; reuse that run.
    Fold(String),
    /// This event has produced too many distinct rejections to keep recording;
    /// carries the newest one so the caller still names a run that exists.
    Throttled(String),
}

/// Decide whether this rejection earns its own run. An identical one inside the
/// window folds into the run already on record (its `updated_at` is bumped, so
/// the run still shows the failure is ongoing); once an event has recorded
/// `MAX_REJECTIONS_PER_EVENT` of them, further ones are dropped.
async fn suppression(
    state: &AppState,
    context: &RejectedRunContext,
) -> flow_like_types::Result<Suppression> {
    if context.reuses_run_id {
        return Ok(Suppression::Record);
    }
    let Some(event_id) = context.event_id.as_deref() else {
        return Ok(Suppression::Record);
    };

    let now = chrono::Utc::now().naive_utc();
    let since = now - chrono::Duration::minutes(DEDUP_WINDOW_MINUTES);
    let recent = || {
        execution_run::Entity::find()
            .filter(execution_run::Column::AppId.eq(&context.app_id))
            .filter(execution_run::Column::EventId.eq(event_id))
            .filter(execution_run::Column::CurrentStep.eq(context.stage.operation_id()))
            .filter(execution_run::Column::CreatedAt.gte(since))
    };

    let existing = recent()
        .filter(execution_run::Column::ErrorMessage.eq(context.reason.clone()))
        .order_by_desc(execution_run::Column::CreatedAt)
        .one(&state.db)
        .await?;

    if let Some(existing) = existing {
        let bump = execution_run::ActiveModel {
            id: Set(existing.id.clone()),
            updated_at: Set(now),
            ..Default::default()
        };
        bump.update(&state.db).await?;
        return Ok(Suppression::Fold(existing.id));
    }

    if recent().count(&state.db).await? >= MAX_REJECTIONS_PER_EVENT {
        let latest = recent()
            .order_by_desc(execution_run::Column::CreatedAt)
            .one(&state.db)
            .await?
            .map(|run| run.id)
            .unwrap_or_else(|| context.run_id.clone());
        return Ok(Suppression::Throttled(latest));
    }

    Ok(Suppression::Record)
}

/// The list entry. A row may already exist when the trigger was refused *after*
/// the run was created (a dispatch that never left the API); finalize it in
/// place rather than losing the reason to a duplicate-key error.
async fn record_run_row(
    state: &AppState,
    context: &RejectedRunContext,
) -> flow_like_types::Result<()> {
    let now = chrono::Utc::now().naive_utc();
    let existing = execution_run::Entity::find_by_id(&context.run_id)
        .filter(execution_run::Column::AppId.eq(&context.app_id))
        .one(&state.db)
        .await?;

    if existing.is_some() {
        let update = execution_run::ActiveModel {
            id: Set(context.run_id.clone()),
            status: Set(RunStatus::Failed),
            log_level: Set(FATAL_LOG_LEVEL),
            error_message: Set(Some(context.reason.clone())),
            current_step: Set(Some(context.stage.operation_id())),
            completed_at: Set(Some(now)),
            updated_at: Set(now),
            ..Default::default()
        };
        update.update(&state.db).await?;
        return Ok(());
    }

    // `started_at` stays NULL: the run never started, and that is the
    // difference between this and a run that failed while executing.
    let run = execution_run::ActiveModel {
        id: Set(context.run_id.clone()),
        board_id: Set(context.board_id.clone().unwrap_or_default()),
        version: Set(context.version.clone()),
        event_id: Set(context.event_id.clone()),
        node_id: Set(context.node_id.clone()),
        status: Set(RunStatus::Failed),
        mode: Set(context.mode.clone()),
        run_variant: Set(RunVariant::Primary),
        variant_name: Set(None),
        shadow_of_run_id: Set(None),
        regression_run_id: Set(None),
        log_level: Set(FATAL_LOG_LEVEL),
        input_payload_len: Set(context.payload_len()),
        input_payload_key: Set(None),
        output_payload_len: Set(0),
        error_message: Set(Some(context.reason.clone())),
        progress: Set(0),
        current_step: Set(Some(context.stage.operation_id())),
        started_at: Set(None),
        completed_at: Set(Some(now)),
        expires_at: Set(Some(now + chrono::Duration::hours(24))),
        user_id: Set(context.user_id.clone()),
        technical_user_id: Set(context.technical_user_id.clone()),
        caller_app_chain: Set(None),
        trace_id: Set(Some(context.run_id.clone())),
        parent_run_id: Set(None),
        correlation_keys: Set(None),
        app_id: Set(context.app_id.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    };
    run.insert(&state.db).await?;

    Ok(())
}

/// The detail view. Without a board there is no log database to write into —
/// the run row alone still carries the reason in `error_message`.
async fn record_run_logs(
    state: &AppState,
    context: &RejectedRunContext,
) -> flow_like_types::Result<()> {
    let Some(board_id) = context.board_id.as_deref() else {
        return Ok(());
    };

    // `ServerExecute` scopes the log store by app, not by subject, so a trigger
    // that died before any actor was resolved (an orphaned cron firing at a
    // deleted sink) can still write its reason.
    let subject = context
        .credential_subject
        .as_deref()
        .or(context.user_id.as_deref())
        .unwrap_or(SYSTEM_CREDENTIAL_SUBJECT);

    let credentials = state
        .scoped_credentials(subject, &context.app_id, CredentialsAccess::ServerExecute)
        .await
        .map_err(|e| flow_like_types::anyhow!("failed to scope log credentials: {e}"))?;
    let logs_db_builder = credentials.into_shared_credentials().to_logs_db_builder()?;

    let rejection = RejectedRun::new(
        context.app_id.clone(),
        board_id,
        context.stage,
        context.reason.clone(),
    )
    .with_run_id(context.run_id.clone())
    .with_event(
        context.event_id.clone().unwrap_or_default(),
        context.event_version.clone(),
    )
    .with_node(context.node_id.clone().unwrap_or_default())
    .with_version_label(context.version.clone())
    .with_payload(context.payload.as_ref());

    record_rejection(logs_db_builder.as_ref(), &rejection, None).await?;

    Ok(())
}

/// Recover app/board/event context from the event row. Used where the trigger
/// died before the event was ever loaded — an orphaned cron schedule firing at
/// a deleted sink, a sink-type mismatch — because the row is then the only
/// place left that knows which board the caller meant.
pub async fn context_for_event(
    state: &AppState,
    event_id: &str,
    stage: RejectionStage,
    reason: impl Into<String>,
) -> Option<RejectedRunContext> {
    let event = crate::entity::event::Entity::find_by_id(event_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()?;

    Some(
        RejectedRunContext::new(event.app_id, stage, reason)
            .with_board(
                event.board_id.unwrap_or_default(),
                event.board_version.clone(),
            )
            .with_event(event.id, event.node_id)
            .with_event_version(Some(event.event_version)),
    )
}
