//! Shared Board ⇄ AST type mapping (core's `VariableType`/`ValueType` → AST `TypeRef`).
//!
//! Used by both `lower` (Board → AST) and `signatures` (Node → signature stub) so the two
//! directions agree on how flow-like types spell out in FlowScript.

use flow_like_ast::model::{Container, TypeRef};

use crate::flow::pin::ValueType;
use crate::flow::variable::VariableType;

pub(crate) fn variable_type_base(ty: &VariableType) -> &'static str {
    match ty {
        VariableType::Execution => "exec",
        VariableType::String => "string",
        VariableType::Integer => "int",
        VariableType::Float => "float",
        VariableType::Boolean => "bool",
        VariableType::Date => "Date",
        VariableType::PathBuf => "Path",
        VariableType::Generic => "any",
        VariableType::Struct => "Struct",
        VariableType::Byte => "bytes",
    }
}

pub(crate) fn value_type_container(value_type: &ValueType) -> Container {
    match value_type {
        ValueType::Normal => Container::Normal,
        ValueType::Array => Container::Array,
        ValueType::HashMap => Container::Map,
        ValueType::HashSet => Container::Set,
    }
}

pub(crate) fn type_ref(data_type: &VariableType, value_type: &ValueType) -> TypeRef {
    TypeRef::new(
        variable_type_base(data_type),
        value_type_container(value_type),
    )
}
