//! Authoritative widget policy for web hosts: describe what a package widget
//! requests, optionally including runtime sources the host extracted from its
//! props, and mint a grant for the policy the viewer approved.
//!
//! The policy is always derived from the version row's stored contract, which
//! publish compared against the bundle's `contract.json`. Callers never supply
//! a policy; minting only accepts the digest of the one this server derives.
//! Runtime sources are never stored: a version 2 grant binds their digest and
//! the host carries the accepted slots in the frame URL as `{token}~{runtime}`,
//! which the document route re-derives on every request.

use super::widget_asset::authorize_widget_version;
use super::widget_grant_jwt::{
    MAX_WIDGET_GRANT_TTL_SECONDS, WidgetGrantParams, WidgetGrantRuntime, sign_widget_grant,
};
use super::widget_sandbox::{
    WIDGET_SANDBOX_ROUTE, bundle_sources, request_authority, serving_origins,
};
use crate::backend_jwt;
use crate::entity::wasm_package_version;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use crate::storage_identity::{BucketIdentity, StorageProviderKind};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use flow_like::hub::{Hub, HubWidgetStorage};
use flow_like_wasm_schema::widget::WidgetContract;
use flow_like_wasm_schema::widget_frame::{
    encode_runtime_component, is_frame_widget_id, is_valid_package_id,
};
use flow_like_wasm_schema::widget_policy::{
    EngineGate, PlatformStorageScope, WIDGET_POLICY_SOURCE_HUB, WidgetPolicyDescriptor,
    WidgetPolicySubject, WidgetRuntimeContext, WidgetRuntimeSourceRequest, engine_from_user_agent,
    is_valid_widget_app_id, reserved_host, validate_runtime_request_shape, widget_engine_gate,
};
use flow_like_wasm_schema::widget_sources::{
    WidgetNetwork, describe_network, warm_widget_source_data, widget_source_data_versions,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;
use utoipa::{IntoParams, ToSchema};

/// Error code of a mint whose digest no longer matches the derived policy.
pub const POLICY_CHANGED_CODE: &str = "POLICY_CHANGED";
/// Error code of a mint on a deployment without a backend signing key.
pub const WIDGET_GRANTS_UNAVAILABLE_CODE: &str = "WIDGET_GRANTS_UNAVAILABLE";
/// Error code of runtime sources outside the request shape limits.
pub const INVALID_RUNTIME_SOURCES_CODE: &str = "INVALID_RUNTIME_SOURCES";
/// Error code of runtime sources sent for a store preview.
pub const RUNTIME_SOURCES_IN_PREVIEW_CODE: &str = "RUNTIME_SOURCES_IN_PREVIEW";
/// Error code of an app id that cannot name a platform-storage scope.
pub const INVALID_APP_ID_CODE: &str = "INVALID_APP_ID";
/// Error code of a describe or mint body that is not the expected JSON object.
pub const INVALID_WIDGET_REQUEST_CODE: &str = "INVALID_WIDGET_REQUEST";
/// Body limit of describe and mint requests.
pub const MAX_WIDGET_POLICY_REQUEST_BYTES: usize = 16 * 1024;
/// Comma-separated alternate names of this deployment that widget sources may
/// never target (CDN distribution, function URL, gateway endpoint). A leading
/// `*.` is accepted and means the name and everything under it.
pub const WIDGET_RESERVED_HOSTS_ENV: &str = "WIDGET_RESERVED_HOSTS";
const DESCRIPTOR_CACHE_NAMESPACE: &str = "widget-policy/v2";
const STORED_WIDGET_CACHE_NAMESPACE: &str = "widget-contract/v1";
const PRIVATE_NO_STORE: &str = "private, no-store";
const NO_STORE: &str = "no-store";
const PLATFORM_APPS_SEGMENT: &str = "apps";

static CONFIGURED_RESERVED_HOSTS: LazyLock<Vec<String>> = LazyLock::new(|| {
    std::env::var(WIDGET_RESERVED_HOSTS_ENV)
        .map(|value| parse_reserved_hosts(&value))
        .unwrap_or_default()
});

/// One widget of one package version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetTarget<'a> {
    pub package_id: &'a str,
    pub version: &'a str,
    pub widget_id: &'a str,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct WidgetPolicyQuery {
    /// `true` describes the store preview, which never gets network sites,
    /// media or microphone access.
    #[serde(default)]
    pub preview: bool,
}

/// Describe a widget including the addresses the host found in its inputs.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetPolicyDescribeRequest {
    #[serde(default)]
    pub preview: bool,
    /// App the widget runs in. Scopes this app's Flow-Like storage and binds
    /// the runtime sources.
    #[serde(default)]
    pub app_id: Option<String>,
    /// Origins per declared network input, as the host extracted them.
    #[serde(default)]
    pub runtime_sources: Vec<WidgetRuntimeSourceRequest>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetGrantRequest {
    pub version: String,
    pub widget_id: String,
    #[serde(default)]
    pub preview: bool,
    /// Digest of the descriptor the viewer approved.
    pub policy_digest: String,
    /// The `appId` of the approved descriptor.
    #[serde(default)]
    pub app_id: Option<String>,
    /// The exact `runtimeSources` of the approved descriptor.
    #[serde(default)]
    pub runtime_sources: Vec<WidgetRuntimeSourceRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetGrantResponse {
    /// Grant segment for the widget sandbox URLs; `null` means run at baseline
    /// (use `0`).
    pub grant: Option<String>,
    /// Seconds the grant stays valid.
    pub expires_in: i64,
    pub policy_digest: String,
    /// Runtime component of the sandbox URLs: frame and document paths use
    /// `{grant}~{runtime}`. `null` without accepted runtime sources.
    pub runtime: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MintDecision {
    PolicyChanged,
    Baseline,
    Unavailable,
    Sign,
}

pub fn mint_decision(
    descriptor: &WidgetPolicyDescriptor,
    requested_digest: &str,
    signing_configured: bool,
) -> MintDecision {
    if descriptor.policy_digest != requested_digest {
        MintDecision::PolicyChanged
    } else if !descriptor.is_ok() || descriptor.policy.is_empty() {
        MintDecision::Baseline
    } else if !signing_configured {
        MintDecision::Unavailable
    } else {
        MintDecision::Sign
    }
}

/// A client bug in a describe or mint request, answered with 400.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetRequestError {
    pub code: &'static str,
    pub message: String,
}

impl WidgetRequestError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn into_response(self) -> Response {
        widget_error_response(StatusCode::BAD_REQUEST, self.code, &self.message)
    }
}

/// Parses a describe or mint body. A `runtimeSources` value that is not a list
/// of `{ slot, sources: string[] }` is `INVALID_RUNTIME_SOURCES`; anything else
/// that does not parse is `INVALID_WIDGET_REQUEST`.
pub fn parse_widget_request<T: DeserializeOwned>(body: &[u8]) -> Result<T, WidgetRequestError> {
    serde_json::from_slice(body).map_err(|error| {
        let runtime_sources_malformed = serde_json::from_slice::<Value>(body)
            .ok()
            .and_then(|value| value.get("runtimeSources").cloned())
            .is_some_and(|value| {
                serde_json::from_value::<Vec<WidgetRuntimeSourceRequest>>(value).is_err()
            });
        if runtime_sources_malformed {
            WidgetRequestError::new(
                INVALID_RUNTIME_SOURCES_CODE,
                format!("invalid_runtime_sources: {error}"),
            )
        } else {
            WidgetRequestError::new(
                INVALID_WIDGET_REQUEST_CODE,
                format!("invalid widget policy request: {error}"),
            )
        }
    })
}

/// Shape, preview and app id rules shared by describe and mint (§14.4.4).
pub fn validate_runtime_request(
    preview: bool,
    app_id: Option<&str>,
    runtime_sources: &[WidgetRuntimeSourceRequest],
) -> Result<(), WidgetRequestError> {
    validate_runtime_request_shape(runtime_sources).map_err(|reason| {
        WidgetRequestError::new(
            INVALID_RUNTIME_SOURCES_CODE,
            format!("invalid_runtime_sources: {reason}"),
        )
    })?;
    if preview
        && runtime_sources
            .iter()
            .any(|entry| !entry.sources.is_empty())
    {
        return Err(WidgetRequestError::new(
            RUNTIME_SOURCES_IN_PREVIEW_CODE,
            "Store previews never get network access; send runtimeSources only without preview",
        ));
    }
    if let Some(app_id) = app_id.filter(|app_id| !is_valid_widget_app_id(app_id)) {
        return Err(WidgetRequestError::new(
            INVALID_APP_ID_CODE,
            format!("App id {app_id:?} must match [A-Za-z0-9_-]{{1,64}}"),
        ));
    }
    Ok(())
}

/// Entries of [`WIDGET_RESERVED_HOSTS_ENV`], without a leading `*.`.
pub fn parse_reserved_hosts(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|entry| {
            let entry = entry.trim();
            entry.strip_prefix("*.").unwrap_or(entry).to_string()
        })
        .filter(|entry| !entry.is_empty())
        .collect()
}

/// Hosts widget CSP sources may not target: configured hub and frontend hosts
/// plus the authority of the current request, normalized and deduplicated.
pub fn widget_reserved_hosts(
    configured: &[Option<&str>],
    request_authority: Option<&str>,
) -> Vec<String> {
    let mut hosts: Vec<String> = configured
        .iter()
        .copied()
        .chain([request_authority])
        .flatten()
        .filter_map(reserved_host)
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

fn configured_hosts(state: &AppState) -> Vec<Option<&str>> {
    let config = &state.platform_config;
    [
        Some(config.domain.as_str()),
        config.app.as_deref(),
        config.web.as_deref(),
    ]
    .into_iter()
    .chain(
        CONFIGURED_RESERVED_HOSTS
            .iter()
            .map(|host| Some(host.as_str())),
    )
    .collect()
}

fn request_reserved_hosts(state: &AppState, headers: &HeaderMap) -> Vec<String> {
    widget_reserved_hosts(
        &configured_hosts(state),
        request_authority(headers).as_deref(),
    )
}

/// Whether the request came through one of the deployment's own hosts (or
/// named none).
fn is_configured_authority(configured: &[Option<&str>], authority: Option<&str>) -> bool {
    let Some(authority) = authority else {
        return true;
    };
    let Some(host) = reserved_host(authority) else {
        return false;
    };
    configured
        .iter()
        .flatten()
        .filter_map(|value| reserved_host(value))
        .any(|configured| configured == host)
}

/// Platform-storage scopes this hub advertises as `widget_storage`.
pub fn hub_platform_storage(hub: &Hub) -> Vec<PlatformStorageScope> {
    hub.widget_storage
        .iter()
        .map(|scope| PlatformStorageScope {
            origin: scope.origin.clone(),
            path_prefix: scope.path_prefix.clone(),
        })
        .collect()
}

/// The widget engine gate for the browser that sent this request.
pub fn request_engine_gate(headers: &HeaderMap) -> EngineGate {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    widget_engine_gate(&engine_from_user_agent(user_agent))
}

/// Deployment and request facts every derivation of one package version's
/// widgets depends on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetRequestContext {
    pub reserved_hosts: Vec<String>,
    pub platform_storage: Vec<PlatformStorageScope>,
    /// Sandbox route prefixes on every serving origin of this request.
    pub bundle_sources: Vec<String>,
    pub engine: EngineGate,
    /// Only requests through a configured host share the descriptor memo; a
    /// client-chosen `X-Forwarded-Host` must not mint new memo entries.
    pub memoize: bool,
}

impl WidgetRequestContext {
    pub fn from_request(
        state: &AppState,
        headers: &HeaderMap,
        package_id: &str,
        version: &str,
    ) -> Self {
        let origins = serving_origins(
            &state.platform_config.domain,
            state.platform_config.secure,
            headers,
        );
        Self {
            reserved_hosts: request_reserved_hosts(state, headers),
            platform_storage: hub_platform_storage(&state.platform_config),
            bundle_sources: bundle_sources(&origins, WIDGET_SANDBOX_ROUTE, package_id, version),
            engine: request_engine_gate(headers),
            memoize: is_configured_authority(
                &configured_hosts(state),
                request_authority(headers).as_deref(),
            ),
        }
    }

    pub fn runtime<'a>(&'a self, app_id: Option<&'a str>) -> WidgetRuntimeContext<'a> {
        WidgetRuntimeContext {
            reserved_hosts: &self.reserved_hosts,
            platform_storage: &self.platform_storage,
            app_id,
            bundle_sources: &self.bundle_sources,
            engine: self.engine,
        }
    }
}

/// A widget's stored contract and bundle hash. Version rows are immutable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredWidget {
    pub bundle_hash: String,
    /// `Err` when the stored contract is missing or unreadable.
    pub contract: Result<WidgetContract, String>,
}

impl StoredWidget {
    pub fn subject(&self, target: WidgetTarget<'_>, preview: bool) -> WidgetPolicySubject {
        WidgetPolicySubject {
            source: WIDGET_POLICY_SOURCE_HUB.to_string(),
            package_id: target.package_id.to_string(),
            package_version: Some(target.version.to_string()),
            bundle_hash: self.bundle_hash.clone(),
            widget_id: target.widget_id.to_string(),
            preview,
        }
    }
}

/// The stored contract of `widget_id` in a version row's `widgets` JSON:
/// `None` when the version has no such widget, `Err` when it cannot be read.
pub fn stored_widget_contract(
    widgets: &Value,
    widget_id: &str,
) -> Option<Result<WidgetContract, String>> {
    let entry = widgets
        .as_array()?
        .iter()
        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(widget_id))?;
    Some(
        entry
            .get("contract")
            .cloned()
            .ok_or_else(|| format!("Stored widget '{widget_id}' has no contract"))
            .and_then(|contract| {
                serde_json::from_value(contract).map_err(|error| {
                    format!("Stored contract of widget '{widget_id}' is unreadable: {error}")
                })
            }),
    )
}

/// Derives the descriptor of a stored contract (§14.4.5); an unreadable
/// contract is `invalid` at the baseline policy.
pub fn describe_stored_widget(
    subject: WidgetPolicySubject,
    contract: &Result<WidgetContract, String>,
    request: &[WidgetRuntimeSourceRequest],
    context: &WidgetRuntimeContext<'_>,
) -> WidgetPolicyDescriptor {
    match contract {
        Ok(contract) => {
            WidgetPolicyDescriptor::describe_with_runtime(subject, contract, request, context)
        }
        Err(reason) => WidgetPolicyDescriptor::invalid(subject, reason.clone()),
    }
}

/// Adds the display-only `network` classification.
pub fn with_network(
    mut descriptor: WidgetPolicyDescriptor,
    contract: &Result<WidgetContract, String>,
) -> WidgetPolicyDescriptor {
    if let Ok(contract) = contract {
        let (purposes, runtime) = descriptor.classifier_inputs(contract);
        descriptor.network = describe_network(&purposes, &runtime, chrono::Utc::now().timestamp());
    }
    descriptor
}

/// Declared-only classification of a contract for store listings, `None`
/// without `csp` or when the contract does not describe.
pub fn declared_widget_network(contract: &WidgetContract) -> Option<WidgetNetwork> {
    contract.csp.as_ref()?;
    let subject = WidgetPolicySubject {
        source: WIDGET_POLICY_SOURCE_HUB.to_string(),
        package_id: String::new(),
        package_version: None,
        bundle_hash: String::new(),
        widget_id: contract.id.clone(),
        preview: false,
    };
    let contract = Ok(contract.clone());
    let descriptor = describe_stored_widget(
        subject,
        &contract,
        &[],
        &WidgetRuntimeContext {
            reserved_hosts: &[],
            platform_storage: &[],
            app_id: None,
            bundle_sources: &[],
            engine: EngineGate::OPEN,
        },
    );
    with_network(descriptor, &contract).network
}

/// Reads the stored widget of a version row, memoized per target. Callers
/// must have passed [`authorize_widget_version`] first.
pub async fn load_stored_widget(
    state: &AppState,
    target: WidgetTarget<'_>,
) -> Result<StoredWidget, ApiError> {
    let cache_key = serde_json::json!([
        STORED_WIDGET_CACHE_NAMESPACE,
        target.package_id,
        target.version,
        target.widget_id
    ])
    .to_string();
    if let Some(cached) = state.get_cache::<StoredWidget>(&cache_key) {
        return Ok(cached);
    }

    let row = wasm_package_version::Entity::find()
        .filter(wasm_package_version::Column::PackageId.eq(target.package_id))
        .filter(wasm_package_version::Column::Version.eq(target.version))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Version '{}' not found", target.version)))?;
    let bundle_hash = row
        .widget_bundle_hash
        .filter(|hash| !hash.is_empty())
        .ok_or_else(|| ApiError::not_found("This package version ships no widgets"))?;
    let contract = stored_widget_contract(&row.widgets, target.widget_id).ok_or_else(|| {
        ApiError::not_found(format!(
            "Widget '{}' not found in version '{}'",
            target.widget_id, target.version
        ))
    })?;

    let stored = StoredWidget {
        bundle_hash,
        contract,
    };
    state.set_cache(cache_key, &stored);
    Ok(stored)
}

fn descriptor_cache_key(
    target: WidgetTarget<'_>,
    preview: bool,
    context: &WidgetRequestContext,
) -> String {
    let (catalog_version, psl_version) = widget_source_data_versions();
    serde_json::json!([
        DESCRIPTOR_CACHE_NAMESPACE,
        target.package_id,
        target.version,
        target.widget_id,
        preview,
        context.reserved_hosts,
        context.platform_storage,
        context.bundle_sources,
        catalog_version,
        psl_version
    ])
    .to_string()
}

/// The declared-only descriptor with `network`, memoized per target, preview
/// mode, deployment facts and classification data. `engine` is set from this
/// request after the memo read.
pub async fn load_widget_policy_descriptor(
    state: &AppState,
    context: &WidgetRequestContext,
    target: WidgetTarget<'_>,
    preview: bool,
) -> Result<WidgetPolicyDescriptor, ApiError> {
    let cache_key = descriptor_cache_key(target, preview, context);
    let cached = context
        .memoize
        .then(|| state.get_cache::<WidgetPolicyDescriptor>(&cache_key))
        .flatten();
    let mut descriptor = match cached {
        Some(cached) => cached,
        None => {
            let stored = load_stored_widget(state, target).await?;
            let descriptor = with_network(
                describe_stored_widget(
                    stored.subject(target, preview),
                    &stored.contract,
                    &[],
                    &context.runtime(None),
                ),
                &stored.contract,
            );
            if context.memoize {
                state.set_cache(cache_key, &descriptor);
            }
            descriptor
        }
    };
    if descriptor.engine.is_some() {
        descriptor.engine = Some(context.engine.support());
    }
    Ok(descriptor)
}

/// Derives the descriptor for runtime sources, without memo and without
/// `network`.
pub async fn load_runtime_descriptor(
    state: &AppState,
    context: &WidgetRequestContext,
    target: WidgetTarget<'_>,
    preview: bool,
    app_id: Option<&str>,
    request: &[WidgetRuntimeSourceRequest],
) -> Result<(WidgetPolicyDescriptor, StoredWidget), ApiError> {
    let stored = load_stored_widget(state, target).await?;
    let descriptor = describe_stored_widget(
        stored.subject(target, preview),
        &stored.contract,
        request,
        &context.runtime(app_id),
    );
    Ok((descriptor, stored))
}

fn ensure_widget_ids(package_id: &str, widget_id: &str) -> Result<(), ApiError> {
    if !is_valid_package_id(package_id) {
        return Err(ApiError::not_found(format!(
            "Package '{}' not found",
            package_id
        )));
    }
    if !is_frame_widget_id(widget_id) {
        return Err(ApiError::not_found(format!(
            "Widget '{}' not found",
            widget_id
        )));
    }
    Ok(())
}

fn widget_error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, NO_STORE)],
        Json(serde_json::json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

fn descriptor_response(descriptor: WidgetPolicyDescriptor) -> Response {
    (
        [(header::CACHE_CONTROL, PRIVATE_NO_STORE)],
        Json(descriptor),
    )
        .into_response()
}

fn grant_response(response: WidgetGrantResponse) -> Response {
    ([(header::CACHE_CONTROL, NO_STORE)], Json(response)).into_response()
}

/// GET /registry/package/{package_id}/widget-policy/{version}/{widget_id}
#[utoipa::path(
    get,
    path = "/registry/package/{package_id}/widget-policy/{version}/{widget_id}",
    tag = "registry",
    description = "Describe what a package widget is allowed to do once the viewer approves it: the sites it can reach, the inputs that can add sites while the app runs, and the browser capabilities it uses. Show this to the viewer before running the widget with a grant.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("version" = String, Path, description = "Package version"),
        ("widget_id" = String, Path, description = "Widget ID"),
        WidgetPolicyQuery
    ),
    responses(
        (status = 200, description = "The widget's declared policy as the server enforces it (private, no-store)", body = WidgetPolicyDescriptor),
        (status = 403, description = "No access to this package"),
        (status = 404, description = "Package, version, or widget not found"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn describe_widget_policy(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Path((package_id, version, widget_id)): Path<(String, String, String)>,
    Query(query): Query<WidgetPolicyQuery>,
) -> Result<Response, ApiError> {
    ensure_widget_ids(&package_id, &widget_id)?;
    authorize_widget_version(&state, &user, &package_id, &version).await?;
    let target = WidgetTarget {
        package_id: &package_id,
        version: &version,
        widget_id: &widget_id,
    };
    let context = WidgetRequestContext::from_request(&state, &headers, &package_id, &version);
    let descriptor = load_widget_policy_descriptor(&state, &context, target, query.preview).await?;
    Ok(descriptor_response(descriptor))
}

/// POST /registry/package/{package_id}/widget-policy/{version}/{widget_id}
#[utoipa::path(
    post,
    path = "/registry/package/{package_id}/widget-policy/{version}/{widget_id}",
    tag = "registry",
    description = "Describe a widget's permissions including addresses provided while the app runs, so the viewer can approve them.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("version" = String, Path, description = "Package version"),
        ("widget_id" = String, Path, description = "Widget ID")
    ),
    request_body = WidgetPolicyDescribeRequest,
    responses(
        (status = 200, description = "The widget's policy including the accepted runtime addresses, and the ones the server refused (private, no-store)", body = WidgetPolicyDescriptor),
        (status = 400, description = "Malformed request: INVALID_RUNTIME_SOURCES, RUNTIME_SOURCES_IN_PREVIEW, INVALID_APP_ID or INVALID_WIDGET_REQUEST"),
        (status = 403, description = "No access to this package"),
        (status = 404, description = "Package, version, or widget not found"),
        (status = 413, description = "Request body over 16 KiB"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn describe_widget_runtime_policy(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Path((package_id, version, widget_id)): Path<(String, String, String)>,
    body: Bytes,
) -> Result<Response, ApiError> {
    ensure_widget_ids(&package_id, &widget_id)?;
    let request: WidgetPolicyDescribeRequest = match parse_widget_request(&body) {
        Ok(request) => request,
        Err(error) => return Ok(error.into_response()),
    };
    if let Err(error) = validate_runtime_request(
        request.preview,
        request.app_id.as_deref(),
        &request.runtime_sources,
    ) {
        return Ok(error.into_response());
    }
    authorize_widget_version(&state, &user, &package_id, &version).await?;
    let target = WidgetTarget {
        package_id: &package_id,
        version: &version,
        widget_id: &widget_id,
    };
    let context = WidgetRequestContext::from_request(&state, &headers, &package_id, &version);
    let (descriptor, stored) = load_runtime_descriptor(
        &state,
        &context,
        target,
        request.preview,
        request.app_id.as_deref(),
        &request.runtime_sources,
    )
    .await?;
    Ok(descriptor_response(with_network(
        descriptor,
        &stored.contract,
    )))
}

/// POST /registry/package/{package_id}/widget-grant
#[utoipa::path(
    post,
    path = "/registry/package/{package_id}/widget-grant",
    tag = "registry",
    description = "Get a short-lived grant that runs a package widget with the permissions the viewer approved. Send the policy digest, app id and runtime addresses of the widget policy description the viewer approved; if the widget's permissions changed since then, describe it again and ask the viewer again. When runtime addresses were approved, load the widget with the grant followed by '~' and the returned runtime value.",
    params(("package_id" = String, Path, description = "Package ID")),
    request_body = WidgetGrantRequest,
    responses(
        (status = 200, description = "The grant, or null when the widget runs without extra permissions", body = WidgetGrantResponse),
        (status = 400, description = "Malformed request: INVALID_RUNTIME_SOURCES, RUNTIME_SOURCES_IN_PREVIEW, INVALID_APP_ID or INVALID_WIDGET_REQUEST"),
        (status = 403, description = "No access to this package"),
        (status = 404, description = "Package, version, or widget not found"),
        (status = 409, description = "The widget's permissions changed since they were described (code POLICY_CHANGED)"),
        (status = 413, description = "Request body over 16 KiB"),
        (status = 503, description = "Widget grants or the WASM registry are not configured on this server")
    ),
    security(("bearer_auth" = []))
)]
pub async fn mint_widget_grant(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Path(package_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    let request: WidgetGrantRequest = match parse_widget_request(&body) {
        Ok(request) => request,
        Err(error) => return Ok(error.into_response()),
    };
    ensure_widget_ids(&package_id, &request.widget_id)?;
    if let Err(error) = validate_runtime_request(
        request.preview,
        request.app_id.as_deref(),
        &request.runtime_sources,
    ) {
        return Ok(error.into_response());
    }
    let authorized = authorize_widget_version(&state, &user, &package_id, &request.version).await?;
    let target = WidgetTarget {
        package_id: &package_id,
        version: &request.version,
        widget_id: &request.widget_id,
    };
    let context =
        WidgetRequestContext::from_request(&state, &headers, &package_id, &request.version);
    let descriptor = if request.runtime_sources.is_empty() {
        load_widget_policy_descriptor(&state, &context, target, request.preview).await?
    } else {
        load_runtime_descriptor(
            &state,
            &context,
            target,
            request.preview,
            request.app_id.as_deref(),
            &request.runtime_sources,
        )
        .await?
        .0
    };

    match mint_decision(
        &descriptor,
        &request.policy_digest,
        backend_jwt::is_configured(),
    ) {
        MintDecision::PolicyChanged => Ok(widget_error_response(
            StatusCode::CONFLICT,
            POLICY_CHANGED_CODE,
            &format!(
                "policy_changed: widget '{}' of {}@{} now has policy {}, not {}; describe it again",
                request.widget_id,
                package_id,
                request.version,
                descriptor.policy_digest,
                request.policy_digest
            ),
        )),
        MintDecision::Baseline => Ok(grant_response(WidgetGrantResponse {
            grant: None,
            expires_in: MAX_WIDGET_GRANT_TTL_SECONDS,
            policy_digest: descriptor.policy_digest,
            runtime: None,
        })),
        MintDecision::Unavailable => {
            tracing::warn!(
                package_id = %package_id,
                widget_id = %request.widget_id,
                "Widget grant requested but backend JWT signing is not configured"
            );
            Ok(widget_error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                WIDGET_GRANTS_UNAVAILABLE_CODE,
                "Widget grants are not configured on this server",
            ))
        }
        MintDecision::Sign => {
            let (runtime, component) = grant_runtime(&descriptor).map_err(|error| {
                ApiError::internal(format!(
                    "Failed to encode runtime sources of widget '{}' of {}@{}: {}",
                    request.widget_id, package_id, request.version, error
                ))
            })?;
            let grant = sign_widget_grant(WidgetGrantParams {
                package_id: package_id.clone(),
                version: request.version.clone(),
                bundle_hash: descriptor.bundle_hash.clone(),
                widget_id: request.widget_id.clone(),
                preview: descriptor.preview,
                policy_digest: descriptor.policy_digest.clone(),
                runtime,
                ttl_seconds: Some(MAX_WIDGET_GRANT_TTL_SECONDS),
            })
            .map_err(|e| {
                ApiError::internal(format!(
                    "Failed to sign widget grant for '{}' of {}@{}: {}",
                    request.widget_id, package_id, request.version, e
                ))
            })?;
            tracing::info!(
                jti = %grant.jti,
                user_id = authorized.sub.as_deref().unwrap_or("anonymous"),
                package_id = %package_id,
                version = %request.version,
                widget_id = %request.widget_id,
                preview = descriptor.preview,
                runtime = component.is_some(),
                "Minted widget grant"
            );
            Ok(grant_response(WidgetGrantResponse {
                grant: Some(grant.token),
                expires_in: MAX_WIDGET_GRANT_TTL_SECONDS,
                policy_digest: descriptor.policy_digest,
                runtime: component,
            }))
        }
    }
}

/// The version 2 claims and URL runtime component of a descriptor with
/// accepted runtime sources; `(None, None)` signs version 1.
pub fn grant_runtime(
    descriptor: &WidgetPolicyDescriptor,
) -> Result<(Option<WidgetGrantRuntime>, Option<String>), String> {
    let (Some(sources), Some(runtime_digest)) = (
        descriptor.runtime_sources(),
        descriptor
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.runtime_digest.clone()),
    ) else {
        return Ok((None, None));
    };
    let component = encode_runtime_component(&sources.slots).map_err(|error| error.to_string())?;
    Ok((
        Some(WidgetGrantRuntime {
            declared_digest: descriptor.declared_digest().to_string(),
            runtime_digest,
            app_id: sources.app_id.clone(),
        }),
        Some(component),
    ))
}

/// Platform-storage scopes of the content bucket (§14.4.5). `endpoint` is the
/// S3 endpoint the content store is built with, when not AWS itself. The meta
/// bucket is never a scope.
pub fn content_storage_scopes(
    content: &BucketIdentity,
    endpoint: Option<&str>,
) -> Vec<HubWidgetStorage> {
    let bucket = content.name.trim();
    if bucket.is_empty() {
        return Vec::new();
    }
    let path_style = |origin: String| HubWidgetStorage {
        origin,
        path_prefix: format!("/{bucket}/{PLATFORM_APPS_SEGMENT}/"),
    };
    let candidates = match content.provider {
        StorageProviderKind::Aws | StorageProviderKind::R2 => {
            match endpoint.or(content.endpoint.as_deref()).map(str::trim) {
                Some(endpoint) if !endpoint.is_empty() => endpoint_origin(endpoint)
                    .map(path_style)
                    .into_iter()
                    .collect(),
                _ if content.provider == StorageProviderKind::Aws => content
                    .region
                    .as_deref()
                    .map(|region| {
                        vec![
                            HubWidgetStorage {
                                origin: format!("https://{bucket}.s3.{region}.amazonaws.com"),
                                path_prefix: format!("/{PLATFORM_APPS_SEGMENT}/"),
                            },
                            path_style(format!("https://s3.{region}.amazonaws.com")),
                        ]
                    })
                    .unwrap_or_default(),
                _ => Vec::new(),
            }
        }
        StorageProviderKind::Gcp => vec![path_style("https://storage.googleapis.com".to_string())],
        StorageProviderKind::Azure => content
            .account
            .as_deref()
            .map(|account| {
                vec![path_style(format!(
                    "https://{account}.blob.core.windows.net"
                ))]
            })
            .unwrap_or_default(),
        StorageProviderKind::Local | StorageProviderKind::Memory | StorageProviderKind::Other => {
            Vec::new()
        }
    };
    let mut scopes: Vec<HubWidgetStorage> = Vec::with_capacity(candidates.len());
    for scope in candidates {
        let usable = PlatformStorageScope {
            origin: scope.origin.clone(),
            path_prefix: scope.path_prefix.clone(),
        }
        .app_source("app")
        .is_some();
        if usable && !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    scopes
}

fn endpoint_origin(endpoint: &str) -> Option<String> {
    let (scheme, rest) = endpoint.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    (!authority.is_empty()).then(|| {
        format!(
            "{}://{}",
            scheme.to_ascii_lowercase(),
            authority.to_ascii_lowercase()
        )
    })
}

/// Endpoint the content store is built with (see `credentials::aws_credentials`).
fn content_store_endpoint(content: &BucketIdentity) -> Option<String> {
    let env = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    match content.provider {
        StorageProviderKind::Aws => env("CONTENT_BUCKET_ENDPOINT")
            .or_else(|| env("S3_PUBLIC_ENDPOINT"))
            .or_else(|| env("AWS_ENDPOINT")),
        _ => None,
    }
}

/// Startup: loads the classification data (logged, never fatal) and derives
/// the hub's `widget_storage` from the content bucket unless the hub config
/// already names it.
pub fn init_widget_policy(hub: &mut Hub, content: &BucketIdentity) {
    if let Err(error) = warm_widget_source_data() {
        tracing::error!(
            %error,
            "Widget source classification data failed to load; widget descriptors omit network details and declared wildcards are refused"
        );
    }
    if hub.widget_storage.is_empty() {
        hub.widget_storage =
            content_storage_scopes(content, content_store_endpoint(content).as_deref());
    }
    tracing::info!(
        scopes = ?hub.widget_storage,
        "Widget platform-storage scopes"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_wasm_schema::widget::{ContractInput, ContractInputType, WidgetCapabilities};
    use flow_like_wasm_schema::widget_frame::decode_runtime_component;
    use flow_like_wasm_schema::widget_policy::{
        CspDirective, WidgetCspPurpose, WidgetNetworkInput, WidgetPolicy, WidgetPolicyStatus,
        WidgetRuntimeStatus,
    };

    const HASH: &str = "4f1c0a3b2d5e6f708192a3b4c5d6e7f80112233445566778899aabbccddeeff0";
    const APP: &str = "app_01";

    fn subject(widget_id: &str, preview: bool) -> WidgetPolicySubject {
        WidgetPolicySubject {
            source: WIDGET_POLICY_SOURCE_HUB.to_string(),
            package_id: "com.example.maps".to_string(),
            package_version: Some("1.2.0".to_string()),
            bundle_hash: HASH.to_string(),
            widget_id: widget_id.to_string(),
            preview,
        }
    }

    fn purpose(reason: &str, connect: &[&str], img: &[&str]) -> WidgetCspPurpose {
        WidgetCspPurpose {
            reason: reason.to_string(),
            connect_src: connect.iter().map(|host| host.to_string()).collect(),
            img_src: img.iter().map(|host| host.to_string()).collect(),
            ..WidgetCspPurpose::default()
        }
    }

    fn network_contract(widget_id: &str, hosts: &[&str]) -> WidgetContract {
        let mut contract = WidgetContract::new(widget_id);
        contract.capabilities = Some(WidgetCapabilities {
            workers: Some(true),
            media: Some(true),
            microphone: Some(true),
            ..Default::default()
        });
        contract.with_csp(vec![
            purpose("Loads vector map tiles", hosts, &[]),
            purpose(
                "Shows map imagery tiles",
                &[],
                &["https://tiles.example-maps.com"],
            ),
        ])
    }

    fn runtime_contract(widget_id: &str) -> WidgetContract {
        let mut contract = network_contract(widget_id, &["https://api.maptiler.com"]);
        contract.inputs.insert(
            "tileUrl".into(),
            ContractInput {
                input_type: ContractInputType::String,
                description: None,
                default: None,
                choices: None,
                min: None,
                max: None,
                schema: None,
                optional: true,
            },
        );
        let mut purposes = contract.csp.clone().unwrap();
        purposes.push(WidgetCspPurpose {
            reason: "Loads map tiles from tile servers given to it at runtime".into(),
            inputs: vec![WidgetNetworkInput {
                path: "tileUrl".into(),
                directives: vec![CspDirective::ConnectSrc, CspDirective::ImgSrc],
                template: None,
            }],
            ..WidgetCspPurpose::default()
        });
        contract.with_csp(purposes)
    }

    fn stored_widgets(contract: &WidgetContract) -> Value {
        serde_json::json!([
            { "id": "other", "name": "Other", "description": "", "contract": WidgetContract::new("other") },
            { "id": contract.id, "name": "Map", "description": "", "contract": contract },
        ])
    }

    fn storage() -> Vec<PlatformStorageScope> {
        vec![PlatformStorageScope {
            origin: "https://s3.eu-central-1.amazonaws.com".into(),
            path_prefix: "/flow-like-content/apps/".into(),
        }]
    }

    struct Facts {
        reserved: Vec<String>,
        storage: Vec<PlatformStorageScope>,
        bundle: Vec<String>,
    }

    impl Facts {
        fn new() -> Self {
            Self {
                reserved: widget_reserved_hosts(&[Some("api.flow-like.com")], None),
                storage: storage(),
                bundle: vec![
                    "https://api.flow-like.com/api/v1/registry/package/com.example.maps/widget-sandbox/1.2.0/"
                        .into(),
                ],
            }
        }

        fn context<'a>(&'a self, app_id: Option<&'a str>) -> WidgetRuntimeContext<'a> {
            WidgetRuntimeContext {
                reserved_hosts: &self.reserved,
                platform_storage: &self.storage,
                app_id,
                bundle_sources: &self.bundle,
                engine: EngineGate::OPEN,
            }
        }
    }

    fn describe(contract: &WidgetContract, preview: bool) -> WidgetPolicyDescriptor {
        let facts = Facts::new();
        describe_stored_widget(
            subject(&contract.id, preview),
            &Ok(contract.clone()),
            &[],
            &facts.context(None),
        )
    }

    fn request(slot: &str, sources: &[&str]) -> Vec<WidgetRuntimeSourceRequest> {
        vec![WidgetRuntimeSourceRequest {
            slot: slot.into(),
            sources: sources.iter().map(|source| source.to_string()).collect(),
        }]
    }

    #[test]
    fn widget_policy_reserved_hosts_normalize_and_dedup() {
        assert_eq!(
            widget_reserved_hosts(
                &[
                    Some("api.flow-like.com"),
                    Some("https://App.Flow-Like.com/"),
                    None,
                    Some(""),
                    Some("flow-like.com"),
                ],
                Some("API.flow-like.com:443"),
            ),
            vec![
                "api.flow-like.com".to_string(),
                "app.flow-like.com".to_string(),
                "flow-like.com".to_string(),
            ]
        );
        assert_eq!(
            parse_reserved_hosts(
                " d111.cloudfront.net, ,*.azurewebsites.net,https://x.lambda-url.eu-central-1.on.aws "
            ),
            vec![
                "d111.cloudfront.net".to_string(),
                "azurewebsites.net".to_string(),
                "https://x.lambda-url.eu-central-1.on.aws".to_string(),
            ]
        );
    }

    #[test]
    fn widget_policy_describe_reads_the_stored_contract_and_strips_previews() {
        let contract = runtime_contract("live-map");
        let widgets = stored_widgets(&contract);
        let facts = Facts::new();

        let full = describe_stored_widget(
            subject("live-map", false),
            &stored_widget_contract(&widgets, "live-map").unwrap(),
            &[],
            &facts.context(None),
        );
        assert!(full.is_ok());
        assert!(full.policy.workers && full.policy.media && full.policy.microphone);
        assert_eq!(full.policy.csp.connect_src, ["https://api.maptiler.com"]);
        assert_eq!(full.policy_digest, full.policy.digest());
        assert_eq!(full.package_version.as_deref(), Some("1.2.0"));
        assert_eq!(full.network_inputs.len(), 1);
        assert_eq!(full.network_inputs[0].path, "tileUrl");
        assert_eq!(full.network_inputs[0].purpose, 2);
        assert_eq!(full.platform_storage.as_deref(), Some(storage().as_slice()));
        let runtime = full.runtime.as_ref().unwrap();
        assert_eq!(runtime.status, WidgetRuntimeStatus::None);
        assert_eq!(runtime.declared_digest, full.policy_digest);

        let json = serde_json::to_value(with_network(full.clone(), &Ok(contract.clone()))).unwrap();
        assert_eq!(json["source"], "hub");
        assert_eq!(json["status"], "ok");
        assert_eq!(
            json["policy"]["csp"]["connectSrc"][0],
            "https://api.maptiler.com"
        );
        assert_eq!(json["networkInputs"][0]["path"], "tileUrl");
        assert_eq!(
            json["platformStorage"][0]["pathPrefix"],
            "/flow-like-content/apps/"
        );
        assert_eq!(json["runtime"]["status"], "none");
        assert_eq!(json["runtime"]["declaredDigest"], full.policy_digest);
        assert_eq!(json["engine"]["runtimeSources"], true);
        assert_eq!(json["network"]["purposes"].as_array().unwrap().len(), 3);
        assert!(json.get("invalidReason").is_none());

        let preview = describe_stored_widget(
            subject("live-map", true),
            &stored_widget_contract(&widgets, "live-map").unwrap(),
            &[],
            &facts.context(None),
        );
        assert!(preview.is_ok());
        assert!(preview.policy.workers);
        assert!(!preview.policy.media && !preview.policy.microphone);
        assert!(preview.policy.csp.is_empty());
        assert!(preview.network_inputs.is_empty());
        assert!(preview.runtime.is_none() && preview.engine.is_none());
        assert_ne!(preview.policy_digest, full.policy_digest);
    }

    #[test]
    fn widget_policy_describe_reports_invalid_policies_at_baseline() {
        let contract = network_contract("live-map", &["https://edge.api.flow-like.com"]);
        let widgets = stored_widgets(&contract);
        let facts = Facts::new();

        let reserved_host = describe_stored_widget(
            subject("live-map", false),
            &stored_widget_contract(&widgets, "live-map").unwrap(),
            &[],
            &facts.context(None),
        );
        assert_eq!(reserved_host.status, WidgetPolicyStatus::Invalid);
        assert!(
            reserved_host
                .invalid_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("edge.api.flow-like.com"))
        );
        assert_eq!(reserved_host.policy, WidgetPolicy::default());
        assert_eq!(
            reserved_host.policy_digest,
            WidgetPolicy::default().digest()
        );

        let unreadable = serde_json::json!([{ "id": "live-map", "contract": { "id": 7 } }]);
        let unreadable = describe_stored_widget(
            subject("live-map", false),
            &stored_widget_contract(&unreadable, "live-map").unwrap(),
            &[],
            &facts.context(None),
        );
        assert_eq!(unreadable.status, WidgetPolicyStatus::Invalid);
        assert_eq!(unreadable.policy, WidgetPolicy::default());

        let public_suffix = describe(&network_contract("live-map", &["https://*.co.uk"]), false);
        assert_eq!(public_suffix.status, WidgetPolicyStatus::Invalid);
        assert!(
            public_suffix
                .invalid_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("public suffix"))
        );
        let tenant_wildcard = describe(
            &network_contract("live-map", &["https://*.customer-maps.com"]),
            false,
        );
        assert!(tenant_wildcard.is_ok());

        assert!(stored_widget_contract(&widgets, "missing").is_none());
        assert!(stored_widget_contract(&serde_json::json!({}), "live-map").is_none());
    }

    #[test]
    fn widget_policy_runtime_sources_widen_the_effective_policy() {
        let contract = runtime_contract("live-map");
        let facts = Facts::new();
        let declared = describe(&contract, false);
        let derived = describe_stored_widget(
            subject("live-map", false),
            &Ok(contract.clone()),
            &request(
                "tileUrl",
                &[
                    "https://a.tiles.customer-maps.com",
                    "https://cdn.api.flow-like.com",
                    "https://*.customer-maps.com",
                ],
            ),
            &facts.context(Some(APP)),
        );
        assert!(derived.is_ok());
        let runtime = derived.runtime.as_ref().unwrap();
        assert_eq!(runtime.status, WidgetRuntimeStatus::Ok);
        assert_eq!(runtime.declared_digest, declared.policy_digest);
        assert_ne!(derived.policy_digest, declared.policy_digest);
        assert!(
            derived
                .policy
                .csp
                .img_src
                .contains(&"https://a.tiles.customer-maps.com".to_string())
        );
        let codes: Vec<(&str, &str)> = runtime
            .rejected
            .iter()
            .map(|rejection| (rejection.source.as_str(), rejection.code.as_str()))
            .collect();
        assert_eq!(
            codes,
            [
                ("https://*.customer-maps.com", "wildcard"),
                ("https://cdn.api.flow-like.com", "reserved-host"),
            ]
        );

        let (claims, component) = grant_runtime(&derived).unwrap();
        let claims = claims.unwrap();
        assert_eq!(claims.declared_digest, declared.policy_digest);
        assert_eq!(
            claims.runtime_digest,
            runtime.runtime_digest.clone().unwrap()
        );
        assert_eq!(claims.app_id.as_deref(), Some(APP));
        let slots = decode_runtime_component(&component.unwrap()).unwrap();
        assert_eq!(slots, derived.runtime_sources().unwrap().slots);

        assert_eq!(grant_runtime(&declared).unwrap(), (None, None));

        let other_app = describe_stored_widget(
            subject("live-map", false),
            &Ok(contract.clone()),
            &request("tileUrl", &["https://a.tiles.customer-maps.com"]),
            &facts.context(Some("other_app")),
        );
        assert_ne!(
            other_app.runtime.unwrap().runtime_digest,
            runtime.runtime_digest,
            "the runtime digest binds the app id"
        );

        let storage = describe_stored_widget(
            subject("live-map", false),
            &Ok(contract.clone()),
            &request(
                "tileUrl",
                &[
                    "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_01/",
                    "https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/other/",
                    "https://s3.eu-central-1.amazonaws.com",
                ],
            ),
            &facts.context(Some(APP)),
        );
        let storage_runtime = storage.runtime.unwrap();
        assert_eq!(storage_runtime.rejected.len(), 2);
        assert!(
            storage_runtime
                .rejected
                .iter()
                .all(|rejection| rejection.code == "platform-storage")
        );
        assert!(storage.policy.csp.connect_src.contains(
            &"https://s3.eu-central-1.amazonaws.com/flow-like-content/apps/app_01/".to_string()
        ));

        let unknown = describe_stored_widget(
            subject("live-map", false),
            &Ok(contract),
            &request("missing", &["https://a.tiles.customer-maps.com"]),
            &facts.context(Some(APP)),
        );
        assert_eq!(unknown.policy_digest, declared.policy_digest);
        assert_eq!(unknown.runtime.unwrap().rejected[0].code, "unknown-slot");
    }

    #[test]
    fn widget_policy_runtime_requests_reject_bad_shapes_and_previews() {
        let too_many: Vec<WidgetRuntimeSourceRequest> = (0..9)
            .map(|index| WidgetRuntimeSourceRequest {
                slot: format!("slot{index}"),
                sources: vec![],
            })
            .collect();
        let duplicate = [request("tileUrl", &[]), request("tileUrl", &[])].concat();
        let long = request("tileUrl", &[&format!("https://{}.com", "a".repeat(300))]);
        for invalid in [too_many, duplicate, long] {
            assert_eq!(
                validate_runtime_request(false, None, &invalid)
                    .unwrap_err()
                    .code,
                INVALID_RUNTIME_SOURCES_CODE
            );
        }
        assert_eq!(
            validate_runtime_request(true, None, &request("tileUrl", &["https://a.com"]))
                .unwrap_err()
                .code,
            RUNTIME_SOURCES_IN_PREVIEW_CODE
        );
        assert!(validate_runtime_request(true, None, &request("tileUrl", &[])).is_ok());
        assert_eq!(
            validate_runtime_request(false, Some("app/1"), &[])
                .unwrap_err()
                .code,
            INVALID_APP_ID_CODE
        );
        assert!(
            validate_runtime_request(false, Some(APP), &request("tileUrl", &["https://a.com"]))
                .is_ok()
        );

        let parsed: WidgetPolicyDescribeRequest = parse_widget_request(
            br#"{"preview":false,"appId":"app_01","runtimeSources":[{"slot":"tileUrl","sources":["https://a.com"]}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.app_id.as_deref(), Some(APP));
        assert_eq!(
            parsed.runtime_sources,
            request("tileUrl", &["https://a.com"])
        );
        let empty: WidgetPolicyDescribeRequest = parse_widget_request(b"{}").unwrap();
        assert!(!empty.preview && empty.runtime_sources.is_empty());

        for (body, code) in [
            (
                &br#"{"runtimeSources":[{"slot":"tileUrl","sources":[7]}]}"#[..],
                INVALID_RUNTIME_SOURCES_CODE,
            ),
            (
                br#"{"runtimeSources":{"slot":"tileUrl"}}"#,
                INVALID_RUNTIME_SOURCES_CODE,
            ),
            (
                br#"{"runtimeSources":[{"slot":"tileUrl","sources":[],"extra":1}]}"#,
                INVALID_RUNTIME_SOURCES_CODE,
            ),
            (
                br#"{"preview":false,"policy":{}}"#,
                INVALID_WIDGET_REQUEST_CODE,
            ),
            (b"not json", INVALID_WIDGET_REQUEST_CODE),
        ] {
            assert_eq!(
                parse_widget_request::<WidgetPolicyDescribeRequest>(body)
                    .unwrap_err()
                    .code,
                code,
                "{}",
                String::from_utf8_lossy(body)
            );
        }

        let grant: WidgetGrantRequest = parse_widget_request(
            br#"{"version":"1.2.0","widgetId":"live-map","policyDigest":"sha256:00","appId":"app_01","runtimeSources":[{"slot":"tileUrl","sources":["https://a.com"]}]}"#,
        )
        .unwrap();
        assert_eq!(grant.app_id.as_deref(), Some(APP));
        assert_eq!(grant.runtime_sources.len(), 1);
        assert_eq!(
            parse_widget_request::<WidgetGrantRequest>(
                br#"{"version":"1.2.0","widgetId":"live-map","policyDigest":"sha256:00","runtimeSources":[{"slot":1}]}"#,
            )
            .unwrap_err()
            .code,
            INVALID_RUNTIME_SOURCES_CODE
        );
    }

    #[test]
    fn widget_policy_mint_rejects_stale_digests_and_skips_empty_policies() {
        let approved = describe(
            &network_contract("live-map", &["https://api.maptiler.com"]),
            false,
        );
        let current = describe(
            &network_contract(
                "live-map",
                &[
                    "https://api.maptiler.com",
                    "https://collector.example-maps.com",
                ],
            ),
            false,
        );
        assert_eq!(
            mint_decision(&current, &approved.policy_digest, true),
            MintDecision::PolicyChanged
        );
        assert_eq!(
            mint_decision(&current, &current.policy_digest, true),
            MintDecision::Sign
        );
        assert_eq!(
            mint_decision(&current, &current.policy_digest, false),
            MintDecision::Unavailable
        );

        let empty = describe(&WidgetContract::new("plain"), false);
        assert!(empty.policy.is_empty());
        assert_eq!(
            mint_decision(&empty, &empty.policy_digest, true),
            MintDecision::Baseline
        );

        let invalid = describe(
            &network_contract("live-map", &["https://api.flow-like.com"]),
            false,
        );
        assert_eq!(
            mint_decision(&invalid, &invalid.policy_digest, true),
            MintDecision::Baseline
        );
    }

    #[test]
    fn widget_policy_grant_response_serializes_null_grants_and_runtime() {
        let response = WidgetGrantResponse {
            grant: None,
            expires_in: MAX_WIDGET_GRANT_TTL_SECONDS,
            policy_digest: WidgetPolicy::default().digest(),
            runtime: None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert!(json["grant"].is_null());
        assert!(json["runtime"].is_null());
        assert_eq!(json["expiresIn"], 3600);
        assert_eq!(json["policyDigest"], WidgetPolicy::default().digest());

        let request: WidgetGrantRequest = serde_json::from_value(serde_json::json!({
            "version": "1.2.0",
            "widgetId": "live-map",
            "policyDigest": "sha256:00"
        }))
        .unwrap();
        assert!(!request.preview);
        assert!(request.app_id.is_none() && request.runtime_sources.is_empty());
    }

    #[test]
    fn widget_policy_describe_responses_are_private_and_uncached() {
        let response = descriptor_response(describe(&WidgetContract::new("plain"), false));
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            PRIVATE_NO_STORE
        );
        let error = WidgetRequestError::new(INVALID_RUNTIME_SOURCES_CODE, "x").into_response();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            error.headers().get(header::CACHE_CONTROL).unwrap(),
            NO_STORE
        );
    }

    #[test]
    fn widget_policy_memo_only_serves_configured_authorities() {
        let configured = [
            Some("api.flow-like.com"),
            Some("https://app.flow-like.com"),
            None,
        ];
        assert!(is_configured_authority(&configured, None));
        assert!(is_configured_authority(
            &configured,
            Some("api.flow-like.com")
        ));
        assert!(is_configured_authority(
            &configured,
            Some("app.flow-like.com:443")
        ));
        assert!(!is_configured_authority(
            &configured,
            Some("r1.example.com")
        ));
        assert!(!is_configured_authority(
            &configured,
            Some("evil.api.flow-like.com")
        ));
    }

    #[test]
    fn widget_policy_cache_key_separates_preview_facts_and_data_versions() {
        let target = WidgetTarget {
            package_id: "com.example.maps",
            version: "1.2.0",
            widget_id: "live-map",
        };
        let context = WidgetRequestContext {
            reserved_hosts: vec!["api.flow-like.com".to_string()],
            platform_storage: storage(),
            bundle_sources: vec![],
            engine: EngineGate::OPEN,
            memoize: true,
        };
        let key = descriptor_cache_key(target, false, &context);
        assert_ne!(key, descriptor_cache_key(target, true, &context));
        for changed in [
            WidgetRequestContext {
                reserved_hosts: vec![],
                ..context.clone()
            },
            WidgetRequestContext {
                platform_storage: vec![],
                ..context.clone()
            },
            WidgetRequestContext {
                bundle_sources: vec!["https://x.com/p/".into()],
                ..context.clone()
            },
        ] {
            assert_ne!(key, descriptor_cache_key(target, false, &changed));
        }
        let gated = WidgetRequestContext {
            engine: EngineGate {
                runtime_sources: false,
                ..EngineGate::OPEN
            },
            ..context.clone()
        };
        assert_eq!(
            key,
            descriptor_cache_key(target, false, &gated),
            "engine is never memoized"
        );
        let (catalog_version, psl_version) = widget_source_data_versions();
        assert!(key.contains(&psl_version) && key.contains(&format!(",{catalog_version},")));
    }

    #[test]
    fn widget_policy_engine_gate_follows_the_user_agent() {
        let mut headers = HeaderMap::new();
        assert_eq!(request_engine_gate(&headers), EngineGate::OPEN);
        headers.insert(
            header::USER_AGENT,
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36"
                .parse()
                .unwrap(),
        );
        assert_eq!(request_engine_gate(&headers), EngineGate::OPEN);
    }

    #[test]
    fn widget_policy_declared_network_classifies_store_widgets() {
        let network = declared_widget_network(&network_contract(
            "live-map",
            &["https://*.s3.eu-central-1.amazonaws.com"],
        ))
        .expect("declared sources classify");
        assert_eq!(network.purposes.len(), 2);
        assert_eq!(
            serde_json::to_value(network.level).unwrap(),
            serde_json::json!("broad")
        );
        assert!(declared_widget_network(&WidgetContract::new("plain")).is_none());
    }

    fn identity(provider: StorageProviderKind) -> BucketIdentity {
        BucketIdentity {
            provider,
            name: "flow-like-content".into(),
            region: Some("eu-central-1".into()),
            account: Some("flowlikeprod".into()),
            endpoint: None,
        }
    }

    fn scope(origin: &str, path_prefix: &str) -> HubWidgetStorage {
        HubWidgetStorage {
            origin: origin.into(),
            path_prefix: path_prefix.into(),
        }
    }

    #[test]
    fn widget_policy_platform_storage_scopes_per_provider() {
        assert_eq!(
            content_storage_scopes(&identity(StorageProviderKind::Aws), None),
            [
                scope(
                    "https://flow-like-content.s3.eu-central-1.amazonaws.com",
                    "/apps/"
                ),
                scope(
                    "https://s3.eu-central-1.amazonaws.com",
                    "/flow-like-content/apps/"
                ),
            ]
        );
        assert_eq!(
            content_storage_scopes(
                &identity(StorageProviderKind::Aws),
                Some("https://Storage.Example.com/ignored")
            ),
            [scope(
                "https://storage.example.com",
                "/flow-like-content/apps/"
            )]
        );
        assert_eq!(
            content_storage_scopes(&identity(StorageProviderKind::Gcp), None),
            [scope(
                "https://storage.googleapis.com",
                "/flow-like-content/apps/"
            )]
        );
        assert_eq!(
            content_storage_scopes(&identity(StorageProviderKind::Azure), None),
            [scope(
                "https://flowlikeprod.blob.core.windows.net",
                "/flow-like-content/apps/"
            )]
        );
        let r2 = BucketIdentity {
            endpoint: Some("https://acc.r2.cloudflarestorage.com".into()),
            ..identity(StorageProviderKind::R2)
        };
        assert_eq!(
            content_storage_scopes(&r2, None),
            [scope(
                "https://acc.r2.cloudflarestorage.com",
                "/flow-like-content/apps/"
            )]
        );

        for none in [
            identity(StorageProviderKind::Local),
            identity(StorageProviderKind::Memory),
            identity(StorageProviderKind::R2),
            BucketIdentity {
                name: String::new(),
                ..identity(StorageProviderKind::Aws)
            },
            BucketIdentity {
                region: None,
                ..identity(StorageProviderKind::Aws)
            },
        ] {
            assert!(content_storage_scopes(&none, None).is_empty(), "{none:?}");
        }
        for unusable in [
            "http://minio.example.com",
            "https://storage.example.com:9000",
        ] {
            assert!(
                content_storage_scopes(&identity(StorageProviderKind::Aws), Some(unusable))
                    .is_empty(),
                "{unusable} cannot be a CSP path source"
            );
        }

        let mut hub: Hub = serde_json::from_str(include_str!(
            "../../../../../apps/backend/kubernetes/flow-like.config.example.json"
        ))
        .unwrap();
        assert!(hub.widget_storage.is_empty());
        assert!(
            serde_json::to_value(&hub)
                .unwrap()
                .get("widget_storage")
                .is_none()
        );
        hub.widget_storage = content_storage_scopes(&identity(StorageProviderKind::Gcp), None);
        let json = serde_json::to_value(&hub).unwrap();
        assert_eq!(
            json["widget_storage"],
            serde_json::json!([{ "origin": "https://storage.googleapis.com", "path_prefix": "/flow-like-content/apps/" }])
        );
        assert_eq!(
            hub_platform_storage(&hub)[0].path_prefix,
            "/flow-like-content/apps/"
        );
    }
}
