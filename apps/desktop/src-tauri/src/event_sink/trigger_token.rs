//! The account a trigger on this device signs in as, and the check that a
//! personal access token belongs to it.

use anyhow::{Context, Result, anyhow, ensure};
use flow_like::flow::execution::UserExecutionContext;
use std::time::Duration;
use tauri::AppHandle;

const EXECUTE_EVENTS_PERMISSION: i64 = 0x2000;
/// PAT levels the hub lets start runs: Read & Write and Admin.
const RUN_CAPABLE_PAT_LEVELS: [i64; 2] = [2, 4];
const RESPONSE_LIMIT: usize = 1024 * 1024;
/// Starts the refusal of another account's token, so the webview can tell it
/// apart from other refusals without matching the wording.
const WRONG_ACCOUNT_CODE: &str = "TRIGGER_TOKEN_WRONG_ACCOUNT:";

/// The signed-in subject on the current profile's hub. A trigger's PAT counts
/// as this account's own only when it was verified against it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TriggerAccount {
    pub profile_id: String,
    pub issuer: String,
    pub sub: String,
    pub hub_origin: String,
}

/// A saved trigger keeps its profile and account when another profile opens in the UI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TriggerProfile {
    pub profile_id: String,
    pub hub_origin: String,
    pub issuer: Option<String>,
    pub subject: Option<String>,
}

impl TriggerProfile {
    pub(crate) async fn capture(profile: &flow_like::profile::Profile) -> Result<Self> {
        let hub_origin =
            flow_like::hub::hub_origin(&profile.hub, profile.secure).unwrap_or_default();
        let auth = crate::native_auth::SessionBinding::capture(&profile.id, &hub_origin)
            .await
            .map_err(|error| anyhow!(error))?;
        Ok(Self {
            profile_id: profile.id.clone(),
            hub_origin,
            issuer: auth.as_ref().map(|auth| auth.issuer.clone()),
            subject: auth.as_ref().map(|auth| auth.subject.clone()),
        })
    }

    pub(crate) fn account(&self) -> Option<TriggerAccount> {
        Some(TriggerAccount {
            profile_id: self.profile_id.clone(),
            issuer: self.issuer.clone()?,
            sub: self.subject.clone()?,
            hub_origin: self.hub_origin.clone(),
        })
    }

    fn matches(
        &self,
        profile: &flow_like::profile::Profile,
        account: Option<&TriggerAccount>,
    ) -> bool {
        !self.profile_id.is_empty()
            && profile.id == self.profile_id
            && flow_like::hub::hub_origin(&profile.hub, profile.secure).unwrap_or_default()
                == self.hub_origin
            && self.issuer.is_some() == self.subject.is_some()
            && self.account().as_ref() == account
    }

    pub(crate) async fn resolve(
        &self,
        app: &AppHandle,
    ) -> Result<(
        crate::profile::UserProfile,
        Option<crate::native_auth::SessionBinding>,
    )> {
        let settings = crate::state::TauriSettingsState::construct(app).await?;
        let settings = settings.lock().await;
        let profile = settings.profile_by_id(&self.profile_id)?;
        let auth = crate::native_auth::SessionBinding::capture(&self.profile_id, &self.hub_origin)
            .await
            .map_err(|error| anyhow!(error))?;
        let account = auth.as_ref().map(TriggerAccount::from_binding);
        ensure!(
            self.matches(&profile.hub_profile, account.as_ref()),
            "The trigger's saved profile, hub, or account changed. Sign in to that profile or enable the trigger again."
        );
        Ok((profile, auth))
    }
}

pub(crate) fn execution_authority(
    auth: Option<crate::native_auth::SessionBinding>,
    token: Option<&str>,
) -> Result<Option<crate::native_auth::SessionBinding>> {
    ensure!(
        auth.as_ref().is_none_or(|auth| auth.is_current()),
        "The trigger account changed"
    );
    let token = token.map(|token| token.strip_prefix("Bearer ").unwrap_or(token));
    if token.is_some_and(|token| token.starts_with("pat_")) {
        return Ok(None);
    }
    if let Some(token) = token {
        let subject = flow_like::flow::execution::extract_sub_from_jwt(token)?;
        ensure!(
            auth.as_ref().is_some_and(|auth| auth.subject == subject),
            "The registered execution token belongs to another or signed-out profile account"
        );
    }
    Ok(auth)
}

/// The hub refused a trigger's PAT itself: revoked, expired or unknown.
#[derive(Debug)]
pub struct TokenRefused(pub String);
impl std::fmt::Display for TokenRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for TokenRefused {}

impl TriggerAccount {
    fn from_binding(binding: &crate::native_auth::SessionBinding) -> Self {
        Self {
            profile_id: binding.profile_id.clone(),
            issuer: binding.issuer.clone(),
            sub: binding.subject.clone(),
            hub_origin: binding.hub.clone(),
        }
    }

    pub(crate) fn profile_binding(&self) -> TriggerProfile {
        TriggerProfile {
            profile_id: self.profile_id.clone(),
            hub_origin: self.hub_origin.clone(),
            issuer: Some(self.issuer.clone()),
            subject: Some(self.sub.clone()),
        }
    }
    /// `None` while nobody is signed in or the profile names no hub.
    pub async fn current(app_handle: &AppHandle) -> Option<Self> {
        let profile = crate::state::TauriSettingsState::current_profile(app_handle)
            .await
            .ok()?;
        Self::signed_in_to(app_handle, &profile.hub_profile).await
    }

    /// The signed-in subject on `hub`, the hub of a profile read once for a
    /// run, so the run talks to the hub its account was checked against.
    pub async fn signed_in_to(
        _app_handle: &AppHandle,
        hub: &flow_like::profile::Profile,
    ) -> Option<Self> {
        let hub_origin = flow_like::hub::hub_origin(&hub.hub, hub.secure)?;
        let binding = crate::native_auth::SessionBinding::capture(&hub.id, &hub_origin)
            .await
            .ok()??;
        Some(Self::from_binding(&binding))
    }
}

pub(crate) fn hub_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(18))
        .build()?)
}

/// PATs are sent as they are, every other token as a bearer token.
pub(crate) fn authorization(token: &str) -> String {
    if token.starts_with("pat_") || token.starts_with("Bearer ") {
        token.into()
    } else {
        format!("Bearer {token}")
    }
}

pub(crate) async fn read_body(mut response: reqwest::Response) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= RESPONSE_LIMIT,
            "Hub response exceeds 1 MiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Resolves who owns `pat` in `app_id` and accepts it only when that is the
/// signed-in account on the current hub, its role may run events and the
/// token's level may start runs. Returns the account the PAT was verified for.
pub async fn verify_token_owner(
    app_handle: &AppHandle,
    app_id: &str,
    pat: &str,
) -> Result<TriggerAccount> {
    let profile = crate::state::TauriSettingsState::current_profile(app_handle).await?;
    let account = TriggerAccount::signed_in_to(app_handle, &profile.hub_profile)
        .await
        .ok_or_else(|| anyhow!("Sign in to the hub before choosing a token for this trigger"))?;
    verify_token_for_profile(&profile.hub_profile, &account, app_id, pat).await
}

/// Keep a save's token verification on the profile captured before its first await.
pub(crate) async fn verify_token_for_profile(
    profile: &flow_like::profile::Profile,
    account: &TriggerAccount,
    app_id: &str,
    pat: &str,
) -> Result<TriggerAccount> {
    ensure!(
        profile.id == account.profile_id
            && flow_like::hub::hub_origin(&profile.hub, profile.secure).as_deref()
                == Some(account.hub_origin.as_str()),
        "The trigger token verification profile changed"
    );
    let identity = token_identity(profile, app_id, pat).await?;
    accept_owner(&identity, account)?;
    if let Some(id) = pat_id(pat) {
        accept_level(token_level(profile, pat, id).await?)?;
    }
    Ok(account.clone())
}

/// Who `pat` is in `app_id`, as the hub answers it.
async fn token_identity(
    hub: &flow_like::profile::Profile,
    app_id: &str,
    pat: &str,
) -> Result<UserExecutionContext> {
    let url =
        flow_like::hub::hub_api_url(&hub.hub, hub.secure, &["apps", app_id, "invoke", "context"])?;
    let response = hub_client()?
        .get(url)
        .header("Authorization", authorization(pat.trim()))
        .send()
        .await
        .context("The hub could not be reached to check the personal access token")?;
    let status = response.status();
    ensure!(
        status != reqwest::StatusCode::UNAUTHORIZED,
        "The hub does not accept this personal access token; it may be revoked or expired"
    );
    ensure!(
        status.is_success(),
        "The hub refused this personal access token for this app ({status})"
    );
    serde_json::from_slice(&read_body(response).await?)
        .context("The hub answered the token check with an unexpected response")
}

/// The id of a `pat_<id>.<secret>` token, if `token` is one.
fn pat_id(token: &str) -> Option<&str> {
    let token = token.trim();
    token
        .strip_prefix("Bearer ")
        .unwrap_or(token)
        .strip_prefix("pat_")?
        .split_once('.')
        .map(|(id, _)| id)
        .filter(|id| !id.is_empty())
}

/// The level of the PAT `id`, read from its own account's token list, which a
/// Read Only token may list too.
async fn token_level(hub: &flow_like::profile::Profile, pat: &str, id: &str) -> Result<i64> {
    let url = flow_like::hub::hub_api_url(&hub.hub, hub.secure, &["user", "pat"])?;
    let response = hub_client()?
        .get(url)
        .header("Authorization", authorization(pat.trim()))
        .send()
        .await
        .context("The hub could not be reached to check the personal access token's level")?;
    let status = response.status();
    ensure!(
        status.is_success(),
        "The hub refused to list this personal access token's level ({status})"
    );
    listed_level(&read_body(response).await?, id)
}

#[derive(serde::Deserialize)]
struct ListedToken {
    id: String,
    permissions: i64,
}

fn listed_level(body: &[u8], id: &str) -> Result<i64> {
    let tokens: Vec<ListedToken> = serde_json::from_slice(body)
        .context("The hub answered the token level check with an unexpected response")?;
    tokens
        .into_iter()
        .find(|token| token.id == id)
        .map(|token| token.permissions)
        .ok_or_else(|| anyhow!("The hub does not list this personal access token for its account"))
}

fn accept_level(level: i64) -> Result<()> {
    ensure!(
        RUN_CAPABLE_PAT_LEVELS.contains(&level),
        "This personal access token is Read Only, so the hub refuses the runs a trigger starts with it; choose a Read & Write token"
    );
    Ok(())
}

fn accept_owner(identity: &UserExecutionContext, account: &TriggerAccount) -> Result<()> {
    ensure!(
        !identity.is_technical_user && identity.sub == account.sub,
        "{WRONG_ACCOUNT_CODE} This personal access token belongs to another account; choose one of your own"
    );
    ensure!(
        identity.has_permission(EXECUTE_EVENTS_PERMISSION),
        "Your role in this app cannot run events, so a trigger cannot sign in with your token"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_token_is_never_verified_against_a_different_profile_or_hub() {
        let profile = flow_like::profile::Profile {
            id: "profile-1".into(),
            hub: "https://hub.example".into(),
            secure: true,
            ..Default::default()
        };
        let account = TriggerAccount {
            profile_id: profile.id.clone(),
            issuer: "https://issuer.example".into(),
            sub: "user-1".into(),
            hub_origin: "https://hub.example".into(),
        };
        for foreign in [
            TriggerAccount {
                profile_id: "profile-2".into(),
                ..account.clone()
            },
            TriggerAccount {
                hub_origin: "https://other.example".into(),
                ..account.clone()
            },
        ] {
            let error = verify_token_for_profile(&profile, &foreign, "app", "pat_never_send")
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "The trigger token verification profile changed"
            );
        }
    }

    #[test]
    fn a_saved_trigger_accepts_only_its_exact_profile_hub_issuer_and_account() {
        let profile = flow_like::profile::Profile {
            id: "profile-1".into(),
            hub: "https://hub.example".into(),
            secure: true,
            ..Default::default()
        };
        let account = TriggerAccount {
            profile_id: profile.id.clone(),
            issuer: "https://issuer.example".into(),
            sub: "user-1".into(),
            hub_origin: "https://hub.example".into(),
        };
        let saved = account.profile_binding();
        assert!(saved.matches(&profile, Some(&account)));
        assert!(!saved.matches(&profile, None));
        for foreign in [
            TriggerAccount {
                profile_id: "profile-2".into(),
                ..account.clone()
            },
            TriggerAccount {
                issuer: "https://other-issuer.example".into(),
                ..account.clone()
            },
            TriggerAccount {
                sub: "user-2".into(),
                ..account.clone()
            },
            TriggerAccount {
                hub_origin: "https://other-hub.example".into(),
                ..account.clone()
            },
        ] {
            assert!(!saved.matches(&profile, Some(&foreign)), "{foreign:?}");
        }
        let moved = flow_like::profile::Profile {
            hub: "https://other-hub.example".into(),
            ..profile.clone()
        };
        assert!(!saved.matches(&moved, Some(&account)));
        let renamed = flow_like::profile::Profile {
            name: "New display name".into(),
            ..profile.clone()
        };
        assert!(saved.matches(&renamed, Some(&account)));
        let anonymous = TriggerProfile {
            issuer: None,
            subject: None,
            ..saved
        };
        assert!(anonymous.matches(&profile, None));
        assert!(!anonymous.matches(&profile, Some(&account)));
    }
    use flow_like::flow::execution::RoleContext;

    fn identity(sub: &str, permissions: i64, technical: bool) -> UserExecutionContext {
        UserExecutionContext {
            sub: sub.to_string(),
            role: Some(RoleContext {
                permissions,
                ..RoleContext::default()
            }),
            is_technical_user: technical,
            ..UserExecutionContext::default()
        }
    }

    #[test]
    fn a_token_counts_only_for_its_signed_in_owner_who_may_run_events() {
        let account = TriggerAccount {
            profile_id: "profile-1".into(),
            issuer: "https://issuer.example".into(),
            sub: "user-1".to_string(),
            hub_origin: "https://hub.example".to_string(),
        };
        assert!(
            accept_owner(
                &identity("user-1", EXECUTE_EVENTS_PERMISSION, false),
                &account
            )
            .is_ok()
        );
        assert!(
            accept_owner(
                &identity("user-2", EXECUTE_EVENTS_PERMISSION, false),
                &account
            )
            .is_err()
        );
        assert!(
            accept_owner(
                &identity("user-1", EXECUTE_EVENTS_PERMISSION, true),
                &account
            )
            .is_err()
        );
        assert!(accept_owner(&identity("user-1", 0x100, false), &account).is_err());
    }

    #[test]
    fn only_another_accounts_token_is_refused_with_the_wrong_account_code() {
        let account = TriggerAccount {
            profile_id: "profile-1".into(),
            issuer: "https://issuer.example".into(),
            sub: "user-1".to_string(),
            hub_origin: "https://hub.example".to_string(),
        };
        let refusal = |sub: &str, permissions: i64, technical: bool| {
            accept_owner(&identity(sub, permissions, technical), &account)
                .unwrap_err()
                .to_string()
        };
        for wrong_account in [
            refusal("user-2", EXECUTE_EVENTS_PERMISSION, false),
            refusal("user-1", EXECUTE_EVENTS_PERMISSION, true),
        ] {
            assert!(
                wrong_account.starts_with("TRIGGER_TOKEN_WRONG_ACCOUNT: "),
                "{wrong_account}"
            );
        }
        assert!(!refusal("user-1", 0x100, false).contains("TRIGGER_TOKEN_WRONG_ACCOUNT"));
    }

    #[test]
    fn only_pats_have_an_id_to_check_the_level_of() {
        assert_eq!(pat_id("pat_abc.secret"), Some("abc"));
        assert_eq!(pat_id(" Bearer pat_abc.secret "), Some("abc"));
        assert_eq!(pat_id("pat_.secret"), None);
        assert_eq!(pat_id("pat_abc"), None);
        assert_eq!(pat_id("eyJhbGciOiJFUzI1NiJ9.e30.sig"), None);
    }

    #[test]
    fn a_read_only_token_cannot_start_runs() {
        let body = br#"[
            {"name":"ro","created_at":1,"valid_until":null,"permissions":1,"id":"ro"},
            {"name":"rw","created_at":1,"valid_until":null,"permissions":2,"id":"rw"},
            {"name":"admin","created_at":1,"valid_until":null,"permissions":4,"id":"admin"}
        ]"#;
        assert!(accept_level(listed_level(body, "ro").unwrap()).is_err());
        assert!(accept_level(listed_level(body, "rw").unwrap()).is_ok());
        assert!(accept_level(listed_level(body, "admin").unwrap()).is_ok());
        assert!(listed_level(body, "missing").is_err());
        assert!(listed_level(b"{}", "rw").is_err());
    }

    #[test]
    fn pats_are_sent_verbatim_and_other_tokens_as_bearer() {
        assert_eq!(authorization("pat_1.secret"), "pat_1.secret");
        assert_eq!(authorization("Bearer abc"), "Bearer abc");
        assert_eq!(authorization("abc"), "Bearer abc");
    }
}
