//! Plan-tier gating for hosted models.
//!
//! The proxy routes (`/chat/completions`, `/responses`, `/embeddings`) enforce this
//! per call and answer with `PAYMENT_REQUIRED`. Automatic model selection has to apply
//! the same rule *before* dispatch, or a caller whose plan excludes the flagship models
//! still gets pointed at one — and only finds out through a mid-stream 402.

use flow_like::{bit::Bit, hub::UserTier};
use flow_like_types::Value;
use std::collections::HashMap;

/// An LLM/VLM without a declared tier is treated as the most restricted one:
/// unlabeled hosted models must not become free-for-all.
pub const DEFAULT_LLM_TIER: &str = "ENTERPRISE";

/// Embedding models predate the tier field, so an unlabeled one stays open.
pub const DEFAULT_EMBEDDING_TIER: &str = "FREE";

/// The tier a provider declares in its params, or `default` when unlabeled.
pub fn declared_tier(params: Option<&HashMap<String, Value>>, default: &str) -> String {
    params
        .and_then(|params| params.get("tier"))
        .and_then(|tier| tier.as_str())
        .unwrap_or(default)
        .to_string()
}

pub fn tier_allows(user_tier: &UserTier, tier: &str) -> bool {
    user_tier.llm_tiers.iter().any(|allowed| allowed == tier)
}

/// Whether the caller's plan covers this bit. Only LLM/VLM bits are judged here —
/// embeddings, TTS and STT carry their own tiers and are gated by their own routes.
pub fn llm_bit_allowed(bit: &Bit, user_tier: &UserTier) -> bool {
    let provider = bit
        .try_to_llm()
        .map(|parameters| parameters.provider)
        .or_else(|| bit.try_to_vlm().map(|parameters| parameters.provider));
    let Some(provider) = provider else {
        return true;
    };
    tier_allows(
        user_tier,
        &declared_tier(provider.params.as_ref(), DEFAULT_LLM_TIER),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::hub::UserTier;
    use flow_like_types::json::json;

    fn tier(tiers: &[&str]) -> UserTier {
        UserTier {
            max_non_visible_projects: 0,
            max_remote_executions: 0,
            execution_tier: "SHARED".to_string(),
            max_total_size: 0,
            max_llm_cost: 0,
            max_llm_calls: None,
            llm_tiers: tiers.iter().map(|t| t.to_string()).collect(),
            product_id: None,
        }
    }

    fn params(tier: &str) -> HashMap<String, Value> {
        HashMap::from([("tier".to_string(), json!(tier))])
    }

    #[test]
    fn unlabeled_models_stay_locked_down() {
        assert_eq!(declared_tier(None, DEFAULT_LLM_TIER), "ENTERPRISE");
        assert_eq!(declared_tier(None, DEFAULT_EMBEDDING_TIER), "FREE");
    }

    #[test]
    fn declared_tier_wins_over_the_default() {
        assert_eq!(
            declared_tier(Some(&params("FREE")), DEFAULT_LLM_TIER),
            "FREE"
        );
    }

    #[test]
    fn a_free_plan_covers_only_its_own_tiers() {
        let free = tier(&["FREE"]);
        assert!(tier_allows(&free, "FREE"));
        assert!(!tier_allows(&free, "PRO"));
        assert!(!tier_allows(&free, "ENTERPRISE"));
    }
}
