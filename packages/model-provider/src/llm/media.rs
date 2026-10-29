use base64::{Engine as _, engine::general_purpose::STANDARD};
use rig::completion::{CompletionError, CompletionRequest, Message};
use rig::message::{
    Audio, AudioMediaType, Document, DocumentMediaType, DocumentSourceKind, Image, MimeType,
    UserContent, Video,
};

use super::user_media::{self, Kind as MediaKind, Source as MediaSource};

/// The user-content conversion a Rig 0.38.2 client runs before it sends a request, and the
/// media the provider behind it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDialect {
    /// Chat Completions at OpenAI, Azure OpenAI and Flow-Like's hosted proxy.
    OpenAIChat,
    /// Responses at OpenAI and Azure OpenAI.
    OpenAIResponses,
    /// Chat Completions at any other server, such as vLLM, llama.cpp or TGI: text, images and
    /// WAV/MP3 audio for audio models, no file part.
    OpenAICompatibleChat,
    /// Responses at any other server: text and images, no file part.
    OpenAICompatibleResponses,
    Anthropic,
    OpenRouter,
    /// Chat Completions converter; Groq reads text and images, no file or audio part.
    Groq,
    /// Chat Completions converter; Together reads text and images.
    Together,
    /// Rig's Hugging Face router converter: text and images.
    HuggingFace,
    /// Chat Completions converter; Kimi reads images inline or as `ms://` file ids, never by
    /// http(s) link, and no file or audio part.
    Moonshot,
    /// Chat Completions converter; Hyperbolic serves text models only.
    Hyperbolic,
    /// Responses converter against llamafile's llama.cpp server, which turns `input_image` into a
    /// chat image and rejects `input_file`.
    Llamafile,
}

impl MediaDialect {
    /// The provider as an error names it.
    fn provider(&self) -> &str {
        match self {
            Self::OpenAIChat => "OpenAI Chat Completions",
            Self::OpenAIResponses => "OpenAI Responses",
            Self::OpenAICompatibleChat | Self::OpenAICompatibleResponses => {
                "the OpenAI-compatible server"
            }
            Self::Anthropic => "Anthropic",
            Self::OpenRouter => "OpenRouter",
            Self::Groq => "Groq",
            Self::Together => "Together AI",
            Self::HuggingFace => "Hugging Face",
            Self::Moonshot => "Moonshot AI",
            Self::Hyperbolic => "Hyperbolic",
            Self::Llamafile => "llamafile",
        }
    }

    /// Whether the provider reads images of type `mime`; an image of unknown type is left to it.
    fn reads_image(self, mime: Option<&str>) -> bool {
        const WEB: [&str; 4] = ["image/jpeg", "image/png", "image/gif", "image/webp"];
        match (self, mime) {
            (Self::Hyperbolic, _) => false,
            (Self::OpenRouter, _) | (_, None) => true,
            (Self::Moonshot, Some(mime)) => {
                WEB.contains(&mime) || matches!(mime, "image/heic" | "image/heif")
            }
            // stb_image's formats; llama.cpp decodes WebP only through an external ffmpeg.
            (Self::Llamafile, Some(mime)) => {
                matches!(mime, "image/jpeg" | "image/png" | "image/gif")
            }
            (_, Some(mime)) => WEB.contains(&mime),
        }
    }

    fn fetches_image_links(self) -> bool {
        !matches!(self, Self::Moonshot | Self::Hyperbolic)
    }

    /// Kimi's own `ms://` file links.
    fn reads_own_link(self, link: &str) -> bool {
        self == Self::Moonshot
            && link
                .get(..5)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("ms://"))
    }

    fn reads_file_ids(self) -> bool {
        matches!(
            self,
            Self::OpenAIChat | Self::OpenAIResponses | Self::Anthropic
        )
    }

    fn reads_inline_pdfs(self) -> bool {
        matches!(
            self,
            Self::OpenAIChat | Self::OpenAIResponses | Self::Anthropic | Self::OpenRouter
        )
    }

    /// Whether the converter sends a text document's text as written, rather than as a text part
    /// the fixup writes.
    fn sends_text_document(self, media_type: Option<&DocumentMediaType>) -> bool {
        match self {
            Self::Anthropic => media_type == Some(&DocumentMediaType::TXT),
            Self::OpenRouter => media_type.is_some(),
            _ => media_type.is_some_and(|media_type| *media_type != DocumentMediaType::PDF),
        }
    }

    /// OpenRouter fetches any document from a public https link, OpenAI's Responses API PDFs.
    fn fetches_document_links(self, pdf: bool) -> bool {
        match self {
            Self::OpenRouter => true,
            Self::OpenAIResponses => pdf,
            _ => false,
        }
    }

    fn reads_audio(self, model: &str, media_type: Option<&AudioMediaType>) -> bool {
        match self {
            Self::OpenAIChat | Self::OpenAICompatibleChat => {
                matches!(media_type, Some(AudioMediaType::WAV | AudioMediaType::MP3))
                    && is_audio_model(model)
            }
            Self::OpenRouter => media_type.is_some(),
            _ => false,
        }
    }
}

/// Chat models that take `input_audio`: OpenAI's audio models and the audio families
/// OpenAI-compatible servers such as vLLM serve.
fn is_audio_model(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    ["audio", "ultravox", "voxtral", "omni"]
        .iter()
        .any(|family| model.contains(family))
}

/// What a user part becomes for the provider.
enum Fit {
    Native,
    Text(String),
    /// A note in place of media the provider cannot read at all.
    Unreadable {
        kind: &'static str,
        note: String,
    },
}

fn unreadable(kind: MediaKind, mime: Option<&str>, source: &MediaSource) -> Fit {
    Fit::Unreadable {
        kind: kind.plural(),
        note: user_media::note(kind, mime, source),
    }
}

/// Rig fails the whole request on one attachment its converter rejects, and the provider on one
/// part it cannot read. So media the provider cannot read at all becomes a note naming its type,
/// and an inline text document its text. A link to media the provider reads, though not by that
/// link, fails the call naming the media's kind, type and host: the run inlines such media before
/// the call, so a link left over means that failed. Neither ever names the link.
pub fn retain_supported(
    dialect: MediaDialect,
    model: &str,
    mut request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    let mut omitted = Vec::new();
    let mut as_text = 0usize;
    for message in request.chat_history.iter_mut() {
        let Message::User { content } = message else {
            continue;
        };
        for part in content.iter_mut() {
            match fit(dialect, model, part)? {
                Fit::Native => {}
                Fit::Text(text) => {
                    *part = UserContent::text(text);
                    as_text += 1;
                }
                Fit::Unreadable { kind, note } => {
                    *part = UserContent::text(note);
                    omitted.push(kind);
                }
            }
        }
    }
    report_replaced(dialect, model, omitted, as_text);
    Ok(request)
}

fn report_replaced(dialect: MediaDialect, model: &str, mut omitted: Vec<&str>, as_text: usize) {
    if omitted.is_empty() && as_text == 0 {
        return;
    }
    let count = omitted.len();
    omitted.sort_unstable();
    omitted.dedup();
    tracing::warn!(
        model,
        ?dialect,
        omitted = count,
        as_text,
        kinds = ?omitted,
        "Replaced user attachments the provider cannot read with text"
    );
}

fn fit(dialect: MediaDialect, model: &str, part: &mut UserContent) -> Result<Fit, CompletionError> {
    send_bytes_as_base64(part);
    match part {
        UserContent::Text(_) | UserContent::ToolResult(_) => Ok(Fit::Native),
        UserContent::Image(image) => image_fit(dialect, image),
        UserContent::Audio(audio) => audio_fit(dialect, model, audio),
        UserContent::Video(video) => video_fit(dialect, video),
        UserContent::Document(document) => document_fit(dialect, document),
    }
}

/// Rig's converters take media bytes as base64.
fn send_bytes_as_base64(part: &mut UserContent) {
    let data = match part {
        UserContent::Image(image) => &mut image.data,
        UserContent::Audio(audio) => &mut audio.data,
        UserContent::Video(video) => &mut video.data,
        UserContent::Document(document) => &mut document.data,
        UserContent::Text(_) | UserContent::ToolResult(_) => return,
    };
    if let DocumentSourceKind::Raw(bytes) = data {
        *data = DocumentSourceKind::Base64(STANDARD.encode(bytes));
    }
}

fn image_fit(dialect: MediaDialect, image: &mut Image) -> Result<Fit, CompletionError> {
    user_media::type_image_bytes(image);
    let fit = readable_image(dialect, image)?;
    if dialect == MediaDialect::HuggingFace && matches!(fit, Fit::Native) {
        send_as_data_url(image);
    }
    Ok(fit)
}

/// Rig's converters refuse image bytes of a type they cannot name.
fn readable_image(dialect: MediaDialect, image: &Image) -> Result<Fit, CompletionError> {
    let source = MediaSource::of(&image.data);
    let mime = user_media::image_mime(image, &source);
    let untyped_bytes =
        image.media_type.is_none() && matches!(image.data, DocumentSourceKind::Base64(_));
    match &source {
        _ if untyped_bytes || !dialect.reads_image(mime) => {
            Ok(unreadable(MediaKind::Image, mime, &source))
        }
        MediaSource::Inline { .. } => Ok(Fit::Native),
        MediaSource::Link(link) => image_link_fit(dialect, mime, link, true),
        MediaSource::LocalLink(link) => image_link_fit(dialect, mime, link, false),
        MediaSource::Text(_) | MediaSource::FileId | MediaSource::Other => {
            Ok(unreadable(MediaKind::Image, mime, &source))
        }
    }
}

/// A link to an image of a type the provider reads.
fn image_link_fit(
    dialect: MediaDialect,
    mime: Option<&str>,
    link: &str,
    public: bool,
) -> Result<Fit, CompletionError> {
    if (public && dialect.fetches_image_links()) || dialect.reads_own_link(link) {
        return Ok(Fit::Native);
    }
    let refused = if dialect.fetches_image_links() {
        user_media::unreachable
    } else {
        user_media::inline_only
    };
    Err(refused(dialect.provider(), MediaKind::Image, mime, link))
}

/// Rig 0.38.2's Hugging Face converter writes `data:image/image/png;base64,…` for a base64
/// image, while it passes a link through unchanged.
fn send_as_data_url(image: &mut Image) {
    if let DocumentSourceKind::Base64(data) = &image.data
        && let Some(media_type) = &image.media_type
    {
        image.data =
            DocumentSourceKind::Url(format!("data:{};base64,{data}", media_type.to_mime_type()));
    }
}

/// Every reader takes audio inline only.
fn audio_fit(dialect: MediaDialect, model: &str, audio: &Audio) -> Result<Fit, CompletionError> {
    let source = MediaSource::of(&audio.data);
    let mime = user_media::mime_of(audio.media_type.as_ref());
    if dialect.reads_audio(model, audio.media_type.as_ref()) {
        match (&audio.data, &source) {
            (DocumentSourceKind::Base64(_), _) => return Ok(Fit::Native),
            (_, MediaSource::Link(link) | MediaSource::LocalLink(link)) => {
                return Err(user_media::inline_only(
                    dialect.provider(),
                    MediaKind::Audio,
                    mime,
                    link,
                ));
            }
            _ => {}
        }
    }
    Ok(unreadable(MediaKind::Audio, mime, &source))
}

fn video_fit(dialect: MediaDialect, video: &Video) -> Result<Fit, CompletionError> {
    let source = MediaSource::of(&video.data);
    let mime = user_media::mime_of(video.media_type.as_ref());
    if dialect == MediaDialect::OpenRouter {
        match (&video.data, &source) {
            (_, MediaSource::LocalLink(link)) => {
                return Err(user_media::unreachable(
                    dialect.provider(),
                    MediaKind::Video,
                    mime,
                    link,
                ));
            }
            (DocumentSourceKind::Url(_), _) => return Ok(Fit::Native),
            (DocumentSourceKind::Base64(_), _) if video.media_type.is_some() => {
                return Ok(Fit::Native);
            }
            _ => {}
        }
    }
    Ok(unreadable(MediaKind::Video, mime, &source))
}

/// Every provider reads a typed text document as its text, so its link is never sent.
fn document_fit(dialect: MediaDialect, document: &mut Document) -> Result<Fit, CompletionError> {
    type_pdf_bytes(document);
    let source = MediaSource::of(&document.data);
    match &source {
        MediaSource::Link(link) => return document_link_fit(dialect, document, link, true),
        MediaSource::LocalLink(link) => return document_link_fit(dialect, document, link, false),
        _ => {}
    }
    if sends_document_as_is(dialect, document) {
        return Ok(Fit::Native);
    }
    let mime = user_media::mime_of(document.media_type.as_ref());
    Ok(match inline_text(document) {
        Some(text) => Fit::Text(format!(
            "[document: {}]\n{text}",
            mime.unwrap_or("unknown type")
        )),
        None => unreadable(
            MediaKind::Document,
            mime.or_else(|| sniffed_document(&document.data)),
            &source,
        ),
    })
}

/// Untyped bytes that are a PDF are typed as one.
fn type_pdf_bytes(document: &mut Document) {
    if document.media_type.is_none()
        && let DocumentSourceKind::Base64(data) = &document.data
        && user_media::leading_bytes(data, 5).starts_with(b"%PDF-")
    {
        document.media_type = Some(DocumentMediaType::PDF);
    }
}

/// Rig's converters paste a non-PDF base64 document into the prompt verbatim, and OpenRouter's
/// sends it as a file, so base64 stays only for PDFs, and only where the provider reads them.
fn sends_document_as_is(dialect: MediaDialect, document: &Document) -> bool {
    let pdf = document.media_type == Some(DocumentMediaType::PDF);
    match &document.data {
        DocumentSourceKind::FileId(_) => dialect.reads_file_ids(),
        DocumentSourceKind::Url(_) => dialect.fetches_document_links(pdf),
        DocumentSourceKind::Base64(_) => pdf && dialect.reads_inline_pdfs(),
        DocumentSourceKind::String(_) => dialect.sends_text_document(document.media_type.as_ref()),
        _ => false,
    }
}

/// A typed text document is read inline everywhere, as is a PDF where the provider reads PDFs.
fn document_link_fit(
    dialect: MediaDialect,
    document: &Document,
    link: &str,
    public: bool,
) -> Result<Fit, CompletionError> {
    let pdf = document.media_type == Some(DocumentMediaType::PDF);
    let mime = user_media::mime_of(document.media_type.as_ref());
    let read_inline = if pdf {
        dialect.reads_inline_pdfs()
    } else {
        document.media_type.is_some()
    };
    if dialect.fetches_document_links(pdf) && public {
        Ok(Fit::Native)
    } else if dialect.fetches_document_links(pdf) {
        Err(user_media::unreachable(
            dialect.provider(),
            MediaKind::Document,
            mime,
            link,
        ))
    } else if read_inline {
        Err(user_media::inline_only(
            dialect.provider(),
            MediaKind::Document,
            mime,
            link,
        ))
    } else {
        Ok(unreadable(
            MediaKind::Document,
            mime,
            &MediaSource::of(&document.data),
        ))
    }
}

/// PDF bytes are PDF syntax, never the document's text, even when they are valid UTF-8. Typed
/// documents other than PDFs are text, so invalid UTF-8 there is a legacy encoding and is decoded
/// lossily; untyped bytes must be valid UTF-8 to count as text.
fn inline_text(document: &Document) -> Option<String> {
    if let DocumentSourceKind::String(text) = &document.data {
        return Some(text.clone());
    }
    if document.media_type == Some(DocumentMediaType::PDF) {
        return None;
    }
    let decoded;
    let bytes = match &document.data {
        DocumentSourceKind::Base64(data) => {
            decoded = STANDARD.decode(data).ok()?;
            &decoded
        }
        DocumentSourceKind::Raw(bytes) => bytes,
        _ => return None,
    };
    match document.media_type {
        None => std::str::from_utf8(bytes).ok().map(str::to_owned),
        Some(_) => Some(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// The type of untyped document bytes, for the note in their place.
fn sniffed_document(data: &DocumentSourceKind) -> Option<&str> {
    match data {
        DocumentSourceKind::Base64(data) => STANDARD
            .decode(data)
            .ok()
            .and_then(|bytes| user_media::sniff_document(&bytes)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::OneOrMany;
    use rig::message::{ImageMediaType, VideoMediaType};
    use serde_json::Value;

    const PRESIGNED: &str = "https://flow-like-content.s3.eu-central-1.amazonaws.com/tmp/user/u/apps/a/runs/r/request/teams/files/0/report.pdf?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=ASIAEXAMPLE%2F20260929%2Feu-central-1%2Fs3%2Faws4_request&X-Amz-Security-Token=secret-token&X-Amz-Signature=deadbeef";
    const HOST: &str = "flow-like-content.s3.eu-central-1.amazonaws.com";
    /// `%PDF-1.4`: valid UTF-8, yet PDF syntax rather than the document's text.
    const PDF_BASE64: &str = "JVBERi0xLjQK";
    const TXT_BASE64: &str = "SGVsbG8K";
    const CSV_BASE64: &str = "YSxiCjEsMgo=";
    /// `Straße;München` saved as Windows-1252, which is not valid UTF-8.
    const WINDOWS_1252_CSV_BASE64: &str = "U3RyYd9lO038bmNoZW4K";
    /// `{"a":1}`; Rig has no JSON document type, so JSON arrives untyped.
    const JSON_BASE64: &str = "eyJhIjoxfQo=";
    const DOCX_BASE64: &str = "UEsDBBQABgAIAAAAIQD/";

    const DIALECTS: [MediaDialect; 12] = [
        MediaDialect::OpenAIChat,
        MediaDialect::OpenAIResponses,
        MediaDialect::OpenAICompatibleChat,
        MediaDialect::OpenAICompatibleResponses,
        MediaDialect::Anthropic,
        MediaDialect::OpenRouter,
        MediaDialect::Groq,
        MediaDialect::Together,
        MediaDialect::HuggingFace,
        MediaDialect::Moonshot,
        MediaDialect::Hyperbolic,
        MediaDialect::Llamafile,
    ];

    #[derive(Debug, Clone, Copy)]
    enum Kind {
        Image,
        Webp,
        Heic,
        Svg,
        UntypedBmp,
        Pdf,
        Txt,
        Csv,
        Windows1252Csv,
        Json,
        Untyped,
        Audio,
        Video,
    }

    #[derive(Debug, Clone, Copy)]
    enum Source {
        Url,
        Base64,
        String,
        FileId,
    }

    #[derive(Debug, Clone, Copy)]
    enum Expect {
        Keep,
        DataUrl,
        Omit,
        AsText,
        /// A link to media the provider reads only some other way.
        Fail,
    }
    use Expect::{AsText, DataUrl, Fail, Keep, Omit};

    const KINDS: [Kind; 13] = [
        Kind::Image,
        Kind::Webp,
        Kind::Heic,
        Kind::Svg,
        Kind::UntypedBmp,
        Kind::Pdf,
        Kind::Txt,
        Kind::Csv,
        Kind::Windows1252Csv,
        Kind::Json,
        Kind::Untyped,
        Kind::Audio,
        Kind::Video,
    ];
    const SOURCES: [Source; 4] = [Source::Url, Source::Base64, Source::String, Source::FileId];

    fn source(kind: Kind, source: Source) -> DocumentSourceKind {
        let inline = match kind {
            Kind::Pdf => PDF_BASE64,
            Kind::Txt => TXT_BASE64,
            Kind::Csv => CSV_BASE64,
            Kind::Windows1252Csv => WINDOWS_1252_CSV_BASE64,
            Kind::Json => JSON_BASE64,
            Kind::Untyped => DOCX_BASE64,
            Kind::Image
            | Kind::Webp
            | Kind::Heic
            | Kind::Svg
            | Kind::UntypedBmp
            | Kind::Audio
            | Kind::Video => "AAAA",
        };
        match source {
            Source::Url => DocumentSourceKind::Url(match kind {
                Kind::UntypedBmp => PRESIGNED.replace("report.pdf", "scan.bmp"),
                _ => PRESIGNED.to_string(),
            }),
            Source::Base64 => DocumentSourceKind::Base64(inline.to_string()),
            Source::String => DocumentSourceKind::String("a,b\n1,2\n".to_string()),
            Source::FileId => DocumentSourceKind::FileId("file-abc".to_string()),
        }
    }

    fn attachment(kind: Kind, from: Source) -> UserContent {
        let data = source(kind, from);
        let document = |media_type| {
            UserContent::Document(Document {
                data: data.clone(),
                media_type,
                additional_params: None,
            })
        };
        let image = |media_type| {
            UserContent::Image(Image {
                data: data.clone(),
                media_type,
                detail: None,
                additional_params: None,
            })
        };
        match kind {
            Kind::Image => image(Some(ImageMediaType::PNG)),
            Kind::Webp => image(Some(ImageMediaType::WEBP)),
            Kind::Heic => image(Some(ImageMediaType::HEIC)),
            Kind::Svg => image(Some(ImageMediaType::SVG)),
            Kind::UntypedBmp => image(None),
            Kind::Pdf => document(Some(DocumentMediaType::PDF)),
            Kind::Txt => document(Some(DocumentMediaType::TXT)),
            Kind::Csv | Kind::Windows1252Csv => document(Some(DocumentMediaType::CSV)),
            Kind::Json | Kind::Untyped => document(None),
            Kind::Audio => UserContent::Audio(Audio {
                data,
                media_type: Some(AudioMediaType::MP3),
                additional_params: None,
            }),
            Kind::Video => UserContent::Video(Video {
                data,
                media_type: Some(VideoMediaType::MP4),
                additional_params: None,
            }),
        }
    }

    /// How an error names the media of `kind`.
    fn described(kind: Kind) -> &'static str {
        match kind {
            Kind::Image => "image (image/png)",
            Kind::Webp => "image (image/webp)",
            Kind::Heic => "image (image/heic)",
            Kind::Svg => "image (image/svg+xml)",
            Kind::UntypedBmp => "image (image/bmp)",
            Kind::Pdf => "document (application/pdf)",
            Kind::Txt => "document (text/plain)",
            Kind::Csv | Kind::Windows1252Csv => "document (text/csv)",
            Kind::Json | Kind::Untyped => "document",
            Kind::Audio => "audio (audio/mp3)",
            Kind::Video => "video (video/mp4)",
        }
    }

    /// The error names the media and the host of its link, never the link.
    fn assert_link_error(error: &str, media: &str, host: &str, case: &str) {
        assert!(
            error.contains(&format!("The {media} from host '{host}' cannot be sent to")),
            "{case}: {error}"
        );
        for secret in ["X-Amz", "report.pdf", "scan.png", "/tmp/"] {
            assert!(!error.contains(secret), "{case} leaked the link: {error}");
        }
    }

    fn request(content: Vec<UserContent>) -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: Some("Answer from the attachments.".to_string()),
            chat_history: OneOrMany::one(Message::User {
                content: OneOrMany::many(content).unwrap(),
            }),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: Some(1024),
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    fn user_parts(request: &CompletionRequest) -> Vec<UserContent> {
        request
            .chat_history
            .iter()
            .flat_map(|message| match message {
                Message::User { content } => content.iter().cloned().collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    fn text_of(part: &UserContent) -> Option<&str> {
        match part {
            UserContent::Text(text) => Some(&text.text),
            _ => None,
        }
    }

    fn note(media_type: &str) -> String {
        format!("[attachment omitted: unsupported file type {media_type}]")
    }

    fn serialized<T: serde::Serialize, E: std::fmt::Display>(
        converted: Result<T, E>,
    ) -> Result<Value, String> {
        let converted = converted.map_err(|error| error.to_string())?;
        serde_json::to_value(converted).map_err(|error| error.to_string())
    }

    fn each_message<M: serde::Serialize, E: std::fmt::Display>(
        request: CompletionRequest,
        convert: impl Fn(Message) -> Result<M, E>,
    ) -> Result<Value, String> {
        request
            .chat_history
            .into_iter()
            .map(|message| serialized(convert(message)))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array)
    }

    /// The body, or the messages, the dialect's Rig client builds from the request.
    fn rig_sends(dialect: MediaDialect, request: CompletionRequest) -> Result<Value, String> {
        use rig::providers::{anthropic, huggingface, openai, openrouter};
        let model = "test-model".to_string();
        match dialect {
            MediaDialect::OpenAIChat | MediaDialect::OpenAICompatibleChat => serialized(
                openai::completion::CompletionRequest::try_from((model, request)),
            ),
            MediaDialect::OpenAIResponses
            | MediaDialect::OpenAICompatibleResponses
            | MediaDialect::Llamafile => serialized(
                openai::responses_api::CompletionRequest::try_from((model, request)),
            ),
            MediaDialect::Anthropic => {
                each_message(request, anthropic::completion::Message::try_from)
            }
            MediaDialect::OpenRouter => {
                each_message(request, Vec::<openrouter::completion::Message>::try_from)
            }
            // Their request types are private; each converts its messages with this.
            MediaDialect::Groq
            | MediaDialect::Together
            | MediaDialect::Moonshot
            | MediaDialect::Hyperbolic => {
                each_message(request, Vec::<openai::completion::Message>::try_from)
            }
            MediaDialect::HuggingFace => {
                each_message(request, Vec::<huggingface::completion::Message>::try_from)
            }
        }
    }

    /// Every typed part of every `content` array.
    fn content_parts(value: &Value) -> Vec<&Value> {
        fn collect<'a>(value: &'a Value, parts: &mut Vec<&'a Value>) {
            match value {
                Value::Array(items) => items.iter().for_each(|item| collect(item, parts)),
                Value::Object(object) => {
                    if let Some(Value::Array(content)) = object.get("content") {
                        parts.extend(content.iter().filter(|part| part.get("type").is_some()));
                    }
                    object.values().for_each(|child| collect(child, parts));
                }
                _ => {}
            }
        }
        let mut parts = Vec::new();
        collect(value, &mut parts);
        parts
    }

    /// Image types each provider documents. Groq, Together, Hugging Face and other
    /// OpenAI-compatible servers name none and get OpenAI's; llama.cpp decodes stb_image's.
    fn documented_image_types(dialect: MediaDialect) -> &'static [&'static str] {
        match dialect {
            MediaDialect::Moonshot => &[
                "image/jpeg",
                "image/png",
                "image/gif",
                "image/webp",
                "image/bmp",
                "image/heic",
                "image/heif",
            ],
            MediaDialect::Llamafile => &["image/jpeg", "image/png", "image/gif", "image/bmp"],
            MediaDialect::Hyperbolic => &[],
            _ => &["image/jpeg", "image/png", "image/gif", "image/webp"],
        }
    }

    /// Whether the provider behind the dialect takes a content part Rig put on the wire.
    fn provider_takes(dialect: MediaDialect, part: &Value) -> bool {
        use MediaDialect::*;
        let image = |url: Option<&str>| {
            url.is_some_and(|url| match url.strip_prefix("data:") {
                Some(data) => data.split_once(";base64,").is_some_and(|(media_type, _)| {
                    documented_image_types(dialect).contains(&media_type)
                }),
                None => dialect != Moonshot,
            })
        };
        match (dialect, part["type"].as_str().unwrap_or_default()) {
            (OpenAIChat | OpenAIResponses | Anthropic | OpenRouter, _) => true,
            (
                Groq | Together | HuggingFace | Moonshot | Hyperbolic | OpenAICompatibleChat,
                "text",
            )
            | (Llamafile | OpenAICompatibleResponses, "input_text") => true,
            (Groq | Together | HuggingFace | Moonshot | OpenAICompatibleChat, "image_url") => {
                image(part["image_url"]["url"].as_str())
            }
            (Llamafile | OpenAICompatibleResponses, "input_image") => {
                image(part["image_url"].as_str())
            }
            _ => false,
        }
    }

    /// The parts Rig sends, or why Rig or the provider would fail the call.
    fn reaches_provider(dialect: MediaDialect, request: CompletionRequest) -> Result<(), String> {
        let sent =
            rig_sends(dialect, request).map_err(|error| format!("Rig rejects it: {error}"))?;
        match content_parts(&sent)
            .into_iter()
            .find(|part| !provider_takes(dialect, part))
        {
            Some(part) => Err(format!("the provider rejects {part}")),
            None => Ok(()),
        }
    }

    fn assert_table(dialect: MediaDialect, model: &str, table: [[Expect; 4]; KINDS.len()]) {
        for (kind, row) in KINDS.into_iter().zip(table) {
            for (from, expected) in SOURCES.into_iter().zip(row) {
                let original = attachment(kind, from);
                let case = format!("{dialect:?} {kind:?} via {from:?}");
                let filtered = match (
                    expected,
                    retain_supported(
                        dialect,
                        model,
                        request(vec![UserContent::text("Summarise this."), original.clone()]),
                    ),
                ) {
                    (Fail, Err(error)) => {
                        assert_link_error(&error.to_string(), described(kind), HOST, &case);
                        continue;
                    }
                    (Fail, Ok(_)) => panic!("{case} must fail the call"),
                    (_, Err(error)) => panic!("{case} failed the call: {error}"),
                    (_, Ok(filtered)) => filtered,
                };
                let parts = user_parts(&filtered);
                assert_eq!(parts.len(), 2, "{case}: the user message lost parts");
                let part = &parts[1];
                match expected {
                    Keep => assert_eq!(part, &original, "{case} must be kept"),
                    DataUrl => {
                        let UserContent::Image(image) = part else {
                            panic!("{case} must stay an image: {part:?}");
                        };
                        let media_type = image.media_type.as_ref().expect("a typed image");
                        assert_eq!(
                            image.data,
                            DocumentSourceKind::Url(format!(
                                "data:{};base64,AAAA",
                                media_type.to_mime_type()
                            )),
                            "{case} must travel as a data URL"
                        );
                    }
                    Omit => {
                        let note =
                            text_of(part).unwrap_or_else(|| panic!("{case} must be replaced"));
                        assert!(
                            note.starts_with("[attachment omitted: unsupported file type "),
                            "{case}: {note}"
                        );
                        assert!(!note.contains("X-Amz"), "{case} leaked the URL: {note}");
                    }
                    AsText => {
                        let text =
                            text_of(part).unwrap_or_else(|| panic!("{case} must become text"));
                        assert!(text.starts_with("[document: "), "{case}: {text}");
                    }
                    Fail => unreachable!(),
                }
                reaches_provider(dialect, filtered)
                    .unwrap_or_else(|error| panic!("{case}: {error}"));
            }
        }
    }

    #[test]
    fn openai_chat_completions() {
        assert_table(
            MediaDialect::OpenAIChat,
            "gpt-4o",
            [
                [Keep, Keep, Omit, Omit],
                [Keep, Keep, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Fail, Keep, AsText, Keep],
                [Fail, AsText, Keep, Keep],
                [Fail, AsText, Keep, Keep],
                [Fail, AsText, Keep, Keep],
                [Omit, AsText, AsText, Keep],
                [Omit, Omit, AsText, Keep],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
            ],
        );
    }

    #[test]
    fn openai_chat_completions_keeps_base64_audio_for_audio_models() {
        for (model, kept) in [("gpt-4o-audio-preview", true), ("gpt-4o", false)] {
            let original = attachment(Kind::Audio, Source::Base64);
            let filtered = retain_supported(
                MediaDialect::OpenAIChat,
                model,
                request(vec![original.clone()]),
            )
            .unwrap();
            assert_eq!(user_parts(&filtered)[0] == original, kept, "{model}");
        }
        for media_type in [Some(AudioMediaType::OGG), Some(AudioMediaType::FLAC), None] {
            let original = UserContent::Audio(Audio {
                data: DocumentSourceKind::Base64("AAAA".into()),
                media_type: media_type.clone(),
                additional_params: None,
            });
            let filtered = retain_supported(
                MediaDialect::OpenAIChat,
                "gpt-4o-audio-preview",
                request(vec![original.clone()]),
            )
            .unwrap();
            assert_ne!(user_parts(&filtered)[0], original, "{media_type:?}");
        }

        let linked = retain_supported(
            MediaDialect::OpenAIChat,
            "gpt-4o-audio-preview",
            request(vec![attachment(Kind::Audio, Source::Url)]),
        )
        .expect_err("audio models take audio inline only");
        assert_link_error(
            &linked.to_string(),
            "audio (audio/mp3)",
            HOST,
            "linked audio",
        );
    }

    #[test]
    fn compatible_servers_keep_base64_audio_for_audio_model_families() {
        for (model, kept) in [
            ("Qwen/Qwen2-Audio-7B-Instruct", true),
            ("fixie-ai/ultravox-v0_5-llama-3_1-8b", true),
            ("mistralai/Voxtral-Mini-3B-2507", true),
            ("Qwen/Qwen2.5-Omni-7B", true),
            ("meta-llama/Llama-3.1-8B-Instruct", false),
        ] {
            let original = attachment(Kind::Audio, Source::Base64);
            let filtered = retain_supported(
                MediaDialect::OpenAICompatibleChat,
                model,
                request(vec![original.clone()]),
            )
            .unwrap();
            assert_eq!(user_parts(&filtered)[0] == original, kept, "{model}");
        }
    }

    #[test]
    fn openai_responses() {
        assert_table(
            MediaDialect::OpenAIResponses,
            "gpt-5.4",
            [
                [Keep, Keep, Omit, Omit],
                [Keep, Keep, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Keep, Keep, AsText, Keep],
                [Fail, AsText, Keep, Keep],
                [Fail, AsText, Keep, Keep],
                [Fail, AsText, Keep, Keep],
                [Omit, AsText, AsText, Keep],
                [Omit, Omit, AsText, Keep],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
            ],
        );
    }

    #[test]
    fn anthropic_messages() {
        assert_table(
            MediaDialect::Anthropic,
            "claude-sonnet-4-6",
            [
                [Keep, Keep, Omit, Omit],
                [Keep, Keep, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Fail, Keep, AsText, Keep],
                [Fail, AsText, Keep, Keep],
                [Fail, AsText, AsText, Keep],
                [Fail, AsText, AsText, Keep],
                [Omit, AsText, AsText, Keep],
                [Omit, Omit, AsText, Keep],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
            ],
        );
    }

    #[test]
    fn openrouter() {
        assert_table(
            MediaDialect::OpenRouter,
            "anthropic/claude-sonnet-4.5",
            [
                [Keep, Keep, Omit, Omit],
                [Keep, Keep, Omit, Omit],
                [Keep, Keep, Omit, Omit],
                [Keep, Keep, Omit, Omit],
                [Keep, Omit, Omit, Omit],
                [Keep, Keep, Keep, Omit],
                [Keep, AsText, Keep, Omit],
                [Keep, AsText, Keep, Omit],
                [Keep, AsText, Keep, Omit],
                [Keep, AsText, AsText, Omit],
                [Keep, Omit, AsText, Omit],
                [Fail, Keep, Omit, Omit],
                [Keep, Keep, Omit, Omit],
            ],
        );
    }

    /// Text and images: no file part, no audio. A text document's link fails, since its text is
    /// read inline.
    const IMAGES_ONLY: [[Expect; 4]; KINDS.len()] = [
        [Keep, Keep, Omit, Omit],
        [Keep, Keep, Omit, Omit],
        [Omit, Omit, Omit, Omit],
        [Omit, Omit, Omit, Omit],
        [Omit, Omit, Omit, Omit],
        [Omit, Omit, AsText, Omit],
        [Fail, AsText, Keep, Omit],
        [Fail, AsText, Keep, Omit],
        [Fail, AsText, Keep, Omit],
        [Omit, AsText, AsText, Omit],
        [Omit, Omit, AsText, Omit],
        [Omit, Omit, Omit, Omit],
        [Omit, Omit, Omit, Omit],
    ];

    #[test]
    fn openai_compatible_servers_get_no_file_part() {
        for (dialect, model) in [
            (
                MediaDialect::OpenAICompatibleChat,
                "Qwen/Qwen2.5-VL-7B-Instruct",
            ),
            (MediaDialect::OpenAICompatibleResponses, "gpt-oss-20b"),
        ] {
            assert_table(dialect, model, IMAGES_ONLY);
        }
    }

    #[test]
    fn groq() {
        assert_table(
            MediaDialect::Groq,
            "meta-llama/llama-4-scout-17b-16e-instruct",
            IMAGES_ONLY,
        );
    }

    #[test]
    fn together() {
        assert_table(
            MediaDialect::Together,
            "Qwen/Qwen2.5-VL-72B-Instruct",
            IMAGES_ONLY,
        );
    }

    #[test]
    fn huggingface() {
        assert_table(
            MediaDialect::HuggingFace,
            "Qwen/Qwen2.5-VL-7B-Instruct",
            [
                [Keep, DataUrl, Omit, Omit],
                [Keep, DataUrl, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, AsText, Omit],
                [Fail, AsText, Keep, Omit],
                [Fail, AsText, Keep, Omit],
                [Fail, AsText, Keep, Omit],
                [Omit, AsText, AsText, Omit],
                [Omit, Omit, AsText, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
            ],
        );
    }

    #[test]
    fn moonshot() {
        assert_table(
            MediaDialect::Moonshot,
            "kimi-k2.6",
            [
                [Fail, Keep, Omit, Omit],
                [Fail, Keep, Omit, Omit],
                [Fail, Keep, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, AsText, Omit],
                [Fail, AsText, Keep, Omit],
                [Fail, AsText, Keep, Omit],
                [Fail, AsText, Keep, Omit],
                [Omit, AsText, AsText, Omit],
                [Omit, Omit, AsText, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
            ],
        );
    }

    #[test]
    fn hyperbolic() {
        assert_table(
            MediaDialect::Hyperbolic,
            "deepseek-ai/DeepSeek-V3",
            [
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, AsText, Omit],
                [Fail, AsText, Keep, Omit],
                [Fail, AsText, Keep, Omit],
                [Fail, AsText, Keep, Omit],
                [Omit, AsText, AsText, Omit],
                [Omit, Omit, AsText, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
            ],
        );
    }

    #[test]
    fn llamafile() {
        assert_table(
            MediaDialect::Llamafile,
            "LLaMA_CPP",
            [
                [Keep, Keep, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, AsText, Omit],
                [Fail, AsText, Keep, Omit],
                [Fail, AsText, Keep, Omit],
                [Fail, AsText, Keep, Omit],
                [Omit, AsText, AsText, Omit],
                [Omit, Omit, AsText, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
            ],
        );
    }

    #[test]
    fn groq_gets_no_file_part_where_openai_chat_keeps_one() {
        for from in [Source::Base64, Source::FileId] {
            let pdf = request(vec![attachment(Kind::Pdf, from)]);
            let kept = retain_supported(MediaDialect::OpenAIChat, "gpt-4o", pdf.clone()).unwrap();
            assert_eq!(
                user_parts(&kept)[0],
                attachment(Kind::Pdf, from),
                "{from:?}"
            );
            assert!(
                reaches_provider(MediaDialect::Groq, pdf.clone()).is_err(),
                "Rig sends Groq a file part for a {from:?} PDF"
            );

            let filtered =
                retain_supported(MediaDialect::Groq, "llama-3.3-70b-versatile", pdf).unwrap();
            assert_eq!(
                text_of(&user_parts(&filtered)[0]),
                Some(note("application/pdf").as_str()),
                "{from:?}"
            );
            reaches_provider(MediaDialect::Groq, filtered).unwrap();
        }
    }

    #[test]
    fn hugging_face_base64_images_travel_as_data_urls() {
        let image = request(vec![attachment(Kind::Image, Source::Base64)]);
        let unfiltered = rig_sends(MediaDialect::HuggingFace, image.clone()).unwrap();
        assert!(
            unfiltered
                .to_string()
                .contains("data:image/image/png;base64,AAAA"),
            "Rig's Hugging Face converter no longer mangles base64 images: {unfiltered}"
        );

        let filtered = rig_sends(
            MediaDialect::HuggingFace,
            retain_supported(MediaDialect::HuggingFace, "model", image).unwrap(),
        )
        .unwrap();
        assert_eq!(
            content_parts(&filtered),
            [&serde_json::json!({
                "type": "image_url",
                "image_url": {"url": "data:image/png;base64,AAAA"},
            })]
        );
    }

    #[test]
    fn moonshot_takes_images_inline_or_by_file_id_never_by_link() {
        let png = |data| {
            UserContent::Image(Image {
                data,
                media_type: Some(ImageMediaType::PNG),
                detail: None,
                additional_params: None,
            })
        };
        for data in [
            DocumentSourceKind::Url("ms://file-abc".into()),
            DocumentSourceKind::Url("data:image/png;base64,AAAA".into()),
            DocumentSourceKind::Base64("AAAA".into()),
        ] {
            let original = png(data);
            let filtered = retain_supported(
                MediaDialect::Moonshot,
                "kimi-k2.6",
                request(vec![original.clone()]),
            )
            .unwrap();
            assert_eq!(user_parts(&filtered)[0], original);
        }
        for (link, host) in [
            (PRESIGNED.to_string(), HOST),
            (PRESIGNED.replacen("https", "HTTP", 1), HOST),
            (
                "asset://localhost/%2FUsers%2Fme%2Fscan.png".to_string(),
                "localhost",
            ),
        ] {
            let error = retain_supported(
                MediaDialect::Moonshot,
                "kimi-k2.6",
                request(vec![png(DocumentSourceKind::Url(link.clone()))]),
            )
            .expect_err("Kimi never fetches a link")
            .to_string();
            assert_link_error(&error, "image (image/png)", host, &link);
            assert!(
                error.contains("cannot be sent to Moonshot AI as a link;"),
                "{error}"
            );
        }
    }

    #[test]
    fn links_the_provider_cannot_fetch_fail_naming_kind_type_and_host() {
        for (link, host) in [
            ("asset://localhost/%2FUsers%2Fme%2Fscan.png", "localhost"),
            (
                "http://storage.example.com/tmp/scan.png?X-Amz-Signature=1",
                "storage.example.com",
            ),
            ("https://10.0.0.7/tmp/scan.png", "10.0.0.7"),
            ("https://localhost/tmp/scan.png", "localhost"),
        ] {
            let png = UserContent::image_url(link, Some(ImageMediaType::PNG), None);
            for dialect in DIALECTS {
                let fixed = retain_supported(dialect, "model", request(vec![png.clone()]));
                if dialect == MediaDialect::Hyperbolic {
                    assert_eq!(
                        text_of(&user_parts(&fixed.unwrap())[0]),
                        Some(note("image/png").as_str())
                    );
                    continue;
                }
                let error = fixed
                    .expect_err("the provider cannot fetch the link")
                    .to_string();
                assert_link_error(&error, "image (image/png)", host, link);
                assert!(error.contains(dialect.provider()), "{error}");
            }
        }

        let pdf = UserContent::document_url(
            "asset://localhost/%2Ftmp%2Freport.pdf",
            Some(DocumentMediaType::PDF),
        );
        for dialect in DIALECTS {
            let fixed = retain_supported(dialect, "model", request(vec![pdf.clone()]));
            if dialect.reads_inline_pdfs() {
                let error = fixed
                    .expect_err("the PDF must have been inlined")
                    .to_string();
                assert_link_error(&error, "document (application/pdf)", "localhost", "pdf");
            } else {
                assert_eq!(
                    text_of(&user_parts(&fixed.unwrap())[0]),
                    Some(note("application/pdf").as_str()),
                    "{dialect:?}"
                );
            }
        }
    }

    #[test]
    fn bytes_travel_as_base64_typed_by_their_content() {
        let png = STANDARD.decode("iVBORw0KGgoAAAANSUhEUg==").unwrap();
        let pdf = b"%PDF-1.7\n\xe2\xe3\xcf\xd3".to_vec();
        let untyped = |data| Document {
            data,
            media_type: None,
            additional_params: None,
        };
        let fixed = retain_supported(
            MediaDialect::OpenAIChat,
            "gpt-4o",
            request(vec![
                UserContent::Image(Image {
                    data: DocumentSourceKind::Raw(png.clone()),
                    media_type: None,
                    detail: None,
                    additional_params: None,
                }),
                UserContent::Document(untyped(DocumentSourceKind::Raw(pdf.clone()))),
                UserContent::image_base64("SUkqAAgAAAA=", None, None),
            ]),
        )
        .unwrap();

        let parts = user_parts(&fixed);
        assert_eq!(
            parts[0],
            UserContent::image_base64(STANDARD.encode(&png), Some(ImageMediaType::PNG), None)
        );
        assert_eq!(
            parts[1],
            UserContent::Document(Document {
                data: DocumentSourceKind::Base64(STANDARD.encode(&pdf)),
                media_type: Some(DocumentMediaType::PDF),
                additional_params: None,
            })
        );
        assert_eq!(text_of(&parts[2]), Some(note("image/tiff").as_str()));
        reaches_provider(MediaDialect::OpenAIChat, fixed).unwrap();

        let docx = b"PK\x03\x04[Content_Types].xml word/document.xml\xff";
        let notes = retain_supported(
            MediaDialect::Groq,
            "model",
            request(vec![
                UserContent::Document(untyped(DocumentSourceKind::Raw(pdf))),
                UserContent::Document(untyped(DocumentSourceKind::Base64(STANDARD.encode(docx)))),
            ]),
        )
        .unwrap();
        assert_eq!(
            user_parts(&notes)
                .iter()
                .map(|part| text_of(part).unwrap().to_string())
                .collect::<Vec<_>>(),
            [note("application/pdf"), note(user_media::DOCX)]
        );
    }

    #[test]
    fn base64_text_documents_reach_the_model_decoded() {
        for dialect in DIALECTS {
            for (kind, expected) in [
                (Kind::Txt, "[document: text/plain]\nHello\n"),
                (Kind::Csv, "[document: text/csv]\na,b\n1,2\n"),
                (
                    Kind::Windows1252Csv,
                    "[document: text/csv]\nStra\u{FFFD}e;M\u{FFFD}nchen\n",
                ),
                (Kind::Json, "[document: unknown type]\n{\"a\":1}\n"),
            ] {
                let filtered = retain_supported(
                    dialect,
                    "model",
                    request(vec![attachment(kind, Source::Base64)]),
                )
                .unwrap();
                assert_eq!(
                    text_of(&user_parts(&filtered)[0]),
                    Some(expected),
                    "{dialect:?} {kind:?}"
                );
            }
        }
    }

    #[test]
    fn unreadable_image_formats_are_named_in_the_note() {
        for dialect in DIALECTS {
            if dialect == MediaDialect::OpenRouter {
                continue;
            }
            for (kind, media_type) in [
                (Kind::Svg, "image/svg+xml"),
                (Kind::UntypedBmp, "image/bmp"),
            ] {
                let filtered = retain_supported(
                    dialect,
                    "model",
                    request(vec![attachment(kind, Source::Url)]),
                )
                .unwrap();
                assert_eq!(
                    text_of(&user_parts(&filtered)[0]),
                    Some(note(media_type).as_str()),
                    "{dialect:?} {kind:?}"
                );
            }
        }
    }

    #[test]
    fn a_message_with_only_unreadable_media_still_carries_text() {
        for dialect in DIALECTS {
            if dialect == MediaDialect::OpenRouter {
                continue;
            }
            let filtered = retain_supported(
                dialect,
                "model",
                request(vec![attachment(Kind::Video, Source::Url)]),
            )
            .unwrap();
            let parts = user_parts(&filtered);
            assert_eq!(parts.len(), 1, "{dialect:?}");
            assert_eq!(
                text_of(&parts[0]),
                Some(note("video/mp4").as_str()),
                "{dialect:?}"
            );
            reaches_provider(dialect, filtered).unwrap();
        }
    }

    #[test]
    fn teams_attachment_shapes_reach_every_provider() {
        let shapes = [
            UserContent::document_url(PRESIGNED, Some(DocumentMediaType::PDF)),
            UserContent::document_url(PRESIGNED, None),
        ];
        for dialect in DIALECTS {
            let rejected_unfiltered = match dialect {
                MediaDialect::OpenAIResponses => [false, true],
                MediaDialect::OpenRouter => [false, false],
                _ => [true, true],
            };
            for (shape, rejected) in shapes.iter().zip(rejected_unfiltered) {
                let teams = request(vec![UserContent::text("What does it say?"), shape.clone()]);
                assert_eq!(
                    reaches_provider(dialect, teams.clone()).is_err(),
                    rejected,
                    "{dialect:?} unfiltered {shape:?}"
                );
                match retain_supported(dialect, "model", teams) {
                    Ok(filtered) => reaches_provider(dialect, filtered)
                        .unwrap_or_else(|error| panic!("{dialect:?} filtered {shape:?}: {error}")),
                    Err(error) => assert!(
                        matches!(dialect, MediaDialect::OpenAIChat | MediaDialect::Anthropic)
                            && shape == &shapes[0],
                        "only providers that read PDFs inline only refuse a PDF link: \
                         {dialect:?} {shape:?}: {error}"
                    ),
                }
            }
        }
    }

    #[tokio::test]
    #[allow(deprecated)]
    async fn providers_send_attachments_through_their_dialect() {
        use crate::egress::ProviderEgress;
        use crate::llm::custom::{self, AmbientCredentials};
        use crate::llm::test_support::{
            chat_completion, json_response, responses_completion, serve_once,
        };
        use crate::provider::ModelProvider;
        use rig::completion::Completion;
        use serde_json::json;
        use std::collections::HashMap;
        use std::time::Duration;

        for (provider_name, dialect) in [
            ("custom:groq", MediaDialect::Groq),
            ("custom:together", MediaDialect::Together),
            ("custom:huggingface", MediaDialect::HuggingFace),
            ("custom:moonshot", MediaDialect::Moonshot),
            ("custom:hyperbolic", MediaDialect::Hyperbolic),
            ("custom:mozilla", MediaDialect::Llamafile),
        ] {
            let reply = match dialect {
                MediaDialect::Llamafile => responses_completion("test-model"),
                _ => chat_completion("test-model", "stop", 1),
            };
            let (endpoint, server) = serve_once(json_response(reply)).await;
            let provider = ModelProvider {
                provider_name: provider_name.to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: Some(HashMap::from([
                    ("api_key".to_string(), json!("test-key")),
                    ("model_id".to_string(), json!("test-model")),
                    ("endpoint".to_string(), json!(endpoint)),
                ])),
            };
            let model = custom::build(
                provider_name,
                &provider,
                ProviderEgress::Open,
                AmbientCredentials::Forbidden,
            )
            .await
            .unwrap();
            let agent = model
                .provider()
                .await
                .unwrap()
                .into_client()
                .agent("test-model")
                .build();
            let attachments = Message::User {
                content: OneOrMany::many(vec![
                    UserContent::text("What do these say?"),
                    attachment(Kind::Image, Source::Base64),
                    attachment(Kind::Pdf, Source::Base64),
                    attachment(Kind::Audio, Source::Base64),
                    attachment(Kind::Csv, Source::Base64),
                ])
                .unwrap(),
            };
            let sent = agent
                .completion(attachments, Vec::<Message>::new())
                .await
                .unwrap()
                .send()
                .await
                .err();
            let body = tokio::time::timeout(Duration::from_secs(10), server)
                .await
                .unwrap_or_else(|_| panic!("{provider_name} sent nothing: {sent:?}"))
                .unwrap();
            let body: Value = serde_json::from_str(&body).unwrap();

            for part in content_parts(&body) {
                assert!(
                    provider_takes(dialect, part),
                    "{provider_name} receives a part it rejects: {part}"
                );
            }
            let sent_text = body.to_string();
            let notes = match dialect {
                MediaDialect::Hyperbolic => 3,
                _ => 2,
            };
            assert_eq!(
                sent_text
                    .matches("[attachment omitted: unsupported file type ")
                    .count(),
                notes,
                "{provider_name}: {body}"
            );
            assert!(
                sent_text.contains("[document: text/csv]"),
                "{provider_name}: {body}"
            );
            assert_eq!(
                sent_text.contains("data:image/png;base64,AAAA"),
                dialect != MediaDialect::Hyperbolic,
                "{provider_name}: {body}"
            );
        }
    }
}
