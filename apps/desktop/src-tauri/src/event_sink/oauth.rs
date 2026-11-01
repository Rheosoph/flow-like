use std::collections::HashMap;

use flow_like::flow::oauth::{OAuthToken, refresh_on_hub};
use flow_like_types::{Result, anyhow};

/// PAT registrations retain their limited grant. A JWT run captures this profile's
/// native account and cannot substitute another account for its registered token.
pub(crate) async fn capture_authority(
    profile: &flow_like::profile::Profile,
    execution_token: Option<&str>,
) -> Result<Option<crate::native_auth::SessionBinding>> {
    let token = execution_token.map(|token| token.strip_prefix("Bearer ").unwrap_or(token));
    if token.is_some_and(|token| token.starts_with("pat_")) {
        return Ok(None);
    }
    let Some(origin) = flow_like::hub::hub_origin(&profile.hub, profile.secure) else {
        return Ok(None);
    };
    let binding = crate::native_auth::SessionBinding::capture(&profile.id, &origin)
        .await
        .map_err(|error| anyhow!(error))?;
    if let Some(token) = token {
        let subject = flow_like::flow::execution::extract_sub_from_jwt(token)?;
        anyhow::ensure!(
            binding
                .as_ref()
                .is_some_and(|binding| binding.subject == subject),
            "The registered execution token belongs to another or signed-out profile account"
        );
    }
    Ok(binding)
}

/// Scheduled runs resolve saved grants just before execution, after profile selection.
pub(crate) async fn resolve_tokens(
    hub: &str,
    secure: bool,
    execution_token: Option<&str>,
    native_auth: Option<&crate::native_auth::SessionBinding>,
    tokens: &HashMap<String, OAuthToken>,
) -> Result<(HashMap<String, OAuthToken>, Option<String>)> {
    let renewable: Vec<_> = tokens
        .values()
        .filter(|token| token.refresh_token.is_some())
        .collect();
    if renewable.is_empty() {
        anyhow::ensure!(
            tokens.values().all(|token| !token.is_expired()),
            "An OAuth access token expired; reconnect the account"
        );
        return Ok((tokens.clone(), None));
    }
    let origin = flow_like::hub::hub_origin(hub, secure)
        .ok_or_else(|| anyhow!("OAuth execution requires a hub"))?;
    for token in &renewable {
        token.ensure_hub(&origin, secure)?;
    }
    let authorization = match execution_token
        .filter(|value| value.trim_start_matches("Bearer ").starts_with("pat_"))
    {
        Some(pat) => pat.to_owned(),
        None => {
            let subject = renewable[0].hub_subject.as_deref().ok_or_else(|| {
                anyhow!("Reconnect this OAuth account to bind its saved grant to an account")
            })?;
            anyhow::ensure!(
                renewable
                    .iter()
                    .all(|token| token.hub_subject.as_deref() == Some(subject)),
                "Saved OAuth grants belong to different Hub accounts"
            );
            let binding = native_auth.ok_or_else(|| {
                anyhow!("Sign in to the OAuth grant's Hub account before running this event")
            })?;
            anyhow::ensure!(
                binding.hub == origin && binding.subject == subject,
                "Saved OAuth grants belong to another native profile account"
            );
            binding
                .access_token(None)
                .await
                .map_err(|error| anyhow!(error))?
        }
    };
    let mut resolved = HashMap::with_capacity(tokens.len());
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    for (provider, token) in tokens {
        let next = if token.refresh_token.is_some() {
            if let Some(binding) = native_auth {
                crate::native_auth::oauth::refresh_provider(binding, &client, provider, token, &[])
                    .await
                    .map_err(|error| anyhow!(error))?
            } else {
                refresh_on_hub(&origin, secure, &authorization, provider, token, None).await?
            }
        } else {
            anyhow::ensure!(
                !token.is_expired(),
                "An OAuth access token expired; reconnect the account"
            );
            token.clone()
        };
        resolved.insert(provider.clone(), next);
    }
    Ok((resolved, Some(authorization)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn events_without_oauth_still_run_without_a_hub() {
        let (tokens, authorization) = resolve_tokens("", true, None, None, &HashMap::new())
            .await
            .unwrap();
        assert!(tokens.is_empty());
        assert!(authorization.is_none());
    }

    #[tokio::test]
    async fn another_hub_is_rejected_before_a_saved_pat_can_be_sent() {
        let mut token = OAuthToken::new("access".into());
        token.refresh_token = Some("fl_oauth_saved".into());
        token.hub_origin = Some("https://original.example".into());
        token.hub_subject = Some("alice".into());
        let error = resolve_tokens(
            "http://127.0.0.1:1",
            false,
            Some("pat_secret"),
            None,
            &HashMap::from([("provider".into(), token)]),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("another hub"));
    }

    #[tokio::test]
    async fn legacy_unbound_grants_require_reconnection_before_refresh() {
        let mut token = OAuthToken::new("access".into());
        token.refresh_token = Some("fl_oauth_saved".into());
        let error = resolve_tokens(
            "http://127.0.0.1:1",
            false,
            Some("pat_secret"),
            None,
            &HashMap::from([("provider".into(), token)]),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("bind its saved grant"));
    }
}
