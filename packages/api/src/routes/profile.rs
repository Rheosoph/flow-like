use std::time::Duration;

use crate::{error::ApiError, state::AppState};
use axum::{
    Router,
    routing::{get, post},
};
use flow_like_types::create_id;

pub mod create_default;
pub mod delete_profile;
pub mod get_profile_bits;
pub mod get_profiles;
pub mod sync_profiles;
pub mod upsert_profile;

/// Generate a signed upload URL for a profile image and return the filename to store in DB.
/// - Upload path: media/users/{sub}/{cuid}.{ext} (auto-converted to webp)
/// - DB stores: canonical webp filename ({cuid}.webp)
pub(crate) async fn generate_upload_url(
    state: &AppState,
    sub: &str,
    extension: &str,
) -> Result<(String, String), ApiError> {
    let id = create_id();
    let upload_extension = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    let upload_extension = if upload_extension.is_empty() {
        "webp".to_string()
    } else {
        upload_extension
    };
    let upload_filename = format!("{}.{}", id, upload_extension);
    let db_filename = format!("{}.webp", id);

    let upload_path = flow_like_storage::Path::from("media")
        .child("users")
        .child(sub)
        .child(upload_filename.as_str());

    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;
    let signed_url = master_store
        .sign("PUT", &upload_path, Duration::from_secs(60 * 60))
        .await?;

    Ok((signed_url.to_string(), db_filename))
}

/// Delete an old profile image from the private content bucket
pub(crate) async fn delete_old_image(
    state: &AppState,
    sub: &str,
    image_id: &str,
) -> Result<(), ApiError> {
    let file_name = if let Some((stem, _ext)) = image_id.rsplit_once('.') {
        format!("{}.webp", stem)
    } else {
        format!("{}.webp", image_id)
    };
    let path = flow_like_storage::Path::from("media")
        .child("users")
        .child(sub)
        .child(file_name.as_str());

    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;
    let store = master_store.as_generic();
    if let Err(e) = store.delete(&path).await {
        tracing::warn!("Failed to delete old profile image: {}", e);
    }

    Ok(())
}

/// Sign a profile image URL for reading.
/// The icon/thumbnail fields store just the filename, we construct the full path here.
pub async fn sign_profile_image(
    sub: &str,
    image_id: &str,
    state: &AppState,
) -> flow_like_types::Result<String> {
    let master_store = state.master_credentials().await?;
    let master_store = master_store.to_store(false).await?;
    let file_name = if let Some((stem, _ext)) = image_id.rsplit_once('.') {
        format!("{}.webp", stem)
    } else {
        format!("{}.webp", image_id)
    };
    let path = flow_like_storage::Path::from("media")
        .child("users")
        .child(sub)
        .child(file_name);
    let url = master_store
        .sign("GET", &path, Duration::from_secs(60 * 5))
        .await?;
    Ok(url.to_string())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_profiles::get_profiles))
        .route("/sync", post(sync_profiles::sync_profiles))
        .route(
            "/{profile_id}",
            post(upsert_profile::upsert_profile).delete(delete_profile::delete_profile),
        )
        .route(
            "/{profile_id}/bits",
            get(get_profile_bits::get_profile_bits),
        )
}
