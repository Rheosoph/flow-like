use super::{
    Connection, client,
    microsoft::{MicrosoftError, retry_after},
    now,
};
use crate::{error::ApiError, state::AppState};
use axum::http::HeaderMap;
use flow_like_types::tokio::{self, sync::Mutex as AsyncMutex};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::Jwk};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::Value;
use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};

const KEYS_URL: &str = "https://login.botframework.com/v1/.well-known/keys";
pub(super) const BOT_SCOPE: &str = "https://api.botframework.com/.default";
pub(super) const GRAPH_SCOPE: &str = "https://graph.microsoft.com/.default";
/// Missing service principal, unknown application, or missing consent in the tenant.
const CONSENT_CODES: [u64; 4] = [7_000_229, 700_016, 650_052, 500_011];
/// Invalid or expired client secret.
const SECRET_CODES: [u64; 2] = [7_000_215, 7_000_222];
const KEYS_TTL_MS: i64 = 24 * 3_600_000;
const KEYS_MAX_STALE_MS: i64 = 7 * 24 * 3_600_000;
const KEYS_REFETCH_INTERVAL_MS: i64 = 5 * 60_000;
const TOKEN_MARGIN_SECS: i64 = 300;
const RENEWAL_WINDOW_MS: i64 = 14 * 86_400_000;
const RENEWAL_BACKOFF_SECS: u64 = 600;

#[derive(Clone)]
struct KeySet {
    keys: Arc<Value>,
    fetched_at: i64,
    attempted_at: i64,
}

static CONNECTOR_KEYS: LazyLock<Mutex<Option<KeySet>>> = LazyLock::new(|| Mutex::new(None));
static KEY_REFRESH: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));
static BOT_TOKENS: LazyLock<TokenCache> = LazyLock::new(token_cache);
static GRAPH_TOKENS: LazyLock<TokenCache> = LazyLock::new(token_cache);
/// Connections whose rotation failed while their secret was still valid.
static RENEWAL_BACKOFF: LazyLock<moka::sync::Cache<String, ()>> = LazyLock::new(|| {
    moka::sync::Cache::builder()
        .max_capacity(10_000)
        .time_to_live(Duration::from_secs(RENEWAL_BACKOFF_SECS))
        .build()
});

type TokenCache = moka::sync::Cache<String, (String, i64)>;

fn token_cache() -> TokenCache {
    moka::sync::Cache::builder()
        .max_capacity(10_000)
        .time_to_live(Duration::from_secs(24 * 3600))
        .build()
}

#[derive(Clone, Deserialize)]
struct ConnectorClaims {
    #[serde(rename = "serviceurl")]
    service_url: String,
}

pub(super) fn service_url(raw: &str) -> Result<reqwest::Url, ApiError> {
    let url = reqwest::Url::parse(raw).map_err(|_| ApiError::UNAUTHORIZED)?;
    let host = url.host_str().ok_or(ApiError::UNAUTHORIZED)?;
    let allowed = host == "smba.trafficmanager.net"
        || host.ends_with(".botapi.skype.com")
        || host.ends_with(".teams.microsoft.com");
    if url.scheme() != "https"
        || !allowed
        || url.port().is_some_and(|port| port != 443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::UNAUTHORIZED);
    }
    Ok(url)
}

pub(super) async fn verify(
    connection: &Connection,
    headers: &HeaderMap,
    activity: &Value,
) -> Result<(), ApiError> {
    let token = bearer_token(headers)?;
    let header = decode_header(token).map_err(|_| ApiError::UNAUTHORIZED)?;
    if header.alg != Algorithm::RS256 {
        return Err(ApiError::UNAUTHORIZED);
    }
    let kid = header.kid.ok_or(ApiError::UNAUTHORIZED)?;
    let keys = connector_keys(&kid).await?;
    verify_token(connection, token, &keys, activity)
}

fn has_key(keys: &Value, kid: &str) -> bool {
    keys["keys"]
        .as_array()
        .is_some_and(|keys| keys.iter().any(|key| key["kid"].as_str() == Some(kid)))
}

/// Keys are refetched daily, or for an unknown `kid` at most every few minutes.
fn needs_refresh(set: Option<&KeySet>, kid: &str, now: i64) -> bool {
    let Some(set) = set else {
        return true;
    };
    now - set.attempted_at >= KEYS_REFETCH_INTERVAL_MS
        && (now - set.fetched_at >= KEYS_TTL_MS || !has_key(&set.keys, kid))
}

async fn connector_keys(kid: &str) -> Result<Arc<Value>, ApiError> {
    let cached = CONNECTOR_KEYS.lock().clone();
    if let Some(set) = cached.filter(|set| !needs_refresh(Some(set), kid, now())) {
        return Ok(set.keys);
    }
    let _refresh = KEY_REFRESH.lock().await;
    let cached = CONNECTOR_KEYS.lock().clone();
    if let Some(set) = cached
        .as_ref()
        .filter(|set| !needs_refresh(Some(set), kid, now()))
    {
        return Ok(set.keys.clone());
    }
    match fetch_connector_keys().await {
        Ok(keys) => {
            let keys = Arc::new(keys);
            *CONNECTOR_KEYS.lock() = Some(KeySet {
                keys: keys.clone(),
                fetched_at: now(),
                attempted_at: now(),
            });
            Ok(keys)
        }
        Err(error) => match cached.filter(|set| now() - set.fetched_at < KEYS_MAX_STALE_MS) {
            Some(mut set) => {
                tracing::warn!(%error, "Using cached Microsoft signing keys after a failed refresh");
                set.attempted_at = now();
                *CONNECTOR_KEYS.lock() = Some(set.clone());
                Ok(set.keys)
            }
            None => Err(error),
        },
    }
}

async fn fetch_connector_keys() -> Result<Value, ApiError> {
    let response = client()?
        .get(KEYS_URL)
        .send()
        .await
        .map_err(|_| ApiError::bad_gateway("Cannot retrieve Microsoft signing keys"))?;
    if !response.status().is_success() {
        return Err(ApiError::bad_gateway(format!(
            "Microsoft signing keys returned HTTP {}",
            response.status()
        )));
    }
    let keys: Value = response
        .json()
        .await
        .map_err(|_| ApiError::bad_gateway("Invalid Microsoft signing keys"))?;
    if !keys["keys"].is_array() {
        return Err(ApiError::bad_gateway("Invalid Microsoft signing keys"));
    }
    Ok(keys)
}

fn bearer_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    let token = crate::middleware::jwt::viewer_authorization(headers)
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(ApiError::UNAUTHORIZED)?;
    if token.len() > 16384 {
        return Err(ApiError::UNAUTHORIZED);
    }
    Ok(token)
}

fn verify_token(
    connection: &Connection,
    token: &str,
    keys: &Value,
    activity: &Value,
) -> Result<(), ApiError> {
    let header = decode_header(token).map_err(|_| ApiError::UNAUTHORIZED)?;
    if header.alg != Algorithm::RS256 {
        return Err(ApiError::UNAUTHORIZED);
    }
    let kid = header.kid.ok_or(ApiError::UNAUTHORIZED)?;
    let key = keys["keys"]
        .as_array()
        .and_then(|keys| keys.iter().find(|key| key["kid"].as_str() == Some(&kid)))
        .ok_or(ApiError::UNAUTHORIZED)?;
    if !endorsed_for_teams(key) {
        return Err(ApiError::UNAUTHORIZED);
    }
    let jwk: Jwk = serde_json::from_value(key.clone()).map_err(|_| ApiError::UNAUTHORIZED)?;
    let key = DecodingKey::from_jwk(&jwk).map_err(|_| ApiError::UNAUTHORIZED)?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[
        connection.client_id.clone(),
        format!("api://{}", connection.client_id),
        format!("api://botid-{}", connection.client_id),
    ]);
    validation.set_issuer(&["https://api.botframework.com"]);
    validation.set_required_spec_claims(&["exp", "nbf", "iss", "aud"]);
    validation.validate_nbf = true;
    validation.leeway = 60;
    let claims = decode::<ConnectorClaims>(token, &key, &validation)
        .map_err(|_| ApiError::UNAUTHORIZED)?
        .claims;
    if activity["channelId"] != "msteams"
        || activity["serviceUrl"]
            .as_str()
            .map(|url| url.trim_end_matches('/'))
            != Some(claims.service_url.trim_end_matches('/'))
        || activity
            .pointer("/channelData/tenant/id")
            .and_then(Value::as_str)
            != Some(&connection.customer_tenant_id)
    {
        return Err(ApiError::FORBIDDEN);
    }
    service_url(&claims.service_url)?;
    Ok(())
}

/// Microsoft also publishes unrestricted keys without an endorsements field. A present
/// list, including an empty one, restricts the key to the channels it names.
fn endorsed_for_teams(key: &Value) -> bool {
    match key.get("endorsements") {
        None | Some(Value::Null) => true,
        Some(values) => values
            .as_array()
            .is_some_and(|values| values.iter().any(|value| value == "msteams")),
    }
}

async fn token_request(
    tenant: &str,
    id: &str,
    secret: &str,
    scope: &str,
) -> Result<reqwest::Response, ApiError> {
    let tenant = super::guid(tenant)?;
    client()?
        .post(format!(
            "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token"
        ))
        .form(&[
            ("grant_type", "client_credentials"),
            ("client_id", id),
            ("client_secret", secret),
            ("scope", scope),
        ])
        .send()
        .await
        .map_err(|_| ApiError::bad_gateway("Microsoft authentication could not be reached"))
}

async fn issued_token(response: reqwest::Response) -> Result<(String, i64), ApiError> {
    let result: Value = response
        .json()
        .await
        .map_err(|_| ApiError::bad_gateway("Invalid Microsoft token response"))?;
    let token = result["access_token"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::bad_gateway("Microsoft did not return an access token"))?;
    Ok((
        token.into(),
        token_expiry(now(), result["expires_in"].as_i64()),
    ))
}

pub(super) async fn client_token(
    tenant: &str,
    id: &str,
    secret: &str,
    scope: &str,
) -> Result<(String, i64), ApiError> {
    let response = token_request(tenant, id, secret, scope).await?;
    let status = response.status();
    if !status.is_success() {
        return Err(if super::transient(status) {
            ApiError::bad_gateway(format!(
                "Microsoft authentication returned HTTP {status}. Retry shortly."
            ))
        } else {
            ApiError::bad_request(
                "Microsoft rejected the bot credentials. Check the home tenant, application ID, and secret value.",
            )
        });
    }
    issued_token(response).await
}

/// The AADSTS numbers of a token endpoint error, from `error_codes` and the description.
fn aadsts_codes(body: &Value) -> Vec<u64> {
    let mut codes: Vec<u64> = body["error_codes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect();
    if let Some(code) = body["error_description"]
        .as_str()
        .and_then(|description| description.strip_prefix("AADSTS"))
        .map(|rest| {
            rest.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|digits| digits.parse().ok())
        && !codes.contains(&code)
    {
        codes.push(code);
    }
    codes
}

/// Maps a failed Graph token request. Only the status and AADSTS number are surfaced,
/// because Microsoft's error bodies can echo identifiers that do not belong in messages.
/// `invalid_client` means consent is missing only when the customer tenant is not the
/// application's home tenant and the secret itself was not rejected.
fn graph_token_rejection(
    status: reqwest::StatusCode,
    retry_after: Option<u64>,
    body: &[u8],
    cross_tenant: bool,
) -> MicrosoftError {
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return MicrosoftError::Throttled(retry_after);
    }
    if status.is_server_error() {
        return MicrosoftError::Unavailable(format!(
            "Microsoft authentication returned HTTP {status}. Retry shortly."
        ));
    }
    let body: Value = serde_json::from_slice(body).unwrap_or_default();
    let codes = aadsts_codes(&body);
    if codes.iter().any(|code| CONSENT_CODES.contains(code))
        || (body["error"] == "invalid_client"
            && cross_tenant
            && !codes.iter().any(|code| SECRET_CODES.contains(code)))
    {
        return MicrosoftError::ConsentRequired;
    }
    MicrosoftError::Invalid(match codes.first() {
        Some(code) => format!(
            "Microsoft refused a Microsoft Graph token for the customer tenant (AADSTS{code}). Check the bot's application ID and secret."
        ),
        None => format!(
            "Microsoft refused a Microsoft Graph token for the customer tenant with HTTP {status}. Check the bot's application ID and secret."
        ),
    })
}

/// An app-only Microsoft Graph token in the customer tenant, for RSC-granted reads.
pub(super) async fn graph_token(
    state: &AppState,
    connection: &Connection,
) -> Result<String, MicrosoftError> {
    let connection = fresh_connection(state, connection).await?;
    let key = graph_token_key(&connection);
    if let Some((token, expires)) = GRAPH_TOKENS.get(&key)
        && expires > now()
    {
        return Ok(token);
    }
    let response = token_request(
        &connection.customer_tenant_id,
        &connection.client_id,
        &connection.secret,
        GRAPH_SCOPE,
    )
    .await?;
    let status = response.status();
    if !status.is_success() {
        let retry_after = retry_after(response.headers());
        let body = response.bytes().await.unwrap_or_default();
        return Err(graph_token_rejection(
            status,
            retry_after,
            &body,
            connection.customer_tenant_id != connection.home_tenant_id,
        ));
    }
    let (token, expires) = issued_token(response).await?;
    GRAPH_TOKENS.insert(key, (token.clone(), expires));
    Ok(token)
}

fn graph_token_key(connection: &Connection) -> String {
    format!(
        "{}:{}",
        token_key(connection),
        connection.customer_tenant_id
    )
}

/// Graph rejected a cached token: the next request acquires a fresh one.
pub(super) fn forget_graph_token(connection: &Connection) {
    GRAPH_TOKENS.invalidate(&graph_token_key(connection));
}

/// Tokens are reused until a margin before Microsoft's stated expiry.
fn token_expiry(now: i64, expires_in: Option<i64>) -> i64 {
    let lifetime = expires_in.unwrap_or(300).clamp(60, 86_400);
    now + (lifetime - TOKEN_MARGIN_SECS.min(lifetime / 2)) * 1000
}

fn secret_expiry(connection: &Connection) -> Option<i64> {
    connection
        .secret_expires_at
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|expiry| expiry.timestamp_millis())
}

fn secret_still_valid(connection: &Connection, now: i64) -> bool {
    secret_expiry(connection).is_some_and(|expiry| expiry > now)
}

/// After a failed rotation, a still-valid secret is not renewed again until the backoff ends.
fn needs_renewal(connection: &Connection, now: i64) -> bool {
    connection.mode == super::AuthMode::FlowLikeManaged
        && secret_expiry(connection).is_some_and(|expiry| {
            expiry < now + RENEWAL_WINDOW_MS
                && (expiry <= now || !RENEWAL_BACKOFF.contains_key(&connection.id))
        })
}

/// A failed renewal keeps the caller's credential until it actually expires.
fn renewed_or_current(
    connection: &Connection,
    renewed: Result<Connection, ApiError>,
    now: i64,
) -> Result<Connection, ApiError> {
    match renewed {
        Err(error) if secret_still_valid(connection, now) => {
            tracing::warn!(
                connection_id = %connection.id,
                %error,
                "Teams credential renewal failed; using the still-valid active credential"
            );
            Ok(connection.clone())
        }
        renewed => renewed,
    }
}

/// Runs `work` on its own task, so a caller that times out or is dropped cannot abandon it
/// halfway through.
async fn detached<T: Send + 'static>(
    operation: &'static str,
    work: impl Future<Output = Result<T, ApiError>> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::spawn(work).await.map_err(|error| {
        tracing::error!(operation, %error, "Teams background task did not complete");
        ApiError::internal(format!("{operation} did not complete. Retry shortly."))
    })?
}

/// The connection whose credentials to use. A managed bot's secret is renewed when it
/// expires within two weeks.
pub(super) async fn fresh_connection(
    state: &AppState,
    connection: &Connection,
) -> Result<Connection, ApiError> {
    if !needs_renewal(connection, now()) {
        return Ok(connection.clone());
    }
    let (owned_state, owned) = (state.clone(), connection.clone());
    let renewed = detached("Teams bot credential renewal", async move {
        renew(&owned_state, &owned).await
    })
    .await;
    renewed_or_current(connection, renewed, now())
}

/// Rotates the secret under the connection's operation lease. While another request holds
/// the lease, or wins the race for it, the stored credential is used if still valid.
async fn renew(state: &AppState, connection: &Connection) -> Result<Connection, ApiError> {
    let (mut current, mut revision) = store_connection(state, &connection.id).await?;
    if current.operation_until > now() {
        if secret_still_valid(&current, now()) {
            return Ok(current);
        }
        return Err(ApiError::service_unavailable(
            "Bot credentials are being renewed. Retry shortly.",
        ));
    }
    // Recheck after loading: another API instance may already have rotated.
    if current.secret == connection.secret {
        current.operation_until = now() + 300_000;
        if let Err(error) = super::store::save_connection(state, &current, revision).await {
            return stored_after_conflict(state, &current.id, error).await;
        }
        revision += 1;
        let result = super::provision::rotate(state, &mut current, &mut revision).await;
        current.operation_until = 0;
        let released = super::store::save_connection(state, &current, revision).await;
        if let Err(error) = result {
            if !secret_still_valid(&current, now()) {
                return Err(error);
            }
            RENEWAL_BACKOFF.insert(current.id.clone(), ());
            tracing::warn!(
                connection_id = %current.id,
                %error,
                backoff_secs = RENEWAL_BACKOFF_SECS,
                "Teams credential rotation failed; retaining the valid active credential until the backoff ends"
            );
        }
        if let Err(error) = released {
            return stored_after_conflict(state, &current.id, error).await;
        }
    }
    Ok(current)
}

/// Another request changed the connection first: its stored credential is used while valid.
async fn stored_after_conflict(
    state: &AppState,
    id: &str,
    error: ApiError,
) -> Result<Connection, ApiError> {
    if error.status() != reqwest::StatusCode::CONFLICT {
        return Err(error);
    }
    let (stored, _) = store_connection(state, id).await?;
    if secret_still_valid(&stored, now()) {
        Ok(stored)
    } else {
        Err(error)
    }
}

pub(super) async fn bot_token(
    state: &AppState,
    connection: &Connection,
) -> Result<String, ApiError> {
    let connection = fresh_connection(state, connection).await?;
    let key = token_key(&connection);
    if let Some((token, expires)) = BOT_TOKENS.get(&key)
        && expires > now()
    {
        return Ok(token);
    }
    let (token, expires) = client_token(
        &connection.home_tenant_id,
        &connection.client_id,
        &connection.secret,
        BOT_SCOPE,
    )
    .await?;
    BOT_TOKENS.insert(key, (token.clone(), expires));
    Ok(token)
}

fn token_key(connection: &Connection) -> String {
    format!(
        "{}:{}:{}",
        connection.id,
        connection.client_id,
        blake3::hash(connection.secret.as_bytes()).to_hex()
    )
}

/// Teams rejected a cached token: the next request acquires a fresh one.
pub(super) fn forget_bot_token(connection: &Connection) {
    BOT_TOKENS.invalidate(&token_key(connection));
}

async fn store_connection(state: &AppState, id: &str) -> Result<(Connection, i32), ApiError> {
    super::store::connection(state, id)
        .await?
        .ok_or(ApiError::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> (Connection, Value, Value, Value) {
        let connection = serde_json::from_value(json!({
            "id":"test", "app_id":"app", "event_id":"event", "mode":"customer_teams",
            "name":"Test", "description":"", "customer_tenant_id":"tenant", "home_tenant_id":"home",
            "client_id":"bot-id", "secret":"test-only", "status":"ready", "allowed_responders":[]
        }))
        .unwrap();
        let claims = json!({"iss":"https://api.botframework.com","aud":"bot-id", "exp":now()/1000+300,
            "nbf":now()/1000-60,"serviceurl":"https://smba.trafficmanager.net/emea/"});
        let activity = json!({"channelId":"msteams","channelData":{"tenant":{"id":"tenant"}},"serviceUrl":"https://smba.trafficmanager.net/emea/"});
        let keys = serde_json::from_str(include_str!("testdata/connector-test-jwks.json")).unwrap();
        (connection, claims, activity, keys)
    }

    fn sign(claims: &Value) -> String {
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(include_bytes!(
            "testdata/connector-test-key.pem"
        ))
        .unwrap();
        let mut header = jsonwebtoken::Header::new(Algorithm::RS256);
        header.kid = Some("test-key".into());
        jsonwebtoken::encode(&header, claims, &key).unwrap()
    }

    #[test]
    fn forwarded_bot_tokens_keep_microsoft_verification_and_header_precedence() {
        use crate::middleware::jwt::FORWARDED_AUTHORIZATION_HEADER;
        use axum::http::header::AUTHORIZATION;

        let (c, claims, activity, keys) = fixture();
        let signed = sign(&claims);
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            "AWS4-HMAC-SHA256 Credential=cloudfront".parse().unwrap(),
        );
        assert!(bearer_token(&headers).is_err());
        headers.insert(
            FORWARDED_AUTHORIZATION_HEADER,
            format!("Bearer {signed}").parse().unwrap(),
        );
        assert!(verify_token(&c, bearer_token(&headers).unwrap(), &keys, &activity).is_ok());

        let mut wrong_audience = claims.clone();
        wrong_audience["aud"] = json!("another-bot");
        headers.insert(
            FORWARDED_AUTHORIZATION_HEADER,
            format!("Bearer {}", sign(&wrong_audience)).parse().unwrap(),
        );
        assert!(verify_token(&c, bearer_token(&headers).unwrap(), &keys, &activity).is_err());

        headers.insert(AUTHORIZATION, format!("Bearer {signed}").parse().unwrap());
        headers.insert(
            FORWARDED_AUTHORIZATION_HEADER,
            "Bearer forged".parse().unwrap(),
        );
        assert!(verify_token(&c, bearer_token(&headers).unwrap(), &keys, &activity).is_err());
    }

    #[test]
    fn connector_accepts_microsoft_audiences_and_lowercase_service_claim() {
        let (c, mut claims, activity, mut keys) = fixture();
        for audience in ["bot-id", "api://bot-id", "api://botid-bot-id"] {
            claims["aud"] = json!(audience);
            assert!(verify_token(&c, &sign(&claims), &keys, &activity).is_ok());
        }
        keys["keys"][0]
            .as_object_mut()
            .unwrap()
            .remove("endorsements");
        assert!(verify_token(&c, &sign(&claims), &keys, &activity).is_ok());
        keys["keys"][0]["endorsements"] = Value::Null;
        assert!(verify_token(&c, &sign(&claims), &keys, &activity).is_ok());
    }

    #[test]
    fn an_empty_endorsement_list_does_not_endorse_teams() {
        let (c, claims, activity, mut keys) = fixture();
        keys["keys"][0]["endorsements"] = json!([]);
        assert!(verify_token(&c, &sign(&claims), &keys, &activity).is_err());
        keys["keys"][0]["endorsements"] = json!("msteams");
        assert!(verify_token(&c, &sign(&claims), &keys, &activity).is_err());
        keys["keys"][0]["endorsements"] = json!(["webchat", "msteams"]);
        assert!(verify_token(&c, &sign(&claims), &keys, &activity).is_ok());
    }

    #[test]
    fn signing_keys_refresh_daily_and_rate_limit_unknown_key_lookups() {
        let keys = Arc::new(json!({"keys":[{"kid":"known"}]}));
        let at = |fetched_at, attempted_at| KeySet {
            keys: keys.clone(),
            fetched_at,
            attempted_at,
        };
        let now = KEYS_TTL_MS * 2;
        assert!(needs_refresh(None, "known", now));
        assert!(!needs_refresh(
            Some(&at(now - 1000, now - 1000)),
            "known",
            now
        ));
        assert!(!needs_refresh(
            Some(&at(now - 1000, now - 1000)),
            "rotated",
            now
        ));
        let settled = now - KEYS_REFETCH_INTERVAL_MS;
        assert!(needs_refresh(Some(&at(settled, settled)), "rotated", now));
        assert!(!needs_refresh(Some(&at(settled, settled)), "known", now));
        let expired = now - KEYS_TTL_MS;
        assert!(needs_refresh(Some(&at(expired, expired)), "known", now));
        assert!(!needs_refresh(Some(&at(expired, now - 1000)), "known", now));
    }

    fn reject(body: Value, cross_tenant: bool) -> MicrosoftError {
        graph_token_rejection(
            reqwest::StatusCode::BAD_REQUEST,
            None,
            body.to_string().as_bytes(),
            cross_tenant,
        )
    }

    #[test]
    fn graph_token_errors_surface_consent_from_aadsts_codes() {
        for body in [
            json!({"error":"invalid_client","error_codes":[7000229],"error_description":"AADSTS7000229: The client application 00000000-0000-0000-0000-000000000001 is missing service principal in the tenant 00000000-0000-0000-0000-000000000002. Trace ID: 1 Correlation ID: 2 Timestamp: 2026-09-28 10:00:00Z"}),
            json!({"error":"unauthorized_client","error_description":"AADSTS700016: Application with identifier 'x' was not found in the directory 'Contoso'."}),
            json!({"error":"invalid_grant","error_codes":[650052]}),
            json!({"error":"invalid_resource","error_codes":[500011],"error_description":"AADSTS500011: The resource principal named https://graph.microsoft.com was not found in the tenant named Contoso."}),
        ] {
            assert_eq!(
                reject(body.clone(), false),
                MicrosoftError::ConsentRequired,
                "{body}"
            );
        }
        let invalid_client =
            json!({"error":"invalid_client","error_description":"The client does not exist."});
        assert_eq!(
            reject(invalid_client.clone(), true),
            MicrosoftError::ConsentRequired
        );
        assert!(matches!(
            reject(invalid_client, false),
            MicrosoftError::Invalid(_)
        ));
    }

    #[test]
    fn graph_token_errors_report_only_the_status_and_aadsts_code() {
        let secret = json!({"error":"invalid_client","error_codes":[7000215],"error_description":"AADSTS7000215: Invalid client secret provided. Ensure the secret being sent is the client secret value test-only-secret. Trace ID: abc"});
        let MicrosoftError::Invalid(message) = reject(secret, true) else {
            panic!("a rejected secret is not a consent problem");
        };
        assert!(message.contains("AADSTS7000215"));
        assert!(!message.contains("test-only-secret") && !message.contains("Trace ID"));

        let near_miss =
            json!({"error":"invalid_request","error_description":"AADSTS7000161: Something else."});
        let MicrosoftError::Invalid(message) = reject(near_miss, true) else {
            panic!("AADSTS7000161 is not AADSTS700016");
        };
        assert!(message.contains("AADSTS7000161"));

        let MicrosoftError::Invalid(message) = graph_token_rejection(
            reqwest::StatusCode::BAD_REQUEST,
            None,
            b"<html>secret</html>",
            true,
        ) else {
            panic!("an unreadable body is an invalid response");
        };
        assert!(message.contains("400") && !message.contains("secret</html>"));
        assert_eq!(
            graph_token_rejection(reqwest::StatusCode::TOO_MANY_REQUESTS, Some(5), b"", true),
            MicrosoftError::Throttled(Some(5))
        );
        assert!(matches!(
            graph_token_rejection(reqwest::StatusCode::SERVICE_UNAVAILABLE, None, b"", true),
            MicrosoftError::Unavailable(_)
        ));
    }

    #[test]
    fn aadsts_codes_merge_the_list_and_the_description() {
        assert_eq!(
            aadsts_codes(
                &json!({"error_codes":[7000229, "x"],"error_description":"AADSTS7000229: missing"})
            ),
            vec![7_000_229]
        );
        assert_eq!(
            aadsts_codes(&json!({"error_description":"AADSTS650052: The app needs access"})),
            vec![650_052]
        );
        assert!(aadsts_codes(&json!({"error_description":"No code here"})).is_empty());
        assert!(aadsts_codes(&Value::Null).is_empty());
    }

    #[test]
    fn graph_tokens_are_cached_per_customer_tenant_and_secret() {
        let (mut c, ..) = fixture();
        let key = graph_token_key(&c);
        assert_ne!(key, token_key(&c));
        c.customer_tenant_id = "other-tenant".into();
        assert_ne!(key, graph_token_key(&c));
        let (mut c, ..) = fixture();
        c.secret = "rotated".into();
        assert_ne!(key, graph_token_key(&c));
    }

    fn expires_in(now: i64, offset_ms: i64) -> Option<String> {
        Some(
            chrono::DateTime::from_timestamp_millis(now + offset_ms)
                .unwrap()
                .to_rfc3339(),
        )
    }

    #[test]
    fn only_managed_secrets_close_to_expiry_are_renewed() {
        let (mut c, ..) = fixture();
        let now = chrono::Utc::now().timestamp_millis();
        c.secret_expires_at = expires_in(now, 86_400_000);
        assert!(!needs_renewal(&c, now));
        c.mode = crate::teams::AuthMode::FlowLikeManaged;
        assert!(needs_renewal(&c, now));
        c.secret_expires_at = expires_in(now, RENEWAL_WINDOW_MS + 60_000);
        assert!(!needs_renewal(&c, now));
        c.secret_expires_at = Some("not a date".into());
        assert!(!needs_renewal(&c, now));
        c.secret_expires_at = None;
        assert!(!needs_renewal(&c, now));
    }

    #[test]
    fn a_secret_is_valid_only_before_a_readable_expiry() {
        let (mut c, ..) = fixture();
        let now = chrono::Utc::now().timestamp_millis();
        c.secret_expires_at = expires_in(now, 60_000);
        assert!(secret_still_valid(&c, now));
        c.secret_expires_at = expires_in(now, 0);
        assert!(!secret_still_valid(&c, now));
        c.secret_expires_at = expires_in(now, -60_000);
        assert!(!secret_still_valid(&c, now));
        c.secret_expires_at = Some("not a date".into());
        assert!(!secret_still_valid(&c, now));
        c.secret_expires_at = None;
        assert!(!secret_still_valid(&c, now));
    }

    #[test]
    fn a_failed_rotation_pauses_renewal_until_the_secret_expires() {
        let (mut c, ..) = fixture();
        let now = chrono::Utc::now().timestamp_millis();
        c.id = "renewal-backoff-test".into();
        c.mode = crate::teams::AuthMode::FlowLikeManaged;
        c.secret_expires_at = expires_in(now, 86_400_000);
        assert!(needs_renewal(&c, now));
        RENEWAL_BACKOFF.insert(c.id.clone(), ());
        assert!(!needs_renewal(&c, now));
        c.secret_expires_at = expires_in(now, -60_000);
        assert!(needs_renewal(&c, now));
        RENEWAL_BACKOFF.invalidate(&c.id);
    }

    #[test]
    fn a_failed_renewal_keeps_a_valid_secret_and_fails_an_expired_one() {
        let (mut c, ..) = fixture();
        let now = chrono::Utc::now().timestamp_millis();
        let failed = || Err(ApiError::service_unavailable("Renewal failed"));
        c.secret_expires_at = expires_in(now, 60_000);
        let kept = renewed_or_current(&c, failed(), now).unwrap();
        assert_eq!((kept.id, kept.secret), (c.id.clone(), c.secret.clone()));
        let mut rotated = c.clone();
        rotated.secret = "rotated".into();
        assert_eq!(
            renewed_or_current(&c, Ok(rotated), now).unwrap().secret,
            "rotated"
        );
        c.secret_expires_at = expires_in(now, -60_000);
        let error = renewed_or_current(&c, failed(), now).unwrap_err();
        assert_eq!(error.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn detached_work_finishes_after_its_caller_gives_up() {
        let (done, finished) = tokio::sync::oneshot::channel();
        let caller = detached("Test renewal", async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = done.send(());
            Ok(())
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(1), caller)
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(5), finished)
                .await
                .is_ok_and(|sent| sent.is_ok())
        );
        let error = detached::<()>("Test renewal", async { panic!("renewal bug") })
            .await
            .unwrap_err();
        assert_eq!(error.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn bot_tokens_are_reused_until_a_margin_before_expiry() {
        assert_eq!(token_expiry(0, Some(3599)), 3_299_000);
        assert_eq!(token_expiry(0, Some(86_399)), 86_099_000);
        assert_eq!(token_expiry(0, Some(1)), 30_000);
        assert_eq!(token_expiry(0, None), 150_000);
    }

    #[test]
    fn connector_rejects_wrong_identity_expiry_channel_and_service_url() {
        let (c, claims, activity, keys) = fixture();
        for (field, value) in [
            ("iss", json!("https://attacker.invalid")),
            ("aud", json!("other-bot")),
            ("exp", json!(now() / 1000 - 120)),
            ("nbf", json!(now() / 1000 + 300)),
        ] {
            let mut invalid = claims.clone();
            invalid[field] = value;
            assert!(
                verify_token(&c, &sign(&invalid), &keys, &activity).is_err(),
                "{field}"
            );
        }
        for field in ["exp", "nbf", "serviceurl"] {
            let mut invalid = claims.clone();
            invalid.as_object_mut().unwrap().remove(field);
            assert!(
                verify_token(&c, &sign(&invalid), &keys, &activity).is_err(),
                "missing {field}"
            );
        }
        for (path, value) in [
            ("/channelData/tenant/id", "other-tenant"),
            ("/channelId", "webchat"),
            ("/serviceUrl", "https://attacker.invalid/"),
        ] {
            let mut invalid = activity.clone();
            *invalid.pointer_mut(path).unwrap() = json!(value);
            assert!(
                verify_token(&c, &sign(&claims), &keys, &invalid).is_err(),
                "{path}"
            );
        }
        let mut other_keys = keys.clone();
        other_keys["keys"][0]["endorsements"] = json!(["webchat"]);
        assert!(verify_token(&c, &sign(&claims), &other_keys, &activity).is_err());
        let forged = jsonwebtoken::encode(
            &jsonwebtoken::Header::new(Algorithm::HS256),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret(b"test-only"),
        )
        .unwrap();
        assert!(verify_token(&c, &forged, &keys, &activity).is_err());
        let mut forged_claims = claims.clone();
        forged_claims["aud"] = json!("stolen-bot");
        let valid = sign(&claims);
        use base64::Engine;
        let parts: Vec<_> = valid.split('.').collect();
        let forged = format!(
            "{}.{}.{}",
            parts[0],
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(forged_claims.to_string()),
            parts[2]
        );
        assert!(verify_token(&c, &forged, &keys, &activity).is_err());
    }
    #[test]
    fn service_urls_cannot_redirect_credentials_to_untrusted_hosts() {
        for good in [
            "https://smba.trafficmanager.net/emea/",
            "https://smba.infra.teams.microsoft.com/teams/",
            "https://foo.botapi.skype.com/amer/",
        ] {
            assert!(service_url(good).is_ok(), "{good}");
        }
        for bad in [
            "http://smba.trafficmanager.net/",
            "https://smba.trafficmanager.net.attacker.com/",
            "https://localhost/",
            "https://127.0.0.1/",
            "https://user@smba.trafficmanager.net/",
            "https://smba.trafficmanager.net:444/",
            "https://smba.trafficmanager.net/?redirect=x",
        ] {
            assert!(service_url(bad).is_err(), "{bad}");
        }
    }
}
