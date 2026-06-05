use std::collections::HashMap;

use crate::{
    entity::user, error::ApiError, middleware::jwt::AppUser,
    routes::profile::create_default::create_default_profile, routes::user::sign_avatar,
    state::AppState, user_management::UserManagement,
};
use axum::{Extension, Json, extract::State};
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveModelTrait, ConnectionTrait, DbBackend, EntityTrait, Statement};

/// Sometimes when the user still has an old jwt, the user info is not updated correctly.
/// In these cases, we want to update the value correctly.
#[tracing::instrument(
    name = "Should update user attribute",
    skip(state, sub, attribute, value)
)]
async fn should_update(
    state: &AppState,
    sub: &str,
    username: &Option<String>,
    attribute: &str,
    value: &Option<String>,
) -> bool {
    let user_manager = UserManagement::new(state).await;
    let actual_value = user_manager.get_attribute(sub, username, attribute).await;

    let mut should_update = true;

    if let Ok(Some(actual_value)) = actual_value
        && Some(actual_value) == *value
    {
        should_update = false;
    }
    should_update
}

/// Older user rows can predate defaults on required columns. SeaORM decodes the
/// generated `user::Model` strictly, so repair those rows before loading them.
async fn repair_legacy_user_defaults(state: &AppState, sub: &str) -> Result<(), sea_orm::DbErr> {
    let backend = state.db.get_database_backend();
    let sql = match backend {
        DbBackend::Postgres => {
            r#"UPDATE "User"
SET
    "permission" = COALESCE("permission", 0),
    "tutorialCompleted" = COALESCE("tutorialCompleted", FALSE),
    "status" = COALESCE("status", 'ACTIVE'::"UserStatus"),
    "tier" = COALESCE("tier", 'FREE'::"UserTier"),
    "totalSize" = COALESCE("totalSize", 0),
    "totalLLMPrice" = COALESCE("totalLLMPrice", 0),
    "totalEmbeddingPrice" = COALESCE("totalEmbeddingPrice", 0),
    "createdAt" = COALESCE("createdAt", NOW()),
    "updatedAt" = COALESCE("updatedAt", NOW())
WHERE "id" = $1
  AND (
      "permission" IS NULL
      OR "tutorialCompleted" IS NULL
      OR "status" IS NULL
      OR "tier" IS NULL
      OR "totalSize" IS NULL
      OR "totalLLMPrice" IS NULL
      OR "totalEmbeddingPrice" IS NULL
      OR "createdAt" IS NULL
      OR "updatedAt" IS NULL
  )"#
        }
        _ => {
            r#"UPDATE "User"
SET
    "permission" = COALESCE("permission", 0),
    "tutorialCompleted" = COALESCE("tutorialCompleted", FALSE),
    "status" = COALESCE("status", 'ACTIVE'),
    "tier" = COALESCE("tier", 'FREE'),
    "totalSize" = COALESCE("totalSize", 0),
    "totalLLMPrice" = COALESCE("totalLLMPrice", 0),
    "totalEmbeddingPrice" = COALESCE("totalEmbeddingPrice", 0),
    "createdAt" = COALESCE("createdAt", CURRENT_TIMESTAMP),
    "updatedAt" = COALESCE("updatedAt", CURRENT_TIMESTAMP)
WHERE "id" = ?
  AND (
      "permission" IS NULL
      OR "tutorialCompleted" IS NULL
      OR "status" IS NULL
      OR "tier" IS NULL
      OR "totalSize" IS NULL
      OR "totalLLMPrice" IS NULL
      OR "totalEmbeddingPrice" IS NULL
      OR "createdAt" IS NULL
      OR "updatedAt" IS NULL
  )"#
        }
    };

    state
        .db
        .execute(Statement::from_sql_and_values(
            backend,
            sql,
            vec![sub.into()],
        ))
        .await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/user/info",
    tag = "user",
    responses(
        (status = 200, description = "Returns the current user's information"),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /user/info", skip(state, user))]
pub async fn user_info(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<user::Model>, ApiError> {
    let sub = user.sub()?;
    let user_info = user.user_info(&state).await?;
    let email = user_info.email.clone();
    let username = user_info.username.clone();
    let preferred_username = user_info.preferred_username.clone();
    repair_legacy_user_defaults(&state, &sub).await?;
    let user_info = user::Entity::find_by_id(&sub).one(&state.db).await?;
    if let Some(mut user_info) = user_info {
        let mut updated_user: Option<user::ActiveModel> = None;
        if let Some(email) = &email
            && user_info.email != Some(email.clone())
            && should_update(&state, &sub, &username, "email", &user_info.email).await
        {
            let mut tmp_updated_user: user::ActiveModel = user_info.clone().into();
            tmp_updated_user.email = sea_orm::ActiveValue::Set(Some(email.clone()));
            updated_user = Some(tmp_updated_user);
        }

        if let Some(username) = &username
            && user_info.username != Some(username.clone())
        {
            let mut tmp_updated_user: user::ActiveModel =
                updated_user.unwrap_or(user_info.clone().into());
            tmp_updated_user.username = sea_orm::ActiveValue::Set(Some(username.clone()));
            updated_user = Some(tmp_updated_user);
        }

        if user_info.tracking_id.is_none() {
            let tracking_id = create_id();
            let mut tmp_updated_user: user::ActiveModel =
                updated_user.unwrap_or(user_info.clone().into());
            tmp_updated_user.tracking_id = sea_orm::ActiveValue::Set(Some(tracking_id));
            updated_user = Some(tmp_updated_user);
        }

        if let Some(preferred_username) = &preferred_username
            && user_info.preferred_username != Some(preferred_username.clone())
            && should_update(
                &state,
                &sub,
                &username,
                "preferred_username",
                &user_info.preferred_username,
            )
            .await
        {
            let mut tmp_updated_user: user::ActiveModel =
                updated_user.unwrap_or(user_info.clone().into());
            tmp_updated_user.preferred_username =
                sea_orm::ActiveValue::Set(Some(preferred_username.clone()));
            updated_user = Some(tmp_updated_user);
        }

        if let Some(mut updated_user) = updated_user {
            updated_user.updated_at = sea_orm::ActiveValue::Set(chrono::Utc::now().naive_utc());
            let new_user = updated_user.update(&state.db).await?;
            user_info = new_user;
        }

        user_info = ensure_stripe_user(&state, user_info, email.clone()).await?;

        if let Some(avatar) = &user_info.avatar {
            let signed_avatar_url = sign_avatar(&user_info.id, avatar, &state).await?;
            user_info.avatar = Some(signed_avatar_url);
        }

        return Ok(Json(user_info));
    }

    let user = user::ActiveModel {
        id: sea_orm::ActiveValue::Set(sub.clone()),
        tracking_id: sea_orm::ActiveValue::Set(Some(create_id())),
        email: sea_orm::ActiveValue::Set(email.clone()),
        stripe_id: sea_orm::ActiveValue::Set(None),
        username: sea_orm::ActiveValue::Set(username),
        preferred_username: sea_orm::ActiveValue::Set(preferred_username),
        created_at: sea_orm::ActiveValue::Set(chrono::Utc::now().naive_utc()),
        updated_at: sea_orm::ActiveValue::Set(chrono::Utc::now().naive_utc()),
        ..Default::default()
    };

    let mut new_user = user::Entity::insert(user)
        .exec_with_returning(&state.db)
        .await?;

    // Create default profile for new user
    if let Err(e) = create_default_profile(&state, &sub).await {
        tracing::warn!(
            "Failed to create default profile for new user {}: {}",
            sub,
            e
        );
        // Don't fail user creation if profile creation fails
    }

    new_user = ensure_stripe_user(&state, new_user, email.clone()).await?;

    Ok(Json(new_user))
}

async fn ensure_stripe_user(
    state: &AppState,
    user_info: user::Model,
    email: Option<String>,
) -> Result<user::Model, ApiError> {
    if user_info.stripe_id.is_some() || !state.platform_config.features.premium {
        return Ok(user_info);
    }

    let stripe_customer = generate_stripe_user(state, &user_info.id, email).await?;
    let mut updated_user: user::ActiveModel = user_info.into();
    updated_user.stripe_id = sea_orm::ActiveValue::Set(Some(stripe_customer.id.to_string()));
    updated_user.updated_at = sea_orm::ActiveValue::Set(chrono::Utc::now().naive_utc());

    Ok(updated_user.update(&state.db).await?)
}

async fn generate_stripe_user(
    state: &AppState,
    sub: &str,
    email: Option<String>,
) -> flow_like_types::Result<stripe::Customer> {
    let stripe_client = state
        .stripe_client
        .as_ref()
        .ok_or(anyhow!("Premium Feature disabled"))?;
    let idempotency_key = format!("flowlike:user:{}", blake3::hash(sub.as_bytes()).to_hex());
    let stripe_client = stripe_client
        .clone()
        .with_strategy(stripe::RequestStrategy::Idempotent(idempotency_key));
    let customer = stripe::Customer::create(
        &stripe_client,
        stripe::CreateCustomer {
            metadata: Some(HashMap::from([
                ("sub".to_string(), sub.to_string()),
                ("platform".to_string(), "FlowLike".to_string()),
            ])),
            email: email.as_deref(),
            ..Default::default()
        },
    )
    .await?;

    Ok(customer)
}
