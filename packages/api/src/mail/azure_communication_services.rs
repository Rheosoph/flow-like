//! Azure Communication Services Email client using Microsoft Entra authentication only.
//!
//! Account keys, connection strings, and client secrets are deliberately unsupported. The
//! production Azure image requires a user-assigned managed identity selected by `AZURE_CLIENT_ID`.

use std::{sync::Arc, time::Duration};

use azure_core::credentials::TokenCredential;
use azure_identity::{ManagedIdentityCredential, ManagedIdentityCredentialOptions, UserAssignedId};
use flow_like::hub::MailConfig;
use flow_like_types::Result;
use reqwest::{Method, StatusCode, Url, header::HeaderMap};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{EmailMessage, MailClient};

const ACS_SCOPE: &str = "https://communication.azure.com//.default";
const API_VERSION: &str = "2025-09-01";
const MAX_CONTENT_BYTES: usize = 9 * 1024 * 1024;
const MAX_SEND_ATTEMPTS: u32 = 4;
const MAX_POLL_DURATION: Duration = Duration::from_secs(5 * 60);
const DEFAULT_POLL_DELAY: Duration = Duration::from_secs(2);
const MAX_POLL_DELAY: Duration = Duration::from_secs(60);

const FORBIDDEN_CREDENTIAL_ENV_VARS: &[&str] = &[
    "ACS_EMAIL_ACCESS_KEY",
    "ACS_EMAIL_CONNECTION_STRING",
    "AZURE_CLIENT_SECRET",
    "AZURE_COMMUNICATION_CONNECTION_STRING",
    "AZURE_COMMUNICATION_KEY",
    "COMMUNICATION_SERVICES_CONNECTION_STRING",
];

pub struct AzureCommunicationServicesMailClient {
    http: reqwest::Client,
    credential: Arc<dyn TokenCredential>,
    endpoint: Url,
    sender: String,
    from_name: String,
}

impl AzureCommunicationServicesMailClient {
    pub fn new(config: &MailConfig) -> Result<Self> {
        reject_secret_credentials()?;

        let endpoint = required_env("ACS_EMAIL_ENDPOINT")?;
        let endpoint = validate_endpoint(&endpoint)?;
        let sender = required_env("ACS_EMAIL_SENDER")?;
        validate_email_address("ACS_EMAIL_SENDER", &sender)?;

        let client_id = required_env("AZURE_CLIENT_ID")?;
        Uuid::parse_str(&client_id).map_err(|_| {
            flow_like_types::anyhow!(
                "AZURE_CLIENT_ID must be the UUID client ID of a user-assigned managed identity"
            )
        })?;

        let credential = ManagedIdentityCredential::new(Some(ManagedIdentityCredentialOptions {
            user_assigned_id: Some(UserAssignedId::ClientId(client_id)),
            ..Default::default()
        }))
        .map_err(|error| {
            flow_like_types::anyhow!("failed to configure the ACS Email managed identity: {error}")
        })?;

        let http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("flow-like-acs-email/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| {
                flow_like_types::anyhow!("failed to construct the ACS Email HTTP client: {error}")
            })?;

        Ok(Self {
            http,
            credential,
            endpoint,
            sender,
            from_name: config.from_name.clone(),
        })
    }

    async fn access_token(&self) -> Result<String> {
        self.credential
            .get_token(&[ACS_SCOPE], None)
            .await
            .map(|token| token.token.secret().to_owned())
            .map_err(|error| {
                flow_like_types::anyhow!(
                    "failed to acquire an ACS Email token from managed identity: {error}"
                )
            })
    }

    fn send_url(&self) -> Result<Url> {
        self.endpoint
            .join(&format!("emails:send?api-version={API_VERSION}"))
            .map_err(|error| flow_like_types::anyhow!("failed to construct ACS send URL: {error}"))
    }

    async fn submit(&self, payload: &AcsEmailRequest<'_>, operation_id: Uuid) -> Result<Url> {
        let send_url = self.send_url()?;

        for attempt in 0..MAX_SEND_ATTEMPTS {
            let token = self.access_token().await?;
            let response = self
                .http
                .post(send_url.clone())
                .bearer_auth(token)
                .header("Operation-Id", operation_id.to_string())
                .header("x-ms-client-request-id", operation_id.to_string())
                .header("x-ms-return-client-request-id", "true")
                .json(payload)
                .send()
                .await;

            match response {
                Ok(response) if response.status() == StatusCode::ACCEPTED => {
                    return operation_location(&self.endpoint, response.headers());
                }
                Ok(response)
                    if is_retryable_status(response.status())
                        && attempt + 1 < MAX_SEND_ATTEMPTS =>
                {
                    let delay = retry_delay(response.headers(), attempt);
                    tracing::warn!(
                        status = %response.status(),
                        attempt = attempt + 1,
                        "ACS Email submission was throttled or unavailable; retrying"
                    );
                    tokio::time::sleep(delay).await;
                }
                Ok(response) => {
                    return Err(service_error("submit", response));
                }
                Err(_error) if attempt + 1 < MAX_SEND_ATTEMPTS => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        "ACS Email submission transport failed; retrying with the same operation ID"
                    );
                    tokio::time::sleep(exponential_delay(attempt)).await;
                }
                Err(error) => {
                    return Err(flow_like_types::anyhow!(
                        "ACS Email submission transport failed after {MAX_SEND_ATTEMPTS} attempts: {error}"
                    ));
                }
            }
        }

        unreachable!("ACS Email submission loop always returns")
    }

    async fn await_completion(&self, operation_url: Url) -> Result<()> {
        let started = tokio::time::Instant::now();

        loop {
            if started.elapsed() >= MAX_POLL_DURATION {
                return Err(flow_like_types::anyhow!(
                    "ACS Email operation did not reach a terminal state within {} seconds",
                    MAX_POLL_DURATION.as_secs()
                ));
            }

            let token = self.access_token().await?;
            let response = self
                .http
                .request(Method::GET, operation_url.clone())
                .bearer_auth(token)
                .header("x-ms-client-request-id", Uuid::new_v4().to_string())
                .header("x-ms-return-client-request-id", "true")
                .send()
                .await
                .map_err(|error| {
                    flow_like_types::anyhow!("ACS Email status request failed: {error}")
                })?;

            if response.status() != StatusCode::OK {
                return Err(service_error("poll", response));
            }

            let delay = retry_delay(response.headers(), 0);
            let result: AcsOperationResult = response.json().await.map_err(|error| {
                flow_like_types::anyhow!("ACS Email returned an invalid operation result: {error}")
            })?;

            match result.status {
                AcsOperationStatus::Succeeded => return Ok(()),
                AcsOperationStatus::Failed | AcsOperationStatus::Canceled => {
                    let code = result
                        .error
                        .as_ref()
                        .and_then(|error| error.code.as_deref())
                        .map(sanitize_service_text)
                        .filter(|code| !code.is_empty())
                        .unwrap_or_else(|| "unknown".to_string());
                    return Err(flow_like_types::anyhow!(
                        "ACS Email operation ended with status {:?} (code: {code})",
                        result.status
                    ));
                }
                AcsOperationStatus::NotStarted | AcsOperationStatus::Running => {
                    tokio::time::sleep(delay).await;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl MailClient for AzureCommunicationServicesMailClient {
    async fn send(&self, message: EmailMessage) -> Result<()> {
        validate_message(&message)?;
        let payload = AcsEmailRequest::new(&self.sender, &message);
        let operation_url = self.submit(&payload, Uuid::new_v4()).await?;
        self.await_completion(operation_url).await
    }

    fn from_email(&self) -> &str {
        &self.sender
    }

    fn from_name(&self) -> &str {
        &self.from_name
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AcsEmailRequest<'a> {
    sender_address: &'a str,
    content: AcsEmailContent<'a>,
    recipients: AcsRecipients<'a>,
    user_engagement_tracking_disabled: bool,
}

impl<'a> AcsEmailRequest<'a> {
    fn new(sender: &'a str, message: &'a EmailMessage) -> Self {
        Self {
            sender_address: sender,
            content: AcsEmailContent {
                subject: &message.subject,
                plain_text: message.body_text.as_deref(),
                html: message.body_html.as_deref(),
            },
            recipients: AcsRecipients {
                to: vec![AcsEmailAddress {
                    address: &message.to,
                }],
            },
            user_engagement_tracking_disabled: true,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AcsEmailContent<'a> {
    subject: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    plain_text: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    html: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct AcsRecipients<'a> {
    to: Vec<AcsEmailAddress<'a>>,
}

#[derive(Debug, Serialize)]
struct AcsEmailAddress<'a> {
    address: &'a str,
}

#[derive(Debug, Deserialize)]
struct AcsOperationResult {
    status: AcsOperationStatus,
    #[serde(default)]
    error: Option<AcsOperationError>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
enum AcsOperationStatus {
    NotStarted,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Debug, Deserialize)]
struct AcsOperationError {
    code: Option<String>,
}

fn required_env(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| flow_like_types::anyhow!("{name} is required for ACS Email"))
}

fn reject_secret_credentials() -> Result<()> {
    for name in FORBIDDEN_CREDENTIAL_ENV_VARS {
        if std::env::var(name)
            .ok()
            .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(flow_like_types::anyhow!(
                "{name} is forbidden for ACS Email; use a user-assigned managed identity"
            ));
        }
    }
    Ok(())
}

fn validate_endpoint(raw: &str) -> Result<Url> {
    let endpoint = Url::parse(raw)
        .map_err(|_| flow_like_types::anyhow!("ACS_EMAIL_ENDPOINT must be a valid URL"))?;
    let host = endpoint
        .host_str()
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| flow_like_types::anyhow!("ACS_EMAIL_ENDPOINT must include a host"))?;
    let valid_host =
        host.ends_with(".communication.azure.com") || host.ends_with(".communications.azure.com");
    let valid_path = endpoint.path().is_empty() || endpoint.path() == "/";

    if endpoint.scheme() != "https"
        || !valid_host
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.port().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !valid_path
    {
        return Err(flow_like_types::anyhow!(
            "ACS_EMAIL_ENDPOINT must be a public-cloud ACS HTTPS origin without credentials, port, path, query, or fragment"
        ));
    }

    Ok(endpoint)
}

fn validate_email_address(label: &str, address: &str) -> Result<()> {
    let mut parts = address.split('@');
    let local = parts.next().unwrap_or_default();
    let domain = parts.next().unwrap_or_default();
    let exactly_one_at = parts.next().is_none();
    let safe_ascii = address.is_ascii()
        && !address
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace());

    if !exactly_one_at
        || local.is_empty()
        || domain.is_empty()
        || address.len() > 254
        || !domain.contains('.')
        || domain.starts_with('.')
        || domain.ends_with('.')
        || !safe_ascii
    {
        return Err(flow_like_types::anyhow!(
            "{label} must be a single valid ASCII email address"
        ));
    }

    Ok(())
}

fn validate_message(message: &EmailMessage) -> Result<()> {
    validate_email_address("email recipient", message.to.trim())?;

    if message.to != message.to.trim() {
        return Err(flow_like_types::anyhow!(
            "email recipient must not contain surrounding whitespace"
        ));
    }
    if message.subject.trim().is_empty() || message.subject.len() > 998 {
        return Err(flow_like_types::anyhow!(
            "email subject must contain 1-998 bytes"
        ));
    }
    if message
        .subject
        .bytes()
        .any(|byte| matches!(byte, b'\r' | b'\n' | 0))
    {
        return Err(flow_like_types::anyhow!(
            "email subject must not contain CR, LF, or NUL"
        ));
    }

    let content_bytes = message.body_text.as_ref().map_or(0, String::len)
        + message.body_html.as_ref().map_or(0, String::len);
    if content_bytes == 0 {
        return Err(flow_like_types::anyhow!(
            "email must have either an HTML or text body"
        ));
    }
    if content_bytes > MAX_CONTENT_BYTES {
        return Err(flow_like_types::anyhow!(
            "email content exceeds the {MAX_CONTENT_BYTES}-byte ACS safety limit"
        ));
    }

    Ok(())
}

fn operation_location(endpoint: &Url, headers: &HeaderMap) -> Result<Url> {
    let raw_location = headers
        .get("Operation-Location")
        .ok_or_else(|| flow_like_types::anyhow!("ACS Email response omitted Operation-Location"))?
        .to_str()
        .map_err(|_| {
            flow_like_types::anyhow!("ACS Email returned an invalid Operation-Location header")
        })?;
    let location = endpoint.join(raw_location).map_err(|_| {
        flow_like_types::anyhow!("ACS Email returned an invalid Operation-Location URL")
    })?;

    if location.scheme() != "https"
        || location.origin() != endpoint.origin()
        || !location.username().is_empty()
        || location.password().is_some()
        || !location.path().starts_with("/emails/operations/")
    {
        return Err(flow_like_types::anyhow!(
            "ACS Email returned an untrusted Operation-Location URL"
        ));
    }

    Ok(location)
}

fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn retry_delay(headers: &HeaderMap, attempt: u32) -> Duration {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| {
            if attempt == 0 {
                DEFAULT_POLL_DELAY
            } else {
                exponential_delay(attempt)
            }
        })
        .clamp(Duration::from_secs(1), MAX_POLL_DELAY)
}

fn exponential_delay(attempt: u32) -> Duration {
    Duration::from_secs(2_u64.saturating_pow(attempt.min(5)))
}

fn service_error(operation: &str, response: reqwest::Response) -> flow_like_types::Error {
    let status = response.status();
    let error_code = response
        .headers()
        .get("x-ms-error-code")
        .and_then(|value| value.to_str().ok())
        .map(sanitize_service_text)
        .filter(|value| !value.is_empty());
    let request_id = response
        .headers()
        .get("x-ms-request-id")
        .and_then(|value| value.to_str().ok())
        .map(sanitize_service_text)
        .filter(|value| !value.is_empty());

    flow_like_types::anyhow!(
        "ACS Email {operation} failed with HTTP {status}{}{}",
        error_code
            .as_deref()
            .map(|code| format!(" (code: {code})"))
            .unwrap_or_default(),
        request_id
            .as_deref()
            .map(|id| format!(" (request: {id})"))
            .unwrap_or_default(),
    )
}

fn sanitize_service_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> EmailMessage {
        EmailMessage {
            to: "recipient@example.com".to_string(),
            subject: "Security alert".to_string(),
            body_html: Some("<p>Review the alert.</p>".to_string()),
            body_text: Some("Review the alert.".to_string()),
        }
    }

    #[test]
    fn serializes_the_acs_contract_and_disables_tracking() {
        let message = message();
        let payload = AcsEmailRequest::new("DoNotReply@example.azurecomm.net", &message);
        let value = serde_json::to_value(payload).expect("payload must serialize");

        assert_eq!(value["senderAddress"], "DoNotReply@example.azurecomm.net");
        assert_eq!(value["content"]["plainText"], "Review the alert.");
        assert_eq!(value["content"]["html"], "<p>Review the alert.</p>");
        assert_eq!(
            value["recipients"]["to"][0]["address"],
            "recipient@example.com"
        );
        assert_eq!(value["userEngagementTrackingDisabled"], true);
    }

    #[test]
    fn rejects_non_acs_or_ambiguous_endpoints() {
        assert!(validate_endpoint("https://flowlike.communication.azure.com").is_ok());
        assert!(validate_endpoint("https://flowlike.westeurope.communications.azure.com/").is_ok());
        assert!(validate_endpoint("https://communication.azure.com.attacker.example").is_err());
        assert!(validate_endpoint("http://flowlike.communication.azure.com").is_err());
        assert!(validate_endpoint("https://flowlike.communication.azure.com/path").is_err());
        assert!(validate_endpoint("https://user@flowlike.communication.azure.com").is_err());
    }

    #[test]
    fn accepts_only_same_origin_operation_locations() {
        let endpoint = validate_endpoint("https://flowlike.communication.azure.com").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            "Operation-Location",
            "/emails/operations/123?api-version=2025-09-01"
                .parse()
                .unwrap(),
        );
        assert!(operation_location(&endpoint, &headers).is_ok());

        headers.insert(
            "Operation-Location",
            "https://attacker.example/emails/operations/123"
                .parse()
                .unwrap(),
        );
        assert!(operation_location(&endpoint, &headers).is_err());
    }

    #[test]
    fn validates_header_and_content_boundaries() {
        assert!(validate_message(&message()).is_ok());

        let mut invalid = message();
        invalid.subject = "subject\r\nBcc: victim@example.com".to_string();
        assert!(validate_message(&invalid).is_err());

        let mut invalid = message();
        invalid.to = "recipient@example.com\r\n".to_string();
        assert!(validate_message(&invalid).is_err());

        let mut invalid = message();
        invalid.body_html = None;
        invalid.body_text = None;
        assert!(validate_message(&invalid).is_err());
    }

    #[test]
    fn understands_every_documented_operation_status() {
        for (status, expected) in [
            ("NotStarted", AcsOperationStatus::NotStarted),
            ("Running", AcsOperationStatus::Running),
            ("Succeeded", AcsOperationStatus::Succeeded),
            ("Failed", AcsOperationStatus::Failed),
            ("Canceled", AcsOperationStatus::Canceled),
        ] {
            let result: AcsOperationResult = serde_json::from_value(serde_json::json!({
                "status": status
            }))
            .expect("documented status must deserialize");
            assert_eq!(result.status, expected);
        }
    }
}
