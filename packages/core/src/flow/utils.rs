use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

use flow_like_types::Value;

use super::execution::internal_pin::InternalPin;

pub async fn evaluate_pin_value_reference(pin: Arc<InternalPin>) -> flow_like_types::Result<Value> {
    let mut current_pin = pin;
    let mut visited_pins = std::collections::HashSet::with_capacity(8);

    loop {
        // Get pin ID for cycle detection
        let pin_id = current_pin.id();

        // Check for circular dependencies
        if !visited_pins.insert(pin_id.to_string()) {
            return Err(flow_like_types::anyhow!(
                "Detected circular dependency in pin chain"
            ));
        }

        // Case 1: Pin has a value - directly return from here
        if let Some(value) = current_pin.get_raw_value().await {
            return Ok(value);
        }

        // Case 2: Pin depends on another pin
        let deps = current_pin.depends_on();
        if let Some(first_dep) = deps.first()
            && let Some(dep_pin) = first_dep.upgrade()
        {
            current_pin = dep_pin;
            continue;
        }

        // Case 3: Use default value if available
        if let Some(default_value) = &current_pin.default_value {
            return Ok(default_value.clone());
        }

        // Case 4: No value found
        return Err(flow_like_types::anyhow!(
            "Pin '{}' has no value, dependencies, or default value",
            current_pin.name()
        ));
    }
}

pub async fn evaluate_pin_value_weak(
    pin: &Weak<InternalPin>,
    overrides: &Option<BTreeMap<String, Value>>,
) -> flow_like_types::Result<Value> {
    let pin = pin
        .upgrade()
        .ok_or_else(|| flow_like_types::anyhow!("Pin is not set"))?;
    evaluate_pin_value(pin, overrides).await
}

pub async fn evaluate_pin_value(
    pin: Arc<InternalPin>,
    overrides: &Option<BTreeMap<String, Value>>,
) -> flow_like_types::Result<Value> {
    let mut current_pin = pin;
    let mut visited_pins = std::collections::HashSet::with_capacity(8);
    let has_overrides = overrides.is_some();

    loop {
        let pin_id = current_pin.id().to_string();

        if !visited_pins.insert(pin_id.clone()) {
            return Err(flow_like_types::anyhow!(
                "Detected circular dependency in pin chain"
            ));
        }

        // Check overrides first — they short-circuit the entire chain
        if let Some(found_override) = overrides.as_ref().and_then(|map| map.get(&pin_id)) {
            return Ok(found_override.clone());
        }

        let deps = current_pin.depends_on();
        let has_deps = deps.first().and_then(|d| d.upgrade());

        // When overrides are active (inside a function context), prefer the
        // dependency chain over shared pin values. Shared pins may carry stale
        // data from previous invocations due to dual-write. Only fall back to
        // shared pin at leaf pins (no deps) where it's the sole value source.
        if !has_overrides {
            if let Some(value) = current_pin.get_raw_value().await {
                return Ok(value);
            }
        }

        if let Some(dep_pin) = has_deps {
            current_pin = dep_pin;
            continue;
        }

        // Leaf pin — no dependency chain to follow. Use shared pin or default.
        if has_overrides {
            if let Some(value) = current_pin.get_raw_value().await {
                return Ok(value);
            }
        }

        if let Some(default_value) = &current_pin.default_value {
            return Ok(default_value.clone());
        }

        return Err(flow_like_types::anyhow!(
            "Pin '{}' has no value, dependencies, or default value",
            current_pin.name()
        ));
    }
}
