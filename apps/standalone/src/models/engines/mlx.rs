//! MLX chat and vision models on Apple-silicon Macs. `flow-like-standalone model-worker --engine
//! mlx` starts the helper of the installed MLX pack, loads the model and serves it through the
//! OpenAI-compatible endpoint of the core runtime, on loopback behind a bearer key. The helper
//! answers one request at a time.

use super::{
    EnginePlan,
    gguf::GgufFacts,
    llamacpp::{self, ModelFiles},
    random_key, worker_launch,
};
use crate::models::runtime::InstalledRuntime;
use anyhow::{Result, ensure};
use flow_like_device_protocol::{
    KvCacheType, MODEL_MAX_CTX_PER_SLOT, MODEL_MIN_CTX_PER_SLOT, ModelKind, ModelSettings,
    ModelSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

const CONFIG_FILE: &str = "config.json";
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
const REQUIRED_FILES: [&str; 3] = [CONFIG_FILE, "tokenizer.json", "tokenizer_config.json"];
const VISION_PROCESSORS: [&str; 2] = ["processor_config.json", "preprocessor_config.json"];

/// What the agent hands its MLX worker on stdin.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub model_id: String,
    pub key: String,
    pub dir: PathBuf,
    pub vision: bool,
    /// The helper of the installed MLX pack; its resource bundles sit beside it.
    pub helper: PathBuf,
    pub max_kv_size: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_bits: Option<u8>,
}

impl Drop for WorkerConfig {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.key);
    }
}

/// What an MLX start uses: the tokens a request may keep in its KV cache, and the cache type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MlxSettings {
    pub max_kv_size: u32,
    pub kv_cache_type: KvCacheType,
}

/// The settings of the model, else its trained context up to 8192 tokens and an f16 cache.
pub fn effective(settings: &ModelSettings, kind: ModelKind, facts: &GgufFacts) -> MlxSettings {
    MlxSettings {
        max_kv_size: settings
            .ctx_per_slot
            .unwrap_or_else(|| llamacpp::default_ctx(kind, facts))
            .clamp(MODEL_MIN_CTX_PER_SLOT, MODEL_MAX_CTX_PER_SLOT),
        kv_cache_type: settings.kv_cache_type.unwrap_or(KvCacheType::F16),
    }
}

fn kv_bits(kind: KvCacheType) -> Option<u8> {
    match kind {
        KvCacheType::F16 => None,
        KvCacheType::Q8 => Some(8),
        KvCacheType::Q4 => Some(4),
    }
}

/// Refuses a model MLX cannot load: it needs its config and tokenizer files at the top of its
/// directory, safetensors weights, and a vision model its processor config.
pub fn check_layout(spec: &ModelSpec) -> Result<()> {
    let has = |name: &str| spec.assets.iter().any(|asset| asset.file_name == name);
    for name in REQUIRED_FILES {
        ensure!(has(name), "MLX model {} lacks {name}", spec.display_name);
    }
    ensure!(
        spec.assets.iter().any(|asset| asset
            .file_name
            .to_ascii_lowercase()
            .ends_with(".safetensors")),
        "MLX model {} has no .safetensors weights",
        spec.display_name
    );
    ensure!(
        spec.kind != ModelKind::Vision || VISION_PROCESSORS.into_iter().any(has),
        "MLX vision model {} lacks processor_config.json or preprocessor_config.json",
        spec.display_name
    );
    Ok(())
}

/// The sizes of the KV cache in a Hugging Face `config.json`; vision models keep them in
/// `text_config`. An unreadable config gives no facts, so the estimate falls back to the weights.
pub fn config_facts(dir: &Path) -> GgufFacts {
    let Some(config) = read_config_json(dir) else {
        return GgufFacts::default();
    };
    let text = config
        .get("text_config")
        .filter(|text| text.is_object())
        .unwrap_or(&config);
    let number = |key: &str| {
        text.get(key)
            .or_else(|| config.get(key))
            .and_then(Value::as_u64)
    };
    let layers = number("num_hidden_layers");
    let heads = number("num_attention_heads");
    let kv_heads = number("num_key_value_heads").or(heads);
    GgufFacts {
        architecture: config
            .get("model_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        block_count: layers,
        context_length: number("max_position_embeddings"),
        embedding_length: number("hidden_size"),
        head_count: heads,
        head_count_kv_total: kv_heads
            .zip(layers)
            .and_then(|(heads, layers)| heads.checked_mul(layers)),
        key_length: number("head_dim"),
        value_length: number("head_dim"),
    }
}

fn read_config_json(dir: &Path) -> Option<Value> {
    let mut text = Vec::new();
    std::fs::File::open(dir.join(CONFIG_FILE))
        .ok()?
        .take(MAX_CONFIG_BYTES)
        .read_to_end(&mut text)
        .ok()?;
    serde_json::from_slice(&text).ok()
}

/// The worker that serves `spec` from its files with the helper of `runtime`, and what it holds:
/// the weights, a KV cache of the context and compute buffers.
pub fn plan(
    model_id: &str,
    spec: &ModelSpec,
    settings: &ModelSettings,
    facts: &GgufFacts,
    files: &ModelFiles,
    runtime: &InstalledRuntime,
) -> Result<EnginePlan> {
    check_layout(spec)?;
    let chosen = effective(settings, spec.kind, facts);
    let key = random_key();
    let config = WorkerConfig {
        model_id: model_id.to_owned(),
        key: key.to_string(),
        dir: files.dir.clone(),
        vision: spec.kind == ModelKind::Vision,
        helper: runtime.entrypoint(),
        max_kv_size: chosen.max_kv_size,
        kv_bits: kv_bits(chosen.kv_cache_type),
    };
    let mut read_only = vec![runtime.dir.clone(), files.dir.clone()];
    read_only.extend(files.blobs.iter().cloned());
    let tokens = u64::from(chosen.max_kv_size);
    Ok(EnginePlan {
        launch: worker_launch("mlx", &config, read_only)?,
        key,
        estimate: llamacpp::cache_estimate(files.sizes, facts, tokens, chosen.kv_cache_type),
        slots: 1,
        probe_tool_template: false,
    })
}

#[cfg(all(feature = "runtime", target_os = "macos", target_arch = "aarch64"))]
mod worker {
    use super::super::{parent_gone, parent_id, read_worker_config, worker_listener};
    use super::WorkerConfig;
    use anyhow::{Context, Result};
    use flow_like_runtime::{
        models::llm::{
            mlx::{MlxEndpoint, MlxRequestLimits, MlxServedModel},
            mlx_pack::MlxModelKind,
        },
        utils::execute::{MlxServiceRuntime, RuntimeLocator},
    };
    use std::{future::Future, path::Path};
    use tokio::signal::unix::{Signal, SignalKind, signal};

    fn locator(config: &WorkerConfig) -> RuntimeLocator {
        RuntimeLocator {
            llama_server: None,
            mlx_service: Some(MlxServiceRuntime {
                entrypoint: config.helper.clone(),
                resources_dir: config
                    .helper
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default(),
            }),
        }
    }

    fn served(config: &WorkerConfig) -> MlxServedModel {
        MlxServedModel {
            directory: config.dir.clone(),
            kind: if config.vision {
                MlxModelKind::Vlm
            } else {
                MlxModelKind::Llm
            },
            limits: MlxRequestLimits {
                max_kv_size: Some(config.max_kv_size),
                kv_bits: config.kv_bits,
                inline_images_only: true,
            },
        }
    }

    /// The outcome of `work`, or `None` when the agent stops this worker or goes away first.
    async fn unless_stopped<T>(
        work: impl Future<Output = T>,
        terminate: &mut Signal,
        parent: u32,
    ) -> Option<T> {
        tokio::select! {
            done = work => Some(done),
            _ = terminate.recv() => None,
            () = parent_gone(parent) => None,
        }
    }

    /// The helper with the model loaded, or `None` when the worker was stopped first.
    async fn loaded(
        config: &WorkerConfig,
        terminate: &mut Signal,
        parent: u32,
    ) -> Result<Option<MlxEndpoint>> {
        let endpoint = MlxEndpoint::start(&locator(config), served(config))
            .await
            .with_context(|| format!("Start the MLX helper of model {}", config.model_id))?;
        let outcome = unless_stopped(endpoint.load(), terminate, parent).await;
        outcome.map(|load| load.map(|()| endpoint)).transpose()
    }

    async fn listen(config: &WorkerConfig, endpoint: &mut MlxEndpoint) -> Result<()> {
        let listener = worker_listener().await?;
        endpoint.serve(listener, &config.key).map(drop)
    }

    /// Reads the configuration from stdin, starts the helper and loads the model before it
    /// listens, so a model that cannot load fails the start. Serves until the agent stops it or
    /// goes away, or the helper exits.
    pub async fn run() -> Result<()> {
        let parent = parent_id();
        let config: WorkerConfig = read_worker_config()?;
        let mut terminate =
            signal(SignalKind::terminate()).context("Watch for the stop of the MLX worker")?;
        let Some(mut endpoint) = loaded(&config, &mut terminate, parent).await? else {
            return Ok(());
        };
        listen(&config, &mut endpoint).await?;
        match unless_stopped(endpoint.exited(), &mut terminate, parent).await {
            Some(()) => anyhow::bail!("The MLX helper of model {} exited", config.model_id),
            None => Ok(()),
        }
    }
}

#[cfg(all(feature = "runtime", target_os = "macos", target_arch = "aarch64"))]
pub use worker::run as run_worker;

/// Starts this test binary as the MLX worker, as the agent binary runs `model-worker --engine
/// mlx`; the configuration still arrives on stdin.
#[cfg(all(
    test,
    feature = "runtime",
    target_os = "macos",
    target_arch = "aarch64"
))]
pub(crate) mod test_worker {
    use super::super::{EngineLaunch, EngineLauncher};
    use anyhow::Result;

    const MODE_VAR: &str = "FLOW_LIKE_TEST_MLX_WORKER";
    const TEST_NAME: &str = "models::engines::mlx::test_worker::mlx_worker_main";

    pub(crate) struct WorkerLauncher;

    impl EngineLauncher for WorkerLauncher {
        fn command(&self, _launch: &EngineLaunch) -> Result<tokio::process::Command> {
            let mut command = tokio::process::Command::new(std::env::current_exe()?);
            command
                .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
                .env(MODE_VAR, "1");
            Ok(command)
        }
    }

    #[test]
    fn mlx_worker_main() {
        if std::env::var_os(MODE_VAR).is_none() {
            return;
        }
        let _ = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .try_init();
        tokio::runtime::Runtime::new()
            .expect("a runtime")
            .block_on(super::run_worker())
            .expect("the MLX worker serves");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::db::RuntimeRecord;
    use flow_like_device_protocol::{
        DigestAlgorithm, ModelAssetDescriptor, ModelAssetDigest, ModelBackend, ModelEngine,
        ModelRuntime,
    };

    fn asset(index: usize, name: &str) -> ModelAssetDescriptor {
        ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Sha256,
                hex: format!("{index:064x}"),
            },
            size: 10,
            file_name: name.into(),
            sources: vec![],
        }
    }

    fn spec(kind: ModelKind, names: &[&str]) -> ModelSpec {
        ModelSpec {
            display_name: "Qwen3 1.7B".into(),
            kind,
            engine: ModelEngine::Mlx,
            assets: names
                .iter()
                .enumerate()
                .map(|(index, name)| asset(index + 1, name))
                .collect(),
            projector: None,
            pooling: None,
        }
    }

    const CHAT_FILES: [&str; 4] = [
        "config.json",
        "tokenizer.json",
        "tokenizer_config.json",
        "model.safetensors",
    ];

    fn qwen_facts() -> GgufFacts {
        GgufFacts {
            block_count: Some(28),
            context_length: Some(40_960),
            embedding_length: Some(2_048),
            head_count: Some(16),
            head_count_kv_total: Some(8 * 28),
            key_length: Some(128),
            value_length: Some(128),
            ..GgufFacts::default()
        }
    }

    fn runtime() -> InstalledRuntime {
        InstalledRuntime {
            record: RuntimeRecord {
                runtime: ModelRuntime::Mlx,
                backend: ModelBackend::Metal,
                build: "b1".into(),
                entrypoint: "bin/flow-like-mlx-service".into(),
                size: 1,
                needs_fallback: false,
                installed_at: 0,
            },
            dir: "/state/runtimes/mlx/b1-metal".into(),
        }
    }

    #[test]
    fn the_worker_gets_its_model_helper_and_limits_on_stdin() -> Result<()> {
        let files = ModelFiles {
            dir: "/state/models/run/qwen".into(),
            blobs: vec!["/state/models/blobs/sha256/1".into()],
            sizes: 1_000_000_000,
        };
        let spec = spec(ModelKind::Chat, &CHAT_FILES);
        let settings = ModelSettings::default();
        let plan = plan("qwen", &spec, &settings, &qwen_facts(), &files, &runtime())?;
        let args: Vec<_> = plan
            .launch
            .args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect();
        assert_eq!(args, ["model-worker", "--engine", "mlx"]);
        assert!(!args.iter().any(|arg| arg.contains(plan.key.as_str())));
        let config: Value =
            serde_json::from_slice(plan.launch.stdin.as_deref().expect("a config"))?;
        assert_eq!(config["key"], plan.key.as_str());
        assert_eq!(
            config["helper"],
            "/state/runtimes/mlx/b1-metal/bin/flow-like-mlx-service"
        );
        assert_eq!(config["dir"], "/state/models/run/qwen");
        assert_eq!(config["max_kv_size"], 8_192);
        assert_eq!(config["vision"], false);
        assert!(config.get("kv_bits").is_none());
        assert_eq!(plan.slots, 1);
        assert!(!plan.probe_tool_template);
        assert_eq!(plan.estimate.weights, 1_000_000_000);
        assert_eq!(plan.estimate.kv_cache, 2 * 8 * 28 * 128 * 2 * 8_192);
        assert!(plan.launch.read_only.contains(&runtime().dir));
        Ok(())
    }

    #[test]
    fn settings_set_the_context_and_the_cache_quantization() {
        let facts = qwen_facts();
        let mut settings = ModelSettings {
            ctx_per_slot: Some(2_048),
            kv_cache_type: Some(KvCacheType::Q8),
            ..ModelSettings::default()
        };
        let chosen = effective(&settings, ModelKind::Chat, &facts);
        assert_eq!(chosen.max_kv_size, 2_048);
        assert_eq!(kv_bits(chosen.kv_cache_type), Some(8));
        settings.kv_cache_type = Some(KvCacheType::Q4);
        assert_eq!(
            kv_bits(effective(&settings, ModelKind::Chat, &facts).kv_cache_type),
            Some(4)
        );
        let small = GgufFacts {
            context_length: Some(2_048),
            ..facts
        };
        let defaults = effective(&ModelSettings::default(), ModelKind::Vision, &small);
        assert_eq!(defaults.max_kv_size, 2_048);
        assert_eq!(defaults.kv_cache_type, KvCacheType::F16);
    }

    #[test]
    fn models_mlx_cannot_load_are_refused() {
        assert!(check_layout(&spec(ModelKind::Chat, &CHAT_FILES)).is_ok());
        for missing in CHAT_FILES {
            let names: Vec<_> = CHAT_FILES
                .into_iter()
                .filter(|name| *name != missing)
                .collect();
            let error = check_layout(&spec(ModelKind::Chat, &names)).unwrap_err();
            assert!(error.to_string().contains("Qwen3 1.7B"), "{error}");
        }
        let nested = [
            "sub/config.json",
            "tokenizer.json",
            "tokenizer_config.json",
            "a.safetensors",
        ];
        assert!(check_layout(&spec(ModelKind::Chat, &nested)).is_err());
        assert!(check_layout(&spec(ModelKind::Vision, &CHAT_FILES)).is_err());
        let mut vision = CHAT_FILES.to_vec();
        vision.push("preprocessor_config.json");
        assert!(check_layout(&spec(ModelKind::Vision, &vision)).is_ok());
        let shards = [
            "config.json",
            "tokenizer.json",
            "tokenizer_config.json",
            "model-00001-of-00002.SafeTensors",
        ];
        assert!(check_layout(&spec(ModelKind::Chat, &shards)).is_ok());
    }

    #[test]
    fn config_facts_come_from_the_text_config_of_vision_models() -> Result<()> {
        let directory = tempfile::tempdir()?;
        assert_eq!(config_facts(directory.path()), GgufFacts::default());
        std::fs::write(
            directory.path().join(CONFIG_FILE),
            r#"{"model_type": "qwen2_5_vl", "max_position_embeddings": 128000,
                "text_config": {"num_hidden_layers": 36, "num_attention_heads": 16,
                                "num_key_value_heads": 2, "hidden_size": 2048}}"#,
        )?;
        let facts = config_facts(directory.path());
        assert_eq!(facts.block_count, Some(36));
        assert_eq!(facts.context_length, Some(128_000));
        assert_eq!(facts.head_count_kv_total, Some(72));
        assert_eq!(facts.kv_elements_per_token(), Some(72 * 2 * 128));
        std::fs::write(directory.path().join(CONFIG_FILE), b"not json")?;
        assert_eq!(config_facts(directory.path()), GgufFacts::default());
        Ok(())
    }
}
