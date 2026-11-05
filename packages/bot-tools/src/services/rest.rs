//! REST calls for OAuth accounts. The token goes only to the hosts the provider configured
//! (never to a host the model or the URL names), over https on the default port, with no
//! redirects and the same bounds as the core `http_request`: 1 MiB out, 2 MiB back, 30 s.

use std::time::Duration;

use flow_like_types::reqwest::{self, Method, Url, header};
use serde_json::{Map, Value};

use super::Credential;
use super::egress::ensure_reachable;
use crate::env::Environment;

const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);

pub struct RestResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub text: String,
}

/// Whether `host` is one of `allowed`: an exact name, or `*.suffix` for any subdomain of it.
pub fn host_allowed(host: &str, allowed: &[String]) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    allowed.iter().any(|pattern| {
        let pattern = pattern.trim().trim_end_matches('.').to_ascii_lowercase();
        match pattern.strip_prefix("*.") {
            Some(suffix) => host
                .strip_suffix(suffix)
                .is_some_and(|prefix| prefix.len() > 1 && prefix.ends_with('.')),
            None => !pattern.is_empty() && host == pattern,
        }
    })
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// The URL a request goes to. A path starts at the account's first API host; a full URL must
/// name one of the account's hosts. Anything else is refused before a token exists.
pub fn request_url(
    api_hosts: &[String],
    path: &str,
    query: &Map<String, Value>,
) -> Result<Url, String> {
    let path = path.trim();
    let raw = if path.starts_with("https://") {
        path.to_string()
    } else if path.starts_with('/') {
        let first = api_hosts
            .iter()
            .find(|host| !host.starts_with("*."))
            .ok_or_else(|| {
                "This account has no fixed API host; pass a full https URL".to_string()
            })?;
        format!("https://{first}{path}")
    } else {
        return Err("The path starts with '/', or is a full https URL".to_string());
    };
    let mut url = Url::parse(&raw).map_err(|error| format!("The path is invalid: {error}"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Use an https URL on the default port, without credentials or a fragment".into(),
        );
    }
    let host = url.host_str().unwrap_or_default().to_string();
    if !host_allowed(&host, api_hosts) {
        return Err(format!(
            "'{host}' is not an API host of this account (allowed: {})",
            api_hosts.join(", ")
        ));
    }
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (name, value) in query {
            let value = scalar(value).ok_or_else(|| {
                format!("Query parameter '{name}' must be a string, number or boolean")
            })?;
            pairs.append_pair(name, &value);
        }
    }
    Ok(url)
}

fn method_of(method: &str) -> Result<Method, String> {
    match method {
        "GET" => Ok(Method::GET),
        "HEAD" => Ok(Method::HEAD),
        "POST" => Ok(Method::POST),
        "PUT" => Ok(Method::PUT),
        "PATCH" => Ok(Method::PATCH),
        "DELETE" => Ok(Method::DELETE),
        other => Err(format!(
            "Method '{other}' is not supported; use GET, HEAD, POST, PUT, PATCH or DELETE"
        )),
    }
}

pub async fn send(
    environment: Environment,
    method: &str,
    url: Url,
    credential: &Credential,
    body: Option<Value>,
) -> Result<RestResponse, String> {
    let method = method_of(method)?;
    ensure_reachable(environment, url.as_str()).await?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .build()
        .map_err(|error| format!("The HTTP client could not be created: {error}"))?;
    let mut request = client
        .request(method, url)
        .header(
            header::ACCEPT,
            "application/json, text/plain;q=0.8, */*;q=0.5",
        )
        .header(header::USER_AGENT, "Flow-Like-Bot");
    if let Some(value) = credential.header() {
        request = request.header(header::AUTHORIZATION, value);
    }
    if let Some(body) = body {
        let bytes =
            serde_json::to_vec(&body).map_err(|error| format!("The body is not JSON: {error}"))?;
        if bytes.len() > MAX_BODY_BYTES {
            return Err(format!("The body is larger than {MAX_BODY_BYTES} bytes"));
        }
        request = request
            .header(header::CONTENT_TYPE, "application/json")
            .body(bytes);
    }
    let mut response = request
        .send()
        .await
        .map_err(|error| format!("The request failed: {}", error.without_url()))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Reading the response failed: {}", error.without_url()))?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(format!(
                "The response is larger than {MAX_RESPONSE_BYTES} bytes; ask for less with query parameters"
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(RestResponse {
        status,
        content_type,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hosts(list: &[&str]) -> Vec<String> {
        list.iter().map(|host| host.to_string()).collect()
    }

    #[test]
    fn the_token_goes_only_to_configured_hosts() {
        let github = hosts(&["api.github.com", "uploads.github.com"]);
        assert!(host_allowed("api.github.com", &github));
        assert!(host_allowed("API.GitHub.com.", &github));
        for evil in [
            "api.github.com.evil.com",
            "evilapi.github.com",
            "github.com",
            "evil.com",
            "api.github.com@evil.com",
        ] {
            assert!(!host_allowed(evil, &github), "{evil}");
        }
        let google = hosts(&["*.googleapis.com"]);
        assert!(host_allowed("gmail.googleapis.com", &google));
        assert!(!host_allowed("googleapis.com", &google));
        assert!(!host_allowed("evilgoogleapis.com", &google));
        assert!(!host_allowed("gmail.googleapis.com.evil.com", &google));
        assert!(!host_allowed("anything.com", &[]));
    }

    #[test]
    fn urls_that_leave_the_allowlist_are_refused_before_any_token_exists() {
        let github = hosts(&["api.github.com"]);
        let none = Map::new();
        assert_eq!(
            request_url(&github, "/user", &none).unwrap().as_str(),
            "https://api.github.com/user"
        );
        for path in [
            "https://evil.com/user",
            "http://api.github.com/user",
            "https://api.github.com:8443/user",
            "https://user:pw@api.github.com/user",
            "https://api.github.com@evil.com/user",
            "https://api.github.com\\@evil.com/",
            "user",
            "//evil.com/x",
        ] {
            let result = request_url(&github, path, &none);
            if let Ok(url) = &result {
                assert_eq!(url.host_str(), Some("api.github.com"), "{path}");
                assert!(url.username().is_empty(), "{path}");
            }
        }
        for path in [
            "https://evil.com/user",
            "http://api.github.com/user",
            "https://api.github.com:8443/user",
            "https://user:pw@api.github.com/user",
            "https://api.github.com@evil.com/user",
            "user",
        ] {
            assert!(request_url(&github, path, &none).is_err(), "{path}");
        }
    }

    #[test]
    fn queries_are_encoded_and_scalar() {
        let hosts = hosts(&["api.notion.com"]);
        let mut query = Map::new();
        query.insert("q".into(), json!("a b&c"));
        query.insert("limit".into(), json!(5));
        let url = request_url(&hosts, "/v1/search", &query).unwrap();
        assert_eq!(url.query(), Some("q=a+b%26c&limit=5"));
        query.insert("nested".into(), json!({"a": 1}));
        assert!(request_url(&hosts, "/v1/search", &query).is_err());
    }
}
