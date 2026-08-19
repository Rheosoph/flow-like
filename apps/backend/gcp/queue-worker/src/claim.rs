//! The Firestore claim that closes the ack-deadline / request-timeout gap.
//!
//! A Pub/Sub push subscription caps its ack deadline at 600 seconds. A Cloud Run
//! request may run for 3600. Anything that outlives the deadline is therefore
//! redelivered *while the first attempt is still working*, and Pub/Sub offers no
//! lease extension on the push path at all — there is no `modifyAckDeadline` to
//! call, because the worker never holds an `ackId`. Without a guard, one
//! forty-minute execution becomes four concurrent executions, and the file
//! tracker's size ledger is double-counted every time.
//!
//! Firestore supplies the missing lock. Every delivery first writes a claim
//! document keyed by the Pub/Sub `messageId` under a create-if-absent
//! precondition, so exactly one delivery can win the create; every other
//! delivery of that `messageId` reads the claim and settles without repeating
//! the work. The document's server-owned `updateTime` is the concurrency token
//! and it behaves exactly like the Azure pop receipt it replaces: it rotates on
//! every successful write and the previous value dies with it. Losing a
//! compare-and-swap therefore means what a lost pop receipt meant — another
//! replica owns this message now — and the worker stops instead of settling
//! something it no longer holds.
//!
//! Every write goes through `commit`, not through the single-document helpers,
//! for one reason: only `commit` returns the resulting `updateTime`, and without
//! it the next compare-and-swap has nothing to swap against.
//!
//! What the claim must never do is *lose* a message. On AWS and Azure the
//! visibility timeout guarantees that a message whose consumer died comes back;
//! here the only thing that comes back is a redelivery, and if a redelivery is
//! answered with an ack because the claim still says `in_progress`, the message
//! is gone for good while the run stays `Running` until its TTL. Two mechanisms
//! keep that from happening. A [`ClaimGuard`] that is dropped without being
//! settled — a panic in a catalog node, a cancelled task — releases the claim
//! from `Drop` on a best-effort basis; and a redelivery that finds a live claim
//! does not ack it but *waits* on it ([`ClaimStore::wait_while_in_flight`]),
//! re-entering `acquire` once the owner has settled or stopped heartbeating, and
//! nacking if the wait budget runs out. Nothing acks a message on the strength of
//! somebody else's heartbeat alone.

use flow_like_gcp_data::firestore::{
    Document, FirestoreClient, FirestoreError, MutationOutcome, validate_collection_id,
};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

pub const DEFAULT_CLAIMS_COLLECTION: &str = "pubsub-claims";

/// A create that lost, followed by a read that found nothing, means the TTL
/// collected the document in between. Three rounds is more than enough to
/// resolve that; a fourth would mean the collection is being rewritten faster
/// than it can be read, which is a condition to report rather than to spin on.
const MAX_ACQUIRE_ROUNDS: u32 = 3;

/// The outcome code a guard writes when it is dropped without being settled.
/// It is what an operator greps for to find claims released by a panic or a
/// cancellation rather than by a decision.
pub const GUARD_DROPPED_CODE: &str = "guard_dropped";

const STATE_IN_PROGRESS: &str = "in_progress";
const STATE_COMPLETED: &str = "completed";
const STATE_RELEASED: &str = "released";
const STATE_DEAD_LETTERED: &str = "dead_lettered";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimState {
    InProgress,
    Completed,
    Released,
    DeadLettered,
}

impl ClaimState {
    fn parse(value: &str) -> Option<Self> {
        match value {
            STATE_IN_PROGRESS => Some(Self::InProgress),
            STATE_COMPLETED => Some(Self::Completed),
            STATE_RELEASED => Some(Self::Released),
            STATE_DEAD_LETTERED => Some(Self::DeadLettered),
            _ => None,
        }
    }

    fn wire_name(self) -> &'static str {
        match self {
            Self::InProgress => STATE_IN_PROGRESS,
            Self::Completed => STATE_COMPLETED,
            Self::Released => STATE_RELEASED,
            Self::DeadLettered => STATE_DEAD_LETTERED,
        }
    }
}

impl Display for ClaimState {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.wire_name())
    }
}

#[derive(Debug)]
pub enum ClaimError {
    /// The claim is no longer this delivery's. Never settle after seeing this.
    Lost,
    Store(FirestoreError),
    Configuration(String),
}

impl Display for ClaimError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lost => formatter.write_str("the claim was taken over by another delivery"),
            Self::Store(error) => write!(formatter, "claim store failure: {error}"),
            Self::Configuration(detail) => write!(formatter, "claim configuration: {detail}"),
        }
    }
}

impl std::error::Error for ClaimError {}

impl From<FirestoreError> for ClaimError {
    fn from(error: FirestoreError) -> Self {
        Self::Store(error)
    }
}

/// What a delivery is allowed to do with the message it just received.
#[derive(Debug)]
pub enum ClaimDecision {
    /// This delivery owns the message. Process it, then settle through the guard.
    Owned(ClaimGuard),
    /// A previous delivery already carried the message to a terminal state.
    Settled {
        state: ClaimState,
        attempts: i64,
        outcome_code: String,
    },
    /// Another delivery is working on it right now and is still heartbeating.
    /// This is a fact, not a licence to ack: see
    /// [`ClaimStore::wait_while_in_flight`].
    InFlight {
        owner: String,
        attempts: i64,
        age_seconds: i64,
    },
}

/// How a wait on somebody else's claim ended.
#[derive(Debug, PartialEq, Eq)]
pub enum InFlightWait {
    /// The claim is no longer a live `in_progress` held by another owner: it
    /// was settled, released, taken over, collected by the TTL, or its heartbeat
    /// aged past the staleness window. `acquire` must be re-entered to learn
    /// which.
    Changed,
    /// The owner was still heartbeating when the budget ran out. The delivery
    /// must be nacked so the broker re-evaluates it later; it must not be acked.
    StillInFlight,
}

/// Proof of ownership for one message. Holding one of these is what permits
/// work, and the `update_time` inside is what permits settling.
///
/// A guard that goes out of scope unsettled releases the claim from `Drop`. That
/// is what turns a panic inside user flow code, or a task cancelled from under
/// the handler, into a `released` claim the next redelivery can take over
/// immediately — instead of an `in_progress` claim that has to age out first.
/// It cannot cover SIGKILL or an OOM kill; those are what the staleness window
/// and [`ClaimStore::wait_while_in_flight`] are for.
#[derive(Debug)]
pub struct ClaimGuard {
    store: Arc<ClaimStoreInner>,
    id: String,
    document: ClaimDocument,
    /// The token that rotates on every write. `None` would mean "write blind",
    /// so the store treats its absence as a lost claim rather than as a licence.
    update_time: Option<String>,
    /// Set by every explicit settle before it returns. `Drop` reads it to decide
    /// whether the claim still needs handing back.
    settled: bool,
}

impl ClaimGuard {
    /// How many deliveries have actually attempted this message. Deliveries shed
    /// as duplicates are deliberately not counted: a job that legitimately
    /// outlives the ack deadline must not be dead-lettered for being slow.
    pub fn attempts(&self) -> i64 {
        self.document.attempts
    }
}

impl Drop for ClaimGuard {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        // Never panic here: a panic in `Drop` during an unwind aborts the
        // process, which would take every other in-flight delivery on this
        // instance down with the one that failed. Everything below is
        // infallible; the write itself happens on a spawned task.
        let store = Arc::clone(&self.store);
        let id = std::mem::take(&mut self.id);
        let document = self.document.clone();
        let update_time = self.update_time.take();

        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                tracing::warn!(
                    claim_id = %id,
                    message_id = %document.message_id,
                    "claim guard dropped without a settle; releasing the claim"
                );
                handle.spawn(async move {
                    store
                        .settle_document(
                            &id,
                            document,
                            update_time,
                            ClaimState::Released,
                            GUARD_DROPPED_CODE,
                        )
                        .await;
                });
            }
            // No runtime means the process is already coming down; the
            // staleness window is what recovers the claim in that case.
            Err(_) => tracing::error!(
                claim_id = %id,
                message_id = %document.message_id,
                "claim guard dropped outside a runtime; the staleness window will recover it"
            ),
        }
    }
}

/// The stored claim. `expires_at` is absent on purpose — the TTL field is a
/// typed `timestampValue` that no `serde` shape can express, so it is written by
/// `set_expires_at` inside the client instead.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ClaimDocument {
    message_id: String,
    subscription: String,
    workload: String,
    state: String,
    owner: String,
    attempts: i64,
    claimed_at_ms: i64,
    heartbeat_at_ms: i64,
    #[serde(default)]
    outcome_code: String,
}

/// Everything a write needs. Shared between the store and every guard it hands
/// out, so that a guard can release itself from `Drop` without a reference back
/// to the store — a plain `impl Drop` on a guard that owns nothing has nothing
/// to write with.
#[derive(Debug)]
struct ClaimStoreInner {
    firestore: FirestoreClient,
    collection: String,
    /// How long a settled claim is kept. It bounds the window in which a
    /// redelivery can still be recognised as a duplicate, so it must comfortably
    /// exceed both the subscription's message retention and the worker's own
    /// processing deadline.
    retention: Duration,
}

pub struct ClaimStore {
    inner: Arc<ClaimStoreInner>,
    workload: String,
    owner: String,
    /// After this long without a heartbeat the owner is presumed dead and the
    /// claim may be taken over. It has to exceed several heartbeat intervals, or
    /// one slow Firestore write hands a live job to a second replica.
    stale_after: Duration,
}

impl ClaimStore {
    pub fn new(
        firestore: FirestoreClient,
        collection: String,
        workload: String,
        stale_after: Duration,
        retention: Duration,
    ) -> Result<Self, ClaimError> {
        validate_collection_id(&collection).map_err(|error| {
            ClaimError::Configuration(format!("invalid claims collection: {error}"))
        })?;

        // Cloud Run's revision name is what an operator greps for when two
        // replicas fight over a claim, and the UUID is what separates two
        // instances of the same revision.
        let revision = std::env::var("K_REVISION")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "local".to_string());
        let owner = format!("{revision}/{}", uuid::Uuid::new_v4());

        Ok(Self {
            inner: Arc::new(ClaimStoreInner {
                firestore,
                collection,
                retention,
            }),
            workload,
            owner,
            stale_after,
        })
    }

    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Decide what this delivery may do with `message_id`.
    pub async fn acquire(
        &self,
        subscription: &str,
        message_id: &str,
    ) -> Result<ClaimDecision, ClaimError> {
        let id = claim_id(subscription, message_id);

        for _ in 0..MAX_ACQUIRE_ROUNDS {
            let now = now_ms();
            let fresh = ClaimDocument {
                message_id: message_id.to_string(),
                subscription: subscription.to_string(),
                workload: self.workload.clone(),
                state: STATE_IN_PROGRESS.to_string(),
                owner: self.owner.clone(),
                attempts: 1,
                claimed_at_ms: now,
                heartbeat_at_ms: now,
                outcome_code: String::new(),
            };

            // Create-if-absent. This single precondition is the whole mutual
            // exclusion: Firestore resolves concurrent creates of the same
            // document name server-side, so exactly one delivery is told
            // `Applied` no matter how many arrive at once.
            match self
                .inner
                .write(&id, &fresh, Precondition::MustNotExist)
                .await?
            {
                (MutationOutcome::Applied, update_time) => {
                    return Ok(ClaimDecision::Owned(self.guard(id, fresh, update_time)));
                }
                (MutationOutcome::Conflict | MutationOutcome::PreconditionFailed, _) => {}
                (MutationOutcome::NotFound, _) => {
                    return Err(ClaimError::Store(FirestoreError::Transport(
                        "a create-if-absent claim write reported NOT_FOUND".to_string(),
                    )));
                }
            }

            let Some(existing) = self.inner.read(&id).await? else {
                // The TTL collected the document between the create and the
                // read. Going around again re-runs the create, which is the only
                // operation that can win the message.
                continue;
            };

            let state = parse_state(&id, &existing.value.state);
            let age_seconds = heartbeat_age_seconds(&existing.value);

            match state {
                ClaimState::Completed => {
                    return Ok(ClaimDecision::Settled {
                        state,
                        attempts: existing.value.attempts,
                        outcome_code: existing.value.outcome_code.clone(),
                    });
                }
                ClaimState::DeadLettered => {
                    // The delivery counter keeps moving even here. It is the only
                    // counter that exists when the subscription has no
                    // dead-letter policy, and it is what eventually lets the
                    // worker stop a message the broker will never take away.
                    let attempts = self
                        .bump_dead_letter_attempts(
                            &id,
                            &existing.value,
                            existing.update_time.as_deref(),
                        )
                        .await;
                    return Ok(ClaimDecision::Settled {
                        state,
                        attempts,
                        outcome_code: existing.value.outcome_code.clone(),
                    });
                }
                ClaimState::InProgress if age_seconds < self.stale_after.as_secs() as i64 => {
                    return Ok(ClaimDecision::InFlight {
                        owner: existing.value.owner.clone(),
                        attempts: existing.value.attempts,
                        age_seconds,
                    });
                }
                // A stale in-progress claim is an owner that died mid-flight; a
                // released claim is an owner that failed and handed it back.
                // Both are taken over the same way, and the compare-and-swap is
                // what stops two replicas from taking over at once.
                ClaimState::InProgress | ClaimState::Released => {
                    let now = now_ms();
                    let mut taken = existing.value.clone();
                    taken.state = STATE_IN_PROGRESS.to_string();
                    taken.owner = self.owner.clone();
                    taken.attempts = existing.value.attempts.saturating_add(1);
                    taken.claimed_at_ms = now;
                    taken.heartbeat_at_ms = now;
                    taken.outcome_code = String::new();

                    match self
                        .inner
                        .write(
                            &id,
                            &taken,
                            Precondition::UpdateTime(existing.update_time.as_deref()),
                        )
                        .await?
                    {
                        (MutationOutcome::Applied, update_time) => {
                            if state == ClaimState::InProgress {
                                tracing::warn!(
                                    claim_id = %id,
                                    previous_owner = %existing.value.owner,
                                    age_seconds,
                                    "took over a claim whose owner stopped heartbeating"
                                );
                            }
                            return Ok(ClaimDecision::Owned(self.guard(id, taken, update_time)));
                        }
                        // Another replica took over first. Re-read rather than
                        // guess: the winner may already have finished.
                        _ => continue,
                    }
                }
            }
        }

        Err(ClaimError::Store(FirestoreError::Transport(format!(
            "claim for message {message_id} could not be resolved in {MAX_ACQUIRE_ROUNDS} rounds"
        ))))
    }

    /// Wait for a claim another delivery holds to stop being a live
    /// `in_progress`, or for `budget` to run out — whichever comes first.
    ///
    /// This is the redelivery side of the ack-deadline gap, and it replaces the
    /// ack that used to stand here. Acking on an observed heartbeat is unsound:
    /// acking any live `ackId` removes the message from the subscription, so if
    /// the owner then dies (OOM, SIGKILL, instance replacement) the message is
    /// gone with no DLQ copy and no terminal status. Instead the redelivery
    /// parks and polls the claim: the owner finishing (`completed`,
    /// `dead_lettered`, `released`) or its heartbeat ageing past `stale_after`
    /// both return [`InFlightWait::Changed`], and the caller re-enters
    /// [`ClaimStore::acquire`], which either recognises the settled duplicate or
    /// takes the message over on *this* delivery. An owner that is still
    /// heartbeating when the budget ends returns [`InFlightWait::StillInFlight`]
    /// and the caller nacks, so the broker brings the message back and the
    /// question is asked again.
    ///
    /// The budget must fit inside the request Cloud Run gave this delivery, and
    /// `stale_after` is the natural value: a dead owner is detected within it by
    /// construction, and a live owner is one the caller should not outwait
    /// anyway. `poll` bounds how long a settled owner goes unnoticed.
    ///
    /// The cost the caller must accept: every nack here counts toward the
    /// subscription's `max_delivery_attempts`, and every parked redelivery holds
    /// one Cloud Run request slot for up to `budget`.
    pub async fn wait_while_in_flight(
        &self,
        subscription: &str,
        message_id: &str,
        budget: Duration,
        poll: Duration,
    ) -> Result<InFlightWait, ClaimError> {
        let id = claim_id(subscription, message_id);
        let started = tokio::time::Instant::now();
        let poll = poll.max(Duration::from_secs(1));

        loop {
            let elapsed = started.elapsed();
            if elapsed >= budget {
                return Ok(InFlightWait::StillInFlight);
            }
            tokio::time::sleep(poll.min(budget - elapsed)).await;

            let Some(existing) = self.inner.read(&id).await? else {
                return Ok(InFlightWait::Changed);
            };
            let live = parse_state(&id, &existing.value.state) == ClaimState::InProgress
                && heartbeat_age_seconds(&existing.value) < self.stale_after.as_secs() as i64;
            if !live {
                return Ok(InFlightWait::Changed);
            }
        }
    }

    /// Prove the owner is still alive.
    ///
    /// This is the push-path substitute for `modifyAckDeadline`. It does not
    /// extend anything Pub/Sub knows about — the message will be redelivered
    /// regardless — it keeps the *claim* fresh so those redeliveries wait on it
    /// instead of taking the message away.
    pub async fn heartbeat(&self, guard: &mut ClaimGuard) -> Result<(), ClaimError> {
        let mut renewed = guard.document.clone();
        renewed.heartbeat_at_ms = now_ms();

        match self
            .inner
            .write(
                &guard.id,
                &renewed,
                Precondition::UpdateTime(guard.update_time.as_deref()),
            )
            .await?
        {
            (MutationOutcome::Applied, update_time) => {
                guard.document = renewed;
                guard.update_time = update_time;
                Ok(())
            }
            _ => Err(ClaimError::Lost),
        }
    }

    pub async fn complete(&self, guard: ClaimGuard) {
        self.settle(guard, ClaimState::Completed, "").await;
    }

    /// Hand the message back for a retry.
    ///
    /// The document is kept, not deleted, so the delivery counter survives. A
    /// deleted claim would be recreated by the next delivery with `attempts = 1`
    /// and the ceiling would never be reached.
    pub async fn release(&self, guard: ClaimGuard, code: &str) {
        self.settle(guard, ClaimState::Released, code).await;
    }

    pub async fn dead_letter(&self, guard: ClaimGuard, code: &str) {
        self.settle(guard, ClaimState::DeadLettered, code).await;
    }

    /// Settling is best effort by design. The work is already done or already
    /// abandoned at this point, and failing the delivery over a bookkeeping
    /// write would replace a clean outcome with a redelivery. The heartbeat
    /// staleness window is what recovers a claim whose settle never landed.
    ///
    /// `settled` is flipped *before* the write, not after: the write is the
    /// settle, and a guard whose settle write failed must not try a second,
    /// contradictory one from `Drop`.
    async fn settle(&self, mut guard: ClaimGuard, state: ClaimState, code: &str) {
        guard.settled = true;
        let id = std::mem::take(&mut guard.id);
        let document = guard.document.clone();
        let update_time = guard.update_time.take();
        self.inner
            .settle_document(&id, document, update_time, state, code)
            .await;
    }

    /// Best-effort increment on the already-dead-lettered path. A failure just
    /// returns the value that was read, so the caller still makes a decision.
    async fn bump_dead_letter_attempts(
        &self,
        id: &str,
        existing: &ClaimDocument,
        update_time: Option<&str>,
    ) -> i64 {
        let mut bumped = existing.clone();
        bumped.attempts = existing.attempts.saturating_add(1);
        bumped.heartbeat_at_ms = now_ms();

        match self
            .inner
            .write(id, &bumped, Precondition::UpdateTime(update_time))
            .await
        {
            Ok((MutationOutcome::Applied, _)) => bumped.attempts,
            _ => existing.attempts,
        }
    }

    fn guard(
        &self,
        id: String,
        document: ClaimDocument,
        update_time: Option<String>,
    ) -> ClaimGuard {
        ClaimGuard {
            store: Arc::clone(&self.inner),
            id,
            document,
            update_time,
            settled: false,
        }
    }
}

impl ClaimStoreInner {
    async fn read(&self, id: &str) -> Result<Option<Document<ClaimDocument>>, ClaimError> {
        Ok(self
            .firestore
            .read_document::<ClaimDocument>(&self.collection, id)
            .await?)
    }

    async fn settle_document(
        &self,
        id: &str,
        mut document: ClaimDocument,
        update_time: Option<String>,
        state: ClaimState,
        code: &str,
    ) {
        document.state = state.wire_name().to_string();
        document.heartbeat_at_ms = now_ms();
        document.outcome_code = code.to_string();

        match self
            .write(
                id,
                &document,
                Precondition::UpdateTime(update_time.as_deref()),
            )
            .await
        {
            Ok((MutationOutcome::Applied, _)) => {}
            Ok((outcome, _)) => tracing::warn!(
                claim_id = %id,
                state = %state,
                ?outcome,
                "claim was no longer ours when settling"
            ),
            Err(error) => tracing::error!(
                claim_id = %id,
                state = %state,
                error = %error,
                "claim settle failed; the staleness window will recover it"
            ),
        }
    }

    async fn write(
        &self,
        id: &str,
        document: &ClaimDocument,
        precondition: Precondition<'_>,
    ) -> Result<(MutationOutcome, Option<String>), ClaimError> {
        let expires_at_ms = now_ms() + self.retention.as_millis() as i64;
        let mut batch = self.firestore.write_batch();
        match precondition {
            Precondition::MustNotExist => batch
                .create(&self.collection, id, document, Some(expires_at_ms))
                .map(|_| ())?,
            // `None` here would mean writing without a token at all, which is
            // exactly the blind write this whole module exists to prevent, so it
            // is expressed as an immediate loss rather than as a permissive
            // fallback.
            Precondition::UpdateTime(None) => {
                return Ok((MutationOutcome::PreconditionFailed, None));
            }
            Precondition::UpdateTime(Some(update_time)) => batch
                .replace(
                    &self.collection,
                    id,
                    document,
                    Some(expires_at_ms),
                    Some(update_time),
                )
                .map(|_| ())?,
        }

        let outcome = self.firestore.commit(batch).await?;
        Ok((
            outcome.outcome,
            outcome.update_times.into_iter().next().flatten(),
        ))
    }
}

/// An unrecognised state is a document this build did not write. Treating it as
/// released lets a rolling deploy make progress instead of wedging behind a
/// value it cannot name.
fn parse_state(id: &str, value: &str) -> ClaimState {
    ClaimState::parse(value).unwrap_or_else(|| {
        tracing::warn!(
            claim_id = %id,
            state = %value,
            "unrecognised claim state; treating it as released"
        );
        ClaimState::Released
    })
}

fn heartbeat_age_seconds(document: &ClaimDocument) -> i64 {
    (now_ms() - document.heartbeat_at_ms) / 1_000
}

#[derive(Clone, Copy)]
enum Precondition<'a> {
    MustNotExist,
    UpdateTime(Option<&'a str>),
}

/// Firestore document IDs may not contain a path separator and Pub/Sub
/// subscription names contain several, so the pair is hashed rather than
/// concatenated. The subscription is part of the key because the same message
/// published to one topic is delivered to every subscription on it with the
/// *same* `messageId`: keying on the ID alone would let the file tracker's claim
/// silence the media transformer's delivery of the same object event.
fn claim_id(subscription: &str, message_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(subscription.as_bytes());
    hasher.update(&[0x1f]);
    hasher.update(message_id.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_ids_are_path_safe_stable_and_subscription_scoped() {
        let id = claim_id("projects/p/subscriptions/file-tracking", "2070443601311540");
        assert_eq!(id.len(), 64);
        assert!(!id.contains('/'));
        assert_eq!(
            id,
            claim_id("projects/p/subscriptions/file-tracking", "2070443601311540")
        );
        assert_ne!(
            id,
            claim_id("projects/p/subscriptions/media", "2070443601311540")
        );
    }

    /// The separator is what stops two different (subscription, message) pairs
    /// from hashing the same bytes by sliding the boundary between them.
    #[test]
    fn claim_ids_do_not_collide_across_a_shifted_boundary() {
        assert_ne!(claim_id("ab", "c"), claim_id("a", "bc"));
    }

    #[test]
    fn claim_states_round_trip_through_the_wire_names() {
        for state in [
            ClaimState::InProgress,
            ClaimState::Completed,
            ClaimState::Released,
            ClaimState::DeadLettered,
        ] {
            assert_eq!(ClaimState::parse(state.wire_name()), Some(state));
        }
        assert_eq!(ClaimState::parse("something-else"), None);
    }

    /// The Drop release exists for exactly one condition; a settled guard must
    /// be inert so a completed message is never flipped back to released, and
    /// an unsettled one dropped outside a runtime must log rather than panic.
    #[test]
    fn a_dropped_guard_never_panics() {
        let store = Arc::new(ClaimStoreInner {
            firestore: FirestoreClient::new(
                "flow-like-gcp-dev".to_string(),
                "(default)".to_string(),
                String::new(),
                flow_like_gcp_data::metadata::TokenSource::static_token("t", i64::MAX),
                0,
            )
            .expect("client"),
            collection: DEFAULT_CLAIMS_COLLECTION.to_string(),
            retention: Duration::from_secs(86_400),
        });
        let guard = |settled: bool| ClaimGuard {
            store: Arc::clone(&store),
            id: "id".to_string(),
            document: ClaimDocument {
                message_id: "m".to_string(),
                subscription: "s".to_string(),
                workload: "w".to_string(),
                state: STATE_COMPLETED.to_string(),
                owner: "o".to_string(),
                attempts: 1,
                claimed_at_ms: 0,
                heartbeat_at_ms: 0,
                outcome_code: String::new(),
            },
            update_time: None,
            settled,
        };
        // No runtime here: the settled guard returns before it looks for one,
        // the unsettled guard finds none and logs.
        drop(guard(true));
        drop(guard(false));
    }
}
