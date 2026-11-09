//! Profile model choices for a bot's configuration tools. Provider credentials stay on the host.

use std::{collections::HashSet, sync::Arc};

use flow_like_model_provider::{
    audio::HostedAudioProvider,
    llm::external::ExternalProvider,
    provider::{ModelProvider, is_hosted_provider_name},
};
use flow_like_types::{Result, anyhow};
use futures::{StreamExt, stream};
use serde_json::{Value, json};

use crate::{
    bit::{Bit, BitTypes},
    profile::Profile,
    state::FlowLikeState,
    utils::http::HTTPClient,
};

fn provider_name(bit: &Bit) -> String {
    bit.try_to_provider()
        .or_else(|| bit.try_to_embedding_provider())
        .or_else(|| bit.try_to_tts().map(|parameters| parameters.provider))
        .or_else(|| bit.try_to_stt().map(|parameters| parameters.provider))
        .map(|provider| provider.provider_name)
        .or_else(|| {
            bit.parameters
                .pointer("/provider/provider_name")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

fn local_model(bit: &Bit) -> bool {
    if bit
        .try_to_embedding()
        .is_some_and(|parameters| parameters.supports_remote())
    {
        return false;
    }
    let provider = provider_name(bit).to_ascii_lowercase();
    provider.starts_with("local:")
        || matches!(
            provider.as_str(),
            "local"
                | "llama.cpp"
                | "llamacpp"
                | "mlx"
                | "device"
                | "ollama"
                | "custom:ollama"
                | "lmstudio"
                | "custom:lmstudio"
        )
        || bit.try_to_provider().is_some_and(|provider| {
            ExternalProvider::from_provider_name(&provider.provider_name)
                .is_some_and(|kind| kind.requires_local_credentials(&provider))
        })
}

fn hosted_audio(bit: &Bit) -> bool {
    let Some(provider) = bit
        .parameters
        .get("provider")
        .and_then(|value| serde_json::from_value::<ModelProvider>(value.clone()).ok())
    else {
        return false;
    };
    let Some(model) = provider.model_id.as_deref() else {
        return false;
    };
    HostedAudioProvider::from_name(&provider.provider_name).is_some()
        && !model.is_empty()
        && model.len() <= 256
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"@/._:-".contains(&byte))
        && !model
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && provider.api_surface.is_none()
        && provider.params.as_ref().is_none_or(|params| {
            params.iter().all(|(key, value)| {
                key == "tier" && value.as_str().is_some_and(|tier| !tier.trim().is_empty())
            })
        })
}

fn roles(bit: &Bit, only_hosted: bool) -> Vec<&'static str> {
    match bit.bit_type {
        BitTypes::Llm
            if bit
                .try_to_llm()
                .is_some_and(|parameters| parameters.context_length > 0) =>
        {
            vec!["primary", "fallbacks", "subtask_model"]
        }
        BitTypes::Vlm
            if bit
                .try_to_vlm()
                .is_some_and(|parameters| parameters.context_length > 0) =>
        {
            let mut roles = vec!["primary", "fallbacks", "subtask_model", "vision_model"];
            let model = bit
                .try_to_provider()
                .and_then(|provider| provider.model_id)
                .unwrap_or_default()
                .to_ascii_lowercase();
            if [
                "image",
                "imagen",
                "flux",
                "kontext",
                "stable-diffusion",
                "diffusion",
                "sdxl",
                "dall-e",
                "sourceful",
                "riverflow",
            ]
            .iter()
            .any(|needle| model.contains(needle))
            {
                roles.push("image_model");
            }
            roles
        }
        BitTypes::SystemOne
            if bit
                .try_to_systemone()
                .is_some_and(|parameters| parameters.context_length > 0)
                && (bit.is_device_model()
                    || crate::models::systemone::supports_provider(&provider_name(bit)))
                && (!only_hosted || is_hosted_provider_name(&provider_name(bit))) =>
        {
            vec!["decision"]
        }
        BitTypes::Embedding
            if bit.try_to_embedding().is_some_and(|parameters| {
                !only_hosted
                    || !is_hosted_provider_name(&parameters.provider.provider_name)
                    || parameters.supports_remote()
            }) =>
        {
            vec!["embedding_model"]
        }
        BitTypes::Stt
            if hosted_audio(bit)
                || (provider_name(bit)
                    .trim()
                    .eq_ignore_ascii_case("local:any-speech-to-text")
                    && bit.try_to_stt().is_some()) =>
        {
            vec!["stt_model"]
        }
        BitTypes::Tts
            if hosted_audio(bit)
                || (provider_name(bit)
                    .trim()
                    .eq_ignore_ascii_case("local:any-tts")
                    && bit.try_to_tts().is_some()) =>
        {
            vec!["tts_model"]
        }
        BitTypes::ImageGeneration
            if serde_json::from_value::<
                flow_like_model_provider::provider::ImageGenerationModelProvider,
            >(bit.parameters.clone())
            .is_ok() =>
        {
            vec!["image_model"]
        }
        _ => Vec::new(),
    }
}

fn model_choice(bit: &Bit, only_hosted: bool) -> Option<Value> {
    let local = local_model(bit);
    let participant_audio = matches!(bit.bit_type, BitTypes::Stt | BitTypes::Tts);
    if only_hosted && local && !participant_audio {
        return None;
    }
    let roles = roles(bit, only_hosted);
    if roles.is_empty() {
        return None;
    }
    let name = bit
        .meta
        .get("en")
        .or_else(|| bit.meta.values().next())
        .map(|meta| meta.name.as_str())
        .unwrap_or(&bit.id);
    Some(
        json!({"id": bit.id, "name": name, "type": bit.bit_type, "roles": roles, "local": local,
        "execution": if participant_audio { "participant_client" } else { "bot_host" }}),
    )
}

/// Lists only activated profile models, with the mapping roles their types support.
/// Discovery does not start local model processes or ask a device to connect.
pub async fn profile_models(
    profile: &Profile,
    state: &Arc<FlowLikeState>,
    only_hosted: bool,
) -> Result<Value> {
    profile_catalogue(profile, state.http_client.clone(), only_hosted).await
}

async fn profile_catalogue(
    profile: &Profile,
    http_client: Arc<HTTPClient>,
    only_hosted: bool,
) -> Result<Value> {
    let mut resolved = stream::iter(profile.bits.clone().into_iter().enumerate().map(
        |(index, reference)| {
            let http_client = http_client.clone();
            async move {
                let bit = profile
                    .resolve_model_reference(&reference, http_client)
                    .await
                    .ok();
                (index, bit.and_then(|bit| model_choice(&bit, only_hosted)))
            }
        },
    ))
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    resolved.sort_by_key(|(index, _)| *index);
    let mut seen = HashSet::new();
    let models: Vec<_> = resolved
        .into_iter()
        .filter_map(|(_, model)| model)
        .filter(|model| seen.insert(model["id"].as_str().unwrap_or_default().to_owned()))
        .collect();
    Ok(json!({
        "profile_id": profile.id,
        "models": models,
        "note": "Recommend a model from this list whose roles include the missing mapping. Model types are checked; provider availability and credentials are checked when the model runs. Speech mappings run on the participating client; local speech needs a compatible desktop client with the model installed. If no suitable model is listed, ask the owner to activate one in this profile."
    }))
}

/// Whether a validated patch selects a model that must be checked against the profile.
/// Clearing a mapping or its fallback list needs no profile lookup.
pub fn model_changes_need_catalogue(changes: &Value) -> bool {
    [
        "embedding_model",
        "stt_model",
        "tts_model",
        "image_model",
        "vision_model",
        "subtask_model",
    ]
    .into_iter()
    .any(|field| changes.get(field).is_some_and(Value::is_string))
        || changes.get("model").is_some_and(|model| {
            ["primary", "decision"]
                .into_iter()
                .any(|field| model.get(field).is_some_and(Value::is_string))
                || model
                    .get("fallbacks")
                    .and_then(Value::as_array)
                    .is_some_and(|ids| !ids.is_empty())
        })
}

/// Refuses invented, inactive or wrong-type model ids before a configuration write.
pub fn validate_model_changes(changes: &Value, catalogue: &Value) -> Result<()> {
    let check = |role: &str, value: &Value| -> Result<()> {
        if value.is_null() {
            return Ok(());
        }
        let id = value
            .as_str()
            .ok_or_else(|| anyhow!("{role} must be a model id or null"))?;
        let valid = catalogue["models"].as_array().is_some_and(|models| {
            models.iter().any(|model| {
                model["id"].as_str() == Some(id)
                    && model["roles"].as_array().is_some_and(|roles| {
                        roles
                            .iter()
                            .any(|candidate| candidate.as_str() == Some(role))
                    })
            })
        });
        if !valid {
            return Err(anyhow!(
                "Model '{id}' is not an activated profile model for {role}. Use bot_models to choose a compatible model."
            ));
        }
        Ok(())
    };
    for role in [
        "embedding_model",
        "stt_model",
        "tts_model",
        "image_model",
        "vision_model",
        "subtask_model",
    ] {
        if let Some(value) = changes.get(role) {
            check(role, value)?;
        }
    }
    if let Some(model) = changes.get("model") {
        for role in ["primary", "decision"] {
            if let Some(value) = model.get(role) {
                check(role, value)?;
            }
        }
        if let Some(fallbacks) = model.get("fallbacks") {
            for value in fallbacks
                .as_array()
                .ok_or_else(|| anyhow!("fallbacks must be a list of model ids"))?
            {
                if value.is_null() {
                    return Err(anyhow!("fallbacks must contain model ids"));
                }
                check("fallbacks", value)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discovery_only_lists_activated_models_in_profile_order() {
        let profile = Profile {
            bits: vec!["second".into(), "first".into(), "second".into()],
            custom_bits: ["first", "inactive", "second"]
                .into_iter()
                .map(|id| {
                    crate::profile::ProfileCustomBit(Bit {
                        id: id.into(),
                        bit_type: BitTypes::ImageGeneration,
                        parameters: json!({"provider":{"provider_name":"hosted:openai"}}),
                        ..Default::default()
                    })
                })
                .collect(),
            ..Default::default()
        };
        let catalogue =
            profile_catalogue(&profile, Arc::new(HTTPClient::new_without_refetch()), false)
                .await
                .unwrap();
        let ids: Vec<_> = catalogue["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["second", "first"]);
        assert!(validate_model_changes(&json!({"image_model":"inactive"}), &catalogue).is_err());
    }

    #[test]
    fn catalogue_never_exposes_provider_parameters() {
        let bit = Bit {
            id: "image".into(),
            bit_type: BitTypes::ImageGeneration,
            parameters: json!({"provider":{"provider_name":"hosted:openai","params":{"api_key":"private-key","base_url":"private-endpoint"}}}),
            ..Default::default()
        };
        let choice = model_choice(&bit, true).unwrap();
        assert_eq!(choice["roles"], json!(["image_model"]));
        assert!(!choice.to_string().contains("private"));
    }

    #[test]
    fn malformed_models_and_unsupported_decision_providers_are_not_recommended() {
        assert!(
            model_choice(
                &Bit {
                    bit_type: BitTypes::Llm,
                    ..Default::default()
                },
                false
            )
            .is_none()
        );
        let mut decision = Bit {
            bit_type: BitTypes::SystemOne,
            parameters: json!({"context_length":4096,"provider":{"provider_name":"hosted:openai"}}),
            ..Default::default()
        };
        assert!(model_choice(&decision, true).is_none());
        decision.parameters["provider"]["provider_name"] = json!("hosted:openrouter");
        assert_eq!(
            model_choice(&decision, true).unwrap()["roles"],
            json!(["decision"])
        );
    }

    #[test]
    fn server_embedding_choices_follow_the_remote_execution_contract() {
        let mut bit = Bit {
            id: "embedding".into(),
            bit_type: BitTypes::Embedding,
            parameters: json!({
                "languages":["en"], "vector_length":384, "input_length":512,
                "prefix":{"query":"", "paragraph":""}, "pooling":"Mean",
                "provider":{"provider_name":"local"},
                "remote":{"model_id":"hosted-embedding"}
            }),
            ..Default::default()
        };
        let choice = model_choice(&bit, true).unwrap();
        assert_eq!(choice["roles"], json!(["embedding_model"]));
        assert_eq!(choice["local"], false);

        bit.parameters.as_object_mut().unwrap().remove("remote");
        assert!(model_choice(&bit, true).is_none());
        assert!(model_choice(&bit, false).is_some());

        bit.parameters["provider"]["provider_name"] = json!("hosted:openai");
        assert!(model_choice(&bit, true).is_none());
        bit.parameters["provider"]["model_id"] = json!("text-embedding-3-small");
        assert!(model_choice(&bit, true).is_some());
    }

    #[test]
    fn hosted_choices_exclude_local_images_and_malformed_speech() {
        let bit = Bit {
            id: "image".into(),
            bit_type: BitTypes::ImageGeneration,
            parameters: json!({"provider":{"provider_name":"local:stablediffusion"}}),
            ..Default::default()
        };
        assert!(model_choice(&bit, true).is_none());
        assert!(model_choice(&bit, false).is_some());
        assert!(
            model_choice(
                &Bit {
                    bit_type: BitTypes::Tts,
                    ..Default::default()
                },
                true
            )
            .is_none()
        );
    }

    #[test]
    fn speech_choices_use_the_participant_clients_audio_routes() {
        for (kind, role, local_provider) in [
            (BitTypes::Tts, "tts_model", "local:any-tts"),
            (BitTypes::Stt, "stt_model", "local:any-speech-to-text"),
        ] {
            let mut bit = Bit {
                id: "speech".into(),
                bit_type: kind,
                parameters: json!({"provider":{"provider_name":"hosted:openai", "model_id":"audio-model"}}),
                ..Default::default()
            };
            let choice = model_choice(&bit, true).unwrap();
            assert_eq!(choice["roles"], json!([role]));
            assert_eq!(choice["local"], false);
            assert_eq!(choice["execution"], "participant_client");

            for invalid in [
                Value::Null,
                json!({"provider":{"provider_name":"hosted:unknown", "model_id":"audio-model"}}),
                json!({"provider":{"provider_name":"hosted:openai"}}),
                json!({"provider":{"provider_name":"hosted:openai", "model_id":"../invalid"}}),
            ] {
                bit.parameters = invalid;
                assert!(model_choice(&bit, false).is_none());
                assert!(model_choice(&bit, true).is_none());
            }
            bit.parameters = json!({"provider":{"provider_name":local_provider}});
            let choice = model_choice(&bit, true).unwrap();
            assert_eq!(choice["roles"], json!([role]));
            assert_eq!(choice["local"], true);
            assert_eq!(choice["execution"], "participant_client");
            // Speech parameters default to the local provider when it is omitted.
            bit.parameters = json!({});
            let choice = model_choice(&bit, true).unwrap();
            assert_eq!(choice["roles"], json!([role]));
            assert_eq!(choice["local"], true);
        }
    }

    #[test]
    fn clearing_model_mappings_does_not_require_a_catalogue() {
        for changes in [
            json!({"name":"Scout"}),
            json!({"image_model":null}),
            json!({"model":{"primary":null,"decision":null,"fallbacks":[]}}),
        ] {
            assert!(!model_changes_need_catalogue(&changes));
        }
        for changes in [
            json!({"image_model":"image"}),
            json!({"model":{"primary":"chat"}}),
            json!({"model":{"decision":"fast"}}),
            json!({"model":{"fallbacks":["chat"]}}),
        ] {
            assert!(model_changes_need_catalogue(&changes));
        }
    }

    #[test]
    fn mappings_reject_unknown_and_incompatible_models_but_allow_clearing() {
        let catalogue = json!({"models":[{"id":"chat","roles":["primary","fallbacks","subtask_model"]},{"id":"image","roles":["image_model"]}]});
        for patch in [
            json!({"image_model":"chat"}),
            json!({"model":{"primary":"unknown"}}),
            json!({"model":{"fallbacks":["image"]}}),
            json!({"model":{"decision":"chat"}}),
        ] {
            assert!(validate_model_changes(&patch, &catalogue).is_err());
        }
        assert!(
            validate_model_changes(
                &json!({"image_model":"image","model":{"primary":"chat","fallbacks":["chat"]}}),
                &catalogue
            )
            .is_ok()
        );
        assert!(
            validate_model_changes(
                &json!({"image_model":null,"model":{"decision":null}}),
                &catalogue
            )
            .is_ok()
        );
        assert!(validate_model_changes(&json!({"name":"Renamed"}), &Value::Null).is_ok());
    }
}
