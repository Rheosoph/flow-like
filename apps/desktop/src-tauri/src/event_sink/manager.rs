use anyhow::{Context, Result};
use flow_like::app::{App, AppVisibility};
use flow_like::flow::event::{Event, EventExecutionMode};
use flow_like::flow::oauth::OAuthToken;
use flow_like::state::FlowLikeState;
use flow_like_types::intercom::BufferedInterComHandler;
use futures::future::join_all;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde_json;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};

use crate::event_sink::cron::CronSchedule;

use super::cron::CronSink;
use super::geolocation::GeofenceClearance;
use super::trigger_token::{
    TokenRefused, TriggerAccount, TriggerProfile, verify_token_for_profile,
};
use super::{EventConfig, EventRegistration, EventSink, has_token, token_owned_by};

pub type DbConnection = Arc<Mutex<Connection>>;

const REGISTRATION_COLUMNS: &str = "event_id, name, type, updated_at, created_at, config, offline, app_id, default_payload, personal_access_token, oauth_tokens, pat_owner_sub, hub_origin, remote, profile_binding";

/// Trigger types whose sink exists only as a placeholder on this device.
const UNIMPLEMENTED_TRIGGERS: [&str; 10] = [
    "file",
    "webhook",
    "slack",
    "web_watcher",
    "github",
    "mqtt",
    "notion",
    "nfc",
    "shortcut",
    "mcp",
];

/// What saving an event does to this device's trigger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalSinkPlan {
    /// Nothing registers locally: unsupported or unimplemented type,
    /// inactive, runs on the hub, or a remote-only sink.
    None,
    /// A registration of the same trigger type and destination exists for
    /// this app; saving refreshes it.
    Existing,
    /// A trigger would start on this device, for the first time, with a new
    /// type or destination, or taken over from another app.
    New,
}

/// What the desktop asks before a save: whether a trigger registers here and
/// whether the user has to choose a token for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LocalTriggerPlan {
    pub registration: LocalSinkPlan,
    /// The trigger signs in to the hub and holds no PAT of the signed-in account.
    pub needs_token: bool,
    /// The app whose trigger for the same event id runs here now; registering
    /// moves it to this app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub moves_from_app: Option<String>,
}

impl LocalTriggerPlan {
    pub const NONE: Self = Self {
        registration: LocalSinkPlan::None,
        needs_token: false,
        moves_from_app: None,
    };
}

/// This device's trigger for an event, as the event settings show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalTriggerState {
    /// The event has no trigger on this device.
    None,
    /// The trigger belongs on this device, but none is registered.
    Unregistered,
    Ready,
    /// The trigger signs in to the hub without a PAT of the signed-in account.
    NeedsToken,
    /// The last refresh failed, or the hub refused the trigger.
    Failed {
        reason: String,
    },
}

/// An online trigger was about to be saved without a PAT of the signed-in
/// account. The existing registration, if any, stays as it was.
#[derive(Debug)]
pub struct MissingTriggerToken(pub String);
impl std::fmt::Display for MissingTriggerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Event {} has no personal access token of the signed-in account for its trigger on this computer",
            self.0
        )
    }
}
impl std::error::Error for MissingTriggerToken {}

/// How a save treats an event's trigger on this device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerUpdate {
    /// The user approved the trigger: create it, or refresh it.
    Register,
    /// Refresh this app's existing registration, and only when it changed.
    /// A trigger of another type or destination is declined, never converted.
    Keep,
    /// Sign this app's existing trigger of the same type and destination in
    /// again; never create or convert one.
    Repair,
}

const NOTHING_TO_REPAIR: &str =
    "This event has no trigger on this computer to repair; save the event to register one";

/// What a save does to an event's trigger, decided before anything is stored
/// or started.
#[derive(Debug)]
enum TriggerDecision {
    Unchanged,
    /// Remove the trigger and keep its credentials.
    Decline,
    /// Store and start this registration.
    Register(Box<EventRegistration>),
}

/// Why the hub refused a trigger, kept so the event's status can say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: String,
    /// The hub refused the PAT itself, so declining must not keep it.
    pub token_refused: bool,
}

/// Whether `app_id` is an offline app. An app that does not load counts as
/// online, so its trigger asks for a token rather than run without one.
pub(crate) async fn app_is_offline(app_handle: &AppHandle, app_id: &str) -> bool {
    let Ok(state) = crate::state::TauriFlowLikeState::construct(app_handle).await else {
        return false;
    };
    App::load(app_id.to_string(), state)
        .await
        .is_ok_and(|app| matches!(app.visibility, AppVisibility::Offline))
}

/// Binds OAuth tokens handed in with a save to the hub that issued them.
fn hub_bound_oauth_tokens(
    mut tokens: HashMap<String, OAuthToken>,
    hub: &flow_like::profile::Profile,
) -> Result<HashMap<String, OAuthToken>> {
    for token in tokens.values_mut() {
        if token.hub_origin.is_some()
            || token
                .refresh_token
                .as_deref()
                .is_some_and(|value| value.starts_with("fl_oauth_"))
        {
            token.ensure_hub(&hub.hub, hub.secure)?;
        } else {
            token.hub_origin = flow_like::hub::hub_origin(&hub.hub, hub.secure);
        }
    }
    Ok(tokens)
}

/// The OAuth tokens handed in with a save, bound to the current profile's hub.
async fn bound_oauth_tokens(
    app_handle: &AppHandle,
    tokens: Option<&HashMap<String, OAuthToken>>,
) -> Result<Option<HashMap<String, OAuthToken>>> {
    let Some(tokens) = tokens else {
        return Ok(None);
    };
    let profile = crate::state::TauriSettingsState::current_profile(app_handle).await?;
    hub_bound_oauth_tokens(tokens.clone(), &profile.hub_profile).map(Some)
}

/// Starts and stops the sink behind a registration. The manager decides what
/// is stored; the lifecycle runs the side effects.
#[async_trait::async_trait]
pub(crate) trait TriggerLifecycle: Send + Sync {
    async fn start(&self, registration: &EventRegistration) -> Result<()>;
    async fn stop(&self, registration: &EventRegistration) -> Result<()>;
}

struct SinkLifecycle<'a> {
    manager: &'a EventSinkManager,
    app_handle: &'a AppHandle,
    /// The hub's clearance of the location trigger a save registers.
    geofence: Option<&'a GeofenceClearance>,
}

#[async_trait::async_trait]
impl TriggerLifecycle for SinkLifecycle<'_> {
    async fn start(&self, registration: &EventRegistration) -> Result<()> {
        self.manager
            .start_trigger(self.app_handle, registration, self.geofence)
            .await
    }

    async fn stop(&self, registration: &EventRegistration) -> Result<()> {
        self.manager
            .stop_trigger(self.app_handle, registration)
            .await
    }
}

/// What a trigger signs in with: its PAT, whom the PAT was verified to belong
/// to, and third-party OAuth tokens.
#[derive(Debug, Clone, Default)]
struct TriggerCredentials {
    personal_access_token: Option<String>,
    pat_owner_sub: Option<String>,
    hub_origin: Option<String>,
    profile_binding: Option<TriggerProfile>,
    oauth_tokens: HashMap<String, OAuthToken>,
}

impl TriggerCredentials {
    fn of(registration: &EventRegistration) -> Self {
        Self {
            personal_access_token: registration.personal_access_token.clone(),
            pat_owner_sub: registration.pat_owner_sub.clone(),
            hub_origin: registration.hub_origin.clone(),
            profile_binding: registration.profile_binding.clone(),
            oauth_tokens: registration.oauth_tokens.clone(),
        }
    }

    fn owned_by(&self, account: Option<&TriggerAccount>) -> bool {
        token_owned_by(
            self.personal_access_token.as_deref(),
            self.pat_owner_sub.as_deref(),
            self.hub_origin.as_deref(),
            self.profile_binding.as_ref(),
            account,
        )
    }

    /// `account`, when `pat` is the PAT held here and was verified for it.
    fn verified_for(&self, pat: &str, account: Option<&TriggerAccount>) -> Option<TriggerAccount> {
        account
            .filter(|_| {
                self.personal_access_token.as_deref() == Some(pat) && self.owned_by(account)
            })
            .cloned()
    }

    /// Whom the held PAT was verified for, if anyone.
    fn owner(&self) -> Option<TriggerAccount> {
        let owner = self.profile_binding.as_ref()?.account()?;
        (self.pat_owner_sub.as_deref() == Some(owner.sub.as_str())
            && self.hub_origin.as_deref() == Some(owner.hub_origin.as_str()))
        .then_some(owner)
    }

    /// The OAuth grants that stay with the trigger once it signs in as
    /// `owner`. A previous owner's grants stay behind, and so does any grant
    /// bound to another hub account.
    fn oauth_tokens_for(&self, owner: &TriggerAccount) -> HashMap<String, OAuthToken> {
        if self.owner().is_some_and(|previous| previous != *owner) {
            return HashMap::new();
        }
        self.oauth_tokens
            .iter()
            .filter(|(_, token)| {
                token
                    .hub_subject
                    .as_deref()
                    .is_none_or(|subject| subject == owner.sub)
            })
            .map(|(provider, token)| (provider.clone(), token.clone()))
            .collect()
    }
}

/// A PAT the caller handed in, with the account it was verified for. Offline
/// apps do not verify the PATs they are given.
#[derive(Clone)]
struct GivenToken {
    pat: String,
    owner: Option<TriggerAccount>,
}

/// Who a save's trigger signs in as: whether its app is offline, the
/// signed-in account, a PAT handed in with the save, and the hub's
/// clearance when the save registers a location trigger.
#[derive(Clone)]
pub struct TriggerSignIn {
    offline: bool,
    profile_binding: TriggerProfile,
    account: Option<TriggerAccount>,
    given: Option<GivenToken>,
    geofence: Option<GeofenceClearance>,
}

impl TriggerSignIn {
    /// Resolves the sign-in for saving `event` in `app_id` with `update` and
    /// `oauth_tokens` before the manager is locked for the save: verifying a
    /// new PAT and clearing a location trigger are hub round trips, and the
    /// fire paths only try the lock. A PAT the save will not use is not
    /// checked, nor one already verified for the signed-in account.
    pub async fn resolve(
        manager: &flow_like_types::sync::Mutex<EventSinkManager>,
        app_handle: &AppHandle,
        app_id: &str,
        event: &Event,
        pat: Option<String>,
        oauth_tokens: Option<&HashMap<String, OAuthToken>>,
        update: TriggerUpdate,
    ) -> Result<Self> {
        let offline = app_is_offline(app_handle, app_id).await;
        let profile = crate::state::TauriSettingsState::current_profile(app_handle).await?;
        let profile_binding = TriggerProfile::capture(&profile.hub_profile).await?;
        let account = profile_binding.account();
        let pat = pat.map(|pat| pat.trim().to_string()).filter(|pat| {
            !pat.is_empty() && EventSinkManager::local_registration_blocker(event).is_none()
        });
        let given = match pat {
            None => None,
            Some(pat) if offline => Some(GivenToken { pat, owner: None }),
            Some(pat) => {
                let held = manager
                    .lock()
                    .await
                    .credentials_for_update(app_id, event, update)?;
                match held {
                    None => None,
                    Some(held) => {
                        let owner = match held.verified_for(&pat, account.as_ref()) {
                            Some(owner) => owner,
                            None => verify_token_for_profile(
                                &profile.hub_profile,
                                account.as_ref().ok_or_else(|| anyhow::anyhow!(
                                    "Sign in to the saved profile before choosing a trigger token"
                                ))?,
                                app_id,
                                &pat,
                            ).await?,
                        };
                        Some(GivenToken {
                            pat,
                            owner: Some(owner),
                        })
                    }
                }
            }
        };
        let mut sign_in = Self {
            offline,
            profile_binding,
            account,
            given,
            geofence: None,
        };
        sign_in.geofence = sign_in
            .location_clearance(manager, app_handle, app_id, event, oauth_tokens, update)
            .await;
        Ok(sign_in)
    }

    /// The hub's clearance of the location trigger saving `event` registers,
    /// decided as the save decides it, under the lock only for that decision.
    /// `None` when the save registers no location trigger; a save that cannot
    /// decide fails again under the lock, with its own error.
    async fn location_clearance(
        &self,
        manager: &flow_like_types::sync::Mutex<EventSinkManager>,
        app_handle: &AppHandle,
        app_id: &str,
        event: &Event,
        oauth_tokens: Option<&HashMap<String, OAuthToken>>,
        update: TriggerUpdate,
    ) -> Option<GeofenceClearance> {
        let config @ EventConfig::GeoLocation(_) =
            EventSinkManager::local_sink_config(event).ok()?
        else {
            return None;
        };
        let oauth_tokens = bound_oauth_tokens(app_handle, oauth_tokens).await.ok()?;
        let decision =
            manager
                .lock()
                .await
                .decide(event, app_id, config, self.clone(), oauth_tokens, update);
        let Ok(TriggerDecision::Register(registration)) = decision else {
            return None;
        };
        Some(GeofenceClearance::resolve(app_handle, &registration).await)
    }
}

/// What the startup prune learns about one app's registrations.
#[derive(Default)]
struct AppRegistrations {
    stale: Vec<String>,
    /// Whether the app is offline; `None` when it did not load.
    offline: Option<bool>,
}

/// Internal storage for event registrations
/// Handles database persistence of event sink configurations
struct RegistrationStorage {
    conn: DbConnection,
}

impl RegistrationStorage {
    fn new(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(db_path).context("Failed to open database")?;
        let storage = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        storage.init_schema()?;
        Ok(storage)
    }

    fn ensure_column(conn: &Connection, table: &str, column: &str, definition: &str) -> Result<()> {
        let has_column: bool = conn
            .prepare("SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2")?
            .query_row(params![table, column], |row| row.get::<_, i32>(0))
            .map(|count| count > 0)
            .unwrap_or(false);

        if !has_column {
            conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {definition}"), [])?;
        }

        Ok(())
    }

    fn ensure_registration_column(conn: &Connection, column: &str, definition: &str) -> Result<()> {
        Self::ensure_column(conn, "event_registrations", column, definition)
    }

    fn connection(&self) -> DbConnection {
        Arc::clone(&self.conn)
    }

    /// Parse config JSON with backwards compatibility
    /// Injects sink_type if missing, based on the registration type
    fn parse_config_json(
        config_json: &str,
        reg_type: &str,
    ) -> Result<EventConfig, serde_json::Error> {
        // First try to parse directly (new format with sink_type)
        if let Ok(config) = serde_json::from_str::<EventConfig>(config_json) {
            return Ok(config);
        }

        // Backwards compatibility: inject sink_type based on registration type
        let mut config_value: serde_json::Value = serde_json::from_str(config_json)?;
        if let Some(obj) = config_value.as_object_mut() {
            // Map registration type to sink_type
            let sink_type = match reg_type {
                "api" | "http" => "http",
                "mail" | "email" => "email",
                _ => reg_type,
            };
            obj.insert(
                "sink_type".to_string(),
                serde_json::Value::String(sink_type.to_string()),
            );
        }

        serde_json::from_value(config_value)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        Self::init_registrations(&conn)?;
        Self::init_credential_stash(&conn)?;
        Self::init_trigger_failures(&conn)
    }

    fn init_registrations(conn: &Connection) -> Result<()> {
        // Use event_id as the primary key since each event can only be attached to one sink
        conn.execute(
            "CREATE TABLE IF NOT EXISTS event_registrations (
                event_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                config TEXT NOT NULL,
                offline INTEGER NOT NULL,
                app_id TEXT NOT NULL,
                default_payload TEXT,
                personal_access_token TEXT,
                oauth_tokens TEXT,
                pat_owner_sub TEXT,
                hub_origin TEXT,
                remote INTEGER NOT NULL DEFAULT 0,
                profile_binding TEXT
            )",
            [],
        )?;
        for (column, definition) in [
            ("default_payload", "default_payload TEXT"),
            ("personal_access_token", "personal_access_token TEXT"),
            ("oauth_tokens", "oauth_tokens TEXT"),
            ("pat_owner_sub", "pat_owner_sub TEXT"),
            ("hub_origin", "hub_origin TEXT"),
            ("remote", "remote INTEGER NOT NULL DEFAULT 0"),
            ("profile_binding", "profile_binding TEXT"),
        ] {
            Self::ensure_registration_column(conn, column, definition)?;
        }
        Ok(())
    }

    fn init_credential_stash(conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS event_credential_stash (
                event_id TEXT PRIMARY KEY,
                personal_access_token TEXT,
                oauth_tokens TEXT,
                app_id TEXT,
                pat_owner_sub TEXT,
                hub_origin TEXT,
                profile_binding TEXT
            )",
            [],
        )?;
        for column in ["app_id", "pat_owner_sub", "hub_origin", "profile_binding"] {
            Self::ensure_column(
                conn,
                "event_credential_stash",
                column,
                &format!("{column} TEXT"),
            )?;
        }
        // Stashed before stashes named their app: no registration can use
        // them, so their tokens must not stay on disk.
        conn.execute(
            "DELETE FROM event_credential_stash WHERE app_id IS NULL",
            [],
        )?;
        Ok(())
    }

    fn init_trigger_failures(conn: &Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS event_trigger_failures (
                event_id TEXT PRIMARY KEY,
                app_id TEXT NOT NULL,
                reason TEXT NOT NULL,
                recorded_at INTEGER NOT NULL
            )",
            [],
        )?;
        Ok(())
    }

    fn unreadable_column(index: usize, error: serde_json::Error) -> rusqlite::Error {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    }

    /// The JSON value stored in column `index`, if any.
    fn json_column<T: serde::de::DeserializeOwned>(
        row: &Row,
        index: usize,
    ) -> rusqlite::Result<Option<T>> {
        row.get::<_, Option<String>>(index)?
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(|error| Self::unreadable_column(index, error))
    }

    fn time_column(row: &Row, index: usize) -> rusqlite::Result<std::time::SystemTime> {
        Ok(std::time::UNIX_EPOCH + Duration::from_secs(row.get::<_, i64>(index)? as u64))
    }

    fn epoch_seconds(time: std::time::SystemTime) -> Result<i64> {
        Ok(time.duration_since(std::time::UNIX_EPOCH)?.as_secs() as i64)
    }

    /// OAuth tokens as stored: `NULL` when there are none.
    fn oauth_tokens_json(tokens: &HashMap<String, OAuthToken>) -> Result<Option<String>> {
        Ok(if tokens.is_empty() {
            None
        } else {
            Some(serde_json::to_string(tokens)?)
        })
    }

    fn registration_from_row(row: &Row) -> rusqlite::Result<EventRegistration> {
        let reg_type: String = row.get(2)?;
        let config_json: String = row.get(5)?;
        let config = Self::parse_config_json(&config_json, &reg_type)
            .map_err(|error| Self::unreadable_column(5, error))?;
        Ok(EventRegistration {
            event_id: row.get(0)?,
            name: row.get(1)?,
            r#type: reg_type,
            updated_at: Self::time_column(row, 3)?,
            created_at: Self::time_column(row, 4)?,
            config,
            offline: row.get::<_, i32>(6)? != 0,
            app_id: row.get(7)?,
            default_payload: Self::json_column(row, 8)?,
            personal_access_token: row.get(9)?,
            pat_owner_sub: row.get(11)?,
            hub_origin: row.get(12)?,
            remote: row.get::<_, i32>(13)? != 0,
            profile_binding: Self::json_column(row, 14)?,
            oauth_tokens: Self::json_column(row, 10)?.unwrap_or_default(),
        })
    }

    fn save_registration(&self, registration: &EventRegistration) -> Result<()> {
        let config_json = serde_json::to_string(&registration.config)?;
        let default_payload_json = registration
            .default_payload
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let oauth_tokens_json = Self::oauth_tokens_json(&registration.oauth_tokens)?;
        let updated_at = Self::epoch_seconds(registration.updated_at)?;
        let created_at = Self::epoch_seconds(registration.created_at)?;

        tracing::info!(
            "Saving registration for event {} with PAT present: {}",
            registration.event_id,
            has_token(registration.personal_access_token.as_deref())
        );

        // Acquire lock in limited scope to minimize lock duration
        {
            let conn = self.conn.lock().unwrap();

            conn.execute(
                &format!(
                    "INSERT OR REPLACE INTO event_registrations ({REGISTRATION_COLUMNS})
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)"
                ),
                params![
                    registration.event_id,
                    registration.name,
                    registration.r#type,
                    updated_at,
                    created_at,
                    config_json,
                    registration.offline as i32,
                    registration.app_id,
                    default_payload_json,
                    registration.personal_access_token,
                    oauth_tokens_json,
                    registration.pat_owner_sub,
                    registration.hub_origin,
                    registration.remote as i32,
                    registration
                        .profile_binding
                        .as_ref()
                        .map(serde_json::to_string)
                        .transpose()?,
                ],
            )?;
        }

        tracing::info!(
            "Successfully saved registration for event {}",
            registration.event_id
        );

        Ok(())
    }

    fn get_registration(&self, event_id: &str) -> Result<Option<EventRegistration>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {REGISTRATION_COLUMNS} FROM event_registrations WHERE event_id = ?1"
                ),
                params![event_id],
                Self::registration_from_row,
            )
            .optional()?)
    }

    fn list_registrations(&self) -> Result<Vec<EventRegistration>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {REGISTRATION_COLUMNS} FROM event_registrations ORDER BY created_at DESC"
        ))?;
        let registrations = stmt
            .query_map([], Self::registration_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(registrations)
    }

    /// Records the offline-ness of the registration's app, which decides
    /// whether the trigger signs in to the hub.
    fn set_offline(&self, event_id: &str, offline: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE event_registrations SET offline = ?2 WHERE event_id = ?1",
            params![event_id, offline as i32],
        )?;
        Ok(())
    }

    fn delete_registration(&self, event_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM event_registrations WHERE event_id = ?1",
            params![event_id],
        )?;
        Ok(())
    }

    /// Keeps a registration's credentials after its trigger was removed, so
    /// allowing it again does not need the token dialog.
    fn stash_credentials(&self, registration: &EventRegistration) -> Result<()> {
        let oauth_tokens_json = Self::oauth_tokens_json(&registration.oauth_tokens)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO event_credential_stash
             (event_id, personal_access_token, oauth_tokens, app_id, pat_owner_sub, hub_origin, profile_binding)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                registration.event_id,
                registration.personal_access_token,
                oauth_tokens_json,
                registration.app_id,
                registration.pat_owner_sub,
                registration.hub_origin,
                registration.profile_binding.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;
        Ok(())
    }

    /// The credentials stashed for `event_id` of `app_id`. Reading keeps them;
    /// only a registration that used them clears them.
    fn stashed_credentials(
        &self,
        app_id: &str,
        event_id: &str,
    ) -> Result<Option<TriggerCredentials>> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT personal_access_token, oauth_tokens, pat_owner_sub, hub_origin, profile_binding
                   FROM event_credential_stash
                  WHERE event_id = ?1 AND app_id = ?2",
                params![event_id, app_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        Self::json_column::<TriggerProfile>(row, 4)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            personal_access_token,
            oauth_tokens_json,
            pat_owner_sub,
            hub_origin,
            profile_binding,
        )) = row
        else {
            return Ok(None);
        };
        let oauth_tokens = oauth_tokens_json
            .map(|json| serde_json::from_str(&json))
            .transpose()?
            .unwrap_or_default();
        Ok(Some(TriggerCredentials {
            personal_access_token,
            pat_owner_sub,
            hub_origin,
            profile_binding,
            oauth_tokens,
        }))
    }

    fn clear_stashed_credentials(&self, app_id: &str, event_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM event_credential_stash WHERE event_id = ?1 AND app_id = ?2",
            params![event_id, app_id],
        )?;
        Ok(())
    }

    fn record_failure(&self, app_id: &str, event_id: &str, reason: &str) -> Result<()> {
        let recorded_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs() as i64;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO event_trigger_failures (event_id, app_id, reason, recorded_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![event_id, app_id, reason, recorded_at],
        )?;
        Ok(())
    }

    fn failure(&self, app_id: &str, event_id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT reason FROM event_trigger_failures WHERE event_id = ?1 AND app_id = ?2",
                params![event_id, app_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    fn clear_failure(&self, app_id: &str, event_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM event_trigger_failures WHERE event_id = ?1 AND app_id = ?2",
            params![event_id, app_id],
        )?;
        Ok(())
    }
}

/// Manager for all event sinks
/// Initializes database and coordinates sink lifecycle
pub struct EventSinkManager {
    db: DbConnection,
    storage: Arc<RegistrationStorage>,
    started_sinks: Arc<flow_like_types::tokio::sync::Mutex<HashSet<String>>>,
}

impl Clone for EventSinkManager {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            storage: Arc::clone(&self.storage),
            started_sinks: Arc::clone(&self.started_sinks),
        }
    }
}

impl EventSinkManager {
    /// Create a new event sink manager
    pub fn new(db_path: &str) -> Result<Self> {
        let storage = Arc::new(RegistrationStorage::new(PathBuf::from(db_path))?);
        let db = storage.connection();

        Ok(Self {
            db,
            storage,
            started_sinks: Arc::new(flow_like_types::tokio::sync::Mutex::new(HashSet::new())),
        })
    }

    pub(super) fn read_registration(
        db: DbConnection,
        event_id: &str,
    ) -> Result<Option<EventRegistration>> {
        RegistrationStorage { conn: db }.get_registration(event_id)
    }

    pub(super) fn read_registrations(db: DbConnection) -> Result<Vec<EventRegistration>> {
        RegistrationStorage { conn: db }.list_registrations()
    }

    /// The PAT an online registration of `app_id` runs with right now. Work
    /// that outlives a run reads it here instead of keeping its own copy, so a
    /// re-saved token replaces it and a removed registration takes it along.
    pub(crate) fn access_token(
        db: DbConnection,
        app_id: &str,
        event_id: &str,
    ) -> Result<Option<String>> {
        let conn = db
            .lock()
            .map_err(|_| anyhow::anyhow!("Event sink database lock poisoned"))?;
        let token = conn
            .query_row(
                "SELECT personal_access_token FROM event_registrations
                 WHERE event_id = ?1 AND app_id = ?2 AND offline = 0",
                params![event_id, app_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?;
        Ok(token.flatten())
    }

    /// Check if a sink type has been started, and mark it as started if not
    async fn ensure_sink_started(
        &self,
        sink_type: &str,
        app_handle: &AppHandle,
        sink: &dyn EventSink,
    ) -> Result<()> {
        let mut started_guard = self.started_sinks.lock().await;
        if started_guard.contains(sink_type) {
            println!(
                "[SINK_MANAGER] Sink {} already started or starting, skipping",
                sink_type
            );
            return Ok(());
        }

        println!("🚀 [SINK_MANAGER] Starting {} sink", sink_type);
        started_guard.insert(sink_type.to_string());
        drop(started_guard);

        if let Err(err) = sink.start(app_handle, self.db.clone()).await {
            let mut started_guard = self.started_sinks.lock().await;
            started_guard.remove(sink_type);
            println!(
                "❌ [SINK_MANAGER] Failed to start {} sink: {}",
                sink_type, err
            );
            return Err(err);
        }

        println!("✅ [SINK_MANAGER] {} sink ready", sink_type);
        Ok(())
    }

    /// Get database connection
    pub fn db(&self) -> DbConnection {
        self.db.clone()
    }

    /// Fire an event by retrieving its configuration and pushing it to the event bus.
    /// This is a centralized method that handles offline status, personal_access_token, oauth_tokens, etc.
    ///
    /// `Ok(false)` means the bus could not take the event right now and a later
    /// attempt may succeed. `Err` means the registration itself cannot fire (it
    /// is missing, or online without a verified access token) until it is
    /// saved again. Whether that token is the signed-in account's on the
    /// current hub is checked when the run starts.
    pub fn fire_event(
        &self,
        app_handle: &AppHandle,
        event_id: &str,
        payload: Option<flow_like_types::Value>,
        callback: Option<Arc<BufferedInterComHandler>>,
    ) -> Result<bool> {
        match self.fire_event_for_retry(app_handle, event_id, payload, callback)? {
            Ok(()) => Ok(true),
            Err(reason) => {
                // Nothing retries this event: the request that fired it is lost.
                tracing::error!("Failed to push event {}: {}", event_id, reason);
                Ok(false)
            }
        }
    }

    /// [`Self::fire_event`] for callers that retry. When the bus does not take
    /// the event, the reason comes back as `Ok(Err(reason))` without being
    /// logged, so the caller can log it once rather than on every attempt.
    pub fn fire_event_for_retry(
        &self,
        app_handle: &AppHandle,
        event_id: &str,
        payload: Option<flow_like_types::Value>,
        callback: Option<Arc<BufferedInterComHandler>>,
    ) -> Result<Result<(), String>> {
        self.fire_event_with_request(app_handle, event_id, payload, callback, None)
    }

    pub(crate) fn fire_event_with_request(
        &self,
        app_handle: &AppHandle,
        event_id: &str,
        payload: Option<flow_like_types::Value>,
        callback: Option<Arc<BufferedInterComHandler>>,
        request_control: Option<Arc<crate::event_bus::EventRequestControl>>,
    ) -> Result<Result<(), String>> {
        tracing::debug!("🔥 [FIRE_EVENT] Starting fire_event for: {}", event_id);

        let registration = self
            .storage
            .get_registration(event_id)?
            .ok_or_else(|| anyhow::anyhow!("Event {event_id} has no trigger on this computer"))?;
        if registration.profile_binding.is_none() {
            return Err(anyhow::anyhow!(
                "Enable the trigger of event {event_id} again to bind it to a profile"
            ));
        }
        if !registration.offline && !registration.has_verified_token() {
            return Err(anyhow::anyhow!(
                "Event {event_id} is registered for online execution without a personal access token verified with the hub; add one to its trigger in the event's settings"
            ));
        }

        let Some(event_bus_state) = app_handle.try_state::<crate::state::TauriEventBusState>()
        else {
            return Ok(Err("event bus not available".to_owned()));
        };
        Ok(event_bus_state
            .0
            .push_registration(registration, payload, callback, request_control))
    }

    fn lifecycle<'a>(&'a self, app_handle: &'a AppHandle) -> SinkLifecycle<'a> {
        SinkLifecycle {
            manager: self,
            app_handle,
            geofence: None,
        }
    }

    /// Saves `registration` and starts its sink. When the sink fails, what
    /// the registration replaced is put back (see [`Self::undo_registration`])
    /// and the sink's error is returned.
    async fn register_event(
        &self,
        lifecycle: &dyn TriggerLifecycle,
        registration: EventRegistration,
    ) -> Result<()> {
        tracing::info!(
            "Registering event {} with type {}",
            registration.event_id,
            registration.r#type
        );

        let previous = self.storage.get_registration(&registration.event_id)?;
        self.storage.save_registration(&registration)?;

        let Err(error) = lifecycle.start(&registration).await else {
            self.storage
                .clear_failure(&registration.app_id, &registration.event_id)?;
            tracing::info!(
                "Registered event {} ({})",
                registration.event_id,
                registration.r#type
            );
            return Ok(());
        };
        self.undo_registration(lifecycle, registration, previous, &error)
            .await?;
        Err(error)
    }

    /// Undoes a registration whose sink failed with `error`. A row that
    /// existed before is written back, still running its old trigger, and the
    /// failure is recorded for the event's status; a brand-new row is removed
    /// again, keeping its credentials. A PAT the hub refused is dropped from
    /// both and the refusal recorded, so the next save asks for another token.
    async fn undo_registration(
        &self,
        lifecycle: &dyn TriggerLifecycle,
        registration: EventRegistration,
        previous: Option<EventRegistration>,
        error: &anyhow::Error,
    ) -> Result<()> {
        let token_refused = error.is::<TokenRefused>();
        let reason = format!("{error:#}");
        match previous {
            Some(previous) => {
                let refused_token = registration
                    .personal_access_token
                    .as_deref()
                    .filter(|_| token_refused);
                self.restore_registration(previous, refused_token, &reason)
            }
            None => {
                self.roll_back_registration(lifecycle, registration, token_refused, &reason)
                    .await
            }
        }
    }

    /// Writes `previous` back after its refresh failed with `reason`, without
    /// `refused_token`, the PAT the hub refused, if it held that one.
    fn restore_registration(
        &self,
        mut previous: EventRegistration,
        refused_token: Option<&str>,
        reason: &str,
    ) -> Result<()> {
        if refused_token
            .is_some_and(|token| previous.personal_access_token.as_deref() == Some(token))
        {
            previous.forget_token();
        }
        self.storage.save_registration(&previous)?;
        self.storage
            .record_failure(&previous.app_id, &previous.event_id, reason)
    }

    /// Removes a brand-new `registration` whose sink failed with `reason`,
    /// keeping its credentials except a PAT the hub refused.
    async fn roll_back_registration(
        &self,
        lifecycle: &dyn TriggerLifecycle,
        mut registration: EventRegistration,
        token_refused: bool,
        reason: &str,
    ) -> Result<()> {
        if let Err(cleanup) = lifecycle.stop(&registration).await {
            tracing::warn!(
                event_id = %registration.event_id,
                error = %cleanup,
                "Cleaning up a trigger that failed to start also failed"
            );
        }
        if token_refused {
            registration.forget_token();
            self.storage
                .record_failure(&registration.app_id, &registration.event_id, reason)?;
        }
        self.keep_credentials(&registration)?;
        self.storage.delete_registration(&registration.event_id)
    }

    /// Stashes what `registration` signs in with, so turning its trigger on
    /// again needs no new PAT; one that holds nothing clears the stash.
    fn keep_credentials(&self, registration: &EventRegistration) -> Result<()> {
        if registration.has_credentials() {
            self.storage.stash_credentials(registration)
        } else {
            self.storage
                .clear_stashed_credentials(&registration.app_id, &registration.event_id)
        }
    }

    /// The sink that runs triggers of `config` and the name it starts under,
    /// or `None` for a trigger type that has no sink yet.
    fn sink_of(config: &EventConfig) -> Option<(&str, &dyn EventSink)> {
        Some(match config {
            EventConfig::Cron(sink) => ("cron", sink),
            EventConfig::Discord(sink) => ("discord", sink),
            EventConfig::Email(sink) => ("email", sink),
            EventConfig::Http(sink) => ("http", sink),
            EventConfig::Rss(sink) => ("rss", sink),
            EventConfig::Slack(sink) => ("slack", sink),
            EventConfig::Telegram(sink) => ("telegram", sink),
            EventConfig::WebWatcher(sink) => ("web_watcher", sink),
            EventConfig::File(sink) => ("file", sink),
            EventConfig::Webhook(sink) => ("webhook", sink),
            EventConfig::GitHub(sink) => ("github", sink),
            EventConfig::Mqtt(sink) => ("mqtt", sink),
            EventConfig::Notion(sink) => ("notion", sink),
            EventConfig::GeoLocation(sink) => ("geolocation", sink),
            EventConfig::Deeplink(sink) => ("deeplink", sink),
            EventConfig::Daemon(sink) => ("daemon", sink),
            EventConfig::Nfc(_) | EventConfig::Shortcut(_) | EventConfig::Mcp(_) => return None,
        })
    }

    /// Starts the sink of `registration`, which is already stored. A location
    /// trigger starts only with `geofence`, the hub's clearance of it.
    async fn start_trigger(
        &self,
        app_handle: &AppHandle,
        registration: &EventRegistration,
        geofence: Option<&GeofenceClearance>,
    ) -> Result<()> {
        let Some((kind, sink)) = Self::sink_of(&registration.config) else {
            tracing::warn!(
                event_id = %registration.event_id,
                "The {} sink is not implemented yet",
                registration.r#type
            );
            return Ok(());
        };
        self.ensure_sink_started(kind, app_handle, sink).await?;
        match &registration.config {
            EventConfig::GeoLocation(location) => {
                location
                    .register_cleared(app_handle, registration, self.db.clone(), geofence)
                    .await
            }
            _ => {
                sink.on_register(app_handle, registration, self.db.clone())
                    .await
            }
        }
    }

    /// Stops the sink of `registration` and deletes its row.
    async fn unregister(
        &self,
        lifecycle: &dyn TriggerLifecycle,
        registration: &EventRegistration,
    ) -> Result<()> {
        lifecycle.stop(registration).await?;
        self.storage.delete_registration(&registration.event_id)?;
        tracing::info!("Unregistered event: {}", registration.event_id);
        Ok(())
    }

    /// Unregister an event
    pub async fn unregister_event(&self, app_handle: &AppHandle, event_id: &str) -> Result<()> {
        let registration = self
            .storage
            .get_registration(event_id)?
            .ok_or_else(|| anyhow::anyhow!("Registration not found: {}", event_id))?;
        self.unregister(&self.lifecycle(app_handle), &registration)
            .await
    }

    /// Stops the sink of `registration`; its row stays.
    async fn stop_trigger(
        &self,
        app_handle: &AppHandle,
        registration: &EventRegistration,
    ) -> Result<()> {
        let Some((_, sink)) = Self::sink_of(&registration.config) else {
            tracing::warn!(
                event_id = %registration.event_id,
                "Unregister called for the unimplemented {} sink",
                registration.r#type
            );
            return Ok(());
        };
        sink.on_unregister(app_handle, registration, self.db.clone())
            .await
    }

    /// The sink configuration this device registers for `event`, or the reason
    /// it must not register one. Events that run on the hub are refused: a
    /// local registration would fire them here and, for cron, retry them
    /// forever. A Remote location region still registers, because only a
    /// device can watch it; it forwards each crossing to the hub.
    pub fn local_sink_config(event: &Event) -> Result<EventConfig, String> {
        if let Some(reason) = Self::event_blocker(event) {
            return Err(reason.to_owned());
        }
        let config = Self::parse_event_config(&event.event_type, &event.config)
            .map_err(|err| format!("sink config could not be parsed: {err:#}"))?;
        if config.is_remote_only() {
            return Err("sink is configured for remote execution".to_owned());
        }
        Ok(config)
    }

    /// Why `event` must not have a trigger here, judged without its config.
    fn event_blocker(event: &Event) -> Option<&str> {
        if event.is_device_source() {
            Some("event is deployed to another device")
        } else if !Self::supports_sink_registration(&event.event_type) {
            Some("event type has no local sink")
        } else if UNIMPLEMENTED_TRIGGERS.contains(&event.event_type.as_str()) {
            Some("trigger type is not implemented on this device")
        } else if !event.active {
            Some("event is inactive")
        } else if Self::runs_on_hub(event) {
            Some("event runs remotely")
        } else {
            Self::platform_blocker(
                &event.event_type,
                cfg!(any(target_os = "ios", target_os = "macos")),
            )
        }
    }

    /// Trigger types that run on the hub but need a device to notice them:
    /// a Remote event of such a type registers here and forwards to the hub.
    pub fn forwards_to_hub(event_type: &str) -> bool {
        matches!(event_type, "geolocation")
    }

    /// Whether `event` runs on the hub with nothing for this device to watch.
    fn runs_on_hub(event: &Event) -> bool {
        event.execution_mode == EventExecutionMode::Remote
            && !Self::forwards_to_hub(&event.event_type)
    }

    fn platform_blocker(event_type: &str, apple: bool) -> Option<&'static str> {
        (event_type == "geolocation" && !apple)
            .then_some("location triggers need iOS or macOS on this device")
    }

    /// Whether a registration of `event` on this device is stale regardless of
    /// the event's other settings: it runs on the hub or on another device.
    fn registration_is_stale(event: &Event) -> bool {
        Self::runs_on_hub(event) || event.is_device_source()
    }

    /// Why `event` must not have a sink registration on this device, if anything.
    pub fn local_registration_blocker(event: &Event) -> Option<String> {
        Self::local_sink_config(event).err()
    }

    /// This app's registration of `event_id`. A row of another app that
    /// reuses the event id is not this event's trigger.
    fn own_registration(&self, app_id: &str, event_id: &str) -> Result<Option<EventRegistration>> {
        Ok(self
            .storage
            .get_registration(event_id)?
            .filter(|registration| registration.app_id == app_id))
    }

    /// The credentials a save of `event` in `app_id` with `update` carries
    /// over, or `None` when the save leaves them alone: Keep and Repair only
    /// touch this app's trigger of the same type and destination.
    fn credentials_for_update(
        &self,
        app_id: &str,
        event: &Event,
        update: TriggerUpdate,
    ) -> Result<Option<TriggerCredentials>> {
        let own = self.own_registration(app_id, &event.id)?;
        let current = own.as_ref().filter(|row| row.is_trigger_of(event));
        if update != TriggerUpdate::Register && current.is_none() {
            return Ok(None);
        }
        self.held_credentials(app_id, &event.id, own.as_ref())
            .map(Some)
    }

    /// What a save of `event_id` in `app_id` carries over: the credentials of
    /// this app's registration `own`, else what a decline stashed.
    fn held_credentials(
        &self,
        app_id: &str,
        event_id: &str,
        own: Option<&EventRegistration>,
    ) -> Result<TriggerCredentials> {
        Ok(match own {
            Some(row) => TriggerCredentials::of(row),
            None => self
                .storage
                .stashed_credentials(app_id, event_id)?
                .unwrap_or_default(),
        })
    }

    /// Removes this app's trigger of `event_id` and keeps its credentials,
    /// so allowing the trigger again needs no new PAT. A refusal is recorded
    /// for the event's status and a refused PAT is not kept; any other
    /// decline replaces an earlier failure.
    async fn decline(
        &self,
        lifecycle: &dyn TriggerLifecycle,
        app_id: &str,
        event_id: &str,
        refusal: Option<&Refusal>,
    ) -> Result<()> {
        if let Some(mut registration) = self.own_registration(app_id, event_id)? {
            if refusal.is_some_and(|refusal| refusal.token_refused) {
                registration.forget_token();
            }
            self.keep_credentials(&registration)?;
            self.unregister(lifecycle, &registration).await?;
        }
        match refusal {
            Some(refusal) => self
                .storage
                .record_failure(app_id, event_id, &refusal.reason),
            None => self.storage.clear_failure(app_id, event_id),
        }
    }

    /// Removes the trigger of `event_id` after the user declined it. Its
    /// credentials are kept, so allowing the trigger later needs no new PAT.
    pub async fn decline_local_registration(
        &self,
        app_handle: &AppHandle,
        app_id: &str,
        event_id: &str,
    ) -> Result<()> {
        self.decline(&self.lifecycle(app_handle), app_id, event_id, None)
            .await
    }

    /// Removes a trigger the hub refused, keeping why for the event's status.
    pub async fn reject_registration(
        &self,
        app_handle: &AppHandle,
        app_id: &str,
        event_id: &str,
        refusal: &Refusal,
    ) -> Result<()> {
        self.decline(&self.lifecycle(app_handle), app_id, event_id, Some(refusal))
            .await
    }

    /// Removes this app's trigger of `event_id` and forgets its credentials.
    pub async fn remove_trigger(
        &self,
        app_handle: &AppHandle,
        app_id: &str,
        event_id: &str,
    ) -> Result<()> {
        if let Some(registration) = self.own_registration(app_id, event_id)? {
            self.unregister(&self.lifecycle(app_handle), &registration)
                .await?;
        }
        self.forget_credentials(app_id, event_id)
    }

    /// Drops what this device keeps of a removed trigger of `event_id`: its
    /// stashed credentials and its last failure.
    pub fn forget_credentials(&self, app_id: &str, event_id: &str) -> Result<()> {
        self.storage.clear_stashed_credentials(app_id, event_id)?;
        self.storage.clear_failure(app_id, event_id)
    }

    /// What saving `event` in `app_id` does to this device's trigger, and
    /// whether the user must choose a token first: an online trigger needs a
    /// PAT of the signed-in account, from its row or from the stash.
    pub fn registration_plan(
        &self,
        event: &Event,
        app_id: &str,
        offline: bool,
        account: Option<&TriggerAccount>,
    ) -> Result<LocalTriggerPlan> {
        if Self::local_registration_blocker(event).is_some() {
            return Ok(LocalTriggerPlan::NONE);
        }
        let row = self.storage.get_registration(&event.id)?;
        let moves_from_app = row
            .as_ref()
            .filter(|row| row.app_id != app_id)
            .map(|row| row.app_id.clone());
        let own = row.filter(|row| row.app_id == app_id);
        let registration = if own
            .as_ref()
            .is_some_and(|row| row.is_trigger_of(event) && row.profile_binding.is_some())
        {
            LocalSinkPlan::Existing
        } else {
            LocalSinkPlan::New
        };
        let held = self.held_credentials(app_id, &event.id, own.as_ref())?;
        Ok(LocalTriggerPlan {
            registration,
            needs_token: !offline && !held.owned_by(account),
            moves_from_app,
        })
    }

    /// This device's trigger for `event` in `app_id`. A row of another app,
    /// trigger type or destination is not this event's trigger.
    pub fn trigger_state(
        &self,
        event: &Event,
        app_id: &str,
        offline: bool,
        account: Option<&TriggerAccount>,
    ) -> Result<LocalTriggerState> {
        if Self::local_registration_blocker(event).is_some() {
            return Ok(LocalTriggerState::None);
        }
        let row = self
            .own_registration(app_id, &event.id)?
            .filter(|row| row.is_trigger_of(event));
        let failure = self.storage.failure(app_id, &event.id)?;
        Ok(match row {
            None => failure.map_or(LocalTriggerState::Unregistered, |reason| {
                LocalTriggerState::Failed { reason }
            }),
            Some(row) => Self::row_state(
                row.profile_binding.is_some() && (offline || row.owns_token(account)),
                failure,
            ),
        })
    }

    /// The state of `registration` judged by its own row, for listings that
    /// do not load the event.
    pub fn registration_state(
        &self,
        registration: &EventRegistration,
        account: Option<&TriggerAccount>,
    ) -> Result<LocalTriggerState> {
        let failure = self
            .storage
            .failure(&registration.app_id, &registration.event_id)?;
        Ok(Self::row_state(registration.can_fire(account), failure))
    }

    fn row_state(signs_in: bool, failure: Option<String>) -> LocalTriggerState {
        match (signs_in, failure) {
            (false, _) => LocalTriggerState::NeedsToken,
            (true, Some(reason)) => LocalTriggerState::Failed { reason },
            (true, None) => LocalTriggerState::Ready,
        }
    }

    /// Registers, refreshes or repairs this device's trigger for `event` of
    /// `app_id`, signing in as `sign_in` resolved beforehand. A blocked event
    /// loses its trigger and keeps its credentials. An online trigger needs a
    /// PAT of the signed-in account, otherwise this fails with
    /// [`MissingTriggerToken`] and the existing row stays as it was. A
    /// location trigger registers only with the clearance `sign_in` carries.
    pub async fn register_from_flow_event(
        &self,
        app_handle: &AppHandle,
        app_id: &str,
        event: &Event,
        mut sign_in: TriggerSignIn,
        oauth_tokens: Option<HashMap<String, OAuthToken>>,
        update: TriggerUpdate,
    ) -> Result<()> {
        let geofence = sign_in.geofence.take();
        let lifecycle = SinkLifecycle {
            manager: self,
            app_handle,
            geofence: geofence.as_ref(),
        };
        let config = match Self::local_sink_config(event) {
            Ok(config) => config,
            Err(reason) => {
                tracing::info!(
                    event_id = %event.id,
                    event_type = %event.event_type,
                    reason = %reason,
                    "Skipping local sink registration"
                );
                return self.decline(&lifecycle, app_id, &event.id, None).await;
            }
        };
        let (profile, _) = sign_in.profile_binding.resolve(app_handle).await?;
        let oauth_tokens = oauth_tokens
            .map(|tokens| hub_bound_oauth_tokens(tokens, &profile.hub_profile))
            .transpose()?;

        let decision = self.decide(event, app_id, config, sign_in, oauth_tokens, update)?;
        if let TriggerDecision::Register(registration) = &decision
            && !registration.remote
        {
            Self::ensure_trigger_approved(app_handle, app_id, event, &profile.hub_profile).await?;
        }
        self.apply(&lifecycle, app_id, &event.id, decision).await
    }

    /// What saving `event` in `app_id` with `config` does to its trigger here.
    /// Keep never creates a trigger and declines one of another type or
    /// destination instead of converting it; Repair only signs in this app's
    /// trigger of the same type and destination.
    fn decide(
        &self,
        event: &Event,
        app_id: &str,
        config: EventConfig,
        sign_in: TriggerSignIn,
        oauth_tokens: Option<HashMap<String, OAuthToken>>,
        update: TriggerUpdate,
    ) -> Result<TriggerDecision> {
        let own = self.own_registration(app_id, &event.id)?;
        let current = own.as_ref().filter(|row| row.is_trigger_of(event));
        if let Some(decision) = Self::settled_by_update(&event.id, update, own.as_ref(), current)? {
            return Ok(decision);
        }

        let held = self.held_credentials(app_id, &event.id, own.as_ref())?;
        let TriggerSignIn {
            offline,
            profile_binding,
            account,
            given,
            ..
        } = sign_in;
        if update == TriggerUpdate::Keep && held.profile_binding.as_ref() != Some(&profile_binding)
        {
            return Err(MissingTriggerToken(event.id.clone()).into());
        }
        if !offline
            && given
                .as_ref()
                .is_some_and(|given| given.owner.as_ref() != account.as_ref())
        {
            return Err(MissingTriggerToken(event.id.clone()).into());
        }
        let registration = Self::next_registration(
            event,
            app_id,
            config,
            offline,
            profile_binding,
            held,
            given,
            oauth_tokens,
        );
        Self::ensure_signs_in(&registration, account.as_ref())?;
        if update == TriggerUpdate::Keep
            && current.is_some_and(|row| row.same_trigger(&registration))
        {
            return Ok(TriggerDecision::Unchanged);
        }
        Ok(TriggerDecision::Register(Box::new(registration)))
    }

    /// An online trigger signs in with a PAT of the signed-in `account`.
    fn ensure_signs_in(
        registration: &EventRegistration,
        account: Option<&TriggerAccount>,
    ) -> Result<()> {
        if registration.can_fire(account) {
            Ok(())
        } else {
            Err(MissingTriggerToken(registration.event_id.clone()).into())
        }
    }

    /// What `update` decides before credentials matter, given this app's row
    /// `own` and `current`, that row when it is the event's trigger.
    fn settled_by_update(
        event_id: &str,
        update: TriggerUpdate,
        own: Option<&EventRegistration>,
        current: Option<&EventRegistration>,
    ) -> Result<Option<TriggerDecision>> {
        Ok(match update {
            TriggerUpdate::Keep if own.is_none() => Some(TriggerDecision::Unchanged),
            TriggerUpdate::Keep if current.is_none() => {
                tracing::info!(
                    event_id = %event_id,
                    "Event changed its trigger type or moved between this device and the hub; its trigger waits to be turned on again"
                );
                Some(TriggerDecision::Decline)
            }
            TriggerUpdate::Repair if current.is_none() => anyhow::bail!(NOTHING_TO_REPAIR),
            _ => None,
        })
    }

    async fn apply(
        &self,
        lifecycle: &dyn TriggerLifecycle,
        app_id: &str,
        event_id: &str,
        decision: TriggerDecision,
    ) -> Result<()> {
        match decision {
            TriggerDecision::Unchanged => Ok(()),
            TriggerDecision::Decline => self.decline(lifecycle, app_id, event_id, None).await,
            TriggerDecision::Register(registration) => {
                self.replace_registration(lifecycle, *registration).await
            }
        }
    }

    /// A trigger that runs `event` on this device needs its board's
    /// automation approved; the save cannot ask, so it only checks.
    async fn ensure_trigger_approved(
        app_handle: &AppHandle,
        app_id: &str,
        event: &Event,
        hub: &flow_like::profile::Profile,
    ) -> Result<()> {
        let state = crate::state::TauriFlowLikeState::construct(app_handle).await?;
        let template = crate::functions::flow::run::resolve_run_template(
            &state,
            app_id,
            &event.board_id,
            event.board_version,
        )
        .await?;
        crate::functions::automation_approval::ensure_automation_approved(
            app_handle,
            app_id,
            &template.board,
            Some(&event.id),
            false,
            hub,
        )
        .await
    }

    /// The registration saving `event` stores: a given PAT wins, otherwise the
    /// held credentials (this app's row, else the stash) carry over. OAuth
    /// grants carry over only while the trigger signs in as the same person.
    fn next_registration(
        event: &Event,
        app_id: &str,
        config: EventConfig,
        offline: bool,
        profile_binding: TriggerProfile,
        mut held: TriggerCredentials,
        given: Option<GivenToken>,
        oauth_tokens: Option<HashMap<String, OAuthToken>>,
    ) -> EventRegistration {
        if held.profile_binding.as_ref() != Some(&profile_binding) {
            held = TriggerCredentials::default();
        }
        let owner = match &given {
            Some(given) => given.owner.clone(),
            None => held.owner(),
        };
        let mut merged_oauth_tokens = match &owner {
            Some(owner) => held.oauth_tokens_for(owner),
            None => held.oauth_tokens.clone(),
        };
        merged_oauth_tokens.extend(oauth_tokens.unwrap_or_default());
        let (personal_access_token, pat_owner_sub, hub_origin) = match given {
            Some(GivenToken { pat, owner }) => match owner {
                Some(owner) => (Some(pat), Some(owner.sub), Some(owner.hub_origin)),
                None => (Some(pat), None, None),
            },
            None => (
                held.personal_access_token,
                held.pat_owner_sub,
                held.hub_origin,
            ),
        };

        EventRegistration {
            event_id: event.id.clone(),
            name: event.name.clone(),
            r#type: event.event_type.clone(),
            updated_at: event.updated_at,
            created_at: event.created_at,
            config,
            offline,
            app_id: app_id.to_string(),
            default_payload: None, // TODO: Parse from event if needed
            personal_access_token,
            pat_owner_sub,
            hub_origin,
            profile_binding: Some(profile_binding),
            remote: event.execution_mode == EventExecutionMode::Remote,
            oauth_tokens: merged_oauth_tokens,
        }
    }

    /// Stores and starts `registration`. Another app's trigger for the same
    /// event id is declined first, and one of another type or destination is
    /// stopped first, so neither keeps running beside it.
    async fn replace_registration(
        &self,
        lifecycle: &dyn TriggerLifecycle,
        registration: EventRegistration,
    ) -> Result<()> {
        if let Some(current) = self.storage.get_registration(&registration.event_id)? {
            if current.app_id != registration.app_id {
                self.decline(lifecycle, &current.app_id, &current.event_id, None)
                    .await?;
            } else if current.r#type != registration.r#type || current.remote != registration.remote
            {
                self.unregister(lifecycle, &current).await?;
            }
        }
        let app_id = registration.app_id.clone();
        let event_id = registration.event_id.clone();
        self.register_event(lifecycle, registration).await?;
        self.storage.clear_stashed_credentials(&app_id, &event_id)
    }

    /// Parse event config bytes based on event_type
    fn parse_event_config(event_type: &str, config_bytes: &[u8]) -> Result<EventConfig> {
        // If config is empty, try to create default config based on type
        if config_bytes.is_empty() {
            return Err(anyhow::anyhow!(
                "Empty config for event type: {}",
                event_type
            ));
        }

        // Deserialize the config JSON
        let config_json: serde_json::Value =
            serde_json::from_slice(config_bytes).context("Failed to parse config as JSON")?;

        // Match event_type to sink type and parse appropriate config
        match event_type {
            "cron" => {
                let cron_config: super::cron::CronSink =
                    serde_json::from_value(config_json).context("Failed to parse cron config")?;
                Ok(EventConfig::Cron(cron_config))
            }
            "api" | "http" => {
                let http_config: super::http::HttpSink =
                    serde_json::from_value(config_json).context("Failed to parse HTTP config")?;
                Ok(EventConfig::Http(http_config))
            }
            "mail" | "email" => {
                let email_config: super::email::EmailSink =
                    serde_json::from_value(config_json).context("Failed to parse email config")?;
                Ok(EventConfig::Email(email_config))
            }
            "rss" => {
                let rss_config: super::rss::RSSSink =
                    serde_json::from_value(config_json).context("Failed to parse RSS config")?;
                Ok(EventConfig::Rss(rss_config))
            }
            "discord" => {
                let discord_config: super::discord::DiscordSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse Discord config")?;
                Ok(EventConfig::Discord(discord_config))
            }
            "telegram" => {
                let telegram_config: super::telegram::TelegramSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse Telegram config")?;
                Ok(EventConfig::Telegram(telegram_config))
            }
            "slack" => {
                let slack_config: super::slack::SlackSink =
                    serde_json::from_value(config_json).context("Failed to parse Slack config")?;
                Ok(EventConfig::Slack(slack_config))
            }
            "deeplink" => {
                let deeplink_config: super::deeplink::DeeplinkSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse deeplink config")?;
                Ok(EventConfig::Deeplink(deeplink_config))
            }
            "webhook" => {
                let webhook_config: super::webhook::WebhookSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse Webhook config")?;
                Ok(EventConfig::Webhook(webhook_config))
            }
            "mqtt" => {
                let mqtt_config: super::mqtt::MQTTSink =
                    serde_json::from_value(config_json).context("Failed to parse MQTT config")?;
                Ok(EventConfig::Mqtt(mqtt_config))
            }
            "github" => {
                let github_config: super::github::GitHubSink =
                    serde_json::from_value(config_json).context("Failed to parse GitHub config")?;
                Ok(EventConfig::GitHub(github_config))
            }
            "file" => {
                let file_config: super::file::FileSink =
                    serde_json::from_value(config_json).context("Failed to parse File config")?;
                Ok(EventConfig::File(file_config))
            }
            "web_watcher" => {
                let web_watcher_config: super::web_watcher::WebWatcherSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse WebWatcher config")?;
                Ok(EventConfig::WebWatcher(web_watcher_config))
            }
            "notion" => {
                let notion_config: super::notion::NotionSink =
                    serde_json::from_value(config_json).context("Failed to parse Notion config")?;
                Ok(EventConfig::Notion(notion_config))
            }
            "geolocation" => {
                let geolocation_config: super::geolocation::GeoLocationSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse GeoLocation config")?;
                Ok(EventConfig::GeoLocation(geolocation_config))
            }
            "nfc" => {
                let nfc_config: super::nfc::NFCSink =
                    serde_json::from_value(config_json).context("Failed to parse NFC config")?;
                Ok(EventConfig::Nfc(nfc_config))
            }
            "shortcut" => {
                let shortcut_config: super::shortcut::ShortcutSink =
                    serde_json::from_value(config_json)
                        .context("Failed to parse Shortcut config")?;
                Ok(EventConfig::Shortcut(shortcut_config))
            }
            "mcp" => {
                let mcp_config: super::mcp::MCPSink =
                    serde_json::from_value(config_json).context("Failed to parse MCP config")?;
                Ok(EventConfig::Mcp(mcp_config))
            }
            "daemon" => {
                let daemon_config: super::daemon::DaemonSink =
                    serde_json::from_value(config_json).context("Failed to parse daemon config")?;
                Ok(EventConfig::Daemon(daemon_config))
            }
            // Add more sink types as needed
            _ => Err(anyhow::anyhow!(
                "Unsupported event type for sink registration: {}",
                event_type
            )),
        }
    }

    /// Removes the trigger of `event_id` and forgets its credentials. With
    /// `app_id`, only a trigger of that app is removed.
    pub async fn remove_event_sink(
        &self,
        app_handle: &AppHandle,
        app_id: Option<&str>,
        event_id: &str,
    ) -> Result<()> {
        tracing::info!("Removing event sink for event: {}", event_id);
        let registration = self
            .storage
            .get_registration(event_id)?
            .filter(|registration| app_id.is_none_or(|app_id| registration.app_id == app_id))
            .ok_or_else(|| anyhow::anyhow!("Registration not found: {}", event_id))?;
        self.unregister(&self.lifecycle(app_handle), &registration)
            .await?;
        self.forget_credentials(&registration.app_id, event_id)
    }

    /// Get all registrations
    pub fn list_registrations(&self) -> Result<Vec<EventRegistration>> {
        self.storage.list_registrations()
    }

    /// Get a specific registration by event_id
    pub fn get_registration(&self, event_id: &str) -> Result<Option<EventRegistration>> {
        self.storage.get_registration(event_id)
    }

    /// Whether `event_id` has a registration that can fire while `account`
    /// is signed in.
    pub fn is_event_active(&self, event_id: &str, account: Option<&TriggerAccount>) -> bool {
        self.storage
            .get_registration(event_id)
            .ok()
            .flatten()
            .is_some_and(|registration| registration.can_fire(account))
    }

    /// Check if an event type supports sink registration
    /// Some event types (like quick_action, chat) don't need sink infrastructure
    pub fn supports_sink_registration(event_type: &str) -> bool {
        matches!(
            event_type,
            "cron"
                | "api"
                | "http"
                | "mail"
                | "email"
                | "rss"
                | "discord"
                | "webhook"
                | "slack"
                | "telegram"
                | "mqtt"
                | "github"
                | "notion"
                | "web_watcher"
                | "file"
                | "geolocation"
                | "deeplink"
                | "nfc"
                | "shortcut"
                | "mcp"
                | "daemon"
        )
    }

    /// Upper bound on resolving one app's events during the startup prune,
    /// which runs while the manager lock blocks every other sink operation.
    const PRUNE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

    /// Which of `event_ids` have a stale registration in `app_id`, and whether
    /// the app is offline. An app or event that does not load counts as
    /// local, so its registrations stay untouched.
    async fn inspect_app_registrations(
        state: Arc<FlowLikeState>,
        app_id: String,
        event_ids: Vec<String>,
    ) -> AppRegistrations {
        let app = match App::load(app_id.clone(), state).await {
            Ok(app) => app,
            Err(err) => {
                tracing::debug!(
                    app_id = %app_id,
                    error = %err,
                    "App of sink registrations did not load, keeping them"
                );
                return AppRegistrations::default();
            }
        };

        let mut stale = Vec::new();
        for event_id in event_ids {
            match app.get_event(&event_id, None).await {
                Ok(event) if Self::registration_is_stale(&event) => stale.push(event_id),
                Ok(_) => {}
                Err(err) => tracing::debug!(
                    event_id = %event_id,
                    error = %err,
                    "Event of sink registration did not load, keeping it"
                ),
            }
        }
        AppRegistrations {
            stale,
            offline: Some(matches!(app.visibility, AppVisibility::Offline)),
        }
    }

    /// Drops registrations whose live event now runs on the hub or only on
    /// another device, and corrects the offline flag of the rest. Stale rows
    /// are left behind by events switched elsewhere (the server, another
    /// device) or saved before those were excluded; the sinks would otherwise
    /// keep firing them locally. Rows marked offline for an online app never
    /// fired. Apps are resolved concurrently, each within
    /// [`Self::PRUNE_LOOKUP_TIMEOUT`].
    async fn prune_stale_registrations(
        &self,
        app_handle: &AppHandle,
        registrations: Vec<EventRegistration>,
    ) -> Vec<EventRegistration> {
        let state = match crate::state::TauriFlowLikeState::construct(app_handle).await {
            Ok(state) => state,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "Flow-Like state unavailable, keeping every sink registration"
                );
                return registrations;
            }
        };

        let mut by_app: HashMap<String, Vec<String>> = HashMap::new();
        for registration in &registrations {
            by_app
                .entry(registration.app_id.clone())
                .or_default()
                .push(registration.event_id.clone());
        }
        let (stale, offline_apps) = Self::inspect_apps(state, by_app).await;

        let mut kept = Vec::with_capacity(registrations.len());
        for mut registration in registrations {
            if stale.contains(&registration.event_id) {
                if let Err(err) = self
                    .remove_remote_registration(app_handle, &registration)
                    .await
                {
                    tracing::error!(
                        event_id = %registration.event_id,
                        error = %err,
                        "Failed to remove local sink registration of a remote event"
                    );
                    kept.push(registration);
                }
                continue;
            }
            if let Some(&offline) = offline_apps.get(&registration.app_id) {
                self.correct_offline(&mut registration, offline);
            }
            kept.push(registration);
        }

        kept
    }

    /// Resolves the apps of `by_app` concurrently: the events whose
    /// registration is stale, and whether each app that loaded is offline.
    async fn inspect_apps(
        state: Arc<FlowLikeState>,
        by_app: HashMap<String, Vec<String>>,
    ) -> (HashSet<String>, HashMap<String, bool>) {
        let lookups = by_app.into_iter().map(|(app_id, event_ids)| {
            let state = Arc::clone(&state);
            async move {
                let found = match flow_like_types::tokio::time::timeout(
                    Self::PRUNE_LOOKUP_TIMEOUT,
                    Self::inspect_app_registrations(state, app_id.clone(), event_ids),
                )
                .await
                {
                    Ok(found) => found,
                    Err(_) => {
                        tracing::warn!(
                            app_id = %app_id,
                            "Timed out resolving the events of sink registrations, keeping them"
                        );
                        AppRegistrations::default()
                    }
                };
                (app_id, found)
            }
        });
        let mut stale = HashSet::new();
        let mut offline_apps = HashMap::new();
        for (app_id, found) in join_all(lookups).await {
            stale.extend(found.stale);
            if let Some(offline) = found.offline {
                offline_apps.insert(app_id, offline);
            }
        }
        (stale, offline_apps)
    }

    /// Records that the app of `registration` is `offline` when its row says
    /// otherwise.
    fn correct_offline(&self, registration: &mut EventRegistration, offline: bool) {
        if offline == registration.offline {
            return;
        }
        match self.storage.set_offline(&registration.event_id, offline) {
            Ok(()) => {
                tracing::info!(
                    event_id = %registration.event_id,
                    offline,
                    "Corrected the offline flag of a sink registration"
                );
                registration.offline = offline;
            }
            Err(err) => tracing::warn!(
                event_id = %registration.event_id,
                error = %err,
                "Failed to correct the offline flag of a sink registration"
            ),
        }
    }

    /// Unregisters `registration` because its event runs remotely. A failing
    /// sink cleanup (for example a handler row that was never written) still
    /// drops the registration row, since the event must not fire here.
    async fn remove_remote_registration(
        &self,
        app_handle: &AppHandle,
        registration: &EventRegistration,
    ) -> Result<()> {
        if let Err(err) = self
            .unregister_event(app_handle, &registration.event_id)
            .await
        {
            tracing::warn!(
                event_id = %registration.event_id,
                error = %err,
                "Sink cleanup failed for a remote event, dropping its registration anyway"
            );
            self.storage.delete_registration(&registration.event_id)?;
        }
        tracing::info!(
            event_id = %registration.event_id,
            event_type = %registration.r#type,
            "Removed local sink registration of an event that runs remotely"
        );
        Ok(())
    }

    /// Initialize all sinks on app startup
    /// This loads all registrations from the database and starts their infrastructure
    /// NOTE: We only need to start the sink workers, not re-register events
    /// (the database already has the registrations)
    pub async fn init_from_storage(&self, app_handle: &AppHandle) -> Result<()> {
        let registrations = self.list_registrations()?;

        println!(
            "🔄 [SINK_MANAGER] Loading {} event registrations from database",
            registrations.len()
        );

        for reg in &registrations {
            println!(
                "🔄 [SINK_MANAGER] Found registration: {} (type: {})",
                reg.event_id, reg.r#type
            );
        }

        let registrations = self
            .prune_stale_registrations(app_handle, registrations)
            .await;

        for registration in &registrations {
            if registration.profile_binding.is_none() {
                tracing::info!(event_id = %registration.event_id,
                    "Trigger needs to be enabled again with a profile binding");
                continue;
            }
            if let EventConfig::Daemon(sink) = &registration.config {
                // Each run resolves its saved profile when it starts and restarts.
                if !registration.offline && !registration.has_verified_token() {
                    tracing::info!(
                        event_id = %registration.event_id,
                        "Daemon event of an online app has no personal access token verified with the hub, not starting it"
                    );
                    continue;
                }
                let result = async {
                    self.ensure_sink_started("daemon", app_handle, sink).await?;
                    sink.on_register(app_handle, registration, self.db.clone())
                        .await
                }
                .await;

                if let Err(err) = result {
                    tracing::error!(
                        event_id = %registration.event_id,
                        error = %err,
                        "Failed to restore daemon event"
                    );
                }
            }
        }

        // Group registrations by sink type to start each sink once
        let mut sink_types = std::collections::HashSet::new();
        for registration in &registrations {
            if registration.profile_binding.is_none() {
                continue;
            }
            // Extract sink type from config
            let sink_type = match &registration.config {
                EventConfig::Discord(_) => "discord",
                EventConfig::Email(_) => "email",
                EventConfig::Http(_) => "http",
                EventConfig::Rss(_) => "rss",
                EventConfig::Slack(_) => "slack",
                EventConfig::Telegram(_) => "telegram",
                EventConfig::WebWatcher(_) => "web_watcher",
                EventConfig::File(_) => "file",
                EventConfig::Webhook(_) => "webhook",
                EventConfig::GitHub(_) => "github",
                EventConfig::Mqtt(_) => "mqtt",
                EventConfig::Notion(_) => "notion",
                EventConfig::GeoLocation(_) => "geolocation",
                EventConfig::Cron(_) => "cron",
                _ => continue,
            };

            sink_types.insert(sink_type);
        }

        // Start each unique sink type without blocking the main initialization path
        println!(
            "📋 [SINK_MANAGER] Unique sink types to start: {:?}",
            sink_types
        );

        for sink_type in sink_types {
            let sink_type = sink_type.to_string();
            let manager = self.clone();
            let app_handle = app_handle.clone();

            flow_like_types::tokio::spawn(async move {
                tracing::info!("⚙️ Starting {} sink during initialization", sink_type);

                let start_result = match sink_type.as_str() {
                    "geolocation" => {
                        manager
                            .ensure_sink_started(
                                "geolocation",
                                &app_handle,
                                &super::geolocation::GeoLocationSink::default(),
                            )
                            .await
                    }
                    "cron" => {
                        let cron_sink = CronSink {
                            schedule: CronSchedule::Expression {
                                expression: "0 0 * * *".to_string(),
                            },
                            last_fired: None,
                            timezone: None,
                            sink_execution: None,
                        };
                        manager
                            .ensure_sink_started("cron", &app_handle, &cron_sink)
                            .await
                    }
                    "http" | "api" => {
                        let http_sink = super::http::HttpSink {
                            path: String::new(),
                            method: String::new(),
                            auth_token: None,
                            sink_execution: None,
                        };
                        manager
                            .ensure_sink_started("http", &app_handle, &http_sink)
                            .await
                    }
                    "discord" => {
                        let discord_sink = super::discord::DiscordSink {
                            token: String::new(),
                            bot_name: None,
                            bot_description: None,
                            intents: None,
                            channel_whitelist: None,
                            channel_blacklist: None,
                            respond_to_mentions: true,
                            respond_to_dms: true,
                            command_prefix: String::new(),
                        };
                        manager
                            .ensure_sink_started("discord", &app_handle, &discord_sink)
                            .await
                    }
                    "telegram" => {
                        let telegram_sink = super::telegram::TelegramSink {
                            bot_token: String::new(),
                            bot_name: None,
                            bot_description: None,
                            chat_whitelist: None,
                            chat_blacklist: None,
                            respond_to_mentions: true,
                            respond_to_private: true,
                            command_prefix: String::new(),
                        };
                        manager
                            .ensure_sink_started("telegram", &app_handle, &telegram_sink)
                            .await
                    }
                    "slack" => {
                        let slack_sink = super::slack::SlackSink {
                            bot_token: String::new(),
                            app_token: None,
                            channel_id: None,
                            team_id: None,
                            last_event_ts: None,
                        };
                        manager
                            .ensure_sink_started("slack", &app_handle, &slack_sink)
                            .await
                    }
                    "email" => {
                        let email_sink = super::email::EmailSink {
                            imap_server: String::new(),
                            imap_port: 993,
                            username: String::new(),
                            password: String::new(),
                            folder: None,
                            use_tls: true,
                            last_seen_uid: None,
                        };
                        manager
                            .ensure_sink_started("email", &app_handle, &email_sink)
                            .await
                    }
                    "rss" => {
                        let rss_sink = super::rss::RSSSink {
                            feed_url: String::new(),
                            poll_interval: 300,
                            headers: None,
                            filter_keywords: None,
                        };
                        manager
                            .ensure_sink_started("rss", &app_handle, &rss_sink)
                            .await
                    }
                    "deeplink" => {
                        let deeplink_sink = super::deeplink::DeeplinkSink {
                            route: String::new(),
                        };
                        manager
                            .ensure_sink_started("deeplink", &app_handle, &deeplink_sink)
                            .await
                    }
                    _ => {
                        tracing::debug!(
                            "Sink type {} will be started on first registration",
                            sink_type
                        );
                        Ok(())
                    }
                };

                if let Err(err) = start_result {
                    tracing::error!("❌ Failed to start {} sink: {}", sink_type, err);
                }
            });
        }

        tracing::info!(
            "✅ Sink initialization complete. {} event registrations ready.",
            registrations.len()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event_sink::daemon::DaemonRestartPolicy;
    use flow_like::flow::event::EventExposure;

    fn flow_event(event_type: &str, active: bool, execution_mode: EventExecutionMode) -> Event {
        let now = std::time::SystemTime::now();
        Event {
            id: "event".to_string(),
            name: "Event".to_string(),
            description: String::new(),
            board_id: "board".to_string(),
            board_version: None,
            node_id: "node".to_string(),
            variables: HashMap::new(),
            config: br#"{"expression":"0 9 * * *"}"#.to_vec(),
            active,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: event_type.to_string(),
            notes: None,
            event_version: (0, 0, 0),
            created_at: now,
            updated_at: now,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode,
            exposure: EventExposure::default(),
            correlation_mappings: None,
        }
    }

    #[test]
    fn supports_daemon_sink_registration() {
        assert!(EventSinkManager::supports_sink_registration("daemon"));
    }

    fn blocker(event: &Event) -> Option<String> {
        EventSinkManager::local_registration_blocker(event)
    }

    const REGION: &[u8] = br#"{"latitude":52.5,"longitude":13.4,"radius":200,"trigger_on":"Both"}"#;
    const APPLE: bool = cfg!(any(target_os = "ios", target_os = "macos"));

    fn region_event(execution_mode: EventExecutionMode) -> Event {
        let mut event = flow_event("geolocation", true, execution_mode);
        event.config = REGION.to_vec();
        event
    }

    fn account() -> TriggerAccount {
        TriggerAccount {
            profile_id: "profile-1".into(),
            issuer: "https://issuer.example".into(),
            sub: "user-1".to_string(),
            hub_origin: "https://hub.example".to_string(),
        }
    }

    fn profile_binding(subject: Option<&str>) -> TriggerProfile {
        TriggerProfile {
            profile_id: "profile-1".into(),
            hub_origin: "https://hub.example".into(),
            issuer: subject.map(|_| "https://issuer.example".into()),
            subject: subject.map(str::to_owned),
        }
    }

    fn cron_config(expression: &str) -> EventConfig {
        EventConfig::Cron(CronSink {
            schedule: CronSchedule::Expression {
                expression: expression.to_string(),
            },
            last_fired: None,
            timezone: None,
            sink_execution: None,
        })
    }

    /// An online cron registration of `app_id` whose PAT, when `owner` is
    /// set, was verified for that subject on the test hub.
    fn cron_registration(
        app_id: &str,
        pat: Option<&str>,
        owner: Option<&str>,
    ) -> EventRegistration {
        let now = std::time::SystemTime::now();
        EventRegistration {
            event_id: "event".to_string(),
            name: "Event".to_string(),
            r#type: "cron".to_string(),
            updated_at: now,
            created_at: now,
            config: cron_config("0 9 * * *"),
            offline: false,
            app_id: app_id.to_string(),
            default_payload: None,
            personal_access_token: pat.map(str::to_string),
            pat_owner_sub: owner.map(str::to_string),
            hub_origin: owner.map(|_| "https://hub.example".to_string()),
            profile_binding: Some(profile_binding(owner)),
            remote: false,
            oauth_tokens: HashMap::new(),
        }
    }

    fn region_registration() -> EventRegistration {
        EventRegistration {
            r#type: "geolocation".to_string(),
            config: EventSinkManager::parse_event_config("geolocation", REGION).unwrap(),
            remote: true,
            ..cron_registration("app", Some("pat_1"), Some("user-1"))
        }
    }

    fn user(sub: &str) -> TriggerAccount {
        TriggerAccount {
            profile_id: "profile-1".into(),
            issuer: "https://issuer.example".into(),
            sub: sub.to_string(),
            hub_origin: "https://hub.example".to_string(),
        }
    }

    /// An online save as `user-1`, optionally handing in `pat` verified for
    /// `owner`.
    fn online_save(given: Option<(&str, &str)>) -> TriggerSignIn {
        TriggerSignIn {
            offline: false,
            profile_binding: account().profile_binding(),
            account: Some(account()),
            given: given.map(|(pat, owner)| GivenToken {
                pat: pat.to_string(),
                owner: Some(user(owner)),
            }),
            geofence: None,
        }
    }

    fn http_config() -> EventConfig {
        EventConfig::Http(super::super::http::HttpSink {
            path: "/hook".to_string(),
            method: "POST".to_string(),
            auth_token: None,
            sink_execution: None,
        })
    }

    fn grant(subject: Option<&str>) -> OAuthToken {
        OAuthToken {
            hub_subject: subject.map(str::to_string),
            ..OAuthToken::new("access".to_string())
        }
    }

    fn providers(tokens: &HashMap<String, OAuthToken>) -> Vec<&str> {
        let mut providers: Vec<&str> = tokens.keys().map(String::as_str).collect();
        providers.sort();
        providers
    }

    /// The cron event of the tests, with the sink config a save derives from it.
    fn cron_event() -> (Event, EventConfig) {
        let event = flow_event("cron", true, EventExecutionMode::Local);
        let config = EventSinkManager::local_sink_config(&event).unwrap();
        (event, config)
    }

    /// This app's trigger of the cron test event, as its save would store it.
    fn cron_trigger(pat: Option<&str>, owner: Option<&str>) -> EventRegistration {
        EventRegistration {
            config: cron_event().1,
            ..cron_registration("app", pat, owner)
        }
    }

    /// What saving the cron test event in `app` decides.
    fn decide_cron(
        manager: &EventSinkManager,
        sign_in: TriggerSignIn,
        update: TriggerUpdate,
    ) -> Result<TriggerDecision> {
        let (event, config) = cron_event();
        manager.decide(&event, "app", config, sign_in, None, update)
    }

    fn registration_of(decision: Result<TriggerDecision>) -> EventRegistration {
        match decision {
            Ok(TriggerDecision::Register(registration)) => *registration,
            other => panic!("expected a registration, got {other:?}"),
        }
    }

    fn three_grants() -> HashMap<String, OAuthToken> {
        HashMap::from([
            ("github".to_string(), grant(None)),
            ("google".to_string(), grant(Some("user-1"))),
            ("dropbox".to_string(), grant(Some("user-3"))),
        ])
    }

    /// A manager on its own database file, removed again on drop.
    struct TestManager {
        manager: EventSinkManager,
        path: PathBuf,
    }

    impl TestManager {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "flow-like-sink-test-{}.sqlite",
                flow_like_types::create_id()
            ));
            Self {
                manager: EventSinkManager::new(path.to_str().unwrap()).unwrap(),
                path,
            }
        }
    }

    impl std::ops::Deref for TestManager {
        type Target = EventSinkManager;
        fn deref(&self) -> &EventSinkManager {
            &self.manager
        }
    }

    impl Drop for TestManager {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Records which trigger types were started and stopped, and refuses to
    /// start when asked to, or refuses the registration's PAT like the hub.
    #[derive(Default)]
    struct FakeLifecycle {
        refuse_start: bool,
        refuse_token: bool,
        started: Mutex<Vec<String>>,
        stopped: Mutex<Vec<String>>,
    }

    impl FakeLifecycle {
        fn refusing() -> Self {
            Self {
                refuse_start: true,
                ..Self::default()
            }
        }

        fn refusing_token() -> Self {
            Self {
                refuse_token: true,
                ..Self::default()
            }
        }

        fn started(&self) -> Vec<String> {
            self.started.lock().unwrap().clone()
        }

        fn stopped(&self) -> Vec<String> {
            self.stopped.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl TriggerLifecycle for FakeLifecycle {
        async fn start(&self, registration: &EventRegistration) -> Result<()> {
            self.started
                .lock()
                .unwrap()
                .push(registration.r#type.clone());
            if self.refuse_token {
                return Err(TokenRefused("The hub no longer accepts this token".into()).into());
            }
            anyhow::ensure!(!self.refuse_start, "sink refused to start");
            Ok(())
        }

        async fn stop(&self, registration: &EventRegistration) -> Result<()> {
            self.stopped
                .lock()
                .unwrap()
                .push(registration.r#type.clone());
            Ok(())
        }
    }

    #[test]
    fn remote_events_never_register_locally() {
        for event_type in ["cron", "http", "daemon", "deeplink"] {
            assert_eq!(
                blocker(&flow_event(event_type, true, EventExecutionMode::Remote)).as_deref(),
                Some("event runs remotely"),
                "{event_type}"
            );
        }
    }

    #[test]
    fn remote_location_regions_register_to_forward_to_the_hub() {
        let region = region_event(EventExecutionMode::Remote);
        assert!(EventSinkManager::forwards_to_hub("geolocation"));
        assert!(!EventSinkManager::registration_is_stale(&region));
        if APPLE {
            let config = EventSinkManager::local_sink_config(&region);
            assert!(
                matches!(config, Ok(EventConfig::GeoLocation(_))),
                "{config:?}"
            );
        }
        assert!(EventSinkManager::registration_is_stale(&flow_event(
            "cron",
            true,
            EventExecutionMode::Remote
        )));
    }

    #[test]
    fn geolocation_is_blocked_off_apple_platforms() {
        assert_eq!(
            EventSinkManager::platform_blocker("geolocation", false),
            Some("location triggers need iOS or macOS on this device")
        );
        assert_eq!(
            EventSinkManager::platform_blocker("geolocation", true),
            None
        );
        assert_eq!(EventSinkManager::platform_blocker("cron", false), None);
        assert_eq!(
            blocker(&region_event(EventExecutionMode::Local)).is_some(),
            !APPLE
        );
    }

    #[test]
    fn stub_types_never_register() {
        for event_type in UNIMPLEMENTED_TRIGGERS {
            assert!(EventSinkManager::supports_sink_registration(event_type));
            assert_eq!(
                blocker(&flow_event(event_type, true, EventExecutionMode::Local)).as_deref(),
                Some("trigger type is not implemented on this device"),
                "{event_type}"
            );
        }
    }

    /// The plan for saving the cron test event in `app`.
    fn cron_plan(
        manager: &EventSinkManager,
        offline: bool,
        account: Option<&TriggerAccount>,
    ) -> LocalTriggerPlan {
        manager
            .registration_plan(
                &flow_event("cron", true, EventExecutionMode::Local),
                "app",
                offline,
                account,
            )
            .unwrap()
    }

    fn expect_plan(registration: LocalSinkPlan, needs_token: bool) -> LocalTriggerPlan {
        LocalTriggerPlan {
            registration,
            needs_token,
            moves_from_app: None,
        }
    }

    #[test]
    fn plan_asks_for_a_token_only_when_an_online_trigger_holds_no_owned_token() {
        let manager = TestManager::new();
        let signed_in = account();
        assert_eq!(
            cron_plan(&manager, false, Some(&signed_in)),
            expect_plan(LocalSinkPlan::New, true)
        );
        assert_eq!(
            cron_plan(&manager, true, Some(&signed_in)),
            expect_plan(LocalSinkPlan::New, false)
        );

        manager
            .storage
            .stash_credentials(&cron_registration("app", Some("pat_1"), Some("user-1")))
            .unwrap();
        assert_eq!(
            cron_plan(&manager, false, Some(&signed_in)),
            expect_plan(LocalSinkPlan::New, false)
        );
        assert_eq!(
            cron_plan(&manager, false, None),
            expect_plan(LocalSinkPlan::New, true)
        );
    }

    #[test]
    fn plan_counts_only_a_token_verified_for_the_signed_in_account() {
        let manager = TestManager::new();
        let rows = [
            (
                cron_registration("app", Some("pat_1"), Some("user-1")),
                false,
            ),
            (cron_registration("app", Some("  "), Some("user-1")), true),
            (cron_registration("app", None, None), true),
            (
                cron_registration("app", Some("pat_2"), Some("user-2")),
                true,
            ),
            (cron_registration("app", Some("pat_3"), None), true),
            (
                EventRegistration {
                    hub_origin: Some("https://other-hub.example".to_string()),
                    ..cron_registration("app", Some("pat_4"), Some("user-1"))
                },
                true,
            ),
        ];
        for (row, needs_token) in rows {
            manager.storage.save_registration(&row).unwrap();
            assert_eq!(
                cron_plan(&manager, false, Some(&account())),
                expect_plan(LocalSinkPlan::Existing, needs_token),
                "{:?} owned by {:?} on {:?}",
                row.personal_access_token,
                row.pat_owner_sub,
                row.hub_origin
            );
        }

        manager
            .storage
            .save_registration(&cron_registration("app", Some("pat_1"), Some("user-1")))
            .unwrap();
        assert_eq!(
            cron_plan(&manager, false, None),
            expect_plan(LocalSinkPlan::Existing, true)
        );
    }

    #[test]
    fn plan_moves_another_apps_trigger_and_registers_nothing_for_a_hub_event() {
        let manager = TestManager::new();
        manager
            .storage
            .stash_credentials(&cron_registration("app", Some("pat_1"), Some("user-1")))
            .unwrap();
        manager
            .storage
            .save_registration(&cron_registration(
                "other-app",
                Some("pat_9"),
                Some("user-1"),
            ))
            .unwrap();
        assert_eq!(
            cron_plan(&manager, false, Some(&account())),
            LocalTriggerPlan {
                registration: LocalSinkPlan::New,
                needs_token: false,
                moves_from_app: Some("other-app".to_string()),
            }
        );
        manager
            .storage
            .clear_stashed_credentials("app", "event")
            .unwrap();
        assert!(cron_plan(&manager, false, Some(&account())).needs_token);

        assert_eq!(
            manager
                .registration_plan(
                    &flow_event("cron", true, EventExecutionMode::Remote),
                    "app",
                    false,
                    Some(&account()),
                )
                .unwrap(),
            LocalTriggerPlan::NONE
        );
    }

    #[test]
    fn trigger_state_flags_a_registration_without_a_token() {
        use LocalTriggerState::{NeedsToken, Ready};
        let manager = TestManager::new();
        let event = flow_event("cron", true, EventExecutionMode::Local);
        let signed_in = account();
        let state = |offline: bool| {
            manager
                .trigger_state(&event, "app", offline, Some(&signed_in))
                .unwrap()
        };
        assert_eq!(state(false), LocalTriggerState::Unregistered);

        let cron = |pat, owner| cron_registration("app", pat, owner);
        let other_hub_row = EventRegistration {
            hub_origin: Some("https://other-hub.example".to_string()),
            ..cron(Some("pat_3"), Some("user-1"))
        };
        let cases = [
            (cron(None, None), true, Ready),
            (cron(Some("pat_1"), Some("user-1")), false, Ready),
            (cron(None, None), false, NeedsToken),
            (cron(Some("  "), Some("user-1")), false, NeedsToken),
            (cron(Some("pat_2"), Some("user-2")), false, NeedsToken),
            (cron(Some("pat_legacy"), None), false, NeedsToken),
            (other_hub_row, false, NeedsToken),
        ];
        for (mut row, offline, expected) in cases {
            row.offline = offline;
            manager.storage.save_registration(&row).unwrap();
            let active = manager.is_event_active("event", Some(&signed_in));
            assert_eq!(state(offline), expected, "{row:?}");
            assert_eq!(active, expected == Ready, "{row:?}");
        }
    }

    #[test]
    fn trigger_state_ignores_blocked_events_and_other_apps_triggers() {
        let manager = TestManager::new();
        manager
            .storage
            .save_registration(&cron_registration("app", Some("pat_1"), Some("user-1")))
            .unwrap();
        for blocked in [
            flow_event("cron", false, EventExecutionMode::Local),
            flow_event("cron", true, EventExecutionMode::Remote),
        ] {
            assert_eq!(
                manager
                    .trigger_state(&blocked, "app", false, Some(&account()))
                    .unwrap(),
                LocalTriggerState::None
            );
        }

        manager
            .storage
            .save_registration(&cron_registration(
                "other-app",
                Some("pat_1"),
                Some("user-1"),
            ))
            .unwrap();
        assert_eq!(
            manager
                .trigger_state(
                    &flow_event("cron", true, EventExecutionMode::Local),
                    "app",
                    false,
                    Some(&account()),
                )
                .unwrap(),
            LocalTriggerState::Unregistered
        );
    }

    #[tokio::test]
    async fn stashed_credentials_stay_until_a_registration_uses_them() {
        let manager = TestManager::new();
        let declined = cron_registration("app", Some("pat_1"), Some("user-1"));
        manager.storage.stash_credentials(&declined).unwrap();

        for _ in 0..2 {
            let stashed = manager
                .storage
                .stashed_credentials("app", "event")
                .unwrap()
                .expect("credentials were stashed");
            assert_eq!(stashed.personal_access_token.as_deref(), Some("pat_1"));
            assert_eq!(stashed.pat_owner_sub.as_deref(), Some("user-1"));
            assert!(stashed.oauth_tokens.is_empty());
        }
        assert!(
            manager
                .storage
                .stashed_credentials("other-app", "event")
                .unwrap()
                .is_none()
        );

        assert!(
            manager
                .replace_registration(&FakeLifecycle::refusing(), declined.clone())
                .await
                .is_err()
        );
        assert!(
            manager
                .storage
                .stashed_credentials("app", "event")
                .unwrap()
                .is_some()
        );

        manager
            .replace_registration(&FakeLifecycle::default(), declined)
            .await
            .unwrap();
        assert!(
            manager
                .storage
                .stashed_credentials("app", "event")
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn failed_refresh_keeps_the_previous_registration() {
        let manager = TestManager::new();
        let event = flow_event("cron", true, EventExecutionMode::Local);
        let previous = cron_registration("app", Some("pat_1"), Some("user-1"));
        manager
            .register_event(&FakeLifecycle::default(), previous.clone())
            .await
            .unwrap();

        let refused = FakeLifecycle::refusing();
        let refreshed = EventRegistration {
            config: cron_config("0 10 * * *"),
            ..previous.clone()
        };
        let error = manager
            .register_event(&refused, refreshed)
            .await
            .unwrap_err();
        assert!(refused.stopped().is_empty());
        let kept = manager.get_registration("event").unwrap().unwrap();
        assert!(kept.same_trigger(&previous));
        assert_eq!(
            manager
                .trigger_state(&event, "app", false, Some(&account()))
                .unwrap(),
            LocalTriggerState::Failed {
                reason: format!("{error:#}")
            }
        );

        manager
            .register_event(&FakeLifecycle::default(), previous)
            .await
            .unwrap();
        assert_eq!(
            manager
                .trigger_state(&event, "app", false, Some(&account()))
                .unwrap(),
            LocalTriggerState::Ready
        );
    }

    #[tokio::test]
    async fn failed_new_registration_is_rolled_back() {
        let manager = TestManager::new();
        let refused = FakeLifecycle::refusing();
        assert!(
            manager
                .register_event(
                    &refused,
                    cron_registration("app", Some("pat_1"), Some("user-1"))
                )
                .await
                .is_err()
        );
        assert!(manager.get_registration("event").unwrap().is_none());
        assert_eq!(refused.stopped(), ["cron"]);
        let stashed = manager
            .storage
            .stashed_credentials("app", "event")
            .unwrap()
            .expect("credentials of the failed registration are kept");
        assert_eq!(stashed.personal_access_token.as_deref(), Some("pat_1"));
        assert_eq!(
            manager
                .trigger_state(
                    &flow_event("cron", true, EventExecutionMode::Local),
                    "app",
                    false,
                    Some(&account()),
                )
                .unwrap(),
            LocalTriggerState::Unregistered
        );
    }

    #[test]
    fn unchanged_keep_refresh_is_a_no_op() {
        let row = cron_registration("app", Some("pat_1"), Some("user-1"));
        let renamed = EventRegistration {
            name: "Renamed".to_string(),
            updated_at: std::time::UNIX_EPOCH,
            ..row.clone()
        };
        assert!(row.same_trigger(&renamed));

        let changes: [fn(&mut EventRegistration); 9] = [
            |row| row.r#type = "http".to_string(),
            |row| row.config = cron_config("0 10 * * *"),
            |row| row.offline = true,
            |row| row.remote = true,
            |row| row.app_id = "other-app".to_string(),
            |row| row.personal_access_token = Some("pat_2".to_string()),
            |row| row.pat_owner_sub = Some("user-2".to_string()),
            |row| row.hub_origin = Some("https://other-hub.example".to_string()),
            |row| {
                row.oauth_tokens
                    .insert("github".to_string(), OAuthToken::new("access".to_string()));
            },
        ];
        for change in changes {
            let mut changed = row.clone();
            change(&mut changed);
            assert!(!row.same_trigger(&changed), "{changed:?}");
        }
    }

    #[tokio::test]
    async fn type_change_unregisters_the_old_sink() {
        let manager = TestManager::new();
        manager
            .register_event(
                &FakeLifecycle::default(),
                cron_registration("app", Some("pat_1"), Some("user-1")),
            )
            .await
            .unwrap();

        let lifecycle = FakeLifecycle::default();
        let http = EventRegistration {
            r#type: "http".to_string(),
            config: EventConfig::Http(super::super::http::HttpSink {
                path: "/hook".to_string(),
                method: "POST".to_string(),
                auth_token: None,
                sink_execution: None,
            }),
            ..cron_registration("app", Some("pat_1"), Some("user-1"))
        };
        manager
            .replace_registration(&lifecycle, http)
            .await
            .unwrap();
        assert_eq!(lifecycle.stopped(), ["cron"]);
        assert_eq!(lifecycle.started(), ["http"]);
        assert_eq!(
            manager.get_registration("event").unwrap().unwrap().r#type,
            "http"
        );
    }

    #[tokio::test]
    async fn registering_for_another_app_moves_the_trigger() {
        let manager = TestManager::new();
        manager
            .register_event(
                &FakeLifecycle::default(),
                cron_registration("other-app", Some("pat_0"), Some("user-1")),
            )
            .await
            .unwrap();

        let lifecycle = FakeLifecycle::default();
        manager
            .replace_registration(
                &lifecycle,
                cron_registration("app", Some("pat_1"), Some("user-1")),
            )
            .await
            .unwrap();
        assert_eq!(lifecycle.stopped(), ["cron"]);
        assert_eq!(
            manager.get_registration("event").unwrap().unwrap().app_id,
            "app"
        );
        let moved = manager
            .storage
            .stashed_credentials("other-app", "event")
            .unwrap()
            .expect("the other app keeps its credentials");
        assert_eq!(moved.personal_access_token.as_deref(), Some("pat_0"));

        manager
            .decline(&lifecycle, "other-app", "event", None)
            .await
            .unwrap();
        assert_eq!(lifecycle.stopped(), ["cron"]);
        assert_eq!(
            manager.get_registration("event").unwrap().unwrap().app_id,
            "app"
        );
    }

    #[test]
    fn set_offline_corrects_a_legacy_flag() {
        let manager = TestManager::new();
        let legacy = EventRegistration {
            offline: true,
            ..cron_registration("app", None, None)
        };
        manager.storage.save_registration(&legacy).unwrap();
        assert!(manager.is_event_active("event", Some(&account())));

        manager.storage.set_offline("event", false).unwrap();
        let healed = manager.get_registration("event").unwrap().unwrap();
        assert!(!healed.offline);
        assert!(!healed.can_fire(Some(&account())));
        assert!(!manager.is_event_active("event", Some(&account())));
    }

    #[tokio::test]
    async fn rejected_forwarding_declines_the_registration() {
        let manager = TestManager::new();
        manager
            .register_event(&FakeLifecycle::default(), region_registration())
            .await
            .unwrap();

        let lifecycle = FakeLifecycle::default();
        let paused = Refusal {
            reason: "The hub refused the location trigger (409 EVENT_PAUSED): paused".to_string(),
            token_refused: false,
        };
        manager
            .decline(&lifecycle, "app", "event", Some(&paused))
            .await
            .unwrap();
        assert!(manager.get_registration("event").unwrap().is_none());
        assert_eq!(lifecycle.stopped(), ["geolocation"]);
        assert_eq!(
            manager.storage.failure("app", "event").unwrap().as_deref(),
            Some(paused.reason.as_str())
        );
        let stashed = manager
            .storage
            .stashed_credentials("app", "event")
            .unwrap()
            .expect("a refusal keeps a working token");
        assert_eq!(stashed.personal_access_token.as_deref(), Some("pat_1"));
        if APPLE {
            let event = region_event(EventExecutionMode::Remote);
            assert_eq!(
                manager
                    .trigger_state(&event, "app", false, Some(&account()))
                    .unwrap(),
                LocalTriggerState::Failed {
                    reason: paused.reason.clone()
                }
            );
            assert_eq!(
                manager
                    .registration_plan(&event, "app", false, Some(&account()))
                    .unwrap(),
                LocalTriggerPlan {
                    registration: LocalSinkPlan::New,
                    needs_token: false,
                    moves_from_app: None,
                }
            );
        }
    }

    #[tokio::test]
    async fn a_refused_token_is_not_kept_when_the_hub_declines_the_trigger() {
        let manager = TestManager::new();
        manager
            .register_event(&FakeLifecycle::default(), region_registration())
            .await
            .unwrap();
        let revoked = Refusal {
            reason: "The hub no longer accepts this trigger's personal access token".to_string(),
            token_refused: true,
        };
        manager
            .decline(&FakeLifecycle::default(), "app", "event", Some(&revoked))
            .await
            .unwrap();
        assert!(manager.get_registration("event").unwrap().is_none());
        assert!(
            manager
                .storage
                .stashed_credentials("app", "event")
                .unwrap()
                .is_none_or(|stash| stash.personal_access_token.is_none())
        );
        assert_eq!(
            manager.storage.failure("app", "event").unwrap().as_deref(),
            Some(revoked.reason.as_str())
        );
    }

    /// The source of the function that starts with `head`, up to the brace
    /// that closes it at `indent`.
    fn function_source(source: &str, head: &str, indent: &str) -> String {
        let start = source.find(head).unwrap_or_else(|| panic!("no {head}"));
        let close = format!("\n{indent}}}\n");
        let length = source[start..]
            .find(&close)
            .unwrap_or_else(|| panic!("{head} never closes"));
        source[start..start + length].to_string()
    }

    #[test]
    fn a_location_trigger_is_cleared_with_the_hub_before_the_save_locks_the_manager() {
        let save = function_source(
            include_str!("../functions/flow/event.rs"),
            "async fn update_local_trigger(",
            "",
        );
        let signed_in = save.find("TriggerSignIn::resolve(").unwrap();
        assert!(signed_in < save.find(".lock()").unwrap(), "{save}");

        let source = include_str!("manager.rs");
        let resolve = function_source(source, "    pub async fn resolve(", "    ");
        assert!(resolve.contains(".location_clearance("), "{resolve}");
        let clearance = function_source(source, "    async fn location_clearance(", "    ");
        let locked = clearance.find(".lock()").unwrap();
        let decided = clearance.find(".decide(").unwrap();
        assert!(!clearance[locked..decided].contains(';'), "{clearance}");
        let unlocked = decided + clearance[decided..].find(';').unwrap();
        let cleared = clearance.find("GeofenceClearance::resolve(").unwrap();
        assert!(unlocked < cleared, "{clearance}");

        let geolocation = include_str!("geolocation.rs");
        for registering in [
            "    pub(crate) async fn register_cleared(",
            "    async fn watch(",
        ] {
            let body = function_source(geolocation, registering, "    ");
            assert!(!body.contains("authorize("), "{body}");
        }
    }

    #[test]
    fn legacy_tables_gain_the_token_owner_columns() {
        let path = std::env::temp_dir().join(format!(
            "flow-like-legacy-sinks-{}.sqlite",
            flow_like_types::create_id()
        ));
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE event_registrations (
                    event_id TEXT PRIMARY KEY, name TEXT NOT NULL, type TEXT NOT NULL,
                    updated_at INTEGER NOT NULL, created_at INTEGER NOT NULL, config TEXT NOT NULL,
                    offline INTEGER NOT NULL, app_id TEXT NOT NULL, default_payload TEXT,
                    personal_access_token TEXT, oauth_tokens TEXT
                );
                CREATE TABLE event_credential_stash (
                    event_id TEXT PRIMARY KEY, personal_access_token TEXT, oauth_tokens TEXT
                );
                INSERT INTO event_registrations VALUES
                    ('event', 'Event', 'cron', 1, 1, '{\"expression\":\"0 9 * * *\"}', 0, 'app', NULL, 'pat_legacy', NULL);
                INSERT INTO event_credential_stash VALUES ('stashed', 'pat_stashed', NULL);",
            )
            .unwrap();
        }

        let manager = EventSinkManager::new(path.to_str().unwrap()).unwrap();
        let legacy = manager.get_registration("event").unwrap().unwrap();
        assert_eq!(legacy.personal_access_token.as_deref(), Some("pat_legacy"));
        assert!(legacy.pat_owner_sub.is_none() && legacy.hub_origin.is_none());
        assert!(legacy.profile_binding.is_none());
        assert!(!legacy.remote);
        assert!(!legacy.can_fire(Some(&account())));
        assert!(!manager.is_event_active("event", Some(&account())));
        assert!(!legacy.owns_token(Some(&account())));
        let stashed: i64 = manager
            .storage
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM event_credential_stash", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(stashed, 0, "stashes that name no app are dropped");

        drop(manager);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn saved_trigger_profile_survives_restart_without_adopting_another_profile() {
        let path = std::env::temp_dir().join(format!(
            "flow-like-profile-sinks-{}.sqlite",
            flow_like_types::create_id()
        ));
        let registration = cron_registration("app", Some("pat_1"), Some("user-1"));
        {
            let manager = EventSinkManager::new(path.to_str().unwrap()).unwrap();
            manager.storage.save_registration(&registration).unwrap();
            manager.storage.stash_credentials(&registration).unwrap();
        }
        let manager = EventSinkManager::new(path.to_str().unwrap()).unwrap();
        let restored = manager.get_registration("event").unwrap().unwrap();
        assert_eq!(restored.profile_binding, registration.profile_binding);
        assert_eq!(restored.token_owner(), Some(account()));
        assert!(restored.can_fire(Some(&account())));
        let stash = manager
            .storage
            .stashed_credentials("app", "event")
            .unwrap()
            .unwrap();
        assert_eq!(stash.profile_binding, registration.profile_binding);
        for foreign in [
            TriggerAccount {
                profile_id: "profile-2".into(),
                ..account()
            },
            TriggerAccount {
                issuer: "https://other-issuer.example".into(),
                ..account()
            },
        ] {
            assert!(!restored.can_fire(Some(&foreign)));
            assert!(!stash.owned_by(Some(&foreign)));
        }
        let legacy = EventRegistration {
            offline: true,
            profile_binding: None,
            ..restored
        };
        assert!(!legacy.can_fire(None));
        drop(manager);
        let _ = std::fs::remove_file(path);
    }

    fn other_hub() -> TriggerAccount {
        TriggerAccount {
            hub_origin: "https://other-hub.example".to_string(),
            ..account()
        }
    }

    #[test]
    fn only_offline_triggers_and_tokens_of_the_signed_in_account_fire() {
        let (signed_in, other_account, other_hub) = (account(), user("user-2"), other_hub());
        let offline = EventRegistration {
            offline: true,
            ..cron_registration("app", None, None)
        };
        for account in [Some(&signed_in), Some(&other_account), None] {
            assert!(offline.can_fire(account), "offline as {account:?}");
        }

        let own = cron_registration("app", Some("pat_1"), Some("user-1"));
        assert!(own.can_fire(Some(&signed_in)));
        assert!(!own.can_fire(Some(&other_account)), "another account");
        assert!(!own.can_fire(Some(&other_hub)), "another hub");
        assert!(!own.can_fire(None), "nobody signed in");

        for unverified in [
            cron_registration("app", Some("pat_legacy"), None),
            cron_registration("app", Some("  "), Some("user-1")),
            cron_registration("app", None, None),
        ] {
            assert!(!unverified.can_fire(Some(&signed_in)), "{unverified:?}");
        }
    }

    #[test]
    fn a_run_signs_in_only_as_the_signed_in_owner_of_its_token_on_its_hub() {
        let owner = account();
        let starts = |token: Option<&str>,
                      owner: Option<&TriggerAccount>,
                      account: Option<&TriggerAccount>| {
            super::super::ensure_signed_in_owner("event", token, owner, account)
                .map_err(|error| error.to_string())
        };
        assert!(starts(Some("pat_1"), Some(&owner), Some(&account())).is_ok());

        let refusal = |account: Option<&TriggerAccount>| {
            starts(Some("pat_1"), Some(&owner), account).unwrap_err()
        };
        let elsewhere = refusal(Some(&other_hub()));
        assert!(
            elsewhere.contains("verified with https://hub.example")
                && elsewhere.contains("uses https://other-hub.example"),
            "{elsewhere}"
        );
        let foreign = refusal(Some(&user("user-2")));
        assert!(foreign.contains("another account"), "{foreign}");
        let signed_out = refusal(None);
        assert!(signed_out.contains("nobody is signed in"), "{signed_out}");

        for (token, owner) in [
            (Some("pat_1"), None),
            (Some("  "), Some(&owner)),
            (None, Some(&owner)),
        ] {
            let unverified = starts(token, owner, Some(&account())).unwrap_err();
            assert!(
                unverified.starts_with("The trigger of event event did not start a run")
                    && unverified.contains("no personal access token verified"),
                "{unverified}"
            );
        }
    }

    #[test]
    fn a_run_starts_exactly_when_its_online_trigger_can_fire() {
        let rows = [
            cron_registration("app", Some("pat_1"), Some("user-1")),
            cron_registration("app", Some("pat_2"), Some("user-2")),
            cron_registration("app", Some("pat_legacy"), None),
            cron_registration("app", Some("  "), Some("user-1")),
            cron_registration("app", None, None),
            EventRegistration {
                hub_origin: Some("https://other-hub.example".to_string()),
                ..cron_registration("app", Some("pat_3"), Some("user-1"))
            },
        ];
        let accounts = [
            Some(account()),
            Some(user("user-2")),
            Some(other_hub()),
            None,
        ];
        for row in &rows {
            for signed_in in &accounts {
                let starts = super::super::ensure_signed_in_owner(
                    &row.event_id,
                    row.personal_access_token.as_deref(),
                    row.token_owner().as_ref(),
                    signed_in.as_ref(),
                )
                .is_ok();
                assert_eq!(
                    starts,
                    row.can_fire(signed_in.as_ref()),
                    "{row:?} as {signed_in:?}"
                );
            }
        }
        assert_eq!(rows[0].token_owner(), Some(account()));
        assert_eq!(rows[2].token_owner(), None);
        assert_eq!(rows[3].token_owner(), None);
    }

    #[test]
    fn a_trigger_whose_keep_refresh_lacks_the_new_accounts_token_does_not_fire_as_it() {
        let manager = TestManager::new();
        let previous = cron_trigger(Some("pat_1"), Some("user-1"));
        manager.storage.save_registration(&previous).unwrap();

        let next_account = user("user-2");
        let as_next_account = TriggerSignIn {
            offline: false,
            profile_binding: next_account.profile_binding(),
            account: Some(next_account.clone()),
            given: None,
            geofence: None,
        };
        let (event, _) = cron_event();
        let error = manager
            .decide(
                &event,
                "app",
                cron_config("0 10 * * *"),
                as_next_account,
                None,
                TriggerUpdate::Keep,
            )
            .unwrap_err();
        assert!(error.is::<MissingTriggerToken>(), "{error:#}");

        let kept = manager.get_registration("event").unwrap().unwrap();
        assert!(kept.same_trigger(&previous));
        assert!(!manager.is_event_active("event", Some(&next_account)));
        assert!(!manager.is_event_active("event", None));
        assert_eq!(
            manager
                .trigger_state(&event, "app", false, Some(&next_account))
                .unwrap(),
            LocalTriggerState::NeedsToken
        );
        assert!(
            super::super::ensure_signed_in_owner(
                "event",
                kept.personal_access_token.as_deref(),
                kept.token_owner().as_ref(),
                Some(&next_account),
            )
            .is_err()
        );
        assert!(manager.is_event_active("event", Some(&account())));
    }

    /// Every run a trigger starts checks whom its PAT was verified for
    /// before anything signs in with it.
    #[test]
    fn runs_check_the_token_owner_before_any_hub_request() {
        let bus = function_source(
            include_str!("../event_bus.rs"),
            "    async fn execute_inner(",
            "    ",
        );
        let bound = bus.find("ensure_signed_in_owner(").expect("bus runs bind");
        for request in [
            "request_authorizer(",
            "RunStorage::resolve(",
            "resolve_tokens(",
            "apply_local_run_identity(",
        ] {
            assert!(bound < bus.find(request).unwrap(), "{request}");
        }

        let daemon = function_source(
            include_str!("../functions/flow/run.rs"),
            "pub(crate) async fn execute_daemon_event(",
            "",
        );
        let bound = daemon
            .find("ensure_signed_in_owner(")
            .expect("daemons bind");
        assert!(daemon.contains("app_is_offline(&app_handle, &app_id)"));
        assert!(!daemon.contains("offline: bool"), "{daemon}");
        for request in ["execution_credentials::prepare(", "execute_internal("] {
            assert!(bound < daemon.find(request).unwrap(), "{request}");
        }
        assert!(daemon.contains("execution_hub,"), "{daemon}");

        let source = include_str!("manager.rs");
        let fire = function_source(source, "    pub(crate) fn fire_event_with_request(", "    ");
        assert!(fire.contains(".push_registration(registration,"), "{fire}");
        let rss = function_source(include_str!("rss.rs"), "    fn push(", "    ");
        assert!(rss.contains(".push_registration(registration,"), "{rss}");
    }

    #[test]
    fn keep_refreshes_only_the_same_trigger_and_never_creates_one() {
        let manager = TestManager::new();
        let keep = || decide_cron(&manager, online_save(None), TriggerUpdate::Keep).unwrap();
        let current = cron_trigger(Some("pat_1"), Some("user-1"));

        assert!(matches!(keep(), TriggerDecision::Unchanged));
        manager
            .storage
            .save_registration(&EventRegistration {
                app_id: "other-app".to_string(),
                ..current.clone()
            })
            .unwrap();
        assert!(matches!(keep(), TriggerDecision::Unchanged));

        manager.storage.save_registration(&current).unwrap();
        assert!(matches!(keep(), TriggerDecision::Unchanged));

        manager
            .storage
            .save_registration(&EventRegistration {
                config: cron_config("0 10 * * *"),
                ..current.clone()
            })
            .unwrap();
        let TriggerDecision::Register(refreshed) = keep() else {
            panic!("a changed config refreshes the trigger");
        };
        assert!(refreshed.same_trigger(&current));
    }

    #[test]
    fn keep_declines_a_trigger_of_another_type_or_destination() {
        let manager = TestManager::new();
        let current = cron_trigger(Some("pat_1"), Some("user-1"));
        for converted in [
            EventRegistration {
                r#type: "http".to_string(),
                config: http_config(),
                ..current.clone()
            },
            EventRegistration {
                remote: true,
                ..current
            },
        ] {
            manager.storage.save_registration(&converted).unwrap();
            let decision = decide_cron(&manager, online_save(None), TriggerUpdate::Keep).unwrap();
            assert!(
                matches!(decision, TriggerDecision::Decline),
                "{converted:?}"
            );
        }
    }

    #[tokio::test]
    async fn keep_declines_a_trigger_whose_type_changed() {
        let manager = TestManager::new();
        manager
            .register_event(
                &FakeLifecycle::default(),
                EventRegistration {
                    r#type: "http".to_string(),
                    config: http_config(),
                    ..cron_trigger(Some("pat_1"), Some("user-1"))
                },
            )
            .await
            .unwrap();

        let decision = decide_cron(&manager, online_save(None), TriggerUpdate::Keep).unwrap();
        let lifecycle = FakeLifecycle::default();
        manager
            .apply(&lifecycle, "app", "event", decision)
            .await
            .unwrap();
        assert_eq!(lifecycle.stopped(), ["http"]);
        assert!(lifecycle.started().is_empty());
        assert!(manager.get_registration("event").unwrap().is_none());
        let stashed = manager
            .storage
            .stashed_credentials("app", "event")
            .unwrap()
            .expect("a declined trigger keeps its credentials");
        assert_eq!(stashed.personal_access_token.as_deref(), Some("pat_1"));
        assert_eq!(
            manager
                .trigger_state(&cron_event().0, "app", false, Some(&account()))
                .unwrap(),
            LocalTriggerState::Unregistered
        );
    }

    #[test]
    fn an_online_trigger_without_a_token_of_the_signed_in_account_changes_nothing() {
        let manager = TestManager::new();
        let missing = |sign_in: TriggerSignIn, update: TriggerUpdate| {
            let error = decide_cron(&manager, sign_in, update).unwrap_err();
            assert!(error.is::<MissingTriggerToken>(), "{update:?}: {error:#}");
        };
        missing(online_save(None), TriggerUpdate::Register);

        let legacy = cron_trigger(Some("pat_legacy"), None);
        manager.storage.save_registration(&legacy).unwrap();
        for update in [
            TriggerUpdate::Keep,
            TriggerUpdate::Register,
            TriggerUpdate::Repair,
        ] {
            missing(online_save(None), update);
        }
        missing(
            online_save(Some(("pat_2", "user-2"))),
            TriggerUpdate::Register,
        );
        assert!(
            manager
                .get_registration("event")
                .unwrap()
                .unwrap()
                .same_trigger(&legacy)
        );
    }

    #[test]
    fn a_verified_token_or_an_offline_app_registers_the_trigger() {
        let manager = TestManager::new();
        manager
            .storage
            .save_registration(&cron_trigger(Some("pat_legacy"), None))
            .unwrap();

        let verified = registration_of(decide_cron(
            &manager,
            online_save(Some(("pat_2", "user-1"))),
            TriggerUpdate::Register,
        ));
        assert_eq!(verified.personal_access_token.as_deref(), Some("pat_2"));
        assert!(verified.owns_token(Some(&account())));

        let offline_save = TriggerSignIn {
            offline: true,
            profile_binding: profile_binding(None),
            account: None,
            given: None,
            geofence: None,
        };
        let offline = registration_of(decide_cron(&manager, offline_save, TriggerUpdate::Register));
        assert!(offline.offline && offline.can_fire(None));
    }

    #[test]
    fn repair_signs_in_only_the_same_trigger_of_this_app() {
        let manager = TestManager::new();
        let repair = || {
            decide_cron(
                &manager,
                online_save(Some(("pat_2", "user-1"))),
                TriggerUpdate::Repair,
            )
        };
        let legacy = cron_trigger(Some("pat_legacy"), None);

        assert_eq!(repair().unwrap_err().to_string(), NOTHING_TO_REPAIR);
        for elsewhere in [
            EventRegistration {
                app_id: "other-app".to_string(),
                ..legacy.clone()
            },
            EventRegistration {
                r#type: "http".to_string(),
                config: http_config(),
                ..legacy.clone()
            },
            EventRegistration {
                remote: true,
                ..legacy.clone()
            },
        ] {
            manager.storage.save_registration(&elsewhere).unwrap();
            assert_eq!(
                repair().unwrap_err().to_string(),
                NOTHING_TO_REPAIR,
                "{elsewhere:?}"
            );
        }

        manager.storage.save_registration(&legacy).unwrap();
        let repaired = registration_of(repair());
        assert_eq!(repaired.personal_access_token.as_deref(), Some("pat_2"));
        assert!(repaired.owns_token(Some(&account())));
    }

    #[test]
    fn a_handed_in_pat_is_checked_unless_this_account_already_verified_it() {
        let signed_in = account();
        let held = TriggerCredentials::of(&cron_registration("app", Some("pat_1"), Some("user-1")));
        assert_eq!(
            held.verified_for("pat_1", Some(&signed_in)),
            Some(signed_in.clone())
        );
        assert_eq!(held.verified_for("pat_2", Some(&signed_in)), None);
        assert_eq!(held.verified_for("pat_1", Some(&user("user-2"))), None);
        assert_eq!(held.verified_for("pat_1", None), None);

        let legacy = TriggerCredentials::of(&cron_registration("app", Some("pat_1"), None));
        assert_eq!(legacy.verified_for("pat_1", Some(&signed_in)), None);
    }

    #[test]
    fn only_a_save_that_uses_the_trigger_reads_its_credentials() {
        let manager = TestManager::new();
        let (event, _) = cron_event();
        let held = |update| {
            manager
                .credentials_for_update("app", &event, update)
                .unwrap()
                .map(|held| held.personal_access_token)
        };
        let pat = |value: &str| Some(Some(value.to_string()));
        manager
            .storage
            .stash_credentials(&cron_registration("app", Some("pat_stash"), Some("user-1")))
            .unwrap();
        assert_eq!(held(TriggerUpdate::Register), pat("pat_stash"));
        assert_eq!(held(TriggerUpdate::Keep), None);
        assert_eq!(held(TriggerUpdate::Repair), None);

        manager
            .storage
            .save_registration(&EventRegistration {
                r#type: "http".to_string(),
                config: http_config(),
                ..cron_trigger(Some("pat_http"), Some("user-1"))
            })
            .unwrap();
        assert_eq!(held(TriggerUpdate::Keep), None);
        assert_eq!(held(TriggerUpdate::Register), pat("pat_http"));

        manager
            .storage
            .save_registration(&cron_trigger(Some("pat_1"), Some("user-1")))
            .unwrap();
        for update in [
            TriggerUpdate::Keep,
            TriggerUpdate::Repair,
            TriggerUpdate::Register,
        ] {
            assert_eq!(held(update), pat("pat_1"), "{update:?}");
        }
    }

    #[test]
    fn oauth_grants_stay_with_the_person_the_trigger_signs_in_as() {
        let held = TriggerCredentials {
            oauth_tokens: three_grants(),
            ..TriggerCredentials::of(&cron_registration("app", Some("pat_1"), Some("user-1")))
        };
        assert_eq!(
            providers(&held.oauth_tokens_for(&user("user-1"))),
            ["github", "google"]
        );
        assert!(held.oauth_tokens_for(&user("user-2")).is_empty());
        assert!(
            held.oauth_tokens_for(&TriggerAccount {
                hub_origin: "https://other-hub.example".to_string(),
                ..user("user-1")
            })
            .is_empty()
        );

        let unverified = TriggerCredentials {
            pat_owner_sub: None,
            hub_origin: None,
            ..held
        };
        assert_eq!(
            providers(&unverified.oauth_tokens_for(&user("user-3"))),
            ["dropbox", "github"]
        );
    }

    #[test]
    fn a_save_carries_only_the_grants_of_whom_the_trigger_signs_in_as() {
        let manager = TestManager::new();
        manager
            .storage
            .save_registration(&EventRegistration {
                oauth_tokens: three_grants(),
                ..cron_trigger(Some("pat_1"), Some("user-1"))
            })
            .unwrap();
        let kept = registration_of(decide_cron(
            &manager,
            online_save(None),
            TriggerUpdate::Register,
        ));
        assert_eq!(providers(&kept.oauth_tokens), ["github", "google"]);

        let next_owner = TriggerSignIn {
            offline: false,
            profile_binding: user("user-2").profile_binding(),
            account: Some(user("user-2")),
            given: Some(GivenToken {
                pat: "pat_2".to_string(),
                owner: Some(user("user-2")),
            }),
            geofence: None,
        };
        let (event, config) = cron_event();
        let slack = HashMap::from([("slack".to_string(), grant(Some("user-2")))]);
        let moved = registration_of(manager.decide(
            &event,
            "app",
            config,
            next_owner,
            Some(slack),
            TriggerUpdate::Register,
        ));
        assert_eq!(moved.pat_owner_sub.as_deref(), Some("user-2"));
        assert_eq!(providers(&moved.oauth_tokens), ["slack"]);
    }

    #[tokio::test]
    async fn a_token_the_hub_refuses_for_a_new_trigger_is_dropped() {
        let manager = TestManager::new();
        let error = manager
            .register_event(
                &FakeLifecycle::refusing_token(),
                EventRegistration {
                    oauth_tokens: HashMap::from([("github".to_string(), grant(None))]),
                    ..cron_registration("app", Some("pat_1"), Some("user-1"))
                },
            )
            .await
            .unwrap_err();
        assert!(error.is::<TokenRefused>());
        assert!(manager.get_registration("event").unwrap().is_none());
        let stashed = manager
            .storage
            .stashed_credentials("app", "event")
            .unwrap()
            .expect("the grants of the failed registration are kept");
        assert!(stashed.personal_access_token.is_none() && stashed.pat_owner_sub.is_none());
        assert_eq!(providers(&stashed.oauth_tokens), ["github"]);
        assert!(cron_plan(&manager, false, Some(&account())).needs_token);
        assert!(matches!(
            manager
                .trigger_state(&cron_event().0, "app", false, Some(&account()))
                .unwrap(),
            LocalTriggerState::Failed { .. }
        ));
    }

    #[tokio::test]
    async fn a_refresh_whose_token_the_hub_refuses_keeps_the_trigger_without_it() {
        let manager = TestManager::new();
        let previous = cron_trigger(Some("pat_1"), Some("user-1"));
        manager
            .register_event(&FakeLifecycle::default(), previous.clone())
            .await
            .unwrap();
        manager
            .register_event(
                &FakeLifecycle::refusing_token(),
                EventRegistration {
                    config: cron_config("0 10 * * *"),
                    ..previous.clone()
                },
            )
            .await
            .unwrap_err();
        let kept = manager.get_registration("event").unwrap().unwrap();
        assert!(kept.personal_access_token.is_none() && !kept.can_fire(Some(&account())));
        assert!(super::super::same_json(&kept.config, &previous.config));
        assert_eq!(
            manager
                .trigger_state(&cron_event().0, "app", false, Some(&account()))
                .unwrap(),
            LocalTriggerState::NeedsToken
        );
    }

    #[test]
    fn only_active_local_sink_events_register_locally() {
        let registered = EventSinkManager::local_sink_config(&flow_event(
            "cron",
            true,
            EventExecutionMode::Local,
        ));
        assert!(
            matches!(registered, Ok(EventConfig::Cron(_))),
            "{registered:?}"
        );
        assert_eq!(
            blocker(&flow_event("cron", false, EventExecutionMode::Local)).as_deref(),
            Some("event is inactive")
        );
        assert_eq!(
            blocker(&flow_event("chat", true, EventExecutionMode::Local)).as_deref(),
            Some("event type has no local sink")
        );
    }

    #[test]
    fn remote_only_or_unparsable_sink_configs_never_register_locally() {
        let mut remote_sink = flow_event("cron", true, EventExecutionMode::Local);
        remote_sink.config = br#"{"expression":"0 9 * * *","sink_execution":"REMOTE"}"#.to_vec();
        assert_eq!(
            blocker(&remote_sink).as_deref(),
            Some("sink is configured for remote execution")
        );

        let mut broken = flow_event("cron", true, EventExecutionMode::Local);
        broken.config = Vec::new();
        let reason = blocker(&broken).expect("empty config never registers");
        assert!(
            reason.starts_with("sink config could not be parsed"),
            "{reason}"
        );
    }

    #[test]
    fn device_source_never_registers_a_local_trigger() {
        let mut event = flow_event("cron", true, EventExecutionMode::Local);
        event.config = br#"{"expression":"0 9 * * *"}"#.to_vec();
        event.set_device_source().unwrap();
        assert_eq!(
            blocker(&event).as_deref(),
            Some("event is deployed to another device")
        );
    }

    #[test]
    fn registrations_of_remote_and_device_only_events_are_stale() {
        let mut device = flow_event("cron", true, EventExecutionMode::Local);
        device.config = br#"{"expression":"0 9 * * *"}"#.to_vec();
        device.set_device_source().unwrap();
        assert!(EventSinkManager::registration_is_stale(&device));
        assert!(EventSinkManager::registration_is_stale(&flow_event(
            "cron",
            true,
            EventExecutionMode::Remote
        )));
        assert!(!EventSinkManager::registration_is_stale(&flow_event(
            "cron",
            true,
            EventExecutionMode::Local
        )));
    }

    #[test]
    fn registrations_of_inactive_events_are_not_pruned() {
        assert!(!EventSinkManager::registration_is_stale(&flow_event(
            "cron",
            false,
            EventExecutionMode::Local
        )));
    }

    #[test]
    fn device_only_upsert_skips_the_sink_whatever_was_requested() {
        use crate::functions::flow::event::{SinkRegistration, effective_sink_mode};

        let mut device = flow_event("cron", true, EventExecutionMode::Local);
        device.config = br#"{"expression":"0 9 * * *"}"#.to_vec();
        device.set_device_source().unwrap();
        for requested in [
            SinkRegistration::Register,
            SinkRegistration::Keep,
            SinkRegistration::Skip,
        ] {
            assert_eq!(
                effective_sink_mode(&device, requested),
                SinkRegistration::Skip
            );
        }

        let ordinary = flow_event("cron", true, EventExecutionMode::Local);
        for requested in [
            SinkRegistration::Register,
            SinkRegistration::Keep,
            SinkRegistration::Skip,
        ] {
            assert_eq!(effective_sink_mode(&ordinary, requested), requested);
        }
    }

    #[test]
    fn event_registers_again_once_the_device_marker_is_cleared() {
        let mut event = flow_event("cron", true, EventExecutionMode::Local);
        event.config = br#"{"expression":"0 9 * * *","__flow_like_source":"device"}"#.to_vec();
        assert!(blocker(&event).is_some());

        event.config = br#"{"expression":"0 9 * * *"}"#.to_vec();
        assert!(!event.is_device_source());
        assert!(matches!(
            EventSinkManager::local_sink_config(&event),
            Ok(EventConfig::Cron(_))
        ));
    }

    #[test]
    fn marker_bearing_configs_still_parse() {
        let cron = EventSinkManager::parse_event_config(
            "cron",
            br#"{"expression":"0 9 * * *","__flow_like_source":"device"}"#,
        );
        assert!(matches!(cron, Ok(EventConfig::Cron(_))), "{cron:?}");

        let http = EventSinkManager::parse_event_config(
            "http",
            br#"{"path":"/hook","method":"POST","auth_token":null,"__flow_like_source":"device"}"#,
        );
        assert!(matches!(http, Ok(EventConfig::Http(_))), "{http:?}");
    }

    #[test]
    fn parses_legacy_daemon_registration_config() {
        let config =
            RegistrationStorage::parse_config_json(r#"{"restart_policy":"always"}"#, "daemon")
                .unwrap();

        match config {
            EventConfig::Daemon(sink) => {
                assert_eq!(sink.restart_policy, DaemonRestartPolicy::Always);
            }
            other => panic!("expected daemon config, got {other:?}"),
        }
    }

    #[test]
    fn parses_tagged_daemon_registration_config() {
        let config = RegistrationStorage::parse_config_json(
            r#"{"sink_type":"daemon","restart_policy":"never"}"#,
            "daemon",
        )
        .unwrap();

        match config {
            EventConfig::Daemon(sink) => {
                assert_eq!(sink.restart_policy, DaemonRestartPolicy::Never);
            }
            other => panic!("expected daemon config, got {other:?}"),
        }
    }

    #[test]
    fn parses_flow_daemon_event_config() {
        let db_path = std::env::temp_dir().join(format!(
            "flow-like-daemon-test-{}.sqlite",
            flow_like_types::create_id()
        ));
        let manager = EventSinkManager::new(db_path.to_str().unwrap()).unwrap();

        let config =
            EventSinkManager::parse_event_config("daemon", br#"{"restart_policy":"always"}"#)
                .unwrap();

        match config {
            EventConfig::Daemon(sink) => {
                assert_eq!(sink.restart_policy, DaemonRestartPolicy::Always);
            }
            other => panic!("expected daemon config, got {other:?}"),
        }

        drop(manager);
        let _ = std::fs::remove_file(db_path);
    }
}
