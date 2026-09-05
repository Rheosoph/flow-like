use std::collections::{HashMap, HashSet};

use crate::{
    flow::{
        board::{
            Board,
            cleanup::{BoardCleanupLogic, PinLookup},
        },
        node::Node,
        pin::Pin,
        variable::Variable,
    },
    utils::hash::hash_string_non_cryptographic,
};

#[derive(Default)]
pub struct FixRefsCleanup {
    pub refs: HashMap<String, String>,
    pub abandoned: HashSet<String>,
}

impl FixRefsCleanup {
    fn resolve_ref_value(&self, key: &str) -> Result<String, Vec<String>> {
        let mut current = key.to_string();
        let mut visited = Vec::new();
        let mut seen = HashSet::new();

        while let Some(next) = self.refs.get(&current) {
            if !seen.insert(current.clone()) {
                return Err(visited);
            }
            visited.push(current);
            current = next.clone();
        }

        Ok(current)
    }

    fn ensure_ref(&mut self, s: &mut String) {
        if self.refs.contains_key(s) {
            let key = s.clone();
            self.abandoned.remove(&key);
            match self.resolve_ref_value(&key) {
                Ok(resolved) => {
                    // Older template paths could compact an already compact key, producing
                    // `outer -> inner -> JSON`. Flatten the used key before abandoned inner refs
                    // are pruned so all consumers retain the supported one-hop representation.
                    self.refs.insert(key, resolved);
                }
                Err(cycle) => {
                    // There is no concrete value with which to repair a cycle. Preserve every
                    // member rather than pruning part of it into a newly dangling reference.
                    for cycle_key in cycle {
                        self.abandoned.remove(&cycle_key);
                    }
                }
            }
            return;
        }
        let hash = hash_string_non_cryptographic(s).to_string();
        self.refs.insert(hash.clone(), std::mem::take(s));
        self.abandoned.remove(&hash);
        *s = hash;
    }

    fn ensure_ref_opt(&mut self, s: &mut Option<String>) {
        if let Some(inner) = s {
            self.ensure_ref(inner);
        }
    }
}

impl BoardCleanupLogic for FixRefsCleanup {
    fn init(board: &mut Board) -> Self
    where
        Self: Sized,
    {
        // JSON/import paths can still construct legacy boards without passing through protobuf
        // migration. Move any reserved entries across before semantic ref resolution begins.
        let legacy_internal_keys = board
            .refs
            .keys()
            .filter(|key| super::super::is_internal_board_ref(key))
            .cloned()
            .collect::<Vec<_>>();
        for key in legacy_internal_keys {
            if let Some(value) = board.refs.remove(&key) {
                let _ = board.insert_internal_ref(key, value);
            }
        }
        Self {
            refs: board.refs.clone(),
            abandoned: board.refs.keys().cloned().collect(),
        }
    }

    fn main_node_iteration(&mut self, node: &mut Node, _pin_lookup: &PinLookup) {
        self.ensure_ref(&mut node.description);
    }

    fn main_pin_iteration(&mut self, pin: &mut Pin, _pin_lookup: &PinLookup) {
        self.ensure_ref(&mut pin.description);
        self.ensure_ref_opt(&mut pin.schema);
    }

    fn main_variable_iteration(&mut self, variable: &mut Variable, _pin_lookup: &PinLookup) {
        self.ensure_ref_opt(&mut variable.schema);
    }

    fn post_process(&mut self, board: &mut Board, _pin_lookup: &PinLookup) {
        board.refs = std::mem::take(&mut self.refs);
        board.refs.retain(|k, _| !self.abandoned.contains(k));
        board.refs.shrink_to_fit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{board::Board, node::Node, variable::VariableType};
    use flow_like_storage::Path;

    #[test]
    fn cleanup_flattens_schema_ref_chains() {
        let schema = r#"{"type":"object","properties":{"value":{"type":"string"}}}"#;
        let mut board = Board::new_detached(Some("board".to_string()), Path::from("boards"));
        board.refs = HashMap::from([
            ("outer-ref".to_string(), "inner-ref".to_string()),
            ("inner-ref".to_string(), schema.to_string()),
        ]);
        let mut node = Node::new("test", "Test", "", "Tests");
        node.add_output_pin("value", "Value", "", VariableType::Struct)
            .schema = Some("outer-ref".to_string());
        board.nodes.insert(node.id.clone(), node);

        board.cleanup();

        let schema_ref = board
            .nodes
            .values()
            .next()
            .and_then(|node| node.get_pin_by_name("value"))
            .and_then(|pin| pin.schema.as_deref())
            .expect("schema ref should remain present");
        assert_eq!(board.refs.get(schema_ref).map(String::as_str), Some(schema));
        assert!(!board.refs.contains_key("inner-ref"));
    }
}
