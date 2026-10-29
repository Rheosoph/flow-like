use std::collections::HashMap;

use crate::{
    entity::user,
    error::ApiError,
    execution::subject::account_user_id,
    middleware::jwt::{AppUser, ExecutorUse},
    routes::profile::create_default::create_default_profile,
    routes::user::identity::{derive_display_name, derive_public_handle, sanitize_display_name},
    routes::user::lookup::UserLookupResponse,
    routes::user::sign_avatar,
    state::AppState,
    user_management::UserManagement,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
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

/// The account a human token belongs to. `/user/info` switches to it once
/// executors read their user through [`executor_user_info`].
#[allow(dead_code)]
pub(crate) fn account_subject(user: &AppUser) -> Result<String, ApiError> {
    if matches!(user, AppUser::Executor(_)) {
        return Err(ApiError::forbidden(
            "Runs read the user they act as from /execution/apps/{app_id}/user",
        ));
    }
    user.sub()
}

/// The account a run acts as. Only the run's own executor token may ask, and
/// runs for an API key or a connected app have no account to show.
fn run_subject<'a>(user: &'a AppUser, app_id: &'a str) -> Result<&'a str, ApiError> {
    let AppUser::Executor(executor) = user else {
        return Err(ApiError::forbidden(
            "This route only accepts executor tokens",
        ));
    };
    if executor.app_id != app_id {
        return Err(ApiError::forbidden(
            "Executor token is bound to a different app",
        ));
    }
    let chained = executor
        .app_chain
        .as_ref()
        .is_some_and(|chain| !chain.is_empty());
    if chained || executor.technical_user_id.is_some() {
        return Err(ApiError::not_found(
            "This run acts for an API key or a connected app, not a user account",
        ));
    }
    account_user_id(&executor.sub)
        .ok_or_else(|| ApiError::not_found("This run does not act as a user account"))
}

/// Profile of the user a run acts as, limited to what this hub shows in user lookups.
#[utoipa::path(
    get,
    path = "/execution/apps/{app_id}/user",
    tag = "execution",
    description = "Profile of the user a run acts as, limited to what this hub shows in user lookups.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "The run's user, as user lookups show them", body = UserLookupResponse),
        (status = 403, description = "Not an executor token, or the run belongs to another app or lost access to it"),
        (status = 404, description = "The run acts for an API key, a connected app or a visitor without an account")
    ),
    security(("executor_jwt" = []))
)]
#[tracing::instrument(name = "GET /execution/apps/{app_id}/user", skip(state, user))]
pub async fn executor_user_info(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<UserLookupResponse>, ApiError> {
    let sub = run_subject(&user, &app_id)?;
    user.execution_app_permission(&app_id, &state, ExecutorUse::Read)
        .await?;
    let account = user::Entity::find_by_id(sub)
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("The account this run acts as no longer exists"))?;
    let lookup = state.platform_config.lookup.clone();
    Ok(Json(
        UserLookupResponse::parse(account, lookup, &state).await,
    ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleware::jwt::{ApiKey, ConnectedAppUser, ExecutorUser, OpenIDUser, PATUser};
    use axum::http::StatusCode;

    fn executor(app_id: &str, sub: &str) -> ExecutorUser {
        ExecutorUser {
            sub: sub.into(),
            app_id: app_id.into(),
            run_id: "run-1".into(),
            technical_user_id: None,
            app_chain: None,
            correlation: None,
            shadow: false,
            hosted_frontend: false,
            event_id: None,
            custom_bits_owner: None,
            unattended: false,
        }
    }

    fn people() -> [AppUser; 2] {
        [
            AppUser::OpenID(OpenIDUser {
                sub: "user-1".into(),
                access_token: "token".into(),
            }),
            AppUser::PAT(PATUser {
                pat: "pat".into(),
                sub: "user-1".into(),
            }),
        ]
    }

    fn machines() -> [AppUser; 2] {
        [
            AppUser::APIKey(ApiKey {
                key_id: "key-1".into(),
                api_key: "secret".into(),
                app_id: "app-1".into(),
                creator_user_id: Some("user-1".into()),
            }),
            AppUser::ConnectedApp(ConnectedAppUser {
                sub: Some("user-1".into()),
                origin_app_id: "app-0".into(),
                target_app_id: "app-1".into(),
                app_chain: vec!["app-0".into()],
                technical_user_id: None,
                run_id: Some("run-0".into()),
                correlation: None,
            }),
        ]
    }

    fn status<T>(result: Result<T, ApiError>) -> Option<StatusCode> {
        result.err().map(|error| error.status())
    }

    #[test]
    fn run_subject_accepts_only_executor_tokens_of_the_path_app() {
        let run = AppUser::Executor(executor("app-1", "user-1"));
        assert_eq!(run_subject(&run, "app-1").unwrap(), "user-1");
        assert_eq!(
            status(run_subject(&run, "app-2")),
            Some(StatusCode::FORBIDDEN)
        );

        for account_run in [
            ExecutorUser {
                shadow: true,
                ..executor("app-1", "user-1")
            },
            ExecutorUser {
                hosted_frontend: true,
                ..executor("app-1", "user-1")
            },
            ExecutorUser {
                app_chain: Some(Vec::new()),
                ..executor("app-1", "user-1")
            },
        ] {
            let run = AppUser::Executor(account_run);
            assert_eq!(run_subject(&run, "app-1").ok(), Some("user-1"), "{run:?}");
        }

        for principal in people()
            .into_iter()
            .chain(machines())
            .chain([AppUser::Unauthorized])
        {
            assert_eq!(
                status(run_subject(&principal, "app-1")),
                Some(StatusCode::FORBIDDEN),
                "{principal:?}"
            );
        }
    }

    #[test]
    fn run_subject_finds_no_account_for_keys_connections_or_placeholders() {
        for no_account in [
            ExecutorUser {
                app_chain: Some(vec!["app-0".into()]),
                ..executor("app-1", "user-1")
            },
            ExecutorUser {
                technical_user_id: Some("key-1".into()),
                ..executor("app-1", "user-1")
            },
            executor("app-1", ""),
            executor("app-1", "local"),
            executor("app-1", "sink:sink-1"),
            executor("app-1", "inbound:event-1"),
            executor("app-1", "regression:suite-1"),
            executor("app-1", "frontend:event-1:anonymous-1"),
        ] {
            let run = AppUser::Executor(no_account);
            assert_eq!(
                status(run_subject(&run, "app-1")),
                Some(StatusCode::NOT_FOUND),
                "{run:?}"
            );
        }
    }

    #[test]
    fn account_subject_accepts_only_people() {
        for person in people() {
            assert_eq!(account_subject(&person).unwrap(), "user-1");
        }
        for principal in machines()
            .into_iter()
            .chain([AppUser::Executor(executor("app-1", "user-1"))])
        {
            assert_eq!(
                status(account_subject(&principal)),
                Some(StatusCode::FORBIDDEN),
                "{principal:?}"
            );
        }
        assert_eq!(
            status(account_subject(&AppUser::Unauthorized)),
            Some(StatusCode::UNAUTHORIZED)
        );
    }

    #[test]
    fn the_run_route_reads_after_the_subject_and_app_checks() {
        let source = include_str!("info.rs");
        let handler = source
            .split_once("\npub async fn executor_user_info(")
            .expect("executor_user_info exists")
            .1;
        let handler = handler
            .split_once("\n}\n")
            .map_or(handler, |(body, _)| body);
        let position = |needle: &str| {
            handler
                .find(needle)
                .unwrap_or_else(|| panic!("executor_user_info calls {needle}"))
        };
        assert!(position("run_subject(&user, &app_id)?") < position(".execution_app_permission("));
        assert!(
            position(".execution_app_permission(&app_id, &state, ExecutorUse::Read)")
                < position("find_by_id(sub)")
        );
        assert!(position("find_by_id(sub)") < position("UserLookupResponse::parse("));
    }
}
