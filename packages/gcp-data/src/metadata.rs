//! Metadata-server access tokens for every GCP data-plane call in this crate.
//!
//! Cloud Run, GKE and GCE all expose the workload's service account through the
//! instance metadata server. That is the only credential source this crate
//! accepts: a service-account key file is a long-lived bearer secret that has to
//! be stored, mounted and rotated, and the whole point of the GCP port is that
//! no such secret exists. An explicit token source is offered for local
//! development only, and never through the environment.

use serde::Deserialize;
use std::{
    fmt::Debug,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// The link-local metadata endpoint, resolved through its stable DNS name.
const METADATA_TOKEN_URL: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";
/// Only a request carrying this header is served, and only a genuine metadata
/// server echoes it back. Checking both directions is what stops a hijacked
/// `metadata.google.internal` (or an SSRF-reachable look-alike) from feeding the
/// process an attacker-controlled token.
const METADATA_FLAVOR_HEADER: &str = "Metadata-Flavor";
const METADATA_FLAVOR_VALUE: &str = "Google";

/// Firestore is part of the Datastore API surface, so this is the scope its REST
/// endpoint checks.
pub const DATASTORE_SCOPE: &str = "https://www.googleapis.com/auth/datastore";
/// Cloud SQL IAM database authentication only needs the login scope. The token
/// is handed to PostgreSQL as a password and therefore travels further than any
/// other token this process holds; minting it narrow means a leaked copy cannot
/// be replayed against the rest of the project.
pub const SQL_LOGIN_SCOPE: &str = "https://www.googleapis.com/auth/sqlservice.login";

/// Tokens are handed out until this margin before expiry, so an in-flight
/// request that was issued a cached token still completes against a valid one.
///
/// 3m45s, not 5m, and the number is not free: the metadata server itself caches
/// the service-account token and only rotates it once roughly five minutes
/// remain, so a margin at or above that floor makes every fetch inside the
/// window hand back the same token, which the next call judges stale again and
/// refetches - one metadata round-trip per Firestore/Cloud SQL call, under the
/// mutex, until the server rotates. Google's own auth libraries sit at 3m45s
/// for exactly this reason. Invariant: `REFRESH_MARGIN_SECONDS` stays strictly
/// below the metadata server's ~300 s rotation floor.
const REFRESH_MARGIN_SECONDS: i64 = 3 * 60 + 45;
/// A token that was fetched this recently is served even inside the refresh
/// margin, as long as it is still usable at all. This is the second half of the
/// guard above: if the metadata server ever hands back a token shorter than the
/// margin, the process uses it for its remaining life instead of hammering the
/// endpoint on every call, mirroring the API's `fetch_metadata_token`.
const MIN_REUSE_SECONDS: i64 = 60;
/// Below this remaining lifetime a token is not handed out even when freshly
/// fetched; the request it would be issued to might outlive it.
const MIN_USABLE_SECONDS: i64 = 30;
const MAX_FETCH_ATTEMPTS: u32 = 3;
const FETCH_RETRY_DELAY_MS: u64 = 250;
const MAX_TOKEN_BODY_BYTES: usize = 16 * 1024;

/// Environment that would redirect, replace or intercept the credential this
/// crate mints. Every one of these is rejected at startup rather than merely
/// ignored: silently ignoring a set `GCE_METADATA_HOST` leaves an operator
/// believing an override took effect, and silently honouring it would let any
/// process that can write the environment steal the workload identity.
pub const FORBIDDEN_CREDENTIAL_SETTINGS: &[&str] = &[
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_APPLICATION_CREDENTIALS_JSON",
    "GOOGLE_CREDENTIALS",
    "GOOGLE_OAUTH_ACCESS_TOKEN",
    "CLOUDSDK_AUTH_ACCESS_TOKEN",
    "GCE_METADATA_HOST",
    "GCE_METADATA_IP",
    "GCE_METADATA_ROOT",
    "METADATA_SERVER_DETECTION",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
];

#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("metadata credential configuration error: {0}")]
    Configuration(String),
    #[error("metadata server transport error: {0}")]
    Transport(String),
    #[error("metadata server returned HTTP {status}: {message}")]
    Service { status: u16, message: String },
    #[error("metadata server response was not a usable access token: {0}")]
    Malformed(String),
}

/// An OAuth2 bearer token plus the absolute instant it stops being accepted.
#[derive(Clone)]
pub struct AccessToken {
    value: String,
    expires_at_unix: i64,
}

impl AccessToken {
    pub fn new(value: impl Into<String>, expires_at_unix: i64) -> Self {
        Self {
            value: value.into(),
            expires_at_unix,
        }
    }

    /// Deliberately a method rather than a public field, so a token never lands
    /// in a log line by way of a struct-update or a derived `Debug`.
    pub fn secret(&self) -> &str {
        &self.value
    }

    pub fn expires_at_unix(&self) -> i64 {
        self.expires_at_unix
    }

    pub fn remaining_seconds(&self) -> i64 {
        self.expires_at_unix.saturating_sub(unix_timestamp())
    }
}

impl Debug for AccessToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AccessToken")
            .field("expires_at_unix", &self.expires_at_unix)
            .finish_non_exhaustive()
    }
}

/// Where a bearer token comes from. `Metadata` is the only variant reachable
/// from the environment; `Static` exists so a developer can drive the clients
/// against a real project with a `gcloud auth print-access-token` value without
/// that path ever being selectable by a deployed process.
#[derive(Clone)]
pub enum TokenSource {
    Metadata(Arc<MetadataTokenProvider>),
    Static(Arc<AccessToken>),
}

impl TokenSource {
    pub fn metadata(scopes: &[&str]) -> Result<Self, MetadataError> {
        ensure_no_forbidden_credential_env()?;
        Ok(Self::Metadata(Arc::new(MetadataTokenProvider::new(
            scopes,
        )?)))
    }

    pub fn static_token(value: impl Into<String>, expires_at_unix: i64) -> Self {
        Self::Static(Arc::new(AccessToken::new(value, expires_at_unix)))
    }

    pub async fn token(&self) -> Result<AccessToken, MetadataError> {
        match self {
            Self::Metadata(provider) => provider.token().await,
            Self::Static(token) => Ok(token.as_ref().clone()),
        }
    }
}

impl Debug for TokenSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Metadata(provider) => formatter
                .debug_struct("TokenSource::Metadata")
                .field("scopes", &provider.scopes)
                .finish(),
            Self::Static(_) => formatter.write_str("TokenSource::Static"),
        }
    }
}

pub struct MetadataTokenProvider {
    http: reqwest::Client,
    url: String,
    scopes: String,
    cached: tokio::sync::Mutex<Option<CachedToken>>,
}

struct CachedToken {
    token: AccessToken,
    fetched_at_unix: i64,
}

impl CachedToken {
    /// Serve the cached token while it clears the refresh margin, or - if the
    /// metadata server handed back something shorter than the margin - for at
    /// least `MIN_REUSE_SECONDS` after it was fetched, so a short token is used
    /// rather than refetched on every call.
    fn is_servable(&self, now_unix: i64) -> bool {
        let remaining = self.token.expires_at_unix().saturating_sub(now_unix);
        remaining > REFRESH_MARGIN_SECONDS
            || (remaining > MIN_USABLE_SECONDS
                && now_unix.saturating_sub(self.fetched_at_unix) < MIN_REUSE_SECONDS)
    }
}

impl MetadataTokenProvider {
    pub fn new(scopes: &[&str]) -> Result<Self, MetadataError> {
        if scopes.is_empty() {
            return Err(MetadataError::Configuration(
                "at least one OAuth2 scope is required; an unscoped token would inherit every \
                 permission the service account holds"
                    .to_string(),
            ));
        }
        let scopes = scopes.join(",");

        // `no_proxy` is load-bearing, not tidiness. reqwest honours `HTTP_PROXY`
        // by default, so without this a proxy variable would route a plaintext
        // credential request through a third party. The forbidden-environment
        // check above is the first line of defence; this is the second, because
        // proxy settings can also arrive from a system configuration file.
        let http = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("flow-like/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| MetadataError::Configuration(error.to_string()))?;

        Ok(Self {
            http,
            url: format!(
                "{METADATA_TOKEN_URL}?scopes={}",
                urlencoding::encode(&scopes)
            ),
            scopes,
            cached: tokio::sync::Mutex::new(None),
        })
    }

    pub async fn token(&self) -> Result<AccessToken, MetadataError> {
        // The fetch happens while the mutex is held. Serialising callers costs a
        // few milliseconds on a cold start and removes the thundering herd that
        // otherwise hits the metadata server every time a token ages out — the
        // metadata server rate-limits, and a throttled token refresh takes down
        // every request in flight at once.
        let mut cached = self.cached.lock().await;
        if let Some(entry) = cached.as_ref()
            && entry.is_servable(unix_timestamp())
        {
            return Ok(entry.token.clone());
        }

        let token = self.fetch().await?;
        *cached = Some(CachedToken {
            token: token.clone(),
            fetched_at_unix: unix_timestamp(),
        });
        Ok(token)
    }

    async fn fetch(&self) -> Result<AccessToken, MetadataError> {
        let mut last_error = None;
        for attempt in 0..MAX_FETCH_ATTEMPTS {
            match self.fetch_once().await {
                Ok(token) => return Ok(token),
                // A malformed or rejected response is a configuration fault and
                // will not heal on its own; only reachability problems are worth
                // another attempt.
                Err(error @ (MetadataError::Transport(_) | MetadataError::Service { .. })) => {
                    tracing::warn!(
                        attempt = attempt + 1,
                        error = %error,
                        "metadata token fetch failed; retrying"
                    );
                    last_error = Some(error);
                    if attempt + 1 < MAX_FETCH_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(
                            FETCH_RETRY_DELAY_MS * u64::from(attempt + 1),
                        ))
                        .await;
                    }
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_error.unwrap_or_else(|| {
            MetadataError::Transport("metadata server was unreachable".to_string())
        }))
    }

    async fn fetch_once(&self) -> Result<AccessToken, MetadataError> {
        let response = self
            .http
            .get(&self.url)
            .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
            .send()
            .await
            .map_err(|error| MetadataError::Transport(error.to_string()))?;

        let status = response.status();
        let flavor_echoed = response
            .headers()
            .get(METADATA_FLAVOR_HEADER)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case(METADATA_FLAVOR_VALUE));
        let body = response
            .bytes()
            .await
            .map_err(|error| MetadataError::Transport(error.to_string()))?;

        if !status.is_success() {
            let body = &body[..body.len().min(MAX_TOKEN_BODY_BYTES)];
            return Err(MetadataError::Service {
                status: status.as_u16(),
                message: String::from_utf8_lossy(body).into_owned(),
            });
        }
        if !flavor_echoed {
            return Err(MetadataError::Malformed(
                "the responder did not echo Metadata-Flavor: Google and is therefore not the \
                 instance metadata server"
                    .to_string(),
            ));
        }

        let payload: MetadataTokenResponse = serde_json::from_slice(&body)
            .map_err(|error| MetadataError::Malformed(error.to_string()))?;
        if payload.access_token.trim().is_empty() {
            return Err(MetadataError::Malformed(
                "access_token was empty".to_string(),
            ));
        }
        if !payload.token_type.eq_ignore_ascii_case("Bearer") {
            return Err(MetadataError::Malformed(format!(
                "unsupported token_type '{}'",
                payload.token_type
            )));
        }

        Ok(AccessToken::new(
            payload.access_token,
            unix_timestamp().saturating_add(payload.expires_in),
        ))
    }
}

impl Debug for MetadataTokenProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MetadataTokenProvider")
            .field("scopes", &self.scopes)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct MetadataTokenResponse {
    access_token: String,
    expires_in: i64,
    #[serde(default = "bearer")]
    token_type: String,
}

fn bearer() -> String {
    "Bearer".to_string()
}

pub fn ensure_no_forbidden_credential_env() -> Result<(), MetadataError> {
    for name in FORBIDDEN_CREDENTIAL_SETTINGS {
        if std::env::var_os(name).is_some() {
            return Err(MetadataError::Configuration(format!(
                "{name} is set; GCP workloads must authenticate through the instance metadata \
                 server only, with no key file, no ambient token and no proxy in the path"
            )));
        }
    }
    Ok(())
}

pub(crate) fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_never_leak_through_debug() {
        let token = AccessToken::new("ya29.super-secret", unix_timestamp() + 3_600);
        let rendered = format!("{token:?}");
        assert!(!rendered.contains("ya29"));
        assert_eq!(token.secret(), "ya29.super-secret");
    }

    #[test]
    fn expired_tokens_report_no_remaining_lifetime() {
        let token = AccessToken::new("value", unix_timestamp() - 10);
        assert!(token.remaining_seconds() <= 0);
    }

    #[test]
    fn refresh_margin_sits_below_the_metadata_server_rotation_floor() {
        // The metadata server rotates its cached token at ~300 s remaining; a
        // margin at or above that turns every call into a refetch.
        assert!(REFRESH_MARGIN_SECONDS < 300);
        assert!(MIN_USABLE_SECONDS < MIN_REUSE_SECONDS);
        assert!(MIN_REUSE_SECONDS < REFRESH_MARGIN_SECONDS);
    }

    #[test]
    fn short_tokens_are_reused_briefly_instead_of_refetched_per_call() {
        let now = unix_timestamp();
        let short = CachedToken {
            token: AccessToken::new("value", now + 240),
            fetched_at_unix: now,
        };
        assert!(short.is_servable(now));
        assert!(short.is_servable(now + MIN_REUSE_SECONDS - 1));
        assert!(!short.is_servable(now + MIN_REUSE_SECONDS));

        let nearly_dead = CachedToken {
            token: AccessToken::new("value", now + MIN_USABLE_SECONDS),
            fetched_at_unix: now,
        };
        assert!(!nearly_dead.is_servable(now));

        let healthy = CachedToken {
            token: AccessToken::new("value", now + 3_600),
            fetched_at_unix: now - 3_000,
        };
        assert!(healthy.is_servable(now));
    }

    #[test]
    fn unscoped_providers_are_refused() {
        assert!(matches!(
            MetadataTokenProvider::new(&[]),
            Err(MetadataError::Configuration(_))
        ));
    }

    #[test]
    fn scopes_are_percent_encoded_into_the_metadata_query() {
        let provider = MetadataTokenProvider::new(&[DATASTORE_SCOPE, SQL_LOGIN_SCOPE])
            .expect("scoped provider must build");
        assert!(provider.url.starts_with(METADATA_TOKEN_URL));
        assert!(provider.url.contains("%2Fauth%2Fdatastore"));
        assert!(provider.url.contains("%2C"));
    }
}
