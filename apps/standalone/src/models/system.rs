//! Hardware facts for the backend choice, fit checks and recommendations. GPUs come from an
//! installed llama.cpp runtime's `--list-devices`, else from cheap OS probes.

use anyhow::{Context, Result};
use flow_like_device_protocol::{
    CapacityFacts, CpuFacts, GpuFacts, MODEL_MAX_CPU_FEATURES, MODEL_MAX_GPUS,
    MODEL_TEXT_MAX_BYTES, ModelBackend, ReleaseTarget, SystemFacts,
};
use std::{path::Path, time::Duration};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const MIB: u64 = 1024 * 1024;

pub fn probe(model_volume: &Path, gpus: Vec<GpuFacts>) -> Result<SystemFacts> {
    let facts = SystemFacts {
        cpu: cpu(),
        ram: ram(),
        gpus: gpus.into_iter().take(MODEL_MAX_GPUS).collect(),
        model_volume: volume(model_volume)?,
    };
    facts
        .validate()
        .context("Probe this device's hardware: the facts are out of bounds")?;
    Ok(facts)
}

/// Bounded to `MODEL_TEXT_MAX_BYTES` without control characters.
fn text(value: &str, fallback: &str) -> String {
    let clean: String = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let mut end = clean.len().min(MODEL_TEXT_MAX_BYTES);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    match clean[..end].trim() {
        "" => fallback.to_owned(),
        bounded => bounded.to_owned(),
    }
}

pub fn cpu() -> CpuFacts {
    let system =
        System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()));
    let brand = system.cpus().first().map(|cpu| cpu.brand()).unwrap_or("");
    let cores = System::physical_core_count()
        .or_else(|| std::thread::available_parallelism().ok().map(usize::from))
        .unwrap_or(1);
    CpuFacts {
        brand: text(brand, "Unknown CPU"),
        arch: std::env::consts::ARCH.to_owned(),
        features: cpu_features(),
        physical_cores: u16::try_from(cores).unwrap_or(u16::MAX).max(1),
    }
}

fn cpu_features() -> Vec<String> {
    let mut features = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        macro_rules! detect {
            ($($feature:tt),*) => {$(
                if std::arch::is_x86_feature_detected!($feature) {
                    features.push($feature.to_owned());
                }
            )*};
        }
        detect!(
            "sse4.2",
            "avx",
            "avx2",
            "fma",
            "f16c",
            "avx512f",
            "avx512bw",
            "avx512vnni",
            "avxvnni"
        );
    }
    #[cfg(target_arch = "aarch64")]
    {
        macro_rules! detect {
            ($($feature:tt),*) => {$(
                if std::arch::is_aarch64_feature_detected!($feature) {
                    features.push($feature.to_owned());
                }
            )*};
        }
        detect!("neon", "dotprod", "fp16", "i8mm", "bf16", "sve", "sve2");
    }
    features.truncate(MODEL_MAX_CPU_FEATURES);
    features
}

pub fn ram() -> CapacityFacts {
    let system = System::new_with_specifics(
        RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let total = system.total_memory();
    CapacityFacts {
        total,
        free: system.available_memory().min(total),
    }
}

fn volume(path: &Path) -> Result<CapacityFacts> {
    let stats = fs2::statvfs(path)
        .with_context(|| format!("Read the capacity of the model volume {}", path.display()))?;
    let total = stats.total_space();
    Ok(CapacityFacts {
        total,
        free: stats.available_space().min(total),
    })
}

fn backend_of(device: &str) -> Option<ModelBackend> {
    let kind = device.trim_end_matches(|character: char| character.is_ascii_digit());
    match kind {
        "MTL" | "Metal" => Some(ModelBackend::Metal),
        "Vulkan" => Some(ModelBackend::Vulkan),
        "CUDA" => Some(ModelBackend::Cuda),
        _ => None,
    }
}

/// Parses `llama-server --list-devices`, e.g. `  MTL0: Apple M4 Max (53084 MiB, 53083 MiB free)`.
/// CPU, BLAS and RPC entries are not GPUs.
pub fn parse_list_devices(output: &str) -> Vec<GpuFacts> {
    output
        .lines()
        .filter_map(|line| {
            let (device, rest) = line.trim().split_once(": ")?;
            let backend = backend_of(device)?;
            let (name, memory) = rest.rsplit_once(" (")?;
            let (total, free) = memory.strip_suffix(" MiB free)")?.split_once(" MiB, ")?;
            let total = total.trim().parse::<u64>().ok()?.saturating_mul(MIB);
            let free = free.trim().parse::<u64>().ok()?.saturating_mul(MIB);
            Some(GpuFacts {
                name: text(name, "GPU"),
                backend,
                memory_total: Some(total),
                memory_free: Some(free.min(total)),
            })
        })
        .take(MODEL_MAX_GPUS)
        .collect()
}

/// Runs an installed llama.cpp entrypoint with `--list-devices`.
pub async fn runtime_gpus(mut command: tokio::process::Command) -> Result<Vec<GpuFacts>> {
    command
        .arg("--list-devices")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(PROBE_TIMEOUT, command.output())
        .await
        .context("List the GPUs of the llama.cpp runtime: it did not answer within 20 s")?
        .context("List the GPUs of the llama.cpp runtime")?;
    anyhow::ensure!(
        output.status.success(),
        "List the GPUs of the llama.cpp runtime: it exited with {}",
        output.status
    );
    Ok(parse_list_devices(&String::from_utf8_lossy(&output.stdout)))
}

/// GPUs only an OS probe shows while an installed runtime listed none: that runtime can't use
/// them, so their memory stays unknown and no fit counts it.
pub fn unseen_by_runtime(gpus: Vec<GpuFacts>) -> Vec<GpuFacts> {
    gpus.into_iter()
        .map(|gpu| GpuFacts {
            memory_total: None,
            memory_free: None,
            ..gpu
        })
        .collect()
}

/// GPUs an OS probe can see before any runtime is installed; memory stays unknown.
pub async fn os_gpus() -> Vec<GpuFacts> {
    #[cfg(target_os = "macos")]
    {
        if std::env::consts::ARCH == "aarch64" {
            return vec![GpuFacts {
                name: cpu().brand,
                backend: ModelBackend::Metal,
                memory_total: None,
                memory_free: None,
            }];
        }
        Vec::new()
    }
    #[cfg(target_os = "linux")]
    {
        let mut gpus = nvidia_smi().await.unwrap_or_default();
        if gpus.is_empty() {
            gpus = drm_gpus();
        }
        gpus.truncate(MODEL_MAX_GPUS);
        gpus
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "linux")]
async fn nvidia_smi() -> Option<Vec<GpuFacts>> {
    let mut command = tokio::process::Command::new("nvidia-smi");
    command
        .args([
            "--query-gpu=name,memory.total,memory.free",
            "--format=csv,noheader,nounits",
        ])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(PROBE_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| parse_nvidia_smi(&String::from_utf8_lossy(&output.stdout)))
}

/// `name, memory.total, memory.free` in MiB, one GPU per line.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_nvidia_smi(output: &str) -> Vec<GpuFacts> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split(',').map(str::trim);
            let name = fields.next()?;
            let total = fields.next()?.parse::<u64>().ok()?.saturating_mul(MIB);
            let free = fields.next()?.parse::<u64>().ok()?.saturating_mul(MIB);
            Some(GpuFacts {
                name: text(name, "NVIDIA GPU"),
                backend: ModelBackend::Vulkan,
                memory_total: Some(total),
                memory_free: Some(free.min(total)),
            })
        })
        .collect()
}

/// `/sys/class/drm/card<N>/device/vendor`, named from the NVIDIA driver where it reports one.
#[cfg(target_os = "linux")]
fn drm_gpus() -> Vec<GpuFacts> {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut cards: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.strip_prefix("card")
                .is_some_and(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()))
        })
        .collect();
    cards.sort_by_key(|entry| entry.file_name());
    let mut nvidia_names = nvidia_driver_names().into_iter();
    cards
        .iter()
        .filter_map(|card| {
            let vendor = std::fs::read_to_string(card.path().join("device/vendor")).ok()?;
            let name = match vendor.trim() {
                "0x10de" => nvidia_names.next().unwrap_or_else(|| "NVIDIA GPU".into()),
                "0x1002" => "AMD GPU".into(),
                "0x8086" => "Intel GPU".into(),
                _ => return None,
            };
            Some(GpuFacts {
                name: text(&name, "GPU"),
                backend: ModelBackend::Vulkan,
                memory_total: None,
                memory_free: None,
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn nvidia_driver_names() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/proc/driver/nvidia/gpus") else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok())
        .map(|e| e.path())
        .collect();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| std::fs::read_to_string(path.join("information")).ok())
        .filter_map(|information| {
            information
                .lines()
                .find_map(|line| line.strip_prefix("Model:"))
                .map(|model| model.trim().to_owned())
        })
        .collect()
}

/// The llama.cpp backend that serves this device best among the published packs: Metal on
/// Apple silicon, Vulkan where a Linux x86_64 host has a GPU, else the CPU.
pub fn recommended_backend(target: ReleaseTarget, gpus: &[GpuFacts]) -> ModelBackend {
    match target {
        ReleaseTarget::MacosAarch64 => ModelBackend::Metal,
        ReleaseTarget::LinuxX86_64 if gpus.iter().any(|gpu| gpu.backend != ModelBackend::Metal) => {
            ModelBackend::Vulkan
        }
        _ => ModelBackend::Cpu,
    }
}

/// Memory the models of this device may hold at once: discrete GPU memory plus three
/// quarters of RAM, or three quarters of unified memory.
pub fn memory_budget(facts: &SystemFacts) -> u64 {
    let ram = facts.ram.total / 4 * 3;
    ram.saturating_add(discrete_memory(facts))
}

/// The memory of the GPUs that do not share RAM.
pub fn discrete_memory(facts: &SystemFacts) -> u64 {
    facts
        .gpus
        .iter()
        .filter(|gpu| gpu.backend != ModelBackend::Metal)
        .filter_map(|gpu| gpu.memory_total)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_devices_output_parses_gpus_only() {
        let output = "Available devices:\n  MTL0: Apple M4 Max (53084 MiB, 53083 MiB free)\n  \
                      BLAS: Accelerate (0 MiB, 0 MiB free)\n  Vulkan1: NVIDIA GeForce RTX 4090 \
                      (Laptop) (24564 MiB, 23000 MiB free)\n  CPU: x86 (1 MiB, 1 MiB free)\n";
        let gpus = parse_list_devices(output);
        assert_eq!(gpus.len(), 2);
        assert_eq!(gpus[0].name, "Apple M4 Max");
        assert_eq!(gpus[0].backend, ModelBackend::Metal);
        assert_eq!(gpus[0].memory_total, Some(53_084 * MIB));
        assert_eq!(gpus[1].name, "NVIDIA GeForce RTX 4090 (Laptop)");
        assert_eq!(gpus[1].backend, ModelBackend::Vulkan);
        assert_eq!(gpus[1].memory_free, Some(23_000 * MIB));
    }

    #[test]
    fn gpus_an_installed_runtime_does_not_list_keep_no_memory() {
        let gpus = unseen_by_runtime(parse_nvidia_smi("NVIDIA GeForce RTX 3090, 24576, 23000\n"));
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 3090");
        assert_eq!(gpus[0].backend, ModelBackend::Vulkan);
        assert_eq!((gpus[0].memory_total, gpus[0].memory_free), (None, None));
    }

    #[test]
    fn nvidia_smi_lines_become_gpus_with_memory() {
        let gpus = parse_nvidia_smi("NVIDIA GeForce RTX 3090, 24576, 23000\nbroken line\n");
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0].name, "NVIDIA GeForce RTX 3090");
        assert_eq!(gpus[0].memory_total, Some(24_576 * MIB));
        assert_eq!(gpus[0].memory_free, Some(23_000 * MIB));
    }

    #[test]
    fn the_probe_answers_valid_facts() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let facts = probe(directory.path(), Vec::new())?;
        assert!(facts.cpu.physical_cores > 0);
        assert!(facts.ram.total > 0);
        assert!(facts.model_volume.total > 0);
        assert_eq!(facts.cpu.arch, std::env::consts::ARCH);
        Ok(())
    }

    #[test]
    fn backend_choice_follows_the_published_packs() {
        let vulkan = GpuFacts {
            name: "GPU".into(),
            backend: ModelBackend::Vulkan,
            memory_total: Some(8 * 1024 * MIB),
            memory_free: None,
        };
        assert_eq!(
            recommended_backend(ReleaseTarget::LinuxX86_64, std::slice::from_ref(&vulkan)),
            ModelBackend::Vulkan
        );
        assert_eq!(
            recommended_backend(ReleaseTarget::LinuxAarch64, &[vulkan]),
            ModelBackend::Cpu
        );
        assert_eq!(
            recommended_backend(ReleaseTarget::MacosAarch64, &[]),
            ModelBackend::Metal
        );
        assert_eq!(
            recommended_backend(ReleaseTarget::LinuxX86_64, &[]),
            ModelBackend::Cpu
        );
    }

    #[test]
    fn text_is_bounded_and_clean() {
        assert_eq!(text("  Apple\u{7} M4 ", "x"), "Apple M4");
        assert_eq!(text("", "Unknown CPU"), "Unknown CPU");
        assert!(text(&"é".repeat(200), "x").len() <= MODEL_TEXT_MAX_BYTES);
    }
}
