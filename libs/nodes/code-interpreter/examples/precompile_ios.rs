//! Release step for iOS: precompiles a sandbox interpreter to a Pulley64 `.cwasm`.
//!
//! cargo run -p flow-like-catalog-code-interpreter --features execute --example precompile_ios -- <in.wasm> <out.cwasm>
//!
//! The engine settings must match the on-device sandbox engine (fuel metering on, iOS memory
//! layout, Wasmtime 48); a mismatch makes the device reject the artifact on load.

use anyhow::{Context, Result, anyhow, bail};
use flow_like_wasm::{WasmConfig, WasmEngine};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output), None) = (args.next(), args.next(), args.next()) else {
        bail!("usage: precompile_ios <in.wasm> <out.cwasm>");
    };
    let wasm = std::fs::read(&input).with_context(|| format!("read {input}"))?;
    let config = WasmConfig {
        fuel_metering: true,
        cache_dir: None,
        target: Some("pulley64".to_string()),
        ..WasmConfig::production()
    }
    .with_ios_memory_layout();
    let engine = WasmEngine::new(config).map_err(|e| anyhow!("{e}"))?;
    let compiled = engine
        .engine()
        .precompile_module(&wasm)
        .map_err(|e| anyhow!("{e:#}"))?;
    std::fs::write(&output, &compiled).with_context(|| format!("write {output}"))?;
    println!("{output}: {} bytes (pulley64)", compiled.len());
    Ok(())
}
