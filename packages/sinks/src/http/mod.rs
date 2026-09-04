//! HTTP sink implementation for server-side API endpoints

use crate::{
    config::HttpSinkConfig,
    traits::{Executor, SinkContext, SinkError, SinkResult, SinkTrait, TriggerResponse},
    types::{SinkRegistration, SinkType},
};
use flow_like_types::utils::constant_time_eq;
use serde::{Deserialize, Serialize};

/// HTTP sink for REST API endpoints
#[derive(Debug, Clone, Default)]
pub struct HttpSink;

impl HttpSink {
    pub fn new() -> Self {
        Self
    }

    /// Parse and validate HTTP sink config from JSON
    pub fn parse_config(config: &flow_like_types::Value) -> SinkResult<HttpSinkConfig> {
        serde_json::from_value(config.clone())
            .map_err(|e| SinkError::InvalidConfig(format!("Invalid HTTP config: {}", e)))
    }

    /// Validate the HTTP method
    fn validate_method(method: &str) -> SinkResult<()> {
        let valid_methods = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
        if !valid_methods.contains(&method.to_uppercase().as_str()) {
            return Err(SinkError::InvalidConfig(format!(
                "Invalid HTTP method: {}. Must be one of: {}",
                method,
                valid_methods.join(", ")
            )));
        }
        Ok(())
    }

    /// Validate the path format
    fn validate_path(path: &str) -> SinkResult<()> {
        if !path.starts_with('/') {
            return Err(SinkError::InvalidConfig(format!(
                "HTTP path must start with '/': {}",
                path
            )));
        }
        if path.contains("..") {
            return Err(SinkError::InvalidConfig(
                "HTTP path cannot contain '..'".to_string(),
            ));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl SinkTrait for HttpSink {
    fn sink_type(&self) -> SinkType {
        SinkType::Http
    }

    fn validate_config(&self, config: &flow_like_types::Value) -> SinkResult<()> {
        let cfg = Self::parse_config(config)?;
        Self::validate_method(&cfg.method)?;
        Self::validate_path(&cfg.path)?;
        Ok(())
    }

    async fn register<E: Executor>(
        &self,
        _ctx: &SinkContext<E>,
        registration: &SinkRegistration,
    ) -> SinkResult<()> {
        // Validate the config
        self.validate_config(&registration.config)?;

        let config = Self::parse_config(&registration.config)?;

        tracing::info!(
            "Registered HTTP sink: {} {} -> event {} (app: {})",
            config.method.to_uppercase(),
            config.path,
            registration.event_id,
            registration.app_id
        );

        Ok(())
    }

    async fn unregister<E: Executor>(
        &self,
        _ctx: &SinkContext<E>,
        registration: &SinkRegistration,
    ) -> SinkResult<()> {
        tracing::info!(
            "Unregistered HTTP sink for event {} (app: {})",
            registration.event_id,
            registration.app_id
        );
        Ok(())
    }

    async fn handle_trigger<E: Executor>(
        &self,
        ctx: &SinkContext<E>,
        registration: &SinkRegistration,
        payload: Option<flow_like_types::Value>,
    ) -> SinkResult<TriggerResponse> {
        tracing::info!(
            "HTTP sink triggered for event {} (app: {})",
            registration.event_id,
            registration.app_id
        );

        // Merge default payload with incoming payload
        let final_payload = match (&registration.default_payload, payload) {
            (Some(default), Some(incoming)) => {
                // Merge incoming over default
                let mut merged = default.clone();
                if let (Some(default_obj), Some(incoming_obj)) =
                    (merged.as_object_mut(), incoming.as_object())
                {
                    for (k, v) in incoming_obj {
                        default_obj.insert(k.clone(), v.clone());
                    }
                }
                Some(merged)
            }
            (Some(default), None) => Some(default.clone()),
            (None, Some(incoming)) => Some(incoming),
            (None, None) => None,
        };

        // Execute the event
        let run_id = ctx
            .executor
            .execute_event(
                &registration.app_id,
                &registration.board_id,
                &registration.event_id,
                final_payload,
                registration.personal_access_token.as_deref(),
            )
            .await?;

        Ok(TriggerResponse::success(Some(run_id)))
    }
}

/// HTTP trigger request - used for validating incoming requests
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTriggerRequest {
    /// The app ID from the URL path
    pub app_id: String,

    /// The path after the app ID
    pub path: String,

    /// The HTTP method
    pub method: String,

    /// Authorization header value
    pub auth_header: Option<String>,

    /// Request body
    pub body: Option<flow_like_types::Value>,
}

impl HttpTriggerRequest {
    /// Verify the auth token if required
    pub fn verify_auth(&self, registration: &SinkRegistration) -> SinkResult<()> {
        if let Some(expected_token) = &registration.auth_token {
            match &self.auth_header {
                Some(header) if constant_time_eq(header.as_bytes(), expected_token.as_bytes()) => {
                    Ok(())
                }
                Some(_) => Err(SinkError::AuthFailed("Invalid auth token".to_string())),
                None => Err(SinkError::AuthFailed("Missing auth token".to_string())),
            }
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SinkExecution;

    const TOKEN: &str = "test-http-auth-token";

    fn registration(auth_token: Option<&str>) -> SinkRegistration {
        SinkRegistration {
            id: "sink-1".to_string(),
            event_id: "event-1".to_string(),
            board_id: "board-1".to_string(),
            app_id: "app-1".to_string(),
            sink_type: SinkType::Http,
            config: serde_json::json!({}),
            execution: SinkExecution::Remote,
            active: true,
            auth_token: auth_token.map(ToOwned::to_owned),
            path: Some("/hook".to_string()),
            method: Some("POST".to_string()),
            cron_expression: None,
            default_payload: None,
            personal_access_token: None,
            oauth_tokens: None,
        }
    }

    fn request(auth_header: Option<&str>) -> HttpTriggerRequest {
        HttpTriggerRequest {
            app_id: "app-1".to_string(),
            path: "/hook".to_string(),
            method: "POST".to_string(),
            auth_header: auth_header.map(ToOwned::to_owned),
            body: None,
        }
    }

    #[test]
    fn auth_accepts_valid_token() {
        assert!(
            request(Some(TOKEN))
                .verify_auth(&registration(Some(TOKEN)))
                .is_ok()
        );
    }

    #[test]
    fn auth_rejects_invalid_token() {
        assert!(matches!(
            request(Some("best-http-auth-token"))
                .verify_auth(&registration(Some(TOKEN))),
            Err(SinkError::AuthFailed(message)) if message == "Invalid auth token"
        ));
    }

    #[test]
    fn auth_rejects_missing_token() {
        assert!(matches!(
            request(None).verify_auth(&registration(Some(TOKEN))),
            Err(SinkError::AuthFailed(message)) if message == "Missing auth token"
        ));
    }

    #[test]
    fn auth_is_optional_when_no_token_is_configured() {
        assert!(request(None).verify_auth(&registration(None)).is_ok());
    }
}
