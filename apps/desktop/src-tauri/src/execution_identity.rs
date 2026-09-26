//! Execution identity for runs started on this device.
//!
//! Offline apps and hosted apps differ fundamentally here. An offline app has
//! no server-side role to consult — the machine owns it, so the run is
//! owner-equivalent. A hosted app does have one, and a local run has to
//! reproduce it, or the same board answers `Has Permission` with "yes" on the
//! desktop and "no" in the cloud.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use flow_like::{
    app::AppVisibility,
    flow::execution::{InternalRun, UserExecutionContext, extract_sub_from_jwt},
    hub::Hub,
    state::FlowLikeState,
};
use flow_like_types::authorization::AuthorizationError;
use serde::{Deserialize, Serialize};

use crate::local_page_actions::LocalPagePrincipalBinding;

/// How long a resolved identity is reused without asking the hub again. Sinks
/// fire as often as every minute, and the answer only changes when an admin
/// edits a role, so a per-run round trip would be pure latency.
const IDENTITY_TTL: Duration = Duration::from_secs(5 * 60);

/// How long a hub-confirmed role keeps authorizing local runs of a hosted app
/// while the hub cannot be reached. A refusal from a reachable hub always wins,
/// so revocations take effect the next time the device is online.
const OFFLINE_AUTHORITY_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// A hub that has not answered by now is treated as unreachable for this run.
const HUB_AUTHORITY_TIMEOUT: Duration = Duration::from_secs(15);

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
/// board-read, board-execution, and Event-execution authority. Only an
/// unreachable hub falls back to the role it last confirmed for this caller,
/// and only within [`OFFLINE_AUTHORITY_MAX_AGE`].
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

/// Return `None` for device-owned apps and a hosted identity for server-backed
/// apps. Unlike ordinary run attribution, this path asks the hub on every
/// invocation. When the hub cannot be reached it accepts the role the hub last
/// confirmed for this caller, if that is recent enough; any answer from a
/// reachable hub, including a refusal, is final.
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

    let authority = authority_key(hub_url, app_id, token);
    match ask_hub(hub_url, token, app_id, state).await {
        Ok(context) => {
            // Run attribution may reuse this answer after the authorization decision;
            // subsequent Board/Page authorization calls still query the hub again.
            store(&cache_key(app_id, token), &context);
            persist_authority(&authority, &context);
            Ok(Some(context))
        }
        Err(error) if hub_unreachable(&error) => {
            match load_authority(&authority, OFFLINE_AUTHORITY_MAX_AGE) {
                Some(context) => {
                    tracing::warn!(
                        app_id = %app_id,
                        error = %error,
                        "Hub unreachable; authorizing the local run with the last role it confirmed"
                    );
                    Ok(Some(context))
                }
                None => Err(error.context(
                    "the hub is unreachable and it has not confirmed this user's role on this device within the last 7 days",
                )),
            }
        }
        Err(error) => {
            if hub_refused(&error) {
                forget_authority(&authority);
            }
            Err(error)
        }
    }
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

    let authority = authority_key(hub_url, app_id, token);
    match ask_hub(hub_url, token, app_id, state).await {
        Ok(context) => {
            store(&key, &context);
            persist_authority(&authority, &context);
            LocalIdentity::Hosted(Box::new(context))
        }
        // A stale answer still reflects a role an admin granted; dropping to
        // "no permissions" over a network blip would break working runs.
        Err(err) => match cached(&key, None).or_else(|| {
            hub_unreachable(&err)
                .then(|| load_authority(&authority, OFFLINE_AUTHORITY_MAX_AGE))
                .flatten()
        }) {
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

async fn ask_hub(
    hub_url: &str,
    token: &str,
    app_id: &str,
    state: &Arc<FlowLikeState>,
) -> flow_like_types::Result<UserExecutionContext> {
    let request = async {
        let hub = Hub::new(hub_url, state.http_client.clone())
            .await
            .map_err(|error| {
                flow_like_types::Error::new(AuthorizationError::Unavailable)
                    .context(format!("hub configuration could not be loaded: {error}"))
            })?;
        hub.execution_context(token, app_id).await
    };
    flow_like_types::tokio::time::timeout(HUB_AUTHORITY_TIMEOUT, request)
        .await
        .unwrap_or_else(|_| {
            Err(flow_like_types::Error::new(AuthorizationError::Unavailable)
                .context("the hub did not answer the execution context request in time"))
        })
}

fn hub_unreachable(error: &flow_like_types::Error) -> bool {
    matches!(
        error.downcast_ref::<AuthorizationError>(),
        Some(AuthorizationError::Unavailable)
    )
}

fn hub_refused(error: &flow_like_types::Error) -> bool {
    matches!(
        error.downcast_ref::<AuthorizationError>(),
        Some(AuthorizationError::Denied)
    )
}

#[derive(Serialize, Deserialize)]
struct PersistedAuthority {
    confirmed_at: u64,
    context: UserExecutionContext,
}

/// Tokens rotate, so the persisted role is keyed by the caller's subject; a
/// token that carries none (a personal access token) is its own identity.
fn authority_key(hub_url: &str, app_id: &str, token: &str) -> String {
    let caller = extract_sub_from_jwt(token)
        .map(|subject| format!("sub:{subject}"))
        .unwrap_or_else(|_| format!("token:{}", blake3::hash(token.as_bytes()).to_hex()));
    let mut hasher = blake3::Hasher::new();
    for part in [
        hub_url.trim().trim_end_matches('/'),
        app_id,
        caller.as_str(),
    ] {
        hasher.update(part.as_bytes());
        hasher.update(&[0]);
    }
    hasher.finalize().to_hex().to_string()
}

fn authority_path(key: &str) -> PathBuf {
    crate::settings::execution_authority_dir().join(format!("{key}.json"))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

fn persist_authority(key: &str, context: &UserExecutionContext) {
    if let Err(error) = write_authority(&authority_path(key), context, unix_now()) {
        tracing::warn!(%error, "Could not persist the confirmed execution role");
    }
}

fn load_authority(key: &str, max_age: Duration) -> Option<UserExecutionContext> {
    read_authority(&authority_path(key), max_age, unix_now())
}

fn write_authority(
    path: &std::path::Path,
    context: &UserExecutionContext,
    confirmed_at: u64,
) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = serde_json::to_vec(&PersistedAuthority {
        confirmed_at,
        context: context.clone(),
    })?;
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, body)?;
    std::fs::rename(&temp, path)
}

/// A confirmation dated in the future (a clock moved back) is not trusted,
/// beyond a few minutes of ordinary clock adjustment.
fn read_authority(
    path: &std::path::Path,
    max_age: Duration,
    now: u64,
) -> Option<UserExecutionContext> {
    const CLOCK_SKEW_SECS: u64 = 5 * 60;
    let persisted: PersistedAuthority = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let fresh = persisted.confirmed_at <= now.saturating_add(CLOCK_SKEW_SECS)
        && now.saturating_sub(persisted.confirmed_at) <= max_age.as_secs();
    if !fresh {
        let _ = std::fs::remove_file(path);
        return None;
    }
    Some(persisted.context)
}

fn forget_authority(key: &str) {
    let _ = std::fs::remove_file(authority_path(key));
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

    const ALICE_FIRST: &str = "e30.eyJzdWIiOiJhbGljZSJ9.first";
    const ALICE_ROTATED: &str = "e30.eyJzdWIiOiJhbGljZSJ9.second";
    const BOB: &str = "e30.eyJzdWIiOiJib2IifQ.signature";

    #[test]
    fn persisted_authority_survives_token_rotation_but_not_a_change_of_caller() {
        let key = |hub, app, token| authority_key(hub, app, token);
        assert_eq!(
            key("https://hub", "app", ALICE_FIRST),
            key("https://hub/", "app", ALICE_ROTATED)
        );
        assert_ne!(
            key("https://hub", "app", ALICE_FIRST),
            key("https://hub", "app", BOB)
        );
        assert_ne!(
            key("https://hub", "app", ALICE_FIRST),
            key("https://other", "app", ALICE_FIRST)
        );
        assert_ne!(
            key("https://hub", "app", ALICE_FIRST),
            key("https://hub", "other", ALICE_FIRST)
        );
        assert_ne!(
            key("https://hub", "app", "pat_one"),
            key("https://hub", "app", "pat_two")
        );
    }

    #[test]
    fn persisted_authority_expires_and_rejects_future_confirmations() {
        let dir =
            std::env::temp_dir().join(format!("flow-like-authority-test-{}", std::process::id()));
        let path = dir.join("entry.json");
        let context = UserExecutionContext::new("alice").with_role(RoleContext::admin());
        let week = OFFLINE_AUTHORITY_MAX_AGE;
        let confirmed = 1_000_000;

        write_authority(&path, &context, confirmed).unwrap();
        assert_eq!(
            read_authority(&path, week, confirmed + week.as_secs())
                .map(|context| context.sub)
                .as_deref(),
            Some("alice")
        );
        assert!(read_authority(&path, week, confirmed + week.as_secs() + 1).is_none());
        assert!(!path.exists(), "an expired confirmation is deleted");

        write_authority(&path, &context, confirmed + 3_600).unwrap();
        assert!(read_authority(&path, week, confirmed).is_none());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_an_unreachable_hub_permits_the_offline_fallback() {
        let error = |kind| flow_like_types::Error::new(kind).context("execution context failed");
        assert!(hub_unreachable(&error(AuthorizationError::Unavailable)));
        for kind in [
            AuthorizationError::Denied,
            AuthorizationError::Expired,
            AuthorizationError::InvalidResponse,
        ] {
            assert!(!hub_unreachable(&error(kind)));
        }
        assert!(!hub_unreachable(&flow_like_types::anyhow!(
            "untyped failure"
        )));
        assert!(hub_refused(&error(AuthorizationError::Denied)));
        assert!(!hub_refused(&error(AuthorizationError::Expired)));
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
