use super::*;
use sha2::{Digest, Sha256};
use std::sync::OnceLock;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

static APP: OnceLock<AppHandle> = OnceLock::new();
static PROMPTS: LazyLock<std::sync::Mutex<HashMap<String, Instant>>> =
    LazyLock::new(std::sync::Mutex::default);
const LOGIN_TTL_SECONDS: u64 = 10 * 60;

pub(crate) fn initialize(app: AppHandle) {
    let _ = APP.set(app);
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct PendingLogin {
    state: String,
    nonce: String,
    verifier: String,
    redirect_uri: String,
    authorization_url: String,
    configuration: Configuration,
    expires: u64,
}

struct Discovery {
    configuration: Configuration,
    authorization_endpoint: String,
    redirect_uri: String,
    scope: String,
}

pub(super) async fn profile_hub(app: &AppHandle, profile_id: &str) -> Result<String, String> {
    let state = crate::state::TauriSettingsState::construct(app)
        .await
        .map_err(|_| "Settings are unavailable")?;
    let settings = state.lock().await;
    let profile = settings
        .profiles
        .get(profile_id)
        .ok_or("The authentication profile no longer exists")?;
    flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
        .ok_or_else(|| "This profile has no valid hub origin".into())
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Cannot initialize authentication client".into())
}

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let response = client
        .get(secure_url(url)?)
        .send()
        .await
        .map_err(|_| "Authentication discovery is unavailable")?;
    if !response.status().is_success() {
        return Err("Authentication discovery was refused".into());
    }
    response_json(response).await
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Authentication configuration requires {name}"))
}

fn redirect_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Invalid authentication redirect URI")?;
    if url.scheme() == "flow-like"
        && url.host_str() == Some("auth")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.path().trim_matches('/').is_empty()
        && url.query().is_none()
        && url.fragment().is_none()
    {
        return Ok(url);
    }
    secure_url(raw)?;
    if url.query().is_some() || url.fragment().is_some() {
        return Err("Authentication redirects cannot contain a query or fragment".into());
    }
    Ok(url)
}

async fn discover(hub: &str) -> Result<Discovery, String> {
    let client = client()?;
    let base = if hub.ends_with("/api/v1") {
        hub.to_owned()
    } else {
        format!("{hub}/api/v1")
    };
    let config = get_json(&client, &format!("{base}/auth/openid")).await?;
    let authority = field(&config, "authority")?.to_owned();
    secure_url(&authority)?;
    let client_id = field(&config, "client_id")?.to_owned();
    let metadata_url = config
        .get("discovery_url")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{}/.well-known/openid-configuration",
                authority.trim_end_matches('/')
            )
        });
    let metadata = get_json(&client, &metadata_url).await?;
    let issuer = field(&metadata, "issuer")?.to_owned();
    let expected_issuer = config
        .get("issuer")
        .and_then(Value::as_str)
        .unwrap_or(&authority);
    if issuer != expected_issuer {
        return Err("Authentication discovery returned another issuer".into());
    }
    let token_endpoint = field(&metadata, "token_endpoint")?.to_owned();
    secure_url(&token_endpoint)?;
    let authorization_endpoint = field(&metadata, "authorization_endpoint")?.to_owned();
    secure_url(&authorization_endpoint)?;
    let jwks_uri = field(&metadata, "jwks_uri")?.to_owned();
    secure_url(&jwks_uri)?;
    let redirect_uri = option_env!("NEXT_PUBLIC_REDIRECT_URL")
        .or_else(|| config.get("redirect_uri").and_then(Value::as_str))
        .ok_or("Authentication redirect URI is missing")?
        .to_owned();
    redirect_url(&redirect_uri)?;
    Ok(Discovery {
        configuration: Configuration {
            authority,
            client_id,
            token_endpoint,
            issuer: Some(issuer),
            jwks_uri: Some(jwks_uri),
            end_session_endpoint: metadata
                .get("end_session_endpoint")
                .and_then(Value::as_str)
                .map(str::to_owned),
            post_logout_redirect_uri: config
                .get("post_logout_redirect_uri")
                .and_then(Value::as_str)
                .map(str::to_owned),
        },
        authorization_endpoint,
        redirect_uri,
        scope: config
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("openid profile email offline_access")
            .to_owned(),
    })
}

pub(super) async fn configuration_for_hub(hub: &str) -> Result<Configuration, String> {
    Ok(discover(hub).await?.configuration)
}

fn random_secret() -> String {
    use chacha20poly1305::aead::{OsRng, rand_core::RngCore};
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn login_request(discovery: Discovery) -> Result<PendingLogin, String> {
    let state = format!("fl-native-{}", random_secret());
    let nonce = random_secret();
    let verifier = random_secret();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = secure_url(&discovery.authorization_endpoint)?;
    url.query_pairs_mut()
        .append_pair("client_id", &discovery.configuration.client_id)
        .append_pair("redirect_uri", &discovery.redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", &discovery.scope)
        .append_pair("state", &state)
        .append_pair("nonce", &nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    Ok(PendingLogin {
        state,
        nonce,
        verifier,
        redirect_uri: discovery.redirect_uri,
        authorization_url: url.to_string(),
        configuration: discovery.configuration,
        expires: now_seconds() + LOGIN_TTL_SECONDS,
    })
}

pub(crate) async fn begin_login(app_handle: AppHandle, profile_id: String) -> Result<(), String> {
    begin_login_for_generation(app_handle, profile_id, None).await
}

async fn begin_login_for_generation(
    app_handle: AppHandle,
    profile_id: String,
    expected_generation: Option<&str>,
) -> Result<(), String> {
    initialize(app_handle.clone());
    let (hub, key, session, generation) = {
        let settings = crate::state::TauriSettingsState::construct(&app_handle)
            .await
            .map_err(|_| "Settings are unavailable")?;
        let settings = settings.lock().await;
        let profile = settings
            .profiles
            .get(&profile_id)
            .ok_or("The authentication profile no longer exists")?;
        let hub = flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
            .ok_or("This profile has no valid hub origin")?;
        // Profile changes revoke under this same lock. Capture the generation before
        // a hub change can invalidate this intent and then restore the previous hub.
        let (key, session) = session(&profile_id, &hub).await?;
        let generation = session.lock().await.load(&key).await?.generation.clone();
        (hub, key, session, generation)
    };
    if expected_generation.is_some_and(|expected| expected != generation) {
        return Err("The authentication session changed".into());
    }
    let discovery = discover(&hub).await?;
    if profile_hub(&app_handle, &profile_id).await? != hub {
        return Err("The profile hub changed during sign-in".into());
    }
    {
        let mut session = session.lock().await;
        let mut saved = session.load(&key).await?.clone();
        if generation != saved.generation {
            return Err("The authentication session changed".into());
        }
        let pending = match saved
            .pending_login
            .as_ref()
            .filter(|pending| pending.expires > now_seconds())
        {
            Some(pending) => pending.clone(),
            None => login_request(discovery)?,
        };
        let url = pending.authorization_url.clone();
        saved.pending_login = Some(pending);
        session.save(&key, saved).await?;
        // Keep the generation lock through opening the browser, so a queued
        // renewal prompt cannot appear after a concurrent intentional logout.
        app_handle
            .opener()
            .open_url(url, None::<String>)
            .map_err(|_| "Could not open the sign-in window".to_string())
    }
}

pub(super) fn changed(profile_id: &str, hub: &str) {
    if let Some(app) = APP.get() {
        let _ = app.emit(
            "native-auth-changed",
            json!({"profileId":profile_id,"hub":hub}),
        );
    }
}

pub(crate) fn request_login(profile_id: String) {
    request_login_inner(profile_id, None);
}

pub(super) fn request_login_for_session(profile_id: String, generation: String) {
    request_login_inner(profile_id, Some(generation));
}

fn request_login_inner(profile_id: String, generation: Option<String>) {
    let Some(app) = APP.get().cloned() else {
        return;
    };
    let Ok(mut prompts) = PROMPTS.lock() else {
        return;
    };
    prompts.retain(|_, started| started.elapsed() < Duration::from_secs(LOGIN_TTL_SECONDS));
    if prompts.contains_key(&profile_id) {
        return;
    }
    prompts.insert(profile_id.clone(), Instant::now());
    drop(prompts);
    tokio::spawn(async move {
        if let Err(error) =
            begin_login_for_generation(app.clone(), profile_id.clone(), generation.as_deref()).await
        {
            if error != "The authentication session changed" {
                let _ = app.emit("native-auth-required", json!({"profileId":profile_id}));
            }
        }
    });
}

pub(super) async fn verify_id_token(
    client: &reqwest::Client,
    config: &Configuration,
    token: &str,
) -> Result<Value, String> {
    verify_id_token_expiry(client, config, token, true)
        .await
        .map_err(|error| error.message)
}

pub(super) struct IdentityVerificationError {
    pub(super) temporary: bool,
    pub(super) message: String,
}

impl IdentityVerificationError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            temporary: false,
            message: message.into(),
        }
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            temporary: true,
            message: message.into(),
        }
    }
}

pub(super) async fn verify_refreshed_identity(
    client: &reqwest::Client,
    config: &Configuration,
    token: &str,
) -> Result<Value, IdentityVerificationError> {
    verify_id_token_expiry(client, config, token, true).await
}

pub(super) async fn verify_imported_id_token(
    client: &reqwest::Client,
    config: &Configuration,
    token: &str,
) -> Result<Value, String> {
    verify_id_token_expiry(client, config, token, false)
        .await
        .map_err(|error| error.message)
}

async fn verify_id_token_expiry(
    client: &reqwest::Client,
    config: &Configuration,
    token: &str,
    check_expiry: bool,
) -> Result<Value, IdentityVerificationError> {
    let url = config.jwks_uri.as_deref().ok_or_else(|| {
        IdentityVerificationError::invalid("Identity signing keys are unavailable")
    })?;
    let url = secure_url(url).map_err(IdentityVerificationError::invalid)?;
    let response = client.get(url).send().await.map_err(|_| {
        IdentityVerificationError::unavailable("Identity signing keys are temporarily unavailable")
    })?;
    if response.status().is_server_error()
        || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
    {
        return Err(IdentityVerificationError::unavailable(
            "Identity signing keys are temporarily unavailable",
        ));
    }
    if !response.status().is_success() {
        return Err(IdentityVerificationError::invalid(
            "Identity signing key retrieval was refused",
        ));
    }
    let jwks = response_json(response).await.map_err(|message| {
        if message == "Authentication service is unavailable" {
            IdentityVerificationError::unavailable(message)
        } else {
            IdentityVerificationError::invalid(message)
        }
    })?;
    let jwks: jsonwebtoken::jwk::JwkSet = serde_json::from_value(jwks)
        .map_err(|_| IdentityVerificationError::invalid("Invalid identity signing keys"))?;
    verify_with_keys(config, token, &jwks, check_expiry).map_err(IdentityVerificationError::invalid)
}

fn verify_with_keys(
    config: &Configuration,
    token: &str,
    jwks: &jsonwebtoken::jwk::JwkSet,
    check_expiry: bool,
) -> Result<Value, String> {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
    let header = decode_header(token).map_err(|_| "Invalid identity token header")?;
    if !matches!(
        header.alg,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    ) {
        return Err("Unsupported identity token signing algorithm".into());
    }
    let key = header
        .kid
        .as_deref()
        .and_then(|kid| jwks.find(kid))
        .or_else(|| (header.kid.is_none() && jwks.keys.len() == 1).then(|| &jwks.keys[0]))
        .ok_or("Identity token signing key is unknown")?;
    if key
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|usage| *usage != jsonwebtoken::jwk::PublicKeyUse::Signature)
        || key
            .common
            .key_operations
            .as_ref()
            .is_some_and(|operations| {
                !operations.contains(&jsonwebtoken::jwk::KeyOperations::Verify)
            })
        || key
            .common
            .key_algorithm
            .as_ref()
            .is_some_and(|algorithm| algorithm.to_string() != format!("{:?}", header.alg))
    {
        return Err("Identity signing key does not permit this signature".into());
    }
    let key = DecodingKey::from_jwk(key).map_err(|_| "Unsupported identity signing key")?;
    let mut validation = Validation::new(header.alg);
    validation.set_audience(&[&config.client_id]);
    validation.set_issuer(&[config.issuer.as_deref().unwrap_or(&config.authority)]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.validate_nbf = true;
    validation.validate_exp = check_expiry;
    validation.leeway = 30;
    let claims = decode::<Value>(token, &key, &validation)
        .map_err(|_| "Identity token verification failed")?
        .claims;
    if claims
        .get("aud")
        .and_then(Value::as_array)
        .is_some_and(|aud| aud.len() > 1)
        && claims.get("azp").and_then(Value::as_str) != Some(config.client_id.as_str())
    {
        return Err("Identity token was issued to another client".into());
    }
    if claims
        .get("iat")
        .and_then(Value::as_u64)
        .is_none_or(|issued| issued > now_seconds() + 30)
    {
        return Err("Identity token has an invalid issue time".into());
    }
    Ok(claims)
}

fn callback_fields(raw: &str) -> Result<(reqwest::Url, HashMap<String, String>), String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "Invalid authentication callback")?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Authentication callbacks cannot contain URL credentials".into());
    }
    if url.fragment().is_some() {
        return Err("Implicit authentication callbacks are not supported".into());
    }
    let mut fields = HashMap::new();
    for (name, value) in url.query_pairs() {
        if fields
            .insert(name.into_owned(), value.into_owned())
            .is_some()
        {
            return Err("Duplicate authentication callback parameter".into());
        }
    }
    Ok((url, fields))
}
fn callback_matches(received: &reqwest::Url, redirect: &str) -> bool {
    let Ok(expected) = redirect_url(redirect) else {
        return false;
    };
    (received.origin() == expected.origin() && received.path() == expected.path())
        || (received.scheme() == "flow-like"
            && received.host_str() == Some("auth")
            && received.port().is_none()
            && received.path().trim_matches('/').is_empty())
}

pub(crate) async fn handle_callback(app: &AppHandle, raw: &str) -> Result<bool, String> {
    let (url, fields) = callback_fields(raw)?;
    let Some(state) = fields
        .get("state")
        .filter(|state| state.starts_with("fl-native-"))
    else {
        return Ok(false);
    };
    let profiles = {
        let settings = crate::state::TauriSettingsState::construct(app)
            .await
            .map_err(|_| "Settings are unavailable")?;
        let settings = settings.lock().await;
        settings
            .profiles
            .values()
            .map(|profile| profile.hub_profile.clone())
            .collect::<Vec<_>>()
    };
    for profile in profiles {
        let Some(hub) = flow_like::hub::hub_origin(&profile.hub, profile.secure) else {
            continue;
        };
        let (key, session) = session(&profile.id, &hub).await?;
        let (pending, generation) = {
            let mut session = session.lock().await;
            let saved = match session.load(&key).await {
                Ok(saved) => saved.clone(),
                Err(_) => continue,
            };
            let Some(pending) = saved
                .pending_login
                .clone()
                .filter(|pending| &pending.state == state)
            else {
                continue;
            };
            if pending.expires <= now_seconds() || !callback_matches(&url, &pending.redirect_uri) {
                return Err("Authentication callback expired or targets another redirect".into());
            }
            if fields.get("iss").is_some_and(|issuer| {
                issuer
                    != pending
                        .configuration
                        .issuer
                        .as_ref()
                        .unwrap_or(&pending.configuration.authority)
            }) || fields
                .get("client_id")
                .is_some_and(|client| client != &pending.configuration.client_id)
            {
                return Err(
                    "Authentication callback belongs to another identity provider or client".into(),
                );
            }
            let generation = saved.generation.clone();
            session
                .save(
                    &key,
                    SavedSession {
                        pending_login: None,
                        ..saved
                    },
                )
                .await?;
            (pending, generation)
        };
        if fields.contains_key("error") {
            return Err("Sign-in was cancelled or refused by the identity provider".into());
        }
        let code = fields
            .get("code")
            .filter(|code| !code.is_empty() && code.len() <= 16 * 1024)
            .ok_or("Authentication callback has no code")?;
        let client = client()?;
        let response = client
            .post(secure_url(&pending.configuration.token_endpoint)?)
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", pending.configuration.client_id.as_str()),
                ("code", code.as_str()),
                ("redirect_uri", pending.redirect_uri.as_str()),
                ("code_verifier", pending.verifier.as_str()),
            ])
            .send()
            .await
            .map_err(|_| "Authentication code exchange is unavailable")?;
        if !response.status().is_success() {
            return Err("Authentication code exchange was refused".into());
        }
        let response = response_json(response).await?;
        let id_token = field(&response, "id_token")?;
        let claims = verify_id_token(&client, &pending.configuration, id_token).await?;
        if claims.get("nonce").and_then(Value::as_str) != Some(pending.nonce.as_str()) {
            return Err("Authentication nonce does not match this login".into());
        }
        let sub = field(&claims, "sub")?;
        let access_token = field(&response, "access_token")?;
        let access_claims = token_claims(access_token)?;
        if access_claims.get("sub").and_then(Value::as_str) != Some(sub) {
            return Err("Authentication tokens belong to different accounts".into());
        }
        let expires = access_claims
            .get("exp")
            .and_then(Value::as_u64)
            .filter(|expires| *expires > now_seconds())
            .ok_or("Authentication returned an expired access token")?;
        if !field(&response, "token_type")?.eq_ignore_ascii_case("Bearer") {
            return Err("Unsupported authentication token type".into());
        }
        let expires = response
            .get("expires_in")
            .and_then(Value::as_u64)
            .map_or(expires, |duration| {
                expires.min(now_seconds().saturating_add(duration))
            });
        let mut user = json!({"profile":claims,"id_token":id_token,"access_token":access_token,"token_type":"Bearer","scope":response["scope"],"expires_at":expires});
        if let Some(refresh) = response.get("refresh_token") {
            if refresh.as_str().is_none_or(|token| {
                token.is_empty()
                    || token.len() > 64 * 1024
                    || !token.bytes().all(|byte| byte.is_ascii_graphic())
            }) {
                return Err("Authentication returned an invalid refresh token".into());
            }
            user["refresh_token"] = refresh.clone();
        }
        if profile_hub(app, &profile.id).await? != hub {
            return Err("The profile hub changed during sign-in".into());
        }
        write_session(
            profile.id.clone(),
            hub.clone(),
            generation,
            pending.configuration,
            user.to_string(),
        )
        .await?;
        if let Ok(mut prompts) = PROMPTS.lock() {
            prompts.remove(&profile.id);
        }
        let _ = app.emit(
            "native-auth-changed",
            json!({"profileId":profile.id,"hub":hub}),
        );
        return Ok(true);
    }
    Err("Authentication state is unknown or was already consumed".into())
}

pub(crate) async fn native_auth_logout(
    app_handle: AppHandle,
    profile_id: String,
) -> Result<(), String> {
    let hub = profile_hub(&app_handle, &profile_id).await?;
    let (key, session) = session(&profile_id, &hub).await?;
    let saved = {
        let mut session = session.lock().await;
        session.load(&key).await?.clone()
    };
    clear_session(profile_id.clone(), hub.clone(), saved.generation).await?;
    if let Ok(mut prompts) = PROMPTS.lock() {
        prompts.remove(&profile_id);
    }
    let _ = app_handle.emit(
        "native-auth-changed",
        json!({"profileId":profile_id,"hub":hub}),
    );
    if let Some(config) = saved.configuration
        && let Some(endpoint) = config.end_session_endpoint
    {
        let mut url = secure_url(&endpoint)?;
        if let Some(token) = saved
            .user
            .as_ref()
            .and_then(|user| user.get("id_token"))
            .and_then(Value::as_str)
        {
            url.query_pairs_mut().append_pair("id_token_hint", token);
        }
        url.query_pairs_mut()
            .append_pair("client_id", &config.client_id);
        if let Some(redirect) = config.post_logout_redirect_uri {
            secure_url(&redirect)?;
            url.query_pairs_mut()
                .append_pair("post_logout_redirect_uri", &redirect);
        }
        app_handle
            .opener()
            .open_url(url.to_string(), None::<String>)
            .map_err(|_| "Could not open the sign-out window")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Configuration {
        Configuration {
            authority: "https://issuer.test".into(),
            client_id: "client".into(),
            token_endpoint: "https://issuer.test/token".into(),
            issuer: Some("https://issuer.test".into()),
            ..Default::default()
        }
    }

    #[test]
    fn native_login_binds_unique_state_nonce_and_pkce_to_the_redirect() {
        let request = login_request(Discovery {
            configuration: config(),
            authorization_endpoint: "https://issuer.test/authorize".into(),
            redirect_uri: "https://app.flow-like.com/desktop/callback".into(),
            scope: "openid offline_access".into(),
        })
        .unwrap();
        let url = reqwest::Url::parse(&request.authorization_url).unwrap();
        let query: HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(query["state"], request.state);
        assert_eq!(query["nonce"], request.nonce);
        assert_eq!(query["code_challenge_method"], "S256");
        assert_eq!(
            query["code_challenge"],
            URL_SAFE_NO_PAD.encode(Sha256::digest(request.verifier.as_bytes()))
        );
        assert_eq!(query["redirect_uri"], request.redirect_uri);
        assert!(!query.contains_key("code_verifier"));
        assert_ne!(request.nonce, request.verifier);
    }

    #[test]
    fn callbacks_reject_duplicate_parameters_fragments_and_wrong_destinations() {
        assert!(callback_fields("flow-like://auth?state=a&state=b&code=c").is_err());
        assert!(callback_fields("flow-like://auth?state=a#access_token=token").is_err());
        assert!(
            callback_fields("https://user@app.flow-like.com/desktop/callback?state=a").is_err()
        );
        let redirect = "https://app.flow-like.com/desktop/callback";
        assert!(callback_matches(
            &reqwest::Url::parse("flow-like://auth?state=a&code=b").unwrap(),
            redirect
        ));
        assert!(!callback_matches(
            &reqwest::Url::parse("https://attacker.test/desktop/callback?state=a").unwrap(),
            redirect
        ));
        assert!(!callback_matches(
            &reqwest::Url::parse("https://app.flow-like.com/another?state=a").unwrap(),
            redirect
        ));
    }

    #[test]
    fn identity_verification_checks_signature_issuer_audience_expiry_and_key_intent() {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, jwk::JwkSet};
        // Public test key used by jsonwebtoken's Ed25519 interoperability fixtures.
        let key=EncodingKey::from_ed_pem(b"-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n-----END PRIVATE KEY-----\n").unwrap();
        let jwk = json!({"kty":"OKP","use":"sig","crv":"Ed25519","x":"2-Jj2UvNCvQiUPNYRgSi0cJSPiJI6Rs6D0UTeEpQVj8","kid":"test","alg":"EdDSA"});
        let keys: JwkSet = serde_json::from_value(json!({"keys":[jwk.clone()]})).unwrap();
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some("test".into());
        let claims = json!({"sub":"alice","iss":"https://issuer.test","aud":"client","iat":now_seconds(),"exp":now_seconds()+3600});
        let token = jsonwebtoken::encode(&header, &claims, &key).unwrap();
        assert!(verify_with_keys(&config(), &token, &keys, true).is_ok());
        for (field, value) in [
            ("iss", json!("https://attacker.test")),
            ("aud", json!("another-client")),
            ("exp", json!(1)),
            ("iat", json!(now_seconds() + 600)),
        ] {
            let mut changed = claims.clone();
            changed[field] = value;
            let token = jsonwebtoken::encode(&header, &changed, &key).unwrap();
            assert!(
                verify_with_keys(&config(), &token, &keys, true).is_err(),
                "{field}"
            );
        }
        let mut encryption_key = jwk.clone();
        encryption_key["use"] = json!("enc");
        let keys = serde_json::from_value(json!({"keys":[encryption_key]})).unwrap();
        assert!(verify_with_keys(&config(), &token, &keys, true).is_err());
        let mut expired = claims;
        expired["exp"] = json!(1);
        let expired = jsonwebtoken::encode(&header, &expired, &key).unwrap();
        let keys = serde_json::from_value(json!({"keys":[jwk]})).unwrap();
        assert!(verify_with_keys(&config(), &expired, &keys, false).is_ok());
    }
}
