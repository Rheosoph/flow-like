//! llama-server with best-practice defaults; every default yields to the model's settings.

use super::{Announce, EngineLaunch, EnginePlan, MemoryEstimate, gguf::GgufFacts, random_key};
use crate::models::runtime::InstalledRuntime;
use anyhow::{Context, Result};
use flow_like_device_protocol::{
    GpuLayers, KvCacheType, MODEL_MAX_CTX_PER_SLOT, MODEL_MIN_CTX_PER_SLOT, ModelBackend,
    ModelKind, ModelPooling, ModelSettings, ModelSpec, SystemFacts,
};
use serde_json::Value;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

const DEFAULT_CTX: u32 = 4_096;
const MAX_DEFAULT_CTX: u32 = 8_192;
const DEFAULT_EMBEDDING_CTX: u32 = 512;
const CACHE_REUSE_TOKENS: &str = "256";
const OVERHEAD_FLOOR: u64 = 256 * 1024 * 1024;
/// Per-token KV bytes of an unreadable header: about what a dense model of this size needs.
const FALLBACK_KV_BYTES_PER_WEIGHT_BYTE: u64 = 40_000;
const MIN_FALLBACK_KV_BYTES: u64 = 32 * 1024;

/// The files of one hosted model in its run directory, named as the engine expects.
pub struct ModelFiles {
    pub dir: PathBuf,
    /// Where each asset's bytes live, for the sandbox.
    pub blobs: Vec<PathBuf>,
    pub sizes: u64,
}

impl ModelFiles {
    pub fn path(&self, file_name: &str) -> PathBuf {
        self.dir.join(file_name)
    }
}

/// The settings a start uses: the model's own, else what fits this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effective {
    pub ctx_per_slot: u32,
    pub parallel: u8,
    pub kv_cache_type: KvCacheType,
    pub threads: u16,
    pub gpu_layers: GpuLayers,
    pub flash_attn: Option<bool>,
}

fn kv_bytes(elements: u64, kind: KvCacheType) -> u64 {
    match kind {
        KvCacheType::F16 => elements.saturating_mul(2),
        KvCacheType::Q8 => elements.saturating_mul(34) / 32,
        KvCacheType::Q4 => elements.saturating_mul(18) / 32,
    }
}

pub fn estimate(weights: u64, header: &GgufFacts, settings: &Effective) -> MemoryEstimate {
    let tokens = u64::from(settings.ctx_per_slot) * u64::from(settings.parallel);
    cache_estimate(weights, header, tokens, settings.kv_cache_type)
}

/// Weights, a KV cache holding `tokens` of the cache type, and compute buffers.
pub fn cache_estimate(
    weights: u64,
    header: &GgufFacts,
    tokens: u64,
    kv_cache_type: KvCacheType,
) -> MemoryEstimate {
    let per_token = header
        .kv_elements_per_token()
        .map(|elements| kv_bytes(elements, kv_cache_type))
        .unwrap_or_else(|| {
            (weights / FALLBACK_KV_BYTES_PER_WEIGHT_BYTE).max(MIN_FALLBACK_KV_BYTES)
        });
    MemoryEstimate {
        weights,
        kv_cache: per_token.saturating_mul(tokens),
        overhead: OVERHEAD_FLOOR.saturating_add(weights / 20),
    }
}

/// The context per slot a model gets unless set: what it was trained for, at most 8192 tokens.
pub fn default_ctx(kind: ModelKind, header: &GgufFacts) -> u32 {
    let trained = header
        .context_length
        .and_then(|ctx| u32::try_from(ctx).ok())
        .filter(|ctx| *ctx > 0);
    match kind {
        ModelKind::Embedding => trained.unwrap_or(DEFAULT_EMBEDDING_CTX),
        _ => trained.unwrap_or(DEFAULT_CTX),
    }
    .clamp(MODEL_MIN_CTX_PER_SLOT, MAX_DEFAULT_CTX)
}

/// Fills the unset settings: 4 slots on a GPU and 2 on the CPU, the model's context up to
/// 8192 per slot, an f16 cache; then a q8_0 cache when flash attention permits it and fewer
/// slots while the default would not fit `budget`.
pub fn effective(
    settings: &ModelSettings,
    kind: ModelKind,
    header: &GgufFacts,
    weights: u64,
    backend: ModelBackend,
    facts: &SystemFacts,
    budget: u64,
) -> Effective {
    let mut chosen = Effective {
        ctx_per_slot: settings
            .ctx_per_slot
            .unwrap_or_else(|| default_ctx(kind, header))
            .clamp(MODEL_MIN_CTX_PER_SLOT, MODEL_MAX_CTX_PER_SLOT),
        parallel: settings
            .parallel
            .unwrap_or(if backend == ModelBackend::Cpu { 2 } else { 4 }),
        kv_cache_type: settings.kv_cache_type.unwrap_or(KvCacheType::F16),
        threads: settings.threads.unwrap_or(facts.cpu.physical_cores),
        gpu_layers: settings.gpu_layers,
        flash_attn: settings.flash_attn,
    };
    let fits = |chosen: &Effective| estimate(weights, header, chosen).total() <= budget;
    if settings.kv_cache_type.is_none() && settings.flash_attn != Some(false) && !fits(&chosen) {
        chosen.kv_cache_type = KvCacheType::Q8;
    }
    while settings.parallel.is_none() && chosen.parallel > 1 && !fits(&chosen) {
        chosen.parallel -= 1;
    }
    chosen
}

fn cache_type(kind: &KvCacheType) -> &str {
    match kind {
        KvCacheType::F16 => "f16",
        KvCacheType::Q8 => "q8_0",
        KvCacheType::Q4 => "q4_0",
    }
}

fn pooling(pooling: &ModelPooling) -> &str {
    match pooling {
        ModelPooling::Mean => "mean",
        ModelPooling::Cls => "cls",
        ModelPooling::Last => "last",
    }
}

/// The GGUF weights llama-server loads; split models name their first part.
pub fn weights_file(spec: &ModelSpec) -> Result<&str> {
    spec.assets
        .iter()
        .map(|asset| asset.file_name.as_str())
        .find(|name| {
            spec.projector.as_deref() != Some(*name) && name.to_ascii_lowercase().ends_with(".gguf")
        })
        .context("A llama.cpp model needs GGUF weights")
}

pub struct LlamaInputs<'a> {
    pub model_id: &'a str,
    pub spec: &'a ModelSpec,
    pub effective: Effective,
    pub header: &'a GgufFacts,
    pub files: &'a ModelFiles,
    pub runtime: &'a InstalledRuntime,
    pub key_file: &'a Path,
    pub chatml: bool,
}

/// Flash attention follows the setting; a quantized KV cache needs it on.
fn flash_attn(effective: &Effective) -> &str {
    match effective.flash_attn {
        Some(true) => "on",
        Some(false) => "off",
        None if effective.kv_cache_type != KvCacheType::F16 => "on",
        None => "auto",
    }
}

fn gpu_layers(layers: GpuLayers) -> String {
    match layers {
        GpuLayers::Auto => "auto".into(),
        GpuLayers::Count(layers) => layers.to_string(),
    }
}

/// What every model is served with: loopback on a port the kernel picks, which the server
/// announces, behind the key file; the effective settings, llama.cpp's own memory fit for what
/// is unset, and the metrics and slots endpoints.
fn server_args(inputs: &LlamaInputs) -> Result<Vec<String>> {
    let effective = &inputs.effective;
    let weights = inputs.files.path(weights_file(inputs.spec)?);
    let ctx_size = u64::from(effective.ctx_per_slot) * u64::from(effective.parallel);
    Ok(vec![
        "--model".into(),
        weights.to_string_lossy().into_owned(),
        "--alias".into(),
        inputs.model_id.into(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        "0".into(),
        "--api-key-file".into(),
        inputs.key_file.to_string_lossy().into_owned(),
        "--ctx-size".into(),
        ctx_size.to_string(),
        "--parallel".into(),
        effective.parallel.to_string(),
        "--threads".into(),
        effective.threads.to_string(),
        "--n-gpu-layers".into(),
        gpu_layers(effective.gpu_layers),
        "--fit".into(),
        "on".into(),
        "--flash-attn".into(),
        flash_attn(effective).into(),
        "--metrics".into(),
        "--slots".into(),
        "--no-webui".into(),
    ])
}

/// Embedding models embed whole inputs in one batch; chat models get jinja templates,
/// separate reasoning, prompt-cache reuse and their projector.
fn kind_args(inputs: &LlamaInputs) -> Vec<String> {
    let spec = inputs.spec;
    let mut args: Vec<String> = Vec::new();
    match spec.kind {
        ModelKind::Embedding => {
            let batch = inputs.effective.ctx_per_slot.to_string();
            args.extend(["--embedding".into(), "--batch-size".into(), batch.clone()]);
            args.extend(["--ubatch-size".into(), batch]);
            if let Some(kind) = spec.pooling {
                args.extend(["--pooling".into(), pooling(&kind).into()]);
            }
        }
        ModelKind::Chat | ModelKind::Vision => {
            args.extend(["--jinja", "--reasoning-format", "deepseek"].map(String::from));
            args.extend(["--cache-reuse", CACHE_REUSE_TOKENS].map(String::from));
            if let Some(projector) = &spec.projector {
                let path = inputs.files.path(projector);
                args.extend(["--mmproj".into(), path.to_string_lossy().into_owned()]);
            }
            if inputs.chatml {
                args.extend(["--chat-template", "chatml"].map(String::from));
            }
        }
    }
    args
}

pub fn plan(inputs: LlamaInputs) -> Result<EnginePlan> {
    let mut args = server_args(&inputs)?;
    let kv_cache_type = inputs.effective.kv_cache_type;
    if kv_cache_type != KvCacheType::F16 {
        let kind = cache_type(&kv_cache_type);
        args.extend(["--cache-type-k", kind, "--cache-type-v", kind].map(String::from));
    }
    args.extend(kind_args(&inputs));
    let LlamaInputs {
        spec,
        effective,
        header,
        files,
        runtime,
        chatml,
        ..
    } = inputs;
    let mut read_only = vec![runtime.dir.clone(), files.dir.clone()];
    read_only.extend(files.blobs.iter().cloned());
    Ok(EnginePlan {
        launch: EngineLaunch {
            program: runtime.entrypoint(),
            args: args.into_iter().map(OsString::from).collect(),
            env: runtime.env(),
            stdin: None,
            read_only,
            announce: Announce::Stderr,
        },
        key: random_key(),
        estimate: estimate(files.sizes, header, &effective),
        slots: effective.parallel,
        probe_tool_template: spec.kind == ModelKind::Chat && !chatml && spec.projector.is_none(),
    })
}

fn find_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    match value {
        Value::Object(map) => map
            .get(key)
            .and_then(Value::as_str)
            .or_else(|| map.values().find_map(|child| find_string(child, key))),
        Value::Array(values) => values.iter().find_map(|child| find_string(child, key)),
        _ => None,
    }
}

/// Whether `/props` shows a chat template that handles tool calls: the capabilities newer
/// builds report, else markers in the template text.
pub fn props_support_tools(props: &Value) -> bool {
    if let Some(caps) = props.get("chat_template_caps").and_then(Value::as_object) {
        return ["supports_tools", "supports_tool_calls"]
            .iter()
            .any(|cap| caps.get(*cap).and_then(Value::as_bool) == Some(true));
    }
    find_string(props, "chat_template_tool_use").is_some_and(|template| !template.trim().is_empty())
        || find_string(props, "chat_template").is_some_and(|template| {
            let template = template.to_lowercase();
            ["tool", "function"]
                .iter()
                .any(|marker| template.contains(marker))
        })
}

/// Engine gauges from `/metrics` and `/slots`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Gauges {
    pub requests_processing: Option<f64>,
    pub requests_deferred: Option<f64>,
    pub kv_cache_usage_ratio: Option<f64>,
    pub prompt_tokens_per_second: Option<f64>,
    pub predicted_tokens_per_second: Option<f64>,
    pub slots: Option<u8>,
    pub slots_busy: Option<u8>,
}

/// Prometheus text format; only the gauges the host reports are kept.
pub fn parse_metrics(text: &str, gauges: &mut Gauges) {
    for line in text.lines().filter(|line| !line.starts_with('#')) {
        let Some((name, value)) = line.rsplit_once(' ') else {
            continue;
        };
        let Ok(value) = value.trim().parse::<f64>() else {
            continue;
        };
        let slot = match name.trim().strip_prefix("llamacpp:") {
            Some("requests_processing") => &mut gauges.requests_processing,
            Some("requests_deferred") => &mut gauges.requests_deferred,
            Some("kv_cache_usage_ratio") => &mut gauges.kv_cache_usage_ratio,
            Some("prompt_tokens_seconds") => &mut gauges.prompt_tokens_per_second,
            Some("predicted_tokens_seconds") => &mut gauges.predicted_tokens_per_second,
            _ => continue,
        };
        *slot = Some(value);
    }
}

pub fn parse_slots(slots: &Value, gauges: &mut Gauges) {
    let Some(slots) = slots.as_array() else {
        return;
    };
    let busy = slots
        .iter()
        .filter(|slot| slot.get("is_processing").and_then(Value::as_bool) == Some(true))
        .count();
    gauges.slots = u8::try_from(slots.len()).ok();
    gauges.slots_busy = u8::try_from(busy).ok();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{db::RuntimeRecord, engines::gguf};
    use flow_like_device_protocol::{
        CapacityFacts, CpuFacts, DigestAlgorithm, ModelAssetDescriptor, ModelAssetDigest,
        ModelEngine, ModelRuntime,
    };

    const GIB: u64 = 1024 * 1024 * 1024;

    fn facts() -> SystemFacts {
        SystemFacts {
            cpu: CpuFacts {
                brand: "Test CPU".into(),
                arch: "aarch64".into(),
                features: vec![],
                physical_cores: 8,
            },
            ram: CapacityFacts {
                total: 64 * GIB,
                free: 32 * GIB,
            },
            gpus: vec![],
            model_volume: CapacityFacts {
                total: 512 * GIB,
                free: 256 * GIB,
            },
        }
    }

    fn header() -> GgufFacts {
        GgufFacts {
            architecture: Some("llama".into()),
            block_count: Some(32),
            context_length: Some(131_072),
            embedding_length: Some(4096),
            head_count: Some(32),
            head_count_kv_total: Some(8 * 32),
            key_length: None,
            value_length: None,
        }
    }

    fn asset(name: &str) -> ModelAssetDescriptor {
        ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Sha256,
                hex: "a".repeat(64),
            },
            size: 5 * GIB,
            file_name: name.into(),
            sources: vec![],
        }
    }

    fn spec(kind: ModelKind) -> ModelSpec {
        ModelSpec {
            display_name: "Qwen".into(),
            kind,
            engine: ModelEngine::Llamacpp,
            assets: vec![
                asset("qwen-00001-of-00002.gguf"),
                asset("qwen-00002-of-00002.gguf"),
            ],
            projector: None,
            pooling: (kind == ModelKind::Embedding).then_some(ModelPooling::Cls),
        }
    }

    fn runtime() -> InstalledRuntime {
        InstalledRuntime {
            record: RuntimeRecord {
                runtime: ModelRuntime::Llamacpp,
                backend: ModelBackend::Metal,
                build: "b10809".into(),
                entrypoint: "llama-server".into(),
                size: 1,
                needs_fallback: false,
                installed_at: 0,
            },
            dir: "/state/runtimes/llamacpp/b10809-metal".into(),
        }
    }

    fn value<'a>(args: &'a [OsString], flag: &str) -> Option<&'a str> {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|index| args.get(index + 1))
            .and_then(|value| value.to_str())
    }

    #[test]
    fn defaults_follow_the_model_and_the_budget() {
        let roomy = effective(
            &ModelSettings::default(),
            ModelKind::Chat,
            &header(),
            5 * GIB,
            ModelBackend::Metal,
            &facts(),
            64 * GIB,
        );
        assert_eq!(roomy.ctx_per_slot, 8_192);
        assert_eq!(roomy.parallel, 4);
        assert_eq!(roomy.kv_cache_type, KvCacheType::F16);
        assert_eq!(roomy.threads, 8);
        let kv = estimate(5 * GIB, &header(), &roomy).kv_cache;
        assert_eq!(kv, 128 * 1024 * 8_192 * 4);

        let tight = effective(
            &ModelSettings::default(),
            ModelKind::Chat,
            &header(),
            5 * GIB,
            ModelBackend::Metal,
            &facts(),
            6 * GIB,
        );
        assert_eq!(tight.kv_cache_type, KvCacheType::Q8);
        assert!(tight.parallel < 4);

        let pinned = ModelSettings {
            parallel: Some(8),
            kv_cache_type: Some(KvCacheType::F16),
            ..ModelSettings::default()
        };
        let kept = effective(
            &pinned,
            ModelKind::Chat,
            &header(),
            5 * GIB,
            ModelBackend::Cpu,
            &facts(),
            GIB,
        );
        assert_eq!((kept.parallel, kept.kv_cache_type), (8, KvCacheType::F16));
    }

    #[test]
    fn tight_memory_keeps_an_unquantized_cache_when_flash_attention_is_off() -> Result<()> {
        let settings = ModelSettings {
            flash_attn: Some(false),
            ..ModelSettings::default()
        };
        let header = header();
        let mut files = files();
        files.sizes = 5 * GIB;
        let effective = effective(
            &settings,
            ModelKind::Chat,
            &header,
            files.sizes,
            ModelBackend::Metal,
            &facts(),
            7 * GIB,
        );
        assert_eq!(effective.kv_cache_type, KvCacheType::F16);
        assert_eq!(effective.parallel, 1);
        assert!(estimate(files.sizes, &header, &effective).total() <= 7 * GIB);
        let plan = plan(LlamaInputs {
            model_id: "qwen",
            spec: &spec(ModelKind::Chat),
            effective,
            header: &header,
            files: &files,
            runtime: &runtime(),
            key_file: Path::new("/state/models/run/qwen/key"),
            chatml: false,
        })?;
        assert_eq!(value(&plan.launch.args, "--flash-attn"), Some("off"));
        assert!(!has(&plan.launch.args, "--cache-type-k"));
        assert!(!has(&plan.launch.args, "--cache-type-v"));
        Ok(())
    }

    fn files() -> ModelFiles {
        ModelFiles {
            dir: "/state/models/run/qwen".into(),
            blobs: vec!["/state/models/blobs/sha256/a".into()],
            sizes: 10 * GIB,
        }
    }

    fn planned(kind: ModelKind, settings: &ModelSettings) -> Result<EnginePlan> {
        let header = gguf::GgufFacts::default();
        let files = files();
        let effective = effective(
            settings,
            kind,
            &header,
            files.sizes,
            ModelBackend::Metal,
            &facts(),
            64 * GIB,
        );
        plan(LlamaInputs {
            model_id: "qwen",
            spec: &spec(kind),
            effective,
            header: &header,
            files: &files,
            runtime: &runtime(),
            key_file: Path::new("/state/models/run/qwen/key"),
            chatml: false,
        })
    }

    fn has(args: &[OsString], flag: &str) -> bool {
        args.iter().any(|arg| arg == flag)
    }

    #[test]
    fn chat_arguments_follow_best_practice() -> Result<()> {
        let chat = planned(ModelKind::Chat, &ModelSettings::default())?;
        let args = &chat.launch.args;
        assert_eq!(
            value(args, "--model"),
            Some("/state/models/run/qwen/qwen-00001-of-00002.gguf")
        );
        assert_eq!(value(args, "--host"), Some("127.0.0.1"));
        assert_eq!(
            value(args, "--port"),
            Some("0"),
            "the kernel picks the port"
        );
        assert_eq!(chat.launch.announce, Announce::Stderr);
        assert_eq!(value(args, "--alias"), Some("qwen"));
        assert_eq!(value(args, "--ctx-size"), Some("16384"));
        assert_eq!(value(args, "--parallel"), Some("4"));
        assert_eq!(value(args, "--n-gpu-layers"), Some("auto"));
        assert_eq!(value(args, "--fit"), Some("on"));
        assert_eq!(value(args, "--flash-attn"), Some("auto"));
        assert_eq!(value(args, "--reasoning-format"), Some("deepseek"));
        assert_eq!(value(args, "--cache-reuse"), Some("256"));
        for flag in ["--metrics", "--slots", "--jinja", "--no-webui"] {
            assert!(has(args, flag), "{flag}");
        }
        assert!(!has(args, "--api-key") && !has(args, "--cache-type-k"));
        assert!(chat.probe_tool_template);
        assert!(chat.launch.read_only.contains(&files().blobs[0]));
        Ok(())
    }

    #[test]
    fn embedding_arguments_embed_whole_inputs() -> Result<()> {
        let embedding = planned(ModelKind::Embedding, &ModelSettings::default())?;
        let args = &embedding.launch.args;
        assert_eq!(value(args, "--pooling"), Some("cls"));
        assert_eq!(value(args, "--ubatch-size"), Some("512"));
        assert!(has(args, "--embedding"));
        assert!(!has(args, "--reasoning-format") && !has(args, "--jinja"));
        assert!(!embedding.probe_tool_template);
        Ok(())
    }

    #[test]
    fn a_quantized_cache_turns_flash_attention_on() -> Result<()> {
        let settings = ModelSettings {
            kv_cache_type: Some(KvCacheType::Q8),
            gpu_layers: GpuLayers::Count(20),
            ..ModelSettings::default()
        };
        let chat = planned(ModelKind::Chat, &settings)?;
        let args = &chat.launch.args;
        assert_eq!(value(args, "--cache-type-k"), Some("q8_0"));
        assert_eq!(value(args, "--cache-type-v"), Some("q8_0"));
        assert_eq!(value(args, "--flash-attn"), Some("on"));
        assert_eq!(value(args, "--n-gpu-layers"), Some("20"));
        Ok(())
    }

    #[test]
    fn gauges_come_from_metrics_and_slots() {
        let mut gauges = Gauges::default();
        parse_metrics(
            "# HELP x\nllamacpp:requests_processing 2\nllamacpp:requests_deferred 1\n\
             llamacpp:kv_cache_usage_ratio 0.5\nllamacpp:predicted_tokens_seconds 42.5\nother 3\n",
            &mut gauges,
        );
        parse_slots(
            &serde_json::json!([{"id": 0, "is_processing": true}, {"id": 1, "is_processing": false}]),
            &mut gauges,
        );
        assert_eq!(gauges.requests_processing, Some(2.0));
        assert_eq!(gauges.requests_deferred, Some(1.0));
        assert_eq!(gauges.kv_cache_usage_ratio, Some(0.5));
        assert_eq!(gauges.predicted_tokens_per_second, Some(42.5));
        assert_eq!((gauges.slots, gauges.slots_busy), (Some(2), Some(1)));
        assert!(props_support_tools(
            &serde_json::json!({"chat_template": "{% if tools %}..."})
        ));
        assert!(!props_support_tools(
            &serde_json::json!({"chat_template": "{{ messages }}"})
        ));
        let caps = |tools: bool| {
            serde_json::json!({"chat_template": "{% if tools %}...",
                               "chat_template_caps": {"supports_tools": tools,
                                                      "supports_tool_calls": tools}})
        };
        assert!(props_support_tools(&caps(true)));
        assert!(!props_support_tools(&caps(false)));
    }
}
