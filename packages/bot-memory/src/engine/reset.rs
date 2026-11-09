use super::*;

impl MemoryEngine {
    /// Clears this conversation's memory while retaining its revision, so drafts prepared
    /// before the reset cannot restore the removed content.
    pub async fn reset(&self, now: DateTime<Utc>) -> Result<()> {
        let started = Instant::now();
        for attempt in 0..CLAIM_ATTEMPTS {
            let revision = self.stores.state.load().await?.revision;
            let commit_now = now + Duration::from_std(started.elapsed())?;
            match self
                .commit_deletion(revision, commit_now, |stores, claimed| async move {
                    stores.clear_content().await?;
                    let state = CompactionState {
                        revision: claimed.revision,
                        lease: claimed.lease,
                        ..CompactionState::default()
                    };
                    Ok(((), state))
                })
                .await?
            {
                CommitOutcome::Applied((), _) => return Ok(()),
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
