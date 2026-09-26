use flow_like::app::{App, duplicate::DuplicateReport};
use tauri::AppHandle;
use tracing::info;

use super::sharing::add_app_to_profile;
use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

/// Copies an offline app into a new offline app on this device and adds it to
/// the current profile.
#[tauri::command(async)]
pub async fn duplicate_local_app(
    app_handle: AppHandle,
    app_id: String,
    name: Option<String>,
) -> Result<DuplicateReport, TauriFunctionError> {
    let profile_id = TauriSettingsState::current_profile(&app_handle)
        .await?
        .hub_profile
        .id;
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let name = name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());

    let report = App::duplicate_local(flow_like_state, &app_id, name)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to duplicate app {app_id}: {e}")))?;

    info!(
        target: "duplicate",
        source_app_id = %app_id,
        app_id = %report.app_id,
        files = report.files_copied,
        warnings = report.warnings.len(),
        "Duplicated app"
    );

    add_app_to_profile(&app_handle, &profile_id, &report.app_id).await?;
    Ok(report)
}
