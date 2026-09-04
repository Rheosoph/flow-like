//! Source-map symbolication for stored crash stack frames.
//!
//! Symbolication is best effort: a missing, mismatched or malformed source map
//! degrades silently to the raw minified frame instead of failing the request.

use serde::{Deserialize, Serialize};
use sourcemap::{DecodedMap, decode_slice};
use utoipa::ToSchema;

/// A single stack frame as stored in `TelemetryErrorEvent.stacktrace`.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize, ToSchema)]
pub struct StackFrame {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineno: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colno: Option<i64>,
    #[serde(default, alias = "inApp", skip_serializing_if = "Option::is_none")]
    pub in_app: Option<bool>,
}

/// A stored source map, keyed by the minified file it belongs to. The map is
/// held as bytes: most come straight off the object store, and validating tens
/// of megabytes as UTF-8 only to hand `decode_slice` a slice again buys nothing.
#[derive(Clone, Debug)]
pub struct SourceMapEntry {
    pub file_name: String,
    pub map: Vec<u8>,
}

/// Last path segment of a URL or path, without query string or fragment.
pub fn basename(path: &str) -> &str {
    let path = path
        .split(['?', '#'])
        .next()
        .unwrap_or(path)
        .trim_end_matches('/');
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn is_in_app(source: &str) -> bool {
    !source.contains("node_modules") && !source.contains("webpack/runtime")
}

/// Rewrites minified frames into original source positions using the supplied
/// maps, matched by file basename. Returns the frames and whether any frame was
/// resolved.
pub fn symbolicate_frames(
    frames: Vec<StackFrame>,
    maps: &[SourceMapEntry],
) -> (Vec<StackFrame>, bool) {
    if frames.is_empty() || maps.is_empty() {
        return (frames, false);
    }

    let decoded: Vec<(&str, DecodedMap)> = maps
        .iter()
        .filter_map(|entry| match decode_slice(&entry.map) {
            Ok(map) => Some((basename(&entry.file_name), map)),
            Err(err) => {
                tracing::debug!(
                    file = %entry.file_name,
                    error = %err,
                    "skipping unreadable telemetry source map"
                );
                None
            }
        })
        .collect();

    let mut symbolicated = false;
    let frames = frames
        .into_iter()
        .map(|frame| match resolve_frame(&frame, &decoded) {
            Some(resolved) => {
                symbolicated = true;
                resolved
            }
            None => frame,
        })
        .collect();

    (frames, symbolicated)
}

fn resolve_frame(frame: &StackFrame, maps: &[(&str, DecodedMap)]) -> Option<StackFrame> {
    let file = frame.file.as_deref()?;
    let lineno = u32::try_from(frame.lineno?).ok()?;
    let map = maps
        .iter()
        .find(|(name, _)| *name == basename(file))
        .map(|(_, map)| map)?;

    // Stack traces are 1-based, source maps are 0-based.
    let line = lineno.saturating_sub(1);
    let column = frame
        .colno
        .and_then(|colno| u32::try_from(colno).ok())
        .unwrap_or(1)
        .saturating_sub(1);
    let token = map.lookup_token(line, column)?;
    let source = token.get_source()?;

    Some(StackFrame {
        function: token
            .get_name()
            .map(|name| name.to_string())
            .or_else(|| frame.function.clone()),
        file: Some(source.to_string()),
        lineno: Some(i64::from(token.get_src_line()) + 1),
        colno: Some(i64::from(token.get_src_col()) + 1),
        in_app: Some(is_in_app(source)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sourcemap::SourceMapBuilder;

    fn build_map(source: &str, name: Option<&str>) -> String {
        let mut builder = SourceMapBuilder::new(Some("main-abc123.js"));
        builder.add(0, 10, 3, 4, Some(source), name, false);
        let mut buffer: Vec<u8> = Vec::new();
        builder.into_sourcemap().to_writer(&mut buffer).unwrap();
        String::from_utf8(buffer).unwrap()
    }

    fn entry(file_name: &str, map: String) -> SourceMapEntry {
        SourceMapEntry {
            file_name: file_name.to_string(),
            map: map.into_bytes(),
        }
    }

    fn minified_frame(file: &str) -> StackFrame {
        StackFrame {
            function: Some("t".to_string()),
            file: Some(file.to_string()),
            lineno: Some(1),
            colno: Some(11),
            in_app: Some(false),
        }
    }

    #[test]
    fn basename_strips_directories_and_query_strings() {
        assert_eq!(basename("https://app.dev/_next/main.js?v=2"), "main.js");
        assert_eq!(basename("/static/chunks/main.js#frag"), "main.js");
        assert_eq!(basename("C:\\build\\main.js"), "main.js");
        assert_eq!(basename("main.js"), "main.js");
    }

    #[test]
    fn resolves_a_matching_frame_to_its_original_position() {
        let maps = vec![entry(
            "main-abc123.js",
            build_map("src/app/page.tsx", Some("handleClick")),
        )];
        let frames = vec![minified_frame(
            "https://app.dev/_next/static/chunks/main-abc123.js",
        )];

        let (frames, symbolicated) = symbolicate_frames(frames, &maps);

        assert!(symbolicated);
        assert_eq!(frames[0].file.as_deref(), Some("src/app/page.tsx"));
        assert_eq!(frames[0].function.as_deref(), Some("handleClick"));
        assert_eq!(frames[0].lineno, Some(4));
        assert_eq!(frames[0].colno, Some(5));
        assert_eq!(frames[0].in_app, Some(true));
    }

    #[test]
    fn keeps_the_minified_function_when_the_map_has_no_name() {
        let maps = vec![entry("main-abc123.js", build_map("src/app/page.tsx", None))];
        let (frames, symbolicated) =
            symbolicate_frames(vec![minified_frame("main-abc123.js")], &maps);

        assert!(symbolicated);
        assert_eq!(frames[0].function.as_deref(), Some("t"));
    }

    #[test]
    fn marks_vendor_sources_as_not_in_app() {
        let maps = vec![entry(
            "main-abc123.js",
            build_map("node_modules/react-dom/index.js", Some("render")),
        )];
        let (frames, symbolicated) =
            symbolicate_frames(vec![minified_frame("main-abc123.js")], &maps);

        assert!(symbolicated);
        assert_eq!(frames[0].in_app, Some(false));
    }

    #[test]
    fn leaves_frames_without_a_matching_map_untouched() {
        let maps = vec![entry(
            "main-abc123.js",
            build_map("src/app/page.tsx", Some("handleClick")),
        )];
        let original = minified_frame("https://app.dev/_next/static/chunks/other-xyz.js");
        let (frames, symbolicated) = symbolicate_frames(vec![original.clone()], &maps);

        assert!(!symbolicated);
        assert_eq!(frames[0], original);
    }

    #[test]
    fn malformed_maps_degrade_to_the_raw_frame() {
        let maps = vec![
            entry("main-abc123.js", "{ not json".to_string()),
            entry("other.js", String::new()),
        ];
        let original = minified_frame("main-abc123.js");
        let (frames, symbolicated) = symbolicate_frames(vec![original.clone()], &maps);

        assert!(!symbolicated);
        assert_eq!(frames[0], original);
    }

    #[test]
    fn frames_without_a_position_are_skipped() {
        let maps = vec![entry(
            "main-abc123.js",
            build_map("src/app/page.tsx", Some("handleClick")),
        )];
        let original = StackFrame {
            function: Some("t".to_string()),
            file: Some("main-abc123.js".to_string()),
            lineno: None,
            colno: None,
            in_app: None,
        };
        let (frames, symbolicated) = symbolicate_frames(vec![original.clone()], &maps);

        assert!(!symbolicated);
        assert_eq!(frames[0], original);
    }

    #[test]
    fn parses_the_frame_shape_written_by_the_ingest_route() {
        let stored = serde_json::json!([
            {
                "function": "t",
                "file": "https://app.dev/_next/static/chunks/main-abc123.js",
                "lineno": 1,
                "colno": 11,
                "in_app": true
            },
            { "file": "main-abc123.js", "in_app": false },
            { "file": "main-abc123.js", "lineno": 9_000_000_000i64, "in_app": true }
        ]);
        let frames: Vec<StackFrame> = serde_json::from_value(stored).unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].lineno, Some(1));
        assert_eq!(frames[1].in_app, Some(false));

        let maps = vec![entry(
            "main-abc123.js",
            build_map("src/app/page.tsx", Some("handleClick")),
        )];
        let (frames, symbolicated) = symbolicate_frames(frames, &maps);
        assert!(symbolicated);
        assert_eq!(frames[0].file.as_deref(), Some("src/app/page.tsx"));
        assert_eq!(frames[2].lineno, Some(9_000_000_000));
    }

    #[test]
    fn stored_frames_accept_both_in_app_spellings() {
        let snake: StackFrame =
            serde_json::from_str(r#"{"file":"a.js","lineno":1,"in_app":true}"#).unwrap();
        let camel: StackFrame =
            serde_json::from_str(r#"{"file":"a.js","lineno":1,"inApp":true}"#).unwrap();
        assert_eq!(snake.in_app, Some(true));
        assert_eq!(camel.in_app, Some(true));
        assert!(serde_json::to_string(&snake).unwrap().contains("in_app"));
    }
}
