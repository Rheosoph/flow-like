pub mod clear;
pub mod clear_ref;
pub mod get;
pub mod has_key;
pub mod keys;
pub mod make;
pub mod remove;
pub mod remove_ref;
pub mod set;
pub mod set_ref;
pub mod size;
pub mod values;

use flow_like::flow::variable::VariableType;
use flow_like_types::Value;

/// Validates that a value matches the map's declared value type before insertion.
///
/// Keys are always strings, so only the value is checked. Structs are validated
/// structurally (must be an object) — their JSON schema is enforced at connection
/// time and in the variable editor, not here, because the runtime pin does not
/// carry the schema. `Generic`, execution pins, and JSON `null` always pass.
pub fn validate_value_type(value: &Value, expected: &VariableType) -> flow_like_types::Result<()> {
    if value.is_null() {
        return Ok(());
    }

    let matches = match expected {
        VariableType::Generic | VariableType::Execution => true,
        VariableType::String | VariableType::Date => value.is_string(),
        VariableType::Integer => {
            value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|f| f.fract() == 0.0)
        }
        VariableType::Float => value.is_number(),
        VariableType::Boolean => value.is_boolean(),
        VariableType::PathBuf => value.is_string() || value.is_object(),
        VariableType::Struct => value.is_object(),
        // A single byte value is an integer in 0..=255.
        VariableType::Byte => value
            .as_i64()
            .or_else(|| value.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
            .is_some_and(|n| (0..=255).contains(&n)),
    };

    if matches {
        Ok(())
    } else {
        Err(flow_like_types::anyhow!(
            "Map value {value} does not match the map's value type {expected:?}"
        ))
    }
}
