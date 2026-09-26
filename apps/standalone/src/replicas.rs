use crate::{
    config::PlacementConfig,
    state::{DesiredState, ObservedState, PlacementRecord, StateStore},
};
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct ReplicaRecord {
    pub slot: u8,
    pub config_revision: u64,
    pub intent_revision: u64,
    pub observed_state: ObservedState,
    pub applied_revision: Option<u64>,
    pub process_id: Option<u32>,
    pub last_error: Option<String>,
}
impl StateStore {
    pub fn list_replicas(&self, id: &str) -> Result<Vec<ReplicaRecord>> {
        let mut query=self.connection.prepare("SELECT slot,config_revision,intent_revision,observed_state,applied_revision,process_id,last_error FROM placement_replicas WHERE placement_id=?1 ORDER BY slot")?;
        let mut rows = query.query([id])?;
        let mut values = Vec::new();
        while let Some(row) = rows.next()? {
            values.push(ReplicaRecord {
                slot: row.get(0)?,
                config_revision: row.get(1)?,
                intent_revision: row.get(2)?,
                observed_state: ObservedState::from_db(&row.get::<_, String>(3)?)?,
                applied_revision: row.get(4)?,
                process_id: row.get(5)?,
                last_error: row.get(6)?,
            });
        }
        Ok(values)
    }
    pub(crate) fn enrich_replicas(&self, record: &mut PlacementRecord) -> Result<()> {
        record.replicas = self.list_replicas(&record.id)?;
        record.running_replicas = record
            .replicas
            .iter()
            .filter(|r| r.process_id.is_some())
            .count()
            .try_into()?;
        record.ready_replicas = record
            .replicas
            .iter()
            .filter(|r| {
                r.slot < record.desired_replicas
                    && r.observed_state == ObservedState::Running
                    && r.config_revision == record.config_revision
                    && r.intent_revision == record.intent_revision
                    && r.applied_revision == Some(record.config_revision)
            })
            .count()
            .try_into()?;
        Ok(())
    }
    pub fn set_replica_count(&self, id: &str, revision: u64, count: u8) -> Result<()> {
        self.with_rollout_transaction(|| {
            self.require_no_active_rollout(id)?;
            ensure!(
                (1..=32).contains(&count),
                "Replica count must be between 1 and 32"
            );
            let record = self.get_placement(id)?.context("Unknown placement")?;
            let config: PlacementConfig = serde_json::from_value(record.config)?;
            config.validate()?;
            ensure!(
                count <= config.max_replicas,
                "Replica count exceeds deployment limit"
            );
            ensure!(
                self.connection.execute(
                    "UPDATE placements SET desired_replicas=?3 WHERE id=?1 AND config_revision=?2",
                    params![id, revision, count]
                )? == 1,
                "Placement revision changed"
            );
            Ok(())
        })
    }
    pub fn clamp_replica_count(&self, id: &str, max: u8) -> Result<()> {
        ensure!((1..=32).contains(&max), "Invalid replica limit");
        self.connection.execute(
            "UPDATE placements SET desired_replicas=MIN(desired_replicas,?2) WHERE id=?1",
            params![id, max],
        )?;
        Ok(())
    }
    pub fn claim_replica(&self, id: &str, slot: u8, revision: u64, intent: u64) -> Result<bool> {
        ensure!(slot < 32, "Invalid replica slot");
        let changed=self.connection.execute("INSERT INTO placement_replicas(placement_id,slot,config_revision,intent_revision,observed_state) SELECT id,?2,?3,?4,'starting' FROM placements WHERE id=?1 AND config_revision=?3 AND intent_revision=?4 AND desired_state='running' AND desired_replicas>?2 ON CONFLICT(placement_id,slot) DO UPDATE SET config_revision=excluded.config_revision,intent_revision=excluded.intent_revision,observed_state='starting',last_error=NULL WHERE placement_replicas.process_id IS NULL",params![id,slot,revision,intent])?;
        if changed == 1 {
            self.aggregate_replicas(id)?;
        }
        Ok(changed == 1)
    }
    pub fn record_replica(
        &self,
        id: &str,
        slot: u8,
        revision: u64,
        intent: u64,
        state: ObservedState,
        pid: Option<u32>,
        error: Option<&str>,
    ) -> Result<()> {
        ensure!(slot < 32 && pid != Some(0), "Invalid replica observation");
        ensure!(
            pid.is_none()
                || matches!(
                    state,
                    ObservedState::Starting | ObservedState::Running | ObservedState::Stopping
                ),
            "Replica process requires a live state"
        );
        ensure!(self.connection.execute("UPDATE placement_replicas SET observed_state=?5,process_id=?6,last_error=?7 WHERE placement_id=?1 AND slot=?2 AND config_revision=?3 AND intent_revision=?4",params![id,slot,revision,intent,state.as_str(),pid,error])?==1,"Replica observation is stale");
        self.aggregate_replicas(id)
    }
    pub fn replica_is_current(
        &self,
        id: &str,
        slot: u8,
        revision: u64,
        intent: u64,
        pid: u32,
    ) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM placement_replicas r JOIN placements p ON p.id=r.placement_id WHERE p.id=?1 AND r.slot=?2 AND r.config_revision=?3 AND r.intent_revision=?4 AND r.process_id=?5 AND p.config_revision=?3 AND p.intent_revision=?4 AND p.desired_state='running' AND p.desired_replicas>?2 AND r.observed_state IN ('starting','running'))",params![id,slot,revision,intent,pid],|r|r.get(0))?)
    }
    pub fn record_replica_prepared(
        &self,
        id: &str,
        slot: u8,
        revision: u64,
        intent: u64,
        pid: u32,
    ) -> Result<bool> {
        let changed=self.connection.execute("UPDATE placement_replicas SET observed_state='running',applied_revision=?3,last_error=NULL WHERE placement_id=?1 AND slot=?2 AND config_revision=?3 AND intent_revision=?4 AND process_id=?5 AND observed_state='starting' AND EXISTS(SELECT 1 FROM placements p WHERE p.id=?1 AND p.config_revision=?3 AND p.intent_revision=?4 AND p.desired_state='running' AND p.desired_replicas>?2)",params![id,slot,revision,intent,pid])?;
        if changed == 1 {
            self.aggregate_replicas(id)?;
        }
        Ok(changed == 1)
    }
    pub fn aggregate_replicas(&self, id: &str) -> Result<()> {
        let row:Option<(String,u64,u64,u8)>=self.connection.query_row("SELECT desired_state,config_revision,intent_revision,desired_replicas FROM placements WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((desired, revision, intent, count)) = row else {
            return Ok(());
        };
        let replicas = self.list_replicas(id)?;
        let live = replicas
            .iter()
            .filter(|r| r.process_id.is_some())
            .collect::<Vec<_>>();
        let ready = replicas
            .iter()
            .filter(|r| {
                r.slot < count
                    && r.observed_state == ObservedState::Running
                    && r.config_revision == revision
                    && r.intent_revision == intent
                    && r.applied_revision == Some(revision)
            })
            .count();
        let current = replicas
            .iter()
            .filter(|r| {
                r.slot < count && r.config_revision == revision && r.intent_revision == intent
            })
            .collect::<Vec<_>>();
        let state = if desired == DesiredState::Stopped.as_str() {
            if live.is_empty() {
                ObservedState::Stopped
            } else {
                ObservedState::Stopping
            }
        } else if ready == count as usize {
            ObservedState::Running
        } else if live
            .iter()
            .any(|r| r.observed_state == ObservedState::Stopping)
        {
            ObservedState::Stopping
        } else if !live.is_empty() {
            ObservedState::Starting
        } else if current
            .iter()
            .any(|r| r.observed_state == ObservedState::Failed)
        {
            ObservedState::Failed
        } else if current
            .iter()
            .any(|r| r.observed_state == ObservedState::Backoff)
        {
            ObservedState::Backoff
        } else if current
            .iter()
            .any(|r| r.observed_state == ObservedState::Starting)
        {
            ObservedState::Starting
        } else if !current.is_empty()
            && current
                .iter()
                .all(|r| r.observed_state == ObservedState::Stopped)
        {
            ObservedState::Stopped
        } else {
            ObservedState::Unknown
        };
        let pid = live.first().and_then(|r| r.process_id);
        let error = current.iter().find_map(|r| r.last_error.as_deref());
        self.connection.execute("UPDATE placements SET observed_state=?2,process_id=?3,last_error=?4,applied_revision=CASE WHEN ?5 THEN config_revision ELSE applied_revision END WHERE id=?1",params![id,state.as_str(),pid,error,ready==count as usize])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Result<(tempfile::TempDir, StateStore)> {
        let dir = tempfile::tempdir()?;
        let mut store = StateStore::open(&dir.path().join("management.sqlite"))?;
        let config = serde_json::json!({"id":"service","project_id":"project","deployment_id":"deployment","revision":"one","source":"offline","project_path":dir.path(),"max_replicas":3,"hosting":{"host":"127.0.0.1","port":8080,"max_in_flight":4,"request_timeout_secs":30,"auth_secret":"service"},"events":[{"event_id":"http","event_version":[1,0,0],"board_version":[1,0,0]}]});
        store.upsert_placement("service", &config, DesiredState::Running)?;
        Ok((dir, store))
    }
    #[test]
    fn pending_start_without_a_process_is_visible_but_never_ready() -> Result<()> {
        let (_dir, mut store) = fixture()?;
        assert!(store.claim_replica("service", 0, 1, 1)?);
        store.record_replica(
            "service",
            0,
            1,
            1,
            ObservedState::Starting,
            None,
            Some("Waiting for previous instance resource leases to retire"),
        )?;
        let pending = store.get_placement("service")?.unwrap();
        assert_eq!(pending.observed_state, ObservedState::Starting);
        assert_eq!(pending.ready_replicas, 0);
        assert_eq!(pending.process_id, None);
        assert_eq!(pending.applied_revision, None);
        let mut next = pending.config;
        next["revision"] = "two".into();
        store.upsert_placement("service", &next, DesiredState::Running)?;
        store.aggregate_replicas("service")?;
        assert_eq!(
            store.get_placement("service")?.unwrap().observed_state,
            ObservedState::Unknown
        );
        Ok(())
    }

    #[test]
    fn scale_preserves_lower_slot_identity_and_fences_removed_slot() -> Result<()> {
        let (_dir, store) = fixture()?;
        assert!(store.claim_replica("service", 0, 1, 1)?);
        store.record_replica("service", 0, 1, 1, ObservedState::Starting, Some(100), None)?;
        assert!(store.record_replica_prepared("service", 0, 1, 1, 100)?);
        store.set_replica_count("service", 1, 3)?;
        for slot in 1..3 {
            assert!(store.claim_replica("service", slot, 1, 1)?);
            store.record_replica(
                "service",
                slot,
                1,
                1,
                ObservedState::Starting,
                Some(100 + slot as u32),
                None,
            )?;
            assert!(store.record_replica_prepared("service", slot, 1, 1, 100 + slot as u32)?);
        }
        let before = store.get_placement("service")?.unwrap();
        assert_eq!(before.ready_replicas, 3);
        assert_eq!(before.observed_state, ObservedState::Running);
        store.set_replica_count("service", 1, 1)?;
        assert!(store.replica_is_current("service", 0, 1, 1, 100)?);
        assert!(!store.replica_is_current("service", 2, 1, 1, 102)?);
        assert!(!store.claim_replica("service", 2, 1, 1)?);
        assert!(!store.record_replica_prepared("service", 2, 1, 1, 102)?);
        let after = store.get_placement("service")?.unwrap();
        assert_eq!(after.ready_replicas, 1);
        assert_eq!(after.intent_revision, before.intent_revision);
        assert_eq!(after.config_revision, before.config_revision);
        assert!(store.set_replica_count("service", 0, 2).is_err());
        assert!(store.set_replica_count("service", 1, 4).is_err());
        Ok(())
    }
    #[test]
    fn readiness_requires_exact_slot_process_and_current_intent() -> Result<()> {
        let (_dir, mut store) = fixture()?;
        assert!(store.claim_replica("service", 0, 1, 1)?);
        store.record_replica("service", 0, 1, 1, ObservedState::Starting, Some(100), None)?;
        assert!(!store.record_replica_prepared("service", 0, 1, 1, 999)?);
        store.set_desired_state("service", DesiredState::Stopped)?;
        assert!(!store.record_replica_prepared("service", 0, 1, 1, 100)?);
        assert!(store.remove_placement("service").is_err());
        store.record_replica("service", 0, 1, 1, ObservedState::Stopped, None, None)?;
        store.remove_placement("service")?;
        assert!(store.list_replicas("service")?.is_empty());
        Ok(())
    }
}
