use std::time::Duration;

use async_trait::async_trait;
use flow_like_types::reqwest::{self, Client, Method, header::HeaderValue};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::request::{RequestMethod, StripeRequest, valid_id};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RetryDisposition {
    Never,
    SameOperation,
    Reconcile,
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum StripeError {
    #[error("invalid Stripe request: {0}")]
    InvalidRequest(String),
    #[error("Stripe transport failed")]
    Transport { retry: RetryDisposition },
    #[error("Stripe rejected the request (HTTP {status}, code {code:?})")]
    Api {
        status: u16,
        code: Option<String>,
        request_id: Option<String>,
        retry: RetryDisposition,
        retry_after_seconds: Option<u64>,
    },
    #[error("Stripe response was incomplete or invalid")]
    InvalidResponse {
        request_id: Option<String>,
        retry: RetryDisposition,
    },
}

impl StripeError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    pub fn retry_disposition(&self) -> RetryDisposition {
        match self {
            Self::InvalidRequest(_) => RetryDisposition::Never,
            Self::Transport { retry }
            | Self::Api { retry, .. }
            | Self::InvalidResponse { retry, .. } => *retry,
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Api { request_id, .. } | Self::InvalidResponse { request_id, .. } => {
                request_id.as_deref()
            }
            _ => None,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct StripeResponse {
    pub body: Value,
    pub request_id: Option<String>,
}

impl std::fmt::Debug for StripeResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StripeResponse")
            .field("request_id", &self.request_id)
            .finish_non_exhaustive()
    }
}

impl StripeResponse {
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, StripeError> {
        serde_json::from_value(self.body.clone()).map_err(|_| StripeError::InvalidResponse {
            request_id: self.request_id.clone(),
            retry: RetryDisposition::Reconcile,
        })
    }
}

/// The caller persists mutations and schedules retries; transport sends exactly once.
#[async_trait]
pub trait StripeGateway: Send + Sync {
    async fn execute(&self, request: &StripeRequest) -> Result<StripeResponse, StripeError>;
}

#[derive(Clone)]
pub struct HttpStripeGateway {
    client: Client,
    authorization: HeaderValue,
    platform_account_id: String,
    livemode: bool,
    api_version: String,
    base_url: String,
}

impl std::fmt::Debug for HttpStripeGateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpStripeGateway")
            .field("platform_account_id", &self.platform_account_id)
            .field("livemode", &self.livemode)
            .field("api_version", &self.api_version)
            .finish_non_exhaustive()
    }
}

impl HttpStripeGateway {
    pub fn new(
        secret_key: &str,
        platform_account_id: impl Into<String>,
        livemode: bool,
        api_version: impl Into<String>,
    ) -> Result<Self, StripeError> {
        let platform_account_id = platform_account_id.into();
        let api_version = api_version.into();
        let probe = StripeRequest {
            method: RequestMethod::Get,
            path: "/v1/account".into(),
            scope: super::StripeScope::platform(&platform_account_id, livemode),
            api_version: api_version.clone(),
            parameters: serde_json::json!({}),
            idempotency_key: None,
        };
        probe.validate()?;
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| StripeError::invalid("cannot construct Stripe HTTP client"))?;
        Ok(Self {
            client,
            authorization: authorization(secret_key, livemode)?,
            platform_account_id,
            livemode,
            api_version,
            base_url: "https://api.stripe.com".into(),
        })
    }

    /// Reads the authenticated platform identity without inventing a connected scope.
    pub async fn discover(
        secret_key: &str,
        livemode: bool,
        api_version: impl Into<String>,
    ) -> Result<Self, StripeError> {
        let mut gateway = Self::new(secret_key, "acct_discovery", livemode, api_version)?;
        let mut request = StripeRequest::get(
            super::StripeScope::platform("acct_discovery", livemode),
            "/v1/account",
            serde_json::json!({}),
        );
        request.api_version = gateway.api_version.clone();
        let response = gateway.execute(&request).await?;
        let id = response
            .body
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| valid_id(id, "acct_"))
            .ok_or_else(|| StripeError::InvalidResponse {
                request_id: response.request_id.clone(),
                retry: RetryDisposition::Never,
            })?;
        gateway.platform_account_id = id.into();
        Ok(gateway)
    }

    pub fn platform_account_id(&self) -> &str {
        &self.platform_account_id
    }

    pub fn livemode(&self) -> bool {
        self.livemode
    }

    pub fn api_version(&self) -> &str {
        &self.api_version
    }
}

fn authorization(secret_key: &str, livemode: bool) -> Result<HeaderValue, StripeError> {
    let prefixes = if livemode {
        ["sk_live_", "rk_live_"]
    } else {
        ["sk_test_", "rk_test_"]
    };
    if !prefixes.iter().any(|prefix| secret_key.starts_with(prefix))
        || secret_key.len() <= 8
        || secret_key.len() > 512
        || !secret_key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(StripeError::invalid(
            "Stripe key does not match the configured mode",
        ));
    }
    let mut header = HeaderValue::from_str(&format!("Bearer {secret_key}"))
        .map_err(|_| StripeError::invalid("invalid Stripe authentication"))?;
    header.set_sensitive(true);
    Ok(header)
}

pub fn classify_response(
    method: RequestMethod,
    status: u16,
    code: Option<&str>,
    should_retry: Option<bool>,
) -> RetryDisposition {
    // A failed mutation can still have side effects even if Stripe says not to retry it.
    if status >= 500 && method == RequestMethod::Post {
        return RetryDisposition::Reconcile;
    }
    if should_retry == Some(false) {
        return RetryDisposition::Never;
    }
    if should_retry == Some(true)
        || status == 429
        || (status == 409 && code == Some("idempotency_key_in_use"))
        || (status >= 500 && method == RequestMethod::Get)
    {
        RetryDisposition::SameOperation
    } else {
        RetryDisposition::Never
    }
}

#[async_trait]
impl StripeGateway for HttpStripeGateway {
    async fn execute(&self, request: &StripeRequest) -> Result<StripeResponse, StripeError> {
        request.validate()?;
        if request.scope.platform_account_id != self.platform_account_id
            || request.scope.livemode != self.livemode
            || request.api_version != self.api_version
        {
            return Err(StripeError::invalid(
                "Stripe operation does not match gateway scope/version",
            ));
        }
        let method = match request.method {
            RequestMethod::Get => Method::GET,
            RequestMethod::Post => Method::POST,
        };
        let mut builder = self
            .client
            .request(method, format!("{}{}", self.base_url, request.path))
            .header(reqwest::header::AUTHORIZATION, self.authorization.clone())
            .header("Stripe-Version", &request.api_version);
        if let Some(account) = &request.scope.connected_account_id {
            builder = builder.header("Stripe-Account", account);
        }
        if let Some(key) = &request.idempotency_key {
            builder = builder.header("Idempotency-Key", key);
        }
        builder = match request.method {
            RequestMethod::Get => builder.query(&request.form_pairs()?),
            RequestMethod::Post => builder
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded",
                )
                .body(request.encoded_form()?),
        };
        let mut response = builder.send().await.map_err(|_| StripeError::Transport {
            retry: RetryDisposition::SameOperation,
        })?;
        let status = response.status().as_u16();
        let request_id = response
            .headers()
            .get("request-id")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let should_retry = response
            .headers()
            .get("stripe-should-retry")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| match h {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            });
        let retry_after_seconds = response
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.parse().ok());
        let indeterminate = || StripeError::InvalidResponse {
            request_id: request_id.clone(),
            retry: if request.method == RequestMethod::Post {
                RetryDisposition::Reconcile
            } else {
                RetryDisposition::SameOperation
            },
        };
        if response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
        {
            return Err(indeterminate());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| indeterminate())? {
            if body.len() + chunk.len() > MAX_RESPONSE_BYTES {
                return Err(indeterminate());
            }
            body.extend_from_slice(&chunk);
        }
        let body: Value = serde_json::from_slice(&body).map_err(|_| indeterminate())?;
        if !(200..300).contains(&status) {
            let code = body
                .pointer("/error/code")
                .and_then(Value::as_str)
                .filter(|code| {
                    code.len() <= 128
                        && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                })
                .map(str::to_owned);
            return Err(StripeError::Api {
                status,
                retry: classify_response(request.method, status, code.as_deref(), should_retry),
                code,
                request_id,
                retry_after_seconds,
            });
        }
        if body
            .get("livemode")
            .and_then(Value::as_bool)
            .is_some_and(|mode| mode != request.scope.livemode)
        {
            return Err(indeterminate());
        }
        Ok(StripeResponse { body, request_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn retry_policy_preserves_unknown_mutation_outcomes() {
        assert_eq!(
            classify_response(RequestMethod::Post, 500, None, Some(false)),
            RetryDisposition::Reconcile
        );
        assert_eq!(
            classify_response(RequestMethod::Post, 429, None, Some(false)),
            RetryDisposition::Never
        );
        assert_eq!(
            classify_response(RequestMethod::Post, 429, None, None),
            RetryDisposition::SameOperation
        );
        assert_eq!(
            classify_response(
                RequestMethod::Post,
                409,
                Some("idempotency_key_in_use"),
                None
            ),
            RetryDisposition::SameOperation
        );
        assert_eq!(
            classify_response(RequestMethod::Post, 409, Some("other"), None),
            RetryDisposition::Never
        );
        assert_eq!(
            classify_response(RequestMethod::Post, 400, None, None),
            RetryDisposition::Never
        );
        assert_eq!(
            classify_response(RequestMethod::Get, 503, None, None),
            RetryDisposition::SameOperation
        );
    }

    #[test]
    fn mode_and_debug_do_not_expose_credentials() {
        assert!(
            HttpStripeGateway::new("sk_test_secret", "acct_platform", true, "2023-10-16").is_err()
        );
        let gateway =
            HttpStripeGateway::new("rk_test_secret", "acct_platform", false, "2023-10-16").unwrap();
        assert!(!format!("{gateway:?}").contains("secret"));
        assert!(gateway.authorization.is_sensitive());
    }

    #[tokio::test]
    async fn transport_sends_scoped_persisted_parameters_and_does_not_retry_500() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut input = Vec::new();
            loop {
                let mut buffer = [0; 1024];
                let read = socket.read(&mut buffer).unwrap();
                assert!(read > 0);
                input.extend_from_slice(&buffer[..read]);
                if let Some(end) = input.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&input[..end]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    if input.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body =
                r#"{"error":{"code":"api_error","message":"do not expose provider details"}}"#;
            write!(socket, "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nRequest-Id: req_contract\r\nStripe-Should-Retry: true\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8(input).unwrap()
        });
        let mut gateway =
            HttpStripeGateway::new("sk_test_contract", "acct_platform", false, "2023-10-16")
                .unwrap();
        gateway.base_url = format!("http://{address}");
        let mut request = StripeRequest::post(
            super::super::StripeScope::platform("acct_platform", false).connected("acct_seller"),
            "/v1/refunds",
            serde_json::json!({"charge":"ch_one","metadata":{"reason":"a&b"}}),
            "refund:one",
        );
        request.api_version = "2023-10-16".into();
        let error = gateway.execute(&request).await.unwrap_err();
        assert_eq!(error.retry_disposition(), RetryDisposition::Reconcile);
        assert_eq!(error.request_id(), Some("req_contract"));
        assert!(!error.to_string().contains("provider details"));
        let input = server.join().unwrap();
        let lower = input.to_ascii_lowercase();
        assert!(lower.starts_with("post /v1/refunds http/1.1"));
        assert!(lower.contains("stripe-account: acct_seller"));
        assert!(lower.contains("stripe-version: 2023-10-16"));
        assert!(lower.contains("idempotency-key: refund:one"));
        assert!(input.ends_with("charge=ch_one&metadata%5Breason%5D=a%26b"));
    }
}
