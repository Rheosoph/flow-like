//! Model reconciliation (EU AI Act §1.4 / §8). Merges the distinct models an
//! app actually uses — from monitoring (`LLMUsageTracking` /
//! `EmbeddingUsageTracking`) and from the static board scan — into
//! `AiActModelObservation` rows, resolving GPAI posture from the platform
//! `AiActModelRegistry`. Runs inline/awaited so it is safe in serverless
//! (Lambda) deployments.

use std::collections::BTreeMap;

use chrono::NaiveDateTime;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QuerySelect,
};

use crate::{
    entity::{
        ai_act_model_observation, ai_act_model_registry, embedding_usage_tracking,
        llm_usage_tracking,
        sea_orm_active_enums::{AiGpaiPosture, AiModelSource},
    },
    error::ApiError,
    routes::app::ai_act::signals::Signals,
    state::AppState,
};

/// A model the reconciliation discovered, keyed by `(provider, model_id)`.
#[derive(Clone, Debug, Default)]
struct DiscoveredModel {
    provider: Option<String>,
    from_monitoring: bool,
    from_board: bool,
    dynamic_selector: bool,
}

fn model_key(provider: Option<&str>, model_id: &str) -> String {
    format!("{}/{}", provider.unwrap_or("unknown"), model_id)
}

/// Posture flags resolved from the registry (or defaults for unknown models).
struct ResolvedPosture {
    posture: AiGpaiPosture,
    hosted: bool,
    open_licence: bool,
    systemic_risk: bool,
    vetted: bool,
}

impl ResolvedPosture {
    fn unknown() -> Self {
        ResolvedPosture {
            posture: AiGpaiPosture::Unknown,
            hosted: false,
            open_licence: false,
            systemic_risk: false,
            vetted: false,
        }
    }
}

/// Reconcile the models for a single app. Returns the number of observations
/// written/updated. `signals` provides the board-scan models; monitoring is
/// queried from the DB.
pub async fn reconcile_app_models(
    state: &AppState,
    app_id: &str,
    signals: &Signals,
) -> Result<usize, ApiError> {
    let now = chrono::Utc::now().naive_utc();
    let mut discovered: BTreeMap<String, (String, DiscoveredModel)> = BTreeMap::new();

    // 1. Monitoring: distinct LLM models used by this app.
    let llm_models: Vec<(String, Option<String>)> = llm_usage_tracking::Entity::find()
        .filter(llm_usage_tracking::Column::AppId.eq(app_id))
        .select_only()
        .column(llm_usage_tracking::Column::ModelId)
        .column(llm_usage_tracking::Column::Provider)
        .distinct()
        .into_tuple()
        .all(&state.db)
        .await?;

    for (model_id, provider) in llm_models {
        let key = model_key(provider.as_deref(), &model_id);
        let entry = discovered
            .entry(key)
            .or_insert_with(|| (model_id.clone(), DiscoveredModel::default()));
        entry.1.provider = provider;
        entry.1.from_monitoring = true;
    }

    // 2. Monitoring: distinct embedding models used by this app.
    let embed_models: Vec<(String, Option<String>)> = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::AppId.eq(app_id))
        .select_only()
        .column(embedding_usage_tracking::Column::ModelId)
        .column(embedding_usage_tracking::Column::Provider)
        .distinct()
        .into_tuple()
        .all(&state.db)
        .await?;

    for (model_id, provider) in embed_models {
        let key = model_key(provider.as_deref(), &model_id);
        let entry = discovered
            .entry(key)
            .or_insert_with(|| (model_id.clone(), DiscoveredModel::default()));
        entry.1.provider = provider;
        entry.1.from_monitoring = true;
    }

    // 3. Board scan models (from signals).
    for m in &signals.models {
        let key = if m.dynamic_selector {
            "::dynamic::".to_string()
        } else {
            model_key(m.provider.as_deref(), &m.model_id)
        };
        let entry = discovered
            .entry(key)
            .or_insert_with(|| (m.model_id.clone(), DiscoveredModel::default()));
        if entry.1.provider.is_none() {
            entry.1.provider = m.provider.clone();
        }
        entry.1.from_board = true;
        entry.1.dynamic_selector |= m.dynamic_selector;
    }

    // 4. Load existing observations for this app to detect drift (new models).
    let existing = ai_act_model_observation::Entity::find()
        .filter(ai_act_model_observation::Column::AppId.eq(app_id))
        .all(&state.db)
        .await?;
    let existing_keys: std::collections::HashSet<String> = existing
        .iter()
        .map(|o| {
            if o.dynamic_selector {
                "::dynamic::".to_string()
            } else {
                model_key(o.provider.as_deref(), &o.model_id)
            }
        })
        .collect();
    let existing_by_key: BTreeMap<String, ai_act_model_observation::Model> = existing
        .into_iter()
        .map(|o| {
            let key = if o.dynamic_selector {
                "::dynamic::".to_string()
            } else {
                model_key(o.provider.as_deref(), &o.model_id)
            };
            (key, o)
        })
        .collect();

    let mut written = 0usize;
    for (key, (model_id, disc)) in discovered {
        let source = match (disc.from_monitoring, disc.from_board) {
            (true, true) => AiModelSource::Both,
            (true, false) => AiModelSource::Monitored,
            (false, true) => AiModelSource::BoardScan,
            (false, false) => continue,
        };

        let posture = resolve_posture(state, disc.provider.as_deref(), &model_id).await;
        let drift = !existing_keys.contains(&key);

        if let Some(existing) = existing_by_key.get(&key) {
            let mut active: ai_act_model_observation::ActiveModel = existing.clone().into();
            active.provider = Set(disc.provider.clone());
            active.source = Set(source);
            active.posture = Set(posture.posture.clone());
            active.hosted = Set(posture.hosted);
            active.open_licence = Set(posture.open_licence);
            active.systemic_risk = Set(posture.systemic_risk);
            active.vetted = Set(posture.vetted);
            active.dynamic_selector = Set(disc.dynamic_selector);
            active.last_seen_at = Set(now);
            active.update(&state.db).await?;
        } else {
            let active = ai_act_model_observation::ActiveModel {
                id: Set(flow_like_types::create_id()),
                app_id: Set(app_id.to_string()),
                model_id: Set(model_id),
                provider: Set(disc.provider.clone()),
                source: Set(source),
                posture: Set(posture.posture.clone()),
                hosted: Set(posture.hosted),
                open_licence: Set(posture.open_licence),
                systemic_risk: Set(posture.systemic_risk),
                vetted: Set(posture.vetted),
                dynamic_selector: Set(disc.dynamic_selector),
                drift_flagged: Set(drift),
                first_seen_at: Set(now),
                last_seen_at: Set(now),
            };
            active.insert(&state.db).await?;
        }
        written += 1;
    }

    Ok(written)
}

/// Resolve a model's posture from the registry. Unknown models default to
/// `UNKNOWN`/unvetted so they surface for admin review.
async fn resolve_posture(
    state: &AppState,
    provider: Option<&str>,
    model_id: &str,
) -> ResolvedPosture {
    let provider = provider.unwrap_or("unknown");
    let record = ai_act_model_registry::Entity::find()
        .filter(ai_act_model_registry::Column::Provider.eq(provider))
        .filter(ai_act_model_registry::Column::ModelId.eq(model_id))
        .one(&state.db)
        .await
        .ok()
        .flatten();

    match record {
        Some(r) => ResolvedPosture {
            posture: r.posture,
            hosted: r.hosted,
            open_licence: r.open_licence,
            systemic_risk: r.systemic_risk,
            vetted: r.vetted,
        },
        None => ResolvedPosture::unknown(),
    }
}

/// Convenience for callers that only have the last-seen timestamp.
#[allow(dead_code)]
pub fn naive_now() -> NaiveDateTime {
    chrono::Utc::now().naive_utc()
}
