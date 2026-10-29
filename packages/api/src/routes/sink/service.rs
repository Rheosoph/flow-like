//! Sink service operations
//!
//! Handles the creation, update, and deletion of sinks in external schedulers
//! (AWS EventBridge, Kubernetes CronJobs) when event sinks are modified.

use super::authority::{RunAsDenial, RunAsVerdict, actor_authority, bind_run_as};
use crate::entity::{event, event_sink, pat, user};
use crate::error::ApiError;
use crate::permission::role_permission::{RolePermissions, has_role_permission};
use crate::routes::app::events::db::{decrypt_token, extract_scheduled_for, get_event_from_db_opt};
use crate::state::AppState;
use axum::http::StatusCode;
use flow_like_types::anyhow;
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::{NotSet, Set},
    ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter,
    QuerySelect, UpdateMany,
    prelude::DateTimeWithTimeZone,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Sink type constants
pub mod sink_types {
    pub const HTTP: &str = "http";
    pub const CRON: &str = "cron";
    pub const DISCORD: &str = "discord";
    pub const TELEGRAM: &str = "telegram";
    pub const EMAIL: &str = "email";
    pub const CHAT: &str = "chat";
}

fn normalize_http_auth_token(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some((scheme, token)) = trimmed.split_once(' ')
        && scheme.eq_ignore_ascii_case("Bearer")
    {
        return token.trim();
    }
    trimmed
}

fn normalized_http_auth_token(value: Option<&str>) -> Option<String> {
    value
        .map(normalize_http_auth_token)
        .filter(|token| !token.is_empty())
        .map(ToOwned::to_owned)
}

fn normalized_http_method(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|method| !method.is_empty())
        .map(str::to_ascii_uppercase)
}

fn normalized_secret(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|secret| !secret.is_empty())
        .map(ToOwned::to_owned)
}

/// Configuration for creating/updating a sink
#[derive(Debug, Clone, Default)]
pub struct SinkConfig {
    pub event_id: String,
    pub app_id: String,
    pub sink_type: String,
    pub path: Option<String>,
    /// HTTP method of an api/http/webhook event (GET/POST/...), stored so different
    /// methods on the same path route to different events. The sink mirrors it: `None`
    /// clears the stored method.
    pub method: Option<String>,
    pub auth_token: Option<String>,
    /// Discord public key or Telegram secret token the bot door verifies. `None` keeps
    /// the stored one, unless the sink type changes.
    pub webhook_secret: Option<String>,
    pub cron_expression: Option<String>,
    pub cron_timezone: Option<String>,
    pub cron_scheduled_for: Option<flow_like_sinks::ScheduledLocal>,
    /// The user saving the event, who becomes the sink's run identity. `None` (system
    /// syncs) leaves the identity as is. When this is someone else than the stored
    /// run-as user, OAuth grants and profile not supplied here are cleared.
    pub run_as_user_id: Option<String>,
    /// Encrypted OAuth tokens JSON (optional - for provider-specific access)
    pub oauth_tokens_encrypted: Option<String>,
    /// Snapshot of the last updater's profile (bits, hubs) for trigger execution
    pub profile_json: Option<serde_json::Value>,
    /// Whether the sink should be active (synced from event.active)
    pub active: Option<bool>,
    /// Drops the stored path and method so the row no longer holds the unique
    /// (app, path, method) slot; set for events that never serve on the hub.
    pub release_route: bool,
}

/// Refused because the actor may not become the run-as user of an armed hub trigger.
pub(crate) const RUN_AS_NOT_ALLOWED: &str = "RUN_AS_NOT_ALLOWED";

const RELEASE_ACTOR: &str = "sink-run-as";

/// The rule every fire checks, applied when `actor` arms a hub trigger (turns it on, or
/// saves an event whose trigger is on), so no sink is armed for someone it would refuse.
/// The app's own state is left to the fire: a sink armed while the project is not active
/// fires once it is.
pub(crate) async fn ensure_actor_may_arm(
    state: &AppState,
    app_id: &str,
    actor: &str,
) -> Result<(), ApiError> {
    match actor_authority(&state.db, app_id, actor, RolePermissions::ExecuteEvents).await? {
        RunAsVerdict::Allowed(_) => Ok(()),
        RunAsVerdict::Denied(denial) => Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            RUN_AS_NOT_ALLOWED,
            arm_refusal(denial),
        )),
        RunAsVerdict::Transient(reason) => Err(ApiError::service_unavailable(format!(
            "Cannot check whether user {actor} may run the hub triggers of app {app_id}: {reason}"
        ))),
    }
}

const fn arm_refusal(denial: RunAsDenial) -> &'static str {
    match denial {
        RunAsDenial::AccountInactive => {
            "Your account is not active, so you cannot turn on this event's hub trigger"
        }
        RunAsDenial::NotMember => {
            "You are not a member of this project, so you cannot turn on this event's hub trigger"
        }
        RunAsDenial::CannotTrigger => {
            "Your role does not allow triggering events, so you cannot turn on this event's hub trigger"
        }
        RunAsDenial::AppInactive | RunAsDenial::TokenInvalid => {
            "You cannot turn on this event's hub trigger"
        }
    }
}

/// Members per run-as query of a release.
const RELEASE_CHUNK: usize = 500;

/// Pauses every sink that runs as `user_id`, in `app_id` or in every app, drops the
/// run-as user with the PAT, OAuth grants and profile it ran with, and tells each app's
/// owners and admins why. A paused sink is marked released, so only runs as that user
/// stop and runs a person triggers go on. A sink that was already off only loses that
/// identity, without a notice, and keeps stopping every run. Whoever turns a sink back
/// on runs it from then on. Returns how many sinks this call paused.
pub(crate) async fn release_member_sinks(
    state: &AppState,
    app_id: Option<&str>,
    user_id: &str,
    denial: RunAsDenial,
) -> flow_like_types::Result<usize> {
    let released: Vec<event_sink::Model> =
        pause_member_sinks(&state.db, app_id, &[user_id.to_owned()])
            .await?
            .into_iter()
            .map(|(_, sink)| sink)
            .collect();
    announce_release(state, &released, denial, Some(user_id)).await;
    Ok(released.len())
}

/// [`release_departed_members`] for one member who left `app_id`.
pub(crate) async fn release_departed_member(
    state: &AppState,
    app_id: &str,
    user_id: &str,
) -> flow_like_types::Result<usize> {
    release_departed_members(state, app_id, &[user_id.to_owned()]).await
}

/// Releases the sinks of members who left `app_id`, see [`release_app_members`].
pub(crate) async fn release_departed_members(
    state: &AppState,
    app_id: &str,
    user_ids: &[String],
) -> flow_like_types::Result<usize> {
    release_app_members(state, app_id, user_ids, RunAsDenial::NotMember).await
}

/// Releases the sinks of members of `app_id` whose new role no longer allows triggering
/// events, see [`release_app_members`]. The role change has committed, so a failure is
/// only logged: every fire refuses those sinks anyway.
pub(crate) async fn release_demoted_members(state: &AppState, app_id: &str, user_ids: &[String]) {
    if let Err(error) =
        release_app_members(state, app_id, user_ids, RunAsDenial::CannotTrigger).await
    {
        tracing::error!(%app_id, members = user_ids.len(), %error, "Failed to pause the event sinks of members whose role no longer allows triggering events");
    }
}

/// Whether a member whose role changes from `before` to `after` can no longer trigger
/// events. Owner and Admin imply ExecuteEvents.
pub(crate) fn loses_execute_events(before: RolePermissions, after: RolePermissions) -> bool {
    let triggers =
        |role: &RolePermissions| has_role_permission(role, RolePermissions::ExecuteEvents);
    triggers(&before) && !triggers(&after)
}

/// [`release_member_sinks`] in `app_id` for each of `user_ids`, together with the app's
/// legacy sinks whose stored PAT is theirs, with one notice per member. Returns how many
/// sinks this call paused.
async fn release_app_members(
    state: &AppState,
    app_id: &str,
    user_ids: &[String],
    denial: RunAsDenial,
) -> flow_like_types::Result<usize> {
    if user_ids.is_empty() {
        return Ok(0);
    }
    let mut released = pause_member_sinks(&state.db, Some(app_id), user_ids).await?;
    released
        .extend(pause_legacy_sinks_of(&state.db, &state.encryption_key, app_id, user_ids).await?);
    let mut paused = 0;
    for (user_id, sinks) in by_member(released) {
        announce_release(state, &sinks, denial, Some(&user_id)).await;
        paused += sinks.len();
    }
    Ok(paused)
}

/// Pauses a legacy sink (stored PAT, no run-as user) whose PAT or PAT owner was refused,
/// as long as it still holds the PAT the refused fire read. `owner` is the PAT's owner
/// when the PAT still names one. Returns whether it paused the sink; one turned off in
/// the meantime only loses the PAT.
pub(crate) async fn release_legacy_sink(
    state: &AppState,
    sink: &event_sink::Model,
    owner: Option<&str>,
    denial: RunAsDenial,
) -> flow_like_types::Result<bool> {
    let paused = pause_legacy_sink(&state.db, sink).await? == RowRelease::Paused;
    if paused {
        announce_release(state, std::slice::from_ref(sink), denial, owner).await;
    }
    Ok(paused)
}

/// Drops a sink's identity and credentials and leaves it off. Only a release that pauses a
/// sink that was on marks it released, which stops the runs that use its run-as user and lets
/// runs a person triggers continue; a sink a person had switched off keeps stopping every run.
fn pause_sink(
    sink_id: &str,
    now: DateTimeWithTimeZone,
    release: RowRelease,
) -> UpdateMany<event_sink::Entity> {
    event_sink::Entity::update_many()
        .set(event_sink::ActiveModel {
            active: Set(false),
            run_as_user_id: Set(None),
            run_as_released_at: match release {
                RowRelease::Paused => Set(Some(now)),
                RowRelease::Cleared | RowRelease::Missed => NotSet,
            },
            pat_encrypted: Set(None),
            oauth_tokens_encrypted: Set(None),
            profile_json: Set(None),
            updated_at: Set(now),
            ..Default::default()
        })
        .filter(event_sink::Column::Id.eq(sink_id))
}

/// What a release did to one sink row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowRelease {
    /// The sink was on and is paused now.
    Paused,
    /// The sink was already off and only lost its identity and credentials.
    Cleared,
    /// The row changed since it was read: another release got there first, or the sink
    /// runs as someone else or holds another PAT now.
    Missed,
}

/// Drops the identity and credentials of `sink` while `guard` still holds, and pauses it.
/// Each update also matches the active state, so the one that applies tells whether the
/// sink was on; the state that was read is tried first.
async fn release_row<C: ConnectionTrait>(
    db: &C,
    sink: &event_sink::Model,
    guard: Condition,
) -> Result<RowRelease, DbErr> {
    let attempts = if sink.active {
        [RowRelease::Paused, RowRelease::Cleared]
    } else {
        [RowRelease::Cleared, RowRelease::Paused]
    };
    let now = chrono::Utc::now().fixed_offset();
    for release in attempts {
        let updated = pause_sink(&sink.id, now, release)
            .filter(guard.clone())
            .filter(event_sink::Column::Active.eq(release == RowRelease::Paused))
            .exec(db)
            .await?;
        if updated.rows_affected == 1 {
            return Ok(release);
        }
    }
    Ok(RowRelease::Missed)
}

/// The rows a release announces: only the sinks it paused. A sink that was already off
/// loses its identity without an audit entry or notice, so nobody is asked to turn back
/// on an automation someone had turned off.
fn announced<T>(rows: Vec<(T, RowRelease)>) -> Vec<T> {
    rows.into_iter()
        .filter_map(|(row, release)| (release == RowRelease::Paused).then_some(row))
        .collect()
}

/// Released sinks per member they ran as, for one notice each.
fn by_member(
    released: Vec<(String, event_sink::Model)>,
) -> BTreeMap<String, Vec<event_sink::Model>> {
    let mut grouped: BTreeMap<String, Vec<event_sink::Model>> = BTreeMap::new();
    for (member, sink) in released {
        grouped.entry(member).or_default().push(sink);
    }
    grouped
}

/// Releases every sink that runs as one of `user_ids`, in `app_id` or in every app, and
/// returns the ones it paused with the member each ran as. Each update is guarded on the
/// run-as user it read, so concurrent releases pause, and announce, a sink once.
async fn pause_member_sinks<C: ConnectionTrait>(
    db: &C,
    app_id: Option<&str>,
    user_ids: &[String],
) -> Result<Vec<(String, event_sink::Model)>, DbErr> {
    let mut released = Vec::new();
    for members in user_ids.chunks(RELEASE_CHUNK) {
        let mut query = event_sink::Entity::find()
            .filter(event_sink::Column::RunAsUserId.is_in(members.iter().map(String::as_str)));
        if let Some(app_id) = app_id {
            query = query.filter(event_sink::Column::AppId.eq(app_id));
        }
        for sink in query.all(db).await? {
            let Some(member) = sink.run_as_user_id.clone() else {
                continue;
            };
            let guard = Condition::all().add(event_sink::Column::RunAsUserId.eq(member.as_str()));
            let release = release_row(db, &sink, guard).await?;
            released.push(((member, sink), release));
        }
    }
    Ok(announced(released))
}

async fn pause_legacy_sink<C: ConnectionTrait>(
    db: &C,
    sink: &event_sink::Model,
) -> Result<RowRelease, DbErr> {
    let Some(ciphertext) = sink.pat_encrypted.as_deref() else {
        return Ok(RowRelease::Missed);
    };
    let guard = Condition::all()
        .add(event_sink::Column::RunAsUserId.is_null())
        .add(event_sink::Column::PatEncrypted.eq(ciphertext));
    release_row(db, sink, guard).await
}

/// Releases the app's legacy sinks whose stored PAT belongs to one of `user_ids`, and
/// returns the ones it paused with that owner. One PAT query covers every legacy sink of
/// the app.
async fn pause_legacy_sinks_of<C: ConnectionTrait>(
    db: &C,
    key: &[u8; 32],
    app_id: &str,
    user_ids: &[String],
) -> Result<Vec<(String, event_sink::Model)>, DbErr> {
    let legacy = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(app_id))
        .filter(event_sink::Column::RunAsUserId.is_null())
        .filter(event_sink::Column::PatEncrypted.is_not_null())
        .all(db)
        .await?;
    let by_pat = stored_pat_ids(legacy, key);
    let owners = pat_owners(db, &by_pat).await?;
    let members: HashSet<&str> = user_ids.iter().map(String::as_str).collect();
    let mut released = Vec::new();
    for (pat_id, sink) in by_pat {
        let Some(owner) = owners
            .get(&pat_id)
            .filter(|owner| members.contains(owner.as_str()))
        else {
            continue;
        };
        let release = pause_legacy_sink(db, &sink).await?;
        released.push(((owner.clone(), sink), release));
    }
    Ok(announced(released))
}

/// Each sink with the id of the PAT it stores. A PAT that does not decrypt or parse
/// names nobody.
fn stored_pat_ids(
    sinks: Vec<event_sink::Model>,
    key: &[u8; 32],
) -> Vec<(String, event_sink::Model)> {
    sinks
        .into_iter()
        .filter_map(|sink| {
            let token = decrypt_token(sink.pat_encrypted.as_deref()?, key)?;
            let (pat_id, _) = super::trigger::parse_pat_token(&token)?;
            Some((pat_id.to_owned(), sink))
        })
        .collect()
}

/// The owner of each stored PAT that still exists, by PAT id.
async fn pat_owners<C: ConnectionTrait>(
    db: &C,
    by_pat: &[(String, event_sink::Model)],
) -> Result<HashMap<String, String>, DbErr> {
    if by_pat.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(pat::Entity::find()
        .select_only()
        .column(pat::Column::Id)
        .column(pat::Column::UserId)
        .filter(pat::Column::Id.is_in(by_pat.iter().map(|(pat_id, _)| pat_id.as_str())))
        .into_tuple::<(String, String)>()
        .all(db)
        .await?
        .into_iter()
        .collect())
}

/// Stops the schedules of released cron sinks, writes one `sink.release` audit entry per
/// sink and notifies each app's owners and admins once. Every sink in `released` was on
/// until the release paused it.
async fn announce_release(
    state: &AppState,
    released: &[event_sink::Model],
    denial: RunAsDenial,
    run_as_user_id: Option<&str>,
) {
    for sink in released {
        record_release(state, sink, denial, run_as_user_id).await;
    }
    if !released.is_empty() {
        notify_release(state, released, denial, run_as_user_id).await;
    }
}

async fn record_release(
    state: &AppState,
    sink: &event_sink::Model,
    denial: RunAsDenial,
    run_as_user_id: Option<&str>,
) {
    if sink.sink_type == sink_types::CRON
        && let Err(error) = disable_external_schedule(state, &sink.event_id).await
    {
        tracing::warn!(event_id = %sink.event_id, %error, "Failed to disable schedule of a released sink");
    }
    crate::audit::record_entry(
        state,
        crate::audit::AuditRecordInput::system(
            RELEASE_ACTOR,
            "sink.release",
            "sink",
            &sink.event_id,
        )
        .on_scope(&sink.app_id)
        .with_details(serde_json::json!({
            "reason": denial.reason(),
            "run_as_user_id": run_as_user_id,
        })),
    )
    .await;
}

async fn notify_release(
    state: &AppState,
    released: &[event_sink::Model],
    denial: RunAsDenial,
    run_as_user_id: Option<&str>,
) {
    let mut by_app: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for sink in released {
        by_app
            .entry(sink.app_id.as_str())
            .or_default()
            .push(sink.event_id.clone());
    }
    let who = released_user_name(state, run_as_user_id, denial).await;
    for (app_id, event_ids) in by_app {
        let names = event_names(state, event_ids).await;
        crate::routes::app::connection::notify_app_admins_at(
            state,
            app_id,
            format!("{} paused", plural_events(names.len())),
            denial.notice(&who, &names),
            "pause",
            format!("/library/config/events?id={app_id}"),
        )
        .await;
    }
}

async fn released_user_name(
    state: &AppState,
    user_id: Option<&str>,
    denial: RunAsDenial,
) -> String {
    let name = match user_id {
        Some(user_id) => user::Entity::find_by_id(user_id)
            .one(&state.db)
            .await
            .ok()
            .flatten()
            .and_then(|user| user.name.or(user.preferred_username).or(user.username))
            .filter(|name| !name.trim().is_empty()),
        None => None,
    };
    name.unwrap_or_else(|| match denial {
        RunAsDenial::NotMember => "A former member".to_owned(),
        _ => "A member".to_owned(),
    })
}

async fn event_names(state: &AppState, event_ids: Vec<String>) -> Vec<String> {
    event::Entity::find()
        .select_only()
        .column(event::Column::Name)
        .filter(event::Column::Id.is_in(event_ids.clone()))
        .into_tuple::<String>()
        .all(&state.db)
        .await
        .unwrap_or(event_ids)
}

fn plural_events(count: usize) -> String {
    if count == 1 {
        "1 event".to_string()
    } else {
        format!("{count} events")
    }
}

/// Sync a sink to the database and external schedulers
///
/// This is called when an event is created/updated to ensure its sink exists
/// and is properly configured in external systems (EventBridge, K8s CronJobs).
pub async fn sync_sink(
    db: &DatabaseConnection,
    state: &AppState,
    config: SinkConfig,
) -> flow_like_types::Result<event_sink::Model> {
    let now = chrono::Utc::now().fixed_offset();

    // Check if sink already exists
    let existing = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&config.event_id))
        .one(db)
        .await?;

    let sink = if let Some(existing) = existing {
        let old_cron = existing.cron_expression.clone();
        let old_cron_timezone = existing.cron_timezone.clone();
        let old_active = existing.active;
        let old_sink_type = existing.sink_type.clone();
        let new_active = config.active.unwrap_or(old_active);

        let updated = synced_sink(existing, &config, now).update(db).await?;

        // Handle scheduler updates for cron sinks
        if config.sink_type == sink_types::CRON {
            let active_changed = new_active != old_active;
            let has_schedule =
                config.cron_expression.is_some() || config.cron_scheduled_for.is_some();
            let schedule_rebuild_needed = cron_schedule_needs_rebuild(
                &old_cron,
                &config.cron_expression,
                &old_cron_timezone,
                &config.cron_timezone,
                &old_sink_type,
                &config.sink_type,
                config.cron_scheduled_for.as_ref(),
            );

            // Collapse "change expression + toggle active" into a single atomic
            // update. The previous two-step dance (update → disable) left a
            // race window where a freshly-enabled schedule could fire before
            // the follow-up disable landed.
            if (schedule_rebuild_needed || active_changed) && has_schedule {
                let cron_expr = config.cron_expression.as_deref().unwrap_or("");
                update_external_scheduler(
                    state,
                    &config.event_id,
                    cron_expr,
                    config.cron_timezone.as_deref(),
                    config.cron_scheduled_for.as_ref(),
                    new_active,
                )
                .await?;
            } else if active_changed {
                // No schedule to (re)build — just toggle state.
                if new_active {
                    enable_external_schedule(state, &config.event_id).await?;
                } else {
                    disable_external_schedule(state, &config.event_id).await?;
                }
            }
        } else if old_sink_type == sink_types::CRON && config.sink_type != sink_types::CRON {
            // Type changed from cron to something else - delete the schedule
            delete_external_schedule(state, &config.event_id).await?;
        }

        updated
    } else {
        // Create new sink
        let sink_id = flow_like_types::create_id();
        let initial_active = config.active.unwrap_or(true);

        let active_model = event_sink::ActiveModel {
            id: Set(sink_id),
            event_id: Set(config.event_id.clone()),
            app_id: Set(config.app_id.clone()),
            sink_type: Set(config.sink_type.clone()),
            active: Set(initial_active),
            path: Set(config.path.clone()),
            method: Set(normalized_http_method(config.method.as_deref())),
            auth_token: Set(normalized_http_auth_token(config.auth_token.as_deref())),
            webhook_secret: Set(normalized_secret(config.webhook_secret.as_deref())),
            cron_expression: Set(config.cron_expression.clone()),
            cron_timezone: Set(config.cron_timezone.clone()),
            pat_encrypted: Set(None),
            run_as_user_id: Set(config.run_as_user_id),
            run_as_released_at: Set(None),
            oauth_tokens_encrypted: Set(config.oauth_tokens_encrypted),
            profile_json: Set(config.profile_json),
            created_at: Set(now),
            updated_at: Set(now),
        };

        let created = active_model.insert(db).await?;

        // Create schedule in external system for cron sinks
        if config.sink_type == sink_types::CRON
            && (config.cron_expression.is_some() || config.cron_scheduled_for.is_some())
        {
            let cron_expr = config.cron_expression.as_deref().unwrap_or("");
            create_external_schedule(
                state,
                &config.event_id,
                cron_expr,
                config.cron_timezone.as_deref(),
                config.cron_scheduled_for.as_ref(),
                initial_active,
            )
            .await?;
        }

        created
    };

    Ok(sink)
}

/// The columns a sync writes to an existing sink. Only supplied fields overwrite, except
/// that the method mirrors the event, a new sink type keeps no bot secret of the old one,
/// and a new run-as user keeps none of the previous person's OAuth grants or profile. A
/// sync that turns the sink on or off ends a release's pause.
fn synced_sink(
    existing: event_sink::Model,
    config: &SinkConfig,
    now: DateTimeWithTimeZone,
) -> event_sink::ActiveModel {
    let rebinds = config
        .run_as_user_id
        .as_deref()
        .is_some_and(|actor| existing.run_as_user_id.as_deref() != Some(actor));
    let type_changed = existing.sink_type != config.sink_type;
    let new_active = config.active.unwrap_or(existing.active);
    let active_changed = new_active != existing.active;
    let mut model: event_sink::ActiveModel = existing.into();

    model.sink_type = Set(config.sink_type.clone());
    model.updated_at = Set(now);
    if active_changed {
        model.active = Set(new_active);
        model.run_as_released_at = Set(None);
    }
    sync_route(&mut model, config);
    sync_bot_and_schedule(&mut model, config, type_changed);
    sync_identity(&mut model, config, rebinds);
    model
}

fn sync_route(model: &mut event_sink::ActiveModel, config: &SinkConfig) {
    // Only an HTTP-typed event has a method, so one that changes type gives up its
    // (app, path, method) slot.
    model.method = Set(normalized_http_method(config.method.as_deref()));
    if config.release_route {
        model.path = Set(None);
        model.method = Set(None);
    }
    if config.path.is_some() {
        model.path = Set(config.path.clone());
    }
    // HTTP sinks mirror the event config exactly (clearing the token clears the
    // column); other sink types only overwrite when a token was supplied.
    if config.sink_type == sink_types::HTTP || config.auth_token.is_some() {
        model.auth_token = Set(normalized_http_auth_token(config.auth_token.as_deref()));
    }
}

/// A bot secret only opens the door of the sink type it was stored for: a Discord public
/// key is public, so it must not become a Telegram secret token.
fn sync_bot_and_schedule(
    model: &mut event_sink::ActiveModel,
    config: &SinkConfig,
    type_changed: bool,
) {
    let secret = normalized_secret(config.webhook_secret.as_deref());
    if type_changed || secret.is_some() {
        model.webhook_secret = Set(secret);
    }
    if config.cron_expression.is_some() {
        model.cron_expression = Set(config.cron_expression.clone());
    }
    if config.cron_timezone.is_some() {
        model.cron_timezone = Set(config.cron_timezone.clone());
    }
}

fn sync_identity(model: &mut event_sink::ActiveModel, config: &SinkConfig, rebinds: bool) {
    if let Some(actor) = &config.run_as_user_id {
        model.run_as_user_id = Set(Some(actor.clone()));
        model.pat_encrypted = Set(None);
    }
    if let Some(tokens) = credential_update(config.oauth_tokens_encrypted.clone(), rebinds) {
        model.oauth_tokens_encrypted = Set(tokens);
    }
    if let Some(profile) = credential_update(config.profile_json.clone(), rebinds) {
        model.profile_json = Set(profile);
    }
}

/// The value to write to a credential column of a sink, or `None` to keep it: what the
/// actor supplied, else nothing once the sink runs as someone new.
fn credential_update<T>(supplied: Option<T>, rebinds: bool) -> Option<Option<T>> {
    match supplied {
        Some(value) => Some(Some(value)),
        None if rebinds => Some(None),
        None => None,
    }
}

fn cron_schedule_needs_rebuild(
    old_cron_expression: &Option<String>,
    new_cron_expression: &Option<String>,
    old_timezone: &Option<String>,
    new_timezone: &Option<String>,
    old_sink_type: &str,
    new_sink_type: &str,
    new_scheduled_for: Option<&flow_like_sinks::ScheduledLocal>,
) -> bool {
    if old_sink_type != new_sink_type {
        return true;
    }

    if old_cron_expression != new_cron_expression {
        return true;
    }

    if old_timezone != new_timezone {
        return true;
    }

    // `scheduled_for` is not mirrored into the EventSink table, so treat any
    // one-time sync as a rebuild to propagate date/time edits to the external
    // scheduler.
    new_scheduled_for.is_some()
}

/// Delete a sink and its external scheduler
pub async fn delete_sink(
    db: &DatabaseConnection,
    state: &AppState,
    event_id: &str,
) -> flow_like_types::Result<()> {
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(event_id))
        .one(db)
        .await?;

    if let Some(sink) = sink {
        // Delete from external scheduler if it's a cron sink (best-effort)
        if sink.sink_type == sink_types::CRON
            && let Err(e) = delete_external_schedule(state, event_id).await
        {
            tracing::warn!(
                event_id = %event_id,
                error = %e,
                "Failed to delete external schedule during sink cleanup, proceeding with DB deletion"
            );
        }

        // Delete from database
        event_sink::Entity::delete_by_id(&sink.id).exec(db).await?;
    }

    Ok(())
}

/// Refuses a change to the hub trigger of an event that runs on its deployed devices only.
pub async fn ensure_event_may_use_hub_trigger(
    db: &DatabaseConnection,
    sink: &event_sink::Model,
) -> Result<(), ApiError> {
    match get_event_from_db_opt(db, &sink.event_id, &sink.app_id).await? {
        Some(event) => crate::routes::app::events::ensure_hub_trigger_allowed(&event),
        None => Ok(()),
    }
}

/// Toggle a sink's active state and update external scheduler.
///
/// Turning a sink on checks the event after the sink row was read. An upsert writes the
/// event row before its sink, so a conversion to device-only racing this call is seen here.
/// Turning it on makes `toggled_by` its run-as user, in the same transaction; turning it
/// off keeps who it runs as, until someone edits what it runs and becomes its run-as user.
pub async fn toggle_sink_active(
    db: &DatabaseConnection,
    state: &AppState,
    event_id: &str,
    toggled_by: &str,
) -> Result<event_sink::Model, ApiError> {
    let sink = sink_of_event(db, event_id).await?;
    let turning_on = !sink.active;
    if turning_on {
        ensure_may_turn_on(db, state, &sink, toggled_by).await?;
    }
    write_toggle(state, &sink, turning_on, toggled_by).await?;
    let updated = sink_of_event(db, event_id).await?;
    toggle_schedule(state, &updated, turning_on)
        .await
        .map_err(ApiError::internal_error)?;
    Ok(updated)
}

async fn sink_of_event(
    db: &DatabaseConnection,
    event_id: &str,
) -> Result<event_sink::Model, ApiError> {
    event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(event_id))
        .one(db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get sink: {}", e)))?
        .ok_or_else(|| ApiError::not_found("Sink not found for this event"))
}

/// Turning a sink on arms it: the event must still use the hub trigger, and the actor
/// becomes the user it runs as.
async fn ensure_may_turn_on(
    db: &DatabaseConnection,
    state: &AppState,
    sink: &event_sink::Model,
    actor: &str,
) -> Result<(), ApiError> {
    ensure_event_may_use_hub_trigger(db, sink).await?;
    ensure_actor_may_arm(state, &sink.app_id, actor).await
}

/// The sink is on before the actor is bound, in the same transaction. The bind keeps OAuth
/// grants only for the person the sink already runs as, so after an edit made while it was
/// off the actor saves theirs again with the event. Either way the sink is no longer paused
/// by a release: turned off by a person, it stops every run.
async fn write_toggle(
    state: &AppState,
    sink: &event_sink::Model,
    turning_on: bool,
    actor: &str,
) -> Result<(), ApiError> {
    state
        .transaction(|txn| {
            let sink_id = sink.id.clone();
            let app_id = sink.app_id.clone();
            let event_ids = vec![sink.event_id.clone()];
            let actor = actor.to_owned();
            Box::pin(async move {
                event_sink::Entity::update_many()
                    .set(event_sink::ActiveModel {
                        active: Set(turning_on),
                        run_as_released_at: Set(None),
                        updated_at: Set(chrono::Utc::now().fixed_offset()),
                        ..Default::default()
                    })
                    .filter(event_sink::Column::Id.eq(&sink_id))
                    .exec(txn)
                    .await?;
                if turning_on {
                    bind_run_as(txn, &app_id, &event_ids, &actor).await?;
                }
                Ok::<_, ApiError>(())
            })
        })
        .await
}

async fn toggle_schedule(
    state: &AppState,
    sink: &event_sink::Model,
    turning_on: bool,
) -> flow_like_types::Result<()> {
    if sink.sink_type != sink_types::CRON {
        return Ok(());
    }
    if turning_on {
        enable_or_recreate_schedule(state, sink).await
    } else {
        disable_external_schedule(state, &sink.event_id).await
    }
}

/// Enables the schedule of a cron sink that is turned back on. A schedule the AWS scheduler
/// reaped while the sink was off is created again from the stored cron.
async fn enable_or_recreate_schedule(
    state: &AppState,
    sink: &event_sink::Model,
) -> flow_like_types::Result<()> {
    let Some(scheduler) = state.sink_scheduler.as_ref() else {
        return Ok(());
    };
    match scheduler.enable_schedule(&sink.event_id).await {
        Ok(()) => {
            tracing::info!(event_id = %sink.event_id, "Enabled external schedule");
            Ok(())
        }
        Err(flow_like_sinks::SchedulerError::NotFound(_)) => {
            let scheduled_for = get_event_from_db_opt(&state.db, &sink.event_id, &sink.app_id)
                .await?
                .and_then(|event| extract_scheduled_for(&event.config));
            if sink.cron_expression.is_none() && scheduled_for.is_none() {
                tracing::warn!(event_id = %sink.event_id, "Cron sink has no schedule to recreate");
                return Ok(());
            }
            create_external_schedule(
                state,
                &sink.event_id,
                sink.cron_expression.as_deref().unwrap_or(""),
                sink.cron_timezone.as_deref(),
                scheduled_for.as_ref(),
                true,
            )
            .await
        }
        Err(error) => {
            tracing::error!(event_id = %sink.event_id, %error, "Failed to enable external schedule");
            Err(anyhow!("Failed to enable schedule: {}", error))
        }
    }
}

/// Create a schedule in the external scheduler (AWS EventBridge, K8s CronJob)
async fn create_external_schedule(
    state: &AppState,
    event_id: &str,
    cron_expression: &str,
    timezone: Option<&str>,
    scheduled_for: Option<&flow_like_sinks::ScheduledLocal>,
    active: bool,
) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        let config = flow_like_sinks::CronSinkConfig {
            expression: cron_expression.to_string(),
            timezone: timezone.unwrap_or("UTC").to_string(),
            scheduled_for: scheduled_for.cloned(),
            active,
        };

        scheduler.create_schedule(event_id, cron_expression, &config).await.map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "Failed to create external schedule");
            anyhow!("Failed to create schedule: {}", e)
        })?;

        tracing::info!(event_id = %event_id, cron = %cron_expression, tz = %config.timezone, one_time = config.is_one_time(), active, "Created external schedule");
    }

    Ok(())
}

/// Update a schedule in the external scheduler
async fn update_external_scheduler(
    state: &AppState,
    event_id: &str,
    cron_expression: &str,
    timezone: Option<&str>,
    scheduled_for: Option<&flow_like_sinks::ScheduledLocal>,
    active: bool,
) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        let config = flow_like_sinks::CronSinkConfig {
            expression: cron_expression.to_string(),
            timezone: timezone.unwrap_or("UTC").to_string(),
            scheduled_for: scheduled_for.cloned(),
            active,
        };

        // Try an update first; on NotFound fall through to create. This
        // removes the check-then-act race between concurrent syncs where two
        // writers both see `exists=false` and the second `create` hits
        // ConflictException.
        match scheduler
            .update_schedule(event_id, cron_expression, &config)
            .await
        {
            Ok(_) => {
                tracing::info!(
                    event_id = %event_id,
                    cron = %cron_expression,
                    active,
                    "Updated external schedule"
                );
            }
            Err(flow_like_sinks::SchedulerError::NotFound(_)) => {
                scheduler
                    .create_schedule(event_id, cron_expression, &config)
                    .await
                    .map_err(|e| {
                        tracing::error!(
                            event_id = %event_id,
                            error = %e,
                            "Failed to create external schedule after missing update"
                        );
                        anyhow!("Failed to create schedule: {}", e)
                    })?;
                tracing::info!(
                    event_id = %event_id,
                    cron = %cron_expression,
                    active,
                    "Re-created missing external schedule"
                );
            }
            Err(e) => {
                tracing::error!(event_id = %event_id, error = %e, "Failed to update external schedule");
                return Err(anyhow!("Failed to update schedule: {}", e));
            }
        }
    }

    Ok(())
}

/// Delete a schedule from the external scheduler
async fn delete_external_schedule(state: &AppState, event_id: &str) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        scheduler.delete_schedule(event_id).await.map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "Failed to delete external schedule");
            anyhow!("Failed to delete schedule: {}", e)
        })?;

        tracing::info!(event_id = %event_id, "Deleted external schedule");
    }

    Ok(())
}

/// Enable a schedule in the external scheduler
async fn enable_external_schedule(state: &AppState, event_id: &str) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        scheduler.enable_schedule(event_id).await.map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "Failed to enable external schedule");
            anyhow!("Failed to enable schedule: {}", e)
        })?;

        tracing::info!(event_id = %event_id, "Enabled external schedule");
    }

    Ok(())
}

/// Disable a schedule in the external scheduler
async fn disable_external_schedule(
    state: &AppState,
    event_id: &str,
) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        scheduler.disable_schedule(event_id).await.map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "Failed to disable external schedule");
            anyhow!("Failed to disable schedule: {}", e)
        })?;

        tracing::info!(event_id = %event_id, "Disabled external schedule");
    }

    Ok(())
}

/// Derive sink type from event type
pub fn sink_type_from_event_type(event_type: &str) -> &str {
    match event_type {
        "cron" => sink_types::CRON,
        "discord" => sink_types::DISCORD,
        "telegram" => sink_types::TELEGRAM,
        "teams" => "teams",
        "email" => sink_types::EMAIL,
        "inbound_email" => "inbound_email",
        "chat" => sink_types::CHAT,
        "api" | "http" | "webhook" => sink_types::HTTP,
        // Default to HTTP for unknown types
        _ => sink_types::HTTP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuilds_on_timezone_change() {
        assert!(cron_schedule_needs_rebuild(
            &Some("0 9 * * *".to_string()),
            &Some("0 9 * * *".to_string()),
            &Some("UTC".to_string()),
            &Some("Europe/Berlin".to_string()),
            "cron",
            "cron",
            None,
        ));
    }

    #[test]
    fn rebuilds_for_one_time_schedules() {
        assert!(cron_schedule_needs_rebuild(
            &None,
            &None,
            &Some("UTC".to_string()),
            &Some("UTC".to_string()),
            "cron",
            "cron",
            Some(&flow_like_sinks::ScheduledLocal {
                date: "2026-04-25".to_string(),
                time: "09:00".to_string(),
            }),
        ));
    }

    #[test]
    fn skips_rebuild_when_schedule_is_unchanged() {
        assert!(!cron_schedule_needs_rebuild(
            &Some("0 9 * * *".to_string()),
            &Some("0 9 * * *".to_string()),
            &Some("UTC".to_string()),
            &Some("UTC".to_string()),
            "cron",
            "cron",
            None,
        ));
    }

    #[test]
    fn credentials_follow_the_person_the_sink_runs_as() {
        assert_eq!(credential_update(Some("mine"), false), Some(Some("mine")));
        assert_eq!(credential_update(Some("mine"), true), Some(Some("mine")));
        assert_eq!(credential_update::<&str>(None, true), Some(None));
        assert_eq!(credential_update::<&str>(None, false), None);
    }

    /// Only a release that pauses a sink that was on marks it released, which stops the runs
    /// as its user alone. A sink the release found switched off keeps its person's switch-off,
    /// which stops every run.
    #[test]
    fn only_the_pause_of_a_sink_that_was_on_marks_it_released() {
        use sea_orm::{DbBackend, QueryTrait};
        let now = chrono::Utc::now().fixed_offset();
        let sql = |release| {
            pause_sink("sink", now, release)
                .build(DbBackend::Postgres)
                .to_string()
        };
        let paused = sql(RowRelease::Paused);
        assert!(paused.contains(r#""runAsReleasedAt" = '"#), "{paused}");
        assert!(paused.contains(r#""active" = FALSE"#), "{paused}");
        let cleared = sql(RowRelease::Cleared);
        assert!(!cleared.contains("runAsReleasedAt"), "{cleared}");
        assert!(cleared.contains(r#""runAsUserId" = NULL"#), "{cleared}");
    }

    /// Turning a sink on or off, by a save or the toggle, ends a release's pause: off, it was
    /// switched off by a person and stops every run. A save that leaves it as it was keeps the
    /// mark.
    #[test]
    fn turning_a_sink_on_or_off_ends_a_release() {
        let now = chrono::Utc::now().fixed_offset();
        let released = || event_sink::Model {
            active: false,
            run_as_user_id: None,
            pat_encrypted: None,
            run_as_released_at: Some(now),
            ..stored_sink(None)
        };
        for (case, active, cleared) in [
            ("turned on", Some(true), true),
            ("kept off", Some(false), false),
            ("left alone", None, false),
        ] {
            let config = SinkConfig {
                active,
                ..telegram_sync(Some("actor"))
            };
            let synced = synced_sink(released(), &config, now);
            assert_eq!(synced.run_as_released_at.is_set(), cleared, "{case}");
            if cleared {
                assert_eq!(synced.run_as_released_at, Set(None), "{case}");
            }
        }
        let switched_off = synced_sink(
            stored_sink(Some("actor")),
            &SinkConfig {
                active: Some(false),
                ..telegram_sync(Some("actor"))
            },
            now,
        );
        assert_eq!(switched_off.active, Set(false));
        assert_eq!(switched_off.run_as_released_at, Set(None));

        let source = include_str!("service.rs");
        let toggle = source
            .split_once("\nasync fn write_toggle(")
            .and_then(|(_, rest)| rest.split_once("\n}\n"))
            .map(|(body, _)| body)
            .unwrap();
        let active = toggle.find("active: Set(turning_on),").unwrap();
        let mark_cleared = toggle.find("run_as_released_at: Set(None),").unwrap();
        assert!(active < mark_cleared);
    }

    /// An edit made while a sink was off left it bound to the editor without grants. Whoever
    /// turns it back on with a save runs it from then on with the grants and profile they
    /// supply, the way any save that arms a sink collects them.
    #[test]
    fn turning_on_a_sink_an_edit_rebound_collects_the_grants_of_who_turns_it_on() {
        let now = chrono::Utc::now().fixed_offset();
        let rebound = event_sink::Model {
            active: false,
            pat_encrypted: None,
            oauth_tokens_encrypted: None,
            profile_json: None,
            ..stored_sink(Some("editor"))
        };
        let profile = serde_json::json!({"id": "owner-profile"});
        let turned_on = synced_sink(
            rebound.clone(),
            &SinkConfig {
                active: Some(true),
                oauth_tokens_encrypted: Some("owner-grants".into()),
                profile_json: Some(profile.clone()),
                ..telegram_sync(Some("owner"))
            },
            now,
        );
        assert_eq!(turned_on.active, Set(true));
        assert_eq!(turned_on.run_as_user_id, Set(Some("owner".into())));
        assert_eq!(
            turned_on.oauth_tokens_encrypted,
            Set(Some("owner-grants".into()))
        );
        assert_eq!(turned_on.profile_json, Set(Some(profile)));

        let without_grants = synced_sink(
            rebound,
            &SinkConfig {
                active: Some(true),
                ..telegram_sync(Some("owner"))
            },
            now,
        );
        assert_eq!(without_grants.oauth_tokens_encrypted, Set(None));
    }

    fn stored_sink(run_as: Option<&str>) -> event_sink::Model {
        let now = chrono::Utc::now().fixed_offset();
        event_sink::Model {
            id: "sink".into(),
            event_id: "event".into(),
            app_id: "app".into(),
            sink_type: sink_types::TELEGRAM.into(),
            active: true,
            path: None,
            auth_token: None,
            webhook_secret: Some("stored-secret".into()),
            cron_expression: None,
            cron_timezone: None,
            pat_encrypted: Some("pat".into()),
            oauth_tokens_encrypted: Some("oauth".into()),
            profile_json: Some(serde_json::json!({"id": "profile"})),
            created_at: now,
            updated_at: now,
            method: None,
            run_as_user_id: run_as.map(str::to_owned),
            run_as_released_at: None,
        }
    }

    fn telegram_sync(actor: Option<&str>) -> SinkConfig {
        SinkConfig {
            event_id: "event".into(),
            app_id: "app".into(),
            sink_type: sink_types::TELEGRAM.into(),
            run_as_user_id: actor.map(str::to_owned),
            ..Default::default()
        }
    }

    /// Saving binds the saver and drops the legacy PAT. The OAuth grants and profile stay
    /// only with the person who stored them; whatever the saver supplies is kept.
    #[test]
    fn a_save_by_someone_else_drops_the_previous_credentials() {
        let now = chrono::Utc::now().fixed_offset();
        let synced = synced_sink(
            stored_sink(Some("previous")),
            &telegram_sync(Some("actor")),
            now,
        );
        assert_eq!(synced.run_as_user_id, Set(Some("actor".into())));
        assert_eq!(synced.pat_encrypted, Set(None));
        assert_eq!(synced.oauth_tokens_encrypted, Set(None));
        assert_eq!(synced.profile_json, Set(None));

        let synced = synced_sink(stored_sink(None), &telegram_sync(Some("actor")), now);
        assert_eq!(synced.oauth_tokens_encrypted, Set(None), "a legacy row");
        assert_eq!(synced.profile_json, Set(None), "a legacy row");

        let synced = synced_sink(
            stored_sink(Some("actor")),
            &telegram_sync(Some("actor")),
            now,
        );
        assert_eq!(synced.pat_encrypted, Set(None));
        assert!(!synced.oauth_tokens_encrypted.is_set());
        assert!(!synced.profile_json.is_set());

        let supplied = SinkConfig {
            oauth_tokens_encrypted: Some("theirs".into()),
            profile_json: Some(serde_json::json!({"id": "theirs"})),
            ..telegram_sync(Some("actor"))
        };
        let synced = synced_sink(stored_sink(Some("previous")), &supplied, now);
        assert_eq!(synced.oauth_tokens_encrypted, Set(Some("theirs".into())));
        assert_eq!(
            synced.profile_json,
            Set(Some(serde_json::json!({"id": "theirs"})))
        );

        let system = synced_sink(stored_sink(Some("previous")), &telegram_sync(None), now);
        for (column, set) in [
            ("run-as", system.run_as_user_id.is_set()),
            ("pat", system.pat_encrypted.is_set()),
            ("oauth", system.oauth_tokens_encrypted.is_set()),
            ("profile", system.profile_json.is_set()),
        ] {
            assert!(!set, "a system sync leaves the {column} alone");
        }
    }

    #[test]
    fn a_bot_secret_is_written_only_when_one_is_provided() {
        let now = chrono::Utc::now().fixed_offset();
        let kept = synced_sink(
            stored_sink(Some("actor")),
            &telegram_sync(Some("actor")),
            now,
        );
        assert!(!kept.webhook_secret.is_set());
        let blank = SinkConfig {
            webhook_secret: Some("  ".into()),
            ..telegram_sync(Some("actor"))
        };
        assert!(
            !synced_sink(stored_sink(Some("actor")), &blank, now)
                .webhook_secret
                .is_set()
        );
        let rotated = SinkConfig {
            webhook_secret: Some(" rotated ".into()),
            ..telegram_sync(Some("actor"))
        };
        assert_eq!(
            synced_sink(stored_sink(Some("actor")), &rotated, now).webhook_secret,
            Set(Some("rotated".into()))
        );
    }

    /// A Discord public key left on a sink that became a Telegram trigger would let
    /// anyone who knows it through the Telegram door.
    #[test]
    fn a_new_sink_type_keeps_no_bot_secret_of_the_old_one() {
        let now = chrono::Utc::now().fixed_offset();
        let discord = || event_sink::Model {
            sink_type: sink_types::DISCORD.into(),
            webhook_secret: Some("discord-public-key".into()),
            ..stored_sink(Some("actor"))
        };
        assert_eq!(
            synced_sink(discord(), &telegram_sync(Some("actor")), now).webhook_secret,
            Set(None)
        );
        let generated = SinkConfig {
            webhook_secret: Some("telegram-secret".into()),
            ..telegram_sync(Some("actor"))
        };
        assert_eq!(
            synced_sink(discord(), &generated, now).webhook_secret,
            Set(Some("telegram-secret".into()))
        );
    }

    /// An event that stops being an HTTP event gives up its method, and with it the HTTP
    /// door and its (app, path, method) slot.
    #[test]
    fn the_sink_method_mirrors_the_event() {
        let now = chrono::Utc::now().fixed_offset();
        let http = || event_sink::Model {
            sink_type: sink_types::HTTP.into(),
            path: Some("/hook".into()),
            method: Some("POST".into()),
            ..stored_sink(Some("actor"))
        };
        let changed = synced_sink(http(), &telegram_sync(Some("actor")), now);
        assert_eq!(changed.method, Set(None));
        assert!(!changed.path.is_set());
        let resaved = SinkConfig {
            sink_type: sink_types::HTTP.into(),
            path: Some("/hook".into()),
            method: Some(" put ".into()),
            ..telegram_sync(Some("actor"))
        };
        assert_eq!(
            synced_sink(http(), &resaved, now).method,
            Set(Some("PUT".into()))
        );
        let device_only = SinkConfig {
            sink_type: sink_types::HTTP.into(),
            release_route: true,
            ..telegram_sync(Some("actor"))
        };
        let released = synced_sink(http(), &device_only, now);
        assert_eq!(released.method, Set(None));
        assert_eq!(released.path, Set(None));
    }

    #[test]
    fn an_arming_refusal_tells_the_actor_why() {
        assert_eq!(
            arm_refusal(RunAsDenial::CannotTrigger),
            "Your role does not allow triggering events, so you cannot turn on this event's hub trigger"
        );
        assert!(arm_refusal(RunAsDenial::NotMember).contains("not a member"));
        assert!(arm_refusal(RunAsDenial::AccountInactive).contains("account is not active"));
    }

    #[test]
    fn only_a_role_change_that_stops_allowing_triggers_releases_sinks() {
        use RolePermissions as P;
        let viewer = P::ReadBoards | P::ReadEvents;
        let trigger = P::ReadEvents | P::ExecuteEvents;
        for (before, after, loses) in [
            (trigger, viewer, true),
            (P::Admin, viewer, true),
            (P::Owner, P::empty(), true),
            (trigger, P::Admin, false),
            (trigger, trigger | P::WriteEvents, false),
            (P::Owner, trigger, false),
            (viewer, P::empty(), false),
            (P::empty(), trigger, false),
        ] {
            assert_eq!(
                loses_execute_events(before, after),
                loses,
                "{before:?} -> {after:?}"
            );
        }
    }

    /// Only a sink the release turned off is audited and announced as paused, not one
    /// that was off already or that another release or a new run-as user got first.
    #[test]
    fn a_release_announces_only_the_sinks_it_paused() {
        assert_eq!(
            announced(vec![
                ("on", RowRelease::Paused),
                ("off", RowRelease::Cleared),
                ("taken", RowRelease::Missed),
                ("also-on", RowRelease::Paused),
            ]),
            ["on", "also-on"]
        );
        assert!(announced(vec![("off", RowRelease::Cleared)]).is_empty());
    }

    #[test]
    fn each_member_gets_one_notice_for_all_their_sinks() {
        let sink = |id: &str| event_sink::Model {
            id: id.into(),
            ..stored_sink(None)
        };
        let grouped = by_member(vec![
            ("bob".into(), sink("bob-1")),
            ("alice".into(), sink("alice-1")),
            ("bob".into(), sink("bob-2")),
        ]);
        let grouped: Vec<(&str, Vec<&str>)> = grouped
            .iter()
            .map(|(member, sinks)| {
                let ids: Vec<&str> = sinks.iter().map(|sink| sink.id.as_str()).collect();
                (member.as_str(), ids)
            })
            .collect();
        assert_eq!(
            grouped,
            [("alice", vec!["alice-1"]), ("bob", vec!["bob-1", "bob-2"])]
        );
    }

    mod postgres {
        use super::*;
        use sea_orm::{
            Database, DatabaseTransaction, DbBackend, QueryOrder, Statement, TransactionTrait,
        };

        const KEY: [u8; 32] = [7; 32];
        const SCHEMA: &str = r#"
            CREATE TABLE public."PAT" (id text PRIMARY KEY, "userId" text NOT NULL);
            CREATE TABLE public."EventSink" (
                id text PRIMARY KEY, "eventId" text NOT NULL UNIQUE, "appId" text NOT NULL,
                "sinkType" text NOT NULL DEFAULT 'http', active boolean NOT NULL,
                path text, "authToken" text, "webhookSecret" text, "cronExpression" text,
                "cronTimezone" text, "patEncrypted" text, "oauthTokensEncrypted" text,
                "profileJson" jsonb, "createdAt" timestamptz(3) NOT NULL DEFAULT now(),
                "updatedAt" timestamptz(3) NOT NULL DEFAULT now(), method text, "runAsUserId" text,
                "runAsReleasedAt" timestamptz(3)
            );
            INSERT INTO public."PAT" VALUES ('leaver-pat', 'leaver'), ('other-pat', 'other');
        "#;

        async fn seeded() -> (sea_orm::DatabaseConnection, DatabaseTransaction) {
            let url = std::env::var("FLOW_LIKE_SINK_TEST_DATABASE_URL").unwrap();
            let db = Database::connect(url).await.unwrap();
            let txn = db.begin().await.unwrap();
            txn.execute_unprepared(SCHEMA).await.unwrap();
            (db, txn)
        }

        async fn insert(
            txn: &DatabaseTransaction,
            id: &str,
            app: &str,
            active: bool,
            run_as: Option<&str>,
            pat: Option<String>,
        ) {
            txn.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                r#"INSERT INTO public."EventSink"
                       (id, "eventId", "appId", active, "runAsUserId", "patEncrypted", "oauthTokensEncrypted", "profileJson")
                   VALUES ($1, $1, $2, $3, $4, $5, 'oauth', '{"id":"profile"}')"#,
                [
                    id.into(),
                    app.into(),
                    active.into(),
                    run_as.map(str::to_owned).into(),
                    pat.into(),
                ],
            ))
            .await
            .unwrap();
        }

        async fn sink(txn: &DatabaseTransaction, id: &str) -> event_sink::Model {
            event_sink::Entity::find_by_id(id)
                .one(txn)
                .await
                .unwrap()
                .unwrap()
        }

        fn released(sink: &event_sink::Model) -> bool {
            !sink.active
                && sink.run_as_user_id.is_none()
                && sink.pat_encrypted.is_none()
                && sink.oauth_tokens_encrypted.is_none()
                && sink.profile_json.is_none()
        }

        fn pairs(released: &[(String, event_sink::Model)]) -> Vec<(&str, &str)> {
            let mut pairs: Vec<(&str, &str)> = released
                .iter()
                .map(|(member, sink)| (member.as_str(), sink.id.as_str()))
                .collect();
            pairs.sort_unstable();
            pairs
        }

        fn leaver() -> Vec<String> {
            vec!["leaver".to_owned()]
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn a_release_pauses_only_that_users_sinks_in_that_app_once() {
            let (_db, txn) = seeded().await;
            insert(&txn, "on", "app", true, Some("leaver"), Some("pat".into())).await;
            insert(&txn, "off", "app", false, Some("leaver"), None).await;
            insert(&txn, "elsewhere", "other-app", true, Some("leaver"), None).await;
            insert(&txn, "teammate", "app", true, Some("other"), None).await;

            let paused = pause_member_sinks(&txn, Some("app"), &leaver())
                .await
                .unwrap();
            assert_eq!(
                pairs(&paused),
                [("leaver", "on")],
                "not the sink that was off"
            );
            assert!(paused[0].1.active, "the row as read");
            for id in ["on", "off"] {
                assert!(released(&sink(&txn, id).await), "{id}");
            }
            for id in ["elsewhere", "teammate"] {
                assert!(!released(&sink(&txn, id).await), "{id}");
            }
            assert!(
                pause_member_sinks(&txn, Some("app"), &leaver())
                    .await
                    .unwrap()
                    .is_empty()
            );

            let everywhere = pause_member_sinks(&txn, None, &leaver()).await.unwrap();
            assert_eq!(pairs(&everywhere), [("leaver", "elsewhere")]);
            assert!(!released(&sink(&txn, "teammate").await));
            txn.rollback().await.unwrap();
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn a_release_of_several_members_pauses_the_sinks_of_each() {
            let (_db, txn) = seeded().await;
            insert(&txn, "alice-on", "app", true, Some("alice"), None).await;
            insert(&txn, "bob-on", "app", true, Some("bob"), None).await;
            insert(&txn, "bob-off", "app", false, Some("bob"), None).await;
            insert(&txn, "carol-on", "app", true, Some("carol"), None).await;

            let members = ["alice".to_owned(), "bob".to_owned()];
            let paused = pause_member_sinks(&txn, Some("app"), &members)
                .await
                .unwrap();
            assert_eq!(pairs(&paused), [("alice", "alice-on"), ("bob", "bob-on")]);
            assert!(released(&sink(&txn, "bob-off").await));
            assert!(!released(&sink(&txn, "carol-on").await));
            txn.rollback().await.unwrap();
        }

        /// The update that applies decides whether a sink counts as paused, not the row
        /// as it was read.
        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn a_sink_toggled_after_it_was_read_is_judged_by_its_state_at_the_release() {
            let (_db, txn) = seeded().await;
            insert(&txn, "turned-on", "app", false, Some("leaver"), None).await;
            insert(&txn, "turned-off", "app", true, Some("leaver"), None).await;
            let read_off = sink(&txn, "turned-on").await;
            let read_on = sink(&txn, "turned-off").await;
            txn.execute_unprepared(r#"UPDATE public."EventSink" SET active = NOT active"#)
                .await
                .unwrap();

            let guard = || Condition::all().add(event_sink::Column::RunAsUserId.eq("leaver"));
            for (read, outcome) in [
                (&read_off, RowRelease::Paused),
                (&read_on, RowRelease::Cleared),
                (&read_on, RowRelease::Missed),
            ] {
                assert_eq!(
                    release_row(&txn, read, guard()).await.unwrap(),
                    outcome,
                    "{}",
                    read.id
                );
            }
            for id in ["turned-on", "turned-off"] {
                assert!(released(&sink(&txn, id).await), "{id}");
            }
            assert!(
                sink(&txn, "turned-on").await.run_as_released_at.is_some(),
                "the release paused it"
            );
            assert!(
                sink(&txn, "turned-off").await.run_as_released_at.is_none(),
                "a person had switched it off"
            );
            txn.rollback().await.unwrap();
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn a_departure_releases_the_legacy_sinks_whose_token_is_the_leavers() {
            let (_db, txn) = seeded().await;
            let token = |pat: &str| crate::routes::app::events::db::encrypt_token(pat, &KEY);
            let leavers = token("pat_leaver-pat.secret");
            insert(
                &txn,
                "leaver-legacy-off",
                "app",
                false,
                None,
                Some(leavers.clone()),
            )
            .await;
            for (id, app, run_as, pat) in [
                ("leaver-legacy", "app", None, leavers.clone()),
                ("other-legacy", "app", None, token("pat_other-pat.secret")),
                ("unreadable", "app", None, "garbage".to_owned()),
                ("malformed", "app", None, token("not-a-pat")),
                ("other-app", "other-app", None, leavers.clone()),
                ("bound", "app", Some("someone"), leavers),
            ] {
                insert(&txn, id, app, true, run_as, Some(pat)).await;
            }

            let paused = pause_legacy_sinks_of(&txn, &KEY, "app", &leaver())
                .await
                .unwrap();
            assert_eq!(pairs(&paused), [("leaver", "leaver-legacy")]);
            for id in ["leaver-legacy", "leaver-legacy-off"] {
                assert!(released(&sink(&txn, id).await), "{id}");
            }
            for id in [
                "other-legacy",
                "unreadable",
                "malformed",
                "other-app",
                "bound",
            ] {
                assert!(!released(&sink(&txn, id).await), "{id}");
            }
            assert!(
                pause_legacy_sinks_of(&txn, &KEY, "app", &leaver())
                    .await
                    .unwrap()
                    .is_empty()
            );

            let others = ["other".to_owned(), "nobody".to_owned()];
            let paused = pause_legacy_sinks_of(&txn, &KEY, "app", &others)
                .await
                .unwrap();
            assert_eq!(pairs(&paused), [("other", "other-legacy")]);
            txn.rollback().await.unwrap();
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn a_legacy_release_needs_the_token_the_refused_fire_read() {
            let (_db, txn) = seeded().await;
            insert(&txn, "legacy", "app", true, None, Some("seen".into())).await;
            let seen = sink(&txn, "legacy").await;
            txn.execute_unprepared(
                r#"UPDATE public."EventSink" SET "patEncrypted" = 'replaced' WHERE id = 'legacy'"#,
            )
            .await
            .unwrap();
            assert_eq!(
                pause_legacy_sink(&txn, &seen).await.unwrap(),
                RowRelease::Missed
            );
            assert!(!released(&sink(&txn, "legacy").await));

            let current = sink(&txn, "legacy").await;
            assert_eq!(
                pause_legacy_sink(&txn, &current).await.unwrap(),
                RowRelease::Paused
            );
            assert!(released(&sink(&txn, "legacy").await));
            assert_eq!(
                pause_legacy_sink(&txn, &current).await.unwrap(),
                RowRelease::Missed
            );
            txn.rollback().await.unwrap();
        }

        #[tokio::test]
        #[ignore = "requires FLOW_LIKE_SINK_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
        async fn a_sync_clears_oauth_and_profile_only_when_the_actor_differs() {
            let (_db, txn) = seeded().await;
            insert(
                &txn,
                "previous",
                "app",
                true,
                Some("previous"),
                Some("pat".into()),
            )
            .await;
            insert(&txn, "same", "app", true, Some("actor"), Some("pat".into())).await;
            let now = chrono::Utc::now().fixed_offset();
            for id in ["previous", "same"] {
                let config = SinkConfig {
                    event_id: id.into(),
                    ..telegram_sync(Some("actor"))
                };
                synced_sink(sink(&txn, id).await, &config, now)
                    .update(&txn)
                    .await
                    .unwrap();
            }
            let rows = event_sink::Entity::find()
                .order_by_asc(event_sink::Column::Id)
                .all(&txn)
                .await
                .unwrap()
                .into_iter()
                .map(|sink| {
                    (
                        sink.id,
                        sink.run_as_user_id,
                        sink.pat_encrypted.is_some(),
                        sink.oauth_tokens_encrypted.is_some(),
                        sink.profile_json.is_some(),
                    )
                })
                .collect::<Vec<_>>();
            let actor = Some("actor".to_owned());
            assert_eq!(
                rows,
                vec![
                    ("previous".to_owned(), actor.clone(), false, false, false),
                    ("same".to_owned(), actor, false, true, true),
                ]
            );
            txn.rollback().await.unwrap();
        }
    }
}
