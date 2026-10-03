//! Path-carried access to the widget files of one package version.
//!
//! A widget iframe sends no credentials, so a version the anonymous rules do
//! not open (a private package, a version only its maintainers see, or any
//! package while `unauthorized_read` is off) never loads in one. The host
//! asks for an access token with its own credentials and puts it into the
//! sandbox URL as a `~{token}` segment right after the version, so the
//! wrapper, the document and every relative asset URL carry it.
//!
//! The token binds one package version and its bundle hash and never names the
//! viewer: widget code can read its own URL, and the token only reaches files
//! of the bundle that code already runs from. A token minted through a
//! project's licence also names the project, and every request re-checks that
//! the project still licenses the version.

use super::widget_asset::authorize_widget_version;
use super::widget_policy::{invalid_app_id, parse_widget_request};
use crate::backend_jwt::{self, BackendJwtError, TokenType, issuer, make_time_claims};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use flow_like_wasm_schema::widget_frame::{is_valid_package_id, is_web_grant_token};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Leads the sandbox path segment that carries an access token.
pub const WIDGET_ACCESS_SEGMENT_PREFIX: char = '~';

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetAccessClaims {
    pub package_id: String,
    pub version: String,
    pub bundle_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,

    #[serde(rename = "typ")]
    pub token_type: TokenType,
    pub iss: String,
    pub aud: String,
    pub iat: i64,
    pub nbf: i64,
    pub exp: i64,
    pub jti: String,
}

impl WidgetAccessClaims {
    pub fn is_bound_to(&self, package_id: &str, version: &str) -> bool {
        self.package_id == package_id && self.version == version
    }
}

/// `app_id` names the project whose licence let the viewer in, when nothing
/// else did.
pub fn sign_widget_access(
    package_id: &str,
    version: &str,
    bundle_hash: &str,
    app_id: Option<&str>,
) -> Result<String, BackendJwtError> {
    let time = make_time_claims(TokenType::WidgetAccess, None);
    backend_jwt::sign(&WidgetAccessClaims {
        package_id: package_id.to_string(),
        version: version.to_string(),
        bundle_hash: bundle_hash.to_string(),
        app_id: app_id.map(str::to_string),
        token_type: TokenType::WidgetAccess,
        iss: issuer().to_string(),
        aud: TokenType::WidgetAccess.audience().to_string(),
        iat: time.iat,
        nbf: time.nbf,
        exp: time.exp,
        jti: flow_like_types::create_id(),
    })
}

/// Verify an access token. Binding it to the request path and the version's
/// bundle hash is the caller's responsibility.
pub fn verify_widget_access(token: &str) -> Result<WidgetAccessClaims, BackendJwtError> {
    if !is_web_grant_token(token) {
        return Err(BackendJwtError::DecodingError(
            "Widget access token is not a compact JWT".to_string(),
        ));
    }
    let claims: WidgetAccessClaims =
        backend_jwt::verify_with_leeway(token, TokenType::WidgetAccess, 0)?;
    if claims.token_type != TokenType::WidgetAccess {
        return Err(BackendJwtError::TokenTypeMismatch {
            expected: TokenType::WidgetAccess,
            got: claims.token_type,
        });
    }
    Ok(claims)
}

/// Splits `~{token}/{rest}` into the token and the sandbox path behind it.
/// `None` when the first segment is not a token, so a bundle file whose path
/// starts with `~` is still served as a file.
pub fn split_access_segment(path: &str) -> Option<(&str, &str)> {
    let (token, rest) = path
        .strip_prefix(WIDGET_ACCESS_SEGMENT_PREFIX)?
        .split_once('/')?;
    (is_web_grant_token(token) && !rest.is_empty()).then_some((token, rest))
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WidgetAccessRequest {
    pub version: String,
    /// Project the widget runs in. Members of a project that uses this
    /// version get access without access to the package itself.
    #[serde(default)]
    pub app_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WidgetAccessResponse {
    /// The `~{access}` sandbox path segment, or null when the widget files
    /// load without one.
    pub access: Option<String>,
    /// Seconds until the access token stops working.
    pub expires_in: i64,
}

/// POST /registry/package/{package_id}/widget-access
#[utoipa::path(
    post,
    path = "/registry/package/{package_id}/widget-access",
    tag = "registry",
    description = "Get access for loading a package widget in a frame, which sends no sign-in. When the widget's files are not public, load them with '~' and the returned access value as the path segment right after the version; when access is null, load them as they are. Members of a project that uses this version get access through the project.",
    params(("package_id" = String, Path, description = "Package ID")),
    request_body = WidgetAccessRequest,
    responses(
        (status = 200, description = "Access for the widget files, or null when they need none", body = WidgetAccessResponse),
        (status = 400, description = "Malformed request: INVALID_APP_ID or INVALID_WIDGET_REQUEST"),
        (status = 403, description = "No access to this package"),
        (status = 404, description = "Package or version not found, or the version ships no widgets"),
        (status = 503, description = "Widget access or the WASM registry is not configured on this server")
    ),
    security(("bearer_auth" = []))
)]
pub async fn mint_widget_access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
    body: Bytes,
) -> Result<Response, ApiError> {
    if !is_valid_package_id(&package_id) {
        return Err(ApiError::not_found(format!(
            "Package '{}' not found",
            package_id
        )));
    }
    let request: WidgetAccessRequest = match parse_widget_request(&body) {
        Ok(request) => request,
        Err(error) => return Ok(error.into_response()),
    };
    if let Some(error) = invalid_app_id(request.app_id.as_deref()) {
        return Ok(error.into_response());
    }

    let authorized = authorize_widget_version(
        &state,
        &user,
        &package_id,
        &request.version,
        request.app_id.as_deref(),
    )
    .await?;
    let anonymous = authorize_widget_version(
        &state,
        &AppUser::Unauthorized,
        &package_id,
        &request.version,
        None,
    )
    .await
    .is_ok();
    let access = if anonymous {
        None
    } else {
        let token = sign_widget_access(
            &package_id,
            &request.version,
            &authorized.bundle_hash,
            authorized.through_project.as_deref(),
        )
        .map_err(|error| {
            tracing::warn!(
                package_id = %package_id,
                version = %request.version,
                %error,
                "Widget access token could not be signed"
            );
            ApiError::service_unavailable("Widget access is not configured on this server")
        })?;
        Some(token)
    };

    Ok((
        [(header::CACHE_CONTROL, "no-store")],
        Json(WidgetAccessResponse {
            access,
            expires_in: TokenType::WidgetAccess.default_ttl_seconds(),
        }),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "4f1c0a3b2d5e6f708192a3b4c5d6e7f80112233445566778899aabbccddeeff0";

    #[test]
    fn widget_access_round_trips_and_binds_one_version() {
        backend_jwt::init_for_tests();
        let token = sign_widget_access("com.example.maps", "1.2.0", HASH, None).unwrap();
        assert!(is_web_grant_token(&token));

        let claims = verify_widget_access(&token).unwrap();
        assert_eq!(claims.bundle_hash, HASH);
        assert_eq!(claims.app_id, None);
        assert!(claims.is_bound_to("com.example.maps", "1.2.0"));
        assert!(!claims.is_bound_to("com.example.maps", "1.3.0"));
        assert!(!claims.is_bound_to("com.example.other", "1.2.0"));
        assert_eq!(
            claims.exp - claims.iat,
            TokenType::WidgetAccess.default_ttl_seconds()
        );

        let through_project =
            sign_widget_access("com.example.maps", "1.2.0", HASH, Some("app_1")).unwrap();
        assert_eq!(
            verify_widget_access(&through_project)
                .unwrap()
                .app_id
                .as_deref(),
            Some("app_1")
        );
    }

    #[test]
    fn widget_grants_never_pass_as_access() {
        use super::super::widget_grant_jwt::{WidgetGrantParams, sign_widget_grant};

        backend_jwt::init_for_tests();
        let grant = sign_widget_grant(WidgetGrantParams {
            package_id: "com.example.maps".into(),
            version: "1.2.0".into(),
            bundle_hash: HASH.into(),
            widget_id: "live-map".into(),
            preview: false,
            policy_digest: format!("sha256:{HASH}"),
            runtime: None,
            ttl_seconds: None,
        })
        .unwrap()
        .token;
        assert!(verify_widget_access(&grant).is_err());
        assert!(verify_widget_access("not-a-token").is_err());
    }

    #[test]
    fn widget_access_segment_splits_off_the_sandbox_path() {
        let token = "eyJhbGciOiJFUzI1NiJ9.eyJzdWIiOiJ4In0.c2lnbmF0dXJl-_x";
        assert!(split_access_segment("frame/live-map/0").is_none());
        assert!(split_access_segment("widgets/live-map/index.0.html").is_none());

        let path = format!("~{token}/frame/live-map/0");
        assert_eq!(
            split_access_segment(&path),
            Some((token, "frame/live-map/0"))
        );

        for plain in [
            format!("~{token}"),
            format!("~{token}/"),
            "~partytown/partytown.js".to_string(),
            "~/frame/live-map/0".to_string(),
        ] {
            assert_eq!(
                split_access_segment(&plain),
                None,
                "{plain} carries no token"
            );
        }
    }
}
