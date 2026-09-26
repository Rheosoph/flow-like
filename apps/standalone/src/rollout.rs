use crate::{
    config::PlacementConfig,
    state::{DesiredState, ObservedState, PlacementRecord, StateStore},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(crate) const ROLLOUT_SCHEMA: &str = "
CREATE TABLE placement_rollouts (
    rollout_id TEXT PRIMARY KEY NOT NULL,
    placement_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    previous_config_json TEXT NOT NULL,
    candidate_config_json TEXT NOT NULL,
    base_revision INTEGER NOT NULL CHECK(base_revision > 0),
    base_intent INTEGER NOT NULL CHECK(base_intent > 0),
    previous_replicas INTEGER NOT NULL CHECK(previous_replicas BETWEEN 1 AND 32),
    candidate_replicas INTEGER NOT NULL CHECK(candidate_replicas BETWEEN 1 AND 32),
    state TEXT NOT NULL CHECK(state IN ('staged','validating','activating','rolling_back','healthy','rolled_back','failed','cancelled')),
    stabilization_seconds INTEGER NOT NULL,
    deadline_seconds INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deadline_at INTEGER,
    stable_since INTEGER,
    cohort TEXT,
    active_revision INTEGER,
    active_intent INTEGER,
    failure_code TEXT
);
CREATE UNIQUE INDEX placement_rollout_active ON placement_rollouts(placement_id)
    WHERE state IN ('staged','validating','activating','rolling_back');
CREATE INDEX placement_rollout_history ON placement_rollouts(placement_id,created_at);
";

const ACTIVE: &str = "('staged','validating','activating','rolling_back')";
const COLUMNS: &str = "rollout_id,placement_id,project_id,previous_config_json,candidate_config_json,base_revision,base_intent,previous_replicas,candidate_replicas,state,stabilization_seconds,deadline_seconds,created_at,updated_at,deadline_at,stable_since,cohort,active_revision,active_intent,failure_code";

#[derive(Clone, Debug)]
pub(crate) struct RolloutRecord {
    pub rollout_id: String,
    pub placement_id: String,
    pub project_id: String,
    pub previous_config: PlacementConfig,
    pub candidate_config: PlacementConfig,
    pub base_revision: u64,
    pub base_intent: u64,
    pub previous_replicas: u8,
    pub candidate_replicas: u8,
    pub state: String,
    pub stabilization_seconds: u32,
    pub deadline_seconds: u32,
    pub created_at: i64,
    pub updated_at: i64,
    pub deadline_at: Option<i64>,
    pub stable_since: Option<i64>,
    pub cohort: Option<String>,
    pub active_revision: Option<u64>,
    pub active_intent: Option<u64>,
    pub failure_code: Option<String>,
}

impl RolloutRecord {
    pub fn status(&self) -> Value {
        json!({
            "rollout_id": self.rollout_id,
            "placement_id": self.placement_id,
            "project_id": self.project_id,
            "state": self.state,
            "base_revision": self.base_revision,
            "active_revision": self.active_revision,
            "active_intent": self.active_intent,
            "previous_replicas": self.previous_replicas,
            "candidate_replicas": self.candidate_replicas,
            "stabilization_seconds": self.stabilization_seconds,
            "deadline_seconds": self.deadline_seconds,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
            "deadline_at": self.deadline_at,
            "stable_since": self.stable_since,
            "failure_code": self.failure_code,
        })
    }

    fn base_matches(&self, placement: &PlacementRecord) -> bool {
        placement.config_revision == self.base_revision
            && placement.intent_revision == self.base_intent
            && placement.desired_replicas == self.previous_replicas
            && placement.desired_state == DesiredState::Running
    }

    fn active_matches(&self, placement: &PlacementRecord) -> bool {
        Some(placement.config_revision) == self.active_revision
            && Some(placement.intent_revision) == self.active_intent
            && placement.desired_replicas
                == if self.state == "rolling_back" {
                    self.previous_replicas
                } else {
                    self.candidate_replicas
                }
            && placement.desired_state == DesiredState::Running
    }
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<RolloutRecord> {
    let config = |index| -> rusqlite::Result<PlacementConfig> {
        let text: String = row.get(index)?;
        serde_json::from_str(&text).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    };
    Ok(RolloutRecord {
        rollout_id: row.get(0)?,
        placement_id: row.get(1)?,
        project_id: row.get(2)?,
        previous_config: config(3)?,
        candidate_config: config(4)?,
        base_revision: row.get(5)?,
        base_intent: row.get(6)?,
        previous_replicas: row.get(7)?,
        candidate_replicas: row.get(8)?,
        state: row.get(9)?,
        stabilization_seconds: row.get(10)?,
        deadline_seconds: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        deadline_at: row.get(14)?,
        stable_since: row.get(15)?,
        cohort: row.get(16)?,
        active_revision: row.get(17)?,
        active_intent: row.get(18)?,
        failure_code: row.get(19)?,
    })
}

fn secret_names(config: &PlacementConfig) -> BTreeSet<&str> {
    config
        .secret_overrides
        .values()
        .map(String::as_str)
        .chain(
            config
                .hosting
                .iter()
                .map(|hosting| hosting.auth_secret.as_str()),
        )
        .collect()
}

impl StateStore {
    /// Management commands already hold a write transaction; supervisor calls need their own.
    pub(crate) fn with_rollout_transaction<T>(&self, run: impl FnOnce() -> Result<T>) -> Result<T> {
        if !self.connection.is_autocommit() {
            return run();
        }
        self.connection.execute_batch("BEGIN IMMEDIATE")?;
        let result = run();
        match result {
            Ok(value) => match self.connection.execute_batch("COMMIT") {
                Ok(()) => Ok(value),
                Err(error) => {
                    let _ = self.connection.execute_batch("ROLLBACK");
                    Err(error.into())
                }
            },
            Err(error) => {
                let _ = self.connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub(crate) fn rollout(&self, id: &str) -> Result<Option<RolloutRecord>> {
        Ok(self
            .connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM placement_rollouts WHERE rollout_id=?1"),
                [id],
                decode,
            )
            .optional()?)
    }

    pub(crate) fn latest_rollout(&self, placement: &str) -> Result<Option<RolloutRecord>> {
        Ok(self.connection.query_row(
            &format!("SELECT {COLUMNS} FROM placement_rollouts WHERE placement_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 1"),
            [placement],
            decode,
        ).optional()?)
    }

    pub(crate) fn require_no_active_rollout(&self, placement: &str) -> Result<()> {
        let active: bool = self.connection.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM placement_rollouts WHERE placement_id=?1 AND state IN {ACTIVE})"),
            [placement], |row| row.get(0),
        )?;
        ensure!(!active, "Placement has an active rollout");
        Ok(())
    }

    pub(crate) fn has_active_rollouts(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM placement_rollouts WHERE state IN {ACTIVE})"),
            [],
            |row| row.get(0),
        )?)
    }

    fn require_no_pending_rollout_host_operation(&self) -> Result<()> {
        let pending: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM host_operations WHERE state IN ('pending','staging','draining','requesting','requested','unknown'))",
            [], |row| row.get(0),
        )?;
        ensure!(!pending, "Host operation is pending");
        Ok(())
    }

    pub(crate) fn stage_rollout(
        &self,
        id: &str,
        candidate: &PlacementConfig,
        expected_revision: u64,
        stabilization_seconds: u32,
        deadline_seconds: u32,
        now: i64,
    ) -> Result<RolloutRecord> {
        crate::config::validate_id("rollout", id)?;
        candidate.validate()?;
        ensure!(now >= 0, "Invalid rollout timestamp");
        ensure!(
            (2..=60).contains(&stabilization_seconds)
                && (10..=600).contains(&deadline_seconds)
                && deadline_seconds > stabilization_seconds + 5,
            "Invalid rollout health limits"
        );
        self.with_rollout_transaction(|| {
            let store = self;
            store.require_no_pending_rollout_host_operation()?;
            store.require_no_active_rollout(&candidate.id)?;
            let previous = store.get_placement(&candidate.id)?.context("Unknown placement")?;
            ensure!(
                previous.config_revision == expected_revision
                    && previous.desired_state == DesiredState::Running,
                "Rollout requires the current running placement revision"
            );
            ensure!(
                previous.config_revision <= (i64::MAX as u64) - 2
                    && previous.intent_revision <= (i64::MAX as u64) - 3,
                "Placement revisions are exhausted"
            );
            let previous_config: PlacementConfig = serde_json::from_value(previous.config)?;
            ensure!(
                candidate.id == previous_config.id
                    && candidate.project_id == previous_config.project_id
                    && candidate.deployment_id == previous_config.deployment_id
                    && candidate.source == previous_config.source,
                "Rollout cannot change placement identity or source"
            );
            // Activation validates both pinned revisions' listener or explicit daemon
            // readiness contracts before changing the running placement.
            let pending: bool = store.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM secret_operations WHERE placement_id=?1 AND state='pending')",
                [&candidate.id], |row| row.get(0),
            )?;
            ensure!(!pending, "Wait for pending secret publication before staging");
            let count: u64 = store.connection.query_row("SELECT COUNT(*) FROM placement_rollouts", [], |row| row.get(0))?;
            ensure!(count < 10_000, "Rollout journal is full");
            store.connection.execute(
                "INSERT INTO placement_rollouts(rollout_id,placement_id,project_id,previous_config_json,candidate_config_json,base_revision,base_intent,previous_replicas,candidate_replicas,state,stabilization_seconds,deadline_seconds,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'staged',?10,?11,?12,?12)",
                params![id,candidate.id,candidate.project_id,serde_json::to_string(&previous_config)?,serde_json::to_string(candidate)?,previous.config_revision,previous.intent_revision,previous.desired_replicas,previous.desired_replicas.min(candidate.max_replicas),stabilization_seconds,deadline_seconds,now],
            )?;
            store.rollout(id)?.context("Missing staged rollout")
        })
    }

    pub(crate) fn rollout_secret_config(&self, id: &str, name: &str) -> Result<PlacementConfig> {
        crate::config::validate_id("secret", name)?;
        let rollout = self.rollout(id)?.context("Unknown rollout")?;
        ensure!(
            rollout.state == "staged",
            "Rollout secrets are already frozen"
        );
        let current = self
            .get_placement(&rollout.placement_id)?
            .context("Placement removed")?;
        ensure!(
            rollout.base_matches(&current),
            "Placement changed during staging"
        );
        ensure!(
            secret_names(&rollout.candidate_config).contains(name)
                && !secret_names(&rollout.previous_config)
                    .iter()
                    .any(|old| old.eq_ignore_ascii_case(name)),
            "Rollout replacement requires a new candidate secret reference"
        );
        Ok(rollout.candidate_config)
    }

    pub(crate) fn begin_rollout_validation(&self, id: &str, now: i64) -> Result<RolloutRecord> {
        self.with_rollout_transaction(|| {
            let store = self;
            store.require_no_pending_rollout_host_operation()?;
            let rollout = store.rollout(id)?.context("Unknown rollout")?;
            ensure!(rollout.state == "staged", "Rollout is not staged");
            ensure!(
                now >= rollout.created_at && now.saturating_sub(rollout.created_at) < 86_400,
                "Staged rollout has expired"
            );
            let current = store.get_placement(&rollout.placement_id)?.context("Placement removed")?;
            ensure!(rollout.base_matches(&current), "Placement changed during staging");
            let deadline = now.checked_add(i64::from(rollout.deadline_seconds)).context("Invalid rollout deadline")?;
            store.connection.execute(
                "UPDATE placement_rollouts SET state='validating',updated_at=?2,deadline_at=?3 WHERE rollout_id=?1",
                params![id,now,deadline],
            )?;
            store.rollout(id)?.context("Missing validating rollout")
        })
    }

    pub(crate) fn validating_rollouts(&self) -> Result<Vec<RolloutRecord>> {
        let mut query = self.connection.prepare(&format!("SELECT {COLUMNS} FROM placement_rollouts WHERE state='validating' ORDER BY created_at,rowid"))?;
        Ok(query
            .query_map([], decode)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn require_rollout_validation(
        &self,
        id: &str,
        config: &PlacementConfig,
        now: i64,
    ) -> Result<RolloutRecord> {
        let rollout = self.rollout(id)?.context("Unknown rollout")?;
        let current = self
            .get_placement(&rollout.placement_id)?
            .context("Placement removed")?;
        ensure!(
            rollout.state == "validating"
                && rollout.base_matches(&current)
                && rollout.deadline_at.is_some_and(|deadline| now < deadline)
                && (config == &rollout.previous_config || config == &rollout.candidate_config),
            "Rollout validation is no longer authorized"
        );
        Ok(rollout)
    }

    pub(crate) fn complete_rollout_validation(
        &self,
        id: &str,
        valid: bool,
        now: i64,
    ) -> Result<()> {
        self.with_rollout_transaction(|| {
            let store = self;
            let Some(rollout) = store.rollout(id)? else { return Ok(()) };
            if rollout.state != "validating" { return Ok(()) }
            let current = store.get_placement(&rollout.placement_id)?;
            if !current.as_ref().is_some_and(|current| rollout.base_matches(current)) {
                return store.finish_rollout(id, "cancelled", Some("superseded"), now);
            }
            if now >= rollout.deadline_at.context("Missing rollout deadline")? {
                return store.finish_rollout(id, "failed", Some("validation_timeout"), now);
            }
            if !valid {
                return store.finish_rollout(id, "failed", Some("validation_failed"), now);
            }
            let revision = rollout.base_revision + 1;
            let intent = rollout.base_intent + 1;
            store.connection.execute(
                "UPDATE placements SET config_json=?2,config_revision=?3,intent_revision=?4,desired_replicas=?5,desired_state='running',last_error=NULL WHERE id=?1",
                params![rollout.placement_id,serde_json::to_string(&rollout.candidate_config)?,revision,intent,rollout.candidate_replicas],
            )?;
            store.connection.execute(
                "UPDATE placement_rollouts SET state='activating',active_revision=?2,active_intent=?3,updated_at=?4,stable_since=NULL,cohort=NULL WHERE rollout_id=?1",
                params![id,revision,intent,now],
            )?;
            Ok(())
        })
    }

    pub(crate) fn cancel_rollout(&self, placement: &str, now: i64) -> Result<()> {
        self.connection.execute(
            &format!("UPDATE placement_rollouts SET state='cancelled',failure_code='stopped',updated_at=?2,stable_since=NULL,cohort=NULL WHERE placement_id=?1 AND state IN {ACTIVE}"),
            params![placement,now],
        )?;
        Ok(())
    }

    pub(crate) fn reset_rollout_observations(&self) -> Result<()> {
        self.connection.execute(
            "UPDATE placement_rollouts SET stable_since=NULL,cohort=NULL WHERE state IN ('activating','rolling_back')", [],
        )?;
        Ok(())
    }

    fn finish_rollout(&self, id: &str, state: &str, code: Option<&str>, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE placement_rollouts SET state=?2,failure_code=COALESCE(?3,failure_code),updated_at=?4,stable_since=NULL,cohort=NULL WHERE rollout_id=?1",
            params![id,state,code,now],
        )?;
        Ok(())
    }

    fn begin_rollback(&self, rollout: &RolloutRecord, code: &str, now: i64) -> Result<()> {
        let revision = rollout
            .active_revision
            .context("Missing active revision")?
            .checked_add(1)
            .context("Placement revision overflow")?;
        let intent = rollout
            .active_intent
            .context("Missing active intent")?
            .checked_add(1)
            .context("Placement intent overflow")?;
        let deadline = now
            .checked_add(i64::from(rollout.deadline_seconds))
            .context("Invalid rollback deadline")?;
        self.connection.execute(
            "UPDATE placements SET config_json=?2,config_revision=?3,intent_revision=?4,desired_replicas=?5,desired_state='running',last_error=NULL WHERE id=?1",
            params![rollout.placement_id,serde_json::to_string(&rollout.previous_config)?,revision,intent,rollout.previous_replicas],
        )?;
        self.connection.execute(
            "UPDATE placement_rollouts SET state='rolling_back',active_revision=?2,active_intent=?3,updated_at=?4,deadline_at=?5,stable_since=NULL,cohort=NULL,failure_code=?6 WHERE rollout_id=?1",
            params![rollout.rollout_id,revision,intent,now,deadline,code],
        )?;
        Ok(())
    }

    /// A single supervisor advances durable intent using current process-bound observations.
    pub(crate) fn reconcile_rollouts(&self, now: i64) -> Result<()> {
        self.with_rollout_transaction(|| {
            let store = self;
            let rollouts = {
                let mut query = store.connection.prepare(&format!("SELECT {COLUMNS} FROM placement_rollouts WHERE state IN {ACTIVE} ORDER BY created_at,rowid"))?;
                query.query_map([], decode)?.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for rollout in rollouts {
                let current = store.get_placement(&rollout.placement_id)?;
                let Some(current) = current else {
                    store.finish_rollout(&rollout.rollout_id, "cancelled", Some("superseded"), now)?;
                    continue;
                };
                if matches!(rollout.state.as_str(), "staged" | "validating") {
                    if !rollout.base_matches(&current) {
                        store.finish_rollout(&rollout.rollout_id, "cancelled", Some("superseded"), now)?;
                    } else if rollout.state == "staged" && now.saturating_sub(rollout.created_at) >= 86_400 {
                        store.finish_rollout(&rollout.rollout_id, "failed", Some("staging_timeout"), now)?;
                    } else if rollout.state == "validating" && now >= rollout.deadline_at.context("Missing rollout deadline")? {
                        store.finish_rollout(&rollout.rollout_id, "failed", Some("validation_timeout"), now)?;
                    }
                    continue;
                }
                if !rollout.active_matches(&current) {
                    store.finish_rollout(&rollout.rollout_id, "cancelled", Some("superseded"), now)?;
                    continue;
                }
                let timed_out = now >= rollout.deadline_at.context("Missing rollout deadline")?;
                let failed = current.replicas.iter().any(|replica| {
                    replica.slot < current.desired_replicas
                        && replica.config_revision == current.config_revision
                        && replica.intent_revision == current.intent_revision
                        && matches!(replica.observed_state, ObservedState::Backoff | ObservedState::Failed)
                });
                if timed_out || failed {
                    if rollout.state == "activating" {
                        store.begin_rollback(&rollout, if timed_out { "activation_timeout" } else { "candidate_failed" }, now)?;
                    } else {
                        store.connection.execute(
                            "UPDATE placements SET desired_state='stopped',intent_revision=intent_revision+1 WHERE id=?1",
                            [&rollout.placement_id],
                        )?;
                        store.finish_rollout(&rollout.rollout_id, "failed", Some(if timed_out { "rollback_timeout" } else { "rollback_failed" }), now)?;
                    }
                    continue;
                }
                let ready = current.ready_replicas == current.desired_replicas
                    && current.replicas.iter().filter(|replica| replica.process_id.is_some()).count() == usize::from(current.desired_replicas)
                    && current.replicas.iter().filter(|replica| replica.slot < current.desired_replicas).all(|replica| {
                        replica.process_id.is_some()
                            && replica.config_revision == current.config_revision
                            && replica.intent_revision == current.intent_revision
                            && replica.observed_state == ObservedState::Running
                            && replica.applied_revision == Some(current.config_revision)
                    });
                if !ready {
                    if rollout.stable_since.is_some() || rollout.cohort.is_some() {
                        store.connection.execute("UPDATE placement_rollouts SET stable_since=NULL,cohort=NULL,updated_at=?2 WHERE rollout_id=?1", params![rollout.rollout_id,now])?;
                    }
                    continue;
                }
                let cohort = serde_json::to_string(&current.replicas.iter().filter(|replica| replica.slot < current.desired_replicas).map(|replica| (replica.slot,replica.process_id)).collect::<Vec<_>>())?;
                if rollout.cohort.as_ref() != Some(&cohort) || rollout.stable_since.is_none() || rollout.stable_since.is_some_and(|since| now < since) {
                    store.connection.execute("UPDATE placement_rollouts SET stable_since=?2,cohort=?3,updated_at=?2 WHERE rollout_id=?1", params![rollout.rollout_id,now,cohort])?;
                } else if now.saturating_sub(rollout.stable_since.unwrap()) >= i64::from(rollout.stabilization_seconds) {
                    store.finish_rollout(&rollout.rollout_id, if rollout.state == "rolling_back" { "rolled_back" } else { "healthy" }, None, now)?;
                }
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Result<(tempfile::TempDir, StateStore, PlacementConfig)> {
        let directory = tempfile::tempdir()?;
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        let config: PlacementConfig = serde_json::from_value(json!({
            "id":"service","project_id":"project","deployment_id":"deployment","revision":"one",
            "source":"offline","project_path":directory.path(),"max_replicas":2,
            "hosting":{"host":"127.0.0.1","port":8080,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"old-auth"},
            "events":[{"event_id":"http","event_version":[1,0,0],"board_version":[1,0,0]}]
        }))?;
        store.upsert_placement(
            "service",
            &serde_json::to_value(&config)?,
            DesiredState::Running,
        )?;
        let mut candidate = config;
        candidate.revision = "two".into();
        Ok((directory, store, candidate))
    }

    fn ready(store: &StateStore, pid: u32) -> Result<()> {
        let placement = store.get_placement("service")?.unwrap();
        for slot in 0..placement.desired_replicas {
            if let Some(old) = placement
                .replicas
                .iter()
                .find(|replica| replica.slot == slot)
            {
                store.record_replica(
                    "service",
                    slot,
                    old.config_revision,
                    old.intent_revision,
                    ObservedState::Stopped,
                    None,
                    None,
                )?;
            }
            assert!(store.claim_replica(
                "service",
                slot,
                placement.config_revision,
                placement.intent_revision
            )?);
            store.record_replica(
                "service",
                slot,
                placement.config_revision,
                placement.intent_revision,
                ObservedState::Starting,
                Some(pid + u32::from(slot)),
                None,
            )?;
            assert!(store.record_replica_prepared(
                "service",
                slot,
                placement.config_revision,
                placement.intent_revision,
                pid + u32::from(slot)
            )?);
        }
        Ok(())
    }

    fn activate(store: &StateStore, candidate: &PlacementConfig) -> Result<()> {
        store.stage_rollout("rollout", candidate, 1, 2, 30, 100)?;
        store.begin_rollout_validation("rollout", 101)?;
        store.complete_rollout_validation("rollout", true, 102)
    }

    #[test]
    fn staged_update_keeps_current_and_requires_a_stable_exact_cohort() -> Result<()> {
        let (_directory, store, candidate) = fixture()?;
        store.stage_rollout("rollout", &candidate, 1, 2, 30, 100)?;
        assert_eq!(store.get_placement("service")?.unwrap().config_revision, 1);
        assert!(store.require_no_active_rollout("service").is_err());
        assert!(
            store
                .stage_rollout("other", &candidate, 1, 2, 30, 100)
                .is_err()
        );
        store.begin_rollout_validation("rollout", 101)?;
        store.complete_rollout_validation("rollout", true, 102)?;
        assert_eq!(store.get_placement("service")?.unwrap().config_revision, 2);
        ready(&store, 1000)?;
        store.reconcile_rollouts(103)?;
        ready(&store, 2000)?;
        store.reconcile_rollouts(104)?;
        store.reconcile_rollouts(105)?;
        assert_eq!(store.rollout("rollout")?.unwrap().state, "activating");
        store.reconcile_rollouts(106)?;
        assert_eq!(store.rollout("rollout")?.unwrap().state, "healthy");
        store.require_no_active_rollout("service")?;
        Ok(())
    }

    #[test]
    fn catalog_service_staging_still_requires_successful_readiness_validation() -> Result<()> {
        let (_directory, mut store, mut candidate) = fixture()?;
        candidate.hosting = None;
        candidate.max_replicas = 1;
        let mut previous = candidate.clone();
        previous.revision = "one".into();
        store.upsert_placement(
            "service",
            &serde_json::to_value(&previous)?,
            DesiredState::Running,
        )?;
        let revision = store.get_placement("service")?.unwrap().config_revision;
        let staged = store.stage_rollout("catalog", &candidate, revision, 2, 30, 100)?;
        assert_eq!(staged.state, "staged");
        assert_eq!(
            store.get_placement("service")?.unwrap().config_revision,
            revision
        );
        store.begin_rollout_validation("catalog", 101)?;
        store.complete_rollout_validation("catalog", false, 102)?;
        assert_eq!(
            store.rollout("catalog")?.unwrap().failure_code.as_deref(),
            Some("validation_failed")
        );
        assert_eq!(
            store.get_placement("service")?.unwrap().config,
            serde_json::to_value(previous)?
        );
        assert_eq!(
            store.get_placement("service")?.unwrap().config_revision,
            revision
        );
        Ok(())
    }

    #[test]
    fn a_live_process_without_application_readiness_cannot_stabilize() -> Result<()> {
        let (_directory, store, candidate) = fixture()?;
        activate(&store, &candidate)?;
        assert!(store.claim_replica("service", 0, 2, 2)?);
        store.record_replica(
            "service",
            0,
            2,
            2,
            ObservedState::Starting,
            Some(1000),
            None,
        )?;
        store.reconcile_rollouts(103)?;
        store.reconcile_rollouts(120)?;
        let rollout = store.rollout("rollout")?.unwrap();
        assert_eq!(rollout.state, "activating");
        assert_eq!(rollout.stable_since, None);
        store.reconcile_rollouts(131)?;
        assert_eq!(store.rollout("rollout")?.unwrap().state, "rolling_back");
        Ok(())
    }

    #[test]
    fn failure_restores_configuration_monotonically_and_checks_rollback_health() -> Result<()> {
        let (directory, store, candidate) = fixture()?;
        let data = directory.path().join("mutable-data");
        std::fs::write(&data, "keep changes")?;
        activate(&store, &candidate)?;
        assert!(store.claim_replica("service", 0, 2, 2)?);
        store.record_replica("service", 0, 2, 2, ObservedState::Backoff, None, None)?;
        store.reconcile_rollouts(103)?;
        let restored = store.get_placement("service")?.unwrap();
        assert_eq!((restored.config_revision, restored.intent_revision), (3, 3));
        assert_eq!(restored.config["revision"], "one");
        assert_eq!(std::fs::read_to_string(data)?, "keep changes");
        ready(&store, 3000)?;
        store.reconcile_rollouts(104)?;
        store.reconcile_rollouts(106)?;
        let result = store.rollout("rollout")?.unwrap();
        assert_eq!(result.state, "rolled_back");
        assert_eq!(result.failure_code.as_deref(), Some("candidate_failed"));
        Ok(())
    }

    #[test]
    fn recovery_preserves_deadline_and_bounds_failed_rollback() -> Result<()> {
        let (directory, store, candidate) = fixture()?;
        activate(&store, &candidate)?;
        ready(&store, 1000)?;
        store.reconcile_rollouts(103)?;
        drop(store);
        let mut store = StateStore::open(&directory.path().join("management.sqlite"))?;
        store.reset_observed()?;
        store.reset_rollout_observations()?;
        let recovered = store.rollout("rollout")?.unwrap();
        assert_eq!(recovered.deadline_at, Some(131));
        assert_eq!(recovered.stable_since, None);
        store.reconcile_rollouts(131)?;
        assert_eq!(store.rollout("rollout")?.unwrap().state, "rolling_back");
        store.reconcile_rollouts(161)?;
        assert_eq!(store.rollout("rollout")?.unwrap().state, "failed");
        let stopped = store.get_placement("service")?.unwrap();
        assert_eq!(stopped.desired_state, DesiredState::Stopped);
        assert_eq!((stopped.config_revision, stopped.intent_revision), (3, 4));
        Ok(())
    }

    #[test]
    fn validation_failure_and_stop_do_not_activate_or_restore_over_new_intent() -> Result<()> {
        let (_directory, store, candidate) = fixture()?;
        store.stage_rollout("invalid", &candidate, 1, 2, 30, 100)?;
        store.begin_rollout_validation("invalid", 101)?;
        store.complete_rollout_validation("invalid", false, 102)?;
        assert_eq!(store.get_placement("service")?.unwrap().config_revision, 1);
        assert_eq!(
            store.rollout("invalid")?.unwrap().failure_code.as_deref(),
            Some("validation_failed")
        );
        activate(&store, &candidate)?;
        store.with_rollout_transaction(|| {
            store.cancel_rollout("service", 103)?;
            store.connection.execute("UPDATE placements SET desired_state='stopped',intent_revision=intent_revision+1 WHERE id='service'", [])?;
            Ok(())
        })?;
        store.reconcile_rollouts(1000)?;
        store.complete_rollout_validation("rollout", true, 1000)?;
        let stopped = store.get_placement("service")?.unwrap();
        assert_eq!((stopped.config_revision, stopped.intent_revision), (2, 3));
        assert_eq!(stopped.desired_state, DesiredState::Stopped);
        assert_eq!(store.rollout("rollout")?.unwrap().state, "cancelled");
        Ok(())
    }

    #[test]
    fn replacements_cannot_overwrite_previous_secrets_or_change_after_activation() -> Result<()> {
        let (_directory, store, mut candidate) = fixture()?;
        candidate.hosting.as_mut().unwrap().auth_secret = "new-auth".into();
        store.stage_rollout("rollout", &candidate, 1, 2, 30, 100)?;
        assert!(store.rollout_secret_config("rollout", "old-auth").is_err());
        assert!(
            store
                .rollout_secret_config("rollout", "not-referenced")
                .is_err()
        );
        assert!(store.rollout_secret_config("rollout", "new-auth").is_ok());
        store.begin_rollout_validation("rollout", 101)?;
        assert!(store.rollout_secret_config("rollout", "new-auth").is_err());
        Ok(())
    }

    #[test]
    fn replica_count_changes_supersede_instead_of_rolling_back_them() -> Result<()> {
        let (_directory, store, candidate) = fixture()?;
        activate(&store, &candidate)?;
        store.connection.execute(
            "UPDATE placements SET desired_replicas=2 WHERE id='service'",
            [],
        )?;
        store.reconcile_rollouts(200)?;
        assert_eq!(store.rollout("rollout")?.unwrap().state, "cancelled");
        let current = store.get_placement("service")?.unwrap();
        assert_eq!((current.config_revision, current.desired_replicas), (2, 2));
        Ok(())
    }

    #[test]
    fn staging_expiry_and_host_operations_cannot_race_activation() -> Result<()> {
        let (_directory, store, candidate) = fixture()?;
        store.connection.execute(
            "INSERT INTO host_operations(operation_id,kind,boot_id,state,created_at) VALUES('reboot','reboot','boot','pending',100)",
            [],
        )?;
        assert!(
            store
                .stage_rollout("rollout", &candidate, 1, 2, 30, 100)
                .is_err()
        );
        store
            .connection
            .execute("UPDATE host_operations SET state='failed'", [])?;
        store.stage_rollout("rollout", &candidate, 1, 2, 30, 100)?;
        store
            .connection
            .execute("UPDATE host_operations SET state='pending'", [])?;
        assert!(store.begin_rollout_validation("rollout", 101).is_err());
        store
            .connection
            .execute("UPDATE host_operations SET state='failed'", [])?;
        assert!(store.begin_rollout_validation("rollout", 86_500).is_err());
        store.reconcile_rollouts(86_500)?;
        let expired = store.rollout("rollout")?.unwrap();
        assert_eq!(expired.state, "failed");
        assert_eq!(expired.failure_code.as_deref(), Some("staging_timeout"));
        assert_eq!(store.get_placement("service")?.unwrap().config_revision, 1);
        store.require_no_active_rollout("service")?;
        Ok(())
    }

    #[test]
    fn validation_completion_is_fenced_by_newer_stop_and_persisted_deadline() -> Result<()> {
        let (_directory, mut store, candidate) = fixture()?;
        store.stage_rollout("rollout", &candidate, 1, 2, 30, 100)?;
        store.begin_rollout_validation("rollout", 101)?;
        store.set_desired_state("service", DesiredState::Stopped)?;
        store.complete_rollout_validation("rollout", true, 102)?;
        assert_eq!(store.rollout("rollout")?.unwrap().state, "cancelled");
        assert_eq!(store.get_placement("service")?.unwrap().config_revision, 1);
        store.set_desired_state("service", DesiredState::Running)?;
        store.stage_rollout("expired", &candidate, 1, 2, 30, 200)?;
        store.begin_rollout_validation("expired", 201)?;
        store.complete_rollout_validation("expired", true, 231)?;
        assert_eq!(
            store.rollout("expired")?.unwrap().failure_code.as_deref(),
            Some("validation_timeout")
        );
        assert_eq!(store.get_placement("service")?.unwrap().config_revision, 1);
        Ok(())
    }
}
