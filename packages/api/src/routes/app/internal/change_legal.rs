use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::app::FrontendConfiguration;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const MAX_LEGAL_URL_BYTES: usize = 2048;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateLegalLinksBody {
    /// Link to the publisher's legal notice (imprint). Send an empty string to
    /// clear it. Omit to leave unchanged.
    #[serde(default)]
    pub legal_notice_url: Option<String>,
    /// Link to the publisher's privacy policy. Send an empty string to clear
    /// it. Omit to leave unchanged.
    #[serde(default)]
    pub privacy_policy_url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct LegalLinksResponse {
    /// Link to the publisher's legal notice (imprint), shown on hosted pages.
    pub legal_notice_url: Option<String>,
    /// Link to the publisher's privacy policy, shown on hosted pages.
    pub privacy_policy_url: Option<String>,
}

impl LegalLinksResponse {
    pub(crate) fn from_frontend(frontend: Option<&FrontendConfiguration>) -> Self {
        Self {
            legal_notice_url: frontend.and_then(|frontend| frontend.legal_notice_url.clone()),
            privacy_policy_url: frontend.and_then(|frontend| frontend.privacy_policy_url.clone()),
        }
    }
}

/// Hosted pages render these as links, so anything but an absolute http(s)
/// URL — `javascript:` above all — is rejected.
fn normalize_legal_url(field: &str, value: &str) -> Result<Option<String>, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_LEGAL_URL_BYTES {
        return Err(ApiError::bad_request(format!(
            "{field} is {} bytes; the limit is {MAX_LEGAL_URL_BYTES}",
            value.len()
        )));
    }
    let url = reqwest::Url::parse(value)
        .map_err(|error| ApiError::bad_request(format!("{field} is not a valid URL: {error}")))?;
    if !matches!(url.scheme(), "https" | "http") || url.host_str().is_none() {
        return Err(ApiError::bad_request(format!(
            "{field} must be an http(s) link, got scheme '{}'",
            url.scheme()
        )));
    }
    Ok(Some(value.to_string()))
}

/// Read the legal notice and privacy policy links shown on the app's hosted
/// pages.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/settings/legal",
    tag = "hosting",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "The app's legal links", body = LegalLinksResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/settings/legal", skip(state, user))]
pub async fn get_legal_links(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<LegalLinksResponse>, ApiError> {
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);

    let app = state
        .scoped_app(
            &sub.sub()?,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::ReadApp,
        )
        .await?;

    Ok(Json(LegalLinksResponse::from_frontend(
        app.frontend.as_ref(),
    )))
}

/// Set the legal notice and privacy policy links shown on the app's hosted
/// pages.
///
/// Gated like publishing a hosted route (`WriteEvents`), since these links
/// only appear on hosted pages. A separate channel from `PUT /apps/{app_id}`
/// for the same reason as the appearance route: that endpoint writes a
/// client-held copy of the app and would revert this from a stale page.
#[utoipa::path(
    patch,
    path = "/apps/{app_id}/settings/legal",
    tag = "hosting",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UpdateLegalLinksBody,
    responses(
        (status = 200, description = "Legal links updated", body = LegalLinksResponse),
        (status = 400, description = "Nothing to update, or a link is not an http(s) URL"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "PATCH /apps/{app_id}/settings/legal", skip(state, user, body))]
pub async fn change_legal_links(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<UpdateLegalLinksBody>,
) -> Result<Json<LegalLinksResponse>, ApiError> {
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);

    if body.legal_notice_url.is_none() && body.privacy_policy_url.is_none() {
        return Err(ApiError::bad_request(
            "supply legal_notice_url or privacy_policy_url",
        ));
    }
    let legal_notice_url = body
        .legal_notice_url
        .as_deref()
        .map(|value| normalize_legal_url("legal_notice_url", value))
        .transpose()?;
    let privacy_policy_url = body
        .privacy_policy_url
        .as_deref()
        .map(|value| normalize_legal_url("privacy_policy_url", value))
        .transpose()?;

    let mut app = state
        .scoped_app(
            &sub.sub()?,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    let current = LegalLinksResponse::from_frontend(app.frontend.as_ref());
    let next = LegalLinksResponse {
        legal_notice_url: legal_notice_url.unwrap_or_else(|| current.legal_notice_url.clone()),
        privacy_policy_url: privacy_policy_url
            .unwrap_or_else(|| current.privacy_policy_url.clone()),
    };
    if next == current {
        return Ok(Json(current));
    }

    let frontend = app
        .frontend
        .get_or_insert_with(FrontendConfiguration::default);
    frontend.legal_notice_url = next.legal_notice_url.clone();
    frontend.privacy_policy_url = next.privacy_policy_url.clone();
    app.save().await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app.settings.legal",
        "App",
        app_id,
        serde_json::json!({
            "legal_notice_url": next.legal_notice_url,
            "privacy_policy_url": next.privacy_policy_url,
        })
    );

    Ok(Json(next))
}

#[cfg(test)]
mod tests {
    use super::normalize_legal_url;

    #[test]
    fn legal_urls_accept_http_links_and_clear_on_blank() {
        assert_eq!(
            normalize_legal_url("f", "  https://example.com/imprint ").unwrap(),
            Some("https://example.com/imprint".to_string())
        );
        assert_eq!(
            normalize_legal_url("f", "http://example.com").unwrap(),
            Some("http://example.com".to_string())
        );
        assert_eq!(normalize_legal_url("f", "   ").unwrap(), None);
    }

    #[test]
    fn legal_urls_reject_scripts_and_relative_paths() {
        assert!(normalize_legal_url("f", "javascript:alert(1)").is_err());
        assert!(normalize_legal_url("f", "data:text/html,hi").is_err());
        assert!(normalize_legal_url("f", "/imprint").is_err());
        assert!(normalize_legal_url("f", "mailto:legal@example.com").is_err());
    }
}
