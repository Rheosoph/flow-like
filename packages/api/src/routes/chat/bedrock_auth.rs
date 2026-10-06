use crate::{error::ApiError, state::State};
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct BedrockTarget {
    pub(super) region: String,
    pub(super) service: &'static str,
}

fn valid_region(region: &str) -> bool {
    let parts: Vec<_> = region.split('-').collect();
    region.len() <= 63
        && parts.len() >= 3
        && parts[0].len() == 2
        && parts[..parts.len() - 1]
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_lowercase()))
        && parts
            .last()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

pub(super) fn default_endpoint(region: &str) -> Result<String, ApiError> {
    if !valid_region(region) {
        return Err(ApiError::service_unavailable(
            "Bedrock IAM authentication requires a valid AWS region",
        ));
    }
    let suffix = if region.starts_with("cn-") {
        "amazonaws.com.cn"
    } else {
        "amazonaws.com"
    };
    Ok(format!(
        "https://bedrock-runtime.{region}.{suffix}/openai/v1"
    ))
}

/// Role credentials may only sign the supported AWS inference endpoints.
pub(super) fn validate_target(raw: &str) -> Result<BedrockTarget, ApiError> {
    let invalid = || {
        ApiError::service_unavailable("Bedrock IAM authentication requires a trusted AWS endpoint")
    };
    let url = reqwest::Url::parse(raw).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        // Reject normalizations such as explicit ports, dot segments and URL userinfo.
        || url.as_str() != raw
    {
        return Err(invalid());
    }
    let host = url.host_str().ok_or_else(invalid)?;
    let (region, service) = if let Some(host) = host.strip_prefix("bedrock-runtime.") {
        let region = host
            .strip_suffix(".amazonaws.com.cn")
            .filter(|region| region.starts_with("cn-"))
            .or_else(|| {
                host.strip_suffix(".amazonaws.com")
                    .filter(|region| !region.starts_with("cn-"))
            })
            .ok_or_else(invalid)?;
        if !matches!(
            url.path(),
            "/openai/v1/chat/completions" | "/openai/v1/responses"
        ) {
            return Err(invalid());
        }
        (region, "bedrock")
    } else if let Some(host) = host.strip_prefix("bedrock-mantle.") {
        let region = host
            .strip_suffix(".api.aws")
            .filter(|region| !region.starts_with("cn-"))
            .ok_or_else(invalid)?;
        if !matches!(url.path(), "/v1/chat/completions" | "/v1/responses") {
            return Err(invalid());
        }
        (region, "bedrock-mantle")
    } else {
        return Err(invalid());
    };
    if !valid_region(region) {
        return Err(invalid());
    }
    Ok(BedrockTarget {
        region: region.to_owned(),
        service,
    })
}

pub(super) async fn request_builder(
    state: &State,
    url: &str,
    body: &serde_json::Value,
    timeout: Duration,
) -> Result<reqwest::RequestBuilder, ApiError> {
    validate_target(url)?;
    #[cfg(feature = "aws")]
    {
        let started = std::time::Instant::now();
        let provider = state.aws_client.credentials_provider();
        let credentials = resolve_credentials(provider.as_ref(), timeout).await?;
        let payload = serde_json::to_vec(body)
            .map_err(|_| ApiError::internal("Could not serialize the Bedrock request"))?;
        let remaining = timeout.checked_sub(started.elapsed()).ok_or_else(|| {
            ApiError::service_unavailable("Bedrock request expired while resolving AWS credentials")
        })?;
        signed_request(
            url,
            payload,
            &credentials,
            std::time::SystemTime::now(),
            remaining,
        )
    }
    #[cfg(not(feature = "aws"))]
    {
        let _ = (state, body, timeout);
        Err(ApiError::service_unavailable(
            "Bedrock IAM authentication is unavailable in this server build",
        ))
    }
}

#[cfg(feature = "aws")]
async fn resolve_credentials(
    provider: Option<&aws_credential_types::provider::SharedCredentialsProvider>,
    timeout: Duration,
) -> Result<aws_credential_types::Credentials, ApiError> {
    use aws_credential_types::provider::ProvideCredentials;

    let provider = provider.ok_or_else(|| {
        ApiError::service_unavailable("No AWS credentials provider is available for Bedrock")
    })?;
    flow_like_types::tokio::time::timeout(
        timeout.min(Duration::from_secs(5)),
        provider.provide_credentials(),
    )
    .await
    .map_err(|_| ApiError::service_unavailable("AWS credentials for Bedrock timed out"))?
    .map_err(|_| ApiError::service_unavailable("AWS credentials for Bedrock are unavailable"))
}

#[cfg(feature = "aws")]
fn signed_request(
    url: &str,
    payload: Vec<u8>,
    credentials: &aws_credential_types::Credentials,
    time: std::time::SystemTime,
    timeout: Duration,
) -> Result<reqwest::RequestBuilder, ApiError> {
    use aws_sigv4::{
        http_request::{PayloadChecksumKind, SignableBody, SignableRequest, SigningSettings, sign},
        sign::v4,
    };

    let target = validate_target(url)?;
    let identity = credentials.clone().into();
    let mut settings = SigningSettings::default();
    settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(&target.region)
        .name(target.service)
        .time(time)
        .settings(settings)
        .build()
        .map_err(|_| ApiError::service_unavailable("Could not configure Bedrock authentication"))?
        .into();
    let request = SignableRequest::new(
        "POST",
        url,
        [("content-type", "application/json")].into_iter(),
        SignableBody::Bytes(&payload),
    )
    .map_err(|_| ApiError::service_unavailable("Could not prepare Bedrock authentication"))?;
    let (instructions, _) = sign(request, &params)
        .map_err(|_| ApiError::service_unavailable("Could not sign the Bedrock request"))?
        .into_parts();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ApiError::service_unavailable("Could not create the Bedrock client"))?;
    // Reuse the signed bytes so later JSON serialization cannot invalidate the signature.
    let mut request = client
        .post(url)
        .timeout(timeout)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload);
    for (name, value) in instructions.headers() {
        let mut value = reqwest::header::HeaderValue::from_str(value).map_err(|_| {
            ApiError::service_unavailable("Could not encode Bedrock authentication")
        })?;
        if matches!(name, "authorization" | "x-amz-security-token") {
            value.set_sensitive(true);
        }
        request = request.header(name, value);
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_the_inference_surface_for_each_signing_service() {
        for (url, region, service) in [
            (
                "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions",
                "us-east-1",
                "bedrock",
            ),
            (
                "https://bedrock-runtime.eu-central-1.amazonaws.com/openai/v1/responses",
                "eu-central-1",
                "bedrock",
            ),
            (
                "https://bedrock-runtime.cn-north-1.amazonaws.com.cn/openai/v1/chat/completions",
                "cn-north-1",
                "bedrock",
            ),
            (
                "https://bedrock-mantle.us-east-1.api.aws/v1/chat/completions",
                "us-east-1",
                "bedrock-mantle",
            ),
            (
                "https://bedrock-mantle.eu-west-1.api.aws/v1/responses",
                "eu-west-1",
                "bedrock-mantle",
            ),
        ] {
            assert_eq!(
                validate_target(url).unwrap(),
                BedrockTarget {
                    region: region.to_owned(),
                    service,
                }
            );
        }
    }

    #[test]
    fn rejects_hostile_and_unrelated_signing_targets() {
        for url in [
            "http://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions",
            "https://bedrock-runtime.us-east-1.amazonaws.com.evil.test/openai/v1/chat/completions",
            "https://bedrock-runtime.us-east-1.amazonaws.com@evil.test/openai/v1/chat/completions",
            "https://evil.test@bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions",
            "https://@bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions",
            "https://bedrock-runtime.us-east-1.amazonaws.com:443/openai/v1/chat/completions",
            "https://bedrock-runtime.us-east-1.amazonaws.com:8443/openai/v1/chat/completions",
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions?model=other",
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions#fragment",
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions/",
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/other/../chat/completions",
            "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/%63hat/completions",
            "https://bedrock-runtime.us-east-1.amazonaws.com/v1/chat/completions",
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/other/invoke",
            "https://bedrock-runtime.us-east-1.amazonaws.com.cn/openai/v1/chat/completions",
            "https://bedrock-runtime.cn-north-1.amazonaws.com/openai/v1/chat/completions",
            "https://bedrock-runtime.us-east-1.extra.amazonaws.com/openai/v1/chat/completions",
            "https://bedrock-runtime.us--east-1.amazonaws.com/openai/v1/chat/completions",
            "https://bedrock-mantle.us-east-1.api.aws.evil.test/v1/chat/completions",
            "https://bedrock-mantle.us-east-1.api.aws/openai/v1/chat/completions",
            "https://bedrock-mantle.cn-north-1.api.aws/v1/chat/completions",
            "https://sts.us-east-1.amazonaws.com/",
            "https://127.0.0.1/openai/v1/chat/completions",
            "https://bedrock-runtime.us-east-1.amazonaws.com\\@evil.test/openai/v1/chat/completions",
        ] {
            assert!(validate_target(url).is_err(), "unexpectedly accepted {url}");
        }
    }

    #[test]
    fn default_endpoint_requires_a_region_without_url_syntax() {
        assert_eq!(
            default_endpoint("eu-central-1").unwrap(),
            "https://bedrock-runtime.eu-central-1.amazonaws.com/openai/v1"
        );
        assert_eq!(
            default_endpoint("cn-north-1").unwrap(),
            "https://bedrock-runtime.cn-north-1.amazonaws.com.cn/openai/v1"
        );
        for region in ["", "us-east-1.evil.test", "us-east-1/", "us-east-1@evil"] {
            assert!(default_endpoint(region).is_err());
        }
    }

    #[cfg(feature = "aws")]
    #[test]
    fn signs_the_transmitted_payload_with_temporary_role_credentials() {
        use aws_credential_types::Credentials;
        use sha2::{Digest, Sha256};

        let credentials = Credentials::new(
            "AKIDEXAMPLE",
            "example-secret-access-key",
            Some("example-session-token".into()),
            None,
            "test",
        );
        let time = std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let timeout = Duration::from_secs(20);
        let payload = serde_json::to_vec(&serde_json::json!({
            "model": "openai.gpt-oss-120b-1:0",
            "messages": [{"role": "user", "content": "Hello ☀"}],
            "stream": true
        }))
        .unwrap();
        for (url, scope) in [
            (
                "https://bedrock-runtime.us-east-1.amazonaws.com/openai/v1/chat/completions",
                "/us-east-1/bedrock/aws4_request",
            ),
            (
                "https://bedrock-mantle.eu-west-1.api.aws/v1/responses",
                "/eu-west-1/bedrock-mantle/aws4_request",
            ),
        ] {
            let request = signed_request(url, payload.clone(), &credentials, time, timeout)
                .unwrap()
                .build()
                .unwrap();
            let headers = request.headers();
            let authorization = headers["authorization"].to_str().unwrap();
            assert!(authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20231114"));
            assert!(authorization.contains(scope));
            assert!(authorization.contains(
                "content-type;host;x-amz-content-sha256;x-amz-date;x-amz-security-token"
            ));
            assert_eq!(headers["x-amz-date"], "20231114T221320Z");
            assert_eq!(headers["x-amz-security-token"], "example-session-token");
            assert!(headers["authorization"].is_sensitive());
            assert!(headers["x-amz-security-token"].is_sensitive());
            assert_eq!(headers["content-type"], "application/json");
            assert_eq!(request.body().unwrap().as_bytes().unwrap(), payload);
            assert_eq!(
                headers["x-amz-content-sha256"].to_str().unwrap(),
                hex::encode(Sha256::digest(&payload))
            );
            assert_eq!(request.timeout(), Some(&timeout));

            let changed = signed_request(url, b"{}".to_vec(), &credentials, time, timeout)
                .unwrap()
                .build()
                .unwrap();
            assert_ne!(headers["authorization"], changed.headers()["authorization"]);
        }
    }

    #[cfg(feature = "aws")]
    #[tokio::test(start_paused = true)]
    async fn unavailable_or_slow_credentials_return_service_unavailable() {
        use aws_credential_types::provider::{
            ProvideCredentials, SharedCredentialsProvider, error::CredentialsError, future,
        };

        #[derive(Debug)]
        struct Unavailable;
        impl ProvideCredentials for Unavailable {
            fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
            where
                Self: 'a,
            {
                future::ProvideCredentials::ready(Err(CredentialsError::not_loaded("unavailable")))
            }
        }

        #[derive(Debug)]
        struct Slow;
        impl ProvideCredentials for Slow {
            fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
            where
                Self: 'a,
            {
                future::ProvideCredentials::new(std::future::pending())
            }
        }

        let unavailable = SharedCredentialsProvider::new(Unavailable);
        let slow = SharedCredentialsProvider::new(Slow);
        for provider in [None, Some(&unavailable), Some(&slow)] {
            let started = tokio::time::Instant::now();
            let error = resolve_credentials(provider, Duration::from_secs(1))
                .await
                .unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
            assert!(started.elapsed() <= Duration::from_secs(1));
        }
    }
}
