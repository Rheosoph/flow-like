use crate::error::CompilerError;
use flow_like_wasm::compilation::{ExtractedNode, WasmLimits};

/// Allow the main module, shim, WASI adapter, and fixup used by componentized
/// packages. The adapter shares the main memory; the fixup shares the shim table.
pub(crate) fn metadata_limits() -> WasmLimits {
    WasmLimits {
        max_instances: 4,
        // Existing packages need up to 84.5 MiB of initial linear memory and
        // 14,915 entries in a function table before get_nodes can run.
        memory_limit: 128 * 1024 * 1024,
        max_table_elements: 20_000,
        ..WasmLimits::restrictive()
    }
}

/// Extract definitions using the compiler's bounded, closed metadata host.
pub async fn extract_nodes(wasm_bytes: &[u8]) -> Result<Vec<ExtractedNode>, CompilerError> {
    flow_like_wasm::compilation::extract_nodes(wasm_bytes, metadata_limits())
        .await
        .map_err(|error| {
            CompilerError::Compilation(format!("Failed to extract node metadata: {error}"))
        })
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
