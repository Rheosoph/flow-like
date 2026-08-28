//! SigV4 query-string presigning of the MQTT-over-WebSocket upgrade request, the way the AWS
//! CRT does it: sign `GET /mqtt` for `iotdevicegateway` with the session token excluded, then
//! append `X-Amz-Security-Token` after the signature.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    SessionTokenMode, SignableBody, SignableRequest, SignatureLocation, SigningParams,
    SigningSettings, sign,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use flow_like_types::channel::AwsTemporaryCredentials;
use flow_like_types::{Result, anyhow};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

pub const IOT_SIGNING_SERVICE: &str = "iotdevicegateway";
pub const PRESIGN_EXPIRES_IN: Duration = Duration::from_secs(300);
const SECURITY_TOKEN_PARAM: &str = "X-Amz-Security-Token";
const CREDENTIALS_PROVIDER_NAME: &str = "flow-like-channel-grant";

/// SigV4 canonical query encoding: everything except unreserved characters.
const QUERY_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

pub fn mqtt_wss_url(endpoint: &str) -> String {
    format!("wss://{endpoint}/mqtt")
}

/// Presigned `wss://{endpoint}/mqtt?...` URL valid for [`PRESIGN_EXPIRES_IN`] from `now`.
pub fn presign_wss_url(
    endpoint: &str,
    region: &str,
    credentials: &AwsTemporaryCredentials,
    now: SystemTime,
) -> Result<String> {
    let base = mqtt_wss_url(endpoint);
    let identity: Identity = Credentials::new(
        credentials.access_key_id.clone(),
        credentials.secret_access_key.clone(),
        Some(credentials.session_token.clone()),
        None,
        CREDENTIALS_PROVIDER_NAME,
    )
    .into();

    let mut settings = SigningSettings::default();
    settings.signature_location = SignatureLocation::QueryParams;
    settings.expires_in = Some(PRESIGN_EXPIRES_IN);
    settings.session_token_mode = SessionTokenMode::Exclude;

    let params: SigningParams = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name(IOT_SIGNING_SERVICE)
        .time(now)
        .settings(settings)
        .build()
        .map_err(|error| anyhow!("AWS IoT presign parameters are invalid: {error}"))?
        .into();

    let request = SignableRequest::new(
        "GET",
        base.as_str(),
        std::iter::empty(),
        SignableBody::Bytes(&[]),
    )
    .map_err(|error| anyhow!("AWS IoT presign request for '{base}' is invalid: {error}"))?;

    let (instructions, _signature) = sign(request, &params)
        .map_err(|error| anyhow!("AWS IoT presign for '{base}' failed: {error}"))?
        .into_parts();
    let (_headers, query_params) = instructions.into_parts();

    let mut query: Vec<String> = query_params
        .iter()
        .filter(|(name, _)| *name != SECURITY_TOKEN_PARAM)
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect();
    if !credentials.session_token.is_empty() {
        query.push(format!(
            "{SECURITY_TOKEN_PARAM}={}",
            encode(&credentials.session_token)
        ));
    }
    Ok(format!("{base}?{}", query.join("&")))
}

fn encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_ENCODE_SET).to_string()
}

/// Re-signs the WebSocket upgrade request on every (re)connect so the timestamp is fresh.
pub(crate) struct Presigner {
    endpoint: String,
    region: String,
    credentials: AwsTemporaryCredentials,
}

impl Presigner {
    pub fn new(endpoint: &str, region: &str, credentials: &AwsTemporaryCredentials) -> Arc<Self> {
        Arc::new(Self {
            endpoint: endpoint.to_string(),
            region: region.to_string(),
            credentials: credentials.clone(),
        })
    }

    pub fn presign(&self, now: SystemTime) -> Result<String> {
        presign_wss_url(&self.endpoint, &self.region, &self.credentials, now)
    }

    pub fn apply(&self, mut request: http::Request<()>) -> http::Request<()> {
        let presigned = self
            .presign(SystemTime::now())
            .and_then(|url| url.parse::<http::Uri>().map_err(Into::into));
        match presigned {
            Ok(uri) => *request.uri_mut() = uri,
            Err(error) => tracing::error!(
                %error,
                endpoint = %self.endpoint,
                "AWS IoT websocket presign failed; sending the upgrade unsigned"
            ),
        }
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    const ENDPOINT: &str = "a1b2c3-ats.iot.eu-central-1.amazonaws.com";

    fn credentials(session_token: &str) -> AwsTemporaryCredentials {
        AwsTemporaryCredentials {
            access_key_id: "ASIAEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: session_token.into(),
            expiration: 1_787_000_000,
        }
    }

    fn fixed_time() -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_787_000_000 - 3600)
    }

    fn query_pairs(url: &str) -> Vec<(String, String)> {
        url.split_once('?')
            .unwrap()
            .1
            .split('&')
            .map(|pair| {
                let (k, v) = pair.split_once('=').unwrap();
                (k.to_string(), v.to_string())
            })
            .collect()
    }

    #[test]
    fn presigned_url_has_sigv4_query_and_token_last() {
        let url = presign_wss_url(
            ENDPOINT,
            "eu-central-1",
            &credentials("tok/en+1"),
            fixed_time(),
        )
        .unwrap();
        assert!(url.starts_with(&format!("wss://{ENDPOINT}/mqtt?")));
        let pairs = query_pairs(&url);
        let names: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "X-Amz-Algorithm",
                "X-Amz-Credential",
                "X-Amz-Date",
                "X-Amz-Expires",
                "X-Amz-SignedHeaders",
                "X-Amz-Signature",
                "X-Amz-Security-Token",
            ]
        );
        let get = |name: &str| pairs.iter().find(|(k, _)| k == name).unwrap().1.clone();
        assert_eq!(get("X-Amz-Algorithm"), "AWS4-HMAC-SHA256");
        let stamp = chrono::DateTime::<chrono::Utc>::from(fixed_time());
        assert_eq!(
            get("X-Amz-Credential"),
            format!(
                "ASIAEXAMPLE%2F{}%2Feu-central-1%2Fiotdevicegateway%2Faws4_request",
                stamp.format("%Y%m%d")
            )
        );
        assert_eq!(
            get("X-Amz-Date"),
            stamp.format("%Y%m%dT%H%M%SZ").to_string()
        );
        assert_eq!(get("X-Amz-Expires"), "300");
        assert_eq!(get("X-Amz-SignedHeaders"), "host");
        let signature = get("X-Amz-Signature");
        assert_eq!(signature.len(), 64);
        assert!(signature.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(get("X-Amz-Security-Token"), "tok%2Fen%2B1");
    }

    #[test]
    fn signature_excludes_session_token_and_depends_on_time() {
        let a =
            presign_wss_url(ENDPOINT, "eu-central-1", &credentials("one"), fixed_time()).unwrap();
        let b =
            presign_wss_url(ENDPOINT, "eu-central-1", &credentials("two"), fixed_time()).unwrap();
        let sig = |url: &str| {
            query_pairs(url)
                .into_iter()
                .find(|(k, _)| k == "X-Amz-Signature")
                .unwrap()
                .1
        };
        assert_eq!(sig(&a), sig(&b));
        let later = presign_wss_url(
            ENDPOINT,
            "eu-central-1",
            &credentials("one"),
            fixed_time() + Duration::from_secs(1),
        )
        .unwrap();
        assert_ne!(sig(&a), sig(&later));
    }

    #[test]
    fn presigner_swaps_uri_and_keeps_headers() {
        let presigner = Presigner::new(ENDPOINT, "eu-central-1", &credentials("tok"));
        let request = http::Request::builder()
            .uri(mqtt_wss_url(ENDPOINT))
            .header("Host", ENDPOINT)
            .header("Sec-WebSocket-Protocol", "mqtt")
            .body(())
            .unwrap();
        let signed = presigner.apply(request);
        assert_eq!(signed.uri().host(), Some(ENDPOINT));
        assert_eq!(signed.uri().path(), "/mqtt");
        assert!(signed.uri().query().unwrap().contains("X-Amz-Signature="));
        assert!(
            signed
                .uri()
                .query()
                .unwrap()
                .ends_with("X-Amz-Security-Token=tok")
        );
        assert_eq!(signed.headers()["Sec-WebSocket-Protocol"], "mqtt");
        assert_eq!(signed.headers()["Host"], ENDPOINT);
    }
}
