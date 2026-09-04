use crate::{
    audit_branch, ensure_permission,
    entity::{membership, role, technical_user},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD},
    create_id,
    rand::{TryRngCore, rngs::OsRng},
};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ApiKeyInput {
    pub name: String,
    pub description: Option<String>,
    pub role_id: Option<String>,
    pub valid_until: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ApiKeyOut {
    pub id: String,
    pub api_key: String,
    pub name: String,
    pub role_name: Option<String>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/api",
    tag = "api-keys",
    description = "Create an API key for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = ApiKeyInput,
    responses(
        (status = 200, description = "API key created", body = ApiKeyOut),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/api", skip(state, user, input))]
pub async fn create_api_key(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(input): Json<ApiKeyInput>,
) -> Result<Json<ApiKeyOut>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    let creator_user_id = permission.effective_user_id()?;
    let creator_membership = membership::Entity::find()
        .filter(
            membership::Column::UserId
                .eq(&creator_user_id)
                .and(membership::Column::AppId.eq(&app_id)),
        )
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::forbidden("Creator is not a member of this app"))?;

    // Validate role_id if provided
    let role_name = if let Some(role_id) = &input.role_id {
        let role = role::Entity::find_by_id(role_id.clone())
            .filter(role::Column::AppId.eq(&app_id))
            .one(&state.db)
            .await?
            .ok_or_else(|| ApiError::bad_request("Role not found"))?;

        // Prevent assigning Owner role to technical users
        let role_permissions =
            RolePermissions::from_bits(role.permissions).ok_or(ApiError::FORBIDDEN)?;
        if role_permissions.contains(RolePermissions::Owner) {
            return Err(ApiError::bad_request(
                "Cannot assign Owner role to technical users",
            ));
        }

        Some(role.name)
    } else {
        None
    };

    let valid_until = match input.valid_until {
        Some(ts) => Some(
            chrono::DateTime::from_timestamp(ts, 0)
                .ok_or_else(|| ApiError::bad_request("Invalid valid_until timestamp"))?
                .fixed_offset(),
        ),
        None => None,
    };

    // Generate secure random key
    let mut secret_bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut secret_bytes)
        .map_err(|e| ApiError::internal(format!("Failed to generate random bytes: {}", e)))?;
    let secret_b64 = URL_SAFE_NO_PAD.encode(secret_bytes);

    // Hash the key for storage
    let mut hasher = blake3::Hasher::new();
    hasher.update(secret_b64.as_bytes());
    let secret_hash = hasher.finalize().to_hex().to_string().to_lowercase();

    let id = create_id();

    let technical_user = technical_user::ActiveModel {
        id: Set(id.clone()),
        name: Set(input.name.clone()),
        description: Set(input.description),
        key: Set(secret_hash),
        role_id: Set(input.role_id),
        app_id: Set(app_id.clone()),
        creator_user_id: Set(Some(creator_user_id)),
        creator_membership_id: Set(Some(creator_membership.id)),
        valid_until: Set(valid_until),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
    };

    technical_user.insert(&state.db).await?;

    // Format: flk_{app_id}.{id}.{secret}
    let api_key = format!("flk_{}.{}.{}", app_id, id, secret_b64);

    audit_branch!(
        state,
        user,
        app_id,
        "apikey.create",
        "ApiKey",
        id,
        format!("API key '{}' created", input.name)
    );
    Ok(Json(ApiKeyOut {
        id,
        api_key,
        name: input.name,
        role_name,
    }))
}
