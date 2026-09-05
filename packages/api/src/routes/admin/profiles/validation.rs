use crate::error::ApiError;
use flow_like::profile::Profile;
use std::collections::HashSet;

pub(super) fn validate_template_id(id: &str) -> Result<(), ApiError> {
    if id.is_empty()
        || id.len() > 200
        || matches!(id, "main" | "media")
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(ApiError::bad_request(
            "Template IDs must contain 1 to 200 letters, numbers, dots, underscores or hyphens; main and media are reserved.",
        ));
    }
    Ok(())
}

fn validate_text(
    value: &str,
    label: &str,
    limit: usize,
    allow_empty: bool,
) -> Result<(), ApiError> {
    if (!allow_empty && value.trim().is_empty())
        || value.chars().count() > limit
        || value.contains('\0')
    {
        return Err(ApiError::bad_request(format!(
            "{label} must {}contain at most {limit} characters.",
            if allow_empty { "" } else { "be nonempty and " },
        )));
    }
    Ok(())
}

fn normalize_media(value: &mut Option<String>, label: &str) -> Result<(), ApiError> {
    let Some(text) = value else {
        return Ok(());
    };
    *text = text.trim().to_string();
    if text.is_empty() {
        *value = None;
        return Ok(());
    }
    validate_text(text, label, 2048, false)?;
    let valid = reqwest::Url::parse(text).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
    });
    if !valid || text.chars().any(char::is_control) {
        return Err(ApiError::bad_request(format!(
            "{label} must be an HTTP or HTTPS image URL without credentials."
        )));
    }
    Ok(())
}

fn normalize_list(
    values: &mut Vec<String>,
    label: &str,
    count: usize,
    length: usize,
) -> Result<(), ApiError> {
    if values.len() > count {
        return Err(ApiError::bad_request(format!(
            "{label} can contain at most {count} items."
        )));
    }
    let mut seen = HashSet::new();
    for value in values.iter_mut() {
        *value = value.trim().to_string();
        validate_text(value, label, length, false)?;
    }
    values.retain(|value| seen.insert(value.clone()));
    Ok(())
}

pub(super) fn prepare_template(profile_id: &str, profile: &mut Profile) -> Result<(), ApiError> {
    validate_template_id(profile_id)?;
    if serde_json::to_vec(profile)?.len() > 256 * 1024 {
        return Err(ApiError::bad_request(
            "A profile template must be at most 256 KiB.",
        ));
    }
    profile.id = profile_id.to_string();
    profile.name = profile.name.trim().to_string();
    validate_text(&profile.name, "Template name", 120, false)?;
    if let Some(description) = &profile.description {
        validate_text(description, "Description", 10_000, true)?;
    }
    normalize_media(&mut profile.icon, "Icon")?;
    normalize_media(&mut profile.thumbnail, "Cover image")?;
    profile.hub = profile.hub.trim().to_string();
    validate_text(&profile.hub, "Hub", 2048, true)?;
    normalize_list(&mut profile.hubs, "Additional hubs", 50, 2048)?;
    normalize_list(&mut profile.bits, "Model references", 500, 2048)?;
    normalize_list(&mut profile.tags, "Tags", 50, 120)?;
    normalize_list(&mut profile.interests, "Interests", 50, 120)?;
    if let Some(apps) = &mut profile.apps {
        if apps.len() > 500 {
            return Err(ApiError::bad_request(
                "A template can contain at most 500 apps.",
            ));
        }
        for app in apps.iter_mut() {
            app.app_id = app.app_id.trim().to_string();
            validate_text(&app.app_id, "App ID", 200, false)?;
        }
        let mut seen = HashSet::new();
        apps.retain(|app| seen.insert(app.app_id.clone()));
    }
    if profile.home_layout.is_some() {
        return Err(ApiError::bad_request(
            "Publish a template's home layout through the Home defaults editor.",
        ));
    }
    profile.home_default_id = Some(profile_id.to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starter() -> Profile {
        Profile {
            name: "Developer".to_string(),
            ..Profile::default()
        }
    }

    #[test]
    fn request_path_controls_new_template_id_and_default_lineage() {
        let mut profile = starter();
        profile.id = "body-id".to_string();
        profile.home_default_id = Some("another-default".to_string());
        prepare_template("requested-id", &mut profile).unwrap();
        assert_eq!(profile.id, "requested-id");
        assert_eq!(profile.home_default_id.as_deref(), Some("requested-id"));
    }

    #[test]
    fn template_fields_are_trimmed_deduplicated_and_media_can_be_cleared() {
        let mut profile = starter();
        profile.name = " Developer ".to_string();
        profile.bits = vec![
            " https://hub.example:model ".to_string(),
            "https://hub.example:model".to_string(),
        ];
        profile.icon = Some(" ".to_string());
        profile.apps = Some(vec![
            flow_like::profile::ProfileApp::new(" starter-app ".to_string()),
            flow_like::profile::ProfileApp::new("starter-app".to_string()),
        ]);
        prepare_template("starter", &mut profile).unwrap();
        assert_eq!(profile.name, "Developer");
        assert_eq!(profile.bits, vec!["https://hub.example:model"]);
        assert!(profile.icon.is_none());
        assert_eq!(profile.apps.unwrap().len(), 1);
    }

    #[test]
    fn reserved_ids_invalid_images_and_out_of_bounds_content_are_rejected() {
        for id in ["main", "media", "", "path/id", "id?query"] {
            assert!(prepare_template(id, &mut starter()).is_err());
        }
        for icon in [
            "javascript:alert(1)",
            "data:image/png;base64,abc",
            "https://user:password@host/image.webp",
        ] {
            let mut profile = starter();
            profile.icon = Some(icon.to_string());
            assert!(prepare_template("starter", &mut profile).is_err());
        }
        let mut profile = starter();
        profile.name = "x".repeat(121);
        assert!(prepare_template("starter", &mut profile).is_err());
        profile.name = "Developer".to_string();
        profile.bits = vec!["model".to_string(); 501];
        assert!(prepare_template("starter", &mut profile).is_err());
    }

    #[test]
    fn template_writes_cannot_publish_home_layouts_or_oversized_theme_json() {
        let mut profile = starter();
        profile.home_layout = Some(serde_json::json!({ "version": 1, "widgets": [] }));
        assert!(prepare_template("starter", &mut profile).is_err());
        profile.home_layout = None;
        profile.theme = Some(serde_json::json!({ "content": "x".repeat(256 * 1024) }));
        assert!(prepare_template("starter", &mut profile).is_err());
    }
}
