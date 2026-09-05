//! Custom token → ID token exchange and refresh for the executor's database session.

use std::time::{Duration, Instant};

use flow_like_types::anyhow;
use serde::{Deserialize, Deserializer};
use tokio::sync::Mutex;

const SIGN_IN_URL: &str =
    "https://identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken";
const REFRESH_URL: &str = "https://securetoken.googleapis.com/v1/token";
/// Tokens are renewed this long before Firebase would reject them.
const REFRESH_MARGIN: Duration = Duration::from_secs(5 * 60);
const DEFAULT_LIFETIME_SECS: u64 = 3600;

struct Session {
    id_token: String,
    refresh_token: Option<String>,
    expires_at: Instant,
}

pub(crate) struct FirebaseAuth {
    client: reqwest::Client,
    api_key: String,
    custom_token: String,
    session: Mutex<Option<Session>>,
}

impl FirebaseAuth {
    pub(crate) fn new(client: reqwest::Client, api_key: String, custom_token: String) -> Self {
        Self {
            client,
            api_key,
            custom_token,
            session: Mutex::new(None),
        }
    }

    /// A valid ID token: the cached one, a refreshed one, or a fresh sign-in with the custom
    /// token (which itself only works within the hour the API minted it for).
    pub(crate) async fn id_token(&self) -> flow_like_types::Result<String> {
        let mut session = self.session.lock().await;
        if let Some(current) = session.as_ref()
            && current.expires_at > Instant::now() + REFRESH_MARGIN
        {
            return Ok(current.id_token.clone());
        }
        let refreshed = match session.as_ref().and_then(|s| s.refresh_token.clone()) {
            Some(refresh_token) => match self.refresh(&refresh_token).await {
                Ok(next) => Some(next),
                Err(_) => {
                    tracing::warn!("firebase id token refresh failed, signing in again");
                    None
                }
            },
            None => None,
        };
        let next = match refreshed {
            Some(next) => next,
            None => self.sign_in().await?,
        };
        let token = next.id_token.clone();
        *session = Some(next);
        Ok(token)
    }

    /// The stream reported `auth_revoked` (or a 401/403): the next [`Self::id_token`] renews.
    pub(crate) async fn invalidate(&self) {
        if let Some(current) = self.session.lock().await.as_mut() {
            current.expires_at = Instant::now();
        }
    }

    async fn sign_in(&self) -> flow_like_types::Result<Session> {
        let response = self
            .client
            .post(SIGN_IN_URL)
            .query(&[("key", self.api_key.as_str())])
            .json(&SignInRequest {
                token: &self.custom_token,
                return_secure_token: true,
            })
            .send()
            .await
            .map_err(|err| {
                anyhow!(
                    "firebase signInWithCustomToken request failed: {}",
                    err.without_url()
                )
            })?;
        let body: SignInResponse = read_json(response, "signInWithCustomToken").await?;
        Ok(Session {
            id_token: body.id_token,
            refresh_token: body.refresh_token,
            expires_at: expiry(body.expires_in),
        })
    }

    async fn refresh(&self, refresh_token: &str) -> flow_like_types::Result<Session> {
        let response = self
            .client
            .post(REFRESH_URL)
            .query(&[("key", self.api_key.as_str())])
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .send()
            .await
            .map_err(|err| {
                anyhow!(
                    "firebase token refresh request failed: {}",
                    err.without_url()
                )
            })?;
        let body: RefreshResponse = read_json(response, "token refresh").await?;
        Ok(Session {
            id_token: body.id_token,
            refresh_token: body
                .refresh_token
                .or_else(|| Some(refresh_token.to_string())),
            expires_at: expiry(body.expires_in),
        })
    }
}

fn expiry(seconds: Option<u64>) -> Instant {
    Instant::now() + Duration::from_secs(seconds.unwrap_or(DEFAULT_LIFETIME_SECS))
}

async fn read_json<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    operation: &str,
) -> flow_like_types::Result<T> {
    let status = response.status();
    let text = response.text().await.map_err(|err| {
        anyhow!(
            "firebase {operation} response unreadable: {}",
            err.without_url()
        )
    })?;
    if !status.is_success() {
        return Err(anyhow!(
            "firebase {operation} returned {status}: {}",
            error_message(&text)
        ));
    }
    serde_json::from_str(&text)
        .map_err(|err| anyhow!("firebase {operation} returned an unexpected body: {err}"))
}

fn error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| body.chars().take(300).collect())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SignInRequest<'a> {
    token: &'a str,
    return_secure_token: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignInResponse {
    id_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default, deserialize_with = "seconds")]
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct RefreshResponse {
    id_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default, deserialize_with = "seconds")]
    expires_in: Option<u64>,
}

/// Google returns lifetimes as strings (`"3600"`); accept numbers too.
fn seconds<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Number(u64),
        Text(String),
    }
    Ok(match Option::<Raw>::deserialize(deserializer)? {
        Some(Raw::Number(n)) => Some(n),
        Some(Raw::Text(text)) => text.trim().parse().ok(),
        None => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifetimes_parse_from_strings_and_numbers() {
        let text: SignInResponse =
            serde_json::from_str(r#"{"idToken":"a","refreshToken":"r","expiresIn":"3600"}"#)
                .unwrap();
        assert_eq!(text.expires_in, Some(3600));
        assert_eq!(text.refresh_token.as_deref(), Some("r"));
        let number: RefreshResponse =
            serde_json::from_str(r#"{"id_token":"a","expires_in":120}"#).unwrap();
        assert_eq!(number.expires_in, Some(120));
        assert!(number.refresh_token.is_none());
        let missing: SignInResponse = serde_json::from_str(r#"{"idToken":"a"}"#).unwrap();
        assert!(missing.expires_in.is_none());
    }

    #[test]
    fn error_messages_prefer_the_google_envelope() {
        assert_eq!(
            error_message(r#"{"error":{"code":400,"message":"INVALID_CUSTOM_TOKEN"}}"#),
            "INVALID_CUSTOM_TOKEN"
        );
        assert_eq!(error_message("gateway timeout"), "gateway timeout");
    }
}
