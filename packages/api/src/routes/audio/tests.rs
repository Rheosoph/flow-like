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
fn catalog_defaults_reach_upstream_and_durable_jobs_without_overriding_requests() {
    let mut bit = audio_bit(BitTypes::Tts, "hosted:openrouter");
    bit.parameters["default_voice"] = json!("af_heart");
    let (provider, implementation) = catalog_provider(&bit).unwrap();
    for explicit in [None, Some("af_bella")] {
        let mut payload = AudioPayload::Speech(HostedSpeechRequest {
            bit_id: bit.id.clone(),
            text: "Hello".into(),
            voice: explicit.map(str::to_owned),
            format: Some("wav".into()),
            stream: false,
        });
        payload.apply_catalog_defaults(&bit).unwrap();
        let job = HostedAudioJob {
            payload,
            provider: provider.clone(),
            pricing: AudioPricing::Legacy,
        };
        let wire = serde_json::to_value(&job).unwrap();
        let restored: HostedAudioJob = serde_json::from_value(wire).unwrap();
        let AudioPayload::Speech(request) = restored.payload else {
            panic!("speech job");
        };
        let upstream = flow_like::flow_like_model_provider::audio::speech_request(
            implementation,
            "hexgrad/kokoro-82m",
            &request,
        )
        .unwrap();
        let flow_like::flow_like_model_provider::audio::UpstreamBody::Json(body) = upstream.body
        else {
            panic!("JSON request");
        };
        assert_eq!(body["voice"], explicit.unwrap_or("af_heart"));
    }

    let mut bit = audio_bit(BitTypes::Stt, "hosted:openrouter");
    bit.parameters["default_language"] = json!("pt-BR");
    for explicit in [None, Some("en")] {
        let mut payload = AudioPayload::Transcription(HostedTranscriptionRequest {
            bit_id: bit.id.clone(),
            audio_base64: "AQ==".into(),
            mime: "audio/wav".into(),
            language: explicit.map(str::to_owned),
        });
        payload.apply_catalog_defaults(&bit).unwrap();
        let AudioPayload::Transcription(request) = payload else {
            panic!("transcription job");
        };
        assert_eq!(
            request.language.as_deref(),
            Some(explicit.unwrap_or("pt-BR"))
        );
    }
}

#[test]
fn catalog_defaults_follow_the_speech_request_contracts() {
    for (field, values) in [
        (
            "default_voice",
            vec![
                json!(""),
                json!("with space"),
                json!("x".repeat(65)),
                json!(7),
            ],
        ),
        (
            "default_language",
            vec![
                json!(""),
                json!("e"),
                json!("en_US"),
                json!("x".repeat(17)),
                json!([]),
            ],
        ),
        (
            "max_input_characters",
            vec![json!(0), json!(-1), json!(4097), json!(2.5), json!("2000")],
        ),
    ] {
        for value in values {
            let mut bit = audio_bit(BitTypes::Tts, "hosted:openrouter");
            bit.parameters[field] = value;
            assert!(
                catalog_provider(&bit).is_err(),
                "{field}: {}",
                bit.parameters[field]
            );
        }
    }
    let bit = audio_bit(BitTypes::Tts, "hosted:openrouter");
    assert!(catalog_provider(&bit).is_ok());
}

#[test]
fn catalog_character_limits_reject_long_input_without_truncating_unicode() {
    let mut bit = audio_bit(BitTypes::Tts, "hosted:openrouter");
    bit.parameters["max_input_characters"] = json!(2);
    assert!(catalog_provider(&bit).is_ok());
    let mut payload = AudioPayload::Speech(HostedSpeechRequest {
        bit_id: bit.id.clone(),
        text: "é🙂".into(),
        voice: Some("thalia".into()),
        format: None,
        stream: false,
    });
    payload.apply_catalog_defaults(&bit).unwrap();
    bit.parameters["max_input_characters"] = json!(1);
    assert!(payload.apply_catalog_defaults(&bit).is_err());
    assert!(
        payload
            .validate_character_limit(catalog_defaults(&bit).unwrap().max_input_characters)
            .is_err()
    );
    let AudioPayload::Speech(request) = payload else {
        panic!("speech request");
    };
    assert_eq!(request.text, "é🙂");
    assert_eq!(request.voice.as_deref(), Some("thalia"));
    bit.bit_type = BitTypes::Stt;
    assert!(catalog_provider(&bit).is_err());
}

#[test]
fn durable_audio_jobs_freeze_prices_and_old_jobs_retain_legacy_tariffs() {
    let mut bit = audio_bit(BitTypes::Tts, "hosted:openrouter");
    bit.parameters["pricing"] = json!({"input_micro_usd_per_million_bytes": 15_000_000});
    let (provider, _) = catalog_provider(&bit).unwrap();
    let job = HostedAudioJob {
        payload: AudioPayload::Speech(HostedSpeechRequest {
            bit_id: bit.id.clone(),
            text: "é🙂".into(),
            voice: None,
            format: None,
            stream: false,
        }),
        provider,
        pricing: AudioPricing::from_bit(&bit).unwrap(),
    };
    assert_eq!(job.payload.reserved_cost(job.pricing).unwrap(), 90);
    let mut saved = serde_json::to_value(job).unwrap();
    bit.parameters["pricing"] = json!({"input_micro_usd_per_million_characters": 100_000_000});
    let restored: HostedAudioJob = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(
        restored.payload.reserved_cost(restored.pricing).unwrap(),
        90
    );
    assert_eq!(
        restored
            .payload
            .reserved_cost(AudioPricing::from_bit(&bit).unwrap())
            .unwrap(),
        200
    );

    saved.as_object_mut().unwrap().remove("pricing");
    let legacy: HostedAudioJob = serde_json::from_value(saved).unwrap();
    assert_eq!(legacy.pricing, AudioPricing::Legacy);
    assert_eq!(legacy.payload.reserved_cost(legacy.pricing).unwrap(), 60);
}

#[test]
fn audio_catalog_validation_rejects_tariffs_for_a_different_modality() {
    let mut bit = audio_bit(BitTypes::Tts, "hosted:openrouter");
    bit.parameters["pricing"] = json!({"input_micro_usd_per_million_seconds": 15_000_000});
    assert!(catalog_provider(&bit).is_err());
    bit.bit_type = BitTypes::Stt;
    assert!(catalog_provider(&bit).is_ok());
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
        let rate = metering::rate(kind, reserved, AudioPricing::Legacy, "official-audio").unwrap();
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

#[tokio::test]
async fn accepted_audio_finishes_after_the_http_waiter_is_cancelled() {
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let (settled_tx, settled_rx) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn(complete_independently(async move {
        started_tx.send(()).unwrap();
        release_rx.await.unwrap();
        settled_tx.send(()).unwrap();
        Ok(())
    }));
    started_rx.await.unwrap();
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), settled_rx)
        .await
        .unwrap()
        .unwrap();
}

fn speech_payload(stream: bool, format: Option<&str>) -> AudioPayload {
    AudioPayload::Speech(HostedSpeechRequest {
        bit_id: "official-audio".into(),
        text: "Hello".into(),
        voice: None,
        format: format.map(str::to_owned),
        stream,
    })
}

#[test]
fn streamed_speech_requires_pcm_and_a_pcm_capable_provider() {
    let pricing = AudioPricing::Legacy;
    for format in [None, Some("mp3"), Some("wav")] {
        let error = speech_payload(true, format)
            .reserved_cost(pricing)
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{format:?}");
    }
    assert!(
        speech_payload(true, Some("pcm"))
            .reserved_cost(pricing)
            .is_ok()
    );
    assert!(speech_payload(false, None).reserved_cost(pricing).is_ok());
}

#[test]
fn providers_without_raw_pcm_answer_with_the_public_stream_code() {
    let streamed = speech_payload(true, Some("pcm"));
    let model = "openai/gpt-4o-mini-tts";
    for provider in HostedAudioProvider::ALL {
        let outcome = streamed.ensure_stream_supported(provider, model);
        if provider.stream_pcm_sample_rate(model).is_some() {
            assert!(outcome.is_ok(), "{provider:?}");
        } else {
            let error = outcome.unwrap_err();
            assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{provider:?}");
            assert_eq!(error.public_code(), "SPEECH_STREAM_UNSUPPORTED");
        }
        assert!(
            speech_payload(false, Some("pcm"))
                .ensure_stream_supported(provider, model)
                .is_ok()
        );
    }
    assert_eq!(
        streamed
            .ensure_stream_supported(HostedAudioProvider::OpenRouter, "hexgrad/kokoro-82m")
            .unwrap_err()
            .public_code(),
        "SPEECH_STREAM_UNSUPPORTED"
    );
    assert_eq!(
        HostedAudioProvider::OpenAi.stream_pcm_sample_rate("tts-1"),
        Some(24_000)
    );
    assert_eq!(
        HostedAudioProvider::Groq.stream_pcm_sample_rate("any"),
        None
    );
}
