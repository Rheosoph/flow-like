use super::*;
use axum::http::StatusCode;
use serde_json::json;

fn audio_bit(bit_type: BitTypes, provider_name: &str) -> Bit {
    Bit {
        id: "official-audio".into(),
        bit_type,
        parameters: json!({"provider": {
            "provider_name": provider_name,
            "model_id": "whisper-large-v3-turbo",
            "params": {"tier": "FREE"},
        }}),
        ..Default::default()
    }
}

#[test]
fn every_hosted_speech_provider_is_accepted_for_both_bit_types() {
    for provider in HostedAudioProvider::ALL {
        for bit_type in [BitTypes::Stt, BitTypes::Tts] {
            let bit = audio_bit(bit_type, provider.provider_name());
            let (_, implementation) = catalog_provider(&bit).unwrap();
            assert_eq!(implementation, provider);
        }
    }
}

#[test]
fn catalog_rejects_client_chosen_providers_credentials_and_local_files() {
    let bit = audio_bit(BitTypes::Stt, "hosted:groq");
    for provider in [
        "custom:openai",
        "local:whisper",
        "hosted:cloudflare",
        "openai",
        "premium",
    ] {
        let mut invalid = bit.clone();
        invalid.parameters["provider"]["provider_name"] = json!(provider);
        assert!(catalog_provider(&invalid).is_err(), "{provider}");
    }
    let mut invalid = bit.clone();
    invalid.bit_type = BitTypes::Llm;
    assert!(catalog_provider(&invalid).is_err());
    for field in ["endpoint", "api_key", "headers"] {
        let mut invalid = bit.clone();
        invalid.parameters["provider"]["params"][field] = json!("injected");
        assert!(catalog_provider(&invalid).is_err(), "{field}");
    }
    for model in ["", "../other", "model?query=true", "https://other.invalid"] {
        let mut invalid = bit.clone();
        invalid.parameters["provider"]["model_id"] = json!(model);
        assert!(catalog_provider(&invalid).is_err(), "{model}");
    }
    let mut invalid = bit.clone();
    invalid.dependencies.push("weight".into());
    assert!(catalog_provider(&invalid).is_err());
    let mut invalid = bit.clone();
    invalid.parameters["assets"] = json!([{"id": "weights"}]);
    assert!(catalog_provider(&invalid).is_err());
    let mut invalid = bit;
    invalid.parameters["provider"]["params"]["tier"] = json!("  ");
    assert!(catalog_provider(&invalid).is_err());
}

#[test]
fn unconfigured_providers_answer_503_with_a_clear_message() {
    for provider in HostedAudioProvider::ALL {
        for value in [None, Some(""), Some("   ")] {
            let error = providers::require_key(provider, value).unwrap_err();
            assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert!(
                error.public_message().unwrap().contains(provider.label()),
                "{error:?}"
            );
        }
        assert_eq!(
            providers::require_key(provider, Some(" key ")).unwrap(),
            "key"
        );
    }
}

#[test]
fn unsupported_provider_names_are_bad_requests() {
    for name in ["custom:openai", "hosted:cloudflare", "gemini", ""] {
        let error = providers::from_name(name).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{name}");
    }
}

#[test]
fn platform_keys_use_distinct_config_names() {
    let mut names: Vec<_> = HostedAudioProvider::ALL
        .into_iter()
        .map(HostedAudioProvider::secret_name)
        .collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), HostedAudioProvider::ALL.len());
    assert!(names.contains(&"OPENROUTER_API_KEY"));
    assert!(names.contains(&"HOSTED_OPENAI_API_KEY"));
}

fn canonical_wav(seconds: u32) -> Vec<u8> {
    let byte_rate = 32_000u32;
    let data_len = (byte_rate * seconds) as usize;
    let mut wav = Vec::from(&b"RIFF"[..]);
    wav.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&16_000u32.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data_len as u32).to_le_bytes());
    wav.resize(wav.len() + data_len, 0);
    wav
}

#[test]
fn transcription_is_metered_by_audio_seconds() {
    assert_eq!(metering::estimated_audio_seconds(&canonical_wav(5)), 5);
    assert_eq!(metering::estimated_audio_seconds(&canonical_wav(1)), 1);
    assert_eq!(metering::estimated_audio_seconds(&[0u8; 3]), 1);
    assert_eq!(metering::estimated_audio_seconds(&vec![0u8; 120_001]), 11);
    assert_eq!(metering::transcription_cost(60), 6_000);
    assert_eq!(metering::transcription_cost(0), 100);
    assert_eq!(metering::settled_seconds(Some(3.2), 10), 4);
    assert_eq!(metering::settled_seconds(Some(0.0), 10), 1);
    assert_eq!(metering::settled_seconds(Some(f64::NAN), 10), 10);
    assert_eq!(metering::settled_seconds(None, 10), 10);
}

#[test]
fn speech_is_metered_by_characters() {
    assert_eq!(metering::speech_cost(1_000_000), 30_000_000);
    assert_eq!(metering::speech_cost(4096), 122_880);
    assert_eq!(metering::speech_cost(0), 30);
}

#[test]
fn provider_reported_cost_rounds_up_and_rejects_bad_numbers() {
    assert_eq!(metering::usd_to_micro(0.0), Some(0));
    assert_eq!(metering::usd_to_micro(0.000_000_1), Some(1));
    assert_eq!(metering::usd_to_micro(0.0012), Some(1_200));
    assert_eq!(metering::usd_to_micro(-1.0), None);
    assert_eq!(metering::usd_to_micro(f64::INFINITY), None);
}

#[test]
fn built_in_rates_are_valid_and_reserve_the_estimated_cost() {
    for (kind, reserved) in [
        (AudioKind::Transcription, 600),
        (AudioKind::Speech, 122_880),
    ] {
        let rate = metering::rate(kind, reserved).unwrap();
        assert_eq!(rate.request_micro_usd, reserved);
        assert!(rate.version.starts_with("audio-"));
    }
    assert_eq!(
        AudioKind::Transcription.invocation_kind(),
        "audio_transcription"
    );
    assert_eq!(AudioKind::Speech.invocation_kind(), "audio_speech");
}

#[test]
fn request_bodies_are_bounded() {
    assert!(MAX_TRANSCRIPTION_BODY_BYTES > MAX_AUDIO_BYTES * 4 / 3);
    assert!(MAX_SPEECH_BODY_BYTES >= 4096 * 4);
}
