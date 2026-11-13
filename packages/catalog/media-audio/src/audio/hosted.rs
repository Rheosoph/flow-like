use super::*;
use flow_like_model_provider::audio::{
    HostedAudioClient, HostedAudioProvider, HostedSpeechRequest, HostedTranscriptionRequest,
};

const SUPPORTED_SPEECH_FORMATS: [&str; 6] = ["mp3", "wav", "opus", "aac", "flac", "pcm"];

pub(super) fn provider_for(bit: &Bit) -> Option<ModelProvider> {
    if !matches!(bit.bit_type, BitTypes::Stt | BitTypes::Tts) {
        return None;
    }
    let provider: ModelProvider = from_value(bit.parameters.get("provider")?.clone()).ok()?;
    HostedAudioProvider::from_name(&provider.provider_name).map(|_| provider)
}

fn bearer_token<'a>(scoped: Option<&'a str>, run: Option<&'a str>) -> Option<&'a str> {
    [scoped, run]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|token| !token.is_empty())
}

fn client(context: &ExecutionContext) -> flow_like_types::Result<HostedAudioClient> {
    let state = &context.app_state;
    let authorizer = state.request_authorizer.clone();
    let token = bearer_token(
        state.hosted_model_token.as_deref(),
        context.token.as_deref(),
    );
    if token.is_none() && authorizer.is_none() {
        bail!("Hosted speech requires an access token for the API proxy");
    }
    let usage = context.model_usage_context();
    let base = usage
        .as_ref()
        .and_then(|usage| usage.api_base_url.as_deref())
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(flow_like_model_provider::embedding::proxy_config::api_base_url);
    let headers = usage
        .as_ref()
        .map(|usage| {
            [
                ("x-flow-like-app-id", usage.app_id.as_deref()),
                ("x-flow-like-run-id", usage.run_id.as_deref()),
            ]
            .into_iter()
            .filter_map(|(name, value)| {
                value
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|value| (name.to_string(), value.to_owned()))
            })
            .collect()
        })
        .unwrap_or_default();
    // Only the runtime and credential broker select the destination of this token.
    let mut client =
        HostedAudioClient::new(&base, token.unwrap_or_default())?.with_usage_headers(headers);
    if let Some(authorizer) = authorizer {
        client = client.with_authorizer(authorizer)?;
    }
    Ok(client)
}

fn speech_request_for(
    bit: &Bit,
    request: &TextToSpeechRequest,
) -> flow_like_types::Result<HostedSpeechRequest> {
    if bit.bit_type != BitTypes::Tts {
        bail!("Hosted text to speech requires a Tts catalog Bit");
    }
    if request.instructions.is_some()
        || request.language.is_some()
        || request.speed.is_some()
        || request.sample_rate.is_some()
        || request.bit_rate.is_some()
        || !request.provider_options.is_empty()
    {
        bail!("Hosted text to speech accepts only the voice and output format options");
    }
    if !SUPPORTED_SPEECH_FORMATS.contains(&request.output_format.as_str()) {
        bail!(
            "Hosted text to speech supports mp3, wav, opus, aac, flac, and pcm, not {}",
            request.output_format
        );
    }
    let request = HostedSpeechRequest {
        bit_id: bit.id.clone(),
        text: request.text.clone(),
        voice: request.voice.clone(),
        format: Some(request.output_format.clone()),
        stream: false,
    };
    request.validate()?;
    Ok(request)
}

fn transcription_request_for(
    bit: &Bit,
    request: &SpeechToTextRequest,
) -> flow_like_types::Result<HostedTranscriptionRequest> {
    if bit.bit_type != BitTypes::Stt {
        bail!("Hosted speech to text requires an Stt catalog Bit");
    }
    if request.translate || request.prompt.is_some() || !request.provider_options.is_empty() {
        bail!("Hosted speech to text accepts only the language option");
    }
    if !request.mime_type.starts_with("audio/") {
        bail!(
            "Hosted speech to text needs an audio file with a known extension such as .wav or .mp3"
        );
    }
    let request = HostedTranscriptionRequest {
        bit_id: bit.id.clone(),
        audio_base64: STANDARD.encode(&request.audio_bytes),
        mime: request.mime_type.clone(),
        language: request.language.clone(),
    };
    request.validate()?;
    Ok(request)
}

pub(super) async fn speak(
    context: &ExecutionContext,
    bit: &Bit,
    request: &TextToSpeechRequest,
) -> flow_like_types::Result<GeneratedAudio> {
    let request = speech_request_for(bit, request)?;
    let speech = client(context)?.speak(&request).await?;
    Ok(GeneratedAudio {
        bytes: speech.bytes,
        mime_type: Some(speech.mime),
        provider_metadata: json!({"bit": bit.id, "characters": request.characters()}),
    })
}

pub(super) async fn transcribe(
    context: &ExecutionContext,
    bit: &Bit,
    request: &SpeechToTextRequest,
) -> flow_like_types::Result<TranscriptionResult> {
    let request = transcription_request_for(bit, request)?;
    let result = client(context)?.transcribe(&request).await?;
    Ok(TranscriptionResult {
        text: result.text,
        provider_metadata: json!({
            "bit": bit.id,
            "language": result.language,
            "duration_ms": result.duration_ms,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bit(bit_type: BitTypes, provider: &str) -> Bit {
        Bit {
            id: "official-audio-bit".into(),
            bit_type,
            parameters: json!({"provider": {
                "provider_name": provider, "model_id": "upstream-model",
                "params": {"endpoint": "https://untrusted.example", "api_key": "not-used"}
            }}),
            ..Default::default()
        }
    }

    fn speech() -> TextToSpeechRequest {
        TextToSpeechRequest {
            text: "Hello".into(),
            voice: Some("nova".into()),
            instructions: None,
            language: None,
            output_format: "mp3".into(),
            speed: None,
            sample_rate: None,
            bit_rate: None,
            provider_options: HashMap::new(),
        }
    }

    fn transcription() -> SpeechToTextRequest {
        SpeechToTextRequest {
            audio_bytes: b"RIFF".to_vec(),
            file_name: "clip.wav".into(),
            mime_type: "audio/wav".into(),
            language: Some("en".into()),
            prompt: None,
            response_format: "json".into(),
            translate: false,
            provider_options: HashMap::new(),
        }
    }

    #[test]
    fn only_stt_and_tts_bits_with_a_hosted_audio_provider_use_the_relay() {
        for name in [
            "hosted:openrouter",
            "hosted:openai",
            "hosted:groq",
            "hosted:gemini",
        ] {
            assert!(provider_for(&bit(BitTypes::Stt, name)).is_some(), "{name}");
            assert!(provider_for(&bit(BitTypes::Tts, name)).is_some(), "{name}");
        }
        for name in ["custom:openai", "local:whisper", "hosted:cloudflare"] {
            assert!(provider_for(&bit(BitTypes::Stt, name)).is_none(), "{name}");
        }
        assert!(provider_for(&bit(BitTypes::Llm, "hosted:openai")).is_none());
    }

    #[test]
    fn speech_request_forwards_the_catalog_id_without_provider_secrets() {
        let request = speech_request_for(&bit(BitTypes::Tts, "hosted:openai"), &speech()).unwrap();
        let body = json!(request);
        assert_eq!(body["bit_id"], "official-audio-bit");
        assert_eq!(body["voice"], "nova");
        assert_eq!(body["format"], "mp3");
        for field in ["endpoint", "api_key", "provider", "model"] {
            assert!(body.get(field).is_none(), "{field}");
        }
    }

    #[test]
    fn speech_rejects_unmetered_options_wrong_bits_and_unsupported_formats() {
        let tts = bit(BitTypes::Tts, "hosted:openai");
        assert!(speech_request_for(&bit(BitTypes::Stt, "hosted:openai"), &speech()).is_err());
        let mut request = speech();
        request.speed = Some(1.2);
        assert!(speech_request_for(&tts, &request).is_err());
        let mut request = speech();
        request.provider_options.insert("n".into(), json!(10));
        assert!(speech_request_for(&tts, &request).is_err());
        let mut request = speech();
        request.output_format = "mulaw".into();
        assert!(speech_request_for(&tts, &request).is_err());
        let mut request = speech();
        request.text = "x".repeat(4097);
        assert!(speech_request_for(&tts, &request).is_err());
    }

    #[test]
    fn transcription_request_encodes_audio_and_rejects_unsupported_options() {
        let stt = bit(BitTypes::Stt, "hosted:groq");
        let request = transcription_request_for(&stt, &transcription()).unwrap();
        assert_eq!(request.bit_id, "official-audio-bit");
        assert_eq!(request.decode().unwrap(), b"RIFF");
        assert_eq!(request.language.as_deref(), Some("en"));
        let mut input = transcription();
        input.translate = true;
        assert!(transcription_request_for(&stt, &input).is_err());
        let mut input = transcription();
        input.prompt = Some("names".into());
        assert!(transcription_request_for(&stt, &input).is_err());
        let mut input = transcription();
        input.mime_type = "application/octet-stream".into();
        assert!(transcription_request_for(&stt, &input).is_err());
        assert!(
            transcription_request_for(&bit(BitTypes::Tts, "hosted:groq"), &transcription())
                .is_err()
        );
    }

    #[test]
    fn scoped_token_takes_precedence_over_the_run_token() {
        assert_eq!(bearer_token(Some(" scoped "), Some("run")), Some("scoped"));
        assert_eq!(bearer_token(Some(" "), Some("run")), Some("run"));
        assert_eq!(bearer_token(None, None), None);
    }
}
