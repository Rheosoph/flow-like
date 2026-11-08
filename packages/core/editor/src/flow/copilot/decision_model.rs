//! Resolve the optional decision model with the credentials of the current turn.

use std::{sync::Arc, time::Duration};

use flow_like_model_provider::{provider::is_hosted_provider_name, systemone::SystemOneModelLogic};
use flow_like_types::{Result, anyhow, tokio::time::timeout};

use crate::{models::llm::ModelUsageContext, profile::Profile, state::FlowLikeState};

pub const DECISION_TIMEOUT: Duration = Duration::from_secs(15);

/// A missing pin disables decision inference. Resolution never substitutes a completion model
/// or a different Bit, and a hosted bot cannot lend its credentials to a customer endpoint.
pub async fn resolve_decision_model(
    state: &Arc<FlowLikeState>,
    profile: &Profile,
    model_id: Option<&str>,
    hosted_only: bool,
    token: Option<String>,
    usage: Option<ModelUsageContext>,
) -> Result<Option<Arc<dyn SystemOneModelLogic>>> {
    let Some(model_id) = model_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    timeout(DECISION_TIMEOUT, async {
        let bit = profile
            .find_bit(model_id, state.http_client.clone())
            .await?;
        let parameters = bit
            .try_to_systemone()
            .filter(|parameters| parameters.context_length > 0)
            .ok_or_else(|| anyhow!("Decision models must be valid SystemOne Bits"))?;
        if hosted_only && !is_hosted_provider_name(&parameters.provider.provider_name) {
            return Err(anyhow!("This bot requires a hosted decision model"));
        }
        state
            .model_factory
            .build_systemone(&bit, state.clone(), token, usage)
            .await
            .map(Some)
    })
    .await
    .map_err(|_| anyhow!("Decision model resolution timed out"))?
}
