//! Public helpers for extracting A2UI components from LLM response text.
//!
//! Re-used by both the rig/bits path (`A2UICopilot`) and the Copilot SDK path
//! so that when a model outputs raw JSON instead of calling tools, we can still
//! recover the component tree.

use crate::a2ui::SurfaceComponent;

/// Everything we can extract from an LLM response that contains A2UI JSON.
#[derive(Debug, Default)]
pub struct ExtractedSurface {
    pub components: Vec<SurfaceComponent>,
    pub root_component_id: Option<String>,
    pub canvas_settings: Option<serde_json::Value>,
}

/// Extract A2UI surface data from an LLM response string.
///
/// Tries, in order:
/// 1. Markdown-fenced ````json … ```` blocks
/// 2. The largest balanced `{ … }` block in the text
pub fn extract_surface_from_response(response: &str) -> ExtractedSurface {
    if let Some(surface) = extract_from_fenced_json(response) {
        return surface;
    }
    if let Some(surface) = extract_from_raw_json(response) {
        return surface;
    }
    ExtractedSurface::default()
}

/// Try each ````json … ```` (and bare ```` … ````) block.
fn extract_from_fenced_json(response: &str) -> Option<ExtractedSurface> {
    let response_lower = response.to_lowercase();

    let mut search_from = 0;
    while let Some(start) = response_lower[search_from..].find("```json") {
        let json_start = search_from + start + 7;
        let json_start = response[json_start..]
            .find(|c: char| !c.is_whitespace() || c == '\n')
            .map(|i| json_start + i)
            .unwrap_or(json_start);
        if let Some(end) = response[json_start..].find("```") {
            let json_str = response[json_start..json_start + end].trim();
            if let Some(surface) = parse_surface_json(json_str) {
                return Some(surface);
            }
        }
        search_from = json_start;
    }

    // Also try bare ``` blocks
    let mut search_from = 0;
    while let Some(start) = response[search_from..].find("```\n") {
        let json_start = search_from + start + 4;
        if let Some(end) = response[json_start..].find("```") {
            let json_str = response[json_start..json_start + end].trim();
            if json_str.starts_with('{') || json_str.starts_with('[') {
                if let Some(surface) = parse_surface_json(json_str) {
                    return Some(surface);
                }
            }
        }
        search_from = json_start;
    }

    None
}

/// Find the largest balanced `{ … }` block in the text.
fn extract_from_raw_json(response: &str) -> Option<ExtractedSurface> {
    let mut best_json: Option<&str> = None;
    let mut best_len = 0;

    let chars: Vec<char> = response.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '{' {
            let start = i;
            let mut depth = 1;
            i += 1;
            while i < chars.len() && depth > 0 {
                match chars[i] {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    '"' => {
                        i += 1;
                        while i < chars.len() && chars[i] != '"' {
                            if chars[i] == '\\' {
                                i += 1;
                            }
                            i += 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            if depth == 0 {
                let byte_start = chars[..start].iter().map(|c| c.len_utf8()).sum::<usize>();
                let byte_end = chars[..i].iter().map(|c| c.len_utf8()).sum::<usize>();
                let candidate = &response[byte_start..byte_end];
                if candidate.len() > best_len {
                    best_len = candidate.len();
                    best_json = Some(candidate);
                }
            }
        } else {
            i += 1;
        }
    }

    best_json.and_then(parse_surface_json)
}

/// Parse a JSON string into an `ExtractedSurface`.
///
/// Handles:
/// - `{"rootComponentId": "…", "canvasSettings": {…}, "components": […]}`
/// - Direct array of components `[…]`
fn parse_surface_json(json_str: &str) -> Option<ExtractedSurface> {
    if let Ok(wrapper) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(components_val) = wrapper.get("components") {
            match serde_json::from_value::<Vec<SurfaceComponent>>(components_val.clone()) {
                Ok(components) if !components.is_empty() => {
                    let root_component_id = wrapper
                        .get("rootComponentId")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let canvas_settings = wrapper.get("canvasSettings").cloned();
                    return Some(ExtractedSurface {
                        components,
                        root_component_id,
                        canvas_settings,
                    });
                }
                _ => {}
            }
        }
    }

    // Try as direct array
    if let Ok(components) = serde_json::from_str::<Vec<SurfaceComponent>>(json_str) {
        if !components.is_empty() {
            return Some(ExtractedSurface {
                components,
                root_component_id: None,
                canvas_settings: None,
            });
        }
    }

    None
}
