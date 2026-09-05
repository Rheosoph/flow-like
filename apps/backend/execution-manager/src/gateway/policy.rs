use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    time::{SystemTime, UNIX_EPOCH},
};

use hyper::{HeaderMap, Method, Uri};
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use tokio::net::lookup_host;

use super::{BoxError, MAX_CAPABILITY_SECONDS};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyData {
    pub callback_url: String,
    pub object_store_url: String,
    pub app_id: String,
    pub run_id: String,
    pub executor_jwt: String,
    pub deadline: f64,
    #[serde(default = "default_buckets")]
    pub buckets: Vec<String>,
    #[serde(default)]
    pub allowed_https_hosts: Vec<String>,
    #[serde(default)]
    pub object_store_tls_gateway: bool,
}

fn default_buckets() -> Vec<String> {
    ["flow-like-meta", "flow-like-content", "flow-like-logs"]
        .map(str::to_owned)
        .into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Origin {
    scheme: String,
    host: String,
    port: u16,
}

impl Origin {
    fn parse(uri: &Uri) -> Result<Self, &'static str> {
        let scheme = uri.scheme_str().ok_or("Absolute HTTP target required")?;
        let authority = uri.authority().ok_or("HTTP authority required")?;
        if !matches!(scheme, "http" | "https") || authority.as_str().contains('@') {
            return Err("Invalid HTTP destination");
        }
        let host = authority.host().to_ascii_lowercase();
        if host.is_empty() {
            return Err("Missing host");
        }
        if authority.as_str().len() != host.len() && authority.port_u16().is_none() {
            return Err("Invalid HTTP port");
        }
        Ok(Self {
            scheme: scheme.into(),
            host,
            port: authority
                .port_u16()
                .unwrap_or(if scheme == "https" { 443 } else { 80 }),
        })
    }

    fn matches_host(&self, host: &str) -> bool {
        let default = if self.scheme == "https" { 443 } else { 80 };
        let host = host.to_ascii_lowercase();
        host == format!("{}:{}", self.host, self.port)
            || (self.port == default && host == self.host)
    }
}

pub struct Policy {
    pub data: PolicyData,
    callback: Origin,
    storage: Origin,
    token: String,
    buckets: HashSet<String>,
    https_hosts: HashSet<String>,
}

pub fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn component(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || b"_.:-".contains(c))
}

fn hostname(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 253
        && !value.starts_with('.')
        && !value.ends_with('.')
        && value.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        })
}

/// Check the original URI before any URL implementation can normalize dot paths.
fn canonical_path(path: &str, storage: bool) -> Result<String, &'static str> {
    let bytes = path.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit())
        {
            return Err("Invalid path encoding");
        }
    }
    let decoded = percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| "Invalid path encoding")?
        .into_owned();
    let ambiguous = |value: &str| {
        value.contains('\\')
            || value.contains("//")
            || value.split('/').any(|part| matches!(part, "." | ".."))
            || value.chars().any(char::is_control)
    };
    if ambiguous(&decoded) || (!storage && decoded.contains('%')) {
        return Err("Ambiguous request path");
    }
    // Literal percent signs in object keys remain valid. Repeated encodings of
    // path separators or dot segments cannot acquire authority downstream.
    if storage && decoded.contains('%') {
        let twice = percent_decode_str(&decoded)
            .decode_utf8()
            .map_err(|_| "Invalid path encoding")?;
        if ambiguous(&twice) || twice.matches('/').count() != decoded.matches('/').count() {
            return Err("Ambiguous request path");
        }
    }
    Ok(decoded)
}

impl Policy {
    pub fn new(data: PolicyData) -> Result<Self, BoxError> {
        if !component(&data.app_id)
            || !component(&data.run_id)
            || data.executor_jwt.is_empty()
            || data.executor_jwt.len() > 32768
            || data.executor_jwt.bytes().any(|c| c <= 32 || c == 127)
            || !data.deadline.is_finite()
            || data.deadline <= now()
            || data.deadline > now() + MAX_CAPABILITY_SECONDS as f64
        {
            return Err("Invalid execution capability".into());
        }
        let callback = Origin::parse(&data.callback_url.parse()?)?;
        let storage = Origin::parse(&data.object_store_url.parse()?)?;
        if callback == storage {
            return Err("Storage and callback must have separate authorities".into());
        }
        if data.buckets.is_empty()
            || data.buckets.len() > 32
            || data.buckets.iter().any(|b| !component(b))
            || data.allowed_https_hosts.len() > 128
            || data.allowed_https_hosts.iter().any(|h| !hostname(h))
        {
            return Err("Invalid egress grants".into());
        }
        let https_hosts: HashSet<_> = data
            .allowed_https_hosts
            .iter()
            .map(|h| h.to_ascii_lowercase())
            .collect();
        if https_hosts.contains(&callback.host) || https_hosts.contains(&storage.host) {
            return Err("Control and storage hosts cannot be integration grants".into());
        }
        Ok(Self {
            token: format!("Bearer {}", data.executor_jwt),
            buckets: data.buckets.iter().cloned().collect(),
            data,
            callback,
            storage,
            https_hosts,
        })
    }

    pub fn authorize(
        &self,
        method: &Method,
        uri: &Uri,
        headers: &HeaderMap,
    ) -> Result<(), &'static str> {
        if now() >= self.data.deadline {
            return Err("Execution capability expired");
        }
        let origin = Origin::parse(uri)?;
        let path = canonical_path(uri.path(), origin == self.storage)?;
        if !origin.matches_host(
            headers
                .get("host")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        ) {
            return Err("Host does not match destination");
        }
        if origin == self.callback {
            if method == Method::GET && path == "/api/v1/execution/.well-known/jwks.json" {
                return Ok(());
            }
            let token = headers
                .get("authorization")
                .map(|h| h.as_bytes())
                .unwrap_or_default();
            if !constant_time_eq::constant_time_eq(token, self.token.as_bytes()) {
                return Err("Callback capability mismatch");
            }
            let base = format!("/api/v1/channels/{}", self.data.run_id);
            let message = path.strip_prefix(&format!("{base}/messages/"));
            let allowed = (method == Method::POST
                && matches!(
                    path.as_str(),
                    "/api/v1/execution/progress" | "/api/v1/execution/events"
                ))
                || (method == Method::GET
                    && path == format!("/api/v1/execution/apps/{}/widgets", self.data.app_id))
                || (method == Method::DELETE && path == base)
                || (method == Method::POST
                    && (path == format!("{base}/messages")
                        || path == format!("{base}/inbound/drain")))
                || (method == Method::GET && path == format!("{base}/status"))
                || (matches!(*method, Method::GET | Method::DELETE)
                    && message.is_some_and(component));
            return if allowed {
                Ok(())
            } else {
                Err("Callback path or method denied")
            };
        }
        if origin == self.storage {
            let admin = [
                "action",
                "policy",
                "acl",
                "cors",
                "lifecycle",
                "replication",
                "versioning",
                "website",
                "notification",
                "logging",
                "encryption",
                "ownershipcontrols",
                "publicaccessblock",
                "requestpayment",
                "accelerate",
                "object-lock",
                "analytics",
                "inventory",
                "metrics",
                "intelligent-tiering",
            ];
            if url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
                .any(|(key, _)| admin.contains(&key.to_ascii_lowercase().as_str()))
                || (method == Method::POST
                    && headers
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|value| {
                            value
                                .split(';')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .eq_ignore_ascii_case("application/x-www-form-urlencoded")
                        }))
            {
                return Err("Object store administrative and STS requests are denied");
            }
            let bucket = path
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or_default();
            if matches!(*method, Method::PUT | Method::DELETE) && path.trim_matches('/') == bucket {
                return Err("Bucket administration is denied");
            }
            return if self.buckets.contains(bucket)
                && matches!(
                    *method,
                    Method::GET | Method::HEAD | Method::PUT | Method::POST | Method::DELETE
                ) {
                Ok(())
            } else {
                Err("Object store administrative requests are denied")
            };
        }
        Err("Destination denied")
    }

    pub async fn connect_destination(&self, target: &str) -> Result<SocketAddr, BoxError> {
        let (host, port) = target.rsplit_once(':').ok_or("Invalid CONNECT target")?;
        if !hostname(host)
            || port.is_empty()
            || port.len() > 5
            || !port.bytes().all(|c| c.is_ascii_digit())
        {
            return Err("Invalid CONNECT target".into());
        }
        let port: u16 = port.parse()?;
        let host = host.to_ascii_lowercase();
        if now() >= self.data.deadline {
            return Err("Execution capability expired".into());
        }
        let storage = self.data.object_store_tls_gateway
            && self.storage
                == Origin {
                    scheme: "https".into(),
                    host: host.clone(),
                    port,
                };
        if !storage && (port != 443 || !self.https_hosts.contains(&host)) {
            return Err("CONNECT destination denied".into());
        }
        let addresses: Vec<_> = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            lookup_host((host.as_str(), port)),
        )
        .await??
        .take(33)
        .collect();
        self.checked_addresses(&addresses, storage)
    }

    fn checked_addresses(
        &self,
        addresses: &[SocketAddr],
        storage: bool,
    ) -> Result<SocketAddr, BoxError> {
        if addresses.is_empty()
            || addresses.len() > 32
            || (!storage && addresses.iter().any(|a| !global_address(a.ip())))
        {
            return Err("Integration DNS resolved to a private or reserved address".into());
        }
        // The caller connects this address directly. No second DNS lookup occurs.
        Ok(addresses[0])
    }
}

fn global_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let n = u32::from(ip);
            let denied = [
                (0x00000000, 8),
                (0x0a000000, 8),
                (0x64400000, 10),
                (0x7f000000, 8),
                (0xa9fe0000, 16),
                (0xac100000, 12),
                (0xc0000000, 24),
                (0xc0000200, 24),
                (0xc0586300, 24),
                (0xc0a80000, 16),
                (0xc6120000, 15),
                (0xc6336400, 24),
                (0xcb007100, 24),
                (0xe0000000, 4),
                (0xf0000000, 4),
            ];
            !denied
                .iter()
                .any(|(base, prefix)| n >> (32 - prefix) == base >> (32 - prefix))
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            // Only global unicast, excluding transition and documentation ranges.
            s[0] & 0xe000 == 0x2000
                && !(s[0] == 0x2001 && (s[1] <= 0x01ff || s[1] == 0x0db8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] & 0xf000 == 0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> PolicyData {
        PolicyData {
            callback_url: "http://callback:8080".into(),
            object_store_url: "http://objects:9000".into(),
            app_id: "app-1".into(),
            run_id: "run-1".into(),
            executor_jwt: "jwt".into(),
            deadline: now() + 300.0,
            buckets: default_buckets(),
            allowed_https_hosts: vec!["example.com".into()],
            object_store_tls_gateway: false,
        }
    }
    fn auth(target: &str, token: bool) -> HeaderMap {
        let uri: Uri = target.parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("host", uri.authority().unwrap().as_str().parse().unwrap());
        if token {
            headers.insert("authorization", "Bearer jwt".parse().unwrap());
        }
        headers
    }

    #[test]
    fn callback_capability_is_bound_to_app_run_and_exact_method() {
        let policy = Policy::new(data()).unwrap();
        for (method, path, token, allowed) in [
            (
                Method::GET,
                "/api/v1/execution/.well-known/jwks.json",
                false,
                true,
            ),
            (Method::POST, "/api/v1/execution/progress", true, true),
            (Method::POST, "/api/v1/execution/progress", false, false),
            (
                Method::GET,
                "/api/v1/channels/run-1/messages/request-1",
                true,
                true,
            ),
            (Method::GET, "/api/v1/channels/run-2/status", true, false),
            (
                Method::GET,
                "/api/v1/execution/apps/app-2/widgets",
                true,
                false,
            ),
            (Method::POST, "/api/v1/execution/%2570rogress", true, false),
            (
                Method::GET,
                "/api/v1/channels/run-1/messages/../grant",
                true,
                false,
            ),
            (
                Method::GET,
                "/api/v1/channels/run-1/messages/%2e%2e/grant",
                true,
                false,
            ),
        ] {
            let target = format!("http://callback:8080{path}");
            assert_eq!(
                policy
                    .authorize(&method, &target.parse().unwrap(), &auth(&target, token))
                    .is_ok(),
                allowed,
                "{path}"
            );
        }
    }

    #[test]
    fn object_paths_allow_keys_but_deny_admin_sts_and_normalization() {
        let policy = Policy::new(data()).unwrap();
        for (path, allowed) in [
            ("/flow-like-meta/app?X-Amz-Signature=sig", true),
            ("/flow-like-content/files/100%25%20growth.csv", true),
            ("/?Action=AssumeRole", false),
            ("/flow-like-meta/app?%41cTiOn=AssumeRole", false),
            ("/minio/admin/v3/info", false),
            ("/private-other/key", false),
            ("/flow-like-meta/../private-other", false),
            ("/flow-like-meta/%252e%252e/private-other", false),
            ("/flow-like-meta/%255cadmin", false),
            ("/flow-like-meta//key", false),
        ] {
            let target = format!("http://objects:9000{path}");
            assert_eq!(
                policy
                    .authorize(
                        &Method::GET,
                        &target.parse().unwrap(),
                        &auth(&target, false)
                    )
                    .is_ok(),
                allowed,
                "{path}"
            );
        }
    }

    #[test]
    fn integration_dns_denies_every_private_or_reserved_answer() {
        let policy = Policy::new(data()).unwrap();
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.2.1",
            "224.0.0.1",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "64:ff9b::7f00:1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            let addr = SocketAddr::new(address.parse().unwrap(), 443);
            assert!(
                policy
                    .checked_addresses(&["93.184.216.34:443".parse().unwrap(), addr], false)
                    .is_err(),
                "{address}"
            );
        }
        for address in [
            "93.184.216.34",
            "2606:4700:4700::1111",
            "2001:4860:4860::8888",
        ] {
            assert!(global_address(address.parse().unwrap()), "{address}");
        }
    }

    #[tokio::test]
    async fn connect_requires_exact_host_and_storage_opt_in() {
        let policy = Policy::new(data()).unwrap();
        for target in [
            "callback:443",
            "objects:443",
            "example.com:22",
            "anything.example.com:443",
            "example.com.:443",
            "example.com:000443",
        ] {
            assert!(policy.connect_destination(target).await.is_err());
        }
        let mut input = data();
        input.object_store_url = "https://objects.example:9443".into();
        assert!(
            Policy::new(input.clone())
                .unwrap()
                .connect_destination("objects.example:9443")
                .await
                .is_err()
        );
        input.allowed_https_hosts.push("callback".into());
        assert!(Policy::new(input).is_err());
    }

    #[test]
    fn invalid_identity_expiry_or_host_cannot_receive_authority() {
        let mut input = data();
        input.deadline = 0.0;
        assert!(Policy::new(input).is_err());
        let mut input = data();
        input.run_id = "../other".into();
        assert!(Policy::new(input).is_err());
        let policy = Policy::new(data()).unwrap();
        let target = "http://objects:9000/flow-like-meta/key";
        let mut headers = auth(target, false);
        headers.insert("host", "callback:8080".parse().unwrap());
        assert!(
            policy
                .authorize(&Method::GET, &target.parse().unwrap(), &headers)
                .is_err()
        );
    }

    #[test]
    fn bucket_administration_and_form_sts_are_denied_while_multipart_works() {
        let policy = Policy::new(data()).unwrap();
        for method in [Method::PUT, Method::DELETE] {
            let target = "http://objects:9000/flow-like-meta/";
            assert!(
                policy
                    .authorize(&method, &target.parse().unwrap(), &auth(target, false))
                    .is_err()
            );
        }
        let target = "http://objects:9000/flow-like-meta/object?uploads";
        let mut headers = auth(target, false);
        assert!(
            policy
                .authorize(&Method::POST, &target.parse().unwrap(), &headers)
                .is_ok()
        );
        headers.insert(
            "content-type",
            "application/x-www-form-urlencoded".parse().unwrap(),
        );
        assert!(
            policy
                .authorize(&Method::POST, &target.parse().unwrap(), &headers)
                .is_err()
        );
    }

    #[test]
    fn maximum_execution_duration_keeps_startup_and_terminal_allowances() {
        let mut input = data();
        input.deadline = now() + 86400.0 + 600.0 + 300.0 + 300.0;
        assert!(Policy::new(input.clone()).is_ok());
        input.deadline = now() + MAX_CAPABILITY_SECONDS as f64 + 60.0;
        assert!(Policy::new(input).is_err());
    }
}
