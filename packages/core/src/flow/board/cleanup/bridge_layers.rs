use std::collections::{BTreeSet, HashMap, HashSet};

use flow_like_types::create_id;

use crate::flow::{
    board::{
        Board, Layer,
        cleanup::{BoardCleanupLogic, NodeOrLayer, NodeOrLayerRef, PinLookup},
    },
    pin::{Pin, PinType},
};

/// Plan for creating bridge pins for a single internal pin
/// Tracks which external connections need to be bridged
#[derive(Default)]
struct BridgePlan {
    /// External pins that the internal pin connects TO (outgoing)
    outside_connected_to: BTreeSet<String>,
    /// External pins that the internal pin depends ON (incoming)
    outside_depends_on: BTreeSet<String>,
}

/// Bridge Layers Cleanup Logic
///
/// This cleanup step handles the creation of "bridge pins" on layer boundaries.
/// When nodes are collapsed into a layer, internal pins may have connections to
/// external pins. Bridge pins are created on the layer to mediate these connections.
///
/// ## Purpose
/// - Find internal pins with external connections that are not already bridged
/// - Create bridge pins to connect internal and external pins
/// - Maintain proper execution flow without circular dependencies
///
/// ## Bridge Pin Types
/// - **Unidirectional**: Single bridge for either incoming OR outgoing connections
/// - **Bidirectional**: Two separate bridges (input + output) for both directions
///
/// ## Example
/// ```text
/// Before collapse:  NodeA → NodeB → NodeC
/// After collapse:   NodeA → [Layer: bridge_in → NodeB → bridge_out] → NodeC
/// ```
#[derive(Default)]
pub struct BridgeLayersCleanup {
    /// Set of all layer IDs
    all_layers: HashSet<String>,
    /// Maps a layer ID to its parent layer ID, if any
    layer_parents: HashMap<String, Option<String>>,
    /// Set of pin IDs that are layer boundary pins (already bridge pins)
    layer_pin_ids: HashSet<String>,
    /// Maps pin ID to the layer it belongs to (None if not in a layer)
    pin_layer: HashMap<String, Option<String>>,
    /// Maps (layer_id, pin_id) to the plan for creating bridge pins
    bridge_plans: HashMap<(String, String), BridgePlan>,
}

impl BoardCleanupLogic for BridgeLayersCleanup {
    fn init(board: &mut Board) -> Self
    where
        Self: Sized,
    {
        Self {
            all_layers: HashSet::with_capacity(10),
            layer_parents: HashMap::with_capacity(10),
            layer_pin_ids: HashSet::with_capacity(50),
            pin_layer: HashMap::with_capacity((board.nodes.len() + board.layers.len()) * 4),
            bridge_plans: HashMap::with_capacity(10),
        }
    }

    fn initial_layer_iteration(&mut self, layer: &Layer) {
        self.all_layers.insert(layer.id.clone());
        self.layer_parents
            .insert(layer.id.clone(), layer.parent_id.clone());
    }

    fn initial_pin_iteration(&mut self, pin: &Pin, parent: NodeOrLayerRef) {
        match parent {
            NodeOrLayerRef::Node(node) => {
                self.pin_layer.insert(pin.id.clone(), node.layer.clone());
            }
            NodeOrLayerRef::Layer(layer) => {
                self.pin_layer
                    .insert(pin.id.clone(), Some(layer.id.clone()));
                // Track this as a layer boundary pin
                self.layer_pin_ids.insert(pin.id.clone());
            }
        }
    }

    fn main_pin_iteration(&mut self, pin: &mut Pin, _pin_lookup: &PinLookup) {
        // Get the layer that this pin belongs to
        let layer = self.pin_layer.get(&pin.id).cloned().flatten();
        let layer_id = if let Some(layer_id) = &layer {
            layer_id.clone()
        } else {
            return;
        };

        // Only process pins inside layers (not top-level pins)
        if !self.all_layers.contains(&layer_id) {
            return;
        }

        // Skip layer boundary pins themselves - they don't need bridging
        if self.layer_pin_ids.contains(&pin.id) {
            return;
        }

        // Collect all outgoing connections (connected_to) that cross layer boundaries
        // These are connections from this internal pin to pins outside the layer
        // Skip connections that already go through boundary pins within this scope
        pin.connected_to.iter().for_each(|connected_to| {
            if self.is_existing_boundary_for_scope(connected_to, &layer_id) {
                return;
            }
            // Skip orphaned pins (deleted nodes) - they won't be in pin_layer
            let Some(connected_layer) = self.pin_layer.get(connected_to) else {
                return;
            };
            if !self.is_pin_within_layer_scope(connected_layer.as_ref(), &layer_id) {
                let key = (layer_id.clone(), pin.id.clone());
                let plan = self.bridge_plans.entry(key).or_default();
                plan.outside_connected_to.insert(connected_to.clone());
            }
        });

        // Collect all incoming connections (depends_on) that cross layer boundaries
        // These are connections from pins outside the layer to this internal pin
        // Skip connections that already go through boundary pins within this scope
        pin.depends_on.iter().for_each(|depends_on| {
            if self.is_existing_boundary_for_scope(depends_on, &layer_id) {
                return;
            }
            // Skip orphaned pins (deleted nodes) - they won't be in pin_layer
            let Some(depends_on_layer) = self.pin_layer.get(depends_on) else {
                return;
            };
            if !self.is_pin_within_layer_scope(depends_on_layer.as_ref(), &layer_id) {
                let key = (layer_id.clone(), pin.id.clone());
                let plan = self.bridge_plans.entry(key).or_default();
                plan.outside_depends_on.insert(depends_on.clone());
            }
        });
    }

    fn post_process(&mut self, board: &mut Board, pin_lookup: &PinLookup) {
        // Process each bridge plan that was collected during the main iteration
        for ((layer_id, layer_pin_id), plan) in self.bridge_plans.drain() {
            if !board.layers.contains_key(&layer_id) {
                tracing::warn!(
                    "Layer {} not found in board during bridge cleanup",
                    layer_id
                );
                continue;
            }

            // Skip if this pin has no external connections (nothing to bridge)
            if plan.outside_connected_to.is_empty() && plan.outside_depends_on.is_empty() {
                continue;
            }

            // Get the original pin inside the layer that needs bridging
            let Some(original_pin) = get_pin_mut(board, pin_lookup, &layer_pin_id) else {
                tracing::warn!(
                    "Pin {} not found in layer {} during bridge cleanup",
                    layer_pin_id,
                    layer_id
                );
                continue;
            };

            let original_pin_id = original_pin.id.clone();

            // Remove external connections from the original pin
            // These will be moved to the bridge pin(s)
            original_pin
                .connected_to
                .retain(|connected_to| !plan.outside_connected_to.contains(connected_to));

            original_pin
                .depends_on
                .retain(|depends_on| !plan.outside_depends_on.contains(depends_on));

            let has_outgoing = !plan.outside_connected_to.is_empty();
            let has_incoming = !plan.outside_depends_on.is_empty();

            // SPECIAL CASE: Bidirectional connections (both incoming AND outgoing)
            // When a pin has both incoming and outgoing external connections, we need
            // TWO separate bridge pins to avoid creating circular dependencies.
            //
            // Example: A for_each node with external input and output
            // Flow: Outside_In → in_bridge → original_pin → out_bridge → Outside_Out
            //
            // Without separate bridges, we'd create: original ⇄ bridge (circular!)
            if has_outgoing && has_incoming {
                // Create INPUT bridge pin (handles incoming connections from outside)
                let in_bridge_pin_id = create_id();
                let mut in_bridge_pin = original_pin.clone();
                in_bridge_pin.id = in_bridge_pin_id.clone();
                in_bridge_pin.pin_type = PinType::Input;
                // Input bridge connects TO the original pin
                in_bridge_pin.connected_to = BTreeSet::from([original_pin.id.clone()]);
                // Input bridge depends ON external pins
                in_bridge_pin.depends_on = plan.outside_depends_on.clone();

                // Original pin now depends on the input bridge instead of external pins
                original_pin.depends_on.insert(in_bridge_pin_id.clone());

                // Create OUTPUT bridge pin (handles outgoing connections to outside)
                let out_bridge_pin_id = create_id();
                let mut out_bridge_pin = original_pin.clone();
                out_bridge_pin.id = out_bridge_pin_id.clone();
                out_bridge_pin.pin_type = PinType::Output;
                // Output bridge connects TO external pins
                out_bridge_pin.connected_to = plan.outside_connected_to.clone();
                // Output bridge depends ON the original pin
                out_bridge_pin.depends_on = BTreeSet::from([original_pin.id.clone()]);

                // Original pin now connects to the output bridge instead of external pins
                original_pin.connected_to.insert(out_bridge_pin_id.clone());

                // Add both bridge pins to the layer
                let layer = if let Some(layer) = board.layers.get_mut(&layer_id) {
                    layer
                } else {
                    continue;
                };

                layer.pins.insert(in_bridge_pin_id.clone(), in_bridge_pin);
                layer.pins.insert(out_bridge_pin_id.clone(), out_bridge_pin);

                // Update external pins that were sending TO the original pin
                // They now send to the input bridge instead
                for dep_pin in &plan.outside_depends_on {
                    let Some(pin) = get_pin_mut(board, pin_lookup, dep_pin) else {
                        continue;
                    };
                    pin.connected_to.insert(in_bridge_pin_id.clone());
                    pin.connected_to.remove(&original_pin_id);
                }

                // Update external pins that were receiving FROM the original pin
                // They now receive from the output bridge instead
                for connected_pin in &plan.outside_connected_to {
                    let Some(pin) = get_pin_mut(board, pin_lookup, connected_pin) else {
                        continue;
                    };
                    pin.depends_on.insert(out_bridge_pin_id.clone());
                    pin.depends_on.remove(&original_pin_id);
                }

                continue;
            }

            // STANDARD CASE: Unidirectional connections (either incoming OR outgoing)
            // We only need a single bridge pin for one-way connections
            let bridge_pin_id = create_id();
            let mut bridge_pin = original_pin.clone();
            bridge_pin.id = bridge_pin_id.clone();
            bridge_pin.connected_to = plan.outside_connected_to.clone();
            bridge_pin.depends_on = plan.outside_depends_on.clone();

            // OUTGOING: original_pin → bridge_pin → external_pins
            if has_outgoing {
                bridge_pin.pin_type = PinType::Output;
                // Original pin sends to bridge
                original_pin.connected_to.insert(bridge_pin_id.clone());
                // Bridge depends on original (receives from it)
                bridge_pin.depends_on.insert(original_pin.id.clone());
            }

            // INCOMING: external_pins → bridge_pin → original_pin
            if has_incoming {
                bridge_pin.pin_type = PinType::Input;
                // Original pin depends on bridge (receives from it)
                original_pin.depends_on.insert(bridge_pin.id.clone());
                // Bridge sends to original
                bridge_pin.connected_to.insert(original_pin.id.clone());
            }

            // Add the bridge pin to the layer
            let layer = if let Some(layer) = board.layers.get_mut(&layer_id) {
                layer
            } else {
                tracing::warn!(
                    "Layer {} not found in board during bridge cleanup",
                    layer_id
                );
                continue;
            };

            layer.pins.insert(bridge_pin_id.clone(), bridge_pin);

            // Update external pins that were receiving FROM the original pin
            // They now receive from the bridge pin instead
            for connected_pin in plan.outside_connected_to {
                let Some(pin) = get_pin_mut(board, pin_lookup, &connected_pin) else {
                    tracing::warn!(
                        "Connected Pin {} not found in pin lookup or board during bridge cleanup",
                        connected_pin
                    );
                    continue;
                };

                pin.depends_on.insert(bridge_pin_id.clone());
                pin.depends_on.remove(&original_pin_id);
            }

            // Update external pins that were sending TO the original pin
            // They now send to the bridge pin instead
            for dep_pin in plan.outside_depends_on {
                let Some(pin) = get_pin_mut(board, pin_lookup, &dep_pin) else {
                    tracing::warn!(
                        "Dependent Pin {} not found in pin lookup or board during bridge cleanup",
                        dep_pin
                    );
                    continue;
                };

                pin.connected_to.insert(bridge_pin_id.clone());
                pin.connected_to.remove(&original_pin_id);
            }
        }
    }
}

impl BridgeLayersCleanup {
    fn is_existing_boundary_for_scope(&self, pin_id: &str, layer_id: &str) -> bool {
        if !self.layer_pin_ids.contains(pin_id) {
            return false;
        }

        self.pin_layer
            .get(pin_id)
            .map(|pin_layer| self.is_pin_within_layer_scope(pin_layer.as_ref(), layer_id))
            .unwrap_or(true)
    }

    fn is_pin_within_layer_scope(&self, pin_layer: Option<&String>, layer_id: &str) -> bool {
        let mut current_layer = pin_layer.cloned();
        // A damaged `parent_id` chain can be cyclic; walking it unguarded hangs the cleanup.
        let mut seen = HashSet::new();

        while let Some(current) = current_layer {
            if current == layer_id {
                return true;
            }

            if !seen.insert(current.clone()) {
                return false;
            }

            current_layer = self.layer_parents.get(&current).cloned().flatten();
        }

        false
    }
}

/// Helper function to get a mutable reference to a pin from the board
/// Uses the pin_lookup to determine if the pin belongs to a node or layer,
/// then retrieves it from the appropriate collection
fn get_pin_mut<'a>(
    board: &'a mut Board,
    pin_lookup: &PinLookup,
    pin_id: &str,
) -> Option<&'a mut Pin> {
    match pin_lookup.get(pin_id) {
        Some((_, parent)) => match parent {
            NodeOrLayer::Node(_) => board
                .nodes
                .get_mut(parent.id())
                .and_then(|n| n.pins.get_mut(pin_id)),
            NodeOrLayer::Layer(_) => board
                .layers
                .get_mut(parent.id())
                .and_then(|l| l.pins.get_mut(pin_id)),
        },
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        time::SystemTime,
    };

    use flow_like_storage::object_store::path::Path;

    use crate::flow::{
        board::{
            Board, ExecutionMode, ExecutionStage, Layer, LayerType, cleanup::BoardCleanupLogic,
        },
        execution::LogLevel,
        node::Node,
        pin::{Pin, PinType, ValueType},
        variable::VariableType,
    };

    use super::BridgeLayersCleanup;

    fn test_board() -> Board {
        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
            pin_index: None,
        }
    }

    #[test]
    fn child_layer_bridges_do_not_create_parent_layer_pins() {
        let mut board = test_board();

        let layer_a = Layer::new("layer-a".to_string(), "A".to_string(), LayerType::Collapsed);
        let mut layer_b = Layer::new("layer-b".to_string(), "B".to_string(), LayerType::Collapsed);
        layer_b.parent_id = Some(layer_a.id.clone());

        board.layers.insert(layer_a.id.clone(), layer_a.clone());
        board.layers.insert(layer_b.id.clone(), layer_b.clone());

        let mut parent_source = Node::new("source", "Source", "", "test");
        parent_source.id = "parent-source".to_string();
        parent_source.layer = Some(layer_a.id.clone());
        let parent_source_pin = parent_source
            .add_output_pin("out", "Out", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        let mut child_node = Node::new("child", "Child", "", "test");
        child_node.id = "child-node".to_string();
        child_node.layer = Some(layer_b.id.clone());
        let child_in_pin = child_node
            .add_input_pin("in", "In", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();
        let child_out_pin = child_node
            .add_output_pin("out", "Out", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        let mut parent_sink = Node::new("sink", "Sink", "", "test");
        parent_sink.id = "parent-sink".to_string();
        parent_sink.layer = Some(layer_a.id.clone());
        let parent_sink_pin = parent_sink
            .add_input_pin("in", "In", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        board
            .nodes
            .insert(parent_source.id.clone(), parent_source.clone());
        board
            .nodes
            .insert(child_node.id.clone(), child_node.clone());
        board
            .nodes
            .insert(parent_sink.id.clone(), parent_sink.clone());

        crate::flow::board::commands::pins::connect_pins::connect_pins(
            &mut board,
            &parent_source.id,
            &parent_source_pin,
            &child_node.id,
            &child_in_pin,
        )
        .unwrap();
        crate::flow::board::commands::pins::connect_pins::connect_pins(
            &mut board,
            &child_node.id,
            &child_out_pin,
            &parent_sink.id,
            &parent_sink_pin,
        )
        .unwrap();

        let mut bridge_layers = BridgeLayersCleanup::init(&mut board);
        let mut pins = HashMap::new();

        for node in board.nodes.values() {
            for pin in node.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (
                        crate::flow::board::cleanup::PinEdges::of(pin),
                        crate::flow::board::cleanup::NodeOrLayer::Node(node.id.clone()),
                    ),
                );
                bridge_layers.initial_pin_iteration(
                    pin,
                    crate::flow::board::cleanup::NodeOrLayerRef::Node(node),
                );
            }
        }

        for layer in board.layers.values() {
            bridge_layers.initial_layer_iteration(layer);

            for pin in layer.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (
                        crate::flow::board::cleanup::PinEdges::of(pin),
                        crate::flow::board::cleanup::NodeOrLayer::Layer(layer.id.clone()),
                    ),
                );
                bridge_layers.initial_pin_iteration(
                    pin,
                    crate::flow::board::cleanup::NodeOrLayerRef::Layer(layer),
                );
            }
        }

        for node in board.nodes.values_mut() {
            for pin in node.pins.values_mut() {
                bridge_layers.main_pin_iteration(pin, &pins);
            }
        }

        for layer in board.layers.values_mut() {
            for pin in layer.pins.values_mut() {
                bridge_layers.main_pin_iteration(pin, &pins);
            }
        }

        bridge_layers.post_process(&mut board, &pins);

        let parent_layer = board.layers.get(&layer_a.id).unwrap();
        let child_layer = board.layers.get(&layer_b.id).unwrap();

        assert!(parent_layer.pins.is_empty());
        assert_eq!(child_layer.pins.len(), 2);

        let input_bridge = child_layer
            .pins
            .values()
            .find(|pin| pin.pin_type == PinType::Input)
            .unwrap();
        let output_bridge = child_layer
            .pins
            .values()
            .find(|pin| pin.pin_type == PinType::Output)
            .unwrap();

        let parent_source = board.nodes.get("parent-source").unwrap();
        let child_node = board.nodes.get("child-node").unwrap();
        let parent_sink = board.nodes.get("parent-sink").unwrap();

        assert_eq!(parent_source.pins[&parent_source_pin].connected_to.len(), 1);
        assert!(
            parent_source.pins[&parent_source_pin]
                .connected_to
                .contains(&input_bridge.id)
        );
        assert_eq!(child_node.pins[&child_in_pin].depends_on.len(), 1);
        assert!(
            child_node.pins[&child_in_pin]
                .depends_on
                .contains(&input_bridge.id)
        );
        assert!(input_bridge.depends_on.contains(&parent_source_pin));
        assert!(input_bridge.connected_to.contains(&child_in_pin));

        assert_eq!(child_node.pins[&child_out_pin].connected_to.len(), 1);
        assert!(
            child_node.pins[&child_out_pin]
                .connected_to
                .contains(&output_bridge.id)
        );
        assert_eq!(parent_sink.pins[&parent_sink_pin].depends_on.len(), 1);
        assert!(
            parent_sink.pins[&parent_sink_pin]
                .depends_on
                .contains(&output_bridge.id)
        );
        assert!(output_bridge.depends_on.contains(&child_out_pin));
        assert!(output_bridge.connected_to.contains(&parent_sink_pin));
    }

    #[test]
    fn ancestor_layer_boundary_pin_creates_child_exec_input_bridge() {
        let mut board = test_board();

        let mut layer_a = Layer::new("layer-a".to_string(), "A".to_string(), LayerType::Collapsed);
        let mut layer_b = Layer::new("layer-b".to_string(), "B".to_string(), LayerType::Collapsed);
        layer_b.parent_id = Some(layer_a.id.clone());

        let parent_boundary_pin = Pin {
            id: "layer-a-exec-in".to_string(),
            name: "exec_in".to_string(),
            friendly_name: "Exec In".to_string(),
            description: String::new(),
            pin_type: PinType::Input,
            data_type: VariableType::Execution,
            schema: None,
            value_type: ValueType::Normal,
            depends_on: BTreeSet::new(),
            connected_to: BTreeSet::new(),
            default_value: None,
            index: 1,
            options: None,
            value: None,
        };
        layer_a
            .pins
            .insert(parent_boundary_pin.id.clone(), parent_boundary_pin.clone());

        board.layers.insert(layer_a.id.clone(), layer_a.clone());
        board.layers.insert(layer_b.id.clone(), layer_b.clone());

        let mut child_node = Node::new("child", "Child", "", "test");
        child_node.id = "child-node".to_string();
        child_node.layer = Some(layer_b.id.clone());
        let child_exec_in = child_node
            .add_input_pin("exec_in", "Exec In", "", VariableType::Execution)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        board
            .nodes
            .insert(child_node.id.clone(), child_node.clone());

        crate::flow::board::commands::pins::connect_pins::connect_pins(
            &mut board,
            &layer_a.id,
            &parent_boundary_pin.id,
            &child_node.id,
            &child_exec_in,
        )
        .unwrap();

        let mut bridge_layers = BridgeLayersCleanup::init(&mut board);
        let mut pins = HashMap::new();

        for node in board.nodes.values() {
            for pin in node.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (
                        crate::flow::board::cleanup::PinEdges::of(pin),
                        crate::flow::board::cleanup::NodeOrLayer::Node(node.id.clone()),
                    ),
                );
                bridge_layers.initial_pin_iteration(
                    pin,
                    crate::flow::board::cleanup::NodeOrLayerRef::Node(node),
                );
            }
        }

        for layer in board.layers.values() {
            bridge_layers.initial_layer_iteration(layer);

            for pin in layer.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (
                        crate::flow::board::cleanup::PinEdges::of(pin),
                        crate::flow::board::cleanup::NodeOrLayer::Layer(layer.id.clone()),
                    ),
                );
                bridge_layers.initial_pin_iteration(
                    pin,
                    crate::flow::board::cleanup::NodeOrLayerRef::Layer(layer),
                );
            }
        }

        for node in board.nodes.values_mut() {
            for pin in node.pins.values_mut() {
                bridge_layers.main_pin_iteration(pin, &pins);
            }
        }

        for layer in board.layers.values_mut() {
            for pin in layer.pins.values_mut() {
                bridge_layers.main_pin_iteration(pin, &pins);
            }
        }

        bridge_layers.post_process(&mut board, &pins);

        let parent_layer = board.layers.get(&layer_a.id).unwrap();
        let child_layer = board.layers.get(&layer_b.id).unwrap();
        let child_node = board.nodes.get("child-node").unwrap();

        assert_eq!(parent_layer.pins.len(), 1);
        assert_eq!(child_layer.pins.len(), 1);

        let bridge_pin = child_layer.pins.values().next().unwrap();
        assert_eq!(bridge_pin.pin_type, PinType::Input);
        assert_eq!(bridge_pin.data_type, VariableType::Execution);
        assert!(bridge_pin.depends_on.contains(&parent_boundary_pin.id));
        assert!(bridge_pin.connected_to.contains(&child_exec_in));

        let updated_parent_pin = &parent_layer.pins[&parent_boundary_pin.id];
        assert_eq!(updated_parent_pin.connected_to.len(), 1);
        assert!(updated_parent_pin.connected_to.contains(&bridge_pin.id));

        let updated_child_pin = &child_node.pins[&child_exec_in];
        assert_eq!(updated_child_pin.depends_on.len(), 1);
        assert!(updated_child_pin.depends_on.contains(&bridge_pin.id));
    }
}
