use crate::{
    audit_branch,
    entity::{app, join_queue, membership, sea_orm_active_enums::Visibility},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct RequestJoinParams {
    pub comment: Option<String>,
}

enum JoinOutcome {
    AutoJoined,
    RequestUpdated,
    RequestSubmitted,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/team/queue",
    tag = "team",
    description = "Request to join an app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = RequestJoinParams,
    responses(
        (status = 200, description = "Join request submitted", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/team/queue", skip(state, user, params))]
pub async fn request_join(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(params): Json<RequestJoinParams>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;
    let new_row_id = create_id();

    let outcome = state
        .transaction(|txn| {
            let app_id = app_id.clone();
            let sub = sub.clone();
            let comment = params.comment.clone();
            let new_row_id = new_row_id.clone();
            Box::pin(async move {
                let membership = membership::Entity::find()
                    .filter(membership::Column::AppId.eq(app_id.clone()))
                    .filter(membership::Column::UserId.eq(sub.clone()))
                    .one(txn)
                    .await?;

                if membership.is_some() {
                    tracing::warn!(
                        "User {} is trying to join app {} but is already a member",
                        sub,
                        app_id
                    );
                    return Err(ApiError::FORBIDDEN);
                }

                let app = app::Entity::find_by_id(app_id.clone())
                    .one(txn)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;

                let default_role_id = app.default_role_id.clone().ok_or(ApiError::NOT_FOUND)?;

                if app.visibility == Visibility::Public && app.price <= 0 {
                    membership::ActiveModel {
                        id: Set(new_row_id),
                        user_id: Set(sub),
                        app_id: Set(app_id),
                        role_id: Set(default_role_id),
                        created_at: Set(chrono::Utc::now().fixed_offset()),
                        updated_at: Set(chrono::Utc::now().fixed_offset()),
                        joined_via: Set(Some("request_join".to_string())),
                    }
                    .insert(txn)
                    .await?;
                    return Ok(JoinOutcome::AutoJoined);
                }

                if !matches!(
                    app.visibility,
                    Visibility::PublicRequestAccess | Visibility::Public
                ) {
                    tracing::warn!(
                        "User {} is trying to join app {} but the app is not public",
                        sub,
                        app_id
                    );
                    return Err(ApiError::FORBIDDEN);
                }

                let existing_request = join_queue::Entity::find()
                    .filter(join_queue::Column::AppId.eq(app_id.clone()))
                    .filter(join_queue::Column::UserId.eq(sub.clone()))
                    .one(txn)
                    .await?;

                if let Some(existing_request) = existing_request {
                    tracing::warn!(
                        "User {} is trying to join app {} but already has a pending request",
                        sub,
                        app_id
                    );

                    let mut existing_request: join_queue::ActiveModel = existing_request.into();
                    existing_request.comment = Set(comment);
                    existing_request.updated_at = Set(chrono::Utc::now().fixed_offset());
                    existing_request.update(txn).await?;
                    return Ok(JoinOutcome::RequestUpdated);
                }

                join_queue::ActiveModel {
                    id: Set(new_row_id),
                    user_id: Set(sub),
                    app_id: Set(app_id),
                    comment: Set(comment),
                    created_at: Set(chrono::Utc::now().fixed_offset()),
                    updated_at: Set(chrono::Utc::now().fixed_offset()),
                }
                .insert(txn)
                .await?;
                Ok::<_, ApiError>(JoinOutcome::RequestSubmitted)
            })
        })
        .await?;

    let (action, resource_type, summary) = match outcome {
        JoinOutcome::AutoJoined => ("membership.join", "membership", "Auto-joined public app"),
        JoinOutcome::RequestUpdated => ("membership.request", "join_queue", "Join request updated"),
        JoinOutcome::RequestSubmitted => {
            ("membership.request", "join_queue", "Join request submitted")
        }
    };
    audit_branch!(state, user, app_id, action, resource_type, sub, summary);
    Ok(Json(()))
}
