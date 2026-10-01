//! Offline access commands and their DTOs (design §4.11).

use super::scope::{ConfiguredTable, ScopeKey, sync_state};
pub(crate) use super::scope::{OfflineLimitsDto, OfflineTablePurpose, OfflineTableStatus};
use super::{OfflineWrites, current_hub, scope::AppOfflineScope, scope_for, texts, token_subject};
use crate::functions::TauriFunctionError;
use anyhow::{Context, Result, anyhow, ensure};
use flow_like::{
    app::{App, AppVisibility},
    credentials::SharedCredentials,
};
use flow_like_device_protocol::{OfflineExpected, OfflineLimits, OfflineResource, StoragePurpose};
use flow_like_offline_writes::{OperationLookup, OperationSummary, TableState, WriteManager};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

const DEFAULT_PAGE: u32 = 50;
const MAX_PAGE: u32 = 256;
const OVERVIEW_BLOCKED_HEADS: u32 = 16;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineTableSelection {
    pub(crate) purpose: OfflineTablePurpose,
    pub(crate) table: String,
    pub(crate) primary_key: String,
    #[serde(default)]
    pub(crate) prefetch: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineTableState {
    pub(crate) purpose: OfflineTablePurpose,
    pub(crate) table: String,
    pub(crate) primary_key: String,
    pub(crate) prefetch: bool,
    pub(crate) status: OfflineTableStatus,
    pub(crate) error: Option<String>,
    pub(crate) pending_count: u64,
    pub(crate) snapshot_version: Option<u64>,
    pub(crate) refreshed_at: Option<i64>,
    pub(crate) cached_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) local_bytes: u64,
    pub(crate) offline_complete: bool,
    pub(crate) downloading: bool,
    pub(crate) key_indexed: Option<bool>,
    pub(crate) mirror_error: Option<String>,
    pub(crate) waiting_for_runs: u32,
    pub(crate) remote_missing: bool,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OfflineSyncState {
    Idle,
    Syncing,
    WaitingForConnection,
    WaitingForSignIn,
    HubError,
    Blocked,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OfflineOperationKind {
    TableInsert,
    TableUpsert,
    TableUpdate,
    TableDelete,
    FileWrite,
    FileDelete,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum OfflineResourceLabel {
    Table {
        purpose: OfflineTablePurpose,
        table: String,
    },
    File {
        purpose: String,
        path: String,
    },
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineOperation {
    pub(crate) sequence: u64,
    pub(crate) operation_id: String,
    pub(crate) kind: OfflineOperationKind,
    pub(crate) resource: OfflineResourceLabel,
    pub(crate) state: String,
    pub(crate) attempts: u32,
    pub(crate) created_at: i64,
    pub(crate) bytes: u64,
    pub(crate) error: Option<String>,
    pub(crate) error_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineQueueStatus {
    pub(crate) pending_count: u64,
    pub(crate) pending_bytes: u64,
    pub(crate) oldest_at: Option<i64>,
    pub(crate) sync_state: OfflineSyncState,
    pub(crate) blocked_heads: Vec<OfflineOperation>,
}

impl Default for OfflineQueueStatus {
    fn default() -> Self {
        Self {
            pending_count: 0,
            pending_bytes: 0,
            oldest_at: None,
            sync_state: OfflineSyncState::Idle,
            blocked_heads: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineHubLimits {
    pub(crate) max_operation_bytes: u64,
    pub(crate) max_file_bytes: u64,
    pub(crate) max_request_bytes: Option<u64>,
}

impl From<OfflineLimits> for OfflineHubLimits {
    fn from(limits: OfflineLimits) -> Self {
        Self {
            max_operation_bytes: limits.max_operation_bytes as u64,
            max_file_bytes: limits.max_file_bytes as u64,
            max_request_bytes: limits.max_request_bytes.map(|bytes| bytes as u64),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineUsage {
    pub(crate) queue_bytes: u64,
    pub(crate) mirror_bytes: u64,
    pub(crate) cache_bytes: u64,
    pub(crate) pinned_bytes: u64,
    pub(crate) required_bytes: u64,
    pub(crate) downloaded_bytes_today: u64,
    pub(crate) max_download_bytes_per_day: Option<u64>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineAppOverview {
    pub(crate) app_id: String,
    pub(crate) available: bool,
    pub(crate) unavailable_reason: Option<String>,
    pub(crate) subject: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) hub_support: Option<bool>,
    pub(crate) hub_limits: Option<OfflineHubLimits>,
    pub(crate) tables: Vec<OfflineTableState>,
    pub(crate) queue: OfflineQueueStatus,
    pub(crate) limits: OfflineLimitsDto,
    pub(crate) usage: OfflineUsage,
    pub(crate) other_accounts_pending: u64,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OfflineOperationLookupState {
    Pending,
    Attempting,
    Blocked,
    Conflict,
    OutcomeUnknown,
    Applied,
    Skipped,
    Superseded,
    Unknown,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineOperationLookup {
    pub(crate) state: OfflineOperationLookupState,
    pub(crate) superseded_by: Option<String>,
    pub(crate) error_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OfflineTableRoute {
    Device,
    Hub,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineKeepBothResult {
    pub(crate) new_path: String,
}

/// A configured table merged with the engine's state; mirror fields stay empty while unregistered.
pub(crate) fn table_state(
    table: &ConfiguredTable,
    engine: Option<&TableState>,
    waiting_for_runs: u32,
) -> OfflineTableState {
    let snapshot_version = engine.and_then(|state| match &state.revision {
        Some(OfflineExpected::TableVersion { version, .. }) => Some(*version),
        _ => None,
    });
    OfflineTableState {
        purpose: table.purpose,
        table: table.table.clone(),
        primary_key: table.primary_key.clone(),
        prefetch: engine.map_or(table.prefetch, |state| state.prefetch),
        status: table.status,
        error: table.error.clone(),
        pending_count: engine.map_or(0, |state| state.pending),
        snapshot_version,
        refreshed_at: engine.and_then(|state| state.refreshed_at),
        cached_bytes: engine.map_or(0, |state| state.cached_bytes),
        total_bytes: engine.and_then(|state| state.total_bytes),
        local_bytes: engine.map_or(0, |state| state.local_bytes),
        offline_complete: engine.is_some_and(|state| state.offline_complete),
        downloading: engine.is_some_and(|state| state.downloading),
        key_indexed: engine.and_then(|state| state.key_indexed),
        mirror_error: engine.and_then(|state| state.mirror_error.clone()),
        waiting_for_runs: if table.status == OfflineTableStatus::Settling {
            waiting_for_runs
        } else {
            0
        },
        remote_missing: engine.is_some_and(|state| state.remote_missing),
    }
}

fn operation_kind(summary: &OperationSummary) -> OfflineOperationKind {
    match summary.mutation_kind.as_deref() {
        Some("table_insert") => OfflineOperationKind::TableInsert,
        Some("table_upsert") => OfflineOperationKind::TableUpsert,
        Some("table_update") => OfflineOperationKind::TableUpdate,
        Some("table_delete") => OfflineOperationKind::TableDelete,
        Some("file_delete") => OfflineOperationKind::FileDelete,
        _ => OfflineOperationKind::FileWrite,
    }
}

fn resource_label(resource: &str) -> OfflineResourceLabel {
    match serde_json::from_str::<OfflineResource>(resource) {
        Ok(OfflineResource::Table { purpose, table, .. }) => OfflineResourceLabel::Table {
            purpose: OfflineTablePurpose::from_storage(purpose)
                .unwrap_or(OfflineTablePurpose::Storage),
            table,
        },
        Ok(OfflineResource::File { purpose, path }) => OfflineResourceLabel::File {
            purpose: purpose.as_str().to_owned(),
            path,
        },
        Err(_) => OfflineResourceLabel::File {
            purpose: StoragePurpose::Files.as_str().to_owned(),
            path: resource.to_owned(),
        },
    }
}

pub(crate) fn operation(summary: OperationSummary) -> OfflineOperation {
    OfflineOperation {
        kind: operation_kind(&summary),
        resource: resource_label(&summary.resource),
        sequence: summary.sequence,
        operation_id: summary.operation_id,
        state: summary.state,
        attempts: summary.attempts,
        created_at: summary.created_at,
        bytes: summary.bytes,
        error: summary.error,
        error_code: summary.error_code,
    }
}

pub(crate) fn lookup(lookup: Option<OperationLookup>) -> OfflineOperationLookup {
    let Some(lookup) = lookup else {
        return OfflineOperationLookup {
            state: OfflineOperationLookupState::Unknown,
            superseded_by: None,
            error_code: None,
        };
    };
    let state = match lookup.state.as_str() {
        "pending" => OfflineOperationLookupState::Pending,
        "attempting" => OfflineOperationLookupState::Attempting,
        "conflict" => OfflineOperationLookupState::Conflict,
        "outcome_unknown" => OfflineOperationLookupState::OutcomeUnknown,
        "applied" => OfflineOperationLookupState::Applied,
        "skipped" => OfflineOperationLookupState::Skipped,
        "superseded" => OfflineOperationLookupState::Superseded,
        _ => OfflineOperationLookupState::Blocked,
    };
    OfflineOperationLookup {
        state,
        superseded_by: lookup.superseded_by,
        error_code: lookup.error_code,
    }
}

/// Pages of queued operations, 50 by default and at most 256.
pub(crate) fn operations(
    manager: &WriteManager,
    after_sequence: Option<u64>,
    limit: Option<u32>,
) -> Result<Vec<OfflineOperation>> {
    let limit = limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    Ok(manager
        .queue()
        .list_operations(after_sequence, limit)?
        .into_iter()
        .map(operation)
        .collect())
}

fn queue_status(scope: &AppOfflineScope, manager: &WriteManager) -> Result<OfflineQueueStatus> {
    let status = manager.queue().status()?;
    let blocked: Vec<OfflineOperation> = manager
        .queue()
        .blocked_heads(OVERVIEW_BLOCKED_HEADS)?
        .into_iter()
        .map(operation)
        .collect();
    Ok(OfflineQueueStatus {
        pending_count: status.pending_count,
        pending_bytes: status.pending_bytes,
        oldest_at: status.oldest_at,
        sync_state: sync_state(blocked.len(), scope.transport(), status.pending_count),
        blocked_heads: blocked,
    })
}

/// Table states of a scope; a table the cloud deleted is recorded as failed with E25.
pub(crate) async fn table_states(
    scope: &AppOfflineScope,
    manager: Option<&Arc<WriteManager>>,
) -> Result<Vec<OfflineTableState>> {
    let engine = match manager {
        Some(manager) => manager.table_states().await?,
        None => Vec::new(),
    };
    let mut states = Vec::new();
    for table in scope.descriptor().tables {
        let state = engine.iter().find(|state| state.table == table.selection());
        let mut table = table;
        if state.is_some_and(|state| state.remote_missing)
            && table.status != OfflineTableStatus::Error
        {
            let error = texts::deleted_in_cloud(&table.table);
            scope.update_descriptor(|descriptor| {
                if let Some(entry) = descriptor
                    .tables
                    .iter_mut()
                    .find(|entry| entry.purpose == table.purpose && entry.table == table.table)
                {
                    entry.status = OfflineTableStatus::Error;
                    entry.error = Some(error.clone());
                }
            })?;
            table.status = OfflineTableStatus::Error;
            table.error = Some(error);
        }
        states.push(table_state(&table, state, scope.waiting_for_runs(&table)));
    }
    Ok(states)
}

fn unavailable_overview(
    app_id: &str,
    reason: &str,
    subject: Option<String>,
    other_accounts_pending: u64,
) -> OfflineAppOverview {
    OfflineAppOverview {
        app_id: app_id.to_owned(),
        available: false,
        unavailable_reason: Some(reason.to_owned()),
        subject,
        provider: None,
        hub_support: None,
        hub_limits: None,
        tables: Vec::new(),
        queue: OfflineQueueStatus::default(),
        limits: OfflineLimitsDto::default(),
        usage: OfflineUsage::default(),
        other_accounts_pending,
    }
}

/// The Offline access page of one scope.
pub(crate) async fn overview(
    registry: &OfflineWrites,
    scope: &Arc<AppOfflineScope>,
) -> Result<OfflineAppOverview> {
    let manager = scope.manager().await.ok();
    let descriptor = scope.descriptor();
    let other_accounts_pending = registry.other_accounts_pending(scope.app_id(), scope.id());
    let (queue, usage) = match &manager {
        Some(manager) => {
            let queue = queue_status(scope, manager)?;
            let mirror = manager.mirror_usage()?;
            let usage = OfflineUsage {
                queue_bytes: queue.pending_bytes,
                mirror_bytes: mirror.used,
                cache_bytes: mirror.cache_bytes,
                pinned_bytes: mirror.pinned_bytes,
                required_bytes: mirror.required,
                downloaded_bytes_today: mirror.downloaded_today,
                max_download_bytes_per_day: mirror.max_download_bytes_per_day,
            };
            (queue, usage)
        }
        None => (OfflineQueueStatus::default(), OfflineUsage::default()),
    };
    Ok(OfflineAppOverview {
        app_id: scope.app_id().to_owned(),
        available: manager.is_some(),
        unavailable_reason: scope.manager_error().map(texts::unavailable),
        subject: Some(scope.subject().to_owned()),
        provider: descriptor.provider.clone(),
        hub_support: descriptor.hub_support,
        hub_limits: descriptor.hub_limits.map(OfflineHubLimits::from),
        tables: table_states(scope, manager.as_ref()).await?,
        queue,
        limits: descriptor.limits,
        usage,
        other_accounts_pending,
    })
}

/// The route of a Data Studio table: configured tables in any status stay on the device.
pub(crate) fn table_route(
    scope: &AppOfflineScope,
    table: &str,
    user_scoped: bool,
) -> OfflineTableRoute {
    match scope.configured(OfflineTablePurpose::scoped(user_scoped), table) {
        Some(_) => OfflineTableRoute::Device,
        None => OfflineTableRoute::Hub,
    }
}

fn error(error: impl std::fmt::Display) -> TauriFunctionError {
    TauriFunctionError::new(&error.to_string())
}

async fn visibility(app_handle: &AppHandle, app_id: &str) -> Result<AppVisibility> {
    let state = crate::state::TauriFlowLikeState::construct(app_handle).await?;
    let app = App::load(app_id.to_owned(), state)
        .await
        .with_context(|| format!("App '{app_id}' is not available on this device"))?;
    Ok(app.visibility)
}

/// The scope of the token's account on the current profile's hub.
async fn scope_of(
    app_handle: &AppHandle,
    app_id: &str,
    token: Option<&str>,
) -> Result<Arc<AppOfflineScope>> {
    let subject = token_subject(token).ok_or_else(|| anyhow!(texts::SIGN_IN_MANAGE))?;
    let key = ScopeKey {
        hub: current_hub(app_handle).await?,
        subject,
        app_id: app_id.to_owned(),
    };
    scope_for(app_handle, key).await
}

/// Online apps only; every command derives the scope from the UI's token.
async fn command_scope(
    app_handle: &AppHandle,
    app_id: &str,
    token: Option<&str>,
) -> Result<Arc<AppOfflineScope>> {
    if matches!(
        visibility(app_handle, app_id).await?,
        AppVisibility::Offline
    ) {
        return Err(anyhow!(texts::ONLINE_PROJECTS_ONLY));
    }
    scope_of(app_handle, app_id, token).await
}

/// State-changing commands also need a signed-in session of the token's account; the
/// token's `sub` alone is not verified on the device.
async fn signed_in_scope(
    app_handle: &AppHandle,
    app_id: &str,
    token: &str,
) -> Result<Arc<AppOfflineScope>> {
    signed_in(command_scope(app_handle, app_id, Some(token)).await?)
}

pub(crate) fn signed_in(scope: Arc<AppOfflineScope>) -> Result<Arc<AppOfflineScope>> {
    ensure!(scope.signed_in(), texts::SIGN_IN_MANAGE);
    Ok(scope)
}

#[tauri::command(async)]
pub async fn offline_writes_overview(
    app_handle: AppHandle,
    app_id: String,
    token: Option<String>,
) -> Result<OfflineAppOverview, TauriFunctionError> {
    let registry = app_handle
        .try_state::<OfflineWrites>()
        .ok_or_else(|| error("Offline changes are not initialized on this device"))?;
    if matches!(
        visibility(&app_handle, &app_id).await?,
        AppVisibility::Offline
    ) {
        return Ok(unavailable_overview(
            &app_id,
            texts::ONLINE_PROJECTS_ONLY,
            None,
            0,
        ));
    }
    if token_subject(token.as_deref()).is_none() {
        let pending = registry.other_accounts_pending(&app_id, "");
        return Ok(unavailable_overview(
            &app_id,
            texts::SIGN_IN_MANAGE,
            None,
            pending,
        ));
    }
    let scope = scope_of(&app_handle, &app_id, token.as_deref()).await?;
    Ok(overview(&registry, &scope).await?)
}

#[tauri::command(async)]
pub async fn offline_writes_set_table(
    app_handle: AppHandle,
    webview: tauri::Webview,
    app_id: String,
    selection: OfflineTableSelection,
    token: String,
    session_id: String,
) -> Result<OfflineTableState, TauriFunctionError> {
    let scope = signed_in_scope(&app_handle, &app_id, &token).await?;
    ConfiguredTable::from_selection(&selection)?;
    let prepared = crate::execution_credentials::prepare(
        scope.hub(),
        &app_id,
        Some(&token),
        Some(&session_id),
        Some(webview.label()),
    )
    .await
    .map_err(|failure| error(texts::capabilities_unreachable(&failure)))?;
    if let SharedCredentials::Renewable(live) = &prepared {
        scope.offer_lease(Arc::downgrade(live));
    }
    match scope.refresh_capabilities(&token).await {
        Ok(true) => {}
        Ok(false) => return Err(error(texts::HUB_UNSUPPORTED)),
        Err(failure) => return Err(error(texts::capabilities_unreachable(&failure))),
    }
    let table = scope.begin_table(&selection).await?;
    let state = table_state(&table, None, 0);
    if scope.setup_pending(&table) {
        let configure = scope.clone();
        tokio::spawn(async move {
            let _lease = prepared;
            configure.configure_table(table).await;
        });
    }
    Ok(state)
}

#[tauri::command(async)]
pub async fn offline_writes_remove_table(
    app_handle: AppHandle,
    app_id: String,
    token: String,
    purpose: OfflineTablePurpose,
    table: String,
) -> Result<(), TauriFunctionError> {
    let scope = signed_in_scope(&app_handle, &app_id, &token).await?;
    Ok(scope.remove_table(purpose, &table).await?)
}

#[tauri::command(async)]
pub async fn offline_writes_set_prefetch(
    app_handle: AppHandle,
    app_id: String,
    token: String,
    purpose: OfflineTablePurpose,
    table: String,
    prefetch: bool,
) -> Result<OfflineTableState, TauriFunctionError> {
    let scope = signed_in_scope(&app_handle, &app_id, &token).await?;
    Ok(scope.set_prefetch(purpose, &table, prefetch).await?)
}

#[tauri::command(async)]
pub async fn offline_writes_set_limits(
    app_handle: AppHandle,
    app_id: String,
    token: String,
    limits: OfflineLimitsDto,
) -> Result<OfflineLimitsDto, TauriFunctionError> {
    let scope = signed_in_scope(&app_handle, &app_id, &token).await?;
    Ok(scope.set_limits(limits).await?)
}

#[tauri::command(async)]
pub async fn offline_writes_operations(
    app_handle: AppHandle,
    app_id: String,
    token: String,
    after_sequence: Option<u64>,
    limit: Option<u32>,
) -> Result<Vec<OfflineOperation>, TauriFunctionError> {
    let scope = command_scope(&app_handle, &app_id, Some(&token)).await?;
    let manager = scope.manager().await?;
    Ok(operations(&manager, after_sequence, limit)?)
}

#[tauri::command(async)]
pub async fn offline_writes_operation_state(
    app_handle: AppHandle,
    app_id: String,
    token: String,
    operation_id: String,
) -> Result<OfflineOperationLookup, TauriFunctionError> {
    let scope = command_scope(&app_handle, &app_id, Some(&token)).await?;
    let manager = scope.manager().await?;
    Ok(lookup(manager.queue().operation_state(&operation_id)?))
}

#[tauri::command(async)]
pub async fn offline_writes_retry(
    app_handle: AppHandle,
    app_id: String,
    token: String,
    operation_id: String,
) -> Result<(), TauriFunctionError> {
    let scope = signed_in_scope(&app_handle, &app_id, &token).await?;
    Ok(scope.retry(&operation_id).await?)
}

#[tauri::command(async)]
pub async fn offline_writes_skip(
    app_handle: AppHandle,
    app_id: String,
    token: String,
    operation_id: String,
    reason: String,
    acknowledge_uncertain: bool,
) -> Result<(), TauriFunctionError> {
    let scope = signed_in_scope(&app_handle, &app_id, &token).await?;
    Ok(scope
        .skip(&operation_id, &reason, acknowledge_uncertain)
        .await?)
}

#[tauri::command(async)]
pub async fn offline_writes_keep_both(
    app_handle: AppHandle,
    app_id: String,
    token: String,
    operation_id: String,
) -> Result<OfflineKeepBothResult, TauriFunctionError> {
    let scope = signed_in_scope(&app_handle, &app_id, &token).await?;
    Ok(OfflineKeepBothResult {
        new_path: scope.keep_both(&operation_id).await?,
    })
}

#[tauri::command(async)]
pub async fn offline_writes_sync_now(
    app_handle: AppHandle,
    app_id: String,
    token: String,
) -> Result<(), TauriFunctionError> {
    let scope = signed_in_scope(&app_handle, &app_id, &token).await?;
    Ok(scope.sync_now().await?)
}

#[tauri::command(async)]
pub async fn offline_writes_table_route(
    app_handle: AppHandle,
    app_id: String,
    token: String,
    table: String,
    user_scoped: bool,
) -> Result<OfflineTableRoute, TauriFunctionError> {
    if token_subject(Some(&token)).is_none() {
        return Ok(OfflineTableRoute::Hub);
    }
    let scope = scope_of(&app_handle, &app_id, Some(&token)).await?;
    Ok(table_route(&scope, &table, user_scoped))
}

#[tauri::command(async)]
pub async fn offline_writes_forget_app(
    app_handle: AppHandle,
    app_id: String,
    token: Option<String>,
    all_accounts: bool,
) -> Result<(), TauriFunctionError> {
    let registry = app_handle
        .try_state::<OfflineWrites>()
        .ok_or_else(|| error("Offline changes are not initialized on this device"))?;
    let current = match token_subject(token.as_deref()) {
        Some(subject) => Some(ScopeKey {
            hub: current_hub(&app_handle).await?,
            subject,
            app_id: app_id.clone(),
        }),
        None => None,
    };
    registry
        .forget(&app_id, current.as_ref(), all_accounts)
        .await?;
    super::emit(
        Some(&app_handle),
        super::TABLES_EVENT,
        super::AppEvent {
            app_id: app_id.clone(),
        },
    );
    super::emit(
        Some(&app_handle),
        super::STATUS_EVENT,
        super::scope::StatusEvent {
            app_id,
            pending_count: 0,
            sync_state: OfflineSyncState::Idle,
            head_state: None,
            blocked_count: 0,
        },
    );
    Ok(())
}
