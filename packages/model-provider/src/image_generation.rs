//! Text-to-image requests through the hosted model gateway.

pub mod openrouter;

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types_contracts::authorization::{
    AuthorizationAttribution, RequestAuthorizer, ResourceAudience,
};
use serde::{Deserialize, Serialize};

use crate::authorization::ScopedRequestAuthorizer;

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// One image, selected by an official catalog Bit rather than an upstream model ID.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedImageRequest {
    pub model: String,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
}

impl HostedImageRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.model.trim().is_empty() && self.model.len() <= 512,
            "Hosted image generation requires a catalog Bit ID"
        );
        ensure!(
            !self.prompt.trim().is_empty() && self.prompt.chars().count() <= 2048,
            "Image prompt must contain between 1 and 2048 characters"
        );
        ensure!(
            self.negative_prompt
                .as_ref()
                .is_none_or(|prompt| prompt.chars().count() <= 2048),
            "Negative image prompt must contain at most 2048 characters"
        );
        for (name, value, supported) in [
            (
                "aspect ratio",
                self.aspect_ratio.as_deref(),
                &["1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"][..],
            ),
            (
                "quality",
                self.quality.as_deref(),
                &["auto", "low", "medium", "high"][..],
            ),
            (
                "output format",
                self.output_format.as_deref(),
                &["png", "jpeg", "webp"][..],
            ),
            (
                "background",
                self.background.as_deref(),
                &["auto", "transparent", "opaque"][..],
            ),
        ] {
            ensure!(
                value.is_none_or(|value| supported.contains(&value)),
                "Unsupported hosted image {name}"
            );
        }
        if let Some(size) = &self.size {
            let dimensions = size.split_once('x').and_then(|(width, height)| {
                Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
            });
            ensure!(
                dimensions.is_some_and(|(width, height)| {
                    (256..=2048).contains(&width)
                        && (256..=2048).contains(&height)
                        && u64::from(width) * u64::from(height) <= 4_194_304
                }),
                "Image size must be WIDTHxHEIGHT with each dimension between 256 and 2048"
            );
        }
        ensure!(
            self.seed.is_none_or(|seed| seed <= u64::from(u32::MAX)),
            "Image seed must fit an unsigned 32-bit integer"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostedImageData {
    pub b64_json: String,
    pub mime_type: String,
}

impl HostedImageData {
    pub fn decode(&self) -> Result<Vec<u8>> {
        ensure!(
            !self.b64_json.is_empty() && self.b64_json.len() <= MAX_IMAGE_BYTES.div_ceil(3) * 4,
            "Hosted image exceeds the size limit or is empty"
        );
        let bytes = STANDARD
            .decode(&self.b64_json)
            .context("Hosted image contains invalid base64 data")?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_IMAGE_BYTES,
            "Hosted image exceeds the size limit or is empty"
        );
        let valid_signature = match self.mime_type.as_str() {
            "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
            "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
            _ => false,
        };
        ensure!(
            valid_signature,
            "Hosted image data does not match its PNG, JPEG, or WebP MIME type"
        );
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostedImageResponse {
    pub model: String,
    pub data: Vec<HostedImageData>,
}

#[derive(Clone)]
pub struct HostedImageClient {
    endpoint: reqwest::Url,
    bearer: String,
    usage_headers: Vec<(String, String)>,
    authorizer: Option<ScopedRequestAuthorizer>,
    client: reqwest::Client,
}

fn endpoint(base: &str, exact_resource: bool) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(base.trim()).context("Invalid hosted image API base")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Hosted image API base must be HTTP or HTTPS without credentials, query, or fragment"
    );
    let base = url.path().trim_end_matches('/');
    let path = if exact_resource || base.ends_with("/api/v1") {
        format!("{base}/images/generations")
    } else {
        format!("{base}/api/v1/images/generations")
    };
    url.set_path(&path);
    Ok(url)
}

impl HostedImageClient {
    /// `base_url` comes from runtime configuration, never from catalog provider parameters.
    pub fn new(base_url: &str, bearer: impl Into<String>) -> Result<Self> {
        Ok(Self {
            endpoint: endpoint(base_url, false)?,
            bearer: bearer.into(),
            usage_headers: Vec::new(),
            authorizer: None,
            // Replaying a generation may create another charge even without a response.
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                // Leave time for the gateway's four-minute generation deadline and worker handoff.
                .timeout(Duration::from_secs(300))
                .build()?,
        })
    }

    pub fn with_usage_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.usage_headers = headers;
        self
    }

    pub fn with_authorizer(mut self, authorizer: Arc<dyn RequestAuthorizer>) -> Result<Self> {
        if let Some(base) = authorizer.resource_base_url(ResourceAudience::HostedModels) {
            self.endpoint = endpoint(&base, true)?;
        }
        if authorizer.attribution() == AuthorizationAttribution::InstanceGrant {
            self.usage_headers.clear();
        }
        self.authorizer = Some(ScopedRequestAuthorizer::new(
            authorizer,
            ResourceAudience::HostedModels,
            self.endpoint.as_str(),
        )?);
        self.bearer.clear();
        Ok(self)
    }

    async fn build_request(&self, request: &HostedImageRequest) -> Result<reqwest::Request> {
        request.validate()?;
        let mut builder = self.client.post(self.endpoint.clone()).json(request);
        if !self.bearer.is_empty() {
            builder = builder.bearer_auth(&self.bearer);
        }
        for (name, value) in &self.usage_headers {
            builder = builder.header(name, value);
        }
        let mut http_request = builder.build()?;
        if let Some(authorizer) = &self.authorizer {
            let url = http_request.url().to_string();
            authorizer
                .authorize("POST", &url, http_request.headers_mut())
                .await?;
        }
        Ok(http_request)
    }

    pub async fn generate(&self, request: &HostedImageRequest) -> Result<HostedImageResponse> {
        let http_request = self.build_request(request).await?;
        let mut response = self
            .client
            .execute(http_request)
            .await
            .context("Hosted image request failed")?;
        let status = response.status();
        let limit = if status.is_success() {
            MAX_RESPONSE_BYTES
        } else {
            8192
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("Hosted image response was interrupted (HTTP {status})"))?
        {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= limit,
                "Hosted image response exceeds the size limit (HTTP {status})"
            );
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            if let Some(message) = gateway_error(&bytes) {
                bail!("Hosted image endpoint returned HTTP {status}: {message}");
            }
            bail!("Hosted image endpoint returned HTTP {status}");
        }
        let response: HostedImageResponse = match serde_json::from_slice(&bytes) {
            Ok(response) => response,
            Err(error) => {
                if let Some(message) = gateway_error(&bytes) {
                    bail!("Hosted image generation failed: {message}");
                }
                return Err(error).context("Invalid hosted image response");
            }
        };
        ensure!(
            response.model == request.model,
            "Hosted image response names a different Bit"
        );
        ensure!(
            response.data.len() == 1,
            "Hosted image response must contain exactly one image"
        );
        response.data[0].decode()?;
        Ok(response)
    }
}

fn gateway_error(body: &[u8]) -> Option<String> {
    let body: serde_json::Value = serde_json::from_slice(body).ok()?;
    let message = body.pointer("/error/message")?.as_str()?;
    // The gateway publishes client-safe messages. Do not expose other response fields.
    Some(message.chars().take(1024).collect())
}

#[cfg(test)]
mod tests;
