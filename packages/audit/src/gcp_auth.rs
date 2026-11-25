//! OAuth tokens shared by audit signing and cloud credentials.

use anyhow::{Result, anyhow};
use serde::Deserialize;
use std::sync::OnceLock;

/// Metadata-server endpoint that mints an OAuth2 token for the service account
/// bound to the running Cloud Run revision, GCE instance or GKE pod.
const METADATA_TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
const DEFAULT_METADATA_HOST: &str = "metadata.google.internal";
const DEFAULT_METADATA_IP: &str = "169.254.169.254";
const METADATA_FLAVOR_HEADER: &str = "Metadata-Flavor";
const METADATA_FLAVOR_VALUE: &str = "Google";
/// A cached metadata token is only reused while this much life remains. The
/// downscoped token handed to a client inherits the base token's remaining
/// lifetime, so serving a nearly-dead base token would mint client credentials
/// that expire minutes after they are issued.
const METADATA_TOKEN_MIN_LIFETIME_SECONDS: i64 = 5 * 60;
/// Ceiling on cache residency. Metadata tokens live ~1h and the metadata server
/// refreshes its own copy near the end of that window; re-asking every ten
/// minutes keeps this process close to a full-lifetime token without turning
/// every scoped-credential request into a metadata round trip.
const METADATA_TOKEN_CACHE_TTL_SECONDS: u64 = 10 * 60;
const METADATA_CONNECT_TIMEOUT_SECONDS: u64 = 3;
const METADATA_REQUEST_TIMEOUT_SECONDS: u64 = 10;

#[derive(Clone)]
struct CachedMetadataToken {
    token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
}

static METADATA_TOKENS: OnceLock<moka::sync::Cache<String, CachedMetadataToken>> = OnceLock::new();

/// Read the master service account key from the environment, treating a
/// set-but-empty variable as absent.
///
/// `std::env::var` returns `Ok("")` for `GOOGLE_APPLICATION_CREDENTIALS_JSON=`,
/// which is the shape an optional Terraform variable renders to. Passing that
/// through as `Some("")` made "no key configured" and "key configured as blank"
/// indistinguishable downstream, where the difference decides whether the
/// process runs on the configured principal or the ambient one.
pub fn service_account_key_from_env() -> Option<String> {
    std::env::var("GOOGLE_APPLICATION_CREDENTIALS_JSON")
        .ok()
        .filter(|key| !key.trim().is_empty())
}

/// Metadata authorities to try, in order: hostname first, link-local IP second.
///
/// Mirrors object_store's `InstanceCredentialProvider` — including the
/// `GCE_METADATA_*` overrides — so the token this module mints and the token
/// object_store mints inside the same process come from the same server. A
/// deployment that redirected one and not the other would quietly run on two
/// different identities. The IP fallback exists because metadata access must
/// survive a pod with no working DNS; the hostname is unresolvable long before
/// the address is unreachable.
fn metadata_authorities() -> [String; 2] {
    let non_empty = |value: String| {
        let value = value.trim().to_string();
        (!value.is_empty()).then_some(value)
    };

    let host = std::env::var("GCE_METADATA_HOST")
        .or_else(|_| std::env::var("GCE_METADATA_ROOT"))
        .ok()
        .and_then(non_empty)
        .unwrap_or_else(|| DEFAULT_METADATA_HOST.to_string());
    let ip = std::env::var("GCE_METADATA_IP")
        .ok()
        .and_then(non_empty)
        .unwrap_or_else(|| DEFAULT_METADATA_IP.to_string());

    [host, ip]
}

/// Fetch an OAuth2 access token for the workload's own service account from the
/// GCE/Cloud Run metadata server.
///
/// This is the keyless path. Under Workload Identity no service account key
/// JSON exists to sign a JWT assertion with, so the metadata server is the only
/// source of a base token — and without it every scoped-credential request on
/// Cloud Run failed outright.
///
/// The returned token never leaves this process: it is the *subject* token of
/// the STS exchange, and only the downscoped result reaches a client. That is
/// why no scope narrowing is requested here. The metadata server hands back the
/// workload's full token (`cloud-platform` on Cloud Run), which is what the
/// Credential Access Boundary exchange expects as its subject; the narrowing
/// that matters is expressed by the boundary itself, which GCP enforces
/// server-side on every object operation.
pub async fn fetch_metadata_token() -> Result<String> {
    let [host, ip] = metadata_authorities();

    let cache = METADATA_TOKENS.get_or_init(|| {
        moka::sync::Cache::builder()
            .max_capacity(4)
            .time_to_live(std::time::Duration::from_secs(
                METADATA_TOKEN_CACHE_TTL_SECONDS,
            ))
            .build()
    });

    // The moka TTL is a ceiling, not the truth — the metadata server decides the
    // real lifetime. Re-checking the advertised expiry keeps a nearly-dead token
    // out of the STS exchange. A freshly fetched short token is still used: the
    // metadata server refreshes on its own schedule and rejecting one here would
    // turn a narrow window into a hard outage.
    if let Some(cached) = cache.get(&host)
        && cached.expires_at - chrono::Utc::now()
            > chrono::Duration::seconds(METADATA_TOKEN_MIN_LIFETIME_SECONDS)
    {
        return Ok(cached.token);
    }

    // `no_proxy` is load-bearing, not tidiness: an ambient HTTP_PROXY would
    // route this request — and the service account token in its response —
    // through a third party. The metadata server is link-local and unroutable,
    // so there is never a legitimate proxy for it. HTTPS is likewise absent by
    // design; the endpoint is plain HTTP on an address only the hypervisor can
    // answer for, which is why `https_only` is not set here as it is elsewhere.
    //
    // Redirects are refused for the same reason: the only correct responder for
    // this request is the link-local address itself, so a 3xx pointing anywhere
    // else is either a misconfiguration or an attempt to feed this process a
    // token from a server it does not trust.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(
            METADATA_CONNECT_TIMEOUT_SECONDS,
        ))
        .timeout(std::time::Duration::from_secs(
            METADATA_REQUEST_TIMEOUT_SECONDS,
        ))
        .build()
        .map_err(|error| anyhow!("failed to construct GCP metadata client: {error}"))?;

    #[derive(Deserialize)]
    struct MetadataTokenResponse {
        access_token: String,
        expires_in: i64,
    }

    let mut last_error: Option<anyhow::Error> = None;
    for authority in [host.as_str(), ip.as_str()] {
        // `Metadata-Flavor: Google` is the SSRF guard, not a formality: the
        // metadata server rejects any request that omits it, so a confused
        // proxy or a redirect-following client cannot be steered into reading
        // the token on an attacker's behalf. Requiring the header back on the
        // response closes the other direction — a hijacked
        // `metadata.google.internal` cannot feed this process a token it
        // controls, because a plain HTTP server does not echo the header.
        let response = client
            .get(format!("http://{}{}", authority, METADATA_TOKEN_PATH))
            .header(METADATA_FLAVOR_HEADER, METADATA_FLAVOR_VALUE)
            .send()
            .await;

        let response = match response {
            Ok(response) => response,
            Err(error) => {
                last_error = Some(anyhow!(
                    "GCP metadata token request to {authority} failed: {error}"
                ));
                continue;
            }
        };

        let status = response.status();
        let flavor_echoed = response
            .headers()
            .get(METADATA_FLAVOR_HEADER)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case(METADATA_FLAVOR_VALUE));

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            last_error = Some(anyhow!(
                "GCP metadata server at {authority} returned HTTP {status}: {body}"
            ));
            continue;
        }

        // Warned about, never enforced. The echo would be a useful signal that
        // the responder really is the metadata server, but Google documents the
        // header only as a *request* requirement: `google-auth` checks it in its
        // `ping()` environment probe and in neither `get()` nor
        // `get_service_account_token()`, object_store's own
        // `InstanceCredentialProvider` never checks it, and GKE has been
        // reported serving token responses without it. Requiring it would trade
        // a total credential outage for a narrow anti-spoofing check that the
        // platform does not promise to support.
        if !flavor_echoed {
            tracing::warn!(
                authority,
                "GCP metadata token response omitted {METADATA_FLAVOR_HEADER}: {METADATA_FLAVOR_VALUE}"
            );
        }

        // A malformed body falls through to the next authority rather than
        // aborting: the hostname answering with junk is exactly the broken-DNS
        // case the link-local address exists to survive.
        let token: MetadataTokenResponse = match response.json().await {
            Ok(token) => token,
            Err(error) => {
                last_error = Some(anyhow!(
                    "invalid GCP metadata token response from {authority}: {error}"
                ));
                continue;
            }
        };

        // Clamped because `chrono::Duration::seconds` panics on an absurd input
        // and this runs inside a request handler. Clamping low degrades to
        // "never serve from cache", which is correct rather than merely safe.
        let lifetime = chrono::Duration::seconds(token.expires_in.clamp(0, 24 * 60 * 60));
        cache.insert(
            host.clone(),
            CachedMetadataToken {
                token: token.access_token.clone(),
                expires_at: chrono::Utc::now() + lifetime,
            },
        );

        return Ok(token.access_token);
    }

    Err(last_error.unwrap_or_else(|| {
        anyhow!(
            "no GCP credentials available: the metadata server was unreachable at {host} and {ip}, \
             and no service account key is configured. On Cloud Run/GKE bind a runtime service \
             account to the workload; off-GCP set GOOGLE_APPLICATION_CREDENTIALS_JSON."
        )
    }))
}

/// Standalone version for tests without State
pub async fn generate_access_token_standalone(
    service_account_key: &str,
    scope: &str,
) -> Result<String> {
    let jwt = create_jwt_assertion(service_account_key, scope)?;
    let token_uri = get_token_uri(service_account_key)?;

    let client = reqwest::Client::new();
    let response = client
        .post(&token_uri)
        .form(&[
            ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
            ("assertion", &jwt),
        ])
        .send()
        .await
        .map_err(|e| anyhow!("Failed to request access token: {}", e))?;

    parse_token_response(response).await
}

fn get_token_uri(service_account_key: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct ServiceAccountKey {
        token_uri: Option<String>,
    }

    let sa_key: ServiceAccountKey = serde_json::from_str(service_account_key)
        .map_err(|e| anyhow!("Failed to parse service account key: {}", e))?;

    Ok(sa_key
        .token_uri
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string()))
}

fn create_jwt_assertion(service_account_key: &str, scope: &str) -> Result<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

    #[derive(Deserialize)]
    struct ServiceAccountKey {
        client_email: String,
        private_key: String,
        token_uri: Option<String>,
    }

    let sa_key: ServiceAccountKey = serde_json::from_str(service_account_key)
        .map_err(|e| anyhow!("Failed to parse service account key: {}", e))?;

    let token_uri = sa_key
        .token_uri
        .unwrap_or_else(|| "https://oauth2.googleapis.com/token".to_string());

    let now = chrono::Utc::now().timestamp();
    let exp = now + 3600;

    let header = serde_json::json!({
        "alg": "RS256",
        "typ": "JWT"
    });

    let claims = serde_json::json!({
        "iss": sa_key.client_email,
        "sub": sa_key.client_email,
        "aud": token_uri,
        "iat": now,
        "exp": exp,
        "scope": scope
    });

    let header_b64 = URL_SAFE_NO_PAD.encode(header.to_string().as_bytes());
    let claims_b64 = URL_SAFE_NO_PAD.encode(claims.to_string().as_bytes());
    let message = format!("{}.{}", header_b64, claims_b64);

    let signature = sign_rs256(&sa_key.private_key, message.as_bytes())?;
    let signature_b64 = URL_SAFE_NO_PAD.encode(&signature);

    Ok(format!("{}.{}", message, signature_b64))
}

async fn parse_token_response(response: reqwest::Response) -> Result<String> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!("Failed to get access token: {} - {}", status, body));
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        access_token: String,
    }

    let token_response: TokenResponse = response
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse token response: {}", e))?;

    Ok(token_response.access_token)
}

/// Sign data with RS256 (RSA-SHA256)
fn sign_rs256(private_key_pem: &str, data: &[u8]) -> Result<Vec<u8>> {
    use rsa::{
        RsaPrivateKey, pkcs1v15::SigningKey, pkcs8::DecodePrivateKey, signature::SignatureEncoding,
        signature::Signer,
    };

    let private_key = RsaPrivateKey::from_pkcs8_pem(private_key_pem)
        .map_err(|e| anyhow!("Failed to parse private key: {}", e))?;

    let signing_key = SigningKey::<sha2::Sha256>::new(private_key);
    let signature = signing_key.sign(data);

    Ok(signature.to_bytes().to_vec())
}
