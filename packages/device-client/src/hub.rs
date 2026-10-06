use crate::{Error, Result, endpoint::checked_url, tls};
use async_trait::async_trait;
use flow_like_device_protocol::{ControllerSignalingRequest, DeviceSignalingResponse};
use std::{sync::Arc, time::Duration};

const ADMISSION_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_ERROR_CHARS: usize = 512;

/// Rendezvous only: an admission lets the relay route frames, never read them. The device
/// verifies the controller certificate itself.
#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait marks the boxed future it already returns as must_use"
)]
pub trait HubClient: Send + Sync {
    async fn controller_admission(
        &self,
        device_id: &str,
        participant_id: &str,
    ) -> Result<DeviceSignalingResponse>;
}

/// The API token of the signed-in profile. It is read for every admission, so refreshed
/// tokens apply without rebuilding the client.
#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait marks the boxed future it already returns as must_use"
)]
pub trait AccessToken: Send + Sync {
    async fn access_token(&self) -> Result<String>;
}

/// `POST {api_base}/devices/{device_id}/signaling/controller`.
pub struct HttpHubClient {
    http: reqwest::Client,
    api_base: reqwest::Url,
    token: Arc<dyn AccessToken>,
}

impl HttpHubClient {
    /// `api_base` is the API root, for example `https://api.flow-like.com/api/v1`.
    pub fn new(api_base: &str, token: Arc<dyn AccessToken>) -> Result<Self> {
        let api_base = checked_url(api_base, "https", "http").ok_or_else(|| {
            Error::Invalid(format!(
                "Hub API base {api_base} must be an HTTPS URL without credentials, query or fragment"
            ))
        })?;
        let http = reqwest::Client::builder()
            .use_preconfigured_tls((*tls::client_config()?).clone())
            .timeout(ADMISSION_TIMEOUT)
            .build()
            .map_err(|error| Error::Invalid(format!("Could not build the hub client: {error}")))?;
        Ok(Self {
            http,
            api_base,
            token,
        })
    }

    fn endpoint(&self, device_id: &str) -> Result<reqwest::Url> {
        let mut url = self.api_base.clone();
        url.path_segments_mut()
            .map_err(|_| {
                Error::Invalid(format!(
                    "Hub API base {} cannot carry a path",
                    self.api_base
                ))
            })?
            .pop_if_empty()
            .extend(["devices", device_id, "signaling", "controller"]);
        Ok(url)
    }
}

#[async_trait]
impl HubClient for HttpHubClient {
    async fn controller_admission(
        &self,
        device_id: &str,
        participant_id: &str,
    ) -> Result<DeviceSignalingResponse> {
        let url = self.endpoint(device_id)?;
        let token = self.token.access_token().await?;
        let response = self
            .http
            .post(url.clone())
            .bearer_auth(token)
            .json(&ControllerSignalingRequest {
                participant_id: participant_id.into(),
            })
            .send()
            .await
            .map_err(|error| Error::Unreachable {
                device_id: device_id.into(),
                message: format!("POST {} failed: {error}", url.path()),
            })?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let detail: String = body.trim().chars().take(MAX_ERROR_CHARS).collect();
            return Err(Error::Refused {
                device_id: device_id.into(),
                status: Some(status.as_u16()),
                message: format!("POST {} answered HTTP {status}: {detail}", url.path()),
            });
        }
        response
            .json::<DeviceSignalingResponse>()
            .await
            .map_err(|error| Error::Refused {
                device_id: device_id.into(),
                status: Some(status.as_u16()),
                message: format!("the admission from {} is malformed: {error}", url.path()),
            })
    }
}
