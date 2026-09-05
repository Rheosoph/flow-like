use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use crate::{
    flow::{
        board::{Board, Comment, Layer, LayerType, commands::Command},
        node::Node,
        pin::{Pin, PinType},
        variable::Variable,
    },
    state::FlowLikeState,
};
use flow_like_types::async_trait;
use flow_like_types::{create_id, json::from_slice};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopyPasteCommand {
    pub original_nodes: Vec<Node>,
    pub original_comments: Vec<Comment>,
    pub original_layers: Vec<Layer>,
    #[serde(default)]
    pub original_variables: Vec<Variable>,
    #[serde(default)]
    pub original_refs: HashMap<String, String>,
    pub new_comments: Vec<Comment>,
    pub new_nodes: Vec<Node>,
    pub new_layers: Vec<Layer>,
    #[serde(default)]
    pub added_refs: Vec<String>,
    #[serde(default)]
    pub added_variables: Vec<String>,
    pub current_layer: Option<String>,
    pub old_mouse: Option<(f32, f32, f32)>,
    pub offset: (f32, f32, f32),
    /// Source id → minted id for every node, pin and layer this paste created.
    ///
    /// Payloads that live outside the board graph — a page hook naming an `events_simple` node, a
    /// widget binding naming a workflow — have no other way to follow the copy. Kept out of the
    /// wire format on purpose: the map describes ids this machine minted, and a replay elsewhere
    /// mints its own, so shipping it would only invite a consumer to trust foreign ids. Replaying
    /// a pre-computed paste (`new_nodes` already populated) therefore leaves it empty.
    #[serde(skip)]
    pub translated_ids: HashMap<String, String>,
}

impl CopyPasteCommand {
    pub fn new(
        original_nodes: Vec<Node>,
        comments: Vec<Comment>,
        layers: Vec<Layer>,
        offset: (f32, f32, f32),
    ) -> Self {
        CopyPasteCommand {
            original_nodes,
            original_comments: comments,
            original_layers: layers,
            original_variables: vec![],
            original_refs: HashMap::new(),
            old_mouse: None,
            current_layer: None,
            offset,
            new_nodes: vec![],
            new_comments: vec![],
            new_layers: vec![],
            added_refs: vec![],
            added_variables: vec![],
            translated_ids: HashMap::new(),
        }
    }

    fn validate_original_refs(&self) -> flow_like_types::Result<()> {
        if let Some(key) = self
            .original_refs
            .keys()
            .find(|key| crate::flow::board::is_internal_board_ref(key))
        {
            return Err(flow_like_types::anyhow!(
                "copy/paste payload contains reserved internal board reference '{key}'"
            ));
        }
        Ok(())
    }

    /// Restore references that are genuinely missing from the pasted node's destination scope.
    ///
    /// Function locals live on their owning layer rather than in `board.variables`. Checking only
    /// the board map turns a valid local reference into a new global with the same id. Resolve the
    /// nearest enclosing function first, matching execution and editor behavior, before falling
    /// back to the historical global-variable recovery path.
    fn restore_missing_variables(&mut self, board: &mut Board) {
        self.added_variables.clear();

        for (index, node) in self.new_nodes.iter().enumerate() {
            for pin in node.pins.values() {
                if pin.name != "var_ref" {
                    continue;
                }
                let Some(var_ref) = pin.default_value.as_deref() else {
                    continue;
                };
                let Ok(var_ref) = from_slice::<String>(var_ref) else {
                    continue;
                };
                if variable_resolves_for_node(board, node, &var_ref) {
                    continue;
                }

                let variable = self
                    .original_variables
                    .iter()
                    .find(|variable| variable.id == var_ref)
                    .cloned()
                    .unwrap_or_else(|| {
                        let source_node = self.original_nodes.get(index).unwrap_or(node);
                        fallback_variable(source_node, node, pin, &var_ref)
                    });

                board.variables.insert(var_ref.clone(), variable);
                self.added_variables.push(var_ref);
            }
        }
    }
}

fn variable_resolves_for_node(board: &Board, node: &Node, variable_id: &str) -> bool {
    let mut current_layer = node.layer.as_deref();
    let mut seen = HashSet::new();
    while let Some(layer_id) = current_layer {
        if !seen.insert(layer_id) {
            break;
        }
        let Some(layer) = board.layers.get(layer_id) else {
            break;
        };
        if matches!(layer.r#type, LayerType::Function) {
            if layer.variables.contains_key(variable_id) {
                return true;
            }
            break;
        }
        current_layer = layer.parent_id.as_deref();
    }

    board.variables.contains_key(variable_id)
}

fn fallback_variable(source_node: &Node, node: &Node, pin: &Pin, id: &str) -> Variable {
    let var_name = if source_node.friendly_name.starts_with("Get ") {
        source_node.friendly_name.replace("Get ", "")
    } else if source_node.friendly_name.starts_with("Set ") {
        source_node.friendly_name.replace("Set ", "")
    } else {
        source_node.friendly_name.clone()
    };
    let value_ref_pin = node
        .pins
        .values()
        .find(|candidate| candidate.name == "value_ref");
    let mut variable = Variable::new(
        &var_name,
        value_ref_pin
            .map(|candidate| candidate.data_type.clone())
            .unwrap_or_else(|| pin.data_type.clone()),
        value_ref_pin
            .map(|candidate| candidate.value_type.clone())
            .unwrap_or_else(|| pin.value_type.clone()),
    );
    variable.id = id.to_string();
    if let Some(value_ref_pin) = value_ref_pin {
        variable.default_value = value_ref_pin.default_value.clone();
        variable.schema = value_ref_pin.schema.clone();
    }
    variable
}

#[async_trait]
impl Command for CopyPasteCommand {
    async fn validate(
        &self,
        _board: &Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        self.validate_original_refs()
    }

    async fn execute(
        &mut self,
        board: &mut Board,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        // Keep the namespace boundary intact even for internal callers that invoke `execute`
        // directly instead of going through `Board::execute_commands` validation.
        self.validate_original_refs()?;
        if !self.new_comments.is_empty()
            || !self.new_nodes.is_empty()
            || !self.new_layers.is_empty()
        {
            for comment in &self.new_comments {
                board.comments.insert(comment.id.clone(), comment.clone());
            }

            for node in &self.new_nodes {
                board.nodes.insert(node.id.clone(), node.clone());
            }

            for layer in &self.new_layers {
                board.layers.insert(layer.id.clone(), layer.clone());
            }

            self.restore_missing_variables(board);

            self.added_refs.clear();
            for (key, value) in &self.original_refs {
                if !board.refs.contains_key(key) {
                    board.refs.insert(key.clone(), value.clone());
                    self.added_refs.push(key.clone());
                }
            }

            return Ok(());
        }

        let node_registry = state.node_registry.read().await.node_registry.clone();

        let mut translated_connection = HashMap::with_capacity(self.original_nodes.len());
        let mut intermediate_nodes = Vec::with_capacity(self.original_nodes.len());
        let mut intermediate_layers = Vec::with_capacity(self.original_layers.len());
        let offset = self.offset;
        let offset = self
            .original_comments
            .first()
            .map(|comment| {
                let old_coors = comment.coordinates;
                (
                    offset.0 - old_coors.0,
                    offset.1 - old_coors.1,
                    offset.2 - old_coors.2,
                )
            })
            .unwrap_or(offset);
        let mut offset = self
            .original_nodes
            .first()
            .map(|node| {
                let old_coors = node.coordinates.unwrap_or((0.0, 0.0, 0.0));
                (
                    offset.0 - old_coors.0,
                    offset.1 - old_coors.1,
                    offset.2 - old_coors.2,
                )
            })
            .unwrap_or(offset);

        if let Some(old_mouse) = self.old_mouse {
            offset = (
                self.offset.0 - old_mouse.0,
                self.offset.1 - old_mouse.1,
                self.offset.2 - old_mouse.2,
            );
        }

        let mut layer_translation = HashMap::with_capacity(self.original_layers.len());

        // First pass: create new IDs and build translation map
        for layer in self.original_layers.iter() {
            let layer_id = create_id();
            layer_translation.insert(layer.id.clone(), layer_id.clone());
            let mut new_layer = layer.clone();
            new_layer.id = layer_id.clone();
            new_layer.coordinates = (
                new_layer.coordinates.0 + offset.0,
                new_layer.coordinates.1 + offset.1,
                new_layer.coordinates.2 + offset.2,
            );

            // Handle parent_id translation for nested layers
            if new_layer.parent_id.is_none() || new_layer.parent_id == Some("".to_string()) {
                new_layer.parent_id = self.current_layer.clone();
            } else if let Some(parent_id) = new_layer.parent_id.clone() {
                // If parent is also being pasted, it will be translated in the second pass
                // For now, keep the original parent_id - it will be updated below
                new_layer.parent_id = Some(parent_id);
            }

            new_layer.pins = layer
                .pins
                .values()
                .map(|pin| {
                    let mut pin = pin.clone();
                    let old_pin_id = pin.id.clone();
                    let new_pin_id = create_id();
                    translated_connection.insert(old_pin_id, new_pin_id.clone());
                    pin.id = new_pin_id.clone();
                    (new_pin_id, pin)
                })
                .collect();

            intermediate_layers.push(new_layer.clone());
        }

        // Second pass: translate parent_ids now that all layer IDs are known
        for layer in intermediate_layers.iter_mut() {
            if let Some(parent_id) = &layer.parent_id
                && let Some(new_parent_id) = layer_translation.get(parent_id)
            {
                layer.parent_id = Some(new_parent_id.clone());
            }
            // Don't insert yet - pin connections need to be translated first in the final pass
        }

        for comment in self.original_comments.iter() {
            let mut new_comment = comment.clone();
            new_comment.id = create_id();
            new_comment.coordinates = (
                new_comment.coordinates.0 + offset.0,
                new_comment.coordinates.1 + offset.1,
                new_comment.coordinates.2 + offset.2,
            );

            if new_comment.layer.is_none() || new_comment.layer == Some("".to_string()) {
                new_comment.layer = self.current_layer.clone();
            } else if let Some(layer_id) = new_comment.layer.clone()
                && let Some(new_layer_id) = layer_translation.get(&layer_id)
            {
                new_comment.layer = Some(new_layer_id.clone());
            }

            board
                .comments
                .insert(new_comment.id.clone(), new_comment.clone());
            self.new_comments.push(new_comment);
        }

        for node in self.original_nodes.iter() {
            let mut new_node = node.clone();
            let blueprint_node = node_registry.get_node(&node.name).ok();

            let blueprint_node = blueprint_node.unwrap_or(node.clone());
            let old_id = new_node.id.clone();
            let new_id = create_id();
            translated_connection.insert(old_id, new_id.clone());
            new_node.id = new_id.clone();
            new_node.category = blueprint_node.category.clone();
            new_node.docs = blueprint_node.docs.clone();
            new_node.icon = blueprint_node.icon.clone();
            new_node.scores = blueprint_node.scores.clone();
            new_node.start = blueprint_node.start;
            new_node.event_callback = blueprint_node.event_callback;
            new_node.wasm = blueprint_node.wasm.clone();
            // Keep the source schema version. Board::node_updates runs after the
            // command and must be able to detect stale copied nodes.
            new_node.long_running = blueprint_node.long_running;
            new_node.only_offline = blueprint_node.only_offline;

            // Preserve user-customized friendly_name and description for start nodes (events)
            let is_start_node = blueprint_node.start.unwrap_or(false);
            if !is_start_node {
                new_node.description = blueprint_node.description.clone();
            }
            new_node.coordinates = Some((
                new_node.coordinates.unwrap_or((0.0, 0.0, 0.0)).0 + offset.0,
                new_node.coordinates.unwrap_or((0.0, 0.0, 0.0)).1 + offset.1,
                new_node.coordinates.unwrap_or((0.0, 0.0, 0.0)).2 + offset.2,
            ));

            if new_node.layer.is_none() || new_node.layer == Some("".to_string()) {
                new_node.layer = self.current_layer.clone();
            } else if let Some(layer_id) = new_node.layer.clone()
                && let Some(new_layer_id) = layer_translation.get(&layer_id)
            {
                new_node.layer = Some(new_layer_id.clone());
            }

            new_node.pins = new_node
                .pins
                .values()
                .map(|pin| {
                    let mut pin = pin.clone();
                    let old_pin_id = pin.id.clone();
                    let (_, blueprint_pin) = blueprint_node
                        .pins
                        .iter()
                        .find(|(_, p)| p.name == pin.name && pin.pin_type == p.pin_type)
                        .unwrap_or((&String::new(), &pin));
                    let blueprint_pin = blueprint_pin.clone();
                    let new_pin_id = create_id();
                    translated_connection.insert(old_pin_id, new_pin_id.clone());
                    pin.id = new_pin_id.clone();
                    pin.description = blueprint_pin.description.clone();

                    // Translate function_layer_id when pasting CallFunction nodes
                    if pin.name == "function_layer_id"
                        && let Some(ref_bytes) = pin.default_value.as_ref()
                        && let Ok(layer_ref) = from_slice::<String>(ref_bytes)
                        && let Some(new_layer_id) = layer_translation.get(&layer_ref)
                        && let Ok(bytes) = flow_like_types::json::to_vec(new_layer_id)
                    {
                        pin.default_value = Some(bytes);
                    }

                    // Only override schema/options from the blueprint when the
                    // pasted pin doesn't already carry one. Dynamic schemas (set
                    // by on_update at runtime) must survive the paste cycle.
                    if pin.schema.is_none() && blueprint_pin.schema.is_some() {
                        pin.schema = blueprint_pin.schema.clone();
                    }
                    if pin.options.is_none() && blueprint_pin.options.is_some() {
                        pin.options = blueprint_pin.options.clone();
                    }

                    if new_node.start.unwrap_or(false)
                        && pin.pin_type == PinType::Input
                        && pin.name != "type"
                    {
                        pin.default_value = None;
                    }

                    (new_pin_id, pin)
                })
                .collect();

            // Preserve user-customized friendly_name for start nodes (events)
            if !is_start_node {
                new_node.friendly_name = blueprint_node.friendly_name.clone();
            }
            intermediate_nodes.push(new_node);
        }

        for node in intermediate_nodes.iter() {
            let mut new_node = node.clone();
            for pin in new_node.pins.values_mut() {
                pin.depends_on = pin
                    .depends_on
                    .iter()
                    .filter(|dep_id| translated_connection.contains_key(*dep_id))
                    .map(|dep_id| translated_connection.get(dep_id).unwrap_or(dep_id).clone())
                    .collect();

                pin.connected_to = pin
                    .connected_to
                    .iter()
                    .filter(|dep_id| translated_connection.contains_key(*dep_id))
                    .map(|dep_id| translated_connection.get(dep_id).unwrap_or(dep_id).clone())
                    .collect();
            }

            // Remap fn_refs to new node IDs (validation deferred until all nodes are inserted)
            if let Some(fn_refs) = &mut new_node.fn_refs {
                fn_refs.fn_refs = fn_refs
                    .fn_refs
                    .iter()
                    .filter_map(|ref_id| translated_connection.get(ref_id).cloned())
                    .collect();
            }

            board.nodes.insert(new_node.id.clone(), new_node.clone());
            self.new_nodes.push(new_node);
        }

        // Validate fn_refs now that all pasted nodes exist in the board
        for node in self.new_nodes.iter_mut() {
            if let Some(fn_refs) = &mut node.fn_refs {
                super::validate_and_deduplicate_fn_refs(fn_refs, board);
            }
            board.nodes.insert(node.id.clone(), node.clone());
        }

        for layer in intermediate_layers.iter() {
            let mut new_layer = layer.clone();
            for pin in new_layer.pins.values_mut() {
                pin.depends_on = pin
                    .depends_on
                    .iter()
                    .filter(|dep_id| translated_connection.contains_key(*dep_id))
                    .map(|dep_id| translated_connection.get(dep_id).unwrap_or(dep_id).clone())
                    .collect();

                pin.connected_to = pin
                    .connected_to
                    .iter()
                    .filter(|dep_id| translated_connection.contains_key(*dep_id))
                    .map(|dep_id| translated_connection.get(dep_id).unwrap_or(dep_id).clone())
                    .collect();
            }
            board.layers.insert(new_layer.id.clone(), new_layer.clone());
            self.new_layers.push(new_layer);
        }

        self.restore_missing_variables(board);

        // Restore referenced schemas/refs that don't already exist in the board
        self.added_refs.clear();
        for (key, value) in &self.original_refs {
            if !board.refs.contains_key(key) {
                board.refs.insert(key.clone(), value.clone());
                self.added_refs.push(key.clone());
            }
        }

        self.translated_ids = translated_connection;

        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        for node in self.new_nodes.iter() {
            board.nodes.remove(&node.id);
        }

        for comment in self.new_comments.iter() {
            board.comments.remove(&comment.id);
        }

        for layer in self.new_layers.iter() {
            board.layers.remove(&layer.id);
        }

        for ref_key in &self.added_refs {
            board.refs.remove(ref_key);
        }
        self.added_refs.clear();

        for var_key in &self.added_variables {
            board.variables.remove(var_key);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        flow::{
            pin::ValueType,
            variable::{Variable, VariableType},
        },
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_storage::Path;

    fn state() -> Arc<FlowLikeState> {
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ))
    }

    fn local_variable(id: &str) -> Variable {
        let mut variable = Variable::new("Local", VariableType::String, ValueType::Normal);
        variable.id = id.to_string();
        variable
    }

    fn variable_get(variable_id: &str, layer: Option<String>) -> Node {
        let mut node = Node::new("variable_get", "Get Local", "", "Variables");
        node.layer = layer;
        node.add_input_pin("var_ref", "Variable", "", VariableType::String)
            .default_value = Some(flow_like_types::json::to_vec(variable_id).unwrap());
        node
    }

    fn variable_ref(node: &Node) -> Option<String> {
        node.get_pin_by_name("var_ref")
            .and_then(|pin| pin.default_value.as_deref())
            .and_then(|value| from_slice::<String>(value).ok())
    }

    #[test]
    fn rejects_internal_refs_from_copy_paste_payloads() {
        let mut command =
            CopyPasteCommand::new(Vec::new(), Vec::new(), Vec::new(), (0.0, 0.0, 0.0));
        command.original_refs.insert(
            format!(
                "{}copilot-receipt/test",
                crate::flow::board::INTERNAL_BOARD_REF_PREFIX
            ),
            "private".to_string(),
        );

        let error = command
            .validate_original_refs()
            .expect_err("reserved refs must not be accepted from copy/paste payloads");
        assert!(
            error
                .to_string()
                .contains("reserved internal board reference")
        );
    }

    #[test]
    fn accepts_public_refs_from_copy_paste_payloads() {
        let mut command =
            CopyPasteCommand::new(Vec::new(), Vec::new(), Vec::new(), (0.0, 0.0, 0.0));
        command
            .original_refs
            .insert("schema/customer".to_string(), "public".to_string());

        command
            .validate_original_refs()
            .expect("ordinary board refs must remain copyable");
    }

    #[flow_like_types::tokio::test]
    async fn paste_reuses_enclosing_function_local_on_execute_and_redo() {
        let mut board = Board::new_detached(Some("board".into()), Path::default());
        let mut function = Layer::new("function".into(), "Function".into(), LayerType::Function);
        let local = local_variable("local");
        function.variables.insert(local.id.clone(), local.clone());

        let mut group = Layer::new("group".into(), "Group".into(), LayerType::Collapsed);
        group.parent_id = Some(function.id.clone());
        board.layers.insert(function.id.clone(), function);
        board.layers.insert(group.id.clone(), group.clone());

        // The clipboard omits a node's layer when it is copied from the open layer. The paste
        // command places it into `current_layer`, which can itself be nested in the function.
        let source = variable_get("local", None);
        let mut command =
            CopyPasteCommand::new(vec![source], Vec::new(), Vec::new(), (0.0, 0.0, 0.0));
        command.current_layer = Some(group.id.clone());
        // Replays receive the executed command, including copied variable metadata. Keeping this
        // here makes the second execute independently cover the precomputed-command branch.
        command.original_variables.push(local);

        command
            .execute(&mut board, state())
            .await
            .expect("initial paste");

        assert!(!board.variables.contains_key("local"));
        assert!(board.layers["function"].variables.contains_key("local"));
        assert!(command.added_variables.is_empty());
        let pasted_id = command.new_nodes[0].id.clone();
        assert_eq!(board.nodes[&pasted_id].layer.as_deref(), Some("group"));
        assert_eq!(
            variable_ref(&board.nodes[&pasted_id]).as_deref(),
            Some("local")
        );

        command.undo(&mut board, state()).await.expect("undo paste");
        assert!(!board.nodes.contains_key(&pasted_id));
        assert!(!board.variables.contains_key("local"));
        assert!(board.layers["function"].variables.contains_key("local"));

        command
            .execute(&mut board, state())
            .await
            .expect("redo paste");

        assert!(board.nodes.contains_key(&pasted_id));
        assert_eq!(board.nodes[&pasted_id].layer.as_deref(), Some("group"));
        assert_eq!(
            variable_ref(&board.nodes[&pasted_id]).as_deref(),
            Some("local")
        );
        assert!(!board.variables.contains_key("local"));
        assert!(board.layers["function"].variables.contains_key("local"));
        assert!(command.added_variables.is_empty());
    }

    #[test]
    fn a_local_in_another_function_does_not_resolve_for_the_pasted_node() {
        let mut board = Board::new_detached(Some("board".into()), Path::default());
        let mut first = Layer::new("first".into(), "First".into(), LayerType::Function);
        let local = local_variable("local");
        first.variables.insert(local.id.clone(), local);
        let second = Layer::new("second".into(), "Second".into(), LayerType::Function);
        board.layers.insert(first.id.clone(), first);
        board.layers.insert(second.id.clone(), second);

        let node = variable_get("local", Some("second".into()));

        assert!(!variable_resolves_for_node(&board, &node, "local"));
    }

    #[flow_like_types::tokio::test]
    async fn pasting_a_function_uses_the_local_on_its_copied_layer() {
        let mut board = Board::new_detached(Some("board".into()), Path::default());
        let mut function = Layer::new("source".into(), "Function".into(), LayerType::Function);
        let local = local_variable("local");
        function.variables.insert(local.id.clone(), local);
        let source = variable_get("local", Some(function.id.clone()));
        let mut command =
            CopyPasteCommand::new(vec![source], Vec::new(), vec![function], (0.0, 0.0, 0.0));

        command
            .execute(&mut board, state())
            .await
            .expect("paste function");

        assert!(!board.variables.contains_key("local"));
        assert!(command.added_variables.is_empty());
        let copied_function = &command.new_layers[0];
        assert!(copied_function.variables.contains_key("local"));
        assert_eq!(
            command.new_nodes[0].layer.as_deref(),
            Some(copied_function.id.as_str())
        );
        assert!(
            board.layers[&copied_function.id]
                .variables
                .contains_key("local")
        );
        assert_eq!(
            board.nodes[&command.new_nodes[0].id].layer.as_deref(),
            Some(copied_function.id.as_str())
        );
    }
}
