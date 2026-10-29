use sea_orm::sea_query::{Expr, ExprTrait};
use std::sync::Arc;
use tracing::Instrument;

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
    http::HeaderMap,
    middleware::Next,
    response::Response,
};
use flow_like::flow::execution::{ExecutionPrincipal, RoleContext, UserExecutionContext};
use flow_like::hub::UserTier;
use flow_like_types::Result;
use flow_like_types::anyhow;
use hyper::header::AUTHORIZATION;
use sea_orm::{
    ColumnTrait, EntityTrait, JoinType, QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};
use serde::de::{self, Unexpected};
use serde::{Deserialize, Deserializer};

use crate::routes::sink::authority::{RunAsVerdict, SinkAuthority, sink_authority};
use crate::state::{AppState, CachedAuth, cached_openid_is_current};

/// Client IP address extracted from the request for audit trail purposes.
/// Checks X-Forwarded-For, X-Real-Ip, then falls back to the peer address.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ClientIp(pub Option<String>);

/// Header the AWS edge uses to forward the viewer's `Authorization` value.
///
/// CloudFront OAC with `signing_behavior = always` overwrites `Authorization`
/// with its own SigV4 origin signature, so a viewer-request function copies
/// the original value into this header first. The Terraform precondition in
/// `modules/aws/workloads/api/main.tf` pins this file to that contract; keep
/// the literal header name in sync with the CloudFront function there.
pub const FORWARDED_AUTHORIZATION_HEADER: &str = "x-flow-like-authorization";

/// Prefix of the SigV4 signature CloudFront OAC writes into `Authorization`.
/// A value with this prefix is CloudFront's origin signature, never a viewer
/// credential.
const SIGV4_AUTHORIZATION_PREFIX: &str = "AWS4-HMAC-SHA256";

/// The viewer's `Authorization` value, resilient to CloudFront OAC signing.
///
/// Prefers `x-flow-like-authorization` when present and non-empty (the AWS
/// edge forwards the real viewer header there), otherwise falls back to
/// `Authorization`, ignoring it when it carries CloudFront's SigV4 signature.
/// Every reader of viewer credentials must go through this helper so the OAC
/// fallback cannot drift between call sites.
pub fn viewer_authorization(headers: &HeaderMap) -> Option<&str> {
    if let Some(forwarded) = headers.get(FORWARDED_AUTHORIZATION_HEADER)
        && let Ok(value) = forwarded.to_str()
        && !value.trim().is_empty()
    {
        return Some(value);
    }

    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    if value.trim_start().starts_with(SIGV4_AUTHORIZATION_PREFIX) {
        return None;
    }
    Some(value)
}

/// Proxies append to `X-Forwarded-For`, so only the entries written by the
/// deployment's own proxies are trustworthy: the client address is the one
/// `trusted_hops` from the right. Without that setting the leftmost entry is
/// used, which the client chooses.
fn forwarded_client_ip(header: &str, trusted_hops: Option<u32>) -> Option<&str> {
    let mut hops = header.split(',').map(str::trim).filter(|ip| !ip.is_empty());
    match trusted_hops {
        None => hops.next(),
        Some(0) => None,
        Some(hops_from_right) => hops.rev().nth(hops_from_right as usize - 1),
    }
}

fn extract_client_ip(request: &Request, trusted_hops: Option<u32>) -> Option<String> {
    if let Some(forwarded) = request.headers().get("x-forwarded-for")
        && let Ok(val) = forwarded.to_str()
    {
        return forwarded_client_ip(val, trusted_hops).map(str::to_string);
    }
    if trusted_hops.is_some() {
        return None;
    }
    if let Some(real_ip) = request.headers().get("x-real-ip")
        && let Ok(val) = real_ip.to_str()
    {
        return Some(val.trim().to_string());
    }
    None
}

fn pat_id_from_token(pat_str: &str) -> Result<String> {
    pat_lookup_parts(pat_str)
        .map(|(id, _)| id.to_string())
        .ok_or_else(|| anyhow!("Invalid PAT format"))
}

#[path = "pat_validation.rs"]
mod pat_validation;
use pat_validation::lookup_parts as pat_lookup_parts;

fn api_key_lookup_parts(api_key: &str) -> Option<(&str, &str, String)> {
    let mut parts = api_key.strip_prefix("flk_")?.split('.');
    let app_id = parts.next()?.trim();
    let key_id = parts.next()?.trim();
    let secret = parts.next()?;
    if app_id.is_empty() || key_id.is_empty() || secret.is_empty() || parts.next().is_some() {
        return None;
    }

    let mut hasher = blake3::Hasher::new();
    hasher.update(secret.as_bytes());
    Some((
        app_id,
        key_id,
        hasher.finalize().to_hex().to_string().to_lowercase(),
    ))
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
    pub name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub middle_name: Option<String>,
    pub nickname: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// Signed replay flag: a shadow or regression run may read live state but
    /// never change it.
    pub shadow: bool,
    /// The run serves a hosted frontend visitor; `sub` is the visitor.
    pub hosted_frontend: bool,
    pub event_id: Option<String>,
    /// User whose private custom Bits the run may call through the API.
    pub custom_bits_owner: Option<String>,
    /// No person triggered the run; `sub` is the user the event runs as or a placeholder,
    /// never someone app connections may pass on as a person.
    pub unattended: bool,
}

impl From<crate::execution::ExecutionClaims> for ExecutorUser {
    fn from(claims: crate::execution::ExecutionClaims) -> Self {
        Self {
            sub: claims.sub,
            app_id: claims.app_id,
            run_id: claims.run_id,
            technical_user_id: claims.technical_user_id,
            app_chain: claims.app_chain,
            correlation: claims.correlation,
            shadow: claims.shadow.unwrap_or(false),
            hosted_frontend: claims.hosted_frontend,
            event_id: claims.event_id,
            custom_bits_owner: claims.custom_bits_owner,
            unattended: claims.unattended,
        }
    }
}

/// What a route does with an executor's app authority. Shadow and regression
/// replays may read, but every `Write` is refused for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorUse {
    Read,
    /// The action completes "Shadow and regression runs cannot …".
    Write(&'static str),
}

impl ExecutorUse {
    fn admit(self, user: &AppUser) -> Result<(), ApiError> {
        match self {
            ExecutorUse::Read => Ok(()),
            ExecutorUse::Write(action) => user.deny_shadow_execution(action),
        }
    }
}

/// Error code of a side effect refused to a shadow or regression run.
pub const SHADOW_SIDE_EFFECT_REFUSED: &str = "SHADOW_SIDE_EFFECT_REFUSED";

pub fn shadow_side_effect_refused(action: &str) -> ApiError {
    ApiError::coded(
        axum::http::StatusCode::FORBIDDEN,
        SHADOW_SIDE_EFFECT_REFUSED,
        format!("Shadow and regression runs cannot {action}"),
    )
}

/// Principal for app-to-app calls: a flow in `origin_app_id` acting on
/// `target_app_id` through an accepted app connection. The token pins both
/// app ids; permissions come from the role assigned to the connection, bounded
/// by the person it passed through.
#[derive(Debug, Clone)]
pub struct ConnectedAppUser {
    /// The subject the calling run passed through end-to-end: the person who started the
    /// call chain, even if they are not a member of the target app, or a placeholder or
    /// pseudonym when no person did (see [`ConnectedAppUser::person`]).
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

impl ConnectedAppUser {
    /// The person who started the call chain: the account the connection passed
    /// through, unless an API key started it. An anonymous visitor's pseudonym and the
    /// placeholders of a run without a user or that no person triggered name nobody.
    pub fn person(&self) -> Option<&str> {
        if self.technical_user_id.is_some() {
            return None;
        }
        self.sub
            .as_deref()
            .and_then(crate::execution::subject::account_user_id)
    }
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
    /// Which kind of principal this is. `sub.is_none()` cannot stand in for it:
    /// API keys and app connections both leave `sub` unset while meaning very
    /// different things to a running flow.
    pub principal: ExecutionPrincipal,
    /// For `ConnectedApp`, the app that made the call.
    pub origin_app_id: Option<String>,
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

    /// Convert to UserExecutionContext for execution.
    pub fn to_user_context(&self) -> UserExecutionContext {
        user_context_from_parts(
            self.principal,
            self.sub.as_deref(),
            self.effective_user_id.as_deref(),
            self.technical_user_id
                .as_deref()
                .unwrap_or(&self.identifier),
            self.origin_app_id.as_deref().unwrap_or_default(),
            RoleContext {
                id: self.role.id.clone(),
                name: self.role.name.clone(),
                permissions: self.role.permissions,
                attributes: self.role.attributes.clone().unwrap_or_default().into(),
                custom_attributes: std::collections::HashMap::new(),
            },
        )
    }
}

/// Maps an authenticated principal onto the identity a run sees.
///
/// The three principals stay distinguishable inside the flow: a human keeps
/// their subject, an API key reports its key id, and a connected app reports
/// the calling app. The subject an API key or app connection passed through
/// lands in `on_behalf_of`, never in `sub`: the context names the principal that
/// called. Whether the run acts as that person is settled where it is dispatched.
fn user_context_from_parts(
    principal: ExecutionPrincipal,
    sub: Option<&str>,
    effective_user_id: Option<&str>,
    key_id: &str,
    origin_app_id: &str,
    role: RoleContext,
) -> UserExecutionContext {
    let on_behalf_of = effective_user_id.map(ToOwned::to_owned);

    match principal {
        ExecutionPrincipal::User => {
            UserExecutionContext::new(sub.unwrap_or_default()).with_role(role)
        }
        ExecutionPrincipal::ApiKey => UserExecutionContext::technical(
            key_id,
            role.id,
            role.name,
            role.permissions,
            role.attributes,
            role.custom_attributes,
        )
        .with_on_behalf_of(on_behalf_of),
        ExecutionPrincipal::ConnectedApp => {
            UserExecutionContext::connected_app(origin_app_id, role).with_on_behalf_of(on_behalf_of)
        }
    }
}

impl AppUser {
    fn app_permission_denial(&self) -> ApiError {
        match self {
            AppUser::Unauthorized => ApiError::unauthorized("Authentication required"),
            _ => ApiError::forbidden("User does not have app permissions"),
        }
    }

    /// Whether this principal is another app acting through a connection token.
    /// The single source of truth for the connected-app discrimination used by
    /// the route-level guards, so a new connected-app-like variant only has to
    /// be handled here.
    pub fn is_connected_app(&self) -> bool {
        matches!(self, AppUser::ConnectedApp(_))
    }

    pub fn is_shadow_execution(&self) -> bool {
        matches!(self, AppUser::Executor(executor) if executor.shadow)
    }

    /// Refuses `action` with 403 `SHADOW_SIDE_EFFECT_REFUSED` when the caller is
    /// a shadow or regression run.
    pub fn deny_shadow_execution(&self, action: &str) -> Result<(), ApiError> {
        if self.is_shadow_execution() {
            return Err(shadow_side_effect_refused(action));
        }
        Ok(())
    }

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
            AppUser::APIKey(key) => key
                .creator_user_id
                .clone()
                .unwrap_or_else(|| format!("app:{}", key.app_id)),
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
        user::Entity::find_by_id(&sub)
            .select_only()
            .column(user::Column::TrackingId)
            .into_tuple::<Option<String>>()
            .one(&state.db)
            .await?
            .ok_or_else(|| AuthorizationError::from(anyhow!("User not found")))
    }

    pub async fn tier(&self, state: &AppState) -> Result<UserTier, AuthorizationError> {
        tier_for_sub(state, &self.effective_user_id()?).await
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

        let res = match user_info_http_client()
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
        let permission_bits = user::Entity::find_by_id(&sub)
            .select_only()
            .column(user::Column::Permission)
            .into_tuple::<i64>()
            .one(&state.db)
            .await?
            .ok_or_else(|| anyhow!("User not found"))?;
        let permission = GlobalPermission::from_bits(permission_bits)
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

    /// Read current app authority from the database so role changes apply on
    /// every replica, including routes that can issue another credential.
    pub async fn app_permission(
        &self,
        app_id: &str,
        state: &AppState,
    ) -> Result<AppPermissionResponse, ApiError> {
        // Keep anonymous principals available to public routes in the JWT
        // middleware, but classify them as unauthenticated as soon as a route
        // asks for app permissions. Previously this fell through to an
        // anyhow-backed ApiError and was reported as a 500 INTERNAL_ERROR.
        if matches!(self, AppUser::Unauthorized) {
            return Err(self.app_permission_denial());
        }

        if let Ok(sub) = self.sub() {
            return user_app_permission_uncached(&sub, app_id, state).await;
        }

        if let AppUser::ConnectedApp(connected_app) = self {
            return connected_app_permission(connected_app, app_id, state).await;
        }

        if let AppUser::APIKey(api_key) = self {
            if api_key.app_id != app_id {
                return Err(ApiError::FORBIDDEN);
            }

            let Some(role_model) =
                api_key_role(&state.db, &api_key.app_id, &api_key.key_id).await?
            else {
                state.auth_cache.invalidate(&hash_token(&api_key.api_key));
                return Err(ApiError::unauthorized("API key is no longer valid"));
            };

            let permissions = role_permissions(&role_model)?;
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
                principal: ExecutionPrincipal::ApiKey,
                origin_app_id: None,
            });
        }

        Err(self.app_permission_denial())
    }

    /// Revalidate the credential as well as the current app role for runtime
    /// entry points that may retain an authenticated principal between requests.
    pub async fn app_permission_fresh(
        &self,
        app_id: &str,
        state: &AppState,
    ) -> Result<AppPermissionResponse, ApiError> {
        match self {
            AppUser::OpenID(user) => user_app_permission_uncached(&user.sub, app_id, state).await,
            AppUser::PAT(user) => {
                validate_pat_fresh(user, state).await?;
                user_app_permission_uncached(&user.sub, app_id, state).await
            }
            AppUser::ConnectedApp(connected) => {
                connected_app_permission_uncached(connected, app_id, state).await
            }
            AppUser::APIKey(api_key) => {
                validate_api_key_fresh(api_key, state).await?;
                // API-key roles already bypass the permission cache and are
                // joined to the freshly revalidated technical user.
                self.app_permission(app_id, state).await
            }
            AppUser::Executor(_) | AppUser::Unauthorized => {
                self.app_permission(app_id, state).await
            }
        }
    }

    /// App authority for runtime callbacks. Every caller declares whether it
    /// only reads or changes state, so a replay can never reach a new write
    /// route by default.
    pub async fn execution_app_permission(
        &self,
        app_id: &str,
        state: &AppState,
        executor_use: ExecutorUse,
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
            executor_use.admit(self)?;

            // Runs initiated through an app connection are bounded by the
            // connection role and, when the run acts as a person, by what that
            // person holds in this app: their role, or only running events when
            // they are not a member. The calling app never exceeds the role it
            // was granted. This also keeps multi-hop chains (A -> B -> C)
            // working, since B's callbacks and token minting resolve against
            // the A -> B connection.
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
            if let Some(key_id) = executor.technical_user_id.as_deref() {
                return api_key_executor_permission(executor, key_id, app_id, state).await;
            }
            if executor.hosted_frontend {
                return hosted_executor_permission(executor, app_id, state).await;
            }
            return member_executor_permission(executor, app_id, state).await;
        }

        self.app_permission(app_id, state).await
    }
}

/// Who an app permission is granted to; the role is resolved separately.
#[derive(Debug, PartialEq, Eq)]
struct Grantee {
    sub: Option<String>,
    effective_user_id: Option<String>,
    technical_user_id: Option<String>,
    identifier: String,
    principal: ExecutionPrincipal,
}

impl Grantee {
    fn user(sub: &str) -> Self {
        Self {
            sub: Some(sub.to_owned()),
            effective_user_id: Some(sub.to_owned()),
            technical_user_id: None,
            identifier: sub.to_owned(),
            principal: ExecutionPrincipal::User,
        }
    }

    /// A run started with an API key acts as that key, like the key itself. The user the
    /// run is attributed to never becomes its subject, so user-scoped routes refuse it.
    fn api_key_run(attributed_to: &str, key_id: &str) -> Self {
        Self {
            sub: None,
            effective_user_id: Some(attributed_to.to_owned()),
            technical_user_id: Some(key_id.to_owned()),
            identifier: key_id.to_owned(),
            principal: ExecutionPrincipal::ApiKey,
        }
    }

    fn grant(
        self,
        state: &AppState,
        role: role::Model,
        permissions: RolePermissions,
    ) -> AppPermissionResponse {
        AppPermissionResponse {
            state: state.clone(),
            permissions,
            role: Arc::new(role),
            sub: self.sub,
            effective_user_id: self.effective_user_id,
            technical_user_id: self.technical_user_id,
            identifier: self.identifier,
            principal: self.principal,
            origin_app_id: None,
        }
    }
}

fn role_permissions(role: &role::Model) -> Result<RolePermissions, ApiError> {
    Ok(RolePermissions::from_bits(role.permissions)
        .ok_or_else(|| anyhow!("Invalid role permission bits"))?)
}

/// The role `user_id` holds in `app_id` through a membership.
async fn member_role<C: sea_orm::ConnectionTrait>(
    db: &C,
    user_id: &str,
    app_id: &str,
) -> Result<Option<role::Model>, ApiError> {
    Ok(role::Entity::find()
        .join(JoinType::InnerJoin, role::Relation::Membership.def())
        .filter(
            membership::Column::UserId
                .eq(user_id)
                .and(membership::Column::AppId.eq(app_id)),
        )
        .one(db)
        .instrument(role_query_span())
        .await?)
}

/// The role of the app's API key `key_id`; `None` once the key is deleted, loses its role
/// or expires.
async fn api_key_role<C: sea_orm::ConnectionTrait>(
    db: &C,
    app_id: &str,
    key_id: &str,
) -> Result<Option<role::Model>, ApiError> {
    let now = chrono::Utc::now().fixed_offset();
    Ok(role::Entity::find()
        .join(JoinType::InnerJoin, role::Relation::TechnicalUser.def())
        .filter(
            technical_user::Column::AppId
                .eq(app_id)
                .and(technical_user::Column::Id.eq(key_id))
                .and(
                    technical_user::Column::ValidUntil
                        .is_null()
                        .or(technical_user::Column::ValidUntil.gte(now)),
                ),
        )
        .one(db)
        .instrument(role_query_span())
        .await?)
}

const API_KEY_RUN_REFUSED: &str =
    "The API key that started this run was deleted, has expired or has no role";

/// The role of the API key that started the executor run `run_id`. A deleted, expired or
/// roleless key refuses every callback of the run alike.
pub(crate) async fn api_key_run_role<C: sea_orm::ConnectionTrait>(
    db: &C,
    app_id: &str,
    key_id: &str,
    run_id: &str,
) -> Result<role::Model, ApiError> {
    api_key_role(db, app_id, key_id).await?.ok_or_else(|| {
        tracing::debug!(
            key_id,
            app_id,
            run_id,
            "API key of an executor run has no current role in the app"
        );
        ApiError::forbidden(API_KEY_RUN_REFUSED)
    })
}

async fn member_executor_permission(
    executor: &ExecutorUser,
    app_id: &str,
    state: &AppState,
) -> Result<AppPermissionResponse, ApiError> {
    let role_model = member_role(&state.db, &executor.sub, app_id)
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
    let permissions = role_permissions(&role_model)?;
    Ok(Grantee::user(&executor.sub).grant(state, role_model, permissions))
}

async fn api_key_executor_permission(
    executor: &ExecutorUser,
    key_id: &str,
    app_id: &str,
    state: &AppState,
) -> Result<AppPermissionResponse, ApiError> {
    let role_model = api_key_run_role(&state.db, app_id, key_id, &executor.run_id).await?;
    let permissions = role_permissions(&role_model)?;
    Ok(Grantee::api_key_run(&executor.sub, key_id).grant(state, role_model, permissions))
}

/// The most a hosted-frontend run grants beyond what its visitor already holds: running
/// the app's events.
const HOSTED_FRONTEND_PERMISSIONS: RolePermissions = RolePermissions::ExecuteEvents;
const HOSTED_VISITOR_ROLE_ID: &str = "hosted-visitor";
const HOSTED_VISITOR_ROLE_NAME: &str = "Visitor";

/// Owner and Admin stand for every permission; a hosted run and a connection call on
/// behalf of a person get the permissions, never the roles.
fn expanded_permissions(role: RolePermissions) -> RolePermissions {
    let privileged = RolePermissions::Owner | RolePermissions::Admin;
    if role.intersects(privileged) {
        RolePermissions::all().difference(privileged)
    } else {
        role
    }
}

/// A person's standing in an app a run reaches on their behalf: a member stands on their
/// role, anyone else may only run the app's events.
fn person_standing(member: Option<RolePermissions>) -> RolePermissions {
    member.map_or(RolePermissions::ExecuteEvents, expanded_permissions)
}

/// Whose authority a hosted-frontend run holds on its callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostedAuthority {
    /// A signed-in visitor's run acts as the visitor: their role in the app, `None` when
    /// they are not a member. The user the event runs as neither caps nor lends anything.
    Visitor(Option<RolePermissions>),
    /// An anonymous visitor's run borrows what the user the event runs as lends, see
    /// [`lent_authority`].
    Lent(Option<RolePermissions>),
}

/// What a hosted-frontend run may do. A signed-in visitor keeps their own standing in the
/// app; an anonymous visitor may only run events, and only while the user the event runs
/// as may. Nothing at all is refused with 403.
fn hosted_permissions(authority: HostedAuthority) -> Result<RolePermissions, ApiError> {
    let permissions = match authority {
        HostedAuthority::Visitor(member) => person_standing(member),
        HostedAuthority::Lent(lent) => {
            lent.map_or(RolePermissions::empty(), expanded_permissions)
                & HOSTED_FRONTEND_PERMISSIONS
        }
    };
    if permissions.is_empty() {
        return Err(ApiError::forbidden(
            "This hosted frontend run has no access to this app",
        ));
    }
    Ok(permissions)
}

/// The role bits the user an event's active sink runs as lends the event's anonymous hosted
/// runs, while that user may still trigger it. That user is the sink's run-as user or, for a
/// legacy sink, the owner its stored PAT still names; any other sink lends nothing.
fn lent_authority(authority: Option<SinkAuthority>) -> Result<Option<RolePermissions>, ApiError> {
    match authority {
        Some(SinkAuthority {
            run_as_user_id: Some(_),
            verdict: RunAsVerdict::Allowed(role),
        }) => Ok(Some(role)),
        Some(SinkAuthority {
            verdict: RunAsVerdict::Transient(reason),
            ..
        }) => Err(ApiError::service_unavailable(reason)),
        _ => Ok(None),
    }
}

async fn visitor_permissions<C: sea_orm::ConnectionTrait>(
    db: &C,
    user_id: &str,
    app_id: &str,
) -> Result<Option<RolePermissions>, ApiError> {
    member_role(db, user_id, app_id)
        .await?
        .as_ref()
        .map(role_permissions)
        .transpose()
}

async fn event_authority<C: sea_orm::ConnectionTrait>(
    db: &C,
    encryption_key: &[u8; 32],
    executor: &ExecutorUser,
    app_id: &str,
) -> Result<Option<RolePermissions>, ApiError> {
    let Some(event_id) = executor.event_id.as_deref() else {
        return Ok(None);
    };
    lent_authority(sink_authority(db, encryption_key, app_id, event_id).await?)
}

/// Read on every callback. A signed-in visitor's run follows the visitor's current role and
/// never reads the event's sink. An anonymous visitor's run reads the user the sink runs as,
/// so a paused sink or a run-as user who lost access stops runs already in flight.
async fn hosted_executor_permissions<C: sea_orm::ConnectionTrait>(
    db: &C,
    encryption_key: &[u8; 32],
    executor: &ExecutorUser,
    app_id: &str,
) -> Result<RolePermissions, ApiError> {
    let authority = match crate::execution::subject::account_user_id(&executor.sub) {
        Some(visitor) => HostedAuthority::Visitor(visitor_permissions(db, visitor, app_id).await?),
        None => HostedAuthority::Lent(event_authority(db, encryption_key, executor, app_id).await?),
    };
    hosted_permissions(authority)
}

fn hosted_visitor_role(app_id: &str, permissions: RolePermissions) -> role::Model {
    role::Model {
        id: HOSTED_VISITOR_ROLE_ID.to_owned(),
        name: HOSTED_VISITOR_ROLE_NAME.to_owned(),
        description: None,
        permissions: permissions.bits(),
        app_id: Some(app_id.to_owned()),
        created_at: Default::default(),
        updated_at: Default::default(),
        attributes: None,
    }
}

/// A hosted-frontend run acts as its visitor, with the permissions of [`hosted_permissions`]
/// in a synthetic visitor role.
async fn hosted_executor_permission(
    executor: &ExecutorUser,
    app_id: &str,
    state: &AppState,
) -> Result<AppPermissionResponse, ApiError> {
    let permissions =
        hosted_executor_permissions(&state.db, &state.encryption_key, executor, app_id)
            .await
            .inspect_err(|error| {
                tracing::debug!(
                    user_id = %executor.sub,
                    app_id = %app_id,
                    run_id = %executor.run_id,
                    event_id = ?executor.event_id,
                    status = %error.status(),
                    "Hosted frontend run has no authority in the app"
                );
            })?;
    Ok(Grantee::user(&executor.sub).grant(
        state,
        hosted_visitor_role(app_id, permissions),
        permissions,
    ))
}

pub(crate) async fn validate_pat_fresh(user: &PATUser, state: &AppState) -> Result<(), ApiError> {
    fresh_pat_permissions(user, state).await.map(|_| ())
}

/// Return the current access level from the same lookup that checks the
/// token secret, owner and expiry.
pub(crate) async fn fresh_pat_permissions(
    user: &PATUser,
    state: &AppState,
) -> Result<i64, ApiError> {
    Ok(fresh_pat(user, state).await?.permissions)
}

pub(crate) async fn fresh_pat(user: &PATUser, state: &AppState) -> Result<pat::Model, ApiError> {
    let now = chrono::Utc::now().fixed_offset();
    let current = pat_validation::lookup_current(&state.db, &user.pat, now).await?;
    let Some(current) = current.filter(|pat| pat_is_current(pat, &user.sub, now)) else {
        return Err(ApiError::unauthorized("PAT is no longer valid"));
    };
    if crate::permission::pat_permission::PatPermission::from_bits(current.permissions).is_none() {
        return Err(ApiError::forbidden(
            crate::permission::pat_permission::LEGACY_PERMISSION_ERROR,
        ));
    }

    Ok(current)
}

fn pat_is_current(
    pat: &pat::Model,
    expected_sub: &str,
    now: sea_orm::prelude::DateTimeWithTimeZone,
) -> bool {
    pat.user_id == expected_sub && pat.valid_until.is_none_or(|valid_until| valid_until >= now)
}

async fn validate_api_key_fresh(api_key: &ApiKey, state: &AppState) -> Result<(), ApiError> {
    let cache_key = hash_token(&api_key.api_key);
    let Some((token_app_id, token_key_id, secret_hash)) = api_key_lookup_parts(&api_key.api_key)
    else {
        state.auth_cache.invalidate(&cache_key);
        return Err(ApiError::unauthorized("API key is no longer valid"));
    };
    if token_app_id != api_key.app_id || token_key_id != api_key.key_id {
        state.auth_cache.invalidate(&cache_key);
        return Err(ApiError::unauthorized("API key is no longer valid"));
    }

    let current = TechnicalUser::find()
        .filter(
            technical_user::Column::Id
                .eq(token_key_id)
                .and(technical_user::Column::AppId.eq(token_app_id))
                .and(technical_user::Column::Key.eq(secret_hash)),
        )
        .one(&state.db)
        .await?;
    let Some(current) = current else {
        state.auth_cache.invalidate(&cache_key);
        return Err(ApiError::unauthorized("API key is no longer valid"));
    };

    let current_creator_user_id = match current.creator_user_id.clone() {
        Some(creator_user_id) => Some(creator_user_id),
        None => resolve_legacy_api_key_creator_user_id(state, &current.app_id).await?,
    };
    if !api_key_record_is_current(
        &current,
        api_key,
        current_creator_user_id.as_deref(),
        chrono::Utc::now().fixed_offset(),
    ) {
        state.auth_cache.invalidate(&cache_key);
        return Err(ApiError::unauthorized("API key is no longer valid"));
    }

    Ok(())
}

fn api_key_record_is_current(
    record: &technical_user::Model,
    cached: &ApiKey,
    current_creator_user_id: Option<&str>,
    now: sea_orm::prelude::DateTimeWithTimeZone,
) -> bool {
    record.id == cached.key_id
        && record.app_id == cached.app_id
        && record
            .valid_until
            .is_none_or(|valid_until| valid_until >= now)
        && current_creator_user_id == cached.creator_user_id.as_deref()
}

/// The plan tier of a user id. Split out of [`AppUser::tier`] so background work
/// that only carries a `sub` — the copilot profile loader, for one — can apply the
/// same plan rules the request-scoped routes do.
pub async fn tier_for_sub(state: &AppState, sub: &str) -> Result<UserTier, AuthorizationError> {
    let user = user::Entity::find_by_id(sub)
        .one(&state.db)
        .await?
        .ok_or_else(|| AuthorizationError::from(anyhow!("User not found")))?;

    let db_tier = match user.tier {
        sea_orm_active_enums::UserTier::Free => "FREE",
        sea_orm_active_enums::UserTier::Premium => "PREMIUM",
        sea_orm_active_enums::UserTier::Max => "MAX",
        sea_orm_active_enums::UserTier::Pro => "PRO",
        sea_orm_active_enums::UserTier::Enterprise => "ENTERPRISE",
    };

    state
        .platform_config
        .tiers
        .get(db_tier)
        .cloned()
        .ok_or_else(|| AuthorizationError::from(anyhow!("Tier not found")))
}

fn user_info_http_client() -> &'static flow_like_types::reqwest::Client {
    static CLIENT: std::sync::OnceLock<flow_like_types::reqwest::Client> =
        std::sync::OnceLock::new();
    CLIENT.get_or_init(flow_like_types::reqwest::Client::new)
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

/// Resolve current project authority without constructing a human principal or
/// consulting the permission cache. Workload grants call this within admission.
pub(crate) async fn fresh_user_role<C: sea_orm::ConnectionTrait>(
    db: &C,
    sub: &str,
    app_id: &str,
) -> Result<RolePermissions, ApiError> {
    let row = db.query_one_raw(sea_orm::Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        r#"SELECT r.permissions FROM "Membership" m JOIN "Role" r ON r.id = m."roleId" AND r."appId" = m."appId" JOIN "App" a ON a.id = m."appId" WHERE m."userId" = $1 AND m."appId" = $2 AND a.status = 'ACTIVE'"#,
        [sub.into(), app_id.into()],
    )).await?.ok_or(ApiError::FORBIDDEN)?;
    RolePermissions::from_bits(row.try_get::<i64>("", "permissions")?)
        .ok_or_else(|| ApiError::internal("Invalid role permission bits"))
}

fn role_query_span() -> tracing::Span {
    tracing::info_span!(
        target: "flow_like::observability",
        "db.query",
        db.operation = "select",
        db.table = "Role"
    )
}

async fn user_app_permission_uncached(
    sub: &str,
    app_id: &str,
    state: &AppState,
) -> Result<AppPermissionResponse, ApiError> {
    let role_model = member_role(&state.db, sub, app_id).await?.ok_or_else(|| {
        tracing::debug!(sub, app_id, "Current app membership role was not found");
        ApiError::FORBIDDEN
    })?;
    let permissions = role_permissions(&role_model)?;
    Ok(Grantee::user(sub).grant(state, role_model, permissions))
}

async fn connected_app_permission(
    connected_app: &ConnectedAppUser,
    app_id: &str,
    state: &AppState,
) -> Result<AppPermissionResponse, ApiError> {
    connected_app_permission_uncached(connected_app, app_id, state).await
}

async fn connected_app_permission_uncached(
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

    let role_model = connected_app_role(&state.db, connected_app, app_id).await?;
    let permissions = role_permissions(&role_model)?;

    Ok(AppPermissionResponse {
        state: state.clone(),
        permissions,
        role: Arc::new(role_model),
        sub: None,
        effective_user_id: connected_app.sub.clone(),
        technical_user_id: connected_app.technical_user_id.clone(),
        identifier: app_connection_cache_sub(&connected_app.origin_app_id),
        principal: ExecutionPrincipal::ConnectedApp,
        origin_app_id: Some(connected_app.origin_app_id.clone()),
    })
}

/// The role a call through `connected_app` holds in `app_id`: the connection's role,
/// keeping only the permissions and column attributes the person it passed through also
/// holds when there is one.
async fn connected_app_role<C: sea_orm::ConnectionTrait>(
    db: &C,
    connected_app: &ConnectedAppUser,
    app_id: &str,
) -> Result<role::Model, ApiError> {
    let origin_app_id = connected_app.origin_app_id.as_str();
    let Some(person) = connected_app.person() else {
        return connection_role(db, origin_app_id, app_id).await;
    };
    let (connection, membership) = flow_like_types::tokio::try_join!(
        connection_role(db, origin_app_id, app_id),
        member_role(db, person, app_id),
    )?;
    let member = membership.as_ref().map(role_permissions).transpose()?;
    let granted = role_permissions(&connection)?;
    let permissions = connected_person_permissions(granted, member).inspect_err(|_| {
        tracing::debug!(
            user_id = %person,
            origin_app_id = %origin_app_id,
            target_app_id = %app_id,
            member = member.is_some(),
            "App connection and the person it passed through share no permission"
        );
    })?;
    let member_columns = membership
        .as_ref()
        .zip(member)
        .map(|(role, permissions)| ColumnAccess::new(role, permissions));
    let attributes = connected_person_attributes(
        &ColumnAccess::new(&connection, granted),
        member_columns.as_ref(),
    );
    Ok(role::Model {
        permissions: permissions.bits(),
        attributes: attributes.map(Into::into),
        ..connection
    })
}

/// Whether the active connection from `origin_app_id` still grants `person` some standing in
/// `app_id`: what a call through it on their behalf would hold (see [`connected_app_role`]).
/// A connection that is gone, pending or roleless, or shares no permission with the person,
/// grants none.
pub(crate) async fn connection_holds_person<C: sea_orm::ConnectionTrait>(
    db: &C,
    origin_app_id: &str,
    app_id: &str,
    person: &str,
) -> Result<bool, ApiError> {
    let call = ConnectedAppUser {
        sub: Some(person.to_owned()),
        origin_app_id: origin_app_id.to_owned(),
        target_app_id: app_id.to_owned(),
        app_chain: vec![origin_app_id.to_owned()],
        technical_user_id: None,
        run_id: None,
        correlation: None,
    };
    match connected_app_role(db, &call, app_id).await {
        Ok(_) => Ok(true),
        Err(error) if error.status() == axum::http::StatusCode::FORBIDDEN => Ok(false),
        Err(error) => Err(error),
    }
}

/// The column attributes a role holds. Owner and Admin read every column, so they hold
/// every attribute.
#[derive(Debug)]
struct ColumnAccess {
    every: bool,
    listed: std::collections::BTreeSet<String>,
}

impl ColumnAccess {
    fn new(role: &role::Model, permissions: RolePermissions) -> Self {
        Self {
            every: permissions.intersects(RolePermissions::Owner | RolePermissions::Admin),
            listed: role
                .attributes
                .iter()
                .flatten()
                .map(|attribute| attribute.trim())
                .filter(|attribute| !attribute.is_empty())
                .map(str::to_owned)
                .collect(),
        }
    }

    fn holds(&self, attribute: &str) -> bool {
        self.every || self.listed.contains(attribute)
    }
}

/// The column attributes a call through an app connection on behalf of a person holds:
/// each attribute either role lists that both hold. A person who is not a member holds
/// none. Such a call never reads every column, because Owner and Admin are never handed
/// out (see [`expanded_permissions`]).
fn connected_person_attributes(
    connection: &ColumnAccess,
    member: Option<&ColumnAccess>,
) -> Option<Vec<String>> {
    let member = member?;
    let shared: Vec<String> = connection
        .listed
        .union(&member.listed)
        .filter(|attribute| connection.holds(attribute) && member.holds(attribute))
        .cloned()
        .collect();
    (!shared.is_empty()).then_some(shared)
}

/// What a call through an app connection on behalf of a person may do: what both the
/// connection's role and the person's standing in the app grant (see [`person_standing`]).
/// Nothing shared is refused with 403.
fn connected_person_permissions(
    connection: RolePermissions,
    member: Option<RolePermissions>,
) -> Result<RolePermissions, ApiError> {
    let permissions = expanded_permissions(connection) & person_standing(member);
    if permissions.is_empty() {
        return Err(ApiError::forbidden(
            "This app connection grants the person it passed through no access to this app",
        ));
    }
    Ok(permissions)
}

/// The role of the active connection from `origin_app_id` to `app_id`.
async fn connection_role<C: sea_orm::ConnectionTrait>(
    db: &C,
    origin_app_id: &str,
    app_id: &str,
) -> Result<role::Model, ApiError> {
    let (connection, role) = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(origin_app_id)
                .and(app_connection::Column::TargetAppId.eq(app_id))
                .and(
                    app_connection::Column::Status
                        .eq(sea_orm_active_enums::AppConnectionStatus::Active),
                ),
        )
        .find_also_related(role::Entity)
        .one(db)
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "db.query",
            db.operation = "select",
            db.table = "AppConnection"
        ))
        .await?
        .ok_or_else(|| {
            tracing::debug!(
                origin_app_id,
                target_app_id = app_id,
                "No active app connection found"
            );
            ApiError::FORBIDDEN
        })?;

    role.filter(|role| role.app_id.as_deref() == Some(app_id))
        .ok_or_else(|| {
            tracing::warn!(
                connection_id = %connection.id,
                "App connection is active but has no valid role for this app"
            );
            ApiError::FORBIDDEN
        })
}

async fn resolve_legacy_api_key_creator_user_id(
    state: &AppState,
    app_id: &str,
) -> Result<Option<String>, AuthorizationError> {
    let app = App::find_by_id(app_id)
        .one(&state.db)
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "db.query",
            db.operation = "select",
            db.table = "App"
        ))
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
            .instrument(tracing::info_span!(
                target: "flow_like::observability",
                "db.query",
                db.operation = "select",
                db.table = "Membership"
            ))
            .await?;

        if let Some(owner) = owner {
            return Ok(Some(owner.user_id));
        }
    }

    let role_permissions = || Expr::col((role::Entity, role::Column::Permissions));
    let oldest_owner = membership::Entity::find()
        .select_only()
        .column(membership::Column::UserId)
        .join(JoinType::InnerJoin, membership::Relation::Role.def())
        .filter(membership::Column::AppId.eq(app_id))
        .filter(
            role_permissions()
                .bit_and(RolePermissions::Owner.bits())
                .ne(0),
        )
        // Roles with unknown permission bits never counted as owners.
        .filter(
            role_permissions()
                .bit_and(!RolePermissions::all().bits())
                .eq(0),
        )
        .order_by_asc(membership::Column::CreatedAt)
        .into_tuple::<String>()
        .one(&state.db)
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "db.query",
            db.operation = "select",
            db.table = "Membership"
        ))
        .await?;

    Ok(oldest_owner)
}

pub async fn jwt_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response<Body>, AuthorizationError> {
    let request = authenticate_request(&state, request).await?;
    Ok(next.run(request).await)
}

fn reserved_credential_principal(token: &str) -> Option<AppUser> {
    crate::devices::jwt::is_device_credential(token).then_some(AppUser::Unauthorized)
}

/// The principal a current executor token stands for, with the token's expiry.
fn executor_principal(token: &str, now: i64) -> Option<(ExecutorUser, i64)> {
    let claims = crate::execution::verify_execution_jwt(token).ok()?;
    let exp = claims.exp;
    crate::backend_jwt::cached_token_is_current(exp, now).then(|| (ExecutorUser::from(claims), exp))
}

#[tracing::instrument(
    target = "flow_like::observability",
    name = "auth.authenticate",
    skip_all
)]
async fn authenticate_request(
    state: &AppState,
    request: Request,
) -> Result<Request, AuthorizationError> {
    let mut request = request;

    let client_ip = ClientIp(extract_client_ip(
        &request,
        state.platform_config.audit.trusted_proxy_hops,
    ));
    request.extensions_mut().insert(client_ip);

    // Try OpenID/JWT or Executor JWT auth
    if let Some(token) = viewer_authorization(request.headers())
        && !token.starts_with("pat_")
    {
        let token = token.strip_prefix("Bearer ").unwrap_or(token);
        let token = token.trim();
        // Device credentials have a separate principal and request proof. Never
        // reinterpret them as a human, even when an OIDC issuer shares our keys.
        if let Some(principal) = reserved_credential_principal(token) {
            request.extensions_mut().insert::<AppUser>(principal);
            return Ok(request);
        }
        let cache_key = hash_token(token);

        // Check cache first
        if let Some(cached) = state.auth_cache.get(&cache_key) {
            match cached {
                CachedAuth::OpenID { sub, exp } => {
                    if cached_openid_is_current(exp, chrono::Utc::now().timestamp()) {
                        let user = AppUser::OpenID(OpenIDUser {
                            sub,
                            access_token: token.to_string(),
                        });
                        request.extensions_mut().insert::<AppUser>(user);
                        return Ok(request);
                    }
                    // Expired cache entry: fall through to fresh validation,
                    // which honors the configured leeway and re-inserts on
                    // success.
                    state.auth_cache.invalidate(&cache_key);
                }
                CachedAuth::Executor { executor, exp } => {
                    if !crate::backend_jwt::cached_token_is_current(
                        exp,
                        chrono::Utc::now().timestamp(),
                    ) {
                        state.auth_cache.invalidate(&cache_key);
                        request
                            .extensions_mut()
                            .insert::<AppUser>(AppUser::Unauthorized);
                        return Ok(request);
                    }
                    request
                        .extensions_mut()
                        .insert::<AppUser>(AppUser::Executor(executor));
                    return Ok(request);
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
                        return Ok(request);
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
                    return Ok(request);
                }
                _ => {}
            }
        }

        // Cache miss - validate token
        let validated = state.validate_token(token).await;
        if let Ok(validated) = validated {
            if let Some(sub) = validated.claims.get("sub").and_then(|sub| sub.as_str()) {
                state.auth_cache.insert(
                    cache_key,
                    CachedAuth::OpenID {
                        sub: sub.to_string(),
                        exp: validated.expires_at,
                    },
                );

                let user = AppUser::OpenID(OpenIDUser {
                    sub: sub.to_string(),
                    access_token: token.to_string(),
                });
                request.extensions_mut().insert::<AppUser>(user);
                return Ok(request);
            }

            // A token that validates cryptographically but has no usable
            // subject is not an internal server failure. Continue trying the
            // other supported token formats; if none match, the request is
            // passed to the route as anonymous below.
            tracing::warn!("Validated OpenID token is missing a string sub claim");
        }

        // OpenID failed — try executor JWT
        if let Some((executor, exp)) = executor_principal(token, chrono::Utc::now().timestamp()) {
            state.auth_cache.insert(
                cache_key,
                CachedAuth::Executor {
                    executor: executor.clone(),
                    exp,
                },
            );
            request
                .extensions_mut()
                .insert::<AppUser>(AppUser::Executor(executor));
            return Ok(request);
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
            return Ok(request);
        }
    }

    // Try PAT auth
    if let Some(raw_token) = viewer_authorization(request.headers()) {
        // Strip "Bearer " prefix if present so PATs sent as standard Bearer tokens are recognized
        let token = raw_token.strip_prefix("Bearer ").unwrap_or(raw_token);
        let token = token.trim();

        if token.starts_with("pat_") {
            let pat_str = token;
            let db_pat = pat_validation::lookup_current(
                &state.db,
                pat_str,
                chrono::Utc::now().fixed_offset(),
            )
            .instrument(tracing::info_span!(
                target: "flow_like::observability",
                "db.query",
                db.operation = "select",
                db.table = "Pat"
            ))
            .await?;

            if let Some(pat) = db_pat {
                if crate::permission::pat_permission::PatPermission::from_bits(pat.permissions)
                    .is_none()
                {
                    return Err(ApiError::forbidden(
                        crate::permission::pat_permission::LEGACY_PERMISSION_ERROR,
                    ));
                }
                let path = request
                    .extensions()
                    .get::<axum::extract::OriginalUri>()
                    .map(|original| original.0.path())
                    .unwrap_or_else(|| request.uri().path());
                if !crate::permission::pat_permission::permits_route(
                    pat.permissions,
                    request.method().as_str(),
                    path,
                ) {
                    return Err(ApiError::forbidden(
                        "Personal access token does not permit this operation",
                    ));
                }

                let pat_user = AppUser::PAT(PATUser {
                    pat: pat_str.to_string(),
                    sub: pat.user_id.clone(),
                });
                request.extensions_mut().insert::<AppUser>(pat_user);
                return Ok(request);
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
                    return Ok(request);
                }
                CachedAuth::Invalid => {
                    request
                        .extensions_mut()
                        .insert::<AppUser>(AppUser::Unauthorized);
                    return Ok(request);
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
            return Ok(request);
        }

        let key_parts = &api_key_str[4..];
        let parts: Vec<&str> = key_parts.split('.').collect();
        if parts.len() != 3 {
            state.auth_cache.insert(cache_key, CachedAuth::Invalid);
            request
                .extensions_mut()
                .insert::<AppUser>(AppUser::Unauthorized);
            return Ok(request);
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
            .instrument(tracing::info_span!(
                target: "flow_like::observability",
                "db.query",
                db.operation = "select",
                db.table = "TechnicalUser"
            ))
            .await?;

        if let Some(app) = db_app {
            if app.app_id != app_id_from_key {
                state.auth_cache.insert(cache_key, CachedAuth::Invalid);
                request
                    .extensions_mut()
                    .insert::<AppUser>(AppUser::Unauthorized);
                return Ok(request);
            }

            if let Some(valid_until) = app.valid_until {
                let now = chrono::Utc::now().fixed_offset();
                if valid_until < now {
                    state.auth_cache.insert(cache_key, CachedAuth::Invalid);
                    request
                        .extensions_mut()
                        .insert::<AppUser>(AppUser::Unauthorized);
                    return Ok(request);
                }
            }

            let creator_user_id = match app.creator_user_id.clone() {
                Some(creator_user_id) => Some(creator_user_id),
                None => resolve_legacy_api_key_creator_user_id(state, &app.app_id).await?,
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
            return Ok(request);
        }
    }

    request
        .extensions_mut()
        .insert::<AppUser>(AppUser::Unauthorized);
    Ok(request)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use sea_orm::{
        ActiveValue::Set, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Schema,
        sea_query::Table,
    };

    async fn role_and_key_tables() -> DatabaseConnection {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        for source in [
            schema.create_table_from_entity(role::Entity),
            schema.create_table_from_entity(technical_user::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            for column in source.get_columns() {
                table.col(column.clone());
            }
            db.execute(&table).await.unwrap();
        }
        db
    }

    /// API keys of `app-1`: `current` and `unexpired` hold `role-1`; `expired`, `roleless`
    /// and the deleted `deleted` hold none.
    pub(crate) async fn api_keys() -> DatabaseConnection {
        let db = role_and_key_tables().await;
        let now = chrono::Utc::now().fixed_offset();
        role::Entity::insert(role::ActiveModel {
            id: Set("role-1".into()),
            name: Set("Runner".into()),
            permissions: Set(RolePermissions::ExecuteEvents.bits()),
            app_id: Set(Some("app-1".into())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();
        let hour = chrono::Duration::hours(1);
        for (id, role_id, valid_until) in [
            ("current", Some("role-1"), None),
            ("unexpired", Some("role-1"), Some(now + hour)),
            ("expired", Some("role-1"), Some(now - hour)),
            ("roleless", None, None),
            ("deleted", Some("role-1"), None),
        ] {
            technical_user::Entity::insert(technical_user::ActiveModel {
                id: Set(id.into()),
                name: Set(id.into()),
                key: Set(format!("{id}-hash")),
                role_id: Set(role_id.map(str::to_owned)),
                app_id: Set("app-1".into()),
                valid_until: Set(valid_until),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            })
            .exec(&db)
            .await
            .unwrap();
        }
        technical_user::Entity::delete_by_id("deleted")
            .exec(&db)
            .await
            .unwrap();
        db
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::sink::authority::RunAsDenial;

    #[test]
    fn device_tokens_never_expose_their_attributed_subject_to_ordinary_routes() {
        crate::backend_jwt::init_for_tests();
        for (header_type, payload_type) in [
            ("flow-like-device-enrollment+jwt", "device_enrollment"),
            ("flow-like-device-session+jwt", "device_session"),
            ("flow-like-device-future+jwt", "future_profile"),
            ("JWT", "instance_resource"),
            ("JWT", "device_future_profile"),
        ] {
            let token = crate::backend_jwt::sign_typed(
                &serde_json::json!({"sub":"attributed-account-owner","typ":payload_type}),
                header_type,
            )
            .unwrap();
            for credential in [token.clone(), format!("DPoP {token}")] {
                let principal = reserved_credential_principal(&credential)
                    .expect("reserved device credential is rejected before the user-auth cache");
                assert!(matches!(principal, AppUser::Unauthorized));
                assert!(principal.sub().is_err());
                assert!(principal.executor_scoped_sub().is_err());
                assert!(principal.effective_user_id().is_err());
                assert!(principal.entity().is_err());
            }
        }
        assert!(reserved_credential_principal("pat_ordinary.secret").is_none());
    }

    #[test]
    fn forwarded_ip_counts_trusted_hops_from_the_right() {
        let header = "203.0.113.9, 198.51.100.4 , 10.0.0.2";
        assert_eq!(forwarded_client_ip(header, None), Some("203.0.113.9"));
        assert_eq!(forwarded_client_ip(header, Some(1)), Some("10.0.0.2"));
        assert_eq!(forwarded_client_ip(header, Some(2)), Some("198.51.100.4"));
        assert_eq!(forwarded_client_ip(header, Some(4)), None);
        assert_eq!(forwarded_client_ip(header, Some(0)), None);
        assert_eq!(forwarded_client_ip(" , ", None), None);
    }
    use axum::{http::StatusCode, response::IntoResponse};

    #[flow_like_types::tokio::test]
    async fn api_key_audit_identity_does_not_require_a_human_creator() {
        let user = AppUser::APIKey(ApiKey {
            key_id: "key-1".into(),
            api_key: "private-token".into(),
            app_id: "app-1".into(),
            creator_user_id: None,
        });
        assert_eq!(user.audit_id().await.unwrap(), "api_key:app:app-1:key-1");
        assert!(user.effective_user_id().is_err());
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                axum::http::HeaderName::try_from(*name).unwrap(),
                value.parse().unwrap(),
            );
        }
        headers
    }

    fn role() -> RoleContext {
        RoleContext {
            id: "role-1".to_string(),
            name: "Runner".to_string(),
            permissions: 0b1000,
            attributes: vec!["runner".to_string()],
            custom_attributes: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn human_principals_keep_their_subject() {
        let context = user_context_from_parts(
            ExecutionPrincipal::User,
            Some("user-123"),
            Some("user-123"),
            "user-123",
            "",
            role(),
        );

        assert_eq!(context.sub, "user-123");
        assert!(!context.is_technical());
        assert_eq!(context.get_key_id(), None);
        assert!(context.on_behalf_of().is_none());
        assert!(context.has_attribute("runner"));
    }

    /// The key's creator is attribution, not an identity the run may act as, so
    /// it must never surface as the executing subject.
    #[test]
    fn api_keys_report_the_key_and_never_borrow_the_creator_subject() {
        let context = user_context_from_parts(
            ExecutionPrincipal::ApiKey,
            None,
            Some("creator-user"),
            "key-1",
            "",
            role(),
        );

        assert_eq!(context.principal, ExecutionPrincipal::ApiKey);
        assert!(context.is_technical());
        assert_eq!(context.sub, "");
        assert_eq!(context.get_key_id(), Some("key-1"));
        assert_eq!(context.on_behalf_of(), Some("creator-user"));
        assert_eq!(context.origin_app_id(), None);
    }

    /// An app calling through a connection used to be indistinguishable from an
    /// API key, down to a fabricated `app-connection::…` key id.
    #[test]
    fn connected_apps_are_not_reported_as_api_keys() {
        let context = user_context_from_parts(
            ExecutionPrincipal::ConnectedApp,
            None,
            Some("initiating-user"),
            &app_connection_cache_sub("origin-app"),
            "origin-app",
            role(),
        );

        assert_eq!(context.principal, ExecutionPrincipal::ConnectedApp);
        assert!(context.is_connected_app());
        assert!(context.is_technical());
        assert_eq!(context.get_key_id(), None);
        assert_eq!(context.origin_app_id(), Some("origin-app"));
        assert_eq!(context.sub, "");
        assert_eq!(context.on_behalf_of(), Some("initiating-user"));
    }

    #[test]
    fn viewer_authorization_prefers_the_forwarded_header() {
        let headers = headers(&[
            ("authorization", "AWS4-HMAC-SHA256 Credential=cloudfront"),
            (FORWARDED_AUTHORIZATION_HEADER, "Bearer viewer-token"),
        ]);
        assert_eq!(viewer_authorization(&headers), Some("Bearer viewer-token"));
    }

    #[test]
    fn viewer_authorization_falls_back_past_an_empty_forwarded_header() {
        let headers = headers(&[
            ("authorization", "Bearer viewer-token"),
            (FORWARDED_AUTHORIZATION_HEADER, "  "),
        ]);
        assert_eq!(viewer_authorization(&headers), Some("Bearer viewer-token"));
    }

    #[test]
    fn viewer_authorization_uses_authorization_when_nothing_is_forwarded() {
        let headers = headers(&[("authorization", "Bearer viewer-token")]);
        assert_eq!(viewer_authorization(&headers), Some("Bearer viewer-token"));
    }

    #[test]
    fn viewer_authorization_ignores_cloudfront_sigv4() {
        let headers = headers(&[(
            "authorization",
            "AWS4-HMAC-SHA256 Credential=cloudfront/20260817/eu-central-1/lambda/aws4_request",
        )]);
        assert_eq!(viewer_authorization(&headers), None);
    }

    #[test]
    fn viewer_authorization_is_none_when_both_headers_are_absent() {
        assert_eq!(viewer_authorization(&HeaderMap::new()), None);
    }

    #[test]
    fn anonymous_app_permission_denial_is_unauthorized() {
        let response = AppUser::Unauthorized
            .app_permission_denial()
            .into_response();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get("x-error-id").is_none());
    }

    fn executor_user(shadow: bool) -> AppUser {
        AppUser::Executor(ExecutorUser {
            sub: "user-id".to_string(),
            app_id: "app-id".to_string(),
            run_id: "run-id".to_string(),
            technical_user_id: None,
            app_chain: None,
            correlation: None,
            shadow,
            hosted_frontend: false,
            event_id: None,
            custom_bits_owner: None,
            unattended: false,
        })
    }

    #[test]
    fn authenticated_app_permission_denial_is_forbidden() {
        let response = executor_user(false).app_permission_denial().into_response();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(response.headers().get("x-error-id").is_none());
    }

    fn signed_params() -> crate::execution::ExecutionJwtParams {
        crate::execution::ExecutionJwtParams {
            user_id: "user-1".into(),
            technical_user_id: Some("key-1".into()),
            run_id: "run-1".into(),
            app_id: "app-1".into(),
            board_id: "board-1".into(),
            event_id: Some("event-1".into()),
            app_chain: Some(vec!["calling-app".into()]),
            correlation: Some(crate::correlation::CorrelationContext::root("root-run")),
            callback_url: "https://api.example".into(),
            token_type: crate::execution::TokenType::Executor,
            ttl_seconds: Some(60),
            shadow: Some(true),
        }
    }

    #[test]
    fn signed_claims_reach_the_executor_principal() {
        crate::backend_jwt::init_for_tests();
        let now = chrono::Utc::now().timestamp();
        let token = crate::execution::sign_execution_jwt(signed_params()).unwrap();
        let mut claims = crate::execution::verify_execution_jwt(&token).unwrap();
        claims.custom_bits_owner = Some("owner-1".into());
        let token = crate::backend_jwt::sign(&claims).unwrap();

        let (executor, exp) = executor_principal(&token, now).expect("a current executor token");
        assert_eq!(exp, claims.exp);
        assert_eq!(
            executor,
            ExecutorUser {
                sub: "user-1".into(),
                app_id: "app-1".into(),
                run_id: "run-1".into(),
                technical_user_id: Some("key-1".into()),
                app_chain: Some(vec!["calling-app".into()]),
                correlation: Some(crate::correlation::CorrelationContext::root("root-run")),
                shadow: true,
                hosted_frontend: false,
                event_id: Some("event-1".into()),
                custom_bits_owner: Some("owner-1".into()),
                unattended: false,
            }
        );
        assert!(AppUser::Executor(executor).is_shadow_execution());
        assert!(executor_principal(&token, exp + 3_600).is_none());

        let mut visitor = signed_params();
        visitor.user_id = "visitor-1".into();
        visitor.technical_user_id = None;
        visitor.app_chain = None;
        visitor.correlation = None;
        visitor.shadow = None;
        let visitor = crate::execution::sign_execution_jwt(visitor).unwrap();
        let (live, _) = executor_principal(&visitor, now).unwrap();
        assert!(!live.shadow && !live.hosted_frontend && !live.unattended);
        let hosted = crate::execution::bind_hosted_frontend(&visitor).unwrap();
        let (hosted, _) = executor_principal(&hosted, now).unwrap();
        assert!(hosted.hosted_frontend && !hosted.shadow && !hosted.unattended);
        assert_eq!(hosted.sub, "visitor-1");
        assert_eq!(hosted.event_id.as_deref(), Some("event-1"));
        assert!(hosted.custom_bits_owner.is_none());

        let mut fire = signed_params();
        fire.technical_user_id = None;
        fire.shadow = None;
        let fire = crate::execution::sign_unattended_execution_jwt(fire).unwrap();
        let (fire, _) = executor_principal(&fire, now).unwrap();
        assert!(fire.unattended && !fire.hosted_frontend && !fire.shadow);
        assert_eq!(fire.sub, "user-1");
        assert_eq!(fire.app_chain, Some(vec!["calling-app".into()]));

        let mut user_token = signed_params();
        user_token.token_type = crate::execution::TokenType::User;
        let user_token = crate::execution::sign_execution_jwt(user_token).unwrap();
        assert!(executor_principal(&user_token, now).is_none());
        assert!(executor_principal("not-a-token", now).is_none());
    }

    #[test]
    fn only_shadow_writes_are_refused() {
        let write = ExecutorUse::Write("change the app cache");
        let refusal = write.admit(&executor_user(true)).unwrap_err();
        assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
        assert_eq!(refusal.public_code(), SHADOW_SIDE_EFFECT_REFUSED);
        assert_eq!(
            refusal.public_message(),
            Some("Shadow and regression runs cannot change the app cache")
        );
        assert!(ExecutorUse::Read.admit(&executor_user(true)).is_ok());

        let others = [
            executor_user(false),
            AppUser::OpenID(OpenIDUser {
                sub: "user-id".into(),
                access_token: "token".into(),
            }),
            AppUser::PAT(PATUser {
                pat: "pat_pat-1.secret".into(),
                sub: "user-id".into(),
            }),
            AppUser::APIKey(ApiKey {
                key_id: "key-1".into(),
                api_key: "secret".into(),
                app_id: "app-id".into(),
                creator_user_id: Some("user-id".into()),
            }),
            AppUser::ConnectedApp(ConnectedAppUser {
                sub: Some("user-id".into()),
                origin_app_id: "origin-app".into(),
                target_app_id: "app-id".into(),
                app_chain: vec!["origin-app".into()],
                technical_user_id: None,
                run_id: Some("origin-run".into()),
                correlation: None,
            }),
        ];
        for user in &others {
            assert!(!user.is_shadow_execution());
            assert!(write.admit(user).is_ok());
            assert!(ExecutorUse::Read.admit(user).is_ok());
            assert!(user.deny_shadow_execution("change the app cache").is_ok());
        }
    }

    #[test]
    fn executor_use_is_admitted_before_any_authority_lookup() {
        let source = include_str!("jwt.rs");
        let body = source
            .split_once("\n    pub async fn execution_app_permission(")
            .expect("executor authority is resolved here")
            .1;
        let body = body.split_once("\n    }\n").map_or(body, |(body, _)| body);
        let position = |step: &str| {
            body.find(step)
                .unwrap_or_else(|| panic!("execution_app_permission calls {step}"))
        };
        let admit = position("executor_use.admit(self)?;");
        assert!(position("executor.app_id != app_id") < admit);
        assert!(position("return Err(ApiError::FORBIDDEN);") < admit);
        assert!(!body[..admit].contains(".await"));
        let lookups = [
            "connected_app_permission(",
            "api_key_executor_permission(",
            "hosted_executor_permission(",
            "member_executor_permission(",
        ]
        .map(position);
        assert!(admit < lookups[0]);
        assert!(lookups.is_sorted(), "chain, API key, hosted, then member");
    }

    #[test]
    fn api_key_runs_act_as_their_key() {
        let grantee = Grantee::api_key_run("creator-1", "key-1");
        assert_eq!(
            grantee,
            Grantee {
                sub: None,
                effective_user_id: Some("creator-1".into()),
                technical_user_id: Some("key-1".into()),
                identifier: "key-1".into(),
                principal: ExecutionPrincipal::ApiKey,
            }
        );
        let context = user_context_from_parts(
            grantee.principal,
            grantee.sub.as_deref(),
            grantee.effective_user_id.as_deref(),
            grantee.technical_user_id.as_deref().unwrap(),
            "",
            role(),
        );
        assert!(context.is_technical());
        assert_eq!(context.sub, "");
        assert_eq!(context.get_key_id(), Some("key-1"));
        assert_eq!(context.on_behalf_of(), Some("creator-1"));

        let member = Grantee::user("user-1");
        assert_eq!(member.sub.as_deref(), Some("user-1"));
        assert_eq!(member.principal, ExecutionPrincipal::User);
        assert_eq!(member.technical_user_id, None);
    }

    /// The callback column of the hosted identity table (the dispatch columns are in
    /// `invoke_event`): a signed-in visitor's run acts as the visitor, on their role when they
    /// are a member and only to run events when they are not, whatever the event's run-as user
    /// may do. An anonymous visitor's run may only run events, and only while that user may.
    /// Owner and Admin are never handed out.
    #[test]
    fn hosted_callbacks_act_as_the_signed_in_visitor_or_borrow_the_events_run_as_user() {
        use HostedAuthority::{Lent, Visitor};
        use RolePermissions as P;
        let (execute, read) = (P::ExecuteEvents, P::ReadTeam);
        let member = P::ReadTemplates | P::ExecuteEvents | P::ListEvents;
        let all = P::all().difference(P::Owner | P::Admin);
        let denied = Err(StatusCode::FORBIDDEN);
        for (case, authority, expected) in [
            ("owner", Visitor(Some(P::Owner)), Ok(all)),
            ("admin", Visitor(Some(P::Admin)), Ok(all)),
            ("member", Visitor(Some(member)), Ok(member)),
            ("reader", Visitor(Some(read)), Ok(read)),
            ("member, no rights", Visitor(Some(P::empty())), denied),
            ("non-member", Visitor(None), Ok(execute)),
            ("lent by owner", Lent(Some(P::Owner)), Ok(execute)),
            ("lent by admin", Lent(Some(P::Admin)), Ok(execute)),
            ("lent by member", Lent(Some(member)), Ok(execute)),
            ("lent by reader", Lent(Some(read)), denied),
            ("lent nothing", Lent(None), denied),
        ] {
            let granted = hosted_permissions(authority).map_err(|error| error.status());
            assert_eq!(granted, expected, "{case}");
            if let Ok(granted) = granted {
                assert!(!granted.intersects(P::Owner | P::Admin), "{case}");
                assert!(!has_role_permission(&granted, P::Admin), "{case}");
            }
        }
    }

    #[test]
    fn only_a_run_as_user_who_may_trigger_lends_authority() {
        let authority = |run_as: Option<&str>, verdict| {
            Some(SinkAuthority {
                run_as_user_id: run_as.map(str::to_owned),
                verdict,
            })
        };
        let owner = RunAsVerdict::Allowed(RolePermissions::Owner);
        assert_eq!(
            lent_authority(authority(Some("owner"), owner)).unwrap(),
            Some(RolePermissions::Owner)
        );
        for lent in [
            None,
            authority(None, RunAsVerdict::Allowed(RolePermissions::empty())),
            authority(None, RunAsVerdict::Denied(RunAsDenial::AppInactive)),
            authority(Some("left"), RunAsVerdict::Denied(RunAsDenial::NotMember)),
            authority(
                Some("viewer"),
                RunAsVerdict::Denied(RunAsDenial::CannotTrigger),
            ),
            authority(
                Some("banned"),
                RunAsVerdict::Denied(RunAsDenial::AccountInactive),
            ),
        ] {
            assert_eq!(lent_authority(lent.clone()).unwrap(), None, "{lent:?}");
        }
        let fault = lent_authority(authority(
            Some("corrupt"),
            RunAsVerdict::Transient("Invalid role permission bits"),
        ))
        .unwrap_err();
        assert_eq!(fault.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// A sink saved before run-as identities lends its anonymous hosted runs what the owner of
    /// its stored PAT holds, as the door ran them, and nothing once the PAT names nobody.
    #[test]
    fn legacy_sinks_lend_what_the_owner_of_their_token_holds() {
        use crate::routes::sink::{
            authority::{RunAsFacts, legacy_authority},
            trigger::PatRefusal,
        };
        use RolePermissions as P;
        let publisher = RunAsFacts {
            app_active: true,
            account_active: true,
            member: true,
            permissions: Some(P::Owner.bits()),
        };
        let lend = |owner| lent_authority(Some(legacy_authority(owner)));
        let lent = lend(Ok(("publisher".to_owned(), publisher))).unwrap();
        assert_eq!(lent, Some(P::Owner));
        assert_eq!(
            hosted_permissions(HostedAuthority::Lent(lent)).unwrap(),
            P::ExecuteEvents
        );

        let departed = RunAsFacts {
            member: false,
            permissions: None,
            ..publisher
        };
        for refused in [
            Ok(("departed".to_owned(), departed)),
            Err(PatRefusal::Invalid),
        ] {
            let lent = lend(refused.clone()).unwrap();
            assert_eq!(lent, None, "{refused:?}");
            assert_eq!(
                hosted_permissions(HostedAuthority::Lent(lent))
                    .unwrap_err()
                    .status(),
                StatusCode::FORBIDDEN,
                "{refused:?}"
            );
        }
        let fault = lend(Err(PatRefusal::Unreadable)).unwrap_err();
        assert_eq!(fault.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    /// An API-key run keeps its key's role only while the key exists, holds a role and is
    /// unexpired; an expired key is refused exactly like a deleted one.
    #[flow_like_types::tokio::test]
    async fn api_key_runs_lose_their_role_once_the_key_is_deleted_or_expires() {
        let db = test_support::api_keys().await;
        for key in ["current", "unexpired"] {
            let role = api_key_run_role(&db, "app-1", key, "run-1").await.unwrap();
            assert_eq!(role.id, "role-1", "{key}");
        }
        let deleted = api_key_run_role(&db, "app-1", "deleted", "run-1")
            .await
            .unwrap_err();
        assert_eq!(deleted.status(), StatusCode::FORBIDDEN);
        assert_eq!(deleted.public_message(), Some(API_KEY_RUN_REFUSED));
        for (app, key) in [
            ("app-1", "expired"),
            ("app-1", "roleless"),
            ("app-2", "current"),
        ] {
            assert!(
                api_key_role(&db, app, key).await.unwrap().is_none(),
                "{key} in {app}"
            );
            let refusal = api_key_run_role(&db, app, key, "run-1").await.unwrap_err();
            assert_eq!(refusal.status(), deleted.status(), "{key} in {app}");
            assert_eq!(
                refusal.public_message(),
                deleted.public_message(),
                "{key} in {app}"
            );
        }
    }

    #[test]
    fn hosted_visitors_get_a_synthetic_role_without_attributes() {
        let role = hosted_visitor_role("app-1", RolePermissions::ExecuteEvents);
        assert_eq!(role.id, HOSTED_VISITOR_ROLE_ID);
        assert_eq!(role.name, HOSTED_VISITOR_ROLE_NAME);
        assert_eq!(role.app_id.as_deref(), Some("app-1"));
        assert_eq!(role.permissions, RolePermissions::ExecuteEvents.bits());
        assert!(role.attributes.is_none());
    }

    fn connected_app(sub: Option<&str>, technical_user_id: Option<&str>) -> ConnectedAppUser {
        ConnectedAppUser {
            sub: sub.map(str::to_owned),
            origin_app_id: "app-a".into(),
            target_app_id: "app-b".into(),
            app_chain: vec!["app-a".into()],
            technical_user_id: technical_user_id.map(str::to_owned),
            run_id: Some("run-a".into()),
            correlation: None,
        }
    }

    #[test]
    fn a_connection_acts_for_the_account_it_passed_through_unless_a_key_started_the_chain() {
        for (sub, technical_user_id, person) in [
            (Some("user-1"), None, Some("user-1")),
            (Some(" user-1 "), None, Some("user-1")),
            (Some("user-1"), Some("key-1"), None),
            (Some("frontend:event-1:run-run-1"), None, None),
            (Some("run:run-1"), None, None),
            (Some("sink:sink-1"), None, None),
            (Some("inbound:event-1"), None, None),
            (Some("regression:suite-1"), None, None),
            (Some("local"), None, None),
            (None, None, None),
        ] {
            assert_eq!(
                connected_app(sub, technical_user_id).person(),
                person,
                "{sub:?} started by {technical_user_id:?}"
            );
        }
    }

    fn column_access(permissions: RolePermissions, attributes: &[&str]) -> ColumnAccess {
        let role = role::Model {
            attributes: Some(
                attributes
                    .iter()
                    .map(|attribute| attribute.to_string())
                    .collect(),
            ),
            ..hosted_visitor_role("app-b", permissions)
        };
        ColumnAccess::new(&role, permissions)
    }

    /// A call on behalf of a person reads a restricted column only when both the connection's
    /// role and the person's member role may: through an attribute both hold, an Owner or
    /// Admin role holding every attribute. A person who is not a member holds none.
    #[test]
    fn connection_calls_for_a_person_keep_only_the_attributes_both_roles_hold() {
        use RolePermissions as P;
        let reader = P::ReadFiles;
        let pii_finance = column_access(reader, &["pii", " finance ", ""]);
        let finance_hr = column_access(reader, &["finance", "hr"]);
        let bare = column_access(reader, &[]);
        let admin = column_access(P::Admin, &[]);
        let owner_ops = column_access(P::Owner, &["ops"]);
        for (case, connection, member, expected) in [
            (
                "shared",
                &pii_finance,
                Some(&finance_hr),
                Some(vec!["finance"]),
            ),
            ("member holds none", &pii_finance, Some(&bare), None),
            ("connection holds none", &bare, Some(&finance_hr), None),
            (
                "admin connection",
                &admin,
                Some(&finance_hr),
                Some(vec!["finance", "hr"]),
            ),
            (
                "owner member",
                &pii_finance,
                Some(&owner_ops),
                Some(vec!["finance", "pii"]),
            ),
            (
                "both privileged",
                &admin,
                Some(&owner_ops),
                Some(vec!["ops"]),
            ),
            ("not a member", &pii_finance, None, None),
            ("not a member, admin connection", &admin, None, None),
        ] {
            let expected: Option<Vec<String>> =
                expected.map(|list| list.into_iter().map(str::to_owned).collect());
            assert_eq!(
                connected_person_attributes(connection, member),
                expected,
                "{case}"
            );
        }
    }

    #[test]
    fn connection_calls_for_a_person_never_exceed_either_role() {
        use RolePermissions as P;
        let execute = P::ExecuteEvents;
        let run_and_read = execute | P::ReadFiles;
        let default_member = P::ReadTemplates | execute | P::ListEvents;
        let editor = run_and_read | P::WriteFiles | P::ReadBoards;
        let everything = P::all().difference(P::Owner | P::Admin);
        for (connection, member, expected) in [
            (run_and_read, Some(editor), run_and_read),
            (run_and_read, Some(P::Owner), run_and_read),
            (editor, Some(default_member), execute),
            (P::Admin, Some(default_member), default_member),
            (P::Owner, Some(P::Admin), everything),
            (editor, None, execute),
            (P::Admin, None, execute),
        ] {
            let granted = connected_person_permissions(connection, member).unwrap();
            assert_eq!(granted, expected, "{member:?} through {connection:?}");
            assert!(!granted.intersects(P::Owner | P::Admin), "{member:?}");
        }
        for (connection, member) in [
            (P::ReadFiles, None),
            (P::ReadFiles | P::ReadBoards, Some(execute | P::WriteFiles)),
            (editor, Some(P::empty())),
        ] {
            let refusal = connected_person_permissions(connection, member).unwrap_err();
            assert_eq!(
                refusal.status(),
                StatusCode::FORBIDDEN,
                "{member:?} through {connection:?}"
            );
        }
    }

    async fn connection_tables() -> sea_orm::DatabaseConnection {
        use sea_orm::{ConnectOptions, ConnectionTrait, Database, Schema, sea_query::Table};
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        let schema = Schema::new(db.get_database_backend());
        for source in [
            schema.create_table_from_entity(role::Entity),
            schema.create_table_from_entity(membership::Entity),
            schema.create_table_from_entity(app_connection::Entity),
        ] {
            let mut table = Table::create();
            table.table(source.get_table_name().unwrap().clone());
            for column in source.get_columns() {
                table.col(column.clone());
            }
            db.execute(&table).await.unwrap();
        }
        db
    }

    async fn add_role(
        db: &sea_orm::DatabaseConnection,
        id: &str,
        app_id: &str,
        permissions: RolePermissions,
        attributes: &[&str],
    ) {
        use sea_orm::ActiveValue::Set;
        let now = chrono::Utc::now().fixed_offset();
        role::Entity::insert(role::ActiveModel {
            id: Set(id.into()),
            name: Set(id.into()),
            permissions: Set(permissions.bits()),
            app_id: Set(Some(app_id.into())),
            created_at: Set(now),
            updated_at: Set(now),
            attributes: Set((!attributes.is_empty()).then(|| {
                attributes
                    .iter()
                    .map(|attribute| attribute.to_string())
                    .collect()
            })),
            ..Default::default()
        })
        .exec(db)
        .await
        .unwrap();
    }

    async fn add_member(db: &sea_orm::DatabaseConnection, user_id: &str, role_id: &str) {
        use sea_orm::ActiveValue::Set;
        let now = chrono::Utc::now().fixed_offset();
        membership::Entity::insert(membership::ActiveModel {
            id: Set(format!("{user_id}-in-app-b")),
            user_id: Set(user_id.into()),
            app_id: Set("app-b".into()),
            role_id: Set(role_id.into()),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(db)
        .await
        .unwrap();
    }

    async fn add_connection(
        db: &sea_orm::DatabaseConnection,
        source: &str,
        role_id: Option<&str>,
        status: sea_orm_active_enums::AppConnectionStatus,
    ) {
        use sea_orm::ActiveValue::Set;
        let now = chrono::Utc::now().fixed_offset();
        app_connection::Entity::insert(app_connection::ActiveModel {
            id: Set(format!("{source}-to-app-b")),
            source_app_id: Set(source.into()),
            target_app_id: Set("app-b".into()),
            role_id: Set(role_id.map(str::to_owned)),
            status: Set(status),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        })
        .exec(db)
        .await
        .unwrap();
    }

    /// `app-b` accepts connections from `app-a` (role `runner`), `app-f` (role `reader`),
    /// `app-p` (pending), `app-n` (no role) and `app-x` (a role of another app). Its members
    /// are `editor-1`, `owner-1` and `auditor-1`, each with the role of that name.
    async fn connected_apps() -> sea_orm::DatabaseConnection {
        use RolePermissions as P;
        use sea_orm_active_enums::AppConnectionStatus::{Active, Pending};
        let db = connection_tables().await;
        let run_and_read = P::ExecuteEvents | P::ReadFiles;
        let roles: [(&str, &str, RolePermissions, &[&str]); 6] = [
            (
                "runner",
                "app-b",
                run_and_read | P::ReadBoards,
                &["pii", "finance"],
            ),
            ("reader", "app-b", P::ReadFiles, &[]),
            (
                "editor",
                "app-b",
                run_and_read | P::WriteFiles,
                &["finance", "hr"],
            ),
            ("owner", "app-b", P::Owner, &["ops"]),
            ("auditor", "app-b", P::ReadLogs, &[]),
            ("foreign", "app-x", P::Owner, &[]),
        ];
        for (id, app_id, permissions, attributes) in roles {
            add_role(&db, id, app_id, permissions, attributes).await;
        }
        for (user_id, role_id) in [
            ("editor-1", "editor"),
            ("owner-1", "owner"),
            ("auditor-1", "auditor"),
        ] {
            add_member(&db, user_id, role_id).await;
        }
        for (source, role_id, status) in [
            ("app-a", Some("runner"), Active),
            ("app-f", Some("reader"), Active),
            ("app-p", Some("runner"), Pending),
            ("app-n", None, Active),
            ("app-x", Some("foreign"), Active),
        ] {
            add_connection(&db, source, role_id, status).await;
        }
        db
    }

    /// A caller from `origin` through its connection to `app-b`.
    fn caller_from(origin: &str, sub: Option<&str>, key: Option<&str>) -> ConnectedAppUser {
        ConnectedAppUser {
            origin_app_id: origin.into(),
            app_chain: vec![origin.into()],
            ..connected_app(sub, key)
        }
    }

    /// A call through a connection holds the connection's role. On behalf of a person it
    /// keeps only the permissions and column attributes that person also holds in the app,
    /// or only running events and no attribute when they are not a member. A chain an API
    /// key started, an anonymous visitor's run and a run nobody triggered act for nobody.
    #[flow_like_types::tokio::test]
    async fn connection_calls_hold_what_the_connection_and_the_person_share() {
        use RolePermissions as P;
        let db = connected_apps().await;
        let execute = P::ExecuteEvents;
        let run_and_read = execute | P::ReadFiles;
        let runner = run_and_read | P::ReadBoards;
        let visitor = Some("frontend:event-1:run-run-1");
        let unattended = Some("run:run-1");
        let connection: &[&str] = &["pii", "finance"];
        let from_a = |sub, key| caller_from("app-a", sub, key);
        for (caller, role_id, expected, attributes) in [
            (
                from_a(Some("editor-1"), None),
                "runner",
                run_and_read,
                &["finance"][..],
            ),
            (
                from_a(Some("owner-1"), None),
                "runner",
                runner,
                &["finance", "pii"],
            ),
            (from_a(Some("stranger-1"), None), "runner", execute, &[]),
            (
                from_a(Some("editor-1"), Some("key-1")),
                "runner",
                runner,
                connection,
            ),
            (from_a(visitor, None), "runner", runner, connection),
            (from_a(unattended, None), "runner", runner, connection),
            (from_a(None, None), "runner", runner, connection),
            (
                caller_from("app-f", None, None),
                "reader",
                P::ReadFiles,
                &[],
            ),
        ] {
            let role = connected_app_role(&db, &caller, "app-b").await.unwrap();
            assert_eq!(role.id, role_id, "{caller:?}");
            assert_eq!(role.permissions, expected.bits(), "{caller:?}");
            let held: Vec<&str> = role
                .attributes
                .iter()
                .flatten()
                .map(String::as_str)
                .collect();
            assert_eq!(held, attributes, "{caller:?}");
        }
    }

    /// A connection that is pending, has no role of the app or does not exist grants
    /// nothing, and neither does one that shares no permission with the person it acts for.
    #[flow_like_types::tokio::test]
    async fn connection_calls_without_a_current_role_or_a_shared_permission_are_refused() {
        let db = connected_apps().await;
        for caller in [
            caller_from("app-a", Some("auditor-1"), None),
            caller_from("app-f", Some("stranger-1"), None),
            caller_from("app-p", None, None),
            caller_from("app-n", None, None),
            caller_from("app-x", None, None),
            caller_from("app-z", Some("owner-1"), None),
        ] {
            let refusal = connected_app_role(&db, &caller, "app-b").await.unwrap_err();
            assert_eq!(refusal.status(), StatusCode::FORBIDDEN, "{caller:?}");
        }
    }

    /// A person a connection passed through stands in the target app while the connection
    /// grants them something: a stranger through a connection that may run events, a member
    /// through one that shares a permission with their role. A connection that only reads,
    /// that is pending, roleless or gone, or that shares nothing with the member, does not.
    #[flow_like_types::tokio::test]
    async fn a_connection_holds_a_person_only_while_it_grants_them_standing() {
        let db = connected_apps().await;
        for (origin, person, holds) in [
            ("app-a", "stranger-1", true),
            ("app-a", "editor-1", true),
            ("app-a", "owner-1", true),
            ("app-a", "auditor-1", false),
            ("app-f", "stranger-1", false),
            ("app-p", "stranger-1", false),
            ("app-n", "stranger-1", false),
            ("app-x", "stranger-1", false),
            ("app-z", "owner-1", false),
        ] {
            assert_eq!(
                connection_holds_person(&db, origin, "app-b", person)
                    .await
                    .unwrap(),
                holds,
                "{person} through {origin}"
            );
        }
    }

    #[test]
    fn stateless_lambda_fresh_page_pat_validation_binds_format_secret_subject_and_expiry() {
        let (id, secret_hash) = pat_lookup_parts("pat_pat-1.secret").unwrap();
        assert_eq!(id, "pat-1");
        assert_eq!(secret_hash, blake3::hash(b"secret").to_hex().to_string());
        for malformed in [
            "pat_",
            "pat_only-id",
            "pat_.secret",
            "pat_pat-1.",
            "pat_pat-1.secret.extra",
            "not-a-pat",
        ] {
            assert!(pat_lookup_parts(malformed).is_none(), "{malformed}");
        }

        let now = chrono::DateTime::from_timestamp(1_800_000_000, 0)
            .unwrap()
            .fixed_offset();
        let record = pat::Model {
            id: "pat-1".into(),
            name: "Page runtime".into(),
            key: secret_hash,
            permissions: 1,
            user_id: "user-1".into(),
            valid_until: Some(now + chrono::Duration::minutes(1)),
            created_at: now,
            updated_at: now,
        };
        assert!(pat_is_current(&record, "user-1", now));
        assert!(!pat_is_current(&record, "user-2", now));

        let mut expired = record;
        expired.valid_until = Some(now);
        assert!(pat_is_current(&expired, "user-1", now));
        expired.valid_until = Some(now - chrono::Duration::milliseconds(1));
        assert!(!pat_is_current(&expired, "user-1", now));
    }

    #[test]
    fn stateless_lambda_fresh_page_api_key_validation_binds_format_app_id_key_id_and_secret() {
        let (app_id, key_id, secret_hash) = api_key_lookup_parts("flk_app-1.key-1.secret").unwrap();
        assert_eq!(app_id, "app-1");
        assert_eq!(key_id, "key-1");
        assert_eq!(secret_hash, blake3::hash(b"secret").to_hex().to_string());
        for malformed in [
            "flk_",
            "flk_app-1.key-1",
            "flk_.key-1.secret",
            "flk_app-1..secret",
            "flk_app-1.key-1.",
            "flk_app-1.key-1.secret.extra",
            "not-an-api-key",
        ] {
            assert!(api_key_lookup_parts(malformed).is_none(), "{malformed}");
        }

        let now = chrono::DateTime::from_timestamp(1_800_000_000, 0)
            .unwrap()
            .fixed_offset();
        let cached = ApiKey {
            key_id: "key-1".into(),
            api_key: "flk_app-1.key-1.secret".into(),
            app_id: "app-1".into(),
            creator_user_id: Some("creator-1".into()),
        };
        let mut record = technical_user::Model {
            id: "key-1".into(),
            name: "Page runtime".into(),
            description: None,
            key: secret_hash,
            role_id: Some("role-1".into()),
            app_id: "app-1".into(),
            valid_until: Some(now + chrono::Duration::minutes(1)),
            created_at: now,
            updated_at: now,
            creator_membership_id: None,
            creator_user_id: Some("creator-1".into()),
        };
        assert!(api_key_record_is_current(
            &record,
            &cached,
            Some("creator-1"),
            now
        ));
        assert!(!api_key_record_is_current(
            &record,
            &cached,
            Some("creator-2"),
            now
        ));
        record.valid_until = Some(now - chrono::Duration::milliseconds(1));
        assert!(!api_key_record_is_current(
            &record,
            &cached,
            Some("creator-1"),
            now
        ));
    }

    mod postgres {
        use super::*;
        use sea_orm::{
            ConnectionTrait, Database, DatabaseConnection, DatabaseTransaction, DbBackend,
            Statement, TransactionTrait,
        };

        const SCHEMA: &str = r#"
            CREATE TABLE public."App" (id text PRIMARY KEY, status text NOT NULL);
            CREATE TABLE public."User" (id text PRIMARY KEY, status text NOT NULL);
            CREATE TABLE public."Role" (
                id text PRIMARY KEY, name text NOT NULL, description text,
                permissions bigint NOT NULL, "appId" text,
                "createdAt" timestamptz(3) NOT NULL DEFAULT now(),
                "updatedAt" timestamptz(3) NOT NULL DEFAULT now(), attributes jsonb
            );
            CREATE TABLE public."Membership" (
                id text PRIMARY KEY, "userId" text NOT NULL, "appId" text NOT NULL, "roleId" text NOT NULL
            );
            CREATE TABLE public."EventSink" (
                id text PRIMARY KEY, "eventId" text NOT NULL UNIQUE, "appId" text NOT NULL,
                active boolean NOT NULL, "runAsUserId" text, "patEncrypted" text
            );
            CREATE TABLE public."PAT" (
                id text PRIMARY KEY, key text NOT NULL UNIQUE, permissions bigint NOT NULL,
                "userId" text NOT NULL, "validUntil" timestamptz(3)
            );
            INSERT INTO public."App" VALUES ('app', 'ACTIVE');
            INSERT INTO public."User" VALUES
                ('publisher', 'ACTIVE'), ('member', 'ACTIVE'), ('reader', 'ACTIVE'),
                ('outsider', 'ACTIVE'), ('departed', 'ACTIVE');
            INSERT INTO public."Membership" VALUES
                ('m-publisher', 'publisher', 'app', 'role-owner'),
                ('m-member', 'member', 'app', 'role-member'),
                ('m-reader', 'reader', 'app', 'role-reader');
            INSERT INTO public."EventSink" VALUES
                ('s-live', 'live', 'app', TRUE, 'publisher'),
                ('s-paused', 'paused', 'app', FALSE, 'publisher'),
                ('s-departed', 'departed', 'app', TRUE, 'departed'),
                ('s-legacy', 'legacy', 'app', TRUE, NULL);
        "#;

        const MEMBER: RolePermissions = RolePermissions::ReadTemplates
            .union(RolePermissions::ExecuteEvents)
            .union(RolePermissions::ListEvents);
        const KEY: [u8; 32] = [7; 32];

        async fn seeded() -> (DatabaseConnection, DatabaseTransaction) {
            let url = std::env::var("FLOW_LIKE_SINK_TEST_DATABASE_URL").unwrap();
            let db = Database::connect(url).await.unwrap();
            let txn = db.begin().await.unwrap();
            txn.execute_unprepared(SCHEMA).await.unwrap();
            for (id, permissions) in [
                ("role-owner", RolePermissions::Owner),
                ("role-member", MEMBER),
                ("role-reader", RolePermissions::ReadTeam),
            ] {
                txn.execute_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    r#"INSERT INTO public."Role" (id, name, permissions, "appId") VALUES ($1, $1, $2, 'app')"#,
                    [id.into(), permissions.bits().into()],
                ))
                .await
                .unwrap();
            }
            (db, txn)
        }

        fn hosted_run(visitor: &str, event_id: &str) -> ExecutorUser {
            ExecutorUser {
                sub: visitor.into(),
                app_id: "app".into(),
                run_id: "run".into(),
                technical_user_id: None,
                app_chain: None,
                correlation: None,
                shadow: false,
                hosted_frontend: true,
                event_id: Some(event_id.into()),
                custom_bits_owner: None,
                unattended: false,
            }
        }

        /// A signed-in visitor's callbacks read the visitor's role and never the event's sink,
        /// whether it is on, paused, runs as someone who left or is missing. An anonymous
        /// visitor's callbacks read the user the sink runs as on every callback.
        #[flow_like_types::tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn hosted_runs_act_as_signed_in_visitors_and_borrow_the_run_as_user_otherwise() {
            let (_db, txn) = seeded().await;
            let execute = Ok(RolePermissions::ExecuteEvents);
            let reader = Ok(RolePermissions::ReadTeam);
            let forbidden = Err(StatusCode::FORBIDDEN);
            let anonymous = "frontend:live:anonymous-session-1";
            for (visitor, event, expected) in [
                (anonymous, "live", execute),
                (anonymous, "paused", forbidden),
                (anonymous, "departed", forbidden),
                (anonymous, "legacy", forbidden),
                (anonymous, "unknown", forbidden),
                ("member", "live", Ok(MEMBER)),
                ("reader", "live", reader),
                ("outsider", "live", execute),
                ("member", "paused", Ok(MEMBER)),
                ("outsider", "paused", execute),
                ("member", "departed", Ok(MEMBER)),
                ("reader", "unknown", reader),
            ] {
                let granted =
                    hosted_executor_permissions(&txn, &KEY, &hosted_run(visitor, event), "app")
                        .await
                        .map_err(|error| error.status());
                assert_eq!(granted, expected, "{visitor} on {event}");
            }
            txn.rollback().await.unwrap();
        }

        /// The door runs a legacy sink's anonymous hosted visit as the owner of its stored PAT,
        /// so its callbacks get the same lent authority, and none once the PAT names nobody. A
        /// signed-in visitor's callbacks never read the PAT.
        #[flow_like_types::tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn hosted_runs_on_legacy_sinks_act_for_the_owner_of_their_token() {
            let (_db, txn) = seeded().await;
            seed_legacy_sinks(&txn).await;
            let anonymous = "frontend:legacy-pat:anonymous-session-1";
            for (visitor, event, expected) in [
                (anonymous, "legacy-pat", Ok(RolePermissions::ExecuteEvents)),
                (anonymous, "legacy-revoked", Err(StatusCode::FORBIDDEN)),
                (
                    anonymous,
                    "legacy-unreadable",
                    Err(StatusCode::SERVICE_UNAVAILABLE),
                ),
                ("member", "legacy-pat", Ok(MEMBER)),
                ("reader", "legacy-pat", Ok(RolePermissions::ReadTeam)),
                ("member", "legacy-revoked", Ok(MEMBER)),
                ("member", "legacy-unreadable", Ok(MEMBER)),
            ] {
                let granted =
                    hosted_executor_permissions(&txn, &KEY, &hosted_run(visitor, event), "app")
                        .await
                        .map_err(|error| error.status());
                assert_eq!(granted, expected, "{visitor} on {event}");
            }
            txn.rollback().await.unwrap();
        }

        /// Legacy sinks whose PAT names the publisher, names a deleted PAT, or cannot be
        /// decrypted.
        async fn seed_legacy_sinks(txn: &DatabaseTransaction) {
            txn.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                r#"INSERT INTO public."PAT" (id, key, permissions, "userId") VALUES ('pat-1', $1, $2, 'publisher')"#,
                [
                    blake3::hash(b"secret").to_hex().to_string().into(),
                    crate::permission::pat_permission::PatPermission::ReadWrite
                        .bits()
                        .into(),
                ],
            ))
            .await
            .unwrap();
            let stored = |token: &str| crate::routes::app::events::db::encrypt_token(token, &KEY);
            for (event, ciphertext) in [
                ("legacy-pat", stored("pat_pat-1.secret")),
                ("legacy-revoked", stored("pat_pat-2.secret")),
                ("legacy-unreadable", "not-a-ciphertext".to_owned()),
            ] {
                txn.execute_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    r#"INSERT INTO public."EventSink" (id, "eventId", "appId", active, "patEncrypted")
                       VALUES ($1, $1, 'app', TRUE, $2)"#,
                    [event.into(), ciphertext.into()],
                ))
                .await
                .unwrap();
            }
        }
    }
}
