use crate::{
    audit,
    entity::pat,
    error::ApiError,
    middleware::jwt::{AppUser, fresh_pat},
    permission::pat_permission::PatPermission,
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use flow_like_types::{
    base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD},
    create_id,
    rand::{TryRngCore, rngs::OsRng},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatOut {
    pub pat: String,
    pub permission: i64,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PatInput {
    pub name: String,
    /// Optional expiration timestamp (in seconds since epoch)
    pub valid_until: Option<i64>,
    pub permissions: Option<i64>,
}

#[utoipa::path(
    put,
    path = "/user/pat",
    tag = "user",
    request_body = PatInput,
    responses(
        (status = 200, description = "Personal access token created", body = PatOut),
        (status = 401, description = "Unauthorized"),
        (status = 400, description = "Invalid input")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "PUT /user/pat", skip(state, user, input))]
pub async fn create_pat(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(input): Json<PatInput>,
) -> Result<Json<PatOut>, ApiError> {
    let sub = user.sub()?;

    let permissions = input.permissions.unwrap_or(PatPermission::ReadOnly.bits());
    validate_permissions(permissions)?;

    let mut valid_until = match input.valid_until {
        Some(ts) => Some(
            chrono::DateTime::from_timestamp(ts, 0)
                .ok_or_else(|| ApiError::bad_request("Invalid valid_until timestamp"))?
                .fixed_offset(),
        ),
        None => None,
    };
    if valid_until.is_some_and(|expiry| expiry <= chrono::Utc::now().fixed_offset()) {
        return Err(ApiError::bad_request(
            "Token expiration must be in the future",
        ));
    }
    if let AppUser::PAT(parent) = &user {
        let parent = fresh_pat(parent, &state).await?;
        valid_until = validate_delegation(
            parent.permissions,
            parent.valid_until,
            permissions,
            valid_until,
        )?;
    }

    let mut secret_bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut secret_bytes)
        .map_err(|e| ApiError::internal(format!("Failed to generate random bytes: {}", e)))?;
    let secret_b64 = URL_SAFE_NO_PAD.encode(secret_bytes);

    let mut hasher = blake3::Hasher::new();
    hasher.update(secret_b64.as_bytes());
    let secret_hash = hasher.finalize().to_hex().to_string().to_lowercase();

    let pat = pat::ActiveModel {
        id: Set(create_id()),
        key: Set(secret_hash),
        name: Set(input.name),
        user_id: Set(sub.to_string()),
        valid_until: Set(valid_until),
        permissions: Set(permissions),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
    };

    let pat = pat.insert(&state.db).await?;
    audit!(
        state,
        user,
        "pat.create",
        "PersonalAccessToken",
        pat.id,
        serde_json::json!({
            "permissions": pat.permissions,
            "valid_until": pat.valid_until,
        })
    );
    let pat_out = PatOut {
        pat: format!("pat_{}.{}", pat.id, secret_b64),
        permission: pat.permissions,
    };
    Ok(Json(pat_out))
}

fn validate_permissions(bits: i64) -> Result<PatPermission, ApiError> {
    PatPermission::from_bits(bits).ok_or_else(|| {
        ApiError::bad_request("Permissions must be 1 (Read Only), 2 (Read & Write), or 4 (Admin)")
    })
}

fn validate_delegation(
    parent_bits: i64,
    parent_expiry: Option<sea_orm::prelude::DateTimeWithTimeZone>,
    child_bits: i64,
    child_expiry: Option<sea_orm::prelude::DateTimeWithTimeZone>,
) -> Result<Option<sea_orm::prelude::DateTimeWithTimeZone>, ApiError> {
    let parent = validate_permissions(parent_bits)?;
    let child = validate_permissions(child_bits)?;
    if child > parent {
        return Err(ApiError::forbidden(
            "Cannot grant permissions beyond the current token",
        ));
    }
    let child_expiry = child_expiry.or(parent_expiry);
    if parent_expiry.is_some_and(|expiry| child_expiry.is_none_or(|child| child > expiry)) {
        return Err(ApiError::forbidden(
            "A delegated token cannot outlive the current token",
        ));
    }
    Ok(child_expiry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_creation_rejects_unknown_masks_and_broader_delegation() {
        for bits in [0, -1, 3, 5, 6, 7, 8, 16, 32, 64, 65] {
            assert!(validate_permissions(bits).is_err());
        }
        assert!(
            validate_delegation(
                PatPermission::ReadOnly.bits(),
                None,
                PatPermission::ReadWrite.bits(),
                None
            )
            .is_err()
        );
        assert!(
            validate_delegation(
                PatPermission::ReadWrite.bits(),
                None,
                PatPermission::Admin.bits(),
                None
            )
            .is_err()
        );
        assert!(
            validate_delegation(
                PatPermission::Admin.bits(),
                None,
                PatPermission::ReadWrite.bits(),
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn delegated_tokens_cannot_outlive_the_parent() {
        let expiry = chrono::Utc::now().fixed_offset();
        assert_eq!(
            validate_delegation(4, Some(expiry), 2, None).unwrap(),
            Some(expiry)
        );
        assert!(
            validate_delegation(
                4,
                Some(expiry),
                2,
                Some(expiry + chrono::Duration::seconds(1))
            )
            .is_err()
        );
        assert_eq!(
            validate_delegation(4, Some(expiry), 2, Some(expiry)).unwrap(),
            Some(expiry)
        );
        assert_eq!(validate_delegation(4, None, 2, None).unwrap(), None);
    }
}
