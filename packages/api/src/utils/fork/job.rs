//! Staged, resumable forks.
//!
//! A `ForkJob` row is the single "fork in progress" marker for every flow.
//! An online → online fork runs `allocate` → `copy_storage` → `write_rows`
//! → `finalize`; an offline → online upload runs `allocate` → `upload`
//! (the desktop pushes the bundle) → `done`. Every destination id is
//! derived from the job id ([`super::ids::derive_id`]), every storage write
//! is a PUT and every row insert is `ON CONFLICT DO NOTHING`, so a step can
//! be re-run from its cursor after a crash, a lost commit or a worker
//! hand-over without leaving duplicates. No transaction touches more than
//! [`DEFAULT_WRITE_CHUNK`] rows.

use super::{
    CopyCheckpoint, ForkContext, ForkPlan, ForkPolicy, ForkReport, copy_object_prefix_resumable,
    db_schema, delete_object_prefix, ids, materialize_meta, owner_membership_id, plan_event_rows,
    plan_meta_rows, plan_package_rows, plan_page_rows, plan_roles, plan_template_meta_rows,
    plan_template_rows, plan_widget_meta_rows, plan_widget_rows, policy,
};
use crate::{
    db::{
        DEFAULT_WRITE_CHUNK, RetryPolicy, delete_in_batches, insert_in_chunks, retry_transaction,
    },
    entity::{
        app, app_package, event, event_alias, event_remote_auth, event_remote_registration,
        event_setup, event_sink, fork_job, membership, meta, page, role,
        sea_orm_active_enums::{ExecutionMode, Status, Visibility},
        template, widget,
    },
    error::ApiError,
    routes::app::events::db::{decrypt_token, encrypt_token},
    state::AppState,
};
use flow_like_storage::Path;
use flow_like_types::create_id;
use sea_orm::{
    ActiveEnum,
    ActiveValue::{NotSet, Set},
    ColumnTrait, Condition, EntityTrait, IntoActiveModel, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
    sea_query::{Expr, OnConflict, Query},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;
use utoipa::ToSchema;

/// How long a job may live before the sweeper aborts it.
pub const FORK_JOB_TTL: Duration = Duration::from_secs(24 * 60 * 60);
/// A job whose row has not moved for this long is considered abandoned by
/// its previous driver and may be resumed by a worker.
pub const STALE_AFTER: Duration = Duration::from_secs(5 * 60);
/// Forks copying at most this many bytes run inside the request.
pub const SYNC_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Event rows carry the fat JSONB columns; a smaller chunk keeps a
/// transaction under DSQL's 10 MiB write set.
const EVENT_ROW_CHUNK: usize = 200;
const SWEEP_BATCH: u64 = 50;
const RESUME_BATCH: u64 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ForkJobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Aborting,
}

impl ForkJobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Running => "RUNNING",
            Self::Done => "DONE",
            Self::Failed => "FAILED",
            Self::Aborting => "ABORTING",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "QUEUED" => Some(Self::Queued),
            "RUNNING" => Some(Self::Running),
            "DONE" => Some(Self::Done),
            "FAILED" => Some(Self::Failed),
            "ABORTING" => Some(Self::Aborting),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ForkJobStep {
    Allocate,
    CopyStorage,
    WriteRows,
    Finalize,
    Upload,
    Done,
    Abort,
}

impl ForkJobStep {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allocate => "allocate",
            Self::CopyStorage => "copy_storage",
            Self::WriteRows => "write_rows",
            Self::Finalize => "finalize",
            Self::Upload => "upload",
            Self::Done => "done",
            Self::Abort => "abort",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allocate" => Some(Self::Allocate),
            "copy_storage" => Some(Self::CopyStorage),
            "write_rows" => Some(Self::WriteRows),
            "finalize" => Some(Self::Finalize),
            "upload" => Some(Self::Upload),
            "done" => Some(Self::Done),
            "abort" => Some(Self::Abort),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForkJobKind {
    /// Server-side copy of an online source app.
    OnlineCopy,
    /// The desktop uploads a local app into an allocated destination.
    OfflineUpload,
}

/// What the job was asked to do. Stored in `ForkJob.policy`; the only
/// secret it can carry is the remote-event token, kept encrypted with the
/// same key as sink PATs and dropped again at finalize.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForkJobSpec {
    pub kind: ForkJobKind,
    pub language: String,
    pub visibility: Visibility,
    pub policy: ForkPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_event_token_encrypted: Option<String>,
}

impl ForkJobSpec {
    pub fn online_copy(
        state: &AppState,
        src_app_row: &app::Model,
        language: &str,
        remote_event_token: Option<&str>,
        visibility: Visibility,
    ) -> Self {
        Self {
            kind: ForkJobKind::OnlineCopy,
            language: language.to_string(),
            visibility,
            policy: ForkPolicy::from_app_row(src_app_row),
            remote_event_token_encrypted: remote_event_token
                .map(|token| encrypt_token(token, &state.encryption_key)),
        }
    }

    pub fn offline_upload(language: &str) -> Self {
        Self {
            kind: ForkJobKind::OfflineUpload,
            language: language.to_string(),
            visibility: Visibility::Private,
            policy: ForkPolicy::default(),
            remote_event_token_encrypted: None,
        }
    }

    pub fn remote_event_token(&self, state: &AppState) -> Option<String> {
        self.remote_event_token_encrypted
            .as_deref()
            .and_then(|encrypted| decrypt_token(encrypted, &state.encryption_key))
    }

    fn parse(job: &fork_job::Model) -> Result<Self, ApiError> {
        serde_json::from_value(job.policy.clone()).map_err(|e| {
            ApiError::internal(format!("fork job {} has an unreadable spec: {e}", job.id))
        })
    }

    fn without_secrets(&self) -> Self {
        Self {
            remote_event_token_encrypted: None,
            ..self.clone()
        }
    }
}

/// Progress inside a step. `prefix` / `last_key` resume a content mirror,
/// `done` lists the sub-steps (mirrors, row tables) already committed.
#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema)]
pub struct ForkJobCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_key: Option<String>,
    #[serde(default)]
    pub done: Vec<String>,
    #[serde(default)]
    pub bytes_copied: u64,
    #[serde(default)]
    pub objects_copied: u64,
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl ForkJobCursor {
    fn parse(job: &fork_job::Model) -> Self {
        job.cursor
            .clone()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default()
    }

    fn is_done(&self, marker: &str) -> bool {
        self.done.iter().any(|d| d == marker)
    }

    fn mark_done(&mut self, marker: &str) {
        if !self.is_done(marker) {
            self.done.push(marker.to_string());
        }
        self.prefix = None;
        self.last_key = None;
    }
}

/// The wire shape of a job, for `GET /apps/fork/jobs/{id}` and the `202`
/// body of `POST /apps/{id}/fork`.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ForkJobView {
    pub job_id: String,
    pub source_app_id: String,
    /// Destination app id. The app exists (hidden) from `allocate` on.
    pub new_app_id: String,
    /// `QUEUED`, `RUNNING`, `DONE`, `FAILED` or `ABORTING`.
    pub status: String,
    /// `allocate`, `copy_storage`, `write_rows`, `finalize`, `upload`,
    /// `done` or `abort`.
    pub step: String,
    pub bytes_copied: u64,
    pub objects_copied: u64,
    /// Present once the job is `DONE`. Carries the top-level id maps only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<ForkReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl From<&fork_job::Model> for ForkJobView {
    fn from(job: &fork_job::Model) -> Self {
        let cursor = ForkJobCursor::parse(job);
        let report: Option<ForkReport> = job
            .report
            .clone()
            .and_then(|value| serde_json::from_value(value).ok());
        let utc = |naive: chrono::NaiveDateTime| {
            chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
        };
        Self {
            job_id: job.id.clone(),
            source_app_id: job.source_app_id.clone(),
            new_app_id: job.dest_app_id.clone(),
            status: job.status.clone(),
            step: job.step.clone(),
            bytes_copied: report
                .as_ref()
                .map(|r| r.bytes_copied)
                .unwrap_or(cursor.bytes_copied),
            objects_copied: report
                .as_ref()
                .map(|r| r.objects_copied)
                .unwrap_or(cursor.objects_copied),
            report,
            last_error: job.last_error.clone(),
            created_at: utc(job.created_at),
            updated_at: utc(job.updated_at),
            expires_at: utc(job.expires_at),
        }
    }
}

fn now() -> chrono::NaiveDateTime {
    chrono::Utc::now().naive_utc()
}

fn do_nothing() -> OnConflict {
    OnConflict::new().do_nothing().to_owned()
}

fn job_status(job: &fork_job::Model) -> Option<ForkJobStatus> {
    ForkJobStatus::parse(&job.status)
}

fn job_step(job: &fork_job::Model) -> Result<ForkJobStep, ApiError> {
    ForkJobStep::parse(&job.step).ok_or_else(|| {
        ApiError::internal(format!(
            "fork job {} is at unknown step {:?}",
            job.id, job.step
        ))
    })
}

/// Whether a fork of `rows` DB rows and `bytes` of storage runs inside the
/// request or is handed to the worker.
pub fn fits_sync(rows: u64, bytes: u64) -> bool {
    rows <= DEFAULT_WRITE_CHUNK as u64 && bytes <= SYNC_MAX_BYTES
}

/// The DB rows a fork of `src_app_id` writes, for [`fits_sync`].
pub async fn count_source_rows(state: &AppState, src_app_id: &str) -> Result<u64, ApiError> {
    let db = &state.db;
    let widget_ids: Vec<String> = widget::Entity::find()
        .filter(widget::Column::AppId.eq(src_app_id))
        .select_only()
        .column(widget::Column::Id)
        .into_tuple()
        .all(db)
        .await?;
    let template_ids: Vec<String> = template::Entity::find()
        .filter(template::Column::AppId.eq(src_app_id))
        .select_only()
        .column(template::Column::Id)
        .into_tuple()
        .all(db)
        .await?;
    let mut rows = widget_ids.len() as u64 + template_ids.len() as u64;
    rows += meta::Entity::find()
        .filter(meta::Column::AppId.eq(src_app_id))
        .count(db)
        .await?;
    rows += role::Entity::find()
        .filter(role::Column::AppId.eq(src_app_id))
        .count(db)
        .await?;
    rows += app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(src_app_id))
        .count(db)
        .await?;
    rows += event::Entity::find()
        .filter(event::Column::AppId.eq(src_app_id))
        .count(db)
        .await?;
    rows += page::Entity::find()
        .filter(page::Column::AppId.eq(src_app_id))
        .count(db)
        .await?;
    rows += event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(src_app_id))
        .count(db)
        .await?;
    if !widget_ids.is_empty() {
        rows += meta::Entity::find()
            .filter(meta::Column::WidgetId.is_in(widget_ids))
            .count(db)
            .await?;
    }
    if !template_ids.is_empty() {
        rows += meta::Entity::find()
            .filter(meta::Column::TemplateId.is_in(template_ids))
            .count(db)
            .await?;
    }
    Ok(rows + 5)
}

/// Create the job row. Nothing else exists yet; `allocate` is the first
/// step of [`run_pass`].
pub async fn enqueue(
    state: &AppState,
    source_app_id: &str,
    user_sub: &str,
    spec: ForkJobSpec,
) -> Result<fork_job::Model, ApiError> {
    let job_id = create_id();
    let now = now();
    let model = fork_job::Model {
        dest_app_id: ids::derive_id(&job_id, &format!("app:{source_app_id}")),
        id: job_id,
        source_app_id: source_app_id.to_string(),
        user_id: user_sub.to_string(),
        status: ForkJobStatus::Queued.as_str().to_string(),
        step: ForkJobStep::Allocate.as_str().to_string(),
        cursor: None,
        policy: serde_json::to_value(&spec)
            .map_err(|e| ApiError::internal(format!("serialize fork spec: {e}")))?,
        report: None,
        last_error: None,
        created_at: now,
        updated_at: now,
        expires_at: now + chrono::Duration::from_std(FORK_JOB_TTL).unwrap_or_default(),
    };
    state
        .transaction(|txn| {
            let row = model.clone().into_active_model();
            Box::pin(async move {
                fork_job::Entity::insert(row)
                    .on_conflict(do_nothing())
                    .exec_without_returning(txn)
                    .await?;
                Ok::<_, ApiError>(())
            })
        })
        .await?;
    Ok(model)
}

/// Run every remaining step of the job in this process and return the full
/// in-memory report. A failure aborts the job (rows, storage, App and job
/// row are removed) before the error is returned, so a synchronous caller
/// never leaves a half-built destination behind.
pub async fn run_inline(
    state: &AppState,
    job: fork_job::Model,
) -> Result<(fork_job::Model, ForkReport), ApiError> {
    let job_for_abort = job.clone();
    match run_pass(state, job).await {
        Ok((job, Some(report))) => Ok((job, report)),
        Ok((job, None)) => {
            abort_best_effort(state, &job_for_abort).await;
            Err(ApiError::internal(format!(
                "fork job {} stopped at step {} without a report",
                job.id, job.step
            )))
        }
        Err(err) => {
            abort_best_effort(state, &job_for_abort).await;
            Err(err)
        }
    }
}

/// Run the job in the background; the caller polls `GET /apps/fork/jobs/{id}`.
pub fn spawn_background(state: AppState, job: fork_job::Model) {
    flow_like_types::tokio::spawn(async move {
        let job_id = job.id.clone();
        if let Err(error) = run_pass(&state, job).await {
            tracing::warn!(job_id, %error, "background fork pass failed");
        }
    });
}

/// Drive the job from its current step until it is done, waiting on the
/// desktop (`upload`), or fails. Returns the refreshed row and, when the
/// pass finalized the fork, the full report. Any error is recorded on the
/// row as `FAILED` before it is returned.
pub async fn run_pass(
    state: &AppState,
    job: fork_job::Model,
) -> Result<(fork_job::Model, Option<ForkReport>), ApiError> {
    let job_id = job.id.clone();
    match run_steps(state, job).await {
        Ok(outcome) => Ok(outcome),
        Err(err) => {
            mark_failed(state, &job_id, &err).await;
            Err(err)
        }
    }
}

async fn run_steps(
    state: &AppState,
    mut job: fork_job::Model,
) -> Result<(fork_job::Model, Option<ForkReport>), ApiError> {
    let spec = ForkJobSpec::parse(&job)?;
    let mut ctx: Option<ForkContext> = None;
    let mut plan: Option<ForkPlan> = None;
    loop {
        match job_step(&job)? {
            ForkJobStep::Allocate => {
                let next = match spec.kind {
                    ForkJobKind::OnlineCopy => {
                        allocate_online_copy(state, &job).await?;
                        ForkJobStep::CopyStorage
                    }
                    ForkJobKind::OfflineUpload => {
                        allocate_upload(state, &job).await?;
                        ForkJobStep::Upload
                    }
                };
                job = advance(state, &job.id, next, ForkJobStatus::Running, None).await?;
            }
            ForkJobStep::CopyStorage => {
                let (ctx, plan) = ensure_plan(state, &job, &spec, &mut ctx, &mut plan).await?;
                let mut cursor = ForkJobCursor::parse(&job);
                copy_content(state, &job.id, ctx, plan, &mut cursor).await?;
                cursor.done.clear();
                job = advance(
                    state,
                    &job.id,
                    ForkJobStep::WriteRows,
                    ForkJobStatus::Running,
                    Some(&cursor),
                )
                .await?;
            }
            ForkJobStep::WriteRows => {
                let (ctx, plan) = ensure_plan(state, &job, &spec, &mut ctx, &mut plan).await?;
                let mut cursor = ForkJobCursor::parse(&job);
                write_rows(state, &job.id, ctx, plan, &mut cursor).await?;
                cursor.done.clear();
                job = advance(
                    state,
                    &job.id,
                    ForkJobStep::Finalize,
                    ForkJobStatus::Running,
                    Some(&cursor),
                )
                .await?;
            }
            ForkJobStep::Finalize => {
                let (ctx, plan) = ensure_plan(state, &job, &spec, &mut ctx, &mut plan).await?;
                let cursor = ForkJobCursor::parse(&job);
                let report = build_report(plan, &cursor);
                let job = finalize(state, &job, &spec, ctx, &report).await?;
                return Ok((job, Some(report)));
            }
            ForkJobStep::Upload | ForkJobStep::Done => return Ok((job, None)),
            ForkJobStep::Abort => {
                abort(state, &job).await?;
                return Ok((job, None));
            }
        }
    }
}

async fn ensure_plan<'a>(
    state: &AppState,
    job: &fork_job::Model,
    spec: &ForkJobSpec,
    ctx: &'a mut Option<ForkContext>,
    plan: &'a mut Option<ForkPlan>,
) -> Result<(&'a ForkContext, &'a ForkPlan), ApiError> {
    if ctx.is_none() {
        *ctx = Some(ForkContext::load(state, job, spec).await?);
    }
    let ctx_ref = ctx.as_ref().expect("context loaded above");
    if plan.is_none() {
        *plan = Some(materialize_meta(state, ctx_ref).await?);
    }
    Ok((ctx_ref, plan.as_ref().expect("plan built above")))
}

fn build_report(plan: &ForkPlan, cursor: &ForkJobCursor) -> ForkReport {
    let mut warnings = plan.warnings.clone();
    warnings.extend(cursor.warnings.iter().cloned());
    ForkReport {
        id_map: plan.maps.clone(),
        skipped: plan.skipped.clone(),
        warnings,
        bytes_copied: cursor.bytes_copied,
        objects_copied: cursor.objects_copied,
    }
}

fn hidden_app_shell(
    dest_app_id: &str,
    forked_from: Option<String>,
    now: chrono::NaiveDateTime,
) -> app::ActiveModel {
    app::ActiveModel {
        id: Set(dest_app_id.to_string()),
        status: Set(Status::Inactive),
        visibility: Set(Visibility::Offline),
        changelog: Set(None),
        default_role_id: NotSet,
        owner_role_id: NotSet,
        primary_category: Set(None),
        secondary_category: Set(None),
        app_type: Set(None),
        rating_sum: Set(0),
        rating_count: Set(0),
        download_count: Set(0),
        interactions_count: Set(0),
        avg_rating: Set(None),
        relevance_score: Set(None),
        total_size: Set(0),
        price: Set(0),
        version: Set(None),
        execution_mode: Set(ExecutionMode::Any),
        bits: Set(Some(Default::default())),
        allow_forking: Set(false),
        fork_policy: Set(None),
        forked_from: Set(forked_from),
        forked_at: Set(Some(now)),
        created_at: Set(now),
        updated_at: Set(now),
    }
}

/// The App row exists from here on, hidden (`INACTIVE` / `OFFLINE`), so
/// listings skip it and the orphan janitor knows the prefix is owned.
/// Counters reset; a fork does not inherit the source's fork policy.
async fn allocate_online_copy(state: &AppState, job: &fork_job::Model) -> Result<(), ApiError> {
    let src = app::Entity::find_by_id(job.source_app_id.as_str())
        .one(&state.db)
        .await?
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "fork source app {} no longer exists",
                job.source_app_id
            ))
        })?;
    let now = now();
    let mut row = hidden_app_shell(&job.dest_app_id, Some(src.id.clone()), now);
    row.changelog = Set(src.changelog.clone());
    row.primary_category = Set(src.primary_category.clone());
    row.secondary_category = Set(src.secondary_category.clone());
    row.app_type = Set(src.app_type.clone());
    row.version = Set(src.version.clone());
    row.execution_mode = Set(src.execution_mode.clone());
    row.bits = Set(src.bits.clone());
    insert_app_shell(state, row).await
}

/// Offline → online: the destination needs its Owner / Admin / User roles
/// and the caller's membership before the desktop can push anything
/// through the app-edit endpoints.
async fn allocate_upload(state: &AppState, job: &fork_job::Model) -> Result<(), ApiError> {
    let now = now();
    let forked_from = (job.source_app_id != OFFLINE_SOURCE).then(|| job.source_app_id.clone());
    insert_app_shell(state, hidden_app_shell(&job.dest_app_id, forked_from, now)).await?;
    let roles = plan_roles(
        &job.id,
        &job.dest_app_id,
        &[],
        &Default::default(),
        None,
        None,
        now,
    );
    write_role_set(state, &job.id, &job.dest_app_id, &job.user_id, roles).await
}

/// Placeholder source id for uploads of local-only apps, which have no
/// server-side lineage.
pub const OFFLINE_SOURCE: &str = "offline";

async fn insert_app_shell(state: &AppState, row: app::ActiveModel) -> Result<(), ApiError> {
    state
        .transaction(|txn| {
            let row = row.clone();
            Box::pin(async move {
                app::Entity::insert(row)
                    .on_conflict(do_nothing())
                    .exec_without_returning(txn)
                    .await?;
                Ok::<_, ApiError>(())
            })
        })
        .await
}

/// Roles, the App's owner / default pointers and the caller's owner
/// membership, in one transaction: the pointers may only be set once the
/// rows behind them exist. Idempotent through derived ids.
async fn write_role_set(
    state: &AppState,
    seed: &str,
    dest_app_id: &str,
    user_sub: &str,
    roles: super::RolePlan,
) -> Result<(), ApiError> {
    let now = now();
    let membership_row = membership::ActiveModel {
        id: Set(owner_membership_id(seed)),
        user_id: Set(user_sub.to_string()),
        app_id: Set(dest_app_id.to_string()),
        role_id: Set(roles.owner_role_id.clone()),
        joined_via: NotSet,
        created_at: Set(now),
        updated_at: Set(now),
    };
    let dest_app_id = dest_app_id.to_string();
    retry_transaction::<_, (), ApiError>(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let role_rows = roles.roles.clone();
            let owner_role_id = roles.owner_role_id.clone();
            let default_role_id = roles.default_role_id.clone();
            let membership_row = membership_row.clone();
            let dest_app_id = dest_app_id.clone();
            Box::pin(async move {
                if !role_rows.is_empty() {
                    role::Entity::insert_many(role_rows)
                        .on_conflict(do_nothing())
                        .exec_without_returning(txn)
                        .await?;
                }
                app::Entity::update_many()
                    .col_expr(app::Column::OwnerRoleId, Expr::value(Some(owner_role_id)))
                    .col_expr(
                        app::Column::DefaultRoleId,
                        Expr::value(Some(default_role_id)),
                    )
                    .col_expr(app::Column::UpdatedAt, Expr::value(now))
                    .filter(app::Column::Id.eq(dest_app_id))
                    .exec(txn)
                    .await?;
                membership::Entity::insert(membership_row)
                    .on_conflict(do_nothing())
                    .exec_without_returning(txn)
                    .await?;
                Ok(())
            })
        },
    )
    .await
}

/// The bulk mirrors, resumed from the cursor: `upload/` (policy), then
/// `storage/` (project database gated per object), then the app's
/// metadata media, then the schema-only database recreation.
async fn copy_content(
    state: &AppState,
    job_id: &str,
    ctx: &ForkContext,
    plan: &ForkPlan,
    cursor: &mut ForkJobCursor,
) -> Result<(), ApiError> {
    let _ = plan;
    let storage_skip = policy::storage_skip(&ctx.policy);
    let mirrors: [(&str, Path, Path, bool, bool); 3] = [
        (
            "upload storage",
            ctx.src_prefix.child("upload"),
            ctx.dst_prefix.child("upload"),
            ctx.policy.files,
            false,
        ),
        (
            "app storage",
            ctx.src_prefix.child("storage"),
            ctx.dst_prefix.child("storage"),
            true,
            true,
        ),
        (
            "app metadata media",
            ctx.src_media_prefix(),
            ctx.dst_media_prefix(),
            true,
            false,
        ),
    ];
    for (label, src, dst, enabled, policy_gated) in mirrors {
        if !enabled || cursor.is_done(label) {
            continue;
        }
        copy_object_prefix_resumable(
            &ctx.src_content_store,
            &ctx.dst_content_store,
            &src,
            &dst,
            label,
            if policy_gated {
                storage_skip.as_deref()
            } else {
                None
            },
            Some(CopyCheckpoint {
                state,
                job_id,
                cursor,
            }),
        )
        .await?;
        cursor.mark_done(label);
        persist_cursor(state, job_id, cursor).await?;
    }

    const DB_SCHEMA: &str = "db_schema";
    if ctx.policy.databases == super::ForkDatabaseMode::SchemaOnly && !cursor.is_done(DB_SCHEMA) {
        let created =
            db_schema::copy_project_db_schemas(state, &ctx.src_app_id, &ctx.dest_app_id).await?;
        if !created.is_empty() {
            cursor.warnings.push(format!(
                "{} database table(s) were recreated empty. Indices were not copied — rebuild them in Data Studio.",
                created.len()
            ));
        }
        cursor.mark_done(DB_SCHEMA);
        persist_cursor(state, job_id, cursor).await?;
    }
    Ok(())
}

/// Destination rows in foreign-key order, one table per sub-step, each
/// chunked into its own retried transaction and recorded in the cursor.
async fn write_rows(
    state: &AppState,
    job_id: &str,
    ctx: &ForkContext,
    plan: &ForkPlan,
    cursor: &mut ForkJobCursor,
) -> Result<(), ApiError> {
    macro_rules! sub_step {
        ($marker:literal, $chunk:expr, $rows:expr) => {
            if !cursor.is_done($marker) {
                insert_in_chunks(
                    &state.db,
                    state.db_dialect,
                    $rows,
                    $chunk,
                    Some(do_nothing()),
                )
                .await?;
                cursor.mark_done($marker);
                persist_cursor(state, job_id, cursor).await?;
            }
        };
    }

    sub_step!("meta", DEFAULT_WRITE_CHUNK, plan_meta_rows(ctx));
    if !cursor.is_done("roles") {
        let roles = plan_roles(
            &ctx.seed,
            &ctx.dest_app_id,
            &plan.roles_to_copy,
            &plan.maps.roles,
            ctx.src_app_row.owner_role_id.as_deref(),
            ctx.src_app_row.default_role_id.as_deref(),
            ctx.now,
        );
        write_role_set(state, &ctx.seed, &ctx.dest_app_id, &ctx.user_sub, roles).await?;
        cursor.mark_done("roles");
        persist_cursor(state, job_id, cursor).await?;
    }
    sub_step!(
        "packages",
        DEFAULT_WRITE_CHUNK,
        plan_package_rows(ctx, plan)
    );
    sub_step!("events", EVENT_ROW_CHUNK, plan_event_rows(ctx, plan));
    sub_step!("pages", DEFAULT_WRITE_CHUNK, plan_page_rows(ctx, plan));
    sub_step!("widgets", DEFAULT_WRITE_CHUNK, plan_widget_rows(ctx, plan));
    sub_step!(
        "widget_meta",
        DEFAULT_WRITE_CHUNK,
        plan_widget_meta_rows(ctx, plan)
    );
    sub_step!(
        "templates",
        DEFAULT_WRITE_CHUNK,
        plan_template_rows(ctx, plan)
    );
    sub_step!(
        "template_meta",
        DEFAULT_WRITE_CHUNK,
        plan_template_meta_rows(ctx, plan)
    );
    sub_step!("sinks", DEFAULT_WRITE_CHUNK, plan.sinks_to_insert.clone());
    Ok(())
}

/// Flip the App to its requested visibility and `ACTIVE`, store the
/// (top-level) report and drop the token from the spec, in one transaction.
async fn finalize(
    state: &AppState,
    job: &fork_job::Model,
    spec: &ForkJobSpec,
    ctx: &ForkContext,
    report: &ForkReport,
) -> Result<fork_job::Model, ApiError> {
    let stored_report = ForkReport {
        id_map: report.id_map.top_level(),
        skipped: report.skipped.clone(),
        warnings: report.warnings.clone(),
        bytes_copied: report.bytes_copied,
        objects_copied: report.objects_copied,
    };
    let report_value = serde_json::to_value(&stored_report)
        .map_err(|e| ApiError::internal(format!("serialize fork report: {e}")))?;
    let spec_value = serde_json::to_value(spec.without_secrets())
        .map_err(|e| ApiError::internal(format!("serialize fork spec: {e}")))?;
    let dest_app_id = job.dest_app_id.clone();
    let job_id = job.id.clone();
    let visibility = ctx.dst_visibility.clone();
    state
        .transaction(|txn| {
            let dest_app_id = dest_app_id.clone();
            let job_id = job_id.clone();
            let visibility = visibility.clone();
            let report_value = report_value.clone();
            let spec_value = spec_value.clone();
            Box::pin(async move {
                let now = now();
                app::Entity::update_many()
                    .col_expr(app::Column::Status, Expr::value(Status::Active.to_value()))
                    .col_expr(app::Column::Visibility, Expr::value(visibility.to_value()))
                    .col_expr(app::Column::UpdatedAt, Expr::value(now))
                    .filter(app::Column::Id.eq(dest_app_id))
                    .exec(txn)
                    .await?;
                let row = fork_job::ActiveModel {
                    id: Set(job_id),
                    status: Set(ForkJobStatus::Done.as_str().to_string()),
                    step: Set(ForkJobStep::Done.as_str().to_string()),
                    cursor: Set(None),
                    policy: Set(spec_value),
                    report: Set(Some(report_value)),
                    last_error: Set(None),
                    updated_at: Set(now),
                    ..Default::default()
                };
                let updated = fork_job::Entity::update(row).exec(txn).await?;
                Ok::<_, ApiError>(updated)
            })
        })
        .await
}

/// Called by `POST /apps/{id}/fork/online/finalize` once the desktop has
/// pushed its bundle. Marks the upload job done; a destination without a
/// job (allocated before jobs existed) is left alone.
pub async fn complete_upload(state: &AppState, dest_app_id: &str) -> Result<(), ApiError> {
    let Some(job) = find_live_by_dest(state, dest_app_id).await? else {
        return Ok(());
    };
    advance(state, &job.id, ForkJobStep::Done, ForkJobStatus::Done, None).await?;
    Ok(())
}

/// The job that owns `dest_app_id`, if it is not done yet.
pub async fn find_live_by_dest(
    state: &AppState,
    dest_app_id: &str,
) -> Result<Option<fork_job::Model>, ApiError> {
    Ok(fork_job::Entity::find()
        .filter(fork_job::Column::DestAppId.eq(dest_app_id))
        .filter(fork_job::Column::Status.ne(ForkJobStatus::Done.as_str()))
        .order_by_desc(fork_job::Column::CreatedAt)
        .one(&state.db)
        .await?)
}

/// Destination app ids with a job that has not finished — their storage
/// prefixes are owned, not orphaned.
pub async fn live_dest_app_ids(state: &AppState) -> Result<HashSet<String>, ApiError> {
    let ids: Vec<String> = fork_job::Entity::find()
        .filter(fork_job::Column::Status.ne(ForkJobStatus::Done.as_str()))
        .select_only()
        .column(fork_job::Column::DestAppId)
        .into_tuple()
        .all(&state.db)
        .await?;
    Ok(ids.into_iter().collect())
}

/// Save the cursor in its own one-row transaction.
pub(crate) async fn persist_cursor(
    state: &AppState,
    job_id: &str,
    cursor: &ForkJobCursor,
) -> Result<(), ApiError> {
    let value = serde_json::to_value(cursor)
        .map_err(|e| ApiError::internal(format!("serialize fork cursor: {e}")))?;
    let job_id = job_id.to_string();
    retry_transaction::<_, (), ApiError>(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let row = fork_job::ActiveModel {
                id: Set(job_id.clone()),
                cursor: Set(Some(value.clone())),
                updated_at: Set(now()),
                ..Default::default()
            };
            Box::pin(async move {
                fork_job::Entity::update(row).exec(txn).await?;
                Ok(())
            })
        },
    )
    .await
}

async fn advance(
    state: &AppState,
    job_id: &str,
    step: ForkJobStep,
    status: ForkJobStatus,
    cursor: Option<&ForkJobCursor>,
) -> Result<fork_job::Model, ApiError> {
    let cursor_value = match cursor {
        Some(cursor) => Some(
            serde_json::to_value(cursor)
                .map_err(|e| ApiError::internal(format!("serialize fork cursor: {e}")))?,
        ),
        None => None,
    };
    let job_id = job_id.to_string();
    retry_transaction::<_, fork_job::Model, ApiError>(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let row = fork_job::ActiveModel {
                id: Set(job_id.clone()),
                status: Set(status.as_str().to_string()),
                step: Set(step.as_str().to_string()),
                cursor: Set(cursor_value.clone()),
                last_error: Set(None),
                updated_at: Set(now()),
                ..Default::default()
            };
            Box::pin(async move { Ok(fork_job::Entity::update(row).exec(txn).await?) })
        },
    )
    .await
}

async fn mark_failed(state: &AppState, job_id: &str, error: &ApiError) {
    let message = error
        .public_message()
        .map(str::to_string)
        .unwrap_or_else(|| error.to_string());
    let row = fork_job::ActiveModel {
        id: Set(job_id.to_string()),
        status: Set(ForkJobStatus::Failed.as_str().to_string()),
        last_error: Set(Some(message.chars().take(2_000).collect())),
        updated_at: Set(now()),
        ..Default::default()
    };
    if let Err(update_error) = fork_job::Entity::update(row).exec(&state.db).await {
        tracing::warn!(job_id, %update_error, "could not record fork job failure");
    }
}

async fn abort_best_effort(state: &AppState, job: &fork_job::Model) {
    if let Err(error) = abort(state, job).await {
        tracing::warn!(job_id = %job.id, dest_app_id = %job.dest_app_id, %error, "fork abort left remnants behind");
    }
}

async fn drain<E>(state: &AppState, condition: Condition) -> Result<(), ApiError>
where
    E: EntityTrait,
    <E::PrimaryKey as sea_orm::PrimaryKeyTrait>::ValueType:
        Into<sea_orm::Value> + sea_orm::TryGetable + Clone + Send + Sync + 'static,
{
    delete_in_batches::<E>(
        &state.db,
        state.db_dialect,
        condition,
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await?;
    Ok(())
}

/// Tear the destination down: child rows first (so the App delete's
/// cascades find nothing), then the storage prefixes on both stores and
/// the app media, then the App and the job row.
pub async fn abort(state: &AppState, job: &fork_job::Model) -> Result<(), ApiError> {
    let dest = job.dest_app_id.as_str();
    let aborting = fork_job::ActiveModel {
        id: Set(job.id.clone()),
        status: Set(ForkJobStatus::Aborting.as_str().to_string()),
        step: Set(ForkJobStep::Abort.as_str().to_string()),
        updated_at: Set(now()),
        ..Default::default()
    };
    if let Err(error) = fork_job::Entity::update(aborting).exec(&state.db).await {
        tracing::debug!(job_id = %job.id, %error, "fork job row not marked aborting");
    }

    let by_app = |column: sea_orm::sea_query::SimpleExpr| Condition::all().add(column);
    drain::<event_sink::Entity>(state, by_app(event_sink::Column::AppId.eq(dest))).await?;
    drain::<event_alias::Entity>(state, by_app(event_alias::Column::AppId.eq(dest))).await?;
    drain::<event_setup::Entity>(state, by_app(event_setup::Column::AppId.eq(dest))).await?;
    drain::<event_remote_registration::Entity>(
        state,
        by_app(event_remote_registration::Column::AppId.eq(dest)),
    )
    .await?;
    drain::<event_remote_auth::Entity>(state, by_app(event_remote_auth::Column::AppId.eq(dest)))
        .await?;
    drain::<event::Entity>(state, by_app(event::Column::AppId.eq(dest))).await?;
    drain::<page::Entity>(state, by_app(page::Column::AppId.eq(dest))).await?;
    drain::<meta::Entity>(
        state,
        by_app(
            meta::Column::WidgetId.in_subquery(
                Query::select()
                    .column(widget::Column::Id)
                    .from(widget::Entity)
                    .and_where(widget::Column::AppId.eq(dest))
                    .to_owned(),
            ),
        ),
    )
    .await?;
    drain::<widget::Entity>(state, by_app(widget::Column::AppId.eq(dest))).await?;
    drain::<meta::Entity>(
        state,
        by_app(
            meta::Column::TemplateId.in_subquery(
                Query::select()
                    .column(template::Column::Id)
                    .from(template::Entity)
                    .and_where(template::Column::AppId.eq(dest))
                    .to_owned(),
            ),
        ),
    )
    .await?;
    drain::<template::Entity>(state, by_app(template::Column::AppId.eq(dest))).await?;
    drain::<app_package::Entity>(state, by_app(app_package::Column::AppId.eq(dest))).await?;
    drain::<membership::Entity>(state, by_app(membership::Column::AppId.eq(dest))).await?;
    drain::<meta::Entity>(state, by_app(meta::Column::AppId.eq(dest))).await?;

    let dest_owned = dest.to_string();
    state
        .transaction(|txn| {
            let dest = dest_owned.clone();
            Box::pin(async move {
                app::Entity::update_many()
                    .col_expr(app::Column::OwnerRoleId, Expr::value(None::<String>))
                    .col_expr(app::Column::DefaultRoleId, Expr::value(None::<String>))
                    .filter(app::Column::Id.eq(dest))
                    .exec(txn)
                    .await?;
                Ok::<_, ApiError>(())
            })
        })
        .await?;
    drain::<role::Entity>(state, by_app(role::Column::AppId.eq(dest))).await?;

    delete_dest_storage(state, dest).await?;

    let job_id = job.id.clone();
    state
        .transaction(|txn| {
            let dest = dest_owned.clone();
            let job_id = job_id.clone();
            Box::pin(async move {
                app::Entity::delete_by_id(dest).exec(txn).await?;
                fork_job::Entity::delete_by_id(job_id).exec(txn).await?;
                Ok::<_, ApiError>(())
            })
        })
        .await
}

/// Every object a destination owns: `apps/{id}` on the meta and content
/// stores and `media/apps/{id}` on the content store.
pub async fn delete_dest_storage(state: &AppState, dest_app_id: &str) -> Result<(), ApiError> {
    let credentials = state.master_credentials().await?;
    let meta_store = credentials.to_store(true).await?.as_generic();
    let content_store = credentials.to_store(false).await?.as_generic();
    let app_prefix = Path::from("apps").child(dest_app_id.to_string());
    let media_prefix = Path::from("media")
        .child("apps")
        .child(dest_app_id.to_string());
    delete_object_prefix(&meta_store, &app_prefix, "destination meta").await?;
    delete_object_prefix(&content_store, &app_prefix, "destination content").await?;
    delete_object_prefix(&content_store, &media_prefix, "destination media").await?;
    Ok(())
}

/// What one worker tick did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ForkTickOutcome {
    pub expired_aborted: u64,
    pub expired_deleted: u64,
    pub resumed: u64,
    pub failed: u64,
}

/// One maintenance tick: abort every job past `expiresAt` (finished ones
/// are just deleted), then resume a handful of jobs whose driver went
/// silent. Safe to call from any number of workers — a job is claimed by a
/// compare-and-set on `updatedAt` before it runs.
pub async fn tick(state: &AppState) -> Result<ForkTickOutcome, ApiError> {
    let mut outcome = ForkTickOutcome::default();
    let now_ts = now();

    let expired = fork_job::Entity::find()
        .filter(fork_job::Column::ExpiresAt.lt(now_ts))
        .order_by_asc(fork_job::Column::ExpiresAt)
        .limit(SWEEP_BATCH)
        .all(&state.db)
        .await?;
    for job in expired {
        if job_status(&job) == Some(ForkJobStatus::Done) {
            fork_job::Entity::delete_by_id(job.id.clone())
                .exec(&state.db)
                .await?;
            outcome.expired_deleted += 1;
            continue;
        }
        match abort(state, &job).await {
            Ok(()) => outcome.expired_aborted += 1,
            Err(error) => {
                tracing::warn!(job_id = %job.id, %error, "expired fork job could not be aborted");
                outcome.failed += 1;
            }
        }
    }

    let stale_before = now_ts - chrono::Duration::from_std(STALE_AFTER).unwrap_or_default();
    let stale = fork_job::Entity::find()
        .filter(fork_job::Column::Status.is_in([
            ForkJobStatus::Queued.as_str(),
            ForkJobStatus::Running.as_str(),
        ]))
        .filter(
            fork_job::Column::Step
                .is_not_in([ForkJobStep::Upload.as_str(), ForkJobStep::Done.as_str()]),
        )
        .filter(fork_job::Column::UpdatedAt.lt(stale_before))
        .order_by_asc(fork_job::Column::UpdatedAt)
        .limit(RESUME_BATCH)
        .all(&state.db)
        .await?;
    for job in stale {
        if !claim(state, &job).await? {
            continue;
        }
        match run_pass(state, job.clone()).await {
            Ok(_) => outcome.resumed += 1,
            Err(error) => {
                tracing::warn!(job_id = %job.id, %error, "resumed fork job failed");
                outcome.failed += 1;
            }
        }
    }
    Ok(outcome)
}

async fn claim(state: &AppState, job: &fork_job::Model) -> Result<bool, ApiError> {
    let job_id = job.id.clone();
    let seen = job.updated_at;
    retry_transaction::<_, bool, ApiError>(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let job_id = job_id.clone();
            Box::pin(async move {
                let result = fork_job::Entity::update_many()
                    .col_expr(fork_job::Column::UpdatedAt, Expr::value(now()))
                    .filter(fork_job::Column::Id.eq(job_id))
                    .filter(fork_job::Column::UpdatedAt.eq(seen))
                    .exec(txn)
                    .await?;
                Ok(result.rows_affected == 1)
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_and_step_round_trip() {
        for status in [
            ForkJobStatus::Queued,
            ForkJobStatus::Running,
            ForkJobStatus::Done,
            ForkJobStatus::Failed,
            ForkJobStatus::Aborting,
        ] {
            assert_eq!(ForkJobStatus::parse(status.as_str()), Some(status));
        }
        for step in [
            ForkJobStep::Allocate,
            ForkJobStep::CopyStorage,
            ForkJobStep::WriteRows,
            ForkJobStep::Finalize,
            ForkJobStep::Upload,
            ForkJobStep::Done,
            ForkJobStep::Abort,
        ] {
            assert_eq!(ForkJobStep::parse(step.as_str()), Some(step));
        }
        assert_eq!(ForkJobStatus::parse("nope"), None);
    }

    #[test]
    fn sync_threshold_follows_the_write_chunk_and_byte_cap() {
        assert!(fits_sync(DEFAULT_WRITE_CHUNK as u64, SYNC_MAX_BYTES));
        assert!(!fits_sync(DEFAULT_WRITE_CHUNK as u64 + 1, 0));
        assert!(!fits_sync(0, SYNC_MAX_BYTES + 1));
    }

    #[test]
    fn cursor_markers_reset_the_prefix_position() {
        let mut cursor = ForkJobCursor {
            prefix: Some("upload storage".to_string()),
            last_key: Some("apps/x/upload/z".to_string()),
            ..Default::default()
        };
        cursor.mark_done("upload storage");
        cursor.mark_done("upload storage");
        assert_eq!(cursor.done, vec!["upload storage".to_string()]);
        assert!(cursor.prefix.is_none() && cursor.last_key.is_none());
        assert!(cursor.is_done("upload storage"));
        assert!(!cursor.is_done("app storage"));
    }

    #[test]
    fn spec_json_never_carries_the_token_after_finalize() {
        let spec = ForkJobSpec {
            kind: ForkJobKind::OnlineCopy,
            language: "en".to_string(),
            visibility: Visibility::Private,
            policy: ForkPolicy::default(),
            remote_event_token_encrypted: Some("enc".to_string()),
        };
        let stripped = serde_json::to_value(spec.without_secrets()).expect("serialize");
        assert!(stripped.get("remote_event_token_encrypted").is_none());
        let parsed: ForkJobSpec = serde_json::from_value(stripped).expect("parse");
        assert_eq!(parsed.kind, ForkJobKind::OnlineCopy);
    }
}
