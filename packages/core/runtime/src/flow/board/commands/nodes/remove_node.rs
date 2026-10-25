use flow_like_types::async_trait;
use schemars::JsonSchema;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::{
    flow::{
        board::{Board, commands::Command},
        node::Node,
    },
    state::FlowLikeState,
};

#[derive(Serialize, Deserialize, Debug, Clone, JsonSchema)]
pub struct RemoveNodeCommand {
    pub node: Node,
    pub connected_nodes: Vec<Node>,
}

/// A run of removals only removes edges and references, so this index remains a safe superset
/// until another kind of command changes the board.
pub(crate) struct NodeRemovalIndex {
    pin_owners: HashMap<String, Vec<String>>,
    referenced_by: HashMap<String, HashSet<String>>,
    stable_node_keys: bool,
}

impl NodeRemovalIndex {
    pub(crate) fn new(board: &Board) -> Self {
        let mut index = Self {
            pin_owners: HashMap::new(),
            referenced_by: HashMap::new(),
            stable_node_keys: true,
        };
        for (node_key, node) in &board.nodes {
            index.stable_node_keys &= node_key == &node.id;
            for pin in node.pins.values() {
                index
                    .pin_owners
                    .entry(pin.id.clone())
                    .or_default()
                    .push(node_key.clone());
            }
            if let Some(refs) = &node.fn_refs {
                for target in &refs.fn_refs {
                    index
                        .referenced_by
                        .entry(target.clone())
                        .or_default()
                        .insert(node_key.clone());
                }
            }
        }
        index
    }

    fn affected_nodes(&self, node: &Node) -> HashSet<String> {
        let mut affected = self
            .referenced_by
            .get(&node.id)
            .cloned()
            .unwrap_or_default();
        for pin in node.pins.values() {
            for connected in pin.connected_to.iter().chain(&pin.depends_on) {
                if let Some(owners) = self.pin_owners.get(connected) {
                    affected.extend(owners.iter().cloned());
                }
            }
        }
        affected
    }
}

impl RemoveNodeCommand {
    pub fn new(node: Node) -> Self {
        RemoveNodeCommand {
            node,
            connected_nodes: vec![],
        }
    }

    pub(crate) fn execute_indexed(
        &mut self,
        board: &mut Board,
        index: Option<&NodeRemovalIndex>,
    ) -> flow_like_types::Result<()> {
        let node = match board.nodes.get(&self.node.id) {
            Some(node) => node,
            None => return Err(flow_like_types::anyhow!("Node not found".to_string())),
        };

        let mut connected_pins = HashSet::new();
        let mut node_pins = HashSet::new();

        node.pins.iter().for_each(|(pin_id, pin)| {
            node_pins.insert(pin_id);
            pin.connected_to.iter().for_each(|connected_pin_id| {
                connected_pins.insert(connected_pin_id.clone());
            });

            pin.depends_on.iter().for_each(|depends_on_pin_id| {
                connected_pins.insert(depends_on_pin_id.clone());
            });
        });

        let mut connected_nodes = vec![];
        let mut changed_nodes = vec![];

        // Legacy writes use the embedded id. A malformed map can gain aliases during removal,
        // so preserve the full scan for that run instead of trusting the original owner index.
        let affected = index
            .filter(|index| index.stable_node_keys)
            .map(|index| index.affected_nodes(node));
        let candidates: Box<dyn Iterator<Item = (&String, &Node)>> = match &affected {
            Some(ids) => Box::new(ids.iter().filter_map(|id| board.nodes.get_key_value(id))),
            None => Box::new(board.nodes.iter()),
        };
        for (node_key, other) in candidates {
            if node_key == &self.node.id {
                continue;
            }

            let mut needs_change = false;

            // Check if this node has pin connections to the deleted node
            for pin in other.pins.values() {
                if connected_pins.contains(&pin.id) {
                    needs_change = true;
                    break;
                }
            }

            // Check if this node references the deleted node via fn_refs
            let has_fn_ref = if let Some(fn_refs) = &other.fn_refs {
                fn_refs.fn_refs.contains(&self.node.id)
            } else {
                false
            };

            if needs_change || has_fn_ref {
                connected_nodes.push(other.clone());

                let mut cloned = other.clone();

                // Clean up pin connections
                cloned.pins.iter_mut().for_each(|(_pin_id, pin)| {
                    pin.connected_to = pin
                        .connected_to
                        .iter()
                        .filter(|connected_pin_id| !node_pins.contains(connected_pin_id))
                        .cloned()
                        .collect();
                    pin.depends_on = pin
                        .depends_on
                        .iter()
                        .filter(|depends_on_pin_id| !node_pins.contains(depends_on_pin_id))
                        .cloned()
                        .collect();
                });

                // Clean up fn_refs - remove the deleted node's ID
                if let Some(fn_refs) = &mut cloned.fn_refs {
                    fn_refs.fn_refs.retain(|node_id| node_id != &self.node.id);
                }

                changed_nodes.push(cloned);
            }
        }

        connected_nodes.sort_by(|left, right| left.id.cmp(&right.id));
        self.connected_nodes = connected_nodes;
        board.nodes.remove(&self.node.id);

        for node in changed_nodes {
            board.nodes.insert(node.id.clone(), node);
        }

        Ok(())
    }
}

#[async_trait]
impl Command for RemoveNodeCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        self.execute_indexed(board, None)
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        board.nodes.insert(self.node.id.clone(), self.node.clone());

        for node in &self.connected_nodes {
            board.nodes.insert(node.id.clone(), node.clone());
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        flow::{
            board::commands::{GenericCommand, nodes::update_node::UpdateNodeCommand},
            node::FnRefs,
            variable::VariableType,
        },
        state::FlowLikeConfig,
        utils::http::HTTPClient,
    };
    use flow_like_storage::Path;

    fn state() -> Arc<FlowLikeState> {
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ))
    }

    fn node(id: &str) -> Node {
        let mut node = Node::new("removal_test", id, "", "Test");
        node.id = id.to_string();
        node.add_input_pin("input", "Input", "", VariableType::String);
        node.add_output_pin("output", "Output", "", VariableType::String);
        node.set_can_be_referenced_by_fns(true);
        node
    }

    fn chain(count: usize) -> Board {
        let mut board = Board::new_detached(Some("removal-test".into()), Path::default());
        let mut nodes = (0..count)
            .map(|index| node(&index.to_string()))
            .collect::<Vec<_>>();
        for index in 1..nodes.len() {
            let output = nodes[index - 1]
                .get_pin_by_name("output")
                .unwrap()
                .id
                .clone();
            let input = nodes[index].get_pin_by_name("input").unwrap().id.clone();
            nodes[index - 1]
                .get_pin_mut_by_name("output")
                .unwrap()
                .connected_to
                .insert(input);
            nodes[index]
                .get_pin_mut_by_name("input")
                .unwrap()
                .depends_on
                .insert(output);
        }
        board.nodes = nodes
            .into_iter()
            .map(|node| (node.id.clone(), node))
            .collect();
        board.cleanup();
        board
    }

    fn node_state(board: &Board) -> serde_json::Value {
        let nodes = board
            .nodes
            .iter()
            .map(|(id, node)| {
                let mut node = node.clone();
                node.hash = None;
                (id, node)
            })
            .collect::<HashMap<_, _>>();
        serde_json::to_value(nodes).unwrap()
    }

    #[flow_like_types::tokio::test]
    async fn indexed_large_removal_matches_scan_and_restores_undo_receipts() {
        let state = state();
        let mut original = chain(1_202);
        original.nodes.get_mut("1201").unwrap().fn_refs = Some(FnRefs {
            fn_refs: vec!["0".to_string(), "600".to_string(), "1200".to_string()],
            can_reference_fns: true,
            can_be_referenced_by_fns: true,
        });
        let index = NodeRemovalIndex::new(&original);
        assert_eq!(index.affected_nodes(&original.nodes["600"]).len(), 3);
        let mut indexed = original.clone();
        let mut scanned = original.clone();
        let mut receipts = Vec::new();
        for id in 0..1_200 {
            let node = original.nodes[&id.to_string()].clone();
            let mut indexed_command = RemoveNodeCommand::new(node.clone());
            let mut scanned_command = RemoveNodeCommand::new(node);
            indexed_command
                .execute_indexed(&mut indexed, Some(&index))
                .unwrap();
            scanned_command
                .execute(&mut scanned, state.clone())
                .await
                .unwrap();
            assert_eq!(
                serde_json::to_value(&indexed_command).unwrap(),
                serde_json::to_value(&scanned_command).unwrap(),
                "removal {id} must retain the same undo receipt"
            );
            receipts.push(indexed_command);
        }
        assert_eq!(
            serde_json::to_value(&indexed.nodes).unwrap(),
            serde_json::to_value(&scanned.nodes).unwrap()
        );
        assert_eq!(indexed.nodes.len(), 2);

        let mut batched = original.clone();
        let executed = batched
            .execute_commands(
                receipts
                    .iter()
                    .cloned()
                    .map(GenericCommand::RemoveNode)
                    .collect(),
                state.clone(),
            )
            .await
            .unwrap();
        assert_eq!(node_state(&batched), node_state(&scanned));
        let mut replay = original.clone();
        let wire = serde_json::to_vec(&executed).unwrap();
        replay
            .execute_commands(serde_json::from_slice(&wire).unwrap(), state.clone())
            .await
            .unwrap();
        assert_eq!(node_state(&replay), node_state(&batched));
        batched.undo(executed, state.clone()).await.unwrap();
        assert_eq!(node_state(&batched), node_state(&original));

        for command in receipts.iter_mut().rev() {
            command.undo(&mut indexed, state.clone()).await.unwrap();
        }
        assert_eq!(
            serde_json::to_value(&indexed.nodes).unwrap(),
            serde_json::to_value(&original.nodes).unwrap()
        );
    }

    #[flow_like_types::tokio::test]
    #[ignore = "manual timing comparison for large FlowScript deletion batches"]
    async fn benchmark_large_removal_batch() {
        let state = state();
        let original = chain(2_402);
        let commands = (0..1_200)
            .map(|id| RemoveNodeCommand::new(original.nodes[&id.to_string()].clone()))
            .collect::<Vec<_>>();
        let mut scanned = original.clone();
        let mut scanned_commands = commands.clone();
        let start = std::time::Instant::now();
        for command in &mut scanned_commands {
            command.execute(&mut scanned, state.clone()).await.unwrap();
        }
        let scan_duration = start.elapsed();

        let mut indexed = original.clone();
        let mut indexed_commands = commands.clone();
        let start = std::time::Instant::now();
        let index = NodeRemovalIndex::new(&indexed);
        for command in &mut indexed_commands {
            command.execute_indexed(&mut indexed, Some(&index)).unwrap();
        }
        let indexed_duration = start.elapsed();

        let mut batched = original;
        let batch = commands
            .into_iter()
            .map(GenericCommand::RemoveNode)
            .collect();
        let start = std::time::Instant::now();
        batched.execute_commands(batch, state).await.unwrap();
        let batch_duration = start.elapsed();
        assert_eq!(node_state(&scanned), node_state(&indexed));
        assert_eq!(node_state(&scanned), node_state(&batched));
        eprintln!(
            "1,200 removals from 2,402 nodes: scan={scan_duration:?}, indexed={indexed_duration:?} ({:.1}x), complete batch={batch_duration:?}",
            scan_duration.as_secs_f64() / indexed_duration.as_secs_f64(),
        );
    }

    #[flow_like_types::tokio::test]
    async fn removal_batch_rebuilds_index_after_a_reference_update() {
        let state = state();
        let mut board = chain(5);
        let mut referencing = board.nodes["4"].clone();
        referencing.fn_refs = Some(FnRefs {
            fn_refs: vec!["2".to_string()],
            can_reference_fns: true,
            can_be_referenced_by_fns: true,
        });
        let remove =
            |id: &str| GenericCommand::RemoveNode(RemoveNodeCommand::new(board.nodes[id].clone()));
        let commands = vec![
            remove("0"),
            remove("1"),
            GenericCommand::UpdateNode(UpdateNodeCommand::new(referencing)),
            remove("2"),
            remove("3"),
        ];
        let original = board.clone();
        let executed = board
            .execute_commands(commands, state.clone())
            .await
            .unwrap();
        let remaining = &board.nodes["4"];
        assert!(remaining.fn_refs.as_ref().unwrap().fn_refs.is_empty());
        assert!(
            remaining
                .get_pin_by_name("input")
                .unwrap()
                .depends_on
                .is_empty()
        );

        let mut replay = original;
        replay.execute_commands(executed, state).await.unwrap();
        assert_eq!(
            serde_json::to_value(&board.nodes).unwrap(),
            serde_json::to_value(&replay.nodes).unwrap()
        );
    }

    #[test]
    fn removal_index_keeps_all_owners_of_a_duplicate_pin() {
        let mut board = chain(3);
        let duplicate = board.nodes["1"].get_pin_by_name("input").unwrap().clone();
        board
            .nodes
            .get_mut("2")
            .unwrap()
            .pins
            .insert(duplicate.id.clone(), duplicate);
        let index = NodeRemovalIndex::new(&board);
        assert_eq!(
            index.affected_nodes(&board.nodes["0"]),
            HashSet::from(["1".to_string(), "2".to_string()])
        );
    }

    #[test]
    fn indexed_removals_preserve_scan_behavior_with_mismatched_node_keys() {
        for embedded_id in ["observer-node", "0"] {
            let mut original = chain(4);
            let observer = original.nodes.get_mut("3").unwrap();
            observer.id = embedded_id.to_string();
            observer.fn_refs = Some(FnRefs {
                fn_refs: vec!["0".to_string(), "2".to_string()],
                can_reference_fns: true,
                can_be_referenced_by_fns: true,
            });
            let index = NodeRemovalIndex::new(&original);
            let mut indexed = original.clone();
            let mut scanned = original.clone();
            for removed_id in ["0", "2"] {
                let node = original.nodes[removed_id].clone();
                let mut indexed_command = RemoveNodeCommand::new(node.clone());
                let mut scanned_command = RemoveNodeCommand::new(node);
                indexed_command
                    .execute_indexed(&mut indexed, Some(&index))
                    .unwrap();
                scanned_command.execute_indexed(&mut scanned, None).unwrap();
                assert_eq!(
                    serde_json::to_value(&indexed_command).unwrap(),
                    serde_json::to_value(&scanned_command).unwrap(),
                    "removal {removed_id} must keep the scan receipt when node 3 stores id {embedded_id}"
                );
                assert_eq!(
                    serde_json::to_value(&indexed.nodes).unwrap(),
                    serde_json::to_value(&scanned.nodes).unwrap(),
                    "removal {removed_id} must keep the scan result when node 3 stores id {embedded_id}"
                );
            }
        }
    }
}
