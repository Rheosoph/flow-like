use std::collections::HashMap;

use crate::{
    entity::user,
    error::ApiError,
    middleware::jwt::AppUser,
    routes::profile::create_default::create_default_profile,
    routes::user::identity::{derive_display_name, derive_public_handle, sanitize_display_name},
    routes::user::sign_avatar,
    state::AppState,
    user_management::UserManagement,
};
use axum::{Extension, Json, extract::State};
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveModelTrait, EntityTrait};

/// Sometimes when the user still has an old jwt, the user info is not updated correctly.
/// In these cases, we want to update the value correctly.
#[tracing::instrument(name = "Should update user attribute", skip_all)]
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
    let sub = user.executor_scoped_sub()?;
    let identity_info = match &user {
        AppUser::OpenID(_) => Some(user.user_info(&state).await?),
        _ => None,
    };
    let email = identity_info.as_ref().and_then(|info| info.email.clone());
    let username = identity_info
        .as_ref()
        .and_then(|info| info.username.clone());
    // Only a handle the provider actually gave us — never its pool-internal one.
    let preferred_username = identity_info.as_ref().and_then(derive_public_handle);
    let display_name = identity_info.as_ref().and_then(derive_display_name);
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

        // Backfills users provisioned before display names were derived, and users
        // created by `ensure_user_exists`, which has no claims to work from. A name
        // the user set themselves via `PUT /user/info` is never overwritten.
        if let Some(display_name) = &display_name
            && user_info
                .name
                .as_deref()
                .and_then(sanitize_display_name)
                .is_none()
        {
            let mut tmp_updated_user: user::ActiveModel =
                updated_user.unwrap_or(user_info.clone().into());
            tmp_updated_user.name = sea_orm::ActiveValue::Set(Some(display_name.clone()));
            updated_user = Some(tmp_updated_user);
        }

        if identity_info.is_some() && user_info.tracking_id.is_none() {
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
            updated_user.updated_at = sea_orm::ActiveValue::Set(chrono::Utc::now().fixed_offset());
            user_info = persist_identity_sync(&state, updated_user, user_info).await?;
        }

        if identity_info.is_some() {
            user_info = ensure_stripe_user(&state, user_info, email.clone()).await?;
        }

        if let Some(avatar) = &user_info.avatar {
            let signed_avatar_url = sign_avatar(&user_info.id, avatar, &state).await?;
            user_info.avatar = Some(signed_avatar_url);
        }

        return Ok(Json(user_info));
    }

    if identity_info.is_none() {
        return Err(ApiError::NOT_FOUND);
    }

    let user = user::ActiveModel {
        id: sea_orm::ActiveValue::Set(sub.clone()),
        tracking_id: sea_orm::ActiveValue::Set(Some(create_id())),
        email: sea_orm::ActiveValue::Set(email.clone()),
        stripe_id: sea_orm::ActiveValue::Set(None),
        username: sea_orm::ActiveValue::Set(username),
        preferred_username: sea_orm::ActiveValue::Set(preferred_username),
        name: sea_orm::ActiveValue::Set(display_name),
        created_at: sea_orm::ActiveValue::Set(chrono::Utc::now().fixed_offset()),
        updated_at: sea_orm::ActiveValue::Set(chrono::Utc::now().fixed_offset()),
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

/// `username` and `preferredUsername` are unique, so an identity sync can collide
/// with another row. `/user/info` gates app startup, so a collision must degrade to
/// "keep the row we have" rather than fail the request.
#[tracing::instrument(name = "Persist identity sync", skip_all)]
async fn persist_identity_sync(
    state: &AppState,
    updated_user: user::ActiveModel,
    current: user::Model,
) -> Result<user::Model, ApiError> {
    let err = match updated_user.clone().update(&state.db).await {
        Ok(user) => return Ok(user),
        Err(err) => err,
    };

    tracing::warn!(
        user_id = %current.id,
        "Identity sync failed, retrying without unique handle columns: {:?}",
        err
    );

    let mut retry = updated_user;
    retry.username = sea_orm::ActiveValue::Unchanged(current.username.clone());
    retry.preferred_username = sea_orm::ActiveValue::Unchanged(current.preferred_username.clone());

    match retry.update(&state.db).await {
        Ok(user) => Ok(user),
        Err(err) => {
            tracing::error!(
                user_id = %current.id,
                "Identity sync failed permanently, serving unsynced user: {:?}",
                err
            );
            Ok(current)
        }
    }
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
    updated_user.updated_at = sea_orm::ActiveValue::Set(chrono::Utc::now().fixed_offset());

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
