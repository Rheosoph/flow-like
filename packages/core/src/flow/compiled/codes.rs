//! Stable u8 codes for the enums embedded in the compiled format.
//!
//! These codes are part of the persisted artifact: never renumber an existing
//! entry — append and bump [`super::format::FORMAT_VERSION`] when a variant is
//! added, so old artifacts either decode identically or are rejected wholesale.

use crate::flow::board::{ExecutionMode, ExecutionStage, LayerCacheScope, LayerType};
use crate::flow::execution::LogLevel;
use crate::flow::node::NodePermission;
use crate::flow::pin::{PinType, ValueType};
use crate::flow::variable::VariableType;
use flow_like_types::{Result, anyhow};

pub fn pin_type_code(v: &PinType) -> u8 {
    match v {
        PinType::Input => 0,
        PinType::Output => 1,
    }
}

pub fn pin_type_from(code: u8) -> Result<PinType> {
    Ok(match code {
        0 => PinType::Input,
        1 => PinType::Output,
        _ => return Err(anyhow!("invalid PinType code {code} in compiled board")),
    })
}

pub fn variable_type_code(v: &VariableType) -> u8 {
    match v {
        VariableType::Execution => 0,
        VariableType::String => 1,
        VariableType::Integer => 2,
        VariableType::Float => 3,
        VariableType::Boolean => 4,
        VariableType::Date => 5,
        VariableType::PathBuf => 6,
        VariableType::Generic => 7,
        VariableType::Struct => 8,
        VariableType::Byte => 9,
    }
}

pub fn variable_type_from(code: u8) -> Result<VariableType> {
    Ok(match code {
        0 => VariableType::Execution,
        1 => VariableType::String,
        2 => VariableType::Integer,
        3 => VariableType::Float,
        4 => VariableType::Boolean,
        5 => VariableType::Date,
        6 => VariableType::PathBuf,
        7 => VariableType::Generic,
        8 => VariableType::Struct,
        9 => VariableType::Byte,
        _ => {
            return Err(anyhow!(
                "invalid VariableType code {code} in compiled board"
            ));
        }
    })
}

pub fn value_type_code(v: &ValueType) -> u8 {
    match v {
        ValueType::Normal => 0,
        ValueType::Array => 1,
        ValueType::HashMap => 2,
        ValueType::HashSet => 3,
    }
}

pub fn value_type_from(code: u8) -> Result<ValueType> {
    Ok(match code {
        0 => ValueType::Normal,
        1 => ValueType::Array,
        2 => ValueType::HashMap,
        3 => ValueType::HashSet,
        _ => return Err(anyhow!("invalid ValueType code {code} in compiled board")),
    })
}

pub fn log_level_code(v: &LogLevel) -> u8 {
    match v {
        LogLevel::Debug => 0,
        LogLevel::Info => 1,
        LogLevel::Warn => 2,
        LogLevel::Error => 3,
        LogLevel::Fatal => 4,
    }
}

pub fn log_level_from(code: u8) -> Result<LogLevel> {
    Ok(match code {
        0 => LogLevel::Debug,
        1 => LogLevel::Info,
        2 => LogLevel::Warn,
        3 => LogLevel::Error,
        4 => LogLevel::Fatal,
        _ => return Err(anyhow!("invalid LogLevel code {code} in compiled board")),
    })
}

pub fn stage_code(v: &ExecutionStage) -> u8 {
    match v {
        ExecutionStage::Dev => 0,
        ExecutionStage::Int => 1,
        ExecutionStage::QA => 2,
        ExecutionStage::PreProd => 3,
        ExecutionStage::Prod => 4,
    }
}

pub fn stage_from(code: u8) -> Result<ExecutionStage> {
    Ok(match code {
        0 => ExecutionStage::Dev,
        1 => ExecutionStage::Int,
        2 => ExecutionStage::QA,
        3 => ExecutionStage::PreProd,
        4 => ExecutionStage::Prod,
        _ => {
            return Err(anyhow!(
                "invalid ExecutionStage code {code} in compiled board"
            ));
        }
    })
}

pub fn execution_mode_code(v: &ExecutionMode) -> u8 {
    match v {
        ExecutionMode::Hybrid => 0,
        ExecutionMode::Remote => 1,
        ExecutionMode::Local => 2,
    }
}

pub fn execution_mode_from(code: u8) -> Result<ExecutionMode> {
    Ok(match code {
        0 => ExecutionMode::Hybrid,
        1 => ExecutionMode::Remote,
        2 => ExecutionMode::Local,
        _ => {
            return Err(anyhow!(
                "invalid ExecutionMode code {code} in compiled board"
            ));
        }
    })
}

pub fn layer_type_code(v: &LayerType) -> u8 {
    match v {
        LayerType::Function => 0,
        LayerType::Macro => 1,
        LayerType::Collapsed => 2,
    }
}

pub fn layer_type_from(code: u8) -> Result<LayerType> {
    Ok(match code {
        0 => LayerType::Function,
        1 => LayerType::Macro,
        2 => LayerType::Collapsed,
        _ => return Err(anyhow!("invalid LayerType code {code} in compiled board")),
    })
}

pub fn layer_cache_scope_code(v: &LayerCacheScope) -> u8 {
    match v {
        LayerCacheScope::App => 0,
        LayerCacheScope::User => 1,
    }
}

pub fn layer_cache_scope_from(code: u8) -> Result<LayerCacheScope> {
    Ok(match code {
        0 => LayerCacheScope::App,
        1 => LayerCacheScope::User,
        _ => {
            return Err(anyhow!(
                "invalid LayerCacheScope code {code} in compiled board"
            ));
        }
    })
}

pub fn node_permission_code(v: &NodePermission) -> u8 {
    match v {
        NodePermission::NetworkHttp => 0,
        NodePermission::NetworkWebsocket => 1,
        NodePermission::NetworkTcp => 2,
        NodePermission::NetworkUdp => 3,
        NodePermission::NetworkDns => 4,
        NodePermission::StorageRead => 5,
        NodePermission::StorageWrite => 6,
        NodePermission::Variables => 7,
        NodePermission::Cache => 8,
        NodePermission::Streaming => 9,
        NodePermission::Models => 10,
        NodePermission::A2ui => 11,
        NodePermission::OAuth => 12,
        NodePermission::Functions => 13,
    }
}

pub fn node_permission_from(code: u8) -> Result<NodePermission> {
    Ok(match code {
        0 => NodePermission::NetworkHttp,
        1 => NodePermission::NetworkWebsocket,
        2 => NodePermission::NetworkTcp,
        3 => NodePermission::NetworkUdp,
        4 => NodePermission::NetworkDns,
        5 => NodePermission::StorageRead,
        6 => NodePermission::StorageWrite,
        7 => NodePermission::Variables,
        8 => NodePermission::Cache,
        9 => NodePermission::Streaming,
        10 => NodePermission::Models,
        11 => NodePermission::A2ui,
        12 => NodePermission::OAuth,
        13 => NodePermission::Functions,
        _ => {
            return Err(anyhow!(
                "invalid NodePermission code {code} in compiled board"
            ));
        }
    })
}
