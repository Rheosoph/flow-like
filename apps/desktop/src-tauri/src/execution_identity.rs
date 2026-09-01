//! Execution identity for runs started on this device.
//!
//! Offline apps and hosted apps differ fundamentally here. An offline app has
//! no server-side role to consult — the machine owns it, so the run is
//! owner-equivalent. A hosted app does have one, and a local run has to
//! reproduce it, or the same board answers `Has Permission` with "yes" on the
//! desktop and "no" in the cloud.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use flow_like::{
    app::AppVisibility,
    flow::execution::{InternalRun, UserExecutionContext},
    hub::Hub,
    state::FlowLikeState,
};

use crate::local_page_actions::LocalPagePrincipalBinding;

/// How long a resolved identity is reused without asking the hub again. Sinks
/// fire as often as every minute, and the answer only changes when an admin
/// edits a role, so a per-run round trip would be pure latency.
const IDENTITY_TTL: Duration = Duration::from_secs(5 * 60);

/// App permissions required before a hosted Page Event may leave the server
/// and execute against the project stored on this device.
const READ_BOARDS_PERMISSION: i64 = 0x100;
const EXECUTE_BOARDS_PERMISSION: i64 = 0x200;
const EXECUTE_EVENTS_PERMISSION: i64 = 0x2000;

static IDENTITY_CACHE: OnceLock<Mutex<HashMap<String, CachedIdentity>>> = OnceLock::new();

#[derive(Clone)]
struct CachedIdentity {
    context: UserExecutionContext,
    resolved_at: Instant,
}

/// What a locally executed run should carry as its identity.
enum LocalIdentity {
    /// The app has no server-side role, so the run is owner-equivalent.
    OwnerEquivalent,
    /// Resolved against the hub: the caller's real subject, role and attributes.
    Hosted(Box<UserExecutionContext>),
    /// Hosted app whose role could not be resolved. The run keeps its subject
    /// but gets no permissions, so a gate fails closed instead of silently
    /// passing as owner.
    Unresolved,
}

/// Resolve the identity for a locally executed run and apply it to the run.
pub async fn apply_local_run_identity(
    run: &mut InternalRun,
    visibility: &AppVisibility,
    app_id: &str,
    token: Option<&str>,
    hub_url: &str,
    state: &Arc<FlowLikeState>,
) {
    match resolve_local_identity(visibility, app_id, token, hub_url, state).await {
        LocalIdentity::OwnerEquivalent => run.set_local_user_context().await,
        LocalIdentity::Hosted(context) => run.set_resolved_user_context(*context).await,
        LocalIdentity::Unresolved => run.set_unresolved_user_context().await,
    }
}

/// Authorize the narrower local path used by governed Page actions.
///
/// The capability-like Page action id only selects an entry from the Page
/// manifest. It does not grant local access. Offline apps are owned by the
/// device, while hosted apps must resolve the ordinary caller token and carry
/// board-read, board-execution, and Event-execution authority. A failed hub
/// lookup never falls back to a cached role for this security decision.
pub(crate) async fn ensure_page_local_execution_authorized(
    visibility: &AppVisibility,
    app_id: &str,
    token: Option<&str>,
    hub_url: &str,
    state: &Arc<FlowLikeState>,
) -> flow_like_types::Result<LocalPagePrincipalBinding> {
    let Some(context) =
        resolve_strict_local_authority(visibility, app_id, token, hub_url, state).await?
    else {
        return Ok(LocalPagePrincipalBinding::offline_owner(app_id));
    };

    if !has_page_local_permissions(&context) {
        return Err(flow_like_types::anyhow!(
            "Page local execution requires ReadBoards, ExecuteBoards, and ExecuteEvents permissions"
        ));
    }

    let token = token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Hosted local execution requires the caller's authentication token"
            )
        })?;
    Ok(LocalPagePrincipalBinding::authenticated(
        &context,
        token,
        hub_url.trim(),
    ))
}

fn has_page_local_permissions(context: &UserExecutionContext) -> bool {
    context.has_permission(READ_BOARDS_PERMISSION)
        && context.has_permission(EXECUTE_BOARDS_PERMISSION)
        && context.has_permission(EXECUTE_EVENTS_PERMISSION)
}

/// Direct board execution accepts a raw node id, so a hosted caller must
/// carry ExecuteBoards independently of any Page or Event permission.
pub(crate) async fn ensure_board_local_execution_authorized(
    visibility: &AppVisibility,
    app_id: &str,
    token: Option<&str>,
    hub_url: &str,
    state: &Arc<FlowLikeState>,
) -> flow_like_types::Result<()> {
    let Some(context) =
        resolve_strict_local_authority(visibility, app_id, token, hub_url, state).await?
    else {
        return Ok(());
    };
    if !has_board_local_permissions(&context) {
        return Err(flow_like_types::anyhow!(
            "Direct local board execution requires ExecuteBoards permission"
        ));
    }
    Ok(())
}

fn has_board_local_permissions(context: &UserExecutionContext) -> bool {
    context.has_permission(EXECUTE_BOARDS_PERMISSION)
}

/// Return `None` for device-owned apps and a freshly valid hosted identity for
/// server-backed apps. Unlike ordinary run attribution, this authorization
/// path resolves against the hub on every invocation and never reuses a cached
/// role when the hub cannot be reached.
async fn resolve_strict_local_authority(
    visibility: &AppVisibility,
    app_id: &str,
    token: Option<&str>,
    hub_url: &str,
    state: &Arc<FlowLikeState>,
) -> flow_like_types::Result<Option<UserExecutionContext>> {
    if matches!(visibility, AppVisibility::Offline) {
        return Ok(None);
    }

    let token = token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Hosted local execution requires the caller's authentication token"
            )
        })?;
    let hub_url = hub_url.trim();
    if hub_url.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Hosted local execution requires a configured hub"
        ));
    }

    let hub = Hub::new(hub_url, state.http_client.clone()).await?;
    let context = hub.execution_context(token, app_id).await?;
    // Run attribution may reuse this answer after the authorization decision;
    // subsequent Board/Page authorization calls still query the hub again.
    store(&cache_key(app_id, token), &context);
    Ok(Some(context))
}

async fn resolve_local_identity(
    visibility: &AppVisibility,
    app_id: &str,
    token: Option<&str>,
    hub_url: &str,
    state: &Arc<FlowLikeState>,
) -> LocalIdentity {
    if matches!(visibility, AppVisibility::Offline) {
        return LocalIdentity::OwnerEquivalent;
    }

    let Some(token) = token.map(str::trim).filter(|token| !token.is_empty()) else {
        tracing::warn!(
            app_id = %app_id,
            "Hosted app executed locally without a token; the run carries no permissions"
        );
        return LocalIdentity::Unresolved;
    };

    let hub_url = hub_url.trim();
    if hub_url.is_empty() {
        tracing::warn!(
            app_id = %app_id,
            "No hub configured; cannot resolve the executing user for a hosted app"
        );
        return LocalIdentity::Unresolved;
    }

    let key = cache_key(app_id, token);
    if let Some(context) = cached(&key, Some(IDENTITY_TTL)) {
        return LocalIdentity::Hosted(Box::new(context));
    }

    let resolved = match Hub::new(hub_url, state.http_client.clone()).await {
        Ok(hub) => hub.execution_context(token, app_id).await,
        Err(err) => Err(err),
    };

    match resolved {
        Ok(context) => {
            store(&key, &context);
            LocalIdentity::Hosted(Box::new(context))
        }
        // A stale answer still reflects a role an admin granted; dropping to
        // "no permissions" over a network blip would break working runs.
        Err(err) => match cached(&key, None) {
            Some(context) => {
                tracing::warn!(
                    app_id = %app_id,
                    error = %err,
                    "Could not refresh the executing user; reusing the last resolved role"
                );
                LocalIdentity::Hosted(Box::new(context))
            }
            None => {
                tracing::warn!(
                    app_id = %app_id,
                    error = %err,
                    "Could not resolve the executing user; the run carries no permissions"
                );
                LocalIdentity::Unresolved
            }
        },
    }
}

fn cache() -> &'static Mutex<HashMap<String, CachedIdentity>> {
    IDENTITY_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Two tokens for the same app resolve to different identities, so the token
/// is part of the key. It is hashed rather than stored: the cache outlives any
/// single run, and a digest is all a lookup needs.
fn cache_key(app_id: &str, token: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(token.as_bytes());
    format!("{}|{}", app_id, hasher.finalize().to_hex())
}

/// `max_age` of `None` accepts an entry of any age — the deliberate fallback
/// when the hub cannot be reached.
fn cached(key: &str, max_age: Option<Duration>) -> Option<UserExecutionContext> {
    let guard = cache().lock().ok()?;
    let entry = guard.get(key)?;
    match max_age {
        Some(max_age) if entry.resolved_at.elapsed() > max_age => None,
        _ => Some(entry.context.clone()),
    }
}

fn store(key: &str, context: &UserExecutionContext) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(
            key.to_string(),
            CachedIdentity {
                context: context.clone(),
                resolved_at: Instant::now(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::execution::RoleContext;

    #[test]
    fn cache_key_separates_apps_and_tokens() {
        assert_ne!(cache_key("app-a", "token"), cache_key("app-b", "token"));
        assert_ne!(cache_key("app-a", "token-1"), cache_key("app-a", "token-2"));
        assert_eq!(cache_key("app-a", "token"), cache_key("app-a", "token"));
    }

    #[test]
    fn cache_key_does_not_leak_the_token() {
        assert!(!cache_key("app-a", "pat_secret.value").contains("secret"));
    }

    #[test]
    fn stale_entries_are_only_served_without_a_max_age() {
        let key = cache_key("stale-app", "token");
        store(
            &key,
            &UserExecutionContext::new("user-1").with_role(RoleContext::admin()),
        );

        assert!(cached(&key, Some(Duration::ZERO)).is_none());
        assert_eq!(
            cached(&key, None).map(|context| context.sub).as_deref(),
            Some("user-1")
        );
    }

    #[test]
    fn page_local_execution_requires_all_three_permissions() {
        let context = |permissions| {
            UserExecutionContext::new("user-1").with_role(RoleContext {
                id: "role-1".to_string(),
                name: "Runtime".to_string(),
                permissions,
                attributes: Vec::new(),
                custom_attributes: HashMap::new(),
            })
        };

        assert!(!has_page_local_permissions(&context(
            READ_BOARDS_PERMISSION
        )));
        assert!(!has_page_local_permissions(&context(
            READ_BOARDS_PERMISSION | EXECUTE_EVENTS_PERMISSION
        )));
        assert!(!has_page_local_permissions(&context(
            READ_BOARDS_PERMISSION | EXECUTE_BOARDS_PERMISSION
        )));
        assert!(has_page_local_permissions(&context(
            READ_BOARDS_PERMISSION | EXECUTE_BOARDS_PERMISSION | EXECUTE_EVENTS_PERMISSION
        )));
    }

    #[test]
    fn owner_and_admin_roles_imply_page_local_permissions() {
        for permissions in [RoleContext::OWNER_PERMISSION, RoleContext::ADMIN_PERMISSION] {
            let context = UserExecutionContext::new("user-1").with_role(RoleContext {
                id: "role-1".to_string(),
                name: "Privileged".to_string(),
                permissions,
                attributes: Vec::new(),
                custom_attributes: HashMap::new(),
            });
            assert!(has_page_local_permissions(&context));
        }
    }

    #[test]
    fn direct_board_execution_requires_execute_boards() {
        let context = |permissions| {
            UserExecutionContext::new("user-1").with_role(RoleContext {
                id: "role-1".to_string(),
                name: "Runtime".to_string(),
                permissions,
                attributes: Vec::new(),
                custom_attributes: HashMap::new(),
            })
        };

        assert!(!has_board_local_permissions(&context(
            EXECUTE_EVENTS_PERMISSION
        )));
        assert!(has_board_local_permissions(&context(
            EXECUTE_BOARDS_PERMISSION
        )));
        assert!(has_board_local_permissions(&context(
            RoleContext::ADMIN_PERMISSION
        )));
    }
}
