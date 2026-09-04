//! Paginated cascade deletion.
//!
//! Aurora DSQL caps a transaction at 3,000 mutated rows, cascades included,
//! so deleting an app cannot be one `DELETE FROM "App"`. Instead a root is
//! queued as a [`DeletionJob`](crate::entity::deletion_job) and drained
//! children-first in chunks of [`CHUNK`] rows, one table per transaction, by
//! whichever worker holds the job's lease. The same code path runs on
//! PostgreSQL and CockroachDB: by the time the root row is deleted every
//! engine-side cascade matches zero rows.
//!
//! ```text
//! enqueue(root, id)  ── one small transaction: DeletionJob row + tombstone
//!         │
//!         ▼
//! run_pass(job)      ── claims the lease, walks plan_for(root).steps from
//!                       job.phase, writes the cursor every 10 chunks,
//!                       suspends when the pass budget is spent
//! ```
//!
//! # Where the pieces live
//!
//! * [`graph`] — the foreign-key edges of every entity, pinned by
//!   `edges.snapshot.txt`.
//! * [`overrides`] — what metadata cannot express: external stores, non-FK
//!   rows keyed by the root, references kept on purpose.
//! * [`plan`] — the ordered [`Step`] list for a root.
//! * [`drain`] — page selects and per-chunk transactions.
//! * [`job`] — `DeletionJob` rows: enqueue, lease, cursor, outcome.
//! * [`external`] — object stores, schedulers, cache backends.
//! * [`worker`] — the in-process ticker; `POST /maintenance/run` drives the
//!   same [`run_queue`] on Lambda and cron deployments.
//!
//! # App plan order
//!
//! `plan_for(DeletionRoot::App)` is a Kahn topological sort of the tables
//! reachable over `Cascade` edges, child before parent, ties by table name.
//! Grouped by what the constraints force (the exact sequence is asserted by
//! the tests in [`plan`]):
//!
//! 1. `Tombstone` — `App.status = INACTIVE`, `visibility = OFFLINE`.
//! 2. `External(AppSinkSchedules)` — cron schedules, read from `EventSink`
//!    before those rows go.
//! 3. Grandchildren before their parents: `ExecutionEvent` and
//!    `ExecutionRunCallerApp` → `ExecutionRun`; `RegressionCaseResult` →
//!    `RegressionSuiteRun`; `PublicationLog` → `PublicationRequest`;
//!    `Meta`/`Comment`/`Feedback` → `Template`; `Meta` → `Widget`;
//!    `AppGroupMember`/`Meta`/`PublicationRequest` → `AppGroup`;
//!    `Invitation`/`TechnicalUser` → `Membership`;
//!    `EventRemoteRegistration` → `EventRemoteAuth` → `Event`, with
//!    `EventSetup`/`EventAlias`/`EventSink` before `Event`.
//! 4. `NullOut` steps sit immediately before the parent they protect and
//!    after the in-set child drained, so they only touch rows outside the
//!    app: the four usage tables' `technicalUserId` before `TechnicalUser`,
//!    `EventRemoteRegistration.authId` before `EventRemoteAuth`,
//!    `TechnicalUser.roleId`/`AppConnection.roleId` and the back-edge
//!    `App.defaultRoleId`/`App.ownerRoleId` before `Role`.
//! 5. `Membership` before `Role` (`Membership.roleId` is `Restrict`).
//! 6. Every table with a direct `appId` cascade that nothing else orders.
//! 7. `SweepSoft` — `AppCacheEntry`, `UsageInvocation`, `UsageAlert`,
//!    `UsageLimitAuditLog`, `FlowScriptApplyFailure` by `appId`.
//! 8. `External(AppStoragePrefixes)`, `External(AppCacheBackend)`.
//! 9. `DeleteRoot`.
//!
//! A table reachable along several cascade paths (`Meta` via `App`,
//! `Template`, `Widget`, `AppGroup`) drains once with one predicate per path;
//! tables without the root column are selected through nested `IN (SELECT …)`
//! subqueries along their path, never through unbounded id lists.
//!
//! # Contract for adopters
//!
//! * [`enqueue`] is idempotent on `(rootKind, rootId)` and returns the job.
//! * [`run_pass`] may be called inline right after enqueueing to finish small
//!   roots synchronously; a `Suspended` outcome means the worker takes over.
//! * Roles are not a job root: `plan_for(Role)` fails with
//!   `RestrictNotCovered` because memberships must be reassigned first.

pub mod drain;
pub mod external;
pub mod graph;
pub mod job;
pub mod overrides;
pub mod plan;
pub mod request;
pub mod worker;

use std::time::Instant;

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use drain::{CHUNK, PassBudget};
pub use external::ExternalStep;
pub use graph::{FkEdge, FkGraph, fk_graph};
pub use plan::{Plan, PlanError, Predicate, RootKey, Step, plan_for};
pub use request::{AcceptedDeletion, Deleted, delete_now};
pub use worker::{DeletionWorkerConfig, spawn_deletion_worker};

use crate::entity::deletion_job;
use crate::error::ApiError;
use crate::state::AppState;
use drain::{DrainOp, Flow, Pass};

/// Jobs claimed by one queue pass.
const QUEUE_BATCH: u64 = 10;

/// The row kinds a deletion job can start from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeletionRoot {
    App,
    User,
    WasmPackage,
    Course,
    CourseModule,
    Lesson,
    Challenge,
    LearningPath,
    Event,
    Role,
    TechnicalUser,
    Membership,
    AppGroup,
    Template,
    Bit,
    ExecutionRun,
}

impl DeletionRoot {
    pub const ALL: [Self; 16] = [
        Self::App,
        Self::User,
        Self::WasmPackage,
        Self::Course,
        Self::CourseModule,
        Self::Lesson,
        Self::Challenge,
        Self::LearningPath,
        Self::Event,
        Self::Role,
        Self::TechnicalUser,
        Self::Membership,
        Self::AppGroup,
        Self::Template,
        Self::Bit,
        Self::ExecutionRun,
    ];

    pub fn table_name(self) -> &'static str {
        match self {
            Self::App => "App",
            Self::User => "User",
            Self::WasmPackage => "WasmPackage",
            Self::Course => "Course",
            Self::CourseModule => "CourseModule",
            Self::Lesson => "Lesson",
            Self::Challenge => "Challenge",
            Self::LearningPath => "LearningPath",
            Self::Event => "Event",
            Self::Role => "Role",
            Self::TechnicalUser => "TechnicalUser",
            Self::Membership => "Membership",
            Self::AppGroup => "AppGroup",
            Self::Template => "Template",
            Self::Bit => "Bit",
            Self::ExecutionRun => "ExecutionRun",
        }
    }

    /// The `DeletionJob.rootKind` value.
    pub fn kind(self) -> &'static str {
        self.table_name()
    }

    pub fn parse(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|root| root.kind() == kind)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum PassOutcome {
    Completed,
    /// The pass budget ran out; the job is claimable again immediately.
    Suspended {
        phase: usize,
    },
    Failed {
        error: String,
    },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct QueueReport {
    pub claimed: u64,
    pub completed: u64,
    pub suspended: u64,
    pub failed: u64,
}

/// Queue `root_id` for deletion and tombstone it. See [`job::enqueue`].
pub async fn enqueue(
    state: &AppState,
    root: DeletionRoot,
    root_id: &str,
    requested_by: Option<&str>,
) -> Result<deletion_job::Model, ApiError> {
    job::enqueue(state, root, root_id, requested_by).await
}

/// Drive a claimed job through its plan until it finishes, the budget runs
/// out, or a step fails. The caller must hold the job's lease
/// ([`job::claim`]); the outcome is persisted on the job row before returning.
pub async fn run_pass(
    state: &AppState,
    job: &deletion_job::Model,
    budget: PassBudget,
) -> Result<PassOutcome, ApiError> {
    let root = DeletionRoot::parse(&job.root_kind).ok_or_else(|| {
        ApiError::bad_request(format!("unknown deletion root kind {}", job.root_kind))
    })?;
    let plan = plan_for(root)?;
    let mut pass = Pass::new(state, job, plan.steps.len(), budget);
    let span = tracing::info_span!("deletion_pass", job_id = %job.id, root = root.kind(), root_id = %job.root_id);
    let _guard = span.enter();

    let outcome = drive(state, &plan, &mut pass).await;
    let progressed = pass.rows() > 0;
    match outcome {
        Ok(Flow::Continue) => {
            if !job::complete(state, &job.id, pass.lease(), pass.cursor()).await? {
                return Ok(lease_lost(&job.id, pass.phase));
            }
            tracing::info!(rows = pass.rows(), "Deletion job completed");
            Ok(PassOutcome::Completed)
        }
        Ok(Flow::Suspend) => {
            if !job::release(
                state,
                &job.id,
                pass.lease(),
                pass.phase,
                pass.cursor(),
                progressed,
            )
            .await?
            {
                return Ok(lease_lost(&job.id, pass.phase));
            }
            tracing::info!(
                phase = pass.phase,
                rows = pass.rows(),
                "Deletion pass suspended on budget"
            );
            Ok(PassOutcome::Suspended { phase: pass.phase })
        }
        // The pass lost its lease mid-flight: the job belongs to another
        // worker now and writing an outcome would rewind that worker's row.
        Err(error) if pass.lease_lost() => {
            tracing::warn!(phase = pass.phase, error = %error, "Deletion pass abandoned");
            Ok(PassOutcome::Failed {
                error: error.to_string(),
            })
        }
        Err(error) => {
            let message = error.to_string();
            // `attempts` counts consecutive failed passes, so a pass that
            // drained rows before erroring does not spend one.
            let attempts = if progressed { 0 } else { job.attempts };
            if !job::fail(state, &job.id, pass.lease(), attempts, pass.phase, &message).await? {
                return Ok(lease_lost(&job.id, pass.phase));
            }
            tracing::error!(
                phase = pass.phase,
                attempts,
                error = %message,
                "Deletion pass failed"
            );
            Ok(PassOutcome::Failed { error: message })
        }
    }
}

fn lease_lost(job_id: &str, phase: usize) -> PassOutcome {
    let error = job::lease_lost(job_id).to_string();
    tracing::warn!(phase, error = %error, "Deletion outcome not written");
    PassOutcome::Failed { error }
}

async fn drive(state: &AppState, plan: &Plan, pass: &mut Pass<'_>) -> Result<Flow, ApiError> {
    let graph = fk_graph();
    let table_meta = |name: &str| {
        graph
            .table(name)
            .cloned()
            .ok_or_else(|| PlanError::UnknownTable(name.to_owned()))
    };
    while pass.phase < plan.steps.len() {
        let step = &plan.steps[pass.phase];
        tracing::debug!(phase = pass.phase, step = %step, "Deletion step");
        let flow = match step {
            Step::Tombstone => {
                job::tombstone_root(state, plan.root, &pass.root_id).await?;
                Flow::Continue
            }
            // An external step is not chunked at its own boundary, so the
            // budget is tested before it starts as well as inside it.
            Step::External(external_step) => {
                if pass.exhausted() {
                    return Ok(Flow::Suspend);
                }
                pass.checkpoint().await?;
                external::run(state, *external_step, pass).await?
            }
            Step::NullOut {
                table,
                column,
                predicates,
            } => {
                drain::drain(
                    pass,
                    &table_meta(table)?,
                    &DrainOp::SetNull {
                        column: column.clone(),
                    },
                    predicates,
                )
                .await?
            }
            Step::Drain { table, predicates } => {
                drain::drain(pass, &table_meta(table)?, &DrainOp::Delete, predicates).await?
            }
            Step::SweepSoft { table, column } => {
                drain::drain(
                    pass,
                    &table_meta(table)?,
                    &DrainOp::Delete,
                    &[Predicate::Root {
                        column: column.clone(),
                    }],
                )
                .await?
            }
            Step::DeleteRoot => {
                drain::delete_root(pass, &table_meta(plan.root.table_name())?).await?;
                Flow::Continue
            }
        };
        if flow == Flow::Suspend {
            return Ok(Flow::Suspend);
        }
        pass.advance().await?;
    }
    Ok(Flow::Continue)
}

/// Claim due jobs and run one pass each until `budget.max_duration` is spent.
pub async fn run_queue(state: &AppState, budget: PassBudget) -> Result<QueueReport, ApiError> {
    let started = Instant::now();
    let mut report = QueueReport::default();
    for due in job::due_jobs(state, QUEUE_BATCH).await? {
        let elapsed = started.elapsed();
        if elapsed >= budget.max_duration {
            break;
        }
        let Some(job) = job::claim(state, &due.id).await? else {
            continue;
        };
        report.claimed += 1;
        let remaining = PassBudget {
            max_chunks: budget.max_chunks,
            max_duration: budget.max_duration - elapsed,
        };
        match run_pass(state, &job, remaining).await? {
            PassOutcome::Completed => report.completed += 1,
            PassOutcome::Suspended { .. } => report.suspended += 1,
            PassOutcome::Failed { .. } => report.failed += 1,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_kinds_round_trip() {
        for root in DeletionRoot::ALL {
            assert_eq!(DeletionRoot::parse(root.kind()), Some(root));
        }
        assert_eq!(DeletionRoot::parse("Swimlane"), None);
    }
}
