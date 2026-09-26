use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, time::Duration};
use uuid::Uuid;

pub const SCHEMA_VERSION: i64 = 11;
const APPLICATION_ID: i64 = 0x464c5341;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredState {
    Running,
    Stopped,
}

impl DesiredState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
        }
    }

    pub(crate) fn from_db(value: &str) -> Result<Self> {
        match value {
            "running" => Ok(Self::Running),
            "stopped" => Ok(Self::Stopped),
            _ => bail!("invalid desired placement state in management database"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedState {
    Unknown,
    Starting,
    Running,
    Stopping,
    Backoff,
    Stopped,
    Failed,
}

impl ObservedState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Backoff => "backoff",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn from_db(value: &str) -> Result<Self> {
        match value {
            "unknown" => Ok(Self::Unknown),
            "starting" => Ok(Self::Starting),
            "running" => Ok(Self::Running),
            "stopping" => Ok(Self::Stopping),
            "backoff" => Ok(Self::Backoff),
            "stopped" => Ok(Self::Stopped),
            "failed" => Ok(Self::Failed),
            _ => bail!("invalid observed placement state in management database"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PlacementRecord {
    pub id: String,
    pub config: Value,
    pub desired_state: DesiredState,
    pub intent_revision: u64,
    pub observed_state: ObservedState,
    pub config_revision: u64,
    pub applied_revision: Option<u64>,
    pub process_id: Option<u32>,
    pub last_error: Option<String>,
    pub desired_replicas: u8,
    pub running_replicas: u8,
    pub ready_replicas: u8,
    pub replicas: Vec<crate::replicas::ReplicaRecord>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RegistrationRecord {
    pub manifest: flow_like_device_protocol::OnboardingManifest,
    pub manifest_jws: String,
    pub binding_jws: Option<String>,
    #[serde(default)]
    pub attempted_bindings: Vec<String>,
    pub receipt: Option<flow_like_device_protocol::DeviceReceipt>,
    pub last_contact_at: Option<i64>,
    pub connection_status: String,
}

/// Stores management metadata. Callers must keep credentials and private keys out of config.
pub struct StateStore {
    pub(crate) connection: Connection,
    device_id: String,
}

impl StateStore {
    /// CLI commands use separate connections; only one locked supervisor reconciles state.
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_busy_timeout(path, Duration::from_secs(5))
    }

    pub(crate) fn open_with_busy_timeout(path: &Path, busy_timeout: Duration) -> Result<Self> {
        let mut connection = Connection::open(path).context("open management database")?;
        connection.busy_timeout(busy_timeout)?;
        connection.pragma_update(None, "foreign_keys", true)?;

        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        let application_id: i64 =
            transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
        ensure!(
            version <= SCHEMA_VERSION,
            "management database was created by a newer agent"
        );
        ensure!(
            application_id == 0 || application_id == APPLICATION_ID,
            "database belongs to another application"
        );

        if version == 0 {
            let existing_tables: i64 = transaction.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            ensure!(
                existing_tables == 0,
                "refusing to initialize a nonempty unversioned database"
            );
            transaction.execute_batch(
                "CREATE TABLE device_identity (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    device_id TEXT NOT NULL UNIQUE
                 );
                 CREATE TABLE placements (
                    id TEXT PRIMARY KEY NOT NULL,
                    config_json TEXT NOT NULL,
                    desired_state TEXT NOT NULL CHECK (desired_state IN ('running', 'stopped')),
                    intent_revision INTEGER NOT NULL CHECK (intent_revision > 0),
                    observed_state TEXT NOT NULL CHECK (observed_state IN (
                        'unknown', 'starting', 'running', 'stopping', 'backoff', 'stopped', 'failed'
                    )),
                    config_revision INTEGER NOT NULL CHECK (config_revision > 0),
                    applied_revision INTEGER CHECK (
                        applied_revision > 0 AND applied_revision <= config_revision
                    ),
                    process_id INTEGER CHECK (process_id > 0),
                    last_error TEXT
                 );",
            )?;
            transaction.execute(
                "INSERT INTO device_identity (singleton, device_id) VALUES (1, ?1)",
                [Uuid::new_v4().to_string()],
            )?;
            transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        } else {
            ensure!(
                application_id == APPLICATION_ID,
                "management database application ID is missing"
            );
        }

        if version < 2 {
            transaction.execute_batch(
                "CREATE TABLE registration (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    record_json TEXT NOT NULL
                 );",
            )?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        if version < 3 {
            transaction.execute_batch(
                "CREATE TABLE workload_instances (
                instance_id TEXT PRIMARY KEY NOT NULL,
                placement_id TEXT NOT NULL,
                launch_binding_json TEXT NOT NULL,
                lease_expires_at INTEGER NOT NULL,
                retire_pending INTEGER NOT NULL DEFAULT 0 CHECK (retire_pending IN (0,1))
            );",
            )?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }

        if version < 4 {
            transaction.execute_batch(
                "ALTER TABLE placements ADD COLUMN desired_replicas INTEGER NOT NULL DEFAULT 1 CHECK(desired_replicas BETWEEN 1 AND 32);
                 CREATE TABLE placement_replicas (
                    placement_id TEXT NOT NULL REFERENCES placements(id) ON DELETE CASCADE,
                    slot INTEGER NOT NULL CHECK(slot BETWEEN 0 AND 31),
                    config_revision INTEGER NOT NULL, intent_revision INTEGER NOT NULL,
                    observed_state TEXT NOT NULL, applied_revision INTEGER,
                    process_id INTEGER, last_error TEXT,
                    PRIMARY KEY(placement_id,slot)
                 );
                 CREATE TABLE management_policy (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    policy_version INTEGER NOT NULL, policy_digest TEXT NOT NULL,
                    policy_jws TEXT NOT NULL
                 );
                 CREATE TABLE management_operations (
                    operation_id TEXT PRIMARY KEY NOT NULL,
                    request_digest TEXT NOT NULL, principal TEXT NOT NULL,
                    project_id TEXT, placement_id TEXT,
                    accepted_at INTEGER NOT NULL, result_json TEXT NOT NULL
                 );
                 CREATE TABLE telemetry_audiences (
                    scope TEXT PRIMARY KEY NOT NULL, policy_jws TEXT NOT NULL
                 );
                 CREATE TABLE telemetry_records (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    placement_id TEXT, kind TEXT NOT NULL,
                    created_at INTEGER NOT NULL, ciphertext BLOB NOT NULL
                 );
                 CREATE INDEX telemetry_scope ON telemetry_records(placement_id,kind,sequence);
                 CREATE TABLE project_artifact_transfers (
                    transfer_id TEXT PRIMARY KEY NOT NULL, project_id TEXT NOT NULL,
                    principal TEXT NOT NULL, descriptor_json TEXT NOT NULL,
                    manifest_ready INTEGER NOT NULL DEFAULT 0, state TEXT NOT NULL,
                    created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL
                 );
                 CREATE INDEX project_artifact_expiry ON project_artifact_transfers(state,expires_at);
                 CREATE TABLE archive_rosters (
                    scope TEXT NOT NULL, kind TEXT NOT NULL, policy_jws TEXT NOT NULL,
                    telemetry_cursor INTEGER NOT NULL DEFAULT 0,
                    sequence INTEGER NOT NULL DEFAULT 0, manifest_digest TEXT,
                    PRIMARY KEY(scope,kind)
                 );
                 CREATE TABLE archive_outbox (
                    archive_id TEXT PRIMARY KEY NOT NULL, scope TEXT NOT NULL,
                    kind TEXT NOT NULL, sequence INTEGER NOT NULL,
                    bundle_json TEXT NOT NULL, created_at INTEGER NOT NULL,
                    uploaded INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE TABLE secret_operations (
                    operation_id TEXT PRIMARY KEY NOT NULL, placement_id TEXT NOT NULL,
                    expected_revision INTEGER NOT NULL, name TEXT NOT NULL,
                    ciphertext BLOB NOT NULL, created_at INTEGER NOT NULL, state TEXT NOT NULL
                 );
                 CREATE TABLE host_operations (
                    operation_id TEXT PRIMARY KEY NOT NULL,
                    kind TEXT NOT NULL, boot_id TEXT NOT NULL,
                    state TEXT NOT NULL, created_at INTEGER NOT NULL, payload_json TEXT
                 );",
            )?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }

        if version < 5 {
            // Telemetry and encrypted audiences outlive placement removal. Their IDs
            // must never acquire another project, deployment or filesystem spelling.
            transaction.execute_batch(
                "CREATE TABLE placement_identities (
                    id TEXT PRIMARY KEY COLLATE NOCASE NOT NULL,
                    project_id TEXT, deployment_id TEXT,
                    retired INTEGER NOT NULL CHECK(retired IN (0,1))
                 );
                 INSERT INTO placement_identities(id,project_id,deployment_id,retired)
                    SELECT id,json_extract(config_json,'$.project_id'),
                        json_extract(config_json,'$.deployment_id'),0 FROM placements;
                 INSERT INTO placement_identities(id,retired)
                    SELECT DISTINCT placement_id,1 FROM (
                        SELECT placement_id FROM telemetry_records
                        UNION SELECT placement_id FROM management_operations
                        UNION SELECT placement_id FROM secret_operations
                        UNION SELECT placement_id FROM workload_instances
                        UNION SELECT scope FROM archive_rosters WHERE scope!='device'
                        UNION SELECT scope FROM archive_outbox WHERE scope!='device'
                        UNION SELECT scope FROM telemetry_audiences WHERE scope!='device'
                    ) WHERE placement_id IS NOT NULL
                      AND NOT EXISTS(SELECT 1 FROM placements p WHERE p.id=placement_id)
                    ON CONFLICT(id) DO NOTHING;
                 CREATE TRIGGER placement_identity_insert BEFORE INSERT ON placements BEGIN
                    SELECT CASE WHEN EXISTS(
                        SELECT 1 FROM placement_identities i WHERE i.id=NEW.id
                        AND (i.id!=NEW.id COLLATE BINARY OR i.retired=1
                            OR i.project_id IS NOT json_extract(NEW.config_json,'$.project_id')
                            OR i.deployment_id IS NOT json_extract(NEW.config_json,'$.deployment_id'))
                    ) THEN RAISE(ABORT,'Placement identity is reserved or immutable') END;
                 END;
                 CREATE TRIGGER placement_identity_remember AFTER INSERT ON placements BEGIN
                    INSERT INTO placement_identities(id,project_id,deployment_id,retired)
                    VALUES(NEW.id,json_extract(NEW.config_json,'$.project_id'),
                        json_extract(NEW.config_json,'$.deployment_id'),0)
                    ON CONFLICT(id) DO NOTHING;
                 END;
                 CREATE TRIGGER placement_identity_update BEFORE UPDATE ON placements BEGIN
                    SELECT CASE WHEN OLD.id!=NEW.id COLLATE BINARY
                        OR json_extract(OLD.config_json,'$.project_id') IS NOT json_extract(NEW.config_json,'$.project_id')
                        OR json_extract(OLD.config_json,'$.deployment_id') IS NOT json_extract(NEW.config_json,'$.deployment_id')
                    THEN RAISE(ABORT,'Placement identity is immutable') END;
                 END;
                 CREATE TRIGGER placement_identity_retire AFTER DELETE ON placements BEGIN
                    UPDATE placement_identities SET retired=1 WHERE id=OLD.id;
                 END;",
            ).context("Reserve placement identities and reject filesystem aliases")?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }

        if version < 6 {
            crate::operational::migrate(&transaction)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        if version < 7 {
            transaction.execute_batch(crate::rollout::ROLLOUT_SCHEMA)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }

        if version < 8 {
            crate::fleet::migrate(&transaction)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }

        if version < 9 {
            transaction.execute_batch(crate::certificates::SCHEMA)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }

        if version < 10 {
            transaction.execute_batch(crate::certificate_requests::SCHEMA)?;
            transaction.execute_batch(crate::certificate_issuers::SCHEMA)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        if version < 11 {
            transaction.execute_batch(crate::acme::SCHEMA)?;
            transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }

        let device_id: String = transaction.query_row(
            "SELECT device_id FROM device_identity WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        Uuid::parse_str(&device_id).context("invalid device identity in management database")?;
        transaction.commit()?;

        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self {
            connection,
            device_id,
        })
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn check_placement_identity(&self, id: &str, config: &Value) -> Result<()> {
        check_placement_identity(&self.connection, id, config)
    }

    pub fn begin_instance(
        &self,
        instance_id: &str,
        placement_id: &str,
        launch_binding: &Value,
        expires_at: i64,
    ) -> Result<()> {
        self.connection.execute("INSERT INTO workload_instances(instance_id,placement_id,launch_binding_json,lease_expires_at) VALUES (?1,?2,?3,?4)", params![instance_id,placement_id,serde_json::to_string(launch_binding)?,expires_at])?;
        Ok(())
    }

    pub fn update_instance_lease(&self, instance_id: &str, expires_at: i64) -> Result<()> {
        ensure!(self.connection.execute("UPDATE workload_instances SET lease_expires_at = MAX(lease_expires_at, ?2) WHERE instance_id = ?1 AND retire_pending = 0", params![instance_id,expires_at])? == 1, "Instance is no longer running");
        Ok(())
    }

    pub fn retire_instance(&self, instance_id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE workload_instances SET retire_pending = 1 WHERE instance_id = ?1",
            [instance_id],
        )?;
        Ok(())
    }

    pub fn retire_previous_instances(&self) -> Result<()> {
        self.connection
            .execute("UPDATE workload_instances SET retire_pending = 1", [])?;
        Ok(())
    }

    pub fn pending_retirements(&self) -> Result<Vec<String>> {
        let mut query = self.connection.prepare("SELECT instance_id FROM workload_instances WHERE retire_pending = 1 ORDER BY lease_expires_at LIMIT 32")?;
        Ok(query
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn has_pending_serving_retirements(
        &self,
        placement_id: &str,
        now: i64,
    ) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM workload_instances WHERE placement_id=?1 AND retire_pending=1 AND lease_expires_at>=?2 AND COALESCE(json_extract(launch_binding_json,'$.purpose'),'workload')!='rollout_validation')",
            params![placement_id, now],
            |row| row.get(0),
        )?)
    }

    pub fn forget_expired_retirements(&self, now: i64) -> Result<()> {
        self.connection.execute(
            "DELETE FROM workload_instances WHERE retire_pending = 1 AND lease_expires_at < ?1",
            [now],
        )?;
        Ok(())
    }

    pub fn forget_instance(&self, instance_id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM workload_instances WHERE instance_id = ?1 AND retire_pending = 1",
            [instance_id],
        )?;
        Ok(())
    }

    pub fn registration(&self) -> Result<Option<RegistrationRecord>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT record_json FROM registration WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).context("Read registration state"))
            .transpose()
    }

    /// Called under the enrollment lock before any redemption request.
    pub fn begin_registration(&mut self, record: &RegistrationRecord) -> Result<()> {
        Uuid::parse_str(&record.manifest.device_id).context("Invalid enrolled device ID")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT record_json FROM registration WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: RegistrationRecord = serde_json::from_str(&existing)?;
            ensure!(
                existing.manifest == record.manifest
                    && existing.manifest_jws == record.manifest_jws,
                "This state directory is already bound to another enrollment"
            );
        } else {
            transaction.execute(
                "INSERT INTO registration VALUES (1, ?1)",
                [serde_json::to_string(record)?],
            )?;
            transaction.execute(
                "UPDATE device_identity SET device_id = ?1 WHERE singleton = 1",
                [&record.manifest.device_id],
            )?;
        }
        transaction.commit()?;
        self.device_id = record.manifest.device_id.clone();
        Ok(())
    }

    pub fn update_registration(&mut self, record: &RegistrationRecord) -> Result<()> {
        let existing = self
            .registration()?
            .context("Device enrollment has not started")?;
        ensure!(
            existing.manifest == record.manifest && existing.manifest_jws == record.manifest_jws,
            "Cannot change the registered device identity"
        );
        self.connection.execute(
            "UPDATE registration SET record_json = ?1 WHERE singleton = 1",
            [serde_json::to_string(record)?],
        )?;
        Ok(())
    }

    /// Reapplying configuration preserves an operator's persisted stop decision.
    pub fn upsert_placement(
        &mut self,
        id: &str,
        config: &Value,
        initial_desired: DesiredState,
    ) -> Result<PlacementRecord> {
        ensure!(!id.trim().is_empty(), "placement ID cannot be empty");
        let config_json = serde_json::to_string(config)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_placement_identity(&transaction, id, config)?;
        let active: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM placement_rollouts WHERE placement_id=?1 AND state IN ('staged','validating','activating','rolling_back'))",
            [id], |row|row.get(0),
        )?;
        ensure!(!active, "A workflow update is active");
        let existing: Option<(String, i64)> = transaction
            .query_row(
                "SELECT config_json, config_revision FROM placements WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        match existing {
            Some((existing_json, revision)) => {
                let existing_config: Value = serde_json::from_str(&existing_json)?;
                if existing_config
                    .get("resource_grant")
                    .is_some_and(|value| !value.is_null())
                    || config
                        .get("resource_grant")
                        .is_some_and(|value| !value.is_null())
                {
                    ensure!(
                        ["id", "project_id", "deployment_id"]
                            .iter()
                            .all(|key| existing_config.get(key) == config.get(key)),
                        "A resource-enabled placement cannot change its project or deployment identity"
                    );
                }
                if existing_config != *config {
                    let next_revision = revision
                        .checked_add(1)
                        .context("placement revision overflow")?;
                    transaction.execute(
                        "UPDATE placements SET config_json = ?2, config_revision = ?3 WHERE id = ?1",
                        params![id, config_json, next_revision],
                    )?;
                }
            }
            None => {
                transaction.execute(
                    "INSERT INTO placements (id, config_json, desired_state, intent_revision, observed_state, config_revision)
                     VALUES (?1, ?2, ?3, 1, 'unknown', 1)",
                    params![id, config_json, initial_desired.as_str()],
                )?;
            }
        }
        transaction.execute(
            "UPDATE placements SET desired_replicas=MIN(desired_replicas,?2) WHERE id=?1",
            params![
                id,
                config
                    .get("max_replicas")
                    .and_then(Value::as_u64)
                    .unwrap_or(1)
            ],
        )?;
        transaction.commit()?;
        self.get_placement(id)?
            .context("placement missing after update")
    }

    pub fn get_placement(&self, id: &str) -> Result<Option<PlacementRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, config_json, desired_state, observed_state, config_revision,
                    applied_revision, process_id, last_error, intent_revision, desired_replicas FROM placements WHERE id = ?1",
        )?;
        let mut rows = statement.query([id])?;
        let mut record = rows.next()?.map(decode_placement).transpose()?;
        if let Some(record) = &mut record {
            self.enrich_replicas(record)?;
        }
        Ok(record)
    }

    pub fn list_placements(&self) -> Result<Vec<PlacementRecord>> {
        let mut statement = self.connection.prepare(
            "SELECT id, config_json, desired_state, observed_state, config_revision,
                    applied_revision, process_id, last_error, intent_revision, desired_replicas FROM placements ORDER BY id",
        )?;
        let mut rows = statement.query([])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            let mut record = decode_placement(row)?;
            self.enrich_replicas(&mut record)?;
            records.push(record);
        }
        Ok(records)
    }

    pub(crate) fn scoped_placements(
        &self,
        scope: &flow_like_device_protocol::ManagementScope,
    ) -> Result<Vec<PlacementRecord>> {
        use flow_like_device_protocol::ManagementScope;
        let (project, placement) = match scope {
            ManagementScope::Device => (None, None),
            ManagementScope::Project { project_id } => (Some(project_id.as_str()), None),
            ManagementScope::Placement {
                project_id,
                placement_id,
            } => (Some(project_id.as_str()), Some(placement_id.as_str())),
        };
        let mut statement = self.connection.prepare(
            "SELECT id, config_json, desired_state, observed_state, config_revision,
                    applied_revision, process_id, last_error, intent_revision, desired_replicas FROM placements
             WHERE (?1 IS NULL OR json_extract(config_json,'$.project_id')=?1 COLLATE BINARY)
               AND (?2 IS NULL OR id=?2 COLLATE BINARY) ORDER BY id LIMIT 1025",
        )?;
        let mut rows = statement.query(params![project, placement])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            ensure!(
                records.len() < 1024,
                "Scoped placement inventory exceeds its complete snapshot limit"
            );
            let mut record = decode_placement(row)?;
            self.enrich_replicas(&mut record)?;
            records.push(record);
        }
        Ok(records)
    }

    pub fn set_desired_state(&mut self, id: &str, state: DesiredState) -> Result<()> {
        self.with_rollout_transaction(|| {
            if state == DesiredState::Stopped {
                self.cancel_rollout(id, crate::enrollment::unix_time()?)?;
            } else {
                self.require_no_active_rollout(id)?;
            }
            let changed = self.connection.execute(
                "UPDATE placements SET desired_state = ?2, intent_revision = intent_revision + 1
             WHERE id = ?1 AND intent_revision < 9223372036854775807",
                params![id, state.as_str()],
            )?;
            ensure!(
                changed == 1,
                "unknown placement or exhausted intent revision: {id}"
            );
            Ok(())
        })
    }

    /// Claim the current intent before spawning; a concurrent stop or removal wins.
    pub fn claim_start(
        &mut self,
        id: &str,
        config_revision: u64,
        intent_revision: u64,
    ) -> Result<bool> {
        let config_revision = i64::try_from(config_revision)?;
        let intent_revision = i64::try_from(intent_revision)?;
        let changed = self.connection.execute(
            "UPDATE placements SET observed_state = 'starting', last_error = NULL
             WHERE id = ?1 AND config_revision = ?2 AND intent_revision = ?3
             AND desired_state = 'running' AND process_id IS NULL
             AND observed_state IN ('unknown', 'stopped', 'backoff', 'failed')",
            params![id, config_revision, intent_revision],
        )?;
        Ok(changed == 1)
    }

    /// Returns false for stale intent or an already stopped placement, without writing.
    pub fn mark_stopped(
        &mut self,
        id: &str,
        config_revision: u64,
        intent_revision: u64,
    ) -> Result<bool> {
        let config_revision = i64::try_from(config_revision)?;
        let intent_revision = i64::try_from(intent_revision)?;
        let changed = self.connection.execute(
            "UPDATE placements SET observed_state = 'stopped', last_error = NULL
             WHERE id = ?1 AND config_revision = ?2 AND intent_revision = ?3
             AND desired_state = 'stopped' AND process_id IS NULL
             AND (observed_state != 'stopped' OR last_error IS NOT NULL)",
            params![id, config_revision, intent_revision],
        )?;
        Ok(changed == 1)
    }

    pub fn record_observed(
        &mut self,
        id: &str,
        state: ObservedState,
        process_id: Option<u32>,
        error: Option<&str>,
        applied_revision: Option<u64>,
    ) -> Result<()> {
        ensure!(process_id != Some(0), "invalid process ID");
        ensure!(
            process_id.is_none()
                || matches!(
                    state,
                    ObservedState::Starting | ObservedState::Running | ObservedState::Stopping
                ),
            "a process ID requires a starting, running or stopping placement"
        );
        ensure!(
            applied_revision.is_none() || state == ObservedState::Running,
            "only a prepared running placement can apply a configuration revision"
        );
        let revision = applied_revision.map(i64::try_from).transpose()?;
        let changed = self.connection.execute(
            "UPDATE placements SET observed_state = ?2, process_id = ?3, last_error = ?4,
                    applied_revision = COALESCE(?5, applied_revision)
             WHERE id = ?1 AND (?5 IS NULL OR config_revision = ?5)",
            params![id, state.as_str(), process_id, error, revision],
        )?;
        ensure!(
            changed == 1,
            "unknown placement or stale configuration: {id}"
        );
        Ok(())
    }

    /// A worker may acknowledge preflight only for the process and intent being supervised.
    pub fn record_prepared(
        &mut self,
        id: &str,
        config_revision: u64,
        intent_revision: u64,
        process_id: u32,
    ) -> Result<bool> {
        ensure!(process_id != 0, "invalid process ID");
        let config_revision = i64::try_from(config_revision)?;
        let intent_revision = i64::try_from(intent_revision)?;
        let changed = self.connection.execute(
            "UPDATE placements SET observed_state = 'running', applied_revision = ?2, last_error = NULL
             WHERE id = ?1 AND config_revision = ?2 AND intent_revision = ?3
             AND process_id = ?4 AND desired_state = 'running' AND observed_state = 'starting'",
            params![id, config_revision, intent_revision, process_id],
        )?;
        Ok(changed == 1)
    }

    /// A persisted PID does not prove that the process still belongs to this placement.
    pub fn reset_observed(&mut self) -> Result<()> {
        self.connection.execute(
            "UPDATE placements SET observed_state = 'unknown', process_id = NULL",
            [],
        )?;
        self.connection.execute(
            "UPDATE placement_replicas SET observed_state='unknown',process_id=NULL",
            [],
        )?;
        Ok(())
    }

    pub fn remove_placement(&mut self, id: &str) -> Result<()> {
        self.with_rollout_transaction(|| {
        self.require_no_active_rollout(id)?;
        let changed = self.connection.execute(
            "DELETE FROM placements WHERE id = ?1 AND desired_state = 'stopped'
             AND observed_state IN ('stopped', 'failed') AND process_id IS NULL
             AND NOT EXISTS(SELECT 1 FROM placement_replicas r WHERE r.placement_id=placements.id AND r.process_id IS NOT NULL)",
            [id],
        )?;
        ensure!(
            changed == 1,
            "placement must be stopped before removal: {id}"
        );
        Ok(())
        })
    }
}

fn check_placement_identity(connection: &Connection, id: &str, config: &Value) -> Result<()> {
    crate::config::validate_id("placement", id)?;
    ensure!(id != "device", "The device telemetry scope is reserved");
    if let Some(config_id) = config.get("id") {
        ensure!(
            config_id.as_str() == Some(id),
            "Placement ID does not match its configuration"
        );
    }
    let identity: Option<(String, Option<String>, Option<String>, bool)> = connection
        .query_row(
            "SELECT id,project_id,deployment_id,retired FROM placement_identities WHERE id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    if let Some((original, project, deployment, retired)) = identity {
        ensure!(
            original == id
                && !retired
                && project.as_deref() == config.get("project_id").and_then(Value::as_str)
                && deployment.as_deref() == config.get("deployment_id").and_then(Value::as_str),
            "Placement identity is reserved or immutable; create a new placement ID"
        );
    }
    Ok(())
}

fn decode_placement(row: &rusqlite::Row<'_>) -> Result<PlacementRecord> {
    Ok(PlacementRecord {
        id: row.get(0)?,
        config: serde_json::from_str(&row.get::<_, String>(1)?)?,
        desired_state: DesiredState::from_db(&row.get::<_, String>(2)?)?,
        intent_revision: row.get::<_, i64>(8)?.try_into()?,
        observed_state: ObservedState::from_db(&row.get::<_, String>(3)?)?,
        config_revision: row.get::<_, i64>(4)?.try_into()?,
        applied_revision: row
            .get::<_, Option<i64>>(5)?
            .map(u64::try_from)
            .transpose()?,
        process_id: row.get(6)?,
        last_error: row.get(7)?,
        desired_replicas: row.get(9)?,
        running_replicas: 0,
        ready_replicas: 0,
        replicas: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn placement_identity_cannot_change_or_be_reused_after_removal() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("management.sqlite");
        let mut store = StateStore::open(&path)?;
        let config = json!({"id":"API","project_id":"project-a","deployment_id":"deployment-a"});
        store.upsert_placement("API", &config, DesiredState::Stopped)?;
        for field in ["project_id", "deployment_id"] {
            let mut other = config.clone();
            other[field] = json!("other");
            assert!(
                store
                    .upsert_placement("API", &other, DesiredState::Stopped)
                    .is_err()
            );
            // Remote Apply uses SQL inside its existing authorization transaction.
            assert!(
                store
                    .connection
                    .execute(
                        "UPDATE placements SET config_json=?1 WHERE id='API'",
                        [serde_json::to_string(&other)?]
                    )
                    .is_err()
            );
        }
        let alias = json!({"id":"api","project_id":"project-a","deployment_id":"deployment-a"});
        assert!(
            store
                .upsert_placement("api", &alias, DesiredState::Stopped)
                .is_err()
        );
        assert!(store.connection.execute("INSERT INTO placements(id,config_json,desired_state,intent_revision,observed_state,config_revision) VALUES('api',?1,'stopped',1,'unknown',1)", [serde_json::to_string(&alias)?]).is_err());
        store.connection.execute(
            "UPDATE placements SET observed_state='stopped' WHERE id='API'",
            [],
        )?;
        store.remove_placement("API")?;
        drop(store);
        let mut store = StateStore::open(&path)?;
        assert!(
            store
                .upsert_placement("API", &config, DesiredState::Stopped)
                .is_err()
        );
        assert!(
            store
                .upsert_placement("api", &alias, DesiredState::Stopped)
                .is_err()
        );
        let replacement =
            json!({"id":"new-api","project_id":"project-b","deployment_id":"deployment-b"});
        assert!(
            store
                .upsert_placement("new-api", &replacement, DesiredState::Stopped)
                .is_ok()
        );
        Ok(())
    }

    #[test]
    fn identity_migration_preserves_active_bindings_and_reserves_orphan_history() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("management.sqlite");
        let mut store = StateStore::open(&path)?;
        let config = json!({"id":"active","project_id":"project","deployment_id":"deployment"});
        store.upsert_placement("active", &config, DesiredState::Stopped)?;
        store.connection.execute("INSERT INTO telemetry_records(placement_id,kind,created_at,ciphertext) VALUES('removed','log',0,x'')", [])?;
        store.connection.execute_batch("DROP TABLE certificate_acme; DROP TABLE certificate_issuers; DROP TABLE certificate_requests; DROP TABLE device_certificates; DROP TABLE certificate_inventory; DROP TABLE fleet_published_streams; DROP TABLE fleet_publication; DROP TABLE placement_rollouts; DROP TRIGGER operational_command_insert; DROP TRIGGER operational_command_update; DROP TRIGGER operational_replica_insert; DROP TRIGGER operational_replica_update; DROP TRIGGER operational_replica_delete; DROP TRIGGER operational_outbox_bound; DROP TABLE operational_message_scopes; DROP TABLE operational_outbox; DROP TABLE operational_coverage; DROP TABLE usage_processes; DROP TABLE usage_totals; DROP TRIGGER placement_identity_insert; DROP TRIGGER placement_identity_remember; DROP TRIGGER placement_identity_update; DROP TRIGGER placement_identity_retire; DROP TABLE placement_identities; PRAGMA user_version=4;")?;
        drop(store);
        let mut store = StateStore::open(&path)?;
        assert!(
            store
                .upsert_placement("active", &config, DesiredState::Stopped)
                .is_ok()
        );
        let other = json!({"id":"removed","project_id":"other","deployment_id":"other"});
        assert!(
            store
                .upsert_placement("removed", &other, DesiredState::Stopped)
                .is_err()
        );
        assert_eq!(
            store.connection.query_row(
                "SELECT COUNT(*) FROM telemetry_records WHERE placement_id='removed'",
                [],
                |row| row.get::<_, i64>(0)
            )?,
            1
        );
        Ok(())
    }

    #[test]
    fn instance_retirement_preserves_launch_metadata_and_conservative_lease() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("management.sqlite");
        let binding = json!({"project_id":"project","deployment_id":"deployment",
            "grant_id":"grant","authz_version":3,"workload_key":{"x":"public-key"}});
        let store = StateStore::open(&path)?;
        store.begin_instance("one", "placement", &binding, 100)?;
        store.update_instance_lease("one", 200)?;
        store.update_instance_lease("one", 150)?;
        store.forget_instance("one")?;
        assert!(store.pending_retirements()?.is_empty());
        drop(store);

        let store = StateStore::open(&path)?;
        store.retire_previous_instances()?;
        assert_eq!(store.pending_retirements()?, ["one"]);
        assert!(store.update_instance_lease("one", 300).is_err());
        let (stored_binding, expiry): (String, i64) = store.connection.query_row(
            "SELECT launch_binding_json,lease_expires_at FROM workload_instances WHERE instance_id='one'",
            [], |row| Ok((row.get(0)?,row.get(1)?)))?;
        assert_eq!(serde_json::from_str::<Value>(&stored_binding)?, binding);
        assert_eq!(expiry, 200);
        store.forget_expired_retirements(200)?;
        assert_eq!(store.pending_retirements()?, ["one"]);
        store.forget_expired_retirements(201)?;
        assert!(store.pending_retirements()?.is_empty());
        Ok(())
    }

    #[test]
    fn serving_retirement_gate_survives_restart_and_excludes_validation() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("management.sqlite");
        let store = StateStore::open(&path)?;
        store.begin_instance("legacy", "placement", &json!({"config_revision":1}), 200)?;
        store.begin_instance("current", "placement", &json!({"purpose":"workload"}), 300)?;
        store.begin_instance(
            "validation",
            "placement",
            &json!({"purpose":"rollout_validation"}),
            400,
        )?;
        store.begin_instance(
            "other",
            "other-placement",
            &json!({"purpose":"workload"}),
            400,
        )?;
        store.retire_instance("validation")?;
        store.retire_instance("other")?;
        assert!(!store.has_pending_serving_retirements("placement", 100)?);
        store.retire_instance("legacy")?;
        assert!(store.has_pending_serving_retirements("placement", 200)?);
        assert!(!store.has_pending_serving_retirements("placement", 201)?);
        store.retire_instance("current")?;
        drop(store);
        let store = StateStore::open(&path)?;
        assert!(store.has_pending_serving_retirements("placement", 201)?);
        store.forget_instance("current")?;
        assert!(!store.has_pending_serving_retirements("placement", 201)?);
        assert!(
            store
                .pending_retirements()?
                .contains(&"validation".to_owned())
        );
        Ok(())
    }

    #[test]
    fn resource_placement_updates_preserve_project_and_deployment_binding() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        let config = json!({"id":"placement","project_id":"project","deployment_id":"deployment",
            "resource_grant":{"grant_id":"grant","authz_version":1,"billing_grant_id":"billing","billing_authz_version":1}});
        store.upsert_placement("placement", &config, DesiredState::Running)?;
        for field in ["id", "project_id", "deployment_id"] {
            let mut changed = config.clone();
            changed[field] = json!("other");
            assert!(
                store
                    .upsert_placement("placement", &changed, DesiredState::Running)
                    .is_err()
            );
        }
        let mut newer = config.clone();
        newer["resource_grant"]["authz_version"] = json!(2);
        let record = store.upsert_placement("placement", &newer, DesiredState::Running)?;
        assert_eq!(record.config_revision, 2);
        assert_eq!(record.config["project_id"], "project");
        Ok(())
    }

    #[test]
    fn device_identity_and_stopped_placement_survive_reopen() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("management.sqlite");
        let device_id;
        {
            let mut store = StateStore::open(&path)?;
            device_id = store.device_id().to_owned();
            store.upsert_placement("rest", &json!({"project": "one"}), DesiredState::Running)?;
            store.set_desired_state("rest", DesiredState::Stopped)?;
        }
        let mut store = StateStore::open(&path)?;
        assert_eq!(store.device_id(), device_id);
        let placement =
            store.upsert_placement("rest", &json!({"project": "one"}), DesiredState::Running)?;
        assert_eq!(placement.desired_state, DesiredState::Stopped);
        assert_eq!(placement.config_revision, 1);
        assert_eq!(placement.intent_revision, 2);
        Ok(())
    }

    #[test]
    fn repeated_explicit_start_changes_intent_without_changing_configuration() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement("rest", &json!({}), DesiredState::Running)?;
        store.set_desired_state("rest", DesiredState::Running)?;
        let placement = store.upsert_placement("rest", &json!({}), DesiredState::Stopped)?;
        assert_eq!(placement.desired_state, DesiredState::Running);
        assert_eq!(placement.intent_revision, 2);
        assert_eq!(placement.config_revision, 1);
        Ok(())
    }

    #[test]
    fn changed_configuration_keeps_observed_revision_until_applied() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement("rest", &json!({"port": 8080}), DesiredState::Running)?;
        store.record_observed("rest", ObservedState::Running, Some(42), None, Some(1))?;
        let placement =
            store.upsert_placement("rest", &json!({"port": 9090}), DesiredState::Running)?;
        assert_eq!(placement.config_revision, 2);
        assert_eq!(placement.applied_revision, Some(1));
        assert_eq!(placement.observed_state, ObservedState::Running);
        assert!(
            store
                .record_observed("rest", ObservedState::Running, Some(43), None, Some(3))
                .is_err()
        );
        assert_eq!(store.get_placement("rest")?.unwrap().process_id, Some(42));
        Ok(())
    }

    #[test]
    fn reconciliation_does_not_trust_stale_process_ids() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement("rest", &json!({}), DesiredState::Running)?;
        store.record_observed("rest", ObservedState::Running, Some(42), None, Some(1))?;
        store.reset_observed()?;
        let placement = store.get_placement("rest")?.unwrap();
        assert_eq!(placement.observed_state, ObservedState::Unknown);
        assert_eq!(placement.process_id, None);
        assert_eq!(placement.desired_state, DesiredState::Running);
        assert_eq!(placement.applied_revision, Some(1));
        assert!(
            store
                .set_desired_state("missing", DesiredState::Stopped)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn removal_requires_persisted_stop_and_process_exit() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement("rest", &json!({}), DesiredState::Running)?;
        store.record_observed("rest", ObservedState::Running, Some(42), None, Some(1))?;
        assert!(store.remove_placement("rest").is_err());
        store.set_desired_state("rest", DesiredState::Stopped)?;
        assert!(store.remove_placement("rest").is_err());
        store.record_observed("rest", ObservedState::Stopped, None, None, None)?;
        store.remove_placement("rest")?;
        assert!(store.get_placement("rest")?.is_none());
        Ok(())
    }

    #[test]
    fn preflight_acknowledgement_rejects_stale_config_intent_and_process() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement("rest", &json!({"port": 8080}), DesiredState::Running)?;
        store.record_observed("rest", ObservedState::Starting, Some(42), None, None)?;
        assert!(!store.record_prepared("rest", 1, 1, 43)?);
        assert_eq!(store.get_placement("rest")?.unwrap().applied_revision, None);

        store.set_desired_state("rest", DesiredState::Stopped)?;
        assert!(!store.record_prepared("rest", 1, 2, 42)?);
        store.set_desired_state("rest", DesiredState::Running)?;
        assert!(!store.record_prepared("rest", 1, 1, 42)?);

        store.upsert_placement("rest", &json!({"port": 9090}), DesiredState::Running)?;
        assert!(!store.record_prepared("rest", 1, 3, 42)?);
        store.record_observed("rest", ObservedState::Starting, Some(43), None, None)?;
        assert!(!store.record_prepared("rest", 2, 3, 42)?);
        assert!(store.record_prepared("rest", 2, 3, 43)?);
        let placement = store.get_placement("rest")?.unwrap();
        assert_eq!(placement.applied_revision, Some(2));
        assert_eq!(placement.observed_state, ObservedState::Running);
        assert!(!store.record_prepared("rest", 2, 3, 43)?);
        Ok(())
    }

    #[test]
    fn ordinary_observation_cannot_apply_a_stale_or_unprepared_revision() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement("rest", &json!({"port": 8080}), DesiredState::Running)?;
        assert!(
            store
                .record_observed("rest", ObservedState::Starting, Some(42), None, Some(1))
                .is_err()
        );
        store.upsert_placement("rest", &json!({"port": 9090}), DesiredState::Running)?;
        assert!(
            store
                .record_observed("rest", ObservedState::Running, Some(42), None, Some(1))
                .is_err()
        );
        let placement = store.get_placement("rest")?.unwrap();
        assert_eq!(placement.applied_revision, None);
        assert_eq!(placement.observed_state, ObservedState::Unknown);
        Ok(())
    }

    #[test]
    fn start_claim_cannot_race_stop_removal_or_another_start() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.upsert_placement("rest", &json!({}), DesiredState::Running)?;
        let initial = store.get_placement("rest")?.unwrap();
        store.set_desired_state("rest", DesiredState::Stopped)?;
        assert!(!store.claim_start("rest", initial.config_revision, initial.intent_revision)?);
        assert!(!store.mark_stopped("rest", 1, 1)?);
        assert!(store.mark_stopped("rest", 1, 2)?);
        assert!(!store.mark_stopped("rest", 1, 2)?);
        store.remove_placement("rest")?;
        assert!(!store.claim_start("rest", 1, 1)?);
        assert!(!store.mark_stopped("rest", 1, 2)?);

        store.upsert_placement("mcp", &json!({}), DesiredState::Running)?;
        assert!(store.claim_start("mcp", 1, 1)?);
        assert!(!store.claim_start("mcp", 1, 1)?);
        store.record_observed("mcp", ObservedState::Starting, Some(42), None, None)?;
        store.set_desired_state("mcp", DesiredState::Stopped)?;
        assert!(!store.mark_stopped("mcp", 1, 2)?);
        assert_eq!(store.get_placement("mcp")?.unwrap().process_id, Some(42));
        Ok(())
    }

    #[test]
    fn enrollment_migration_preserves_existing_local_device_and_placements() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("management.sqlite");
        let mut store = StateStore::open(&path)?;
        let device_id = store.device_id().to_owned();
        store.upsert_placement("rest", &json!({"revision":"one"}), DesiredState::Stopped)?;
        store.connection.execute_batch(
            "DROP TABLE certificate_acme; DROP TABLE certificate_issuers; DROP TABLE certificate_requests; DROP TABLE device_certificates; DROP TABLE certificate_inventory; DROP TABLE fleet_published_streams; DROP TABLE fleet_publication; DROP TABLE placement_rollouts; DROP TRIGGER operational_command_insert; DROP TRIGGER operational_command_update; DROP TRIGGER operational_replica_insert; DROP TRIGGER operational_replica_update; DROP TRIGGER operational_replica_delete; DROP TRIGGER operational_outbox_bound; DROP TABLE operational_message_scopes; DROP TABLE operational_outbox; DROP TABLE operational_coverage; DROP TABLE usage_processes; DROP TABLE usage_totals; DROP TRIGGER placement_identity_insert; DROP TRIGGER placement_identity_remember; DROP TRIGGER placement_identity_update; DROP TRIGGER placement_identity_retire; DROP TABLE placement_identities; DROP TABLE placement_replicas; ALTER TABLE placements DROP COLUMN desired_replicas; DROP TABLE registration; DROP TABLE workload_instances; DROP TABLE management_policy; DROP TABLE management_operations; DROP TABLE telemetry_audiences; DROP TABLE telemetry_records; DROP TABLE host_operations; DROP TABLE secret_operations; DROP TABLE archive_rosters; DROP TABLE archive_outbox; DROP TABLE project_artifact_transfers; PRAGMA user_version = 1;",
        )?;
        drop(store);
        let store = StateStore::open(&path)?;
        assert_eq!(store.device_id(), device_id);
        assert_eq!(
            store.get_placement("rest")?.unwrap().desired_state,
            DesiredState::Stopped
        );
        assert!(store.registration()?.is_none());
        assert_eq!(
            store
                .connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))?,
            SCHEMA_VERSION
        );
        Ok(())
    }

    #[test]
    fn newer_schema_and_foreign_database_are_rejected_without_reinitializing() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("management.sqlite");
        let store = StateStore::open(&path)?;
        let device_id = store.device_id().to_owned();
        store
            .connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)?;
        drop(store);
        assert!(StateStore::open(&path).is_err());
        let connection = Connection::open(&path)?;
        let persisted_id: String =
            connection.query_row("SELECT device_id FROM device_identity", [], |row| {
                row.get(0)
            })?;
        assert_eq!(persisted_id, device_id);

        let foreign_path = directory.path().join("project.sqlite");
        let foreign = Connection::open(&foreign_path)?;
        foreign.execute("CREATE TABLE project_data (id INTEGER)", [])?;
        drop(foreign);
        assert!(StateStore::open(&foreign_path).is_err());
        Ok(())
    }
}
