use crate::{error::ApiError, usage_accounting::HostedRateSnapshot};
use flow_like::bit::{Bit, BitTypes};
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
    if matches!(bit.bit_type, BitTypes::Llm | BitTypes::Vlm) {
        read_bit_pricing(bit).map_err(ApiError::bad_request)?;
    }
    Ok(())
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
    fn pricing_validation_covers_llm_and_vlm_only() {
        let mut bit = model_bit(Some(json!({})));
        bit.bit_type = BitTypes::Vlm;
        assert!(validate_bit_pricing(&bit).is_err());
        bit.bit_type = BitTypes::Embedding;
        assert!(validate_bit_pricing(&bit).is_ok());
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
