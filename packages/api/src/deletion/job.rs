//! `DeletionJob` rows: one per root, claimed by lease, resumable by phase.

use std::time::Duration;

use chrono::NaiveDateTime;
use flow_like_types::create_id;
use sea_orm::sea_query::{Expr, ExprTrait, OnConflict};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseTransaction, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set,
};

use super::DeletionRoot;
use super::plan::plan_for;
use crate::db::{RetryPolicy, retry_transaction};
use crate::entity::deletion_job::{ActiveModel, Column, Entity, Model};
use crate::entity::sea_orm_active_enums::{Status, Visibility};
use crate::entity::{app, deletion_job};
use crate::error::ApiError;
use crate::state::AppState;

pub const STATUS_QUEUED: &str = "QUEUED";
pub const STATUS_RUNNING: &str = "RUNNING";
pub const STATUS_DONE: &str = "DONE";
pub const STATUS_FAILED: &str = "FAILED";
pub const STATUSES: [&str; 4] = [STATUS_QUEUED, STATUS_RUNNING, STATUS_DONE, STATUS_FAILED];

/// How long a claim holds a job before another worker may resume it.
pub const LEASE: Duration = Duration::from_secs(5 * 60);
/// Failed passes before a job stops being retried automatically.
pub const MAX_ATTEMPTS: i32 = 5;
/// Chunks between cursor writes; each write also extends the lease.
pub const CHECKPOINT_EVERY_CHUNKS: usize = 10;
const MAX_ERROR_CHARS: usize = 2_000;

fn now() -> NaiveDateTime {
    chrono::Utc::now().naive_utc()
}

fn lease_from(now: NaiveDateTime) -> NaiveDateTime {
    now + chrono::Duration::from_std(LEASE).unwrap_or(chrono::Duration::minutes(5))
}

/// Hide the root from readers while its rows drain.
///
/// An app flips to `INACTIVE`/`OFFLINE`, the marker listings already skip
/// for in-flight forks. Other roots are hidden by their own state or are
/// small enough to disappear within the same pass.
async fn tombstone(
    txn: &DatabaseTransaction,
    root: DeletionRoot,
    root_id: &str,
) -> Result<(), sea_orm::DbErr> {
    if root == DeletionRoot::App {
        app::Entity::update_many()
            .set(app::ActiveModel {
                status: Set(Status::Inactive),
                visibility: Set(Visibility::Offline),
                updated_at: Set(now()),
                ..Default::default()
            })
            .filter(app::Column::Id.eq(root_id))
            .exec(txn)
            .await?;
    }
    Ok(())
}

pub async fn tombstone_root(
    state: &AppState,
    root: DeletionRoot,
    root_id: &str,
) -> Result<(), ApiError> {
    let root_id = root_id.to_owned();
    state
        .transaction(move |txn| {
            let root_id = root_id.clone();
            Box::pin(async move {
                tombstone(txn, root, &root_id).await?;
                Ok::<_, ApiError>(())
            })
        })
        .await
}

/// Queue the deletion of `root_id`, tombstoning it in the same transaction.
///
/// Idempotent on `(rootKind, rootId)`: a queued or running job is returned
/// as is, a failed one is re-queued from phase 0, a finished one is returned
/// unchanged.
pub async fn enqueue(
    state: &AppState,
    root: DeletionRoot,
    root_id: &str,
    requested_by: Option<&str>,
) -> Result<Model, ApiError> {
    plan_for(root)?;
    let root_kind = root.kind().to_owned();
    let root_id = root_id.to_owned();
    let requested_by = requested_by.map(str::to_owned);
    state
        .transaction(move |txn| {
            let root_kind = root_kind.clone();
            let root_id = root_id.clone();
            let requested_by = requested_by.clone();
            Box::pin(async move {
                let existing = find_by_root(txn, &root_kind, &root_id).await?;
                let job = match existing {
                    Some(job) if job.status == STATUS_FAILED => {
                        let now = now();
                        deletion_job::ActiveModel {
                            id: Set(job.id.clone()),
                            status: Set(STATUS_QUEUED.to_owned()),
                            phase: Set(0),
                            cursor: Set(None),
                            attempts: Set(0),
                            lease_until: Set(None),
                            last_error: Set(None),
                            requested_by: Set(requested_by.clone().or(job.requested_by)),
                            updated_at: Set(now),
                            ..Default::default()
                        }
                        .update(txn)
                        .await?
                    }
                    Some(job) => job,
                    None => {
                        let now = now();
                        Entity::insert(ActiveModel {
                            id: Set(create_id()),
                            root_kind: Set(root_kind.clone()),
                            root_id: Set(root_id.clone()),
                            status: Set(STATUS_QUEUED.to_owned()),
                            phase: Set(0),
                            cursor: Set(None),
                            attempts: Set(0),
                            lease_until: Set(None),
                            last_error: Set(None),
                            requested_by: Set(requested_by.clone()),
                            created_at: Set(now),
                            updated_at: Set(now),
                        })
                        .on_conflict(
                            OnConflict::columns([Column::RootKind, Column::RootId])
                                .do_nothing()
                                .to_owned(),
                        )
                        .exec_without_returning(txn)
                        .await?;
                        find_by_root(txn, &root_kind, &root_id)
                            .await?
                            .ok_or_else(|| {
                                sea_orm::DbErr::RecordNotFound(format!(
                                    "DeletionJob {root_kind}/{root_id} vanished after insert"
                                ))
                            })?
                    }
                };
                tombstone(txn, root, &root_id).await?;
                Ok::<_, ApiError>(job)
            })
        })
        .await
}

async fn find_by_root(
    txn: &DatabaseTransaction,
    root_kind: &str,
    root_id: &str,
) -> Result<Option<Model>, sea_orm::DbErr> {
    Entity::find()
        .filter(Column::RootKind.eq(root_kind))
        .filter(Column::RootId.eq(root_id))
        .one(txn)
        .await
}

/// Jobs whose lease has lapsed, oldest first.
pub async fn due_jobs(state: &AppState, limit: u64) -> Result<Vec<Model>, ApiError> {
    let now = now();
    Ok(Entity::find()
        .filter(Column::Status.is_in([STATUS_QUEUED, STATUS_RUNNING]))
        .filter(
            Condition::any()
                .add(Column::LeaseUntil.is_null())
                .add(Column::LeaseUntil.lt(now)),
        )
        .order_by_asc(Column::CreatedAt)
        .limit(limit)
        .all(&state.db)
        .await?)
}

/// Take a 5-minute lease on `job_id`; `None` when another worker holds it.
pub async fn claim(state: &AppState, job_id: &str) -> Result<Option<Model>, ApiError> {
    let job_id = job_id.to_owned();
    retry_transaction::<_, Option<Model>, ApiError>(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let job_id = job_id.clone();
            Box::pin(async move {
                let now = now();
                let result = Entity::update_many()
                    .col_expr(Column::LeaseUntil, Expr::value(lease_from(now)))
                    .col_expr(Column::Status, Expr::value(STATUS_RUNNING))
                    .col_expr(Column::Attempts, Expr::col(Column::Attempts).add(1))
                    .col_expr(Column::UpdatedAt, Expr::value(now))
                    .filter(Column::Id.eq(&job_id))
                    .filter(Column::Status.is_in([STATUS_QUEUED, STATUS_RUNNING]))
                    .filter(
                        Condition::any()
                            .add(Column::LeaseUntil.is_null())
                            .add(Column::LeaseUntil.lt(now)),
                    )
                    .exec(txn)
                    .await?;
                if result.rows_affected == 0 {
                    return Ok(None);
                }
                Ok(Entity::find_by_id(&job_id).one(txn).await?)
            })
        },
    )
    .await
}

async fn update(state: &AppState, model: ActiveModel) -> Result<(), ApiError> {
    state
        .transaction(move |txn| {
            let model = model.clone();
            Box::pin(async move {
                model.update(txn).await?;
                Ok::<_, ApiError>(())
            })
        })
        .await
}

/// Record progress and extend the lease.
pub async fn checkpoint(
    state: &AppState,
    job_id: &str,
    phase: usize,
    cursor: serde_json::Value,
) -> Result<(), ApiError> {
    let now = now();
    update(
        state,
        ActiveModel {
            id: Set(job_id.to_owned()),
            phase: Set(i32::try_from(phase).unwrap_or(i32::MAX)),
            cursor: Set(Some(cursor)),
            lease_until: Set(Some(lease_from(now))),
            updated_at: Set(now),
            ..Default::default()
        },
    )
    .await
}

/// The pass ran out of budget; make the job claimable again right away.
pub async fn release(
    state: &AppState,
    job_id: &str,
    phase: usize,
    cursor: serde_json::Value,
) -> Result<(), ApiError> {
    update(
        state,
        ActiveModel {
            id: Set(job_id.to_owned()),
            phase: Set(i32::try_from(phase).unwrap_or(i32::MAX)),
            cursor: Set(Some(cursor)),
            lease_until: Set(None),
            updated_at: Set(now()),
            ..Default::default()
        },
    )
    .await
}

pub async fn complete(
    state: &AppState,
    job_id: &str,
    cursor: serde_json::Value,
) -> Result<(), ApiError> {
    update(
        state,
        ActiveModel {
            id: Set(job_id.to_owned()),
            status: Set(STATUS_DONE.to_owned()),
            cursor: Set(Some(cursor)),
            lease_until: Set(None),
            last_error: Set(None),
            updated_at: Set(now()),
            ..Default::default()
        },
    )
    .await
}

/// Record a failed pass. The job goes back to the queue until it has used
/// [`MAX_ATTEMPTS`] passes, then stays `FAILED` for an operator.
pub async fn fail(
    state: &AppState,
    job_id: &str,
    attempts: i32,
    phase: usize,
    error: &str,
) -> Result<(), ApiError> {
    let status = if attempts >= MAX_ATTEMPTS {
        STATUS_FAILED
    } else {
        STATUS_QUEUED
    };
    let message: String = error.chars().take(MAX_ERROR_CHARS).collect();
    update(
        state,
        ActiveModel {
            id: Set(job_id.to_owned()),
            status: Set(status.to_owned()),
            phase: Set(i32::try_from(phase).unwrap_or(i32::MAX)),
            lease_until: Set(None),
            last_error: Set(Some(message)),
            updated_at: Set(now()),
            ..Default::default()
        },
    )
    .await
}

/// Operator retry: back to the queue from phase 0 with a fresh attempt budget.
pub async fn retry(state: &AppState, job_id: &str) -> Result<Model, ApiError> {
    let job = get(state, job_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("deletion job {job_id} not found")))?;
    if job.status == STATUS_DONE {
        return Err(ApiError::conflict(format!(
            "deletion job {job_id} already finished"
        )));
    }
    update(
        state,
        ActiveModel {
            id: Set(job.id.clone()),
            status: Set(STATUS_QUEUED.to_owned()),
            phase: Set(0),
            cursor: Set(None),
            attempts: Set(0),
            lease_until: Set(None),
            last_error: Set(None),
            updated_at: Set(now()),
            ..Default::default()
        },
    )
    .await?;
    get(state, job_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("deletion job {job_id} not found")))
}

pub async fn get(state: &AppState, job_id: &str) -> Result<Option<Model>, ApiError> {
    Ok(Entity::find_by_id(job_id).one(&state.db).await?)
}

pub async fn list(
    state: &AppState,
    status: Option<&str>,
    limit: u64,
) -> Result<Vec<Model>, ApiError> {
    let mut query = Entity::find();
    if let Some(status) = status {
        query = query.filter(Column::Status.eq(status));
    }
    Ok(query
        .order_by_desc(Column::UpdatedAt)
        .limit(limit)
        .all(&state.db)
        .await?)
}
