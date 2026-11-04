use super::*;
use HostedAudioProvider::*;

fn transcription() -> HostedTranscriptionRequest {
    HostedTranscriptionRequest {
        bit_id: "stt-bit".into(),
        audio_base64: STANDARD.encode(b"RIFFdata"),
        mime: "audio/wav".into(),
        language: None,
    }
}

fn speech() -> HostedSpeechRequest {
    HostedSpeechRequest {
        bit_id: "tts-bit".into(),
        text: "Hello there".into(),
        ..Default::default()
    }
}

fn json_body(request: &UpstreamRequest) -> &Value {
    match &request.body {
        UpstreamBody::Json(value) => value,
        UpstreamBody::Multipart { .. } => panic!("expected a JSON body"),
    }
}

#[test]
fn provider_names_parse_only_the_hosted_scheme() {
    for provider in HostedAudioProvider::ALL {
        assert_eq!(
            HostedAudioProvider::from_name(provider.provider_name()),
            Some(provider)
        );
    }
    assert_eq!(
        HostedAudioProvider::from_name(" Hosted:OpenAI "),
        Some(OpenAi)
    );
    for name in [
        "custom:openai",
        "openai",
        "hosted:cloudflare",
        "premium",
        "",
    ] {
        assert_eq!(HostedAudioProvider::from_name(name), None, "{name}");
    }
}

#[test]
fn transcription_validation_bounds_audio_mime_and_language() {
    transcription().validate().unwrap();
    assert_eq!(transcription().decode().unwrap(), b"RIFFdata");
    let mut request = transcription();
    request.mime = "audio/webm;codecs=opus".into();
    request.validate().unwrap();
    for mime in ["text/plain", "audio", "image/png", "audio/we bm"] {
        let mut invalid = transcription();
        invalid.mime = mime.into();
        assert!(invalid.validate().is_err(), "{mime}");
    }
    let mut invalid = transcription();
    invalid.language = Some("en US".into());
    assert!(invalid.validate().is_err());
    let mut invalid = transcription();
    invalid.audio_base64.clear();
    assert!(invalid.validate().is_err());
    let mut invalid = transcription();
    invalid.audio_base64 = "A".repeat(MAX_AUDIO_BYTES.div_ceil(3) * 4 + 4);
    assert!(invalid.validate().is_err());
    let mut invalid = transcription();
    invalid.audio_base64 = "not base64!".into();
    assert!(invalid.decode().is_err());
    let mut invalid = transcription();
    invalid.bit_id = " ".into();
    assert!(invalid.validate().is_err());
}

#[test]
fn audio_over_twenty_five_megabytes_is_rejected_after_decoding() {
    let mut request = transcription();
    request.audio_base64 = STANDARD.encode(vec![0u8; MAX_AUDIO_BYTES]);
    assert_eq!(request.decode().unwrap().len(), MAX_AUDIO_BYTES);
    request.audio_base64 = STANDARD.encode(vec![0u8; MAX_AUDIO_BYTES + 1]);
    assert!(request.decode().is_err());
}

#[test]
fn speech_validation_bounds_text_voice_and_format() {
    speech().validate().unwrap();
    let mut request = speech();
    request.text = "x".repeat(MAX_SPEECH_CHARS);
    request.validate().unwrap();
    request.text.push('x');
    assert!(request.validate().is_err());
    let mut request = speech();
    request.text = "é".repeat(MAX_SPEECH_CHARS);
    request.validate().unwrap();
    assert_eq!(request.characters(), MAX_SPEECH_CHARS);
    let long_voice = "v".repeat(65);
    for voice in ["", "a b", "../x", long_voice.as_str()] {
        let mut invalid = speech();
        invalid.voice = Some(voice.into());
        assert!(invalid.validate().is_err(), "{voice}");
    }
    let mut invalid = speech();
    invalid.format = Some("ogg".into());
    assert!(invalid.validate().is_err());
    let mut invalid = speech();
    invalid.text = "  ".into();
    assert!(invalid.validate().is_err());
}

#[test]
fn requests_reject_unknown_fields_such_as_provider_overrides() {
    for field in ["provider", "endpoint", "api_key", "model"] {
        let mut body = serde_json::to_value(speech()).unwrap();
        body[field] = json!("injected");
        assert!(serde_json::from_value::<HostedSpeechRequest>(body).is_err());
        let mut body = serde_json::to_value(transcription()).unwrap();
        body[field] = json!("injected");
        assert!(serde_json::from_value::<HostedTranscriptionRequest>(body).is_err());
    }
}

#[test]
fn speech_wire_matches_each_provider() {
    let mut request = speech();
    let openai = speech_request(OpenAi, "gpt-4o-mini-tts", &request).unwrap();
    assert_eq!(openai.url, "https://api.openai.com/v1/audio/speech");
    assert_eq!(
        json_body(&openai),
        &json!({"model":"gpt-4o-mini-tts","input":"Hello there","voice":"alloy","response_format":"mp3"})
    );
    let groq = speech_request(Groq, "canopylabs/orpheus-v1-english", &request).unwrap();
    assert_eq!(json_body(&groq)["voice"], "troy");
    assert_eq!(json_body(&groq)["response_format"], "wav");
    let xai = speech_request(Xai, "grok", &request).unwrap();
    assert_eq!(xai.url, "https://api.x.ai/v1/tts");
    assert_eq!(json_body(&xai)["output_format"]["codec"], "mp3");
    assert_eq!(json_body(&xai)["voice_id"], "eve");
    let gemini = speech_request(Gemini, "gemini-2.5-flash-preview-tts", &request).unwrap();
    assert_eq!(
        gemini.url,
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-preview-tts:generateContent"
    );
    assert_eq!(
        json_body(&gemini)["generationConfig"]["speechConfig"]["voiceConfig"]["prebuiltVoiceConfig"]
            ["voiceName"],
        "Kore"
    );
    assert!(speech_request(Gemini, "../x", &request).is_err());
    assert!(speech_request(Mistral, "voxtral-mini-tts-2603", &request).is_err());
    request.voice = Some("voice-1".into());
    let mistral = speech_request(Mistral, "voxtral-mini-tts-2603", &request).unwrap();
    assert_eq!(json_body(&mistral)["voice_id"], "voice-1");
    request.format = Some("pcm".into());
    let together = speech_request(Together, "orpheus", &request).unwrap();
    assert_eq!(json_body(&together)["response_format"], "raw");
    let router = speech_request(OpenRouter, "elevenlabs/eleven-turbo-v2", &speech()).unwrap();
    assert_eq!(router.url, "https://openrouter.ai/api/v1/audio/speech");
}

#[test]
fn transcription_wire_matches_each_provider() {
    let mut request = transcription();
    request.language = Some("de".into());
    let openai = transcription_request(OpenAi, "gpt-4o-mini-transcribe", &request).unwrap();
    assert_eq!(openai.url, "https://api.openai.com/v1/audio/transcriptions");
    match &openai.body {
        UpstreamBody::Multipart {
            fields,
            file_name,
            mime,
        } => {
            assert!(fields.contains(&("response_format".into(), "json".into())));
            assert!(fields.contains(&("language".into(), "de".into())));
            assert_eq!(file_name, "audio.wav");
            assert_eq!(mime, "audio/wav");
        }
        UpstreamBody::Json(_) => panic!("expected multipart"),
    }
    let groq = transcription_request(Groq, "whisper-large-v3-turbo", &request).unwrap();
    assert!(matches!(&groq.body, UpstreamBody::Multipart { fields, .. }
        if fields.contains(&("response_format".into(), "verbose_json".into()))));
    let xai = transcription_request(Xai, "grok", &request).unwrap();
    assert_eq!(xai.url, "https://api.x.ai/v1/stt");
    let router = transcription_request(OpenRouter, "google/gemini-2.5-flash", &request).unwrap();
    assert_eq!(router.url, "https://openrouter.ai/api/v1/chat/completions");
    let content = &json_body(&router)["messages"][0]["content"];
    assert_eq!(content[1]["input_audio"]["format"], "wav");
    assert!(content[0]["text"].as_str().unwrap().contains("de"));
    let gemini = transcription_request(Gemini, "gemini-2.5-flash", &request).unwrap();
    assert_eq!(
        json_body(&gemini)["contents"][0]["parts"][1]["inlineData"]["mimeType"],
        "audio/wav"
    );
}

fn parse(provider: HostedAudioProvider, value: Value) -> Result<ParsedTranscription> {
    parse_transcription_response(provider, &serde_json::to_vec(&value).unwrap())
}

#[test]
fn transcription_responses_parse_per_provider() {
    let openai = parse(
        OpenAi,
        json!({"text":" Hello world ","language":"english","duration":3.25}),
    )
    .unwrap();
    assert_eq!(openai.transcription.text, "Hello world");
    assert_eq!(openai.transcription.language.as_deref(), Some("english"));
    assert_eq!(openai.transcription.duration_ms, Some(3250));
    assert_eq!(openai.billed_seconds, Some(3.25));
    let usage = parse(
        OpenAi,
        json!({"text":"hi","usage":{"type":"duration","seconds":7}}),
    )
    .unwrap();
    assert_eq!(usage.billed_seconds, Some(7.0));
    let router = parse(
        OpenRouter,
        json!({"choices":[{"message":{"content":"spoken words"}}],"usage":{"cost":0.0012}}),
    )
    .unwrap();
    assert_eq!(router.transcription.text, "spoken words");
    assert_eq!(router.provider_cost_usd, Some(0.0012));
    assert_eq!(router.billed_seconds, None);
    let parts = parse(
        OpenRouter,
        json!({"choices":[{"message":{"content":[{"type":"text","text":"a "},{"type":"text","text":"b"}]}}]}),
    )
    .unwrap();
    assert_eq!(parts.transcription.text, "a b");
    let gemini = parse(
        Gemini,
        json!({"candidates":[{"content":{"parts":[{"text":"one"},{"text":"two"}]}}]}),
    )
    .unwrap();
    assert_eq!(gemini.transcription.text, "one\ntwo");
    assert_eq!(gemini.provider_cost_usd, None);
    assert_eq!(
        parse(Xai, json!({"text":"grok text"}))
            .unwrap()
            .transcription
            .text,
        "grok text"
    );
    assert_eq!(
        parse(Mistral, json!({"text":"voxtral"}))
            .unwrap()
            .transcription
            .text,
        "voxtral"
    );
    let mut free = json!({"text":"x","usage":{"cost":0.5}});
    free["duration"] = json!(-1.0);
    let ignored = parse(Groq, free).unwrap();
    assert_eq!(ignored.provider_cost_usd, None);
    assert_eq!(ignored.billed_seconds, None);
}

#[test]
fn transcription_errors_cover_empty_and_malformed_bodies() {
    assert!(parse(OpenAi, json!({"text":"  "})).is_err());
    assert!(parse(OpenAi, json!({"other":1})).is_err());
    assert!(parse(Gemini, json!({"candidates":[]})).is_err());
    assert!(parse(OpenRouter, json!({"choices":[]})).is_err());
    assert!(parse_transcription_response(OpenAi, b"not json").is_err());
}

#[test]
fn speech_responses_parse_per_provider() {
    let binary =
        parse_speech_response(OpenAi, Some("audio/mpeg; charset=x"), b"ID3", "mp3").unwrap();
    assert_eq!(binary.mime, "audio/mpeg");
    assert_eq!(binary.bytes, b"ID3");
    let fallback =
        parse_speech_response(Groq, Some("application/octet-stream"), b"RIFF", "wav").unwrap();
    assert_eq!(fallback.mime, "audio/wav");
    assert!(parse_speech_response(OpenAi, Some("application/json"), b"{}", "mp3").is_err());
    assert!(parse_speech_response(OpenAi, None, b"", "mp3").is_err());
    let mistral_body =
        serde_json::to_vec(&json!({"audio_data": STANDARD.encode(b"mp3bytes")})).unwrap();
    let mistral = parse_speech_response(Mistral, None, &mistral_body, "mp3").unwrap();
    assert_eq!(mistral.bytes, b"mp3bytes");
    assert_eq!(mistral.mime, "audio/mpeg");
    assert!(parse_speech_response(Mistral, None, b"{}", "mp3").is_err());
    let pcm = vec![0u8; 100];
    let gemini_body = serde_json::to_vec(&json!({"candidates":[{"content":{"parts":[
        {"inlineData":{"mimeType":"audio/L16;codec=pcm;rate=24000","data":STANDARD.encode(&pcm)}}
    ]}}]}))
    .unwrap();
    let wav = parse_speech_response(Gemini, None, &gemini_body, "wav").unwrap();
    assert_eq!(wav.mime, "audio/wav");
    assert_eq!(&wav.bytes[..4], b"RIFF");
    assert_eq!(wav.bytes.len(), 144);
    let raw = parse_speech_response(Gemini, None, &gemini_body, "pcm").unwrap();
    assert_eq!(raw.bytes, pcm);
    assert!(parse_speech_response(Gemini, None, br#"{"candidates":[]}"#, "wav").is_err());
}

#[test]
fn client_base_resolves_the_audio_resource() {
    assert_eq!(
        audio_base("https://api.example.com", false)
            .unwrap()
            .as_str(),
        "https://api.example.com/api/v1/audio"
    );
    assert_eq!(
        audio_base("https://api.example.com/api/v1/", false)
            .unwrap()
            .as_str(),
        "https://api.example.com/api/v1/audio"
    );
    assert_eq!(
        audio_base("https://api.example.com/gateway", true)
            .unwrap()
            .as_str(),
        "https://api.example.com/gateway/audio"
    );
    assert!(audio_base("https://user:pw@example.com", false).is_err());
    assert!(audio_base("ftp://example.com", false).is_err());
}
