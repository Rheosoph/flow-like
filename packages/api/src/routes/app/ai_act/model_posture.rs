use std::collections::BTreeMap;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

use super::signals::{DetectedModel, Signals};
use crate::{
    entity::{
        ai_act_model_registry, embedding_usage_tracking, llm_usage_tracking,
        sea_orm_active_enums::AiGpaiPosture,
    },
    error::ApiError,
    state::AppState,
};

fn model_key(provider: Option<&str>, model_id: &str) -> (String, String) {
    let provider = provider.map(str::trim).filter(|value| !value.is_empty());
    (
        provider.unwrap_or("unknown").to_string(),
        model_id.trim().to_string(),
    )
}

pub(crate) fn resolve_registry_entry<'a>(
    provider: Option<&str>,
    model_id: &str,
    registry: impl IntoIterator<Item = &'a ai_act_model_registry::Model>,
) -> Option<&'a ai_act_model_registry::Model> {
    let key = model_key(provider, model_id);
    let mut candidates = registry.into_iter().filter(|entry| {
        let entry_key = model_key(Some(&entry.provider), &entry.model_id);
        entry_key.1 == key.1 && (key.0 == "unknown" || entry_key.0 == key.0)
    });
    if key.0 != "unknown" {
        return candidates.next();
    }
    // A model id alone identifies a provider only when the registry has a
    // single concrete provider for it. Never borrow another provider's review.
    let mut concrete =
        candidates.filter(|entry| model_key(Some(&entry.provider), "").0 != "unknown");
    let entry = concrete.next()?;
    concrete.next().is_none().then_some(entry)
}

fn merge_monitored_models(
    models: &mut Vec<DetectedModel>,
    monitored: impl IntoIterator<Item = (String, Option<String>)>,
) {
    let mut by_key = BTreeMap::new();
    for model in models.drain(..) {
        by_key.insert(
            (
                model.dynamic_selector,
                model_key(model.provider.as_deref(), &model.model_id),
            ),
            model,
        );
    }
    for (model_id, provider) in monitored {
        if model_id.trim().is_empty() {
            continue;
        }
        by_key
            .entry((false, model_key(provider.as_deref(), &model_id)))
            .or_insert(DetectedModel {
                model_id,
                provider,
                ..Default::default()
            });
    }
    models.extend(by_key.into_values());
}

pub(crate) fn apply_registry_postures(
    models: &mut [DetectedModel],
    registry: &[ai_act_model_registry::Model],
) {
    for model in models {
        // Snapshot flags are never trusted after a registry review changes.
        model.vetted = false;
        model.posture_known = false;
        if model.dynamic_selector {
            continue;
        }
        if let Some(entry) =
            resolve_registry_entry(model.provider.as_deref(), &model.model_id, registry)
        {
            model.provider = Some(entry.provider.clone());
            model.vetted = entry.vetted;
            model.posture_known = entry.posture != AiGpaiPosture::Unknown;
        }
    }
}

/// Add models observed during execution and resolve their current registry
/// review. Call after board reconciliation so monitored models keep their source.
pub(crate) async fn enrich_scoring_signals(
    state: &AppState,
    app_id: &str,
    signals: &mut Signals,
) -> Result<(), ApiError> {
    let llm_models: Vec<(String, Option<String>)> = llm_usage_tracking::Entity::find()
        .filter(llm_usage_tracking::Column::AppId.eq(app_id))
        .select_only()
        .column(llm_usage_tracking::Column::ModelId)
        .column(llm_usage_tracking::Column::Provider)
        .distinct()
        .into_tuple()
        .all(&state.db)
        .await?;
    let embedding_models: Vec<(String, Option<String>)> = embedding_usage_tracking::Entity::find()
        .filter(embedding_usage_tracking::Column::AppId.eq(app_id))
        .select_only()
        .column(embedding_usage_tracking::Column::ModelId)
        .column(embedding_usage_tracking::Column::Provider)
        .distinct()
        .into_tuple()
        .all(&state.db)
        .await?;
    merge_monitored_models(
        &mut signals.models,
        llm_models.into_iter().chain(embedding_models),
    );

    let model_ids: Vec<_> = signals
        .models
        .iter()
        .map(|model| model.model_id.clone())
        .collect();
    if model_ids.is_empty() {
        return Ok(());
    }
    let registry = ai_act_model_registry::Entity::find()
        .filter(ai_act_model_registry::Column::ModelId.is_in(model_ids))
        .all(&state.db)
        .await?;
    apply_registry_postures(&mut signals.models, &registry);
    merge_monitored_models(&mut signals.models, std::iter::empty());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered(provider: &str, model_id: &str) -> ai_act_model_registry::Model {
        let now = chrono::Utc::now().fixed_offset();
        ai_act_model_registry::Model {
            id: "registry-entry".into(),
            provider: provider.into(),
            model_id: model_id.into(),
            posture: AiGpaiPosture::Hosted,
            hosted: true,
            open_licence: false,
            systemic_risk: false,
            vetted: true,
            note: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn registry_reviews_apply_only_to_the_matching_provider_and_fixed_model() {
        let mut models = vec![
            DetectedModel {
                model_id: "model".into(),
                provider: Some("provider-a".into()),
                ..Default::default()
            },
            DetectedModel {
                model_id: "model".into(),
                provider: Some("provider-b".into()),
                ..Default::default()
            },
            DetectedModel {
                model_id: "model".into(),
                provider: Some("provider-a".into()),
                dynamic_selector: true,
                ..Default::default()
            },
        ];
        apply_registry_postures(&mut models, &[registered("provider-a", "model")]);
        assert!(models[0].vetted && models[0].posture_known);
        assert!(!models[1].vetted && !models[1].posture_known);
        assert!(!models[2].vetted && !models[2].posture_known);

        apply_registry_postures(&mut models, &[]);
        assert!(!models[0].vetted && !models[0].posture_known);
    }

    #[test]
    fn unknown_posture_does_not_count_as_reviewed_posture() {
        let mut entry = registered("provider", "model");
        entry.posture = AiGpaiPosture::Unknown;
        let mut models = vec![DetectedModel {
            model_id: "model".into(),
            provider: Some("provider".into()),
            ..Default::default()
        }];
        apply_registry_postures(&mut models, &[entry]);
        assert!(models[0].vetted);
        assert!(!models[0].posture_known);
    }

    #[test]
    fn missing_provider_resolves_only_when_the_registry_identity_is_unambiguous() {
        let mut models = vec![DetectedModel {
            model_id: "model".into(),
            ..Default::default()
        }];
        apply_registry_postures(&mut models, &[registered("provider-a", "model")]);
        assert_eq!(models[0].provider.as_deref(), Some("provider-a"));
        assert!(models[0].vetted && models[0].posture_known);

        models[0].provider = None;
        apply_registry_postures(
            &mut models,
            &[
                registered("provider-a", "model"),
                registered("provider-b", "model"),
            ],
        );
        assert!(models[0].provider.is_none());
        assert!(!models[0].vetted && !models[0].posture_known);

        apply_registry_postures(&mut models, &[registered("unknown", "model")]);
        assert!(models[0].provider.is_none());
        assert!(!models[0].vetted && !models[0].posture_known);
    }

    #[test]
    fn monitoring_merges_duplicates_without_erasing_dynamic_selectors() {
        let mut models = vec![
            DetectedModel {
                model_id: "model".into(),
                provider: Some("provider".into()),
                ..Default::default()
            },
            DetectedModel {
                model_id: "dynamic".into(),
                dynamic_selector: true,
                ..Default::default()
            },
        ];
        merge_monitored_models(
            &mut models,
            [
                ("model".into(), Some("provider".into())),
                ("embedding".into(), Some("provider".into())),
                ("embedding".into(), Some("provider".into())),
            ],
        );
        assert_eq!(models.len(), 3);
        assert_eq!(
            models.iter().filter(|model| model.dynamic_selector).count(),
            1
        );
    }
}
