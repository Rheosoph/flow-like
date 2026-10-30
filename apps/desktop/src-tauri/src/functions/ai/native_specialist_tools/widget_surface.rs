use super::*;
use std::time::{Duration, Instant};
use tauri::Emitter;

#[derive(Clone)]
struct Capture {
    owner: String,
    observed: Instant,
    snapshot: Value,
}
static CAPTURED: LazyLock<DashMap<String, Capture>> = LazyLock::new(DashMap::new);
fn identity(session: &NativePlatformSession) -> String {
    json!([session.profile.id, session.hub, session.subject]).to_string()
}
fn path(session: &NativePlatformSession) -> Result<std::path::PathBuf, String> {
    Ok(session
        .app_handle
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("flowpilot-widget-proposals")
        .join(format!(
            "{}.json",
            blake3::hash(identity(session).as_bytes()).to_hex()
        )))
}
pub(super) fn snapshot(session: &NativePlatformSession) -> Option<Value> {
    CAPTURED
        .get(&identity(session))
        .filter(|capture| capture.observed.elapsed() < Duration::from_secs(6))
        .map(|capture| capture.snapshot.clone())
}
pub(super) fn fingerprint(snapshot: &Value) -> String {
    contract_fingerprint("widget-surface-v1", snapshot)
}
#[tauri::command]
pub async fn flowpilot_capture_widget_snapshot(
    app_handle: AppHandle,
    webview_window: tauri::WebviewWindow,
    snapshot: Option<Value>,
    expected_owner: Option<Value>,
) -> Result<Value, String> {
    let session = NativePlatformSession::new(&app_handle, None).await?;
    let key = identity(&session);
    let owner = webview_window.label().to_owned();
    CAPTURED.retain(|_, capture| capture.observed.elapsed() < Duration::from_secs(6));
    let Some(snapshot) = snapshot else {
        CAPTURED.remove_if(&key, |_, capture| capture.owner == owner);
        return Ok(json!({"profile_id":session.profile.id}));
    };
    validate_snapshot_owner(&session, expected_owner.as_ref())?;
    if !matches!(snapshot["kind"].as_str(), Some("page" | "widget"))
        || snapshot["surfaceId"].as_str().is_none_or(str::is_empty)
        || snapshot["appId"].as_str().is_none_or(str::is_empty)
    {
        return Err("The widget surface requires exact kind, surfaceId and appId.".into());
    }
    session.require_visible(snapshot["appId"].as_str().unwrap())?;
    if serde_json::to_vec(&snapshot)
        .map_err(|e| e.to_string())?
        .len()
        > 4 * 1024 * 1024
    {
        return Err("The widget editor snapshot is too large.".into());
    }
    let _: Vec<SurfaceComponent> =
        serde_json::from_value(snapshot["currentComponents"].clone()).map_err(|e| e.to_string())?;
    if snapshot["selectedComponentIds"]
        .as_array()
        .is_none_or(|ids| ids.iter().any(|id| !id.is_string()))
    {
        return Err("The selected widget components are invalid.".into());
    }
    session.assert_current_profile().await?;
    let fingerprint = fingerprint(&snapshot);
    if CAPTURED
        .get(&key)
        .is_some_and(|capture| capture.owner != owner && capture.snapshot != snapshot)
    {
        return Err("Another widget editor has a different draft. Close one editor before running FlowPilot.".into());
    }
    CAPTURED.insert(
        key,
        Capture {
            owner,
            observed: Instant::now(),
            snapshot,
        },
    );
    Ok(json!({"profile_id":session.profile.id,"fingerprint":fingerprint}))
}
#[tauri::command]
pub async fn flowpilot_get_widget_proposal(app_handle: AppHandle) -> Result<Option<Value>, String> {
    let session = NativePlatformSession::new(&app_handle, None).await?;
    let bytes = match tokio::fs::read(path(&session)?).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("The retained widget proposal is too large.".into());
    }
    let proposal: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    session.assert_current_profile().await?;
    Ok(Some(proposal))
}
#[tauri::command]
pub async fn flowpilot_resolve_widget_proposal(
    app_handle: AppHandle,
    proposal_id: String,
) -> Result<(), String> {
    let session = NativePlatformSession::new(&app_handle, None).await?;
    let _lock = target_lock(format!("{}:widget-proposal", identity(&session))).await;
    let path = path(&session)?;
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let proposal: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if proposal["proposal_id"] != proposal_id {
        return Err("The retained widget proposal changed.".into());
    }
    session.assert_current_profile().await?;
    tokio::fs::remove_file(path)
        .await
        .map_err(|e| e.to_string())
}
pub(super) async fn stage(
    session: &NativePlatformSession,
    before: &Value,
    response: &UnifiedCopilotResponse,
    request_id: &str,
) -> Result<Value, String> {
    use tokio::io::AsyncWriteExt;
    session.assert_current_profile().await?;
    let _lock = target_lock(format!("{}:widget-proposal", identity(session))).await;
    let proposal = json!({"proposal_id":request_id,"profile_id":session.profile.id,"run_id":session.context.as_ref().and_then(|c|c.run_id.clone()),"conversation_id":session.context.as_ref().and_then(|c|c.conversation_id.clone()),"expected_fingerprint":fingerprint(before),"surface_id":before["surfaceId"],"app_id":before["appId"],"board_id":before["boardId"],"page_id":before["pageId"],"widget_id":before["widgetId"],"components":response.components,"canvas_settings":response.canvas_settings});
    let mut proposal = proposal;
    proposal["owner"] = snapshot_owner(session);
    let bytes = serde_json::to_vec(&proposal).map_err(|e| e.to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("The widget proposal is too large to retain.".into());
    }
    let path = path(session)?;
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .map_err(|e| e.to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", flow_like_types::create_id()));
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&temporary).await.map_err(|e| e.to_string())?;
    file.write_all(&bytes).await.map_err(|e| e.to_string())?;
    file.sync_all().await.map_err(|e| e.to_string())?;
    session.assert_current_profile().await?;
    tokio::fs::rename(temporary, path)
        .await
        .map_err(|e| e.to_string())?;
    let _ = session
        .app_handle
        .emit("flowpilot:widget-components-staged", &proposal);
    Ok(
        json!({"status":"ok","staged":true,"applied":false,"proposal_id":request_id,"component_count":response.components.len(),"app_id":before["appId"],"page_id":before["pageId"],"widget_id":before["widgetId"],"specialist_message":response.message,"message":"Generated components are retained for review. Apply them in the matching builder; they have not changed the saved page or widget."}),
    )
}
