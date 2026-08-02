use super::{
    execution::LogLevel,
    node::{Node, NodeLogic},
    pin::Pin,
    variable::{Variable, VariableType},
};
use crate::{
    a2ui::widget::Page,
    app::App,
    state::FlowLikeState,
    utils::compression::{
        compress_to_file, compress_to_file_create, compress_to_file_update, from_compressed,
        from_compressed_json, from_compressed_with_meta,
    },
};
use commands::GenericCommand;
use flow_like_storage::object_store::{self, ObjectStore, UpdateVersion, path::Path};
use flow_like_types::proto;
use flow_like_types::{FromProto, ToProto, create_id, sync::Mutex};
use futures::StreamExt;
use highway::{HighwayHash, HighwayHasher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
    time::SystemTime,
};
use tracing::instrument;

pub mod cleanup;
pub mod commands;

/// Reserved board-ref namespace for host bookkeeping that must be persisted atomically with a
/// board mutation but must never participate in FlowScript, semantic fingerprints, or user-facing
/// context. Values under this prefix are opaque to the workflow engine.
pub const INTERNAL_BOARD_REF_PREFIX: &str = "__flow_like_internal_v1/";

pub fn is_internal_board_ref(key: &str) -> bool {
    key.starts_with(INTERNAL_BOARD_REF_PREFIX)
}

#[derive(Debug, Clone)]
pub enum BoardParent {
    App(Weak<Mutex<App>>),
}

/// An immutable board snapshot that has been fully written but has not yet
/// been made the floating draft's current version.
///
/// Callers that publish other metadata referring to the snapshot should commit
/// that metadata first, then call [`Board::commit_prepared_snapshot`]. Keeping
/// these two phases separate prevents a failed metadata write from advancing
/// the user's draft and losing the fact that it still differs from the
/// previously published action implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreparedBoardSnapshot {
    board_id: String,
    version: (u32, u32, u32),
}

impl PreparedBoardSnapshot {
    pub fn board_id(&self) -> &str {
        &self.board_id
    }

    pub fn version(&self) -> (u32, u32, u32) {
        self.version
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq)]
pub enum ExecutionStage {
    Dev,
    Int,
    QA,
    PreProd,
    Prod,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Default, Debug, PartialEq, Eq)]
pub enum ExecutionMode {
    #[default]
    Hybrid,
    Remote,
    Local,
}
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub enum LayerType {
    Function,
    Macro,
    Collapsed,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub enum VersionType {
    Major,
    Minor,
    Patch,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct Layer {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub r#type: LayerType,
    pub nodes: HashMap<String, Node>,
    pub variables: HashMap<String, Variable>,
    pub comments: HashMap<String, Comment>,
    pub coordinates: (f32, f32, f32),
    pub in_coordinates: Option<(f32, f32, f32)>,
    pub out_coordinates: Option<(f32, f32, f32)>,
    pub pins: HashMap<String, Pin>,
    pub comment: Option<String>,
    pub error: Option<String>,
    pub color: Option<String>,
    pub hash: Option<u64>,
}

impl Layer {
    pub fn new(id: String, name: String, r#type: LayerType) -> Self {
        Layer {
            id,
            parent_id: None,
            name,
            r#type,
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            coordinates: (0.0, 0.0, 0.0),
            in_coordinates: None,
            out_coordinates: None,
            pins: HashMap::new(),
            comment: None,
            error: None,
            color: None,
            hash: None,
        }
    }

    pub fn hash(&mut self) {
        let mut hasher = HighwayHasher::new(highway::Key([
            0x0123456789abcdfe,
            0xfedcba9876543210,
            0x0011223344556677,
            0x8899aabbccddeeff,
        ]));

        hasher.append(self.id.as_bytes());
        hasher.append(self.name.as_bytes());
        hasher.append(format!("{:?}", self.r#type).as_bytes());

        if let Some(parent_id) = &self.parent_id {
            hasher.append(parent_id.as_bytes());
        }

        let mut sorted_nodes: Vec<_> = self.nodes.iter().collect();
        sorted_nodes.sort_by_key(|(id, _)| *id);
        for (id, node) in sorted_nodes {
            hasher.append(id.as_bytes());
            hasher.append(node.id.as_bytes());
        }

        let mut sorted_variables: Vec<_> = self.variables.iter().collect();
        sorted_variables.sort_by_key(|(id, _)| *id);
        for (id, variable) in sorted_variables {
            hasher.append(id.as_bytes());
            hasher.append(variable.id.as_bytes());
        }

        let mut sorted_comments: Vec<_> = self.comments.iter().collect();
        sorted_comments.sort_by_key(|(id, _)| *id);
        for (id, comment) in sorted_comments {
            hasher.append(id.as_bytes());
            hasher.append(comment.id.as_bytes());
        }

        let mut sorted_pins: Vec<_> = self.pins.iter().collect();
        sorted_pins.sort_by_key(|(id, _)| *id);
        for (_id, pin) in sorted_pins {
            pin.hash(&mut hasher);
        }

        hasher.append(&self.coordinates.0.to_le_bytes());
        hasher.append(&self.coordinates.1.to_le_bytes());
        hasher.append(&self.coordinates.2.to_le_bytes());

        if let Some(comment) = &self.comment {
            hasher.append(comment.as_bytes());
        }

        if let Some(color) = &self.color {
            hasher.append(color.as_bytes());
        }

        self.hash = Some(hasher.finalize64());
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct Board {
    pub id: String,
    pub name: String,
    pub description: String,
    pub nodes: HashMap<String, Node>,
    pub variables: HashMap<String, Variable>,
    pub comments: HashMap<String, Comment>,
    pub viewport: (f32, f32, f32),
    pub version: (u32, u32, u32),
    pub stage: ExecutionStage,
    pub log_level: LogLevel,
    pub execution_mode: ExecutionMode,
    pub refs: HashMap<String, String>,
    /// Persisted host bookkeeping, intentionally excluded from Board JSON and all semantic
    /// workflow surfaces. External crates can only access this map through the prefix-validating
    /// methods on `Board`.
    #[serde(skip)]
    pub(crate) internal_refs: HashMap<String, String>,
    pub layers: HashMap<String, Layer>,
    pub page_ids: Vec<String>,
    pub hash: Option<u64>,

    pub created_at: SystemTime,
    pub updated_at: SystemTime,

    #[serde(skip)]
    pub parent: Option<BoardParent>,

    #[serde(skip)]
    pub board_dir: Path,

    #[serde(skip)]
    pub logic_nodes: HashMap<String, Arc<dyn NodeLogic>>,

    #[serde(skip)]
    pub app_state: Option<Arc<FlowLikeState>>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct BoardUndoRedoStack {
    pub undo_stack: Vec<String>,
    pub redo_stack: Vec<String>,
}

fn append_optional_u64(hasher: &mut HighwayHasher, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.append(&[1]);
            hasher.append(&value.to_le_bytes());
        }
        None => hasher.append(&[0]),
    }
}

fn stage_marker(stage: &ExecutionStage) -> u8 {
    match stage {
        ExecutionStage::Dev => 0,
        ExecutionStage::Int => 1,
        ExecutionStage::QA => 2,
        ExecutionStage::PreProd => 3,
        ExecutionStage::Prod => 4,
    }
}

fn execution_mode_marker(mode: &ExecutionMode) -> u8 {
    match mode {
        ExecutionMode::Hybrid => 0,
        ExecutionMode::Remote => 1,
        ExecutionMode::Local => 2,
    }
}

impl Board {
    /// Create a new board with a unique ID
    /// The board is created in the base directory appended with the ID
    pub fn new(id: Option<String>, base_dir: Path, app_state: Arc<FlowLikeState>) -> Self {
        let mut board = Self::new_detached(id, base_dir);
        board.app_state = Some(app_state);
        board
    }

    /// Create a board without runtime state. Deterministic transforms, importers, and fixtures can
    /// use this constructor and attach credentials before calling storage or execution methods.
    pub fn new_detached(id: Option<String>, base_dir: Path) -> Self {
        let id = id.unwrap_or(create_id());
        let board_dir = base_dir;

        let mut board = Board {
            id,
            name: "New Board".to_string(),
            description: "Your new Workflow!".to_string(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            log_level: LogLevel::Info,
            stage: ExecutionStage::Dev,
            execution_mode: ExecutionMode::Hybrid,
            viewport: (0.0, 0.0, 0.0),
            version: (0, 0, 1),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            parent: None,
            board_dir,
            logic_nodes: HashMap::new(),
            app_state: None,
        };
        board.hash();
        board
    }

    pub fn mark_changed(&mut self) {
        self.updated_at = SystemTime::now();
        self.hash();
    }

    /// Read one host-owned board reference. Public workflow refs are deliberately inaccessible
    /// through this API, and a non-reserved key can never alias into the internal namespace.
    pub fn internal_ref(&self, key: &str) -> Option<&str> {
        is_internal_board_ref(key)
            .then(|| self.internal_refs.get(key).map(String::as_str))
            .flatten()
    }

    /// Persist one host-owned board reference after enforcing the reserved namespace boundary.
    pub fn insert_internal_ref(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> flow_like_types::Result<Option<String>> {
        let key = key.into();
        if !is_internal_board_ref(&key) {
            return Err(flow_like_types::anyhow!(
                "internal board reference keys must start with '{INTERNAL_BOARD_REF_PREFIX}'"
            ));
        }
        Ok(self.internal_refs.insert(key, value.into()))
    }

    /// Remove one host-owned board reference. Non-reserved keys are never removed by this API.
    pub fn remove_internal_ref(&mut self, key: &str) -> Option<String> {
        is_internal_board_ref(key)
            .then(|| self.internal_refs.remove(key))
            .flatten()
    }

    /// Iterate over one reserved sub-namespace without exposing the backing map for mutation.
    pub fn internal_refs_with_prefix<'a>(
        &'a self,
        prefix: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a str)> + 'a {
        let valid_prefix = is_internal_board_ref(prefix);
        self.internal_refs.iter().filter_map(move |(key, value)| {
            (valid_prefix && key.starts_with(prefix)).then_some((key.as_str(), value.as_str()))
        })
    }

    /// Retain selected values in one reserved sub-namespace while leaving every other host-owned
    /// namespace untouched.
    pub fn retain_internal_refs_with_prefix<F>(
        &mut self,
        prefix: &str,
        mut retain: F,
    ) -> flow_like_types::Result<()>
    where
        F: FnMut(&str, &str) -> bool,
    {
        if !is_internal_board_ref(prefix) {
            return Err(flow_like_types::anyhow!(
                "internal board reference prefixes must start with '{INTERNAL_BOARD_REF_PREFIX}'"
            ));
        }
        self.internal_refs
            .retain(|key, value| !key.starts_with(prefix) || retain(key.as_str(), value.as_str()));
        self.internal_refs.shrink_to_fit();
        Ok(())
    }

    /// Remove all host bookkeeping before a board is copied into a semantic artifact such as a
    /// template, immutable published version, or fork.
    pub fn clear_internal_refs(&mut self) {
        self.internal_refs.clear();
    }

    /// Derives a governed ontology action's parameter schema from the start
    /// node's `parameters` struct pin.
    ///
    /// This is the authoritative source: the schema is read from the pinned,
    /// published board that actually executes, so a governed action can never
    /// advertise a contract its implementation board does not honor. Returns
    /// `None` when the start node has no typed `parameters` pin, which callers
    /// treat as "accepts any object payload". A present but malformed schema
    /// is an error: silently treating it as absent would widen the governed
    /// action contract.
    pub fn action_parameter_schema(
        &self,
        start_node_id: &str,
    ) -> flow_like_types::Result<Option<flow_like_types::Value>> {
        let node = self.nodes.get(start_node_id).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Start node '{}' does not exist in the action implementation board",
                start_node_id
            )
        })?;
        let Some(pin) = node.pins.values().find(|pin| {
            pin.name == "parameters"
                && pin.data_type == VariableType::Struct
                && pin.schema.is_some()
        }) else {
            return Ok(None);
        };
        let raw_schema = pin
            .schema
            .as_deref()
            .expect("the selected parameters pin has a schema");
        let parsed: flow_like_types::Value =
            flow_like_types::json::from_str(raw_schema).map_err(|error| {
                flow_like_types::anyhow!(
                    "The parameters schema on start node '{}' is invalid JSON: {error}",
                    start_node_id
                )
            })?;
        if !parsed.is_object() {
            return Err(flow_like_types::anyhow!(
                "The parameters schema on start node '{}' must be a JSON object",
                start_node_id
            ));
        }
        Ok(Some(parsed))
    }

    pub fn hash(&mut self) {
        for node in self.nodes.values_mut() {
            node.hash();
        }
        for variable in self.variables.values_mut() {
            variable.hash();
        }
        for comment in self.comments.values_mut() {
            comment.hash();
        }
        for layer in self.layers.values_mut() {
            for node in layer.nodes.values_mut() {
                node.hash();
            }
            for variable in layer.variables.values_mut() {
                variable.hash();
            }
            for comment in layer.comments.values_mut() {
                comment.hash();
            }
            layer.hash();
        }

        let mut hasher = HighwayHasher::new(highway::Key([
            0x0123456789abcdfe,
            0xfedcba9876543210,
            0x0011223344556677,
            0x8899aabbccddeeff,
        ]));

        hasher.append(self.id.as_bytes());
        hasher.append(self.name.as_bytes());
        hasher.append(self.description.as_bytes());
        hasher.append(&self.version.0.to_le_bytes());
        hasher.append(&self.version.1.to_le_bytes());
        hasher.append(&self.version.2.to_le_bytes());
        hasher.append(&self.viewport.0.to_le_bytes());
        hasher.append(&self.viewport.1.to_le_bytes());
        hasher.append(&self.viewport.2.to_le_bytes());
        hasher.append(&[stage_marker(&self.stage)]);
        hasher.append(&[self.log_level.to_u8()]);
        hasher.append(&[execution_mode_marker(&self.execution_mode)]);

        let mut refs = self
            .refs
            .iter()
            .filter(|(key, _)| !is_internal_board_ref(key))
            .collect::<Vec<_>>();
        refs.sort_by_key(|(key, _)| *key);
        for (key, value) in refs {
            hasher.append(key.as_bytes());
            hasher.append(value.as_bytes());
        }

        for page_id in &self.page_ids {
            hasher.append(page_id.as_bytes());
        }

        let mut nodes = self.nodes.iter().collect::<Vec<_>>();
        nodes.sort_by_key(|(id, _)| *id);
        for (id, node) in nodes {
            hasher.append(id.as_bytes());
            append_optional_u64(&mut hasher, node.hash);
        }

        let mut variables = self.variables.iter().collect::<Vec<_>>();
        variables.sort_by_key(|(id, _)| *id);
        for (id, variable) in variables {
            hasher.append(id.as_bytes());
            append_optional_u64(&mut hasher, variable.hash);
        }

        let mut comments = self.comments.iter().collect::<Vec<_>>();
        comments.sort_by_key(|(id, _)| *id);
        for (id, comment) in comments {
            hasher.append(id.as_bytes());
            append_optional_u64(&mut hasher, comment.hash);
        }

        let mut layers = self.layers.iter().collect::<Vec<_>>();
        layers.sort_by_key(|(id, _)| *id);
        for (id, layer) in layers {
            hasher.append(id.as_bytes());
            append_optional_u64(&mut hasher, layer.hash);

            let mut layer_nodes = layer.nodes.iter().collect::<Vec<_>>();
            layer_nodes.sort_by_key(|(id, _)| *id);
            for (id, node) in layer_nodes {
                hasher.append(id.as_bytes());
                append_optional_u64(&mut hasher, node.hash);
            }

            let mut layer_variables = layer.variables.iter().collect::<Vec<_>>();
            layer_variables.sort_by_key(|(id, _)| *id);
            for (id, variable) in layer_variables {
                hasher.append(id.as_bytes());
                append_optional_u64(&mut hasher, variable.hash);
            }

            let mut layer_comments = layer.comments.iter().collect::<Vec<_>>();
            layer_comments.sort_by_key(|(id, _)| *id);
            for (id, comment) in layer_comments {
                hasher.append(id.as_bytes());
                append_optional_u64(&mut hasher, comment.hash);
            }
        }

        self.hash = Some(hasher.finalize64());
    }

    /// Recompute catalog-derived node schemas for an already-open board.
    ///
    /// Hosts call this when their node/widget registry becomes available or is
    /// replaced after the board was loaded. The refresh is deliberately
    /// in-memory only: it must not make the board look user-edited or persist
    /// schema-only changes behind the user's back.
    pub async fn refresh_node_definitions(&mut self, state: Arc<FlowLikeState>) {
        let updated_at = self.updated_at;
        let hash = self.hash;

        // A registry replacement can also replace NodeLogic implementations.
        // Do not let the board's per-node logic cache pin it to the old one.
        self.logic_nodes.clear();
        self.node_updates(state).await;
        self.cleanup();

        self.updated_at = updated_at;
        self.hash = hash;
    }

    async fn node_updates(&mut self, state: Arc<FlowLikeState>) {
        let registry = state.node_registry().clone();
        let registry = registry.read().await;

        // First, sync node schemas for any version mismatches
        // This runs BEFORE on_update so dynamic nodes can still add their pins
        cleanup::sync_node_schema::sync_board_node_schemas(self, &registry.node_registry).await;

        const MAX_PASSES: usize = 10;
        for _ in 0..MAX_PASSES {
            let mut changed = false;

            let node_ids: Vec<String> = self.nodes.keys().cloned().collect();
            for node_id in node_ids {
                let Some(mut node) = self.nodes.remove(&node_id) else {
                    continue;
                };
                let old_hash = node.hash;

                let node_logic = match self.logic_nodes.get(&node.name) {
                    Some(logic) => Arc::clone(logic),
                    None => match registry.instantiate(&node) {
                        Ok(new_logic) => {
                            self.logic_nodes
                                .insert(node.name.clone(), Arc::clone(&new_logic));
                            Arc::clone(&new_logic)
                        }
                        Err(_) => {
                            self.nodes.insert(node_id, node);
                            continue;
                        }
                    },
                };
                node_logic.on_update(&mut node, self).await;

                node.hash();
                if node.hash != old_hash {
                    changed = true;
                }

                self.nodes.insert(node_id, node);
            }

            let layer_ids: Vec<String> = self.layers.keys().cloned().collect();
            for layer_id in layer_ids {
                let layer_node_ids: Vec<String> = match self.layers.get(&layer_id) {
                    Some(layer) => layer.nodes.keys().cloned().collect(),
                    None => continue,
                };

                for node_id in layer_node_ids {
                    let Some(mut node) = self
                        .layers
                        .get_mut(&layer_id)
                        .and_then(|layer| layer.nodes.remove(&node_id))
                    else {
                        continue;
                    };
                    let old_hash = node.hash;

                    let node_logic = match self.logic_nodes.get(&node.name) {
                        Some(logic) => Arc::clone(logic),
                        None => match registry.instantiate(&node) {
                            Ok(new_logic) => {
                                self.logic_nodes
                                    .insert(node.name.clone(), Arc::clone(&new_logic));
                                Arc::clone(&new_logic)
                            }
                            Err(_) => {
                                if let Some(layer) = self.layers.get_mut(&layer_id) {
                                    layer.nodes.insert(node_id, node);
                                }
                                continue;
                            }
                        },
                    };
                    node_logic.on_update(&mut node, self).await;

                    node.hash();
                    if node.hash != old_hash {
                        changed = true;
                    }

                    if let Some(layer) = self.layers.get_mut(&layer_id) {
                        layer.nodes.insert(node_id, node);
                    }
                }
            }

            if !changed {
                break;
            }
        }

        for layer in self.layers.values_mut() {
            layer.hash();
        }

        for variable in self.variables.values_mut() {
            variable.hash();
        }

        for comment in self.comments.values_mut() {
            comment.hash();
        }
    }

    async fn rollback_commands(
        &mut self,
        commands: &mut [GenericCommand],
        state: Arc<FlowLikeState>,
    ) -> Vec<String> {
        let mut recovery_errors = Vec::new();
        for command in commands.iter_mut().rev() {
            if let Err(error) = command.undo(self, state.clone()).await {
                recovery_errors.push(format!("rollback undo failed: {error}"));
            }
        }
        recovery_errors
    }

    async fn reapply_commands(
        &mut self,
        commands: &mut [GenericCommand],
        state: Arc<FlowLikeState>,
    ) -> Vec<String> {
        let mut recovery_errors = Vec::new();
        for command in commands.iter_mut() {
            if let Err(error) = command.execute(self, state.clone()).await {
                recovery_errors.push(format!("rollback execute failed: {error}"));
            }
        }
        recovery_errors
    }

    pub async fn execute_command(
        &mut self,
        command: GenericCommand,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<GenericCommand> {
        let mut command = command;
        if tracing::enabled!(tracing::Level::DEBUG) {
            let cmd_json = serde_json::to_string(&command).unwrap_or_default();
            tracing::debug!(command = %cmd_json, "Executing board command");
        }
        command.validate(self, state.clone()).await?;
        if let Err(e) = command.execute(self, state.clone()).await {
            let mut recovery_errors = Vec::new();
            if let Err(rollback_error) = command.undo(self, state.clone()).await {
                recovery_errors.push(format!(
                    "failed to rollback current command after execute error: {rollback_error}"
                ));
            }
            tracing::error!(error = ?e, "Board command execution failed");
            let primary_error = e.to_string();
            let error = if recovery_errors.is_empty() {
                e
            } else {
                flow_like_types::anyhow!(
                    "Command execution failed: {primary_error}; recovery errors: {}",
                    recovery_errors.join(" | ")
                )
            };
            return Err(error);
        }
        tracing::debug!("Board command executed successfully");
        self.node_updates(state).await;
        self.cleanup();
        self.mark_changed();
        Ok(command)
    }

    pub async fn execute_commands(
        &mut self,
        commands: Vec<GenericCommand>,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<Vec<GenericCommand>> {
        let mut commands = commands;
        for index in 0..commands.len() {
            if let Err(error) = commands[index].validate(self, state.clone()).await {
                let recovery_errors = self
                    .rollback_commands(&mut commands[..index], state.clone())
                    .await;
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Command batch validation failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }

            if let Err(error) = commands[index].execute(self, state.clone()).await {
                let mut recovery_errors = Vec::new();
                if let Err(rollback_error) = commands[index].undo(self, state.clone()).await {
                    recovery_errors.push(format!(
                        "failed to rollback current command at index {index}: {rollback_error}"
                    ));
                }
                recovery_errors.extend(
                    self.rollback_commands(&mut commands[..index], state.clone())
                        .await,
                );
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Command batch execution failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }
        }
        self.node_updates(state).await;
        self.cleanup();
        self.mark_changed();
        Ok(commands)
    }

    pub async fn undo(
        &mut self,
        commands: Vec<GenericCommand>,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let mut commands = commands;
        for index in (0..commands.len()).rev() {
            if let Err(error) = commands[index].undo(self, state.clone()).await {
                let mut recovery_errors = Vec::new();
                if let Err(rollback_error) = commands[index].execute(self, state.clone()).await {
                    recovery_errors.push(format!(
                        "failed to restore current command at index {index}: {rollback_error}"
                    ));
                }
                recovery_errors.extend(
                    self.reapply_commands(&mut commands[index + 1..], state.clone())
                        .await,
                );
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Undo failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }
        }
        self.node_updates(state).await;
        self.cleanup();
        self.mark_changed();
        Ok(())
    }

    pub async fn redo(
        &mut self,
        commands: Vec<GenericCommand>,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let mut commands = commands;
        for index in 0..commands.len() {
            if let Err(error) = commands[index].validate(self, state.clone()).await {
                let recovery_errors = self
                    .rollback_commands(&mut commands[..index], state.clone())
                    .await;
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Redo validation failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }

            if let Err(error) = commands[index].execute(self, state.clone()).await {
                let mut recovery_errors = Vec::new();
                if let Err(rollback_error) = commands[index].undo(self, state.clone()).await {
                    recovery_errors.push(format!(
                        "failed to rollback current redo command at index {index}: {rollback_error}"
                    ));
                }
                recovery_errors.extend(
                    self.rollback_commands(&mut commands[..index], state.clone())
                        .await,
                );
                return Err(if recovery_errors.is_empty() {
                    error
                } else {
                    flow_like_types::anyhow!(
                        "Redo failed at index {index}: {error}; recovery errors: {}",
                        recovery_errors.join(" | ")
                    )
                });
            }
        }
        self.node_updates(state).await;
        self.cleanup();
        self.mark_changed();
        Ok(())
    }

    pub fn get_pin_by_id(&self, pin_id: &str) -> Option<&Pin> {
        for node in self.nodes.values() {
            if let Some(pin) = node.pins.get(pin_id) {
                return Some(pin);
            }
        }

        for layer in self.layers.values() {
            if let Some(pin) = layer.pins.get(pin_id) {
                return Some(pin);
            }
            for node in layer.nodes.values() {
                if let Some(pin) = node.pins.get(pin_id) {
                    return Some(pin);
                }
            }
        }

        None
    }

    pub fn get_dependent_nodes(&self, node_id: &str) -> Vec<&Node> {
        let mut dependent_nodes = HashMap::new();
        for node in self.nodes.values() {
            for pin in node.pins.values() {
                if pin.depends_on.contains(node_id) {
                    dependent_nodes.insert(&node.id, node);
                }
            }
        }

        dependent_nodes.values().cloned().collect()
    }

    pub fn get_connected_nodes(&self, node_id: &str) -> Vec<&Node> {
        let mut connected_nodes = HashMap::new();
        for node in self.nodes.values() {
            for pin in node.pins.values() {
                if pin.connected_to.contains(node_id) {
                    connected_nodes.insert(&node.id, node);
                }
            }
        }

        connected_nodes.values().cloned().collect()
    }

    pub fn get_variable(&self, variable_id: &str) -> Option<&Variable> {
        self.variables.get(variable_id)
    }

    /// Search for a variable in board globals AND all layer-scoped variables.
    pub fn get_any_variable(&self, variable_id: &str) -> Option<Variable> {
        if let Some(var) = self.variables.get(variable_id) {
            return Some(var.clone());
        }
        for layer in self.layers.values() {
            if let Some(var) = layer.variables.get(variable_id) {
                return Some(var.clone());
            }
        }
        None
    }

    /// Writes an immutable snapshot of the current board (and its pages) at the
    /// given version without changing the working `version` or touching the
    /// floating "latest" board. Used to publish the exact version a governed
    /// ontology action pins, so it can be validated and invoked reproducibly.
    pub async fn snapshot_at_version(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let store = self.get_store(store).await?;

        // Always serialize a board whose embedded version agrees with the
        // immutable path. In particular, the two-phase action publisher calls
        // this while the floating draft still points at its previous version.
        let mut published = self.clone();
        published.version = version;
        published.clear_internal_refs();
        published.hash();

        let board_version_path = self
            .board_dir
            .child("versions")
            .child(self.id.clone())
            .child(format!("{}_{}_{}.board", version.0, version.1, version.2));

        match store.head(&board_version_path).await {
            Ok(_) => {
                if published
                    .snapshot_matches_current(version, Some(store.clone()))
                    .await?
                    && published
                        .snapshot_matches_persisted_draft(version, Some(store.clone()))
                        .await?
                {
                    // A previous attempt may have committed the immutable
                    // snapshot and lost its response. Identical retries are
                    // safe, provided the persisted floating draft still names
                    // that content. A stale process-local board must not make
                    // an old implementation look current.
                    return Ok(());
                }
                return Err(Self::immutable_snapshot_conflict(version));
            }
            Err(object_store::Error::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }

        // Copy pages first and atomically create the board last. The board file
        // is the snapshot's existence marker; once present it can never be
        // replaced by a later draft or a racing publisher.
        for page_id in &self.page_ids {
            let src_path = self.page_path(page_id);
            let dst_path = self.versioned_page_path(version, page_id);
            let page_proto: proto::Page = from_compressed(store.clone(), src_path).await?;
            match store.head(&dst_path).await {
                Ok(_) => {
                    let existing: proto::Page =
                        from_compressed(store.clone(), dst_path.clone()).await?;
                    if existing != page_proto {
                        return Err(Self::immutable_snapshot_conflict(version));
                    }
                }
                Err(object_store::Error::NotFound { .. }) => {
                    if let Err(create_error) =
                        compress_to_file_create(store.clone(), dst_path.clone(), &page_proto).await
                    {
                        // A racing identical publisher is success. A racing
                        // different publisher owns this version slot and must
                        // not be overwritten.
                        let raced: flow_like_types::Result<proto::Page> =
                            from_compressed(store.clone(), dst_path).await;
                        if !matches!(raced, Ok(ref existing) if existing == &page_proto) {
                            return Err(create_error);
                        }
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }

        let board = published.to_proto();
        if let Err(create_error) =
            compress_to_file_create(store.clone(), board_version_path, &board).await
        {
            if !published
                .snapshot_matches_current(version, Some(store.clone()))
                .await
                .unwrap_or(false)
            {
                return Err(create_error);
            }
        }

        // The board object is the publication marker, so do one final read of
        // the authoritative floating draft (and all current page objects)
        // before returning a snapshot that callers may reference from other
        // metadata. This detects a board/page edit that raced the page-first
        // copy and prevents a torn immutable snapshot from being installed as
        // an action implementation. The already-created version remains an
        // inert immutable orphan and a later attempt advances to a fresh patch.
        if !published
            .snapshot_matches_persisted_draft(version, Some(store))
            .await?
        {
            return Err(flow_like_types::anyhow!(
                "Board draft changed while publishing immutable version {}.{}.{}",
                version.0,
                version.1,
                version.2
            ));
        }

        Ok(())
    }

    fn immutable_snapshot_conflict(version: (u32, u32, u32)) -> flow_like_types::Error {
        flow_like_types::anyhow!(
            "Board version {}.{}.{} already contains different immutable snapshot data",
            version.0,
            version.1,
            version.2
        )
    }

    /// Return whether a version slot is empty or contains only immutable
    /// artifacts identical to this draft. A board-last publication can leave
    /// page objects behind when interrupted; identical retries resume them,
    /// while different drafts skip the occupied patch rather than poisoning
    /// all future retries.
    pub async fn snapshot_version_slot_is_compatible(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<bool> {
        let store = self.get_store(store).await?;
        self.snapshot_version_slot_is_compatible_with_store(version, store)
            .await
    }

    async fn snapshot_version_slot_is_compatible_with_store(
        &self,
        version: (u32, u32, u32),
        store: Arc<dyn ObjectStore>,
    ) -> flow_like_types::Result<bool> {
        let board_path = Self::proto_path(&self.board_dir, &self.id, Some(version));
        match store.head(&board_path).await {
            Ok(_) => {
                return self.snapshot_matches_current(version, Some(store)).await;
            }
            Err(object_store::Error::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }

        for page_id in &self.page_ids {
            let path = self.versioned_page_path(version, page_id);
            match store.head(&path).await {
                Ok(_) => {
                    let current: proto::Page =
                        from_compressed(store.clone(), self.page_path(page_id)).await?;
                    let existing: proto::Page = from_compressed(store.clone(), path).await?;
                    if current != existing {
                        return Ok(false);
                    }
                }
                Err(object_store::Error::NotFound { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(true)
    }

    /// Recompute the deterministic board hash instead of trusting a possibly
    /// stale serialized `hash` field.
    pub fn content_hash(&self) -> u64 {
        let mut board = self.clone();
        board.hash();
        board.hash.expect("Board::hash always sets a hash")
    }

    /// Return whether an existing immutable snapshot contains the same board
    /// and page content as the current draft when assigned `version`.
    /// Normalizing the draft version makes this useful between the prepare and
    /// commit phases, while the floating board still carries its old version.
    pub async fn snapshot_matches_current(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<bool> {
        let store = self.get_store(store).await?;
        let proto: proto::Board = from_compressed(
            store.clone(),
            Self::proto_path(&self.board_dir, &self.id, Some(version)),
        )
        .await?;
        let snapshot = Self::from_proto(proto);
        let mut current = self.clone();
        current.version = version;
        current.hash();
        if snapshot.content_hash() != current.content_hash() {
            return Ok(false);
        }
        for page_id in &self.page_ids {
            let current: proto::Page =
                from_compressed(store.clone(), self.page_path(page_id)).await?;
            let published: proto::Page =
                from_compressed(store.clone(), self.versioned_page_path(version, page_id)).await?;
            if current != published {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Compare an immutable version with the authoritative floating board
    /// object and its current page objects, bypassing any process-local board
    /// registry entry. If no floating object exists (only possible for a
    /// not-yet-saved in-memory board), fall back to the caller's draft.
    pub async fn snapshot_matches_persisted_draft(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<bool> {
        let store = self.get_store(store).await?;
        let floating_path = Self::proto_path(&self.board_dir, &self.id, None);
        match store.head(&floating_path).await {
            Ok(_) => {}
            Err(object_store::Error::NotFound { .. }) => {
                return self.snapshot_matches_current(version, Some(store)).await;
            }
            Err(error) => return Err(error.into()),
        }

        let proto: proto::Board = from_compressed(store.clone(), floating_path).await?;
        let mut floating = if let Some(app_state) = self.app_state.clone() {
            Self::from_loaded_proto(proto, self.board_dir.clone(), app_state).await
        } else {
            let mut floating = Self::from_proto(proto);
            floating.board_dir = self.board_dir.clone();
            floating
        };
        floating.app_state = self.app_state.clone();
        floating
            .snapshot_matches_current(version, Some(store))
            .await
    }

    /// Validate an already-created snapshot as the current draft's prepared
    /// publication. This is used to finish a commit whose floating-board save
    /// previously failed after the referring ontology was persisted.
    pub async fn prepared_snapshot_at_version(
        &self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<PreparedBoardSnapshot> {
        if !self.snapshot_matches_current(version, store).await? {
            return Err(Self::immutable_snapshot_conflict(version));
        }
        Ok(PreparedBoardSnapshot {
            board_id: self.id.clone(),
            version,
        })
    }

    /// Prepare a fresh immutable patch snapshot without changing or saving the
    /// floating board. Interrupted page-first attempts are resumed when their
    /// content matches and skipped when another draft owns the candidate slot.
    pub async fn prepare_snapshot_at_fresh_patch_version(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<PreparedBoardSnapshot> {
        let store = self.get_store(store).await?;
        let first_patch = self
            .version
            .2
            .checked_add(1)
            .ok_or_else(|| flow_like_types::anyhow!("Board patch version overflow"))?;
        let mut next = (self.version.0, self.version.1, first_patch);

        loop {
            if self
                .snapshot_version_slot_is_compatible_with_store(next, store.clone())
                .await?
            {
                match self.snapshot_at_version(next, Some(store.clone())).await {
                    Ok(()) => {
                        return Ok(PreparedBoardSnapshot {
                            board_id: self.id.clone(),
                            version: next,
                        });
                    }
                    Err(error) => {
                        // If a different publisher won the race, advance to a
                        // new patch. Preserve unrelated storage failures.
                        if self
                            .snapshot_version_slot_is_compatible_with_store(next, store.clone())
                            .await?
                        {
                            return Err(error);
                        }
                    }
                }
            }
            next.2 = next
                .2
                .checked_add(1)
                .ok_or_else(|| flow_like_types::anyhow!("Board patch version overflow"))?;
        }
    }

    /// Advance the floating board to a prepared immutable snapshot after the
    /// metadata that refers to it has committed. If the user edited the draft
    /// in the meantime, leave it untouched; a subsequent publication will
    /// create another patch from that newer content.
    pub async fn commit_prepared_snapshot(
        &mut self,
        prepared: &PreparedBoardSnapshot,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<bool> {
        if prepared.board_id != self.id || prepared.version < self.version {
            return Ok(false);
        }
        let store = self.get_store(store).await?;
        // Protect unsaved in-process edits first.
        if !self
            .snapshot_matches_current(prepared.version, Some(store.clone()))
            .await?
        {
            return Ok(false);
        }

        // Then compare and conditionally update the authoritative floating
        // object. `open_board` may return a process-local cached board, so an
        // unconditional save here could otherwise overwrite another process's
        // edit made after the immutable snapshot was prepared.
        let floating_path = self.board_dir.child(format!("{}.board", self.id));
        let (floating_proto, floating_meta): (proto::Board, _) =
            match from_compressed_with_meta(store.clone(), floating_path.clone()).await {
                Ok(loaded) => loaded,
                Err(load_error) => match store.head(&floating_path).await {
                    Err(object_store::Error::NotFound { .. }) => {
                        let mut floating = self.clone();
                        floating.version = prepared.version;
                        floating.mark_changed();
                        compress_to_file_create(store, floating_path, &floating.to_proto()).await?;
                        self.version = prepared.version;
                        self.updated_at = floating.updated_at;
                        self.hash();
                        return Ok(true);
                    }
                    _ => return Err(load_error),
                },
            };
        let mut floating = Self::from_proto(floating_proto);
        floating.board_dir = self.board_dir.clone();
        if floating.version > prepared.version
            || !floating
                .snapshot_matches_current(prepared.version, Some(store.clone()))
                .await?
        {
            return Ok(false);
        }
        if floating.version == prepared.version {
            self.version = prepared.version;
            self.hash();
            return Ok(true);
        }

        floating.version = prepared.version;
        floating.mark_changed();
        if floating_meta.e_tag.is_none() && floating_meta.version.is_none() {
            // This backend cannot offer a compare-and-swap token. Leaving the
            // pointer behind is safe because reconciliation recognizes the
            // prepared patch; an unconditional write could lose another
            // writer's draft.
            return Ok(false);
        }
        compress_to_file_update(
            store,
            floating_path,
            &floating.to_proto(),
            UpdateVersion {
                e_tag: floating_meta.e_tag,
                version: floating_meta.version,
            },
        )
        .await?;
        self.version = prepared.version;
        self.updated_at = floating.updated_at;
        self.hash();
        Ok(true)
    }

    /// Publish the current draft under a fresh patch version without touching
    /// any existing versioned object. This is used when an ontology action's
    /// current pin already names an older immutable snapshot but the working
    /// board has since changed.
    pub async fn snapshot_at_fresh_patch_version(
        &mut self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<(u32, u32, u32)> {
        let store = self.get_store(store).await?;
        let prepared = self
            .prepare_snapshot_at_fresh_patch_version(Some(store.clone()))
            .await?;
        let version = prepared.version();
        if !self
            .commit_prepared_snapshot(&prepared, Some(store))
            .await?
        {
            return Err(flow_like_types::anyhow!(
                "Board draft changed while publishing version {}.{}.{}",
                version.0,
                version.1,
                version.2
            ));
        }
        Ok(version)
    }

    pub async fn create_version(
        &mut self,
        version_type: VersionType,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<(u32, u32, u32)> {
        let store = self.get_store(store).await?;
        let existing = self.get_versions(Some(store.clone())).await?;
        let mut published = self.version;
        if existing.contains(&published) {
            if !self
                .snapshot_matches_current(published, Some(store.clone()))
                .await?
            {
                published = self
                    .snapshot_at_fresh_patch_version(Some(store.clone()))
                    .await?;
            }
        } else {
            self.snapshot_at_version(published, Some(store.clone()))
                .await?;
        }

        let bump =
            |version: (u32, u32, u32)| -> flow_like_types::Result<_> {
                Ok(match &version_type {
                    VersionType::Major => (
                        version.0.checked_add(1).ok_or_else(|| {
                            flow_like_types::anyhow!("Board major version overflow")
                        })?,
                        0,
                        0,
                    ),
                    VersionType::Minor => (
                        version.0,
                        version.1.checked_add(1).ok_or_else(|| {
                            flow_like_types::anyhow!("Board minor version overflow")
                        })?,
                        0,
                    ),
                    VersionType::Patch => (
                        version.0,
                        version.1,
                        version.2.checked_add(1).ok_or_else(|| {
                            flow_like_types::anyhow!("Board patch version overflow")
                        })?,
                    ),
                })
            };
        let existing = self.get_versions(Some(store.clone())).await?;
        let mut new_version = bump(published)?;
        while existing.contains(&new_version) {
            new_version = bump(new_version)?;
        }

        self.version = new_version;
        self.mark_changed();
        self.save(Some(store)).await?;
        Ok(new_version)
    }

    pub async fn get_versions(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Vec<(u32, u32, u32)>> {
        let versions_dir = self
            .board_dir
            .clone()
            .child("versions")
            .child(self.id.clone());

        let store = match store {
            Some(store) => store,
            None => self
                .app_state
                .as_ref()
                .expect("app_state should always be set")
                .config
                .read()
                .await
                .stores
                .app_meta_store
                .clone()
                .ok_or(flow_like_types::anyhow!("Project store not found"))?
                .as_generic(),
        };

        let mut versions = store.list(Some(&versions_dir));
        let mut version_list = Vec::new();

        while let Some(meta) = versions.next().await {
            let meta = meta?;
            let file_name = match meta.location.filename() {
                Some(name) => name,
                None => continue,
            };
            if !file_name.ends_with(".board") {
                continue;
            }
            let version = file_name.strip_suffix(".board").unwrap_or(file_name);
            if version == "latest" {
                continue;
            }
            let version = version.strip_prefix("v").unwrap_or(version);
            let version = version.split("_").collect::<Vec<&str>>();

            if version.len() < 3 {
                continue;
            }

            let version = (
                version[0].parse::<u32>().unwrap_or(0),
                version[1].parse::<u32>().unwrap_or(0),
                version[2].parse::<u32>().unwrap_or(0),
            );

            version_list.push(version);
        }
        Ok(version_list)
    }

    /// Resolve the on-disk storage path for a board's compressed proto.
    /// `board_dir` is the per-app root (e.g. `apps/{app_id}`). When `version`
    /// is `None` this returns the floating "latest" path; otherwise the
    /// immutable per-version path.
    pub fn proto_path(board_dir: &Path, id: &str, version: Option<(u32, u32, u32)>) -> Path {
        match version {
            Some((maj, min, pat)) => board_dir
                .child("versions")
                .child(id.to_string())
                .child(format!("{}_{}_{}.board", maj, min, pat)),
            None => board_dir.child(format!("{}.board", id)),
        }
    }

    /// Fetch and decompress the board's proto representation. This step is
    /// independent of any `FlowLikeState` and produces no per-request data,
    /// so the result is safe to share across users (cf. executor's proto cache).
    #[instrument(name = "Board::load_proto", skip(store), level = "debug")]
    pub async fn load_proto(
        store: Arc<dyn ObjectStore>,
        board_dir: &Path,
        id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<flow_like_types::proto::Board> {
        let path = Self::proto_path(board_dir, id, version);
        from_compressed(store, path).await
    }

    /// Like [`Self::load_proto`] but additionally returns the storage
    /// [`ObjectMeta`] (e_tag / last_modified). Use this when caching the proto
    /// — a subsequent HEAD against the same path lets you detect mutations
    /// without re-downloading the body.
    #[instrument(name = "Board::load_proto_with_meta", skip(store), level = "debug")]
    pub async fn load_proto_with_meta(
        store: Arc<dyn ObjectStore>,
        board_dir: &Path,
        id: &str,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<(
        flow_like_types::proto::Board,
        flow_like_storage::object_store::ObjectMeta,
    )> {
        let path = Self::proto_path(board_dir, id, version);
        from_compressed_with_meta(store, path).await
    }

    /// Build a fully-initialised `Board` from a previously-loaded proto.
    /// Runs `node_updates` so dynamic nodes/schema migrations apply against
    /// the caller's registry — this is per-request and must not be cached
    /// across users.
    pub async fn from_loaded_proto(
        proto: flow_like_types::proto::Board,
        board_dir: Path,
        app_state: Arc<FlowLikeState>,
    ) -> Self {
        let mut board = Board::from_proto(proto);
        board.board_dir = board_dir;
        board.app_state = Some(app_state.clone());
        board.logic_nodes = HashMap::new();

        board.node_updates(app_state).await;
        board.cleanup();

        board
    }

    #[instrument(name = "Board::load", skip(app_state, path), level = "debug")]
    pub async fn load(
        path: Path,
        id: &str,
        app_state: Arc<FlowLikeState>,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Self> {
        let store = app_state
            .config
            .read()
            .await
            .stores
            .app_meta_store
            .clone()
            .ok_or_else(|| {
                tracing::error!("Project store not found while loading board: id={}", id);
                flow_like_types::anyhow!("Project store not found")
            })?
            .as_generic();

        let proto = Self::load_proto(store, &path, id, version).await?;
        Ok(Self::from_loaded_proto(proto, path, app_state).await)
    }

    pub async fn save(&self, store: Option<Arc<dyn ObjectStore>>) -> flow_like_types::Result<()> {
        let to = self.board_dir.child(format!("{}.board", self.id));
        let store = match store {
            Some(store) => store,
            None => self
                .app_state
                .as_ref()
                .expect("app_state should always be set")
                .config
                .read()
                .await
                .stores
                .app_meta_store
                .clone()
                .ok_or(flow_like_types::anyhow!("Project store not found"))?
                .as_generic(),
        };

        let board = self.to_proto();
        compress_to_file(store, to, &board).await?;
        Ok(())
    }

    // PAGE FUNCTIONS

    fn pages_dir(&self) -> Path {
        self.board_dir.child(format!("_{}", self.id))
    }

    fn page_path(&self, page_id: &str) -> Path {
        self.pages_dir().child(format!("{}.page", page_id))
    }

    fn versioned_pages_dir(&self, version: (u32, u32, u32)) -> Path {
        self.board_dir
            .child("versions")
            .child(self.id.clone())
            .child(format!("{}_{}_{}", version.0, version.1, version.2))
    }

    fn versioned_page_path(&self, version: (u32, u32, u32), page_id: &str) -> Path {
        self.versioned_pages_dir(version)
            .child(format!("{}.page", page_id))
    }

    fn template_pages_dir(&self, template_id: &str) -> Path {
        self.board_dir.child(format!("_template_{}", template_id))
    }

    fn template_page_path(&self, template_id: &str, page_id: &str) -> Path {
        self.template_pages_dir(template_id)
            .child(format!("{}.page", page_id))
    }

    fn versioned_template_pages_dir(&self, template_id: &str, version: (u32, u32, u32)) -> Path {
        self.board_dir
            .child("templates")
            .child("versions")
            .child(template_id)
            .child(format!("{}_{}_{}", version.0, version.1, version.2))
    }

    fn versioned_template_page_path(
        &self,
        template_id: &str,
        version: (u32, u32, u32),
        page_id: &str,
    ) -> Path {
        self.versioned_template_pages_dir(template_id, version)
            .child(format!("{}.page", page_id))
    }

    async fn get_store(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Arc<dyn ObjectStore>> {
        match store {
            Some(s) => Ok(s),
            None => self
                .app_state
                .as_ref()
                .ok_or_else(|| flow_like_types::anyhow!("app_state not set"))?
                .config
                .read()
                .await
                .stores
                .app_meta_store
                .clone()
                .ok_or_else(|| flow_like_types::anyhow!("Project store not found"))
                .map(|s| s.as_generic()),
        }
    }

    pub fn get_page_ids(&self) -> &[String] {
        &self.page_ids
    }

    pub fn page_count(&self) -> usize {
        self.page_ids.len()
    }

    pub async fn load_page(
        &self,
        page_id: &str,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Page> {
        let store = self.get_store(store).await?;
        self.load_page_with_legacy_fallback(&store, page_id).await
    }

    pub async fn load_all_pages(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Vec<Page>> {
        let store = self.get_store(store).await?;
        let mut pages = Vec::with_capacity(self.page_ids.len());
        for page_id in &self.page_ids {
            pages.push(self.load_page_with_legacy_fallback(&store, page_id).await?);
        }
        Ok(pages)
    }

    /// Load a page from the canonical board-scoped binary-proto path,
    /// falling back to the legacy app-level JSON path written by the
    /// removed `App::save_page`. On a successful fallback the page is
    /// migrated in-place: the proto is written to the canonical path
    /// and the legacy file is removed, so subsequent reads short-circuit
    /// to the canonical lookup. Migration writes are best-effort —
    /// failures are logged and the loaded page is still returned, so a
    /// transient storage hiccup never turns a successful read into a
    /// hard error.
    async fn load_page_with_legacy_fallback(
        &self,
        store: &Arc<dyn ObjectStore>,
        page_id: &str,
    ) -> flow_like_types::Result<Page> {
        let canonical = self.page_path(page_id);
        match from_compressed::<proto::Page>(store.clone(), canonical.clone()).await {
            Ok(p) => Ok(p.into()),
            Err(canonical_err) => {
                let legacy = self.board_dir.child(format!("{}.page", page_id));
                match from_compressed_json::<Page>(store.clone(), legacy.clone()).await {
                    Ok(page) => {
                        let proto: proto::Page = page.clone().into();
                        if let Err(e) = compress_to_file(store.clone(), canonical, &proto).await {
                            tracing::warn!(
                                "page {} legacy→canonical migration write failed: {e}",
                                page_id
                            );
                        } else if let Err(e) = store.delete(&legacy).await {
                            tracing::warn!(
                                "page {} legacy→canonical migration: canonical written but legacy delete failed: {e}",
                                page_id
                            );
                        }
                        Ok(page)
                    }
                    Err(legacy_err) => Err(flow_like_types::anyhow!(
                        "page {} not found at canonical path ({}) or legacy app-level path ({})",
                        page_id,
                        canonical_err,
                        legacy_err
                    )),
                }
            }
        }
    }

    pub async fn load_versioned_page(
        &self,
        page_id: &str,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Page> {
        let store = self.get_store(store).await?;
        let path = self.versioned_page_path(version, page_id);
        let page_proto: proto::Page = from_compressed(store, path).await?;
        Ok(page_proto.into())
    }

    pub async fn save_page(
        &mut self,
        page: &Page,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let store = self.get_store(store).await?;
        let path = self.page_path(&page.id);
        let page_proto: proto::Page = page.clone().into();
        compress_to_file(store, path, &page_proto).await?;

        if !self.page_ids.contains(&page.id) {
            self.page_ids.push(page.id.clone());
        }
        self.mark_changed();
        Ok(())
    }

    pub async fn delete_page(
        &mut self,
        page_id: &str,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let store = self.get_store(store).await?;
        // Best-effort delete on the canonical path; missing files
        // (e.g. data only ever written via the legacy `App::save_page`)
        // shouldn't fail the call.
        let _ = store.delete(&self.page_path(page_id)).await;
        // Also evict any legacy app-level copy so a subsequent load
        // can't resurrect the page through the fallback reader.
        let legacy = self.board_dir.child(format!("{}.page", page_id));
        let _ = store.delete(&legacy).await;
        self.page_ids.retain(|id| id != page_id);
        self.mark_changed();
        Ok(())
    }

    pub fn get_required_element_ids(&self) -> std::collections::HashSet<String> {
        let mut required_ids = std::collections::HashSet::new();
        for node in self.nodes.values() {
            Self::extract_element_refs_from_node(node, &mut required_ids);
        }
        for layer in self.layers.values() {
            for node in layer.nodes.values() {
                Self::extract_element_refs_from_node(node, &mut required_ids);
            }
        }
        required_ids
    }

    fn extract_element_refs_from_node(
        node: &Node,
        required_ids: &mut std::collections::HashSet<String>,
    ) {
        for pin in node.pins.values() {
            if pin.name == "element_ref"
                && let Some(default_value) = &pin.default_value
                && let Ok(value) =
                    flow_like_types::json::from_slice::<flow_like_types::Value>(default_value)
                && let Some(id) = value.as_str()
                && !id.is_empty()
            {
                required_ids.insert(id.to_string());
            }
        }
    }

    pub async fn get_execution_elements(
        &self,
        page_id: &str,
        wildcard: bool,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<std::collections::HashMap<String, flow_like_types::Value>> {
        let mut elements = std::collections::HashMap::new();

        let page = match self.load_page(page_id, store).await {
            Ok(p) => p,
            Err(_) => return Ok(elements),
        };

        if wildcard {
            for component in &page.components {
                let full_id = format!("{}/{}", page_id, component.id);
                if let Ok(value) = flow_like_types::json::to_value(component) {
                    elements.insert(full_id, value);
                }
            }
        } else {
            let required_ids = self.get_required_element_ids();
            let component_ids: std::collections::HashSet<String> = required_ids
                .iter()
                .filter_map(|id| {
                    if id.starts_with(&format!("{}/", page_id)) {
                        Some(
                            id.strip_prefix(&format!("{}/", page_id))
                                .unwrap()
                                .to_string(),
                        )
                    } else if !id.contains('/') {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect();

            for component in &page.components {
                if component_ids.contains(&component.id) {
                    let full_id = format!("{}/{}", page_id, component.id);
                    if let Ok(value) = flow_like_types::json::to_value(component) {
                        elements.insert(full_id, value);
                    }
                }
            }
        }

        Ok(elements)
    }

    // TEMPLATE FUNCTIONS

    pub async fn save_as_template(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let to = self.board_dir.child(format!("{}.template", self.id));
        let store = self.get_store(store).await?;

        let mut template = self.clone();
        template.clear_internal_refs();
        let board = template.to_proto();
        compress_to_file(store.clone(), to, &board).await?;

        for page_id in &self.page_ids {
            let src_path = self.page_path(page_id);
            let dst_path = self.template_page_path(&self.id, page_id);
            let page_proto: proto::Page = from_compressed(store.clone(), src_path).await?;
            compress_to_file(store.clone(), dst_path, &page_proto).await?;
        }

        Ok(())
    }

    pub async fn overwrite_template_version(
        &mut self,
        version: (u32, u32, u32),
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let store = self.get_store(store).await?;

        let to = self
            .board_dir
            .child("templates")
            .child("versions")
            .child(self.id.clone())
            .child(format!(
                "{}_{}_{}.template",
                version.0, version.1, version.2
            ));

        let mut template = self.clone();
        template.clear_internal_refs();
        let board = template.to_proto();
        compress_to_file(store.clone(), to, &board).await?;

        for page_id in &self.page_ids {
            let src_path = self.page_path(page_id);
            let dst_path = self.versioned_template_page_path(&self.id, version, page_id);
            let page_proto: proto::Page = from_compressed(store.clone(), src_path).await?;
            compress_to_file(store.clone(), dst_path, &page_proto).await?;
        }

        Ok(())
    }

    pub async fn create_template(
        &mut self,
        template_id: String,
        version_type: VersionType,
        old_template: Option<Board>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<(u32, u32, u32)> {
        let store = self.get_store(store).await?;

        let version = old_template
            .as_ref()
            .map(|t| t.version)
            .unwrap_or((0, 0, 0));

        let mut new_version = (0, 0, 0);

        if let Some(old_template) = &old_template {
            let to = self
                .board_dir
                .child("templates")
                .child("versions")
                .child(self.id.clone())
                .child(format!(
                    "{}_{}_{}.template",
                    version.0, version.1, version.2
                ));
            let mut old_template = old_template.clone();
            old_template.clear_internal_refs();
            compress_to_file(store.clone(), to, &old_template.to_proto()).await?;

            for page_id in &old_template.page_ids {
                let src_path = old_template.template_page_path(&old_template.id, page_id);
                let dst_path = self.versioned_template_page_path(&self.id, version, page_id);
                let page_proto: proto::Page = from_compressed(store.clone(), src_path).await?;
                compress_to_file(store.clone(), dst_path, &page_proto).await?;
            }

            new_version = match version_type {
                VersionType::Major => (version.0 + 1, 0, 0),
                VersionType::Minor => (version.0, version.1 + 1, 0),
                VersionType::Patch => (version.0, version.1, version.2 + 1),
            }
        }

        let mut template = self.clone();
        template.id = template_id;
        template.version = new_version;

        for variable in template.variables.values_mut() {
            if variable.secret {
                variable.default_value = None;
            }
        }

        template.mark_changed();
        template.save_as_template(Some(store)).await?;
        Ok(new_version)
    }

    pub async fn load_template(
        path: Path,
        template_id: &str,
        app_state: Arc<FlowLikeState>,
        version: Option<(u32, u32, u32)>,
    ) -> flow_like_types::Result<Self> {
        let store = app_state
            .config
            .read()
            .await
            .stores
            .app_meta_store
            .clone()
            .ok_or(flow_like_types::anyhow!("Project store not found"))?
            .as_generic();

        let board_dir = path.clone();
        let path = if let Some(version) = version {
            path.child("templates")
                .child("versions")
                .child(template_id)
                .child(format!(
                    "{}_{}_{}.template",
                    version.0, version.1, version.2
                ))
        } else {
            path.child(format!("{}.template", template_id))
        };

        let board: flow_like_types::proto::Board = from_compressed(store, path).await?;
        let mut board = Board::from_proto(board);
        board.board_dir = board_dir;
        board.app_state = Some(app_state.clone());
        board.logic_nodes = HashMap::new();

        // Sync node schemas on load to handle version migrations
        board.node_updates(app_state).await;
        board.cleanup();

        Ok(board)
    }

    pub async fn load_template_page(
        &self,
        template_id: &str,
        page_id: &str,
        version: Option<(u32, u32, u32)>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Page> {
        let store = self.get_store(store).await?;

        let path = if let Some(v) = version {
            self.versioned_template_page_path(template_id, v, page_id)
        } else {
            self.template_page_path(template_id, page_id)
        };

        let page_proto: proto::Page = from_compressed(store, path).await?;
        Ok(page_proto.into())
    }

    pub async fn load_all_template_pages(
        &self,
        template_id: &str,
        version: Option<(u32, u32, u32)>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<HashMap<String, Page>> {
        let store = self.get_store(store).await?;

        let mut pages = HashMap::new();
        for page_id in &self.page_ids {
            let path = if let Some(v) = version {
                self.versioned_template_page_path(template_id, v, page_id)
            } else {
                self.template_page_path(template_id, page_id)
            };
            let page_proto: proto::Page = from_compressed(store.clone(), path).await?;
            pages.insert(page_id.clone(), page_proto.into());
        }
        Ok(pages)
    }

    pub async fn copy_template_pages_to_board(
        &self,
        template_id: &str,
        version: Option<(u32, u32, u32)>,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let store = self.get_store(store).await?;

        for page_id in &self.page_ids {
            let src_path = if let Some(v) = version {
                self.versioned_template_page_path(template_id, v, page_id)
            } else {
                self.template_page_path(template_id, page_id)
            };
            let dst_path = self.page_path(page_id);
            let page_proto: proto::Page = from_compressed(store.clone(), src_path).await?;
            compress_to_file(store.clone(), dst_path, &page_proto).await?;
        }
        Ok(())
    }

    pub async fn get_template_versions(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<Vec<(u32, u32, u32)>> {
        let versions_dir = self
            .board_dir
            .clone()
            .child("templates")
            .child("versions")
            .child(self.id.clone());

        let store = self.get_store(store).await?;

        let mut versions = store.list(Some(&versions_dir));
        let mut version_list = Vec::new();

        while let Some(Ok(meta)) = versions.next().await {
            let file_name = match meta.location.filename() {
                Some(name) => name,
                None => continue,
            };
            if !file_name.ends_with(".template") {
                continue;
            }
            let version = file_name.strip_suffix(".template").unwrap_or(file_name);
            if version == "latest" {
                continue;
            }
            let version = version.strip_prefix("v").unwrap_or(version);
            let version = version.split("_").collect::<Vec<&str>>();

            if version.len() < 3 {
                continue;
            }

            let version = (
                version[0].parse::<u32>().unwrap_or(0),
                version[1].parse::<u32>().unwrap_or(0),
                version[2].parse::<u32>().unwrap_or(0),
            );

            version_list.push(version);
        }
        Ok(version_list)
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub enum CommentType {
    Text,
    Image,
    Video,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Comment {
    pub id: String,
    pub author: Option<String>,
    pub content: String,
    pub comment_type: CommentType,
    pub timestamp: SystemTime,
    pub coordinates: (f32, f32, f32),
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub layer: Option<String>,
    pub color: Option<String>,
    pub z_index: Option<i32>,
    pub hash: Option<u64>,
    pub is_locked: Option<bool>,
}

impl Comment {
    pub fn hash(&mut self) {
        let mut hasher = HighwayHasher::new(highway::Key([
            0x0123456789abcdfe,
            0xfedcba9876543210,
            0x0011223344556677,
            0x8899aabbccddeeff,
        ]));

        hasher.append(self.id.as_bytes());
        hasher.append(self.content.as_bytes());
        hasher.append(format!("{:?}", self.comment_type).as_bytes());

        if let Some(author) = &self.author {
            hasher.append(author.as_bytes());
        }

        hasher.append(&self.coordinates.0.to_le_bytes());
        hasher.append(&self.coordinates.1.to_le_bytes());
        hasher.append(&self.coordinates.2.to_le_bytes());

        if let Some(width) = self.width {
            hasher.append(&width.to_le_bytes());
        }

        if let Some(height) = self.height {
            hasher.append(&height.to_le_bytes());
        }

        if let Some(layer) = &self.layer {
            hasher.append(layer.as_bytes());
        }

        if let Some(color) = &self.color {
            hasher.append(color.as_bytes());
        }

        if let Some(z_index) = self.z_index {
            hasher.append(&z_index.to_le_bytes());
        }

        if let Some(is_locked) = self.is_locked {
            hasher.append(&[is_locked as u8]);
        }

        self.hash = Some(hasher.finalize64());
    }
}

#[cfg(test)]
mod tests {
    use crate::{state::FlowLikeConfig, utils::http::HTTPClient};
    use flow_like_storage::{
        files::store::FlowLikeStore,
        object_store::{self, path::Path},
    };
    use flow_like_types::{FromProto, ToProto};
    use flow_like_types::{Message, tokio};
    use std::sync::Arc;

    async fn flow_state() -> Arc<crate::state::FlowLikeState> {
        let mut config: FlowLikeConfig = FlowLikeConfig::new();
        config.register_app_meta_store(FlowLikeStore::Other(Arc::new(
            object_store::memory::InMemory::new(),
        )));
        let http_client = HTTPClient::new_without_refetch();
        let flow_like_state = crate::state::FlowLikeState::new(config, http_client);
        Arc::new(flow_like_state)
    }

    struct RefreshDefinitionLogic {
        label: &'static str,
    }

    #[flow_like_types::async_trait]
    impl crate::flow::node::NodeLogic for RefreshDefinitionLogic {
        fn get_node(&self) -> crate::flow::node::Node {
            crate::flow::node::Node::new("refresh_definition_test", self.label, "", "test")
        }

        async fn run(
            &self,
            _: &mut crate::flow::execution::context::ExecutionContext,
        ) -> flow_like_types::Result<()> {
            Ok(())
        }

        async fn on_update(&self, node: &mut crate::flow::node::Node, _: &super::Board) {
            node.friendly_name = self.label.to_string();
        }
    }

    #[tokio::test]
    async fn refresh_node_definitions_uses_current_registry_without_marking_board_dirty() {
        use crate::flow::node::NodeLogic;

        let state = flow_state().await;
        let old_logic: Arc<dyn NodeLogic> = Arc::new(RefreshDefinitionLogic { label: "Old logic" });
        let new_logic: Arc<dyn NodeLogic> = Arc::new(RefreshDefinitionLogic { label: "New logic" });

        let mut board = super::Board::new(None, Path::from("boards"), state.clone());
        let node = old_logic.get_node();
        let node_id = node.id.clone();
        board.nodes.insert(node_id.clone(), node);
        board
            .logic_nodes
            .insert("refresh_definition_test".to_string(), old_logic);

        state.node_registry().write().await.push_node(new_logic);
        let updated_at = board.updated_at;
        board.hash = Some(0xdead_beef);

        board.refresh_node_definitions(state).await;

        assert_eq!(board.nodes[&node_id].friendly_name, "New logic");
        assert_eq!(board.updated_at, updated_at);
        assert_eq!(board.hash, Some(0xdead_beef));
    }

    #[tokio::test]
    async fn serialize_board() {
        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let board = super::Board::new(None, base_dir, state);

        let mut buf = Vec::new();
        board.to_proto().encode(&mut buf).unwrap();
        let deser_board =
            super::Board::from_proto(flow_like_types::proto::Board::decode(&buf[..]).unwrap());

        assert_eq!(board.id, deser_board.id);
    }

    #[tokio::test]
    async fn internal_refs_roundtrip_in_proto_but_not_board_json_or_semantic_hash() {
        let state = flow_state().await;
        let mut board = super::Board::new(None, Path::from("boards"), state);
        let original_hash = board.content_hash();
        let key = format!("{}test-receipt", super::INTERNAL_BOARD_REF_PREFIX);
        board
            .insert_internal_ref(key.clone(), "opaque")
            .expect("reserved key");

        assert_eq!(board.content_hash(), original_hash);
        let json = serde_json::to_value(&board).expect("board JSON");
        assert!(json.get("internal_refs").is_none());
        assert!(
            json.get("refs")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|refs| !refs.contains_key(&key))
        );

        let proto = board.to_proto();
        assert_eq!(
            proto.internal_refs.get(&key).map(String::as_str),
            Some("opaque")
        );
        let restored = super::Board::from_proto(proto);
        assert_eq!(restored.internal_ref(&key), Some("opaque"));
        assert!(!restored.refs.contains_key(&key));
    }

    #[tokio::test]
    async fn legacy_prefixed_refs_migrate_to_internal_proto_storage() {
        let state = flow_state().await;
        let board = super::Board::new(None, Path::from("boards"), state);
        let mut proto = board.to_proto();
        let key = format!("{}legacy", super::INTERNAL_BOARD_REF_PREFIX);
        proto.refs.insert(key.clone(), "receipt".to_string());

        let restored = super::Board::from_proto(proto);
        assert_eq!(restored.internal_ref(&key), Some("receipt"));
        assert!(!restored.refs.contains_key(&key));
        let migrated = restored.to_proto();
        assert!(!migrated.refs.contains_key(&key));
        assert_eq!(
            migrated.internal_refs.get(&key).map(String::as_str),
            Some("receipt")
        );
    }

    #[tokio::test]
    async fn governed_action_parameter_schema_fails_closed_when_authored_schema_is_malformed() {
        use crate::flow::{node::Node, variable::VariableType};

        let state = flow_state().await;
        let mut board = super::Board::new(None, Path::from("boards"), state);
        assert!(board.action_parameter_schema("missing-start").is_err());
        let mut start = Node::new("start", "Start", "", "events");
        let start_id = start.id.clone();
        let parameters_pin = start
            .add_output_pin("parameters", "Parameters", "", VariableType::Struct)
            .id
            .clone();
        start.pins.get_mut(&parameters_pin).unwrap().schema = Some("not-json".to_string());
        board.nodes.insert(start_id.clone(), start);

        assert!(board.action_parameter_schema(&start_id).is_err());
        board
            .nodes
            .get_mut(&start_id)
            .unwrap()
            .pins
            .get_mut(&parameters_pin)
            .unwrap()
            .schema = Some("[]".to_string());
        assert!(board.action_parameter_schema(&start_id).is_err());
    }

    #[tokio::test]
    async fn immutable_snapshot_rejects_overwrite_and_publishes_fresh_patch() {
        use crate::a2ui::widget::Page;

        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut board = super::Board::new(None, base_dir.clone(), state.clone());
        let original_version = board.version;
        let original_name = board.name.clone();
        let mut page = Page::new("page-1", "Original page", "/");
        board.save_page(&page, None).await.unwrap();

        board
            .snapshot_at_version(original_version, None)
            .await
            .unwrap();
        board
            .snapshot_at_version(original_version, None)
            .await
            .expect("an identical retry must be idempotent");
        assert!(
            board
                .snapshot_matches_current(original_version, None)
                .await
                .unwrap()
        );
        page.name = "Edited page".to_string();
        board.save_page(&page, None).await.unwrap();
        assert!(
            !board
                .snapshot_matches_current(original_version, None)
                .await
                .unwrap(),
            "page-only edits must make the snapshot stale"
        );
        board.name = "Edited draft".to_string();
        board.mark_changed();

        assert!(
            board
                .snapshot_at_version(original_version, None)
                .await
                .is_err(),
            "an existing version must never be overwritten"
        );
        let original = super::Board::load(
            base_dir.clone(),
            &board.id,
            state.clone(),
            Some(original_version),
        )
        .await
        .unwrap();
        assert_eq!(original.name, original_name);
        let original_page = board
            .load_versioned_page("page-1", original_version, None)
            .await
            .unwrap();
        assert_eq!(original_page.name, "Original page");

        let fresh = board.snapshot_at_fresh_patch_version(None).await.unwrap();
        assert_eq!(
            fresh,
            (
                original_version.0,
                original_version.1,
                original_version.2 + 1
            )
        );
        let published = super::Board::load(base_dir, &board.id, state, Some(fresh))
            .await
            .unwrap();
        assert_eq!(published.name, "Edited draft");
        let published_page = board
            .load_versioned_page("page-1", fresh, None)
            .await
            .unwrap();
        assert_eq!(published_page.name, "Edited page");
    }

    #[tokio::test]
    async fn prepared_snapshot_advances_only_after_commit_and_never_overwrites_newer_draft() {
        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut board = super::Board::new(None, base_dir.clone(), state.clone());
        let original_version = board.version;
        board.save(None).await.unwrap();
        board
            .snapshot_at_version(original_version, None)
            .await
            .unwrap();

        board.name = "Prepared action implementation".to_string();
        board.mark_changed();
        board.save(None).await.unwrap();
        let prepared = board
            .prepare_snapshot_at_fresh_patch_version(None)
            .await
            .unwrap();
        assert_eq!(board.version, original_version);
        assert_eq!(prepared.version(), (0, 0, 2));

        let floating = super::Board::load(base_dir.clone(), &board.id, state.clone(), None)
            .await
            .unwrap();
        assert_eq!(floating.version, original_version);
        assert_eq!(floating.name, "Prepared action implementation");

        assert!(
            board
                .commit_prepared_snapshot(&prepared, None)
                .await
                .unwrap()
        );
        assert_eq!(board.version, prepared.version());
        let committed = super::Board::load(base_dir.clone(), &board.id, state.clone(), None)
            .await
            .unwrap();
        assert_eq!(committed.version, prepared.version());

        committed
            .snapshot_at_version(committed.version, None)
            .await
            .unwrap();
        let mut newer_draft = committed;
        newer_draft.name = "Concurrent edit".to_string();
        newer_draft.mark_changed();
        newer_draft.save(None).await.unwrap();
        let second_prepared = newer_draft
            .prepare_snapshot_at_fresh_patch_version(None)
            .await
            .unwrap();
        newer_draft.name = "Edit after prepare".to_string();
        newer_draft.mark_changed();
        newer_draft.save(None).await.unwrap();
        assert!(
            !newer_draft
                .commit_prepared_snapshot(&second_prepared, None)
                .await
                .unwrap(),
            "a post-prepare edit must keep the floating draft at its current version"
        );
        assert_eq!(newer_draft.version, prepared.version());
    }

    #[tokio::test]
    async fn snapshot_publication_rejects_a_stale_cached_floating_board() {
        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut persisted = super::Board::new(None, base_dir.clone(), state.clone());
        persisted.name = "Original draft".to_string();
        persisted.mark_changed();
        persisted.save(None).await.unwrap();

        // Simulate a second API process retaining an older in-memory board
        // after another process has committed a newer floating draft at the
        // same semantic version.
        let stale = super::Board::load(base_dir.clone(), &persisted.id, state.clone(), None)
            .await
            .unwrap();
        persisted.name = "Newer authoritative draft".to_string();
        persisted.mark_changed();
        persisted.save(None).await.unwrap();

        let candidate = (stale.version.0, stale.version.1, stale.version.2 + 1);
        let error = stale
            .snapshot_at_version(candidate, None)
            .await
            .expect_err("stale cached content must never become a publishable snapshot");
        assert!(error.to_string().contains("Board draft changed"));
        assert!(
            !stale
                .snapshot_matches_persisted_draft(candidate, None)
                .await
                .unwrap(),
            "the immutable orphan must not be mistaken for the authoritative draft"
        );
    }

    #[tokio::test]
    async fn interrupted_page_snapshot_resumes_or_skips_conflicting_patch() {
        use crate::{a2ui::widget::Page, utils::compression::compress_to_file_create};

        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut board = super::Board::new(None, base_dir, state);
        let page = Page::new("page-1", "Current page", "/");
        board.save_page(&page, None).await.unwrap();
        board.save(None).await.unwrap();
        let store = board.get_store(None).await.unwrap();
        let first_candidate = (board.version.0, board.version.1, board.version.2 + 1);

        // Simulate a process that copied a page and stopped before writing the
        // board existence marker. The next attempt must finish that version.
        let page_proto: flow_like_types::proto::Page = page.clone().into();
        compress_to_file_create(
            store.clone(),
            board.versioned_page_path(first_candidate, "page-1"),
            &page_proto,
        )
        .await
        .unwrap();
        let resumed = board
            .prepare_snapshot_at_fresh_patch_version(Some(store.clone()))
            .await
            .unwrap();
        assert_eq!(resumed.version(), first_candidate);

        board
            .commit_prepared_snapshot(&resumed, Some(store.clone()))
            .await
            .unwrap();
        board.name = "Another draft".to_string();
        board.mark_changed();
        board.save(Some(store.clone())).await.unwrap();
        let conflicting_candidate = (board.version.0, board.version.1, board.version.2 + 1);
        let conflicting_page = Page::new("page-1", "Other publisher's page", "/");
        let conflicting_page_proto: flow_like_types::proto::Page = conflicting_page.into();
        compress_to_file_create(
            store.clone(),
            board.versioned_page_path(conflicting_candidate, "page-1"),
            &conflicting_page_proto,
        )
        .await
        .unwrap();

        let prepared = board
            .prepare_snapshot_at_fresh_patch_version(Some(store))
            .await
            .unwrap();
        assert_eq!(
            prepared.version(),
            (
                conflicting_candidate.0,
                conflicting_candidate.1,
                conflicting_candidate.2 + 1
            ),
            "a conflicting orphan page must not permanently poison publication"
        );
    }

    #[tokio::test]
    async fn undo_against_diverged_board_errors_and_restores_undone_tail() {
        use crate::flow::{
            board::{
                Comment, CommentType,
                commands::{
                    GenericCommand, comments::upsert_comment::UpsertCommentCommand,
                    pins::connect_pins::ConnectPinsCommand,
                },
            },
            node::Node,
            variable::VariableType,
        };
        use std::time::SystemTime;

        let state = flow_state().await;
        let base_dir = Path::from("boards");
        let mut board = super::Board::new(None, base_dir, state.clone());

        let mut from_node = Node::new("test_from", "From", "", "test");
        let from_pin = from_node
            .add_output_pin("exec_out", "Out", "", VariableType::Execution)
            .id
            .clone();
        let mut to_node = Node::new("test_to", "To", "", "test");
        let to_pin = to_node
            .add_input_pin("exec_in", "In", "", VariableType::Execution)
            .id
            .clone();
        let from_id = from_node.id.clone();
        let to_id = to_node.id.clone();
        board.nodes.insert(from_id.clone(), from_node);
        board.nodes.insert(to_id.clone(), to_node);

        let comment = Comment {
            id: "comment-1".to_string(),
            author: None,
            content: "recorded after the connect".to_string(),
            comment_type: CommentType::Text,
            timestamp: SystemTime::now(),
            coordinates: (0.0, 0.0, 0.0),
            width: None,
            height: None,
            layer: None,
            color: None,
            z_index: None,
            hash: None,
            is_locked: None,
        };

        let commands = vec![
            GenericCommand::ConnectPin(ConnectPinsCommand::new(
                from_id.clone(),
                to_id.clone(),
                from_pin.clone(),
                to_pin.clone(),
            )),
            GenericCommand::UpsertComment(UpsertCommentCommand::new(comment)),
        ];

        let executed = board
            .execute_commands(commands, state.clone())
            .await
            .unwrap();
        assert!(board.comments.contains_key("comment-1"));

        // Divergence: the board was rewritten underneath the recorded history —
        // the connection's source node no longer exists.
        board.nodes.remove(&from_id);

        let result = board.undo(executed, state.clone()).await;

        assert!(result.is_err(), "undo against a diverged board must fail");
        assert!(
            board.comments.contains_key("comment-1"),
            "commands undone before the failure must be re-applied so the board is not left partially rolled back"
        );
    }
}
