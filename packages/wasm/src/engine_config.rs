use crate::error::{WasmError, WasmResult};
use flow_like_wasm_schema::limits::WasmLimits;
use wasmtime::{Config, OptLevel};

/// Engine settings shared by compilation workers and execution hosts.
#[derive(Clone, Debug)]
pub struct EngineConfig {
    pub parallel_compilation: bool,
    pub opt_level: OptLevel,
    pub fuel_metering: bool,
    pub epoch_interruption: bool,
    pub compiler_enabled: bool,
    pub ios_memory_layout: bool,
    pub macos_signal_traps: bool,
    pub target: Option<String>,
    pub limits: WasmLimits,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            parallel_compilation: true,
            opt_level: OptLevel::Speed,
            fuel_metering: true,
            epoch_interruption: true,
            compiler_enabled: true,
            ios_memory_layout: cfg!(target_os = "ios"),
            macos_signal_traps: false,
            target: None,
            limits: WasmLimits::default(),
        }
    }
}

impl EngineConfig {
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    pub fn with_ios_memory_layout(mut self) -> Self {
        self.ios_memory_layout = true;
        self
    }

    pub(crate) fn to_wasmtime_config(&self) -> WasmResult<Config> {
        let mut config = Config::new();
        if cfg!(target_os = "macos") && self.macos_signal_traps {
            config.macos_use_mach_ports(false);
        }

        if self.compiler_enabled {
            config.parallel_compilation(self.parallel_compilation);
            config.cranelift_opt_level(self.opt_level);
        }

        // Runtime settings
        config.consume_fuel(self.fuel_metering);
        config.epoch_interruption(self.epoch_interruption);

        // Enable WASM GC and exception handling proposals (needed for Kotlin/Wasm, etc.)
        config.wasm_gc(true);
        config.wasm_exceptions(true);
        config.wasm_function_references(true);

        // Enable SIMD proposals and wide arithmetic
        config.wasm_simd(true);
        config.wasm_relaxed_simd(true);
        config.wasm_wide_arithmetic(true);

        // Memory settings
        config.memory_init_cow(true);
        if self.ios_memory_layout
            || self
                .target
                .as_deref()
                .is_some_and(|target| target.contains("apple-ios"))
        {
            // iOS can reject Wasmtime's default 4GiB+ linear-memory reservation
            // during component instantiation. Use moving dynamic memories there
            // and keep only a modest growth reservation.
            config.memory_init_cow(false);
            config.memory_reservation(0);
            config.memory_guard_size(0);
            config.memory_reservation_for_growth(64 * 1024 * 1024);
        }

        // Apply resource limits
        let limits = &self.limits;
        config.max_wasm_stack(limits.max_stack_depth as usize * 1024);

        // Explicitly select the host baseline too. Without this, Wasmtime
        // infers CPU features from the compiler machine and its serialized
        // output may be rejected by another machine of the same architecture.
        if let Some(triple) = self.target.as_deref().or_else(|| {
            flow_like_wasm_schema::runtime::portable_target(
                std::env::consts::OS,
                std::env::consts::ARCH,
            )
        }) {
            config.target(triple).map_err(|e| {
                WasmError::compilation(format!("Unsupported compilation target '{triple}': {e}"))
            })?;
        }

        Ok(config)
    }
}
