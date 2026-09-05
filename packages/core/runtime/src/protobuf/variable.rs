use crate::flow::{
    pin::{PinType, ValueType},
    variable::{Variable, VariableType},
};
use flow_like_types::Value;
use flow_like_types::{FromProto, ToProto, sync::Mutex};
use std::sync::Arc;

impl VariableType {
    pub fn to_proto(&self) -> i32 {
        match self {
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

    pub fn from_proto(value: i32) -> Self {
        match value {
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
            _ => VariableType::Generic, // Default for unknown values
        }
    }
}

impl ValueType {
    pub fn to_proto(&self) -> i32 {
        match self {
            ValueType::Array => 0,
            ValueType::Normal => 1,
            ValueType::HashMap => 2,
            ValueType::HashSet => 3,
        }
    }

    pub fn from_proto(value: i32) -> Self {
        match value {
            0 => ValueType::Array,
            1 => ValueType::Normal,
            2 => ValueType::HashMap,
            3 => ValueType::HashSet,
            _ => ValueType::Normal, // Default
        }
    }
}

impl PinType {
    pub fn to_proto(&self) -> i32 {
        match self {
            PinType::Input => 0,
            PinType::Output => 1,
        }
    }

    pub fn from_proto(value: i32) -> Self {
        match value {
            0 => PinType::Input,
            1 => PinType::Output,
            _ => PinType::Input, // Default
        }
    }
}

impl ToProto<flow_like_types::proto::Variable> for Variable {
    fn to_proto(&self) -> flow_like_types::proto::Variable {
        flow_like_types::proto::Variable {
            id: self.id.clone(),
            name: self.name.clone(),
            category: self.category.clone().unwrap_or_default(),
            description: self.description.clone().unwrap_or_default(),
            default_value: self.default_value.clone().unwrap_or_default(),
            data_type: self.data_type.to_proto(),
            value_type: self.value_type.to_proto(),
            exposed: self.exposed,
            secret: self.secret,
            editable: self.editable,
            hash: self.hash,
            schema: self.schema.clone(),
            runtime_configured: self.runtime_configured,
        }
    }
}

impl FromProto<flow_like_types::proto::Variable> for Variable {
    fn from_proto(proto: flow_like_types::proto::Variable) -> Self {
        Variable {
            id: proto.id,
            name: proto.name,
            category: if proto.category.is_empty() {
                None
            } else {
                Some(proto.category)
            },
            description: if proto.description.is_empty() {
                None
            } else {
                Some(proto.description)
            },
            default_value: if proto.default_value.is_empty() {
                None
            } else {
                Some(proto.default_value)
            },
            data_type: VariableType::from_proto(proto.data_type),
            value_type: ValueType::from_proto(proto.value_type),
            exposed: proto.exposed,
            secret: proto.secret,
            editable: proto.editable,
            value: Arc::new(Mutex::new(Value::Null)),
            hash: proto.hash,
            schema: proto.schema,
            runtime_configured: proto.runtime_configured,
        }
    }
}
