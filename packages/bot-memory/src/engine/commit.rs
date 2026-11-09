use super::*;
use crate::facts::UpdateAction;
use crate::tools::forget_facts;

#[derive(Debug)]
pub(crate) enum CommitOutcome<T> {
    Applied(T, CompactionState),
    Stale(CompactionState),
}

impl<T> CommitOutcome<T> {
    pub(super) fn applied(self, expected: u64) -> Result<(T, CompactionState)> {
        match self {
            Self::Applied(value, state) => Ok((value, state)),
            Self::Stale(state) => Err(StateConflict {
                expected,
                found: state.revision,
            }
            .into()),
        }
    }
}

impl MemoryEngine {
    /// Read the revision before the tables. A concurrent commit invalidates this entire draft,
    /// including one that observed a partially applied multi-table change.
    pub(super) async fn snapshot(&self) -> Result<CompactionState> {
        self.stores.state.initialize().await?;
        let state = self.stores.state.load().await?;
        self.stores.refresh().await?;
        Ok(state)
    }

    /// Background work defers a live storage commit instead of spending model calls on a
    /// snapshot that will be invalidated when that commit finishes.
    pub(super) async fn background_snapshot(&self, now: DateTime<Utc>) -> Result<CompactionState> {
        let state = self.snapshot().await?;
        if let Some(lease) = state.lease.as_ref().filter(|lease| lease.until > now) {
            return Err(MemoryBusy { until: lease.until }.into());
        }
        Ok(state)
    }

    /// Pruning by a concurrent forget can make a draft's Lance snapshot unreadable. Treat
    /// that as stale preparation, while preserving genuine errors on an unchanged revision.
    pub(super) async fn check_preparation<T>(
        &self,
        expected: u64,
        preparation: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let result = preparation.await;
        let current = self.stores.state.load().await?;
        if current.revision != expected {
            return Err(StateConflict {
                expected,
                found: current.revision,
            }
            .into());
        }
        result
    }

    /// Own the storage phase independently of the caller. Cancelling a turn can discard its
    /// preparation, but cannot abandon an acquired lease or half of an in-flight commit.
    pub(super) async fn commit<T, F, Fut>(
        &self,
        expected: u64,
        now: DateTime<Utc>,
        apply: F,
    ) -> Result<CommitOutcome<T>>
    where
        T: Send + 'static,
        F: FnOnce(Arc<MemoryStores>, CompactionState) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(T, CompactionState)>> + Send + 'static,
    {
        self.commit_with_wait(expected, now, WRITE_LEASE_WAIT, apply)
            .await
    }

    pub(super) async fn commit_with_wait<T, F, Fut>(
        &self,
        expected: u64,
        now: DateTime<Utc>,
        max_wait: WaitDuration,
        apply: F,
    ) -> Result<CommitOutcome<T>>
    where
        T: Send + 'static,
        F: FnOnce(Arc<MemoryStores>, CompactionState) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(T, CompactionState)>> + Send + 'static,
    {
        self.commit_options(expected, now, max_wait, false, apply)
            .await
    }

    pub(super) async fn commit_deletion<T, F, Fut>(
        &self,
        expected: u64,
        now: DateTime<Utc>,
        apply: F,
    ) -> Result<CommitOutcome<T>>
    where
        T: Send + 'static,
        F: FnOnce(Arc<MemoryStores>, CompactionState) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(T, CompactionState)>> + Send + 'static,
    {
        self.commit_options(expected, now, WRITE_LEASE_WAIT, true, apply)
            .await
    }

    async fn commit_options<T, F, Fut>(
        &self,
        expected: u64,
        now: DateTime<Utc>,
        max_wait: WaitDuration,
        invalidate_saves: bool,
        apply: F,
    ) -> Result<CommitOutcome<T>>
    where
        T: Send + 'static,
        F: FnOnce(Arc<MemoryStores>, CompactionState) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(T, CompactionState)>> + Send + 'static,
    {
        let engine = self.clone();
        flow_like_types::tokio::spawn(async move {
            let started = Instant::now();
            let _work = engine.work.lock().await;
            let waiting = Instant::now();
            let mut claimed = loop {
                let mut current = engine.stores.state.load().await?;
                if current.revision != expected {
                    return Ok(CommitOutcome::Stale(current));
                }
                if invalidate_saves {
                    current.forget_generation = current.forget_generation.checked_add(1)
                        .ok_or_else(|| flow_like_types::anyhow!("Memory deletion generation exhausted"))?;
                }
                let claim_now = now + Duration::from_std(started.elapsed())?;
                match engine.claim(current, claim_now).await {
                    Ok(claimed) => break claimed,
                    Err(error) if error.is::<MemoryBusy>() || error.is::<StateConflict>() => {
                        let remaining = max_wait.saturating_sub(waiting.elapsed());
                        if remaining.is_zero() {
                            return Err(error);
                        }
                        sleep(WRITE_LEASE_POLL.min(remaining)).await;
                    }
                    Err(error) => return Err(error),
                }
            };
            let work = apply(engine.stores.clone(), claimed.clone());
            flow_like_types::tokio::pin!(work);
            let mut heartbeat = flow_like_types::tokio::time::interval_at(
                Instant::now() + MAINTENANCE_HEARTBEAT,
                MAINTENANCE_HEARTBEAT,
            );
            let result = loop {
                flow_like_types::tokio::select! {
                    result = &mut work => break result,
                    _ = heartbeat.tick() => {
                        let renewed = CompactionState {
                            lease: Some(Lease {
                                holder: engine.id.clone(),
                                until: now + Duration::from_std(started.elapsed())? + Duration::minutes(LEASE_MINUTES),
                            }),
                            ..claimed.clone()
                        };
                        match engine.stores.state.save(&renewed).await {
                            Ok(state) => claimed = state,
                            Err(error) => break Err(error),
                        }
                    }
                }
            };
            match result {
                Ok((value, mut state)) => {
                    state.revision = claimed.revision;
                    state.forget_generation = claimed.forget_generation;
                    if invalidate_saves {
                        // Persist the sanitized state before pruning, while the lease still
                        // excludes other writers. A reset must remove old Now blocks too.
                        state.lease = claimed.lease.clone();
                        match engine.stores.state.save(&state).await {
                            Ok(saved) => {
                                claimed = saved.clone();
                                state = saved;
                            }
                            Err(error) => {
                                engine.release_failed_commit(claimed).await;
                                return Err(error);
                            }
                        }
                        if let Err(error) = engine.stores.prune().await {
                            tracing::warn!("Pruning removed memory versions failed: {error:#}");
                        }
                    }
                    match engine.finish(state).await {
                        Ok(state) => Ok(CommitOutcome::Applied(value, state)),
                        Err(error) => {
                            engine.release_failed_commit(claimed).await;
                            Err(error)
                        }
                    }
                }
                Err(error) => {
                    engine.release_failed_commit(claimed).await;
                    Err(error)
                }
            }
        }).await?
    }

    async fn release_failed_commit(&self, claimed: CompactionState) {
        if let Err(error) = self.finish(claimed).await {
            tracing::warn!("Giving the memory lease back failed; it runs out by itself: {error:#}");
        }
    }

    pub(crate) async fn save_fact(
        &self,
        fact: Fact,
        now: DateTime<Utc>,
    ) -> Result<(Fact, UpdateAction)> {
        let started = Instant::now();
        let mut initial_snapshot = None;
        for attempt in 0..CLAIM_ATTEMPTS {
            let state = self.snapshot().await?;
            let revision = state.revision;
            let (initial_revision, forget_generation) =
                *initial_snapshot.get_or_insert((revision, state.forget_generation));
            if state.forget_generation != forget_generation {
                return Err(StateConflict {
                    expected: initial_revision,
                    found: revision,
                }
                .into());
            }
            let prepared = match self
                .check_preparation(
                    revision,
                    prepare_facts(
                        &self.stores.facts,
                        self.llm.as_ref(),
                        vec![fact.clone()],
                        self.timezone,
                        now,
                        true,
                    ),
                )
                .await
            {
                Ok(prepared) => prepared,
                Err(error) if error.is::<StateConflict>() && attempt + 1 < CLAIM_ATTEMPTS => {
                    continue;
                }
                Err(error) => return Err(error),
            };
            let candidate = fact.clone();
            let commit_now = now + Duration::from_std(started.elapsed())?;
            match self
                .commit(revision, commit_now, move |stores, state| async move {
                    let report = apply_prepared_facts(&stores.facts, prepared, now).await?;
                    let action = report
                        .decisions
                        .into_iter()
                        .next()
                        .map_or(UpdateAction::Add, |decision| decision.action);
                    let id = match &action {
                        UpdateAction::Merge { target } => target.as_str(),
                        _ => &candidate.id,
                    };
                    let stored = stores.facts.get(id).await?.ok_or_else(|| {
                        flow_like_types::anyhow!(
                            "The saved memory is no longer available; search before confirming it"
                        )
                    })?;
                    Ok(((stored, action), state))
                })
                .await?
            {
                CommitOutcome::Applied(value, _) => return Ok(value),
                CommitOutcome::Stale(state)
                    if state.forget_generation != forget_generation
                        || attempt + 1 == CLAIM_ATTEMPTS =>
                {
                    return Err(StateConflict {
                        expected: revision,
                        found: state.revision,
                    }
                    .into());
                }
                CommitOutcome::Stale(_) => {}
            }
        }
        unreachable!("the last attempt returns its conflict")
    }

    pub(crate) async fn forget_fact(&self, id: &str, now: DateTime<Utc>) -> Result<Fact> {
        let started = Instant::now();
        for attempt in 0..CLAIM_ATTEMPTS {
            let state = self.snapshot().await?;
            let revision = state.revision;
            let id = id.to_owned();
            let commit_now = now + Duration::from_std(started.elapsed())?;
            match self
                .commit_deletion(revision, commit_now, move |stores, state| async move {
                    let fact = stores
                        .facts
                        .get(&id)
                        .await?
                        .ok_or_else(|| flow_like_types::anyhow!("No fact {id}"))?;
                    forget_facts(&stores, std::slice::from_ref(&fact), false).await?;
                    Ok((fact, state))
                })
                .await?
            {
                CommitOutcome::Applied(value, _) => return Ok(value),
                CommitOutcome::Stale(state) if attempt + 1 == CLAIM_ATTEMPTS => {
                    return Err(StateConflict {
                        expected: revision,
                        found: state.revision,
                    }
                    .into());
                }
                CommitOutcome::Stale(_) => {}
            }
        }
        unreachable!("the last attempt returns its conflict")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{HashEmbedder, StubLlm};
    use flow_like_types::tokio::sync::Notify;

    async fn engine(dir: &tempfile::TempDir) -> MemoryEngine {
        let stores = MemoryStores::open_local(dir.path(), Arc::new(HashEmbedder::new("hash", 16)))
            .await
            .unwrap();
        MemoryEngine::new(
            stores,
            Arc::new(StubLlm::new()),
            CompactionConfig::default(),
            Tz::UTC,
        )
    }

    #[tokio::test]
    async fn cancelling_the_caller_does_not_abandon_an_acquired_commit() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir).await;
        let state = engine.snapshot().await.unwrap();
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let caller = {
            let engine = engine.clone();
            let entered = entered.clone();
            let release = release.clone();
            flow_like_types::tokio::spawn(async move {
                engine
                    .commit(state.revision, Utc::now(), move |_, mut state| async move {
                        entered.notify_one();
                        release.notified().await;
                        state.compactions = 1;
                        Ok(((), state))
                    })
                    .await
            })
        };
        entered.notified().await;
        assert!(engine.stores.state.load().await.unwrap().lease.is_some());
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        release.notify_one();
        flow_like_types::tokio::time::timeout(WaitDuration::from_secs(5), async {
            loop {
                let state = engine.stores.state.load().await.unwrap();
                if state.lease.is_none() {
                    assert_eq!(state.compactions, 1);
                    break;
                }
                sleep(WaitDuration::from_millis(10)).await;
            }
        })
        .await
        .expect("the owned commit finishes and releases its lease");
    }

    #[tokio::test]
    async fn failed_deletion_releases_lease_and_still_invalidates_older_saves() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine(&dir).await;
        let before = engine.snapshot().await.unwrap();
        let result = engine
            .commit_deletion(before.revision, Utc::now(), |_, _| async {
                Err::<((), CompactionState), _>(flow_like_types::anyhow!("storage failure"))
            })
            .await;
        assert!(result.is_err());
        let after = engine.stores.state.load().await.unwrap();
        assert!(after.lease.is_none());
        assert!(after.revision > before.revision);
        assert_eq!(after.forget_generation, before.forget_generation + 1);
    }
}
