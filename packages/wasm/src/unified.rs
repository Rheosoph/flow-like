use crate::abi::{WasmExecutionInput, WasmExecutionResult, WasmNodeDefinition};
use crate::engine::WasmEngine;
use crate::error::WasmResult;
use crate::host_functions::HostState;
use crate::instance::WasmInstance;
use crate::limits::WasmSecurityConfig;
use crate::module::WasmModule;
use std::sync::Arc;

#[cfg(feature = "component-model")]
use crate::component::{instance::WasmComponentInstance, WasmComponent};

/// A loaded WASM binary — either a core module or a Component Model component.
pub enum LoadedWasm {
    Module(Arc<WasmModule>),
    #[cfg(feature = "component-model")]
    Component(Arc<WasmComponent>),
}

impl LoadedWasm {
    pub async fn instantiate(
        &self,
        engine: &WasmEngine,
        security: WasmSecurityConfig,
    ) -> WasmResult<UnifiedInstance> {
        match self {
            LoadedWasm::Module(m) => {
                let instance = WasmInstance::new(engine, m.clone(), security).await?;
                Ok(UnifiedInstance::Module(instance))
            }
            #[cfg(feature = "component-model")]
            LoadedWasm::Component(c) => {
                let instance = WasmComponentInstance::new(engine, c.clone(), security).await?;
                Ok(UnifiedInstance::Component(instance))
            }
        }
    }

    pub fn hash(&self) -> &str {
        match self {
            LoadedWasm::Module(m) => m.hash(),
            #[cfg(feature = "component-model")]
            LoadedWasm::Component(c) => c.hash(),
        }
    }
}

impl Clone for LoadedWasm {
    fn clone(&self) -> Self {
        match self {
            LoadedWasm::Module(m) => LoadedWasm::Module(m.clone()),
            #[cfg(feature = "component-model")]
            LoadedWasm::Component(c) => LoadedWasm::Component(c.clone()),
        }
    }
}

/// A unified WASM instance — wraps either a core module or component instance.
pub enum UnifiedInstance {
    Module(WasmInstance),
    #[cfg(feature = "component-model")]
    Component(WasmComponentInstance),
}

impl UnifiedInstance {
    pub async fn call_get_nodes(&mut self) -> WasmResult<Vec<WasmNodeDefinition>> {
        match self {
            UnifiedInstance::Module(i) => i.call_get_nodes().await,
            #[cfg(feature = "component-model")]
            UnifiedInstance::Component(i) => i.call_get_nodes().await,
        }
    }

    pub async fn call_run(
        &mut self,
        input: &WasmExecutionInput,
    ) -> WasmResult<WasmExecutionResult> {
        match self {
            UnifiedInstance::Module(i) => i.call_run(input).await,
            #[cfg(feature = "component-model")]
            UnifiedInstance::Component(i) => i.call_run(input).await,
        }
    }

    pub fn host_state(&self) -> &HostState {
        match self {
            UnifiedInstance::Module(i) => i.host_state(),
            #[cfg(feature = "component-model")]
            UnifiedInstance::Component(i) => i.host_state(),
        }
    }

    pub fn host_state_mut(&mut self) -> &mut HostState {
        match self {
            UnifiedInstance::Module(i) => i.host_state_mut(),
            #[cfg(feature = "component-model")]
            UnifiedInstance::Component(i) => i.host_state_mut(),
        }
    }

    pub fn is_package(&self) -> bool {
        match self {
            UnifiedInstance::Module(i) => i.is_package(),
            #[cfg(feature = "component-model")]
            UnifiedInstance::Component(_) => true,
        }
    }
}
