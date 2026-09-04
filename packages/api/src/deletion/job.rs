//! `DeletionJob` rows: one per root, claimed by lease, resumable by phase.

use std::time::Duration;

use chrono::NaiveDateTime;
use flow_like_types::{anyhow, create_id};
use sea_orm::sea_query::{Expr, ExprTrait, OnConflict};
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, Condition, DatabaseTransaction, EntityTrait,
    QueryFilter, QueryOrder, QuerySelect, Set,
};

use super::DeletionRoot;
use super::plan::plan_for;
use crate::db::{RetryPolicy, retry_transaction};
use crate::entity::deletion_job::{ActiveModel, Column, Entity, Model};
use crate::entity::sea_orm_active_enums::{
    Status, UserStatus, Visibility, WasmPackageStatus, WasmPackageVisibility,
};
use crate::entity::{
    app, app_group, course, deletion_job, event, learning_path, user, wasm_package,
};
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

/// Hide the root from readers while its rows drain, so a `202` never leaves a
/// live-looking row whose children and storage are disappearing underneath it.
///
/// Every root that carries a state column of its own is flipped to the value
/// its listings and read paths already treat as "not available". The roots
/// without such a column — `Template`, `CourseModule`, `Lesson`, `Challenge`,
/// `Membership`, `TechnicalUser`, `Bit`, `ExecutionRun` — can only be hidden by
/// their listings excluding rows with an unfinished `DeletionJob`.
async fn tombstone(
    txn: &DatabaseTransaction,
    root: DeletionRoot,
    root_id: &str,
) -> Result<(), sea_orm::DbErr> {
    let now = now();
    match root {
        DeletionRoot::App => {
            app::Entity::update_many()
                .set(app::ActiveModel {
                    status: Set(Status::Inactive),
                    visibility: Set(Visibility::Offline),
                    updated_at: Set(now),
                    ..Default::default()
                })
                .filter(app::Column::Id.eq(root_id))
                .exec(txn)
                .await?;
        }
        DeletionRoot::AppGroup => {
            app_group::Entity::update_many()
                .set(app_group::ActiveModel {
                    status: Set(Status::Inactive),
                    visibility: Set(Visibility::Offline),
                    updated_at: Set(now),
                    ..Default::default()
                })
                .filter(app_group::Column::Id.eq(root_id))
                .exec(txn)
                .await?;
        }
        DeletionRoot::WasmPackage => {
            wasm_package::Entity::update_many()
                .set(wasm_package::ActiveModel {
                    status: Set(WasmPackageStatus::Disabled),
                    visibility: Set(WasmPackageVisibility::Private),
                    updated_at: Set(now),
                    ..Default::default()
                })
                .filter(wasm_package::Column::Id.eq(root_id))
                .exec(txn)
                .await?;
        }
        DeletionRoot::Course => {
            course::Entity::update_many()
                .set(course::ActiveModel {
                    is_published: Set(false),
                    updated_at: Set(now),
                    ..Default::default()
                })
                .filter(course::Column::Id.eq(root_id))
                .exec(txn)
                .await?;
        }
        DeletionRoot::LearningPath => {
            learning_path::Entity::update_many()
                .set(learning_path::ActiveModel {
                    is_published: Set(false),
                    updated_at: Set(now),
                    ..Default::default()
                })
                .filter(learning_path::Column::Id.eq(root_id))
                .exec(txn)
                .await?;
        }
        DeletionRoot::Event => {
            event::Entity::update_many()
                .set(event::ActiveModel {
                    active: Set(false),
                    updated_at: Set(now),
                    ..Default::default()
                })
                .filter(event::Column::Id.eq(root_id))
                .exec(txn)
                .await?;
        }
        DeletionRoot::User => {
            user::Entity::update_many()
                .set(user::ActiveModel {
                    status: Set(UserStatus::Inactive),
                    updated_at: Set(now),
                    ..Default::default()
                })
                .filter(user::Column::Id.eq(root_id))
                .exec(txn)
                .await?;
        }
        DeletionRoot::Template
        | DeletionRoot::CourseModule
        | DeletionRoot::Lesson
        | DeletionRoot::Challenge
        | DeletionRoot::Role
        | DeletionRoot::TechnicalUser
        | DeletionRoot::Membership
        | DeletionRoot::Bit
        | DeletionRoot::ExecutionRun => {}
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
///
/// The body is a compare-and-set on the job's own lease, so re-running it
/// after an ambiguous commit would read back the claim it just made and report
/// "someone else holds it" while having burned an attempt: this is the one
/// place that must not retry an ambiguous commit.
pub async fn claim(state: &AppState, job_id: &str) -> Result<Option<Model>, ApiError> {
    let job_id = job_id.to_owned();
    retry_transaction::<_, Option<Model>, ApiError>(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::default(),
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

/// Write `model` onto the job only while it still carries `lease`.
///
/// A lapsed lease can be re-claimed by another worker at any moment, so an
/// unfenced write from the old pass would overwrite the new worker's phase,
/// status and attempts. `false` means the job now belongs to someone else.
async fn write_fenced(
    state: &AppState,
    job_id: &str,
    lease: Option<NaiveDateTime>,
    model: ActiveModel,
) -> Result<bool, ApiError> {
    let job_id = job_id.to_owned();
    state
        .transaction(move |txn| {
            let job_id = job_id.clone();
            let model = model.clone();
            Box::pin(async move {
                let mut query = Entity::update_many()
                    .set(model)
                    .filter(Column::Id.eq(job_id));
                if let Some(lease) = lease {
                    query = query.filter(Column::LeaseUntil.eq(lease));
                }
                Ok::<_, ApiError>(query.exec(txn).await?.rows_affected > 0)
            })
        })
        .await
}

/// The error a pass aborts with once a fenced write matched no row.
pub fn lease_lost(job_id: &str) -> ApiError {
    ApiError::internal_error(anyhow!(
        "deletion job {job_id} lost its lease to another worker"
    ))
}

/// Record progress and extend the lease, returning the new `leaseUntil` the
/// next write must be fenced on, or `None` when the lease was lost.
///
/// `progressed` resets `attempts`: a pass that moved rows is not a failed
/// attempt, so a long root cannot exhaust [`MAX_ATTEMPTS`] just by needing
/// more passes than that.
pub async fn checkpoint(
    state: &AppState,
    job_id: &str,
    lease: Option<NaiveDateTime>,
    phase: usize,
    cursor: serde_json::Value,
    progressed: bool,
) -> Result<Option<NaiveDateTime>, ApiError> {
    let now = now();
    let renewed = lease_from(now);
    let model = ActiveModel {
        phase: Set(i32::try_from(phase).unwrap_or(i32::MAX)),
        cursor: Set(Some(cursor)),
        lease_until: Set(Some(renewed)),
        attempts: if progressed {
            Set(0)
        } else {
            ActiveValue::NotSet
        },
        updated_at: Set(now),
        ..Default::default()
    };
    Ok(write_fenced(state, job_id, lease, model)
        .await?
        .then_some(renewed))
}

/// The pass ran out of budget; make the job claimable again right away.
pub async fn release(
    state: &AppState,
    job_id: &str,
    lease: Option<NaiveDateTime>,
    phase: usize,
    cursor: serde_json::Value,
    progressed: bool,
) -> Result<bool, ApiError> {
    write_fenced(
        state,
        job_id,
        lease,
        ActiveModel {
            phase: Set(i32::try_from(phase).unwrap_or(i32::MAX)),
            cursor: Set(Some(cursor)),
            lease_until: Set(None),
            attempts: if progressed {
            Set(0)
        } else {
            ActiveValue::NotSet
        },
            updated_at: Set(now()),
            ..Default::default()
        },
    )
    .await
}

pub async fn complete(
    state: &AppState,
    job_id: &str,
    lease: Option<NaiveDateTime>,
    cursor: serde_json::Value,
) -> Result<bool, ApiError> {
    write_fenced(
        state,
        job_id,
        lease,
        ActiveModel {
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

/// Record a failed pass. `attempts` counts *consecutive* failed passes, so the
/// caller passes 0 for a pass that made progress; the job goes back to the
/// queue until it has used [`MAX_ATTEMPTS`] of them, then stays `FAILED` for
/// an operator.
pub async fn fail(
    state: &AppState,
    job_id: &str,
    lease: Option<NaiveDateTime>,
    attempts: i32,
    phase: usize,
    error: &str,
) -> Result<bool, ApiError> {
    let status = status_after_failure(attempts);
    let message: String = error.chars().take(MAX_ERROR_CHARS).collect();
    write_fenced(
        state,
        job_id,
        lease,
        ActiveModel {
            status: Set(status.to_owned()),
            phase: Set(i32::try_from(phase).unwrap_or(i32::MAX)),
            attempts: Set(attempts),
            lease_until: Set(None),
            last_error: Set(Some(message)),
            updated_at: Set(now()),
            ..Default::default()
        },
    )
    .await
}

/// The status a pass's outcome writes for `attempts` consecutive failures.
pub fn status_after_failure(attempts: i32) -> &'static str {
    if attempts >= MAX_ATTEMPTS {
        STATUS_FAILED
    } else {
        STATUS_QUEUED
    }
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
