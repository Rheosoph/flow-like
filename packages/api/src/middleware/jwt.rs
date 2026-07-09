use std::sync::Arc;

use crate::{
    entity::{
        app_connection, membership, pat, prelude::*, role, sea_orm_active_enums, technical_user,
        user,
    },
    error::{ApiError, AuthorizationError},
    permission::{
        global_permission::GlobalPermission,
        role_permission::{RolePermissions, has_role_permission},
    },
};
use axum::{
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use flow_like::hub::UserTier;
use flow_like_types::Result;
use flow_like_types::anyhow;
use hyper::header::AUTHORIZATION;
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};
use serde::de::{self, Unexpected};
use serde::{Deserialize, Deserializer};

use crate::state::{AppState, CachedAuth};

/// Client IP address extracted from the request for audit trail purposes.
/// Checks X-Forwarded-For, X-Real-Ip, then falls back to the peer address.
#[derive(Debug, Clone)]
pub struct ClientIp(pub Option<String>);

fn extract_client_ip(request: &Request) -> Option<String> {
    if let Some(forwarded) = request.headers().get("x-forwarded-for")
        && let Ok(val) = forwarded.to_str()
    {
        // X-Forwarded-For can contain multiple IPs; the first is the original client
        return val.split(',').next().map(|ip| ip.trim().to_string());
    }
    if let Some(real_ip) = request.headers().get("x-real-ip")
        && let Ok(val) = real_ip.to_str()
    {
        return Some(val.trim().to_string());
    }
    None
}

fn pat_id_from_token(pat_str: &str) -> Result<String> {
    if !pat_str.starts_with("pat_") {
        return Err(anyhow!("Not a PAT"));
    }
    let pat_parts = &pat_str[4..];
    let parts: Vec<&str> = pat_parts.split('.').collect();
    if parts.len() != 2 {
        return Err(anyhow!("Invalid PAT format"));
    }
    Ok(parts[0].to_string())
}

fn deserialize_opt_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = opt else {
        return Ok(None);
    };
    match value {
        serde_json::Value::Bool(b) => Ok(Some(b)),
        serde_json::Value::String(s) => {
            let sl = s.to_ascii_lowercase();
            match sl.as_str() {
                "true" => Ok(Some(true)),
                "false" => Ok(Some(false)),
                other => Err(de::Error::invalid_value(
                    Unexpected::Str(other),
                    &"true or false",
                )),
            }
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                match i {
                    0 => Ok(Some(false)),
                    1 => Ok(Some(true)),
                    other => Err(de::Error::invalid_value(
                        Unexpected::Signed(other),
                        &"0 or 1 for boolean",
                    )),
                }
            } else {
                Err(de::Error::custom("invalid numeric value for boolean"))
            }
        }
        other => Err(de::Error::custom(format!(
            "invalid type for boolean field: expected bool | 'true' | 'false' | 0 | 1, got {}",
            other
        ))),
    }
}

#[derive(Debug, Deserialize)]
pub struct UserInfo {
    pub sub: String,

    // Standard OIDC claims (all optional; presence depends on granted scopes & attributes)
    pub email: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_bool")]
    pub email_verified: Option<bool>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub middle_name: Option<String>,
    pub preferred_username: Option<String>,
    pub phone_number: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_bool")]
    pub phone_number_verified: Option<bool>,
    pub picture: Option<String>,
    pub birthdate: Option<String>,
    pub updated_at: Option<u64>,

    pub username: Option<String>,

    #[serde(flatten)]
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct OpenIDUser {
    pub sub: String,
    pub access_token: String,
}

#[derive(Debug, Clone)]
pub struct PATUser {
    pub pat: String,
    pub sub: String,
}

#[derive(Debug, Clone)]
pub struct ApiKey {
    pub key_id: String,
    pub api_key: String,
    pub app_id: String,
    pub creator_user_id: Option<String>,
}

impl ApiKey {
    pub fn masked_key(&self) -> String {
        if self.api_key.len() <= 4 {
            "****".to_string()
        } else {
            let visible_part = &self.api_key[self.api_key.len() - 4..];
            format!("****{}", visible_part)
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExecutorUser {
    pub sub: String,
    pub app_id: String,
    pub run_id: String,
    pub technical_user_id: Option<String>,
    /// If the run was triggered through an app connection: the chain of apps
    /// that led to it (last element = the app that called this one). Threaded
    /// into outgoing app-connection tokens so chains stay transparent.
    pub app_chain: Option<Vec<String>>,
    /// Process-mining correlation carried by the run, threaded into outgoing
    /// app-connection tokens so downstream runs inherit the case.
    pub correlation: Option<crate::correlation::CorrelationContext>,
}

/// Principal for app-to-app calls: a flow in `origin_app_id` acting on
/// `target_app_id` through an accepted app connection. The token pins both
/// app ids; permissions come from the role assigned to the connection.
#[derive(Debug, Clone)]
pub struct ConnectedAppUser {
    /// The user that originally initiated the call chain, passed through
    /// end-to-end even if they are not a member of the target app.
    pub sub: Option<String>,
    pub origin_app_id: String,
    pub target_app_id: String,
    /// Full chain of apps that led to this call (last element = origin).
    pub app_chain: Vec<String>,
    pub technical_user_id: Option<String>,
    pub run_id: Option<String>,
    /// Process-mining correlation inherited from the calling run (trace root +
    /// business keys), so this run joins the same case.
    pub correlation: Option<crate::correlation::CorrelationContext>,
}

#[derive(Debug, Clone)]
pub enum AppUser {
    OpenID(OpenIDUser),
    PAT(PATUser),
    APIKey(ApiKey),
    Executor(ExecutorUser),
    ConnectedApp(ConnectedAppUser),
    Unauthorized,
}

pub struct AppPermissionResponse {
    pub state: AppState,
    pub permissions: RolePermissions,
    pub role: Arc<role::Model>,
    pub sub: Option<String>,
    pub effective_user_id: Option<String>,
    pub technical_user_id: Option<String>,
    pub identifier: String,
}

impl AppPermissionResponse {
    pub fn has_permission(&self, permission: RolePermissions) -> bool {
        has_role_permission(&self.permissions, permission)
    }

    pub fn sub(&self) -> Result<String> {
        self.sub.clone().ok_or_else(|| anyhow!("No sub available"))
    }

    pub fn effective_user_id(&self) -> Result<String> {
        self.effective_user_id
            .clone()
            .ok_or_else(|| anyhow!("No effective user available"))
    }

    pub fn technical_user_id(&self) -> Option<&str> {
        self.technical_user_id.as_deref()
    }

    /// Either returns the sub if available or in case of API keys it returns the key ID.
    /// This is useful for identifying the user in logs or other contexts where a unique identifier is needed.
    pub fn identifier(&self) -> String {
        self.identifier.clone()
    }

    /// Convert to UserExecutionContext for execution
    pub fn to_user_context(&self) -> flow_like::flow::execution::UserExecutionContext {
        use flow_like::flow::execution::{RoleContext, UserExecutionContext};

        let role_context = RoleContext {
            id: self.role.id.clone(),
            name: self.role.name.clone(),
            permissions: self.role.permissions,
            attributes: self.role.attributes.clone().unwrap_or_default(),
            custom_attributes: std::collections::HashMap::new(),
        };

        // Check if this is a technical user (API key) - sub is None for API keys
        let is_technical_user = self.sub.is_none();

        if is_technical_user {
            // For API keys, use the technical user constructor with key_id
            UserExecutionContext::technical(
                self.identifier.clone(),
                self.role.id.clone(),
                self.role.name.clone(),
                self.role.permissions,
                self.role.attributes.clone().unwrap_or_default(),
                std::collections::HashMap::new(),
            )
        } else {
            let sub = self.sub.clone().unwrap_or_default();
            UserExecutionContext::new(sub).with_role(role_context)
        }
    }
}

impl AppUser {
    pub fn sub(&self) -> Result<String, AuthorizationError> {
        match self {
            AppUser::OpenID(user) => Ok(user.sub.clone()),
            AppUser::PAT(user) => Ok(user.sub.clone()),
            AppUser::Executor(_) => Err(ApiError::forbidden(
                "Executor user is not allowed on this endpoint",
            )),
            AppUser::APIKey(_) => Err(ApiError::forbidden(
                "APIKey user is not allowed on this endpoint",
            )),
            AppUser::ConnectedApp(_) => Err(ApiError::forbidden(
                "Connected app is not allowed on this endpoint",
            )),
            AppUser::Unauthorized => Err(ApiError::UNAUTHORIZED),
        }
    }

    /// Like `sub()` but also accepts Executor JWTs.
    /// Only call this on endpoints that explicitly opt into executor auth.
    pub fn executor_scoped_sub(&self) -> Result<String, AuthorizationError> {
        match self {
            AppUser::OpenID(user) => Ok(user.sub.clone()),
            AppUser::PAT(user) => Ok(user.sub.clone()),
            AppUser::Executor(user) => Ok(user.sub.clone()),
            AppUser::APIKey(_) => Err(ApiError::forbidden(
                "APIKey user is not allowed on this endpoint",
            )),
            AppUser::ConnectedApp(_) => Err(ApiError::forbidden(
                "Connected app is not allowed on this endpoint",
            )),
            AppUser::Unauthorized => Err(ApiError::UNAUTHORIZED),
        }
    }

    pub fn entity(&self) -> Result<AppUser, AuthorizationError> {
        match self {
            AppUser::OpenID(user) => Ok(AppUser::OpenID(user.clone())),
            AppUser::PAT(user) => Ok(AppUser::PAT(user.clone())),
            AppUser::APIKey(api_key) => Ok(AppUser::APIKey(api_key.clone())),
            AppUser::Executor(executor) => Ok(AppUser::Executor(executor.clone())),
            AppUser::ConnectedApp(app) => Ok(AppUser::ConnectedApp(app.clone())),
            AppUser::Unauthorized => Err(ApiError::UNAUTHORIZED),
        }
    }

    pub fn app_id(&self) -> Result<String, AuthorizationError> {
        match self {
            AppUser::Executor(user) => Ok(user.app_id.clone()),
            AppUser::APIKey(api_key) => Ok(api_key.app_id.clone()),
            _ => Err(ApiError::forbidden(
                "Only Executor and APIKey users have an app_id in this context",
            )),
        }
    }

    pub fn technical_user_id(&self) -> Option<&str> {
        match self {
            AppUser::APIKey(api_key) => Some(api_key.key_id.as_str()),
            AppUser::Executor(executor) => executor.technical_user_id.as_deref(),
            AppUser::ConnectedApp(app) => app.technical_user_id.as_deref(),
            _ => None,
        }
    }

    pub fn effective_user_id(&self) -> Result<String, AuthorizationError> {
        match self {
            AppUser::OpenID(user) => Ok(user.sub.clone()),
            AppUser::PAT(user) => Ok(user.sub.clone()),
            AppUser::Executor(user) => Ok(user.sub.clone()),
            AppUser::APIKey(api_key) => api_key
                .creator_user_id
                .clone()
                .ok_or_else(|| ApiError::forbidden("API key is not linked to a creator user")),
            // The passed-through sub is attribution metadata, not an identity
            // this token can act as. Endpoints that authorize purely by the
            // effective user id must not accept app-connection tokens; the
            // sub is exposed via AppPermissionResponse after the connection
            // role has been checked.
            AppUser::ConnectedApp(_) => Err(ApiError::forbidden(
                "App connection tokens cannot act as the initiating user",
            )),
            AppUser::Unauthorized => Err(ApiError::UNAUTHORIZED),
        }
    }

    // Adds the exact method of access (OpenID, PAT, API Key) to the audit log for better traceability
    pub async fn audit_id(&self) -> Result<String, AuthorizationError> {
        let sub = match self {
            AppUser::ConnectedApp(app) => app
                .sub
                .clone()
                .unwrap_or_else(|| app_connection_cache_sub(&app.origin_app_id)),
            _ => self.effective_user_id()?,
        };
        let method = match self {
            AppUser::OpenID(_) => "openid",
            AppUser::PAT(_) => "pat",
            AppUser::APIKey(_) => "api_key",
            AppUser::Executor(_) => "executor",
            AppUser::ConnectedApp(_) => "app_connection",
            AppUser::Unauthorized => "unauthorized",
        };
        let method_id = match self {
            AppUser::OpenID(_user) => None,
            AppUser::PAT(user) => Some(pat_id_from_token(&user.pat)?),
            AppUser::APIKey(api_key) => Some(api_key.key_id.clone()),
            AppUser::Executor(executor) => Some(executor.run_id.clone()),
            AppUser::ConnectedApp(app) => Some(app.origin_app_id.clone()),
            AppUser::Unauthorized => None,
        };
        Ok(format!(
            "{}:{}:{}",
            method,
            sub,
            method_id.unwrap_or_default()
        ))
    }

    pub async fn tracking_id(
        &self,
        state: &AppState,
    ) -> Result<Option<String>, AuthorizationError> {
        let sub = self.effective_user_id()?;
        let user = user::Entity::find_by_id(&sub)
            .one(&state.db)
            .await?
            .ok_or_else(|| AuthorizationError::from(anyhow!("User not found")))?;
        Ok(user.tracking_id)
    }

    pub async fn tier(&self, state: &AppState) -> Result<UserTier, AuthorizationError> {
        let sub = self.effective_user_id()?;
        let user = user::Entity::find_by_id(&sub)
            .one(&state.db)
            .await?
            .ok_or_else(|| AuthorizationError::from(anyhow!("User not found")))?;

        let db_tier = match user.tier {
            sea_orm_active_enums::UserTier::Free => "FREE",
            sea_orm_active_enums::UserTier::Premium => "PREMIUM",
            sea_orm_active_enums::UserTier::Pro => "PRO",
            sea_orm_active_enums::UserTier::Enterprise => "ENTERPRISE",
        };

        let tier = state
            .platform_config
            .tiers
            .get(db_tier)
            .cloned()
            .ok_or_else(|| AuthorizationError::from(anyhow!("Tier not found")))?;
        Ok(tier)
    }

    pub async fn get_user(&self, state: &AppState) -> Result<user::Model, AuthorizationError> {
        let sub = self.sub()?;
        user::Entity::find_by_id(&sub)
            .one(&state.db)
            .await?
            .ok_or_else(|| AuthorizationError::from(anyhow!("User not found")))
    }

    pub async fn user_info(&self, state: &AppState) -> flow_like_types::Result<UserInfo> {
        let user = match self {
            AppUser::OpenID(user) => user,
            AppUser::PAT(_) => return Err(anyhow!("PAT user does not have user info")),
            AppUser::APIKey(_) => return Err(anyhow!("APIKey user does not have user info")),
            AppUser::Executor(_) => return Err(anyhow!("Executor user does not have user info")),
            AppUser::ConnectedApp(_) => {
                return Err(anyhow!("Connected app does not have user info"));
            }
            AppUser::Unauthorized => {
                return Err(anyhow!("Unauthorized user does not have user info"));
            }
        };

        let endpoint: &str = state
            .platform_config
            .authentication
            .as_ref()
            .and_then(|c| c.openid.as_ref())
            .and_then(|o| o.user_info_url.as_deref())
            .ok_or_else(|| anyhow!("User info URL not configured"))?;

        let client = flow_like_types::reqwest::Client::new();
        let res = match client
            .get(endpoint)
            .bearer_auth(&user.access_token)
            .send()
            .await
        {
            Ok(res) => res,
            Err(err) => {
                tracing::error!("Failed to fetch user info from {}: {}", endpoint, err);
                return Err(anyhow!("Failed to fetch user info"));
            }
        };

        match res.status() {
            flow_like_types::reqwest::StatusCode::OK => Ok(res.json::<UserInfo>().await?),
            status => {
                let body = res.text().await.unwrap_or_default();
                flow_like_types::bail!("UserInfo error {}: {}", status, body)
            }
        }
    }

    pub async fn global_permission(&self, state: AppState) -> Result<GlobalPermission, ApiError> {
        let sub = self.sub()?;
        let user = user::Entity::find_by_id(&sub)
            .one(&state.db)
            .await?
            .ok_or_else(|| anyhow!("User not found"))?;
        let permission = GlobalPermission::from_bits(user.permission)
            .ok_or_else(|| anyhow!("Invalid permission bits"))?;
        Ok(permission)
    }

    pub async fn check_global_permission(
        &self,
        state: &AppState,
        permission: GlobalPermission,
    ) -> Result<GlobalPermission, ApiError> {
        let global_permission = self.global_permission(state.clone()).await?;
        let has_permission = global_permission.contains(permission)
            || global_permission.contains(GlobalPermission::Admin);
        if has_permission {
            Ok(global_permission)
        } else {
            Err(ApiError::FORBIDDEN)
        }
    }

    pub async fn app_permission(
        &self,
        app_id: &str,
        state: &AppState,
    ) -> Result<AppPermissionResponse, ApiError> {
        let sub = self.sub();
        if let Ok(sub) = sub {
            let cached_permission = state.check_permission(&sub, app_id);

            if let Some(role_model) = cached_permission {
                let permissions = RolePermissions::from_bits(role_model.permissions)
                    .ok_or_else(|| anyhow!("Invalid role permission bits"))?;
                return Ok(AppPermissionResponse {
                    state: state.clone(),
                    permissions,
                    role: role_model.clone(),
                    sub: Some(sub.clone()),
                    effective_user_id: Some(sub.clone()),
                    technical_user_id: None,
                    identifier: sub,
                });
            }

            let role_model = role::Entity::find()
                .join(JoinType::InnerJoin, role::Relation::Membership.def())
                .filter(
                    membership::Column::UserId
                        .eq(&sub)
                        .and(membership::Column::AppId.eq(app_id)),
                )
                .one(&state.db)
                .await?
                .ok_or_else(|| {
                    tracing::debug!("Role not found for user {} in app {}", sub, app_id);
                    ApiError::FORBIDDEN
                })?;

            let permissions = RolePermissions::from_bits(role_model.permissions)
                .ok_or_else(|| anyhow!("Invalid role permission bits"))?;

            state.put_permission(&sub, app_id, Arc::new(role_model.clone()));

            return Ok(AppPermissionResponse {
                state: state.clone(),
                permissions,
                role: Arc::new(role_model),
                sub: Some(sub.clone()),
                effective_user_id: Some(sub.clone()),
                technical_user_id: None,
                identifier: sub,
            });
        }

        if let AppUser::ConnectedApp(connected_app) = self {
            return connected_app_permission(connected_app, app_id, state).await;
        }

        if let AppUser::APIKey(api_key) = self {
            if api_key.app_id != app_id {
                return Err(ApiError::FORBIDDEN);
            }

            let role_model = role::Entity::find()
                .join(JoinType::InnerJoin, role::Relation::TechnicalUser.def())
                .filter(
                    technical_user::Column::AppId
                        .eq(&api_key.app_id)
                        .and(technical_user::Column::Id.eq(&api_key.key_id)),
                )
                .one(&state.db)
                .await?
                .ok_or_else(|| ApiError::from(anyhow!("Technical user not found for API Key")))?;

            let permissions = RolePermissions::from_bits(role_model.permissions)
                .ok_or_else(|| anyhow!("Invalid role permission bits"))?;
            let effective_user_id = match api_key.creator_user_id.clone() {
                Some(creator_user_id) => Some(creator_user_id),
                None => resolve_legacy_api_key_creator_user_id(state, &api_key.app_id).await?,
            };

            return Ok(AppPermissionResponse {
                state: state.clone(),
                permissions,
                role: Arc::new(role_model),
                sub: None,
                effective_user_id,
                technical_user_id: Some(api_key.key_id.clone()),
                identifier: api_key.key_id.clone(),
            });
        }

        Err(ApiError::from(anyhow!(
            "User does not have app permissions"
        )))
    }

    pub async fn execution_app_permission(
        &self,
        app_id: &str,
        state: &AppState,
    ) -> Result<AppPermissionResponse, ApiError> {
        if let AppUser::Executor(executor) = self {
            if executor.app_id != app_id {
                tracing::warn!(
                    token_app_id = %executor.app_id,
                    path_app_id = %app_id,
                    run_id = %executor.run_id,
                    "Executor token app_id does not match request path"
                );
                return Err(ApiError::FORBIDDEN);
            }

            // Runs initiated through an app connection are bounded by the
            // connection role, not by the passed-through user's membership in
            // this app: the user may not be a member at all, and even if they
            // are, the calling app must not exceed the role it was granted.
            // This also keeps multi-hop chains (A -> B -> C) working, since
            // B's callbacks and token minting resolve against the A -> B
            // connection instead of the original user's membership in B.
            if let Some(app_chain) = &executor.app_chain
                && let Some(calling_app_id) = app_chain.last()
            {
                let connected = ConnectedAppUser {
                    sub: Some(executor.sub.clone()),
                    origin_app_id: calling_app_id.clone(),
                    target_app_id: app_id.to_string(),
                    app_chain: app_chain.clone(),
                    technical_user_id: executor.technical_user_id.clone(),
                    run_id: Some(executor.run_id.clone()),
                    correlation: executor.correlation.clone(),
                };
                return connected_app_permission(&connected, app_id, state).await;
            }

            if let Some(role_model) = state.check_permission(&executor.sub, app_id) {
                let permissions = RolePermissions::from_bits(role_model.permissions)
                    .ok_or_else(|| anyhow!("Invalid role permission bits"))?;
                return Ok(AppPermissionResponse {
                    state: state.clone(),
                    permissions,
                    role: role_model.clone(),
                    sub: Some(executor.sub.clone()),
                    effective_user_id: Some(executor.sub.clone()),
                    technical_user_id: executor.technical_user_id.clone(),
                    identifier: executor.sub.clone(),
                });
            }

            let role_model = role::Entity::find()
                .join(JoinType::InnerJoin, role::Relation::Membership.def())
                .filter(
                    membership::Column::UserId
                        .eq(&executor.sub)
                        .and(membership::Column::AppId.eq(app_id)),
                )
                .one(&state.db)
                .await?
                .ok_or_else(|| {
                    tracing::debug!(
                        user_id = %executor.sub,
                        app_id = %app_id,
                        run_id = %executor.run_id,
                        "Role not found for executor user in app"
                    );
                    ApiError::FORBIDDEN
                })?;

            let permissions = RolePermissions::from_bits(role_model.permissions)
                .ok_or_else(|| anyhow!("Invalid role permission bits"))?;

            state.put_permission(&executor.sub, app_id, Arc::new(role_model.clone()));

            return Ok(AppPermissionResponse {
                state: state.clone(),
                permissions,
                role: Arc::new(role_model),
                sub: Some(executor.sub.clone()),
                effective_user_id: Some(executor.sub.clone()),
                technical_user_id: executor.technical_user_id.clone(),
                identifier: executor.sub.clone(),
            });
        }

        self.app_permission(app_id, state).await
    }
}

fn hash_token(token: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(token.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// Cache key used for app-connection permissions in the permission cache.
/// Namespaced so it can never collide with a real user sub.
pub fn app_connection_cache_sub(origin_app_id: &str) -> String {
    format!("app-connection::{}", origin_app_id)
}

async fn connected_app_permission(
    connected_app: &ConnectedAppUser,
    app_id: &str,
    state: &AppState,
) -> Result<AppPermissionResponse, ApiError> {
    if connected_app.target_app_id != app_id {
        tracing::warn!(
            token_target_app_id = %connected_app.target_app_id,
            path_app_id = %app_id,
            origin_app_id = %connected_app.origin_app_id,
            "App connection token target does not match request path"
        );
        return Err(ApiError::FORBIDDEN);
    }

    let cache_sub = app_connection_cache_sub(&connected_app.origin_app_id);

    let role_model = if let Some(role_model) = state.check_permission(&cache_sub, app_id) {
        role_model
    } else {
        let (connection, role) = app_connection::Entity::find()
            .filter(
                app_connection::Column::SourceAppId
                    .eq(&connected_app.origin_app_id)
                    .and(app_connection::Column::TargetAppId.eq(app_id))
                    .and(
                        app_connection::Column::Status
                            .eq(sea_orm_active_enums::AppConnectionStatus::Active),
                    ),
            )
            .find_also_related(role::Entity)
            .one(&state.db)
            .await?
            .ok_or_else(|| {
                tracing::debug!(
                    origin_app_id = %connected_app.origin_app_id,
                    target_app_id = %app_id,
                    "No active app connection found"
                );
                ApiError::FORBIDDEN
            })?;

        let role_model = role
            .filter(|role| role.app_id.as_deref() == Some(app_id))
            .ok_or_else(|| {
                tracing::warn!(
                    connection_id = %connection.id,
                    "App connection is active but has no valid role for this app"
                );
                ApiError::FORBIDDEN
            })?;

        let role_model = Arc::new(role_model);
        state.put_permission(&cache_sub, app_id, role_model.clone());
        role_model
    };

    let permissions = RolePermissions::from_bits(role_model.permissions)
        .ok_or_else(|| anyhow!("Invalid role permission bits"))?;

    Ok(AppPermissionResponse {
        state: state.clone(),
        permissions,
        role: role_model,
        sub: None,
        effective_user_id: connected_app.sub.clone(),
        technical_user_id: connected_app.technical_user_id.clone(),
        identifier: app_connection_cache_sub(&connected_app.origin_app_id),
    })
}

async fn resolve_legacy_api_key_creator_user_id(
    state: &AppState,
    app_id: &str,
) -> Result<Option<String>, AuthorizationError> {
    let app = App::find_by_id(app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;

    if let Some(owner_role_id) = app.owner_role_id {
        let owner = membership::Entity::find()
            .filter(
                membership::Column::AppId
                    .eq(app_id)
                    .and(membership::Column::RoleId.eq(owner_role_id)),
            )
            .order_by_asc(membership::Column::CreatedAt)
            .one(&state.db)
            .await?;

        if let Some(owner) = owner {
            return Ok(Some(owner.user_id));
        }
    }

    let members_with_roles = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id))
        .order_by_asc(membership::Column::CreatedAt)
        .find_also_related(role::Entity)
        .all(&state.db)
        .await?;

    for (member, role) in members_with_roles {
        if let Some(role) = role
            && let Some(permissions) = RolePermissions::from_bits(role.permissions)
            && permissions.contains(RolePermissions::Owner)
        {
            return Ok(Some(member.user_id));
        }
    }

    Ok(None)
}

pub async fn jwt_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response<Body>, AuthorizationError> {
    let mut request = request;

    let client_ip = ClientIp(extract_client_ip(&request));
    request.extensions_mut().insert(client_ip);

    // Try OpenID/JWT or Executor JWT auth
    if let Some(auth_header) = request.headers().get(AUTHORIZATION)
        && let Ok(token) = auth_header.to_str()
        && !token.starts_with("pat_")
    {
        let token = if token.starts_with("Bearer ") {
            &token[7..]
        } else {
            token
        };
        let token = token.trim();
        let cache_key = hash_token(token);

        // Check cache first
        if let Some(cached) = state.auth_cache.get(&cache_key) {
            match cached {
                CachedAuth::OpenID { sub } => {
                    let user = AppUser::OpenID(OpenIDUser {
                        sub,
                        access_token: token.to_string(),
                    });
                    request.extensions_mut().insert::<AppUser>(user);
                    return Ok(next.run(request).await);
                }
                CachedAuth::Executor {
                    sub,
                    app_id,
                    run_id,
                    technical_user_id,
                    app_chain,
                    correlation,
                } => {
                    let user = AppUser::Executor(ExecutorUser {
                        sub,
                        app_id,
                        run_id,
                        technical_user_id,
                        app_chain,
                        correlation,
                    });
                    request.extensions_mut().insert::<AppUser>(user);
                    return Ok(next.run(request).await);
                }
                CachedAuth::AppConnection {
                    sub,
                    origin_app_id,
                    target_app_id,
                    app_chain,
                    technical_user_id,
                    run_id,
                    correlation,
                    exp,
                } => {
                    // App-connection tokens are short-lived; never honor a
                    // cached entry beyond the token's own expiry.
                    if exp <= chrono::Utc::now().timestamp() {
                        request
                            .extensions_mut()
                            .insert::<AppUser>(AppUser::Unauthorized);
                        return Ok(next.run(request).await);
                    }
                    let user = AppUser::ConnectedApp(ConnectedAppUser {
                        sub,
                        origin_app_id,
                        target_app_id,
                        app_chain,
                        technical_user_id,
                        run_id,
                        correlation,
                    });
                    request.extensions_mut().insert::<AppUser>(user);
                    return Ok(next.run(request).await);
                }
                _ => {}
            }
        }

        // Cache miss - validate token
        let claims = state.validate_token(token);
        if let Ok(claims) = claims {
            let sub = claims.get("sub").ok_or(anyhow!("sub not found"))?;
            let sub = sub.as_str().ok_or(anyhow!("sub not a string"))?;

            state.auth_cache.insert(
                cache_key,
                CachedAuth::OpenID {
                    sub: sub.to_string(),
                },
            );

            let user = AppUser::OpenID(OpenIDUser {
                sub: sub.to_string(),
                access_token: token.to_string(),
            });
            request.extensions_mut().insert::<AppUser>(user);
            return Ok(next.run(request).await);
        }

        // OpenID failed — try executor JWT
        if let Ok(claims) = crate::execution::verify_execution_jwt(token) {
            state.auth_cache.insert(
                cache_key,
                CachedAuth::Executor {
                    sub: claims.sub.clone(),
                    app_id: claims.app_id.clone(),
                    run_id: claims.run_id.clone(),
                    technical_user_id: claims.technical_user_id.clone(),
                    app_chain: claims.app_chain.clone(),
                    correlation: claims.correlation.clone(),
                },
            );
            let user = AppUser::Executor(ExecutorUser {
                sub: claims.sub,
                app_id: claims.app_id,
                run_id: claims.run_id,
                technical_user_id: claims.technical_user_id,
                app_chain: claims.app_chain,
                correlation: claims.correlation,
            });
            request.extensions_mut().insert::<AppUser>(user);
            return Ok(next.run(request).await);
        }

        // Executor failed — try app-connection JWT (app-to-app calls)
        if let Ok(claims) = crate::app_connection_jwt::verify(token) {
            state.auth_cache.insert(
                cache_key,
                CachedAuth::AppConnection {
                    sub: claims.sub.clone(),
                    origin_app_id: claims.origin_app_id.clone(),
                    target_app_id: claims.target_app_id.clone(),
                    app_chain: claims.app_chain.clone(),
                    technical_user_id: claims.technical_user_id.clone(),
                    run_id: claims.run_id.clone(),
                    correlation: claims.correlation.clone(),
                    exp: claims.exp,
                },
            );
            let user = AppUser::ConnectedApp(ConnectedAppUser {
                sub: claims.sub,
                origin_app_id: claims.origin_app_id,
                target_app_id: claims.target_app_id,
                app_chain: claims.app_chain,
                technical_user_id: claims.technical_user_id,
                run_id: claims.run_id,
                correlation: claims.correlation,
            });
            request.extensions_mut().insert::<AppUser>(user);
            return Ok(next.run(request).await);
        }
    }

    // Try PAT auth
    if let Some(auth_header) = request.headers().get(AUTHORIZATION)
        && let Ok(raw_token) = auth_header.to_str()
    {
        // Strip "Bearer " prefix if present so PATs sent as standard Bearer tokens are recognized
        let token = if raw_token.starts_with("Bearer ") {
            &raw_token[7..]
        } else {
            raw_token
        };
        let token = token.trim();

        if token.starts_with("pat_") {
            let pat_str = token;
            let cache_key = hash_token(pat_str);

            // Check cache first
            if let Some(cached) = state.auth_cache.get(&cache_key) {
                match cached {
                    CachedAuth::PAT { sub } => {
                        let pat_user = AppUser::PAT(PATUser {
                            pat: pat_str.to_string(),
                            sub,
                        });
                        request.extensions_mut().insert::<AppUser>(pat_user);
                        return Ok(next.run(request).await);
                    }
                    CachedAuth::Invalid => {
                        // Token was previously validated as invalid/expired
                        request
                            .extensions_mut()
                            .insert::<AppUser>(AppUser::Unauthorized);
                        return Ok(next.run(request).await);
                    }
                    _ => {}
                }
            }

            // Cache miss - validate PAT
            if !pat_str.starts_with("pat_") {
                return Err(AuthorizationError::from(anyhow!("Invalid PAT format")));
            }
            let pat_parts = &pat_str[4..];
            let parts: Vec<&str> = pat_parts.split('.').collect();
            if parts.len() != 2 {
                return Err(AuthorizationError::from(anyhow!("Invalid PAT format")));
            }
            let pat_id = parts[0];
            let pat_secret = parts[1];

            let mut hasher = blake3::Hasher::new();
            hasher.update(pat_secret.as_bytes());
            let secret_hash = hasher.finalize().to_hex().to_string().to_lowercase();

            let db_pat = Pat::find()
                .filter(
                    pat::Column::Id
                        .eq(pat_id)
                        .and(pat::Column::Key.eq(secret_hash)),
                )
                .one(&state.db)
                .await?;

            if let Some(pat) = db_pat {
                if let Some(valid_until) = pat.valid_until {
                    let now = chrono::Utc::now().naive_utc();
                    if valid_until < now {
                        state.auth_cache.insert(cache_key, CachedAuth::Invalid);
                        return Err(AuthorizationError::from(anyhow!("PAT is expired")));
                    }
                }

                // Cache valid PAT
                state.auth_cache.insert(
                    cache_key,
                    CachedAuth::PAT {
                        sub: pat.user_id.clone(),
                    },
                );

                let pat_user = AppUser::PAT(PATUser {
                    pat: pat_str.to_string(),
                    sub: pat.user_id.clone(),
                });
                request.extensions_mut().insert::<AppUser>(pat_user);
                return Ok(next.run(request).await);
            }
        }
    }

    // Try API key auth
    if let Some(api_key_header) = request.headers().get("x-api-key")
        && let Ok(api_key_str) = api_key_header.to_str()
    {
        let cache_key = hash_token(api_key_str);

        // Check cache first
        if let Some(cached) = state.auth_cache.get(&cache_key) {
            match cached {
                CachedAuth::ApiKey {
                    key_id,
                    app_id,
                    creator_user_id,
                } => {
                    let app_user = AppUser::APIKey(ApiKey {
                        key_id,
                        api_key: api_key_str.to_string(),
                        app_id,
                        creator_user_id,
                    });
                    request.extensions_mut().insert::<AppUser>(app_user);
                    return Ok(next.run(request).await);
                }
                CachedAuth::Invalid => {
                    request
                        .extensions_mut()
                        .insert::<AppUser>(AppUser::Unauthorized);
                    return Ok(next.run(request).await);
                }
                _ => {}
            }
        }

        // Cache miss - parse and validate API key
        // Format: flk_{app_id}.{key_id}.{secret}
        if !api_key_str.starts_with("flk_") {
            state.auth_cache.insert(cache_key, CachedAuth::Invalid);
            request
                .extensions_mut()
                .insert::<AppUser>(AppUser::Unauthorized);
            return Ok(next.run(request).await);
        }

        let key_parts = &api_key_str[4..];
        let parts: Vec<&str> = key_parts.split('.').collect();
        if parts.len() != 3 {
            state.auth_cache.insert(cache_key, CachedAuth::Invalid);
            request
                .extensions_mut()
                .insert::<AppUser>(AppUser::Unauthorized);
            return Ok(next.run(request).await);
        }

        let app_id_from_key = parts[0];
        let key_id = parts[1];
        let key_secret = parts[2];

        // Hash the secret for lookup
        let mut hasher = blake3::Hasher::new();
        hasher.update(key_secret.as_bytes());
        let secret_hash = hasher.finalize().to_hex().to_string().to_lowercase();

        let db_app = TechnicalUser::find()
            .filter(
                technical_user::Column::Id
                    .eq(key_id)
                    .and(technical_user::Column::Key.eq(secret_hash)),
            )
            .one(&state.db)
            .await?;

        if let Some(app) = db_app {
            if app.app_id != app_id_from_key {
                state.auth_cache.insert(cache_key, CachedAuth::Invalid);
                request
                    .extensions_mut()
                    .insert::<AppUser>(AppUser::Unauthorized);
                return Ok(next.run(request).await);
            }

            if let Some(valid_until) = app.valid_until {
                let now = chrono::Utc::now().naive_utc();
                if valid_until < now {
                    state.auth_cache.insert(cache_key, CachedAuth::Invalid);
                    return Err(AuthorizationError::from(anyhow!("API Key is expired")));
                }
            }

            let creator_user_id = match app.creator_user_id.clone() {
                Some(creator_user_id) => Some(creator_user_id),
                None => resolve_legacy_api_key_creator_user_id(&state, &app.app_id).await?,
            };

            // Cache valid API key
            state.auth_cache.insert(
                cache_key,
                CachedAuth::ApiKey {
                    key_id: app.id.clone(),
                    app_id: app.app_id.clone(),
                    creator_user_id: creator_user_id.clone(),
                },
            );

            let app_user = AppUser::APIKey(ApiKey {
                key_id: app.id.clone(),
                api_key: api_key_str.to_string(),
                app_id: app.app_id.clone(),
                creator_user_id,
            });
            request.extensions_mut().insert::<AppUser>(app_user);
            return Ok(next.run(request).await);
        }
    }

    request
        .extensions_mut()
        .insert::<AppUser>(AppUser::Unauthorized);
    Ok(next.run(request).await)
}
