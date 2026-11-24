use crate::{
    entity::{app, app_package, sea_orm_active_enums::Visibility},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::fork_permission::{ForkTargetKind, check_can_fork},
    state::AppState,
    utils::fork::{
        ForkPolicy,
        packages::{self, BlockedPackage, PinSplit, RepinnedPackage},
        preview::{
            ForkSizeBreakdown, RemoteTokenSite, compute_fork_size_breakdown,
            detect_remote_token_sites,
        },
    },
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Clone, Debug, Default, Deserialize, IntoParams)]
pub struct ForkPreviewQuery {
    /// Where the fork would land. Defaults to `online` (same backend);
    /// `offline` exercises the unauthenticated public-fork gate when the
    /// caller is anonymous.
    pub target: Option<ForkPreviewTarget>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ForkPreviewTarget {
    #[default]
    Online,
    Offline,
}

impl From<ForkPreviewTarget> for ForkTargetKind {
    fn from(t: ForkPreviewTarget) -> Self {
        match t {
            ForkPreviewTarget::Online => ForkTargetKind::Online,
            ForkPreviewTarget::Offline => ForkTargetKind::Offline,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ForkPreviewResponse {
    pub source_app_id: String,
    /// Sum of every object's size under `apps/{source_app_id}/...`.
    pub total_size_bytes: u64,
    /// Object count under the same prefix.
    pub total_object_count: u64,
    /// Hard cap from the deployment config (`forking.max_size_bytes`).
    pub max_size_bytes: u64,
    /// Hard cap from the deployment config (`forking.max_file_count`).
    pub max_file_count: u64,
    /// Whether the fork — after applying the owner's policy — fits within
    /// both caps. An app that is too large as a whole can still be
    /// forkable if the policy excludes its database or files.
    pub within_limits: bool,
    /// The source owner's fork policy. Read-only for the forker: the owner
    /// decides what a fork of their app contains. Absent for a caller who can
    /// neither fork the app nor see it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork_policy: Option<ForkPolicy>,
    /// Per-category sizes of the source, so the UI can show what each
    /// category costs and what the policy excludes. Absent like `fork_policy`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_breakdown: Option<ForkSizeBreakdown>,
    /// Bytes that will actually be copied once the policy is applied.
    pub selected_size_bytes: u64,
    /// Objects that will actually be copied once the policy is applied.
    pub selected_object_count: u64,
    /// True if any remote-event tokens were detected on the source —
    /// caller should prompt the user for one before invoking the fork.
    pub requires_token: bool,
    /// Per-site detail (HTTP auth_token, PAT, OAuth) so the UI can
    /// explain *which* events need re-auth.
    pub remote_token_sites: Vec<RemoteTokenSite>,
    /// Whether the project owner has opted into the Fork-an-app
    /// feature (`App.allow_forking`).
    pub allow_forking: bool,
    /// Whether the calling user passes the permission gate for the
    /// requested `target` (auth + read-set OR public-free anonymous).
    pub user_can_fork: bool,
    /// Reason `user_can_fork` is false, when applicable. Empty string
    /// when the caller is allowed.
    pub disallow_reason: String,
    /// Packages the source pins that the fork would drop: the caller doesn't
    /// hold them, or no version of them can be downloaded. Buying or
    /// requesting access before forking keeps the ones that allow it. Empty
    /// when the caller can't fork or the fork exceeds the caps.
    pub blocked_packages: Vec<BlockedPackage>,
    /// Packages the fork pins at another version than the source, because
    /// the pinned one isn't published. Empty under the same conditions.
    pub repinned_packages: Vec<RepinnedPackage>,
}

impl ForkPreviewResponse {
    /// The answer for a caller who can neither fork the app nor see it: whether a fork is
    /// possible for them and why not. What the app holds stays out of it.
    fn verdict(
        source_app_id: String,
        allow_forking: bool,
        disallow_reason: String,
        max_size_bytes: u64,
        max_file_count: u64,
    ) -> Self {
        Self {
            source_app_id,
            total_size_bytes: 0,
            total_object_count: 0,
            max_size_bytes,
            max_file_count,
            within_limits: true,
            fork_policy: None,
            size_breakdown: None,
            selected_size_bytes: 0,
            selected_object_count: 0,
            requires_token: false,
            remote_token_sites: Vec::new(),
            allow_forking,
            user_can_fork: false,
            disallow_reason,
            blocked_packages: Vec::new(),
            repinned_packages: Vec::new(),
        }
    }
}

/// Apps the store shows to everyone.
fn is_listed(visibility: &Visibility) -> bool {
    matches!(
        visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    )
}

/// Whether the caller may learn what the app holds without being able to fork it: the app is
/// listed, or they are in it. The owner looks at this preview while forking is still off.
async fn sees_app(state: &AppState, user: &AppUser, app_row: &app::Model) -> bool {
    is_listed(&app_row.visibility) || user.app_permission(&app_row.id, state).await.is_ok()
}

/// Pre-fork dry run. Returns the size + count totals, detected
/// remote-token sites, and the permission-gate verdict for the caller.
/// Does **not** allocate any state — safe to call as a probe before
/// committing to a full fork.
#[utoipa::path(
    get,
    path = "/apps/{app_id}/fork/preview",
    tag = "forking",
    description = "Compute the size and remote-token requirements of a hypothetical fork of this app, and report whether the caller is allowed to perform it. A caller who can neither fork the app nor see it (it is not listed and they are not a member) gets the verdict only: sizes are zero and the fork policy, size breakdown and token sites are left out.",
    params(
        ("app_id" = String, Path, description = "Source application ID"),
        ForkPreviewQuery
    ),
    responses(
        (status = 200, description = "Fork preview", body = ForkPreviewResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Source app not found")
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/fork/preview", skip(state, user, params))]
pub async fn get_fork_preview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<ForkPreviewQuery>,
) -> Result<Json<ForkPreviewResponse>, ApiError> {
    let target_kind = params.target.unwrap_or_default().into();

    // The caller-facing permission verdict is reported in the response
    // body rather than via 403 so the UI can show *why* the button is
    // disabled (forking off, no read access, anonymous online, etc.).
    let (user_can_fork, disallow_reason, app_row) =
        match check_can_fork(&user, &app_id, &state, target_kind).await {
            Ok(row) => (true, String::new(), row),
            Err(err) => {
                let reason = err.to_string();
                // Still need the row to fill out size / requires_token
                // when it's just a permission miss. NOT_FOUND propagates.
                let row = app::Entity::find_by_id(app_id.as_str())
                    .one(&state.db)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;
                (false, reason, row)
            }
        };

    let max_size_bytes = state.platform_config.forking.max_size_bytes;
    let max_file_count = state.platform_config.forking.max_file_count;

    if !user_can_fork && !sees_app(&state, &user, &app_row).await {
        return Ok(Json(ForkPreviewResponse::verdict(
            app_id,
            app_row.allow_forking,
            disallow_reason,
            max_size_bytes,
            max_file_count,
        )));
    }

    let fork_policy = ForkPolicy::from_app_row(&app_row);
    let size_breakdown = compute_fork_size_breakdown(&state, &app_id).await?;
    let (total_size_bytes, total_object_count) = size_breakdown.total();
    let (selected_size_bytes, selected_object_count) = size_breakdown.selected(&fork_policy);
    let remote_token_sites = detect_remote_token_sites(&state, &app_id).await?;

    // Caps apply to what will actually be copied, not to the whole app —
    // otherwise the policy could never rescue an over-cap app, which is
    // the main reason to exclude a category.
    let within_limits =
        selected_size_bytes <= max_size_bytes && selected_object_count <= max_file_count;

    let pins = package_pins(&state, &user, &app_id, user_can_fork, within_limits).await?;

    Ok(Json(ForkPreviewResponse {
        source_app_id: app_id,
        total_size_bytes,
        total_object_count,
        max_size_bytes,
        max_file_count,
        within_limits,
        fork_policy: Some(fork_policy),
        size_breakdown: Some(size_breakdown),
        selected_size_bytes,
        selected_object_count,
        requires_token: !remote_token_sites.is_empty(),
        remote_token_sites,
        allow_forking: app_row.allow_forking,
        user_can_fork,
        disallow_reason,
        blocked_packages: pins.blocked,
        repinned_packages: pins.repinned,
    }))
}

/// Only a caller who can actually fork learns which packages the source pins;
/// offering to buy one for a fork that can't happen would sell nothing.
async fn package_pins(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    user_can_fork: bool,
    within_limits: bool,
) -> Result<PinSplit, ApiError> {
    if !(user_can_fork && within_limits) {
        return Ok(PinSplit::default());
    }
    let pins = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .all(&state.db)
        .await?;
    packages::split_pins(&state.db, user.sub().ok().as_deref(), &pins).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_apps_the_store_shows_are_listed() {
        for (visibility, listed) in [
            (Visibility::Public, true),
            (Visibility::PublicRequestAccess, true),
            (Visibility::Private, false),
            (Visibility::Prototype, false),
            (Visibility::Offline, false),
        ] {
            assert_eq!(is_listed(&visibility), listed, "{visibility:?}");
        }
    }

    #[test]
    fn a_verdict_says_why_not_and_nothing_about_what_the_app_holds() {
        let verdict = ForkPreviewResponse::verdict(
            "app-1".to_string(),
            true,
            "join this app before forking it".to_string(),
            1_000,
            10,
        );
        let json = serde_json::to_value(&verdict).unwrap();
        let fields = json.as_object().unwrap();
        assert_eq!(json["user_can_fork"], false);
        assert_eq!(json["allow_forking"], true);
        assert_eq!(json["disallow_reason"], "join this app before forking it");
        assert_eq!(json["max_size_bytes"], 1_000);
        assert_eq!(json["max_file_count"], 10);
        for absent in ["fork_policy", "size_breakdown"] {
            assert!(!fields.contains_key(absent), "{absent}");
        }
        for zero in [
            "total_size_bytes",
            "total_object_count",
            "selected_size_bytes",
            "selected_object_count",
        ] {
            assert_eq!(json[zero], 0, "{zero}");
        }
        for empty in [
            "remote_token_sites",
            "blocked_packages",
            "repinned_packages",
        ] {
            assert_eq!(json[empty].as_array().map(Vec::len), Some(0), "{empty}");
        }
        assert_eq!(json["requires_token"], false);
    }

    /// The storage listing and the token-site scan run only after the caller is known to fork
    /// or see the app, and the only earlier answer is the verdict.
    #[test]
    fn nothing_about_the_app_is_read_before_the_caller_may_see_it() {
        let source = include_str!("preview.rs");
        let handler = source
            .split_once("\npub async fn get_fork_preview(")
            .expect("get_fork_preview exists")
            .1;
        let handler = handler
            .split_once("\n}\n")
            .map_or(handler, |(body, _)| body);
        let position = |needle: &str| {
            handler
                .find(needle)
                .unwrap_or_else(|| panic!("get_fork_preview contains {needle}"))
        };
        let gate = position("if !user_can_fork && !sees_app(&state, &user, &app_row).await {");
        let verdict = position("ForkPreviewResponse::verdict(");
        assert!(gate < verdict);
        for read in [
            "ForkPolicy::from_app_row(",
            "compute_fork_size_breakdown(",
            "detect_remote_token_sites(",
            "package_pins(",
        ] {
            assert!(verdict < position(read), "{read}");
        }
    }
}
