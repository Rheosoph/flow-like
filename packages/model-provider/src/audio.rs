//! Speech-to-text and text-to-speech through the hosted model gateway.
//!
//! The request and response types are the wire contract of `/audio/transcriptions` and
//! `/audio/speech`. The provider helpers are pure: the gateway builds an upstream request with
//! them and parses the upstream answer, so every provider quirk lives in one place.

use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types_contracts::authorization::{
    AuthorizationAttribution, RequestAuthorizer, ResourceAudience,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::authorization::ScopedRequestAuthorizer;

pub const MAX_AUDIO_BYTES: usize = 25 * 1024 * 1024;
pub const MAX_SPEECH_CHARS: usize = 4096;
const MAX_TRANSCRIPTION_RESPONSE_BYTES: usize = 1024 * 1024;
const SPEECH_FORMATS: [&str; 6] = ["mp3", "wav", "opus", "aac", "flac", "pcm"];
const TRANSCRIBE_PROMPT: &str = "Transcribe this audio. Return only the transcript text.";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedTranscriptionRequest {
    pub bit_id: String,
    pub audio_base64: String,
    pub mime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

pub fn base_mime(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn valid_bit_id(bit_id: &str) -> bool {
    !bit_id.trim().is_empty() && bit_id.len() <= 512
}

fn valid_audio_mime(mime: &str) -> bool {
    let base = base_mime(mime);
    let Some((kind, subtype)) = base.split_once('/') else {
        return false;
    };
    mime.len() <= 128
        && (kind == "audio" || matches!(base.as_str(), "video/webm" | "video/mp4"))
        && !subtype.is_empty()
        && subtype
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-+.".contains(&byte))
}

fn audio_extension(mime: &str) -> &'static str {
    match base_mime(mime).as_str() {
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => "wav",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/mp4" | "audio/m4a" | "audio/x-m4a" | "audio/aac" | "video/mp4" => "m4a",
        "audio/webm" | "video/webm" => "webm",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/flac" | "audio/x-flac" => "flac",
        _ => "wav",
    }
}

impl HostedTranscriptionRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            valid_bit_id(&self.bit_id),
            "Hosted transcription requires a catalog Bit ID"
        );
        ensure!(
            valid_audio_mime(&self.mime),
            "Audio MIME type must be an audio type such as audio/wav or audio/webm"
        );
        ensure!(
            self.language.as_deref().is_none_or(|language| {
                (2..=16).contains(&language.len())
                    && language
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            }),
            "Language must be a short language code such as en or pt-BR"
        );
        ensure!(
            !self.audio_base64.is_empty()
                && self.audio_base64.len() <= MAX_AUDIO_BYTES.div_ceil(3) * 4,
            "Audio must not be empty and must be at most 25 MB"
        );
        Ok(())
    }

    pub fn decode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = STANDARD
            .decode(&self.audio_base64)
            .context("Audio contains invalid base64 data")?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= MAX_AUDIO_BYTES,
            "Audio must not be empty and must be at most 25 MB"
        );
        Ok(bytes)
    }

    pub fn file_name(&self) -> String {
        format!("audio.{}", audio_extension(&self.mime))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HostedTranscription {
    pub text: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedSpeechRequest {
    pub bit_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}

impl HostedSpeechRequest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            valid_bit_id(&self.bit_id),
            "Hosted speech requires a catalog Bit ID"
        );
        ensure!(
            !self.text.trim().is_empty() && self.text.chars().count() <= MAX_SPEECH_CHARS,
            "Speech text must contain between 1 and 4096 characters"
        );
        ensure!(
            self.voice.as_deref().is_none_or(|voice| {
                !voice.is_empty()
                    && voice.len() <= 64
                    && voice
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
            }),
            "Voice must be 1 to 64 letters, digits, or - _ . :"
        );
        ensure!(
            self.format
                .as_deref()
                .is_none_or(|format| SPEECH_FORMATS.contains(&format)),
            "Speech format must be one of mp3, wav, opus, aac, flac, or pcm"
        );
        Ok(())
    }

    pub fn requested_format(&self) -> &str {
        self.format.as_deref().unwrap_or("mp3")
    }

    pub fn characters(&self) -> usize {
        self.text.chars().count()
    }
}

#[derive(Clone, Debug)]
pub struct HostedSpeech {
    pub bytes: Vec<u8>,
    pub mime: String,
}

fn format_mime(format: &str) -> &'static str {
    match format {
        "wav" => "audio/wav",
        "pcm" => "audio/pcm",
        "opus" => "audio/opus",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        _ => "audio/mpeg",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HostedAudioProvider {
    OpenRouter,
    OpenAi,
    Groq,
    Mistral,
    Together,
    Xai,
    Gemini,
}

impl HostedAudioProvider {
    pub const ALL: [Self; 7] = [
        Self::OpenRouter,
        Self::OpenAi,
        Self::Groq,
        Self::Mistral,
        Self::Together,
        Self::Xai,
        Self::Gemini,
    ];

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|provider| provider.provider_name() == name)
    }

    pub fn provider_name(self) -> &'static str {
        match self {
            Self::OpenRouter => "hosted:openrouter",
            Self::OpenAi => "hosted:openai",
            Self::Groq => "hosted:groq",
            Self::Mistral => "hosted:mistral",
            Self::Together => "hosted:together",
            Self::Xai => "hosted:xai",
            Self::Gemini => "hosted:gemini",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::OpenAi => "openai",
            Self::Groq => "groq",
            Self::Mistral => "mistral",
            Self::Together => "together",
            Self::Xai => "xai",
            Self::Gemini => "gemini",
        }
    }

    pub fn base_url(self) -> &'static str {
        match self {
            Self::OpenRouter => "https://openrouter.ai/api/v1",
            Self::OpenAi => "https://api.openai.com/v1",
            Self::Groq => "https://api.groq.com/openai/v1",
            Self::Mistral => "https://api.mistral.ai/v1",
            Self::Together => "https://api.together.xyz/v1",
            Self::Xai => "https://api.x.ai/v1",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta",
        }
    }

    pub fn secret_name(self) -> &'static str {
        match self {
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::OpenAi => "HOSTED_OPENAI_API_KEY",
            Self::Groq => "HOSTED_GROQ_API_KEY",
            Self::Mistral => "HOSTED_MISTRAL_API_KEY",
            Self::Together => "HOSTED_TOGETHER_API_KEY",
            Self::Xai => "HOSTED_XAI_API_KEY",
            Self::Gemini => "HOSTED_GEMINI_API_KEY",
        }
    }

    pub fn auth_header(self, key: &str) -> (&'static str, String) {
        match self {
            Self::Gemini => ("x-goog-api-key", key.to_owned()),
            _ => ("authorization", format!("Bearer {key}")),
        }
    }

    /// The format of the audio this provider returns for `request`.
    pub fn speech_format(self, request: &HostedSpeechRequest) -> &'static str {
        let requested = request.requested_format();
        match self {
            Self::Groq => "wav",
            Self::OpenRouter => {
                if requested == "pcm" {
                    "pcm"
                } else {
                    "mp3"
                }
            }
            Self::Gemini => {
                if requested == "pcm" {
                    "pcm"
                } else {
                    "wav"
                }
            }
            _ => SPEECH_FORMATS
                .into_iter()
                .find(|format| *format == requested)
                .unwrap_or("mp3"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UpstreamBody {
    Json(Value),
    /// The caller attaches the decoded audio as the `file` part.
    Multipart {
        fields: Vec<(String, String)>,
        file_name: String,
        mime: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpstreamRequest {
    pub url: String,
    pub body: UpstreamBody,
}

fn ensure_path_safe_model(model: &str) -> Result<()> {
    ensure!(
        !model.is_empty()
            && !model.contains(['/', ':', '?', '#', '\\', '%'])
            && model != "."
            && model != "..",
        "Hosted Gemini speech requires a plain upstream model ID"
    );
    Ok(())
}

pub fn speech_request(
    provider: HostedAudioProvider,
    model: &str,
    request: &HostedSpeechRequest,
) -> Result<UpstreamRequest> {
    request.validate()?;
    let base = provider.base_url();
    let format = provider.speech_format(request);
    let voice = request.voice.as_deref();
    let openai_like = |default_voice: &str, wire_format: &str| UpstreamRequest {
        url: format!("{base}/audio/speech"),
        body: UpstreamBody::Json(json!({
            "model": model,
            "input": request.text,
            "voice": voice.unwrap_or(default_voice),
            "response_format": wire_format,
        })),
    };
    Ok(match provider {
        HostedAudioProvider::OpenRouter => openai_like("alloy", format),
        HostedAudioProvider::OpenAi => openai_like("alloy", format),
        HostedAudioProvider::Groq => openai_like("troy", format),
        HostedAudioProvider::Together => {
            openai_like("tara", if format == "pcm" { "raw" } else { format })
        }
        HostedAudioProvider::Mistral => {
            let voice = voice.ok_or_else(|| {
                anyhow::anyhow!("Hosted Mistral speech requires a voice ID in the voice field")
            })?;
            UpstreamRequest {
                url: format!("{base}/audio/speech"),
                body: UpstreamBody::Json(json!({
                    "model": model,
                    "input": request.text,
                    "voice_id": voice,
                    "response_format": format,
                })),
            }
        }
        HostedAudioProvider::Xai => UpstreamRequest {
            url: format!("{base}/tts"),
            body: UpstreamBody::Json(json!({
                "text": request.text,
                "voice_id": voice.unwrap_or("eve"),
                "language": "auto",
                "output_format": {"codec": format},
            })),
        },
        HostedAudioProvider::Gemini => {
            ensure_path_safe_model(model)?;
            UpstreamRequest {
                url: format!("{base}/models/{model}:generateContent"),
                body: UpstreamBody::Json(json!({
                    "contents": [{"parts": [{"text": request.text}]}],
                    "generationConfig": {
                        "responseModalities": ["AUDIO"],
                        "speechConfig": {"voiceConfig": {"prebuiltVoiceConfig": {
                            "voiceName": voice.unwrap_or("Kore"),
                        }}},
                    },
                })),
            }
        }
    })
}

pub fn transcription_request(
    provider: HostedAudioProvider,
    model: &str,
    request: &HostedTranscriptionRequest,
) -> Result<UpstreamRequest> {
    request.validate()?;
    let base = provider.base_url();
    let prompt = match request.language.as_deref() {
        Some(language) => format!("{TRANSCRIBE_PROMPT} The spoken language is {language}."),
        None => TRANSCRIBE_PROMPT.to_owned(),
    };
    let multipart = |path: &str, mut fields: Vec<(String, String)>| {
        if let Some(language) = &request.language {
            fields.push(("language".into(), language.clone()));
        }
        UpstreamRequest {
            url: format!("{base}/{path}"),
            body: UpstreamBody::Multipart {
                fields,
                file_name: request.file_name(),
                mime: base_mime(&request.mime),
            },
        }
    };
    Ok(match provider {
        HostedAudioProvider::OpenAi
        | HostedAudioProvider::Groq
        | HostedAudioProvider::Together
        | HostedAudioProvider::Mistral => {
            let response_format = if model.to_ascii_lowercase().contains("whisper") {
                "verbose_json"
            } else {
                "json"
            };
            multipart(
                "audio/transcriptions",
                vec![
                    ("model".into(), model.to_owned()),
                    ("response_format".into(), response_format.into()),
                ],
            )
        }
        HostedAudioProvider::Xai => multipart("stt", vec![("format".into(), "true".into())]),
        HostedAudioProvider::OpenRouter => UpstreamRequest {
            url: format!("{base}/chat/completions"),
            body: UpstreamBody::Json(json!({
                "model": model,
                "stream": false,
                "messages": [{"role": "user", "content": [
                    {"type": "text", "text": prompt},
                    {"type": "input_audio", "input_audio": {
                        "data": request.audio_base64,
                        "format": audio_extension(&request.mime),
                    }},
                ]}],
            })),
        },
        HostedAudioProvider::Gemini => {
            ensure_path_safe_model(model)?;
            UpstreamRequest {
                url: format!("{base}/models/{model}:generateContent"),
                body: UpstreamBody::Json(json!({"contents": [{"parts": [
                    {"text": prompt},
                    {"inlineData": {
                        "mimeType": base_mime(&request.mime),
                        "data": request.audio_base64,
                    }},
                ]}]})),
            }
        }
    })
}

fn wav_from_pcm_s16le(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let bits_per_sample = 16u16;
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

fn sample_rate_from_mime(mime: &str) -> Option<u32> {
    mime.split(';').find_map(|part| {
        part.trim()
            .strip_prefix("rate=")
            .and_then(|rate| rate.parse::<u32>().ok())
    })
}

fn decode_speech_base64(encoded: &str, provider: &str) -> Result<Vec<u8>> {
    ensure!(
        encoded.len() <= MAX_AUDIO_BYTES.div_ceil(3) * 4,
        "{provider} speech exceeds the size limit"
    );
    STANDARD
        .decode(encoded.as_bytes())
        .with_context(|| format!("{provider} speech contained invalid base64"))
}

fn gemini_speech(value: &Value, format: &str) -> Result<HostedSpeech> {
    let inline = value
        .get("candidates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|candidate| candidate.pointer("/content/parts")?.as_array())
        .flatten()
        .find_map(|part| part.get("inlineData").or_else(|| part.get("inline_data")))
        .context("Gemini speech response contained no inline audio")?;
    let data = inline
        .get("data")
        .and_then(Value::as_str)
        .context("Gemini speech inline audio had no data")?;
    let bytes = decode_speech_base64(data, "Gemini")?;
    let mime = inline
        .get("mimeType")
        .or_else(|| inline.get("mime_type"))
        .and_then(Value::as_str)
        .unwrap_or("audio/pcm");
    let lower = mime.to_ascii_lowercase();
    if format != "pcm" && (lower.contains("pcm") || lower.contains("l16")) {
        let rate = sample_rate_from_mime(mime).unwrap_or(24_000);
        return Ok(HostedSpeech {
            bytes: wav_from_pcm_s16le(&bytes, rate, 1),
            mime: "audio/wav".into(),
        });
    }
    Ok(HostedSpeech {
        bytes,
        mime: base_mime(mime),
    })
}

pub fn parse_speech_response(
    provider: HostedAudioProvider,
    content_type: Option<&str>,
    body: &[u8],
    format: &str,
) -> Result<HostedSpeech> {
    ensure!(
        !body.is_empty(),
        "{} returned an empty speech response",
        provider.label()
    );
    let content_type = content_type.map(base_mime).unwrap_or_default();
    let speech = match provider {
        HostedAudioProvider::Mistral => {
            let value: Value =
                serde_json::from_slice(body).context("Mistral speech returned invalid JSON")?;
            let encoded = value
                .get("audio_data")
                .or_else(|| value.get("audio"))
                .and_then(Value::as_str)
                .context("Mistral speech response contained no audio_data")?;
            HostedSpeech {
                bytes: decode_speech_base64(encoded, "Mistral")?,
                mime: format_mime(format).into(),
            }
        }
        HostedAudioProvider::Gemini => {
            let value: Value =
                serde_json::from_slice(body).context("Gemini speech returned invalid JSON")?;
            gemini_speech(&value, format)?
        }
        _ => {
            ensure!(
                !content_type.contains("json"),
                "{} returned JSON instead of audio",
                provider.label()
            );
            HostedSpeech {
                bytes: body.to_vec(),
                mime: if content_type.starts_with("audio/") {
                    content_type
                } else {
                    format_mime(format).into()
                },
            }
        }
    };
    ensure!(
        !speech.bytes.is_empty() && speech.bytes.len() <= MAX_AUDIO_BYTES,
        "{} speech is empty or exceeds the size limit",
        provider.label()
    );
    Ok(speech)
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedTranscription {
    pub transcription: HostedTranscription,
    /// Audio length the provider reports it billed or measured, when it reports one.
    pub billed_seconds: Option<f64>,
    /// Cost in USD the provider reports for this request (OpenRouter only).
    pub provider_cost_usd: Option<f64>,
}

fn non_negative(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
}

fn message_text(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

fn gemini_text(value: &Value) -> Option<String> {
    let parts: Vec<&str> = value
        .get("candidates")?
        .as_array()?
        .iter()
        .filter_map(|candidate| candidate.pointer("/content/parts")?.as_array())
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

pub fn parse_transcription_response(
    provider: HostedAudioProvider,
    body: &[u8],
) -> Result<ParsedTranscription> {
    let value: Value = serde_json::from_slice(body)
        .with_context(|| format!("{} transcription returned invalid JSON", provider.label()))?;
    let text = match provider {
        HostedAudioProvider::OpenRouter => value
            .pointer("/choices/0/message/content")
            .and_then(message_text),
        HostedAudioProvider::Gemini => gemini_text(&value),
        _ => ["text", "transcript", "transcription"]
            .into_iter()
            .find_map(|key| value.get(key).and_then(Value::as_str))
            .map(str::to_owned),
    }
    .map(|text| text.trim().to_owned())
    .filter(|text| !text.is_empty())
    .with_context(|| format!("{} transcription contained no text", provider.label()))?;
    let billed_seconds = non_negative(value.get("duration"))
        .or_else(|| non_negative(value.pointer("/usage/seconds")));
    let language = value
        .get("language")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|language| !language.is_empty() && language.len() <= 32)
        .map(str::to_owned);
    let provider_cost_usd = (provider == HostedAudioProvider::OpenRouter)
        .then(|| non_negative(value.pointer("/usage/cost")))
        .flatten();
    Ok(ParsedTranscription {
        transcription: HostedTranscription {
            text,
            language,
            duration_ms: billed_seconds.map(|seconds| (seconds * 1000.0).round() as u64),
        },
        billed_seconds,
        provider_cost_usd,
    })
}

#[derive(Clone)]
pub struct HostedAudioClient {
    base: reqwest::Url,
    bearer: String,
    usage_headers: Vec<(String, String)>,
    authorizer: Option<ScopedRequestAuthorizer>,
    client: reqwest::Client,
}

fn audio_base(base: &str, exact_resource: bool) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(base.trim()).context("Invalid hosted audio API base")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "Hosted audio API base must be HTTP or HTTPS without credentials, query, or fragment"
    );
    let path = url.path().trim_end_matches('/');
    let path = if exact_resource || path.ends_with("/api/v1") {
        format!("{path}/audio")
    } else {
        format!("{path}/api/v1/audio")
    };
    url.set_path(&path);
    Ok(url)
}

fn gateway_error(body: &[u8]) -> Option<String> {
    let body: Value = serde_json::from_slice(body).ok()?;
    let message = body.pointer("/error/message")?.as_str()?;
    Some(message.chars().take(1024).collect())
}

impl HostedAudioClient {
    /// `base_url` comes from runtime configuration, never from catalog provider parameters.
    pub fn new(base_url: &str, bearer: impl Into<String>) -> Result<Self> {
        Ok(Self {
            base: audio_base(base_url, false)?,
            bearer: bearer.into(),
            usage_headers: Vec::new(),
            authorizer: None,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(180))
                .build()?,
        })
    }

    pub fn with_usage_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.usage_headers = headers;
        self
    }

    pub fn with_authorizer(mut self, authorizer: Arc<dyn RequestAuthorizer>) -> Result<Self> {
        if let Some(base) = authorizer.resource_base_url(ResourceAudience::HostedModels) {
            self.base = audio_base(&base, true)?;
        }
        if authorizer.attribution() == AuthorizationAttribution::InstanceGrant {
            self.usage_headers.clear();
        }
        self.authorizer = Some(ScopedRequestAuthorizer::new(
            authorizer,
            ResourceAudience::HostedModels,
            self.base.as_str(),
        )?);
        self.bearer.clear();
        Ok(self)
    }

    async fn post(
        &self,
        path: &str,
        body: &impl Serialize,
        limit: usize,
    ) -> Result<(Vec<u8>, Option<String>)> {
        let url = format!("{}/{path}", self.base.as_str().trim_end_matches('/'));
        let mut builder = self.client.post(&url).json(body);
        if !self.bearer.is_empty() {
            builder = builder.bearer_auth(&self.bearer);
        }
        for (name, value) in &self.usage_headers {
            builder = builder.header(name, value);
        }
        let mut request = builder.build()?;
        if let Some(authorizer) = &self.authorizer {
            authorizer
                .authorize("POST", &url, request.headers_mut())
                .await?;
        }
        let mut response = self
            .client
            .execute(request)
            .await
            .context("Hosted audio request failed")?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let limit = if status.is_success() { limit } else { 8192 };
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= limit,
                "Hosted audio response exceeds the size limit (HTTP {status})"
            );
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            if let Some(message) = gateway_error(&bytes) {
                bail!("Hosted audio endpoint returned HTTP {status}: {message}");
            }
            bail!("Hosted audio endpoint returned HTTP {status}");
        }
        Ok((bytes, content_type))
    }

    pub async fn transcribe(
        &self,
        request: &HostedTranscriptionRequest,
    ) -> Result<HostedTranscription> {
        request.validate()?;
        let (bytes, _) = self
            .post("transcriptions", request, MAX_TRANSCRIPTION_RESPONSE_BYTES)
            .await?;
        serde_json::from_slice(&bytes).context("Invalid hosted transcription response")
    }

    pub async fn speak(&self, request: &HostedSpeechRequest) -> Result<HostedSpeech> {
        request.validate()?;
        let (bytes, content_type) = self.post("speech", request, MAX_AUDIO_BYTES).await?;
        ensure!(!bytes.is_empty(), "Hosted speech response is empty");
        Ok(HostedSpeech {
            bytes,
            mime: content_type
                .as_deref()
                .map(base_mime)
                .filter(|mime| mime.starts_with("audio/"))
                .unwrap_or_else(|| format_mime(request.requested_format()).into()),
        })
    }
}

#[cfg(test)]
mod tests;
