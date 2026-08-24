use std::collections::{HashMap, HashSet};

use crate::flow::board::{
    Layer, LayerType,
    cleanup::{BoardCleanupLogic, PinLookup},
};

/// Repairs damaged layer hierarchies. A layer that parents itself, sits in a parent cycle or
/// hangs off a deleted layer is rendered nowhere and makes every parent walk — breadcrumbs,
/// "layer up", pin bridging — either wrong or non-terminating. Such a layer is moved back to
/// the board root, where it is at least reachable again.
///
/// Modules are organizational only, so this also restores their two invariants: a module lives
/// at the root or inside another module, and it carries neither boundary pins nor a cache.
pub struct FixLayerParents {
    parents: HashMap<String, Option<String>>,
    modules: HashSet<String>,
}

impl FixLayerParents {
    fn strip_module_boundary(layer: &mut Layer) {
        if !layer.pins.is_empty() {
            tracing::warn!(
                "Module layer {} carried {} boundary pin(s), dropping them",
                layer.id,
                layer.pins.len()
            );
            layer.pins.clear();
        }

        if layer.cache.is_some() {
            tracing::warn!(
                "Module layer {} carried cache settings, clearing them",
                layer.id
            );
            layer.cache = None;
        }
    }

    fn should_detach(&self, layer_id: &str, parent_id: &str) -> bool {
        let mut current = Some(parent_id.to_string());
        let mut seen = HashSet::new();

        while let Some(id) = current {
            if id == layer_id {
                return true;
            }

            // A cycle that does not contain this layer is repaired through its own members.
            if !seen.insert(id.clone()) {
                return false;
            }

            match self.parents.get(&id) {
                Some(parent) => current = parent.clone(),
                None => return true,
            }
        }

        false
    }
}

impl BoardCleanupLogic for FixLayerParents {
    fn init(_board: &mut crate::flow::board::Board) -> Self {
        Self {
            parents: HashMap::new(),
            modules: HashSet::new(),
        }
    }

    fn initial_layer_iteration(&mut self, layer: &Layer) {
        self.parents
            .insert(layer.id.clone(), layer.parent_id.clone());

        if matches!(layer.r#type, LayerType::Module) {
            self.modules.insert(layer.id.clone());
        }
    }

    fn main_layer_iteration(&mut self, layer: &mut Layer, _pin_lookup: &PinLookup) {
        let is_module = matches!(layer.r#type, LayerType::Module);
        if is_module {
            Self::strip_module_boundary(layer);
        }

        let Some(parent_id) = layer.parent_id.clone() else {
            return;
        };

        if self.should_detach(&layer.id, &parent_id) {
            tracing::warn!(
                "Layer {} had an unusable parent ({}), moving it to the board root",
                layer.id,
                parent_id
            );
            layer.parent_id = None;
            return;
        }

        if is_module && !self.modules.contains(&parent_id) {
            tracing::warn!(
                "Module layer {} was parented under a non-module layer ({}), moving it to the board root",
                layer.id,
                parent_id
            );
            layer.parent_id = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::{Board, LayerType};
    use flow_like_storage::Path;

    fn layer(board: &mut Board, id: &str, parent: Option<&str>) {
        let mut layer = Layer::new(id.into(), id.into(), LayerType::Collapsed);
        layer.parent_id = parent.map(str::to_string);
        board.layers.insert(id.into(), layer);
    }

    fn cleaned(board: &mut Board) -> HashMap<String, Option<String>> {
        board.cleanup();
        board
            .layers
            .values()
            .map(|layer| (layer.id.clone(), layer.parent_id.clone()))
            .collect()
    }

    #[test]
    fn a_self_parented_layer_returns_to_the_root() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        layer(&mut board, "a", Some("a"));
        layer(&mut board, "b", None);
        layer(&mut board, "c", Some("b"));

        let parents = cleaned(&mut board);
        assert_eq!(parents["a"], None);
        assert_eq!(parents["c"], Some("b".into()));
    }

    #[test]
    fn a_parent_cycle_is_broken() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        layer(&mut board, "a", Some("b"));
        layer(&mut board, "b", Some("a"));

        let parents = cleaned(&mut board);
        assert_eq!(parents["a"], None);
        assert_eq!(parents["b"], None);
    }

    #[test]
    fn a_layer_under_a_deleted_parent_returns_to_the_root() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        layer(&mut board, "a", Some("gone"));

        let parents = cleaned(&mut board);
        assert_eq!(parents["a"], None);
    }

    fn module(board: &mut Board, id: &str, parent: Option<&str>) {
        let mut layer = Layer::new(id.into(), id.into(), LayerType::Module);
        layer.parent_id = parent.map(str::to_string);
        board.layers.insert(id.into(), layer);
    }

    #[test]
    fn a_module_under_a_function_layer_returns_to_the_root() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        let mut function = Layer::new("fn".into(), "Fn".into(), LayerType::Function);
        function.parent_id = None;
        board.layers.insert("fn".into(), function);
        module(&mut board, "mod", Some("fn"));

        let parents = cleaned(&mut board);
        assert_eq!(parents["mod"], None);
    }

    #[test]
    fn a_module_under_a_module_keeps_its_parent() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "outer", None);
        module(&mut board, "inner", Some("outer"));

        let parents = cleaned(&mut board);
        assert_eq!(parents["inner"], Some("outer".into()));
    }

    #[test]
    fn a_module_loses_its_pins_and_cache() {
        use crate::flow::board::LayerCache;
        use crate::flow::node::Node;
        use crate::flow::variable::VariableType;

        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "mod", None);
        let mut node = Node::new("n", "N", "", "test");
        let pin = node
            .add_output_pin("out", "Out", "", VariableType::String)
            .clone();
        let stored = board.layers.get_mut("mod").expect("module");
        stored.pins.insert(pin.id.clone(), pin);
        stored.cache = Some(LayerCache::default());

        board.cleanup();

        let healed = &board.layers["mod"];
        assert!(healed.pins.is_empty());
        assert_eq!(healed.cache, None);
    }
}
