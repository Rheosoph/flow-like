use crate::template_profile;
use flow_like::profile::{Profile, Settings};

impl From<template_profile::Model> for Profile {
    fn from(model: template_profile::Model) -> Self {
        let created = model.created_at.and_utc().to_rfc3339();
        let updated = model.updated_at.and_utc().to_rfc3339();

        Self {
            id: model.id,
            name: model.name,
            description: model.description,
            icon: model.icon,
            apps: Some(vec![]),
            shortcuts: Some(vec![]),
            secure: model.secure,
            bits: model.bit_ids.unwrap_or_default().into_inner(),
            custom_bits: vec![],
            hub: model.hub,
            hubs: model.hubs.unwrap_or_default().into_inner(),
            interests: model.interests.unwrap_or_default().into_inner(),
            settings: Settings::default(),
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
            apps: None,
            bit_ids: Some(profile.bits.into()),
            hub: profile.hub,
            hubs: Some(profile.hubs.into()),
            interests: Some(profile.interests.into()),
            settings: None,
            tags: Some(profile.tags.into()),
            thumbnail: profile.thumbnail,
            theme: None,
            created_at: chrono::DateTime::parse_from_rfc3339(&profile.created)
                .unwrap_or_default()
                .naive_utc(),
            updated_at: chrono::DateTime::parse_from_rfc3339(&profile.updated)
                .unwrap_or_default()
                .naive_utc(),
        }
    }
}
