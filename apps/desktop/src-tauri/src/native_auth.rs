//! Per-profile native login authority. Rust owns interactive sign-in, token exchanges,
//! renewal and encrypted persistence for foreground and background callers.
use flow_like_types::base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

mod interactive;
pub(crate) mod oauth;
pub(crate) mod recovery;
mod store;
pub(crate) use interactive::{handle_callback, initialize, request_login};

/// Profile removal and hub changes revoke both saved grants and active device/run leases.
pub(crate) async fn invalidate_profile(
    profile: &flow_like::profile::Profile,
) -> Result<(), String> {
    let Some(hub) = flow_like::hub::hub_origin(&profile.hub, profile.secure) else {
        return Ok(());
    };
    let profile_id = profile.id.clone();
    tokio::spawn(async move { invalidate_session(&profile_id, &hub).await })
        .await
        .map_err(|_| "Authentication invalidation worker failed")?
}

async fn invalidate_session(profile_id: &str, hub: &str) -> Result<(), String> {
    let (key, session) = session(profile_id, hub).await?;
    let mut session = session.lock().await;
    session.load(&key).await?;
    // A tombstone also fences an import or login still waiting for discovery.
    session.save(&key, SavedSession::default()).await?;
    interactive::changed(profile_id, hub);
    Ok(())
}

#[tauri::command]
pub(crate) async fn begin_login(
    app_handle: tauri::AppHandle,
    profile_id: String,
) -> Result<(), String> {
    tokio::spawn(async move { interactive::begin_login(app_handle, profile_id).await })
        .await
        .map_err(|_| "Authentication sign-in worker failed")?
}

#[tauri::command]
pub(crate) async fn native_auth_logout(
    app_handle: tauri::AppHandle,
    profile_id: String,
) -> Result<(), String> {
    tokio::spawn(async move { interactive::native_auth_logout(app_handle, profile_id).await })
        .await
        .map_err(|_| "Authentication sign-out worker failed")?
}

const MAX_DOCUMENT: usize = 256 * 1024;
const RENEW_EARLY_SECONDS: u64 = 30;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Configuration {
    authority: String,
    client_id: String,
    token_endpoint: String,
    #[serde(default)]
    issuer: Option<String>,
    #[serde(default)]
    jwks_uri: Option<String>,
    #[serde(default)]
    end_session_endpoint: Option<String>,
    #[serde(default)]
    post_logout_redirect_uri: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SavedSession {
    generation: String,
    configuration: Option<Configuration>,
    user: Option<Value>,
    #[serde(default)]
    pending_login: Option<interactive::PendingLogin>,
}

impl Default for SavedSession {
    fn default() -> Self {
        Self {
            generation: uuid::Uuid::new_v4().to_string(),
            configuration: None,
            user: None,
            pending_login: None,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    generation: String,
    initialized: bool,
    user: Option<String>,
    authority: Option<String>,
    client_id: Option<String>,
}

#[derive(Default)]
struct Session {
    saved: Option<SavedSession>,
    initialized: bool,
    dirty: bool,
    revoked: Arc<AtomicBool>,
}

static SESSIONS: LazyLock<Mutex<HashMap<String, Arc<Mutex<Session>>>>> =
    LazyLock::new(Mutex::default);
static LEGACY_IMPORT_LOCK: LazyLock<Mutex<()>> = LazyLock::new(Mutex::default);

async fn claim_legacy_import(
    profile_id: &str,
    hub: &str,
    config: &Configuration,
) -> Result<(), String> {
    let _claim = LEGACY_IMPORT_LOCK.lock().await;
    // oidc-client-ts stored one grant per authority/client, shared across hubs and windows.
    let key = json!(["legacy-oidc-import-v1", config.authority, config.client_id]).to_string();
    let owner = json!({"profileId":profile_id,"hub":canonical_hub(hub)?});
    if let Some(saved) = store::read(&key).await? {
        let saved: Value = serde_json::from_slice(&saved)
            .map_err(|_| "Invalid legacy authentication migration claim")?;
        if saved != owner {
            return Err("This browser login was already assigned to another profile. Sign in to this profile again.".into());
        }
        return Ok(());
    }
    store::write(
        &key,
        serde_json::to_vec(&owner).map_err(|_| "Cannot encode authentication migration claim")?,
    )
    .await
}

fn secure_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Invalid authentication URL")?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && loopback)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || raw.contains('\\')
    {
        return Err("Authentication requires HTTPS or a loopback HTTP endpoint".into());
    }
    Ok(url)
}

fn canonical_hub(raw: &str) -> Result<String, String> {
    let url = secure_url(raw)?;
    if url.query().is_some() || url.path().contains('%') {
        return Err("Invalid hub origin".into());
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

async fn session(profile_id: &str, hub: &str) -> Result<(String, Arc<Mutex<Session>>), String> {
    if profile_id.is_empty() || profile_id.len() > 256 || profile_id.chars().any(char::is_control) {
        return Err("A valid profile ID is required for authentication".into());
    }
    let hub = json!([profile_id, canonical_hub(hub)?]).to_string();
    let mut sessions = SESSIONS.lock().await;
    if sessions.len() >= 64 && !sessions.contains_key(&hub) {
        return Err("Too many native authentication sessions".into());
    }
    let session = sessions.entry(hub.clone()).or_default().clone();
    Ok((hub, session))
}

impl Session {
    async fn load(&mut self, hub: &str) -> Result<&SavedSession, String> {
        if self.saved.is_none() {
            let bytes = store::read(hub).await?;
            self.initialized = bytes.is_some();
            self.saved = Some(match bytes {
                Some(bytes) => serde_json::from_slice(&bytes)
                    .map_err(|_| "Invalid saved authentication session")?,
                None => SavedSession::default(),
            });
        }
        Ok(self.saved.as_ref().expect("session loaded"))
    }

    async fn save(&mut self, hub: &str, saved: SavedSession) -> Result<(), String> {
        if self
            .saved
            .as_ref()
            .is_some_and(|previous| previous.generation != saved.generation)
        {
            self.revoked.store(true, Ordering::Release);
            self.revoked = Arc::new(AtomicBool::new(false));
            #[cfg(all(desktop, not(test)))]
            tokio::spawn(crate::device_models::drop_signed_out());
        }
        let bytes =
            serde_json::to_vec(&saved).map_err(|_| "Cannot encode authentication session")?;
        // Never publish a rotated refresh token until its durable replacement is committed.
        // On write failure retain the replacement in memory: the provider may already have
        // invalidated the old grant, so retrying it would destroy recovery in this process.
        let result = store::write(hub, bytes).await;
        self.dirty = result.is_err();
        self.saved = Some(saved);
        self.initialized = true;
        result
    }

    fn snapshot(&self) -> Result<Snapshot, String> {
        let saved = self
            .saved
            .as_ref()
            .ok_or("Authentication session is not loaded")?;
        Ok(Snapshot {
            generation: saved.generation.clone(),
            initialized: self.initialized,
            user: saved
                .user
                .as_ref()
                .map(|user| {
                    // React needs the current user and access bearer. Renewal grants
                    // remain exclusively in the native encrypted authority.
                    let mut public_user = user.clone();
                    if let Some(user) = public_user.as_object_mut() {
                        user.remove("refresh_token");
                    }
                    serde_json::to_string(&public_user)
                })
                .transpose()
                .map_err(|_| "Cannot encode authentication session")?,
            authority: saved
                .configuration
                .as_ref()
                .map(|config| config.authority.clone()),
            client_id: saved
                .configuration
                .as_ref()
                .map(|config| config.client_id.clone()),
        })
    }
}

fn token_claims(token: &str) -> Result<Value, String> {
    if token.is_empty() || token.len() > 64 * 1024 || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("Invalid authentication token".into());
    }
    let encoded = token
        .split('.')
        .nth(1)
        .ok_or("Invalid authentication token")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| "Invalid authentication token")?;
    serde_json::from_slice(&bytes).map_err(|_| "Invalid authentication token".into())
}

fn subject(user: &Value) -> Result<&str, String> {
    user.pointer("/profile/sub")
        .and_then(Value::as_str)
        .filter(|sub| !sub.is_empty() && sub.len() <= 1024)
        .ok_or_else(|| "Authentication subject is missing".into())
}

fn access(user: &Value) -> Result<&str, String> {
    user.get("access_token")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Authentication token is missing".into())
}

fn expires_at(user: &Value) -> Result<u64, String> {
    let claims = token_claims(access(user)?)?;
    if claims.get("sub").and_then(Value::as_str) != Some(subject(user)?) {
        return Err("Authentication token belongs to another account".into());
    }
    let token_expiry = claims
        .get("exp")
        .and_then(Value::as_u64)
        .ok_or("Authentication token expiry is missing")?;
    Ok(user
        .get("expires_at")
        .and_then(Value::as_u64)
        .map_or(token_expiry, |expiry| expiry.min(token_expiry)))
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[tauri::command]
pub(crate) async fn native_auth_read(profile_id: String, hub: String) -> Result<Snapshot, String> {
    let (hub, session) = session(&profile_id, &hub).await?;
    let mut session = session.lock().await;
    let saved = session.load(&hub).await?.clone();
    if session.dirty {
        session.save(&hub, saved).await?;
    }
    session.snapshot()
}

#[tauri::command]
pub(crate) async fn native_auth_write(
    app_handle: tauri::AppHandle,
    profile_id: String,
    hub: String,
    generation: String,
    configuration: Configuration,
    user: String,
) -> Result<Snapshot, String> {
    tokio::spawn(async move {
        if interactive::profile_hub(&app_handle, &profile_id).await? != canonical_hub(&hub)? {
            return Err("The login hub does not match this profile".into());
        }
        let trusted = interactive::configuration_for_hub(&hub).await?;
        if trusted.authority != configuration.authority
            || trusted.client_id != configuration.client_id
        {
            return Err("The login configuration does not match this profile's hub".into());
        }
        let mut parsed: Value =
            serde_json::from_str(&user).map_err(|_| "Invalid authentication session")?;
        let id = parsed
            .get("id_token")
            .and_then(Value::as_str)
            .ok_or("An identity token is required to import this login")?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "Cannot initialize authentication client")?;
        let claims = interactive::verify_imported_id_token(&client, &trusted, id).await?;
        if claims.get("sub").and_then(Value::as_str) != Some(subject(&parsed)?) {
            return Err("Imported tokens belong to different accounts".into());
        }
        // Account ownership comes from the verified ID token, including its issuer.
        parsed["profile"]["iss"] = claims["iss"].clone();
        // Profile mutations hold this same settings lock while revoking native authority.
        let settings = crate::state::TauriSettingsState::construct(&app_handle)
            .await
            .map_err(|_| "Settings are unavailable")?;
        let settings = settings.lock().await;
        let profile = settings
            .profiles
            .get(&profile_id)
            .ok_or("The authentication profile no longer exists")?;
        if flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
            .as_deref()
            != Some(canonical_hub(&hub)?.as_str())
        {
            return Err("The profile hub changed during authentication import".into());
        }
        claim_legacy_import(&profile_id, &hub, &trusted).await?;
        commit_session(
            profile_id,
            hub,
            generation,
            trusted,
            parsed.to_string(),
            true,
        )
        .await
    })
    .await
    .map_err(|_| "Authentication commit worker failed")?
}

async fn write_session(
    profile_id: String,
    hub: String,
    generation: String,
    configuration: Configuration,
    user: String,
) -> Result<Snapshot, String> {
    commit_session(profile_id, hub, generation, configuration, user, false).await
}

async fn commit_session(
    profile_id: String,
    hub: String,
    generation: String,
    configuration: Configuration,
    user: String,
    import_only: bool,
) -> Result<Snapshot, String> {
    let (hub, session) = session(&profile_id, &hub).await?;
    secure_url(&configuration.authority)?;
    secure_url(&configuration.token_endpoint)?;
    if configuration.client_id.is_empty()
        || configuration.client_id.len() > 1024
        || user.len() > MAX_DOCUMENT
    {
        return Err("Invalid authentication configuration".into());
    }
    let user: Value = serde_json::from_str(&user).map_err(|_| "Invalid authentication session")?;
    expires_at(&user)?;
    let mut session = session.lock().await;
    session.load(&hub).await?;
    if import_only && session.initialized {
        return Err("This profile's authentication is already managed by Rust".into());
    }
    let saved = session.load(&hub).await?;
    if saved.generation != generation {
        return Err("The authentication session changed".into());
    }
    let saved = SavedSession {
        generation: uuid::Uuid::new_v4().to_string(),
        configuration: Some(configuration),
        user: Some(user),
        pending_login: None,
    };
    session.save(&hub, saved).await?;
    session.snapshot()
}

#[tauri::command]
pub(crate) async fn native_auth_clear(
    profile_id: String,
    hub: String,
    generation: String,
) -> Result<Snapshot, String> {
    tokio::spawn(async move { clear_session(profile_id, hub, generation).await })
        .await
        .map_err(|_| "Authentication sign-out worker failed")?
}

async fn clear_session(
    profile_id: String,
    hub: String,
    generation: String,
) -> Result<Snapshot, String> {
    let (key, session) = session(&profile_id, &hub).await?;
    let mut session = session.lock().await;
    let saved = session.load(&key).await?;
    if saved.generation != generation {
        return Err("The authentication session changed".into());
    }
    // Persist a tombstone so a second window cannot migrate an old localStorage grant again.
    session.save(&key, SavedSession::default()).await?;
    let snapshot = session.snapshot()?;
    interactive::changed(&profile_id, &hub);
    Ok(snapshot)
}

async fn response_json(mut response: reqwest::Response) -> Result<Value, String> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Authentication service is unavailable")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_DOCUMENT {
            return Err("Authentication response is too large".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Invalid authentication response".into())
}

fn refreshed_user(
    previous: &Value,
    response: Value,
    config: &Configuration,
    now: u64,
) -> Result<Value, String> {
    let token = response
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or("Authentication response has no access token")?;
    let token_type = response
        .get("token_type")
        .and_then(Value::as_str)
        .ok_or("Authentication response has no token type")?;
    if !token_type.eq_ignore_ascii_case("Bearer") {
        return Err("Unsupported authentication token type".into());
    }
    let claims = token_claims(token)?;
    if claims.get("sub").and_then(Value::as_str) != Some(subject(previous)?) {
        return Err("Refreshed token belongs to another account".into());
    }
    let expiry = claims
        .get("exp")
        .and_then(Value::as_u64)
        .filter(|expiry| *expiry > now)
        .ok_or("Refreshed token has expired")?;
    let mut user = previous.clone();
    user["access_token"] = json!(token);
    user["token_type"] = json!(token_type);
    user["expires_at"] = json!(
        response
            .get("expires_in")
            .and_then(Value::as_u64)
            .map_or(expiry, |seconds| expiry.min(now.saturating_add(seconds)))
    );
    if let Some(refresh) = response.get("refresh_token") {
        if refresh.as_str().is_none_or(str::is_empty) {
            return Err("Invalid replacement refresh token".into());
        }
        user["refresh_token"] = refresh.clone();
    }
    if let Some(id_token) = response.get("id_token") {
        let claims = token_claims(id_token.as_str().ok_or("Invalid ID token")?)?;
        let old = previous
            .get("id_token")
            .and_then(Value::as_str)
            .map(token_claims)
            .transpose()?;
        let expected_issuer = old
            .as_ref()
            .and_then(|claims| claims.get("iss"))
            .and_then(Value::as_str)
            .unwrap_or(&config.authority);
        let audience = claims.get("aud");
        let correct_audience = audience.and_then(Value::as_str) == Some(config.client_id.as_str())
            || audience.and_then(Value::as_array).is_some_and(|aud| {
                aud.iter()
                    .any(|value| value.as_str() == Some(config.client_id.as_str()))
            });
        if claims.get("sub").and_then(Value::as_str) != Some(subject(previous)?)
            || claims.get("iss").and_then(Value::as_str) != Some(expected_issuer)
            || !correct_audience
            || claims
                .get("exp")
                .and_then(Value::as_u64)
                .is_none_or(|expiry| expiry <= now)
        {
            return Err("Refreshed ID token does not match the signed-in account".into());
        }
        user["id_token"] = id_token.clone();
    }
    if let Some(scope) = response.get("scope").filter(|scope| scope.is_string()) {
        user["scope"] = scope.clone();
    }
    Ok(user)
}

async fn refresh(
    profile_id: &str,
    hub: &str,
    expected_subject: &str,
    generation: Option<&str>,
    rejected_token: Option<&str>,
) -> Result<Snapshot, String> {
    // A tool cancellation must not abandon an exchange after the provider rotates the
    // grant. The owned worker commits the response even if its caller stops waiting.
    let profile_id = profile_id.to_owned();
    let hub = hub.to_owned();
    let expected_subject = expected_subject.to_owned();
    let generation = generation.map(str::to_owned);
    let rejected_token = rejected_token.map(str::to_owned);
    tokio::spawn(async move {
        refresh_owned(
            &profile_id,
            &hub,
            &expected_subject,
            generation.as_deref(),
            rejected_token.as_deref(),
        )
        .await
    })
    .await
    .map_err(|_| "Authentication refresh worker failed")?
}

async fn refresh_owned(
    profile_id: &str,
    hub: &str,
    expected_subject: &str,
    generation: Option<&str>,
    rejected_token: Option<&str>,
) -> Result<Snapshot, String> {
    let (key, session) = session(profile_id, hub).await?;
    // One lock includes the exchange and durable commit. Login/logout serialize after an
    // in-flight exchange, and all foreground/background callers reuse its rotated grant.
    let mut session = session.lock().await;
    let saved = session.load(&key).await?.clone();
    if generation.is_some_and(|generation| generation != saved.generation) {
        return Err("The authentication session changed".into());
    }
    if session.dirty {
        session.save(&key, saved.clone()).await?;
    }
    let user = saved
        .user
        .as_ref()
        .ok_or("Sign in to this hub to continue")?;
    if subject(user)? != expected_subject {
        return Err("The Hub account changed".into());
    }
    let token = access(user)?;
    if expires_at(user)? > now_seconds().saturating_add(RENEW_EARLY_SECONDS)
        && rejected_token != Some(token)
    {
        return session.snapshot();
    }
    let Some(refresh_token) = user
        .get("refresh_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
    else {
        interactive::request_login_for_session(profile_id.to_owned(), saved.generation.clone());
        return Err("Sign in again to renew this session".into());
    };
    let config = saved
        .configuration
        .as_ref()
        .ok_or("Authentication configuration is missing")?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Cannot initialize authentication client")?;
    let response = client
        .post(secure_url(&config.token_endpoint)?)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", config.client_id.as_str()),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await
        .map_err(|_| "Authentication service is unavailable")?;
    let status = response.status();
    let response = response_json(response).await?;
    if !status.is_success() {
        if matches!(status.as_u16(), 400 | 401)
            && response
                .get("error")
                .and_then(Value::as_str)
                .is_some_and(|error| {
                    matches!(
                        error,
                        "invalid_grant"
                            | "login_required"
                            | "consent_required"
                            | "interaction_required"
                    )
                })
        {
            session.save(&key, SavedSession::default()).await?;
            interactive::changed(profile_id, hub);
            interactive::request_login_for_session(
                profile_id.to_owned(),
                session.snapshot()?.generation,
            );
            return Err("invalid_grant".into());
        }
        return Err(
            "Authentication service refused renewal; the saved session was retained".into(),
        );
    }
    let renewed = match refreshed_user(user, response.clone(), config, now_seconds()) {
        Ok(renewed) => renewed,
        Err(error) => {
            // A successful exchange may have consumed the previous grant. Invalid
            // replacement credentials cannot be reused or attributed to this account.
            session.save(&key, SavedSession::default()).await?;
            interactive::changed(profile_id, hub);
            interactive::request_login_for_session(
                profile_id.to_owned(),
                session.snapshot()?.generation,
            );
            return Err(error);
        }
    };
    if let Some(id_token) = response.get("id_token").and_then(Value::as_str)
        && config.jwks_uri.is_some()
        && let Err(error) = interactive::verify_refreshed_identity(&client, config, id_token).await
    {
        // A provider may already have rotated its grant while JWKS retrieval failed.
        // Keep only that replacement grant; unverified access/ID tokens are never published.
        if error.temporary
            && let Some(rotated) = response.get("refresh_token")
        {
            let mut retained = saved.clone();
            if let Some(user) = retained.user.as_mut() {
                user["refresh_token"] = rotated.clone();
            }
            session.save(&key, retained).await?;
        } else if !error.temporary {
            session.save(&key, SavedSession::default()).await?;
            interactive::changed(profile_id, hub);
            interactive::request_login_for_session(
                profile_id.to_owned(),
                session.snapshot()?.generation,
            );
        }
        return Err(error.message);
    }
    session
        .save(
            &key,
            SavedSession {
                user: Some(renewed),
                ..saved
            },
        )
        .await?;
    let snapshot = session.snapshot()?;
    interactive::changed(profile_id, hub);
    Ok(snapshot)
}

#[tauri::command]
pub(crate) async fn native_auth_refresh(
    profile_id: String,
    hub: String,
    subject: String,
    generation: String,
    rejected_token: Option<String>,
) -> Result<Snapshot, String> {
    refresh(
        &profile_id,
        &hub,
        &subject,
        Some(&generation),
        rejected_token.as_deref(),
    )
    .await
}

pub(crate) async fn current_subject(profile_id: &str, hub: &str) -> Result<Option<String>, String> {
    let snapshot = native_auth_read(profile_id.into(), hub.into()).await?;
    snapshot
        .user
        .map(|user| {
            let user: Value =
                serde_json::from_str(&user).map_err(|_| "Invalid authentication session")?;
            subject(&user).map(str::to_owned)
        })
        .transpose()
}

pub(crate) async fn access_token(
    profile_id: &str,
    hub: &str,
    expected_subject: &str,
    rejected_token: Option<&str>,
) -> Result<String, String> {
    let snapshot = refresh(profile_id, hub, expected_subject, None, rejected_token).await?;
    let user: Value = serde_json::from_str(snapshot.user.as_deref().ok_or("Sign in to continue")?)
        .map_err(|_| "Invalid authentication session")?;
    access(&user).map(str::to_owned)
}

/// A run retains its login generation even if the same account signs in again later.
#[derive(Clone, Debug)]
pub(crate) struct SessionBinding {
    pub(crate) profile_id: String,
    pub(crate) hub: String,
    pub(crate) issuer: String,
    pub(crate) subject: String,
    pub(crate) generation: String,
    revoked: Arc<AtomicBool>,
}

impl SessionBinding {
    #[cfg(test)]
    pub(crate) fn for_test(profile_id: &str, hub: &str, issuer: &str, subject: &str) -> Self {
        Self {
            profile_id: profile_id.into(),
            hub: hub.into(),
            issuer: issuer.into(),
            subject: subject.into(),
            generation: uuid::Uuid::new_v4().to_string(),
            revoked: Arc::new(AtomicBool::new(false)),
        }
    }
    pub(crate) async fn capture(profile_id: &str, hub: &str) -> Result<Option<Self>, String> {
        if hub.is_empty() {
            return Ok(None);
        }
        let (key, session) = session(profile_id, hub).await?;
        let mut session = session.lock().await;
        let saved = session.load(&key).await?;
        let Some(user) = &saved.user else {
            return Ok(None);
        };
        let subject = subject(user)?.to_owned();
        let issuer =
            user.pointer("/profile/iss")
                .and_then(Value::as_str)
                .or_else(|| {
                    saved.configuration.as_ref().and_then(|config| {
                        config.issuer.as_deref().or(Some(config.authority.as_str()))
                    })
                })
                .unwrap_or_default()
                .to_owned();
        let generation = saved.generation.clone();
        Ok(Some(Self {
            profile_id: profile_id.to_owned(),
            hub: canonical_hub(hub)?,
            issuer,
            subject,
            generation,
            revoked: session.revoked.clone(),
        }))
    }

    pub(crate) async fn access_token(
        &self,
        rejected_token: Option<&str>,
    ) -> Result<String, String> {
        if !self.is_current() {
            return Err("The authentication session changed".into());
        }
        let snapshot = refresh(
            &self.profile_id,
            &self.hub,
            &self.subject,
            Some(&self.generation),
            rejected_token,
        )
        .await?;
        let user: Value =
            serde_json::from_str(snapshot.user.as_deref().ok_or("Sign in to continue")?)
                .map_err(|_| "Invalid authentication session")?;
        access(&user).map(str::to_owned)
    }

    /// Selection can inspect the saved bearer while offline. Dispatchers must use
    /// `access_token` or the request authorizer before contacting the hub.
    pub(crate) async fn cached_access_token(&self) -> Result<String, String> {
        if !self.is_current() {
            return Err("The authentication session changed".into());
        }
        let (key, session) = session(&self.profile_id, &self.hub).await?;
        let mut session = session.lock().await;
        let saved = session.load(&key).await?;
        let user = saved.user.as_ref().ok_or("Sign in to continue")?;
        if saved.generation != self.generation || subject(user)? != self.subject {
            return Err("The authentication session changed".into());
        }
        access(user).map(str::to_owned)
    }

    pub(crate) fn is_current(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests;
