use std::collections::{HashMap, HashSet};

use crate::flow::{
    board::{
        Board,
        cleanup::{BoardCleanupLogic, PinLookup},
    },
    node::Node,
};

#[derive(Default)]
pub struct FixInitialCoordinates {
    pub layer_coordinates: HashMap<String, Vec<(f32, f32, f32)>>,
    pub dirty_layer: HashSet<String>,
}

impl FixInitialCoordinates {
    fn place_io_for(&self, coordinates: &[(f32, f32, f32)]) -> ((f32, f32, f32), (f32, f32, f32)) {
        let mut min_x = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        let mut sum_z = 0.0f32;
        let mut count = 0.0f32;

        for &(x, y, z) in coordinates {
            if x < min_x {
                min_x = x;
            }
            if x > max_x {
                max_x = x;
            }
            if y < min_y {
                min_y = y;
            }
            if y > max_y {
                max_y = y;
            }
            sum_z += z;
            count += 1.0;
        }

        let center_x = 0.5 * (min_x + max_x);
        let center_y = 0.5 * (min_y + max_y);
        let avg_z = if count > 0.0 { sum_z / count } else { 0.0 };

        let width = (max_x - min_x).max(100.0);
        let height = (max_y - min_y).max(60.0);
        let horizontal = width >= height;
        let axis = if horizontal { width } else { height };
        let margin = (axis * 0.25).clamp(200.0, 300.0);

        if horizontal {
            (
                (min_x - margin, center_y, avg_z),
                (max_x + margin, center_y, avg_z),
            )
        } else {
            (
                (center_x, min_y - margin, avg_z),
                (center_x, max_y + margin, avg_z),
            )
        }
    }
}

impl BoardCleanupLogic for FixInitialCoordinates {
    fn init(_board: &mut Board) -> Self
    where
        Self: Sized,
    {
        Self {
            layer_coordinates: HashMap::new(),
            dirty_layer: HashSet::new(),
        }
    }

    fn initial_layer_iteration(&mut self, layer: &crate::flow::board::Layer) {
        if let Some(parent_layer_id) = layer.parent_id.as_ref() {
            // We dont know yet if the parent layer is actually dirty, but we should still push our coordinates into the layer_coordinates
            self.layer_coordinates
                .entry(parent_layer_id.clone())
                .or_default()
                .push(layer.coordinates);
        }

        if layer.in_coordinates.is_some() && layer.out_coordinates.is_some() {
            return;
        }

        self.dirty_layer.insert(layer.id.clone());
        self.layer_coordinates.entry(layer.id.clone()).or_default();
    }

    fn initial_node_iteration(&mut self, node: &Node) {
        if let Some(layer_id) = &node.layer
            && let Some(coordinates) = &node.coordinates
        {
            self.layer_coordinates
                .entry(layer_id.clone())
                .or_default()
                .push(*coordinates);
        }
    }

    fn main_layer_iteration(
        &mut self,
        layer: &mut crate::flow::board::Layer,
        _pin_lookup: &PinLookup,
    ) {
        let is_dirty = self.dirty_layer.contains(&layer.id);
        if !is_dirty {
            return;
        }

        let coordinates = self.layer_coordinates.remove(&layer.id).unwrap_or_default();
        if coordinates.is_empty() {
            layer.in_coordinates = Some((-200.0, 0.0, 0.0));
            layer.out_coordinates = Some((200.0, 0.0, 0.0));
            return;
        }

        let (in_c, out_c) = self.place_io_for(&coordinates);
        layer.in_coordinates = Some(in_c);
        layer.out_coordinates = Some(out_c);
    }
}
