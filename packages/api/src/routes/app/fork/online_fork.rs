use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    permission::fork_permission::{ForkTargetKind, check_can_fork},
    state::AppState,
    utils::fork::{
        ForkOptions, ForkPolicy, ForkReport, ForkTarget, fork_with_options,
        preview::{detect_remote_token_sites, ensure_fork_within_limits},
    },
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct OnlineForkBody {
    /// Single bearer-style token (PAT) reused at every detected
    /// HTTP-auth-token / sink-PAT site on the source app. OAuth-bound
    /// events are *always* cleared and reported regardless. Required
    /// only if the preview endpoint reported a token site that is
    /// `is_token_replaceable() == true`.
    pub remote_event_token: Option<String>,
    /// Language for the destination's default metadata. Falls back to
    /// the source's metadata language; "en" if neither is set.
    pub language: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct OnlineForkResponse {
    /// The destination app id. The fork is fully materialized (storage +
    /// DB rows) under the calling user's account with `Private`
    /// visibility — the caller can immediately load it like any other
    /// app they own.
    pub new_app_id: String,
    pub report: ForkReport,
}

/// Fork an online source app to an online destination on the calling
/// user's account. The destination is materialized end-to-end: storage
/// prefix is written, an `App` row is created with `Private` visibility,
/// fresh Owner / Admin / Member roles are inserted, the caller is
/// granted Owner membership, and every event / page / widget / template
/// row is mirrored from the source.
///
/// This is the same code path the course flow uses internally
/// (`shared_app.rs`); the difference is that this endpoint enforces the
/// project-level `allow_forking` opt-in and read-permission gate.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/fork",
    tag = "forking",
    description = "Create an online → online fork of this app on the calling user's account.",
    params(
        ("app_id" = String, Path, description = "Source application ID"),
    ),
    request_body = OnlineForkBody,
    responses(
        (status = 200, description = "Fork materialized; the destination is ready to load", body = OnlineForkResponse),
        (status = 400, description = "Source app exceeds size cap, or remote tokens detected without `remote_event_token`"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — caller lacks read perms or the source has not opted in to forking"),
        (status = 404, description = "Source app not found"),
        (status = 503, description = "Forking is disabled by the deployment configuration")
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/fork", skip(state, user, body))]
pub async fn online_fork(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<OnlineForkBody>,
) -> Result<Json<OnlineForkResponse>, ApiError> {
    let src_app = check_can_fork(&user, &app_id, &state, ForkTargetKind::Online).await?;

    let policy = ForkPolicy::from_app_row(&src_app);
    ensure_fork_within_limits(&state, &app_id, &policy).await?;

    let token_sites = detect_remote_token_sites(&state, &app_id).await?;
    let needs_replaceable_token = token_sites.iter().any(|s| s.is_token_replaceable());
    if needs_replaceable_token && body.remote_event_token.is_none() {
        return Err(ApiError::bad_request(format!(
            "source app has {} remote-token site(s) — supply `remote_event_token` in the body before forking; OAuth sites will be cleared and reported regardless",
            token_sites
                .iter()
                .filter(|s| s.is_token_replaceable())
                .count()
        )));
    }

    let user_sub = user.sub()?;
    let language = body.language.clone().unwrap_or_else(|| "en".to_string());

    let options = ForkOptions {
        source_app_id: &app_id,
        target_user_sub: Some(&user_sub),
        target_mode: ForkTarget::OnlineSameStore,
        language: &language,
        remote_event_token: body.remote_event_token.as_deref(),
        requested_visibility: Some(flow_like::app::AppVisibility::Private),
    };
    let (new_app_id, report) = fork_with_options(&state, options).await?;

    Ok(Json(OnlineForkResponse { new_app_id, report }))
}
