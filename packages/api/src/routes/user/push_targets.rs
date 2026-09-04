use crate::{
    entity::{
        push_notification_target,
        sea_orm_active_enums::{PushNotificationTargetPlatform, PushNotificationTargetProvider},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    push_notifications::{configured_provider, prepare_provider_target_registration},
    routes::app::events::db::decrypt_token,
    routes::user::ensure_user_exists,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
    sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const USER_DISABLED_PUSH_REASON: &str = "User disabled push notifications";

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PushTargetPlatformDto {
    Ios,
    Android,
    Desktop,
}

#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct RegisterPushTargetRequest {
    pub device_id: String,
    pub platform: PushTargetPlatformDto,
    pub token: String,
    pub device_name: Option<String>,
    pub channel_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RegisterPushTargetResponse {
    pub id: String,
    pub provider: String,
    pub success: bool,
    pub push_enabled: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UnregisterPushTargetResponse {
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PushTargetStatusResponse {
    pub device_id: String,
    pub provider: Option<String>,
    pub registered: bool,
    pub push_enabled: bool,
    pub platform: Option<String>,
    pub device_name: Option<String>,
    pub channel_id: Option<String>,
    pub failure_count: Option<i32>,
    pub last_registered_at: Option<chrono::DateTime<chrono::Utc>>,
    pub last_seen_at: Option<chrono::DateTime<chrono::Utc>>,
    pub invalidated_at: Option<chrono::DateTime<chrono::Utc>>,
    pub invalidation_reason: Option<String>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdatePushTargetRequest {
    pub push_enabled: bool,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UnregisterPushTargetQuery {
    pub reason: Option<String>,
}

#[utoipa::path(
    post,
    path = "/user/push-targets/register",
    tag = "user",
    request_body = RegisterPushTargetRequest,
    responses(
        (status = 200, description = "Push target registered", body = RegisterPushTargetResponse),
        (status = 400, description = "Push notifications are not enabled or payload is invalid"),
        (status = 401, description = "Unauthorized")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /user/push-targets/register", skip(state, user, body))]
pub async fn register_push_target(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<RegisterPushTargetRequest>,
) -> Result<Json<RegisterPushTargetResponse>, ApiError> {
    let sub = user.sub()?;
    ensure_user_exists(&state, &sub).await?;

    let config = &state.platform_config.push_notifications;
    if !config.enabled {
        return Err(ApiError::bad_request(
            "Push notifications are disabled".to_string(),
        ));
    }

    let provider = configured_provider(config).ok_or_else(|| {
        ApiError::bad_request("Push notification provider is not configured".to_string())
    })?;

    let platform = map_platform(&body.platform);
    match platform {
        PushNotificationTargetPlatform::Desktop if !config.allow_desktop => {
            return Err(ApiError::bad_request(
                "Desktop push targets are disabled".to_string(),
            ));
        }
        PushNotificationTargetPlatform::Android | PushNotificationTargetPlatform::Ios
            if !config.allow_mobile =>
        {
            return Err(ApiError::bad_request(
                "Mobile push targets are disabled".to_string(),
            ));
        }
        _ => {}
    }

    if body.device_id.trim().is_empty() || body.token.trim().is_empty() {
        return Err(ApiError::bad_request(
            "device_id and token are required".to_string(),
        ));
    }

    let now = chrono::Utc::now().fixed_offset();
    let token_encrypted = encrypt_token(&body.token, &state.encryption_key);

    // Disable targets on the same device+provider owned by a different user.
    // Multiple devices per user are allowed; what's not allowed is two users
    // sharing the same physical device id — the previous owner must be cut off
    // when the device is handed over (sign-out/sign-in flow).
    push_notification_target::Entity::update_many()
        .col_expr(
            push_notification_target::Column::PushEnabled,
            Expr::value(false),
        )
        .col_expr(
            push_notification_target::Column::InvalidatedAt,
            Expr::value(Some(now)),
        )
        .col_expr(
            push_notification_target::Column::InvalidationReason,
            Expr::value(Some("Device reassigned to another user")),
        )
        .col_expr(
            push_notification_target::Column::UpdatedAt,
            Expr::value(now),
        )
        .filter(push_notification_target::Column::DeviceId.eq(body.device_id.clone()))
        .filter(push_notification_target::Column::Provider.eq(provider.clone()))
        .filter(push_notification_target::Column::UserId.ne(sub.clone()))
        .filter(push_notification_target::Column::PushEnabled.eq(true))
        .exec(&state.db)
        .await?;

    let existing = find_existing_push_target(
        &state,
        &sub,
        &body.device_id,
        &platform,
        &provider,
        &body.token,
    )
    .await?;

    let provider_registration = prepare_provider_target_registration(
        &state,
        &sub,
        &body.device_id,
        platform.clone(),
        provider.clone(),
        &body.token,
        existing
            .as_ref()
            .and_then(|target| target.endpoint_arn.as_deref()),
        existing
            .as_ref()
            .and_then(|target| target.installation_id.as_deref()),
    )
    .await
    .map_err(|error| ApiError::bad_request(error.to_string()))?;

    let mut push_enabled = true;
    let target_id = if let Some(existing) = existing {
        push_enabled = should_auto_enable_target(&existing);
        let mut active: push_notification_target::ActiveModel = existing.into();
        active.device_id = Set(body.device_id.clone());
        active.platform = Set(platform);
        active.token_encrypted = Set(token_encrypted);
        active.endpoint_arn = Set(provider_registration.endpoint_arn.clone());
        active.installation_id = Set(provider_registration.installation_id.clone());
        active.channel_id = Set(body.channel_id.clone());
        active.device_name = Set(body.device_name.clone());
        active.metadata = Set(body.metadata.clone());
        active.push_enabled = Set(push_enabled);
        active.failure_count = Set(0);
        if push_enabled {
            active.invalidated_at = Set(None);
            active.invalidation_reason = Set(None);
        }
        active.last_registered_at = Set(now);
        active.last_seen_at = Set(now);
        active.updated_at = Set(now);
        active.update(&state.db).await?.id
    } else {
        let target_id = create_id();
        push_notification_target::ActiveModel {
            id: Set(target_id.clone()),
            user_id: Set(sub.clone()),
            device_id: Set(body.device_id.clone()),
            platform: Set(platform),
            provider: Set(provider.clone()),
            token_encrypted: Set(token_encrypted),
            endpoint_arn: Set(provider_registration.endpoint_arn),
            installation_id: Set(provider_registration.installation_id),
            channel_id: Set(body.channel_id.clone()),
            device_name: Set(body.device_name.clone()),
            metadata: Set(body.metadata.clone()),
            push_enabled: Set(true),
            failure_count: Set(0),
            last_registered_at: Set(now),
            last_seen_at: Set(now),
            invalidated_at: Set(None),
            invalidation_reason: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&state.db)
        .await?;
        target_id
    };

    Ok(Json(RegisterPushTargetResponse {
        id: target_id,
        provider: provider_name(&provider).to_string(),
        success: true,
        push_enabled,
    }))
}

#[utoipa::path(
    get,
    path = "/user/push-targets/{device_id}",
    tag = "user",
    params(("device_id" = String, Path, description = "Device ID to inspect")),
    responses(
        (status = 200, description = "Push target status", body = PushTargetStatusResponse),
        (status = 401, description = "Unauthorized")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /user/push-targets/{device_id}", skip_all)]
pub async fn get_push_target_status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(device_id): Path<String>,
) -> Result<Json<PushTargetStatusResponse>, ApiError> {
    let sub = user.sub()?;
    ensure_user_exists(&state, &sub).await?;

    let provider = configured_provider(&state.platform_config.push_notifications);
    let target = match &provider {
        Some(provider) => find_push_target_by_device(&state, &sub, &device_id, provider).await?,
        None => None,
    };

    Ok(Json(target_status_response(
        device_id,
        provider.as_ref(),
        target,
    )))
}

#[utoipa::path(
    patch,
    path = "/user/push-targets/{device_id}",
    tag = "user",
    params(("device_id" = String, Path, description = "Device ID to update")),
    request_body = UpdatePushTargetRequest,
    responses(
        (status = 200, description = "Push target updated", body = PushTargetStatusResponse),
        (status = 400, description = "Push notifications are not enabled or provider is not configured"),
        (status = 401, description = "Unauthorized")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "PATCH /user/push-targets/{device_id}", skip_all)]
pub async fn update_push_target_status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(device_id): Path<String>,
    Json(body): Json<UpdatePushTargetRequest>,
) -> Result<Json<PushTargetStatusResponse>, ApiError> {
    let sub = user.sub()?;
    ensure_user_exists(&state, &sub).await?;

    let config = &state.platform_config.push_notifications;
    let provider = configured_provider(config).ok_or_else(|| {
        ApiError::bad_request("Push notification provider is not configured".to_string())
    })?;

    if body.push_enabled && !config.enabled {
        return Err(ApiError::bad_request(
            "Push notifications are disabled".to_string(),
        ));
    }

    let now = chrono::Utc::now().fixed_offset();
    let mut update = push_notification_target::Entity::update_many()
        .col_expr(
            push_notification_target::Column::PushEnabled,
            Expr::value(body.push_enabled),
        )
        .col_expr(
            push_notification_target::Column::LastSeenAt,
            Expr::value(now),
        )
        .col_expr(
            push_notification_target::Column::UpdatedAt,
            Expr::value(now),
        )
        .filter(push_notification_target::Column::UserId.eq(sub.clone()))
        .filter(push_notification_target::Column::DeviceId.eq(device_id.clone()))
        .filter(push_notification_target::Column::Provider.eq(provider.clone()));

    if body.push_enabled {
        update = update
            .col_expr(
                push_notification_target::Column::FailureCount,
                Expr::value(0),
            )
            .col_expr(
                push_notification_target::Column::InvalidatedAt,
                Expr::value(None::<chrono::DateTime<chrono::FixedOffset>>),
            )
            .col_expr(
                push_notification_target::Column::InvalidationReason,
                Expr::value(None::<String>),
            );
    } else {
        update = update
            .col_expr(
                push_notification_target::Column::InvalidatedAt,
                Expr::value(Some(now)),
            )
            .col_expr(
                push_notification_target::Column::InvalidationReason,
                Expr::value(Some(USER_DISABLED_PUSH_REASON.to_string())),
            );
    }

    let update_result = update.exec(&state.db).await?;
    if update_result.rows_affected == 0 {
        return Err(ApiError::bad_request("Push target not found".to_string()));
    }

    let target = find_push_target_by_device(&state, &sub, &device_id, &provider).await?;
    Ok(Json(target_status_response(
        device_id,
        Some(&provider),
        target,
    )))
}

#[utoipa::path(
    delete,
    path = "/user/push-targets/{device_id}",
    tag = "user",
    params(("device_id" = String, Path, description = "Device ID to disable")),
    responses(
        (status = 200, description = "Push target disabled", body = UnregisterPushTargetResponse),
        (status = 401, description = "Unauthorized")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "DELETE /user/push-targets/{device_id}", skip_all)]
pub async fn unregister_push_target(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(device_id): Path<String>,
    Query(query): Query<UnregisterPushTargetQuery>,
) -> Result<Json<UnregisterPushTargetResponse>, ApiError> {
    let sub = user.sub()?;
    let now = chrono::Utc::now().fixed_offset();
    let reason = unregister_reason(query.reason.as_deref());

    push_notification_target::Entity::update_many()
        .col_expr(
            push_notification_target::Column::PushEnabled,
            Expr::value(false),
        )
        .col_expr(
            push_notification_target::Column::InvalidatedAt,
            Expr::value(Some(now)),
        )
        .col_expr(
            push_notification_target::Column::InvalidationReason,
            Expr::value(Some(reason)),
        )
        .col_expr(
            push_notification_target::Column::UpdatedAt,
            Expr::value(now),
        )
        .filter(push_notification_target::Column::UserId.eq(sub))
        .filter(push_notification_target::Column::DeviceId.eq(device_id))
        .filter(push_notification_target::Column::PushEnabled.eq(true))
        .exec(&state.db)
        .await?;

    Ok(Json(UnregisterPushTargetResponse { success: true }))
}

fn unregister_reason(reason: Option<&str>) -> String {
    match reason.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    }) {
        Some("permission_revoked") => "User revoked notification permission".to_string(),
        Some("sign_out") => "User signed out on this device".to_string(),
        Some("user_disabled") => USER_DISABLED_PUSH_REASON.to_string(),
        Some(value) => format!("Client unregistered push target: {}", value),
        None => "Client unregistered push target".to_string(),
    }
}

fn should_auto_enable_target(target: &push_notification_target::Model) -> bool {
    target.invalidation_reason.as_deref() != Some(USER_DISABLED_PUSH_REASON)
}

fn map_platform(platform: &PushTargetPlatformDto) -> PushNotificationTargetPlatform {
    match platform {
        PushTargetPlatformDto::Ios => PushNotificationTargetPlatform::Ios,
        PushTargetPlatformDto::Android => PushNotificationTargetPlatform::Android,
        PushTargetPlatformDto::Desktop => PushNotificationTargetPlatform::Desktop,
    }
}

fn provider_name(provider: &PushNotificationTargetProvider) -> &'static str {
    match provider {
        PushNotificationTargetProvider::Fcm => "FCM",
        PushNotificationTargetProvider::AwsSns => "AWS_SNS",
        PushNotificationTargetProvider::AzureNotificationHubs => "AZURE_NOTIFICATION_HUBS",
    }
}

fn platform_name(platform: &PushNotificationTargetPlatform) -> &'static str {
    match platform {
        PushNotificationTargetPlatform::Ios => "IOS",
        PushNotificationTargetPlatform::Android => "ANDROID",
        PushNotificationTargetPlatform::Desktop => "DESKTOP",
    }
}

fn utc(dt: chrono::DateTime<chrono::FixedOffset>) -> chrono::DateTime<chrono::Utc> {
    dt.to_utc()
}

fn target_status_response(
    device_id: String,
    provider: Option<&PushNotificationTargetProvider>,
    target: Option<push_notification_target::Model>,
) -> PushTargetStatusResponse {
    match target {
        Some(target) => PushTargetStatusResponse {
            device_id,
            provider: Some(provider_name(&target.provider).to_string()),
            registered: true,
            push_enabled: target.push_enabled,
            platform: Some(platform_name(&target.platform).to_string()),
            device_name: target.device_name,
            channel_id: target.channel_id,
            failure_count: Some(target.failure_count),
            last_registered_at: Some(utc(target.last_registered_at)),
            last_seen_at: Some(utc(target.last_seen_at)),
            invalidated_at: target.invalidated_at.map(utc),
            invalidation_reason: target.invalidation_reason,
            updated_at: Some(utc(target.updated_at)),
        },
        None => PushTargetStatusResponse {
            device_id,
            provider: provider.map(provider_name).map(str::to_string),
            registered: false,
            push_enabled: false,
            platform: None,
            device_name: None,
            channel_id: None,
            failure_count: None,
            last_registered_at: None,
            last_seen_at: None,
            invalidated_at: None,
            invalidation_reason: None,
            updated_at: None,
        },
    }
}

async fn find_push_target_by_device(
    state: &AppState,
    user_id: &str,
    device_id: &str,
    provider: &PushNotificationTargetProvider,
) -> Result<Option<push_notification_target::Model>, sea_orm::DbErr> {
    push_notification_target::Entity::find()
        .filter(push_notification_target::Column::UserId.eq(user_id.to_string()))
        .filter(push_notification_target::Column::DeviceId.eq(device_id.to_string()))
        .filter(push_notification_target::Column::Provider.eq(provider.clone()))
        .one(&state.db)
        .await
}

async fn find_existing_push_target(
    state: &AppState,
    user_id: &str,
    device_id: &str,
    platform: &PushNotificationTargetPlatform,
    provider: &PushNotificationTargetProvider,
    token: &str,
) -> Result<Option<push_notification_target::Model>, sea_orm::DbErr> {
    if let Some(existing) = push_notification_target::Entity::find()
        .filter(push_notification_target::Column::UserId.eq(user_id.to_string()))
        .filter(push_notification_target::Column::DeviceId.eq(device_id.to_string()))
        .filter(push_notification_target::Column::Provider.eq(provider.clone()))
        .one(&state.db)
        .await?
    {
        return Ok(Some(existing));
    }

    let candidates = push_notification_target::Entity::find()
        .filter(push_notification_target::Column::UserId.eq(user_id.to_string()))
        .filter(push_notification_target::Column::Platform.eq(platform.clone()))
        .filter(push_notification_target::Column::Provider.eq(provider.clone()))
        .order_by_desc(push_notification_target::Column::LastSeenAt)
        .all(&state.db)
        .await?;

    let matching = find_matching_token_target(candidates, token, &state.encryption_key);
    if let Some(target) = matching.as_ref() {
        tracing::info!(
            target_id = %target.id,
            user_id = %user_id,
            old_device_id = %target.device_id,
            new_device_id = %device_id,
            platform = ?platform,
            provider = ?provider,
            push_enabled = target.push_enabled,
            invalidated_at = ?target.invalidated_at,
            invalidation_reason = ?target.invalidation_reason,
            "Reusing push target by token match after device id drift"
        );
    }

    Ok(matching)
}

fn find_matching_token_target<I>(
    candidates: I,
    token: &str,
    encryption_key: &[u8; 32],
) -> Option<push_notification_target::Model>
where
    I: IntoIterator<Item = push_notification_target::Model>,
{
    candidates.into_iter().find(|candidate| {
        decrypt_token(&candidate.token_encrypted, encryption_key).as_deref() == Some(token)
    })
}

fn encrypt_token(token: &str, key: &[u8; 32]) -> String {
    crate::utils::crypto::encrypt_secret(token, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(
        id: &str,
        token: &str,
        encryption_key: &[u8; 32],
        push_enabled: bool,
        invalidation_reason: Option<&str>,
    ) -> push_notification_target::Model {
        let now = chrono::Utc::now().fixed_offset();
        push_notification_target::Model {
            id: id.to_string(),
            user_id: "user-1".to_string(),
            device_id: format!("device-{id}"),
            platform: PushNotificationTargetPlatform::Ios,
            provider: PushNotificationTargetProvider::Fcm,
            token_encrypted: encrypt_token(token, encryption_key),
            endpoint_arn: None,
            installation_id: None,
            channel_id: None,
            device_name: None,
            metadata: None,
            push_enabled,
            last_registered_at: now,
            last_seen_at: now,
            invalidated_at: None,
            invalidation_reason: invalidation_reason.map(str::to_string),
            created_at: now,
            updated_at: now,
            failure_count: 0,
        }
    }

    #[test]
    fn find_matching_token_target_reuses_disabled_legacy_row() {
        let encryption_key = [7u8; 32];
        let matching = target("old-ios", "ios-token", &encryption_key, false, None);
        let other = target("other-ios", "other-token", &encryption_key, true, None);

        let found = find_matching_token_target(vec![other, matching], "ios-token", &encryption_key)
            .expect("expected token match");

        assert_eq!(found.id, "old-ios");
        assert!(!found.push_enabled);
        assert_eq!(found.invalidation_reason, None);
    }

    #[test]
    fn find_matching_token_target_ignores_different_tokens() {
        let encryption_key = [9u8; 32];
        let other = target("other-ios", "other-token", &encryption_key, true, None);

        let found = find_matching_token_target(vec![other], "ios-token", &encryption_key);

        assert!(found.is_none());
    }

    #[test]
    fn user_disabled_targets_are_not_auto_enabled_by_registration() {
        let encryption_key = [4u8; 32];
        let target = target(
            "disabled-ios",
            "ios-token",
            &encryption_key,
            false,
            Some(USER_DISABLED_PUSH_REASON),
        );

        assert!(!should_auto_enable_target(&target));
    }

    #[test]
    fn invalidated_targets_are_auto_enabled_by_registration() {
        let encryption_key = [5u8; 32];
        let target = target(
            "invalidated-ios",
            "ios-token",
            &encryption_key,
            false,
            Some("15 consecutive invalidation-class failures"),
        );

        assert!(should_auto_enable_target(&target));
    }
}
