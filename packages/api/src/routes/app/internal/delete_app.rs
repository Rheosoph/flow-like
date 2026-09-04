use crate::{
    audit_branch,
    deletion::{self, AcceptedDeletion, Deleted, DeletionRoot, job},
    ensure_permission,
    entity::{app, deletion_job},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension,
    extract::{Path, State},
};
use sea_orm::sea_query::{Expr, ExprTrait, IntoColumnRef, Query as SeaQuery, SimpleExpr};
use sea_orm::{
    ColumnTrait, DatabaseTransaction, EntityTrait, ModelTrait, QueryFilter,
};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}",
    tag = "apps",
    description = "Delete an application with everything it owns. The app is hidden immediately; a small app is fully removed before the response, a large one is drained by the deletion queue.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Application deleted"),
        (status = 202, description = "Application hidden and queued for deletion; follow the job on `GET /admin/deletions/{job_id}`", body = AcceptedDeletion),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}", skip(state, user))]
pub async fn delete_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Deleted<()>, ApiError> {
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub_id = sub.sub()?;

    sub.role
        .find_related(app::Entity)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    // Storage prefixes, sink schedules and the cache backend are steps of the
    // plan, so they run outside every database transaction and re-run on a
    // resumed pass.
    let deleted =
        deletion::delete_now(&state, DeletionRoot::App, &app_id, Some(&sub_id), ()).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app.delete",
        "App",
        app_id,
        "Application deleted"
    );
    Ok(deleted)
}

/// Row ids whose deletion has not finished yet must stay out of listings:
/// [`deletion::delete_now`] answers `202` with the root row still present, so
/// without this predicate a caller sees a live-looking root whose children and
/// storage are already going.
pub(crate) fn not_pending_deletion(root: DeletionRoot, id_col: impl IntoColumnRef) -> SimpleExpr {
    let mut pending = SeaQuery::select();
    pending
        .expr(Expr::val(1))
        .from(deletion_job::Entity)
        .and_where(
            Expr::col((deletion_job::Entity, deletion_job::Column::RootKind)).eq(root.kind()),
        )
        .and_where(
            Expr::col((deletion_job::Entity, deletion_job::Column::Status)).ne(job::STATUS_DONE),
        )
        .and_where(Expr::col((deletion_job::Entity, deletion_job::Column::RootId)).equals(id_col));
    Expr::not_exists(pending)
}

/// What a `DeletionJob` in `status` means for re-creating its root under the
/// same id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReuseVerdict {
    /// A worker holds the job; its drains would race the new rows.
    Busy,
    /// The job row has to go before the root is written again.
    ClearJob,
}

/// A `DONE` job is cleared too: [`deletion::job::enqueue`] is idempotent on
/// `(rootKind, rootId)` and hands back a finished job unchanged, so leaving the
/// row behind would make the next delete of the re-created root a no-op.
pub(crate) fn reuse_verdict(status: &str) -> ReuseVerdict {
    if status == job::STATUS_RUNNING {
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
pub(crate) async fn cancel_pending_deletion(
    txn: &DatabaseTransaction,
    root: DeletionRoot,
    root_id: &str,
) -> Result<(), ApiError> {
    let Some(job) = deletion_job::Entity::find()
        .filter(deletion_job::Column::RootKind.eq(root.kind()))
        .filter(deletion_job::Column::RootId.eq(root_id))
        .one(txn)
        .await?
    else {
        return Ok(());
    };
    match reuse_verdict(&job.status) {
        ReuseVerdict::Busy => Err(ApiError::conflict(format!(
            "{} {root_id} is being deleted; retry once the deletion job has finished",
            root.table_name()
        ))),
        ReuseVerdict::ClearJob => {
            deletion_job::Entity::delete_by_id(job.id).exec(txn).await?;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::course;
    use sea_orm::sea_query::PostgresQueryBuilder;

    #[test]
    fn a_queued_or_finished_job_is_cleared_and_a_running_one_blocks() {
        assert_eq!(reuse_verdict(job::STATUS_QUEUED), ReuseVerdict::ClearJob);
        assert_eq!(reuse_verdict(job::STATUS_FAILED), ReuseVerdict::ClearJob);
        assert_eq!(reuse_verdict(job::STATUS_DONE), ReuseVerdict::ClearJob);
        assert_eq!(reuse_verdict(job::STATUS_RUNNING), ReuseVerdict::Busy);
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
        assert!(sql.contains(r#""DeletionJob"."rootKind" = 'Course'"#), "{sql}");
        assert!(sql.contains(r#""DeletionJob"."status" <> 'DONE'"#), "{sql}");
    }
}
