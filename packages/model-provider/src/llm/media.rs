use base64::{Engine as _, engine::general_purpose::STANDARD};
use rig::completion::{CompletionRequest, Message};
use rig::message::{
    Audio, AudioMediaType, Document, DocumentMediaType, DocumentSourceKind, Image, ImageMediaType,
    MimeType, UserContent, Video,
};

/// The user-content conversion a Rig 0.38.2 client runs before it sends a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaDialect {
    /// Chat Completions: OpenAI, Azure, Bedrock and the hosted OpenAI-compatible proxy.
    OpenAIChat,
    OpenAIResponses,
    Anthropic,
    OpenRouter,
}

enum Fit {
    Native,
    Text(String),
    Unreadable,
}

/// Rig fails the whole request on one attachment its converter rejects. Replaces each such part
/// with a text part: inline text documents keep their content, anything else becomes a note.
pub fn retain_supported(
    dialect: MediaDialect,
    model: &str,
    mut request: CompletionRequest,
) -> CompletionRequest {
    let mut omitted = Vec::new();
    let mut as_text = 0usize;
    for message in request.chat_history.iter_mut() {
        let Message::User { content } = message else {
            continue;
        };
        for part in content.iter_mut() {
            match fit(dialect, model, part) {
                Fit::Native => {}
                Fit::Text(text) => {
                    *part = UserContent::text(text);
                    as_text += 1;
                }
                Fit::Unreadable => {
                    let Some((kind, media_type, source)) = describe(part) else {
                        continue;
                    };
                    let note = format!(
                        "[attachment omitted: this model cannot read {kind} ({media_type}) sent as {}]",
                        source_label(source)
                    );
                    omitted.push(kind);
                    *part = UserContent::text(note);
                }
            }
        }
    }

    if !omitted.is_empty() || as_text > 0 {
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
    request
}

fn fit(dialect: MediaDialect, model: &str, part: &UserContent) -> Fit {
    let readable = match part {
        UserContent::Text(_) | UserContent::ToolResult(_) => true,
        UserContent::Image(image) => image_fits(dialect, image),
        UserContent::Audio(audio) => audio_fits(dialect, model, audio),
        UserContent::Video(video) => video_fits(dialect, video),
        UserContent::Document(document) => document_fits(dialect, document),
    };
    match part {
        _ if readable => Fit::Native,
        UserContent::Document(document) => inline_text(document).map_or(Fit::Unreadable, |text| {
            Fit::Text(format!(
                "[document: {}]\n{text}",
                mime(document.media_type.as_ref())
            ))
        }),
        _ => Fit::Unreadable,
    }
}

/// Image link extensions of formats OpenAI and Anthropic reject, for links without a media type.
const UNREADABLE_IMAGE_EXTENSIONS: [&str; 8] =
    ["avif", "bmp", "heic", "heif", "ico", "svg", "tif", "tiff"];

fn image_fits(dialect: MediaDialect, image: &Image) -> bool {
    let media_type = image.media_type.as_ref();
    match (dialect, &image.data) {
        (MediaDialect::OpenRouter, DocumentSourceKind::Url(_)) => true,
        (MediaDialect::OpenRouter, DocumentSourceKind::Base64(_)) => media_type.is_some(),
        (_, DocumentSourceKind::Url(url)) => {
            media_type.map_or_else(|| !has_unreadable_image_extension(url), web_image)
        }
        (_, DocumentSourceKind::Base64(_)) => media_type.is_some_and(web_image),
        _ => false,
    }
}

fn web_image(media_type: &ImageMediaType) -> bool {
    matches!(
        media_type,
        ImageMediaType::JPEG | ImageMediaType::PNG | ImageMediaType::GIF | ImageMediaType::WEBP
    )
}

fn has_unreadable_image_extension(url: &str) -> bool {
    url.split(['?', '#'])
        .next()
        .and_then(|path| path.rsplit('/').next())
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, extension)| {
            UNREADABLE_IMAGE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

fn audio_fits(dialect: MediaDialect, model: &str, audio: &Audio) -> bool {
    if !matches!(audio.data, DocumentSourceKind::Base64(_)) {
        return false;
    }
    match dialect {
        MediaDialect::OpenAIChat => {
            matches!(
                audio.media_type,
                Some(AudioMediaType::WAV | AudioMediaType::MP3)
            ) && model.to_ascii_lowercase().contains("audio")
        }
        MediaDialect::OpenRouter => audio.media_type.is_some(),
        MediaDialect::OpenAIResponses | MediaDialect::Anthropic => false,
    }
}

fn video_fits(dialect: MediaDialect, video: &Video) -> bool {
    dialect == MediaDialect::OpenRouter
        && match &video.data {
            DocumentSourceKind::Url(_) => true,
            DocumentSourceKind::Base64(_) => video.media_type.is_some(),
            _ => false,
        }
}

/// Rig hands OpenAI and Anthropic the base64 payload of any non-PDF document as literal text, so
/// only PDFs stay base64 there.
fn document_fits(dialect: MediaDialect, document: &Document) -> bool {
    use MediaDialect::{Anthropic, OpenAIChat, OpenAIResponses, OpenRouter};
    let typed = document.media_type.is_some();
    let pdf = document.media_type == Some(DocumentMediaType::PDF);
    match (dialect, &document.data) {
        (OpenAIChat | OpenAIResponses | Anthropic, DocumentSourceKind::FileId(_)) => true,
        (OpenAIResponses, DocumentSourceKind::Url(_)) => pdf,
        (OpenAIChat | OpenAIResponses | Anthropic, DocumentSourceKind::Base64(_)) => pdf,
        (OpenAIChat | OpenAIResponses, DocumentSourceKind::String(_)) => typed && !pdf,
        (Anthropic, DocumentSourceKind::String(_)) => {
            document.media_type == Some(DocumentMediaType::TXT)
        }
        (OpenRouter, DocumentSourceKind::Url(_)) => true,
        (OpenRouter, DocumentSourceKind::Base64(_) | DocumentSourceKind::String(_)) => typed,
        _ => false,
    }
}

/// Typed documents other than PDFs are text, so invalid UTF-8 there is a legacy encoding and is
/// decoded lossily; untyped bytes must be valid UTF-8 to count as text.
fn inline_text(document: &Document) -> Option<String> {
    let decoded;
    let bytes = match &document.data {
        DocumentSourceKind::String(text) => return Some(text.clone()),
        DocumentSourceKind::Base64(data) => {
            decoded = STANDARD.decode(data).ok()?;
            &decoded
        }
        DocumentSourceKind::Raw(bytes) => bytes,
        _ => return None,
    };
    match document.media_type {
        None | Some(DocumentMediaType::PDF) => std::str::from_utf8(bytes).ok().map(str::to_owned),
        Some(_) => Some(String::from_utf8_lossy(bytes).into_owned()),
    }
}

fn describe(part: &UserContent) -> Option<(&'static str, &'static str, &DocumentSourceKind)> {
    match part {
        UserContent::Image(image) => Some(("images", mime(image.media_type.as_ref()), &image.data)),
        UserContent::Audio(audio) => Some(("audio", mime(audio.media_type.as_ref()), &audio.data)),
        UserContent::Video(video) => Some(("video", mime(video.media_type.as_ref()), &video.data)),
        UserContent::Document(document) => Some((
            "documents",
            mime(document.media_type.as_ref()),
            &document.data,
        )),
        UserContent::Text(_) | UserContent::ToolResult(_) => None,
    }
}

fn mime<T: MimeType>(media_type: Option<&T>) -> &'static str {
    media_type.map_or("unknown type", MimeType::to_mime_type)
}

fn source_label(source: &DocumentSourceKind) -> &'static str {
    match source {
        DocumentSourceKind::Url(_) => "a link",
        DocumentSourceKind::FileId(_) => "a file id",
        DocumentSourceKind::Base64(_)
        | DocumentSourceKind::String(_)
        | DocumentSourceKind::Raw(_) => "inline data",
        _ => "an empty source",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::OneOrMany;
    use rig::message::VideoMediaType;

    const PRESIGNED: &str = "https://flow-like-content.s3.eu-central-1.amazonaws.com/tmp/user/u/apps/a/runs/r/request/teams/files/0/report.pdf?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=ASIAEXAMPLE%2F20260929%2Feu-central-1%2Fs3%2Faws4_request&X-Amz-Security-Token=secret-token&X-Amz-Signature=deadbeef";
    const PDF_BASE64: &str = "JVBERi0xLjQK";
    const TXT_BASE64: &str = "SGVsbG8K";
    const CSV_BASE64: &str = "YSxiCjEsMgo=";
    /// `Straße;München` saved as Windows-1252, which is not valid UTF-8.
    const WINDOWS_1252_CSV_BASE64: &str = "U3RyYd9lO038bmNoZW4K";
    const DOCX_BASE64: &str = "UEsDBBQABgAIAAAAIQD/";

    #[derive(Debug, Clone, Copy)]
    enum Kind {
        Image,
        Heic,
        Svg,
        UntypedBmp,
        Pdf,
        Txt,
        Csv,
        Windows1252Csv,
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
        Omit,
        AsText,
    }
    use Expect::{AsText, Keep, Omit};

    const KINDS: [Kind; 11] = [
        Kind::Image,
        Kind::Heic,
        Kind::Svg,
        Kind::UntypedBmp,
        Kind::Pdf,
        Kind::Txt,
        Kind::Csv,
        Kind::Windows1252Csv,
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
            Kind::Untyped => DOCX_BASE64,
            Kind::Image | Kind::Heic | Kind::Svg | Kind::UntypedBmp | Kind::Audio | Kind::Video => {
                "AAAA"
            }
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
            Kind::Heic => image(Some(ImageMediaType::HEIC)),
            Kind::Svg => image(Some(ImageMediaType::SVG)),
            Kind::UntypedBmp => image(None),
            Kind::Pdf => document(Some(DocumentMediaType::PDF)),
            Kind::Txt => document(Some(DocumentMediaType::TXT)),
            Kind::Csv | Kind::Windows1252Csv => document(Some(DocumentMediaType::CSV)),
            Kind::Untyped => document(None),
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

    /// Converts the request the way the dialect's Rig client does before sending it.
    fn rig_converts(dialect: MediaDialect, request: CompletionRequest) -> Result<(), String> {
        use rig::providers::{anthropic, openai, openrouter};
        let model = "test-model".to_string();
        match dialect {
            MediaDialect::OpenAIChat => {
                openai::completion::CompletionRequest::try_from((model, request))
                    .map(drop)
                    .map_err(|error| error.to_string())
            }
            MediaDialect::OpenAIResponses => {
                openai::responses_api::CompletionRequest::try_from((model, request))
                    .map(drop)
                    .map_err(|error| error.to_string())
            }
            MediaDialect::Anthropic => request.chat_history.into_iter().try_for_each(|message| {
                anthropic::completion::Message::try_from(message)
                    .map(drop)
                    .map_err(|error| error.to_string())
            }),
            MediaDialect::OpenRouter => request.chat_history.into_iter().try_for_each(|message| {
                Vec::<openrouter::completion::Message>::try_from(message)
                    .map(drop)
                    .map_err(|error| error.to_string())
            }),
        }
    }

    fn assert_table(dialect: MediaDialect, model: &str, table: [[Expect; 4]; KINDS.len()]) {
        for (kind, row) in KINDS.into_iter().zip(table) {
            for (from, expected) in SOURCES.into_iter().zip(row) {
                let original = attachment(kind, from);
                let filtered = retain_supported(
                    dialect,
                    model,
                    request(vec![UserContent::text("Summarise this."), original.clone()]),
                );
                let parts = user_parts(&filtered);
                let case = format!("{dialect:?} {kind:?} via {from:?}");
                assert_eq!(parts.len(), 2, "{case}: the user message lost parts");
                let part = &parts[1];
                match expected {
                    Keep => assert_eq!(part, &original, "{case} must be kept"),
                    Omit => {
                        let note =
                            text_of(part).unwrap_or_else(|| panic!("{case} must be replaced"));
                        assert!(note.starts_with("[attachment omitted: "), "{case}: {note}");
                        assert!(!note.contains("X-Amz"), "{case} leaked the URL: {note}");
                    }
                    AsText => {
                        let text =
                            text_of(part).unwrap_or_else(|| panic!("{case} must become text"));
                        assert!(text.starts_with("[document: "), "{case}: {text}");
                    }
                }
                rig_converts(dialect, filtered)
                    .unwrap_or_else(|error| panic!("{case}: Rig still rejects it: {error}"));
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
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Keep, AsText, Keep],
                [Omit, AsText, Keep, Keep],
                [Omit, AsText, Keep, Keep],
                [Omit, AsText, Keep, Keep],
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
            );
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
            );
            assert_ne!(user_parts(&filtered)[0], original, "{media_type:?}");
        }
    }

    #[test]
    fn openai_responses() {
        assert_table(
            MediaDialect::OpenAIResponses,
            "gpt-5.4",
            [
                [Keep, Keep, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Keep, Keep, AsText, Keep],
                [Omit, AsText, Keep, Keep],
                [Omit, AsText, Keep, Keep],
                [Omit, AsText, Keep, Keep],
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
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Omit, Omit, Omit],
                [Omit, Keep, AsText, Keep],
                [Omit, AsText, Keep, Keep],
                [Omit, AsText, AsText, Keep],
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
                [Keep, Omit, Omit, Omit],
                [Keep, Keep, Keep, Omit],
                [Keep, Keep, Keep, Omit],
                [Keep, Keep, Keep, Omit],
                [Keep, Keep, Keep, Omit],
                [Keep, Omit, AsText, Omit],
                [Omit, Keep, Omit, Omit],
                [Keep, Keep, Omit, Omit],
            ],
        );
    }

    #[test]
    fn base64_text_documents_reach_the_model_decoded() {
        for dialect in [
            MediaDialect::OpenAIChat,
            MediaDialect::OpenAIResponses,
            MediaDialect::Anthropic,
        ] {
            for (kind, expected) in [
                (Kind::Txt, "[document: text/plain]\nHello\n"),
                (Kind::Csv, "[document: text/csv]\na,b\n1,2\n"),
                (
                    Kind::Windows1252Csv,
                    "[document: text/csv]\nStra\u{FFFD}e;M\u{FFFD}nchen\n",
                ),
            ] {
                let filtered = retain_supported(
                    dialect,
                    "model",
                    request(vec![attachment(kind, Source::Base64)]),
                );
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
        for dialect in [
            MediaDialect::OpenAIChat,
            MediaDialect::OpenAIResponses,
            MediaDialect::Anthropic,
        ] {
            let filtered = retain_supported(
                dialect,
                "model",
                request(vec![attachment(Kind::Heic, Source::Url)]),
            );
            assert_eq!(
                text_of(&user_parts(&filtered)[0]),
                Some(
                    "[attachment omitted: this model cannot read images (image/heic) sent as a link]"
                ),
                "{dialect:?}"
            );
        }
    }

    #[test]
    fn a_message_with_only_unreadable_media_still_carries_text() {
        for dialect in [
            MediaDialect::OpenAIChat,
            MediaDialect::OpenAIResponses,
            MediaDialect::Anthropic,
        ] {
            let filtered = retain_supported(
                dialect,
                "model",
                request(vec![attachment(Kind::Video, Source::Url)]),
            );
            let parts = user_parts(&filtered);
            assert_eq!(parts.len(), 1, "{dialect:?}");
            assert_eq!(
                text_of(&parts[0]),
                Some(
                    "[attachment omitted: this model cannot read video (video/mp4) sent as a link]"
                ),
                "{dialect:?}"
            );
            rig_converts(dialect, filtered).unwrap();
        }
    }

    #[test]
    fn teams_attachment_shapes_reach_every_provider() {
        let shapes = [
            UserContent::document_url(PRESIGNED, Some(DocumentMediaType::PDF)),
            UserContent::document_url(PRESIGNED, None),
        ];
        for (dialect, rejected_unfiltered) in [
            (MediaDialect::OpenAIChat, [true, true]),
            (MediaDialect::OpenAIResponses, [false, true]),
            (MediaDialect::Anthropic, [true, true]),
            (MediaDialect::OpenRouter, [false, false]),
        ] {
            for (shape, rejected) in shapes.iter().zip(rejected_unfiltered) {
                let teams = request(vec![UserContent::text("What does it say?"), shape.clone()]);
                assert_eq!(
                    rig_converts(dialect, teams.clone()).is_err(),
                    rejected,
                    "{dialect:?} unfiltered {shape:?}"
                );
                rig_converts(dialect, retain_supported(dialect, "model", teams)).unwrap_or_else(
                    |error| panic!("{dialect:?} rejects the filtered {shape:?}: {error}"),
                );
            }
        }
    }
}
