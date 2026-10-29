use std::any::Any;

use super::ModelLogic;
use super::user_media::{self, DOC, DOCX, Doc, Kind, MediaSplice, PDF, Part, Source};
use crate::provider::random_provider;
use crate::{
    egress::ProviderEgress,
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
};
use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use rig::completion::{CompletionError, CompletionRequest};
use rig::message::{Document, UserContent};
use serde_json::json;

const NAME: &str = "Perplexity";

/// The file types Sonar reads, by link extension.
const FILE_EXTENSIONS: [&str; 5] = ["pdf", "doc", "docx", "txt", "rtf"];

/// Rig's converter refuses every user part but text; images travel past it as `image_url` parts
/// and files as `file_url` parts.
fn perplexity_request(
    model: &str,
    request: CompletionRequest,
) -> Result<CompletionRequest, CompletionError> {
    user_media::deliver(NAME, model, request, media_part)
}

fn media_part(part: &UserContent) -> Result<Part, CompletionError> {
    match part {
        UserContent::Image(image) => user_media::web_image(NAME, image, false),
        UserContent::Document(document) => file_part(document),
        part => user_media::text_only(NAME, part),
    }
}

/// Sonar reads PDF, DOC, DOCX, TXT and RTF files from public links, and PDF and Word files as
/// base64 without a `data:` prefix. Text documents are sent as their text.
fn file_part(document: &Document) -> Result<Part, CompletionError> {
    let url = match Doc::of(document) {
        Doc::File {
            mime: Some(PDF | DOCX | DOC),
            source: Source::Inline { base64, .. },
        } => base64.into_owned(),
        Doc::File {
            mime,
            source: Source::Link(link),
        } if reads_link(mime, link) => link.to_string(),
        Doc::File {
            mime,
            source: Source::LocalLink(link),
        } if reads_link(mime, link) => {
            return Err(user_media::unreachable(NAME, Kind::Document, mime, link));
        }
        doc => return doc.read_as_text(NAME, document),
    };
    Ok(Part::Wire(
        json!({ "type": "file_url", "file_url": { "url": url } }),
    ))
}

fn reads_link(mime: Option<&str>, link: &str) -> bool {
    match mime {
        Some(mime) => matches!(mime, PDF | DOCX | DOC | "text/plain" | "text/rtf"),
        None => link
            .split(['?', '#'])
            .next()
            .and_then(|path| path.rsplit('/').next())
            .and_then(|name| name.rsplit_once('.'))
            .is_some_and(|(_, extension)| {
                FILE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
            }),
    }
}

pub struct PerplexityModel {
    client: rig::providers::perplexity::Client<MediaSplice>,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl PerplexityModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> anyhow::Result<Self> {
        let perplexity_config = random_provider(&config.perplexity_config)?;
        let api_key = perplexity_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::perplexity::Client::builder()
            .api_key(&api_key)
            .http_client(MediaSplice::default());

        if let Some(endpoint) = perplexity_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(PerplexityModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(
        provider: &ModelProvider,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));

        let mut builder = rig::providers::perplexity::Client::builder()
            .api_key(api_key)
            .http_client(MediaSplice::for_egress(egress)?);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(PerplexityModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }
}

impl Cacheable for PerplexityModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for PerplexityModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(user_media::constructor(
            self.client.clone(),
            perplexity_request,
        ))
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::test_support::{json_response, serve_once};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use rig::message::{DocumentMediaType, DocumentSourceKind};
    use serde_json::Value;
    use user_media::tests::{
        PDF_BASE64, PRESIGNED_PDF, assert_names_host_only, document, image, request,
    };

    async fn perplexity_at(endpoint: &str) -> PerplexityModel {
        PerplexityModel::from_provider(
            &ModelProvider {
                provider_name: "perplexity".to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: Some(std::collections::HashMap::from([
                    ("api_key".to_string(), json!("key")),
                    ("endpoint".to_string(), json!(endpoint)),
                ])),
            },
            ProviderEgress::Open,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn images_and_files_reach_sonar_as_content_parts() {
        let (endpoint, server) = serve_once(json_response(json!({
            "id": "sonar-test",
            "model": "sonar-pro",
            "object": "chat.completion",
            "created": 1,
            "choices": [{
                "index": 0,
                "finish_reason": "stop",
                "message": {"role": "assistant", "content": "ok"},
                "delta": {"role": "assistant", "content": ""},
            }],
            "usage": {"prompt_tokens": 12, "completion_tokens": 1, "total_tokens": 13},
        })))
        .await;
        let model = perplexity_at(&endpoint).await;
        let mut docx = b"PK\x03\x04[Content_Types].xml word/document.xml".to_vec();
        docx.push(0xff);
        let mut xlsx = b"PK\x03\x04[Content_Types].xml xl/workbook.xml".to_vec();
        xlsx.push(0xff);
        let jpeg = "/9j/4AAQSkZJRg==";

        model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("sonar-pro")
            .completion(request(vec![
                UserContent::text("Summarise these."),
                image(DocumentSourceKind::Base64(jpeg.to_string()), None),
                document(
                    DocumentSourceKind::Base64(PDF_BASE64.to_string()),
                    Some(DocumentMediaType::PDF),
                ),
                document(DocumentSourceKind::Url(PRESIGNED_PDF.to_string()), None),
                document(DocumentSourceKind::Raw(docx.clone()), None),
                document(DocumentSourceKind::Raw(xlsx), None),
                document(
                    DocumentSourceKind::String("Quarterly notes".to_string()),
                    Some(DocumentMediaType::TXT),
                ),
            ]))
            .await
            .unwrap();

        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(
            body["messages"][1],
            json!({
                "role": "user",
                "content": [
                    {"type": "text", "text": "Summarise these."},
                    {"type": "image_url", "image_url": {"url": format!("data:image/jpeg;base64,{jpeg}")}},
                    {"type": "file_url", "file_url": {"url": PDF_BASE64}},
                    {"type": "file_url", "file_url": {"url": PRESIGNED_PDF}},
                    {"type": "file_url", "file_url": {"url": STANDARD.encode(&docx)}},
                    {"type": "text", "text": "[attachment omitted: unsupported file type document]"},
                    {"type": "text", "text": "Quarterly notes"},
                ],
            })
        );
    }

    #[tokio::test]
    async fn a_link_to_a_csv_file_fails_naming_its_host() {
        let (endpoint, server) = serve_once(json_response(json!({}))).await;
        let model = perplexity_at(&endpoint).await;

        let error = model
            .provider()
            .await
            .unwrap()
            .client()
            .completion_model("sonar-pro")
            .completion(request(vec![document(
                DocumentSourceKind::Url(
                    "https://files.example.com/tmp/report.pdf.csv?X-Amz-Signature=1".to_string(),
                ),
                Some(DocumentMediaType::CSV),
            )]))
            .await
            .unwrap_err()
            .to_string();

        assert_names_host_only(&error, "files.example.com");
        assert!(error.contains("document (text/csv)"), "{error}");
        assert!(!server.is_finished());
        server.abort();
    }
}
