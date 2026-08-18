//! Google-signed OIDC validation for Pub/Sub push deliveries.
//!
//! A push endpoint on Cloud Run is reachable by anything that can resolve its
//! URL, so the bearer token in the `Authorization` header is the only thing that
//! separates a genuine delivery from an attacker posting a forged execution job.
//! Three independent facts have to hold before a body is even parsed: the token
//! is signed by a key Google publishes, its audience is exactly this service, and
//! its `email` claim is exactly the service account the subscription was
//! configured to push as. Dropping any one of them turns the worker into an
//! open remote-execution endpoint — the audience alone would accept a token
//! minted for this URL by any Google identity, and the signature alone would
//! accept a token minted for a completely different service.

use jsonwebtoken::jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use std::fmt::{Display, Formatter};
use std::time::{Duration, Instant};

/// Google's OAuth2 signing keys. This is the discovery document's
/// `jwks_uri` value, pinned rather than discovered: one fewer request on the
/// hot path, and one fewer document whose contents decide which keys are
/// trusted.
const GOOGLE_JWKS_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";

/// Google mints identity tokens with the URL form and, on some legacy paths,
/// with the bare hostname. Both spellings resolve to the same signing keys, so
/// accepting both narrows nothing while rejecting the wrong one would fail
/// closed at an unpredictable moment.
const ACCEPTED_ISSUERS: &[&str] = &["https://accounts.google.com", "accounts.google.com"];

/// Google rotates its signing keys roughly daily and always publishes the
/// successor before it starts using it, so a cached set stays valid far longer
/// than this. The ceiling exists to bound how long a revoked key stays trusted.
const JWKS_MAX_AGE: Duration = Duration::from_secs(60 * 60);
/// An unknown `kid` triggers one out-of-band refresh, but no more often than
/// this. Without the floor, a flood of tokens carrying random key IDs would turn
/// every request into an outbound fetch and let an unauthenticated caller drive
/// this worker's egress.
const JWKS_MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// Clock skew between Cloud Run and Google's token minter. Deliberately small:
/// the tokens are minted seconds before delivery and live for an hour.
const CLOCK_SKEW_LEEWAY_SECONDS: u64 = 60;
const MAX_JWKS_BYTES: usize = 128 * 1024;

#[derive(Debug)]
pub enum PushAuthError {
    /// No usable `Authorization: Bearer` header.
    Missing,
    /// The token is present but is not a token this worker will ever accept.
    Rejected(String),
    /// Google's key set could not be read. Distinct from `Rejected` on purpose:
    /// this is an outage on Google's side, not an intrusion attempt, and it must
    /// not be counted as one in the alerting that watches rejections.
    KeySetUnavailable(String),
}

impl Display for PushAuthError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => formatter.write_str("no bearer token on the push delivery"),
            Self::Rejected(detail) => write!(formatter, "push token rejected: {detail}"),
            Self::KeySetUnavailable(detail) => {
                write!(formatter, "Google key set unavailable: {detail}")
            }
        }
    }
}

impl std::error::Error for PushAuthError {}

/// The subset of an identity token this worker acts on. `aud`, `iss`, `exp`,
/// `nbf` and `sub` are enforced by [`Validation`] before this ever deserializes,
/// so only the two claims the worker itself checks appear here.
#[derive(Debug, Deserialize)]
pub struct PushClaims {
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub email_verified: bool,
}

struct CachedKeys {
    keys: JwkSet,
    fetched_at: Instant,
}

/// The audience is not held here. It is supplied per call by the caller, which
/// either has it pinned from configuration or derives it from the request being
/// served (see `PushAudience` in `main.rs`); the verifier's job is only to
/// insist that the token's `aud` is exactly the value it is handed.
pub struct PushTokenVerifier {
    http: reqwest::Client,
    service_account_email: String,
    keys: tokio::sync::RwLock<Option<CachedKeys>>,
}

impl PushTokenVerifier {
    pub fn new(service_account_email: String) -> Result<Self, PushAuthError> {
        // `no_proxy` is load-bearing for the same reason it is in
        // `flow_like_gcp_data::metadata`: reqwest honours `HTTP_PROXY` by
        // default, and a proxy in front of the JWKS fetch would let whoever set
        // that variable choose which keys this worker trusts.
        let http = reqwest::Client::builder()
            .no_proxy()
            .https_only(true)
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .user_agent(concat!("flow-like/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| PushAuthError::KeySetUnavailable(error.to_string()))?;

        Ok(Self {
            http,
            service_account_email,
            keys: tokio::sync::RwLock::new(None),
        })
    }

    /// Fetch the key set once at startup so the first real delivery does not pay
    /// for it, and so a project whose egress cannot reach Google fails at boot
    /// rather than by nacking every message.
    pub async fn warm(&self) -> Result<(), PushAuthError> {
        self.refresh().await
    }

    /// `audience` is the exact `aud` the token must carry — the service URL the
    /// subscription's `oidc_token.audience` names. An empty audience is refused
    /// outright rather than matched, because `jsonwebtoken` would treat an
    /// empty expected set as "no audience check".
    pub async fn verify(
        &self,
        authorization: Option<&str>,
        audience: &str,
    ) -> Result<PushClaims, PushAuthError> {
        if audience.is_empty() {
            return Err(PushAuthError::Rejected(
                "no expected audience for this delivery".to_string(),
            ));
        }
        let token = bearer_token(authorization).ok_or(PushAuthError::Missing)?;

        // The header is read before any key lookup, which means an unparseable
        // token costs nothing beyond a base64 decode.
        let header = decode_header(token)
            .map_err(|error| PushAuthError::Rejected(format!("unreadable header: {error}")))?;
        if header.alg != Algorithm::RS256 {
            return Err(PushAuthError::Rejected(format!(
                "unsupported algorithm {:?}; Google signs identity tokens with RS256",
                header.alg
            )));
        }
        let kid = header
            .kid
            .ok_or_else(|| PushAuthError::Rejected("token carries no key ID".to_string()))?;

        let key = self.decoding_key(&kid).await?;

        let mut validation = Validation::new(Algorithm::RS256);
        // `Validation::new` already pins the algorithm list to RS256 alone; it is
        // restated because an empty or widened list here is the classic JWT
        // confusion bug, and the assignment makes the invariant greppable.
        validation.algorithms = vec![Algorithm::RS256];
        validation.set_audience(&[audience]);
        validation.set_issuer(ACCEPTED_ISSUERS);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        validation.leeway = CLOCK_SKEW_LEEWAY_SECONDS;
        // Presence, not just correctness. Without this a token that simply omits
        // `aud` would sail past the audience check instead of failing it.
        // `jsonwebtoken` honours only the five registered claims here, so `iat`
        // is deliberately absent rather than silently ignored.
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);

        let data = decode::<PushClaims>(token, &key, &validation)
            .map_err(|error| PushAuthError::Rejected(error.to_string()))?;

        // The audience proves the token was minted *for* this service; the email
        // proves it was minted *by* the pusher. Only the pair is meaningful:
        // any principal that can mint an identity token can choose this
        // audience, so without the email check any Google identity in the world
        // could post an execution job here.
        if !data.claims.email_verified {
            return Err(PushAuthError::Rejected(
                "token's email claim is not verified".to_string(),
            ));
        }
        if !data
            .claims
            .email
            .eq_ignore_ascii_case(&self.service_account_email)
        {
            // The presented address is deliberately not logged from here: it is
            // attacker-controlled text on this path, and the caller logs the
            // rejection with the expected value only.
            return Err(PushAuthError::Rejected(
                "token was not minted by the configured push service account".to_string(),
            ));
        }

        Ok(data.claims)
    }

    async fn decoding_key(&self, kid: &str) -> Result<DecodingKey, PushAuthError> {
        if let Some(key) = self.cached_key(kid).await? {
            return Ok(key);
        }

        // A `kid` that is not in the cache means either a rotation this process
        // has not seen or a forged header. One refresh distinguishes them; the
        // interval floor keeps the second case from being free to the attacker.
        let stale_enough = {
            let cached = self.keys.read().await;
            cached
                .as_ref()
                .is_none_or(|cached| cached.fetched_at.elapsed() >= JWKS_MIN_REFRESH_INTERVAL)
        };
        if !stale_enough {
            return Err(PushAuthError::Rejected(format!(
                "key ID {kid} is not in Google's published key set"
            )));
        }

        self.refresh().await?;
        self.cached_key(kid).await?.ok_or_else(|| {
            PushAuthError::Rejected(format!("key ID {kid} is not in Google's published key set"))
        })
    }

    async fn cached_key(&self, kid: &str) -> Result<Option<DecodingKey>, PushAuthError> {
        let cached = self.keys.read().await;
        let Some(cached) = cached.as_ref() else {
            return Ok(None);
        };
        if cached.fetched_at.elapsed() > JWKS_MAX_AGE {
            return Ok(None);
        }
        let Some(jwk) = cached.keys.find(kid) else {
            return Ok(None);
        };

        // Google publishes RSA keys only. Pinning the key type here means a
        // future entry of another family cannot silently change which algorithm
        // family the signature is checked against.
        if !matches!(jwk.algorithm, AlgorithmParameters::RSA(_)) {
            return Err(PushAuthError::Rejected(format!(
                "key ID {kid} is not an RSA key"
            )));
        }
        if !matches!(jwk.common.key_algorithm, None | Some(KeyAlgorithm::RS256)) {
            return Err(PushAuthError::Rejected(format!(
                "key ID {kid} is not published for RS256"
            )));
        }

        DecodingKey::from_jwk(jwk)
            .map(Some)
            .map_err(|error| PushAuthError::Rejected(format!("unusable signing key: {error}")))
    }

    async fn refresh(&self) -> Result<(), PushAuthError> {
        let response = self
            .http
            .get(GOOGLE_JWKS_URL)
            .send()
            .await
            .map_err(|error| PushAuthError::KeySetUnavailable(error.to_string()))?;
        let status = response.status();
        let body = response
            .bytes()
            .await
            .map_err(|error| PushAuthError::KeySetUnavailable(error.to_string()))?;
        if !status.is_success() {
            return Err(PushAuthError::KeySetUnavailable(format!(
                "HTTP {}",
                status.as_u16()
            )));
        }
        if body.len() > MAX_JWKS_BYTES {
            return Err(PushAuthError::KeySetUnavailable(format!(
                "key set of {} bytes exceeds the {MAX_JWKS_BYTES} byte ceiling",
                body.len()
            )));
        }

        let keys: JwkSet = serde_json::from_slice(&body)
            .map_err(|error| PushAuthError::KeySetUnavailable(error.to_string()))?;
        if keys.keys.is_empty() {
            return Err(PushAuthError::KeySetUnavailable(
                "Google published an empty key set".to_string(),
            ));
        }

        let key_count = keys.keys.len();
        let mut cached = self.keys.write().await;
        *cached = Some(CachedKeys {
            keys,
            fetched_at: Instant::now(),
        });
        tracing::info!(key_count, "refreshed Google's OIDC signing keys");
        Ok(())
    }
}

/// Extract the token from an `Authorization` header value.
///
/// The scheme comparison is case-insensitive because RFC 7235 says the scheme
/// token is, and a case-sensitive check here would be a silent interoperability
/// trap rather than a control.
fn bearer_token(authorization: Option<&str>) -> Option<&str> {
    let value = authorization?.trim();
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() { None } else { Some(token) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_bearer_tokens() {
        assert_eq!(
            bearer_token(Some("Bearer abc.def.ghi")),
            Some("abc.def.ghi")
        );
        assert_eq!(
            bearer_token(Some("bearer  abc.def.ghi ")),
            Some("abc.def.ghi")
        );
        assert_eq!(bearer_token(Some("Basic abc")), None);
        assert_eq!(bearer_token(Some("Bearer   ")), None);
        assert_eq!(bearer_token(Some("abc.def.ghi")), None);
        assert_eq!(bearer_token(None), None);
    }

    #[test]
    fn issuer_allowlist_holds_both_google_spellings() {
        assert!(ACCEPTED_ISSUERS.contains(&"https://accounts.google.com"));
        assert!(ACCEPTED_ISSUERS.contains(&"accounts.google.com"));
        assert_eq!(ACCEPTED_ISSUERS.len(), 2);
    }
}
