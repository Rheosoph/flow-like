use crate::{
    audit_branch, ensure_permission,
    entity::{
        app, membership, publication_log, publication_request, sea_orm_active_enums::Visibility,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateVisibilityBody {
    #[schema(value_type = String)]
    pub visibility: Visibility,
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

    let txn = state.db.begin().await?;

    let app = app::Entity::find()
        .filter(app::Column::Id.eq(&app_id))
        .one(&txn)
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

    // The user should be able to switch between Prototype and Private visibility without restrictions
    if matches!(app.visibility, Visibility::Private | Visibility::Prototype)
        && matches!(body.visibility, Visibility::Private | Visibility::Prototype)
    {
        let mut app = app.into_active_model();
        app.visibility = Set(body.visibility.clone());
        app.updated_at = Set(chrono::Utc::now().naive_utc());
        app.update(&txn).await?;

        // If the visibility is changed to Private, remove all other users
        if body.visibility == Visibility::Private {
            membership::Entity::delete_many()
                .filter(membership::Column::AppId.eq(&app_id))
                .filter(membership::Column::UserId.ne(sub.clone()))
                .exec(&txn)
                .await?;
        }

        txn.commit().await?;
        audit_branch!(
            state,
            user,
            app_id,
            "app.visibility",
            "App",
            app_id,
            format!("Visibility changed to {:?}", body.visibility)
        );
        return Ok(Json(()));
    }

    if matches!(
        app.visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    ) && matches!(
        body.visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    ) {
        let mut app = app.into_active_model();
        app.visibility = Set(body.visibility.clone());
        app.updated_at = Set(chrono::Utc::now().naive_utc());

        app.update(&txn).await?;
        txn.commit().await?;
        audit_branch!(
            state,
            user,
            app_id,
            "app.visibility",
            "App",
            app_id,
            format!("Visibility changed to {:?}", body.visibility)
        );
        return Ok(Json(()));
    }

    if app.visibility == Visibility::Prototype
        && matches!(
            body.visibility,
            Visibility::Public | Visibility::PublicRequestAccess
        )
    {
        let old_visibility = app.visibility.clone();
        let mut updated_app = app.into_active_model();
        updated_app.updated_at = Set(chrono::Utc::now().naive_utc());

        let request = publication_request::ActiveModel {
            id: Set(create_id()),
            app_id: Set(app_id.clone()),
            approver_id: NotSet,
            target_visibility: Set(body.visibility.clone()),
            status: Set(crate::entity::sea_orm_active_enums::PublicationRequestStatus::Pending),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        };

        let request = request.insert(&txn).await?;

        let log_entry = publication_log::ActiveModel {
            id: Set(create_id()),
            author_id: Set(Some(sub.clone())),
            request_id: Set(request.id),
            message: Set(Some("Request initiated".to_string())),
            visibility: Set(Some(old_visibility)),
            created_at: Set(chrono::Utc::now().naive_utc()),
            updated_at: Set(chrono::Utc::now().naive_utc()),
        };

        log_entry.insert(&txn).await?;
        updated_app.update(&txn).await?;
        txn.commit().await?;
        audit_branch!(
            state,
            user,
            app_id,
            "app.visibility.request",
            "App",
            app_id,
            format!("Publication review requested for {:?}", body.visibility)
        );
        return Ok(Json(()));
    }

    Err(ApiError::FORBIDDEN)
}
