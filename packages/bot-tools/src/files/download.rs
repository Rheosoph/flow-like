//! `file_save` from an http(s) URL. A signed link carries its credentials in the query, so the
//! query and fragment never reach a result, an error or a log.

use flow_like_types::reqwest::Url;
use serde_json::json;

use super::{guess_mime, handle_for, markdown_url, workspace_of};
use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::layout::{FILES_DIR, normalize_path, sanitize_name};
use crate::outcome::ToolOutput;
use crate::web::{base_mime, extension_of, fetch_get, site_of};

const DEFAULT_NAME: &str = "download";

fn percent_decode(text: &str) -> String {
    let raw = text.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        let decoded = raw
            .get(index + 1..index + 3)
            .filter(|_| raw[index] == b'%')
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match decoded {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(raw[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn disposition_name(header: &str) -> Option<String> {
    let mut plain = None;
    for part in header.split(';').skip(1) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                let encoded = value.split_once("''").map_or(value, |(_, rest)| rest);
                return Some(percent_decode(encoded.trim_matches('"')));
            }
            "filename" => plain = Some(value.trim_matches('"').to_string()),
            _ => {}
        }
    }
    plain
}

fn usable(name: String) -> Option<String> {
    let clean = sanitize_name(&name);
    (clean != "file").then_some(clean)
}

fn file_name(url: &Url, disposition: Option<&str>, mime: &str) -> String {
    let from_url = || {
        url.path_segments()?
            .rfind(|segment| !segment.is_empty())
            .map(percent_decode)
    };
    let chosen = disposition
        .and_then(disposition_name)
        .and_then(usable)
        .or_else(|| from_url().and_then(usable))
        .unwrap_or_else(|| DEFAULT_NAME.to_string());
    if chosen.contains('.') && !chosen.ends_with('.') {
        chosen
    } else {
        format!("{}.{}", chosen.trim_end_matches('.'), extension_of(mime))
    }
}

fn parse(raw: &str) -> ToolResult<Url> {
    let url = Url::parse(raw.trim())
        .map_err(|_| ToolError::invalid("The url is not a valid http(s) address"))?;
    if site_of(&url).is_none() {
        return Err(ToolError::invalid(
            "Use an http or https url without a user name or password",
        ));
    }
    Ok(url)
}

fn bare(url: &Url) -> String {
    let mut bare = url.clone();
    bare.set_query(None);
    bare.set_fragment(None);
    bare.to_string()
}

pub(super) async fn save_from_url(
    env: &dyn ToolEnv,
    raw: &str,
    path: Option<&str>,
) -> ToolResult<ToolOutput> {
    let workspace = workspace_of(env, "save files")?;
    let url = parse(raw)?;
    let source = bare(&url);
    let max = env.limits().max_write_bytes;
    let fetched = fetch_get(env, &url, usize::try_from(max).unwrap_or(usize::MAX)).await?;
    if (300..400).contains(&fetched.status) {
        let target = fetched
            .location
            .as_deref()
            .and_then(|location| url.join(location).ok())
            .and_then(|target| target.host_str().map(str::to_string));
        let message = match target {
            Some(host) => format!(
                "{source} answered HTTP {} and redirects to {host}; redirects are not followed, ask for the final link",
                fetched.status
            ),
            None => format!(
                "{source} answered HTTP {}; redirects are not followed",
                fetched.status
            ),
        };
        return Err(ToolError::new(code::FAILED, message));
    }
    if !(200..300).contains(&fetched.status) {
        return Err(ToolError::new(
            code::FAILED,
            format!("{source} answered HTTP {}", fetched.status),
        ));
    }
    let content_mime = base_mime(&fetched.content_type);
    let path = match path {
        Some(path) => normalize_path(path)?,
        None => format!(
            "{FILES_DIR}/{}",
            file_name(&url, fetched.content_disposition.as_deref(), &content_mime)
        ),
    };
    let mime = if content_mime.is_empty() {
        guess_mime(&path)
    } else {
        content_mime
    };
    let bytes = flow_like_types::Bytes::from(fetched.bytes);
    env.write_workspace(&path, bytes.clone(), &mime).await?;
    let handle = handle_for(&workspace, &path, &bytes, &mime);
    Ok(ToolOutput::new(json!({
        "handle": handle,
        "markdown_url": markdown_url(&handle),
        "source": source,
    }))
    .saved(handle))
}
