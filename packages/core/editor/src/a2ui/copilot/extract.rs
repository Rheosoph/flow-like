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
/// 2. Balanced `{ … }` blocks in the text, largest first
pub fn extract_surface_from_response(response: &str) -> ExtractedSurface {
    if let Some(surface) = extract_from_fenced_json(response) {
        return surface;
    }
    if let Some(surface) = extract_from_raw_json(response) {
        return surface;
    }
    ExtractedSurface::default()
}

/// ASCII-case-insensitive substring search that returns byte offsets valid in
/// `haystack` (unlike searching a `to_lowercase()` copy, whose byte offsets
/// diverge as soon as a character changes UTF-8 length when lowercased).
fn find_ascii_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let haystack = haystack.as_bytes().get(from..)?;
    let needle = needle.as_bytes();
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
        .map(|i| from + i)
}

/// Try each ````json … ```` (and bare ```` … ````) block.
fn extract_from_fenced_json(response: &str) -> Option<ExtractedSurface> {
    let mut search_from = 0;
    while let Some(start) = find_ascii_ci(response, "```json", search_from) {
        let json_start = start + 7;
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
            if (json_str.starts_with('{') || json_str.starts_with('['))
                && let Some(surface) = parse_surface_json(json_str)
            {
                return Some(surface);
            }
        }
        search_from = json_start;
    }

    None
}

/// Try every balanced `{ … }` block in the text, largest first, and return the
/// first one that parses as a surface. Trying only the single largest block
/// loses the tree whenever the model also emitted a bigger non-surface JSON
/// object (e.g. a reasoning/config blob).
fn extract_from_raw_json(response: &str) -> Option<ExtractedSurface> {
    let mut candidates: Vec<&str> = Vec::new();

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
                candidates.push(&response[byte_start..byte_end]);
            }
        } else {
            i += 1;
        }
    }

    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.len()));
    candidates.into_iter().find_map(parse_surface_json)
}

/// Parse a JSON string into an `ExtractedSurface`.
///
/// Handles:
/// - `{"rootComponentId": "…", "canvasSettings": {…}, "components": […]}`
/// - Direct array of components `[…]`
fn parse_surface_json(json_str: &str) -> Option<ExtractedSurface> {
    if let Ok(wrapper) = serde_json::from_str::<serde_json::Value>(json_str)
        && let Some(components_val) = wrapper.get("components")
    {
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

    // Try as direct array
    if let Ok(components) = serde_json::from_str::<Vec<SurfaceComponent>>(json_str)
        && !components.is_empty()
    {
        return Some(ExtractedSurface {
            components,
            root_component_id: None,
            canvas_settings: None,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface_json() -> &'static str {
        r#"{"rootComponentId":"root","canvasSettings":{"backgroundColor":"bg-background"},"components":[{"id":"root","component":{"type":"text","content":{"literalString":"hi"}}}]}"#
    }

    #[test]
    fn fenced_json_block_extracts_components_root_and_canvas() {
        let response = format!("Here is the UI:\n```json\n{}\n```\nDone.", surface_json());
        let surface = extract_surface_from_response(&response);
        assert_eq!(surface.components.len(), 1);
        assert_eq!(surface.components[0].id, "root");
        assert_eq!(surface.root_component_id.as_deref(), Some("root"));
        assert!(surface.canvas_settings.is_some());
    }

    #[test]
    fn uppercase_fence_is_recognized() {
        let response = format!("```JSON\n{}\n```", surface_json());
        let surface = extract_surface_from_response(&response);
        assert_eq!(surface.components.len(), 1);
    }

    #[test]
    fn multibyte_uppercase_prefix_does_not_panic_or_lose_the_fence() {
        // 'İ' (U+0130) grows from 2 to 3 bytes when lowercased; byte offsets
        // computed on the lowercased text are invalid in the original string.
        let prefix = "İ".repeat(100);
        let response = format!("{}```json\n{}\n```", prefix, surface_json());
        let surface = extract_from_fenced_json(&response).expect("fenced path must find the block");
        assert_eq!(surface.components.len(), 1);
        assert_eq!(surface.root_component_id.as_deref(), Some("root"));
    }

    #[test]
    fn multibyte_prefix_with_short_tail_does_not_slice_out_of_bounds() {
        // With enough multibyte-uppercase prefix and a short JSON tail, the
        // lowercased-offset arithmetic previously pointed past the end of the
        // original string and panicked.
        let prefix = "İ".repeat(100);
        let response = format!(
            "{}```json\n{{\"components\":[{{\"id\":\"a\",\"component\":{{}}}}]}}\n```",
            prefix
        );
        let surface = extract_surface_from_response(&response);
        assert_eq!(surface.components.len(), 1);
    }

    #[test]
    fn bare_fence_block_is_extracted() {
        let response = format!("```\n{}\n```", surface_json());
        let surface = extract_surface_from_response(&response);
        assert_eq!(surface.components.len(), 1);
    }

    #[test]
    fn second_fence_is_used_when_first_is_not_a_surface() {
        let response = format!(
            "```json\n{{\"plan\": \"first draft\"}}\n```\ntext\n```json\n{}\n```",
            surface_json()
        );
        let surface = extract_surface_from_response(&response);
        assert_eq!(surface.components.len(), 1);
    }

    #[test]
    fn direct_component_array_is_extracted() {
        let response = r#"```json
[{"id":"a","component":{"type":"text","content":{"literalString":"x"}}}]
```"#;
        let surface = extract_surface_from_response(response);
        assert_eq!(surface.components.len(), 1);
        assert!(surface.root_component_id.is_none());
    }

    #[test]
    fn smaller_surface_block_wins_over_larger_non_surface_block() {
        // No fences: raw-JSON extraction must not give up just because the
        // LARGEST balanced block is not a surface.
        let big_noise = format!(
            "{{\"reasoning\": \"{}\"}}",
            "long analysis text ".repeat(30)
        );
        let response = format!("My notes {} and the tree {}", big_noise, surface_json());
        let surface = extract_surface_from_response(&response);
        assert_eq!(surface.components.len(), 1);
        assert_eq!(surface.root_component_id.as_deref(), Some("root"));
    }

    #[test]
    fn response_without_json_yields_empty_surface() {
        let surface = extract_surface_from_response("No UI here, just prose.");
        assert!(surface.components.is_empty());
        assert!(surface.root_component_id.is_none());
        assert!(surface.canvas_settings.is_none());
    }
}
