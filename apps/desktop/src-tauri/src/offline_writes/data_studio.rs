//! Data Studio routing (design §4.12): online apps never reach the device databases.

use super::{
    current_hub,
    scope::{AppOfflineScope, OfflineTablePurpose, ScopeKey},
    scope_for, texts, token_subject,
};
use anyhow::anyhow;
use flow_like::{
    app::{App, AppVisibility},
    flow_like_storage::databases::vector::lancedb::{DatabaseSelector, LanceDBVectorStore},
};
use std::sync::Arc;
use tauri::AppHandle;

pub(crate) enum DataStudioTarget {
    /// Offline apps: the device databases, as before.
    Device,
    /// A configured, active table of an online app: its offline copy and queue.
    Managed(Box<LanceDBVectorStore>),
}

/// Rules 1-3: Offline apps use the device; online apps need the token's account and a table.
pub(crate) enum Route<'a> {
    Device,
    Account { subject: String, table: &'a str },
}

pub(crate) fn route<'a>(
    visibility: &AppVisibility,
    table: Option<&'a str>,
    token: Option<&str>,
) -> flow_like_types::Result<Route<'a>> {
    if matches!(visibility, AppVisibility::Offline) {
        return Ok(Route::Device);
    }
    let subject = token_subject(token).ok_or_else(|| anyhow!(texts::SIGN_IN_DATA))?;
    let table = table.ok_or_else(|| anyhow!(texts::HUB_ONLY_TABLES))?;
    Ok(Route::Account { subject, table })
}

async fn visibility(
    app_handle: &AppHandle,
    app_id: &str,
) -> flow_like_types::Result<AppVisibility> {
    let state = crate::state::TauriFlowLikeState::construct(app_handle).await?;
    let app = App::load(app_id.to_owned(), state)
        .await
        .map_err(|error| anyhow!("App '{app_id}' is not available on this device: {error}"))?;
    Ok(app.visibility)
}

/// Table-less device databases (graphs, saved queries) exist only for Offline apps (E33).
pub(crate) async fn ensure_device_app(
    app_handle: &AppHandle,
    app_id: &str,
) -> flow_like_types::Result<()> {
    match visibility(app_handle, app_id).await? {
        AppVisibility::Offline => Ok(()),
        _ => Err(anyhow!(texts::HUB_ONLY_TABLES)),
    }
}

pub(crate) async fn resolve(
    app_handle: &AppHandle,
    app_id: &str,
    table: Option<&str>,
    user_scoped: bool,
    token: Option<&str>,
    selector: Option<DatabaseSelector>,
) -> flow_like_types::Result<DataStudioTarget> {
    let visibility = visibility(app_handle, app_id).await?;
    let Route::Account { subject, table } = route(&visibility, table, token)? else {
        return Ok(DataStudioTarget::Device);
    };
    let key = ScopeKey {
        hub: current_hub(app_handle).await?,
        subject,
        app_id: app_id.to_owned(),
    };
    let scope = scope_for(app_handle, key).await?;
    managed(&scope, table, user_scoped, selector).await
}

/// Rules 4 and 5 of §4.12 within the token account's scope, which must be signed in here.
pub(crate) async fn managed(
    scope: &Arc<AppOfflineScope>,
    table: &str,
    user_scoped: bool,
    selector: Option<DatabaseSelector>,
) -> flow_like_types::Result<DataStudioTarget> {
    if !scope.signed_in() {
        return Err(anyhow!(texts::SIGN_IN_DATA));
    }
    let purpose = OfflineTablePurpose::scoped(user_scoped);
    if scope.configured(purpose, table).is_none() {
        return Err(anyhow!(texts::not_configured(table)));
    }
    let manager = scope
        .manager()
        .await
        .map_err(|error| anyhow!(texts::unavailable(error)))?;
    let path = scope.database_path(purpose);
    match manager
        .managed_store(&path, table, selector.unwrap_or_default())
        .await?
    {
        Some(store) => Ok(DataStudioTarget::Managed(Box::new(store))),
        None => Err(anyhow!(texts::table_not_ready(table))),
    }
}
