use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Token passed from the frontend after OAuth authentication
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct OAuthToken {
    /// The access token for API calls
    pub access_token: String,
    /// Optional refresh token for obtaining new access tokens
    pub refresh_token: Option<String>,
    /// Unix timestamp when the access token expires
    pub expires_at: Option<u64>,
    /// The token type (usually "Bearer")
    pub token_type: Option<String>,
}

impl OAuthToken {
    pub fn new(access_token: String) -> Self {
        OAuthToken {
            access_token,
            refresh_token: None,
            expires_at: None,
            token_type: Some("Bearer".to_string()),
        }
    }

    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            // Consider expired if less than 60 seconds remaining
            expires_at <= now + 60
        } else {
            false
        }
    }

    pub fn bearer_header(&self) -> String {
        format!(
            "{} {}",
            self.token_type.as_deref().unwrap_or("Bearer"),
            self.access_token
        )
    }
}

/// Collection of OAuth tokens keyed by provider ID
pub type OAuthTokens = HashMap<String, OAuthToken>;
