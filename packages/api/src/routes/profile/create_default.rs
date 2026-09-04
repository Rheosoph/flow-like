use crate::{entity::profile, error::ApiError, state::AppState};
use flow_like_types::{Value, create_id};
use sea_orm::{ActiveModelTrait, ActiveValue::Set};
use serde_json::json;

pub async fn create_default_profile(
    state: &AppState,
    user_id: &str,
) -> Result<profile::Model, ApiError> {
    let config = &state.platform_config;

    // Get default hub from config
    let default_hub = if config.domain.is_empty() {
        "api.flow-like.com".to_string()
    } else {
        config.domain.clone()
    };

    let profile_id = create_id();

    // Create default profile based on config
    let new_profile = profile::ActiveModel {
        id: Set(profile_id),
        user_id: Set(user_id.to_string()),
        name: Set("Default Profile".to_string()),
        description: Set(Some("Your default profile".to_string())),
        interests: Set(Some(Default::default())),
        tags: Set(Some(Default::default())),
        theme: Set(None),
        bit_ids: Set(Some(Default::default())),
        apps: Set(Some(Value::Array(vec![]))),
        settings: Set(Some(json!({
            "connection_mode": "simplebezier"
        }))),
        hub: Set(default_hub.clone()),
        hubs: Set(Some(vec![default_hub].into())),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        ..Default::default()
    };

    let created_profile = new_profile.insert(&state.db).await?;
    Ok(created_profile)
}
