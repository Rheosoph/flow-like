use flow_like_types::Value;
use flow_like_types::interaction::submit_interaction_response;
use tauri::AppHandle;

use crate::functions::TauriFunctionError;

#[tauri::command(async)]
pub async fn respond_to_interaction(
    _app_handle: AppHandle,
    interaction_id: String,
    value: Value,
) -> Result<(), TauriFunctionError> {
    let submitted = submit_interaction_response(&interaction_id, value).await;
    if !submitted {
        return Err(TauriFunctionError::new(&format!(
            "Interaction {} already responded or not found",
            interaction_id
        )));
    }
    Ok(())
}
