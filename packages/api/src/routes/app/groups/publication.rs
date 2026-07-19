use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, TransactionTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    audit_branch, ensure_permission,
    entity::{
        app_group, publication_log, publication_request,
        sea_orm_active_enums::{PublicationRequestStatus, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    publication::{
        PublicationTarget,
        gate::{group_member_assessments, is_public_target, require_group_assessments},
        target::new_request,
    },
    routes::app::{
        connection::deny_connected_app,
        groups::{
            GroupInfo, crud::single_group, group_display_name, notify_group_members_of_visibility,
            parse_visibility, visibility_to_string,
        },
    },
    state::AppState,
};

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ChangeGroupVisibilityRequest {
    /// "PRIVATE" | "PROTOTYPE" | "PUBLIC" | "PUBLIC_REQUEST_ACCESS"
    pub visibility: String,
    /// Optional note for the reviewer, recorded on the publication request.
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangeGroupVisibilityResponse {
    /// True when the change needed review rather than taking effect now.
    pub review_requested: bool,
    /// The suite's visibility after this call.
    pub visibility: String,
    /// Id of the created publication request, when one was needed.
    pub request_id: Option<String>,
    pub group: GroupInfo,
}

/// Loads a suite and proves the caller may steer it: the request must come
/// through the suite's anchor app, and the caller must own that app.
async fn ensure_group_owner(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    group_id: &str,
) -> Result<app_group::Model, ApiError> {
    deny_connected_app(user)?;
    ensure_permission!(user, app_id, state, RolePermissions::Owner);

    let group = app_group::Entity::find_by_id(group_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if group.owner_app_id != app_id {
        return Err(ApiError::FORBIDDEN);
    }
    Ok(group)
}

#[utoipa::path(
    patch,
    path = "/apps/{app_id}/groups/{group_id}/visibility",
    tag = "groups",
    description = "Change a suite's visibility. Moving to a public visibility submits it for review, exactly like publishing an app.",
    params(
        ("app_id" = String, Path, description = "Owner (anchor) application ID"),
        ("group_id" = String, Path, description = "Suite ID")
    ),
    request_body = ChangeGroupVisibilityRequest,
    responses(
        (status = 200, description = "Visibility changed or submitted for review", body = ChangeGroupVisibilityResponse),
        (status = 400, description = "Invalid target or review already pending"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Suite not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/groups/{group_id}/visibility",
    skip(state, user)
)]
pub async fn change_group_visibility(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, group_id)): Path<(String, String)>,
    Json(payload): Json<ChangeGroupVisibilityRequest>,
) -> Result<Json<ChangeGroupVisibilityResponse>, ApiError> {
    let group = ensure_group_owner(&state, &user, &app_id, &group_id).await?;
    let target_visibility = parse_visibility(&payload.visibility)?;

    if target_visibility == Visibility::Offline {
        return Err(ApiError::bad_request(
            "Suites cannot be taken offline; delete the suite instead.".to_string(),
        ));
    }

    let current = group.visibility.clone();
    if current == target_visibility {
        return Ok(Json(ChangeGroupVisibilityResponse {
            review_requested: false,
            visibility: visibility_to_string(&current),
            request_id: None,
            group: single_group(&state, &group_id).await?.0,
        }));
    }

    let now = chrono::Utc::now().naive_utc();
    let currently_public = is_public_target(&current);
    let wants_public = is_public_target(&target_visibility);

    // Same branch table as apps: stepping down, or moving between the two
    // non-public levels, is self-serve. Only *becoming* publicly reachable is
    // reviewed — and once reviewed, switching between the two public levels is
    // free, because that review already covered public exposure.
    let self_serve = !wants_public || currently_public;

    if self_serve {
        let mut active = group.clone().into_active_model();
        active.visibility = Set(target_visibility.clone());
        active.updated_at = Set(now);
        active.update(&state.db).await?;

        notify_group_members_of_visibility(
            &state,
            &group_id,
            &group_display_name(&state, &group_id).await,
            &target_visibility,
        )
        .await;

        audit_branch!(
            state,
            user,
            app_id,
            "app_group.visibility",
            "AppGroup",
            group_id,
            format!("Suite visibility changed to {:?}", target_visibility)
        );

        return Ok(Json(ChangeGroupVisibilityResponse {
            review_requested: false,
            visibility: visibility_to_string(&target_visibility),
            request_id: None,
            group: single_group(&state, &group_id).await?.0,
        }));
    }

    let existing = publication_request::Entity::find()
        .filter(publication_request::Column::GroupId.eq(&group_id))
        .filter(
            publication_request::Column::Status
                .eq(PublicationRequestStatus::Pending)
                .or(publication_request::Column::Status.eq(PublicationRequestStatus::OnHold)),
        )
        .one(&state.db)
        .await?;
    if existing.is_some() {
        return Err(ApiError::bad_request(
            "A review is already pending for this suite.".to_string(),
        ));
    }

    // A suite carries no assessment of its own, so it is gated on the union of
    // its active member apps'.
    require_group_assessments(&state, &group_id).await?;

    let txn = state.db.begin().await?;
    let request_id = create_id();
    new_request(
        request_id.clone(),
        &PublicationTarget::Group(group_id.clone()),
        target_visibility.clone(),
        None,
        now,
    )
    .insert(&txn)
    .await?;

    publication_log::ActiveModel {
        id: Set(create_id()),
        request_id: Set(request_id.clone()),
        author_id: Set(user.sub().ok()),
        message: Set(payload
            .message
            .clone()
            .or_else(|| Some("Request initiated".to_string()))),
        visibility: Set(Some(current.clone())),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&txn)
    .await?;
    txn.commit().await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app_group.visibility.request",
        "AppGroup",
        group_id,
        format!("Publication review requested for {:?}", target_visibility)
    );

    Ok(Json(ChangeGroupVisibilityResponse {
        review_requested: true,
        visibility: visibility_to_string(&current),
        request_id: Some(request_id),
        group: single_group(&state, &group_id).await?.0,
    }))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupPublicationLogItem {
    pub id: String,
    pub author_id: Option<String>,
    pub message: Option<String>,
    pub visibility: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupPublicationRequestItem {
    pub id: String,
    pub group_id: String,
    pub target_visibility: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub logs: Vec<GroupPublicationLogItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MemberReadinessItem {
    pub app_id: String,
    /// Latest EU AI Act assessment status, or null if never started.
    pub ai_act_status: Option<String>,
    /// False when this app currently blocks the suite from being published.
    pub ready: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupPublicationStatus {
    pub current_visibility: String,
    pub requests: Vec<GroupPublicationRequestItem>,
    /// Per-member EU AI Act readiness driving the publish gate. Empty when the
    /// AI Act feature is disabled for this deployment.
    pub member_readiness: Vec<MemberReadinessItem>,
    /// True when a publish request would be accepted right now.
    pub can_request_publication: bool,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/groups/{group_id}/publication",
    tag = "groups",
    description = "Publication history and publish-readiness of a suite, including which member apps still block it.",
    params(
        ("app_id" = String, Path, description = "Owner (anchor) application ID"),
        ("group_id" = String, Path, description = "Suite ID")
    ),
    responses(
        (status = 200, description = "Publication status", body = GroupPublicationStatus),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Suite not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/groups/{group_id}/publication",
    skip(state, user)
)]
pub async fn get_group_publication(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, group_id)): Path<(String, String)>,
) -> Result<Json<GroupPublicationStatus>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let group = app_group::Entity::find_by_id(&group_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if group.owner_app_id != app_id {
        return Err(ApiError::FORBIDDEN);
    }

    let requests = publication_request::Entity::find()
        .filter(publication_request::Column::GroupId.eq(&group_id))
        .order_by_desc(publication_request::Column::CreatedAt)
        .all(&state.db)
        .await?;

    let request_ids: Vec<String> = requests.iter().map(|r| r.id.clone()).collect();
    let mut logs_by_request: std::collections::HashMap<String, Vec<GroupPublicationLogItem>> =
        std::collections::HashMap::new();
    if !request_ids.is_empty() {
        for log in publication_log::Entity::find()
            .filter(publication_log::Column::RequestId.is_in(request_ids))
            .order_by_asc(publication_log::Column::CreatedAt)
            .all(&state.db)
            .await?
        {
            logs_by_request
                .entry(log.request_id.clone())
                .or_default()
                .push(GroupPublicationLogItem {
                    id: log.id,
                    author_id: log.author_id,
                    message: log.message,
                    visibility: log.visibility.as_ref().map(visibility_to_string),
                    created_at: log.created_at.to_string(),
                });
        }
    }

    let has_open_request = requests.iter().any(|r| {
        matches!(
            r.status,
            PublicationRequestStatus::Pending | PublicationRequestStatus::OnHold
        )
    });

    let member_readiness: Vec<MemberReadinessItem> = if state.platform_config.features.ai_act {
        group_member_assessments(&state, &group_id)
            .await?
            .into_iter()
            .map(|m| MemberReadinessItem {
                ready: m.is_clear(),
                ai_act_status: m.status.map(|s| format!("{:?}", s).to_uppercase()),
                app_id: m.app_id,
            })
            .collect()
    } else {
        vec![]
    };

    let can_request_publication = !has_open_request
        && !is_public_target(&group.visibility)
        && require_group_assessments(&state, &group_id).await.is_ok();

    Ok(Json(GroupPublicationStatus {
        current_visibility: visibility_to_string(&group.visibility),
        requests: requests
            .into_iter()
            .map(|r| GroupPublicationRequestItem {
                target_visibility: visibility_to_string(&r.target_visibility),
                status: format!("{:?}", r.status).to_uppercase(),
                created_at: r.created_at.to_string(),
                updated_at: r.updated_at.to_string(),
                logs: logs_by_request.remove(&r.id).unwrap_or_default(),
                group_id: group_id.clone(),
                id: r.id,
            })
            .collect(),
        member_readiness,
        can_request_publication,
    }))
}
