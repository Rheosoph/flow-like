use sea_orm::sea_query::ExprTrait;
use std::time::Duration;

use crate::{entity::profile as profile_entity, error::ApiError, state::AppState};
use axum::{
    Router,
    routing::{get, post},
};
use flow_like_types::create_id;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

pub mod create_default;
pub mod delete_profile;
pub mod get_profile_bits;
pub mod get_profiles;
pub mod sync_profiles;
pub mod upsert_profile;

/// Keep an omitted patch field distinct from an explicit reset to null.
pub(crate) fn deserialize_nullable<'de, D, T>(
    deserializer: D,
) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

pub(crate) fn validate_home_patch(
    layout: &Option<Option<flow_like_types::Value>>,
    default_id: &Option<Option<String>>,
) -> Result<(), ApiError> {
    if let Some(Some(layout)) = layout {
        flow_like::profile::validate_home_layout(layout).map_err(ApiError::bad_request)?;
    }
    if let Some(Some(id)) = default_id
        && (id.is_empty()
            || id.len() > 200
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c)))
    {
        return Err(ApiError::bad_request("Invalid home default ID"));
    }
    Ok(())
}

/// Profile IDs are user-local keys. Historically the database used a global
/// primary key, so insert paths still detect unique violations for rolling
/// deploys and legacy schemas.
pub(crate) async fn find_profile_for_user(
    db: &DatabaseConnection,
    user_id: &str,
    profile_id: &str,
) -> Result<Option<profile_entity::Model>, sea_orm::DbErr> {
    profile_entity::Entity::find()
        .filter(
            profile_entity::Column::UserId
                .eq(user_id)
                .and(profile_entity::Column::Id.eq(profile_id)),
        )
        .one(db)
        .await
}

pub(crate) fn is_profile_unique_violation(err: &sea_orm::DbErr) -> bool {
    let err = format!("{err:?}");
    err.contains("Profile_pkey")
        || (err.contains("23505") && err.contains("Profile"))
        || (err.contains("duplicate key value") && err.contains("Profile"))
}

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

#[cfg(test)]
mod home_tests {
    use super::{sync_profiles::SyncProfileRequest, upsert_profile::ProfileBody};
    use serde_json::json;

    #[test]
    fn profile_partial_update_preserves_omitted_layout_and_recognizes_reset() {
        let omitted: ProfileBody = serde_json::from_value(json!({"name":"Renamed"})).unwrap();
        assert_eq!(omitted.home_layout, None);
        assert_eq!(omitted.home_default_id, None);
        let reset: ProfileBody = serde_json::from_value(json!({"home_layout":null})).unwrap();
        assert_eq!(reset.home_layout, Some(None));
        assert_eq!(reset.home_default_id, None);
        let layout = json!({"version":1,"widgets":[]});
        let custom: ProfileBody =
            serde_json::from_value(json!({"home_layout":layout,"home_default_id":"template"}))
                .unwrap();
        assert_eq!(custom.home_layout, Some(Some(layout)));
        assert_eq!(custom.home_default_id, Some(Some("template".into())));
    }

    #[test]
    fn profile_sync_distinguishes_old_clients_from_reset_and_keeps_lineage() {
        let omitted: SyncProfileRequest =
            serde_json::from_value(json!({"id":"id","name":"Name"})).unwrap();
        assert_eq!(omitted.home_layout, None);
        let reset: SyncProfileRequest = serde_json::from_value(
            json!({"id":"id","name":"Name","home_layout":null,"home_default_id":"template"}),
        )
        .unwrap();
        assert_eq!(reset.home_layout, Some(None));
        assert_eq!(reset.home_default_id, Some(Some("template".into())));
    }
}
