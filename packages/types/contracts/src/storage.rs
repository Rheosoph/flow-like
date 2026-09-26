//! Storage paths shared by credential issuers and runtime writers.

const MAX_STORAGE_PATH_SEGMENT_CHARS: usize = 80;
const STORAGE_PATH_SEGMENT_DIGEST_CHARS: usize = 12;

/// Keep safe identifiers readable; append a digest when rewriting could collide.
pub fn storage_path_segment(value: &str, fallback: &str) -> String {
    let mut sanitized = String::with_capacity(value.len().min(MAX_STORAGE_PATH_SEGMENT_CHARS));
    let mut lossy = value.chars().count() > MAX_STORAGE_PATH_SEGMENT_CHARS;
    for ch in value.chars().take(MAX_STORAGE_PATH_SEGMENT_CHARS) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
            lossy = true;
        }
    }

    let trimmed = sanitized.trim_matches(|ch| ch == '.' || ch == '_');
    lossy |= trimmed.len() != sanitized.len();

    if !lossy {
        return if trimmed.is_empty() {
            fallback.to_string()
        } else {
            trimmed.to_string()
        };
    }

    let digest = blake3::hash(value.as_bytes()).to_hex();
    let digest = &digest[..STORAGE_PATH_SEGMENT_DIGEST_CHARS];
    let base = if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    };
    let base: String = base
        .chars()
        .take(MAX_STORAGE_PATH_SEGMENT_CHARS - STORAGE_PATH_SEGMENT_DIGEST_CHARS - 1)
        .collect();
    let base = base.trim_end_matches(['.', '_']);
    let base = if base.is_empty() { fallback } else { base };
    format!("{base}-{digest}")
}

/// The scratch prefixes every scoped credential must authorise, built from the
/// same segments the writers use. Returned as a pair so no caller can sanitise
/// one and forget the other.
pub fn temporary_prefixes(sub: &str, app_id: &str) -> (String, String) {
    let app_segment = storage_path_segment(app_id, "app");
    (
        format!(
            "tmp/user/{}/apps/{}",
            storage_path_segment(sub, "user"),
            app_segment
        ),
        format!("tmp/global/apps/{}", app_segment),
    )
}
