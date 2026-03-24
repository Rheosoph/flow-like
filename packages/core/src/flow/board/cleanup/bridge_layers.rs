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
            layer_pin_ids: HashSet::with_capacity(50),
            pin_layer: HashMap::with_capacity((board.nodes.len() + board.layers.len()) * 4),
            bridge_plans: HashMap::with_capacity(10),
        }
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
        // Skip connections that already go through layer pins (already bridged)
        pin.connected_to.iter().for_each(|connected_to| {
            // If connected to a layer pin, it's already bridged
            if self.layer_pin_ids.contains(connected_to) {
                return;
            }
            // Skip orphaned pins (deleted nodes) - they won't be in pin_layer
            let Some(connected_layer) = self.pin_layer.get(connected_to) else {
                return;
            };
            if *connected_layer != Some(layer_id.clone()) {
                let key = (layer_id.clone(), pin.id.clone());
                let plan = self.bridge_plans.entry(key).or_default();
                plan.outside_connected_to.insert(connected_to.clone());
            }
        });

        // Collect all incoming connections (depends_on) that cross layer boundaries
        // These are connections from pins outside the layer to this internal pin
        // Skip connections that already go through layer pins (already bridged)
        pin.depends_on.iter().for_each(|depends_on| {
            // If depends on a layer pin, it's already bridged
            if self.layer_pin_ids.contains(depends_on) {
                return;
            }
            // Skip orphaned pins (deleted nodes) - they won't be in pin_layer
            let Some(depends_on_layer) = self.pin_layer.get(depends_on) else {
                return;
            };
            if *depends_on_layer != Some(layer_id.clone()) {
                let key = (layer_id.clone(), pin.id.clone());
                let plan = self.bridge_plans.entry(key).or_default();
                plan.outside_depends_on.insert(depends_on.clone());
            }
        });
    }

    fn initial_layer_iteration(&mut self, layer: &Layer) {
        self.all_layers.insert(layer.id.clone());
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

/// Helper function to get a mutable reference to a pin from the board
/// Uses the pin_lookup to determine if the pin belongs to a node or layer,
/// then retrieves it from the appropriate collection
fn get_pin_mut<'a>(
    board: &'a mut Board,
    pin_lookup: &PinLookup,
    pin_id: &str,
) -> Option<&'a mut Pin> {
    match pin_lookup.get(pin_id) {
        Some((pin_meta, parent)) => match parent {
            NodeOrLayer::Node(_) => board
                .nodes
                .get_mut(parent.id())
                .and_then(|n| n.pins.get_mut(&pin_meta.id)),
            NodeOrLayer::Layer(_) => board
                .layers
                .get_mut(parent.id())
                .and_then(|l| l.pins.get_mut(&pin_meta.id)),
        },
        None => None,
    }
}
