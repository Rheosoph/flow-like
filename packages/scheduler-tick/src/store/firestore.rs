//! Firestore [`TickStore`]: one document per schedule in the `scheduler`
//! collection, the CAS being the `currentDocument.updateTime` precondition.
//! There is no partition key on Firestore; `app_id` is stored as a field so the
//! document shape matches Cosmos and operators can filter on it.

use super::{ScheduleDocument, container_from_env};
use crate::{ClaimOutcome, ScheduleState, StoreError, TickStore};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flow_like_gcp_data::firestore::{
    FirestoreClient, FirestoreError, MutationOutcome, validate_collection_id,
};

pub const COLLECTION_ENV: &str = "FIRESTORE_SCHEDULER_COLLECTION";

#[derive(Clone, Debug)]
pub struct FirestoreTickStore {
    client: FirestoreClient,
    collection: String,
}

impl FirestoreTickStore {
    pub fn new(client: FirestoreClient, collection: impl Into<String>) -> Result<Self, StoreError> {
        let collection = collection.into();
        validate_collection_id(&collection).map_err(store_error)?;
        Ok(Self { client, collection })
    }

    /// `FirestoreClient::from_env` (`GCP_PROJECT_ID`, `FIRESTORE_DATABASE`,
    /// `FIRESTORE_COLLECTION_PREFIX`; refuses emulator/endpoint overrides) plus
    /// `FIRESTORE_SCHEDULER_COLLECTION` (default `scheduler`).
    pub fn from_env() -> Result<Self, StoreError> {
        let client = FirestoreClient::from_env().map_err(store_error)?;
        Self::new(client, container_from_env(COLLECTION_ENV))
    }
}

#[async_trait]
impl TickStore for FirestoreTickStore {
    async fn read(
        &self,
        event_id: &str,
        _app_id: &str,
    ) -> Result<Option<ScheduleState>, StoreError> {
        let stored = self
            .client
            .read_document::<ScheduleDocument>(&self.collection, event_id)
            .await
            .map_err(store_error)?;
        stored
            .map(|document| -> Result<ScheduleState, StoreError> {
                // Firestore keeps the concurrency token in the envelope, not the
                // body; a document without one cannot be claimed safely.
                let version = document.update_time.ok_or_else(|| {
                    StoreError(format!(
                        "Firestore document {event_id} in {} came back without an updateTime",
                        self.collection
                    ))
                })?;
                Ok(ScheduleState {
                    last_fired_at: document.value.last_fired_at,
                    cron_expression: document.value.cron_expression,
                    version,
                })
            })
            .transpose()
    }

    async fn claim(
        &self,
        event_id: &str,
        app_id: &str,
        cron_expression: &str,
        expected_version: Option<&str>,
        new_last_fired_at: DateTime<Utc>,
    ) -> Result<ClaimOutcome, StoreError> {
        let document = ScheduleDocument::new(event_id, app_id, cron_expression, new_last_fired_at);
        let outcome = match expected_version {
            None => self
                .client
                .create_document(&self.collection, event_id, &document, None)
                .await
                .map_err(store_error)?,
            Some(update_time) => self
                .client
                .replace_document(
                    &self.collection,
                    event_id,
                    &document,
                    None,
                    Some(update_time),
                )
                .await
                .map_err(store_error)?,
        };
        match (expected_version, outcome) {
            (_, MutationOutcome::Applied) => Ok(ClaimOutcome::Claimed),
            // Create raced another tick's create (ALREADY_EXISTS).
            (None, MutationOutcome::Conflict) => Ok(ClaimOutcome::Lost),
            // Replace lost the updateTime race, or was ABORTED under contention:
            // both mean another writer is on this document right now.
            (Some(_), MutationOutcome::PreconditionFailed | MutationOutcome::Conflict) => {
                Ok(ClaimOutcome::Lost)
            }
            (_, other) => Err(StoreError(format!(
                "Firestore claim for {event_id} in {} returned unexpected outcome {other:?}",
                self.collection
            ))),
        }
    }
}

fn store_error(error: FirestoreError) -> StoreError {
    StoreError(error.to_string())
}
