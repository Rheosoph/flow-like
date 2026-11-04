use crate::{error::ApiError, usage_accounting::HostedRateSnapshot};

/// Built-in tariff for every hosted audio provider: about USD 0.006 per transcribed minute.
pub(super) const TRANSCRIPTION_MICRO_USD_PER_SECOND: i64 = 100;
/// Built-in tariff for every hosted audio provider: USD 30 per million synthesized characters.
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
) -> Result<HostedRateSnapshot, ApiError> {
    let rate = HostedRateSnapshot {
        version: kind.rate_version().into(),
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
