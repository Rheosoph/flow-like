use crate::{BackendChoice, Error, Result};
use burn::tensor::Device;
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

type ResolvedCache = HashMap<(BackendChoice, bool), BackendChoice>;
static RESOLVED: OnceLock<Mutex<ResolvedCache>> = OnceLock::new();

/// Select a usable backend. Auto prefers CUDA, ROCm, WGPU, then CPU.
/// Explicit choices remain strict.
pub fn resolve_backend(choice: &BackendChoice) -> Result<BackendChoice> {
    resolve_backend_with_fallback(choice, false)
}

/// Resolve an automatic or explicit preference by executing a numerical device probe.
/// Successful selections are cached for this process. Explicit GPU fallback moves through
/// lower-priority GPU backends and then CPU; Auto always permits this search.
pub fn resolve_backend_with_fallback(
    choice: &BackendChoice,
    allow_fallback: bool,
) -> Result<BackendChoice> {
    let mut cache = RESOLVED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let key = (choice.clone(), allow_fallback);
    if let Some(resolved) = cache.get(&key) {
        return Ok(resolved.clone());
    }
    let resolved = resolve_with_probe(choice, allow_fallback, &compiled_backends(), |candidate| {
        probe_candidate(
            candidate,
            !matches!(
                choice,
                BackendChoice::Wgpu | BackendChoice::WgpuAdapter { .. }
            ),
        )
        .map(|_| ())
    })?;
    cache.insert((resolved.clone(), false), resolved.clone());
    cache.insert(key, resolved.clone());
    Ok(resolved)
}

fn resolve_with_probe(
    choice: &BackendChoice,
    allow_fallback: bool,
    compiled: &[&str],
    mut probe: impl FnMut(&BackendChoice) -> Result<()>,
) -> Result<BackendChoice> {
    use BackendChoice::*;
    let candidates = match (choice, allow_fallback) {
        (Auto, _) => vec![Cuda { device: 0 }, Rocm { device: 0 }, Wgpu, Cpu],
        (Cuda { device }, true) => vec![choice.clone(), Rocm { device: *device }, Wgpu, Cpu],
        (Rocm { .. }, true) => vec![choice.clone(), Wgpu, Cpu],
        (Wgpu | WgpuAdapter { .. }, true) => vec![choice.clone(), Cpu],
        _ => vec![choice.clone()],
    };
    let mut failures = Vec::new();
    for candidate in candidates {
        let name = match &candidate {
            Cpu => "cpu",
            Wgpu | WgpuAdapter { .. } => "wgpu",
            Cuda { .. } => "cuda",
            Rocm { .. } => "rocm",
            Auto => unreachable!("automatic candidates must be concrete"),
        };
        if !compiled.contains(&name) {
            failures.push(format!("{candidate:?}: not enabled in this build"));
            continue;
        }
        match probe(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) => failures.push(format!("{candidate:?}: {error}")),
        }
    }
    Err(Error::Backend(format!(
        "No usable backend for {choice:?}: {}",
        failures.join("; ")
    )))
}

pub(crate) fn resolve_for_resume(
    requested: &BackendChoice,
    previous: Option<&BackendChoice>,
) -> Result<BackendChoice> {
    resolve_resume_with(requested, previous, resolve_backend)
}

fn resolve_resume_with(
    requested: &BackendChoice,
    previous: Option<&BackendChoice>,
    mut resolve: impl FnMut(&BackendChoice) -> Result<BackendChoice>,
) -> Result<BackendChoice> {
    if matches!(requested, BackendChoice::Auto) {
        if let Some(previous) = previous.filter(|previous| !matches!(previous, BackendChoice::Auto))
        {
            if let Ok(resolved) = resolve(previous) {
                return Ok(resolved);
            }
        }
    }
    resolve(requested)
}

pub(crate) fn device(choice: &BackendChoice, training: bool) -> Result<Device> {
    let device = std::panic::catch_unwind(|| -> Result<Device> {
        #[cfg(ml_backend_wgpu)]
        if matches!(
            choice,
            BackendChoice::Wgpu | BackendChoice::WgpuAdapter { .. }
        ) && Device::enumerate(burn::tensor::DeviceType::Wgpu).is_empty()
        {
            return Err(Error::Backend("No WGPU adapter is available".into()));
        }
        let selected: Device = match choice {
            #[cfg(ml_backend_cpu)]
            BackendChoice::Cpu => Ok(Device::flex()),
            #[cfg(ml_backend_wgpu)]
            BackendChoice::Wgpu => Ok(Device::wgpu(Default::default())),
            #[cfg(ml_backend_wgpu)]
            BackendChoice::WgpuAdapter { kind, device } => {
                use crate::WgpuAdapterKind;
                use burn::tensor::DeviceKind;
                let kind = match kind {
                    WgpuAdapterKind::Discrete => DeviceKind::DiscreteGpu(*device),
                    WgpuAdapterKind::Integrated => DeviceKind::IntegratedGpu(*device),
                    WgpuAdapterKind::Virtual => DeviceKind::VirtualGpu(*device),
                };
                Ok(Device::wgpu(kind))
            }
            #[cfg(ml_backend_cuda)]
            BackendChoice::Cuda { device } => {
                cuda_driver_preflight()?;
                Ok(Device::cuda(*device))
            }
            #[cfg(ml_backend_rocm)]
            BackendChoice::Rocm { device } => {
                // HIP's generated C-ABI wrappers cannot unwind when a driver library is absent.
                crate::rocm_preflight::check()?;
                Ok(Device::rocm(*device))
            }
            #[allow(unreachable_patterns)]
            _ => Err(Error::Backend(format!(
                "{choice:?} is not enabled in this build"
            ))),
        }?;
        // Device constructors are lazy; allocate once so unavailable hardware returns a Result.
        let _ = burn::tensor::Tensor::<1>::zeros([1], &selected).into_data();
        selected
            .sync()
            .map_err(|error| Error::Backend(error.to_string()))?;
        Ok(selected)
    })
    .map_err(|_| {
        Error::Backend(format!(
            "Failed to initialize {choice:?}; verify the device and driver"
        ))
    })??;
    Ok(if training { device.autodiff() } else { device })
}

#[cfg(ml_backend_cuda)]
fn cuda_driver_preflight() -> Result<()> {
    static DRIVER: OnceLock<std::result::Result<libloading::Library, String>> = OnceLock::new();
    DRIVER
        .get_or_init(|| {
            let names: &[&str] = if cfg!(target_os = "windows") {
                &["nvcuda.dll", "cuda.dll"]
            } else if cfg!(target_os = "macos") {
                &["libcuda.dylib", "libnvcuda.dylib"]
            } else {
                &[
                    "libcuda.so",
                    "libcuda.so.1",
                    "libnvcuda.so",
                    "libnvcuda.so.1",
                ]
            };
            for name in names {
                if let Ok(library) = unsafe { libloading::Library::new(name) } {
                    return Ok(library);
                }
            }
            Err("CUDA driver library is unavailable".into())
        })
        .as_ref()
        .map(|_| ())
        .map_err(|error| Error::Backend(error.clone()))
}
/// Backends compiled into this binary. Hardware availability is checked when opening a session.
pub fn compiled_backends() -> Vec<&'static str> {
    let mut result = Vec::new();
    #[cfg(ml_backend_cpu)]
    result.push("cpu");
    #[cfg(ml_backend_wgpu)]
    result.push("wgpu");
    #[cfg(ml_backend_cuda)]
    result.push("cuda");
    #[cfg(ml_backend_rocm)]
    result.push("rocm");
    result
}

/// Execute a forward/backward calculation on the requested device without silently falling back.
pub fn probe_backend(choice: &BackendChoice) -> Result<crate::BackendProbe> {
    let choice = if matches!(choice, BackendChoice::Auto) {
        resolve_backend(choice)?
    } else {
        choice.clone()
    };
    probe_concrete(&choice)
}

fn probe_concrete(choice: &BackendChoice) -> Result<crate::BackendProbe> {
    probe_candidate(choice, false)
}

fn probe_candidate(
    choice: &BackendChoice,
    require_physical_gpu: bool,
) -> Result<crate::BackendProbe> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use burn::tensor::Tensor;
        let device = device(choice, true)?;
        #[cfg(ml_backend_wgpu)]
        if require_physical_gpu
            && matches!(
                choice,
                BackendChoice::Wgpu | BackendChoice::WgpuAdapter { .. }
            )
            && device
                .identity()
                .is_none_or(|identity| identity.physical.is_none())
        {
            return Err(Error::Backend(
                "WGPU selected a software adapter; using the next backend".into(),
            ));
        }
        #[cfg(not(ml_backend_wgpu))]
        let _ = require_physical_gpu;
        let input = Tensor::<2>::from_floats([[1.0, 2.0], [3.0, 4.0]], &device).require_grad();
        let output = input.clone().square().sum();
        let value = output
            .clone()
            .into_data()
            .iter::<f32>()
            .next()
            .ok_or_else(|| Error::Backend("Probe produced no output".into()))?;
        let gradients = output.backward();
        let gradient = input
            .grad(&gradients)
            .ok_or_else(|| Error::Backend("Backend did not produce gradients".into()))?
            .into_data()
            .iter::<f32>()
            .collect::<Vec<_>>();
        device
            .sync()
            .map_err(|error| Error::Backend(error.to_string()))?;
        let mut maximum_error = (value - 30.0).abs();
        for (actual, expected) in gradient.iter().zip([2.0, 4.0, 6.0, 8.0]) {
            maximum_error = maximum_error.max((actual - expected).abs());
        }
        if gradient.len() != 4
            || !value.is_finite()
            || gradient.iter().any(|x| !x.is_finite())
            || !maximum_error.is_finite()
            || maximum_error > 1e-5
        {
            return Err(Error::Backend(format!(
                "Backend numerical probe failed: error={maximum_error}"
            )));
        }
        Ok(crate::BackendProbe {
            backend: choice.clone(),
            device: format!("{device:?}"),
            max_absolute_error: maximum_error,
        })
    }))
    .map_err(|_| {
        Error::Backend(format!(
            "{choice:?} panicked during its forward/backward probe"
        ))
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_resume_reuses_available_backend_and_migrates_when_unavailable() {
        let previous = BackendChoice::Cuda { device: 1 };
        let mut visited = Vec::new();
        let result = resolve_resume_with(&BackendChoice::Auto, Some(&previous), |choice| {
            visited.push(choice.clone());
            if matches!(choice, BackendChoice::Auto) {
                Ok(BackendChoice::Cpu)
            } else {
                Err(Error::Backend("GPU no longer present".into()))
            }
        })
        .unwrap();
        assert_eq!(result, BackendChoice::Cpu);
        assert_eq!(visited, vec![previous.clone(), BackendChoice::Auto]);
        assert_eq!(
            resolve_resume_with(&BackendChoice::Auto, Some(&previous), |choice| {
                assert_eq!(choice, &previous);
                Ok(previous.clone())
            })
            .unwrap(),
            previous
        );
        assert!(
            resolve_resume_with(&previous, Some(&BackendChoice::Cpu), |choice| {
                assert_eq!(choice, &previous);
                Err(Error::Backend("strict GPU unavailable".into()))
            })
            .is_err()
        );
    }

    #[test]
    fn automatic_selection_probes_in_priority_order_and_skips_disabled_backends() {
        let mut visited = Vec::new();
        let chosen = resolve_with_probe(
            &BackendChoice::Auto,
            false,
            &["cuda", "rocm", "wgpu", "cpu"],
            |candidate| {
                visited.push(candidate.clone());
                if matches!(candidate, BackendChoice::Wgpu) {
                    Ok(())
                } else {
                    Err(Error::Backend("missing driver".into()))
                }
            },
        )
        .unwrap();
        assert_eq!(chosen, BackendChoice::Wgpu);
        assert_eq!(
            visited,
            vec![
                BackendChoice::Cuda { device: 0 },
                BackendChoice::Rocm { device: 0 },
                BackendChoice::Wgpu
            ]
        );
        let chosen = resolve_with_probe(&BackendChoice::Auto, false, &["cpu"], |candidate| {
            assert_eq!(*candidate, BackendChoice::Cpu);
            Ok(())
        })
        .unwrap();
        assert_eq!(chosen, BackendChoice::Cpu);
    }

    #[test]
    fn strict_device_errors_and_fallback_preserves_requested_gpu_index() {
        let requested = BackendChoice::Cuda { device: 3 };
        let mut visited = Vec::new();
        assert!(
            resolve_with_probe(&requested, false, &["cuda", "cpu"], |candidate| {
                visited.push(candidate.clone());
                Err(Error::Backend("no such GPU".into()))
            })
            .is_err()
        );
        assert_eq!(visited, vec![requested.clone()]);
        visited.clear();
        let selected =
            resolve_with_probe(&requested, true, &["cuda", "rocm", "cpu"], |candidate| {
                visited.push(candidate.clone());
                if matches!(candidate, BackendChoice::Cpu) {
                    Ok(())
                } else {
                    Err(Error::Backend("no such GPU".into()))
                }
            })
            .unwrap();
        assert_eq!(selected, BackendChoice::Cpu);
        assert_eq!(
            visited,
            vec![
                requested,
                BackendChoice::Rocm { device: 3 },
                BackendChoice::Cpu
            ]
        );
    }
}
