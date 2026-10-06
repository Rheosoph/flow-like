//! Recommendations of the model host (plan §3.6): a code, its parameters and, where one
//! exists, the one request that fixes it. The device computes them, so its facts stay on it;
//! the client owns the copy for each code.

use super::engines::llamacpp::Gauges;
use flow_like_device_protocol::{
    HostedModel, HostedModelState, KvCacheType, MODEL_MAX_PARALLEL, ModelBackend, ModelEngine,
    ModelKind, ModelRuntime, ModelSettings, ModelsRequest, Recommendation, RecommendationCode,
    RecommendationTier, RecommendationValue, ReleaseTarget, Residency, RuntimeInfo, SystemFacts,
};
use std::collections::BTreeMap;

const PRESSURE_PERCENT: u64 = 90;
const DISK_LOW_PERCENT: u64 = 10;
const QUEUE_P95_MS: u32 = 1_000;
const SLOW_TTFT_MS: u32 = 2_000;
const CACHE_SHARE_PERCENT: u64 = 20;
const IDLE_HOURS: u64 = 24;
const NEAR_LIMIT_SHARE_PERCENT: u64 = 5;

/// What the statistics show of one hosted model.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelUsage {
    pub requests_24h: u64,
    /// The highest hourly p95 of the last hours that served requests.
    pub queue_p95_ms: Option<u32>,
    /// The highest hourly p95 of the last hours that served enough requests to tell.
    pub ttft_p95_ms: Option<u32>,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    /// Hours since the model was last used or configured, once nothing used it for a day.
    pub idle_hours: Option<u64>,
}

/// How close recent requests came to a slot's context, and what twice the context costs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextUse {
    pub ctx_per_slot: u32,
    /// Share of recent requests that used at least 90 % of a slot's context.
    pub near_limit_percent: u64,
    pub suggested_ctx: u32,
    pub extra_bytes: u64,
}

pub struct ModelInputs {
    pub model: HostedModel,
    pub gauges: Option<Gauges>,
    pub usage: ModelUsage,
    pub context: Option<ContextUse>,
}

/// Everything the rules read; `system` carries RAM and model volume as they are now.
pub struct Inputs {
    pub target: ReleaseTarget,
    pub system: SystemFacts,
    /// Installed llama.cpp packs.
    pub installed: Vec<RuntimeInfo>,
    /// Packs of the last verified runtime manifest for this target.
    pub available: Vec<RuntimeInfo>,
    pub models: Vec<ModelInputs>,
    pub container_gpu_hidden: bool,
}

type DeviceRule = fn(&Inputs) -> Option<Recommendation>;
type ModelRule = fn(&Inputs, &ModelInputs) -> Option<Recommendation>;

const DEVICE_RULES: [DeviceRule; 4] = [gpu_unused, memory_pressure, disk_low, container_gpu_hidden];
const MODEL_RULES: [ModelRule; 7] = [
    partial_offload,
    requests_queued,
    kv_pressure,
    ctx_truncation,
    idle_resident,
    slow_ttft,
    cpu_threads,
];

/// Every recommendation, the most urgent first.
pub fn recommend(inputs: &Inputs) -> Vec<Recommendation> {
    let mut found: Vec<Recommendation> = DEVICE_RULES
        .iter()
        .filter_map(|rule| rule(inputs))
        .chain(runtime_outdated(inputs))
        .collect();
    for model in &inputs.models {
        found.extend(MODEL_RULES.iter().filter_map(|rule| rule(inputs, model)));
    }
    found.sort_by(|left, right| order(left).cmp(&order(right)));
    found
}

fn order(recommendation: &Recommendation) -> (RecommendationTier, u8, Option<&str>) {
    (
        recommendation.tier,
        recommendation.code as u8,
        recommendation.model_id.as_deref(),
    )
}

fn number(key: &str, value: u64) -> (String, RecommendationValue) {
    (key.to_owned(), RecommendationValue::Number(value))
}

fn text(key: &str, value: &str) -> (String, RecommendationValue) {
    (key.to_owned(), RecommendationValue::Text(value.to_owned()))
}

fn found(
    code: RecommendationCode,
    tier: RecommendationTier,
    model_id: Option<&str>,
    params: impl IntoIterator<Item = (String, RecommendationValue)>,
    fix: Option<ModelsRequest>,
) -> Recommendation {
    Recommendation {
        code,
        tier,
        model_id: model_id.map(str::to_owned),
        params: params.into_iter().collect::<BTreeMap<_, _>>(),
        fix,
    }
}

/// A `Configure` of the model's current revision with one change applied.
fn configure(
    model: &HostedModel,
    change: impl FnOnce(&mut ModelSettings, &mut Residency),
) -> ModelsRequest {
    let mut settings = model.settings.clone();
    let mut residency = model.residency;
    change(&mut settings, &mut residency);
    ModelsRequest::Configure {
        model_id: model.id.clone(),
        expected_revision: model.revision,
        settings,
        residency,
    }
}

fn percent(part: u64, whole: u64) -> u64 {
    if whole == 0 {
        return 0;
    }
    (u128::from(part) * 100 / u128::from(whole)) as u64
}

fn llamacpp(model: &ModelInputs) -> bool {
    model.model.engine == ModelEngine::Llamacpp
}

fn loaded(model: &HostedModel) -> Option<(u64, u64, u8)> {
    match model.state {
        HostedModelState::Loaded {
            ram_bytes,
            vram_bytes,
            slots,
            ..
        } => Some((ram_bytes, vram_bytes, slots)),
        _ => None,
    }
}

/// A GPU the device would use while every installed llama.cpp pack runs on another backend.
fn gpu_unused(inputs: &Inputs) -> Option<Recommendation> {
    let backend = super::system::recommended_backend(inputs.target, &inputs.system.gpus);
    let gpu = inputs.system.gpus.first()?;
    let offered = inputs
        .available
        .iter()
        .any(|pack| pack.runtime == ModelRuntime::Llamacpp && pack.backend == backend);
    let unused =
        !inputs.installed.is_empty() && inputs.installed.iter().all(|pack| pack.backend != backend);
    (backend != ModelBackend::Cpu && offered && unused).then(|| {
        found(
            RecommendationCode::GpuUnused,
            RecommendationTier::Soon,
            None,
            [text("gpu", &gpu.name)],
            Some(ModelsRequest::InstallRuntime {
                runtime: ModelRuntime::Llamacpp,
                backend,
                manifest_jws: None,
            }),
        )
    })
}

/// RAM nearly used up with models loaded: the largest loaded model loads on demand, or
/// unloads when it already does.
fn memory_pressure(inputs: &Inputs) -> Option<Recommendation> {
    let ram = &inputs.system.ram;
    let used = percent(ram.total.saturating_sub(ram.free), ram.total);
    let largest = inputs
        .models
        .iter()
        .filter_map(|model| loaded(&model.model).map(|(ram, ..)| (ram, &model.model)))
        .max_by(|left, right| left.0.cmp(&right.0).then(right.1.id.cmp(&left.1.id)))?
        .1;
    let fix = match largest.residency {
        Residency::AlwaysOn => configure(largest, |_, residency| {
            *residency = Residency::default();
        }),
        _ => ModelsRequest::Unload {
            model_id: largest.id.clone(),
        },
    };
    (used >= PRESSURE_PERCENT).then(|| {
        found(
            RecommendationCode::MemoryPressure,
            RecommendationTier::Now,
            Some(&largest.id),
            [number("memory_percent", used)],
            Some(fix),
        )
    })
}

fn disk_low(inputs: &Inputs) -> Option<Recommendation> {
    let volume = &inputs.system.model_volume;
    (volume.total > 0 && percent(volume.free, volume.total) < DISK_LOW_PERCENT).then(|| {
        found(
            RecommendationCode::DiskLow,
            RecommendationTier::Soon,
            None,
            [number("free_bytes", volume.free)],
            None,
        )
    })
}

fn container_gpu_hidden(inputs: &Inputs) -> Option<Recommendation> {
    inputs.container_gpu_hidden.then(|| {
        found(
            RecommendationCode::ContainerGpuHidden,
            RecommendationTier::Soon,
            None,
            [],
            None,
        )
    })
}

/// An installed pack whose slot the signed manifest offers in another build.
fn runtime_outdated(inputs: &Inputs) -> Vec<Recommendation> {
    inputs
        .installed
        .iter()
        .filter_map(|installed| {
            let latest = inputs.available.iter().find(|pack| {
                pack.runtime == installed.runtime
                    && pack.backend == installed.backend
                    && pack.build != installed.build
            })?;
            Some(found(
                RecommendationCode::RuntimeOutdated,
                RecommendationTier::Later,
                None,
                [text("latest", &latest.build)],
                Some(ModelsRequest::InstallRuntime {
                    runtime: latest.runtime,
                    backend: latest.backend,
                    manifest_jws: None,
                }),
            ))
        })
        .collect()
}

/// Weights plus cache of a loaded model exceed the memory of the discrete GPUs it runs on.
fn partial_offload(inputs: &Inputs, model: &ModelInputs) -> Option<Recommendation> {
    let (_, vram_bytes, _) = loaded(&model.model)?;
    let discrete = inputs
        .system
        .gpus
        .iter()
        .filter(|gpu| matches!(gpu.backend, ModelBackend::Vulkan | ModelBackend::Cuda))
        .map(|gpu| gpu.memory_total)
        .sum::<Option<u64>>()?;
    (llamacpp(model) && vram_bytes > 0 && discrete > 0 && vram_bytes > discrete).then(|| {
        found(
            RecommendationCode::PartialOffload,
            RecommendationTier::Soon,
            Some(&model.model.id),
            [],
            None,
        )
    })
}

/// Requests wait for a slot now, or waited over a second recently: twice the slots.
fn requests_queued(_: &Inputs, model: &ModelInputs) -> Option<Recommendation> {
    let deferred = model
        .gauges
        .and_then(|gauges| gauges.requests_deferred)
        .map(|deferred| deferred.max(0.0).round() as u64)
        .filter(|deferred| *deferred > 0);
    let wait = model
        .usage
        .queue_p95_ms
        .filter(|wait| *wait >= QUEUE_P95_MS);
    if deferred.is_none() && wait.is_none() {
        return None;
    }
    let fix = loaded(&model.model)
        .map(|(.., slots)| slots)
        .filter(|slots| llamacpp(model) && *slots < MODEL_MAX_PARALLEL)
        .map(|slots| {
            configure(&model.model, |settings, _| {
                settings.parallel = Some(slots.saturating_mul(2).min(MODEL_MAX_PARALLEL));
            })
        });
    let params = deferred
        .map(|deferred| number("deferred", deferred))
        .into_iter()
        .chain(wait.map(|wait| number("queue_p95_ms", u64::from(wait))));
    let tier = if deferred.is_some() {
        RecommendationTier::Now
    } else {
        RecommendationTier::Soon
    };
    Some(found(
        RecommendationCode::RequestsQueued,
        tier,
        Some(&model.model.id),
        params,
        fix,
    ))
}

/// The engine reports its context cache nearly full; an 8-bit cache halves it.
fn kv_pressure(_: &Inputs, model: &ModelInputs) -> Option<Recommendation> {
    let ratio = model.gauges?.kv_cache_usage_ratio?;
    let used = (ratio.clamp(0.0, 1.0) * 100.0).round() as u64;
    let quantized = matches!(
        model.model.settings.kv_cache_type,
        Some(KvCacheType::Q8 | KvCacheType::Q4)
    );
    let fix = (!quantized).then(|| {
        configure(&model.model, |settings, _| {
            settings.kv_cache_type = Some(KvCacheType::Q8);
        })
    });
    (used >= PRESSURE_PERCENT).then(|| {
        found(
            RecommendationCode::KvPressure,
            RecommendationTier::Now,
            Some(&model.model.id),
            [number("kv_percent", used)],
            fix,
        )
    })
}

fn ctx_truncation(_: &Inputs, model: &ModelInputs) -> Option<Recommendation> {
    let context = model.context?;
    let raise = context.near_limit_percent >= NEAR_LIMIT_SHARE_PERCENT
        && context.suggested_ctx > context.ctx_per_slot;
    raise.then(|| {
        found(
            RecommendationCode::CtxTruncation,
            RecommendationTier::Soon,
            Some(&model.model.id),
            [
                number("percent", context.near_limit_percent),
                number("suggested_ctx", u64::from(context.suggested_ctx)),
                number("extra_bytes", context.extra_bytes),
            ],
            Some(configure(&model.model, |settings, _| {
                settings.ctx_per_slot = Some(context.suggested_ctx);
            })),
        )
    })
}

/// Always on, yet nothing used it for a day: on demand frees its memory between uses.
fn idle_resident(_: &Inputs, model: &ModelInputs) -> Option<Recommendation> {
    let hours = model
        .usage
        .idle_hours
        .filter(|hours| *hours >= IDLE_HOURS)?;
    (model.model.residency == Residency::AlwaysOn && model.usage.requests_24h == 0).then(|| {
        found(
            RecommendationCode::IdleResident,
            RecommendationTier::Later,
            Some(&model.model.id),
            [number("idle_hours", hours)],
            Some(configure(&model.model, |_, residency| {
                *residency = Residency::default();
            })),
        )
    })
}

/// Slow first tokens while the prompt cache rarely hits; the engine reuses the cache already,
/// so the copy suggests a fixed system prompt.
fn slow_ttft(_: &Inputs, model: &ModelInputs) -> Option<Recommendation> {
    let ttft = model
        .usage
        .ttft_p95_ms
        .filter(|ttft| *ttft >= SLOW_TTFT_MS)?;
    let usage = &model.usage;
    let misses = percent(usage.cached_tokens, usage.prompt_tokens) < CACHE_SHARE_PERCENT;
    (model.model.kind != ModelKind::Embedding && usage.prompt_tokens > 0 && misses).then(|| {
        found(
            RecommendationCode::SlowTtft,
            RecommendationTier::Later,
            Some(&model.model.id),
            [number("ttft_p95_ms", u64::from(ttft))],
            None,
        )
    })
}

/// A thread count other than the physical cores; the device default is one per core.
fn cpu_threads(inputs: &Inputs, model: &ModelInputs) -> Option<Recommendation> {
    let threads = model.model.settings.threads?;
    let cores = inputs.system.cpu.physical_cores;
    (llamacpp(model) && threads != cores).then(|| {
        found(
            RecommendationCode::CpuThreads,
            RecommendationTier::Later,
            Some(&model.model.id),
            [
                number("threads", u64::from(threads)),
                number("physical_cores", u64::from(cores)),
            ],
            Some(configure(&model.model, |settings, _| {
                settings.threads = None
            })),
        )
    })
}

#[cfg(feature = "runtime")]
pub use gather::{current, live_facts};

/// Reads the inputs from a running host.
#[cfg(feature = "runtime")]
mod gather {
    use super::*;
    use crate::{
        enrollment::unix_time,
        models::{
            db::HostedModelRecord,
            engines::{
                gguf::{self, GgufFacts},
                llamacpp::{self, Effective},
            },
            host::ModelHost,
            stats::Stats,
            system,
        },
    };
    use anyhow::Result;
    use flow_like_device_protocol::{
        GpuFacts, MODEL_MAX_CTX_PER_SLOT, ModelAssetDigest, ModelSpec, StatsStep,
    };
    use std::{
        collections::HashMap,
        path::Path,
        sync::{LazyLock, Mutex},
    };

    const HOUR: i64 = 3_600;
    const RECENT_HOURS: usize = 3;
    const MIN_TTFT_REQUESTS: u64 = 5;
    const MIN_CONTEXT_REQUESTS: usize = 10;
    const IDLE_WINDOW_HOURS: i64 = 7 * 24;

    /// GGUF headers by weights digest; reading one walks the whole tokenizer table.
    static HEADERS: LazyLock<Mutex<HashMap<ModelAssetDigest, GgufFacts>>> =
        LazyLock::new(Default::default);

    /// The host's hardware facts with RAM and the model volume read now.
    pub fn live_facts(host: &ModelHost) -> Result<SystemFacts> {
        system::probe(host.store().root(), host.supervisor().facts().gpus)
    }

    pub fn current(host: &ModelHost) -> Result<Vec<Recommendation>> {
        Ok(recommend(&inputs(host, unix_time()?)?))
    }

    fn inputs(host: &ModelHost, now: i64) -> Result<Inputs> {
        let system = live_facts(host)?;
        let installed: Vec<RuntimeInfo> = host
            .runtimes()
            .installed_of(ModelRuntime::Llamacpp)?
            .iter()
            .map(|runtime| runtime.info())
            .collect();
        let available = host
            .runtimes()
            .available()?
            .into_iter()
            .map(|pack| RuntimeInfo {
                runtime: pack.runtime,
                backend: pack.backend,
                build: pack.build,
                installed: false,
                size: pack.size,
            })
            .collect();
        let backend = installed
            .first()
            .map_or(ModelBackend::Cpu, |pack| pack.backend);
        let records = host.store().with_db(|db| db.hosted_models())?;
        let models = host
            .supervisor()
            .models()
            .into_iter()
            .map(|model| {
                let record = records.iter().find(|record| record.id == model.id);
                model_inputs(host, (&system, backend), record, model, now)
            })
            .collect::<Result<_>>()?;
        Ok(Inputs {
            target: ReleaseTarget::current()?,
            container_gpu_hidden: container_gpu_hidden(&system.gpus),
            system,
            installed,
            available,
            models,
        })
    }

    fn model_inputs(
        host: &ModelHost,
        engine: (&SystemFacts, ModelBackend),
        record: Option<&HostedModelRecord>,
        model: HostedModel,
        now: i64,
    ) -> Result<ModelInputs> {
        let configured_at = record.map_or(now, |record| record.updated_at.max(record.created_at));
        let usage = usage(host.stats(), &model, configured_at, now)?;
        let context = match record {
            Some(record) if wants_context(&model) => context_use(host, engine, record, now)?,
            _ => None,
        };
        Ok(ModelInputs {
            gauges: host.supervisor().gauges(&model.id),
            usage,
            context,
            model,
        })
    }

    fn hour_end(now: i64) -> i64 {
        now - now.rem_euclid(HOUR) + HOUR
    }

    fn usage(
        stats: &Stats,
        model: &HostedModel,
        configured_at: i64,
        now: i64,
    ) -> Result<ModelUsage> {
        let to = hour_end(now);
        let day = stats
            .query(Some(&model.id), to - 24 * HOUR, to, StatsStep::Hour)?
            .series;
        let recent = day.requests.len().saturating_sub(RECENT_HOURS);
        let hours = || recent..day.requests.len();
        let requests_24h = day.requests.iter().sum();
        let idle = requests_24h == 0 && model.residency == Residency::AlwaysOn;
        Ok(ModelUsage {
            requests_24h,
            queue_p95_ms: hours()
                .filter(|hour| day.requests[*hour] > 0)
                .filter_map(|hour| day.queue_wait_p95_ms[hour])
                .max(),
            ttft_p95_ms: hours()
                .filter(|hour| day.requests[*hour] >= MIN_TTFT_REQUESTS)
                .filter_map(|hour| day.ttft_p95_ms[hour])
                .max(),
            prompt_tokens: day.prompt_tokens[recent..].iter().sum(),
            cached_tokens: day.cached_tokens[recent..].iter().sum(),
            idle_hours: if idle {
                idle_hours(stats, &model.id, configured_at, now)?
            } else {
                None
            },
        })
    }

    /// Hours since the last request of the past week, or since the last configuration.
    fn idle_hours(
        stats: &Stats,
        model_id: &str,
        configured_at: i64,
        now: i64,
    ) -> Result<Option<u64>> {
        let to = hour_end(now);
        let from = to - IDLE_WINDOW_HOURS * HOUR;
        let week = stats
            .query(Some(model_id), from, to, StatsStep::Hour)?
            .series;
        let last_used = week
            .requests
            .iter()
            .rposition(|requests| *requests > 0)
            .map_or(0, |hour| from + (hour as i64 + 1) * HOUR);
        Ok(u64::try_from((now - last_used.max(configured_at)) / HOUR).ok())
    }

    fn wants_context(model: &HostedModel) -> bool {
        model.engine == ModelEngine::Llamacpp
            && model.kind != ModelKind::Embedding
            && matches!(model.state, HostedModelState::Loaded { .. })
    }

    /// Tokens each successful request of the last hour held in its slot.
    fn recent_tokens(host: &ModelHost, model_id: &str, now: i64) -> Result<Vec<u64>> {
        let rows = host
            .store()
            .with_db(|db| db.requests_between(now - HOUR, now + 1, Some(model_id)))?;
        Ok(rows
            .iter()
            .filter(|request| request.status < 400)
            .map(|request| {
                request
                    .prompt_tokens
                    .saturating_add(request.completion_tokens)
            })
            .collect())
    }

    /// Recent requests against the context the engine runs with, which the model's settings
    /// or else its GGUF header decide as at load time.
    fn context_use(
        host: &ModelHost,
        (facts, backend): (&SystemFacts, ModelBackend),
        record: &HostedModelRecord,
        now: i64,
    ) -> Result<Option<ContextUse>> {
        let used = recent_tokens(host, &record.id, now)?;
        if used.len() < MIN_CONTEXT_REQUESTS {
            return Ok(None);
        }
        let Some(header) = header(host, &record.spec)? else {
            return Ok(None);
        };
        let weights = record.spec.assets.iter().map(|asset| asset.size).sum();
        let budget = host.supervisor().memory_budget();
        let effective = llamacpp::effective(
            &record.settings,
            record.spec.kind,
            &header,
            weights,
            backend,
            facts,
            budget,
        );
        Ok(Some(context_fit(&used, &header, weights, effective)))
    }

    fn context_fit(
        used: &[u64],
        header: &GgufFacts,
        weights: u64,
        effective: Effective,
    ) -> ContextUse {
        let ctx = effective.ctx_per_slot;
        let near = used
            .iter()
            .filter(|tokens| tokens.saturating_mul(10) >= u64::from(ctx) * 9)
            .count();
        let trained = header
            .context_length
            .and_then(|length| u32::try_from(length).ok())
            .unwrap_or(MODEL_MAX_CTX_PER_SLOT);
        let suggested = ctx
            .saturating_mul(2)
            .min(trained.max(ctx))
            .min(MODEL_MAX_CTX_PER_SLOT);
        let raised = Effective {
            ctx_per_slot: suggested,
            ..effective
        };
        let bytes = |settings: &Effective| llamacpp::estimate(weights, header, settings).total();
        ContextUse {
            ctx_per_slot: ctx,
            near_limit_percent: percent(near as u64, used.len() as u64),
            suggested_ctx: suggested,
            extra_bytes: bytes(&raised).saturating_sub(bytes(&effective)),
        }
    }

    fn header(host: &ModelHost, spec: &ModelSpec) -> Result<Option<GgufFacts>> {
        let Ok(name) = llamacpp::weights_file(spec) else {
            return Ok(None);
        };
        let Some(asset) = spec.assets.iter().find(|asset| asset.file_name == name) else {
            return Ok(None);
        };
        if let Some(cached) = lock!(HEADERS).get(&asset.digest) {
            return Ok(Some(cached.clone()));
        }
        let Some(path) = host.store().path_of(&asset.digest)? else {
            return Ok(None);
        };
        let header = gguf::read(&path)?;
        lock!(HEADERS).insert(asset.digest.clone(), header.clone());
        Ok(Some(header))
    }

    /// A container asked for NVIDIA GPUs, yet none is visible inside it.
    fn container_gpu_hidden(gpus: &[GpuFacts]) -> bool {
        let container = ["/.dockerenv", "/run/.containerenv"]
            .iter()
            .any(|marker| Path::new(marker).exists());
        let requested = ["NVIDIA_VISIBLE_DEVICES", "NVIDIA_DRIVER_CAPABILITIES"]
            .iter()
            .filter_map(std::env::var_os)
            .any(|value| !matches!(value.to_str(), Some("" | "void" | "none")));
        cfg!(target_os = "linux") && gpus.is_empty() && container && requested
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn header(context_length: u64) -> GgufFacts {
            GgufFacts {
                architecture: Some("llama".into()),
                block_count: Some(32),
                context_length: Some(context_length),
                embedding_length: Some(4096),
                head_count: Some(32),
                head_count_kv_total: Some(256),
                key_length: Some(128),
                value_length: Some(128),
            }
        }

        fn effective(ctx_per_slot: u32) -> Effective {
            Effective {
                ctx_per_slot,
                parallel: 4,
                kv_cache_type: KvCacheType::F16,
                threads: 8,
                gpu_layers: flow_like_device_protocol::GpuLayers::Auto,
                flash_attn: None,
            }
        }

        #[test]
        fn context_fit_counts_requests_near_the_slot_limit_and_prices_twice_the_context() {
            let used = [7_400, 7_500, 100, 200, 300, 400, 500, 600, 700, 800];
            let fit = context_fit(&used, &header(32_768), 4_000_000_000, effective(8_192));
            assert_eq!(fit.ctx_per_slot, 8_192);
            assert_eq!(fit.near_limit_percent, 20);
            assert_eq!(fit.suggested_ctx, 16_384);
            let per_token = 256 * 256 * 2;
            assert_eq!(fit.extra_bytes, per_token * 8_192 * 4);
            let trained = context_fit(&used, &header(8_192), 4_000_000_000, effective(8_192));
            assert_eq!(
                trained.suggested_ctx, 8_192,
                "never past the trained context"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{
        CapacityFacts, CpuFacts, DigestAlgorithm, GpuFacts, GpuLayers, ModelAssetDigest,
    };

    const GIB: u64 = 1024 * 1024 * 1024;

    fn system(gpus: Vec<GpuFacts>) -> SystemFacts {
        SystemFacts {
            cpu: CpuFacts {
                brand: "AMD Ryzen 9".into(),
                arch: "x86_64".into(),
                features: vec!["avx2".into()],
                physical_cores: 16,
            },
            ram: CapacityFacts {
                total: 64 * GIB,
                free: 40 * GIB,
            },
            gpus,
            model_volume: CapacityFacts {
                total: 1_000 * GIB,
                free: 500 * GIB,
            },
        }
    }

    fn rtx(memory: Option<u64>) -> GpuFacts {
        GpuFacts {
            name: "NVIDIA GeForce RTX 4090".into(),
            backend: ModelBackend::Vulkan,
            memory_total: memory,
            memory_free: memory,
        }
    }

    fn pack(backend: ModelBackend, build: &str, installed: bool) -> RuntimeInfo {
        RuntimeInfo {
            runtime: ModelRuntime::Llamacpp,
            backend,
            build: build.into(),
            installed,
            size: GIB,
        }
    }

    fn inputs(models: Vec<ModelInputs>) -> Inputs {
        Inputs {
            target: ReleaseTarget::LinuxX86_64,
            system: system(vec![rtx(Some(24 * GIB))]),
            installed: vec![pack(ModelBackend::Vulkan, "b10809", true)],
            available: vec![
                pack(ModelBackend::Cpu, "b10809", false),
                pack(ModelBackend::Vulkan, "b10809", false),
            ],
            models,
            container_gpu_hidden: false,
        }
    }

    fn model(id: &str, state: HostedModelState) -> ModelInputs {
        ModelInputs {
            model: HostedModel {
                id: id.into(),
                display_name: id.into(),
                kind: ModelKind::Chat,
                engine: ModelEngine::Llamacpp,
                assets: vec![ModelAssetDigest {
                    algorithm: DigestAlgorithm::Blake3,
                    hex: "a".repeat(64),
                }],
                asset_count: None,
                settings: ModelSettings::default(),
                residency: Residency::default(),
                revision: 3,
                state,
            },
            gauges: None,
            usage: ModelUsage::default(),
            context: None,
        }
    }

    fn loaded(ram_bytes: u64, vram_bytes: u64) -> HostedModelState {
        HostedModelState::Loaded {
            ram_bytes,
            vram_bytes,
            slots: 4,
            slots_busy: 1,
        }
    }

    fn codes(inputs: &Inputs) -> Vec<RecommendationCode> {
        recommend(inputs).iter().map(|found| found.code).collect()
    }

    fn only(inputs: &Inputs, code: RecommendationCode) -> Recommendation {
        let found: Vec<_> = recommend(inputs)
            .into_iter()
            .filter(|found| found.code == code)
            .collect();
        assert_eq!(found.len(), 1, "{code:?} in {found:?}");
        found.into_iter().next().expect("one recommendation")
    }

    fn configured(found: &Recommendation) -> (&ModelSettings, &Residency, u64) {
        match found.fix.as_ref().expect("a fix") {
            ModelsRequest::Configure {
                settings,
                residency,
                expected_revision,
                ..
            } => (settings, residency, *expected_revision),
            other => panic!("expected a configure fix, not {other:?}"),
        }
    }

    #[test]
    fn a_healthy_device_has_no_recommendations() {
        assert!(recommend(&inputs(vec![model("qwen", loaded(GIB, 6 * GIB))])).is_empty());
    }

    #[test]
    fn a_gpu_beside_only_a_cpu_pack_offers_the_gpu_pack() {
        let mut device = inputs(vec![]);
        device.installed = vec![pack(ModelBackend::Cpu, "b10809", true)];
        let found = only(&device, RecommendationCode::GpuUnused);
        assert_eq!(found.tier, RecommendationTier::Soon);
        assert_eq!(
            found.params["gpu"],
            RecommendationValue::Text("NVIDIA GeForce RTX 4090".into())
        );
        assert_eq!(
            found.fix,
            Some(ModelsRequest::InstallRuntime {
                runtime: ModelRuntime::Llamacpp,
                backend: ModelBackend::Vulkan,
                manifest_jws: None,
            })
        );
        device
            .available
            .retain(|pack| pack.backend == ModelBackend::Cpu);
        assert!(codes(&device).is_empty(), "no pack is offered for the GPU");
        device.installed.clear();
        assert!(
            codes(&device).is_empty(),
            "the first load installs the best pack"
        );
    }

    #[test]
    fn memory_pressure_moves_the_largest_loaded_model_to_on_demand_or_unloads_it() {
        let mut big = model("big", loaded(30 * GIB, 0));
        big.model.residency = Residency::AlwaysOn;
        let mut device = inputs(vec![model("small", loaded(2 * GIB, 0)), big]);
        device.system.ram.free = 4 * GIB;
        let found = only(&device, RecommendationCode::MemoryPressure);
        assert_eq!(found.model_id.as_deref(), Some("big"));
        assert_eq!(
            found.params["memory_percent"],
            RecommendationValue::Number(93)
        );
        let (_, residency, revision) = configured(&found);
        assert_eq!(*residency, Residency::default());
        assert_eq!(revision, 3);
        device.models[1].model.residency = Residency::default();
        assert_eq!(
            only(&device, RecommendationCode::MemoryPressure).fix,
            Some(ModelsRequest::Unload {
                model_id: "big".into()
            })
        );
        device.models.iter_mut().for_each(|model| {
            model.model.state = HostedModelState::Stopped;
        });
        assert!(codes(&device).is_empty(), "nothing to unload");
    }

    #[test]
    fn a_full_model_volume_and_a_hidden_container_gpu_are_named() {
        let mut device = inputs(vec![]);
        device.system.model_volume.free = 50 * GIB;
        device.container_gpu_hidden = true;
        let found = only(&device, RecommendationCode::DiskLow);
        assert_eq!(
            found.params["free_bytes"],
            RecommendationValue::Number(50 * GIB)
        );
        assert_eq!(found.fix, None);
        only(&device, RecommendationCode::ContainerGpuHidden);
    }

    #[test]
    fn an_installed_pack_with_another_signed_build_is_outdated() {
        let mut device = inputs(vec![]);
        device.available[1].build = "b11000".into();
        let found = only(&device, RecommendationCode::RuntimeOutdated);
        assert_eq!(
            found.params["latest"],
            RecommendationValue::Text("b11000".into())
        );
        assert_eq!(found.tier, RecommendationTier::Later);
    }

    #[test]
    fn a_model_larger_than_the_discrete_gpu_is_partly_offloaded() {
        let device = inputs(vec![model("huge", loaded(GIB, 30 * GIB))]);
        let found = only(&device, RecommendationCode::PartialOffload);
        assert_eq!(found.model_id.as_deref(), Some("huge"));
        let mut unknown = inputs(vec![model("huge", loaded(GIB, 30 * GIB))]);
        unknown.system.gpus = vec![rtx(None)];
        assert!(codes(&unknown).is_empty(), "GPU memory is unknown");
    }

    #[test]
    fn queued_requests_double_the_slots() {
        let mut queued = model("qwen", loaded(GIB, 6 * GIB));
        queued.gauges = Some(Gauges {
            requests_deferred: Some(2.0),
            ..Gauges::default()
        });
        queued.usage.queue_p95_ms = Some(1_500);
        let found = only(&inputs(vec![queued]), RecommendationCode::RequestsQueued);
        assert_eq!(found.tier, RecommendationTier::Now);
        assert_eq!(found.params["deferred"], RecommendationValue::Number(2));
        assert_eq!(
            found.params["queue_p95_ms"],
            RecommendationValue::Number(1_500)
        );
        assert_eq!(configured(&found).0.parallel, Some(8));
        let mut waited = model("qwen", loaded(GIB, 6 * GIB));
        waited.usage.queue_p95_ms = Some(1_200);
        let found = only(&inputs(vec![waited]), RecommendationCode::RequestsQueued);
        assert_eq!(found.tier, RecommendationTier::Soon);
        assert!(!found.params.contains_key("deferred"));
        let mut quick = model("qwen", loaded(GIB, 6 * GIB));
        quick.usage.queue_p95_ms = Some(900);
        assert!(codes(&inputs(vec![quick])).is_empty());
    }

    #[test]
    fn a_full_context_cache_offers_an_eight_bit_cache() {
        let mut full = model("qwen", loaded(GIB, 6 * GIB));
        full.gauges = Some(Gauges {
            kv_cache_usage_ratio: Some(0.93),
            ..Gauges::default()
        });
        let found = only(&inputs(vec![full]), RecommendationCode::KvPressure);
        assert_eq!(found.params["kv_percent"], RecommendationValue::Number(93));
        assert_eq!(configured(&found).0.kv_cache_type, Some(KvCacheType::Q8));
    }

    #[test]
    fn prompts_near_the_context_limit_raise_the_context() {
        let mut long = model("qwen", loaded(GIB, 6 * GIB));
        long.context = Some(ContextUse {
            ctx_per_slot: 8_192,
            near_limit_percent: 12,
            suggested_ctx: 16_384,
            extra_bytes: 2 * GIB,
        });
        let found = only(&inputs(vec![long]), RecommendationCode::CtxTruncation);
        assert_eq!(found.params["percent"], RecommendationValue::Number(12));
        assert_eq!(
            found.params["suggested_ctx"],
            RecommendationValue::Number(16_384)
        );
        assert_eq!(
            found.params["extra_bytes"],
            RecommendationValue::Number(2 * GIB)
        );
        assert_eq!(configured(&found).0.ctx_per_slot, Some(16_384));
        let mut rare = model("qwen", loaded(GIB, 6 * GIB));
        rare.context = Some(ContextUse {
            near_limit_percent: 2,
            ..long_context()
        });
        assert!(codes(&inputs(vec![rare])).is_empty());
    }

    fn long_context() -> ContextUse {
        ContextUse {
            ctx_per_slot: 8_192,
            near_limit_percent: 12,
            suggested_ctx: 16_384,
            extra_bytes: 2 * GIB,
        }
    }

    #[test]
    fn an_always_on_model_nobody_used_for_a_day_goes_on_demand() {
        let mut idle = model("qwen", loaded(GIB, 6 * GIB));
        idle.model.residency = Residency::AlwaysOn;
        idle.usage.idle_hours = Some(30);
        let found = only(&inputs(vec![idle]), RecommendationCode::IdleResident);
        assert_eq!(found.params["idle_hours"], RecommendationValue::Number(30));
        assert_eq!(*configured(&found).1, Residency::default());
        let mut demand = model("qwen", loaded(GIB, 6 * GIB));
        demand.usage.idle_hours = Some(30);
        assert!(codes(&inputs(vec![demand])).is_empty());
    }

    #[test]
    fn slow_first_tokens_with_cache_misses_are_named() {
        let mut slow = model("qwen", loaded(GIB, 6 * GIB));
        slow.usage.ttft_p95_ms = Some(4_000);
        slow.usage.prompt_tokens = 100_000;
        slow.usage.cached_tokens = 5_000;
        let found = only(&inputs(vec![slow]), RecommendationCode::SlowTtft);
        assert_eq!(
            found.params["ttft_p95_ms"],
            RecommendationValue::Number(4_000)
        );
        let mut cached = model("qwen", loaded(GIB, 6 * GIB));
        cached.usage.ttft_p95_ms = Some(4_000);
        cached.usage.prompt_tokens = 100_000;
        cached.usage.cached_tokens = 60_000;
        assert!(codes(&inputs(vec![cached])).is_empty());
    }

    #[test]
    fn a_thread_count_other_than_the_cores_returns_to_the_default() {
        let mut pinned = model("qwen", loaded(GIB, 6 * GIB));
        pinned.model.settings.threads = Some(4);
        pinned.model.settings.gpu_layers = GpuLayers::Count(20);
        let found = only(&inputs(vec![pinned]), RecommendationCode::CpuThreads);
        assert_eq!(found.params["threads"], RecommendationValue::Number(4));
        assert_eq!(
            found.params["physical_cores"],
            RecommendationValue::Number(16)
        );
        let (settings, ..) = configured(&found);
        assert_eq!(settings.threads, None);
        assert_eq!(
            settings.gpu_layers,
            GpuLayers::Count(20),
            "other settings stay"
        );
    }

    #[test]
    fn recommendations_validate_and_come_most_urgent_first() {
        let mut queued = model("a-model", loaded(30 * GIB, 30 * GIB));
        queued.gauges = Some(Gauges {
            requests_deferred: Some(1.0),
            kv_cache_usage_ratio: Some(0.95),
            ..Gauges::default()
        });
        queued.model.settings.threads = Some(2);
        let mut device = inputs(vec![queued]);
        device.system.ram.free = GIB;
        device.system.model_volume.free = GIB;
        device.available[1].build = "b11000".into();
        let found = recommend(&device);
        assert!(found.len() >= 6);
        for recommendation in &found {
            recommendation.validate().expect("a valid recommendation");
        }
        let tiers: Vec<_> = found.iter().map(|found| found.tier).collect();
        let mut sorted = tiers.clone();
        sorted.sort();
        assert_eq!(tiers, sorted);
        assert_eq!(found[0].tier, RecommendationTier::Now);
    }
}
