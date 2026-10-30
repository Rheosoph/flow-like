//! Saved provider grants and app consent used by native workflow execution.
use super::{SessionBinding, canonical_hub, session, store};
use flow_like::flow::{compiled::prerun::PrerunOAuthRequirement, oauth::OAuthToken};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Arc, LazyLock, Weak},
};
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

#[derive(Clone, Default, Deserialize, Serialize)]
pub(crate) struct Snapshot {
    tokens: Vec<Value>,
    consents: Vec<Value>,
}

static LOCKS: LazyLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> = LazyLock::new(Mutex::default);

async fn scope_lock(key: &str) -> Result<Arc<Mutex<()>>, String> {
    let mut locks = LOCKS.lock().await;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    if locks.len() >= 256 {
        return Err("Too many concurrent OAuth account operations".into());
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key.to_owned(), Arc::downgrade(&lock));
    Ok(lock)
}

fn key(binding: &SessionBinding) -> String {
    json!([
        "provider-oauth-v2",
        binding.profile_id,
        binding.hub,
        binding.issuer,
        binding.subject
    ])
    .to_string()
}

fn binding_matches(
    value: &Value,
    hub: &str,
    subject: &str,
    issuer: &str,
    profile_id: &str,
) -> bool {
    let Some(raw) = value.get("sessionKey").and_then(Value::as_str) else {
        return false;
    };
    let Ok(parts) = serde_json::from_str::<Vec<String>>(raw) else {
        return false;
    };
    parts.len() == 4
        && canonical_hub(&parts[0]).ok().as_deref() == Some(hub)
        && parts[1] == issuer
        && parts[2] == subject
        && parts[3] == profile_id
}

fn validate(snapshot: &Snapshot, binding: &SessionBinding) -> Result<(), String> {
    if snapshot.tokens.len() > 256 || snapshot.consents.len() > 2048 {
        return Err("Too many saved OAuth connections or consents".into());
    }
    let mut token_ids = HashSet::new();
    for token in &snapshot.tokens {
        let owner = token
            .get("binding")
            .ok_or("OAuth connection has no account binding")?;
        if !binding_matches(
            owner,
            &binding.hub,
            &binding.subject,
            &binding.issuer,
            &binding.profile_id,
        ) || token
            .get("hub_subject")
            .and_then(Value::as_str)
            .is_some_and(|sub| sub != binding.subject)
            || token
                .get("hub_origin")
                .and_then(Value::as_str)
                .is_some_and(|hub| canonical_hub(hub).ok().as_deref() != Some(binding.hub.as_str()))
        {
            return Err("The OAuth connection belongs to another account or hub".into());
        }
        let session_key = owner
            .get("sessionKey")
            .and_then(Value::as_str)
            .ok_or("OAuth connection has no account binding")?;
        let provider_id = nonempty(token, "providerId", 256)?;
        let provider_key = nonempty(owner, "providerKey", 16 * 1024)?;
        validate_provider_key(provider_key, provider_id)?;
        let credential_key = json!([session_key, provider_key]).to_string();
        if !token_ids.insert(credential_key.clone()) {
            return Err("Duplicate saved OAuth connection".into());
        }
        if token.get("id").and_then(Value::as_str) != Some(credential_key.as_str())
            || token.get("sessionKey").and_then(Value::as_str) != Some(session_key)
            || canonical_hub(nonempty(owner, "apiBaseUrl", 4096)?)
                .ok()
                .as_deref()
                != Some(binding.hub.as_str())
        {
            return Err("OAuth connection metadata does not match its account binding".into());
        }
        let access = nonempty(token, "access_token", 64 * 1024)?;
        if !access.bytes().all(|byte| byte.is_ascii_graphic())
            || token
                .get("token_type")
                .filter(|value| !value.is_null())
                .is_some_and(|kind| {
                    kind.as_str()
                        .is_none_or(|kind| !kind.eq_ignore_ascii_case("Bearer"))
                })
        {
            return Err("Invalid OAuth access token".into());
        }
        validate_scopes(token)?;
        if token
            .get("refresh_token")
            .and_then(Value::as_str)
            .is_some_and(|grant| {
                !grant.starts_with("fl_oauth_")
                    || grant.len() > 4096
                    || !grant.bytes().all(|byte| byte.is_ascii_graphic())
            })
        {
            return Err("Reconnect this OAuth provider to use its Hub-owned refresh grant".into());
        }
    }
    if snapshot.consents.iter().any(|consent| {
        !binding_matches(
            consent,
            &binding.hub,
            &binding.subject,
            &binding.issuer,
            &binding.profile_id,
        )
    }) {
        return Err("OAuth consent belongs to another account or hub".into());
    }
    let mut consent_ids = HashSet::new();
    for consent in &snapshot.consents {
        let app_id = nonempty(consent, "appId", 256)?;
        let provider_id = nonempty(consent, "providerId", 256)?;
        let credential_key = nonempty(consent, "credentialKey", 32 * 1024)?;
        if !consent_ids.insert(json!([app_id, credential_key]).to_string()) {
            return Err("Duplicate saved OAuth consent".into());
        }
        let parts: Vec<String> = serde_json::from_str(credential_key)
            .map_err(|_| "Invalid OAuth consent credential binding")?;
        if parts.len() != 2
            || Some(parts[0].as_str()) != consent.get("sessionKey").and_then(Value::as_str)
            || consent.get("id").and_then(Value::as_str)
                != Some(json!([app_id, credential_key]).to_string().as_str())
        {
            return Err("OAuth consent metadata does not match its account binding".into());
        }
        validate_provider_key(&parts[1], provider_id)?;
        validate_scopes(consent)?;
    }
    Ok(())
}

/// Reconcile this operation's changes without importing unrelated stale browser rows.
fn merge_snapshot(
    existing: Snapshot,
    previous: &Snapshot,
    next: &Snapshot,
) -> Result<Snapshot, String> {
    fn rows(values: &[Value]) -> Result<BTreeMap<String, Value>, String> {
        values
            .iter()
            .map(|value| Ok((nonempty(value, "id", 64 * 1024)?.to_owned(), value.clone())))
            .collect()
    }
    fn merge(existing: &[Value], previous: &[Value], next: &[Value]) -> Result<Vec<Value>, String> {
        let mut saved = rows(existing)?;
        let previous = rows(previous)?;
        let next = rows(next)?;
        for id in previous.keys().filter(|id| !next.contains_key(*id)) {
            saved.remove(id);
        }
        for (id, value) in next {
            if previous.get(&id) != Some(&value) {
                saved.insert(id, value);
            }
        }
        Ok(saved.into_values().collect())
    }
    Ok(Snapshot {
        tokens: merge(&existing.tokens, &previous.tokens, &next.tokens)?,
        consents: merge(&existing.consents, &previous.consents, &next.consents)?,
    })
}

fn nonempty<'a>(value: &'a Value, name: &str, limit: usize) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
        })
        .ok_or_else(|| format!("Invalid OAuth {name}"))
}

fn validate_provider_key(raw: &str, provider_id: &str) -> Result<(), String> {
    let parts: Vec<String> =
        serde_json::from_str(raw).map_err(|_| "Invalid OAuth provider binding")?;
    if parts.len() != 4 || parts[0] != provider_id || parts[1].len() > 1024 {
        return Err("Invalid OAuth provider binding".into());
    }
    super::secure_url(&parts[2])?;
    super::secure_url(&parts[3])?;
    Ok(())
}

fn validate_scopes(value: &Value) -> Result<(), String> {
    let scopes = value
        .get("scopes")
        .and_then(Value::as_array)
        .ok_or("Missing OAuth scopes")?;
    if scopes.len() > 256
        || scopes.iter().any(|scope| {
            scope.as_str().is_none_or(|scope| {
                scope.is_empty() || scope.len() > 1024 || scope.chars().any(char::is_control)
            })
        })
    {
        return Err("Invalid OAuth scopes".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn native_oauth_sync(
    profile_id: String,
    hub: String,
    generation: String,
    snapshot: Snapshot,
    previous: Option<Snapshot>,
    initialize_only: Option<bool>,
) -> Result<(), String> {
    tokio::spawn(async move {
        let initialize_only = initialize_only.unwrap_or(false);
        if initialize_only == previous.is_some() {
            return Err(
                "OAuth persistence requires initialization or an operation's previous snapshot"
                    .into(),
            );
        }
        let binding = SessionBinding::capture(&profile_id, &hub)
            .await?
            .ok_or("Sign in to save OAuth connections")?;
        if binding.generation != generation {
            return Err("The authentication session changed".into());
        }
        let key = key(&binding);
        let lock = scope_lock(&key).await?;
        let _guard = lock.lock().await;
        let existing = store::read(&key).await?;
        if initialize_only && existing.is_some() {
            return Ok(());
        }
        validate(&snapshot, &binding)?;
        let snapshot = match (previous.as_ref(), existing) {
            (Some(previous), Some(existing)) => {
                validate(previous, &binding)?;
                let existing: Snapshot = serde_json::from_slice(&existing)
                    .map_err(|_| "Invalid saved OAuth connections")?;
                validate(&existing, &binding)?;
                merge_snapshot(existing, previous, &snapshot)?
            }
            (Some(previous), None) => {
                validate(previous, &binding)?;
                snapshot
            }
            (None, _) => snapshot,
        };
        validate(&snapshot, &binding)?;
        let bytes = serde_json::to_vec(&snapshot).map_err(|_| "Cannot encode OAuth connections")?;
        if bytes.len() > 480 * 1024 {
            return Err("Saved OAuth connections exceed the storage limit".into());
        }
        // Always acquire the OAuth lock first: resolution may refresh Hub auth
        // while holding it, so the opposite order would deadlock with this sync.
        let (session_key, session) = session(&profile_id, &hub).await?;
        let mut session = session.lock().await;
        if session.load(&session_key).await?.generation != generation {
            return Err("The authentication session changed".into());
        }
        store::write(&key, bytes).await
    })
    .await
    .map_err(|_| "OAuth persistence worker failed")?
}

fn scopes(value: &Value) -> Vec<String> {
    value
        .get("scopes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn has_scopes(provider: &str, granted: &[String], required: &[String]) -> bool {
    fn normalized<'a>(provider: &str, scope: &'a str) -> &'a str {
        match (provider, scope) {
            ("google", "https://www.googleapis.com/auth/userinfo.email") => "email",
            ("google", "https://www.googleapis.com/auth/userinfo.profile") => "profile",
            _ => scope,
        }
    }
    required.iter().all(|scope| {
        granted
            .iter()
            .any(|grant| normalized(provider, grant) == normalized(provider, scope))
    })
}

pub(crate) async fn refresh_provider(
    binding: &SessionBinding,
    client: &reqwest::Client,
    provider: &str,
    token: &OAuthToken,
    required_scopes: &[String],
) -> Result<OAuthToken, String> {
    let refresh = token
        .refresh_token
        .as_deref()
        .ok_or("OAuth refresh grant is missing")?;
    if !refresh.starts_with("fl_oauth_") || refresh.len() > 4096 {
        return Err("Reconnect this OAuth provider to use its Hub-owned refresh grant".into());
    }
    let url = flow_like::hub::hub_api_url(&binding.hub, true, &["oauth", "refresh", provider])
        .map_err(|_| "Invalid OAuth Hub URL")?;
    let mut authorization = binding.access_token(None).await?;
    for attempt in 0..2 {
        if !binding.is_current() {
            return Err("The authentication session changed".into());
        }
        let response = client
            .post(url.clone())
            .bearer_auth(&authorization)
            .json(&json!({"refresh_token":refresh,"scope":null}))
            .send()
            .await
            .map_err(|_| "OAuth renewal is temporarily unavailable")?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
            authorization = binding.access_token(Some(&authorization)).await?;
            continue;
        }
        if !response.status().is_success() {
            return Err(
                if response.status().is_server_error()
                    || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                {
                    "OAuth renewal is temporarily unavailable".into()
                } else {
                    format!("Reconnect OAuth provider '{provider}' before running this workflow")
                },
            );
        }
        let response = super::response_json(response).await?;
        if !binding.is_current() {
            return Err("The authentication session changed".into());
        }
        if let Some(returned_scopes) = response.get("scope").and_then(Value::as_str)
            && !has_scopes(
                provider,
                &returned_scopes
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>(),
                required_scopes,
            )
        {
            return Err(format!(
                "Reconnect OAuth provider '{provider}' to authorize the required scopes"
            ));
        }
        let access_token = response
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| {
                !token.is_empty()
                    && token.len() <= 64 * 1024
                    && token.bytes().all(|byte| byte.is_ascii_graphic())
            })
            .ok_or("OAuth renewal returned an invalid access token")?;
        if response
            .get("refresh_token")
            .and_then(Value::as_str)
            .is_some_and(|returned| returned != refresh)
        {
            return Err(
                "The Hub returned a different canonical OAuth grant; reconnect the provider".into(),
            );
        }
        let token_type = response
            .get("token_type")
            .and_then(Value::as_str)
            .unwrap_or("Bearer");
        if !token_type.eq_ignore_ascii_case("Bearer") {
            return Err("Unsupported OAuth token type".into());
        }
        return Ok(OAuthToken {
            access_token: access_token.to_owned(),
            refresh_token: Some(refresh.to_owned()),
            expires_at: response
                .get("expires_in")
                .and_then(Value::as_u64)
                .map(|duration| super::now_seconds().saturating_add(duration)),
            token_type: Some(token_type.to_owned()),
            hub_origin: Some(binding.hub.clone()),
            hub_subject: Some(binding.subject.clone()),
        });
    }
    Err("The Hub rejected the renewed authentication session".into())
}

pub(crate) async fn resolve(
    app: &AppHandle,
    binding: Option<&SessionBinding>,
    app_id: &str,
    requirements: &[PrerunOAuthRequirement],
) -> Result<HashMap<String, OAuthToken>, String> {
    if requirements.is_empty() {
        return Ok(HashMap::new());
    }
    let binding = binding.ok_or("Sign in and authorize the workflow's OAuth providers")?;
    let key = key(binding);
    let lock = scope_lock(&key).await?;
    let _guard = lock.lock().await;
    let snapshot = match store::read(&key).await? {
        Some(bytes) => serde_json::from_slice::<Snapshot>(&bytes)
            .map_err(|_| "Invalid saved OAuth connections")?,
        None => Snapshot::default(),
    };
    validate(&snapshot, binding)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| "Cannot initialize OAuth client")?;
    let base = if binding.hub.ends_with("/api/v1") {
        binding.hub.clone()
    } else {
        format!("{}/api/v1", binding.hub)
    };
    let response = client
        .get(&base)
        .send()
        .await
        .map_err(|_| "OAuth provider configuration is unavailable")?;
    if !response.status().is_success() {
        return Err("OAuth provider configuration was refused".into());
    }
    let config = super::response_json(response).await?;
    let mut resolved = HashMap::new();
    let mut missing = Vec::new();
    for requirement in requirements {
        let id = &requirement.provider_id;
        let Some(provider) = config
            .get("oauth_providers")
            .and_then(|providers| providers.get(id))
        else {
            missing.push(id.clone());
            continue;
        };
        let provider_key = json!([
            id,
            provider
                .get("client_id")
                .and_then(Value::as_str)
                .unwrap_or(""),
            provider["auth_url"],
            provider["token_url"]
        ])
        .to_string();
        let mut required = requirement.scopes.clone();
        required.extend(scopes(provider));
        let token = snapshot.tokens.iter().find(|token| {
            token.get("providerId").and_then(Value::as_str) == Some(id)
                && token
                    .pointer("/binding/providerKey")
                    .and_then(Value::as_str)
                    == Some(provider_key.as_str())
                && has_scopes(id, &scopes(token), &required)
                && snapshot.consents.iter().any(|consent| {
                    consent.get("appId").and_then(Value::as_str) == Some(app_id)
                        && consent.get("providerId").and_then(Value::as_str) == Some(id)
                        && consent.get("credentialKey").and_then(Value::as_str)
                            == Some(
                                json!([token.pointer("/binding/sessionKey"), provider_key])
                                    .to_string()
                                    .as_str(),
                            )
                        && has_scopes(id, &scopes(consent), &required)
                })
        });
        let Some(token) = token else {
            missing.push(id.clone());
            continue;
        };
        let mut token: OAuthToken =
            serde_json::from_value(token.clone()).map_err(|_| "Invalid saved OAuth connection")?;
        token.hub_origin = Some(binding.hub.clone());
        token.hub_subject = Some(binding.subject.clone());
        if token.refresh_token.is_some() {
            token = refresh_provider(binding, &client, id, &token, &required).await?;
        } else if token.is_expired() {
            missing.push(id.clone());
            continue;
        }
        resolved.insert(id.clone(), token);
    }
    if !binding.is_current() {
        return Err("The authentication session changed".into());
    }
    if !missing.is_empty() {
        let _ = app.emit(
            "flowpilot://oauth-required",
            json!({"profileId":binding.profile_id,"appId":app_id,"providerIds":missing}),
        );
        return Err(format!(
            "OAuth authorization and app consent are required for: {}. Open the app's connection settings, then retry.",
            missing.join(", ")
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_scopes_and_account_binding_are_checked() {
        assert!(has_scopes(
            "google",
            &["https://www.googleapis.com/auth/userinfo.email".into()],
            &["email".into()]
        ));
        assert!(!has_scopes("other", &["read".into()], &["write".into()]));
        let owner = json!({"sessionKey":json!(["https://hub.example","issuer","alice","profile"]).to_string()});
        assert!(binding_matches(
            &owner,
            "https://hub.example",
            "alice",
            "issuer",
            "profile"
        ));
        assert!(!binding_matches(
            &owner,
            "https://hub.example",
            "bob",
            "issuer",
            "profile"
        ));
        assert!(!binding_matches(
            &owner,
            "https://other.example",
            "alice",
            "issuer",
            "profile"
        ));
        assert!(!binding_matches(
            &owner,
            "https://hub.example",
            "alice",
            "other-issuer",
            "profile"
        ));
        assert!(!binding_matches(
            &owner,
            "https://hub.example",
            "alice",
            "issuer",
            "other-profile"
        ));
        let legacy =
            json!({"sessionKey":json!(["https://hub.example","issuer","alice"]).to_string()});
        assert!(!binding_matches(
            &legacy,
            "https://hub.example",
            "alice",
            "issuer",
            "profile"
        ));
    }

    fn connection() -> (SessionBinding, Snapshot) {
        let binding = SessionBinding {
            profile_id: "profile".into(),
            hub: "https://hub.example".into(),
            issuer: "issuer".into(),
            subject: "alice".into(),
            generation: "generation".into(),
            revoked: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let session = json!([
            binding.hub,
            binding.issuer,
            binding.subject,
            binding.profile_id
        ])
        .to_string();
        let provider = json!([
            "provider",
            "client",
            "https://provider.example/authorize",
            "https://provider.example/token"
        ])
        .to_string();
        let credential = json!([session, provider]).to_string();
        let token = json!({"id":credential,"sessionKey":session,"providerId":"provider","binding":{"sessionKey":session,"providerKey":provider,"apiBaseUrl":binding.hub},"access_token":"access","refresh_token":"fl_oauth_saved","token_type":"Bearer","scopes":["read"]});
        let consent = json!({"id":json!(["app",credential]).to_string(),"sessionKey":session,"appId":"app","providerId":"provider","credentialKey":credential,"scopes":["read"]});
        (
            binding,
            Snapshot {
                tokens: vec![token],
                consents: vec![consent],
            },
        )
    }

    #[test]
    fn saved_connections_and_consent_require_consistent_ownership() {
        let (binding, snapshot) = connection();
        assert!(validate(&snapshot, &binding).is_ok());
        for pointer in [
            "/id",
            "/sessionKey",
            "/binding/apiBaseUrl",
            "/binding/providerKey",
            "/providerId",
            "/refresh_token",
        ] {
            let mut changed = snapshot.clone();
            *changed.tokens[0].pointer_mut(pointer).unwrap() = json!("another-owner");
            assert!(validate(&changed, &binding).is_err(), "{pointer}");
        }
        for pointer in ["/id", "/appId", "/providerId", "/credentialKey"] {
            let mut changed = snapshot.clone();
            *changed.consents[0].pointer_mut(pointer).unwrap() = json!("another-owner");
            assert!(validate(&changed, &binding).is_err(), "{pointer}");
        }
        let mut other_profile = binding.clone();
        other_profile.profile_id = "other-profile".into();
        assert_ne!(key(&binding), key(&other_profile));
        assert!(validate(&snapshot, &other_profile).is_err());
    }

    #[tokio::test]
    async fn oauth_operations_share_a_lock_only_for_the_same_account_scope() {
        let first = scope_lock("first").await.unwrap();
        let same = scope_lock("first").await.unwrap();
        let other = scope_lock("second").await.unwrap();
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn browser_deltas_preserve_native_authority_for_unchanged_rows() {
        let (_, previous) = connection();
        let mut native = previous.clone();
        native.tokens[0]["access_token"] = json!("native-current");
        let mut browser = previous.clone();
        // A stale browser row is not an instruction to replace a native grant.
        browser.consents.clear();
        let merged = merge_snapshot(native, &previous, &browser).unwrap();
        assert_eq!(merged.tokens[0]["access_token"], "native-current");
        assert!(merged.consents.is_empty());
        // A rejected earlier browser mutation stays absent through unrelated changes.
        let mut absent = previous.clone();
        absent.tokens.clear();
        let merged = merge_snapshot(absent, &previous, &browser).unwrap();
        assert!(merged.tokens.is_empty());
        // An explicit provider update is the only way to replace its saved row.
        browser.tokens[0]["access_token"] = json!("explicit-update");
        let merged = merge_snapshot(previous.clone(), &previous, &browser).unwrap();
        assert_eq!(merged.tokens[0]["access_token"], "explicit-update");
    }
}
