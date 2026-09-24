use super::LayaConfig;
use crate::onnx::{
    execution_providers::{configured_session_builder, ensure_ort_initialized},
    model_cache::{
        LAYA_MODELS, ModelSpec, hash_field, validate_model_cache_dir, with_verified_models,
    },
};
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_catalog_core::FlowPath;
use flow_like_model_provider::ml::ort::session::{Session, builder::GraphOptimizationLevel};
use flow_like_storage::object_store::{GetResult, ObjectStoreExt};
use flow_like_types::{
    Cacheable, Result, anyhow,
    futures::StreamExt,
    sync::Mutex,
    tokio::{io::AsyncWriteExt, sync::OnceCell},
};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, sync::Arc};
use tokenizers::Tokenizer;

const REVISION: &str = "d9d003d543e63d6d3375c21d44624136bd1e0bad";

struct BuiltinAsset {
    role: &'static str,
    file: &'static str,
    sha256: &'static str,
    size_bytes: u64,
}

const ASSETS: [BuiltinAsset; 3] = [
    BuiltinAsset {
        role: "weights",
        file: "model.onnx",
        sha256: "0b095e005a4c295cae74d47b7eb6931c369d48f5b720b45278d774165798310c",
        size_bytes: 646_870_871,
    },
    BuiltinAsset {
        role: "tokenizer",
        file: "tokenizer/tokenizer.json",
        sha256: "609d8f4c067cd3950f88594c5a802616cea245823836ef5848ee4fc40aab5b6f",
        size_bytes: 34_363_188,
    },
    BuiltinAsset {
        role: "config",
        file: "rl_agent_config.json",
        sha256: "9a669a70961064c3c6cc76d2afb8bc5fb10dcd8349bb66e5f7b9b1afb74440d5",
        size_bytes: 473,
    },
];

pub(super) struct LoadedLaya {
    pub session: Mutex<Session>,
    pub tokenizer: Tokenizer,
    pub config: LayaConfig,
}

struct CachedLaya {
    model: Arc<OnceCell<Arc<LoadedLaya>>>,
}

impl Cacheable for CachedLaya {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn model_cache_key(model_dir: &FlowPath, providers: &[String]) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"flowlike-loaded-laya-v2");
    hash_field(&mut hasher, model_dir.store_ref.as_bytes());
    hash_field(&mut hasher, model_dir.object_path().as_ref().as_bytes());
    for asset in &ASSETS {
        hash_field(&mut hasher, asset.role.as_bytes());
        hash_field(&mut hasher, asset.sha256.as_bytes());
    }
    for provider in providers {
        hash_field(&mut hasher, provider.as_bytes());
    }
    format!("laya-model:{}", hex::encode(hasher.finalize()))
}

fn child_path(directory: &FlowPath, file: &str) -> FlowPath {
    let mut child = directory.clone();
    let parent = directory.path.trim_end_matches('/');
    child.path = if parent.is_empty() {
        file.to_string()
    } else {
        format!("{parent}/{file}")
    };
    child
}

async fn download_cache_dir(
    context: &mut ExecutionContext,
    model_dir: &FlowPath,
    specs: &[ModelSpec],
) -> Result<FlowPath> {
    // Older nodes cached managed assets directly in Cache Directory. Keep using that cache
    // when present, so migrating the pin does not download the entire model again.
    if validate_model_cache_dir(model_dir, LAYA_MODELS.label).is_ok() {
        let store = model_dir.to_store(context).await?;
        for spec in specs {
            match store
                .as_generic()
                .head(&spec.cache_path(model_dir).object_path())
                .await
            {
                Ok(_) => return Ok(model_dir.clone()),
                Err(flow_like_storage::object_store::Error::NotFound { .. }) => {}
                Err(error) => {
                    return Err(anyhow!("Failed to inspect Laya model cache: {error}"));
                }
            }
        }
    }
    // Scope quota scans even when the selected model directory is a store root.
    Ok(child_path(model_dir, ".laya-cache"))
}

fn builtin_spec(asset: &BuiltinAsset) -> Result<ModelSpec> {
    ModelSpec::new(
        &LAYA_MODELS,
        asset.role,
        asset.size_bytes,
        &format!(
            "https://huggingface.co/mizchi/laya-multilingual-onnx/resolve/{REVISION}/{}",
            asset.file
        ),
        asset.sha256,
    )
}

/// Reuses one initialized model per directory in the execution context. Missing assets use
/// pinned, verified downloads in the directory's managed cache.
pub(super) async fn load_laya(
    context: &mut ExecutionContext,
    model_dir: &FlowPath,
) -> Result<Arc<LoadedLaya>> {
    let providers = ensure_ort_initialized()?.active_providers;
    let key = model_cache_key(model_dir, &providers);
    let cell = {
        let mut cache = context.cache.write().await;
        if let Some(entry) = cache.get(&key) {
            entry
                .as_any()
                .downcast_ref::<CachedLaya>()
                .ok_or_else(|| anyhow!("Laya model cache key is occupied by another type"))?
                .model
                .clone()
        } else {
            let model = Arc::new(OnceCell::new());
            cache.insert(
                key,
                Arc::new(CachedLaya {
                    model: model.clone(),
                }),
            );
            model
        }
    };
    cell.get_or_try_init(|| build_laya(context, model_dir))
        .await
        .cloned()
}

async fn build_laya(
    context: &mut ExecutionContext,
    model_dir: &FlowPath,
) -> Result<Arc<LoadedLaya>> {
    let temporary = tempfile::Builder::new()
        .prefix("flowlike-laya-custom-")
        .tempdir()?;
    let mut local_paths: [PathBuf; 3] = std::array::from_fn(|index| {
        temporary
            .path()
            .join(format!("{}-asset", ASSETS[index].role))
    });
    let missing = materialize_directory(context, model_dir, &local_paths).await?;
    let specs: Vec<ModelSpec> = missing
        .iter()
        .map(|&index| builtin_spec(&ASSETS[index]))
        .collect::<Result<_>>()?;
    let build = move |paths: Vec<PathBuf>| -> Result<Arc<LoadedLaya>> {
        // Own custom files until ORT finishes reading, including when an async load is cancelled.
        let _temporary = temporary;
        for (index, path) in missing.into_iter().zip(paths) {
            local_paths[index] = path;
        }
        build_from_paths(local_paths)
    };
    if specs.is_empty() {
        return flow_like_types::tokio::task::spawn_blocking(move || build(Vec::new()))
            .await
            .map_err(|error| anyhow!("Laya model build task panicked: {error}"))?;
    }
    let cache_dir = download_cache_dir(context, model_dir, &specs).await?;
    validate_model_cache_dir(&cache_dir, LAYA_MODELS.label)?;
    with_verified_models(context, &cache_dir, &specs, "flowlike-laya-", build).await
}

async fn materialize_directory(
    context: &mut ExecutionContext,
    model_dir: &FlowPath,
    local_paths: &[PathBuf; 3],
) -> Result<Vec<usize>> {
    let mut missing = Vec::new();
    for (index, asset) in ASSETS.iter().enumerate() {
        let source = child_path(model_dir, asset.file);
        if materialize_custom(context, &source, &local_paths[index], asset.role).await? {
            continue;
        }
        // Local tokenizer exports often keep tokenizer.json next to model.onnx.
        if asset.role == "tokenizer"
            && materialize_custom(
                context,
                &child_path(model_dir, "tokenizer.json"),
                &local_paths[index],
                asset.role,
            )
            .await?
        {
            continue;
        }
        missing.push(index);
    }
    Ok(missing)
}

fn custom_file_lookup(
    result: flow_like_storage::object_store::Result<GetResult>,
    source: &FlowPath,
    role: &str,
) -> Result<Option<GetResult>> {
    match result {
        Ok(result) => Ok(Some(result)),
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(None),
        Err(error) => Err(anyhow!(
            "Failed to read Laya {role} '{}': {error}",
            source.path
        )),
    }
}

async fn materialize_custom(
    context: &mut ExecutionContext,
    source: &FlowPath,
    destination: &std::path::Path,
    role: &str,
) -> Result<bool> {
    let store = source.to_store(context).await?;
    // Read the primary store directly: FlowPath's general cache lookup treats read errors as
    // missing files, which could replace an inaccessible custom model with the builtin model.
    let result = custom_file_lookup(
        store.as_generic().get(&source.object_path()).await,
        source,
        role,
    )?;
    let Some(result) = result else {
        return Ok(false);
    };
    let mut stream = result.into_stream();
    let mut file = flow_like_types::tokio::fs::File::create(destination).await?;
    while let Some(chunk) = stream.next().await {
        file.write_all(&chunk?).await?;
    }
    file.flush().await?;
    Ok(true)
}

fn build_from_paths(paths: [PathBuf; 3]) -> Result<Arc<LoadedLaya>> {
    let config: LayaConfig = flow_like_types::json::from_slice(&std::fs::read(&paths[2])?)
        .map_err(|error| anyhow!("Invalid Laya rl_agent_config.json: {error}"))?;
    config.validate()?;
    let mut tokenizer = Tokenizer::from_file(&paths[1])
        .map_err(|error| anyhow!("Invalid Laya tokenizer.json: {error}"))?;
    tokenizer.with_padding(None);
    tokenizer
        .with_truncation(None)
        .map_err(|error| anyhow!("Failed to disable Laya tokenizer truncation: {error}"))?;
    let session = configured_session_builder()?
        .with_optimization_level(GraphOptimizationLevel::Level1)
        .map_err(|error| anyhow!("Failed to configure Laya graph optimization: {error}"))?
        .commit_from_file(&paths[0])
        .map_err(|error| anyhow!("Failed to load the Laya ONNX weights: {error}"))?;
    super::validate_session(&session)?;
    Ok(Arc::new(LoadedLaya {
        session: Mutex::new(session),
        tokenizer,
        config,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::{Board, ExecutionStage},
            execution::{LogLevel, Run, internal_node::InternalNode, internal_pin::InternalPin},
            node::{Node, NodeLogic},
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_storage::{
        files::store::FlowLikeStore,
        object_store::{ObjectStoreExt, memory::InMemory},
    };
    use flow_like_types::{json::json, sync::RwLock};
    use std::sync::Weak;

    async fn context() -> ExecutionContext {
        context_for_node(Node::new(
            "test_laya",
            "Laya test",
            "Laya loading test",
            "Tests",
        ))
        .await
    }

    async fn context_for_node(node: Node) -> ExecutionContext {
        let mut pins = AHashMap::new();
        let mut name_cache: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();
        for pin in node.pins.values() {
            let internal_pin = Arc::new(InternalPin::new(pin, false));
            name_cache
                .entry(pin.name.clone())
                .or_default()
                .push(internal_pin.clone());
            pins.insert(pin.id.clone(), internal_pin);
        }
        let node = Arc::new(InternalNode::new(
            node,
            pins,
            Arc::new(super::super::LayaNode::new()),
            name_cache,
        ));
        for pin in &node.pins {
            pin.init_node(Arc::downgrade(&node));
        }
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let run: Weak<Mutex<Run>> = Weak::new();
        ExecutionContext::new(
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
        .await
    }

    async fn memory_context() -> (ExecutionContext, Arc<InMemory>) {
        let context = context().await;
        let store = Arc::new(InMemory::new());
        context
            .set_cache("store", Arc::new(FlowLikeStore::Memory(store.clone())))
            .await;
        (context, store)
    }

    fn path(value: &str) -> FlowPath {
        FlowPath::new(value.to_string(), "store".to_string(), None)
    }

    #[test]
    fn bundled_assets_are_pinned_managed_roles() {
        assert_eq!(ASSETS.map(|asset| asset.role), LAYA_MODELS.roles);
        for asset in &ASSETS {
            assert!(asset.size_bytes > 0);
            let spec = builtin_spec(asset).unwrap();
            assert_eq!(spec.role(), asset.role);
            assert_eq!(spec.expected_sha256(), asset.sha256);
        }
    }

    #[test]
    fn complete_bundled_model_fits_the_mobile_download_cache() {
        let bundle_bytes: u64 = ASSETS.iter().map(|asset| asset.size_bytes).sum();
        assert!(bundle_bytes > 512 * 1024 * 1024);
        assert!(bundle_bytes <= crate::onnx::model_cache::model_cache_quota_bytes(true));
    }

    #[test]
    fn model_directories_stores_and_providers_have_distinct_cache_entries() {
        let directory = path("models/laya");
        let providers = vec!["CPUExecutionProvider".to_string()];
        let base_key = model_cache_key(&directory, &providers);
        assert_ne!(base_key, model_cache_key(&path("models/other"), &providers));
        let other_store = FlowPath::new(directory.path.clone(), "other-store".into(), None);
        assert_ne!(base_key, model_cache_key(&other_store, &providers));
        assert_ne!(
            base_key,
            model_cache_key(&directory, &["CoreMLExecutionProvider".to_string()])
        );
    }

    #[test]
    fn equivalent_flow_paths_share_one_model() {
        let raw = FlowPath {
            path: "models/custom model".to_string(),
            store_ref: "store".to_string(),
            cache_store_ref: Some("cache".to_string()),
        };
        assert_eq!(
            model_cache_key(&raw, &[]),
            model_cache_key(&path("models/custom%20model"), &[])
        );
        assert_eq!(
            child_path(&path("models/laya/"), "model.onnx").path,
            "models/laya/model.onnx"
        );
    }

    #[tokio::test]
    async fn model_directory_streams_existing_assets_and_marks_only_missing_assets() {
        let (mut context, store) = memory_context().await;
        let model_dir = path("models/laya");
        let bytes = vec![42; 128 * 1024];
        store
            .put(
                &child_path(&model_dir, "model.onnx").object_path(),
                bytes.clone().into(),
            )
            .await
            .unwrap();
        store
            .put(
                &child_path(&model_dir, "tokenizer.json").object_path(),
                b"root tokenizer".to_vec().into(),
            )
            .await
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let destinations = std::array::from_fn(|index| directory.path().join(index.to_string()));
        let missing = materialize_directory(&mut context, &model_dir, &destinations)
            .await
            .unwrap();
        assert_eq!(missing, [2]);
        assert_eq!(std::fs::read(&destinations[0]).unwrap(), bytes);
        assert_eq!(std::fs::read(&destinations[1]).unwrap(), b"root tokenizer");
        assert!(!destinations[2].exists());

        store
            .put(
                &child_path(&model_dir, "tokenizer/tokenizer.json").object_path(),
                b"nested tokenizer".to_vec().into(),
            )
            .await
            .unwrap();
        store
            .put(
                &child_path(&model_dir, "rl_agent_config.json").object_path(),
                b"custom config".to_vec().into(),
            )
            .await
            .unwrap();
        assert!(
            materialize_directory(&mut context, &model_dir, &destinations)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            std::fs::read(&destinations[1]).unwrap(),
            b"nested tokenizer"
        );
        assert_eq!(std::fs::read(&destinations[2]).unwrap(), b"custom config");
    }

    #[tokio::test]
    async fn empty_model_directory_marks_the_complete_bundle_for_download() {
        let (mut context, _) = memory_context().await;
        let directory = tempfile::tempdir().unwrap();
        let destinations = std::array::from_fn(|index| directory.path().join(index.to_string()));
        assert_eq!(
            materialize_directory(&mut context, &path("models/laya"), &destinations)
                .await
                .unwrap(),
            [0, 1, 2]
        );
    }

    #[tokio::test]
    async fn invalid_custom_configuration_is_reported_without_replacing_the_file() {
        let (mut context, store) = memory_context().await;
        let model_dir = path("models/laya");
        for asset in &ASSETS {
            store
                .put(
                    &child_path(&model_dir, asset.file).object_path(),
                    b"invalid custom file".to_vec().into(),
                )
                .await
                .unwrap();
        }
        let error = build_laya(&mut context, &model_dir).await.err().unwrap();
        assert!(
            error
                .to_string()
                .contains("Invalid Laya rl_agent_config.json")
        );
        let config = store
            .get(&child_path(&model_dir, "rl_agent_config.json").object_path())
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap();
        assert_eq!(config.as_ref(), b"invalid custom file");
    }

    #[test]
    fn permission_failures_cannot_be_treated_as_missing_custom_models() {
        let denied = flow_like_storage::object_store::Error::PermissionDenied {
            path: "models/laya/model.onnx".to_string(),
            source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "access denied")
                .into(),
        };
        let error = custom_file_lookup(Err(denied), &path("models/laya/model.onnx"), "weights")
            .unwrap_err();
        assert!(error.to_string().contains("models/laya/model.onnx"));
        assert!(error.to_string().contains("access denied"));
    }

    #[tokio::test]
    async fn new_directories_scope_cache_scans_and_legacy_caches_are_reused() {
        let (mut context, store) = memory_context().await;
        let specs = ASSETS
            .iter()
            .map(builtin_spec)
            .collect::<Result<Vec<_>>>()
            .unwrap();
        let model_dir = path("models/laya");
        assert_eq!(
            download_cache_dir(&mut context, &model_dir, &specs)
                .await
                .unwrap()
                .path,
            "models/laya/.laya-cache"
        );
        assert_eq!(
            download_cache_dir(&mut context, &path("/"), &specs)
                .await
                .unwrap()
                .path,
            ".laya-cache"
        );
        store
            .put(
                &specs[0].cache_path(&model_dir).object_path(),
                b"legacy cache".to_vec().into(),
            )
            .await
            .unwrap();
        assert_eq!(
            download_cache_dir(&mut context, &model_dir, &specs)
                .await
                .unwrap()
                .path,
            model_dir.path
        );
    }

    #[tokio::test]
    #[ignore = "requires LAYA_MODEL_DIR with the model bundle"]
    async fn each_updated_mode_executes_with_only_its_selected_output() {
        use super::super::{LayaNode, LayaResult};

        let directory =
            PathBuf::from(std::env::var_os("LAYA_MODEL_DIR").expect("set LAYA_MODEL_DIR"));
        let mut shared_context = context().await;
        let model_dir = FlowPath::from_pathbuf(directory, &mut shared_context)
            .await
            .unwrap();
        let logic = LayaNode::new();
        let board = Board::new_detached(None, flow_like_storage::Path::from("test-laya"));
        let mut first_model: Option<Arc<LoadedLaya>> = None;
        for mode in ["choice", "score", "noul"] {
            let mut node = logic.get_node();
            node.get_pin_mut_by_name("question_type")
                .unwrap()
                .set_default_value(Some(json!(mode)));
            logic.on_update(&mut node, &board).await;
            assert!(node.error.is_none());
            assert_eq!(node.get_pin_by_name("criteria").is_some(), mode != "noul");
            let mut context = context_for_node(node).await;
            context.cache = shared_context.cache.clone();
            context
                .set_pin_value("model_dir", json!(model_dir))
                .await
                .unwrap();
            context
                .set_pin_value("text", json!("Paris is the capital of France."))
                .await
                .unwrap();
            context
                .set_pin_value(
                    "instructions",
                    json!("The text states that Paris is the capital of France."),
                )
                .await
                .unwrap();
            if mode != "noul" {
                context
                    .set_pin_value("criteria", json!(["false", "true"]))
                    .await
                    .unwrap();
            }

            logic
                .run(&mut context)
                .await
                .unwrap_or_else(|error| panic!("{mode} execution failed: {error:#}"));
            let result: LayaResult = context.evaluate_pin("result").await.unwrap();
            assert_eq!(result.question_type.name(), mode);
            assert!(result.input_tokens > 0);
            assert_eq!(result.probabilities.len(), 2);
            assert!(result.confidence.is_finite());
            assert_eq!(
                context.evaluate_pin::<f64>("confidence").await.unwrap(),
                result.confidence
            );
            assert!(context.evaluate_pin::<bool>("exec_out").await.unwrap());
            for (output, value) in [
                ("choice", json!(result.choice)),
                ("score", json!(result.score)),
                ("noul", json!(result.noul)),
            ] {
                if output == mode {
                    assert!(!value.is_null());
                    assert_eq!(
                        context
                            .evaluate_pin::<flow_like_types::Value>(output)
                            .await
                            .unwrap(),
                        value
                    );
                } else {
                    assert!(value.is_null());
                    assert!(context.get_pin_by_name(output).await.is_err());
                }
            }
            let loaded = load_laya(&mut context, &model_dir).await.unwrap();
            if let Some(first_model) = &first_model {
                assert!(Arc::ptr_eq(first_model, &loaded));
            } else {
                first_model = Some(loaded);
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires LAYA_MODEL_DIR with the model bundle and network for a 473-byte config download"]
    async fn model_directory_loads_local_assets_repairs_missing_config_cache_and_reuses_session() {
        let directory =
            PathBuf::from(std::env::var_os("LAYA_MODEL_DIR").expect("set LAYA_MODEL_DIR"));
        {
            let mut direct_context = context().await;
            let model_dir = FlowPath::from_pathbuf(directory.clone(), &mut direct_context)
                .await
                .unwrap();
            let direct = load_laya(&mut direct_context, &model_dir).await.unwrap();
            let reused = load_laya(&mut direct_context, &model_dir).await.unwrap();
            assert!(Arc::ptr_eq(&direct, &reused));
        }

        let partial = tempfile::tempdir().unwrap();
        std::fs::hard_link(
            directory.join("model.onnx"),
            partial.path().join("model.onnx"),
        )
        .unwrap();
        std::fs::hard_link(
            directory.join("tokenizer/tokenizer.json"),
            partial.path().join("tokenizer.json"),
        )
        .unwrap();
        std::fs::create_dir(partial.path().join(".laya-cache")).unwrap();
        let config_spec = builtin_spec(&ASSETS[2]).unwrap();
        let config_cache = partial
            .path()
            .join(".laya-cache")
            .join(config_spec.cache_file_name());
        std::fs::write(&config_cache, b"corrupt config").unwrap();
        let mut context = context().await;
        let model_dir = FlowPath::from_pathbuf(partial.path().to_path_buf(), &mut context)
            .await
            .unwrap();
        let loaded = load_laya(&mut context, &model_dir).await.unwrap();
        assert_eq!(
            hex::encode(Sha256::digest(std::fs::read(&config_cache).unwrap())),
            ASSETS[2].sha256
        );
        assert!(loaded.tokenizer.get_padding().is_none());
        assert!(loaded.tokenizer.get_truncation().is_none());
        let reused = load_laya(&mut context, &model_dir).await.unwrap();
        assert!(Arc::ptr_eq(&loaded, &reused));
        drop(loaded);
        drop(reused);
        drop(context);

        // An existing custom config remains authoritative even when a valid builtin is cached.
        std::fs::write(
            partial.path().join("rl_agent_config.json"),
            b"invalid custom config",
        )
        .unwrap();
        let mut fresh_context = self::context().await;
        let model_dir = FlowPath::from_pathbuf(partial.path().to_path_buf(), &mut fresh_context)
            .await
            .unwrap();
        let error = load_laya(&mut fresh_context, &model_dir)
            .await
            .err()
            .unwrap();
        assert!(
            error
                .to_string()
                .contains("Invalid Laya rl_agent_config.json")
        );
        assert_eq!(
            std::fs::read(partial.path().join("rl_agent_config.json")).unwrap(),
            b"invalid custom config"
        );
    }
}
