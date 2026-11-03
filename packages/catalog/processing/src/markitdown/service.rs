//! Document conversion and goal-directed summaries without a flow: the caller supplies the
//! file's bytes and, where a model reads pages, a model it built itself.

use flow_like_model_provider::history::History;
use flow_like_model_provider::llm::ModelLogic;
use flow_like_model_provider::llm::lenient_settings::{LenientSettings, RejectedSettings};
use flow_like_types::{Bytes, tokio};
use markitdown::{ConversionOptions, MarkItDown, SummaryConfig, create_llm_client_with_config};
use std::sync::Arc;

pub use flow_like_model_provider::history::HistoryThinking;
pub use markitdown::{Document, DocumentSummary, LlmConfig, SharedLlmClient, SummaryStrategy};

/// A model that reads document pages, with the request settings applied to every call.
#[derive(Clone)]
pub struct ReaderModel {
    pub model: Arc<dyn ModelLogic>,
    /// The name the provider's thinking parameters are mapped for: the model Bit's id.
    pub model_name: String,
    pub thinking: HistoryThinking,
    pub config: LlmConfig,
}

/// markitdown's client for a [`ReaderModel`]. Temperature, thinking and max tokens are each
/// dropped once the provider rejects them, for that request and every later one.
#[derive(Clone)]
pub struct ReaderClient {
    pub client: SharedLlmClient,
    /// Settings left out while the client was built.
    pub warnings: Vec<String>,
    rejected: RejectedSettings,
}

impl ReaderClient {
    /// The settings the provider rejected since the last call, worded for a log.
    pub fn rejected_settings(&self) -> Vec<String> {
        self.rejected
            .take_unreported()
            .into_iter()
            .map(|rejected| {
                format!(
                    "The model rejected the {} setting, so it is left out of this and later requests: {}",
                    rejected.setting.label(),
                    rejected.error
                )
            })
            .collect()
    }
}

/// markitdown builds its own requests, so the provider's thinking parameters are mapped from a
/// History once and merged into every request it sends.
pub async fn build_llm_client(model: &ReaderModel) -> flow_like_types::Result<ReaderClient> {
    let mut history = History::new(model.model_name.clone(), Vec::new());
    history.thinking = Some(model.thinking);
    let mut warnings = Vec::new();
    let params = match model.model.agent_settings(&Some(history)) {
        Ok(settings) => settings.params,
        Err(error) => {
            warnings.push(format!(
                "The model takes no Thinking setting, so it is left out: {error}"
            ));
            None
        }
    };

    #[allow(deprecated)]
    let completion_handle = model.model.completion_model_handle(None).await?;
    let lenient = LenientSettings::new(completion_handle, params);
    let rejected = lenient.rejected();

    Ok(ReaderClient {
        client: create_llm_client_with_config(lenient, model.config.clone()),
        warnings,
        rejected,
    })
}

#[derive(Clone, Debug, Default)]
pub struct ConvertOptions {
    /// Extract embedded images into the pages.
    pub extract_images: bool,
    /// Send every PDF page to the model instead of only pages whose text layer reads poorly.
    pub force_ocr: bool,
    /// The file's path, given to the model as context for its images.
    pub source_path: Option<String>,
}

/// Converts a document into pages of markdown. Without a reader, text layers are read as they
/// are; with one, the model transcribes pages that need vision and describes images.
/// `extension` (e.g. `.pdf`) picks the converter; `None` detects the format from the bytes.
pub async fn convert(
    bytes: Bytes,
    extension: Option<&str>,
    options: &ConvertOptions,
    reader: Option<&ReaderClient>,
) -> flow_like_types::Result<Document> {
    let mut conversion = ConversionOptions::default()
        .with_images(options.extract_images)
        .with_force_llm_ocr(options.force_ocr);
    if let Some(reader) = reader {
        conversion = conversion.with_llm(reader.client.clone());
    }
    convert_isolated(
        bytes,
        located(conversion, extension, options.source_path.as_deref()),
    )
    .await
}

#[derive(Clone, Debug)]
pub struct SummarizeOptions {
    /// What the summary must answer or concentrate on, e.g. a question. `None` (or blank)
    /// writes a general summary.
    pub goal: Option<String>,
    /// Language of the summary and notes; `None` writes in the document's language.
    pub language: Option<String>,
    pub strategy: SummaryStrategy,
    /// Target length of the summary in words.
    pub summary_words: usize,
    /// 0 disables key points.
    pub max_key_points: usize,
    /// Send embedded images to the model. Rendered pages of scanned PDFs are always sent.
    pub include_images: bool,
    /// Concurrent model requests, at least one.
    pub parallel_requests: usize,
    /// The file's path, given to the model as context for its images.
    pub source_path: Option<String>,
}

impl Default for SummarizeOptions {
    fn default() -> Self {
        Self {
            goal: None,
            language: None,
            strategy: SummaryStrategy::MapReduce,
            summary_words: 250,
            max_key_points: 8,
            include_images: true,
            parallel_requests: 4,
            source_path: None,
        }
    }
}

impl SummarizeOptions {
    pub fn summary_config(&self) -> SummaryConfig {
        let mut config = SummaryConfig::default()
            .with_strategy(self.strategy)
            .with_summary_words(self.summary_words)
            .with_max_key_points(self.max_key_points)
            .with_images(self.include_images)
            .with_concurrency(self.parallel_requests.max(1));
        if let Some(goal) = non_blank(self.goal.as_deref()) {
            config = config.with_focus(goal);
        }
        if let Some(language) = non_blank(self.language.as_deref()) {
            config = config.with_language(language);
        }
        config
    }
}

fn non_blank(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub struct FileSummary {
    /// Summary, key points with page citations, failed pages and token usage.
    pub summary: DocumentSummary,
    /// The document as read for the summary: its pages hold the notes for each page.
    pub document: Document,
    /// Summary, key points and page notes as one markdown document.
    pub markdown: String,
}

/// Reads the whole document with the reader's model and summarizes it towards the goal.
/// Pages are read, never transcribed. A read that fails for some pages still succeeds and
/// lists them in [`DocumentSummary::failed_pages`].
pub async fn summarize(
    bytes: Bytes,
    extension: Option<&str>,
    options: &SummarizeOptions,
    reader: &ReaderClient,
) -> flow_like_types::Result<FileSummary> {
    let conversion = ConversionOptions::default()
        .with_images(options.include_images)
        .with_llm(reader.client.clone())
        .with_summarization(options.summary_config());
    let document = convert_isolated(
        bytes,
        located(conversion, extension, options.source_path.as_deref()),
    )
    .await?;
    let summary = document.summary.clone().ok_or_else(|| match &options.source_path {
        Some(path) => flow_like_types::anyhow!(
            "Summarizing `{}` returned no summary; the converter ignored summarization mode",
            path
        ),
        None => flow_like_types::anyhow!(
            "Summarizing the document returned no summary; the converter ignored summarization mode"
        ),
    })?;
    let markdown = document.to_markdown();
    Ok(FileSummary {
        summary,
        document,
        markdown,
    })
}

fn located(
    mut conversion: ConversionOptions,
    extension: Option<&str>,
    source_path: Option<&str>,
) -> ConversionOptions {
    if let Some(extension) = extension {
        conversion = conversion.with_extension(extension);
    }
    if let Some(path) = source_path {
        conversion = conversion.with_image_context_path(path);
        conversion.url = Some(path.to_string());
    }
    conversion
}

/// Converters parse untrusted files, so a panic inside one becomes this call's error.
async fn convert_isolated(
    bytes: Bytes,
    options: ConversionOptions,
) -> flow_like_types::Result<Document> {
    let handle =
        tokio::task::spawn(
            async move { MarkItDown::new().convert_bytes(bytes, Some(options)).await },
        );

    match handle.await {
        Ok(Ok(document)) => Ok(document),
        Ok(Err(e)) => Err(flow_like_types::anyhow!(
            "Document conversion failed: {}",
            e
        )),
        Err(e) if e.is_panic() => {
            let panic_msg = if let Ok(reason) = e.try_into_panic() {
                if let Some(s) = reason.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = reason.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "Unknown panic".to_string()
                }
            } else {
                "Unknown panic".to_string()
            };
            Err(flow_like_types::anyhow!(
                "Document conversion panicked: {}",
                panic_msg
            ))
        }
        Err(e) => Err(flow_like_types::anyhow!(
            "Document conversion task failed: {}",
            e
        )),
    }
}

#[cfg(test)]
impl ReaderClient {
    fn from_client(client: SharedLlmClient) -> Self {
        Self {
            client,
            warnings: Vec::new(),
            rejected: RejectedSettings::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use markitdown::MockLlmClient;
    use markitdown::error::MarkitdownError;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../apps/desktop/lib/university/courses/specialist/document-processing/fixtures"
    );

    fn fixture(name: &str) -> Bytes {
        Bytes::from(std::fs::read(format!("{FIXTURES}/{name}")).unwrap())
    }

    fn expected() -> flow_like_types::Value {
        flow_like_types::json::from_slice(
            &std::fs::read(format!("{FIXTURES}/expected.json")).unwrap(),
        )
        .unwrap()
    }

    fn page_count(name: &str) -> usize {
        expected()["expected_page_counts"][name].as_u64().unwrap() as usize
    }

    fn mock_reader(text: &str) -> ReaderClient {
        ReaderClient::from_client(Arc::new(
            MockLlmClient::new()
                .with_text_response(text)
                .with_image_response(text),
        ))
    }

    #[tokio::test]
    async fn converts_a_text_pdf_without_a_model() {
        let document = convert(
            fixture("digital-agreement.pdf"),
            Some(".pdf"),
            &ConvertOptions::default(),
            None,
        )
        .await
        .unwrap();

        assert_eq!(document.pages.len(), page_count("digital-agreement.pdf"));
        let markdown = document.to_markdown();
        let facts = expected();
        for fact in ["agreement_id", "signer", "email"] {
            let value = facts[fact].as_str().unwrap();
            assert!(
                markdown.contains(value),
                "{fact} `{value}` missing in {markdown}"
            );
        }
    }

    #[tokio::test]
    async fn only_a_reader_recovers_the_scanned_page_of_a_mixed_pdf() {
        let text_only = convert(
            fixture("mixed-agreement.pdf"),
            Some(".pdf"),
            &ConvertOptions::default(),
            None,
        )
        .await
        .unwrap();
        let numbers: Vec<u32> = text_only
            .pages
            .iter()
            .map(|page| page.page_number)
            .collect();
        assert_eq!(numbers, vec![1], "the scanned page has no text layer");

        let read = convert(
            fixture("mixed-agreement.pdf"),
            Some(".pdf"),
            &ConvertOptions::default(),
            Some(&mock_reader("Support response target: 4 hours")),
        )
        .await
        .unwrap();
        let numbers: Vec<u32> = read.pages.iter().map(|page| page.page_number).collect();
        assert_eq!(numbers.len(), page_count("mixed-agreement.pdf"));
        assert_eq!(numbers, vec![1, 2]);
        assert!(read.pages[0].to_markdown().contains("PRACTICE-001"));
        assert!(
            read.pages[1]
                .to_markdown()
                .contains(expected()["mixed_page_2_fact"].as_str().unwrap())
        );
    }

    #[tokio::test]
    async fn detects_the_format_when_no_extension_is_given() {
        let detected = convert(
            fixture("digital-agreement.pdf"),
            None,
            &ConvertOptions::default(),
            None,
        )
        .await
        .unwrap();
        assert_eq!(detected.pages.len(), 1);

        let error = convert(
            Bytes::from_static(b"plain words"),
            Some(".definitely-not-a-format"),
            &ConvertOptions::default(),
            None,
        )
        .await
        .unwrap_err();
        assert!(
            error.to_string().starts_with("Document conversion failed"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_reader_transcribes_scanned_pages() {
        let document = convert(
            fixture("scanned-agreement.pdf"),
            Some(".pdf"),
            &ConvertOptions {
                force_ocr: true,
                source_path: Some("agreements/scanned-agreement.pdf".to_string()),
                ..ConvertOptions::default()
            },
            Some(&mock_reader("Agreement PRACTICE-001 transcribed")),
        )
        .await
        .unwrap();

        assert_eq!(document.pages.len(), page_count("scanned-agreement.pdf"));
        assert!(
            document
                .to_markdown()
                .contains("Agreement PRACTICE-001 transcribed")
        );
    }

    const SUMMARY_REPLY: &str = "<notes page=\"1\">\n- Renewal notice of 30 days\n</notes>\n\
                                 <notes page=\"2\">\n- Support response target: 4 hours\n</notes>\n\
                                 <summary>The agreement renews after 30 days.</summary>\n\
                                 <key_points>\n- Renews after 30 days [p. 1]\n</key_points>";

    #[derive(Default)]
    struct RecordingLlm {
        prompts: std::sync::Mutex<Vec<String>>,
        config: LlmConfig,
    }

    #[async_trait::async_trait]
    impl markitdown::LlmClient for RecordingLlm {
        async fn describe_image(&self, _: &[u8], _: &str) -> Result<String, MarkitdownError> {
            Ok(String::new())
        }

        async fn describe_image_base64(&self, _: &str, _: &str) -> Result<String, MarkitdownError> {
            Ok(String::new())
        }

        async fn describe_images_batch(
            &self,
            images: &[(&[u8], &str)],
        ) -> Result<Vec<String>, MarkitdownError> {
            Ok(vec![String::new(); images.len()])
        }

        async fn convert_page_image(&self, _: &[u8], _: &str) -> Result<String, MarkitdownError> {
            Ok(String::new())
        }

        async fn convert_page_images_batch(&self, pages: &[(&[u8], &str)]) -> Vec<Option<String>> {
            vec![None; pages.len()]
        }

        async fn complete(&self, prompt: &str) -> Result<String, MarkitdownError> {
            self.prompts.lock().unwrap().push(prompt.to_string());
            Ok(SUMMARY_REPLY.to_string())
        }

        fn config(&self) -> &LlmConfig {
            &self.config
        }
    }

    #[tokio::test]
    async fn summarizes_a_file_towards_the_goal() {
        let llm = Arc::new(RecordingLlm::default());
        let reader = ReaderClient::from_client(llm.clone());

        let summary = summarize(
            fixture("mixed-agreement.pdf"),
            Some(".pdf"),
            &SummarizeOptions {
                goal: Some("When does the agreement renew?".to_string()),
                source_path: Some("agreements/mixed-agreement.pdf".to_string()),
                ..SummarizeOptions::default()
            },
            &reader,
        )
        .await
        .unwrap();

        assert_eq!(
            summary.summary.summary,
            "The agreement renews after 30 days."
        );
        assert_eq!(summary.summary.key_points.len(), 1);
        assert!(summary.summary.failed_pages.is_empty());
        assert_eq!(summary.document.pages.len(), 2);
        assert_eq!(summary.markdown, summary.document.to_markdown());
        assert!(
            summary
                .markdown
                .contains("Support response target: 4 hours")
        );

        let prompts = llm.prompts.lock().unwrap();
        assert_eq!(
            prompts.len(),
            1,
            "a two-page document is read in one request"
        );
        assert!(prompts[0].contains("When does the agreement renew?"));
        assert!(prompts[0].contains("PRACTICE-001"));
        assert!(reader.rejected_settings().is_empty());
    }

    #[test]
    fn summary_options_map_onto_the_summary_config() {
        let options = SummarizeOptions {
            goal: Some("  financial risks ".to_string()),
            language: Some("German".to_string()),
            strategy: SummaryStrategy::Sequential,
            summary_words: 120,
            max_key_points: 0,
            include_images: false,
            parallel_requests: 0,
            source_path: None,
        };
        let config = options.summary_config();
        assert_eq!(config.focus.as_deref(), Some("financial risks"));
        assert_eq!(config.language.as_deref(), Some("German"));
        assert_eq!(config.strategy, SummaryStrategy::Sequential);
        assert_eq!(config.summary_words, 120);
        assert_eq!(config.max_key_points, 0);
        assert!(!config.include_images);
        assert_eq!(config.concurrency, Some(1));

        let blank = SummarizeOptions {
            goal: Some("   ".to_string()),
            language: Some(String::new()),
            ..SummarizeOptions::default()
        }
        .summary_config();
        assert_eq!(blank.focus, None);
        assert_eq!(blank.language, None);
    }

    #[test]
    fn default_summary_options_match_the_summarize_file_pins() {
        use flow_like::flow::node::NodeLogic;

        let node = super::super::SummarizeFileNode::new().get_node();
        let literal = |name: &str| {
            node.get_pin_by_name(name)
                .and_then(|pin| pin.default_value.as_ref())
                .and_then(|bytes| {
                    flow_like_types::json::from_slice::<flow_like_types::Value>(bytes).ok()
                })
                .unwrap_or_else(|| panic!("{name} has a default"))
        };
        let defaults = SummarizeOptions::default();

        assert_eq!(literal("summary_words"), defaults.summary_words);
        assert_eq!(literal("max_key_points"), defaults.max_key_points);
        assert_eq!(literal("include_images"), defaults.include_images);
        assert_eq!(literal("parallel_requests"), defaults.parallel_requests);
        assert_eq!(literal("strategy"), "MapReduce");
        assert_eq!(defaults.strategy, SummaryStrategy::MapReduce);
        assert_eq!(literal("focus"), "");
        assert_eq!(defaults.goal, None);
    }
}
