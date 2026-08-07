//! Compatibility matrix for real-world NER models against the `onnx_ner` node.
//!
//! Every case downloads the actual HuggingFace ONNX export into
//! `tests/models/ner/<repo-slug>/` and drives it through `infer_ner`, the same entry point the
//! node uses. The point is not only to prove that supported models work, but that unsupported
//! ones fail with a descriptive error instead of silently returning an empty result.
//!
//! ```sh
//! cargo test -p flow-like-catalog-onnx --test ner_models --features execute -- --ignored --nocapture
//! ```
//!
//! Downloads are cached; delete `tests/models/ner/` to reclaim the disk space.

use flow_like_catalog_onnx::load::external_data_candidates;
use flow_like_catalog_onnx::ner::{NerOptions, NerResult, infer_ner};
use flow_like_model_provider::ml::ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::ValueType,
};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

const NER_TEXT: &str = "Angela Merkel visited Microsoft headquarters in Redmond, Washington, on 3 March 2021 and later met Satya Nadella, who grew up in Hyderabad, India.";

const PII_TEXT: &str = "Please contact Dr. Sarah Mueller at sarah.mueller@example.com or +49 30 1234567. Her IBAN is DE89370400440532013000 and she lives in Friedrichstrasse 12, 10117 Berlin.";

/// How a model is expected to be driven.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    /// Plain token classification: `input_ids` in, `[batch, seq, labels]` out.
    TokenClassification,
    /// GLiNER v1 single graph: needs prompt-encoded labels plus span inputs.
    Gliner,
    /// GLiNER2: a pipeline of separate graphs.
    Gliner2,
}

struct Case {
    repo: &'static str,
    variant: &'static str,
    /// HF repo-relative path of the graph.
    model: &'static str,
    /// HF repo-relative paths that must sit next to the graph (external tensor data).
    sidecars: &'static [&'static str],
    /// HF repo-relative path of `tokenizer.json`, if the repo ships one.
    tokenizer: Option<&'static str>,
    config: Option<&'static str>,
    family: Family,
}

const CASES: &[Case] = &[
    // ---- fixed-label token classification ------------------------------------------------
    Case {
        repo: "onnx-community/distilbert-NER-ONNX",
        variant: "fp32",
        model: "onnx/model.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/distilbert-NER-ONNX",
        variant: "fp16",
        model: "onnx/model_fp16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/distilbert-NER-ONNX",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/distilbert-NER-ONNX",
        variant: "q4f16",
        model: "onnx/model_q4f16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/bert-base-multilingual-cased-ner-hrl-ONNX",
        variant: "fp32",
        model: "onnx/model.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/bert-base-multilingual-cased-ner-hrl-ONNX",
        variant: "fp16",
        model: "onnx/model_fp16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/bert-base-multilingual-cased-ner-hrl-ONNX",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/bert-base-multilingual-cased-ner-hrl-ONNX",
        variant: "q4f16",
        model: "onnx/model_q4f16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "jdp8/wikineural-multilingual-ner",
        variant: "fp32",
        model: "onnx/model.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "jdp8/wikineural-multilingual-ner",
        variant: "fp16",
        model: "onnx/model_fp16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "jdp8/wikineural-multilingual-ner",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "jdp8/wikineural-multilingual-ner",
        variant: "q4f16",
        model: "onnx/model_q4f16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/modernbert-ner-conll2003-ONNX",
        variant: "fp32",
        model: "onnx/model.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/modernbert-ner-conll2003-ONNX",
        variant: "fp16",
        model: "onnx/model_fp16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/modernbert-ner-conll2003-ONNX",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/modernbert-ner-conll2003-ONNX",
        variant: "q4f16",
        model: "onnx/model_q4f16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/xlm-roberta-large-finetuned-conll03-english-ONNX",
        variant: "fp32",
        model: "onnx/model.onnx",
        sidecars: &["onnx/model.onnx_data"],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/xlm-roberta-large-finetuned-conll03-english-ONNX",
        variant: "fp16",
        model: "onnx/model_fp16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/xlm-roberta-large-finetuned-conll03-english-ONNX",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/xlm-roberta-large-finetuned-conll03-english-ONNX",
        variant: "q4f16",
        model: "onnx/model_q4f16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "Jean-Baptiste/roberta-large-ner-english",
        variant: "fp32",
        model: "model.onnx",
        sidecars: &[],
        tokenizer: None,
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    // ---- PII ------------------------------------------------------------------------------
    Case {
        repo: "onnx-community/multilang-pii-ner-ONNX",
        variant: "fp32",
        model: "onnx/model.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/multilang-pii-ner-ONNX",
        variant: "fp16",
        model: "onnx/model_fp16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/multilang-pii-ner-ONNX",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/multilang-pii-ner-ONNX",
        variant: "q4f16",
        model: "onnx/model_q4f16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/pii-ner-nemotron-ONNX",
        variant: "fp32",
        model: "onnx/model.onnx",
        sidecars: &["onnx/model.onnx_data"],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/pii-ner-nemotron-ONNX",
        variant: "fp16",
        model: "onnx/model_fp16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/pii-ner-nemotron-ONNX",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/pii-ner-nemotron-ONNX",
        variant: "q4f16",
        model: "onnx/model_q4f16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "rulesentry-io/ettin-32m-nemotron-pii-onnx",
        variant: "fp32",
        model: "model.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    // ---- domain-specific fixed-label ------------------------------------------------------
    Case {
        repo: "onnx-community/NeuroBERT-NER-ONNX",
        variant: "fp32",
        model: "onnx/model.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/NeuroBERT-NER-ONNX",
        variant: "fp16",
        model: "onnx/model_fp16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/NeuroBERT-NER-ONNX",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    Case {
        repo: "onnx-community/NeuroBERT-NER-ONNX",
        variant: "q4f16",
        model: "onnx/model_q4f16.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::TokenClassification,
    },
    // ---- GLiNER v1 zero-shot --------------------------------------------------------------
    Case {
        repo: "onnx-community/gliner_small-v2.1",
        variant: "fp32",
        model: "onnx/model.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner,
    },
    Case {
        repo: "onnx-community/gliner_small-v2.1",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner,
    },
    Case {
        repo: "onnx-community/gliner_medium_news-v2.1",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner,
    },
    Case {
        repo: "onnx-community/gliner_multi-v2.1",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner,
    },
    Case {
        repo: "onnx-community/gliner_large-v2.1",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner,
    },
    Case {
        repo: "onnx-community/gliner_large_bio-v0.1",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner,
    },
    Case {
        repo: "onnx-community/gliner_multi_pii-v1",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner,
    },
    Case {
        repo: "onnx-community/NuNER_Zero",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner,
    },
    Case {
        repo: "onnx-community/NuNER_Zero-span",
        variant: "int8",
        model: "onnx/model_int8.onnx",
        sidecars: &[],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner,
    },
    // ---- GLiNER2 multi-graph pipelines ----------------------------------------------------
    Case {
        repo: "lmo3/gliner2-multi-v1-onnx",
        variant: "fp32",
        model: "onnx/encoder.onnx",
        sidecars: &["onnx/encoder.onnx.data"],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner2,
    },
    Case {
        repo: "lmo3/gliner2-large-v1-onnx",
        variant: "fp32",
        model: "onnx/encoder.onnx",
        sidecars: &["onnx/encoder.onnx.data"],
        tokenizer: Some("tokenizer.json"),
        config: Some("config.json"),
        family: Family::Gliner2,
    },
    Case {
        repo: "SemplificaAI/gliner2-privacy-filter-PII-multi",
        variant: "fp32",
        model: "fp32_v2/encoder_fp32.onnx",
        sidecars: &[],
        tokenizer: Some("fp32_v2/tokenizer.json"),
        config: None,
        family: Family::Gliner2,
    },
];

/// Additional graphs probed for the multi-graph pipelines, to show what a full implementation
/// would have to orchestrate.
const PIPELINE_GRAPHS: &[(&str, &[&str])] = &[
    (
        "lmo3/gliner2-multi-v1-onnx",
        &[
            "onnx/encoder.onnx",
            "onnx/span_rep.onnx",
            "onnx/classifier.onnx",
            "onnx/count_embed.onnx",
        ],
    ),
    (
        "lmo3/gliner2-large-v1-onnx",
        &[
            "onnx/encoder.onnx",
            "onnx/span_rep.onnx",
            "onnx/classifier.onnx",
            "onnx/count_embed.onnx",
        ],
    ),
    (
        "SemplificaAI/gliner2-privacy-filter-PII-multi",
        &[
            "fp32_v2/encoder_fp32.onnx",
            "fp32_v2/span_rep_fp32.onnx",
            "fp32_v2/classifier_fp32.onnx",
            "fp32_v2/scorer_fp32.onnx",
            "fp32_v2/token_gather_fp32.onnx",
            "fp32_v2/schema_gather_fp32.onnx",
            "fp32_v2/count_lstm_fixed_fp32.onnx",
            "fp32_v2/count_pred_argmax_fp32.onnx",
        ],
    ),
];

#[derive(PartialEq, Eq, Clone, Copy)]
enum Status {
    Works,
    RejectedCleanly,
    SilentlyWrong,
    Unavailable,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Works => "WORKS",
            Status::RejectedCleanly => "rejected (clean error)",
            Status::SilentlyWrong => "SILENT FAILURE",
            Status::Unavailable => "unavailable",
        }
    }
}

impl Family {
    fn label(self) -> &'static str {
        match self {
            Family::TokenClassification => "token-classification",
            Family::Gliner => "gliner",
            Family::Gliner2 => "gliner2",
        }
    }
}

struct Row {
    model: String,
    variant: String,
    family: Family,
    size_mb: f64,
    inputs: String,
    output: String,
    labels: String,
    entities: String,
    status: Status,
    detail: String,
}

fn models_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("models")
        .join("ner")
}

fn slug(repo: &str) -> String {
    repo.replace('/', "__")
}

/// Cache path for an HF repo file. Only the basename is kept so external tensor data keeps the
/// exact filename the graph references.
fn cached_path(repo: &str, hf_path: &str) -> PathBuf {
    let name = hf_path.rsplit('/').next().unwrap_or(hf_path);
    models_root().join(slug(repo)).join(name)
}

fn fetch(repo: &str, hf_path: &str) -> Result<PathBuf, String> {
    let path = cached_path(repo, hf_path);
    if path.exists() && fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 0 {
        return Ok(path);
    }

    fs::create_dir_all(path.parent().expect("cache dir"))
        .map_err(|e| format!("create cache dir: {e}"))?;

    let url = format!("https://huggingface.co/{repo}/resolve/main/{hf_path}");
    println!("  downloading {url}");
    let response = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(60 * 30))
        .build()
        .map_err(|e| format!("http client: {e}"))?
        .get(&url)
        .send()
        .map_err(|e| format!("GET {url}: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("GET {url}: HTTP {}", response.status()));
    }

    let bytes = response.bytes().map_err(|e| format!("read body: {e}"))?;
    let mut file = fs::File::create(&path).map_err(|e| format!("create file: {e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("write file: {e}"))?;
    Ok(path)
}

/// Read `id2label` out of a HuggingFace `config.json`, ordered by class index.
fn labels_from_config(path: &Path) -> Option<Vec<String>> {
    let raw = fs::read_to_string(path).ok()?;
    let config: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let id2label = config.get("id2label")?.as_object()?;

    let mut ordered: BTreeMap<usize, String> = BTreeMap::new();
    for (key, value) in id2label {
        ordered.insert(key.parse::<usize>().ok()?, value.as_str()?.to_string());
    }

    if ordered.is_empty() {
        return None;
    }
    if ordered.keys().copied().ne(0..ordered.len()) {
        return None;
    }
    Some(ordered.into_values().collect())
}

fn describe_type(value_type: &ValueType) -> String {
    match value_type {
        ValueType::Tensor { ty, shape, .. } => format!("{ty:?}{shape}"),
        other => format!("{other:?}"),
    }
}

fn describe_inputs(session: &Session) -> String {
    session
        .inputs()
        .iter()
        .map(|input| input.name().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn describe_primary_output(session: &Session) -> String {
    session
        .outputs()
        .first()
        .map(|output| format!("{} {}", output.name(), describe_type(output.dtype())))
        .unwrap_or_else(|| "<none>".to_string())
}

/// Load the way the product does: `load_onnx` reads the FlowPath bytes, commits from memory, and
/// on failure retries with any external tensor sidecar registered in memory.
fn load_like_the_node(path: &Path) -> Result<Session, String> {
    let bytes = fs::read(path).map_err(|e| format!("read model: {e}"))?;
    let direct = flow_like_model_provider::ml::ort_runtime::configured_session_builder()
        .map_err(|e| format!("session builder: {e}"))?
        .commit_from_memory(&bytes);
    let direct_error = match direct {
        Ok(session) => return Ok(session),
        Err(error) => error,
    };

    // Same ladder as `load_onnx`: fp16/q4f16 exports trip ORT's level-3 fusions.
    let reduced = flow_like_model_provider::ml::ort_runtime::configured_session_builder()
        .map_err(|e| format!("session builder: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level2)
        .map_err(|e| format!("optimization level: {e}"))?
        .commit_from_memory(&bytes);
    if let Ok(session) = reduced {
        return Ok(session);
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let sidecars: Vec<(String, Vec<u8>)> = external_data_candidates(file_name)
        .into_iter()
        .filter_map(|candidate| {
            let sibling = path.with_file_name(&candidate);
            fs::read(&sibling)
                .ok()
                .filter(|bytes| !bytes.is_empty())
                .map(|bytes| (candidate, bytes))
        })
        .collect();

    if sidecars.is_empty() {
        return Err(format!("commit_from_memory: {direct_error}"));
    }

    let mut builder = flow_like_model_provider::ml::ort_runtime::configured_session_builder()
        .map_err(|e| format!("session builder: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level2)
        .map_err(|e| format!("optimization level: {e}"))?;
    for (name, data) in sidecars {
        builder = builder
            .with_external_initializer_file_in_memory(&name, std::borrow::Cow::Owned(data))
            .map_err(|e| format!("register external data `{name}`: {e}"))?;
    }
    builder
        .commit_from_memory(&bytes)
        .map_err(|e| format!("commit_from_memory with external data: {e}"))
}

/// Loading straight from disk resolves external tensor data, which `commit_from_memory` cannot.
fn load_from_file(path: &Path) -> Result<Session, String> {
    flow_like_model_provider::ml::ort_runtime::configured_session_builder()
        .map_err(|e| format!("session builder: {e}"))?
        .commit_from_file(path)
        .map_err(|e| format!("commit_from_file: {e}"))
}

fn summarize_entities(result: &NerResult, limit: usize) -> String {
    if result.entities.is_empty() {
        return "<none>".to_string();
    }
    let mut parts: Vec<String> = result
        .entities
        .iter()
        .take(limit)
        .map(|entity| {
            format!(
                "{}={} ({:.2})",
                entity.entity_type, entity.text, entity.confidence
            )
        })
        .collect();
    if result.entities.len() > limit {
        parts.push(format!("… +{}", result.entities.len() - limit));
    }
    parts.join(", ")
}

/// Verify the decoded struct is internally consistent, not just non-empty.
fn validate_result(result: &NerResult, source: &str) -> Result<(), String> {
    if result.text != source {
        return Err("result.text does not echo the input".to_string());
    }
    if result.tokens.is_empty() {
        return Err("no token predictions were produced".to_string());
    }
    for token in &result.tokens {
        if !token.confidence.is_finite() || !(0.0..=1.0).contains(&token.confidence) {
            return Err(format!(
                "token `{}` has out-of-range confidence {}",
                token.token, token.confidence
            ));
        }
        if token.label.is_empty() {
            return Err(format!("token `{}` has an empty label", token.token));
        }
    }
    for entity in &result.entities {
        if entity.start_char >= entity.end_char {
            return Err(format!(
                "entity `{}` has an empty span {}..{}",
                entity.text, entity.start_char, entity.end_char
            ));
        }
        if entity.end_char > source.len() {
            return Err(format!(
                "entity `{}` ends at {} beyond the {}-byte input",
                entity.text,
                entity.end_char,
                source.len()
            ));
        }
        if !source.is_char_boundary(entity.start_char) || !source.is_char_boundary(entity.end_char)
        {
            return Err(format!(
                "entity `{}` span {}..{} is not on a char boundary",
                entity.text, entity.start_char, entity.end_char
            ));
        }
        if source[entity.start_char..entity.end_char] != entity.text {
            return Err(format!(
                "entity text `{}` does not match the source span `{}`",
                entity.text,
                &source[entity.start_char..entity.end_char]
            ));
        }
        if entity.entity_type.is_empty() {
            return Err(format!("entity `{}` has an empty type", entity.text));
        }
        if !entity.confidence.is_finite() {
            return Err(format!("entity `{}` has a non-finite score", entity.text));
        }
    }
    Ok(())
}

fn run_case(case: &Case) -> Row {
    let mut row = Row {
        model: case.repo.to_string(),
        variant: case.variant.to_string(),
        family: case.family,
        size_mb: 0.0,
        inputs: "-".to_string(),
        output: "-".to_string(),
        labels: "-".to_string(),
        entities: "-".to_string(),
        status: Status::Unavailable,
        detail: String::new(),
    };

    println!("\n=== {} [{}] ===", case.repo, case.variant);

    let model_path = match fetch(case.repo, case.model) {
        Ok(path) => path,
        Err(error) => {
            row.detail = format!("download failed: {error}");
            return row;
        }
    };
    row.size_mb = fs::metadata(&model_path).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;

    for sidecar in case.sidecars {
        match fetch(case.repo, sidecar) {
            Ok(path) => {
                row.size_mb += fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as f64 / 1e6;
            }
            Err(error) => {
                row.detail = format!("download failed: {error}");
                return row;
            }
        }
    }

    let Some(tokenizer_path) = case.tokenizer else {
        row.status = Status::RejectedCleanly;
        row.detail =
            "repo ships no tokenizer.json (vocab.json + merges.txt only); the node requires one"
                .to_string();
        println!("  {}", row.detail);
        return row;
    };

    let tokenizer_file = match fetch(case.repo, tokenizer_path) {
        Ok(path) => path,
        Err(error) => {
            row.detail = format!("tokenizer download failed: {error}");
            return row;
        }
    };
    let tokenizer = match Tokenizer::from_file(&tokenizer_file) {
        Ok(tokenizer) => tokenizer,
        Err(error) => {
            row.status = Status::RejectedCleanly;
            row.detail = format!("tokenizer.json rejected: {error}");
            return row;
        }
    };

    let labels = case
        .config
        .and_then(|config| fetch(case.repo, config).ok())
        .and_then(|path| labels_from_config(&path));
    row.labels = match &labels {
        Some(labels) => format!("{} from config.json", labels.len()),
        None => "none in config.json".to_string(),
    };

    let mut session = match load_like_the_node(&model_path) {
        Ok(session) => session,
        Err(memory_error) => match load_from_file(&model_path) {
            Ok(session) => {
                row.detail = format!(
                    "load_onnx path (commit_from_memory) FAILS, commit_from_file works: {memory_error}"
                );
                println!("  {}", row.detail);
                session
            }
            Err(file_error) => {
                row.status = Status::RejectedCleanly;
                row.detail = format!("model does not load at all: {file_error}");
                println!("  {}", row.detail);
                return row;
            }
        },
    };

    row.inputs = describe_inputs(&session);
    row.output = describe_primary_output(&session);
    println!("  inputs : {}", row.inputs);
    println!("  output : {}", row.output);
    println!("  labels : {}", row.labels);

    let options = NerOptions {
        labels: labels.clone().unwrap_or_default(),
        threshold: 0.5,
        max_length: 512,
    };

    let mut summaries = Vec::new();
    let mut first_error: Option<String> = None;

    for (name, text) in [("general", NER_TEXT), ("pii", PII_TEXT)] {
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
            infer_ner(&mut session, &tokenizer, text, &options)
        }));

        match outcome {
            Ok(Ok(result)) => {
                if let Err(problem) = validate_result(&result, text) {
                    row.status = Status::SilentlyWrong;
                    row.detail = format!("malformed struct on `{name}` text: {problem}");
                    println!("  {}", row.detail);
                    return row;
                }
                let summary = summarize_entities(&result, 8);
                println!("  {name:<7}: {summary}");
                summaries.push(format!("[{name}] {summary}"));
            }
            Ok(Err(error)) => {
                let message = error.to_string();
                println!("  {name:<7}: ERROR {message}");
                first_error.get_or_insert(message);
            }
            Err(_) => {
                let message = "inference panicked".to_string();
                println!("  {name:<7}: {message}");
                first_error.get_or_insert(message);
            }
        }
    }

    if let Some(error) = first_error {
        row.status = Status::RejectedCleanly;
        if row.detail.is_empty() {
            row.detail = error;
        } else {
            row.detail = format!("{}; {error}", row.detail);
        }
        return row;
    }

    let found_any = summaries.iter().any(|s| !s.contains("<none>"));
    row.entities = summaries.join("  ");
    row.status = if found_any {
        Status::Works
    } else {
        Status::SilentlyWrong
    };
    if !found_any && row.detail.is_empty() {
        row.detail = "loaded and ran but produced zero entities on both probes".to_string();
    }
    row
}

/// Report the graph inventory of a multi-graph pipeline; the node drives exactly one graph.
fn probe_pipeline(repo: &str, graphs: &[&str]) {
    println!("\n=== pipeline inventory: {repo} ===");
    for graph in graphs {
        let Ok(path) = fetch(repo, graph) else {
            println!("  {graph}: <download failed>");
            continue;
        };
        for sidecar in [format!("{graph}.data"), format!("{graph}_data")] {
            let _ = fetch(repo, &sidecar);
        }
        match load_from_file(&path) {
            Ok(session) => {
                println!(
                    "  {graph}\n     in : {}\n     out: {}",
                    describe_inputs(&session),
                    session
                        .outputs()
                        .iter()
                        .map(|o| format!("{} {}", o.name(), describe_type(o.dtype())))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            Err(error) => println!("  {graph}: load failed: {error}"),
        }
    }
}

#[test]
#[ignore = "downloads several GB of ONNX models"]
fn ner_model_compatibility_matrix() {
    let ep = flow_like_model_provider::ml::ort_runtime::initialize_ort();
    println!(
        "ONNX Runtime providers: {:?} (accelerated: {})",
        ep.active_providers, ep.accelerated
    );
    println!("model cache: {}", models_root().display());

    let mut rows = Vec::new();
    for case in CASES {
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| run_case(case)));
        rows.push(outcome.unwrap_or_else(|_| Row {
            model: case.repo.to_string(),
            variant: case.variant.to_string(),
            family: case.family,
            size_mb: 0.0,
            inputs: "-".to_string(),
            output: "-".to_string(),
            labels: "-".to_string(),
            entities: "-".to_string(),
            status: Status::SilentlyWrong,
            detail: "harness panicked while driving this model".to_string(),
        }));
    }

    for (repo, graphs) in PIPELINE_GRAPHS {
        probe_pipeline(repo, graphs);
    }

    println!("\n\n## NER model compatibility\n");
    println!("| Model | Variant | Kind | Size | Status | Labels | Detail |");
    println!("| --- | --- | --- | ---: | --- | --- | --- |");
    for row in &rows {
        println!(
            "| {} | {} | {} | {:.0} MB | {} | {} | {} |",
            row.model,
            row.variant,
            row.family.label(),
            row.size_mb,
            row.status.label(),
            row.labels,
            match (row.detail.is_empty(), row.entities.as_str()) {
                (true, _) => row.entities.clone(),
                (false, "-") => row.detail.clone(),
                (false, entities) => format!("{} || {}", row.detail, entities),
            }
        );
    }

    println!("\n## Graph signatures\n");
    for row in &rows {
        if row.inputs != "-" {
            println!(
                "- {} [{}]\n    in : {}\n    out: {}",
                row.model, row.variant, row.inputs, row.output
            );
        }
    }

    println!("\n## Entity output\n");
    for row in &rows {
        if row.status == Status::Works {
            println!("- {} [{}]\n    {}", row.model, row.variant, row.entities);
        }
    }

    let silent: Vec<&Row> = rows
        .iter()
        .filter(|row| row.status == Status::SilentlyWrong)
        .collect();
    assert!(
        silent.is_empty(),
        "these models neither worked nor produced a descriptive error: {:?}",
        silent
            .iter()
            .map(|row| format!("{} [{}]: {}", row.model, row.variant, row.detail))
            .collect::<Vec<_>>()
    );

    let working = rows
        .iter()
        .filter(|row| row.status == Status::Works)
        .count();
    assert!(
        working > 0,
        "no NER model in the matrix produced entities; the node is broken"
    );
    println!(
        "\n{working}/{} model+variant combinations produce entities.",
        rows.len()
    );
}

/// The fp16/q4f16 exports all die inside ORT's graph optimizer. Check whether lowering the
/// optimization level is a viable workaround before recommending one.
#[test]
#[ignore = "requires the cached fp16 models"]
fn fp16_optimization_level_workaround() {
    use flow_like_model_provider::ml::ort::session::builder::GraphOptimizationLevel;

    let candidates = [
        ("onnx-community/distilbert-NER-ONNX", "onnx/model_fp16.onnx"),
        ("onnx-community/NeuroBERT-NER-ONNX", "onnx/model_fp16.onnx"),
        ("onnx-community/NeuroBERT-NER-ONNX", "onnx/model_q4f16.onnx"),
        (
            "onnx-community/modernbert-ner-conll2003-ONNX",
            "onnx/model_fp16.onnx",
        ),
    ];

    for (repo, model) in candidates {
        let Ok(path) = fetch(repo, model) else {
            println!("{repo} {model}: not cached, skipping");
            continue;
        };
        let Ok(bytes) = fs::read(&path) else { continue };

        for (name, level) in [
            ("Disable", GraphOptimizationLevel::Disable),
            ("Level1", GraphOptimizationLevel::Level1),
            ("Level2", GraphOptimizationLevel::Level2),
        ] {
            let outcome = flow_like_model_provider::ml::ort_runtime::configured_session_builder()
                .map_err(|e| e.to_string())
                .and_then(|builder| {
                    builder
                        .with_optimization_level(level)
                        .map_err(|e| e.to_string())
                })
                .and_then(|mut builder| {
                    builder
                        .commit_from_memory(&bytes)
                        .map_err(|e| e.to_string())
                });

            match outcome {
                Ok(session) => println!(
                    "{repo} {model} [{name}]: LOADS — output {}",
                    describe_primary_output(&session)
                ),
                Err(error) => {
                    let first_line = error.lines().next().unwrap_or(&error);
                    println!("{repo} {model} [{name}]: fails — {first_line}");
                }
            }
        }
    }
}

/// Print the full GLiNER tensor contract so the node can be built against facts, not guesses.
#[test]
#[ignore = "requires the cached gliner models"]
fn gliner_graph_contract() {
    for (repo, model) in [
        ("onnx-community/gliner_small-v2.1", "onnx/model_int8.onnx"),
        ("onnx-community/NuNER_Zero", "onnx/model_int8.onnx"),
    ] {
        let Ok(path) = fetch(repo, model) else {
            continue;
        };
        let Ok(session) = load_from_file(&path) else {
            continue;
        };
        println!("\n=== {repo} ===");
        for input in session.inputs() {
            println!(
                "  IN  {:<16} {}",
                input.name(),
                describe_type(input.dtype())
            );
        }
        for output in session.outputs() {
            println!(
                "  OUT {:<16} {}",
                output.name(),
                describe_type(output.dtype())
            );
        }
        if let Ok(config) = fetch(repo, "gliner_config.json")
            && let Ok(raw) = fs::read_to_string(&config)
        {
            let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap_or_default();
            println!(
                "  config: max_width={:?} span_mode={:?} ent={:?} sep={:?} max_len={:?}",
                parsed.get("max_width"),
                parsed.get("span_mode"),
                parsed.get("ent_token"),
                parsed.get("sep_token"),
                parsed.get("max_len")
            );
        }
    }
}

/// Drive every cached GLiNER export through the zero-shot node.
#[test]
#[ignore = "requires the cached gliner models"]
fn gliner_zero_shot_matrix() {
    use flow_like_catalog_onnx::gliner::{GlinerOptions, infer_gliner};

    let cases: &[(&str, &str, usize)] = &[
        ("onnx-community/gliner_small-v2.1", "onnx/model.onnx", 12),
        (
            "onnx-community/gliner_small-v2.1",
            "onnx/model_int8.onnx",
            12,
        ),
        // int8 is broken in this repo: scores collapse to ~0.15 and labels scramble.
        (
            "onnx-community/gliner_medium_news-v2.1",
            "onnx/model.onnx",
            12,
        ),
        (
            "onnx-community/gliner_multi-v2.1",
            "onnx/model_int8.onnx",
            12,
        ),
        (
            "onnx-community/gliner_large-v2.1",
            "onnx/model_int8.onnx",
            12,
        ),
        (
            "onnx-community/gliner_large_bio-v0.1",
            "onnx/model_int8.onnx",
            12,
        ),
        (
            "onnx-community/gliner_multi_pii-v1",
            "onnx/model_int8.onnx",
            12,
        ),
        ("onnx-community/NuNER_Zero", "onnx/model_int8.onnx", 1),
        ("onnx-community/NuNER_Zero-span", "onnx/model_int8.onnx", 12),
    ];

    let probes: &[(&str, &str, &[&str])] = &[
        (
            "general",
            NER_TEXT,
            &["person", "company", "city", "country", "date"],
        ),
        (
            "pii",
            PII_TEXT,
            &["person", "email address", "phone number", "iban", "address"],
        ),
    ];

    let mut worked = 0usize;
    let mut failures = Vec::new();

    for (repo, model, max_width) in cases {
        println!("\n=== {repo} [{model}] ===");
        let (Ok(model_path), Ok(tokenizer_path)) =
            (fetch(repo, model), fetch(repo, "tokenizer.json"))
        else {
            println!("  not cached, skipping");
            continue;
        };

        let mut session = match load_like_the_node(&model_path) {
            Ok(session) => session,
            Err(error) => {
                println!("  load failed: {error}");
                failures.push(format!("{repo}: load failed: {error}"));
                continue;
            }
        };
        let tokenizer = match Tokenizer::from_file(&tokenizer_path) {
            Ok(tokenizer) => tokenizer,
            Err(error) => {
                println!("  tokenizer failed: {error}");
                continue;
            }
        };

        let mut any_entities = false;
        for (name, text, labels) in probes {
            let options = GlinerOptions {
                labels: labels.iter().map(|label| label.to_string()).collect(),
                threshold: 0.5,
                max_width: *max_width,
                ..Default::default()
            };

            match infer_gliner(&mut session, &tokenizer, text, &options) {
                Ok(result) => {
                    if let Err(problem) = validate_gliner(&result, text) {
                        println!("  {name:<7}: MALFORMED {problem}");
                        failures.push(format!("{repo} [{name}]: {problem}"));
                        continue;
                    }
                    any_entities |= !result.entities.is_empty();
                    let rendered: Vec<String> = result
                        .entities
                        .iter()
                        .map(|entity| {
                            format!("{}={} ({:.2})", entity.label, entity.text, entity.score)
                        })
                        .collect();
                    println!(
                        "  {name:<7}: {}",
                        if rendered.is_empty() {
                            "<none>".to_string()
                        } else {
                            rendered.join(", ")
                        }
                    );
                }
                Err(error) => {
                    println!("  {name:<7}: ERROR {error}");
                    failures.push(format!("{repo} [{name}]: {error}"));
                }
            }
        }

        if any_entities {
            worked += 1;
        } else {
            failures.push(format!("{repo}: produced no entities on either probe"));
        }
    }

    println!(
        "\n{worked}/{} GLiNER models extracted entities.",
        cases.len()
    );
    assert!(
        failures.is_empty(),
        "GLiNER problems: {}",
        failures.join(" | ")
    );
}

/// The struct has to describe the input it came from, not merely be non-empty.
fn validate_gliner(
    result: &flow_like_catalog_onnx::gliner::GlinerResult,
    source: &str,
) -> Result<(), String> {
    if result.text != source {
        return Err("result.text does not echo the input".to_string());
    }
    for entity in &result.entities {
        if entity.start_char >= entity.end_char || entity.end_char > source.len() {
            return Err(format!(
                "entity `{}` has span {}..{} outside the {}-byte input",
                entity.text,
                entity.start_char,
                entity.end_char,
                source.len()
            ));
        }
        if source[entity.start_char..entity.end_char] != entity.text {
            return Err(format!(
                "entity text `{}` does not match source span `{}`",
                entity.text,
                &source[entity.start_char..entity.end_char]
            ));
        }
        if !(0.0..=1.0).contains(&entity.score) {
            return Err(format!(
                "entity `{}` has score {} outside 0..1",
                entity.text, entity.score
            ));
        }
        if !result.labels.contains(&entity.label) {
            return Err(format!(
                "entity `{}` carries label `{}` which was never requested",
                entity.text, entity.label
            ));
        }
    }
    Ok(())
}

/// Why does gliner_medium_news return nothing? Look at the raw score distribution.
#[test]
#[ignore = "diagnostic"]
fn gliner_medium_news_diagnostic() {
    use flow_like_catalog_onnx::gliner::{GlinerOptions, infer_gliner};

    let repo = "onnx-community/gliner_medium_news-v2.1";
    for model in ["onnx/model_int8.onnx", "onnx/model.onnx"] {
        let (Ok(model_path), Ok(tokenizer_path)) =
            (fetch(repo, model), fetch(repo, "tokenizer.json"))
        else {
            println!("{model}: not cached");
            continue;
        };
        let Ok(mut session) = load_like_the_node(&model_path) else {
            println!("{model}: load failed");
            continue;
        };
        let Ok(tokenizer) = Tokenizer::from_file(&tokenizer_path) else {
            continue;
        };

        for threshold in [0.5f32, 0.2, 0.05, 0.0] {
            let options = GlinerOptions {
                labels: vec![
                    "person".to_string(),
                    "company".to_string(),
                    "city".to_string(),
                ],
                threshold,
                max_width: 12,
                ..Default::default()
            };
            match infer_gliner(&mut session, &tokenizer, NER_TEXT, &options) {
                Ok(result) => {
                    let top: Vec<String> = result
                        .entities
                        .iter()
                        .take(6)
                        .map(|e| format!("{}={} ({:.3})", e.label, e.text, e.score))
                        .collect();
                    println!(
                        "{model} @{threshold}: {} entities  {}",
                        result.entities.len(),
                        top.join(", ")
                    );
                }
                Err(error) => println!("{model} @{threshold}: ERROR {error}"),
            }
        }
    }
}
