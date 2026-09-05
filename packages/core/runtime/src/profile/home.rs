use flow_like_types::Value;
use std::collections::HashSet;

pub const MAX_HOME_LAYOUT_BYTES: usize = 128 * 1024;

/// Validate saved home configuration without depending on the current widget catalog.
/// Unknown widget types survive upgrades and can render an unavailable state.
pub fn validate_home_layout(layout: &Value) -> Result<(), String> {
    if flow_like_types::json::to_vec(layout)
        .map_err(|_| "Home layout is not valid JSON")?
        .len()
        > MAX_HOME_LAYOUT_BYTES
    {
        return Err("Home layout exceeds 128 KiB".into());
    }
    let object = layout.as_object().ok_or("Home layout must be an object")?;
    if object.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Unsupported home layout version".into());
    }
    optional_text(object.get("title"), 256)?;
    optional_text(object.get("description"), 2000)?;
    let widgets = object
        .get("widgets")
        .and_then(Value::as_array)
        .ok_or("Home layout requires a widgets array")?;
    if widgets.len() > 80 {
        return Err("A home layout supports at most 80 widgets".into());
    }
    let mut ids = HashSet::new();
    for widget in widgets {
        let id = required_text(widget.get("id"), 128)?;
        if !ids.insert(id) {
            return Err("Widget IDs must be unique".into());
        }
        required_text(widget.get("type"), 80)?;
        optional_text(widget.get("title"), 256)?;
        optional_text(widget.get("description"), 2000)?;
        let size = widget.get("size").ok_or("Widget size is required")?;
        for (field, maximum) in [("columns", 12), ("rows", 12)] {
            let value = size.get(field).and_then(Value::as_u64).unwrap_or(0);
            if value == 0 || value > maximum {
                return Err(format!("Widget {field} must be between 1 and {maximum}"));
            }
        }
        let appearance = widget
            .get("appearance")
            .ok_or("Widget appearance is required")?;
        required_text(appearance.get("variant"), 80)?;
        required_text(appearance.get("accent"), 128)?;
        if !widget.get("config").is_some_and(Value::is_object) {
            return Err("Widget config must be an object".into());
        }
    }
    Ok(())
}

fn required_text(value: Option<&Value>, maximum: usize) -> Result<&str, String> {
    let value = value.and_then(Value::as_str).ok_or("Expected text")?;
    if value.is_empty() || value.len() > maximum {
        return Err(format!("Text must contain 1 to {maximum} bytes"));
    }
    Ok(value)
}

fn optional_text(value: Option<&Value>, maximum: usize) -> Result<(), String> {
    if let Some(value) = value {
        let value = value.as_str().ok_or("Expected text")?;
        if value.len() > maximum {
            return Err(format!("Text exceeds {maximum} bytes"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    fn layout() -> Value {
        json!({"version":1,"widgets":[{"id":"embed","type":"app-embed",
            "size":{"columns":6,"rows":4},
            "appearance":{"variant":"card","accent":"default"},
            "config":{"appId":"app","path":"/chat?filter=one&filter=two"}}]})
    }

    #[test]
    fn home_layout_roundtrip_preserves_query_parameters_and_default_lineage() {
        let mut profile = super::super::Profile::default();
        profile.home_layout = Some(layout());
        profile.home_default_id = Some("template".into());
        validate_home_layout(profile.home_layout.as_ref().unwrap()).unwrap();
        let restored: super::super::Profile =
            flow_like_types::json::from_slice(&flow_like_types::json::to_vec(&profile).unwrap())
                .unwrap();
        assert_eq!(restored, profile);
        profile.home_layout = None;
        let restored: super::super::Profile =
            flow_like_types::json::from_slice(&flow_like_types::json::to_vec(&profile).unwrap())
                .unwrap();
        assert_eq!(restored.home_layout, None);
        assert_eq!(restored.home_default_id.as_deref(), Some("template"));
    }

    #[test]
    fn rejects_invalid_versions_duplicate_ids_sizes_and_oversized_config() {
        let mut invalid = layout();
        invalid["version"] = json!(2);
        assert!(validate_home_layout(&invalid).is_err());
        let mut invalid = layout();
        let duplicate = invalid["widgets"][0].clone();
        invalid["widgets"].as_array_mut().unwrap().push(duplicate);
        assert!(validate_home_layout(&invalid).is_err());
        let mut invalid = layout();
        invalid["widgets"][0]["size"]["columns"] = json!(13);
        assert!(validate_home_layout(&invalid).is_err());
        let mut invalid = layout();
        invalid["widgets"][0]["config"]["text"] = json!("x".repeat(MAX_HOME_LAYOUT_BYTES));
        assert!(validate_home_layout(&invalid).is_err());
    }
}
