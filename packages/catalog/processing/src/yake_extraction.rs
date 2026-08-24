use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use std::collections::HashSet;
#[cfg(feature = "execute")]
use whatlang::detect;
#[cfg(feature = "execute")]
use yake_rust::{Config, StopWords, get_n_best};

#[crate::register_node]
#[derive(Default)]
pub struct YakeExtractionNode {}

impl YakeExtractionNode {
    pub fn new() -> Self {
        YakeExtractionNode {}
    }
}

#[async_trait]
impl NodeLogic for YakeExtractionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_processing_yake_extraction",
            "YAKE Keywords",
            "Extracts keywords from text using YAKE (Yet Another Keyword Extractor). YAKE is an unsupervised automatic keyword extraction method that uses statistical features from the text itself.",
            "AI/Processing",
        );
        node.set_flowscript_name("ai.processing", "extractKeywordsYake");
        node.add_icon("/flow/icons/key.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_governance(10)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "text",
            "Text",
            "The text to extract keywords from",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "language",
            "Language",
            "Language code for stop words. Use 'auto' for automatic detection.",
            VariableType::String,
        )
        .set_default_value(Some(json!("auto")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "auto".to_string(),
                    "en".to_string(),
                    "de".to_string(),
                    "fr".to_string(),
                    "es".to_string(),
                    "it".to_string(),
                    "pt".to_string(),
                    "nl".to_string(),
                    "pl".to_string(),
                    "ru".to_string(),
                    "ar".to_string(),
                    "fi".to_string(),
                    "tr".to_string(),
                    "sv".to_string(),
                    "da".to_string(),
                    "no".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "ngrams",
            "N-grams",
            "Maximum n-gram size (1-3). Higher values extract longer phrases.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(2)));

        node.add_input_pin(
            "max_keywords",
            "Max Keywords",
            "Maximum number of keywords to return",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "dedup_threshold",
            "Dedup Threshold",
            "Levenshtein distance threshold for deduplication (0.0-1.0). Lower means stricter deduplication.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.9)));

        node.add_output_pin(
            "keywords",
            "Keywords",
            "Extracted keywords as a string set",
            VariableType::String,
        )
        .set_value_type(ValueType::HashSet);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let text: String = context.evaluate_pin("text").await?;
        let language: String = context.evaluate_pin("language").await?;
        let ngrams: i64 = context.evaluate_pin("ngrams").await?;
        let max_keywords: i64 = context.evaluate_pin("max_keywords").await?;
        let dedup_threshold: f64 = context.evaluate_pin("dedup_threshold").await?;

        let lang_code = if language == "auto" {
            detect(&text)
                .map(|info| lang_to_code(info.lang()))
                .unwrap_or("en")
                .to_string()
        } else {
            language
        };

        let stop_words = StopWords::predefined(&lang_code)
            .unwrap_or_else(|| StopWords::predefined("en").unwrap());

        let config = Config {
            ngrams: ngrams.clamp(1, 3) as usize,
            deduplication_threshold: dedup_threshold,
            ..Config::default()
        };

        let keywords = get_n_best(max_keywords.max(1) as usize, &text, &stop_words, &config);

        let result: HashSet<String> = keywords.into_iter().map(|item| item.keyword).collect();

        context.set_pin_value("keywords", json!(result)).await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Processing requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn lang_to_code(lang: whatlang::Lang) -> &'static str {
    use whatlang::Lang::*;
    match lang {
        Eng => "en",
        Deu => "de",
        Fra => "fr",
        Spa => "es",
        Ita => "it",
        Por => "pt",
        Nld => "nl",
        Pol => "pl",
        Rus => "ru",
        Tur => "tr",
        Swe => "sv",
        Dan => "da",
        Nob => "no",
        Fin => "fi",
        Ara => "ar",
        _ => "en",
    }
}
