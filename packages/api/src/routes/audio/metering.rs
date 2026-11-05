use flow_like::bit::{Bit, BitTypes};
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, usage_accounting::HostedRateSnapshot};

/// Legacy tariff when a hosted audio Bit has no explicit pricing: USD 0.006 per minute.
pub(super) const TRANSCRIPTION_MICRO_USD_PER_SECOND: i64 = 100;
/// Legacy tariff when a hosted audio Bit has no explicit pricing: USD 30 per million characters.
pub(super) const SPEECH_MICRO_USD_PER_CHARACTER: i64 = 30;
const COMPRESSED_BYTES_PER_SECOND: u64 = 12_000;
const MAX_REQUEST_MS: i64 = 120_000;
const WAV_HEADER_BYTES: usize = 44;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AudioKind {
    Transcription,
    Speech,
}

impl AudioKind {
    pub(super) fn invocation_kind(self) -> &'static str {
        match self {
            Self::Transcription => "audio_transcription",
            Self::Speech => "audio_speech",
        }
    }

    fn rate_version(self) -> &'static str {
        match self {
            Self::Transcription => "audio-transcription-2026-11-04",
            Self::Speech => "audio-speech-2026-11-04",
        }
    }
}

/// Captured at admission so catalog edits cannot change a queued request's tariff.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "unit",
    content = "micro_usd_per_million",
    rename_all = "snake_case"
)]
pub(super) enum AudioPricing {
    #[default]
    Legacy,
    Seconds(i64),
    Characters(i64),
    Bytes(i64),
}

impl AudioPricing {
    pub(super) fn from_bit(bit: &Bit) -> Result<Self, ApiError> {
        let Some(pricing) = bit
            .parameters
            .get("pricing")
            .filter(|value| !value.is_null())
        else {
            return Ok(Self::Legacy);
        };
        let pricing = pricing
            .as_object()
            .filter(|pricing| pricing.len() == 1)
            .ok_or_else(|| {
                ApiError::bad_request("Hosted audio pricing requires exactly one input tariff")
            })?;
        let (field, value) = pricing.iter().next().expect("one pricing field");
        let amount = value
            .as_i64()
            .filter(|amount| *amount >= 0)
            .ok_or_else(|| {
                ApiError::bad_request(
                    "Hosted audio pricing must use nonnegative integer micro-USD amounts",
                )
            })?;
        let tariff = match field.as_str() {
            "input_micro_usd_per_million_seconds" => Self::Seconds(amount),
            "input_micro_usd_per_million_characters" => Self::Characters(amount),
            "input_micro_usd_per_million_bytes" => Self::Bytes(amount),
            _ => return Err(ApiError::bad_request("Unknown hosted audio pricing unit")),
        };
        tariff.validate(match bit.bit_type {
            BitTypes::Stt => AudioKind::Transcription,
            BitTypes::Tts => AudioKind::Speech,
            _ => {
                return Err(ApiError::bad_request(
                    "Audio pricing requires an Stt or Tts Bit",
                ));
            }
        })?;
        Ok(tariff)
    }

    pub(super) fn validate(self, kind: AudioKind) -> Result<(), ApiError> {
        match (self, kind) {
            (Self::Legacy, _)
            | (Self::Seconds(0..), AudioKind::Transcription)
            | (Self::Characters(0..) | Self::Bytes(0..), AudioKind::Speech) => Ok(()),
            _ => Err(ApiError::bad_request(
                "Hosted transcription pricing requires seconds; speech pricing requires characters or UTF-8 bytes",
            )),
        }
    }

    pub(super) fn transcription_cost(self, seconds: i64) -> Result<i64, ApiError> {
        self.validate(AudioKind::Transcription)?;
        Ok(match self {
            Self::Seconds(rate) => cost_per_million(seconds.max(1) as u128, rate),
            _ => transcription_cost(seconds),
        })
    }

    pub(super) fn speech_cost(self, text: &str) -> Result<i64, ApiError> {
        self.validate(AudioKind::Speech)?;
        Ok(match self {
            Self::Characters(rate) => cost_per_million(text.chars().count().max(1) as u128, rate),
            Self::Bytes(rate) => cost_per_million(text.len().max(1) as u128, rate),
            _ => speech_cost(text.chars().count()),
        })
    }

    pub(super) fn speech_cost_source(self) -> &'static str {
        match self {
            Self::Bytes(_) => "utf8_byte_tariff",
            _ => "character_tariff",
        }
    }
}

fn cost_per_million(units: u128, rate: i64) -> i64 {
    let cost = units.saturating_mul(rate as u128).div_ceil(1_000_000);
    i64::try_from(cost).unwrap_or(i64::MAX)
}

/// Audio length before the provider reports one: exact for canonical WAV, otherwise a
/// compressed-audio bitrate guess. Always at least one second.
pub(super) fn estimated_audio_seconds(audio: &[u8]) -> i64 {
    let wav_byte_rate = (audio.len() > WAV_HEADER_BYTES
        && audio.starts_with(b"RIFF")
        && audio.get(8..12) == Some(b"WAVE"))
    .then(|| audio.get(28..32))
    .flatten()
    .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    .filter(|rate| *rate > 0);
    let seconds = match wav_byte_rate {
        Some(rate) => ((audio.len() - WAV_HEADER_BYTES) as u64).div_ceil(u64::from(rate)),
        None => (audio.len() as u64).div_ceil(COMPRESSED_BYTES_PER_SECOND),
    };
    i64::try_from(seconds).unwrap_or(i64::MAX).max(1)
}

pub(super) fn settled_seconds(reported: Option<f64>, estimated: i64) -> i64 {
    match reported {
        Some(seconds) if seconds.is_finite() && seconds >= 0.0 && seconds < i64::MAX as f64 => {
            (seconds.ceil() as i64).max(1)
        }
        _ => estimated,
    }
}

pub(super) fn transcription_cost(seconds: i64) -> i64 {
    seconds
        .max(1)
        .saturating_mul(TRANSCRIPTION_MICRO_USD_PER_SECOND)
}

pub(super) fn speech_cost(characters: usize) -> i64 {
    i64::try_from(characters)
        .unwrap_or(i64::MAX)
        .max(1)
        .saturating_mul(SPEECH_MICRO_USD_PER_CHARACTER)
}

pub(super) fn usd_to_micro(usd: f64) -> Option<i64> {
    let micro = (usd * 1_000_000.0).ceil();
    (usd.is_finite() && usd >= 0.0 && micro < i64::MAX as f64).then_some(micro as i64)
}

pub(super) fn rate(
    kind: AudioKind,
    reserved_micro_usd: i64,
    pricing: AudioPricing,
    bit_id: &str,
) -> Result<HostedRateSnapshot, ApiError> {
    pricing.validate(kind)?;
    let version = if pricing == AudioPricing::Legacy {
        kind.rate_version().into()
    } else {
        let digest = blake3::hash(serde_json::to_string(&pricing)?.as_bytes());
        format!("audio-bit:{bit_id}:pricing:v1:{digest}")
    };
    let rate = HostedRateSnapshot {
        version,
        provider_pricing_available: true,
        input_micro_usd_per_million_tokens: 0,
        input_micro_usd_per_million_bytes: None,
        max_input_bytes: None,
        output_micro_usd_per_million_tokens: 0,
        request_micro_usd: reserved_micro_usd,
        context_tokens: 1,
        usd_micro_per_eur: 1_159_200,
        funding_basis_points: 0,
        api_micro_usd_per_million_ms: 20_001,
        serving_request_micro_usd: 1,
        max_request_ms: MAX_REQUEST_MS,
    };
    rate.validate()?;
    Ok(rate)
}

#[cfg(test)]
mod pricing_tests {
    use super::*;
    use serde_json::json;

    fn bit(bit_type: BitTypes, pricing: serde_json::Value) -> Bit {
        Bit {
            id: "official-audio".into(),
            bit_type,
            parameters: json!({"pricing": pricing}),
            ..Default::default()
        }
    }

    #[test]
    fn catalog_tariffs_accept_one_unit_matching_the_audio_kind() {
        for (kind, field, expected) in [
            (
                BitTypes::Stt,
                "input_micro_usd_per_million_seconds",
                AudioPricing::Seconds(7),
            ),
            (
                BitTypes::Tts,
                "input_micro_usd_per_million_characters",
                AudioPricing::Characters(7),
            ),
            (
                BitTypes::Tts,
                "input_micro_usd_per_million_bytes",
                AudioPricing::Bytes(7),
            ),
        ] {
            assert_eq!(
                AudioPricing::from_bit(&bit(kind, json!({field: 7}))).unwrap(),
                expected
            );
        }
        assert_eq!(
            AudioPricing::from_bit(&bit(BitTypes::Tts, json!(null))).unwrap(),
            AudioPricing::Legacy
        );
    }

    #[test]
    fn catalog_tariffs_reject_ambiguous_unknown_or_invalid_rates() {
        for pricing in [
            json!({}),
            json!([]),
            json!({"input_micro_usd_per_million_characters": 1, "input_micro_usd_per_million_bytes": 1}),
            json!({"input_micro_usd_per_million_tokens": 1}),
            json!({"request_micro_usd": 1}),
            json!({"input_micro_usd_per_million_characters": -1}),
            json!({"input_micro_usd_per_million_characters": 0.5}),
            json!({"input_micro_usd_per_million_characters": "1"}),
            json!({"input_micro_usd_per_million_characters": null}),
            json!({"input_micro_usd_per_million_characters": u64::MAX}),
            json!({"input_micro_usd_per_million_seconds": 1}),
        ] {
            assert!(
                AudioPricing::from_bit(&bit(BitTypes::Tts, pricing.clone())).is_err(),
                "{pricing}"
            );
        }
        assert!(
            AudioPricing::from_bit(&bit(
                BitTypes::Stt,
                json!({"input_micro_usd_per_million_bytes": 1})
            ))
            .is_err()
        );
    }

    #[test]
    fn fractional_tariffs_round_the_total_once_and_count_utf8_bytes() {
        assert_eq!(
            AudioPricing::Characters(620_000)
                .speech_cost("abc")
                .unwrap(),
            2
        );
        assert_eq!(
            AudioPricing::Characters(620_000)
                .speech_cost(&"a".repeat(1_000))
                .unwrap(),
            620
        );
        assert_eq!(
            AudioPricing::Characters(1_000_000)
                .speech_cost("é🙂")
                .unwrap(),
            2
        );
        assert_eq!(
            AudioPricing::Bytes(1_000_000).speech_cost("é🙂").unwrap(),
            6
        );
        assert_eq!(
            AudioPricing::Seconds(3_333_334)
                .transcription_cost(60)
                .unwrap(),
            201
        );
        assert_eq!(
            AudioPricing::Seconds(3_333_334)
                .transcription_cost(0)
                .unwrap(),
            4
        );
        assert_eq!(AudioPricing::Bytes(0).speech_cost("free").unwrap(), 0);
        assert_eq!(AudioPricing::Seconds(0).transcription_cost(60).unwrap(), 0);
    }

    #[test]
    fn restored_tariffs_validate_units_and_saturate_large_costs() {
        assert!(AudioPricing::Bytes(7).transcription_cost(1).is_err());
        assert!(AudioPricing::Seconds(7).speech_cost("hello").is_err());
        assert!(AudioPricing::Characters(-7).speech_cost("hello").is_err());
        assert_eq!(
            AudioPricing::Seconds(i64::MAX)
                .transcription_cost(i64::MAX)
                .unwrap(),
            i64::MAX
        );
        assert_eq!(AudioPricing::Legacy.speech_cost("é🙂").unwrap(), 60);
        assert_eq!(AudioPricing::Legacy.transcription_cost(60).unwrap(), 6_000);
    }

    #[test]
    fn rate_snapshots_identify_the_catalog_tariff_and_keep_the_reservation() {
        let pricing = AudioPricing::Characters(620_000);
        let rate = rate(AudioKind::Speech, 620, pricing, "kokoro").unwrap();
        assert_eq!(rate.request_micro_usd, 620);
        assert!(rate.version.starts_with("audio-bit:kokoro:pricing:v1:"));
        let restored = serde_json::from_value(serde_json::to_value(pricing).unwrap()).unwrap();
        assert_eq!(
            rate.version,
            super::rate(AudioKind::Speech, 2, restored, "kokoro")
                .unwrap()
                .version
        );
        assert_ne!(
            rate.version,
            super::rate(
                AudioKind::Speech,
                620,
                AudioPricing::Bytes(620_000),
                "kokoro"
            )
            .unwrap()
            .version
        );
        assert_ne!(
            rate.version,
            super::rate(
                AudioKind::Speech,
                620,
                AudioPricing::Characters(630_000),
                "kokoro"
            )
            .unwrap()
            .version
        );
    }
}
