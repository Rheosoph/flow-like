//! Deleting a root from a request handler.
//!
//! The handler queues the root and runs one request-sized pass inline. A root
//! that empties within [`PassBudget::inline`] is gone before the response is
//! written and the endpoint answers exactly as it did before; anything larger
//! stays tombstoned, keeps its job, and answers `202` with
//! [`AcceptedDeletion`] so the caller can follow it on
//! `GET /admin/deletions/{job_id}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

use super::drain::PassBudget;
use super::{DeletionRoot, PassOutcome, job, run_pass};
use crate::entity::deletion_job;
use crate::error::ApiError;
use crate::state::AppState;

/// The deletion job behind a `202`.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AcceptedDeletion {
    pub job_id: String,
    /// The kind of row being deleted, e.g. `App`.
    pub root_kind: String,
    pub root_id: String,
    /// `QUEUED`, `RUNNING` or `FAILED`.
    pub status: String,
    /// Plan steps already finished.
    pub phase: i32,
    /// Why the inline pass stopped, when it stopped on an error. The job stays
    /// queued and is retried by the deletion worker.
    pub last_error: Option<String>,
}

impl From<&deletion_job::Model> for AcceptedDeletion {
    fn from(job: &deletion_job::Model) -> Self {
        Self {
            job_id: job.id.clone(),
            root_kind: job.root_kind.clone(),
            root_id: job.root_id.clone(),
            status: job.status.clone(),
            phase: job.phase,
            last_error: job.last_error.clone(),
        }
    }
}

/// The endpoint's own body once the root is gone, or the job that finishes
/// the work.
pub enum Deleted<T> {
    Completed(T),
    Accepted(AcceptedDeletion),
}

impl<T: Serialize> IntoResponse for Deleted<T> {
    fn into_response(self) -> Response {
        match self {
            Self::Completed(body) => Json(body).into_response(),
            Self::Accepted(accepted) => (StatusCode::ACCEPTED, Json(accepted)).into_response(),
        }
    }
}

/// Queue `root_id` for deletion and run one request-sized first pass inline.
///
/// `body` is the response the endpoint answers with when that pass finished
/// the plan.
pub async fn delete_now<T>(
    state: &AppState,
    root: DeletionRoot,
    root_id: &str,
    requested_by: Option<&str>,
    body: T,
) -> Result<Deleted<T>, ApiError> {
    let queued = super::enqueue(state, root, root_id, requested_by).await?;
    if queued.status == job::STATUS_DONE {
        return Ok(Deleted::Completed(body));
    }
    let Some(claimed) = job::claim(state, &queued.id).await? else {
        return Ok(Deleted::Accepted(AcceptedDeletion::from(&queued)));
    };
    match run_pass(state, &claimed, PassBudget::inline()).await? {
        PassOutcome::Completed => Ok(Deleted::Completed(body)),
        PassOutcome::Suspended { .. } | PassOutcome::Failed { .. } => {
            let current = job::get(state, &claimed.id).await?.unwrap_or(claimed);
            Ok(Deleted::Accepted(AcceptedDeletion::from(&current)))
        }
    }
}
