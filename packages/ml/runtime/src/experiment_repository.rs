use crate::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
#[path = "experiment_repository_tests.rs"]
mod tests;

pub(crate) fn initialize_experiments(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS experiments(id TEXT PRIMARY KEY,scope TEXT NOT NULL,body TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS experiment_trials(id TEXT PRIMARY KEY,experiment_id TEXT NOT NULL,job_id TEXT NOT NULL UNIQUE,idempotency_key TEXT NOT NULL,candidate_index INTEGER NOT NULL,body TEXT NOT NULL,UNIQUE(experiment_id,idempotency_key),UNIQUE(experiment_id,candidate_index));
         CREATE TABLE IF NOT EXISTS experiment_datasets(experiment_id TEXT NOT NULL,snapshot_id TEXT NOT NULL,body TEXT NOT NULL,PRIMARY KEY(experiment_id,snapshot_id));
         CREATE TABLE IF NOT EXISTS experiment_events(experiment_id TEXT NOT NULL,sequence INTEGER NOT NULL,body TEXT NOT NULL,PRIMARY KEY(experiment_id,sequence));",
    )?;
    Ok(())
}

/// Charge the interrupted slice before reserving another. The enclosing job transaction
/// commits budget exhaustion as a settled failure, so retrying cannot erase the charge.
pub(crate) fn reserve_recovered_trial(
    connection: &Connection,
    job: &TrainingJob,
    at_ms: i64,
) -> Result<Option<String>> {
    let body: Option<String> = connection
        .query_row(
            "SELECT body FROM experiment_trials WHERE job_id=?",
            [&job.id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(body) = body else {
        return Ok(None);
    };
    let mut trial: ExperimentTrial = serde_json::from_str(&body)?;
    if !matches!(job.status, JobStatus::Paused | JobStatus::Interrupted)
        || trial.status != ExperimentTrialStatus::Scheduled
    {
        return Err(Error::Conflict(
            "resume experiment-owned cancelled or failed jobs through their experiment".into(),
        ));
    }
    let mut value = experiment(connection, &trial.experiment_id)?;
    if value.status != ExperimentStatus::Running || value.selected_trial_id.is_some() {
        return Err(Error::Conflict(
            "experiment no longer permits candidate training".into(),
        ));
    }
    if at_ms < value.updated_at_ms {
        return Err(invalid("recovery predates experiment state"));
    }
    let previous = trial.reserved_training_time_ms;
    value.usage.reserved_training_time_ms = value
        .usage
        .reserved_training_time_ms
        .checked_sub(previous)
        .ok_or_else(|| invalid("inconsistent training reservation"))?;
    value.usage.training_time_ms = value
        .usage
        .training_time_ms
        .checked_add(previous)
        .ok_or_else(|| invalid("training time counter overflow"))?;
    trial.training_time_ms = trial
        .training_time_ms
        .checked_add(previous)
        .ok_or_else(|| invalid("trial training time counter overflow"))?;
    trial.reserved_training_time_ms = 0;
    let reserve = value.request.budget.worker_limits.maximum_duration_ms;
    let remaining = value
        .usage
        .training_time_ms
        .checked_add(value.usage.reserved_training_time_ms)
        .and_then(|used| used.checked_add(reserve));
    let failure = if at_ms as i128 - value.created_at_ms as i128
        > value.request.budget.maximum_wall_time_ms as i128
    {
        Some("experiment wall time budget exhausted during recovery".to_string())
    } else if remaining.is_none_or(|total| total > value.request.budget.maximum_training_time_ms) {
        Some("experiment training time budget exhausted during recovery".to_string())
    } else {
        None
    };
    if let Some(reason) = &failure {
        value.usage.reserved_artifact_bytes = value
            .usage
            .reserved_artifact_bytes
            .checked_sub(trial.reserved_artifact_bytes)
            .ok_or_else(|| invalid("inconsistent artifact reservation"))?;
        trial.reserved_artifact_bytes = 0;
        trial.status = ExperimentTrialStatus::Failed;
        trial.error = Some(reason.clone());
        value.stop_reason = Some(reason.clone());
        if unsettled_trials(connection, &value.id)? <= 1 {
            value.status = ExperimentStatus::BudgetExhausted;
        }
    } else {
        value.usage.reserved_training_time_ms += reserve;
        trial.reserved_training_time_ms = reserve;
    }
    trial.updated_at_ms = at_ms;
    touch(&mut value, at_ms);
    save_trial(connection, &trial)?;
    save_experiment(connection, &value)?;
    Ok(failure)
}

impl TrainingRepository {
    /// Prepared datasets may refit feature transforms while the accepted source row stays fixed.
    pub fn record_prepared_sample(
        &self,
        stream: &StreamKey,
        sample: &TrainingSample,
    ) -> Result<SampleReceipt> {
        self.writable()?;
        let connection = self.connection()?;
        let scope = serde_json::to_string(stream)?;
        let existing: Option<String> = connection
            .query_row(
                "SELECT body FROM annotations WHERE scope=? AND sample_id=? AND revision=?",
                params![scope, sample.id, sample.annotation_revision],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: TrainingSample = serde_json::from_str(&existing)?;
            if immutable_sample(&existing)? != immutable_sample(sample)? {
                return Err(Error::Conflict(
                    "prepared sample changed accepted identity, provenance or target".into(),
                ));
            }
            let accepted_sequence = connection
                .query_row(
                    "SELECT sequence FROM accepted WHERE scope=? AND sample_id=?",
                    params![scope, sample.id],
                    |row| row.get(0),
                )
                .optional()?;
            return Ok(SampleReceipt {
                inserted: false,
                accepted_sequence,
            });
        }
        drop(connection);
        self.record_sample(stream, sample)
    }
    pub(crate) fn check_experiment_artifact_budget(&self, job_id: &str, bytes: u64) -> Result<()> {
        let connection = self.connection()?;
        let body: Option<String> = connection
            .query_row(
                "SELECT body FROM experiment_trials WHERE job_id=?",
                [job_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(body) = body {
            let trial: ExperimentTrial = serde_json::from_str(&body)?;
            if bytes > trial.reserved_artifact_bytes {
                return Err(invalid("model exceeds its experiment artifact reservation"));
            }
        }
        Ok(())
    }
    /// Preserve a prepared split and verify its labels against the accepted sample ledger.
    /// Inputs may be transformed; identity, provenance and targets must remain unchanged.
    pub fn import_experiment_snapshot(
        &self,
        mut snapshot: DatasetSnapshot,
    ) -> Result<DatasetSnapshot> {
        self.writable()?;
        validate_snapshot(&snapshot)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_snapshot_truth(&tx, &snapshot)?;
        if snapshot.id.is_empty() {
            snapshot.id = id();
        }
        snapshot.digest = snapshot_hash(&snapshot)?;
        insert_snapshot(&tx, &snapshot)?;
        tx.commit()?;
        Ok(snapshot)
    }

    pub fn create_experiment(&self, request: ExperimentRequest, at_ms: i64) -> Result<Experiment> {
        self.writable()?;
        validate_request(&request)?;
        let snapshot =
            self.get_snapshot_limited(&request.snapshot_id, request.budget.maximum_dataset_bytes)?;
        if snapshot.stream != request.stream || snapshot.as_of_ms > at_ms {
            return Err(invalid(
                "experiment snapshot belongs to another stream or future time",
            ));
        }
        validate_snapshot(&snapshot)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_snapshot_truth(&tx, &snapshot)?;
        let mut experiment = Experiment {
            id: id(),
            request,
            status: ExperimentStatus::Running,
            generation: 0,
            usage: ExperimentUsage::default(),
            best_trial_id: None,
            selected_trial_id: None,
            selected_at_ms: None,
            final_evaluation_id: None,
            target_met: false,
            unmet_constraints: vec![],
            stop_reason: None,
            created_at_ms: at_ms,
            updated_at_ms: at_ms,
        };
        crate::learning_repository::validate_learning_experiment_create(
            &tx,
            &experiment.request,
            at_ms,
        )?;
        let preprocessing = experiment.request.preprocessing_manifest.clone();
        register_dataset(&tx, &mut experiment, &snapshot, preprocessing)?;
        for candidate in &experiment.request.candidates {
            validate_candidate_dataset(&tx, &experiment, candidate)?;
        }
        save_experiment(&tx, &experiment)?;
        tx.commit()?;
        Ok(experiment)
    }

    pub fn get_experiment(&self, experiment_id: &str) -> Result<Experiment> {
        let connection = self.connection()?;
        let mut value: Experiment = read(
            &connection,
            "SELECT body FROM experiments WHERE id=?",
            experiment_id,
        )?;
        if value.status == ExperimentStatus::CancelRequested
            && active_jobs(&connection, experiment_id)? == 0
        {
            value.status = ExperimentStatus::Cancelled;
        }
        Ok(value)
    }

    pub fn list_experiments(&self, stream: &StreamKey) -> Result<Vec<Experiment>> {
        query(
            &self.connection()?,
            "SELECT body FROM experiments WHERE scope=? ORDER BY rowid",
            &serde_json::to_string(stream)?,
        )
    }

    pub fn list_experiment_trials(&self, experiment_id: &str) -> Result<Vec<ExperimentTrial>> {
        self.get_experiment(experiment_id)?;
        query(
            &self.connection()?,
            "SELECT body FROM experiment_trials WHERE experiment_id=? ORDER BY candidate_index",
            experiment_id,
        )
    }

    pub fn get_experiment_trial(&self, trial_id: &str) -> Result<ExperimentTrial> {
        read(
            &self.connection()?,
            "SELECT body FROM experiment_trials WHERE id=?",
            trial_id,
        )
    }

    /// Append bounded diagnostic evidence without changing experiment selection or metrics.
    pub fn record_experiment_event(
        &self,
        experiment_id: &str,
        kind: &str,
        body: Value,
        at_ms: i64,
    ) -> Result<()> {
        self.writable()?;
        if kind.is_empty()
            || kind.len() > 128
            || !kind
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-.:".contains(&byte))
        {
            return Err(invalid(
                "experiment event kind must be a bounded identifier",
            ));
        }
        if serialized_bytes(&body)? > 64 * 1024 {
            return Err(invalid("experiment event exceeds 64 KiB"));
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let experiment = experiment(&tx, experiment_id)?;
        if at_ms < experiment.created_at_ms {
            return Err(invalid("experiment event predates experiment creation"));
        }
        append_event(&tx, experiment_id, kind, body, at_ms)?;
        tx.commit()?;
        Ok(())
    }

    pub fn experiment_events(&self, experiment_id: &str) -> Result<Vec<Value>> {
        let connection = self.connection()?;
        experiment(&connection, experiment_id)?;
        query(
            &connection,
            "SELECT body FROM experiment_events WHERE experiment_id=? ORDER BY sequence",
            experiment_id,
        )
    }

    /// Read the full validation cohort without materializing final-test samples in Rust.
    pub fn get_experiment_search_snapshot(
        &self,
        trial_id: &str,
        maximum_bytes: u64,
    ) -> Result<DatasetSnapshot> {
        let connection = self.connection()?;
        let trial: ExperimentTrial = read(
            &connection,
            "SELECT body FROM experiment_trials WHERE id=?",
            trial_id,
        )?;
        let limit = maximum_bytes.min(i64::MAX as u64) as i64;
        let body:Option<Option<String>>=connection.query_row("SELECT CASE WHEN length(CAST(body AS BLOB)) <= ? THEN json_set(body,'$.test',json('[]')) ELSE NULL END FROM snapshots WHERE id=?",params![limit,trial.dataset_snapshot_id],|row|row.get(0)).optional()?;
        let mut snapshot: DatasetSnapshot = serde_json::from_str(
            &body
                .ok_or_else(|| Error::NotFound(trial.dataset_snapshot_id.clone()))?
                .ok_or_else(|| invalid("search snapshot exceeds the read budget"))?,
        )?;
        snapshot.id = format!("{}:search", snapshot.id);
        snapshot.digest = snapshot_hash(&snapshot)?;
        Ok(snapshot)
    }

    pub fn update_experiment_tables(
        &self,
        experiment_id: &str,
        created: Vec<Value>,
        updated: Vec<Value>,
        at_ms: i64,
    ) -> Result<Experiment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = experiment(&tx, experiment_id)?;
        if at_ms < value.updated_at_ms {
            return Err(invalid(
                "table output registration predates experiment state",
            ));
        }
        for table in created {
            if !value.request.created_tables.contains(&table) {
                value.request.created_tables.push(table);
            }
        }
        for table in updated {
            if !value.request.updated_table_versions.contains(&table) {
                value.request.updated_table_versions.push(table);
            }
        }
        touch(&mut value, at_ms);
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(value)
    }

    pub fn experiment_result(&self, experiment_id: &str) -> Result<ExperimentResult> {
        let mut connection = self.connection()?;
        let tx = connection.transaction()?;
        let mut experiment = experiment(&tx, experiment_id)?;
        if experiment.status == ExperimentStatus::CancelRequested
            && active_jobs(&tx, experiment_id)? == 0
        {
            experiment.status = ExperimentStatus::Cancelled;
        }
        let trials: Vec<ExperimentTrial> = query(
            &tx,
            "SELECT body FROM experiment_trials WHERE experiment_id=? ORDER BY candidate_index",
            experiment_id,
        )?;
        let best = experiment
            .selected_trial_id
            .as_ref()
            .or(experiment.best_trial_id.as_ref())
            .and_then(|id| trials.iter().find(|t| &t.id == id))
            .and_then(|trial| trial.artifact_id.as_deref());
        let best_artifact = best
            .map(|id| read(&tx, "SELECT body FROM artifacts WHERE id=?", id))
            .transpose()?;
        let final_evaluation = if experiment.status == ExperimentStatus::Completed {
            experiment
                .final_evaluation_id
                .as_deref()
                .map(|id| read(&tx, "SELECT body FROM evaluations WHERE id=?", id))
                .transpose()?
        } else {
            None
        };
        let datasets = query(
            &tx,
            "SELECT body FROM experiment_datasets WHERE experiment_id=? ORDER BY rowid",
            experiment_id,
        )?;
        let events = query(
            &tx,
            "SELECT body FROM experiment_events WHERE experiment_id=? ORDER BY sequence",
            experiment_id,
        )?;
        tx.commit()?;
        Ok(ExperimentResult {
            experiment,
            trials,
            datasets,
            best_artifact,
            final_evaluation,
            events,
        })
    }

    pub fn cancel_experiment(&self, experiment_id: &str, at_ms: i64) -> Result<Experiment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = experiment(&tx, experiment_id)?;
        if matches!(
            value.status,
            ExperimentStatus::Completed
                | ExperimentStatus::Failed
                | ExperimentStatus::BudgetExhausted
        ) {
            return Ok(value);
        }
        if at_ms < value.updated_at_ms {
            return Err(invalid("cancellation predates experiment state"));
        }
        let trials: Vec<ExperimentTrial> = query(
            &tx,
            "SELECT body FROM experiment_trials WHERE experiment_id=? ORDER BY candidate_index",
            experiment_id,
        )?;
        let mut running = false;
        for mut trial in trials {
            let mut job: TrainingJob =
                read(&tx, "SELECT body FROM jobs WHERE id=?", &trial.job_id)?;
            match job.status {
                JobStatus::Running | JobStatus::CancelRequested => {
                    job.status = JobStatus::CancelRequested;
                    running = true;
                }
                JobStatus::Queued | JobStatus::Paused | JobStatus::Interrupted => {
                    job.status = JobStatus::Cancelled;
                    if trial.status == ExperimentTrialStatus::Scheduled {
                        value.usage.reserved_training_time_ms = value
                            .usage
                            .reserved_training_time_ms
                            .saturating_sub(trial.reserved_training_time_ms);
                        trial.reserved_training_time_ms = 0;
                        value.usage.reserved_artifact_bytes = value
                            .usage
                            .reserved_artifact_bytes
                            .saturating_sub(trial.reserved_artifact_bytes);
                        trial.reserved_artifact_bytes = 0;
                        trial.status = ExperimentTrialStatus::Cancelled;
                        trial.updated_at_ms = at_ms;
                        save_trial(&tx, &trial)?;
                    }
                }
                _ => continue,
            }
            job.updated_at_ms = at_ms;
            update_job(&tx, &job)?;
        }
        value.status = if running {
            ExperimentStatus::CancelRequested
        } else {
            ExperimentStatus::Cancelled
        };
        value.stop_reason = Some("cancelled".into());
        touch(&mut value, at_ms);
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(value)
    }

    pub fn resume_experiment(&self, experiment_id: &str, at_ms: i64) -> Result<Experiment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = experiment(&tx, experiment_id)?;
        if !matches!(
            value.status,
            ExperimentStatus::Cancelled | ExperimentStatus::CancelRequested
        ) || value.selected_trial_id.is_some()
            || active_jobs(&tx, experiment_id)? != 0
        {
            return Err(Error::Conflict("experiment must be stopped before resuming; a revealed final holdout cannot be reused for search".into()));
        }
        value.status = ExperimentStatus::Running;
        ensure_running(&value, at_ms)?;
        let trials: Vec<ExperimentTrial> = query(
            &tx,
            "SELECT body FROM experiment_trials WHERE experiment_id=? ORDER BY candidate_index",
            experiment_id,
        )?;
        for mut trial in trials {
            let mut job: TrainingJob =
                read(&tx, "SELECT body FROM jobs WHERE id=?", &trial.job_id)?;
            if job.status == JobStatus::Succeeded
                && trial.status == ExperimentTrialStatus::Cancelled
            {
                trial.status = ExperimentTrialStatus::Scheduled;
                trial.error = None;
                trial.updated_at_ms = at_ms;
                save_trial(&tx, &trial)?;
                continue;
            }
            if job.status != JobStatus::Cancelled {
                continue;
            }
            if trial.reserved_training_time_ms > 0 {
                value.usage.reserved_training_time_ms = value
                    .usage
                    .reserved_training_time_ms
                    .saturating_sub(trial.reserved_training_time_ms);
                value.usage.training_time_ms = value
                    .usage
                    .training_time_ms
                    .saturating_add(trial.reserved_training_time_ms);
                trial.training_time_ms = trial
                    .training_time_ms
                    .saturating_add(trial.reserved_training_time_ms);
            }
            let reserve = value.request.budget.worker_limits.maximum_duration_ms;
            if value
                .usage
                .training_time_ms
                .saturating_add(value.usage.reserved_training_time_ms)
                .saturating_add(reserve)
                > value.request.budget.maximum_training_time_ms
            {
                return Err(invalid(
                    "remaining training budget cannot resume this experiment",
                ));
            }
            value.usage.reserved_training_time_ms += reserve;
            trial.reserved_training_time_ms = reserve;
            if trial.reserved_artifact_bytes == 0 {
                let reserve = value.request.budget.worker_limits.maximum_artifact_bytes;
                if value
                    .usage
                    .artifact_bytes
                    .saturating_add(value.usage.reserved_artifact_bytes)
                    .saturating_add(reserve)
                    > value.request.budget.maximum_artifact_bytes
                {
                    return Err(invalid(
                        "remaining artifact budget cannot resume this experiment",
                    ));
                }
                value.usage.reserved_artifact_bytes += reserve;
                trial.reserved_artifact_bytes = reserve;
            }
            trial.status = ExperimentTrialStatus::Scheduled;
            trial.error = None;
            trial.updated_at_ms = at_ms;
            job.status = JobStatus::Queued;
            job.generation += 1;
            job.lease_owner = None;
            job.lease_expires_at_ms = None;
            job.error = None;
            job.updated_at_ms = at_ms;
            update_job(&tx, &job)?;
            save_trial(&tx, &trial)?;
        }
        value.stop_reason = None;
        touch(&mut value, at_ms);
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(value)
    }

    /// Resume only the already selected model's final audit. Search remains permanently sealed.
    pub fn resume_frozen_experiment_audit(
        &self,
        experiment_id: &str,
        at_ms: i64,
    ) -> Result<Experiment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = experiment(&tx, experiment_id)?;
        if !matches!(
            value.status,
            ExperimentStatus::Cancelled | ExperimentStatus::CancelRequested
        ) || value.selected_trial_id.is_none()
            || value.final_evaluation_id.is_some()
            || active_jobs(&tx, experiment_id)? != 0
        {
            return Err(Error::Conflict(
                "only a stopped, frozen final audit can resume".into(),
            ));
        }
        if at_ms < value.updated_at_ms
            || (at_ms as i128 - value.created_at_ms as i128)
                > value.request.budget.maximum_wall_time_ms as i128
        {
            return Err(invalid(
                "experiment wall time budget exhausted or clock moved backwards",
            ));
        }
        value.status = ExperimentStatus::Evaluating;
        value.stop_reason = None;
        touch(&mut value, at_ms);
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(value)
    }

    /// Only the trusted in-crate controller records validation measurements. Agent tools
    /// receive read access to these results and cannot submit their own metric assertions.
    pub(crate) fn record_experiment_trial_validation(
        &self,
        trial_id: &str,
        metrics: &BTreeMap<String, f64>,
        training_time_ms: u64,
        at_ms: i64,
    ) -> Result<ExperimentTrial> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut trial: ExperimentTrial = read(
            &tx,
            "SELECT body FROM experiment_trials WHERE id=?",
            trial_id,
        )?;
        if trial.status == ExperimentTrialStatus::Validated {
            if trial.validation_metrics != *metrics {
                return Err(Error::Conflict(
                    "trial validation results are immutable".into(),
                ));
            }
            return Ok(trial);
        }
        let mut value = experiment(&tx, &trial.experiment_id)?;
        if value.selected_trial_id.is_some() || value.status != ExperimentStatus::Running {
            return Err(Error::Conflict(
                "experiment no longer accepts validation results".into(),
            ));
        }
        if metrics.values().any(|v| !v.is_finite())
            || !metrics.contains_key(&value.request.goals.primary_metric)
        {
            return Err(invalid(
                "validation metrics must be finite and include the selection objective",
            ));
        }
        let job: TrainingJob = read(&tx, "SELECT body FROM jobs WHERE id=?", &trial.job_id)?;
        if job.status != JobStatus::Succeeded {
            return Err(Error::Conflict("trial has no completed model".into()));
        }
        let artifact: ModelArtifact = read(
            &tx,
            "SELECT body FROM artifacts WHERE id=?",
            job.artifact_id
                .as_deref()
                .ok_or_else(|| invalid("completed job lacks artifact"))?,
        )?;
        if artifact.job_id != job.id
            || artifact.snapshot_id != trial.snapshot_id
            || artifact.stream != value.request.stream
        {
            return Err(invalid("trial artifact provenance mismatch"));
        }
        settle_time(&mut value, &mut trial, training_time_ms)?;
        if trial.artifact_id.is_none() {
            value.usage.artifact_bytes = value
                .usage
                .artifact_bytes
                .checked_add(artifact.blob.bytes)
                .ok_or_else(|| invalid("artifact budget overflow"))?;
        }
        if value.usage.artifact_bytes > value.request.budget.maximum_artifact_bytes
            || value.usage.training_time_ms > value.request.budget.maximum_training_time_ms
        {
            value.status = ExperimentStatus::BudgetExhausted;
            value.stop_reason = Some("resource_budget_exhausted".into());
        }
        trial.artifact_id = Some(artifact.id);
        trial.status = ExperimentTrialStatus::Validated;
        trial.validation_metrics = metrics.clone();
        trial.updated_at_ms = at_ms;
        let best = value
            .best_trial_id
            .as_deref()
            .map(|id| {
                read::<ExperimentTrial>(&tx, "SELECT body FROM experiment_trials WHERE id=?", id)
            })
            .transpose()?;
        if best
            .as_ref()
            .is_none_or(|best| better(&trial, best, &value.request.goals))
        {
            value.best_trial_id = Some(trial.id.clone());
        }
        save_trial(&tx, &trial)?;
        touch(&mut value, at_ms);
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(trial)
    }

    pub(crate) fn fail_experiment_trial(
        &self,
        trial_id: &str,
        error: &str,
        training_time_ms: u64,
        at_ms: i64,
    ) -> Result<ExperimentTrial> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut trial: ExperimentTrial = read(
            &tx,
            "SELECT body FROM experiment_trials WHERE id=?",
            trial_id,
        )?;
        if matches!(
            trial.status,
            ExperimentTrialStatus::Validated
                | ExperimentTrialStatus::Failed
                | ExperimentTrialStatus::Cancelled
        ) {
            return Ok(trial);
        }
        let mut value = experiment(&tx, &trial.experiment_id)?;
        let mut job: TrainingJob = read(&tx, "SELECT body FROM jobs WHERE id=?", &trial.job_id)?;
        if matches!(job.status, JobStatus::Running | JobStatus::CancelRequested) {
            return Err(Error::Conflict(
                "active worker must settle before recording trial failure".into(),
            ));
        }
        if job.status == JobStatus::Queued {
            job.status = JobStatus::Failed;
            job.error = Some(error.into());
            job.updated_at_ms = at_ms;
            update_job(&tx, &job)?;
        }
        settle_time(&mut value, &mut trial, training_time_ms)?;
        if trial.artifact_id.is_none()
            && let Some(artifact_id) = job.artifact_id.as_deref()
        {
            let artifact: ModelArtifact =
                read(&tx, "SELECT body FROM artifacts WHERE id=?", artifact_id)?;
            value.usage.artifact_bytes = value
                .usage
                .artifact_bytes
                .checked_add(artifact.blob.bytes)
                .ok_or_else(|| invalid("artifact budget overflow"))?;
            trial.artifact_id = Some(artifact.id);
        }
        trial.status = if job.status == JobStatus::Cancelled
            || matches!(
                value.status,
                ExperimentStatus::Cancelled | ExperimentStatus::CancelRequested
            ) {
            ExperimentTrialStatus::Cancelled
        } else {
            ExperimentTrialStatus::Failed
        };
        trial.error = Some(error.into());
        trial.updated_at_ms = at_ms;
        save_trial(&tx, &trial)?;
        if value.status == ExperimentStatus::CancelRequested && active_jobs(&tx, &value.id)? == 0 {
            value.status = ExperimentStatus::Cancelled;
        }
        touch(&mut value, at_ms);
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(trial)
    }

    pub(crate) fn select_experiment_winner(
        &self,
        experiment_id: &str,
        at_ms: i64,
    ) -> Result<Experiment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = experiment(&tx, experiment_id)?;
        if matches!(
            value.status,
            ExperimentStatus::Cancelled | ExperimentStatus::CancelRequested
        ) {
            return Err(Error::Cancelled);
        }
        if value.selected_trial_id.is_some() {
            return Ok(value);
        }
        if !matches!(
            value.status,
            ExperimentStatus::Running | ExperimentStatus::BudgetExhausted
        ) || unsettled_trials(&tx, experiment_id)? != 0
        {
            return Err(Error::Conflict(
                "stop all trials before selecting the final model".into(),
            ));
        }
        let winner = value
            .best_trial_id
            .clone()
            .ok_or_else(|| invalid("experiment has no validated candidate"))?;
        value.selected_trial_id = Some(winner);
        value.selected_at_ms = Some(at_ms);
        value.status = ExperimentStatus::Evaluating;
        touch(&mut value, at_ms);
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(value)
    }

    pub(crate) fn stop_experiment_budget(
        &self,
        experiment_id: &str,
        reason: &str,
        at_ms: i64,
    ) -> Result<Experiment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = experiment(&tx, experiment_id)?;
        if !matches!(
            value.status,
            ExperimentStatus::Running | ExperimentStatus::BudgetExhausted
        ) || value.selected_trial_id.is_some()
            || unsettled_trials(&tx, experiment_id)? != 0
        {
            return Err(Error::Conflict(
                "stop active trials before ending the experiment budget".into(),
            ));
        }
        value.status = ExperimentStatus::BudgetExhausted;
        value.stop_reason = Some(reason.into());
        touch(&mut value, at_ms);
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(value)
    }

    pub(crate) fn finalize_experiment(
        &self,
        experiment_id: &str,
        evaluation_id: Option<&str>,
        stop_reason: &str,
        at_ms: i64,
    ) -> Result<Experiment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = experiment(&tx, experiment_id)?;
        if matches!(
            value.status,
            ExperimentStatus::Cancelled | ExperimentStatus::CancelRequested
        ) {
            return Err(Error::Cancelled);
        }
        if value.status == ExperimentStatus::Completed {
            if value.final_evaluation_id.as_deref() != evaluation_id {
                return Err(Error::Conflict(
                    "final experiment evidence is immutable".into(),
                ));
            }
            return Ok(value);
        }
        if unsettled_trials(&tx, experiment_id)? != 0 {
            return Err(Error::Conflict(
                "experiment still has active workers".into(),
            ));
        }
        value.target_met = false;
        value.unmet_constraints.clear();
        if let Some(evaluation_id) = evaluation_id {
            if value.status != ExperimentStatus::Evaluating {
                return Err(Error::Conflict(
                    "freeze the winner before reading final test evidence".into(),
                ));
            }
            let trial: ExperimentTrial = read(
                &tx,
                "SELECT body FROM experiment_trials WHERE id=?",
                value
                    .selected_trial_id
                    .as_deref()
                    .ok_or_else(|| invalid("no selected trial"))?,
            )?;
            let report: EvaluationReport = read(
                &tx,
                "SELECT body FROM evaluations WHERE id=?",
                evaluation_id,
            )?;
            let artifact: ModelArtifact = read(
                &tx,
                "SELECT body FROM artifacts WHERE id=?",
                trial
                    .artifact_id
                    .as_deref()
                    .ok_or_else(|| invalid("winner has no artifact"))?,
            )?;
            if report.artifact_id != artifact.id
                || report.dataset_digest != artifact.dataset_digest
                || report.task != value.request.goals.task
                || report.created_at_ms > at_ms
                || report.created_at_ms < value.selected_at_ms.unwrap_or(value.updated_at_ms)
            {
                return Err(invalid(
                    "final evaluation provenance, task or time differs from the selected model",
                ));
            }
            let predictions: Vec<PredictionRecord> = query(
                &tx,
                "SELECT body FROM predictions WHERE artifact_id=? ORDER BY sample_id",
                &artifact.id,
            )?;
            if digest(&serde_json::to_vec(&predictions)?) != report.evidence_digest
                || report.truth_digest.as_deref()
                    != Some(
                        crate::repository::evaluation_truth_digest(
                            &tx,
                            &artifact.stream,
                            &predictions,
                        )?
                        .as_str(),
                    )
            {
                return Err(Error::Conflict("final evaluation evidence changed".into()));
            }
            let dataset: DatasetSnapshot = read(
                &tx,
                "SELECT body FROM snapshots WHERE id=?",
                &trial.dataset_snapshot_id,
            )?;
            let test_ids: BTreeSet<_> = dataset
                .test
                .iter()
                .map(|sample| sample.id.as_str())
                .collect();
            if predictions
                .iter()
                .map(|prediction| prediction.sample_id.as_str())
                .collect::<BTreeSet<_>>()
                != test_ids
            {
                return Err(invalid(
                    "final evidence must cover exactly the frozen holdout",
                ));
            }
            let expected_truth: BTreeMap<_, _> = dataset
                .test
                .iter()
                .map(|sample| (&sample.id, sample))
                .collect();
            for prediction in &predictions {
                if prediction.actual_source.is_some() {
                    let actual = crate::repository::validate_prediction_truth(
                        &tx,
                        &artifact.stream,
                        prediction,
                    )?;
                    let original = expected_truth
                        .get(&actual.id)
                        .ok_or_else(|| invalid("holdout sample was not frozen"))?;
                    if immutable_sample(original)? != immutable_sample(&actual)? {
                        return Err(Error::Conflict(
                            "final holdout annotations changed after experiment creation".into(),
                        ));
                    }
                }
            }
            if report.audited_samples < value.request.goals.minimum_audited_samples {
                value.unmet_constraints.push(format!(
                    "audited_samples: {} < {}",
                    report.audited_samples, value.request.goals.minimum_audited_samples
                ));
            }
            for bound in &value.request.goals.bounds {
                if !satisfies(&report.metrics, bound) {
                    value
                        .unmet_constraints
                        .push(format!("metric {} does not meet its target", bound.name));
                }
            }
            if !report
                .metrics
                .contains_key(&value.request.goals.primary_metric)
            {
                value
                    .unmet_constraints
                    .push("primary metric is undefined on final holdout".into());
            }
            if value.request.goals.bounds.is_empty() {
                value
                    .unmet_constraints
                    .push("no acceptance target was configured".into());
            }
            value.target_met = value.unmet_constraints.is_empty();
            value.final_evaluation_id = Some(evaluation_id.into());
            value.status = ExperimentStatus::Completed;
        } else {
            if value.selected_trial_id.is_some() {
                value
                    .unmet_constraints
                    .push("final independent evaluation is unavailable".into());
            }
            value.status = if stop_reason.contains("budget") {
                ExperimentStatus::BudgetExhausted
            } else {
                ExperimentStatus::Failed
            };
        }
        value.stop_reason = Some(stop_reason.into());
        touch(&mut value, at_ms);
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(value)
    }

    pub fn append_experiment_candidates(
        &self,
        experiment_id: &str,
        candidates: Vec<TrainingRequest>,
        at_ms: i64,
    ) -> Result<Experiment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut experiment = experiment(&tx, experiment_id)?;
        ensure_running(&experiment, at_ms)?;
        if experiment
            .request
            .candidates
            .len()
            .checked_add(candidates.len())
            .is_none_or(|n| n > experiment.request.budget.maximum_trials)
        {
            return Err(invalid("candidate count exceeds experiment trial budget"));
        }
        for candidate in &candidates {
            validate_candidate(candidate)?;
            validate_candidate_dataset(&tx, &experiment, candidate)?;
        }
        experiment.request.candidates.extend(candidates);
        touch(&mut experiment, at_ms);
        save_experiment(&tx, &experiment)?;
        tx.commit()?;
        Ok(experiment)
    }

    pub fn register_experiment_dataset(
        &self,
        experiment_id: &str,
        mut derived: DatasetSnapshot,
        preprocessing_manifest: Value,
        at_ms: i64,
    ) -> Result<ExperimentDataset> {
        self.writable()?;
        validate_snapshot(&derived)?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut experiment = experiment(&tx, experiment_id)?;
        ensure_running(&experiment, at_ms)?;
        let original: DatasetSnapshot = read(
            &tx,
            "SELECT body FROM snapshots WHERE id=?",
            &experiment.request.snapshot_id,
        )?;
        if derived.stream != original.stream
            || derived.policy != original.policy
            || derived.as_of_ms != original.as_of_ms
            || derived.cutoff_sequence != original.cutoff_sequence
            || derived.excluded != original.excluded
        {
            return Err(invalid(
                "derived dataset changed its source, split policy or cutoff",
            ));
        }
        for (before, after) in [
            (&original.train, &derived.train),
            (&original.validation, &derived.validation),
            (&original.test, &derived.test),
        ] {
            if before.len() != after.len() {
                return Err(invalid("derived dataset changed split membership"));
            }
            let before: BTreeMap<_, _> = before.iter().map(|sample| (&sample.id, sample)).collect();
            for sample in after {
                let original = before
                    .get(&sample.id)
                    .ok_or_else(|| invalid("derived dataset moved a sample between splits"))?;
                if immutable_sample(original)? != immutable_sample(sample)? {
                    return Err(invalid(
                        "derived dataset changed sample identity, target or provenance",
                    ));
                }
            }
        }
        if derived.id.is_empty() {
            derived.id = id();
        }
        derived.digest = snapshot_hash(&derived)?;
        insert_snapshot(&tx, &derived)?;
        let dataset = register_dataset(&tx, &mut experiment, &derived, preprocessing_manifest)?;
        touch(&mut experiment, at_ms);
        save_experiment(&tx, &experiment)?;
        tx.commit()?;
        Ok(dataset)
    }

    /// Charge a conservative call allowance before invoking an external model. A failed call
    /// keeps its reservation charged, so retries cannot bypass the experiment budget.
    pub fn consume_experiment_llm_budget(
        &self,
        experiment_id: &str,
        tokens: u64,
        cost_micros: u64,
        at_ms: i64,
    ) -> Result<Experiment> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut experiment = experiment(&tx, experiment_id)?;
        charge_llm(&mut experiment, tokens, cost_micros, at_ms)?;
        save_experiment(&tx, &experiment)?;
        tx.commit()?;
        Ok(experiment)
    }

    /// Reserve one provider call and its conservative cost atomically across process restarts.
    pub fn reserve_experiment_consultation(
        &self,
        experiment_id: &str,
        tokens: u64,
        cost_micros: u64,
        maximum_calls: usize,
        at_ms: i64,
    ) -> Result<Experiment> {
        self.writable()?;
        if maximum_calls == 0 || maximum_calls > 32 || tokens == 0 {
            return Err(invalid(
                "consultations require 1..32 calls and a positive token reservation",
            ));
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut value = experiment(&tx, experiment_id)?;
        let calls: usize = tx.query_row(
            "SELECT count(*) FROM experiment_events WHERE experiment_id=? AND json_extract(body,'$.kind')='consultation_reserved'",
            [experiment_id], |row| row.get(0),
        )?;
        if calls >= maximum_calls {
            return Err(invalid("experiment consultation call budget exhausted"));
        }
        charge_llm(&mut value, tokens, cost_micros, at_ms)?;
        append_event(
            &tx,
            experiment_id,
            "consultation_reserved",
            serde_json::json!({"tokens":tokens,"cost_micros":cost_micros,"maximum_calls":maximum_calls}),
            at_ms,
        )?;
        save_experiment(&tx, &value)?;
        tx.commit()?;
        Ok(value)
    }

    /// Extend a validated Burn candidate from its durable optimizer checkpoint.
    pub(crate) fn continue_experiment_trial(
        &self,
        parent_trial_id: &str,
        epochs: usize,
        at_ms: i64,
    ) -> Result<ExperimentTrial> {
        self.writable()?;
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let parent: ExperimentTrial = read(
            &tx,
            "SELECT body FROM experiment_trials WHERE id=?",
            parent_trial_id,
        )?;
        let key = format!("continue:{parent_trial_id}:{epochs}");
        let previous: Option<String> = tx
            .query_row(
                "SELECT body FROM experiment_trials WHERE experiment_id=? AND idempotency_key=?",
                params![parent.experiment_id, key],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            return Ok(serde_json::from_str(&previous)?);
        }
        let mut value = experiment(&tx, &parent.experiment_id)?;
        ensure_running(&value, at_ms)?;
        let job: TrainingJob = read(&tx, "SELECT body FROM jobs WHERE id=?", &parent.job_id)?;
        if parent.status != ExperimentTrialStatus::Validated
            || job.status != JobStatus::Succeeded
            || job.request.engine != "burn"
        {
            return Err(invalid(
                "successive halving requires a validated Burn trial",
            ));
        }
        let checkpoint = job
            .checkpoint
            .clone()
            .ok_or_else(|| invalid("parent trial has no optimizer checkpoint"))?;
        let current = job
            .request
            .recipe
            .pointer("/config/epochs")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("Burn recipe has no epoch budget"))?;
        let maximum = job
            .request
            .recipe
            .pointer("/search/max_epochs")
            .and_then(Value::as_u64)
            .ok_or_else(|| invalid("parent trial did not permit successive halving"))?;
        if epochs as u64 <= current || epochs as u64 > maximum {
            return Err(invalid(
                "continuation epochs must increase within the fixed search limit",
            ));
        }
        let mut request = job.request;
        request.recipe["config"]["epochs"] = serde_json::json!(epochs);
        let rung = request
            .recipe
            .pointer("/search/rung")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| invalid("search rung overflow"))?;
        request.recipe["search"]["rung"] = serde_json::json!(rung);
        request.recipe["search"]["parent_trial_id"] = serde_json::json!(parent_trial_id);
        request.recipe["dataset_snapshot_id"] = serde_json::json!(parent.dataset_snapshot_id);
        let index = value.request.candidates.len();
        if index >= value.request.budget.maximum_trials {
            return Err(invalid("experiment trial budget exhausted"));
        }
        value.request.candidates.push(request);
        let trial = submit_trial(&tx, &mut value, index, &key, at_ms, Some(checkpoint))?;
        tx.commit()?;
        Ok(trial)
    }

    /// Submit a distinct recipe on the frozen dataset without consuming the stream's N counter.
    pub fn submit_experiment_trial(
        &self,
        experiment_id: &str,
        candidate_index: usize,
        idempotency_key: &str,
        at_ms: i64,
    ) -> Result<ExperimentTrial> {
        self.writable()?;
        if idempotency_key.trim().is_empty() {
            return Err(invalid("trial idempotency key must be nonempty"));
        }
        let mut connection = self.connection()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous: Option<String> = tx.query_row("SELECT body FROM experiment_trials WHERE experiment_id=? AND (idempotency_key=? OR candidate_index=?)", params![experiment_id,idempotency_key,candidate_index], |row| row.get(0)).optional()?;
        if let Some(previous) = previous {
            let trial: ExperimentTrial = serde_json::from_str(&previous)?;
            if trial.candidate_index != candidate_index || trial.idempotency_key != idempotency_key
            {
                return Err(Error::Conflict(
                    "candidate or idempotency key was already submitted".into(),
                ));
            }
            return Ok(trial);
        }
        let mut experiment = experiment(&tx, experiment_id)?;
        let trial = submit_trial(
            &tx,
            &mut experiment,
            candidate_index,
            idempotency_key,
            at_ms,
            None,
        )?;
        tx.commit()?;
        Ok(trial)
    }
}

fn read<T: DeserializeOwned>(connection: &Connection, sql: &str, key: &str) -> Result<T> {
    let body: Option<String> = connection
        .query_row(sql, [key], |row| row.get(0))
        .optional()?;
    Ok(serde_json::from_str(
        &body.ok_or_else(|| Error::NotFound(key.into()))?,
    )?)
}
fn query<T: DeserializeOwned>(connection: &Connection, sql: &str, key: &str) -> Result<Vec<T>> {
    let mut statement = connection.prepare(sql)?;
    statement
        .query_map([key], |row| row.get::<_, String>(0))?
        .map(|row| Ok(serde_json::from_str(&row?)?))
        .collect()
}
fn experiment(connection: &Connection, key: &str) -> Result<Experiment> {
    read(connection, "SELECT body FROM experiments WHERE id=?", key)
}
fn save_experiment(connection: &Connection, experiment: &Experiment) -> Result<()> {
    if serialized_bytes(&experiment.request)?
        > experiment
            .request
            .budget
            .maximum_dataset_bytes
            .min(64 * 1024 * 1024)
    {
        return Err(invalid(
            "experiment request metadata exceeds its byte budget",
        ));
    }
    connection.execute("INSERT INTO experiments(id,scope,body) VALUES (?,?,?) ON CONFLICT(id) DO UPDATE SET body=excluded.body",params![experiment.id,serde_json::to_string(&experiment.request.stream)?,serde_json::to_string(experiment)?])?;
    Ok(())
}
fn save_trial(connection: &Connection, trial: &ExperimentTrial) -> Result<()> {
    connection.execute(
        "UPDATE experiment_trials SET body=? WHERE id=?",
        params![serde_json::to_string(trial)?, trial.id],
    )?;
    Ok(())
}
fn touch(experiment: &mut Experiment, at_ms: i64) {
    experiment.generation += 1;
    experiment.updated_at_ms = at_ms;
}
fn ensure_running(experiment: &Experiment, at_ms: i64) -> Result<()> {
    if experiment.status != ExperimentStatus::Running || experiment.selected_trial_id.is_some() {
        return Err(Error::Conflict("experiment is not accepting work".into()));
    }
    if at_ms < experiment.updated_at_ms
        || (at_ms as i128 - experiment.created_at_ms as i128)
            > experiment.request.budget.maximum_wall_time_ms as i128
    {
        return Err(invalid(
            "experiment wall time budget exhausted or clock moved backwards",
        ));
    }
    Ok(())
}
fn charge_llm(
    experiment: &mut Experiment,
    tokens: u64,
    cost_micros: u64,
    at_ms: i64,
) -> Result<()> {
    ensure_running(experiment, at_ms)?;
    let new_tokens = experiment
        .usage
        .llm_tokens
        .checked_add(tokens)
        .ok_or_else(|| invalid("LLM token counter overflow"))?;
    let new_cost = experiment
        .usage
        .llm_cost_micros
        .checked_add(cost_micros)
        .ok_or_else(|| invalid("LLM cost counter overflow"))?;
    if new_tokens > experiment.request.budget.maximum_llm_tokens
        || new_cost > experiment.request.budget.maximum_llm_cost_micros
    {
        return Err(invalid("experiment LLM budget exhausted"));
    }
    experiment.usage.llm_tokens = new_tokens;
    experiment.usage.llm_cost_micros = new_cost;
    touch(experiment, at_ms);
    Ok(())
}
fn append_event(
    connection: &Connection,
    experiment_id: &str,
    kind: &str,
    body: Value,
    at_ms: i64,
) -> Result<()> {
    let sequence: u64 = connection.query_row(
        "SELECT count(*) FROM experiment_events WHERE experiment_id=?",
        [experiment_id],
        |row| row.get(0),
    )?;
    if sequence >= 256 {
        return Err(invalid("experiment event limit of 256 reached"));
    }
    let event = serde_json::json!({"sequence":sequence,"kind":kind,"body":body,"at_ms":at_ms});
    if serialized_bytes(&event)? > 64 * 1024 {
        return Err(invalid("experiment event exceeds 64 KiB"));
    }
    connection.execute(
        "INSERT INTO experiment_events(experiment_id,sequence,body) VALUES(?,?,?)",
        params![experiment_id, sequence, serde_json::to_string(&event)?],
    )?;
    Ok(())
}
fn validate_candidate(candidate: &TrainingRequest) -> Result<()> {
    if candidate.engine.trim().is_empty()
        || !candidate.recipe.is_object()
        || !candidate.compute.is_object()
    {
        return Err(invalid("candidate needs engine and object recipe/compute"));
    }
    Ok(())
}
fn validate_request(request: &ExperimentRequest) -> Result<()> {
    if [
        &request.stream.project_id,
        &request.stream.stream_id,
        &request.stream.inspection_version,
    ]
    .iter()
    .any(|s| s.trim().is_empty())
        || !request.spec.is_object()
        || request.goals.primary_metric.trim().is_empty()
        || request.goals.minimum_audited_samples == 0
    {
        return Err(invalid(
            "experiment needs stream, task, metric and audited sample minimum",
        ));
    }
    let b = &request.budget;
    if b.maximum_trials == 0
        || b.maximum_trials > 10_000
        || b.maximum_parallel_trials == 0
        || b.maximum_parallel_trials > b.maximum_trials
        || b.maximum_wall_time_ms == 0
        || b.maximum_training_time_ms == 0
        || b.maximum_dataset_bytes == 0
        || b.maximum_artifact_bytes == 0
        || b.worker_limits.maximum_duration_ms > b.maximum_training_time_ms
        || b.worker_limits.maximum_artifact_bytes > b.maximum_artifact_bytes
    {
        return Err(invalid("invalid experiment budgets"));
    }
    let limits = &b.worker_limits;
    if limits.memory_budget_bytes == 0
        || limits.maximum_artifact_bytes == 0
        || limits.maximum_checkpoint_bytes == 0
        || limits.maximum_duration_ms == 0
        || limits.lease_duration_ms < 300
    {
        return Err(invalid("invalid experiment worker limits"));
    }
    if request.candidates.len() > b.maximum_trials {
        return Err(invalid("candidate count exceeds trial budget"));
    }
    for candidate in &request.candidates {
        validate_candidate(candidate)?;
    }
    for bound in &request.goals.bounds {
        if bound.name.trim().is_empty()
            || bound.minimum.is_none() && bound.maximum.is_none()
            || bound.minimum.is_some_and(|x| !x.is_finite())
            || bound.maximum.is_some_and(|x| !x.is_finite())
            || bound.minimum.zip(bound.maximum).is_some_and(|(a, b)| a > b)
        {
            return Err(invalid("invalid experiment metric bound"));
        }
    }
    Ok(())
}
fn validate_snapshot(snapshot: &DatasetSnapshot) -> Result<()> {
    if snapshot.train.is_empty() || snapshot.validation.is_empty() || snapshot.test.is_empty() {
        return Err(invalid(
            "experiment needs nonempty training, validation and final test splits",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut groups = BTreeMap::new();
    match snapshot.policy {
        SplitPolicy::Group {
            train_fraction,
            validation_fraction,
            ..
        } if !train_fraction.is_finite()
            || !validation_fraction.is_finite()
            || train_fraction <= 0.0
            || validation_fraction <= 0.0
            || train_fraction + validation_fraction >= 1.0 =>
        {
            return Err(invalid("invalid grouped split fractions"));
        }
        SplitPolicy::Time {
            train_end_ms,
            validation_end_ms,
            embargo_ms,
        } if embargo_ms < 0
            || train_end_ms >= validation_end_ms
            || train_end_ms.checked_add(embargo_ms).is_none()
            || validation_end_ms.checked_add(embargo_ms).is_none() =>
        {
            return Err(invalid("invalid temporal split boundaries"));
        }
        _ => {}
    }
    for (partition, samples) in [&snapshot.train, &snapshot.validation, &snapshot.test]
        .into_iter()
        .enumerate()
    {
        for sample in samples {
            if sample.id.trim().is_empty()
                || sample.group_id.trim().is_empty()
                || !sample.accepted
                || sample.label_available_at_ms > snapshot.as_of_ms
                || !ids.insert(&sample.id)
            {
                return Err(invalid(
                    "snapshot has duplicate, unavailable or unaccepted samples",
                ));
            }
            if groups
                .insert(&sample.group_id, partition)
                .is_some_and(|previous| previous != partition)
            {
                return Err(invalid(
                    "experiment split shares a group between partitions",
                ));
            }
            if let SplitPolicy::Time {
                train_end_ms,
                validation_end_ms,
                embargo_ms,
            } = snapshot.policy
            {
                let payload = sample.payload.get("sample").unwrap_or(&sample.payload);
                let start = payload
                    .get("window_start_ms")
                    .and_then(Value::as_i64)
                    .unwrap_or(sample.captured_at_ms);
                let end = payload
                    .get("window_end_ms")
                    .and_then(Value::as_i64)
                    .unwrap_or(sample.captured_at_ms)
                    .max(
                        payload
                            .pointer("/outcome/target_end_ms")
                            .and_then(Value::as_i64)
                            .unwrap_or(sample.captured_at_ms),
                    );
                let valid = start <= end
                    && match partition {
                        0 => end <= train_end_ms,
                        1 => start > train_end_ms + embargo_ms && end <= validation_end_ms,
                        _ => start > validation_end_ms + embargo_ms,
                    };
                if !valid {
                    return Err(invalid(
                        "prepared temporal split violates outcome/window embargo",
                    ));
                }
            }
        }
    }
    Ok(())
}
fn immutable_sample(sample: &TrainingSample) -> Result<Value> {
    let mut value = serde_json::to_value(sample)?;
    let payload = &mut value["payload"];
    let sample_payload = if payload.get("sample").is_some() {
        &mut payload["sample"]
    } else {
        payload
    };
    if let Some(object) = sample_payload.as_object_mut() {
        object.remove("input");
    }
    Ok(value)
}
fn validate_snapshot_truth(connection: &Connection, snapshot: &DatasetSnapshot) -> Result<()> {
    let scope = serde_json::to_string(&snapshot.stream)?;
    for sample in snapshot
        .train
        .iter()
        .chain(&snapshot.validation)
        .chain(&snapshot.test)
    {
        let body: Option<String> = connection
            .query_row(
                "SELECT body FROM annotations WHERE scope=? AND sample_id=? AND revision=?",
                params![scope, sample.id, sample.annotation_revision],
                |row| row.get(0),
            )
            .optional()?;
        let original: TrainingSample = serde_json::from_str(&body.ok_or_else(|| {
            invalid("prepared snapshot sample was not recorded in the annotation ledger")
        })?)?;
        if immutable_sample(&original)? != immutable_sample(sample)? {
            return Err(invalid(
                "prepared snapshot changed accepted target or provenance",
            ));
        }
    }
    Ok(())
}
fn snapshot_hash(snapshot: &DatasetSnapshot) -> Result<String> {
    Ok(digest(&serde_json::to_vec(
        &serde_json::json!({"stream":snapshot.stream,"policy":snapshot.policy,"train":snapshot.train,"validation":snapshot.validation,"test":snapshot.test,"excluded":snapshot.excluded}),
    )?))
}
fn insert_snapshot(connection: &Connection, snapshot: &DatasetSnapshot) -> Result<()> {
    let previous: Option<String> = connection
        .query_row(
            "SELECT body FROM snapshots WHERE id=?",
            [&snapshot.id],
            |row| row.get(0),
        )
        .optional()?;
    let body = serde_json::to_string(snapshot)?;
    if let Some(previous) = previous {
        if previous != body {
            return Err(Error::Conflict(
                "immutable snapshot ID already exists".into(),
            ));
        }
        return Ok(());
    }
    connection.execute(
        "INSERT INTO snapshots(id,scope,body) VALUES (?,?,?)",
        params![snapshot.id, serde_json::to_string(&snapshot.stream)?, body],
    )?;
    Ok(())
}
fn register_dataset(
    connection: &Connection,
    experiment: &mut Experiment,
    snapshot: &DatasetSnapshot,
    preprocessing_manifest: Value,
) -> Result<ExperimentDataset> {
    let previous: Option<String> = connection
        .query_row(
            "SELECT body FROM experiment_datasets WHERE experiment_id=? AND snapshot_id=?",
            params![experiment.id, snapshot.id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(previous) = previous {
        let dataset: ExperimentDataset = serde_json::from_str(&previous)?;
        if dataset.digest != snapshot.digest
            || dataset.preprocessing_manifest != preprocessing_manifest
        {
            return Err(Error::Conflict("dataset registration is immutable".into()));
        }
        return Ok(dataset);
    }
    let mut training = snapshot.clone();
    training.id = id();
    training.test.clear();
    if matches!(
        experiment.request.spec.get("task").and_then(Value::as_str),
        Some("visual_anomaly" | "sensor_anomaly" | "sequence_autoencoder")
    ) {
        for partition in [&mut training.train, &mut training.validation] {
            partition.retain(|sample| {
                let payload = sample.payload.get("sample").unwrap_or(&sample.payload);
                payload
                    .pointer("/annotation/is_anomaly")
                    .and_then(Value::as_bool)
                    == Some(false)
            });
        }
        if training.train.is_empty() || training.validation.is_empty() {
            return Err(invalid(
                "anomaly training requires normal examples in training and validation",
            ));
        }
    }
    let spec = &experiment.request.spec;
    if !crate::repository::training_ready(
        &training,
        &serde_json::json!({"inspection_task":spec["task"],"labels":spec["labels"],"minimum_examples":spec["minimum_examples"],"minimum_examples_per_class":spec["minimum_examples_per_class"]}),
    )? {
        return Err(invalid(
            "experiment training snapshot does not meet task readiness requirements",
        ));
    }
    training.digest = snapshot_hash(&training)?;
    let bytes = serialized_bytes(snapshot)?
        .checked_add(serialized_bytes(&training)?)
        .ok_or_else(|| invalid("dataset size overflow"))?;
    let total = experiment
        .usage
        .dataset_bytes
        .checked_add(bytes)
        .ok_or_else(|| invalid("dataset budget overflow"))?;
    if total > experiment.request.budget.maximum_dataset_bytes {
        return Err(invalid("experiment dataset storage budget exhausted"));
    }
    insert_snapshot(connection, &training)?;
    let dataset = ExperimentDataset {
        snapshot_id: snapshot.id.clone(),
        digest: snapshot.digest.clone(),
        training_snapshot_id: training.id,
        training_digest: training.digest,
        preprocessing_manifest,
    };
    connection.execute(
        "INSERT INTO experiment_datasets(experiment_id,snapshot_id,body) VALUES (?,?,?)",
        params![experiment.id, snapshot.id, serde_json::to_string(&dataset)?],
    )?;
    experiment.usage.dataset_bytes = total;
    Ok(dataset)
}
fn validate_candidate_dataset(
    connection: &Connection,
    experiment: &Experiment,
    request: &TrainingRequest,
) -> Result<ExperimentDataset> {
    let id = request
        .recipe
        .get("dataset_snapshot_id")
        .and_then(Value::as_str)
        .unwrap_or(&experiment.request.snapshot_id);
    let body: Option<String> = connection
        .query_row(
            "SELECT body FROM experiment_datasets WHERE experiment_id=? AND snapshot_id=?",
            params![experiment.id, id],
            |row| row.get(0),
        )
        .optional()?;
    Ok(serde_json::from_str(&body.ok_or_else(|| {
        invalid("candidate dataset is not registered to this experiment")
    })?)?)
}

fn active_jobs(connection: &Connection, experiment_id: &str) -> Result<usize> {
    Ok(connection.query_row("SELECT count(*) FROM experiment_trials t JOIN jobs j ON j.id=t.job_id WHERE t.experiment_id=? AND j.status IN ('queued','running','cancel_requested','paused','interrupted')",[experiment_id],|row|row.get(0))?)
}
fn unsettled_trials(connection: &Connection, experiment_id: &str) -> Result<usize> {
    Ok(connection.query_row("SELECT count(*) FROM experiment_trials t JOIN jobs j ON j.id=t.job_id WHERE t.experiment_id=? AND (json_extract(t.body,'$.status')='scheduled' OR j.status IN ('queued','running','cancel_requested','paused','interrupted'))",[experiment_id],|row|row.get(0))?)
}
fn serialized_bytes<T: serde::Serialize>(value: &T) -> Result<u64> {
    struct Counter(u64);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| std::io::Error::other("serialized size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut count = Counter(0);
    serde_json::to_writer(&mut count, value)?;
    Ok(count.0)
}
fn update_job(connection: &Connection, job: &TrainingJob) -> Result<()> {
    let status = serde_json::to_value(job.status)?;
    connection.execute(
        "UPDATE jobs SET status=?,body=? WHERE id=?",
        params![
            status
                .as_str()
                .ok_or_else(|| invalid("invalid job state"))?,
            serde_json::to_string(job)?,
            job.id
        ],
    )?;
    if job.status.is_terminal() || job.status == JobStatus::Queued {
        connection.execute(
            "DELETE FROM resource_reservations WHERE job_id=?",
            [&job.id],
        )?;
    }
    Ok(())
}
fn settle_time(
    experiment: &mut Experiment,
    trial: &mut ExperimentTrial,
    milliseconds: u64,
) -> Result<()> {
    experiment.usage.reserved_training_time_ms = experiment
        .usage
        .reserved_training_time_ms
        .checked_sub(trial.reserved_training_time_ms)
        .ok_or_else(|| invalid("inconsistent training reservation"))?;
    experiment.usage.training_time_ms = experiment
        .usage
        .training_time_ms
        .checked_add(milliseconds)
        .ok_or_else(|| invalid("training time counter overflow"))?;
    trial.training_time_ms = trial.training_time_ms.saturating_add(milliseconds);
    experiment.usage.reserved_artifact_bytes = experiment
        .usage
        .reserved_artifact_bytes
        .checked_sub(trial.reserved_artifact_bytes)
        .ok_or_else(|| invalid("inconsistent artifact reservation"))?;
    trial.reserved_artifact_bytes = 0;
    trial.reserved_training_time_ms = 0;
    Ok(())
}
fn satisfies(metrics: &BTreeMap<String, f64>, bound: &MetricBound) -> bool {
    metrics.get(&bound.name).is_some_and(|v| {
        v.is_finite()
            && bound.minimum.is_none_or(|min| *v >= min)
            && bound.maximum.is_none_or(|max| *v <= max)
    })
}
fn better(
    candidate: &ExperimentTrial,
    previous: &ExperimentTrial,
    goals: &ExperimentGoals,
) -> bool {
    let eligible = |trial: &ExperimentTrial| {
        trial
            .validation_metrics
            .get("audited_samples")
            .is_some_and(|n| *n >= goals.minimum_audited_samples as f64)
            && goals
                .bounds
                .iter()
                .all(|bound| satisfies(&trial.validation_metrics, bound))
    };
    if eligible(candidate) != eligible(previous) {
        return eligible(candidate);
    }
    let a = candidate.validation_metrics[&goals.primary_metric];
    let b = previous.validation_metrics[&goals.primary_metric];
    if a == b {
        return candidate.candidate_index < previous.candidate_index;
    }
    match goals.direction {
        MetricDirection::Maximize => a > b,
        MetricDirection::Minimize => a < b,
    }
}

fn submit_trial(
    tx: &Connection,
    experiment: &mut Experiment,
    candidate_index: usize,
    idempotency_key: &str,
    at_ms: i64,
    checkpoint: Option<CheckpointRef>,
) -> Result<ExperimentTrial> {
    let experiment_id = experiment.id.clone();
    ensure_running(experiment, at_ms)?;
    crate::learning_repository::validate_learning_dispatch(tx, experiment)?;
    let budget = &experiment.request.budget;
    let mut request = experiment
        .request
        .candidates
        .get(candidate_index)
        .ok_or_else(|| invalid("unknown experiment candidate"))?
        .clone();
    let dataset = validate_candidate_dataset(&tx, experiment, &request)?;
    request.recipe["preprocessing_manifest"] = dataset.preprocessing_manifest.clone();
    let mut spec = experiment.request.spec.clone();
    let training: DatasetSnapshot = read(
        &tx,
        "SELECT body FROM snapshots WHERE id=?",
        &dataset.training_snapshot_id,
    )?;
    let sample = training
        .train
        .first()
        .ok_or_else(|| invalid("training dataset is empty"))?;
    let payload = sample.payload.get("sample").unwrap_or(&sample.payload);
    if let Some(shape) = payload.pointer("/input/shape") {
        spec["input_shape"] = shape.clone();
    }
    request.recipe["inspection_spec"] = spec;
    let deadline = i64::try_from(
        experiment.created_at_ms as i128 + experiment.request.budget.maximum_wall_time_ms as i128,
    )
    .map_err(|_| invalid("experiment deadline overflows the supported clock"))?;
    request.recipe["experiment_deadline_ms"] = serde_json::json!(deadline);
    let active = unsettled_trials(tx, &experiment_id)?;
    if active >= budget.maximum_parallel_trials {
        return Err(Error::Conflict(
            "experiment concurrency budget is occupied".into(),
        ));
    }
    if experiment.usage.submitted_trials >= budget.maximum_trials {
        return Err(invalid("experiment trial budget exhausted"));
    }
    let reserved = budget.worker_limits.maximum_duration_ms;
    let total = experiment
        .usage
        .training_time_ms
        .checked_add(experiment.usage.reserved_training_time_ms)
        .and_then(|n| n.checked_add(reserved))
        .ok_or_else(|| invalid("training budget overflow"))?;
    if total > budget.maximum_training_time_ms {
        return Err(invalid("experiment training time budget exhausted"));
    }
    let reserved_artifact = budget.worker_limits.maximum_artifact_bytes;
    if experiment
        .usage
        .artifact_bytes
        .saturating_add(experiment.usage.reserved_artifact_bytes)
        .saturating_add(reserved_artifact)
        > budget.maximum_artifact_bytes
    {
        return Err(invalid("experiment artifact budget exhausted"));
    }
    let mut checkpoint = checkpoint;
    if let Some(checkpoint) = &mut checkpoint {
        checkpoint.request_digest = digest(&serde_json::to_vec(&request)?);
    }
    let trial_id = id();
    let job = TrainingJob {
        id: id(),
        stream: experiment.request.stream.clone(),
        snapshot_id: dataset.training_snapshot_id,
        request,
        status: JobStatus::Queued,
        generation: 0,
        lease_owner: None,
        lease_expires_at_ms: None,
        checkpoint,
        artifact_id: None,
        error: None,
        progress: Value::Null,
        created_at_ms: at_ms,
        updated_at_ms: at_ms,
    };
    // The scheduling scope is internal. Sample/artifact stream identity stays unchanged.
    tx.execute(
        "INSERT INTO jobs(id,scope,status,body) VALUES (?,?,'queued',?)",
        params![
            job.id,
            format!("experiment:{experiment_id}:{trial_id}"),
            serde_json::to_string(&job)?
        ],
    )?;
    let trial = ExperimentTrial {
        id: trial_id,
        experiment_id: experiment_id.clone(),
        candidate_index,
        idempotency_key: idempotency_key.into(),
        job_id: job.id,
        dataset_snapshot_id: dataset.snapshot_id,
        snapshot_id: job.snapshot_id,
        artifact_id: None,
        status: ExperimentTrialStatus::Scheduled,
        validation_metrics: BTreeMap::new(),
        training_time_ms: 0,
        reserved_training_time_ms: reserved,
        reserved_artifact_bytes: reserved_artifact,
        error: None,
        created_at_ms: at_ms,
        updated_at_ms: at_ms,
    };
    tx.execute("INSERT INTO experiment_trials(id,experiment_id,job_id,idempotency_key,candidate_index,body) VALUES (?,?,?,?,?,?)",params![trial.id,experiment_id,trial.job_id,idempotency_key,candidate_index,serde_json::to_string(&trial)?])?;
    experiment.usage.submitted_trials += 1;
    experiment.usage.reserved_training_time_ms += reserved;
    experiment.usage.reserved_artifact_bytes += reserved_artifact;
    touch(experiment, at_ms);
    save_experiment(&tx, experiment)?;
    Ok(trial)
}
