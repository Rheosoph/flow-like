use crate::template_profile;
use flow_like::profile::Profile;

impl From<template_profile::Model> for Profile {
    fn from(model: template_profile::Model) -> Self {
        let created = model.created_at.to_rfc3339();
        let updated = model.updated_at.to_rfc3339();

        Self {
            home_default_id: Some(model.id.clone()),
            home_layout: None,
            id: model.id,
            name: model.name,
            description: model.description,
            icon: model.icon,
            apps: model
                .apps
                .and_then(|apps| serde_json::from_value(apps).ok()),
            shortcuts: Some(vec![]),
            secure: model.secure,
            bits: model.bit_ids.unwrap_or_default().into_inner(),
            custom_bits: vec![],
            hub: model.hub,
            hubs: model.hubs.unwrap_or_default().into_inner(),
            interests: model.interests.unwrap_or_default().into_inner(),
            settings: model
                .settings
                .and_then(|settings| serde_json::from_value(settings).ok())
                .unwrap_or_default(),
            tags: model.tags.unwrap_or_default().into_inner(),
            theme: model.theme,
            thumbnail: model.thumbnail,
            created,
            updated,
        }
    }
}

impl From<Profile> for template_profile::Model {
    fn from(profile: Profile) -> Self {
        Self {
            id: profile.id,
            name: profile.name,
            description: profile.description,
            icon: profile.icon,
            secure: profile.secure,
            apps: profile
                .apps
                .and_then(|apps| serde_json::to_value(apps).ok()),
            bit_ids: Some(profile.bits.into()),
            hub: profile.hub,
            hubs: Some(profile.hubs.into()),
            interests: Some(profile.interests.into()),
            settings: serde_json::to_value(profile.settings).ok(),
            tags: Some(profile.tags.into()),
            thumbnail: profile.thumbnail,
            theme: profile.theme,
            created_at: chrono::DateTime::parse_from_rfc3339(&profile.created).unwrap_or_default(),
            updated_at: chrono::DateTime::parse_from_rfc3339(&profile.updated).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::profile::{ConnectionMode, ProfileApp, ProfileCustomBit, Settings};

    #[test]
    fn starter_template_roundtrip_keeps_apps_settings_images_and_home_lineage() {
        let profile = Profile {
            id: "starter-developer".to_string(),
            name: "Developer".to_string(),
            description: Some("Build workflows with the team.".to_string()),
            icon: Some("https://cdn.example/icon.webp".to_string()),
            thumbnail: Some("https://cdn.example/cover.webp".to_string()),
            bits: vec!["https://hub.example:model".to_string()],
            apps: Some(vec![ProfileApp::new("starter-app".to_string())]),
            settings: Settings {
                connection_mode: ConnectionMode::Straight,
            },
            theme: Some(serde_json::json!({ "appearance": "dark" })),
            home_layout: Some(serde_json::json!({ "version": 1, "widgets": [] })),
            home_default_id: Some("unrelated-default".to_string()),
            ..Profile::default()
        };
        let restored = Profile::from(template_profile::Model::from(profile.clone()));
        assert_eq!(restored.id, profile.id);
        assert_eq!(restored.apps, profile.apps);
        assert_eq!(restored.settings, profile.settings);
        assert_eq!(restored.theme, profile.theme);
        assert_eq!(restored.icon, profile.icon);
        assert_eq!(restored.thumbnail, profile.thumbnail);
        assert_eq!(restored.description, profile.description);
        assert_eq!(restored.bits, profile.bits);
        assert_eq!(
            restored.home_default_id.as_deref(),
            Some("starter-developer")
        );
        assert!(restored.home_layout.is_none());
    }

    #[test]
    fn legacy_template_without_settings_or_apps_loads_with_safe_defaults() {
        let mut model = template_profile::Model::from(Profile::default());
        model.apps = None;
        model.settings = None;
        let restored = Profile::from(model);
        assert!(restored.apps.is_none());
        assert_eq!(restored.settings, Settings::default());
    }

    #[test]
    fn templates_do_not_persist_private_provider_configuration_or_personal_shortcuts() {
        let profile = Profile {
            custom_bits: vec![ProfileCustomBit(flow_like::bit::Bit {
                parameters: serde_json::json!({ "api_key": "fixture-private-secret" }),
                ..flow_like::bit::Bit::default()
            })],
            ..Profile::default()
        };
        let model = template_profile::Model::from(profile);
        assert!(
            !serde_json::to_string(&model)
                .unwrap()
                .contains("fixture-private-secret")
        );
        let restored = Profile::from(model);
        assert!(restored.custom_bits.is_empty());
        assert!(restored.shortcuts.unwrap().is_empty());
    }
}
