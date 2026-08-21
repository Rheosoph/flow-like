use std::collections::{BTreeSet, HashMap};

use crate::flow::{
    board::{
        Board, Layer,
        cleanup::{
            bridge_layers::BridgeLayersCleanup, fix_layer_parents::FixLayerParents,
            fix_pin_connections::FixPinsCleanup, fix_refs::FixRefsCleanup,
            order_pin_indices::PinIndicesCleanup, sync_known_schemas::SyncKnownSchemasCleanup,
        },
    },
    node::Node,
    pin::Pin,
    variable::Variable,
};

pub mod bridge_layers;
pub mod fix_initial_coordinates;
pub mod fix_layer_parents;
pub mod fix_pin_connections;
pub mod fix_refs;
pub mod order_pin_indices;
pub mod sync_known_schemas;
pub mod sync_node_schema;

pub type PinLookup = HashMap<String, (PinEdges, NodeOrLayer)>;

/// A pin's reciprocal edges, snapshotted before the main pass begins.
///
/// The main pass holds `&mut` on the board while steps look up *other* pins, so this cannot borrow
/// from the board and has to own what it exposes. It carries only the two edge sets the steps read:
/// snapshotting whole pins meant copying every schema string and default value on the board — the
/// bulk of a pin's size — on every apply, none of which was ever looked at.
pub struct PinEdges {
    pub connected_to: BTreeSet<String>,
    pub depends_on: BTreeSet<String>,
}

impl PinEdges {
    fn of(pin: &Pin) -> Self {
        PinEdges {
            connected_to: pin.connected_to.clone(),
            depends_on: pin.depends_on.clone(),
        }
    }
}

pub enum NodeOrLayer {
    Node(String),
    Layer(String),
}

pub enum NodeOrLayerRef<'a> {
    Node(&'a Node),
    Layer(&'a Layer),
}

impl NodeOrLayerRef<'_> {
    pub fn id(&self) -> &str {
        match self {
            NodeOrLayerRef::Node(node) => node.id.as_str(),
            NodeOrLayerRef::Layer(layer) => layer.id.as_str(),
        }
    }
}

impl NodeOrLayer {
    pub fn is_node(&self) -> bool {
        matches!(self, NodeOrLayer::Node(_))
    }

    pub fn is_layer(&self) -> bool {
        matches!(self, NodeOrLayer::Layer(_))
    }

    pub fn id(&self) -> &str {
        match self {
            NodeOrLayer::Node(node_id) => node_id.as_str(),
            NodeOrLayer::Layer(layer_id) => layer_id.as_str(),
        }
    }
}

pub trait BoardCleanupLogic {
    fn init(board: &mut Board) -> Self
    where
        Self: Sized;

    fn initial_node_iteration(&mut self, _node: &Node) {}
    fn main_node_iteration(&mut self, _node: &mut Node, _pin_lookup: &PinLookup) {}

    fn initial_pin_iteration(&mut self, _pin: &Pin, _parent: NodeOrLayerRef) {}
    fn main_pin_iteration(&mut self, _pin: &mut Pin, _pin_lookup: &PinLookup) {}

    fn initial_layer_iteration(&mut self, _layer: &Layer) {}
    fn main_layer_iteration(&mut self, _layer: &mut Layer, _pin_lookup: &PinLookup) {}

    fn main_variable_iteration(&mut self, _variable: &mut Variable, _pin_lookup: &PinLookup) {}

    fn post_process(&mut self, _board: &mut Board, _pin_lookup: &PinLookup) {}
}

impl Board {
    pub fn cleanup(&mut self) {
        let mut bridge_layers = BridgeLayersCleanup::init(self);
        let mut fix_pin_connections = FixPinsCleanup::init(self);
        let mut sync_known_schemas = SyncKnownSchemasCleanup::init(self);
        let mut fix_refs = FixRefsCleanup::init(self);
        let mut order_pin_indices = PinIndicesCleanup::init(self);
        let mut fix_initial_coordinates =
            fix_initial_coordinates::FixInitialCoordinates::init(self);
        let mut fix_layer_parents = FixLayerParents::init(self);

        let mut pins: PinLookup =
            HashMap::with_capacity((self.nodes.len() + self.layers.len()) * 4);

        let mut steps: Vec<&mut dyn BoardCleanupLogic> = vec![
            &mut fix_layer_parents,
            &mut fix_initial_coordinates,
            &mut sync_known_schemas,
            &mut fix_refs,
            &mut fix_pin_connections,
            &mut bridge_layers,
            &mut order_pin_indices,
        ];

        // Initial Processing
        for node in self.nodes.values() {
            for step in steps.iter_mut() {
                (*step).initial_node_iteration(node);
            }

            for pin in node.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (PinEdges::of(pin), NodeOrLayer::Node(node.id.clone())),
                );
                for step in steps.iter_mut() {
                    (*step).initial_pin_iteration(pin, NodeOrLayerRef::Node(node));
                }
            }
        }

        for layer in self.layers.values() {
            for step in steps.iter_mut() {
                (*step).initial_layer_iteration(layer);
            }

            for pin in layer.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (PinEdges::of(pin), NodeOrLayer::Layer(layer.id.clone())),
                );
                for step in steps.iter_mut() {
                    (*step).initial_pin_iteration(pin, NodeOrLayerRef::Layer(layer));
                }
            }

            for node in layer.nodes.values() {
                for step in steps.iter_mut() {
                    (*step).initial_node_iteration(node);
                }

                for pin in node.pins.values() {
                    pins.insert(
                        pin.id.clone(),
                        (PinEdges::of(pin), NodeOrLayer::Node(node.id.clone())),
                    );
                    for step in steps.iter_mut() {
                        (*step).initial_pin_iteration(pin, NodeOrLayerRef::Node(node));
                    }
                }
            }
        }

        // Main Processing
        for node in self.nodes.values_mut() {
            for step in steps.iter_mut() {
                (*step).main_node_iteration(node, &pins);
            }

            for pin in node.pins.values_mut() {
                for step in steps.iter_mut() {
                    (*step).main_pin_iteration(pin, &pins);
                }
            }
        }

        for layer in self.layers.values_mut() {
            for step in steps.iter_mut() {
                (*step).main_layer_iteration(layer, &pins);
            }

            for pin in layer.pins.values_mut() {
                for step in steps.iter_mut() {
                    (*step).main_pin_iteration(pin, &pins);
                }
            }

            for node in layer.nodes.values_mut() {
                for step in steps.iter_mut() {
                    (*step).main_node_iteration(node, &pins);
                }

                for pin in node.pins.values_mut() {
                    for step in steps.iter_mut() {
                        (*step).main_pin_iteration(pin, &pins);
                    }
                }
            }
        }

        for variable in self.variables.values_mut() {
            for step in steps.iter_mut() {
                (*step).main_variable_iteration(variable, &pins);
            }
        }

        for layer in self.layers.values_mut() {
            for variable in layer.variables.values_mut() {
                for step in steps.iter_mut() {
                    (*step).main_variable_iteration(variable, &pins);
                }
            }
        }

        for step in steps.iter_mut() {
            (*step).post_process(self, &pins);
        }
    }
}
