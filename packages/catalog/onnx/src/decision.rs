//! Model selection for the Typed Decision node. Downloads remain in the supplied FlowPath.
use super::{
    execution_providers::{configured_session_builder, ensure_ort_initialized},
    laya::{
        DECISION_MODELS, DEFAULT_DECISION_MODEL, LayaOptions, LayaResult, infer_laya_directory,
    },
    model_cache::{hash_field, with_verified_models},
};
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_catalog_core::FlowPath;
use flow_like_model_provider::ml::ort::session::{Session, builder::GraphOptimizationLevel};
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::{
    Cacheable, Result, anyhow,
    futures::StreamExt,
    sync::Mutex,
    tokio::{io::AsyncWriteExt, sync::OnceCell},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokenizers::Tokenizer;

pub(crate) mod gliner;
mod presets;

const MAX_CONFIG_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
struct BundleConfig {
    format: String,
    source_model: Option<String>,
    #[serde(default)]
    external_data: Vec<String>,
    #[serde(flatten)]
    inference: gliner::GlinerConfig,
}

impl BundleConfig {
    fn validate(&self, model: &str) -> Result<()> {
        if self.format != "gliner2-classifier" {
            return Err(anyhow!(
                "Unsupported decision bundle format '{}'; export with tools/export-decision-model.py",
                self.format
            ));
        }
        if model != "custom" && self.source_model.as_deref() != Some(model) {
            return Err(anyhow!(
                "The decision bundle was exported for {:?}, but Model selects {model}. Select custom or use the matching model directory",
                self.source_model
            ));
        }
        // A single fixed companion keeps external tensor locations inside the selected bundle.
        if self.external_data.len() > 1
            || self
                .external_data
                .iter()
                .any(|file| file != "model.onnx_data")
        {
            return Err(anyhow!(
                "External decision weights must be a single model.onnx_data file; re-export the bundle"
            ));
        }
        self.inference.validate()
    }
}

enum Sessions {
    Combined(Session),
    Split {
        encoder: Session,
        classifier: Session,
    },
}

struct LoadedDecision {
    sessions: Mutex<Sessions>,
    tokenizer: Tokenizer,
    config: gliner::GlinerConfig,
    // Keep external tensors available for providers that defer reading them.
    _directory: Option<tempfile::TempDir>,
}

struct CachedDecision(Arc<OnceCell<Arc<LoadedDecision>>>);

impl Cacheable for CachedDecision {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn child(directory: &FlowPath, name: &str) -> FlowPath {
    let mut path = directory.clone();
    let parent = path.path.trim_end_matches('/');
    path.path = if parent.is_empty() {
        name.into()
    } else {
        format!("{parent}/{name}")
    };
    path
}

fn cache_key(model: &str, directory: &FlowPath, providers: &[String]) -> String {
    let mut hash = Sha256::new();
    hash_field(&mut hash, b"flowlike-decision-session-v1");
    hash_field(&mut hash, model.as_bytes());
    hash_field(&mut hash, directory.store_ref.as_bytes());
    hash_field(&mut hash, directory.object_path().as_ref().as_bytes());
    for provider in providers {
        hash_field(&mut hash, provider.as_bytes());
    }
    format!("decision-model:{}", hex::encode(hash.finalize()))
}

async fn read_config(context: &mut ExecutionContext, path: &FlowPath) -> Result<Option<Vec<u8>>> {
    let store = path.to_store(context).await?;
    let result = match store.as_generic().get(&path.object_path()).await {
        Ok(result) => result,
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => return Ok(None),
        Err(error) => {
            return Err(anyhow!(
                "Failed to read decision config '{}': {error}",
                path.path
            ));
        }
    };
    let mut bytes = Vec::new();
    let mut stream = result.into_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len() + chunk.len() > MAX_CONFIG_BYTES {
            return Err(anyhow!("Decision config exceeds 1 MiB"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(bytes))
}

async fn copy_asset(
    context: &mut ExecutionContext,
    source: &FlowPath,
    destination: &Path,
) -> Result<()> {
    let store = source.to_store(context).await?;
    let result = store.as_generic().get(&source.object_path()).await
        .map_err(|error| anyhow!("Cannot read decision bundle asset '{}': {error}. Complete the ONNX export before running this node", source.path))?;
    let mut stream = result.into_stream();
    let mut file = flow_like_types::tokio::fs::File::create(destination).await?;
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    Ok(())
}

fn session(path: &Path) -> Result<Session> {
    configured_session_builder()?
        .with_optimization_level(GraphOptimizationLevel::Level1)
        .map_err(|error| anyhow!("Failed to configure decision ONNX session: {error}"))?
        .commit_from_file(path)
        .map_err(|error| {
            anyhow!(
                "Failed to load decision ONNX weights '{}': {error}",
                path.display()
            )
        })
}

fn tokenizer(path: &Path) -> Result<Tokenizer> {
    let mut tokenizer = Tokenizer::from_file(path)
        .map_err(|error| anyhow!("Invalid decision tokenizer: {error}"))?;
    tokenizer.with_padding(None);
    tokenizer
        .with_truncation(None)
        .map_err(|error| anyhow!("Invalid decision tokenizer truncation: {error}"))?;
    Ok(tokenizer)
}

fn parse_bundle_config(bytes: &[u8], model: &str) -> Result<BundleConfig> {
    let config: BundleConfig = flow_like_types::json::from_slice(bytes)
        .map_err(|error| anyhow!("Invalid decision_config.json: {error}"))?;
    config.validate(model)?;
    Ok(config)
}

fn local_bundle_config(path: &Path, model: &str) -> Result<BundleConfig> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take((MAX_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(anyhow!("Decision config exceeds 1 MiB"));
    }
    parse_bundle_config(&bytes, model)
}

fn index_preset_assets(
    layout: presets::PresetLayout,
    roles: &[&'static str],
    paths: Vec<PathBuf>,
) -> Result<HashMap<&'static str, PathBuf>> {
    let required: &[&str] = match layout {
        presets::PresetLayout::Combined => &["weights", "tokenizer"],
        presets::PresetLayout::Split => &["encoder", "classifier", "tokenizer"],
        presets::PresetLayout::Bundle => &["weights", "external-data", "tokenizer", "config"],
    };
    if roles.len() != required.len() || paths.len() != required.len() {
        return Err(anyhow!(
            "Decision preset assets do not match its {layout:?} layout"
        ));
    }
    let mut indexed = HashMap::with_capacity(required.len());
    for (&role, path) in roles.iter().zip(paths) {
        if !required.contains(&role) || indexed.insert(role, path).is_some() {
            return Err(anyhow!(
                "Decision preset has an unexpected or duplicate asset role '{role}'"
            ));
        }
    }
    Ok(indexed)
}

fn retain_bundle_file(source: &Path, target: &Path) -> Result<()> {
    // The verified cache helper still needs its source files to persist a fresh download.
    // A hard link keeps external tensors available without duplicating multi-GB weights.
    if std::fs::hard_link(source, target).is_err() {
        std::fs::copy(source, target).map_err(|error| {
            anyhow!(
                "Failed to retain decision bundle file '{}': {error}",
                target.display()
            )
        })?;
    }
    Ok(())
}

fn build_preset_paths(
    model: &str,
    layout: presets::PresetLayout,
    roles: &[&'static str],
    paths: Vec<PathBuf>,
) -> Result<Arc<LoadedDecision>> {
    let assets = index_preset_assets(layout, roles, paths)?;
    let mut directory = None;
    let mut config = gliner::GlinerConfig::default();
    let sessions = match layout {
        presets::PresetLayout::Combined => {
            let session = session(&assets["weights"])?;
            gliner::validate_session(&session)?;
            Sessions::Combined(session)
        }
        presets::PresetLayout::Split => {
            let encoder = session(&assets["encoder"])?;
            let classifier = session(&assets["classifier"])?;
            gliner::validate_split_sessions(&encoder, &classifier)?;
            Sessions::Split {
                encoder,
                classifier,
            }
        }
        presets::PresetLayout::Bundle => {
            let bundle = local_bundle_config(&assets["config"], model)?;
            if bundle.external_data != ["model.onnx_data"] {
                return Err(anyhow!(
                    "Downloaded decision bundle must declare model.onnx_data"
                ));
            }
            let temporary = tempfile::Builder::new()
                .prefix("flowlike-decision-bundle-")
                .tempdir()?;
            for (role, filename) in [
                ("weights", "model.onnx"),
                ("external-data", "model.onnx_data"),
            ] {
                retain_bundle_file(&assets[role], &temporary.path().join(filename))?;
            }
            let session = session(&temporary.path().join("model.onnx"))?;
            gliner::validate_session(&session)?;
            directory = Some(temporary);
            config = bundle.inference;
            Sessions::Combined(session)
        }
    };
    Ok(Arc::new(LoadedDecision {
        sessions: Mutex::new(sessions),
        tokenizer: tokenizer(&assets["tokenizer"])?,
        config,
        _directory: directory,
    }))
}

async fn build_preset(
    context: &mut ExecutionContext,
    model: &str,
    directory: &FlowPath,
    preset: presets::Preset,
) -> Result<Arc<LoadedDecision>> {
    let cache = child(
        directory,
        &format!(".decision-cache/{}", model.rsplit('/').next().unwrap()),
    );
    let model = model.to_owned();
    let roles = preset
        .assets
        .iter()
        .map(|asset| asset.role())
        .collect::<Vec<_>>();
    let layout = preset.layout;
    with_verified_models(
        context,
        &cache,
        &preset.assets,
        "flowlike-decision-",
        move |paths| build_preset_paths(&model, layout, &roles, paths),
    )
    .await
}

async fn build_model(
    context: &mut ExecutionContext,
    model: &str,
    directory: &FlowPath,
) -> Result<Arc<LoadedDecision>> {
    // An exported bundle is all-or-nothing. Never mix a user's weights with a preset tokenizer.
    if let Some(bytes) = read_config(context, &child(directory, "decision_config.json")).await? {
        let config = parse_bundle_config(&bytes, model)?;
        let temporary = tempfile::Builder::new()
            .prefix("flowlike-decision-custom-")
            .tempdir()?;
        for file in ["model.onnx", "tokenizer.json"]
            .into_iter()
            .chain(config.external_data.iter().map(String::as_str))
        {
            copy_asset(
                context,
                &child(directory, file),
                &temporary.path().join(file),
            )
            .await?;
        }
        return flow_like_types::tokio::task::spawn_blocking(move || {
            let session = session(&temporary.path().join("model.onnx"))?;
            gliner::validate_session(&session)?;
            Ok(Arc::new(LoadedDecision {
                sessions: Mutex::new(Sessions::Combined(session)),
                tokenizer: tokenizer(&temporary.path().join("tokenizer.json"))?,
                config: config.inference,
                _directory: Some(temporary),
            }))
        })
        .await
        .map_err(|error| anyhow!("Decision model loading task failed: {error}"))?;
    }
    let Some(preset) = presets::preset(model)? else {
        return Err(anyhow!(if model == "custom" {
            "Custom Model Directory needs a complete Laya bundle or decision_config.json, model.onnx and tokenizer.json from tools/export-decision-model.py".to_string()
        } else {
            format!(
                "{model} needs a local ONNX export. Run tools/export-decision-model.py --model {model} --output <directory>, then connect that directory as Model Directory. No Python is needed during inference"
            )
        }));
    };
    build_preset(context, model, directory, preset).await
}

pub(crate) async fn infer_decision(
    context: &mut ExecutionContext,
    model: &str,
    model_dir: &FlowPath,
    text: &str,
    options: &LayaOptions,
) -> Result<LayaResult> {
    if !DECISION_MODELS.contains(&model) {
        return Err(anyhow!("Unknown decision model '{model}'"));
    }
    if model == DEFAULT_DECISION_MODEL {
        return infer_laya_directory(context, model_dir, text, options, false).await;
    }
    if model == "custom"
        && read_config(context, &child(model_dir, "rl_agent_config.json"))
            .await?
            .is_some()
    {
        return infer_laya_directory(context, model_dir, text, options, true).await;
    }
    let providers = ensure_ort_initialized()?.active_providers;
    let key = cache_key(model, model_dir, &providers);
    let cell = {
        let mut cache = context.cache.write().await;
        if let Some(value) = cache.get(&key) {
            value
                .as_any()
                .downcast_ref::<CachedDecision>()
                .ok_or_else(|| anyhow!("Decision model cache key is occupied by another type"))?
                .0
                .clone()
        } else {
            let cell = Arc::new(OnceCell::new());
            cache.insert(key, Arc::new(CachedDecision(cell.clone())));
            cell
        }
    };
    let loaded = cell
        .get_or_try_init(|| build_model(context, model, model_dir))
        .await?
        .clone();
    let text = text.to_string();
    let options = options.clone();
    flow_like_types::tokio::task::spawn_blocking(move || {
        let mut sessions = loaded.sessions.blocking_lock();
        match &mut *sessions {
            Sessions::Combined(session) => {
                gliner::infer_gliner(session, &loaded.tokenizer, &text, &options, &loaded.config)
            }
            Sessions::Split {
                encoder,
                classifier,
            } => gliner::infer_gliner_split(
                encoder,
                classifier,
                &loaded.tokenizer,
                &text,
                &options,
                &loaded.config,
            ),
        }
    })
    .await
    .map_err(|error| anyhow!("Decision inference task failed: {error}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{LogLevel, Run, internal_node::InternalNode},
            node::Node,
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_storage::{files::store::FlowLikeStore, object_store::memory::InMemory};
    use flow_like_types::sync::RwLock;
    use std::sync::Weak;

    async fn memory_context() -> (ExecutionContext, Arc<InMemory>, FlowPath) {
        let node = Arc::new(InternalNode::new(
            Node::new("test_decision", "Decision test", "", "Tests"),
            AHashMap::new(),
            Arc::new(crate::laya::LayaNode::new()),
            AHashMap::new(),
        ));
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let run: Weak<Mutex<Run>> = Weak::new();
        let context = ExecutionContext::new(
            Arc::new(AHashMap::new()),
            &run,
            &state,
            &node,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await;
        let store = Arc::new(InMemory::new());
        context
            .set_cache("store", Arc::new(FlowLikeStore::Memory(store.clone())))
            .await;
        (
            context,
            store,
            FlowPath::new("models".into(), "store".into(), None),
        )
    }

    #[tokio::test]
    async fn malformed_or_partial_local_bundles_never_fall_back_to_preset_downloads() {
        let (mut context, store, directory) = memory_context().await;
        let path = child(&directory, "decision_config.json").object_path();
        store.put(&path, "{".into()).await.unwrap();
        let error = build_model(&mut context, "fastino/GLiNER2.5-Decide", &directory)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("Invalid decision_config.json"));
        store
            .put(
                &path,
                r#"{"format":"gliner2-classifier","source_model":"fastino/GLiNER2.5-Decide"}"#
                    .into(),
            )
            .await
            .unwrap();
        let error = build_model(&mut context, "fastino/GLiNER2.5-Decide", &directory)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("model.onnx"));
        assert!(!error.to_string().contains("download"));
    }

    #[tokio::test]
    async fn custom_models_require_local_weights_without_downloading() {
        let (mut context, _, directory) = memory_context().await;
        let error = build_model(&mut context, "custom", &directory)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("export-decision-model.py"));
    }

    #[tokio::test]
    async fn external_tensor_must_exist_before_a_bundle_is_loaded() {
        let (mut context, store, directory) = memory_context().await;
        store
            .put(
                &child(&directory, "decision_config.json").object_path(),
                r#"{"format":"gliner2-classifier","external_data":["model.onnx_data"]}"#.into(),
            )
            .await
            .unwrap();
        for file in ["model.onnx", "tokenizer.json"] {
            store
                .put(&child(&directory, file).object_path(), "stub".into())
                .await
                .unwrap();
        }
        let error = build_model(&mut context, "custom", &directory)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("model.onnx_data"));
    }

    #[test]
    fn selections_and_stores_have_independent_sessions() {
        let directory = FlowPath::new("models".into(), "store".into(), None);
        let key = cache_key("custom", &directory, &[]);
        assert_ne!(key, cache_key("fastino/GLiNER2.5-Decide", &directory, &[]));
        assert_ne!(key, cache_key("custom", &child(&directory, "other"), &[]));
        assert_ne!(key, cache_key("custom", &directory, &["CoreML".into()]));
        let other = FlowPath::new("models".into(), "other-store".into(), None);
        assert_ne!(key, cache_key("custom", &other, &[]));
    }

    #[test]
    fn exported_bundle_rejects_wrong_preset_and_external_paths() {
        let mut config: BundleConfig = flow_like_types::json::from_value(flow_like_types::json::json!({
            "format": "gliner2-classifier", "source_model": "fastino/GLiNER2.5-multi-Decide", "max_length": 512
        })).unwrap();
        assert!(config.validate("fastino/GLiNER2.5-multi-Decide").is_ok());
        assert!(config.validate("fastino/GLiNER2.5-Decide-1B").is_err());
        assert!(config.validate("custom").is_ok());
        config.external_data = vec!["../weights".into()];
        assert!(config.validate("custom").is_err());
    }

    #[test]
    fn every_gliner_model_has_a_downloadable_preset() {
        for model in &DECISION_MODELS[1..7] {
            assert!(
                presets::preset(model).unwrap().is_some(),
                "missing preset {model}"
            );
        }
    }

    #[test]
    fn preset_layout_checks_roles_and_ignores_asset_order() {
        let paths = ["tokenizer", "graph", "config", "data"]
            .into_iter()
            .map(PathBuf::from)
            .collect();
        let indexed = index_preset_assets(
            presets::PresetLayout::Bundle,
            &["tokenizer", "weights", "config", "external-data"],
            paths,
        )
        .unwrap();
        assert_eq!(indexed["weights"], PathBuf::from("graph"));
        assert_eq!(indexed["external-data"], PathBuf::from("data"));
        assert!(
            index_preset_assets(
                presets::PresetLayout::Bundle,
                &["weights", "external-data", "tokenizer"],
                vec![PathBuf::new(); 3]
            )
            .is_err()
        );
        assert!(
            index_preset_assets(
                presets::PresetLayout::Bundle,
                &["weights", "external-data", "tokenizer", "tokenizer"],
                vec![PathBuf::new(); 4]
            )
            .is_err()
        );
        assert!(
            index_preset_assets(
                presets::PresetLayout::Combined,
                &["encoder", "tokenizer"],
                vec![PathBuf::new(); 2]
            )
            .is_err()
        );
    }

    #[test]
    fn downloaded_config_is_bounded_and_must_match_the_selected_model() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("config.json");
        std::fs::write(&path, vec![b' '; MAX_CONFIG_BYTES + 1]).unwrap();
        assert!(
            local_bundle_config(&path, "custom")
                .unwrap_err()
                .to_string()
                .contains("1 MiB")
        );
        std::fs::write(&path,
            r#"{"format":"gliner2-classifier","source_model":"fastino/GLiNER2.5-multi-Decide","external_data":["model.onnx_data"]}"#).unwrap();
        assert!(local_bundle_config(&path, "fastino/GLiNER2.5-multi-Decide").is_ok());
        assert!(local_bundle_config(&path, "fastino/GLiNER2.5-Decide-1B").is_err());
    }

    #[test]
    fn retained_external_weights_survive_source_cleanup() {
        let source = tempfile::tempdir().unwrap();
        let retained = tempfile::tempdir().unwrap();
        let source_path = source.path().join("downloaded-weights.onnx");
        let target = retained.path().join("model.onnx_data");
        std::fs::write(&source_path, b"external tensor fixture").unwrap();
        retain_bundle_file(&source_path, &target).unwrap();
        drop(source);
        assert_eq!(std::fs::read(&target).unwrap(), b"external tensor fixture");
        drop(retained);
        assert!(!target.exists());
    }

    #[tokio::test]
    #[ignore = "Set DECISION_MODEL_DIR to a bundle from tools/export-decision-model.py"]
    async fn custom_flowpath_bundle_runs_all_modes_and_reuses_its_session() {
        use crate::laya::LayaQuestionType;

        let (mut context, _, _) = memory_context().await;
        let root = std::env::var_os("DECISION_MODEL_DIR")
            .expect("Set DECISION_MODEL_DIR to an exported decision model directory");
        let directory = FlowPath::from_pathbuf(root.into(), &mut context)
            .await
            .unwrap();
        let providers = ensure_ort_initialized().unwrap().active_providers;
        let key = cache_key("custom", &directory, &providers);
        let text = "I love this product and the support was excellent.";
        let cases = [
            (
                LayaQuestionType::Choice,
                "Which sentiment applies?",
                vec!["positive", "neutral", "negative"],
            ),
            (
                LayaQuestionType::Score,
                "How satisfied is the customer?",
                vec!["dissatisfied", "neutral", "satisfied"],
            ),
            (LayaQuestionType::Noul, "The customer is satisfied.", vec![]),
        ];
        let mut first_model = None;
        let mut first_cache_size = None;
        for (question_type, instructions, criteria) in cases {
            let options = LayaOptions {
                question_type,
                instructions: instructions.into(),
                criteria: criteria.into_iter().map(String::from).collect(),
            };
            let result = infer_decision(&mut context, "custom", &directory, text, &options)
                .await
                .unwrap();
            assert!(result.act_probability.is_none());
            assert!(result.confidence.is_finite());
            assert!(result.input_tokens > 0);
            assert!(
                result
                    .probabilities
                    .iter()
                    .all(|value| value.probability.is_finite()
                        && (0.0..=1.0).contains(&value.probability))
            );
            let total: f64 = result
                .probabilities
                .iter()
                .map(|value| value.probability)
                .sum();
            assert!((total - 1.0).abs() < 0.001);
            match question_type {
                LayaQuestionType::Choice => assert!(result.choice.is_some()),
                LayaQuestionType::Score => assert!(result.score.unwrap().is_finite()),
                LayaQuestionType::Noul => assert!(result.noul.unwrap().is_finite()),
            }
            let cache = context.cache.read().await;
            let loaded = cache
                .get(&key)
                .unwrap()
                .as_any()
                .downcast_ref::<CachedDecision>()
                .unwrap()
                .0
                .get()
                .unwrap()
                .clone();
            if let Some(first) = &first_model {
                assert!(Arc::ptr_eq(first, &loaded));
                assert_eq!(first_cache_size, Some(cache.len()));
            } else {
                first_model = Some(loaded);
                first_cache_size = Some(cache.len());
            }
        }
    }

    #[tokio::test]
    #[ignore = "Set DECISION_SPLIT_MODEL_DIR to the published GLiNER2.5 Small split ONNX files"]
    async fn published_small_preset_reuses_flowpath_disk_cache() {
        let source = std::path::PathBuf::from(std::env::var_os("DECISION_SPLIT_MODEL_DIR")
            .expect("Set DECISION_SPLIT_MODEL_DIR to a directory with encoder.onnx, classifier.onnx and tokenizer.json"));
        let temporary = tempfile::tempdir().unwrap();
        let cache = temporary.path().join(".decision-cache/gliner2.5-small-v1");
        std::fs::create_dir_all(&cache).unwrap();
        let model = "fastino/gliner2.5-small-v1";
        let preset = presets::preset(model).unwrap().unwrap();
        let mut cached_files = Vec::new();
        for (spec, source_file) in
            preset
                .assets
                .iter()
                .zip(["encoder.onnx", "classifier.onnx", "tokenizer.json"])
        {
            let target = cache.join(spec.cache_file_name());
            std::fs::copy(source.join(source_file), &target).unwrap();
            let modified = std::fs::metadata(&target).unwrap().modified().unwrap();
            cached_files.push((target, modified));
        }
        let options = LayaOptions {
            question_type: crate::laya::LayaQuestionType::Choice,
            instructions: "Which sentiment applies?".into(),
            criteria: vec!["positive".into(), "neutral".into(), "negative".into()],
        };
        // A fresh execution context has no model session, so the second run must read
        // and verify the persisted FlowPath cache again.
        for _ in 0..2 {
            let (mut context, _, _) = memory_context().await;
            let directory = FlowPath::from_pathbuf(temporary.path().to_path_buf(), &mut context)
                .await
                .unwrap();
            let result = infer_decision(
                &mut context,
                model,
                &directory,
                "I love this product and the support was excellent.",
                &options,
            )
            .await
            .unwrap();
            assert_eq!(result.choice.as_deref(), Some("positive"));
            assert_eq!(
                result
                    .probabilities
                    .iter()
                    .map(|value| value.probability)
                    .collect::<Vec<_>>(),
                vec![0.9996, 0.0002, 0.0002]
            );
        }
        for (target, modified) in cached_files {
            assert_eq!(
                std::fs::metadata(target).unwrap().modified().unwrap(),
                modified
            );
        }
    }

    #[tokio::test]
    #[ignore = "Set DECISION_MODEL_DIR to an exported bundle with model.onnx_data"]
    async fn downloaded_bundle_keeps_external_weights_and_reuses_verified_disk_cache() {
        use super::super::model_cache::{DECISION_MODELS as FAMILY, ModelSpec};

        let source = PathBuf::from(
            std::env::var_os("DECISION_MODEL_DIR")
                .expect("Set DECISION_MODEL_DIR to an exported decision bundle"),
        );
        let config = local_bundle_config(&source.join("decision_config.json"), "custom").unwrap();
        let model = config
            .source_model
            .as_deref()
            .expect("exported bundle source_model");
        let temporary = tempfile::tempdir().unwrap();
        let cache = temporary
            .path()
            .join(".decision-cache")
            .join(model.rsplit('/').next().unwrap());
        std::fs::create_dir_all(&cache).unwrap();
        // Deliberately shuffle the roles: bundle loading must use their names.
        let files = [
            ("tokenizer", "tokenizer.json"),
            ("config", "decision_config.json"),
            ("external-data", "model.onnx_data"),
            ("weights", "model.onnx"),
        ];
        let mut specs = Vec::new();
        let mut cached_files = Vec::new();
        for (role, file) in files {
            let path = source.join(file);
            let size = std::fs::metadata(&path).unwrap().len();
            let mut hasher = Sha256::new();
            let mut reader = std::fs::File::open(&path).unwrap();
            let mut chunk = vec![0u8; 1024 * 1024];
            loop {
                let size = reader.read(&mut chunk).unwrap();
                if size == 0 {
                    break;
                }
                hasher.update(&chunk[..size]);
            }
            let hash = hex::encode(hasher.finalize());
            let spec = ModelSpec::new(
                &FAMILY,
                role,
                size,
                &format!("https://example.invalid/{file}"),
                &hash,
            )
            .unwrap();
            let target = cache.join(spec.cache_file_name());
            std::fs::copy(path, &target).unwrap();
            let modified = std::fs::metadata(&target).unwrap().modified().unwrap();
            cached_files.push((target, modified));
            specs.push(spec);
        }
        if std::env::var_os("DECISION_VERIFY_PRESET").is_some() {
            let preset = presets::preset(model)
                .unwrap()
                .expect("exported source model must have a built-in preset");
            assert_eq!(preset.layout, presets::PresetLayout::Bundle);
            assert_eq!(preset.assets.len(), specs.len());
            for asset in &preset.assets {
                let source = specs
                    .iter()
                    .find(|spec| spec.role() == asset.role())
                    .expect("published preset role must exist in the exported bundle");
                assert_eq!(source.expected_sha256(), asset.expected_sha256());
            }
            specs = preset.assets;
        }
        let options = LayaOptions {
            question_type: crate::laya::LayaQuestionType::Choice,
            instructions: "Which sentiment applies?".into(),
            criteria: vec!["positive".into(), "neutral".into(), "negative".into()],
        };
        for _ in 0..2 {
            let (mut context, _, _) = memory_context().await;
            let directory = FlowPath::from_pathbuf(temporary.path().to_path_buf(), &mut context)
                .await
                .unwrap();
            let loaded = build_preset(
                &mut context,
                model,
                &directory,
                presets::Preset {
                    assets: specs.clone(),
                    layout: presets::PresetLayout::Bundle,
                },
            )
            .await
            .unwrap();
            let retained = loaded
                ._directory
                .as_ref()
                .expect("bundle retains external tensors")
                .path()
                .to_path_buf();
            assert!(retained.join("model.onnx").is_file());
            assert!(retained.join("model.onnx_data").is_file());
            assert_eq!(loaded.config.max_length, config.inference.max_length);
            assert_eq!(loaded.config.temperature, config.inference.temperature);
            let result = {
                let mut sessions = loaded.sessions.lock().await;
                let Sessions::Combined(session) = &mut *sessions else {
                    panic!("expected combined bundle graph")
                };
                gliner::infer_gliner(
                    session,
                    &loaded.tokenizer,
                    "I love this product and the support was excellent.",
                    &options,
                    &loaded.config,
                )
                .unwrap()
            };
            assert_eq!(result.choice.as_deref(), Some("positive"));
            assert!(
                result
                    .probabilities
                    .iter()
                    .all(|value| value.probability.is_finite())
            );
            drop(loaded);
            assert!(!retained.exists());
        }
        for (target, modified) in cached_files {
            assert_eq!(
                std::fs::metadata(target).unwrap().modified().unwrap(),
                modified
            );
        }
    }
}
