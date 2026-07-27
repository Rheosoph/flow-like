//! Deterministic issue fingerprinting for anonymous error reports.
//!
//! Fingerprints intentionally ignore line and column numbers so that edits
//! above a throw site do not split an existing issue, and normalize ids,
//! hashes and quoted values out of stack-less titles so that one logical bug
//! stays one issue.

use sha2::{Digest, Sha256};

/// Number of leading frames that identify an issue.
const FINGERPRINT_FRAMES: usize = 5;
/// Minimum length of a `[0-9a-f-]` run that is treated as an id or hash.
const MIN_HEX_RUN: usize = 8;

/// The parts of a stack frame that identify an issue.
#[derive(Debug, Clone, Copy)]
pub struct FingerprintFrame<'a> {
    pub function: Option<&'a str>,
    pub file: Option<&'a str>,
    pub in_app: bool,
}

/// Groups an error into a stable issue: `sha256(kind + "\n" + frames_or_title)`,
/// where frames are the top in-app frames (falling back to the top frames)
/// reduced to `basename(file) + ":" + function`.
pub fn fingerprint(kind: &str, title: &str, stacktrace: &[FingerprintFrame<'_>]) -> String {
    let in_app: Vec<&FingerprintFrame<'_>> = stacktrace
        .iter()
        .filter(|frame| frame.in_app)
        .take(FINGERPRINT_FRAMES)
        .collect();
    let selected = if in_app.is_empty() {
        stacktrace.iter().take(FINGERPRINT_FRAMES).collect()
    } else {
        in_app
    };

    let mut input = String::from(kind);
    input.push('\n');
    if selected.is_empty() {
        input.push_str(&normalize_title(title));
    } else {
        for (index, frame) in selected.iter().enumerate() {
            if index > 0 {
                input.push('\n');
            }
            input.push_str(basename(frame.file.unwrap_or_default()));
            input.push(':');
            input.push_str(frame.function.unwrap_or_default());
        }
    }

    hex::encode(Sha256::digest(input.as_bytes()))
}

/// Collapses the volatile parts of an error message: lowercases, replaces
/// quoted values with `?`, id/hash runs and digit runs with `0`, and collapses
/// whitespace.
pub fn normalize_title(title: &str) -> String {
    let lowered = title.to_lowercase();
    let unquoted = mask_quoted(&lowered);
    let unhashed = mask_hex_runs(&unquoted);
    let undigited = mask_digit_runs(&unhashed);
    undigited
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn basename(file: &str) -> &str {
    let path = file.split(['?', '#']).next().unwrap_or(file);
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn mask_quoted(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(ch) = chars.next() {
        if matches!(ch, '"' | '\'' | '`') {
            out.push('?');
            for next in chars.by_ref() {
                if next == ch {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn mask_hex_runs(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut run = String::new();
    for ch in input.chars() {
        if ch.is_ascii_hexdigit() || ch == '-' {
            run.push(ch);
            continue;
        }
        flush_hex_run(&mut out, &mut run);
        out.push(ch);
    }
    flush_hex_run(&mut out, &mut run);
    out
}

fn flush_hex_run(out: &mut String, run: &mut String) {
    if run.len() >= MIN_HEX_RUN && run.chars().any(|ch| ch.is_ascii_hexdigit()) {
        out.push('0');
    } else {
        out.push_str(run);
    }
    run.clear();
}

fn mask_digit_runs(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_run = false;
    for ch in input.chars() {
        if ch.is_ascii_digit() {
            if !in_run {
                out.push('0');
                in_run = true;
            }
            continue;
        }
        in_run = false;
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        function: &'static str,
        file: &'static str,
        in_app: bool,
    ) -> FingerprintFrame<'static> {
        FingerprintFrame {
            function: Some(function),
            file: Some(file),
            in_app,
        }
    }

    #[test]
    fn is_a_lowercase_sha256_hex_digest() {
        let fp = fingerprint("TypeError", "boom", &[]);
        assert_eq!(fp.len(), 64);
        assert!(
            fp.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
        );
    }

    #[test]
    fn same_frames_group_regardless_of_path_and_line_drift() {
        let build_a = [
            frame("renderBoard", "/build/2026-07-01/src/board.ts", true),
            frame("run", "/build/2026-07-01/src/run.ts", true),
        ];
        let build_b = [
            frame(
                "renderBoard",
                "https://cdn.example.com/assets/board.ts?v=9",
                true,
            ),
            frame("run", "C:\\ci\\other\\run.ts", true),
        ];
        assert_eq!(
            fingerprint("TypeError", "boom", &build_a),
            fingerprint("TypeError", "boom", &build_b)
        );
    }

    #[test]
    fn different_kinds_do_not_group() {
        let frames = [frame("run", "run.ts", true)];
        assert_ne!(
            fingerprint("TypeError", "boom", &frames),
            fingerprint("RangeError", "boom", &frames)
        );
    }

    #[test]
    fn titles_are_ignored_when_a_stacktrace_is_present() {
        let frames = [frame("run", "run.ts", true)];
        assert_eq!(
            fingerprint("TypeError", "failed for user 12", &frames),
            fingerprint("TypeError", "failed for user 934", &frames)
        );
    }

    #[test]
    fn prefers_in_app_frames_over_vendor_frames() {
        let with_vendor = [
            frame("dispatch", "node_modules/react-dom/index.js", false),
            frame("renderBoard", "board.ts", true),
        ];
        let without_vendor = [frame("renderBoard", "board.ts", true)];
        assert_eq!(
            fingerprint("TypeError", "boom", &with_vendor),
            fingerprint("TypeError", "boom", &without_vendor)
        );
    }

    #[test]
    fn falls_back_to_the_top_frames_when_nothing_is_in_app() {
        let frames = [
            frame("dispatch", "vendor.js", false),
            frame("invoke", "vendor.js", false),
        ];
        let other = [frame("dispatch", "vendor.js", false)];
        assert_ne!(
            fingerprint("TypeError", "boom", &frames),
            fingerprint("TypeError", "boom", &other)
        );
    }

    #[test]
    fn only_the_top_five_frames_matter() {
        let mut base: Vec<FingerprintFrame<'static>> =
            (0..5).map(|_| frame("run", "run.ts", true)).collect();
        let short = fingerprint("TypeError", "boom", &base);
        base.push(frame("deep", "deep.ts", true));
        assert_eq!(short, fingerprint("TypeError", "boom", &base));
    }

    #[test]
    fn stackless_errors_group_by_normalized_title() {
        assert_eq!(
            fingerprint("HttpError", "Request 4711 timed out after 1200ms", &[]),
            fingerprint("HttpError", "Request 88 timed out after 30ms", &[])
        );
        assert_ne!(
            fingerprint("HttpError", "Request timed out", &[]),
            fingerprint("HttpError", "Connection refused", &[])
        );
    }

    #[test]
    fn frames_without_file_or_function_are_stable() {
        let anonymous = [FingerprintFrame {
            function: None,
            file: None,
            in_app: true,
        }];
        assert_eq!(
            fingerprint("TypeError", "boom", &anonymous),
            fingerprint("TypeError", "other", &anonymous)
        );
    }

    #[test]
    fn normalize_title_lowercases_and_collapses_whitespace() {
        assert_eq!(
            normalize_title("  Boom   Happened\n Here "),
            "boom happened here"
        );
    }

    #[test]
    fn normalize_title_masks_digit_runs() {
        assert_eq!(normalize_title("timeout after 1200ms"), "timeout after 0ms");
        assert_eq!(
            normalize_title("row 7 of 12345 failed"),
            "row 0 of 0 failed"
        );
    }

    #[test]
    fn normalize_title_masks_uuids_and_hex_runs() {
        assert_eq!(
            normalize_title("app 550E8400-E29B-41D4-A716-446655440000 missing"),
            "app 0 missing"
        );
        assert_eq!(
            normalize_title("chunk deadbeefcafe not found"),
            "chunk 0 not found"
        );
        assert_eq!(normalize_title("board face missing"), "board face missing");
    }

    #[test]
    fn normalize_title_masks_quoted_values() {
        assert_eq!(
            normalize_title("Cannot read properties of undefined (reading 'boardId')"),
            "cannot read properties of undefined (reading ?)"
        );
        assert_eq!(
            normalize_title("module \"@flow-like/ui\" not found"),
            "module ? not found"
        );
    }
}
