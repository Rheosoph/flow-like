use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

use ahash::AHashSet;
use flow_like_types::Value;

use super::execution::internal_pin::InternalPin;

/// Tracks the small pin chains seen in normal execution without allocating.
///
/// The hash set is created only when a chain exceeds the inline capacity.
pub(crate) struct InlineVisitedPins<const N: usize> {
    inline: [usize; N],
    len: usize,
    overflow: Option<AHashSet<usize>>,
}

impl<const N: usize> InlineVisitedPins<N> {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            inline: [0; N],
            len: 0,
            overflow: None,
        }
    }

    #[inline]
    pub(crate) fn insert(&mut self, pin: &Arc<InternalPin>) -> bool {
        let key = Arc::as_ptr(pin) as usize;

        if let Some(overflow) = &mut self.overflow {
            return overflow.insert(key);
        }

        if self.inline[..self.len].contains(&key) {
            return false;
        }

        if self.len < N {
            self.inline[self.len] = key;
            self.len += 1;
            return true;
        }

        let mut overflow = AHashSet::with_capacity(N.saturating_mul(2).max(1));
        overflow.extend(self.inline);
        let inserted = overflow.insert(key);
        self.overflow = Some(overflow);
        inserted
    }
}

pub async fn evaluate_pin_value_reference(pin: Arc<InternalPin>) -> flow_like_types::Result<Value> {
    let mut current_pin = pin;
    let mut visited_pins = InlineVisitedPins::<16>::new();

    loop {
        // Check for circular dependencies
        if !visited_pins.insert(&current_pin) {
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
            return Ok(default_value.as_ref().clone());
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
    overrides: &Option<BTreeMap<String, Arc<Value>>>,
) -> flow_like_types::Result<Value> {
    let pin = pin
        .upgrade()
        .ok_or_else(|| flow_like_types::anyhow!("Pin is not set"))?;
    evaluate_pin_value(pin, overrides).await
}

pub async fn evaluate_pin_value(
    pin: Arc<InternalPin>,
    overrides: &Option<BTreeMap<String, Arc<Value>>>,
) -> flow_like_types::Result<Value> {
    let mut current_pin = pin;
    let mut visited_pins = InlineVisitedPins::<16>::new();
    let has_overrides = overrides.is_some();

    loop {
        if !visited_pins.insert(&current_pin) {
            return Err(flow_like_types::anyhow!(
                "Detected circular dependency in pin chain"
            ));
        }

        // Check overrides first — they short-circuit the entire chain
        if let Some(found_override) = overrides.as_ref().and_then(|map| map.get(current_pin.id())) {
            return Ok(found_override.as_ref().clone());
        }

        let deps = current_pin.depends_on();
        let has_deps = deps.first().and_then(|d| d.upgrade());

        // When overrides are active (inside a function context), prefer the
        // dependency chain over shared pin values. Shared pins may carry stale
        // data from previous invocations due to dual-write. Only fall back to
        // shared pin at leaf pins (no deps) where it's the sole value source.
        if !has_overrides && let Some(value) = current_pin.get_raw_value().await {
            return Ok(value);
        }

        if let Some(dep_pin) = has_deps {
            current_pin = dep_pin;
            continue;
        }

        // Leaf pin — no dependency chain to follow. Use shared pin or default.
        if has_overrides && let Some(value) = current_pin.get_raw_value().await {
            return Ok(value);
        }

        if let Some(default_value) = &current_pin.default_value {
            return Ok(default_value.as_ref().clone());
        }

        return Err(flow_like_types::anyhow!(
            "Pin '{}' has no value, dependencies, or default value",
            current_pin.name()
        ));
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::Arc,
    };

    use flow_like_types::{json::json, tokio};

    use super::{InlineVisitedPins, evaluate_pin_value, evaluate_pin_value_reference};
    use crate::flow::{
        execution::internal_pin::InternalPin,
        pin::{Pin, PinType, ValueType},
        variable::VariableType,
    };

    fn internal_pin(id: usize) -> Arc<InternalPin> {
        Arc::new(InternalPin::new(
            &Pin {
                id: format!("pin-{id}"),
                name: format!("pin-{id}"),
                friendly_name: String::new(),
                description: String::new(),
                pin_type: PinType::Input,
                data_type: VariableType::String,
                schema: None,
                value_type: ValueType::Normal,
                depends_on: BTreeSet::new(),
                connected_to: BTreeSet::new(),
                default_value: None,
                index: id as u16,
                options: None,
                value: None,
            },
            false,
        ))
    }

    #[test]
    fn inline_visited_pins_allocates_only_after_capacity() {
        let pins: Vec<_> = (0..17).map(internal_pin).collect();
        let mut visited = InlineVisitedPins::<16>::new();

        for pin in pins.iter().take(16) {
            assert!(visited.insert(pin));
        }
        assert!(visited.overflow.is_none());
        assert!(!visited.insert(&pins[0]));

        assert!(visited.insert(&pins[16]));
        assert!(visited.overflow.is_some());
        assert!(!visited.insert(&pins[16]));
        assert!(!visited.insert(&pins[0]));
    }

    #[tokio::test]
    async fn evaluation_detects_cycles_after_inline_capacity() {
        let pins: Vec<_> = (0..17).map(internal_pin).collect();
        for (index, pin) in pins.iter().enumerate() {
            let next = (index + 1) % pins.len();
            pin.init_depends_on(vec![Arc::downgrade(&pins[next])]);
        }

        let error = evaluate_pin_value(pins[0].clone(), &None)
            .await
            .expect_err("cycle must be rejected");
        assert!(error.to_string().contains("circular dependency"));

        let error = evaluate_pin_value_reference(pins[0].clone())
            .await
            .expect_err("cycle must be rejected");
        assert!(error.to_string().contains("circular dependency"));
    }

    /// The shared cell is the only value source a loop pin has once the reader walks
    /// past its own pin, so a loop that publishes iterations there alone is visible to
    /// every branch sharing the graph. Mirroring into the scope is what keeps a nested
    /// loop's iteration private, and this is the resolution rule that makes it work.
    #[tokio::test]
    async fn an_active_scope_shadows_the_shared_cell_at_the_source_pin() {
        let source = internal_pin(0);
        let consumer = internal_pin(1);
        consumer.init_depends_on(vec![Arc::downgrade(&source)]);
        source.set_value(json!("shared")).await;

        let unscoped = evaluate_pin_value(consumer.clone(), &None)
            .await
            .expect("shared cell resolves without a scope");
        assert_eq!(unscoped, json!("shared"));

        let scope = Some(BTreeMap::from([(
            source.id.to_string(),
            Arc::new(json!("scoped")),
        )]));
        let scoped = evaluate_pin_value(consumer, &scope)
            .await
            .expect("scope resolves through the dependency chain");
        assert_eq!(scoped, json!("scoped"));
    }
}
