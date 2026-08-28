//! API-side helpers: client access tokens, data-plane REST tokens, and the naming rules the
//! grants are built from (one group per channel, one literal role per side).

use flow_like_types::channel::now_unix;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Serialize;

pub const DATA_PLANE_API_VERSION: &str = "2024-12-01";
const GROUP_PREFIX: &str = "run:";
const MAX_CHANNEL_ID_CHARS: usize = 1000;
/// RFC 3986 unreserved characters stay literal; everything else (including `:`) is encoded.
const URL_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

#[derive(Serialize)]
struct ClientClaims<'a> {
    aud: String,
    iat: i64,
    exp: i64,
    sub: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    role: Vec<String>,
    #[serde(rename = "webpubsub.group", skip_serializing_if = "Vec::is_empty")]
    webpubsub_group: Vec<String>,
}

#[derive(Serialize)]
struct RestClaims<'a> {
    aud: &'a str,
    iat: i64,
    exp: i64,
}

pub fn normalize_endpoint(endpoint: &str) -> &str {
    endpoint.trim_end_matches('/')
}

fn strip_scheme(endpoint: &str) -> &str {
    endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest)
}

fn lifetime(ttl_secs: i64) -> flow_like_types::Result<(i64, i64)> {
    if ttl_secs <= 0 {
        flow_like_types::bail!("Azure Web PubSub token ttl must be positive, got {ttl_secs}s");
    }
    let iat = now_unix();
    Ok((iat, iat + ttl_secs))
}

fn sign(claims: &impl Serialize, access_key: &str) -> flow_like_types::Result<String> {
    encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(access_key.as_bytes()),
    )
    .map_err(|e| flow_like_types::anyhow!("signing Azure Web PubSub token: {e}"))
}

/// Audience of every client access token for `hub`: `{endpoint}/client/hubs/{hub}`.
pub fn client_audience(endpoint: &str, hub: &str) -> String {
    format!("{}/client/hubs/{hub}", normalize_endpoint(endpoint))
}

/// HS256 client access token signed with the hub access key. `roles` are literal
/// `webpubsub.*` role strings; `initial_groups` become the `webpubsub.group` claim so the
/// connection is joined on connect (omitted when empty).
pub fn client_access_token(
    endpoint: &str,
    hub: &str,
    access_key: &str,
    user_id: &str,
    roles: &[String],
    initial_groups: &[String],
    ttl_secs: i64,
) -> flow_like_types::Result<String> {
    let (iat, exp) = lifetime(ttl_secs)?;
    sign(
        &ClientClaims {
            aud: client_audience(endpoint, hub),
            iat,
            exp,
            sub: user_id,
            role: roles.to_vec(),
            webpubsub_group: initial_groups.to_vec(),
        },
        access_key,
    )
}

/// `wss://{host}/client/hubs/{hub}?access_token={token}`.
pub fn client_ws_url(endpoint: &str, hub: &str, token: &str) -> String {
    let host = strip_scheme(normalize_endpoint(endpoint));
    format!("wss://{host}/client/hubs/{hub}?access_token={token}")
}

/// Group a channel's pushes travel through. Group names are role literals, so a `.` would
/// open the wildcard hierarchy and is rejected along with whitespace and control characters.
pub fn group_for(channel_id: &str) -> flow_like_types::Result<String> {
    if channel_id.is_empty() {
        flow_like_types::bail!("channel id must not be empty");
    }
    if channel_id.chars().count() > MAX_CHANNEL_ID_CHARS {
        flow_like_types::bail!(
            "channel id exceeds {MAX_CHANNEL_ID_CHARS} characters and cannot name an Azure Web PubSub group"
        );
    }
    if let Some(offender) = channel_id
        .chars()
        .find(|c| *c == '.' || c.is_whitespace() || c.is_control())
    {
        flow_like_types::bail!(
            "channel id {channel_id:?} contains {offender:?}, which is not allowed in an Azure Web PubSub group name"
        );
    }
    Ok(format!("{GROUP_PREFIX}{channel_id}"))
}

/// The browser may only answer: send into the group, never join or read it.
pub fn client_roles(group: &str) -> Vec<String> {
    vec![format!("webpubsub.sendToGroup.{group}")]
}

/// The executor may only listen: join the group, never send into it.
pub fn executor_roles(group: &str) -> Vec<String> {
    vec![format!("webpubsub.joinLeaveGroup.{group}")]
}

/// HS256 data-plane REST token; `request_url` must be byte-identical to the URL sent,
/// query string included.
pub fn rest_token(
    access_key: &str,
    request_url: &str,
    ttl_secs: i64,
) -> flow_like_types::Result<String> {
    let (iat, exp) = lifetime(ttl_secs)?;
    sign(
        &RestClaims {
            aud: request_url,
            iat,
            exp,
        },
        access_key,
    )
}

/// `POST` target for sending to `group`; the group is percent-encoded exactly as the REST
/// token audience expects it.
pub fn send_to_group_url(endpoint: &str, hub: &str, group: &str) -> String {
    format!(
        "{}/api/hubs/{hub}/groups/{}/:send?api-version={DATA_PLANE_API_VERSION}",
        normalize_endpoint(endpoint),
        utf8_percent_encode(group, URL_COMPONENT)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation, decode};
    use serde::Deserialize;
    use serde_json::Value;

    const ENDPOINT: &str = "https://demo.webpubsub.azure.com/";
    const HUB: &str = "channels";
    const KEY: &str = "0123456789abcdef0123456789abcdef";

    #[derive(Deserialize)]
    struct Claims {
        aud: String,
        sub: Option<String>,
        iat: i64,
        exp: i64,
        #[serde(default)]
        role: Vec<String>,
        #[serde(rename = "webpubsub.group", default)]
        groups: Vec<String>,
    }

    fn decode_claims(token: &str, aud: &str) -> Claims {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[aud]);
        decode::<Claims>(
            token,
            &DecodingKey::from_secret(KEY.as_bytes()),
            &validation,
        )
        .expect("token validates against its audience")
        .claims
    }

    fn raw_claims(token: &str) -> Value {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_aud = false;
        decode::<Value>(
            token,
            &DecodingKey::from_secret(KEY.as_bytes()),
            &validation,
        )
        .unwrap()
        .claims
    }

    #[test]
    fn client_token_claims_roundtrip() {
        let group = group_for("run1").unwrap();
        let token = client_access_token(
            ENDPOINT,
            HUB,
            KEY,
            "user-7",
            &executor_roles(&group),
            std::slice::from_ref(&group),
            300,
        )
        .unwrap();
        let claims = decode_claims(
            &token,
            "https://demo.webpubsub.azure.com/client/hubs/channels",
        );
        assert_eq!(
            claims.aud,
            "https://demo.webpubsub.azure.com/client/hubs/channels"
        );
        assert_eq!(claims.sub.as_deref(), Some("user-7"));
        assert_eq!(claims.role, vec!["webpubsub.joinLeaveGroup.run:run1"]);
        assert_eq!(claims.groups, vec!["run:run1"]);
        assert_eq!(claims.exp - claims.iat, 300);
    }

    #[test]
    fn client_token_rejects_other_audience() {
        let token = client_access_token(ENDPOINT, HUB, KEY, "u", &[], &[], 60).unwrap();
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&["https://demo.webpubsub.azure.com/client/hubs/other"]);
        assert!(
            decode::<Claims>(
                &token,
                &DecodingKey::from_secret(KEY.as_bytes()),
                &validation
            )
            .is_err()
        );
    }

    #[test]
    fn client_token_omits_empty_groups_and_roles() {
        let token = client_access_token(ENDPOINT, HUB, KEY, "u", &[], &[], 60).unwrap();
        let claims = raw_claims(&token);
        assert!(claims.get("webpubsub.group").is_none());
        assert!(claims.get("role").is_none());
        assert_eq!(claims["sub"], "u");
    }

    #[test]
    fn token_ttl_must_be_positive() {
        assert!(client_access_token(ENDPOINT, HUB, KEY, "u", &[], &[], 0).is_err());
        assert!(rest_token(KEY, "https://x/y", -1).is_err());
    }

    #[test]
    fn rest_token_audience_is_request_url() {
        let url = send_to_group_url(ENDPOINT, HUB, "run:abc");
        let token = rest_token(KEY, &url, 60).unwrap();
        let claims = decode_claims(&token, &url);
        assert_eq!(claims.aud, url);
        assert!(claims.sub.is_none());
        assert!(claims.role.is_empty());
    }

    #[test]
    fn roles_are_exactly_one_scoped_literal() {
        assert_eq!(client_roles("run:x"), vec!["webpubsub.sendToGroup.run:x"]);
        assert_eq!(
            executor_roles("run:x"),
            vec!["webpubsub.joinLeaveGroup.run:x"]
        );
    }

    #[test]
    fn group_for_validates_channel_id() {
        assert_eq!(group_for("abc123").unwrap(), "run:abc123");
        assert!(group_for("").is_err());
        assert!(group_for("a.b").is_err());
        assert!(group_for("a b").is_err());
        assert!(group_for("a\nb").is_err());
        assert!(group_for(&"x".repeat(1000)).is_ok());
        assert!(group_for(&"x".repeat(1001)).is_err());
    }

    #[test]
    fn ws_url_shape() {
        assert_eq!(
            client_ws_url(ENDPOINT, HUB, "tok.en"),
            "wss://demo.webpubsub.azure.com/client/hubs/channels?access_token=tok.en"
        );
        assert_eq!(
            client_ws_url("wss://demo.webpubsub.azure.com", HUB, "t"),
            "wss://demo.webpubsub.azure.com/client/hubs/channels?access_token=t"
        );
    }

    #[test]
    fn send_url_encodes_group() {
        assert_eq!(
            send_to_group_url(ENDPOINT, HUB, "run:abc"),
            "https://demo.webpubsub.azure.com/api/hubs/channels/groups/run%3Aabc/:send?api-version=2024-12-01"
        );
    }
}
