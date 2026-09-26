use std::collections::{HashMap, HashSet};

use crate::{
    flow::{
        board::{
            Board, INTERNAL_BOARD_REF_PREFIX,
            cleanup::{BoardCleanupLogic, PinLookup},
        },
        node::Node,
        pin::Pin,
        variable::Variable,
    },
    utils::hash::hash_string_non_cryptographic,
};

/// Refs a cleanup pruned stay reachable here for a while. Undo snapshots, redo payloads and peers'
/// histories still carry their keys; without the text behind a key `ensure_ref` would hash the key
/// itself and the description or schema would become a 20-digit number for good.
const PRUNED_REF_NAMESPACE: &str = "pruned_ref/";
const MAX_PRUNED_REFS: usize = 1024;
const MAX_PRUNED_REF_BYTES: usize = 1 << 20;

fn pruned_ref_prefix() -> String {
    format!("{INTERNAL_BOARD_REF_PREFIX}{PRUNED_REF_NAMESPACE}")
}

#[derive(Default)]
pub struct FixRefsCleanup {
    pub refs: HashMap<String, String>,
    pub abandoned: HashSet<String>,
    pruned: HashSet<String>,
    revived: HashSet<String>,
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
        if self.pruned.contains(s.as_str()) {
            self.revived.insert(s.clone());
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

/// Entries are `"{sequence}:{text}"`, so eviction drops the oldest first.
fn remember_pruned_refs(board: &mut Board, prefix: &str, pruned: Vec<(String, String)>) {
    let mut entries: Vec<(u64, String, usize)> = board
        .internal_refs_with_prefix(prefix)
        .filter_map(|(key, value)| {
            let (sequence, text) = value.split_once(':')?;
            Some((sequence.parse().ok()?, key.to_string(), text.len()))
        })
        .collect();
    let first = entries
        .iter()
        .map(|(sequence, ..)| sequence + 1)
        .max()
        .unwrap_or(0);

    for (sequence, (key, text)) in (first..).zip(pruned) {
        let internal_key = format!("{prefix}{key}");
        entries.push((sequence, internal_key.clone(), text.len()));
        board
            .internal_refs
            .insert(internal_key, format!("{sequence}:{text}"));
    }

    entries.sort_unstable_by_key(|(sequence, ..)| std::cmp::Reverse(*sequence));
    let mut bytes = 0;
    for (index, (_, key, len)) in entries.into_iter().enumerate() {
        bytes += len;
        if index >= MAX_PRUNED_REFS || bytes > MAX_PRUNED_REF_BYTES {
            board.internal_refs.remove(&key);
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
        let prefix = pruned_ref_prefix();
        let pruned = board
            .internal_refs_with_prefix(&prefix)
            .map(|(key, _)| key[prefix.len()..].to_string())
            .collect();
        Self {
            refs: board.refs.clone(),
            abandoned: board.refs.keys().cloned().collect(),
            pruned,
            revived: HashSet::new(),
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
        let prefix = pruned_ref_prefix();

        // A pruned key is live again when a restored snapshot carries it, or when its text was
        // written anew and hashed to the same key.
        for key in &self.pruned {
            let revived = self.revived.contains(key);
            if !revived && !board.refs.contains_key(key) {
                continue;
            }
            if let Some(entry) = board.internal_refs.remove(&format!("{prefix}{key}"))
                && revived
                && let Some((_, text)) = entry.split_once(':')
            {
                board.refs.insert(key.clone(), text.to_string());
            }
        }

        let pruned: Vec<(String, String)> = self
            .abandoned
            .iter()
            .filter_map(|key| board.refs.remove(key).map(|text| (key.clone(), text)))
            .collect();
        board.refs.shrink_to_fit();
        if !pruned.is_empty() {
            remember_pruned_refs(board, &prefix, pruned);
        }
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

    fn board_with_described_node(description: &str) -> (Board, String) {
        let mut board = Board::new_detached(Some("board".to_string()), Path::from("boards"));
        let node = Node::new("test", "Test", description, "Tests");
        let node_id = node.id.clone();
        board.nodes.insert(node_id.clone(), node);
        board.cleanup();
        (board, node_id)
    }

    fn describe(board: &mut Board, node_id: &str, description: &str) -> String {
        board.nodes.get_mut(node_id).unwrap().description = description.to_string();
        board.cleanup();
        board.nodes[node_id].description.clone()
    }

    #[test]
    fn a_restored_key_gets_its_pruned_text_back() {
        let (mut board, node_id) = board_with_described_node("Customer id");
        let key = board.nodes[&node_id].description.clone();

        describe(&mut board, &node_id, "Customer UUID");
        assert!(
            !board.refs.contains_key(&key),
            "unused refs are still pruned"
        );

        assert_eq!(describe(&mut board, &node_id, &key), key);
        assert_eq!(
            board.refs.get(&key).map(String::as_str),
            Some("Customer id")
        );
        assert!(
            board
                .internal_ref(&format!("{}{key}", pruned_ref_prefix()))
                .is_none(),
            "a revived ref leaves the graveyard"
        );
    }

    #[test]
    fn retyping_pruned_text_clears_its_graveyard_entry() {
        let (mut board, node_id) = board_with_described_node("Customer id");
        let key = board.nodes[&node_id].description.clone();

        describe(&mut board, &node_id, "Customer UUID");
        assert_eq!(describe(&mut board, &node_id, "Customer id"), key);

        assert!(
            board
                .internal_ref(&format!("{}{key}", pruned_ref_prefix()))
                .is_none()
        );
    }

    #[test]
    fn the_graveyard_keeps_only_the_newest_pruned_refs() {
        let (mut board, node_id) = board_with_described_node("description 0");
        let mut keys = vec![board.nodes[&node_id].description.clone()];
        for index in 1..=MAX_PRUNED_REFS + 1 {
            keys.push(describe(
                &mut board,
                &node_id,
                &format!("description {index}"),
            ));
        }

        let prefix = pruned_ref_prefix();
        assert_eq!(
            board.internal_refs_with_prefix(&prefix).count(),
            MAX_PRUNED_REFS
        );
        assert!(
            board
                .internal_ref(&format!("{prefix}{}", keys[0]))
                .is_none()
        );
        assert!(
            board
                .internal_ref(&format!("{prefix}{}", keys[MAX_PRUNED_REFS]))
                .is_some()
        );
        assert!(crate::flow::board::is_internal_board_ref(&prefix));
    }
}
