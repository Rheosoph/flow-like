use crate::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
#[path = "learning_repository_tests.rs"]
mod tests;

pub(crate) fn initialize_learning(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS learning_projects(id TEXT PRIMARY KEY,scope TEXT NOT NULL UNIQUE,body TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS learning_observations(project_id TEXT NOT NULL,sample_id TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(project_id,sample_id));
    CREATE TABLE IF NOT EXISTS learning_reviews(sequence INTEGER PRIMARY KEY AUTOINCREMENT,project_id TEXT NOT NULL,sample_id TEXT NOT NULL,revision INTEGER NOT NULL,body TEXT NOT NULL,UNIQUE(project_id,sample_id,revision));
    CREATE TABLE IF NOT EXISTS learning_review_selections(project_id TEXT NOT NULL,key TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(project_id,key));
    CREATE TABLE IF NOT EXISTS learning_review_queue(project_id TEXT NOT NULL,sample_id TEXT NOT NULL,PRIMARY KEY(project_id,sample_id));
    CREATE TABLE IF NOT EXISTS learning_consumed_reviews(project_id TEXT NOT NULL,sample_id TEXT NOT NULL,revision INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(project_id,sample_id));
    CREATE TABLE IF NOT EXISTS learning_cycles(id TEXT PRIMARY KEY,project_id TEXT NOT NULL,key TEXT NOT NULL,experiment_id TEXT UNIQUE,body TEXT NOT NULL,UNIQUE(project_id,key));
    CREATE TABLE IF NOT EXISTS learning_exposure(project_id TEXT NOT NULL,sample_id TEXT NOT NULL,group_id TEXT NOT NULL,role TEXT NOT NULL,cycle_id TEXT NOT NULL,PRIMARY KEY(project_id,sample_id,role));
    CREATE INDEX IF NOT EXISTS learning_exposure_groups ON learning_exposure(project_id,group_id,role);
    CREATE TABLE IF NOT EXISTS learning_changes(sequence INTEGER PRIMARY KEY AUTOINCREMENT,project_id TEXT NOT NULL,key TEXT NOT NULL,body TEXT NOT NULL,UNIQUE(project_id,key));
    CREATE TABLE IF NOT EXISTS learning_comparisons(cycle_id TEXT PRIMARY KEY,body TEXT NOT NULL);
    CREATE UNIQUE INDEX IF NOT EXISTS unique_learning_experiment ON experiments(json_extract(body,'$.request.context.learning_cycle_id')) WHERE json_extract(body,'$.request.context.learning_cycle_id') IS NOT NULL;")?;
    let has_revision:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('learning_consumed_reviews') WHERE name='revision')",[],|r|r.get(0))?;
    if !has_revision {
        connection.execute(
            "ALTER TABLE learning_consumed_reviews ADD COLUMN revision INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    Ok(())
}

impl TrainingRepository {
    pub fn find_learning_cycle_experiment(&self, cycle_id: &str) -> Result<Option<Experiment>> {
        let connection = self.connection()?;
        cycle(&connection, cycle_id)?;
        let body:Option<String>=connection.query_row("SELECT body FROM experiments WHERE json_extract(body,'$.request.context.learning_cycle_id')=?",[cycle_id],|r|r.get(0)).optional()?;
        body.map(|body| Ok(serde_json::from_str(&body)?))
            .transpose()
    }
    pub fn create_learning_project(
        &self,
        request: LearningProjectRequest,
        at_ms: i64,
    ) -> Result<LearningProject> {
        self.writable()?;
        validate_request(&request)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let project = LearningProject {
            id: id(),
            request,
            generation: 0,
            state: LearningState::Collecting,
            paused_from: None,
            usage: LearningUsage::default(),
            active_cycle_id: None,
            champion_artifact_id: None,
            candidate_artifact_id: None,
            canary_started_at_ms: None,
            review_watermark: 0,
            product_change_watermark: 0,
            last_cycle_at_ms: None,
            created_at_ms: at_ms,
            updated_at_ms: at_ms,
        };
        tx.execute(
            "INSERT INTO learning_projects(id,scope,body) VALUES(?,?,?)",
            params![
                project.id,
                serde_json::to_string(&project.request.stream)?,
                serde_json::to_string(&project)?
            ],
        )?;
        tx.commit()?;
        Ok(project)
    }
    pub fn get_learning_project(&self, project_id: &str) -> Result<LearningProject> {
        read(
            &self.connection()?,
            "SELECT body FROM learning_projects WHERE id=?",
            project_id,
        )
    }
    pub fn learning_projects(&self, scope: &str) -> Result<Vec<LearningProject>> {
        let values: Vec<LearningProject> = query(
            &self.connection()?,
            "SELECT body FROM learning_projects WHERE json_extract(body,'$.request.stream.project_id')=? ORDER BY rowid",
            scope,
        )?;
        Ok(values)
    }
    pub fn update_learning_project(
        &self,
        project_id: &str,
        expected_generation: i64,
        source: Value,
        policy: LearningPolicy,
        at_ms: i64,
    ) -> Result<LearningProject> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut project = project(&tx, project_id)?;
        cas(&project, expected_generation, at_ms)?;
        if project.active_cycle_id.is_some() {
            return Err(Error::Conflict(
                "wait for the current learning cycle before changing policy".into(),
            ));
        }
        project.request.source = source;
        project.request.policy = policy;
        validate_request(&project.request)?;
        touch(&mut project, at_ms);
        save_project(&tx, &project)?;
        tx.commit()?;
        Ok(project)
    }
    pub fn pause_learning_project(
        &self,
        project_id: &str,
        expected_generation: i64,
        at_ms: i64,
    ) -> Result<LearningProject> {
        self.set_learning_paused(project_id, expected_generation, true, at_ms)
    }
    pub fn resume_learning_project(
        &self,
        project_id: &str,
        expected_generation: i64,
        at_ms: i64,
    ) -> Result<LearningProject> {
        self.set_learning_paused(project_id, expected_generation, false, at_ms)
    }
    fn set_learning_paused(
        &self,
        project_id: &str,
        expected_generation: i64,
        paused: bool,
        at_ms: i64,
    ) -> Result<LearningProject> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = project(&tx, project_id)?;
        cas(&value, expected_generation, at_ms)?;
        if paused && value.state != LearningState::Paused {
            value.paused_from = Some(value.state);
            value.state = LearningState::Paused;
        } else if !paused && value.state == LearningState::Paused {
            value.state = value
                .paused_from
                .take()
                .unwrap_or(LearningState::Collecting);
        }
        touch(&mut value, at_ms);
        save_project(&tx, &value)?;
        tx.commit()?;
        Ok(value)
    }
    pub fn record_learning_observation(
        &self,
        project_id: &str,
        observation: &LearningObservation,
    ) -> Result<bool> {
        self.writable()?;
        validate_observation(observation)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = project(&tx, project_id)?;
        if observation
            .slices
            .keys()
            .any(|key| !value.request.policy.declared_slices.contains(key))
        {
            return Err(invalid("observation uses an undeclared analysis slice"));
        }
        if let Some(prediction) = &observation.student {
            if let Some(id) = &prediction.artifact_id {
                let artifact: ModelArtifact =
                    read(&tx, "SELECT body FROM artifacts WHERE id=?", id)?;
                if artifact.stream != value.request.stream {
                    return Err(invalid(
                        "observation model belongs to another learning stream",
                    ));
                }
            }
        }
        let body = serde_json::to_string(observation)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT body FROM learning_observations WHERE project_id=? AND sample_id=?",
                params![project_id, observation.sample_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            if previous != body {
                return Err(Error::Conflict(
                    "learning observation identity is immutable".into(),
                ));
            }
            return Ok(false);
        }
        let count: usize = tx.query_row(
            "SELECT count(*) FROM learning_observations WHERE project_id=?",
            [project_id],
            |r| r.get(0),
        )?;
        if count >= value.request.budget.maximum_observations {
            return Err(invalid("learning observation budget exhausted"));
        }
        reserve_collection_bytes(&mut value, body.len() as u64)?;
        save_project(&tx, &value)?;
        tx.execute(
            "INSERT INTO learning_observations(project_id,sample_id,body) VALUES(?,?,?)",
            params![project_id, observation.sample_id, body],
        )?;
        tx.commit()?;
        Ok(true)
    }
    pub fn learning_observations(&self, project_id: &str) -> Result<Vec<LearningObservation>> {
        self.get_learning_project(project_id)?;
        query(
            &self.connection()?,
            "SELECT body FROM learning_observations WHERE project_id=? ORDER BY rowid",
            project_id,
        )
    }
    pub fn record_learning_review(
        &self,
        project_id: &str,
        review: &LearningReview,
    ) -> Result<bool> {
        self.writable()?;
        if review.source == LabelSource::Teacher
            || review.reviewer.trim().is_empty()
            || review.revision == 0
            || !review.annotation.is_object()
        {
            return Err(invalid(
                "learning outcomes require a measured or reviewed annotation and provenance",
            ));
        }
        bounded(review, 64 * 1024)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = project(&tx, project_id)?;
        let observation: LearningObservation = read_two(
            &tx,
            "SELECT body FROM learning_observations WHERE project_id=? AND sample_id=?",
            project_id,
            &review.sample_id,
        )?;
        if review.available_at_ms < observation.captured_at_ms {
            return Err(invalid("review predates its observation"));
        }
        let payload = observation
            .sample
            .get("sample")
            .unwrap_or(&observation.sample);
        let feature_end = payload
            .get("window_end_ms")
            .and_then(Value::as_i64)
            .unwrap_or(observation.captured_at_ms);
        let target_end = review
            .outcome
            .as_ref()
            .or_else(|| payload.get("outcome"))
            .and_then(|v| v.get("target_end_ms"))
            .and_then(Value::as_i64)
            .unwrap_or(feature_end);
        if review.available_at_ms < feature_end.max(target_end) {
            return Err(invalid(
                "review outcome was unavailable before its feature or target window ended",
            ));
        }
        validate_annotation(&value, &review.annotation)?;
        let latest:Option<String>=tx.query_row("SELECT body FROM learning_reviews WHERE project_id=? AND sample_id=? ORDER BY revision DESC LIMIT 1",params![project_id,review.sample_id],|r|r.get(0)).optional()?;
        if let Some(latest) = latest {
            let latest: LearningReview = serde_json::from_str(&latest)?;
            if review == &latest {
                return Ok(false);
            }
            if review.revision <= latest.revision {
                return Err(Error::Conflict(
                    "review revisions must increase; an existing revision is immutable".into(),
                ));
            }
        }
        let body = serde_json::to_string(review)?;
        reserve_collection_bytes(&mut value, body.len() as u64)?;
        // A review may arrive while promotion evaluates its evidence. Invalidate
        // that promotion without moving the project clock to a future outcome.
        value.generation = value
            .generation
            .checked_add(1)
            .ok_or_else(|| invalid("learning project generation overflow"))?;
        save_project(&tx, &value)?;
        tx.execute(
            "INSERT INTO learning_reviews(project_id,sample_id,revision,body) VALUES(?,?,?,?)",
            params![project_id, review.sample_id, review.revision, body],
        )?;
        tx.execute(
            "DELETE FROM learning_review_queue WHERE project_id=? AND sample_id=?",
            params![project_id, review.sample_id],
        )?;
        tx.commit()?;
        Ok(true)
    }
    pub fn learning_training_reviews(&self, project_id: &str) -> Result<Vec<LearningReview>> {
        self.get_learning_project(project_id)?;
        query(
            &self.connection()?,
            "SELECT r.body FROM learning_reviews r WHERE project_id=? AND revision=(SELECT max(s.revision) FROM learning_reviews s WHERE s.project_id=r.project_id AND s.sample_id=r.sample_id) ORDER BY sequence",
            project_id,
        )
    }
    pub fn learning_training_reviews_at(
        &self,
        project_id: &str,
        at_ms: i64,
    ) -> Result<Vec<LearningReview>> {
        let connection = self.connection()?;
        project(&connection, project_id)?;
        latest_reviews_at(&connection, project_id, at_ms)
    }
    pub fn select_learning_reviews(
        &self,
        project_id: &str,
        key: &str,
        at_ms: i64,
    ) -> Result<Vec<LearningReviewItem>> {
        self.writable()?;
        if key.is_empty() || key.len() > 256 {
            return Err(invalid("review selection needs a bounded idempotency key"));
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = project(&tx, project_id)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT body FROM learning_review_selections WHERE project_id=? AND key=?",
                params![project_id, key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            return Ok(serde_json::from_str(&previous)?);
        }
        if at_ms < value.created_at_ms {
            return Err(invalid("selection predates project"));
        }
        let mut statement=tx.prepare("SELECT o.body FROM learning_observations o WHERE o.project_id=? AND NOT EXISTS(SELECT 1 FROM learning_reviews r WHERE r.project_id=o.project_id AND r.sample_id=o.sample_id) AND NOT EXISTS(SELECT 1 FROM learning_review_queue q WHERE q.project_id=o.project_id AND q.sample_id=o.sample_id) ORDER BY o.rowid DESC LIMIT ?")?;
        let pool: Vec<LearningObservation> = statement
            .query_map(
                params![project_id, value.request.policy.review_pool_size],
                |r| r.get::<_, String>(0),
            )?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect::<Result<_>>()?;
        drop(statement);
        let selected = select_diverse_reviews(pool, &value.request.policy);
        let body = serde_json::to_string(&selected)?;
        reserve_collection_bytes(&mut value, body.len() as u64)?;
        save_project(&tx, &value)?;
        for item in &selected {
            tx.execute(
                "INSERT INTO learning_review_queue(project_id,sample_id) VALUES(?,?)",
                params![project_id, item.observation.sample_id],
            )?;
        }
        tx.execute(
            "INSERT INTO learning_review_selections(project_id,key,body) VALUES(?,?,?)",
            params![project_id, key, body],
        )?;
        tx.commit()?;
        Ok(selected)
    }
    pub fn record_learning_product_change(
        &self,
        project_id: &str,
        key: &str,
        description: &str,
        at_ms: i64,
    ) -> Result<bool> {
        self.writable()?;
        if key.is_empty()
            || key.len() > 256
            || description.trim().is_empty()
            || description.len() > 4096
        {
            return Err(invalid(
                "product change needs bounded identity and description",
            ));
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = project(&tx, project_id)?;
        if at_ms < value.created_at_ms {
            return Err(invalid("product change predates project"));
        }
        let body = serde_json::json!({"description":description,"at_ms":at_ms}).to_string();
        let previous: Option<String> = tx
            .query_row(
                "SELECT body FROM learning_changes WHERE project_id=? AND key=?",
                params![project_id, key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            if serde_json::from_str::<Value>(&previous)?
                .get("description")
                .and_then(Value::as_str)
                != Some(description)
            {
                return Err(Error::Conflict("product change is immutable".into()));
            }
            return Ok(false);
        }
        reserve_collection_bytes(&mut value, body.len() as u64)?;
        save_project(&tx, &value)?;
        tx.execute(
            "INSERT INTO learning_changes(project_id,key,body) VALUES(?,?,?)",
            params![project_id, key, body],
        )?;
        tx.commit()?;
        Ok(true)
    }
    pub fn learning_error_analysis(&self, project_id: &str) -> Result<LearningErrorAnalysis> {
        let value = self.get_learning_project(project_id)?;
        let at_ms = now_ms();
        let mut observations = self.learning_observations(project_id)?;
        observations.retain(|o| o.captured_at_ms <= at_ms);
        let reviews = self.learning_training_reviews_at(project_id, at_ms)?;
        Ok(error_analysis(&value, &observations, &reviews))
    }
    pub fn learning_consultation_error_analysis(
        &self,
        project_id: &str,
    ) -> Result<LearningErrorAnalysis> {
        let connection = self.connection()?;
        let value = project(&connection, project_id)?;
        let at_ms = now_ms();
        // Diagnostics shown to the search agent count as training exposure. Only report
        // rows whose IDs or groups already prevent their use in any future audit.
        let mut statement = connection.prepare(
            "SELECT o.body FROM learning_observations o WHERE o.project_id=?
            AND json_extract(o.body,'$.captured_at_ms')<=?
            AND EXISTS(SELECT 1 FROM learning_exposure e WHERE e.project_id=o.project_id AND e.role='train' AND (e.sample_id=o.sample_id OR e.group_id=json_extract(o.body,'$.group_id')))
            AND NOT EXISTS(SELECT 1 FROM learning_exposure e WHERE e.project_id=o.project_id AND e.role='audit' AND (e.sample_id=o.sample_id OR e.group_id=json_extract(o.body,'$.group_id')))
            ORDER BY o.rowid",
        )?;
        let observations = statement
            .query_map(params![project_id, at_ms], |row| row.get::<_, String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?))
            .collect::<Result<Vec<LearningObservation>>>()?;
        let reviews = latest_reviews_at(&connection, project_id, at_ms)?;
        Ok(error_analysis(&value, &observations, &reviews))
    }
    pub fn learning_next_actions(
        &self,
        project_id: &str,
        at_ms: i64,
    ) -> Result<LearningNextActions> {
        let connection = self.connection()?;
        let value = project(&connection, project_id)?;
        next_actions(&connection, &value, at_ms)
    }
    pub fn reserve_learning_cycle(
        &self,
        project_id: &str,
        key: &str,
        budget: ExperimentBudget,
        at_ms: i64,
    ) -> Result<LearningCycle> {
        self.writable()?;
        if key.is_empty() || key.len() > 256 {
            return Err(invalid("learning cycle needs a bounded idempotency key"));
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<String> = tx
            .query_row(
                "SELECT body FROM learning_cycles WHERE project_id=? AND key=?",
                params![project_id, key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(old) = old {
            let cycle: LearningCycle = serde_json::from_str(&old)?;
            if serde_json::to_value(&cycle.budget)? != serde_json::to_value(&budget)? {
                return Err(Error::Conflict(
                    "cycle reservation budget is immutable".into(),
                ));
            }
            return Ok(cycle);
        }
        let mut value = project(&tx, project_id)?;
        let ready = next_actions(&tx, &value, at_ms)?;
        if !ready.ready_to_train {
            return Err(Error::Conflict(format!(
                "learning project is not ready: {}",
                ready.blocking.join(", ")
            )));
        }
        validate_cycle_budget(&value, &budget)?;
        let reviewed_through = review_watermark(&tx, project_id, at_ms)?;
        let observations_through = tx.query_row(
            "SELECT COALESCE(max(rowid),0) FROM learning_observations WHERE project_id=? AND json_extract(body,'$.captured_at_ms')<=?",
            params![project_id, at_ms],
            |r| r.get(0),
        )?;
        let product_change_through = change_watermark(&tx, project_id)?;
        let reviewed_sample_ids = available_new_reviews(&tx, project_id, at_ms)?;
        let reviewed_revisions = latest_reviews_at(&tx, project_id, at_ms)?
            .into_iter()
            .filter(|r| reviewed_sample_ids.contains(&r.sample_id))
            .map(|r| (r.sample_id, r.revision))
            .collect();
        let cycle = LearningCycle {
            id: id(),
            project_id: project_id.into(),
            key: key.into(),
            state: LearningCycleState::Reserved,
            budget,
            experiment_id: None,
            artifact_id: None,
            evaluation_id: None,
            audit_sample_ids: vec![],
            audit_group_ids: vec![],
            canary_sample_ids: vec![],
            reviewed_through,
            observations_through,
            reviewed_sample_ids,
            reviewed_revisions,
            product_change_through,
            created_at_ms: at_ms,
            updated_at_ms: at_ms,
            error: None,
        };
        add_reservation(&mut value.usage.reserved, &cycle.budget, true)?;
        value.usage.cycles_started += 1;
        value.active_cycle_id = Some(cycle.id.clone());
        value.state = LearningState::Training;
        touch(&mut value, at_ms);
        save_project(&tx, &value)?;
        save_cycle(&tx, &cycle)?;
        tx.commit()?;
        Ok(cycle)
    }
    pub fn attach_learning_experiment(
        &self,
        cycle_id: &str,
        experiment_id: &str,
        at_ms: i64,
    ) -> Result<LearningCycle> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut cycle = cycle(&tx, cycle_id)?;
        if cycle.experiment_id.as_deref() == Some(experiment_id) {
            return Ok(cycle);
        }
        if cycle.state != LearningCycleState::Reserved {
            return Err(Error::Conflict(
                "learning cycle already has an experiment or was aborted".into(),
            ));
        }
        let value = project(&tx, &cycle.project_id)?;
        if value.active_cycle_id.as_deref() != Some(cycle_id)
            || value.state != LearningState::Training
        {
            return Err(Error::Conflict("learning cycle is not active".into()));
        }
        let experiment: Experiment = read(
            &tx,
            "SELECT body FROM experiments WHERE id=?",
            experiment_id,
        )?;
        if experiment.request.stream != value.request.stream
            || experiment
                .request
                .context
                .get("learning_cycle_id")
                .and_then(Value::as_str)
                != Some(cycle_id)
            || task_identity(&experiment.request.spec) != task_identity(&value.request.spec)
            || serde_json::to_value(&experiment.request.goals)?
                != serde_json::to_value(&value.request.goals)?
            || experiment.created_at_ms < cycle.created_at_ms
            || experiment.usage.submitted_trials != 0
            || experiment.status != ExperimentStatus::Running
        {
            return Err(invalid(
                "experiment does not match the reserved learning task, stream, goals or unused state",
            ));
        }
        budget_within(&experiment.request.budget, &cycle.budget)?;
        let snapshot: DatasetSnapshot = read(
            &tx,
            "SELECT body FROM snapshots WHERE id=?",
            &experiment.request.snapshot_id,
        )?;
        if snapshot.test.len() < value.request.policy.minimum_audit_samples {
            return Err(invalid("learning cycle needs enough fresh audit samples"));
        }
        let source_ids: BTreeSet<_> = snapshot
            .train
            .iter()
            .chain(&snapshot.validation)
            .chain(&snapshot.test)
            .map(|s| s.id.as_str())
            .collect();
        cycle
            .reviewed_sample_ids
            .retain(|id| source_ids.contains(id.as_str()));
        cycle
            .reviewed_revisions
            .retain(|id, _| source_ids.contains(id.as_str()));
        for (role, partition) in [
            (
                "train",
                snapshot
                    .train
                    .iter()
                    .chain(&snapshot.validation)
                    .collect::<Vec<_>>(),
            ),
            ("audit", snapshot.test.iter().collect::<Vec<_>>()),
        ] {
            for sample in partition {
                let forbidden:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM learning_exposure WHERE project_id=? AND (sample_id=? OR group_id=?) AND (role='audit' OR ?='audit') AND NOT (cycle_id=? AND role=?))",params![cycle.project_id,sample.id,sample.group_id,role,cycle_id,role],|r|r.get(0))?;
                if forbidden {
                    return Err(invalid(
                        "learning split reused a sealed audit sample/group or audited previous training data",
                    ));
                }
                if role == "audit" && (!sample.accepted || sample.source == LabelSource::Teacher) {
                    return Err(invalid(
                        "fresh learning audit requires reviewed or measured outcomes",
                    ));
                }
                tx.execute("INSERT OR IGNORE INTO learning_exposure(project_id,sample_id,group_id,role,cycle_id) VALUES(?,?,?,?,?)",params![cycle.project_id,sample.id,sample.group_id,role,cycle_id])?;
            }
        }
        cycle.audit_sample_ids = snapshot.test.iter().map(|s| s.id.clone()).collect();
        cycle.audit_group_ids = snapshot
            .test
            .iter()
            .map(|s| s.group_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        cycle.experiment_id = Some(experiment_id.into());
        cycle.state = LearningCycleState::Attached;
        cycle.updated_at_ms = at_ms;
        save_cycle(&tx, &cycle)?;
        tx.commit()?;
        Ok(cycle)
    }
    pub fn get_learning_cycle(&self, cycle_id: &str) -> Result<LearningCycle> {
        cycle(&self.connection()?, cycle_id)
    }
    pub fn learning_cycles(&self, project_id: &str) -> Result<Vec<LearningCycle>> {
        self.get_learning_project(project_id)?;
        query(
            &self.connection()?,
            "SELECT body FROM learning_cycles WHERE project_id=? ORDER BY rowid",
            project_id,
        )
    }
    pub fn list_learning_cycles(&self, project_id: &str) -> Result<Vec<LearningCycle>> {
        self.learning_cycles(project_id)
    }
    pub fn learning_dataset_exclusions(
        &self,
        project_id: &str,
    ) -> Result<LearningDatasetExclusions> {
        let connection = self.connection()?;
        project(&connection, project_id)?;
        let mut statement=connection.prepare("SELECT sample_id,group_id,role FROM learning_exposure WHERE project_id=? ORDER BY sample_id")?;
        let mut result = LearningDatasetExclusions::default();
        for row in statement.query_map([project_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })? {
            let (sample, group, role) = row?;
            if role == "audit" {
                result.sealed_audit_sample_ids.push(sample);
                result.sealed_audit_group_ids.push(group);
            } else {
                result.previously_trained_sample_ids.push(sample);
                result.previously_trained_group_ids.push(group);
            }
        }
        result.sealed_audit_group_ids.sort();
        result.sealed_audit_group_ids.dedup();
        result.previously_trained_group_ids.sort();
        result.previously_trained_group_ids.dedup();
        Ok(result)
    }
    pub fn abort_learning_cycle(
        &self,
        cycle_id: &str,
        reason: &str,
        at_ms: i64,
    ) -> Result<LearningCycle> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut cycle = cycle(&tx, cycle_id)?;
        if cycle.state == LearningCycleState::Aborted {
            return Ok(cycle);
        }
        if cycle.experiment_id.is_some() {
            return Err(Error::Conflict("attached experiments must be cancelled and settled before releasing learning budget".into()));
        }
        let mut value = project(&tx, &cycle.project_id)?;
        add_reservation(&mut value.usage.reserved, &cycle.budget, false)?;
        cycle.state = LearningCycleState::Aborted;
        cycle.error = Some(reason.chars().take(4096).collect());
        cycle.updated_at_ms = at_ms;
        value.active_cycle_id = None;
        let state = if value.champion_artifact_id.is_some() {
            LearningState::Active
        } else {
            LearningState::Collecting
        };
        set_phase(&mut value, state);
        touch(&mut value, at_ms);
        save_project(&tx, &value)?;
        save_cycle(&tx, &cycle)?;
        tx.commit()?;
        Ok(cycle)
    }
    pub fn settle_learning_cycle(&self, cycle_id: &str, at_ms: i64) -> Result<LearningCycle> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut cycle = cycle(&tx, cycle_id)?;
        if cycle.state == LearningCycleState::Settled {
            return Ok(cycle);
        }
        let experiment_id = cycle
            .experiment_id
            .as_deref()
            .ok_or_else(|| invalid("learning cycle has no experiment"))?;
        let experiment: Experiment = read(
            &tx,
            "SELECT body FROM experiments WHERE id=?",
            experiment_id,
        )?;
        if !matches!(
            experiment.status,
            ExperimentStatus::Completed
                | ExperimentStatus::Failed
                | ExperimentStatus::Cancelled
                | ExperimentStatus::BudgetExhausted
        ) || experiment.usage.reserved_training_time_ms != 0
            || experiment.usage.reserved_artifact_bytes != 0
        {
            return Err(Error::Conflict(
                "learning experiment must finish and settle worker reservations first".into(),
            ));
        }
        let mut value = project(&tx, &cycle.project_id)?;
        if value.active_cycle_id.as_deref() != Some(cycle_id) {
            return Err(Error::Conflict("learning cycle is no longer active".into()));
        }
        add_reservation(&mut value.usage.reserved, &cycle.budget, false)?;
        add_usage(&mut value.usage.spent, &experiment.usage)?;
        value.usage.cycles_finished += 1;
        value.review_watermark = cycle.reviewed_through;
        value.product_change_watermark = cycle.product_change_through;
        value.last_cycle_at_ms = Some(at_ms);
        for sample_id in &cycle.reviewed_sample_ids {
            tx.execute(
                "INSERT INTO learning_consumed_reviews(project_id,sample_id,revision) VALUES(?,?,?) ON CONFLICT(project_id,sample_id) DO UPDATE SET revision=max(revision,excluded.revision)",
                params![cycle.project_id, sample_id,cycle.reviewed_revisions.get(sample_id).copied().unwrap_or(0)],
            )?;
        }
        if let Some(selected) = experiment
            .selected_trial_id
            .as_deref()
            .or(experiment.best_trial_id.as_deref())
        {
            let trial: ExperimentTrial = read(
                &tx,
                "SELECT body FROM experiment_trials WHERE id=?",
                selected,
            )?;
            cycle.artifact_id = trial.artifact_id;
        }
        cycle.evaluation_id = experiment.final_evaluation_id;
        cycle.state = LearningCycleState::Settled;
        cycle.updated_at_ms = at_ms;
        value.candidate_artifact_id = cycle.artifact_id.clone();
        value.active_cycle_id = None;
        let next = if cycle.evaluation_id.is_some() {
            LearningState::Shadow
        } else if value.champion_artifact_id.is_some() {
            LearningState::Active
        } else {
            LearningState::Collecting
        };
        set_phase(&mut value, next);
        touch(&mut value, at_ms);
        save_project(&tx, &value)?;
        save_cycle(&tx, &cycle)?;
        tx.commit()?;
        Ok(cycle)
    }
}

impl TrainingRepository {
    fn learning_cohort_report(
        &self,
        artifact_id: &str,
        task: EvaluationTask,
        cohort: &BTreeSet<String>,
        at_ms: i64,
    ) -> Result<EvaluationReport> {
        #[cfg(feature = "native")]
        {
            self.evaluate_task_cohort(artifact_id, task, Some(cohort), at_ms)
        }
        #[cfg(not(feature = "native"))]
        {
            if task != EvaluationTask::Classification {
                return Err(invalid(
                    "typed learning evaluation requires the native evaluation feature",
                ));
            }
            self.evaluate_classification_cohort(artifact_id, Some(cohort), at_ms)
        }
    }
    pub fn get_learning_comparison(&self, cycle_id: &str) -> Result<LearningComparison> {
        read(
            &self.connection()?,
            "SELECT body FROM learning_comparisons WHERE cycle_id=?",
            cycle_id,
        )
    }
    /// Review corrections require fresh canary evidence; saved predictions remain immutable.
    pub fn reconcile_learning_canary_reviews(&self, project_id: &str, at_ms: i64) -> Result<bool> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = project(&tx, project_id)?;
        let rejected = reject_stale_canary_reviews(&tx, &mut value, at_ms)?;
        tx.commit()?;
        Ok(!rejected)
    }
    pub fn learning_canary_ready(&self, project_id: &str, at_ms: i64) -> Result<bool> {
        let connection = self.connection()?;
        let value = project(&connection, project_id)?;
        if value.state != LearningState::Canary {
            return Ok(false);
        }
        let cohort = canary_cohort(&connection, &value, at_ms)?;
        if cohort.len() < value.request.policy.minimum_audit_samples {
            return Ok(false);
        }
        if stale_canary_review(&connection, &value, &cohort, at_ms)?.is_some() {
            return Ok(false);
        }
        for artifact_id in [&value.candidate_artifact_id, &value.champion_artifact_id]
            .into_iter()
            .flatten()
        {
            for sample_id in &cohort {
                let present: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM predictions WHERE artifact_id=? AND sample_id=?)",
                    params![artifact_id, sample_id],
                    |r| r.get(0),
                )?;
                if !present {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    pub fn learning_canary_sample_ids(&self, project_id: &str, at_ms: i64) -> Result<Vec<String>> {
        let connection = self.connection()?;
        let value = project(&connection, project_id)?;
        Ok(canary_cohort(&connection, &value, at_ms)?
            .into_iter()
            .collect())
    }
    pub fn compare_learning_cycle(&self, cycle_id: &str, at_ms: i64) -> Result<LearningComparison> {
        self.writable()?;
        let cycle = self.get_learning_cycle(cycle_id)?;
        let initial = self.get_learning_project(&cycle.project_id)?;
        if cycle.state != LearningCycleState::Settled || cycle.evaluation_id.is_none() {
            return Err(invalid(
                "learning comparison requires a completed independent experiment audit",
            ));
        }
        let candidate = cycle
            .artifact_id
            .as_deref()
            .ok_or_else(|| invalid("cycle has no candidate artifact"))?;
        let cohort: BTreeSet<_> = cycle.audit_sample_ids.iter().cloned().collect();
        if cohort.len() != cycle.audit_sample_ids.len()
            || cohort.len() < initial.request.policy.minimum_audit_samples
        {
            return Err(invalid("learning audit cohort is incomplete"));
        }
        let candidate_report = self.learning_cohort_report(
            candidate,
            initial.request.goals.task.clone(),
            &cohort,
            at_ms,
        )?;
        let champion_report = initial
            .champion_artifact_id
            .as_deref()
            .filter(|id| *id != candidate)
            .map(|id| {
                self.learning_cohort_report(id, initial.request.goals.task.clone(), &cohort, at_ms)
            })
            .transpose()?;
        let reasons = comparison_failures(&initial, &candidate_report, champion_report.as_ref());
        let comparison = LearningComparison {
            cycle_id: cycle_id.into(),
            candidate_artifact_id: candidate.into(),
            champion_artifact_id: initial.champion_artifact_id.clone(),
            candidate_evaluation_id: candidate_report.id.clone(),
            champion_evaluation_id: champion_report.as_ref().map(|r| r.id.clone()),
            candidate_metrics: candidate_report.metrics.clone(),
            champion_metrics: champion_report
                .as_ref()
                .map(|r| r.metrics.clone())
                .unwrap_or_default(),
            eligible: reasons.is_empty(),
            reasons,
            evaluated_at_ms: at_ms,
        };
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = project(&tx, &cycle.project_id)?;
        cas(&value, initial.generation, at_ms)?;
        verify_cohort_seal(&tx, &candidate_report, &cohort, &value.request.stream)?;
        if let Some(report) = &champion_report {
            verify_cohort_seal(&tx, report, &cohort, &value.request.stream)?;
        }
        tx.execute("INSERT INTO learning_comparisons(cycle_id,body) VALUES(?,?) ON CONFLICT(cycle_id) DO UPDATE SET body=excluded.body",params![cycle_id,serde_json::to_string(&comparison)?])?;
        if !comparison.eligible && value.candidate_artifact_id.as_deref() == Some(candidate) {
            value.candidate_artifact_id = None;
            value.canary_started_at_ms = None;
            let state = if value.champion_artifact_id.is_some() {
                LearningState::Active
            } else {
                LearningState::Collecting
            };
            set_phase(&mut value, state);
            touch(&mut value, at_ms);
            save_project(&tx, &value)?;
        }
        tx.commit()?;
        Ok(comparison)
    }
    pub fn promote_learning_cycle(
        &self,
        cycle_id: &str,
        expected_deployment_generation: i64,
        at_ms: i64,
    ) -> Result<Deployment> {
        self.writable()?;
        let cycle = self.get_learning_cycle(cycle_id)?;
        let before = self.get_learning_project(&cycle.project_id)?;
        let candidate = cycle
            .artifact_id
            .as_deref()
            .ok_or_else(|| invalid("cycle has no candidate"))?;
        if before.champion_artifact_id.as_deref() == Some(candidate) {
            let deployment = self.get_deployment(&before.request.deployment_id)?;
            if deployment.active_artifact_id == candidate {
                return Ok(deployment);
            }
        }
        if before.state == LearningState::Paused
            || before.candidate_artifact_id.as_deref() != Some(candidate)
        {
            return Err(Error::Conflict(
                "candidate is not awaiting promotion in this project".into(),
            ));
        }
        if !self.reconcile_learning_canary_reviews(&before.id, at_ms)? {
            return Err(invalid(
                "canary review changed after prediction; candidate rejected",
            ));
        }
        let comparison = self.compare_learning_cycle(cycle_id, at_ms)?;
        if !comparison.eligible {
            return Err(invalid(format!(
                "learning candidate failed its independent gate: {}",
                comparison.reasons.join(", ")
            )));
        }
        let mut canary_reports = None;
        if before.state == LearningState::Canary {
            let cohort = canary_cohort(&self.connection()?, &before, at_ms)?;
            if cohort.len() < before.request.policy.minimum_audit_samples {
                return Err(Error::Conflict("canary needs fresh independently reviewed candidate traffic before full promotion".into()));
            }
            if cycle.canary_sample_ids.is_empty() {
                let mut connection = self.connection()?;
                let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut saved = crate::learning_repository::cycle(&tx, cycle_id)?;
                if !saved.canary_sample_ids.is_empty()
                    && saved
                        .canary_sample_ids
                        .iter()
                        .cloned()
                        .collect::<BTreeSet<_>>()
                        != cohort
                {
                    return Err(Error::Conflict(
                        "canary audit was frozen concurrently".into(),
                    ));
                }
                saved.canary_sample_ids = cohort.iter().cloned().collect();
                for sample_id in &cohort {
                    let observation: LearningObservation = read_two(
                        &tx,
                        "SELECT body FROM learning_observations WHERE project_id=? AND sample_id=?",
                        &before.id,
                        sample_id,
                    )?;
                    tx.execute("INSERT OR IGNORE INTO learning_exposure(project_id,sample_id,group_id,role,cycle_id) VALUES(?,?,?,'audit',?)",params![before.id,sample_id,observation.group_id,cycle_id])?;
                }
                save_cycle(&tx, &saved)?;
                tx.commit()?;
            }
            let candidate_report = self.learning_cohort_report(
                candidate,
                before.request.goals.task.clone(),
                &cohort,
                at_ms,
            )?;
            let champion_report = before
                .champion_artifact_id
                .as_deref()
                .map(|id| {
                    self.learning_cohort_report(
                        id,
                        before.request.goals.task.clone(),
                        &cohort,
                        at_ms,
                    )
                })
                .transpose()?;
            let failures =
                comparison_failures(&before, &candidate_report, champion_report.as_ref());
            if !failures.is_empty() {
                let reason = format!("canary evidence failed its gate: {}", failures.join(", "));
                let mut connection = self.connection()?;
                let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut value = project(&tx, &before.id)?;
                cas(&value, before.generation, at_ms)?;
                value.candidate_artifact_id = None;
                value.canary_started_at_ms = None;
                value.state = LearningState::Active;
                touch(&mut value, at_ms);
                save_project(&tx, &value)?;
                let mut cycle = crate::learning_repository::cycle(&tx, cycle_id)?;
                cycle.error = Some(reason.clone());
                cycle.updated_at_ms = at_ms;
                save_cycle(&tx, &cycle)?;
                let mut rejected = comparison;
                rejected.eligible = false;
                rejected.reasons = failures;
                rejected.candidate_metrics = candidate_report.metrics;
                rejected.champion_metrics = champion_report
                    .as_ref()
                    .map(|r| r.metrics.clone())
                    .unwrap_or_default();
                rejected.candidate_evaluation_id = candidate_report.id;
                rejected.champion_evaluation_id = champion_report.map(|r| r.id);
                tx.execute(
                    "UPDATE learning_comparisons SET body=? WHERE cycle_id=?",
                    params![serde_json::to_string(&rejected)?, cycle_id],
                )?;
                tx.commit()?;
                return Err(invalid(reason));
            }
            canary_reports = Some((cohort, candidate_report, champion_report));
        }
        let report = {
            #[cfg(feature = "native")]
            {
                self.evaluate_task(candidate, before.request.goals.task.clone(), at_ms)?
            }
            #[cfg(not(feature = "native"))]
            {
                self.evaluate_classification(candidate, at_ms)?
            }
        };
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = project(&tx, &before.id)?;
        cas(&value, before.generation, at_ms)?;
        // Bind the final decision to the current review revisions while the
        // promotion transaction prevents another review from arriving.
        if reject_stale_canary_reviews(&tx, &mut value, at_ms)? {
            tx.commit()?;
            return Err(invalid(
                "canary review changed after prediction; candidate rejected",
            ));
        }
        let cohort = cycle.audit_sample_ids.iter().cloned().collect();
        let candidate_report: EvaluationReport = read(
            &tx,
            "SELECT body FROM evaluations WHERE id=?",
            &comparison.candidate_evaluation_id,
        )?;
        verify_cohort_seal(&tx, &candidate_report, &cohort, &value.request.stream)?;
        if let Some(id) = comparison.champion_evaluation_id.as_deref() {
            let champion: EvaluationReport =
                read(&tx, "SELECT body FROM evaluations WHERE id=?", id)?;
            verify_cohort_seal(&tx, &champion, &cohort, &value.request.stream)?;
        }
        if let Some((cohort, candidate, champion)) = &canary_reports {
            verify_cohort_seal(&tx, candidate, cohort, &value.request.stream)?;
            if let Some(champion) = champion {
                verify_cohort_seal(&tx, champion, cohort, &value.request.stream)?;
            }
        }
        if value.state != LearningState::Canary
            && value.request.policy.canary_fraction > 0.
            && value.champion_artifact_id.is_some()
        {
            let deployment: Deployment = read(
                &tx,
                "SELECT body FROM deployments WHERE id=?",
                &value.request.deployment_id,
            )?;
            if deployment.generation != expected_deployment_generation
                || Some(&deployment.active_artifact_id) != value.champion_artifact_id.as_ref()
            {
                return Err(Error::Conflict(
                    "champion deployment generation changed".into(),
                ));
            }
            value.state = LearningState::Canary;
            value.canary_started_at_ms = Some(at_ms);
            touch(&mut value, at_ms);
            save_project(&tx, &value)?;
            tx.commit()?;
            return Ok(deployment);
        }
        let deployment = self.promote_in_transaction(
            &tx,
            &value.request.deployment_id,
            expected_deployment_generation,
            &report.id,
            None,
            Some(MetricPromotionPolicy {
                minimum_audited_samples: value
                    .request
                    .goals
                    .minimum_audited_samples
                    .max(value.request.policy.minimum_audit_samples),
                bounds: value.request.goals.bounds.clone(),
            }),
            at_ms,
        )?;
        value.champion_artifact_id = Some(candidate.into());
        value.candidate_artifact_id = None;
        value.canary_started_at_ms = None;
        value.state = LearningState::Active;
        touch(&mut value, at_ms);
        save_project(&tx, &value)?;
        tx.commit()?;
        Ok(deployment)
    }
    pub fn rollback_learning_project(
        &self,
        project_id: &str,
        expected_deployment_generation: i64,
        at_ms: i64,
    ) -> Result<Deployment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = project(&tx, project_id)?;
        let deployment = self.rollback_in_transaction(
            &tx,
            &value.request.deployment_id,
            expected_deployment_generation,
            at_ms,
        )?;
        if deployment.stream != value.request.stream {
            return Err(invalid(
                "rollback deployment belongs to a different learning stream",
            ));
        }
        value.champion_artifact_id = Some(deployment.active_artifact_id.clone());
        value.candidate_artifact_id = None;
        value.canary_started_at_ms = None;
        set_phase(&mut value, LearningState::Active);
        touch(&mut value, at_ms);
        save_project(&tx, &value)?;
        tx.commit()?;
        Ok(deployment)
    }
    pub fn learning_route(&self, project_id: &str, sample_key: &str) -> Result<LearningRoute> {
        let value = self.get_learning_project(project_id)?;
        if value.state == LearningState::Paused {
            return Ok(LearningRoute::Teacher);
        }
        let champion = if let Some(id) = value.champion_artifact_id.as_ref() {
            let deployment = self.get_deployment(&value.request.deployment_id)?;
            if deployment.paused {
                return Ok(LearningRoute::Teacher);
            }
            if deployment.stream != value.request.stream || &deployment.active_artifact_id != id {
                return Err(Error::Conflict(
                    "learning champion differs from the durable deployment".into(),
                ));
            }
            Some(id.clone())
        } else {
            None
        };
        if value.state == LearningState::Canary {
            if let (Some(candidate), Some(champion)) =
                (value.candidate_artifact_id.clone(), champion.clone())
            {
                let hash = digest(format!("{}:{candidate}:{sample_key}", value.id).as_bytes());
                let bucket = u64::from_str_radix(&hash[..16], 16)
                    .map_err(|_| invalid("invalid route hash"))?
                    as f64
                    / u64::MAX as f64;
                if bucket < value.request.policy.canary_fraction {
                    return Ok(LearningRoute::Canary {
                        artifact_id: candidate,
                        champion_artifact_id: champion,
                    });
                }
            }
        }
        if value.state == LearningState::Shadow {
            if let Some(candidate) = value.candidate_artifact_id {
                return Ok(LearningRoute::Shadow {
                    champion_artifact_id: champion,
                    candidate_artifact_id: candidate,
                });
            }
        }
        Ok(champion
            .map(|artifact_id| LearningRoute::Champion { artifact_id })
            .unwrap_or(LearningRoute::Teacher))
    }
}

fn reject_stale_canary_reviews(
    connection: &Connection,
    value: &mut LearningProject,
    at_ms: i64,
) -> Result<bool> {
    if value.state != LearningState::Canary {
        return Ok(false);
    }
    let cohort = canary_cohort(connection, value, at_ms)?;
    let Some(sample_id) = stale_canary_review(connection, value, &cohort, at_ms)? else {
        return Ok(false);
    };
    let candidate = value
        .candidate_artifact_id
        .as_deref()
        .ok_or_else(|| invalid("canary has no candidate"))?;
    let mut cycle: LearningCycle = read_two(
        connection,
        "SELECT body FROM learning_cycles WHERE project_id=? AND json_extract(body,'$.artifact_id')=? ORDER BY rowid DESC LIMIT 1",
        &value.id,
        candidate,
    )?;
    if cycle.canary_sample_ids.is_empty() {
        cycle.canary_sample_ids = cohort.iter().cloned().collect();
    }
    for sample_id in &cohort {
        let observation: LearningObservation = read_two(
            connection,
            "SELECT body FROM learning_observations WHERE project_id=? AND sample_id=?",
            &value.id,
            sample_id,
        )?;
        connection.execute("INSERT OR IGNORE INTO learning_exposure(project_id,sample_id,group_id,role,cycle_id) VALUES(?,?,?,'audit',?)", params![value.id,sample_id,observation.group_id,cycle.id])?;
    }
    cycle.error = Some(format!(
        "Canary review changed after prediction for sample '{sample_id}'; candidate rejected and champion retained"
    ));
    cycle.updated_at_ms = at_ms;
    save_cycle(connection, &cycle)?;
    let mut comparison: LearningComparison = read(
        connection,
        "SELECT body FROM learning_comparisons WHERE cycle_id=?",
        &cycle.id,
    )?;
    comparison.eligible = false;
    comparison.reasons.push("canary_review_changed".into());
    connection.execute(
        "UPDATE learning_comparisons SET body=? WHERE cycle_id=?",
        params![serde_json::to_string(&comparison)?, cycle.id],
    )?;
    value.candidate_artifact_id = None;
    value.canary_started_at_ms = None;
    value.state = LearningState::Active;
    touch(value, at_ms);
    save_project(connection, value)?;
    Ok(true)
}

fn stale_canary_review(
    connection: &Connection,
    value: &LearningProject,
    cohort: &BTreeSet<String>,
    at_ms: i64,
) -> Result<Option<String>> {
    for sample_id in cohort {
        let body: Option<String> = connection.query_row(
            "SELECT body FROM learning_reviews WHERE project_id=? AND sample_id=? AND json_extract(body,'$.available_at_ms')<=? ORDER BY revision DESC LIMIT 1",
            params![value.id, sample_id, at_ms],
            |row| row.get(0),
        ).optional()?;
        let review: LearningReview =
            serde_json::from_str(&body.ok_or_else(|| invalid("canary review is unavailable"))?)?;
        for artifact_id in [&value.candidate_artifact_id, &value.champion_artifact_id]
            .into_iter()
            .flatten()
        {
            let body: Option<String> = connection
                .query_row(
                    "SELECT body FROM predictions WHERE artifact_id=? AND sample_id=?",
                    params![artifact_id, sample_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(body) = body else { continue };
            let prediction: PredictionRecord = serde_json::from_str(&body)?;
            // Old records did not bind their truth to a review revision. Only
            // an initial review can be trusted without that binding.
            let revision = prediction
                .details
                .get("learning_review_revision")
                .and_then(Value::as_u64)
                .unwrap_or(1);
            if revision != review.revision {
                return Ok(Some(sample_id.clone()));
            }
            let accepted: TrainingSample = read_two(
                connection,
                "SELECT body FROM annotations WHERE scope=? AND sample_id=? ORDER BY revision DESC LIMIT 1",
                &serde_json::to_string(&value.request.stream)?,
                sample_id,
            )?;
            let payload = accepted.payload.get("sample").unwrap_or(&accepted.payload);
            if !accepted.accepted
                || accepted.source != review.source
                || !review_annotation_matches(payload.get("annotation"), &review.annotation)?
                || payload.get("outcome").filter(|v| !v.is_null())
                    != review.outcome.as_ref().filter(|v| !v.is_null())
                || accepted.label_available_at_ms < review.available_at_ms
            {
                return Ok(Some(sample_id.clone()));
            }
        }
    }
    Ok(None)
}

fn review_annotation_matches(accepted: Option<&Value>, reviewed: &Value) -> Result<bool> {
    let Some(accepted) = accepted else {
        return Ok(false);
    };
    #[cfg(any(feature = "native", feature = "burn"))]
    {
        // Preparation converts numeric targets to their declared tensor types.
        // Compare those exact values rather than their original JSON spelling.
        let accepted: flow_like_ml_core::Annotation = serde_json::from_value(accepted.clone())?;
        let reviewed: flow_like_ml_core::Annotation = serde_json::from_value(reviewed.clone())?;
        Ok(accepted == reviewed)
    }
    #[cfg(not(any(feature = "native", feature = "burn")))]
    {
        // Execution-only builds evaluate classification without the typed core.
        if accepted.get("kind").and_then(Value::as_str) == Some("class")
            && reviewed.get("kind").and_then(Value::as_str) == Some("class")
        {
            return Ok(accepted
                .get("class_id")
                .and_then(Value::as_u64)
                .is_some_and(|class| {
                    reviewed.get("class_id").and_then(Value::as_u64) == Some(class)
                }));
        }
        Ok(accepted == reviewed)
    }
}

fn canary_cohort(
    connection: &Connection,
    value: &LearningProject,
    at_ms: i64,
) -> Result<BTreeSet<String>> {
    let Some(start) = value.canary_started_at_ms else {
        return Ok(BTreeSet::new());
    };
    let Some(candidate) = &value.candidate_artifact_id else {
        return Ok(BTreeSet::new());
    };
    let frozen:Option<String>=connection.query_row("SELECT body FROM learning_cycles WHERE project_id=? AND json_extract(body,'$.artifact_id')=? ORDER BY rowid DESC LIMIT 1",params![value.id,candidate],|r|r.get(0)).optional()?;
    if let Some(frozen) = frozen {
        let cycle: LearningCycle = serde_json::from_str(&frozen)?;
        if !cycle.canary_sample_ids.is_empty() {
            return Ok(cycle.canary_sample_ids.into_iter().collect());
        }
    }
    let mut statement=connection.prepare("SELECT DISTINCT o.sample_id FROM learning_observations o JOIN learning_reviews r ON r.project_id=o.project_id AND r.sample_id=o.sample_id WHERE o.project_id=? AND json_extract(o.body,'$.captured_at_ms')>? AND json_extract(o.body,'$.student.artifact_id')=? AND json_extract(r.body,'$.available_at_ms')>? AND json_extract(r.body,'$.available_at_ms')<=? AND NOT EXISTS(SELECT 1 FROM learning_exposure e WHERE e.project_id=o.project_id AND (e.sample_id=o.sample_id OR e.group_id=json_extract(o.body,'$.group_id')))")?;
    Ok(statement
        .query_map(params![value.id, start, candidate, start, at_ms], |row| {
            row.get(0)
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn comparison_failures(
    value: &LearningProject,
    candidate: &EvaluationReport,
    champion: Option<&EvaluationReport>,
) -> Vec<String> {
    let mut reasons = vec![];
    if candidate.audited_samples
        < value
            .request
            .policy
            .minimum_audit_samples
            .max(value.request.goals.minimum_audited_samples)
    {
        reasons.push("insufficient_independent_audit_samples".into());
    }
    for bound in &value.request.goals.bounds {
        if candidate.metrics.get(&bound.name).is_none_or(|v| {
            !v.is_finite()
                || bound.minimum.is_some_and(|min| *v < min)
                || bound.maximum.is_some_and(|max| *v > max)
        }) {
            reasons.push(format!("unmet_metric:{}", bound.name));
        }
    }
    let name = &value.request.goals.primary_metric;
    match candidate
        .metrics
        .get(name)
        .copied()
        .filter(|v| v.is_finite())
    {
        None => reasons.push("candidate_primary_metric_unavailable".into()),
        Some(candidate) => {
            if let Some(champion) = champion {
                match champion
                    .metrics
                    .get(name)
                    .copied()
                    .filter(|v| v.is_finite())
                {
                    None => reasons.push("champion_primary_metric_unavailable".into()),
                    Some(champion) => {
                        let improvement = match value.request.goals.direction {
                            MetricDirection::Maximize => candidate - champion,
                            MetricDirection::Minimize => champion - candidate,
                        };
                        if improvement < value.request.policy.minimum_improvement {
                            reasons.push("candidate_did_not_improve_over_champion".into());
                        }
                    }
                }
            }
        }
    }
    reasons
}
fn verify_cohort_seal(
    connection: &Connection,
    report: &EvaluationReport,
    cohort: &BTreeSet<String>,
    stream: &StreamKey,
) -> Result<()> {
    let artifact: ModelArtifact = read(
        connection,
        "SELECT body FROM artifacts WHERE id=?",
        &report.artifact_id,
    )?;
    if &artifact.stream != stream {
        return Err(invalid("learning evidence belongs to another stream"));
    }
    let mut predictions: Vec<PredictionRecord> = query(
        connection,
        "SELECT body FROM predictions WHERE artifact_id=? ORDER BY sample_id",
        &report.artifact_id,
    )?;
    predictions.retain(|p| cohort.contains(&p.sample_id));
    if predictions.len() != cohort.len()
        || predictions.iter().any(|p| {
            !matches!(
                p.actual_source,
                Some(LabelSource::Reviewed | LabelSource::ObservedOutcome)
            )
        })
        || digest(&serde_json::to_vec(&predictions)?) != report.evidence_digest
        || report.truth_digest.as_deref()
            != Some(
                crate::repository::evaluation_truth_digest(connection, stream, &predictions)?
                    .as_str(),
            )
    {
        return Err(Error::Conflict(
            "learning audit predictions or accepted truth changed".into(),
        ));
    }
    Ok(())
}

fn read<T: DeserializeOwned>(connection: &Connection, sql: &str, key: &str) -> Result<T> {
    let body: Option<String> = connection.query_row(sql, [key], |r| r.get(0)).optional()?;
    Ok(serde_json::from_str(
        &body.ok_or_else(|| Error::NotFound(key.into()))?,
    )?)
}
fn read_two<T: DeserializeOwned>(
    connection: &Connection,
    sql: &str,
    a: &str,
    b: &str,
) -> Result<T> {
    let body: Option<String> = connection
        .query_row(sql, params![a, b], |r| r.get(0))
        .optional()?;
    Ok(serde_json::from_str(
        &body.ok_or_else(|| Error::NotFound(b.into()))?,
    )?)
}
fn query<T: DeserializeOwned>(connection: &Connection, sql: &str, key: &str) -> Result<Vec<T>> {
    let mut statement = connection.prepare(sql)?;
    statement
        .query_map([key], |r| r.get::<_, String>(0))?
        .map(|row| Ok(serde_json::from_str(&row?)?))
        .collect()
}
fn project(connection: &Connection, id: &str) -> Result<LearningProject> {
    read(
        connection,
        "SELECT body FROM learning_projects WHERE id=?",
        id,
    )
}
fn cycle(connection: &Connection, id: &str) -> Result<LearningCycle> {
    read(
        connection,
        "SELECT body FROM learning_cycles WHERE id=?",
        id,
    )
}
fn save_project(connection: &Connection, value: &LearningProject) -> Result<()> {
    bounded(value, 1024 * 1024)?;
    connection.execute(
        "UPDATE learning_projects SET body=? WHERE id=?",
        params![serde_json::to_string(value)?, value.id],
    )?;
    Ok(())
}
fn save_cycle(connection: &Connection, value: &LearningCycle) -> Result<()> {
    connection.execute("INSERT INTO learning_cycles(id,project_id,key,experiment_id,body) VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET experiment_id=excluded.experiment_id,body=excluded.body",params![value.id,value.project_id,value.key,value.experiment_id,serde_json::to_string(value)?])?;
    Ok(())
}
fn touch(value: &mut LearningProject, at_ms: i64) {
    value.generation += 1;
    value.updated_at_ms = at_ms;
}
fn cas(value: &LearningProject, generation: i64, at_ms: i64) -> Result<()> {
    if value.generation != generation || at_ms < value.updated_at_ms {
        return Err(Error::Conflict(
            "learning project generation or clock changed".into(),
        ));
    }
    Ok(())
}
fn set_phase(value: &mut LearningProject, state: LearningState) {
    if value.state == LearningState::Paused {
        value.paused_from = Some(state);
    } else {
        value.state = state;
    }
}
fn bounded<T: serde::Serialize>(value: &T, limit: u64) -> Result<()> {
    struct Sink(u64);
    impl std::io::Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| std::io::Error::other("size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = Sink(0);
    serde_json::to_writer(&mut sink, value)?;
    if sink.0 > limit {
        return Err(invalid("learning record exceeds its byte budget"));
    }
    Ok(())
}
fn task_identity(spec: &Value) -> Value {
    serde_json::json!({"task":spec.get("task"),"labels":spec.get("labels"),"prediction_horizon_ms":spec.get("prediction_horizon_ms")})
}
fn validate_request(value: &LearningProjectRequest) -> Result<()> {
    bounded(value, 1024 * 1024)?;
    let p = &value.policy;
    let b = &value.budget;
    if value.stream.project_id.is_empty()
        || value.stream.stream_id.is_empty()
        || value.stream.inspection_version.is_empty()
        || value.deployment_id.is_empty()
        || !value.spec.is_object()
        || value.goals.bounds.is_empty()
        || p.minimum_new_reviews == 0
        || p.minimum_audit_samples == 0
        || p.review_batch_size == 0
        || p.review_batch_size > 512
        || p.review_pool_size < p.review_batch_size
        || p.review_pool_size > 10000
        || p.drift_window < 2
        || p.drift_window > 10000
        || p.declared_slices.len() > 64
        || b.maximum_cycles == 0
        || b.maximum_cycles > 10000
        || b.maximum_observations == 0
        || b.maximum_observations > 1_000_000
        || b.maximum_observation_bytes == 0
        || b.maximum_observation_bytes > 1024 * 1024 * 1024
        || b.maximum_training_time_ms == 0
        || b.maximum_artifact_bytes == 0
    {
        return Err(invalid(
            "invalid learning project task, policy or finite budgets",
        ));
    }
    if [
        p.uncertainty_weight,
        p.disagreement_weight,
        p.diversity_weight,
        p.embedding_shift_threshold,
        p.minimum_improvement,
    ]
    .iter()
    .any(|v| !v.is_finite() || *v < 0. || *v > 1e12)
        || !p.maximum_observed_error.is_finite()
        || !(0.0..=1.0).contains(&p.maximum_observed_error)
        || !p.canary_fraction.is_finite()
        || !(0.0..=1.0).contains(&p.canary_fraction)
    {
        return Err(invalid(
            "invalid learning selection, drift or canary threshold",
        ));
    }
    Ok(())
}
fn validate_observation(value: &LearningObservation) -> Result<()> {
    bounded(value, 256 * 1024)?;
    if value.sample_id.is_empty()
        || value.sample_id.len() > 512
        || value.group_id.is_empty()
        || value.group_id.len() > 512
        || value.embedding.len() > 2048
        || value
            .embedding
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1e12)
        || value.slices.len() > 64
    {
        return Err(invalid("invalid bounded learning observation"));
    }
    for prediction in [&value.student, &value.teacher].into_iter().flatten() {
        if !prediction.confidence.is_finite()
            || !(0.0..=1.0).contains(&prediction.confidence)
            || prediction.probabilities.len() > 4096
            || prediction
                .probabilities
                .iter()
                .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            return Err(invalid(
                "learning prediction probabilities must be finite and calibrated",
            ));
        }
    }
    Ok(())
}
fn validate_annotation(project: &LearningProject, annotation: &Value) -> Result<()> {
    #[cfg(any(feature = "native", feature = "burn"))]
    {
        let parsed: flow_like_ml_core::Annotation = serde_json::from_value(annotation.clone())?;
        parsed
            .validate(
                project
                    .request
                    .spec
                    .get("labels")
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len),
                1024 * 1024,
            )
            .map_err(|error| invalid(error.to_string()))?;
    }
    let kind = annotation
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("review annotation has no kind"))?;
    if !matches!(
        kind,
        "class" | "scalar" | "values" | "anomaly" | "boxes" | "mask" | "instance_masks" | "events"
    ) {
        return Err(invalid("review must contain an outcome"));
    }
    if kind == "class" {
        let class = annotation
            .get("class_id")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("class outcome needs class_id"))?;
        if project
            .request
            .spec
            .get("labels")
            .and_then(Value::as_array)
            .is_none_or(|labels| class as usize >= labels.len())
        {
            return Err(invalid("review class is outside fixed label order"));
        }
    }
    Ok(())
}
fn reserve_collection_bytes(value: &mut LearningProject, bytes: u64) -> Result<()> {
    let total = value
        .usage
        .observation_bytes
        .checked_add(bytes)
        .ok_or_else(|| invalid("learning collection size overflow"))?;
    if total > value.request.budget.maximum_observation_bytes {
        return Err(invalid("learning collection byte budget exhausted"));
    }
    value.usage.observation_bytes = total;
    Ok(())
}
fn review_watermark(connection: &Connection, id: &str, at_ms: i64) -> Result<i64> {
    Ok(connection.query_row("SELECT COALESCE(max(sequence),0) FROM learning_reviews WHERE project_id=? AND json_extract(body,'$.available_at_ms')<=?",params![id,at_ms],|r|r.get(0))?)
}
fn change_watermark(connection: &Connection, id: &str) -> Result<i64> {
    Ok(connection.query_row(
        "SELECT COALESCE(max(sequence),0) FROM learning_changes WHERE project_id=?",
        [id],
        |r| r.get(0),
    )?)
}
fn latest_reviews_at(
    connection: &Connection,
    project_id: &str,
    at_ms: i64,
) -> Result<Vec<LearningReview>> {
    let mut statement=connection.prepare("SELECT r.body FROM learning_reviews r WHERE r.project_id=? AND json_extract(r.body,'$.available_at_ms')<=? AND r.revision=(SELECT max(s.revision) FROM learning_reviews s WHERE s.project_id=r.project_id AND s.sample_id=r.sample_id AND json_extract(s.body,'$.available_at_ms')<=?) ORDER BY r.sequence")?;
    statement
        .query_map(params![project_id, at_ms, at_ms], |r| r.get::<_, String>(0))?
        .map(|row| Ok(serde_json::from_str(&row?)?))
        .collect()
}
fn available_new_reviews(connection: &Connection, id: &str, at_ms: i64) -> Result<Vec<String>> {
    let mut statement = connection.prepare(
        "SELECT DISTINCT r.sample_id FROM learning_reviews r
        JOIN learning_observations o ON o.project_id=r.project_id AND o.sample_id=r.sample_id
        WHERE r.project_id=? AND json_extract(r.body,'$.available_at_ms')<=?
        AND NOT EXISTS(SELECT 1 FROM learning_consumed_reviews old WHERE old.project_id=r.project_id AND old.sample_id=r.sample_id AND old.revision>=r.revision)
        AND NOT EXISTS(SELECT 1 FROM learning_exposure e WHERE e.project_id=r.project_id AND e.role='audit' AND (e.sample_id=r.sample_id OR e.group_id=json_extract(o.body,'$.group_id')))
        ORDER BY r.sample_id",
    )?;
    Ok(statement
        .query_map(params![id, at_ms], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}
fn fresh_signal_ids(
    connection: &Connection,
    project_id: &str,
    at_ms: i64,
    reviews: bool,
) -> Result<BTreeSet<String>> {
    // Keep both sequence and availability boundaries so delayed observations and outcomes
    // inserted before a cycle become fresh evidence when their timestamps mature.
    let sql = if reviews {
        "SELECT DISTINCT r.sample_id FROM learning_reviews r WHERE r.project_id=? AND json_extract(r.body,'$.available_at_ms')<=? AND NOT EXISTS(SELECT 1 FROM learning_cycles c WHERE c.project_id=r.project_id AND json_extract(c.body,'$.state')='settled' AND r.sequence<=json_extract(c.body,'$.reviewed_through') AND json_extract(r.body,'$.available_at_ms')<=json_extract(c.body,'$.created_at_ms'))"
    } else {
        "SELECT r.sample_id FROM learning_observations r WHERE r.project_id=? AND json_extract(r.body,'$.captured_at_ms')<=? AND NOT EXISTS(SELECT 1 FROM learning_cycles c WHERE c.project_id=r.project_id AND json_extract(c.body,'$.state')='settled' AND r.rowid<=COALESCE(json_extract(c.body,'$.observations_through'),0) AND json_extract(r.body,'$.captured_at_ms')<=json_extract(c.body,'$.created_at_ms'))"
    };
    let mut statement = connection.prepare(sql)?;
    Ok(statement
        .query_map(params![project_id, at_ms], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?)
}
fn budget_within(actual: &ExperimentBudget, reserved: &ExperimentBudget) -> Result<()> {
    if actual.maximum_trials > reserved.maximum_trials
        || actual.maximum_training_time_ms > reserved.maximum_training_time_ms
        || actual.maximum_llm_tokens > reserved.maximum_llm_tokens
        || actual.maximum_llm_cost_micros > reserved.maximum_llm_cost_micros
        || actual.maximum_artifact_bytes > reserved.maximum_artifact_bytes
        || actual.maximum_dataset_bytes > reserved.maximum_dataset_bytes
        || actual.maximum_wall_time_ms > reserved.maximum_wall_time_ms
    {
        return Err(invalid("experiment exceeds its learning reservation"));
    }
    Ok(())
}
fn validate_cycle_budget(value: &LearningProject, budget: &ExperimentBudget) -> Result<()> {
    let total = &value.request.budget;
    let spent = &value.usage.spent;
    let reserved = &value.usage.reserved;
    if value.usage.cycles_started >= total.maximum_cycles
        || budget.maximum_training_time_ms == 0
        || budget.maximum_artifact_bytes == 0
        || [
            (
                spent.training_time_ms,
                reserved.training_time_ms,
                budget.maximum_training_time_ms,
                total.maximum_training_time_ms,
            ),
            (
                spent.llm_tokens,
                reserved.llm_tokens,
                budget.maximum_llm_tokens,
                total.maximum_llm_tokens,
            ),
            (
                spent.llm_cost_micros,
                reserved.llm_cost_micros,
                budget.maximum_llm_cost_micros,
                total.maximum_llm_cost_micros,
            ),
            (
                spent.artifact_bytes,
                reserved.artifact_bytes,
                budget.maximum_artifact_bytes,
                total.maximum_artifact_bytes,
            ),
        ]
        .iter()
        .any(|(a, b, c, max)| {
            a.checked_add(*b)
                .and_then(|v| v.checked_add(*c))
                .is_none_or(|v| v > *max)
        })
    {
        return Err(invalid("learning project aggregate budget exhausted"));
    }
    Ok(())
}
fn add_reservation(
    value: &mut ExperimentUsage,
    budget: &ExperimentBudget,
    add: bool,
) -> Result<()> {
    for (target, amount) in [
        (&mut value.training_time_ms, budget.maximum_training_time_ms),
        (&mut value.llm_tokens, budget.maximum_llm_tokens),
        (&mut value.llm_cost_micros, budget.maximum_llm_cost_micros),
        (&mut value.artifact_bytes, budget.maximum_artifact_bytes),
    ] {
        *target = if add {
            target.checked_add(amount)
        } else {
            target.checked_sub(amount)
        }
        .ok_or_else(|| invalid("learning reservation accounting overflow"))?;
    }
    Ok(())
}
fn add_usage(value: &mut ExperimentUsage, usage: &ExperimentUsage) -> Result<()> {
    value.submitted_trials = value
        .submitted_trials
        .checked_add(usage.submitted_trials)
        .ok_or_else(|| invalid("trial counter overflow"))?;
    for (target, amount) in [
        (&mut value.training_time_ms, usage.training_time_ms),
        (&mut value.llm_tokens, usage.llm_tokens),
        (&mut value.llm_cost_micros, usage.llm_cost_micros),
        (&mut value.artifact_bytes, usage.artifact_bytes),
        (&mut value.dataset_bytes, usage.dataset_bytes),
    ] {
        *target = target
            .checked_add(amount)
            .ok_or_else(|| invalid("learning usage accounting overflow"))?;
    }
    Ok(())
}

pub(crate) fn validate_learning_experiment_create(
    connection: &Connection,
    request: &ExperimentRequest,
    at_ms: i64,
) -> Result<()> {
    let Some(cycle_id) = request
        .context
        .get("learning_cycle_id")
        .filter(|id| !id.is_null())
    else {
        return Ok(());
    };
    let cycle_id = cycle_id
        .as_str()
        .ok_or_else(|| invalid("learning cycle ID must be a string"))?;
    let cycle = cycle(connection, cycle_id)?;
    let value = project(connection, &cycle.project_id)?;
    if cycle.state != LearningCycleState::Reserved
        || cycle.experiment_id.is_some()
        || value.state != LearningState::Training
        || value.active_cycle_id.as_deref() != Some(cycle_id)
        || request.stream != value.request.stream
        || task_identity(&request.spec) != task_identity(&value.request.spec)
        || serde_json::to_value(&request.goals)? != serde_json::to_value(&value.request.goals)?
        || at_ms < cycle.created_at_ms
    {
        return Err(Error::Conflict(
            "experiment does not match an active learning reservation".into(),
        ));
    }
    budget_within(&request.budget, &cycle.budget)
}

pub(crate) fn validate_learning_dispatch(
    connection: &Connection,
    value: &Experiment,
) -> Result<()> {
    let Some(id) = value
        .request
        .context
        .get("learning_cycle_id")
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    let cycle = cycle(connection, id)?;
    let project = project(connection, &cycle.project_id)?;
    if cycle.state != LearningCycleState::Attached
        || cycle.experiment_id.as_deref() != Some(&value.id)
        || project.active_cycle_id.as_deref() != Some(id)
        || project.state != LearningState::Training
    {
        return Err(Error::Conflict(
            "learning experiment must attach to an active, unpaused cycle before dispatch".into(),
        ));
    }
    Ok(())
}

fn select_diverse_reviews(
    mut pool: Vec<LearningObservation>,
    policy: &LearningPolicy,
) -> Vec<LearningReviewItem> {
    pool.sort_by(|a, b| a.sample_id.cmp(&b.sample_id));
    let mut support = BTreeMap::<String, usize>::new();
    for observation in &pool {
        if let Some(label) = observation.student.as_ref().and_then(|p| p.label.as_ref()) {
            *support.entry(label.clone()).or_default() += 1;
        }
    }
    let mut chosen = Vec::<LearningReviewItem>::new();
    let mut selected_classes = BTreeMap::<String, usize>::new();
    while !pool.is_empty() && chosen.len() < policy.review_batch_size {
        let mut best = 0;
        let mut best_score = f64::NEG_INFINITY;
        let mut best_quota_priority = 0.;
        let mut best_reasons = vec![];
        for (index, observation) in pool.iter().enumerate() {
            let uncertainty = observation
                .student
                .as_ref()
                .map_or(1., |prediction| 1. - prediction.confidence);
            let disagreement = match (&observation.student, &observation.teacher) {
                (Some(a), Some(b))
                    if a.label.is_some() && b.label.is_some() && a.label != b.label =>
                {
                    1.
                }
                (Some(a), Some(b))
                    if !a.probabilities.is_empty()
                        && a.probabilities.len() == b.probabilities.len() =>
                {
                    let sa: f64 = a.probabilities.iter().sum();
                    let sb: f64 = b.probabilities.iter().sum();
                    if sa > 0. && sb > 0. {
                        0.5 * a
                            .probabilities
                            .iter()
                            .zip(&b.probabilities)
                            .map(|(a, b)| (a / sa - b / sb).abs())
                            .sum::<f64>()
                    } else {
                        0.
                    }
                }
                _ => 0.,
            };
            let distance = chosen
                .iter()
                .filter_map(|previous| {
                    embedding_distance(&observation.embedding, &previous.observation.embedding)
                })
                .reduce(f64::min)
                .map_or(0., |d| d / (1. + d));
            let label = observation.student.as_ref().and_then(|p| p.label.as_ref());
            let rare = label.is_some_and(|label| {
                selected_classes.get(label).copied().unwrap_or(0) < policy.rare_class_quota
            });
            let class_bonus = if rare {
                let count = label
                    .and_then(|label| support.get(label))
                    .copied()
                    .unwrap_or(1);
                1. + 1. / count as f64
            } else {
                0.
            };
            let score = policy.uncertainty_weight * uncertainty
                + policy.disagreement_weight * disagreement
                + policy.diversity_weight * distance
                + class_bonus;
            if class_bonus > best_quota_priority
                || (class_bonus == best_quota_priority && score > best_score)
            {
                best = index;
                best_score = score;
                best_quota_priority = class_bonus;
                best_reasons = vec![format!("uncertainty={uncertainty:.4}")];
                if disagreement > 0. {
                    best_reasons.push("teacher_student_disagreement".into());
                }
                if distance > 0. {
                    best_reasons.push("embedding_diversity".into());
                }
                if rare {
                    best_reasons.push("class_quota".into());
                }
            }
        }
        let observation = pool.remove(best);
        if let Some(label) = observation.student.as_ref().and_then(|p| p.label.as_ref()) {
            *selected_classes.entry(label.clone()).or_default() += 1;
        }
        chosen.push(LearningReviewItem {
            observation,
            score: best_score,
            reasons: best_reasons,
        });
    }
    chosen
}
fn embedding_distance(a: &[f64], b: &[f64]) -> Option<f64> {
    if a.is_empty() || a.len() != b.len() {
        None
    } else {
        Some(
            a.iter()
                .zip(b)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f64>()
                .sqrt()
                / (a.len() as f64).sqrt(),
        )
    }
}
fn outcome_label(value: &LearningProject, review: &LearningReview) -> Option<String> {
    match review.annotation.get("kind").and_then(Value::as_str)? {
        "class" => value
            .request
            .spec
            .get("labels")?
            .as_array()?
            .get(review.annotation.get("class_id")?.as_u64()? as usize)?
            .as_str()
            .map(str::to_owned),
        "anomaly" => review
            .annotation
            .get("is_anomaly")?
            .as_bool()
            .map(|v| if v { "anomaly".into() } else { "normal".into() }),
        _ => None,
    }
}
fn add_error(metrics: &mut LearningErrorMetrics, actual: &str, predicted: Option<&str>) {
    metrics.samples += 1;
    metrics.errors += usize::from(predicted != Some(actual));
    *metrics
        .confusion
        .entry(actual.into())
        .or_default()
        .entry(predicted.unwrap_or("<missing>").into())
        .or_default() += 1;
}
fn finish_errors(metrics: &mut LearningErrorMetrics) {
    metrics.accuracy =
        (metrics.samples > 0).then(|| 1. - metrics.errors as f64 / metrics.samples as f64);
    for (actual, predictions) in &metrics.confusion {
        let support: usize = predictions.values().sum();
        metrics.per_class_recall.insert(
            actual.clone(),
            predictions.get(actual).copied().unwrap_or(0) as f64 / support as f64,
        );
    }
}
fn error_analysis(
    value: &LearningProject,
    observations: &[LearningObservation],
    reviews: &[LearningReview],
) -> LearningErrorAnalysis {
    let reviews: BTreeMap<_, _> = reviews.iter().map(|r| (&r.sample_id, r)).collect();
    let mut result = LearningErrorAnalysis {
        overall: LearningErrorMetrics::default(),
        slices: BTreeMap::new(),
        excluded_unreviewed: 0,
    };
    for observation in observations {
        let Some(review) = reviews.get(&observation.sample_id) else {
            result.excluded_unreviewed += 1;
            continue;
        };
        let Some(actual) = outcome_label(value, review) else {
            continue;
        };
        let predicted = observation
            .student
            .as_ref()
            .and_then(|p| p.label.as_deref());
        add_error(&mut result.overall, &actual, predicted);
        for (key, slice) in &observation.slices {
            if value.request.policy.declared_slices.contains(key) {
                add_error(
                    result.slices.entry(format!("{key}={slice}")).or_default(),
                    &actual,
                    predicted,
                );
            }
        }
    }
    finish_errors(&mut result.overall);
    for metrics in result.slices.values_mut() {
        finish_errors(metrics);
    }
    result
}
fn next_actions(
    connection: &Connection,
    value: &LearningProject,
    at_ms: i64,
) -> Result<LearningNextActions> {
    let new_reviews = available_new_reviews(connection, &value.id, at_ms)?.len();
    let mut observations: Vec<LearningObservation> = query(
        connection,
        "SELECT body FROM learning_observations WHERE project_id=? ORDER BY rowid",
        &value.id,
    )?;
    observations.retain(|o| o.captured_at_ms <= at_ms);
    observations.sort_by_key(|o| o.captured_at_ms);
    let window = value.request.policy.drift_window;
    let recent = &observations[observations.len().saturating_sub(window)..];
    let reviews = latest_reviews_at(connection, &value.id, at_ms)?;
    let errors = error_analysis(value, recent, &reviews);
    let fresh_observations = fresh_signal_ids(connection, &value.id, at_ms, false)?;
    let fresh_reviews = fresh_signal_ids(connection, &value.id, at_ms, true)?;
    let observed_error = errors.overall.accuracy.map(|accuracy| 1. - accuracy);
    let centroid = |items: &[LearningObservation]| -> Option<Vec<f64>> {
        let size = items.first()?.embedding.len();
        if size == 0 || items.iter().any(|item| item.embedding.len() != size) {
            return None;
        }
        let mut values = vec![0.; size];
        for item in items {
            for (sum, value) in values.iter_mut().zip(&item.embedding) {
                *sum += *value / items.len() as f64;
            }
        }
        Some(values)
    };
    let embedding_shift = if observations.len() >= window * 2 {
        centroid(&observations[observations.len() - window * 2..observations.len() - window])
            .zip(centroid(recent))
            .and_then(|(a, b)| embedding_distance(&a, &b))
    } else {
        None
    };
    let mut reasons = vec![];
    let mut blocking = vec![];
    if value.usage.cycles_finished == 0 {
        reasons.push("initial_training".into());
    }
    if new_reviews >= value.request.policy.minimum_new_reviews {
        reasons.push("new_reviewed_examples".into());
    }
    if recent
        .iter()
        .any(|o| fresh_observations.contains(&o.sample_id))
        && embedding_shift
            .is_some_and(|shift| shift > value.request.policy.embedding_shift_threshold)
    {
        reasons.push("embedding_drift".into());
    }
    if errors.overall.samples >= value.request.policy.minimum_audit_samples
        && recent.iter().any(|o| fresh_reviews.contains(&o.sample_id))
        && observed_error.is_some_and(|error| error > value.request.policy.maximum_observed_error)
    {
        reasons.push("observed_error".into());
    }
    if change_watermark(connection, &value.id)? > value.product_change_watermark {
        reasons.push("product_change".into());
    }
    if value.state == LearningState::Paused {
        blocking.push("paused".into());
    }
    if value.active_cycle_id.is_some() {
        blocking.push("cycle_in_progress".into());
    }
    if matches!(value.state, LearningState::Shadow | LearningState::Canary) {
        blocking.push("candidate_requires_promotion_or_rejection".into());
    }
    if value.usage.cycles_started >= value.request.budget.maximum_cycles
        || value.usage.spent.training_time_ms >= value.request.budget.maximum_training_time_ms
    {
        blocking.push("aggregate_budget_exhausted".into());
    }
    if at_ms < value.updated_at_ms
        || value.last_cycle_at_ms.is_some_and(|last| {
            (at_ms as i128 - last as i128) < value.request.policy.cooldown_ms as i128
        })
    {
        blocking.push("cooldown".into());
    }
    if reasons.is_empty() {
        blocking.push("no_retraining_trigger".into());
    }
    if value.usage.cycles_finished > 0 && new_reviews < value.request.policy.minimum_new_reviews {
        blocking.push("insufficient_new_reviews".into());
    }
    Ok(LearningNextActions {
        project_id: value.id.clone(),
        state: value.state,
        new_reviews,
        ready_to_train: blocking.is_empty(),
        reasons,
        blocking,
        embedding_shift,
        observed_error,
    })
}
