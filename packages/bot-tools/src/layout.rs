use crate::error::ToolError;

pub const INBOX_DIR: &str = "bot/inbox";
pub const CODE_DIR: &str = "bot/code";
pub const IMAGES_DIR: &str = "bot/images";
pub const RENDERS_DIR: &str = "bot/renders";
pub const NOTES_DIR: &str = "bot/notes";
pub const EXTRACTIONS_DIR: &str = "bot/extractions";
pub const EXPORTS_DIR: &str = "bot/exports";
pub const FILES_DIR: &str = "bot/files";
pub const WEB_DIR: &str = "bot/web";

pub fn inbox_path(seq: u64, part: &str, name: &str) -> String {
    format!("{INBOX_DIR}/{seq}/{part}-{}", sanitize_name(name))
}

pub fn sanitize_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).trim();
    let clean: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim_matches('.').to_string();
    if clean.is_empty() {
        "file".to_string()
    } else {
        clean
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// A relative, `/`-separated storage path without empty, `.` or `..` segments.
pub fn normalize_path(path: &str) -> Result<String, ToolError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(ToolError::invalid("The path is empty"));
    }
    if trimmed.contains('\\') || trimmed.contains('\0') {
        return Err(ToolError::invalid(format!(
            "The path '{trimmed}' must use '/' separators and no control characters"
        )));
    }
    let segments: Vec<&str> = trimmed.trim_start_matches('/').split('/').collect();
    if segments
        .iter()
        .any(|segment| segment.is_empty() || *segment == "." || *segment == "..")
    {
        return Err(ToolError::invalid(format!(
            "The path '{trimmed}' must be relative, without empty, '.' or '..' segments"
        )));
    }
    Ok(segments.join("/"))
}
