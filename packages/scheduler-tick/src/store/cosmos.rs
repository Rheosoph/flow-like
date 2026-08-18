//! Cosmos DB [`TickStore`]: one item per schedule in the `scheduler` container
//! (partition key `/app_id`), the CAS being Cosmos' own `If-Match` on `_etag`.

use super::{ScheduleDocument, container_from_env};
use crate::{ClaimOutcome, ScheduleState, StoreError, TickStore};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flow_like_azure_data::cosmos::{
    CosmosClient, CosmosError, MutationOutcome, validate_container_id,
};
use serde::Deserialize;

pub const CONTAINER_ENV: &str = "COSMOS_SCHEDULER_CONTAINER";

/// The read shape. Cosmos carries the concurrency token inside the body as
/// `_etag`, alongside the other system properties serde ignores.
#[derive(Deserialize)]
struct StoredDocument {
    cron_expression: String,
    last_fired_at: DateTime<Utc>,
    #[serde(rename = "_etag", default)]
    etag: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CosmosTickStore {
    client: CosmosClient,
    container: String,
}

impl CosmosTickStore {
    pub fn new(client: CosmosClient, container: impl Into<String>) -> Result<Self, StoreError> {
        let container = container.into();
        validate_container_id(&container).map_err(store_error)?;
        Ok(Self { client, container })
    }

    /// `CosmosClient::from_env` (`COSMOS_ENDPOINT`, `COSMOS_DATABASE`,
    /// `COSMOS_AUTH_MODE`, `AZURE_CLIENT_ID`) plus `COSMOS_SCHEDULER_CONTAINER`
    /// (default `scheduler`).
    pub fn from_env() -> Result<Self, StoreError> {
        let client = CosmosClient::from_env().map_err(store_error)?;
        Self::new(client, container_from_env(CONTAINER_ENV))
    }
}

#[async_trait]
impl TickStore for CosmosTickStore {
    async fn read(
        &self,
        event_id: &str,
        app_id: &str,
    ) -> Result<Option<ScheduleState>, StoreError> {
        let stored: Option<StoredDocument> = self
            .client
            .read_document(&self.container, event_id, app_id)
            .await
            .map_err(store_error)?;
        stored
            .map(|stored| -> Result<ScheduleState, StoreError> {
                // Every Cosmos item has an `_etag`; a read without one is a
                // response this client does not understand, not a claimable state.
                let version = stored.etag.ok_or_else(|| {
                    StoreError(format!(
                        "Cosmos item {event_id} in {} came back without an _etag",
                        self.container
                    ))
                })?;
                Ok(ScheduleState {
                    last_fired_at: stored.last_fired_at,
                    cron_expression: stored.cron_expression,
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
                .create_document(&self.container, app_id, &document)
                .await
                .map_err(store_error)?,
            Some(etag) => self
                .client
                .replace_document(&self.container, event_id, app_id, &document, Some(etag))
                .await
                .map_err(store_error)?,
        };
        match (expected_version, outcome) {
            (_, MutationOutcome::Applied) => Ok(ClaimOutcome::Claimed),
            // Create raced another tick's create; replace raced another tick's write.
            (None, MutationOutcome::Conflict) | (Some(_), MutationOutcome::PreconditionFailed) => {
                Ok(ClaimOutcome::Lost)
            }
            // A replace against an item that vanished (or a create that hit a
            // precondition) is neither win nor loss; surface it once and let the
            // next tick re-read and create.
            (_, other) => Err(StoreError(format!(
                "Cosmos claim for {event_id} in {} returned unexpected outcome {other:?}",
                self.container
            ))),
        }
    }
}

fn store_error(error: CosmosError) -> StoreError {
    StoreError(error.to_string())
}
