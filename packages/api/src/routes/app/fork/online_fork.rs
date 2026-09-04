use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    permission::fork_permission::{ForkTargetKind, check_can_fork},
    state::AppState,
    utils::fork::{
        ForkPolicy, ForkReport,
        job::{self, ForkJobSpec, ForkJobView},
        preview::{
            compute_fork_size_breakdown, detect_remote_token_sites, ensure_breakdown_within_limits,
        },
    },
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
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
/// Small forks (at most one write chunk of rows and 64 MiB of storage)
/// complete inside the request and answer `200`. Larger ones are staged
/// as a fork job: the response is `202` with the job id, the destination
/// app exists hidden from the start, and the caller polls
/// `GET /apps/fork/jobs/{job_id}` until `status` is `DONE`.
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
        (status = 202, description = "Fork staged as a job; poll `GET /apps/fork/jobs/{job_id}`", body = ForkJobView),
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
) -> Result<Response, ApiError> {
    let src_app = check_can_fork(&user, &app_id, &state, ForkTargetKind::Online).await?;

    let policy = ForkPolicy::from_app_row(&src_app);
    let breakdown = compute_fork_size_breakdown(&state, &app_id).await?;
    ensure_breakdown_within_limits(&state, &breakdown, &policy)?;

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
    let spec = ForkJobSpec::online_copy(
        &state,
        &src_app,
        &language,
        body.remote_event_token.as_deref(),
        crate::entity::sea_orm_active_enums::Visibility::Private,
    );

    let (selected_bytes, _) = breakdown.selected(&policy);
    let rows = job::count_source_rows(&state, &app_id).await?;
    let fork_job = job::enqueue(&state, &app_id, &user_sub, spec).await?;

    if job::fits_sync(rows, selected_bytes) {
        let (finished, report) = job::run_inline(&state, fork_job).await?;
        return Ok(Json(OnlineForkResponse {
            new_app_id: finished.dest_app_id,
            report,
        })
        .into_response());
    }

    let view = ForkJobView::from(&fork_job);
    job::spawn_background(state, fork_job);
    Ok((StatusCode::ACCEPTED, Json(view)).into_response())
}
