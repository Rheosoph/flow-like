//! A plain, guarded GET shared by `web_request` and `file_save`: public hosts only, the resolved
//! addresses pinned, no redirects, a time limit and a byte cap.

use std::time::Duration;

use flow_like_types::reqwest::{self, Url, header};

use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::services::egress::{Locality, Reachable, resolve_public};

pub(crate) const TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Fetched {
    pub status: u16,
    pub content_type: String,
    pub content_disposition: Option<String>,
    pub location: Option<String>,
    pub bytes: Vec<u8>,
}

pub(crate) enum ReadError {
    TooLarge,
    Failed(String),
}

pub(crate) fn pinned_client(reachable: &Reachable) -> ToolResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT);
    if !reachable.addrs.is_empty() {
        builder = builder.resolve_to_addrs(&reachable.host, &reachable.addrs);
    }
    builder.build().map_err(|error| {
        ToolError::new(
            code::FAILED,
            format!("The HTTP client could not be created: {error}"),
        )
    })
}

pub(crate) async fn read_capped(
    response: &mut reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, ReadError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        ReadError::Failed(format!(
            "Reading the response failed: {}",
            error.without_url()
        ))
    })? {
        if bytes.len() + chunk.len() > max_bytes {
            return Err(ReadError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn header_text(response: &reqwest::Response, name: header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// GET without credentials. Errors never carry the URL's query or fragment.
pub(crate) async fn fetch_get(
    env: &dyn ToolEnv,
    url: &Url,
    max_bytes: usize,
) -> ToolResult<Fetched> {
    let site = url.host_str().unwrap_or("the site").to_string();
    let reachable = resolve_public(env.environment(), url.as_str())
        .await
        .map_err(ToolError::refused)?;
    if reachable.locality == Locality::PrivateName {
        return Err(ToolError::refused(format!(
            "The name {site} resolves to a private network address, which downloads never reach"
        )));
    }
    let client = pinned_client(&reachable)?;
    let mut response = client
        .get(reachable.url)
        .header(header::USER_AGENT, "Flow-Like-Bot")
        .send()
        .await
        .map_err(|error| {
            let what = if error.is_timeout() {
                "timed out"
            } else {
                "failed"
            };
            ToolError::new(
                code::FAILED,
                format!("The download from {site} {what}: {}", error.without_url()),
            )
        })?;
    let status = response.status().as_u16();
    let content_type = header_text(&response, header::CONTENT_TYPE).unwrap_or_default();
    let content_disposition = header_text(&response, header::CONTENT_DISPOSITION);
    let location = header_text(&response, header::LOCATION);
    let bytes = if (200..300).contains(&status) {
        read_capped(&mut response, max_bytes)
            .await
            .map_err(|error| match error {
                ReadError::TooLarge => ToolError::new(
                    code::TOO_LARGE,
                    format!("The file from {site} is larger than the {max_bytes} bytes a workspace file may hold"),
                ),
                ReadError::Failed(message) => ToolError::new(code::FAILED, message),
            })?
    } else {
        Vec::new()
    };
    Ok(Fetched {
        status,
        content_type,
        content_disposition,
        location,
        bytes,
    })
}
