#![cfg(feature = "local-ml")]

use anyhow::{Context, Result, ensure};
use flow_like_model_provider::{
    embedding::native::{
        NativeImageEmbedding, NativeTextEmbedding, Pooling, SessionOptions, TokenizedBatch,
        TokenizerFiles,
    },
    tokenizer::load_tokenizer_from_file,
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use text_splitter::{ChunkConfig, ChunkSizer, TextSplitter};

const MAX_COSINE_DISTANCE: f64 = 1e-5;
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/embedding_parity"
);

#[derive(Deserialize)]
struct Manifest {
    schema_version: u32,
    inputs: Vec<String>,
    image_fixtures: Vec<(u32, u32, u32)>,
    models: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    bit: Value,
    status: String,
    #[serde(default)]
    artifacts: Vec<Artifact>,
    vector_file: Option<String>,
    max_tokens: Option<usize>,
    #[serde(default)]
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Artifact {
    role: String,
    hash: String,
    size: u64,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    rows: usize,
    dimensions: usize,
    offset_floats: usize,
    chunk_counts: Option<Vec<usize>>,
    batch_size: Option<usize>,
    max_tokens: Option<usize>,
}

fn manifest() -> Manifest {
    serde_json::from_slice(&std::fs::read(Path::new(FIXTURES).join("manifest.json")).unwrap())
        .unwrap()
}

#[test]
fn published_embedding_golden_vectors_are_complete_and_finite() {
    let manifest = manifest();
    assert_eq!(manifest.schema_version, 1);
    assert!(!manifest.inputs.is_empty());
    assert!(!manifest.models.is_empty());
    for model in manifest.models {
        if model.status == "remote_only" {
            continue;
        }
        assert_eq!(
            model.status, "captured",
            "Missing numeric reference for {}",
            model.bit["id"]
        );
        let values = read_vectors(&model).unwrap();
        assert!(!model.cases.is_empty());
        let mut expected = 0;
        for case in &model.cases {
            assert_eq!(case.offset_floats, expected);
            assert!(case.dimensions > 0 && case.rows > 0);
            expected += case.dimensions * case.rows;
        }
        assert_eq!(values.len(), expected);
        assert!(values.iter().all(|value| value.is_finite()));
    }
}

/// Run with FLOW_LIKE_EMBEDDING_MODELS pointing at directories named by published Bit ID.
/// Golden vectors were captured with the frozen application wrapper and FastEmbed 5.17.2.
#[test]
#[ignore = "Requires downloaded published models in FLOW_LIKE_EMBEDDING_MODELS"]
fn every_published_local_model_matches_frozen_pipeline_under_cosine_distance() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("FLOW_LIKE_EMBEDDING_MODELS")
            .context("Set FLOW_LIKE_EMBEDDING_MODELS to the published model asset directories")?,
    );
    let manifest = manifest();
    let mut checked_models = 0;
    let mut checked_vectors = 0;
    let mut optimized_text_models = 0;
    let mut optimized_text_batches = 0;
    let mut truncating_text_batches = 0;
    let mut borrowed_image_models = 0;
    let mut borrowed_image_batches = 0;
    let mut maximum: f64 = 0.0;
    for model in &manifest.models {
        if model.status == "remote_only" {
            continue;
        }
        ensure!(
            model.status == "captured",
            "Published model {} has no numeric reference: {}",
            model.bit["id"],
            model.status
        );
        let id = model.bit["id"].as_str().context("Bit ID")?;
        let directory = root.join(id);
        verify_artifacts(&directory, &model.artifacts).with_context(|| format!("Model {id}"))?;
        let golden = read_vectors(model)?;
        if model.bit["type"] == "ImageEmbedding" {
            let mut encoder = NativeImageEmbedding::new_from_file(
                directory.join("model.onnx"),
                std::fs::read(directory.join("preprocessor_config.json"))?,
                SessionOptions::default(),
            )?;
            for case in &model.cases {
                let images = manifest
                    .image_fixtures
                    .iter()
                    .map(|&(width, height, seed)| {
                        image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(
                            width,
                            height,
                            |x, y| {
                                image::Rgb([
                                    ((x * 3 + y + seed * 41) % 256) as u8,
                                    ((x + y * 5 + seed * 23) % 256) as u8,
                                    ((x * 7 + y * 11 + seed * 13) % 256) as u8,
                                ])
                            },
                        ))
                    })
                    .collect::<Vec<_>>();
                let borrowed = images.iter().collect::<Vec<_>>();
                let pixels = encoder
                    .preprocessor()
                    .preprocess_batch_borrowed(&borrowed)?;
                let owned_pixels = encoder.preprocessor().preprocess_batch(images)?;
                ensure!(
                    pixels == owned_pixels,
                    "Borrowed image pixels changed for {id}/{}",
                    case.name
                );
                let actual = encoder.embed_pixels(&pixels)?;
                borrowed_image_batches += 1;
                maximum = maximum.max(compare(id, case, &actual, &golden)?);
                checked_vectors += actual.len();
            }
            borrowed_image_models += 1;
        } else {
            let mut model_optimized_batches = 0;
            let files = tokenizer_files(&directory)?;
            let pooling = if model.bit["parameters"]["pooling"] == "CLS" {
                Pooling::Cls
            } else {
                Pooling::Mean
            };
            let mut active_limit = model.max_tokens.context("Missing max_tokens")?;
            let mut encoder = NativeTextEmbedding::new_from_file(
                directory.join("model.onnx"),
                files.clone(),
                active_limit,
                pooling,
                SessionOptions::default(),
            )?;
            let query = model.bit["parameters"]["prefix"]["query"]
                .as_str()
                .unwrap_or("");
            let document = model.bit["parameters"]["prefix"]["paragraph"]
                .as_str()
                .unwrap_or("");
            for case in &model.cases {
                let limit = case.max_tokens.unwrap_or(model.max_tokens.unwrap());
                if limit != active_limit {
                    encoder = NativeTextEmbedding::new_from_file(
                        directory.join("model.onnx"),
                        files.clone(),
                        limit,
                        pooling,
                        SessionOptions::default(),
                    )?;
                    active_limit = limit;
                }
                let actual = if let Some(batch_size) = case.batch_size {
                    let texts = manifest
                        .inputs
                        .iter()
                        .map(|text| format!("{query}{text}"))
                        .collect::<Vec<_>>();
                    let preprocessor = encoder.preprocessor();
                    let context = preprocessor
                        .tokenizer()
                        .get_truncation()
                        .context("Published tokenizer has no context limit")?
                        .max_length;
                    let mut tensors = TokenizedBatch::default();
                    let mut vectors = Vec::with_capacity(texts.len());
                    for batch in texts.chunks(batch_size) {
                        let original = encoder.tokenize(batch)?;
                        let full = preprocessor.encode_untruncated(batch)?;
                        if full.iter().all(|encoding| encoding.len() <= context) {
                            preprocessor.pad_encodings_into(full, &mut tensors)?;
                            ensure!(
                                tensors == original,
                                "Reused tokenization changed tensors for {id}/{}",
                                case.name
                            );
                            vectors.extend(encoder.embed_tokens(&tensors)?);
                            optimized_text_batches += 1;
                            model_optimized_batches += 1;
                        } else {
                            // Modern requests reject these full inputs. Preserve the old raw
                            // fixture's truncation and batch composition for its numeric check.
                            vectors.extend(encoder.embed_tokens(&original)?);
                            truncating_text_batches += 1;
                        }
                    }
                    vectors
                } else {
                    let prefix = if case.name.ends_with("document") {
                        document
                    } else {
                        query
                    };
                    let (vectors, chunks) = runtime_embed(
                        &mut encoder,
                        &files,
                        &manifest.inputs,
                        prefix,
                        limit,
                        query.len().max(document.len()),
                    )?;
                    if let Some(expected) = &case.chunk_counts {
                        ensure!(
                            chunks == *expected,
                            "Chunk boundaries changed for {id}/{}",
                            case.name
                        );
                    }
                    vectors
                };
                maximum = maximum.max(compare(id, case, &actual, &golden)?);
                checked_vectors += actual.len();
            }
            ensure!(
                model_optimized_batches > 0,
                "No optimized preprocessing batches checked for {id}"
            );
            optimized_text_models += 1;
            eprintln!("{id}: {model_optimized_batches} optimized text batches matched exactly");
        }
        checked_models += 1;
        eprintln!("{id}: {} cases matched", model.cases.len());
    }
    ensure!(checked_models > 0, "No local embedding models were checked");
    eprintln!(
        "{checked_models} models, {checked_vectors} vectors, max cosine distance {maximum:.3e}"
    );
    eprintln!(
        "Optimized preprocessing: {optimized_text_models} text models / {optimized_text_batches} batches, \
         {truncating_text_batches} legacy truncation batches; \
         {borrowed_image_models} image models / {borrowed_image_batches} borrowed batches"
    );
    Ok(())
}

fn verify_artifacts(directory: &Path, artifacts: &[Artifact]) -> Result<()> {
    use std::io::Read;
    for artifact in artifacts {
        let path = directory.join(&artifact.role);
        let mut file = std::fs::File::open(&path).with_context(|| path.display().to_string())?;
        ensure!(
            file.metadata()?.len() == artifact.size,
            "Artifact size differs: {}",
            path.display()
        );
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        ensure!(
            hasher.finalize().to_hex().as_str() == artifact.hash,
            "Artifact hash differs: {}",
            path.display()
        );
    }
    Ok(())
}

fn read_vectors(model: &Model) -> Result<Vec<f32>> {
    let bytes = std::fs::read(
        Path::new(FIXTURES).join(
            model
                .vector_file
                .as_ref()
                .context("Missing vector fixture")?,
        ),
    )?;
    ensure!(bytes.len() % 4 == 0, "Invalid f32 fixture length");
    Ok(bytes
        .chunks_exact(4)
        .map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap()))
        .collect())
}

fn compare(id: &str, case: &Case, actual: &[Vec<f32>], golden: &[f32]) -> Result<f64> {
    ensure!(
        actual.len() == case.rows,
        "Row count differs for {id}/{}",
        case.name
    );
    let mut maximum: f64 = 0.0;
    for (row, vector) in actual.iter().enumerate() {
        ensure!(
            vector.len() == case.dimensions,
            "Dimensions differ for {id}/{} row {row}",
            case.name
        );
        let start = case.offset_floats + row * case.dimensions;
        let reference = &golden[start..start + case.dimensions];
        let mut dot = 0.0;
        let mut left = 0.0;
        let mut right = 0.0;
        for (&value, &expected) in vector.iter().zip(reference) {
            ensure!(
                value.is_finite(),
                "Nonfinite vector for {id}/{} row {row}",
                case.name
            );
            let value = f64::from(value);
            let expected = f64::from(expected);
            dot += value * expected;
            left += value * value;
            right += expected * expected;
        }
        let distance = if left == 0.0 && right == 0.0 {
            0.0
        } else {
            ensure!(
                left > 0.0 && right > 0.0,
                "Zero/nonzero vector mismatch for {id}"
            );
            (1.0 - dot / (left * right).sqrt()).max(0.0)
        };
        ensure!(
            distance <= MAX_COSINE_DISTANCE,
            "Cosine distance {distance:.9e} exceeds {MAX_COSINE_DISTANCE} for {id}/{} row {row}",
            case.name
        );
        maximum = maximum.max(distance);
    }
    Ok(maximum)
}

fn tokenizer_files(directory: &Path) -> Result<TokenizerFiles> {
    let read = |name| std::fs::read(directory.join(name));
    Ok(TokenizerFiles {
        tokenizer_file: read("tokenizer.json")?,
        config_file: read("config.json")?,
        special_tokens_map_file: read("special_tokens_map.json")?,
        tokenizer_config_file: read("tokenizer_config.json")?,
    })
}

// Frozen runtime policy exercises prefixes, adaptive batches and weighted chunk pooling
// around the native adapter against vectors captured before the implementation changed.
fn runtime_embed(
    encoder: &mut NativeTextEmbedding,
    files: &TokenizerFiles,
    inputs: &[String],
    prefix: &str,
    max_tokens: usize,
    max_prefix: usize,
) -> Result<(Vec<Vec<f32>>, Vec<usize>)> {
    let capacity = max_tokens.saturating_sub(2 + max_prefix.div_ceil(3)).max(1);
    let sizer = Arc::new(load_tokenizer_from_file(Arc::new(files.clone()), capacity)?);
    let chunker = TextSplitter::new(ChunkConfig::new(capacity).with_sizer(sizer.clone()));
    let mut pieces = Vec::new();
    let mut spans = Vec::new();
    for text in inputs {
        let start = pieces.len();
        if text.len().div_ceil(3).max(1) <= capacity {
            pieces.push(format!("{prefix}{text}"));
        } else {
            pieces.extend(chunker.chunks(text).map(|chunk| format!("{prefix}{chunk}")));
        }
        if pieces.len() == start {
            pieces.push(format!("{prefix}{text}"));
        }
        spans.push(pieces.len() - start);
    }
    let mut vectors = Vec::new();
    let mut start = 0;
    while start < pieces.len() {
        let mut longest = 0;
        let mut count = 0;
        while start + count < pieces.len() {
            let candidate =
                longest.max(pieces[start + count].len().div_ceil(3).clamp(1, max_tokens));
            if count > 0 && (count + 1) * candidate * candidate > 4096 * 4096 {
                break;
            }
            longest = candidate;
            count += 1;
        }
        vectors.extend(encoder.embed(&pieces[start..start + count], Some(count))?);
        start += count;
    }
    let mut result = Vec::new();
    let mut offset = 0;
    for &span in &spans {
        if span == 1 {
            result.push(vectors[offset].clone());
        } else {
            let mut pooled = vec![0.0f32; vectors[offset].len()];
            let mut total = 0.0f32;
            for (vector, text) in vectors[offset..offset + span]
                .iter()
                .zip(&pieces[offset..offset + span])
            {
                let weight = sizer.size(text).max(1) as f32;
                total += weight;
                for (sum, value) in pooled.iter_mut().zip(vector) {
                    *sum += value * weight;
                }
            }
            for value in &mut pooled {
                *value /= total;
            }
            let norm = pooled.iter().map(|value| value * value).sum::<f32>().sqrt();
            if norm > 0.0 {
                for value in &mut pooled {
                    *value /= norm;
                }
            }
            result.push(pooled);
        }
        offset += span;
    }
    Ok((result, spans))
}
