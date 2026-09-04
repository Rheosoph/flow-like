use crate::{
    audit_branch,
    db::delete_in_batches,
    ensure_permission,
    entity::{app, membership, publication_log, sea_orm_active_enums::Visibility},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    publication::{PublicationTarget, target::new_request},
    routes::app::team::remove_user::detach_membership_children,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Memberships per purge transaction. Each membership drags its packages,
/// invitations and created API keys along, so this stays well under the
/// per-transaction row budget.
const MEMBERSHIP_PURGE_CHUNK: usize = 500;

/// Delete every membership matching `condition`, page by page.
///
/// A membership cascades into its invitations and the technical users it
/// created, and deleting a technical user nulls its rows in four usage tables —
/// tens of thousands of rows for a single API key. Detaching those children in
/// their own bounded sweeps first leaves the membership rows as the only work
/// the delete transaction has to do.
async fn purge_memberships(state: &AppState, condition: &Condition) -> Result<(), ApiError> {
    loop {
        let ids: Vec<String> = membership::Entity::find()
            .filter(condition.clone())
            .select_only()
            .column(membership::Column::Id)
            .order_by_asc(membership::Column::Id)
            .limit(MEMBERSHIP_PURGE_CHUNK as u64)
            .into_tuple()
            .all(&state.db)
            .await?;
        if ids.is_empty() {
            return Ok(());
        }
        for membership_id in &ids {
            detach_membership_children(state, membership_id).await?;
        }
        let removed = delete_in_batches::<membership::Entity>(
            &state.db,
            state.db_dialect,
            condition.clone().add(membership::Column::Id.is_in(ids)),
            MEMBERSHIP_PURGE_CHUNK,
            Some(1),
        )
        .await?;
        if removed.rows == 0 {
            return Ok(());
        }
    }
}

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateVisibilityBody {
    #[schema(value_type = String)]
    pub visibility: Visibility,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transition {
    /// Private <-> Prototype: no restrictions.
    Toggle,
    /// Public <-> Public Request Join: no restrictions.
    PublicSwap,
    /// Prototype -> Public / Public Request Join: goes to review.
    Review,
}

fn transition(from: &Visibility, to: &Visibility) -> Option<Transition> {
    let restricted = |v: &Visibility| matches!(v, Visibility::Private | Visibility::Prototype);
    let public = |v: &Visibility| matches!(v, Visibility::Public | Visibility::PublicRequestAccess);
    if restricted(from) && restricted(to) {
        Some(Transition::Toggle)
    } else if public(from) && public(to) {
        Some(Transition::PublicSwap)
    } else if *from == Visibility::Prototype && public(to) {
        Some(Transition::Review)
    } else {
        None
    }
}

/// The following visibility changes are allowed:
/// - From Private to Prototype (no restrictions)
/// - From Public to Public Request Join (no restrictions)
/// - From Public Request Join to Public (no restrictions)
///
/// - From Prototype to Private (all users except the owner are removed)
/// - From Prototype to Public (goes to review)
/// - From Prototype to Public Request Join (goes to review)
/// - From Public to Prototype (requires review -> might be a paid app for example)
/// - From Public Request Join to Prototype (requires review -> might be a paid app for example)
#[utoipa::path(
    patch,
    path = "/apps/{app_id}/visibility",
    tag = "apps",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UpdateVisibilityBody,
    responses(
        (status = 200, description = "Visibility updated"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "PATCH /apps/{app_id}/visibility", skip(state, user, body))]
pub async fn change_visibility(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<UpdateVisibilityBody>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub = user.sub()?;

    let app = app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if app.visibility == body.visibility {
        tracing::warn!(
            "App {} already has visibility set to {:?}",
            app_id,
            body.visibility
        );
        return Ok(Json(()));
    }

    let Some(transition) = transition(&app.visibility, &body.visibility) else {
        return Err(ApiError::FORBIDDEN);
    };
    let target = body.visibility.clone();
    let purge_members = transition == Transition::Toggle && target == Visibility::Private;
    let other_members = Condition::all()
        .add(membership::Column::AppId.eq(app_id.clone()))
        .add(membership::Column::UserId.ne(sub.clone()));

    // Going private removes every other member. The purge runs in bounded
    // batches around the flip rather than inside it: the fan-out behind one
    // membership does not fit a transaction that also has to stay atomic with
    // the visibility change.
    if purge_members {
        purge_memberships(&state, &other_members).await?;
    }

    let request_id = create_id();
    let log_id = create_id();

    state
        .transaction(|txn| {
            let app_id = app_id.clone();
            let sub = sub.clone();
            let target = target.clone();
            let request_id = request_id.clone();
            let log_id = log_id.clone();
            Box::pin(async move {
                let app = app::Entity::find_by_id(&app_id)
                    .one(txn)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;
                let now = chrono::Utc::now().naive_utc();

                match transition {
                    Transition::Toggle | Transition::PublicSwap => {
                        let mut app = app.into_active_model();
                        app.visibility = Set(target);
                        app.updated_at = Set(now);
                        app.update(txn).await?;
                    }
                    Transition::Review => {
                        let old_visibility = app.visibility.clone();
                        let mut updated_app = app.into_active_model();
                        updated_app.updated_at = Set(now);

                        new_request(
                            request_id.clone(),
                            &PublicationTarget::App(app_id),
                            target,
                            None,
                            now,
                        )
                        .insert(txn)
                        .await?;

                        publication_log::ActiveModel {
                            id: Set(log_id),
                            author_id: Set(Some(sub)),
                            request_id: Set(request_id),
                            message: Set(Some("Request initiated".to_string())),
                            visibility: Set(Some(old_visibility)),
                            created_at: Set(now),
                            updated_at: Set(now),
                        }
                        .insert(txn)
                        .await?;
                        updated_app.update(txn).await?;
                    }
                }
                Ok::<_, ApiError>(())
            })
        })
        .await?;

    // Nobody can join a private app, so this second sweep only has to catch
    // whoever slipped in between the first one and the flip.
    if purge_members {
        purge_memberships(&state, &other_members).await?;
    }

    let (action, summary) = match transition {
        Transition::Toggle | Transition::PublicSwap => (
            "app.visibility",
            format!("Visibility changed to {:?}", body.visibility),
        ),
        Transition::Review => (
            "app.visibility.request",
            format!("Publication review requested for {:?}", body.visibility),
        ),
    };
    audit_branch!(state, user, app_id, action, "App", app_id, summary);
    Ok(Json(()))
}
