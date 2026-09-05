use sea_orm::sea_query::ExprTrait;
use std::time::Duration;

use crate::{entity::profile as profile_entity, error::ApiError, state::AppState};
use axum::{
    Router,
    routing::{get, post},
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};

pub mod create_default;
pub mod delete_profile;
pub mod get_profile_bits;
pub mod get_profiles;
pub(crate) mod media;
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

pub(crate) fn validate_profile_name(name: &str) -> Result<String, ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(ApiError::bad_request(
            "Profile name must contain between 1 and 100 characters",
        ));
    }
    Ok(name.to_owned())
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
    fn profile_theme_patch_distinguishes_omitted_reset_and_custom() {
        let omitted: ProfileBody = serde_json::from_value(json!({"name":"Workspace"})).unwrap();
        assert_eq!(omitted.theme, None);
        let reset: ProfileBody = serde_json::from_value(json!({"theme":null})).unwrap();
        assert_eq!(reset.theme, Some(None));
        let theme = json!({"id":"Custom", "styles":{}});
        let custom: ProfileBody = serde_json::from_value(json!({"theme":theme})).unwrap();
        assert_eq!(custom.theme, Some(Some(theme)));
    }

    #[test]
    fn execution_preferences_cannot_be_mistaken_for_profile_settings() {
        assert!(
            serde_json::from_value::<ProfileBody>(json!({
                "settings":{"gpu_mode":false,"max_context_size":8192}
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ProfileBody>(json!({
                "name":"Workspace", "settings":{"connection_mode":"simplebezier"}
            }))
            .is_ok()
        );
        assert_eq!(
            super::validate_profile_name("  Workspace  ").unwrap(),
            "Workspace"
        );
        assert!(super::validate_profile_name(" \n ").is_err());
        assert!(super::validate_profile_name(&"x".repeat(101)).is_err());
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
