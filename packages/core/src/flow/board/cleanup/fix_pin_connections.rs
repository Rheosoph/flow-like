use std::collections::{HashMap, HashSet};

use crate::flow::{
    board::{
        Board,
        cleanup::{BoardCleanupLogic, NodeOrLayerRef, PinLookup},
    },
    pin::Pin,
};

#[derive(Default)]
pub struct FixPinsCleanup {
    pub node_pins_connected_to_remove: HashMap<String, HashMap<String, HashSet<String>>>,
    pub node_pins_depends_on_remove: HashMap<String, HashMap<String, HashSet<String>>>,
    pub pin_parent: HashMap<String, String>,
}

impl BoardCleanupLogic for FixPinsCleanup {
    fn init(_board: &mut Board) -> Self
    where
        Self: Sized,
    {
        Self {
            node_pins_connected_to_remove: HashMap::with_capacity(10),
            node_pins_depends_on_remove: HashMap::with_capacity(10),
            pin_parent: HashMap::with_capacity(100),
        }
    }

    fn initial_pin_iteration(&mut self, pin: &Pin, parent: NodeOrLayerRef) {
        self.pin_parent
            .insert(pin.id.clone(), parent.id().to_string());
    }

    fn main_pin_iteration(&mut self, pin: &mut Pin, pin_lookup: &PinLookup) {
        let owner_parent_id_opt = self.pin_parent.get(&pin.id);

        for connected_to in pin.connected_to.iter() {
            match pin_lookup.get(connected_to) {
                Some((target_pin, _)) => {
                    if let Some(owner_parent_id) = owner_parent_id_opt
                        && !target_pin.depends_on.contains(&pin.id)
                    {
                        self.node_pins_connected_to_remove
                            .entry(owner_parent_id.clone())
                            .or_default()
                            .entry(pin.id.clone())
                            .or_default()
                            .insert(connected_to.clone());
                    }
                }
                None => {
                    if let Some(owner_parent_id) = owner_parent_id_opt {
                        self.node_pins_connected_to_remove
                            .entry(owner_parent_id.clone())
                            .or_default()
                            .entry(pin.id.clone())
                            .or_default()
                            .insert(connected_to.clone());
                    }
                }
            }
        }

        for depends_on in pin.depends_on.iter() {
            match pin_lookup.get(depends_on) {
                Some((target_pin, _)) => {
                    if let Some(owner_parent_id) = owner_parent_id_opt
                        && !target_pin.connected_to.contains(&pin.id)
                    {
                        self.node_pins_depends_on_remove
                            .entry(owner_parent_id.clone())
                            .or_default()
                            .entry(pin.id.clone())
                            .or_default()
                            .insert(depends_on.clone());
                    }
                }
                None => {
                    if let Some(owner_parent_id) = owner_parent_id_opt {
                        self.node_pins_depends_on_remove
                            .entry(owner_parent_id.clone())
                            .or_default()
                            .entry(pin.id.clone())
                            .or_default()
                            .insert(depends_on.clone());
                    }
                }
            }
        }
    }

    fn post_process(&mut self, board: &mut Board, _pin_lookup: &PinLookup) {
        for (node_id, pins) in self.node_pins_connected_to_remove.drain() {
            if let Some(node) = board.nodes.get_mut(&node_id) {
                for (pin_id, to_remove) in &pins {
                    if let Some(pin) = node.pins.get_mut(pin_id) {
                        for connected_pin in to_remove {
                            pin.connected_to.remove(connected_pin);
                        }
                    }
                }
            } else if let Some(layer) = board.layers.get_mut(&node_id) {
                for (pin_id, to_remove) in &pins {
                    if let Some(pin) = layer.pins.get_mut(pin_id) {
                        for connected_pin in to_remove {
                            pin.connected_to.remove(connected_pin);
                        }
                    }
                }
            } else {
                for layer in board.layers.values_mut() {
                    if let Some(node) = layer.nodes.get_mut(&node_id) {
                        for (pin_id, to_remove) in &pins {
                            if let Some(pin) = node.pins.get_mut(pin_id) {
                                for connected_pin in to_remove {
                                    pin.connected_to.remove(connected_pin);
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }

        for (node_id, pins) in self.node_pins_depends_on_remove.drain() {
            if let Some(node) = board.nodes.get_mut(&node_id) {
                for (pin_id, to_remove) in &pins {
                    if let Some(pin) = node.pins.get_mut(pin_id) {
                        for dep_pin in to_remove {
                            pin.depends_on.remove(dep_pin);
                        }
                    }
                }
            } else if let Some(layer) = board.layers.get_mut(&node_id) {
                for (pin_id, to_remove) in &pins {
                    if let Some(pin) = layer.pins.get_mut(pin_id) {
                        for dep_pin in to_remove {
                            pin.depends_on.remove(dep_pin);
                        }
                    }
                }
            } else {
                for layer in board.layers.values_mut() {
                    if let Some(node) = layer.nodes.get_mut(&node_id) {
                        for (pin_id, to_remove) in &pins {
                            if let Some(pin) = node.pins.get_mut(pin_id) {
                                for dep_pin in to_remove {
                                    pin.depends_on.remove(dep_pin);
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
    }
}
