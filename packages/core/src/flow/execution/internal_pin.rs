use flow_like_types::{Value, json::from_value};
use parking_lot::RwLock;
use serde::de::DeserializeOwned;
use std::{
    collections::HashSet,
    sync::{Arc, OnceLock, Weak},
};

use crate::flow::pin::{Pin, PinType};
use crate::flow::variable::VariableType;

use super::internal_node::InternalNode;

/// InternalPin represents a pin during execution.
///
/// Design: The execution graph is **immutable after construction**. Only the `value` changes
/// during execution. This design allows lock-free access to all graph structure and metadata.
///
/// - All metadata (id, name, type, etc.) - immutable, accessed without locks
/// - Graph connections (connected_to, depends_on) - set once during construction via OnceLock
/// - Only `value` uses RwLock as it changes during execution
pub struct InternalPin {
    // === Immutable metadata (no synchronization needed) ===
    /// Original pin ID
    pub id: String,
    /// Pin name for lookup
    pub name: String,
    /// Input or Output
    pub pin_type: PinType,
    /// Data type (Execution, String, Number, etc.)
    pub data_type: VariableType,
    /// Whether this pin has a default value
    pub has_default: bool,
    /// Cached default value (immutable)
    pub default_value: Option<Value>,
    /// Whether this is a layer relay pin
    pub layer_pin: bool,
    /// Pin ordering index
    pub index: u16,
    /// Reference to parent node (set once after construction via OnceLock)
    node: OnceLock<Weak<InternalNode>>,

    // === Graph structure (set once during construction via OnceLock) ===
    /// Pins this output connects to (for output pins)
    connected_to: OnceLock<Vec<Weak<InternalPin>>>,
    /// Pins this input depends on (for input pins)
    depends_on: OnceLock<Vec<Weak<InternalPin>>>,

    // === Mutable runtime state (needs synchronization) ===
    /// Runtime value - the ONLY field that changes during execution
    pub value: RwLock<Option<Value>>,
}

impl InternalPin {
    /// Create a new InternalPin from a Pin definition.
    /// Graph connections (connected_to, depends_on, node) must be set via init_* methods.
    pub fn new(pin: &Pin, layer_pin: bool) -> Self {
        Self {
            id: pin.id.clone(),
            name: pin.name.clone(),
            pin_type: pin.pin_type.clone(),
            data_type: pin.data_type.clone(),
            has_default: pin.default_value.is_some(),
            default_value: pin
                .default_value
                .as_ref()
                .and_then(|v| flow_like_types::json::from_slice(v).ok()),
            layer_pin,
            index: pin.index,
            node: OnceLock::new(),
            connected_to: OnceLock::new(),
            depends_on: OnceLock::new(),
            value: RwLock::new(None),
        }
    }

    // === One-time initialization methods (called during graph construction) ===

    /// Set the parent node reference (can only be called once)
    pub fn init_node(&self, node: Weak<InternalNode>) {
        let _ = self.node.set(node);
    }

    /// Set the connected_to pins (can only be called once)
    pub fn init_connected_to(&self, pins: Vec<Weak<InternalPin>>) {
        let _ = self.connected_to.set(pins);
    }

    /// Set the depends_on pins (can only be called once)
    pub fn init_depends_on(&self, pins: Vec<Weak<InternalPin>>) {
        let _ = self.depends_on.set(pins);
    }

    // === Accessors for graph structure (returns empty slice if not initialized) ===

    /// Get the parent node
    #[inline]
    pub fn node(&self) -> Option<&Weak<InternalNode>> {
        self.node.get()
    }

    /// Get connected pins
    #[inline]
    pub fn connected_to(&self) -> &[Weak<InternalPin>] {
        self.connected_to.get().map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Get dependency pins
    #[inline]
    pub fn depends_on(&self) -> &[Weak<InternalPin>] {
        self.depends_on.get().map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Reset value for re-execution
    pub async fn reset(&self) {
        *self.value.write() = None;
    }

    // === Value access (the only operations needing synchronization) ===

    /// Set the runtime value
    #[inline]
    pub async fn set_value(&self, value: Value) {
        *self.value.write() = Some(value);
    }

    /// Get value deserialized to type T
    pub async fn get_value<T: DeserializeOwned>(&self) -> Option<T> {
        let guard = self.value.read();
        let value = guard.as_ref()?;
        from_value::<T>(value.clone()).ok()
    }

    /// Get raw value clone
    #[inline]
    pub async fn get_raw_value(&self) -> Option<Value> {
        self.value.read().clone()
    }

    /// Check if value is set
    #[inline]
    pub async fn has_value(&self) -> bool {
        self.value.read().is_some()
    }

    // === Graph traversal (lock-free since graph is immutable) ===

    /// Get all nodes connected through this pin's outputs
    pub fn get_connected_nodes(&self) -> Vec<Arc<InternalNode>> {
        let mut result = Vec::new();
        let mut node_ids = HashSet::new();
        let mut visited_pins: HashSet<*const InternalPin> = HashSet::new();
        let mut stack: Vec<Arc<InternalPin>> = Vec::new();

        // Seed with directly connected pins
        for weak in self.connected_to() {
            if let Some(p) = weak.upgrade() {
                stack.push(p);
            }
        }

        while let Some(pin_arc) = stack.pop() {
            let pin_ptr = Arc::as_ptr(&pin_arc);
            if !visited_pins.insert(pin_ptr) {
                continue;
            }

            if let Some(node_weak) = pin_arc.node() {
                if let Some(node_arc) = node_weak.upgrade() {
                    let id = node_arc.node_id();
                    if node_ids.insert(id.to_string()) {
                        result.push(node_arc);
                    }
                }
            } else {
                // Layer pin - follow through
                for next in pin_arc.connected_to() {
                    if let Some(next_arc) = next.upgrade() {
                        stack.push(next_arc);
                    }
                }
            }
        }

        result
    }

    /// Get all nodes this pin depends on
    pub fn get_dependent_nodes(&self) -> Vec<Arc<InternalNode>> {
        let deps = self.depends_on();
        let seed = deps.len();
        let mut result = Vec::with_capacity(seed);
        let mut node_ids = HashSet::with_capacity(seed * 2);
        let mut visited_pins: HashSet<*const InternalPin> = HashSet::with_capacity(seed * 4);
        let mut stack: Vec<Arc<InternalPin>> = Vec::with_capacity(seed);

        for weak in deps {
            if let Some(p) = weak.upgrade() {
                stack.push(p);
            }
        }

        while let Some(pin_arc) = stack.pop() {
            let pin_ptr = Arc::as_ptr(&pin_arc);
            if !visited_pins.insert(pin_ptr) {
                continue;
            }

            if let Some(node_weak) = pin_arc.node() {
                if let Some(node_arc) = node_weak.upgrade() {
                    let id = node_arc.node_id();
                    if node_ids.insert(id.to_string()) {
                        result.push(node_arc);
                    }
                }
            } else {
                // Layer pin - follow through
                for next in pin_arc.depends_on() {
                    if let Some(next_arc) = next.upgrade() {
                        stack.push(next_arc);
                    }
                }
            }
        }

        result
    }

    /// Get both connected and dependent nodes
    pub fn get_connected_and_dependent_nodes(&self) -> Vec<Arc<InternalNode>> {
        let mut connected = self.get_connected_nodes();
        let dependent = self.get_dependent_nodes();
        connected.extend(dependent);
        connected
    }

    /// Check if this pin's parent node is pure (lock-free)
    pub fn is_pure(&self) -> bool {
        if let Some(node) = self.node()
            && let Some(internal_node) = node.upgrade()
        {
            return internal_node.is_pure_cached();
        }
        // Pins without a parent (layer pins) report as pure
        true
    }

    // === Cached metadata accessors (lock-free) ===

    #[inline]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[inline]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    pub fn pin_type(&self) -> &PinType {
        &self.pin_type
    }

    #[inline]
    pub fn data_type(&self) -> &VariableType {
        &self.data_type
    }

    #[inline]
    pub fn has_default(&self) -> bool {
        self.has_default
    }
}
