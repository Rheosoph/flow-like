//! Admin visibility for cascade deletion jobs: list, inspect, retry, and run
//! one queue pass on demand.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::{
    deletion::{self, DeletionRoot, PassBudget, QueueReport, job, plan_for},
    entity::deletion_job,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};

const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 200;

#[derive(Clone, Debug, Default, Deserialize, IntoParams)]
pub struct ListDeletionJobsQuery {
    /// `QUEUED`, `RUNNING`, `DONE` or `FAILED`; all when omitted.
    pub status: Option<String>,
    /// Rows to return, newest first (default 50, maximum 200).
    pub limit: Option<u64>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct DeletionJobView {
    pub id: String,
    pub root_kind: String,
    pub root_id: String,
    pub status: String,
    /// Index of the next plan step to run.
    pub phase: i32,
    /// Steps in the root's plan, when the root kind is known.
    pub total_steps: Option<usize>,
    pub cursor: Option<serde_json::Value>,
    pub attempts: i32,
    pub lease_until: Option<chrono::NaiveDateTime>,
    pub last_error: Option<String>,
    pub requested_by: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
}

impl From<deletion_job::Model> for DeletionJobView {
    fn from(job: deletion_job::Model) -> Self {
        let total_steps = DeletionRoot::parse(&job.root_kind)
            .and_then(|root| plan_for(root).ok())
            .map(|plan| plan.steps.len());
        Self {
            id: job.id,
            root_kind: job.root_kind,
            root_id: job.root_id,
            status: job.status,
            phase: job.phase,
            total_steps,
            cursor: job.cursor,
            attempts: job.attempts,
            lease_until: job.lease_until,
            last_error: job.last_error,
            requested_by: job.requested_by,
            created_at: job.created_at,
            updated_at: job.updated_at,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ListDeletionJobsResponse {
    pub jobs: Vec<DeletionJobView>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct DeletionStepView {
    pub phase: usize,
    pub description: String,
    pub table: Option<String>,
    /// Whether the job has moved past this step.
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct DeletionJobDetail {
    pub job: DeletionJobView,
    pub steps: Vec<DeletionStepView>,
}

#[utoipa::path(
    get,
    path = "/admin/deletions",
    tag = "admin",
    description = "List cascade deletion jobs, newest first, optionally filtered by status.",
    params(ListDeletionJobsQuery),
    responses(
        (status = 200, description = "Deletion jobs", body = ListDeletionJobsResponse),
        (status = 400, description = "Unknown status filter"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required")
    )
)]
#[tracing::instrument(name = "GET /admin/deletions", skip(state, user))]
pub async fn list_deletion_jobs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<ListDeletionJobsQuery>,
) -> Result<Json<ListDeletionJobsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    if let Some(status) = query.status.as_deref()
        && !job::STATUSES.contains(&status)
    {
        return Err(ApiError::bad_request(format!(
            "unknown status {status}; expected one of {}",
            job::STATUSES.join(", ")
        )));
    }
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let jobs = job::list(&state, query.status.as_deref(), limit).await?;
    Ok(Json(ListDeletionJobsResponse {
        jobs: jobs.into_iter().map(Into::into).collect(),
    }))
}

#[utoipa::path(
    get,
    path = "/admin/deletions/{job_id}",
    tag = "admin",
    description = "Inspect one cascade deletion job together with the plan it is walking.",
    params(("job_id" = String, Path, description = "Deletion job id")),
    responses(
        (status = 200, description = "Job and plan", body = DeletionJobDetail),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required"),
        (status = 404, description = "No such job")
    )
)]
#[tracing::instrument(name = "GET /admin/deletions/{job_id}", skip(state, user))]
pub async fn get_deletion_job(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(job_id): Path<String>,
) -> Result<Json<DeletionJobDetail>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let job = job::get(&state, &job_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("deletion job {job_id} not found")))?;
    let steps = DeletionRoot::parse(&job.root_kind)
        .and_then(|root| plan_for(root).ok())
        .map(|plan| {
            let phase = usize::try_from(job.phase).unwrap_or(0);
            plan.steps
                .iter()
                .enumerate()
                .map(|(index, step)| DeletionStepView {
                    phase: index,
                    description: step.to_string(),
                    table: step.table().map(str::to_owned),
                    done: index < phase || job.status == job::STATUS_DONE,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Json(DeletionJobDetail {
        job: job.into(),
        steps,
    }))
}

#[utoipa::path(
    post,
    path = "/admin/deletions/{job_id}/retry",
    tag = "admin",
    description = "Re-queue a failed or stuck cascade deletion job from its first step.",
    params(("job_id" = String, Path, description = "Deletion job id")),
    responses(
        (status = 200, description = "Job re-queued", body = DeletionJobView),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required"),
        (status = 404, description = "No such job"),
        (status = 409, description = "Job already finished")
    )
)]
#[tracing::instrument(name = "POST /admin/deletions/{job_id}/retry", skip(state, user))]
pub async fn retry_deletion_job(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(job_id): Path<String>,
) -> Result<Json<DeletionJobView>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let job = job::retry(&state, &job_id).await?;
    Ok(Json(job.into()))
}

#[utoipa::path(
    post,
    path = "/admin/deletions/run",
    tag = "admin",
    description = "Run one pass over the cascade deletion queue now, within the configured pass budget.",
    responses(
        (status = 200, description = "Pass report", body = QueueReport),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required")
    )
)]
#[tracing::instrument(name = "POST /admin/deletions/run", skip(state, user))]
pub async fn run_deletion_queue(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<QueueReport>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let report = deletion::run_queue(&state, PassBudget::from_env()).await?;
    Ok(Json(report))
}
