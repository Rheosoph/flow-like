//! Node for Fitting a TF-IDF Vectorizer
//!
//! This node learns a vocabulary from a text column and produces a fitted vectorizer that turns
//! documents into numeric vectors, using the [`linfa_preprocessing`] crate.

use crate::ml::NodeMLModel;
#[cfg(feature = "execute")]
use crate::ml::{MAX_ML_PREDICTION_RECORDS, MLModel, ModelWithMeta};
use flow_like::flow::board::Board;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_catalog_core::NodeDBConnection;
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{Result, async_trait, json::json};
#[cfg(feature = "execute")]
use linfa_preprocessing::tf_idf_vectorization::{FittedTfIdfVectorizer, TfIdfVectorizer};
#[cfg(feature = "execute")]
use ndarray::Array1;
#[cfg(feature = "execute")]
use std::collections::HashSet;

/// Dropdown value → serde name of the `TfIdfMethod` variant it selects.
#[cfg(feature = "execute")]
const METHODS: [(&str, &str); 3] = [
    ("Smooth", "Smooth"),
    ("Non-Smooth", "NonSmooth"),
    ("Textbook", "Textbook"),
];

/// linfa 0.8.1 ships no setter for the idf method: `TfIdfVectorizer::default()` hardcodes `Smooth`
/// and the field is private on both the parameter set and the fitted vectorizer. The method is only
/// read at transform time, so it is rewritten here through the same serde representation that model
/// persistence already depends on.
#[cfg(feature = "execute")]
fn with_method(fitted: FittedTfIdfVectorizer, method: &str) -> Result<FittedTfIdfVectorizer> {
    let variant = METHODS
        .iter()
        .find(|(label, _)| *label == method)
        .map(|(_, variant)| *variant)
        .ok_or_else(|| {
            anyhow!(
                "Unknown TF-IDF method `{method}`, expected one of {:?}",
                METHODS.map(|(label, _)| label)
            )
        })?;

    if variant == "Smooth" {
        return Ok(fitted);
    }

    let mut value = flow_like_types::json::to_value(&fitted)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| anyhow!("Fitted TF-IDF vectorizer did not serialize as an object"))?;
    object.insert("method".to_string(), json!(variant));
    Ok(flow_like_types::json::from_value(value)?)
}

#[crate::register_node]
#[derive(Default)]
pub struct FitTfIdfVectorizerNode {}

impl FitTfIdfVectorizerNode {
    pub fn new() -> Self {
        FitTfIdfVectorizerNode {}
    }
}

#[async_trait]
impl NodeLogic for FitTfIdfVectorizerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "fit_tfidf_vectorizer",
            "Fit TF-IDF Vectorizer",
            "Learn a vocabulary from a text column and turn documents into numeric vectors weighted by term frequency times inverse document frequency. Feed the fitted vectorizer to Apply Transform to vectorize a column, then train a classifier such as Multinomial Naive Bayes on the result. Tokenization always uses the built-in regex tokenizer, because a custom tokenizer function cannot be persisted and would make the saved model unloadable.",
            "AI/ML/Preprocessing",
        );
        node.set_flowscript_name("ml", "fitTfidfVectorizer");
        node.add_icon("/flow/icons/chart-network.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5) // The vocabulary is verbatim training text and travels with the model
                .set_security(6)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger that begins vectorizer fitting",
            VariableType::Execution,
        );

        node.add_input_pin(
            "source",
            "Data Source",
            "Choose which backend supplies the documents",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Database".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("Database")));

        node.add_input_pin(
            "method",
            "IDF Method",
            "Weighting formula. Smooth: log((1+n)/(1+df))+1, never divides by zero. Non-Smooth: log(n/df)+1, sharper but requires every term to appear at least once. Textbook: log(n/(1+df)), which discounts terms appearing in nearly every document down to a negative weight, so it cannot feed Multinomial Naive Bayes.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Smooth".to_string(),
                    "Non-Smooth".to_string(),
                    "Textbook".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Smooth")));

        node.add_input_pin(
            "n_gram_min",
            "Min N-Gram",
            "Smallest number of adjacent tokens forming a vocabulary entry (1 = single words)",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 10.)).build())
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "n_gram_max",
            "Max N-Gram",
            "Largest number of adjacent tokens forming a vocabulary entry. Must not be smaller than Min N-Gram.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 10.)).build())
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "convert_to_lowercase",
            "Lowercase",
            "Lowercase every document before tokenizing, so casing variants collapse into one vocabulary entry",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "max_features",
            "Max Features",
            "Keep only the most frequent N vocabulary entries, which caps the width of the produced vectors. 0 keeps all of them.",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((0., 1_000_000.)).build())
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "min_document_frequency",
            "Min Document Frequency",
            "Drop terms appearing in a smaller share of documents than this (0-1). Useful to remove typos and one-off tokens.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 1.)).build())
        .set_default_value(Some(json!(0.0)));

        node.add_input_pin(
            "max_document_frequency",
            "Max Document Frequency",
            "Drop terms appearing in a larger share of documents than this (0-1). Useful to remove boilerplate that carries no signal.",
            VariableType::Float,
        )
        .set_options(PinOptions::new().set_range((0., 1.)).build())
        .set_default_value(Some(json!(1.0)));

        node.add_input_pin(
            "stopwords",
            "Stopwords",
            "Comma separated words to exclude from the vocabulary, e.g. `the, and, of`. Leave empty to keep every term.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated once the vectorizer is fitted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "model",
            "Model",
            "Thread-safe handle to the fitted TF-IDF vectorizer, for use with Apply Transform",
            VariableType::Struct,
        )
        .set_schema::<NodeMLModel>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "vocabulary",
            "Vocabulary",
            "Learned vocabulary entries, in the same order as the columns of the produced vectors",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let source: String = context.evaluate_pin("source").await?;
        let method: String = context.evaluate_pin("method").await?;
        let n_gram_min: i64 = context.evaluate_pin("n_gram_min").await?;
        let n_gram_max: i64 = context.evaluate_pin("n_gram_max").await?;
        let lowercase: bool = context.evaluate_pin("convert_to_lowercase").await?;
        let max_features: i64 = context.evaluate_pin("max_features").await?;
        let min_document_frequency: f64 = context.evaluate_pin("min_document_frequency").await?;
        let max_document_frequency: f64 = context.evaluate_pin("max_document_frequency").await?;
        let stopwords: String = context.evaluate_pin("stopwords").await?;

        if n_gram_min < 1 || n_gram_max < 1 {
            return Err(anyhow!(
                "N-gram boundaries must be at least 1, got ({n_gram_min}, {n_gram_max})"
            ));
        }
        if n_gram_min > n_gram_max {
            return Err(anyhow!(
                "Min N-Gram {n_gram_min} is larger than Max N-Gram {n_gram_max}"
            ));
        }
        if max_features < 0 {
            return Err(anyhow!(
                "Max Features must not be negative, got {max_features}"
            ));
        }
        if !(0.0..=1.0).contains(&min_document_frequency)
            || !(0.0..=1.0).contains(&max_document_frequency)
        {
            return Err(anyhow!(
                "Document frequencies are relative shares and must lie in 0..=1, got ({min_document_frequency}, {max_document_frequency})"
            ));
        }
        if min_document_frequency > max_document_frequency {
            return Err(anyhow!(
                "Min Document Frequency {min_document_frequency} is larger than Max Document Frequency {max_document_frequency}"
            ));
        }

        let t0 = std::time::Instant::now();
        let documents = match source.as_str() {
            "Database" => {
                let database: NodeDBConnection = context.evaluate_pin("database").await?;
                let records_col: String = context.evaluate_pin("records").await?;

                let records = {
                    let cached_db = database.load(context).await?;
                    cached_db.ensure_flushed().await?;
                    let database = cached_db.db.read().await;
                    let schema = database.schema().await?;
                    let existing_cols: HashSet<String> =
                        schema.fields.iter().map(|f| f.name().clone()).collect();
                    if !existing_cols.contains(&records_col) {
                        return Err(anyhow!(format!(
                            "Database doesn't contain text col `{}`!",
                            records_col
                        )));
                    }
                    database
                        .filter(
                            "true",
                            Some(vec![records_col.to_string()]),
                            MAX_ML_PREDICTION_RECORDS,
                            0,
                        )
                        .await?
                };
                context.log_message(
                    &format!("Got {} documents for fitting", records.len()),
                    LogLevel::Debug,
                );

                let documents = records
                    .iter()
                    .enumerate()
                    .map(|(row, value)| {
                        value
                            .get(&records_col)
                            .and_then(|v| v.as_str())
                            .map(ToOwned::to_owned)
                            .ok_or_else(|| {
                                anyhow!(
                                    "Row {row}: column `{records_col}` is not text. The TF-IDF Vectorizer fits on a string column, not on a vector column."
                                )
                            })
                    })
                    .collect::<Result<Vec<String>>>()?;
                Array1::from(documents)
            }
            _ => return Err(anyhow!("Datasource Not Implemented!")),
        };
        if documents.is_empty() {
            return Err(anyhow!("No documents to fit the TF-IDF vectorizer on"));
        }
        context.log_message(
            &format!("Preprocess data: {:?}", t0.elapsed()),
            LogLevel::Debug,
        );

        // Vocabulary entries are lowercased before the stopword check, so a mixed-case stopword
        // could never match once lowercasing is on.
        let stopword_list: Vec<String> = stopwords
            .split(',')
            .map(|word| {
                let word = word.trim();
                if lowercase {
                    word.to_lowercase()
                } else {
                    word.to_string()
                }
            })
            .filter(|word| !word.is_empty())
            .collect();

        let t0 = std::time::Instant::now();
        let mut params = TfIdfVectorizer::default()
            .convert_to_lowercase(lowercase)
            .n_gram_range(n_gram_min as usize, n_gram_max as usize)
            .document_frequency(min_document_frequency as f32, max_document_frequency as f32);
        if max_features > 0 {
            params = params.max_features(Some(max_features as usize));
        }
        if !stopword_list.is_empty() {
            params = params.stopwords(&stopword_list);
        }

        let fitted = params
            .fit(&documents)
            .map_err(|err| anyhow!("Fitting the TF-IDF vectorizer failed: {err}"))?;
        let fitted = with_method(fitted, &method)?;
        context.log_message(
            &format!("Fit vectorizer: {:?}", t0.elapsed()),
            LogLevel::Debug,
        );

        let vocabulary = fitted.vocabulary().clone();
        if vocabulary.is_empty() {
            return Err(anyhow!(
                "The learned vocabulary is empty, so every document would vectorize to a zero-width vector. Loosen the filters: document frequency ({min_document_frequency}, {max_document_frequency}), {} stopword(s), max features {max_features}. Note that the tokenizer also skips single-character tokens.",
                stopword_list.len()
            ));
        }
        context.log_message(
            &format!(
                "Learned {} vocabulary entries using the {} method",
                vocabulary.len(),
                method
            ),
            LogLevel::Debug,
        );

        let model = MLModel::TfIdfVectorizer(ModelWithMeta {
            model: fitted,
            classes: None,
        });
        let node_model = NodeMLModel::new(context, model).await;
        context.set_pin_value("model", json!(node_model)).await?;
        context
            .set_pin_value("vocabulary", json!(vocabulary))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> Result<()> {
        Err(flow_like_types::anyhow!(
            "ML execution requires the 'execute' feature. Rebuild with --features execute"
        ))
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        use flow_like_catalog_core::NodeDBConnection;

        let source_pin: String = node
            .get_pin_by_name("source")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<Value>(&bytes).ok())
            .and_then(|json| json.as_str().map(ToOwned::to_owned))
            .unwrap_or_default();

        if source_pin == *"Database" {
            if node.get_pin_by_name("database").is_none() {
                node.add_input_pin(
                    "database",
                    "Database",
                    "Database Connection",
                    VariableType::Struct,
                )
                .set_schema::<NodeDBConnection>()
                .set_options(PinOptions::new().set_enforce_schema(true).build());
            }
            if node.get_pin_by_name("records").is_none() {
                node.add_input_pin(
                    "records",
                    "Text Col",
                    "Column Containing the Documents to Learn the Vocabulary From",
                    VariableType::String,
                )
                .set_default_value(Some(json!("text")));
            }
        } else {
            node.error = Some("Datasource Not Implemented".to_string());
        }
    }
}
