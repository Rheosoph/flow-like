pub mod instance;
pub mod linker;

use crate::abi::WasmNodeDefinition;
use crate::engine::WasmEngine;
use crate::error::{WasmError, WasmResult};
use crate::limits::WasmSecurityConfig;
use instance::WasmComponentInstance;
use wasmtime::component::Component;

pub struct WasmComponent {
    component: Component,
    bytes: Vec<u8>,
    hash: String,
    node_definition: parking_lot::RwLock<Option<WasmNodeDefinition>>,
}

impl WasmComponent {
    pub async fn from_bytes(engine: &WasmEngine, bytes: &[u8], hash: String) -> WasmResult<Self> {
        let component = Component::new(engine.engine(), bytes).map_err(|e| {
            WasmError::compilation(format!("Failed to compile WASM component: {}", e))
        })?;

        Ok(Self {
            component,
            bytes: bytes.to_vec(),
            hash,
            node_definition: parking_lot::RwLock::new(None),
        })
    }

    /// Wrap an already-deserialized (AOT-cached) component
    pub fn from_precompiled(
        component: Component,
        original_bytes: &[u8],
        hash: String,
    ) -> WasmResult<Self> {
        Ok(Self {
            component,
            bytes: original_bytes.to_vec(),
            hash,
            node_definition: parking_lot::RwLock::new(None),
        })
    }

    pub fn component(&self) -> &Component {
        &self.component
    }

    pub fn hash(&self) -> &str {
        &self.hash
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub async fn get_node_definition(
        self: &std::sync::Arc<Self>,
        engine: &WasmEngine,
        security: &WasmSecurityConfig,
    ) -> WasmResult<WasmNodeDefinition> {
        {
            let cached = self.node_definition.read();
            if let Some(def) = cached.as_ref() {
                return Ok(def.clone());
            }
        }

        let mut instance =
            WasmComponentInstance::new(engine, self.clone(), security.clone()).await?;
        let definition = instance.call_get_node().await?;

        {
            let mut cache = self.node_definition.write();
            *cache = Some(definition.clone());
        }

        Ok(definition)
    }

    pub async fn instantiate(
        self: &std::sync::Arc<Self>,
        engine: &WasmEngine,
        security: &WasmSecurityConfig,
    ) -> WasmResult<WasmComponentInstance> {
        WasmComponentInstance::new(engine, self.clone(), security.clone()).await
    }
}

impl std::fmt::Debug for WasmComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmComponent")
            .field("hash", &self.hash)
            .finish()
    }
}

pub fn is_component_model(bytes: &[u8]) -> bool {
    bytes.len() >= 8
        && bytes[0..4] == [0x00, 0x61, 0x73, 0x6D]
        && bytes[4..8] == [0x0D, 0x00, 0x01, 0x00]
}
