//! `DeletionJob` rows: one per root, claimed by lease, resumable by phase.

use std::time::Duration;

use chrono::{DateTime, FixedOffset, SubsecRound};
use flow_like_types::{anyhow, create_id};
use sea_orm::sea_query::{
    Expr, ExprTrait, IntoColumnRef, OnConflict, Query as SeaQuery, SimpleExpr,
};
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

/// Now, truncated to the millisecond the `timestamptz(3)` columns store.
///
/// `leaseUntil` is fenced on with an equality compare in [`write_fenced`], and
/// [`checkpoint`] hands the caller the value it just wrote rather than reading
/// it back. At the microsecond precision `Utc::now` carries, that value never
/// equals the millisecond one the column actually stores, so every fenced
/// write after the first checkpoint matched no row and reported a lost lease —
/// leaving the job stuck at its current phase until it exhausted
/// [`MAX_ATTEMPTS`]. Minting the instant at the storage precision keeps the
/// value the job holds and the value the row carries identical.
fn now() -> DateTime<FixedOffset> {
    chrono::Utc::now().trunc_subsecs(3).fixed_offset()
}

fn lease_from(now: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
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

/// The job currently attached to `(root, root_id)`, read on the pool.
///
/// [`enqueue`] resurrects a `FAILED` job from phase 0, so a caller that has to
/// know whether a deletion is parked must read it *before* enqueueing again.
pub async fn find(
    state: &AppState,
    root: DeletionRoot,
    root_id: &str,
) -> Result<Option<Model>, ApiError> {
    Ok(Entity::find()
        .filter(Column::RootKind.eq(root.kind()))
        .filter(Column::RootId.eq(root_id))
        .one(&state.db)
        .await?)
}

/// Row ids whose deletion has not finished yet must stay out of listings:
/// [`super::delete_now`] answers `202` with the root row still present, so
/// without this predicate a caller sees a live-looking root whose children and
/// storage are already going.
pub fn not_pending_deletion(root: DeletionRoot, id_col: impl IntoColumnRef) -> SimpleExpr {
    let mut pending = SeaQuery::select();
    pending
        .expr(Expr::val(1))
        .from(Entity)
        .and_where(Expr::col((Entity, Column::RootKind)).eq(root.kind()))
        .and_where(Expr::col((Entity, Column::Status)).ne(STATUS_DONE))
        .and_where(Expr::col((Entity, Column::RootId)).equals(id_col));
    Expr::not_exists(pending)
}

/// What a `DeletionJob` in `status` means for re-creating its root under the
/// same id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReuseVerdict {
    /// A worker holds the job; its drains would race the new rows.
    Busy,
    /// The job row has to go before the root is written again.
    ClearJob,
}

/// A `DONE` job is cleared too: [`enqueue`] is idempotent on
/// `(rootKind, rootId)` and hands back a finished job unchanged, so leaving the
/// row behind would make the next delete of the re-created root a no-op.
pub fn reuse_verdict(status: &str) -> ReuseVerdict {
    if status == STATUS_RUNNING {
        ReuseVerdict::Busy
    } else {
        ReuseVerdict::ClearJob
    }
}

/// Drop the deletion job of a root that is being re-created under the same id.
///
/// Caller-supplied ids are stable — the University CLI republishes a manifest
/// under the ids it already used — so a job left over from a `202` would drain
/// the rows the caller just authored. Runs in the caller's transaction so the
/// job cannot be re-claimed between the check and the insert.
pub async fn cancel(
    txn: &DatabaseTransaction,
    root: DeletionRoot,
    root_id: &str,
) -> Result<(), ApiError> {
    let Some(job) = find_by_root(txn, root.kind(), root_id).await? else {
        return Ok(());
    };
    match reuse_verdict(&job.status) {
        ReuseVerdict::Busy => Err(ApiError::conflict(format!(
            "{} {root_id} is being deleted; retry once the deletion job has finished",
            root.table_name()
        ))),
        ReuseVerdict::ClearJob => {
            Entity::delete_by_id(job.id).exec(txn).await?;
            Ok(())
        }
    }
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
    lease: Option<DateTime<FixedOffset>>,
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
    lease: Option<DateTime<FixedOffset>>,
    phase: usize,
    cursor: serde_json::Value,
    progressed: bool,
) -> Result<Option<DateTime<FixedOffset>>, ApiError> {
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
    lease: Option<DateTime<FixedOffset>>,
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
    lease: Option<DateTime<FixedOffset>>,
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
    lease: Option<DateTime<FixedOffset>>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deletion::graph::fk_graph;
    use crate::entity::course;
    use sea_orm::sea_query::PostgresQueryBuilder;

    /// Every date column is `timestamptz(3)`, and `checkpoint` fences the next
    /// write on the `leaseUntil` value it just wrote instead of reading it
    /// back. A sub-millisecond instant therefore never matches the stored row:
    /// against a live Aurora DSQL cluster the pass tombstoned the root, moved
    /// to phase 1 and then failed every subsequent fenced write with "lost its
    /// lease to another worker", so no cascade could ever finish.
    #[test]
    fn the_lease_instant_is_minted_at_the_stored_precision() {
        let now = now();
        assert_eq!(
            now.timestamp_subsec_micros() % 1_000,
            0,
            "now() carries sub-millisecond precision the column cannot store, \
             so a fenced write comparing it would match no row"
        );
        let lease = lease_from(now);
        assert_eq!(
            lease.timestamp_subsec_micros() % 1_000,
            0,
            "lease_from must stay at the stored precision"
        );
        assert_eq!(lease, lease.trunc_subsecs(3), "lease must round-trip");
    }

    /// H2: `job::cancel` was wired into the *insert* branch of each
    /// id-accepting upsert only. A `202` merely tombstones the root — the row
    /// survives until the drain reaches `DeleteRoot` — so re-authoring the id
    /// (the University CLI republishing a manifest under ids it already used)
    /// lands in the **update** branch, and the worker later deletes the rows
    /// that were just written. The cancel has to sit above the branch, in the
    /// transaction that writes the row.
    #[test]
    fn re_creating_a_root_cancels_its_deletion_job_through_the_update_branch() {
        // (source file, the cancel call, the two branch writes it must cover)
        const SITES: [(&str, &str, [&str; 2]); 6] = [
            (
                "src/routes/course/courses.rs",
                "job::cancel(txn, DeletionRoot::Course,",
                [".update(txn)", ".insert(txn)"],
            ),
            (
                "src/routes/course/modules.rs",
                "job::cancel(txn, DeletionRoot::CourseModule,",
                [".update(txn)", ".insert(txn)"],
            ),
            (
                "src/routes/course/lessons.rs",
                "job::cancel(txn, DeletionRoot::Lesson,",
                [".update(txn)", ".insert(txn)"],
            ),
            (
                "src/routes/course/challenges.rs",
                "job::cancel(txn, DeletionRoot::Challenge,",
                [".update(txn)", ".insert(txn)"],
            ),
            (
                "src/routes/course/paths.rs",
                "job::cancel(txn, DeletionRoot::LearningPath,",
                [".update(txn)", ".insert(txn)"],
            ),
            (
                "src/routes/registry/server.rs",
                "job::cancel(&package_write, DeletionRoot::WasmPackage,",
                [".update(&package_write)", ".insert(&package_write)"],
            ),
        ];
        /// Both writes are the arms of one `if let` under the cancel, so they
        /// sit a few dozen lines below it; anything further away is a write
        /// outside the transaction that cancels.
        const BODY_LINES: usize = 60;

        for (relative, cancel, writes) in SITES {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
            let body = std::fs::read_to_string(&path).expect("readable source file");
            let lines: Vec<&str> = body.lines().collect();
            let cancel_at = lines
                .iter()
                .position(|line| line.contains(cancel))
                .unwrap_or_else(|| panic!("{relative} no longer calls `{cancel}`"));
            let window = lines[cancel_at..lines.len().min(cancel_at + BODY_LINES)].join("\n");
            for write in writes {
                assert!(
                    window.contains(write),
                    "{relative}: `{write}` is not covered by the `{cancel}` above it — \
                     a root re-created through that branch keeps its pending deletion job"
                );
            }
        }
    }

    /// `upsert_template` splits its branches across two functions, so each
    /// carries its own cancel; the update branch used to write to the pool.
    #[test]
    fn both_template_upsert_branches_cancel_a_pending_deletion() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/routes/app/template/upsert_template.rs");
        let body = std::fs::read_to_string(&path).expect("readable source file");
        assert_eq!(
            body.matches("job::cancel(txn, DeletionRoot::Template,")
                .count(),
            2,
            "create_template and upsert_template must each cancel a pending deletion job"
        );
        assert!(
            !body.contains("template.update(&state.db)"),
            "the update branch has to run in the transaction that cancels the job"
        );
    }

    /// The package row and its cancel share one transaction; without the
    /// commit the publish would roll the row back.
    #[test]
    fn the_wasm_package_publish_commits_its_cancel_transaction() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes/registry/server.rs");
        let body = std::fs::read_to_string(&path).expect("readable source file");
        assert!(
            body.contains("package_write.commit()"),
            "{}",
            path.display()
        );
    }

    #[test]
    fn a_queued_or_finished_job_is_cleared_and_a_running_one_blocks() {
        assert_eq!(reuse_verdict(STATUS_QUEUED), ReuseVerdict::ClearJob);
        assert_eq!(reuse_verdict(STATUS_FAILED), ReuseVerdict::ClearJob);
        assert_eq!(reuse_verdict(STATUS_DONE), ReuseVerdict::ClearJob);
        assert_eq!(reuse_verdict(STATUS_RUNNING), ReuseVerdict::Busy);
    }

    #[test]
    fn listing_predicate_correlates_the_root_id_and_ignores_finished_jobs() {
        let sql = SeaQuery::select()
            .expr(Expr::val(1))
            .from(course::Entity)
            .and_where(not_pending_deletion(
                DeletionRoot::Course,
                (course::Entity, course::Column::Id),
            ))
            .to_string(PostgresQueryBuilder);

        assert!(
            sql.contains("NOT EXISTS(SELECT 1 FROM \"DeletionJob\""),
            "{sql}"
        );
        assert!(
            sql.contains(r#""DeletionJob"."rootId" = "Course"."id""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#""DeletionJob"."rootKind" = 'Course'"#),
            "{sql}"
        );
        assert!(sql.contains(r#""DeletionJob"."status" <> 'DONE'"#), "{sql}");
    }

    /// `attempts` counts consecutive failed passes: a pass that drained rows
    /// reports 0, so a root that simply needs more than [`MAX_ATTEMPTS`]
    /// passes never parks itself at `FAILED`.
    #[test]
    fn progress_resets_the_failure_budget() {
        assert_eq!(status_after_failure(0), STATUS_QUEUED);
        assert_eq!(status_after_failure(MAX_ATTEMPTS - 1), STATUS_QUEUED);
        assert_eq!(status_after_failure(MAX_ATTEMPTS), STATUS_FAILED);
        assert_eq!(status_after_failure(MAX_ATTEMPTS + 1), STATUS_FAILED);
    }

    /// Every root whose table carries a state column must be flipped by
    /// [`tombstone`]; the rest are only hideable through their listings.
    #[test]
    fn every_root_with_a_state_column_is_tombstoned() {
        const STATE_COLUMNS: [&str; 4] = ["status", "visibility", "isPublished", "active"];
        const TOMBSTONED: [DeletionRoot; 7] = [
            DeletionRoot::App,
            DeletionRoot::AppGroup,
            DeletionRoot::WasmPackage,
            DeletionRoot::Course,
            DeletionRoot::LearningPath,
            DeletionRoot::Event,
            DeletionRoot::User,
        ];
        for root in DeletionRoot::ALL {
            // `ExecutionRun.status` records how the run ended, not whether it
            // is visible; a run is hidden by its own `expiresAt` sweep.
            if root == DeletionRoot::ExecutionRun {
                continue;
            }
            let table = root.table_name();
            let meta = fk_graph().table(table).expect(table);
            let stateful = STATE_COLUMNS.iter().any(|column| meta.has_column(column));
            assert_eq!(
                stateful,
                TOMBSTONED.contains(&root),
                "{table} carries a state column but tombstone() leaves it visible \
                 (or the reverse); add an arm or a listing filter"
            );
        }
    }
}
