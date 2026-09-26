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

/// A stylesheet large enough to hit this is a mistake, not a design. Rejected
/// whole rather than truncated — a stylesheet cut mid-rule renders worse than
/// no stylesheet at all, and silently.
const MAX_CUSTOM_CSS_BYTES: usize = 40_000;

#[derive(Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateAppearanceBody {
    /// App-wide stylesheet. Send the complete sheet; it replaces what is
    /// stored. Send an empty string to clear it. Omit to leave unchanged.
    #[serde(default)]
    pub custom_css: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct AppearanceResponse {
    /// The stored app-wide stylesheet. `None` on an app that has never had one.
    pub custom_css: Option<String>,
}

/// Read the app-wide stylesheet for editing.
///
/// This is the OWNER-facing read. The runtime does not use it — a page renders
/// the stylesheet from its bootstrap response, which an ordinary viewer can
/// reach. Reading the sheet here would 403 every non-owner.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/settings/appearance",
    tag = "appearance",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Current app-wide stylesheet", body = AppearanceResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/settings/appearance", skip(state, user))]
pub async fn get_appearance(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AppearanceResponse>, ApiError> {
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    let app = state
        .scoped_app(
            &sub.sub()?,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    Ok(Json(AppearanceResponse {
        custom_css: app.frontend.and_then(|frontend| frontend.custom_css),
    }))
}

/// Replace the app-wide stylesheet.
///
/// Deliberately a separate channel from `PUT /apps/{app_id}`. That endpoint
/// copies a hand-written field whitelist over the manifest from a client-held
/// copy of the app, and `App::save` has no compare-and-swap, so routing the
/// stylesheet through it would let an unrelated settings save from a stale
/// page silently revert it.
#[utoipa::path(
    patch,
    path = "/apps/{app_id}/settings/appearance",
    tag = "appearance",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UpdateAppearanceBody,
    responses(
        (status = 200, description = "Stylesheet updated"),
        (status = 400, description = "Nothing to update, or stylesheet too large"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(
    name = "PATCH /apps/{app_id}/settings/appearance",
    skip(state, user, body)
)]
pub async fn change_appearance(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<UpdateAppearanceBody>,
) -> Result<Json<()>, ApiError> {
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    let Some(custom_css) = body.custom_css else {
        return Err(ApiError::bad_request("supply custom_css"));
    };

    if custom_css.len() > MAX_CUSTOM_CSS_BYTES {
        return Err(ApiError::bad_request(&format!(
            "stylesheet is {} bytes; the limit is {MAX_CUSTOM_CSS_BYTES}",
            custom_css.len()
        )));
    }

    // An empty sheet is a clear, not an empty rule set — storing `Some("")`
    // would leave an inert `<style>` on every page forever.
    let next = (!custom_css.trim().is_empty()).then_some(custom_css);

    let mut app = state
        .scoped_app(
            &sub.sub()?,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    let current = app
        .frontend
        .as_ref()
        .and_then(|frontend| frontend.custom_css.as_deref());
    if current == next.as_deref() {
        return Ok(Json(()));
    }

    match app.frontend.as_mut() {
        Some(frontend) => frontend.custom_css = next.clone(),
        None => {
            app.frontend = Some(FrontendConfiguration {
                landing_page: None,
                custom_css: next.clone(),
            })
        }
    }
    app.save().await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app.settings.appearance",
        "App",
        app_id,
        serde_json::json!({ "custom_css_bytes": next.as_deref().map(str::len) })
    );

    Ok(Json(()))
}
