use crate::{
    entity::{
        notification, push_notification_target,
        sea_orm_active_enums::{
            NotificationType, PushNotificationTargetPlatform, PushNotificationTargetProvider,
        },
    },
    notification_images::prepare_notification_icon,
    routes::app::events::db::decrypt_token,
    state::AppState,
};
use flow_like::hub::{PushNotificationProviderType, PushNotificationsConfig};
use flow_like_secrets::{ExposeSecret, SecretRef};
use flow_like_types::create_id;
use flow_like_types::tokio::sync::RwLock;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::StatusCode;
use sea_orm::sea_query::ExprTrait;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Instant;

const NOTIFICATION_DEDUPE_WINDOW_SECONDS: i64 = 10;

#[cfg(feature = "azure")]
use base64::Engine;
#[cfg(feature = "azure")]
use hmac::{Hmac, Mac};
#[cfg(feature = "azure")]
use sha2::Sha256;

fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

struct CachedAccessToken {
    token: String,
    expires_at: Instant,
}

static GOOGLE_TOKEN_CACHE: OnceLock<RwLock<Option<CachedAccessToken>>> = OnceLock::new();

fn google_token_cache() -> &'static RwLock<Option<CachedAccessToken>> {
    GOOGLE_TOKEN_CACHE.get_or_init(|| RwLock::new(None))
}

#[derive(Clone, Debug)]
pub struct DispatchNotificationInput {
    pub user_id: String,
    pub app_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub image: Option<String>,
    pub link: Option<String>,
    pub notification_type: NotificationType,
    pub source_run_id: Option<String>,
    pub source_node_id: Option<String>,
}

/// Provider acceptance does not confirm that a device displayed the notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PushDispatchStatus {
    Disabled,
    NoTargets,
    Accepted,
    Partial,
    Failed,
    /// An existing record was reused without a new push attempt; its earlier outcome is unknown.
    Deduplicated,
}

impl PushDispatchStatus {
    pub fn is_success(self) -> bool {
        !matches!(self, Self::Partial | Self::Failed | Self::Deduplicated)
    }

    fn from_counts(accepted: usize, failed: usize) -> Self {
        match (accepted, failed) {
            (0, 0) => Self::NoTargets,
            (_, 0) => Self::Accepted,
            (0, _) => Self::Failed,
            _ => Self::Partial,
        }
    }
}

pub struct DispatchNotificationResult {
    pub id: String,
    pub push_status: PushDispatchStatus,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderTargetRegistration {
    pub endpoint_arn: Option<String>,
    pub installation_id: Option<String>,
}

/// The fields of a Google service-account key file this crate signs with.
#[derive(Debug, Deserialize)]
pub(crate) struct GoogleServiceAccount {
    pub(crate) client_email: String,
    pub(crate) private_key: String,
    #[serde(default)]
    pub(crate) private_key_id: Option<String>,
    #[serde(default)]
    pub(crate) token_uri: Option<String>,
}

impl GoogleServiceAccount {
    pub(crate) fn parse(service_account_json: &str) -> flow_like_types::Result<Self> {
        let account: Self = serde_json::from_str(service_account_json)
            .map_err(|e| flow_like_types::anyhow!("service account JSON is malformed: {e}"))?;
        if account.client_email.trim().is_empty() || account.private_key.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "service account JSON is missing client_email or private_key"
            ));
        }
        Ok(account)
    }
}

const FIREBASE_MESSAGING_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

#[derive(Debug, Serialize)]
struct GoogleClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: usize,
    iat: usize,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
}

#[derive(Debug, Deserialize)]
struct FcmErrorResponse {
    error: Option<FcmErrorBody>,
}

#[derive(Debug, Deserialize)]
struct FcmErrorBody {
    #[allow(dead_code)]
    code: Option<u16>,
    #[allow(dead_code)]
    message: Option<String>,
    #[allow(dead_code)]
    status: Option<String>,
    details: Option<Vec<FcmErrorDetail>>,
}

#[derive(Debug, Deserialize)]
struct FcmErrorDetail {
    #[serde(rename = "errorCode")]
    error_code: Option<String>,
}

enum FcmOutcome {
    Success,
    InvalidateTarget(String),
    Transient(u16, String),
    Permanent(String),
}

pub async fn dispatch_notification(
    state: &AppState,
    input: DispatchNotificationInput,
) -> Result<String, sea_orm::DbErr> {
    Ok(dispatch_notification_with_status(state, input).await?.id)
}

pub async fn dispatch_notification_with_status(
    state: &AppState,
    mut input: DispatchNotificationInput,
) -> Result<DispatchNotificationResult, sea_orm::DbErr> {
    let prepared_icon = prepare_notification_icon(state, &input.user_id, input.icon.as_deref())
        .await
        .map_err(|error| sea_orm::DbErr::Custom(error.to_string()))?;
    input.icon = prepared_icon.stored_icon;
    if input.source_run_id.is_some() || input.source_node_id.is_some() {
        let cutoff = chrono::Utc::now().fixed_offset()
            - chrono::Duration::seconds(NOTIFICATION_DEDUPE_WINDOW_SECONDS);
        let mut existing = notification::Entity::find()
            .filter(notification::Column::UserId.eq(input.user_id.clone()))
            .filter(notification::Column::Title.eq(input.title.clone()))
            .filter(notification::Column::Type.eq(input.notification_type.clone()))
            .filter(notification::Column::CreatedAt.gte(cutoff));

        existing = match &input.app_id {
            Some(value) => existing.filter(notification::Column::AppId.eq(value.clone())),
            None => existing.filter(notification::Column::AppId.is_null()),
        };
        existing = match &input.description {
            Some(value) => existing.filter(notification::Column::Description.eq(value.clone())),
            None => existing.filter(notification::Column::Description.is_null()),
        };
        existing = match &input.icon {
            Some(value) => existing.filter(notification::Column::Icon.eq(value.clone())),
            None => existing.filter(notification::Column::Icon.is_null()),
        };
        existing = match &input.link {
            Some(value) => existing.filter(notification::Column::Link.eq(value.clone())),
            None => existing.filter(notification::Column::Link.is_null()),
        };
        existing = match &input.source_run_id {
            Some(value) => existing.filter(notification::Column::SourceRunId.eq(value.clone())),
            None => existing.filter(notification::Column::SourceRunId.is_null()),
        };
        existing = match &input.source_node_id {
            Some(value) => existing.filter(notification::Column::SourceNodeId.eq(value.clone())),
            None => existing.filter(notification::Column::SourceNodeId.is_null()),
        };

        if let Some(notification) = existing
            .order_by_desc(notification::Column::CreatedAt)
            .one(&state.db)
            .await?
        {
            tracing::info!(
                notification_id = %notification.id,
                user_id = %input.user_id,
                source_run_id = ?input.source_run_id,
                source_node_id = ?input.source_node_id,
                "Reusing recently-created matching notification"
            );
            return Ok(DispatchNotificationResult {
                id: notification.id,
                push_status: PushDispatchStatus::Deduplicated,
            });
        }
    }

    let notification_id = create_id();
    let notification = notification::ActiveModel {
        id: Set(notification_id.clone()),
        user_id: Set(input.user_id.clone()),
        app_id: Set(input.app_id.clone()),
        title: Set(input.title.clone()),
        description: Set(input.description.clone()),
        icon: Set(input.icon.clone()),
        link: Set(input.link.clone()),
        r#type: Set(input.notification_type.clone()),
        read: Set(false),
        source_run_id: Set(input.source_run_id.clone()),
        source_node_id: Set(input.source_node_id.clone()),
        created_at: Set(chrono::Utc::now().fixed_offset()),
        read_at: Set(None),
    };

    notification.insert(&state.db).await?;

    input.icon = prepared_icon.push_icon;
    let push_status = match push_to_user(state, &notification_id, &input).await {
        Ok(status) => status,
        Err(error) => {
            tracing::warn!(
                error = %error,
                notification_id = %notification_id,
                user_id = %input.user_id,
                "Failed to dispatch push notification"
            );
            PushDispatchStatus::Failed
        }
    };

    Ok(DispatchNotificationResult {
        id: notification_id,
        push_status,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn prepare_provider_target_registration(
    state: &AppState,
    user_id: &str,
    device_id: &str,
    platform: PushNotificationTargetPlatform,
    provider: PushNotificationTargetProvider,
    token: &str,
    existing_endpoint_arn: Option<&str>,
    existing_installation_id: Option<&str>,
) -> flow_like_types::Result<ProviderTargetRegistration> {
    let config = &state.platform_config.push_notifications;

    match provider {
        PushNotificationTargetProvider::Fcm => Ok(ProviderTargetRegistration::default()),
        PushNotificationTargetProvider::AwsSns => {
            #[cfg(feature = "aws")]
            {
                let endpoint_arn = register_aws_sns_target(
                    state,
                    config,
                    user_id,
                    device_id,
                    &platform,
                    token,
                    existing_endpoint_arn,
                )
                .await?;

                Ok(ProviderTargetRegistration {
                    endpoint_arn: Some(endpoint_arn),
                    installation_id: None,
                })
            }
            #[cfg(not(feature = "aws"))]
            {
                let _ = (
                    state,
                    user_id,
                    device_id,
                    platform,
                    token,
                    existing_endpoint_arn,
                    existing_installation_id,
                );
                Err(flow_like_types::anyhow!(
                    "AWS SNS push registration requires the aws feature"
                ))
            }
        }
        PushNotificationTargetProvider::AzureNotificationHubs => {
            #[cfg(feature = "azure")]
            {
                let installation_id = register_azure_installation(
                    config,
                    user_id,
                    device_id,
                    &platform,
                    token,
                    existing_installation_id,
                )
                .await?;

                Ok(ProviderTargetRegistration {
                    endpoint_arn: None,
                    installation_id: Some(installation_id),
                })
            }
            #[cfg(not(feature = "azure"))]
            {
                let _ = (
                    state,
                    user_id,
                    device_id,
                    platform,
                    token,
                    existing_endpoint_arn,
                    existing_installation_id,
                );
                Err(flow_like_types::anyhow!(
                    "Azure Notification Hubs registration requires the azure feature"
                ))
            }
        }
    }
}

pub fn configured_provider(
    config: &PushNotificationsConfig,
) -> Option<PushNotificationTargetProvider> {
    match config.provider.as_ref()? {
        PushNotificationProviderType::Fcm => Some(PushNotificationTargetProvider::Fcm),
        PushNotificationProviderType::AwsSns => Some(PushNotificationTargetProvider::AwsSns),
        PushNotificationProviderType::AzureNotificationHubs => {
            Some(PushNotificationTargetProvider::AzureNotificationHubs)
        }
    }
}

async fn push_to_user(
    state: &AppState,
    notification_id: &str,
    input: &DispatchNotificationInput,
) -> flow_like_types::Result<PushDispatchStatus> {
    let config = &state.platform_config.push_notifications;
    if !config.enabled {
        return Ok(PushDispatchStatus::Disabled);
    }

    let Some(provider) = configured_provider(config) else {
        return Err(flow_like_types::anyhow!(
            "Push notifications are enabled without a provider"
        ));
    };

    let stale_cutoff = chrono::Utc::now().fixed_offset() - chrono::Duration::days(30);

    let targets = push_notification_target::Entity::find()
        .filter(push_notification_target::Column::UserId.eq(input.user_id.clone()))
        .filter(push_notification_target::Column::Provider.eq(provider.clone()))
        .filter(push_notification_target::Column::PushEnabled.eq(true))
        .filter(push_notification_target::Column::InvalidatedAt.is_null())
        .filter(push_notification_target::Column::LastSeenAt.gt(stale_cutoff))
        .order_by_desc(push_notification_target::Column::LastSeenAt)
        .all(&state.db)
        .await?;

    let mut accepted = 0;
    let mut failed = 0;
    for target in targets {
        if !is_target_allowed(config, &target.platform) {
            continue;
        }

        let Some(token) = decrypt_token(&target.token_encrypted, &state.encryption_key) else {
            failed += 1;
            tracing::warn!(target_id = %target.id, "Failed to decrypt push token");
            continue;
        };

        let result = match provider {
            PushNotificationTargetProvider::Fcm => {
                send_via_fcm(state, config, &target, &token, notification_id, input).await
            }
            PushNotificationTargetProvider::AwsSns => {
                #[cfg(feature = "aws")]
                {
                    send_via_aws_sns(state, &target, notification_id, input).await
                }
                #[cfg(not(feature = "aws"))]
                {
                    Err(flow_like_types::anyhow!(
                        "AWS SNS push delivery requires the aws feature"
                    ))
                }
            }
            PushNotificationTargetProvider::AzureNotificationHubs => {
                #[cfg(feature = "azure")]
                {
                    send_via_azure_notification_hubs(config, &target, notification_id, input).await
                }
                #[cfg(not(feature = "azure"))]
                {
                    Err(flow_like_types::anyhow!(
                        "Azure Notification Hubs delivery requires the azure feature"
                    ))
                }
            }
        };

        if let Err(error) = result {
            failed += 1;
            let message = error.to_string();
            if should_invalidate_target(&message) {
                record_invalidation_failure(state, &target.id, &message).await?;
            }

            tracing::warn!(
                error = %message,
                target_id = %target.id,
                provider = ?provider,
                failure_count = target.failure_count,
                "Push target send failed"
            );
        } else {
            accepted += 1;
            // Successful delivery clears the consecutive-failure streak so a
            // healthy device never accumulates toward the disable threshold.
            if target.failure_count > 0 {
                reset_failure_count(state, &target.id).await?;
            }
        }
    }

    Ok(PushDispatchStatus::from_counts(accepted, failed))
}

fn is_target_allowed(
    config: &PushNotificationsConfig,
    platform: &PushNotificationTargetPlatform,
) -> bool {
    match platform {
        PushNotificationTargetPlatform::Desktop => config.allow_desktop,
        PushNotificationTargetPlatform::Android | PushNotificationTargetPlatform::Ios => {
            config.allow_mobile
        }
    }
}

const MAX_PUSH_PAYLOAD_BYTES: usize = 4096;
const MAX_PUSH_IMAGE_URL_BYTES: usize = 2048;

fn is_push_image_url(value: &str) -> bool {
    if value.len() > MAX_PUSH_IMAGE_URL_BYTES {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return false;
    }
    match host.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_multicast()
                && !ip.is_documentation()
                && ip.octets()[0] != 0
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            !ip.is_loopback()
                && !ip.is_unspecified()
                && !ip.is_unique_local()
                && !ip.is_unicast_link_local()
                && !ip.is_multicast()
                && ip.to_ipv4_mapped().is_none()
        }
        Err(_) => host.contains('.'),
    }
}

fn notification_image_url(input: &DispatchNotificationInput) -> Option<&str> {
    input
        .image
        .as_deref()
        .filter(|url| is_push_image_url(url))
        .or_else(|| input.icon.as_deref().filter(|url| is_push_image_url(url)))
}

fn push_text(value: &str, max_bytes: usize) -> &str {
    let mut end = value.len().min(max_bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn remove_payload_keys(value: &mut serde_json::Value, keys: &[&str]) {
    if let Some(object) = value.as_object_mut() {
        object.retain(|key, _| !keys.contains(&key.as_str()));
        for child in object.values_mut() {
            remove_payload_keys(child, keys);
        }
    }
}

fn shorten_payload_preview(value: &mut serde_json::Value, title_bytes: usize, body_bytes: usize) {
    if let Some(object) = value.as_object_mut() {
        for (key, child) in object.iter_mut() {
            if let Some(text) = child.as_str() {
                let limit = match key.as_str() {
                    "title" => Some(title_bytes),
                    "body" => Some(body_bytes),
                    _ => None,
                };
                if let Some(limit) = limit {
                    *child = serde_json::json!(push_text(text, limit));
                }
            } else {
                shorten_payload_preview(child, title_bytes, body_bytes);
            }
        }
    }
}

/// Preserve routing when media and text previews compete for the payload budget.
fn fit_push_payload(mut payload: serde_json::Value) -> flow_like_types::Result<serde_json::Value> {
    if serde_json::to_vec(&payload)?.len() <= MAX_PUSH_PAYLOAD_BYTES {
        return Ok(payload);
    }
    remove_payload_keys(
        &mut payload,
        &["icon", "image", "fcm_options", "mutable-content"],
    );
    for (title_bytes, body_bytes) in [(128, 512), (64, 128)] {
        if serde_json::to_vec(&payload)?.len() <= MAX_PUSH_PAYLOAD_BYTES {
            return Ok(payload);
        }
        shorten_payload_preview(&mut payload, title_bytes, body_bytes);
    }
    if serde_json::to_vec(&payload)?.len() > MAX_PUSH_PAYLOAD_BYTES {
        return Err(flow_like_types::anyhow!(
            "Push payload exceeds 4096 bytes while preserving notification routing"
        ));
    }
    Ok(payload)
}

#[cfg(any(feature = "aws", feature = "azure", test))]
fn apns_payload(
    input: &DispatchNotificationInput,
    data: HashMap<String, String>,
) -> flow_like_types::Result<serde_json::Value> {
    let mut payload = serde_json::json!({
        "aps": {
            "alert": {
                "title": push_text(&input.title, 256),
                "body": push_text(input.description.as_deref().unwrap_or_default(), 1024),
            },
            "sound": "default",
        }
    });
    if let Some(map) = payload.as_object_mut() {
        for (key, value) in data {
            map.insert(key, serde_json::Value::String(value));
        }
    }
    if let Some(image) = notification_image_url(input) {
        payload["aps"]["mutable-content"] = serde_json::json!(1);
        payload["image"] = serde_json::json!(image);
    }
    fit_push_payload(payload)
}

fn should_invalidate_target(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    if lower.starts_with("[invalid-push-target]") {
        return true;
    }
    // Generic FCM INVALID_ARGUMENT also describes payload errors.
    if lower.starts_with("fcm ") {
        return false;
    }
    lower.contains("unregistered")
        || lower.contains("not registered")
        || lower.contains("invalid registration")
        || lower.contains("requested entity was not found")
        || lower.contains("endpointdisabled")
        || lower.contains("endpoint is disabled")
        || lower.contains("invalid token")
        || lower.contains("sender_id_mismatch")
}

/// Number of consecutive provider-classified "token is dead" failures we
/// tolerate before marking a push target as invalidated. A single failure
/// (transient FCM/APNs hiccup, fluky network) must NEVER kill a target —
/// the device may be online and the next send will succeed and reset the
/// counter. Only sustained, repeated invalidation-class errors disable.
const INVALIDATION_FAILURE_THRESHOLD: i32 = 15;

/// Increment the consecutive-failure counter on a target after the provider
/// returned an invalidation-class error. When the counter crosses
/// [`INVALIDATION_FAILURE_THRESHOLD`], the target is marked disabled with the
/// last seen reason. Successful sends call [`reset_failure_count`] to clear
/// the counter.
async fn record_invalidation_failure(
    state: &AppState,
    target_id: &str,
    reason: &str,
) -> Result<(), sea_orm::DbErr> {
    let now = chrono::Utc::now().fixed_offset();

    let updated = push_notification_target::Entity::update_many()
        .col_expr(
            push_notification_target::Column::FailureCount,
            sea_orm::sea_query::Expr::col(push_notification_target::Column::FailureCount).add(1),
        )
        .col_expr(
            push_notification_target::Column::UpdatedAt,
            sea_orm::sea_query::Expr::value(now),
        )
        .filter(push_notification_target::Column::Id.eq(target_id.to_string()))
        .exec_with_returning(&state.db)
        .await?;

    let Some(target) = updated.into_iter().next() else {
        return Ok(());
    };

    if target.failure_count >= INVALIDATION_FAILURE_THRESHOLD && target.push_enabled {
        let summary = format!(
            "{} consecutive invalidation-class failures; last reason: {}",
            target.failure_count, reason
        );
        push_notification_target::Entity::update_many()
            .col_expr(
                push_notification_target::Column::PushEnabled,
                sea_orm::sea_query::Expr::value(false),
            )
            .col_expr(
                push_notification_target::Column::InvalidatedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .col_expr(
                push_notification_target::Column::InvalidationReason,
                sea_orm::sea_query::Expr::value(Some(summary.clone())),
            )
            .col_expr(
                push_notification_target::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(now),
            )
            .filter(push_notification_target::Column::Id.eq(target_id.to_string()))
            .exec(&state.db)
            .await?;

        tracing::warn!(
            target_id = %target_id,
            failure_count = target.failure_count,
            reason = %summary,
            "Push target disabled after exceeding failure threshold"
        );
    }

    Ok(())
}

/// Reset the consecutive-failure counter to zero. Called after a successful
/// send so a previously-flaky-but-recovered target doesn't accumulate
/// failures across long windows. Filters on `FailureCount > 0` to skip the
/// no-op write on the steady-state happy path.
async fn reset_failure_count(state: &AppState, target_id: &str) -> Result<(), sea_orm::DbErr> {
    push_notification_target::Entity::update_many()
        .col_expr(
            push_notification_target::Column::FailureCount,
            sea_orm::sea_query::Expr::value(0),
        )
        .filter(push_notification_target::Column::Id.eq(target_id.to_string()))
        .filter(push_notification_target::Column::FailureCount.gt(0))
        .exec(&state.db)
        .await?;
    Ok(())
}

/// Reads the service account JSON via the secret store.
/// The resolved value can be either the raw JSON string or a path to a `.json` file.
async fn resolve_service_account_json(
    state: &AppState,
    env_name: &str,
) -> flow_like_types::Result<String> {
    let value = state
        .secrets
        .get_secret_string(&SecretRef::new(env_name))
        .await
        .map(|s| s.expose_secret().to_string())
        .map_err(|_| {
            flow_like_types::anyhow!("FCM service account secret '{}' is not set", env_name)
        })?;

    let trimmed = value.trim();
    if trimmed.starts_with('{') {
        return Ok(value);
    }

    std::fs::read_to_string(trimmed).map_err(|e| {
        flow_like_types::anyhow!("Failed to read service account file '{}': {}", trimmed, e)
    })
}

async fn send_via_fcm(
    state: &AppState,
    config: &PushNotificationsConfig,
    target: &push_notification_target::Model,
    token: &str,
    notification_id: &str,
    input: &DispatchNotificationInput,
) -> flow_like_types::Result<()> {
    let fcm = config
        .fcm
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("Missing FCM push configuration"))?;

    let service_account_json =
        resolve_service_account_json(state, &fcm.service_account_json_env).await?;

    let url = format!(
        "https://fcm.googleapis.com/v1/projects/{}/messages:send",
        fcm.project_id
    );

    let body = fcm_message_body(config, target, token, notification_id, input)?;

    const MAX_RETRIES: u32 = 2;
    let mut attempt = 0;

    loop {
        let access_token = fetch_google_access_token(&service_account_json).await?;

        let response = shared_http_client()
            .post(&url)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await?;

        let outcome = classify_fcm_response(response).await;
        match outcome {
            FcmOutcome::Success => return Ok(()),
            FcmOutcome::InvalidateTarget(reason) => {
                return Err(flow_like_types::anyhow!("[invalid-push-target] {}", reason));
            }
            FcmOutcome::Transient(status, text) => {
                attempt += 1;
                if attempt > MAX_RETRIES {
                    return Err(flow_like_types::anyhow!(
                        "FCM request failed after {} retries with status {}: {}",
                        MAX_RETRIES,
                        status,
                        text
                    ));
                }
                let delay = std::time::Duration::from_millis(500 * 2u64.pow(attempt - 1));
                flow_like_types::tokio::time::sleep(delay).await;
            }
            FcmOutcome::Permanent(reason) => {
                return Err(flow_like_types::anyhow!("{}", reason));
            }
        }
    }
}

fn fcm_message_body(
    config: &PushNotificationsConfig,
    target: &push_notification_target::Model,
    token: &str,
    notification_id: &str,
    input: &DispatchNotificationInput,
) -> flow_like_types::Result<serde_json::Value> {
    let data = notification_data(notification_id, target, input);
    let apns_data = data.clone();

    let mut notification_obj = serde_json::json!({
        "title": push_text(&input.title, 256),
        "body": push_text(input.description.as_deref().unwrap_or_default(), 1024),
    });
    if target.platform != PushNotificationTargetPlatform::Ios
        && let Some(image_url) = notification_image_url(input)
    {
        notification_obj["image"] = serde_json::Value::String(image_url.to_string());
    }

    let mut body = serde_json::json!({
        "message": {
            "token": token,
            "notification": notification_obj,
            "data": data,
        }
    });

    if target.platform == PushNotificationTargetPlatform::Android {
        let mut android_notification = serde_json::Map::new();
        if let Some(channel_id) = target.channel_id.clone().or(config.channel_id.clone()) {
            android_notification.insert(
                "channel_id".to_string(),
                serde_json::Value::String(channel_id),
            );
        }
        if let Some(image_url) = notification_image_url(input) {
            android_notification.insert(
                "image".to_string(),
                serde_json::Value::String(image_url.to_string()),
            );
        }
        if !android_notification.is_empty() {
            body["message"]["android"] = serde_json::json!({
                "notification": android_notification,
            });
        }
    }

    // iOS-specific: sound + badge via APNS payload
    if target.platform == PushNotificationTargetPlatform::Ios {
        let mut apns = serde_json::json!({
            "payload": {
                "aps": {
                    "sound": "default",
                    "badge": 1,
                }
            }
        });

        if let Some(payload) = apns
            .get_mut("payload")
            .and_then(serde_json::Value::as_object_mut)
        {
            for (key, value) in apns_data {
                payload.insert(key, value);
            }
        }

        if let Some(image_url) = notification_image_url(input) {
            apns["payload"]["aps"]["mutable-content"] = serde_json::json!(1);
            apns["fcm_options"] = serde_json::json!({
                "image": image_url,
            });
        }

        body["message"]["apns"] = apns;
        // The native client reads custom fields from APNs userInfo. Keeping a
        // second copy in FCM data wastes the notification's payload budget.
        body["message"].as_object_mut().unwrap().remove("data");
    }

    fit_push_payload(body)
}

async fn classify_fcm_response(response: reqwest::Response) -> FcmOutcome {
    let status = response.status();
    if status.is_success() {
        return FcmOutcome::Success;
    }

    let status_code = status.as_u16();
    let text = response.text().await.unwrap_or_default();
    classify_fcm_error(status_code, text)
}

fn classify_fcm_error(status_code: u16, text: String) -> FcmOutcome {
    let fcm_error_code = serde_json::from_str::<FcmErrorResponse>(&text)
        .ok()
        .and_then(|r| r.error)
        .and_then(|e| {
            e.details
                .and_then(|d| d.into_iter().find_map(|detail| detail.error_code))
        });

    let reason = if let Some(ref code) = fcm_error_code {
        format!("FCM error {}: {} (HTTP {})", code, text, status_code)
    } else {
        format!("FCM request failed with status {}: {}", status_code, text)
    };

    match fcm_error_code.as_deref() {
        Some("UNREGISTERED") | Some("SENDER_ID_MISMATCH") => FcmOutcome::InvalidateTarget(reason),
        _ if matches!(status_code, 429 | 500 | 503) => FcmOutcome::Transient(status_code, reason),
        _ => FcmOutcome::Permanent(reason),
    }
}

fn notification_data(
    notification_id: &str,
    target: &push_notification_target::Model,
    input: &DispatchNotificationInput,
) -> serde_json::Map<String, serde_json::Value> {
    notification_string_data(notification_id, target, input)
        .into_iter()
        .map(|(key, value)| (key, serde_json::Value::String(value)))
        .collect()
}

fn notification_string_data(
    notification_id: &str,
    target: &push_notification_target::Model,
    input: &DispatchNotificationInput,
) -> HashMap<String, String> {
    let mut data = HashMap::new();
    data.insert("notification_id".to_string(), notification_id.to_string());
    if target.device_id.len() <= 256 {
        data.insert("device_id".to_string(), target.device_id.clone());
    }
    data.insert(
        "notification_type".to_string(),
        match &input.notification_type {
            NotificationType::Workflow => "WORKFLOW".to_string(),
            NotificationType::System => "SYSTEM".to_string(),
        },
    );

    if let Some(app_id) = &input.app_id {
        data.insert("app_id".to_string(), app_id.clone());
    }
    if let Some(link) = &input.link {
        data.insert("link".to_string(), link.clone());
    }
    // Icons belong in the media fields. Inline image bytes cannot fit push payloads;
    // the authenticated history endpoint supplies the persistent notification icon.
    if let Some(run_id) = &input.source_run_id {
        data.insert("source_run_id".to_string(), run_id.clone());
    }
    if let Some(node_id) = &input.source_node_id {
        data.insert("source_node_id".to_string(), node_id.clone());
    }

    data
}

#[cfg(feature = "aws")]
async fn register_aws_sns_target(
    state: &AppState,
    config: &PushNotificationsConfig,
    user_id: &str,
    device_id: &str,
    platform: &PushNotificationTargetPlatform,
    token: &str,
    existing_endpoint_arn: Option<&str>,
) -> flow_like_types::Result<String> {
    let aws = config
        .aws_sns
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("Missing AWS SNS push configuration"))?;
    let platform_application_arn = aws_platform_application_arn(aws, platform)?;
    let client = aws_sdk_sns::Client::new(&state.aws_client);
    let custom_user_data = format!("user:{};device:{}", user_id, device_id);

    if let Some(endpoint_arn) = existing_endpoint_arn {
        client
            .set_endpoint_attributes()
            .endpoint_arn(endpoint_arn)
            .attributes("Token", token)
            .attributes("Enabled", "true")
            .attributes("CustomUserData", &custom_user_data)
            .send()
            .await?;
        return Ok(endpoint_arn.to_string());
    }

    let created = client
        .create_platform_endpoint()
        .platform_application_arn(platform_application_arn)
        .token(token)
        .custom_user_data(custom_user_data)
        .send()
        .await?;

    created
        .endpoint_arn()
        .map(ToOwned::to_owned)
        .ok_or_else(|| flow_like_types::anyhow!("AWS SNS did not return an endpoint ARN"))
}

#[cfg(feature = "aws")]
async fn send_via_aws_sns(
    state: &AppState,
    target: &push_notification_target::Model,
    notification_id: &str,
    input: &DispatchNotificationInput,
) -> flow_like_types::Result<()> {
    let endpoint_arn = target
        .endpoint_arn
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("AWS SNS target is missing an endpoint ARN"))?;
    let payload = aws_sns_payload(
        target,
        input,
        notification_string_data(notification_id, target, input),
    )?;

    aws_sdk_sns::Client::new(&state.aws_client)
        .publish()
        .target_arn(endpoint_arn)
        .message_structure("json")
        .message(payload)
        .send()
        .await?;

    Ok(())
}

#[cfg(feature = "aws")]
fn aws_platform_application_arn(
    aws: &flow_like::hub::AwsSnsPushNotificationsConfig,
    platform: &PushNotificationTargetPlatform,
) -> flow_like_types::Result<String> {
    let env_name = match platform {
        PushNotificationTargetPlatform::Android => &aws.android_platform_application_arn_env,
        PushNotificationTargetPlatform::Ios => &aws.ios_platform_application_arn_env,
        PushNotificationTargetPlatform::Desktop => {
            return Err(flow_like_types::anyhow!(
                "AWS SNS desktop push targets are not supported"
            ));
        }
    };

    std::env::var(env_name).map_err(|_| {
        flow_like_types::anyhow!(
            "AWS SNS platform application ARN env var '{}' is not set",
            env_name
        )
    })
}

#[cfg(feature = "aws")]
fn aws_sns_payload(
    target: &push_notification_target::Model,
    input: &DispatchNotificationInput,
    data: HashMap<String, String>,
) -> flow_like_types::Result<String> {
    let default_body = push_text(input.description.as_deref().unwrap_or(&input.title), 1024);
    let payload = match target.platform {
        PushNotificationTargetPlatform::Android => {
            let mut notification = serde_json::json!({
                "title": push_text(&input.title, 256),
                "body": push_text(input.description.as_deref().unwrap_or_default(), 1024),
            });

            if let Some(channel_id) = &target.channel_id {
                notification["channel_id"] = serde_json::Value::String(channel_id.clone());
            }
            if let Some(image_url) = notification_image_url(input) {
                notification["image"] = serde_json::Value::String(image_url.to_string());
            }

            serde_json::json!({
                "default": default_body,
                "GCM": serde_json::to_string(&fit_push_payload(serde_json::json!({
                    "notification": notification,
                    "data": data,
                }))?)?,
            })
        }
        PushNotificationTargetPlatform::Ios => {
            let apns = apns_payload(input, data)?;

            serde_json::json!({
                "default": default_body,
                "APNS": serde_json::to_string(&apns)?,
                "APNS_SANDBOX": serde_json::to_string(&apns)?,
            })
        }
        PushNotificationTargetPlatform::Desktop => {
            return Err(flow_like_types::anyhow!(
                "AWS SNS desktop push targets are not supported"
            ));
        }
    };

    Ok(serde_json::to_string(&payload)?)
}

#[cfg(feature = "azure")]
async fn register_azure_installation(
    config: &PushNotificationsConfig,
    user_id: &str,
    device_id: &str,
    platform: &PushNotificationTargetPlatform,
    token: &str,
    existing_installation_id: Option<&str>,
) -> flow_like_types::Result<String> {
    let azure = config
        .azure_notification_hubs
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("Missing Azure Notification Hubs configuration"))?;
    let resource_uri = azure_hub_resource_uri(azure);
    let installation_id = existing_installation_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| sanitize_installation_id(&format!("{}-{}", user_id, device_id)));
    let authorization = azure_sas_token(
        &resource_uri,
        &std::env::var(&azure.sas_key_name_env).map_err(|_| {
            flow_like_types::anyhow!(
                "Azure Notification Hubs SAS key name env var '{}' is not set",
                azure.sas_key_name_env
            )
        })?,
        &std::env::var(&azure.sas_key_value_env).map_err(|_| {
            flow_like_types::anyhow!(
                "Azure Notification Hubs SAS key value env var '{}' is not set",
                azure.sas_key_value_env
            )
        })?,
    )?;

    let response = reqwest::Client::new()
        .put(format!(
            "{}/installations/{}?api-version=2020-06",
            resource_uri, installation_id
        ))
        .header("Authorization", authorization)
        .header("Content-Type", "application/json;charset=utf-8")
        .json(&serde_json::json!({
            "installationId": installation_id,
            "platform": azure_platform_name(platform)?,
            "pushChannel": token,
            "tags": [format!("user:{}", user_id), format!("device:{}", device_id)],
        }))
        .send()
        .await?;

    if response.status().is_success() {
        Ok(installation_id)
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(flow_like_types::anyhow!(
            "Azure Notification Hubs installation request failed with status {}: {}",
            status,
            text
        ))
    }
}

#[cfg(feature = "azure")]
async fn send_via_azure_notification_hubs(
    config: &PushNotificationsConfig,
    target: &push_notification_target::Model,
    notification_id: &str,
    input: &DispatchNotificationInput,
) -> flow_like_types::Result<()> {
    let azure = config
        .azure_notification_hubs
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("Missing Azure Notification Hubs configuration"))?;
    let resource_uri = azure_hub_resource_uri(azure);
    let authorization = azure_sas_token(
        &resource_uri,
        &std::env::var(&azure.sas_key_name_env).map_err(|_| {
            flow_like_types::anyhow!(
                "Azure Notification Hubs SAS key name env var '{}' is not set",
                azure.sas_key_name_env
            )
        })?,
        &std::env::var(&azure.sas_key_value_env).map_err(|_| {
            flow_like_types::anyhow!(
                "Azure Notification Hubs SAS key value env var '{}' is not set",
                azure.sas_key_value_env
            )
        })?,
    )?;

    let response = reqwest::Client::new()
        .post(format!("{}/messages/?api-version=2015-01", resource_uri))
        .header("Authorization", authorization)
        .header("Content-Type", "application/json;charset=utf-8")
        .header(
            "ServiceBusNotification-Format",
            azure_message_format(&target.platform)?,
        )
        .header(
            "ServiceBusNotification-Tags",
            format!("device:{}", target.device_id),
        )
        .body(azure_message_payload(target, notification_id, input)?)
        .send()
        .await?;

    if response.status().is_success() {
        Ok(())
    } else {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        Err(flow_like_types::anyhow!(
            "Azure Notification Hubs send failed with status {}: {}",
            status,
            text
        ))
    }
}

#[cfg(feature = "azure")]
fn azure_message_payload(
    target: &push_notification_target::Model,
    notification_id: &str,
    input: &DispatchNotificationInput,
) -> flow_like_types::Result<String> {
    let data = notification_data(notification_id, target, input);

    match target.platform {
        PushNotificationTargetPlatform::Android => {
            let mut notification = serde_json::json!({
                "title": push_text(&input.title, 256),
                "body": push_text(input.description.as_deref().unwrap_or_default(), 1024),
            });
            if let Some(image_url) = notification_image_url(input) {
                notification["image"] = serde_json::Value::String(image_url.to_string());
            }

            Ok(serde_json::to_string(&fit_push_payload(
                serde_json::json!({
                    "message": {
                        "notification": notification,
                        "data": data,
                    }
                }),
            )?)?)
        }
        PushNotificationTargetPlatform::Ios => {
            let apns = apns_payload(
                input,
                notification_string_data(notification_id, target, input),
            )?;
            Ok(serde_json::to_string(&apns)?)
        }
        PushNotificationTargetPlatform::Desktop => Err(flow_like_types::anyhow!(
            "Azure Notification Hubs desktop push targets are not supported"
        )),
    }
}

#[cfg(feature = "azure")]
fn azure_message_format(
    platform: &PushNotificationTargetPlatform,
) -> flow_like_types::Result<&'static str> {
    match platform {
        PushNotificationTargetPlatform::Android => Ok("fcmv1"),
        PushNotificationTargetPlatform::Ios => Ok("apple"),
        PushNotificationTargetPlatform::Desktop => Err(flow_like_types::anyhow!(
            "Azure Notification Hubs desktop push targets are not supported"
        )),
    }
}

#[cfg(feature = "azure")]
fn azure_platform_name(
    platform: &PushNotificationTargetPlatform,
) -> flow_like_types::Result<&'static str> {
    match platform {
        PushNotificationTargetPlatform::Android => Ok("fcmv1"),
        PushNotificationTargetPlatform::Ios => Ok("apns"),
        PushNotificationTargetPlatform::Desktop => Err(flow_like_types::anyhow!(
            "Azure Notification Hubs desktop push targets are not supported"
        )),
    }
}

#[cfg(feature = "azure")]
fn azure_hub_resource_uri(
    azure: &flow_like::hub::AzureNotificationHubsPushNotificationsConfig,
) -> String {
    format!(
        "https://{}.servicebus.windows.net/{}",
        azure.namespace, azure.hub_name
    )
}

#[cfg(feature = "azure")]
fn azure_sas_token(
    resource_uri: &str,
    key_name: &str,
    key_value: &str,
) -> flow_like_types::Result<String> {
    type HmacSha256 = Hmac<Sha256>;

    let expiry = (chrono::Utc::now().timestamp() + 3600).to_string();
    let encoded_uri = urlencoding::encode(resource_uri).into_owned();
    let to_sign = format!("{}\n{}", encoded_uri, expiry);
    let key_bytes = base64::engine::general_purpose::STANDARD.decode(key_value)?;
    let mut mac = HmacSha256::new_from_slice(&key_bytes)?;
    mac.update(to_sign.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());

    Ok(format!(
        "SharedAccessSignature sr={}&sig={}&se={}&skn={}",
        encoded_uri,
        urlencoding::encode(&signature),
        expiry,
        urlencoding::encode(key_name),
    ))
}

#[cfg(feature = "azure")]
fn sanitize_installation_id(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

async fn fetch_google_access_token(service_account_json: &str) -> flow_like_types::Result<String> {
    // Check cache first
    let cache = google_token_cache();
    {
        let guard = cache.read().await;
        if let Some(cached) = guard.as_ref()
            && Instant::now() < cached.expires_at
        {
            return Ok(cached.token.clone());
        }
    }

    // Cache miss or expired – fetch a new token
    let token =
        exchange_google_service_account(service_account_json, FIREBASE_MESSAGING_SCOPE).await?;

    // Cache the new token with a 55-minute TTL (5-min safety buffer on 60-min lifetime)
    {
        let mut guard = cache.write().await;
        *guard = Some(CachedAccessToken {
            token: token.clone(),
            expires_at: Instant::now() + std::time::Duration::from_secs(55 * 60),
        });
    }

    Ok(token)
}

/// Exchange a service-account key for an OAuth access token carrying `scope` (space separated
/// scopes allowed). Uncached: callers own their cache policy.
pub(crate) async fn exchange_google_service_account(
    service_account_json: &str,
    scope: &str,
) -> flow_like_types::Result<String> {
    let service_account = GoogleServiceAccount::parse(service_account_json)?;
    let now = chrono::Utc::now().timestamp() as usize;
    let aud = service_account
        .token_uri
        .clone()
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string());

    let claims = GoogleClaims {
        iss: &service_account.client_email,
        scope,
        aud: &aud,
        iat: now,
        exp: now + 3600,
    };

    let jwt = encode(
        &Header::new(Algorithm::RS256),
        &claims,
        &EncodingKey::from_rsa_pem(service_account.private_key.as_bytes())?,
    )?;

    let response = shared_http_client()
        .post(&aud)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", jwt.as_str()),
        ])
        .send()
        .await?;

    if response.status() != StatusCode::OK {
        let text = response.text().await.unwrap_or_default();
        return Err(flow_like_types::anyhow!(
            "Google OAuth token exchange failed: {}",
            text
        ));
    }

    let token: GoogleTokenResponse = response.json().await?;
    Ok(token.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_config() -> PushNotificationsConfig {
        PushNotificationsConfig {
            channel_id: Some("default-channel".to_string()),
            ..Default::default()
        }
    }

    fn target(platform: PushNotificationTargetPlatform) -> push_notification_target::Model {
        let now = chrono::Utc::now().fixed_offset();

        push_notification_target::Model {
            id: "target-id".to_string(),
            user_id: "user-id".to_string(),
            device_id: "device-id".to_string(),
            platform,
            provider: PushNotificationTargetProvider::Fcm,
            token_encrypted: "encrypted-token".to_string(),
            endpoint_arn: None,
            installation_id: None,
            channel_id: Some("target-channel".to_string()),
            device_name: None,
            metadata: None,
            push_enabled: true,
            failure_count: 0,
            last_registered_at: now,
            last_seen_at: now,
            invalidated_at: None,
            invalidation_reason: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn notification_input() -> DispatchNotificationInput {
        DispatchNotificationInput {
            user_id: "user-id".to_string(),
            app_id: Some("app-id".to_string()),
            title: "Build finished".to_string(),
            description: Some("The workflow completed.".to_string()),
            icon: None,
            image: Some("https://cdn.example.com/image.png".to_string()),
            link: Some("flow-like://notification/target-id".to_string()),
            notification_type: NotificationType::Workflow,
            source_run_id: Some("run-id".to_string()),
            source_node_id: None,
        }
    }

    fn message_object(body: &serde_json::Value) -> &serde_json::Map<String, serde_json::Value> {
        body.get("message")
            .and_then(serde_json::Value::as_object)
            .expect("FCM body contains a message object")
    }

    #[test]
    fn fcm_android_body_includes_android_notification_options() {
        let body = fcm_message_body(
            &push_config(),
            &target(PushNotificationTargetPlatform::Android),
            "fcm-token",
            "notification-id",
            &notification_input(),
        )
        .unwrap();

        let message = message_object(&body);
        let android = message.get("android").expect("android options are present");
        assert_eq!(message.get("token"), Some(&serde_json::json!("fcm-token")));
        assert_eq!(android["notification"]["channel_id"], "target-channel");
        assert_eq!(
            android["notification"]["image"],
            "https://cdn.example.com/image.png"
        );
    }

    #[test]
    fn fcm_ios_body_omits_android_options() {
        let body = fcm_message_body(
            &push_config(),
            &target(PushNotificationTargetPlatform::Ios),
            "fcm-token",
            "notification-id",
            &notification_input(),
        )
        .unwrap();

        let message = message_object(&body);
        assert!(!message.contains_key("android"));
        assert!(message.contains_key("apns"));
        assert!(!message.contains_key("data"));
        assert_eq!(
            message["apns"]["payload"]["link"],
            "flow-like://notification/target-id"
        );
        assert_eq!(message["apns"]["payload"]["app_id"], "app-id");
    }

    #[test]
    fn fcm_desktop_body_omits_mobile_platform_options() {
        let body = fcm_message_body(
            &push_config(),
            &target(PushNotificationTargetPlatform::Desktop),
            "fcm-token",
            "notification-id",
            &notification_input(),
        )
        .unwrap();

        let message = message_object(&body);
        assert!(!message.contains_key("android"));
        assert!(!message.contains_key("apns"));
    }

    #[test]
    fn invalid_argument_payload_errors_do_not_disable_devices() {
        let response = serde_json::json!({"error": {
            "message": "Android message is too big",
            "details": [{"errorCode": "INVALID_ARGUMENT"}],
        }})
        .to_string();
        let FcmOutcome::Permanent(reason) = classify_fcm_error(400, response) else {
            panic!("a payload error must not invalidate a registration");
        };
        assert!(!should_invalidate_target(&reason));
        assert!(matches!(
            classify_fcm_error(
                404,
                r#"{"error":{"details":[{"errorCode":"UNREGISTERED"}]}}"#.to_string()
            ),
            FcmOutcome::InvalidateTarget(_)
        ));
        assert!(should_invalidate_target(
            "[invalid-push-target] FCM error UNREGISTERED"
        ));
    }

    #[test]
    fn image_urls_must_be_public_https_addresses() {
        for rejected in [
            "data:image/png;base64,aaaa",
            "asset://localhost/tmp/test.png",
            "http://asset.localhost/test.png",
            "https://localhost/test.png",
            "https://asset.localhost/test.png",
            "https://device.local/test.png",
            "https://127.0.0.1/test.png",
            "https://10.0.0.1/test.png",
            "https://[::1]/test.png",
            "https://[::ffff:127.0.0.1]/test.png",
            "https://name:password@example.com/test.png",
        ] {
            assert!(!is_push_image_url(rejected), "must reject {rejected}");
        }
        assert!(is_push_image_url(
            "https://cdn.example.com/image.png?signature=abc"
        ));
        assert!(!is_push_image_url(&format!(
            "https://cdn.example.com/{}.png",
            "a".repeat(2048)
        )));
    }

    #[test]
    fn inline_images_never_enter_push_payloads() {
        let mut input = notification_input();
        input.image = None;
        input.icon = Some(format!("data:image/png;base64,{}", "a".repeat(50_000)));
        let target = target(PushNotificationTargetPlatform::Ios);
        let body = fcm_message_body(&push_config(), &target, "token", "id", &input).unwrap();
        let encoded = serde_json::to_string(&body).unwrap();
        assert!(!encoded.contains("base64"));
        assert!(body["message"]["data"].get("icon").is_none());
        assert!(body["message"]["apns"].get("fcm_options").is_none());
    }

    #[test]
    fn fcm_ios_image_uses_one_media_url_and_mutable_content() {
        let mut input = notification_input();
        input.icon = input.image.take();
        let body = fcm_message_body(
            &push_config(),
            &target(PushNotificationTargetPlatform::Ios),
            "token",
            "id",
            &input,
        )
        .unwrap();
        assert_eq!(
            body["message"]["apns"]["payload"]["aps"]["mutable-content"],
            1
        );
        assert_eq!(
            body["message"]["apns"]["fcm_options"]["image"],
            "https://cdn.example.com/image.png"
        );
        assert!(body["message"]["notification"].get("image").is_none());
        assert_eq!(
            serde_json::to_string(&body)
                .unwrap()
                .matches("https://cdn.example.com/image.png")
                .count(),
            1
        );
    }

    #[test]
    fn large_optional_content_cannot_exceed_push_limit() {
        let mut input = notification_input();
        input.title = "é".repeat(1000);
        input.description = Some("a".repeat(10_000));
        input.image = Some(format!("https://cdn.example.com/{}.png", "a".repeat(1800)));
        input.link = Some(format!("/{}", "a".repeat(1000)));
        for platform in [
            PushNotificationTargetPlatform::Ios,
            PushNotificationTargetPlatform::Android,
        ] {
            let body =
                fcm_message_body(&push_config(), &target(platform), "token", "id", &input).unwrap();
            assert!(serde_json::to_vec(&body).unwrap().len() <= MAX_PUSH_PAYLOAD_BYTES);
            assert!(
                !body["message"]["notification"]["title"]
                    .as_str()
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(
            fcm_message_body(
                &push_config(),
                &target(PushNotificationTargetPlatform::Ios),
                &"a".repeat(5000),
                "id",
                &input
            )
            .is_err()
        );
    }

    #[test]
    fn long_routes_are_preserved_or_rejected_explicitly() {
        let mut input = notification_input();
        input.description = Some("preview ".repeat(500));
        input.link = Some(format!(
            "/use?id=app-id&route=/config&value={}",
            "a".repeat(2600)
        ));
        for platform in [
            PushNotificationTargetPlatform::Ios,
            PushNotificationTargetPlatform::Android,
        ] {
            let target = target(platform.clone());
            let body = fcm_message_body(&push_config(), &target, "token", "id", &input).unwrap();
            let data = if platform == PushNotificationTargetPlatform::Ios {
                &body["message"]["apns"]["payload"]
            } else {
                &body["message"]["data"]
            };
            assert_eq!(data["link"].as_str(), input.link.as_deref());
            assert_eq!(data["app_id"], "app-id");
            assert_eq!(data["source_run_id"], "run-id");
            assert!(serde_json::to_vec(&body).unwrap().len() <= MAX_PUSH_PAYLOAD_BYTES);
        }
        input.link = Some(format!("/use?value={}", "a".repeat(5000)));
        assert!(
            fcm_message_body(
                &push_config(),
                &target(PushNotificationTargetPlatform::Ios),
                "token",
                "id",
                &input
            )
            .is_err()
        );
        assert!(
            apns_payload(
                &input,
                notification_string_data(
                    "id",
                    &target(PushNotificationTargetPlatform::Ios),
                    &input
                )
            )
            .is_err()
        );
    }

    #[test]
    fn apns_payload_includes_image_for_native_providers() {
        let input = notification_input();
        let payload = apns_payload(&input, HashMap::new()).unwrap();
        assert_eq!(payload["aps"]["mutable-content"], 1);
        assert_eq!(payload["image"], "https://cdn.example.com/image.png");
        assert!(serde_json::to_vec(&payload).unwrap().len() <= MAX_PUSH_PAYLOAD_BYTES);
    }

    #[cfg(feature = "aws")]
    #[test]
    fn sns_apple_payloads_include_mutable_image_attachment() {
        let input = notification_input();
        let target = target(PushNotificationTargetPlatform::Ios);
        let payload: serde_json::Value = serde_json::from_str(
            &aws_sns_payload(
                &target,
                &input,
                notification_string_data("id", &target, &input),
            )
            .unwrap(),
        )
        .unwrap();
        for platform in ["APNS", "APNS_SANDBOX"] {
            let apns: serde_json::Value =
                serde_json::from_str(payload[platform].as_str().unwrap()).unwrap();
            assert_eq!(apns["aps"]["mutable-content"], 1);
            assert_eq!(apns["image"], "https://cdn.example.com/image.png");
        }
    }

    #[cfg(feature = "azure")]
    #[test]
    fn azure_apple_payload_includes_mutable_image_attachment() {
        let payload: serde_json::Value = serde_json::from_str(
            &azure_message_payload(
                &target(PushNotificationTargetPlatform::Ios),
                "id",
                &notification_input(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["aps"]["mutable-content"], 1);
        assert_eq!(payload["image"], "https://cdn.example.com/image.png");
    }

    #[test]
    fn delivery_summary_distinguishes_partial_failure_and_no_targets() {
        assert_eq!(
            PushDispatchStatus::from_counts(0, 0),
            PushDispatchStatus::NoTargets
        );
        assert_eq!(
            PushDispatchStatus::from_counts(2, 0),
            PushDispatchStatus::Accepted
        );
        assert_eq!(
            PushDispatchStatus::from_counts(0, 2),
            PushDispatchStatus::Failed
        );
        assert_eq!(
            PushDispatchStatus::from_counts(1, 1),
            PushDispatchStatus::Partial
        );
        assert!(!PushDispatchStatus::Partial.is_success());
        assert!(!PushDispatchStatus::Failed.is_success());
        assert!(!PushDispatchStatus::Deduplicated.is_success());
        assert!(PushDispatchStatus::Disabled.is_success());
        assert!(PushDispatchStatus::NoTargets.is_success());
    }
}
