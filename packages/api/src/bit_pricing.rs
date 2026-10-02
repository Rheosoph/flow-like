use crate::{error::ApiError, usage_accounting::HostedRateSnapshot};
use flow_like::bit::{Bit, BitTypes};
use flow_like::flow_like_model_provider::provider::RemoteEmbeddingProvider;
use serde::{Deserialize, Serialize};

const DEFAULT_CONTEXT_TOKENS: i64 = 131_072;

#[derive(Default, Deserialize, Serialize)]
struct BitPricing {
    input_micro_usd_per_million_tokens: i64,
    output_micro_usd_per_million_tokens: i64,
    #[serde(default)]
    request_micro_usd: i64,
}

fn read_bit_pricing(bit: &Bit) -> Result<Option<BitPricing>, String> {
    let Some(pricing) = bit
        .parameters
        .get("pricing")
        .filter(|value| !value.is_null())
    else {
        return Ok(None);
    };
    if !pricing.is_object() {
        return Err("Bit pricing must be an object".into());
    }
    let pricing: BitPricing = serde_json::from_value(pricing.clone())
        .map_err(|error| format!("Invalid Bit pricing: {error}"))?;
    if pricing.input_micro_usd_per_million_tokens < 0
        || pricing.output_micro_usd_per_million_tokens < 0
        || pricing.request_micro_usd < 0
    {
        return Err("Bit pricing must contain nonnegative integer micro-USD amounts".into());
    }
    Ok(Some(pricing))
}

pub(crate) fn validate_bit_pricing(bit: &Bit) -> Result<(), ApiError> {
    match bit.bit_type {
        BitTypes::Llm | BitTypes::Vlm => {
            read_bit_pricing(bit).map_err(ApiError::bad_request)?;
        }
        BitTypes::Embedding => {
            let implementation = bit
                .parameters
                .pointer("/remote/implementation")
                .filter(|value| !value.is_null())
                .map(|value| serde_json::from_value::<RemoteEmbeddingProvider>(value.clone()))
                .transpose()
                .map_err(|_| ApiError::bad_request("Unknown remote embedding provider"))?
                .unwrap_or_default();
            if implementation != RemoteEmbeddingProvider::Internal {
                hosted_embedding_rate(bit)?;
            } else if bit
                .parameters
                .get("pricing")
                .is_some_and(|value| !value.is_null())
            {
                read_embedding_pricing(bit)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Serialize)]
struct EmbeddingPricing {
    input_micro_usd_per_million_tokens: Option<i64>,
    input_micro_usd_per_million_bytes: Option<i64>,
    max_input_bytes: Option<i64>,
    request_micro_usd: i64,
}

fn read_embedding_pricing(bit: &Bit) -> Result<EmbeddingPricing, ApiError> {
    let pricing = bit
        .parameters
        .get("pricing")
        .and_then(|value| value.as_object())
        .ok_or_else(|| {
            ApiError::bad_request("Hosted embedding requires an explicit pricing object")
        })?;
    let amount = |name: &str| -> Result<Option<i64>, ApiError> {
        pricing
            .get(name)
            .map(|value| {
                value.as_i64().filter(|amount| *amount >= 0).ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "Embedding pricing {name} must be a nonnegative integer"
                    ))
                })
            })
            .transpose()
    };
    let tokens = amount("input_micro_usd_per_million_tokens")?;
    let bytes = amount("input_micro_usd_per_million_bytes")?;
    if tokens.is_some() == bytes.is_some() {
        return Err(ApiError::bad_request(
            "Embedding pricing requires exactly one input tariff: tokens or UTF-8 bytes",
        ));
    }
    let max_input_bytes = amount("max_input_bytes")?;
    if max_input_bytes == Some(0) || (bytes.is_some() && max_input_bytes.is_none()) {
        return Err(ApiError::bad_request(
            "Embedding byte pricing requires a positive max_input_bytes limit",
        ));
    }
    if amount("output_micro_usd_per_million_tokens")?.is_some_and(|rate| rate != 0) {
        return Err(ApiError::bad_request(
            "Embedding output token pricing must be zero",
        ));
    }
    Ok(EmbeddingPricing {
        input_micro_usd_per_million_tokens: tokens,
        input_micro_usd_per_million_bytes: bytes,
        max_input_bytes,
        request_micro_usd: amount("request_micro_usd")?.unwrap_or(0),
    })
}

/// External embeddings require a saved tariff so admission reserves their cost.
pub(crate) fn hosted_embedding_rate(bit: &Bit) -> Result<HostedRateSnapshot, ApiError> {
    let pricing = read_embedding_pricing(bit)?;
    let context_tokens = bit
        .parameters
        .get("input_length")
        .and_then(|value| value.as_u64())
        .filter(|context| (1..=u64::from(u32::MAX)).contains(context))
        .map(|context| context as i64)
        .ok_or_else(|| {
            ApiError::bad_request("Hosted embedding requires a positive input_length")
        })?;
    let source = serde_json::json!({
        "pricing": pricing,
        "context_tokens": context_tokens,
    });
    let digest = blake3::hash(source.to_string().as_bytes());
    let rate = HostedRateSnapshot {
        version: format!("embedding-bit:{}:pricing:v1:{digest}", bit.id),
        provider_pricing_available: true,
        input_micro_usd_per_million_tokens: pricing.input_micro_usd_per_million_tokens.unwrap_or(0),
        input_micro_usd_per_million_bytes: pricing.input_micro_usd_per_million_bytes,
        max_input_bytes: pricing.max_input_bytes,
        output_micro_usd_per_million_tokens: 0,
        request_micro_usd: pricing.request_micro_usd,
        context_tokens,
        usd_micro_per_eur: 1_159_200,
        funding_basis_points: 550,
        api_micro_usd_per_million_ms: 20_001,
        serving_request_micro_usd: 1,
        max_request_ms: 120_000,
    };
    rate.validate()?;
    Ok(rate)
}

/// The saved Bit supplies the provider tariff for admission and settlement.
/// Missing prices leave provider cost unknown while preserving serving charges.
pub(crate) fn hosted_bit_rate(bit: &Bit) -> (HostedRateSnapshot, bool) {
    let provider = bit
        .parameters
        .pointer("/provider/provider_name")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let model = bit
        .parameters
        .pointer("/provider/model_id")
        .and_then(|value| value.as_str())
        .unwrap_or("unknown");
    let (pricing, pricing_source) = match read_bit_pricing(bit) {
        Ok(Some(pricing)) => (Some(pricing), "pricing"),
        Ok(None) => {
            tracing::warn!(
                bit_id = %bit.id,
                provider,
                model,
                "Hosted Bit pricing is missing; continuing without a configured provider price"
            );
            (None, "missing-pricing")
        }
        Err(error) => {
            tracing::warn!(
                bit_id = %bit.id,
                provider,
                model,
                %error,
                "Hosted Bit pricing is invalid; continuing without a configured provider price"
            );
            (None, "invalid-pricing")
        }
    };
    let pricing_available = pricing.is_some();
    let pricing = pricing.unwrap_or_default();
    let context_tokens = bit
        .parameters
        .get("context_length")
        .and_then(|value| value.as_u64())
        .filter(|context| (1..=u64::from(u32::MAX)).contains(context))
        .map(|context| context as i64)
        .unwrap_or_else(|| {
            tracing::warn!(
                bit_id = %bit.id,
                provider,
                model,
                context_tokens = DEFAULT_CONTEXT_TOKENS,
                "Hosted Bit context length is missing or invalid; using the default token limit"
            );
            DEFAULT_CONTEXT_TOKENS
        });
    let source = serde_json::json!({
        "pricing": pricing,
        "context_tokens": context_tokens,
    });
    let digest = blake3::hash(source.to_string().as_bytes());
    (
        HostedRateSnapshot {
            version: format!("bit:{}:{pricing_source}:v1:{digest}", bit.id),
            provider_pricing_available: pricing_available,
            input_micro_usd_per_million_tokens: pricing.input_micro_usd_per_million_tokens,
            input_micro_usd_per_million_bytes: None,
            max_input_bytes: None,
            output_micro_usd_per_million_tokens: pricing.output_micro_usd_per_million_tokens,
            request_micro_usd: pricing.request_micro_usd,
            context_tokens,
            usd_micro_per_eur: 1_159_200,
            funding_basis_points: 550,
            api_micro_usd_per_million_ms: 20_001,
            serving_request_micro_usd: 1,
            max_request_ms: 240_000,
        },
        pricing_available,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::Value;
    use serde_json::json;

    fn model_bit(pricing: Option<Value>) -> Bit {
        let mut bit = Bit {
            id: "flowpilot-sonnet".into(),
            bit_type: BitTypes::Llm,
            parameters: json!({
                "context_length": 200_000,
                "provider": {
                    "provider_name": "Premium",
                    "model_id": "@preset/claude-sonnet"
                }
            }),
            ..Default::default()
        };
        if let Some(pricing) = pricing {
            bit.parameters["pricing"] = pricing;
        }
        bit
    }

    fn token_pricing() -> Value {
        json!({
            "input_micro_usd_per_million_tokens": 3_000_000,
            "output_micro_usd_per_million_tokens": 15_000_000
        })
    }

    #[test]
    fn preset_model_uses_its_saved_bit_price() {
        let bit = model_bit(Some(token_pricing()));
        validate_bit_pricing(&bit).unwrap();
        let (rate, available) = hosted_bit_rate(&bit);
        assert!(available);
        assert!(rate.provider_pricing_available);
        assert_eq!(rate.provider_cost(1_000, 100), 4_500);
        assert_eq!(rate.request_micro_usd, 0);
        assert_eq!(rate.context_tokens, 200_000);
        assert!(rate.version.starts_with("bit:flowpilot-sonnet:pricing:v1:"));
        rate.validate().unwrap();
    }

    #[test]
    fn absent_and_null_prices_allow_runtime_and_admin_save() {
        for pricing in [None, Some(Value::Null)] {
            let bit = model_bit(pricing);
            validate_bit_pricing(&bit).unwrap();
            let (rate, available) = hosted_bit_rate(&bit);
            assert!(!available);
            assert!(!rate.provider_pricing_available);
            assert_eq!(rate.provider_cost(1_000, 100), 0);
            assert!(rate.serving_cost(100) > 0);
            assert!(rate.version.contains(":missing-pricing:"));
            rate.validate().unwrap();
        }
    }

    #[test]
    fn explicit_zero_prices_remain_known() {
        let bit = model_bit(Some(json!({
            "input_micro_usd_per_million_tokens": 0,
            "output_micro_usd_per_million_tokens": 0,
            "request_micro_usd": 0,
        })));
        validate_bit_pricing(&bit).unwrap();
        let (rate, available) = hosted_bit_rate(&bit);
        assert!(available);
        assert!(rate.provider_pricing_available);
        assert_eq!(rate.provider_cost(1_000, 100), 0);
    }

    #[test]
    fn invalid_prices_warn_at_runtime_and_fail_admin_validation() {
        let mut invalid_prices = vec![
            json!({}),
            json!([]),
            json!([1, 2]),
            json!([1, 2, 3]),
            json!("3.00"),
            json!(false),
        ];
        for field in [
            "input_micro_usd_per_million_tokens",
            "output_micro_usd_per_million_tokens",
            "request_micro_usd",
        ] {
            for invalid in [
                json!(-1),
                json!(0.5),
                json!("3"),
                Value::Null,
                json!(u64::MAX),
            ] {
                let mut pricing = token_pricing();
                pricing[field] = invalid;
                invalid_prices.push(pricing);
            }
        }
        for pricing in invalid_prices {
            let bit = model_bit(Some(pricing));
            assert_eq!(
                validate_bit_pricing(&bit).unwrap_err().status(),
                axum::http::StatusCode::BAD_REQUEST
            );
            let (rate, available) = hosted_bit_rate(&bit);
            assert!(!available);
            assert!(!rate.provider_pricing_available);
            assert_eq!(rate.provider_cost(1_000, 100), 0);
            assert!(rate.version.contains(":invalid-pricing:"));
            rate.validate().unwrap();
        }
    }

    #[test]
    fn integer_prices_accept_the_full_nonnegative_range() {
        let bit = model_bit(Some(json!({
            "input_micro_usd_per_million_tokens": i64::MAX,
            "output_micro_usd_per_million_tokens": i64::MAX,
            "request_micro_usd": i64::MAX,
        })));
        validate_bit_pricing(&bit).unwrap();
        let (rate, available) = hosted_bit_rate(&bit);
        assert!(available);
        assert_eq!(rate.provider_cost(i64::MAX, i64::MAX), i64::MAX);
    }

    #[test]
    fn internal_embeddings_can_omit_pricing_and_other_bit_types_are_unchanged() {
        let mut bit = model_bit(Some(json!({})));
        bit.bit_type = BitTypes::Vlm;
        assert!(validate_bit_pricing(&bit).is_err());
        bit.bit_type = BitTypes::Embedding;
        assert!(validate_bit_pricing(&bit).is_err());
        bit.parameters.as_object_mut().unwrap().remove("pricing");
        assert!(validate_bit_pricing(&bit).is_ok());
        bit.parameters["remote"] = json!({"implementation": "Internal"});
        assert!(validate_bit_pricing(&bit).is_ok());
        bit.parameters["remote"] = json!({"model_id": "legacy-model"});
        assert!(validate_bit_pricing(&bit).is_ok());
        bit.bit_type = BitTypes::File;
        bit.parameters["pricing"] = json!({});
        assert!(validate_bit_pricing(&bit).is_ok());
    }

    fn embedding_bit(pricing: Value) -> Bit {
        Bit {
            id: "hosted-embedding".into(),
            bit_type: BitTypes::Embedding,
            parameters: json!({
                "input_length": 8192,
                "remote": {"implementation": "OpenAI"},
                "pricing": pricing,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn external_embeddings_require_explicit_pricing_for_every_provider() {
        for implementation in [
            "CloudflareWorkersAI",
            "OpenAI",
            "AzureOpenAI",
            "HuggingfaceEndpoint",
            "OpenAICompatible",
            "Cohere",
            "VoyageAI",
        ] {
            let mut bit = embedding_bit(Value::Null);
            bit.parameters["remote"]["implementation"] = json!(implementation);
            assert!(validate_bit_pricing(&bit).is_err(), "{implementation}");
            bit.parameters["pricing"] = json!({"input_micro_usd_per_million_tokens": 0});
            validate_bit_pricing(&bit).unwrap();
            let rate = hosted_embedding_rate(&bit).unwrap();
            assert!(rate.provider_pricing_available);
            assert_eq!(rate.provider_cost(8192, 0), 0);
            assert_eq!(rate.context_tokens, 8192);
            assert_eq!(rate.max_request_ms, 120_000);
        }
    }

    #[test]
    fn embedding_tariffs_preserve_units_and_request_fees() {
        let token_bit = embedding_bit(json!({
            "input_micro_usd_per_million_tokens": 20_000,
            "output_micro_usd_per_million_tokens": 0,
            "request_micro_usd": 3,
        }));
        let token_rate = hosted_embedding_rate(&token_bit).unwrap();
        assert_eq!(token_rate.provider_cost(1000, 0), 23);
        assert_eq!(token_rate.input_micro_usd_per_million_bytes, None);

        let byte_bit = embedding_bit(json!({
            "input_micro_usd_per_million_bytes": 50_000,
            "max_input_bytes": 1_000_000,
            "request_micro_usd": 3,
        }));
        let byte_rate = hosted_embedding_rate(&byte_bit).unwrap();
        assert_eq!(byte_rate.provider_cost_bytes(1000).unwrap(), 53);
        assert_eq!(byte_rate.input_micro_usd_per_million_tokens, 0);
        assert_eq!(byte_rate.max_input_bytes, Some(1_000_000));
        validate_bit_pricing(&byte_bit).unwrap();
    }

    #[test]
    fn embedding_pricing_rejects_ambiguous_units_and_invalid_limits() {
        for pricing in [
            Value::Null,
            json!({}),
            json!([]),
            json!({"input_micro_usd_per_million_tokens": 0, "input_micro_usd_per_million_bytes": 0, "max_input_bytes": 1}),
            json!({"input_micro_usd_per_million_bytes": 0}),
            json!({"input_micro_usd_per_million_bytes": 0, "max_input_bytes": 0}),
            json!({"input_micro_usd_per_million_tokens": 1, "output_micro_usd_per_million_tokens": 1}),
        ] {
            assert!(hosted_embedding_rate(&embedding_bit(pricing)).is_err());
        }
        for field in [
            "input_micro_usd_per_million_tokens",
            "input_micro_usd_per_million_bytes",
            "max_input_bytes",
            "output_micro_usd_per_million_tokens",
            "request_micro_usd",
        ] {
            for invalid in [
                json!(-1),
                json!(0.5),
                json!("1"),
                Value::Null,
                json!(u64::MAX),
            ] {
                let mut pricing = if field == "input_micro_usd_per_million_bytes" {
                    json!({"input_micro_usd_per_million_bytes": 1, "max_input_bytes": 10})
                } else {
                    json!({"input_micro_usd_per_million_tokens": 1})
                };
                pricing[field] = invalid;
                assert!(
                    hosted_embedding_rate(&embedding_bit(pricing)).is_err(),
                    "{field}"
                );
            }
        }
        for invalid in [
            Value::Null,
            json!(0),
            json!(-1),
            json!(0.5),
            json!("8192"),
            json!(u64::from(u32::MAX) + 1),
        ] {
            let mut bit = embedding_bit(json!({"input_micro_usd_per_million_tokens": 1}));
            bit.parameters["input_length"] = invalid;
            assert!(hosted_embedding_rate(&bit).is_err());
        }
    }

    #[test]
    fn embedding_snapshot_is_stable_until_pricing_or_input_limit_changes() {
        let mut bit = embedding_bit(json!({"input_micro_usd_per_million_tokens": 20_000}));
        let original = hosted_embedding_rate(&bit).unwrap();
        let saved = serde_json::to_value(&original).unwrap();
        assert_eq!(
            original.version,
            hosted_embedding_rate(&bit).unwrap().version
        );
        bit.parameters["pricing"]["input_micro_usd_per_million_tokens"] = json!(40_000);
        let repriced = hosted_embedding_rate(&bit).unwrap();
        assert_ne!(original.version, repriced.version);
        bit.parameters["input_length"] = json!(512);
        assert_ne!(
            repriced.version,
            hosted_embedding_rate(&bit).unwrap().version
        );
        let restored: HostedRateSnapshot = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.provider_cost(1000, 0), 20);
        assert_eq!(restored.context_tokens, 8192);
    }

    #[test]
    fn invalid_context_lengths_use_a_valid_fallback() {
        for context in [
            Value::Null,
            json!(0),
            json!(-1),
            json!(1.5),
            json!("200000"),
            json!(u64::from(u32::MAX) + 1),
        ] {
            let mut bit = model_bit(None);
            bit.parameters["context_length"] = context;
            let (rate, available) = hosted_bit_rate(&bit);
            assert!(!available);
            assert_eq!(rate.context_tokens, DEFAULT_CONTEXT_TOKENS);
            rate.validate().unwrap();
        }
        let mut bit = model_bit(None);
        bit.parameters
            .as_object_mut()
            .unwrap()
            .remove("context_length");
        assert_eq!(
            hosted_bit_rate(&bit).0.context_tokens,
            DEFAULT_CONTEXT_TOKENS
        );
    }

    #[test]
    fn saved_snapshot_keeps_original_price_after_bit_edits() {
        let mut bit = model_bit(Some(token_pricing()));
        let (original, _) = hosted_bit_rate(&bit);
        let reservation = serde_json::to_value(&original).unwrap();
        assert_eq!(original.version, hosted_bit_rate(&bit).0.version);

        bit.parameters["pricing"]["input_micro_usd_per_million_tokens"] = json!(4_000_000);
        let (repriced, _) = hosted_bit_rate(&bit);
        assert_ne!(original.version, repriced.version);
        assert_eq!(repriced.provider_cost(1_000, 100), 5_500);

        bit.parameters["context_length"] = json!(300_000);
        assert_ne!(repriced.version, hosted_bit_rate(&bit).0.version);
        let reserved: HostedRateSnapshot = serde_json::from_value(reservation).unwrap();
        assert_eq!(reserved.version, original.version);
        assert_eq!(reserved.provider_cost(1_000, 100), 4_500);
        assert_eq!(reserved.context_tokens, 200_000);
    }
}
