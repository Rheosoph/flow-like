use super::SessionBinding;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{
    DialogExt, MessageDialogButtons, MessageDialogKind, MessageDialogResult,
};

#[tauri::command]
pub(crate) async fn native_auth_recover_background_work(
    app_handle: AppHandle,
    profile_id: String,
) -> Result<Value, String> {
    let settings = crate::state::TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|error| error.to_string())?;
    let profile = settings
        .lock()
        .await
        .profiles
        .get(&profile_id)
        .cloned()
        .ok_or("The selected profile no longer exists")?
        .hub_profile;
    let hub = flow_like::hub::hub_origin(&profile.hub, profile.secure)
        .ok_or("This profile has no hub")?;
    let binding = SessionBinding::capture(&profile_id, &hub)
        .await?
        .ok_or("Sign in to this profile before assigning earlier work")?;
    binding.access_token(None).await?;
    let registry = app_handle
        .try_state::<crate::offline_writes::OfflineWrites>()
        .ok_or("Offline storage is not initialized")?;
    let summary = registry
        .legacy_summary(&binding)
        .map_err(|error| error.to_string())?;
    let reports = crate::run_reports::legacy_report_count(&app_handle, &binding)
        .await
        .map_err(|error| error.to_string())?;
    if summary.scopes == 0 && reports == 0 {
        return Ok(
            json!({"recoveredScopes":0,"recoveredReports":0,"conflicts":summary.conflicts,"cancelled":false}),
        );
    }
    let profile_name: String = profile
        .name
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let message = format!(
        "Assign earlier background work to this profile?\n\nProfile: {profile_name}\nProfile ID: {profile_id}\nHub: {hub}\nAccount: {}\n\n{} offline app caches, {} pending changes, and {reports} run reports will use this profile's login.\n\n{} app caches already have data in this profile and will be left unchanged. Cached tables and pending writes remain on this device.",
        binding.subject,
        summary.scopes,
        summary.pending_changes,
        summary.conflicts.len()
    );
    let (sender, receiver) = tokio::sync::oneshot::channel();
    app_handle.dialog().message(message).title("Recover earlier background work")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom("Assign to profile".into(), "Cancel".into()))
        .show_with_result(move |result| {
            let accepted = matches!(result, MessageDialogResult::Ok | MessageDialogResult::Yes)
                || matches!(result, MessageDialogResult::Custom(ref value) if value == "Assign to profile");
            let _ = sender.send(accepted);
        });
    if !tokio::time::timeout(std::time::Duration::from_secs(600), receiver)
        .await
        .map_err(|_| "Recovery confirmation expired")?
        .map_err(|_| "Recovery confirmation closed")?
    {
        return Ok(
            json!({"recoveredScopes":0,"recoveredReports":0,"conflicts":summary.conflicts,"cancelled":true}),
        );
    }
    // The owned worker finishes an approved descriptor commit if its webview closes.
    let app = app_handle.clone();
    tokio::spawn(async move {
        binding.access_token(None).await?;
        let registry = app.try_state::<crate::offline_writes::OfflineWrites>().ok_or("Offline storage is not initialized")?;
        let recovered = registry.recover_legacy(&app, &binding).await.map_err(|error| error.to_string())?;
        if !binding.is_current() { return Err("The profile authentication changed during recovery".into()); }
        let recovered_reports = crate::run_reports::recover_legacy_reports(&app, &binding).await.map_err(|error| error.to_string())?;
        Ok(json!({"recoveredScopes":recovered.scopes,"recoveredReports":recovered_reports,"conflicts":recovered.conflicts,"cancelled":false}))
    }).await.map_err(|_| "Background recovery worker failed")?
}
