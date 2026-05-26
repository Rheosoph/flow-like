use super::{
    execution::LogLevel,
    node::{Node, NodeLogic},
    pin::Pin,
    variable::Variable,
};
use crate::{
    a2ui::widget::Page,
    app::App,
    state::FlowLikeState,
    utils::compression::{
        compress_to_file, from_compressed, from_compressed_json, from_compressed_with_meta,
    },
};
use commands::GenericCommand;
use flow_like_storage::object_store::{ObjectStore, path::Path};
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

#[derive(Debug, Clone)]
pub enum BoardParent {
    App(Weak<Mutex<App>>),
}

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub enum ExecutionStage {
    Dev,
    Int,
    QA,
    PreProd,
    Prod,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Default, Debug)]
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
            parent: None,
            board_dir,
            logic_nodes: HashMap::new(),
            app_state: Some(app_state.clone()),
        };
        board.hash();
        board
    }

    pub fn mark_changed(&mut self) {
        self.updated_at = SystemTime::now();
        self.hash();
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

        let mut refs = self.refs.iter().collect::<Vec<_>>();
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
        let cmd_json = serde_json::to_string(&command).unwrap_or_default();
        println!("[Board] execute_command: {}", cmd_json);
        command.validate(self, state.clone()).await?;
        if let Err(e) = command.execute(self, state.clone()).await {
            let mut recovery_errors = Vec::new();
            if let Err(rollback_error) = command.undo(self, state.clone()).await {
                recovery_errors.push(format!(
                    "failed to rollback current command after execute error: {rollback_error}"
                ));
            }
            println!("[Board] execute_command ❌ ERROR: {:?}", e);
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
        println!("[Board] execute_command ✓ Success");
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

    pub async fn create_version(
        &mut self,
        version_type: VersionType,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<(u32, u32, u32)> {
        let version = self.version;
        let store = self.get_store(store).await?;

        let board_version_path = self
            .board_dir
            .child("versions")
            .child(self.id.clone())
            .child(format!("{}_{}_{}.board", version.0, version.1, version.2));

        let board = self.to_proto();
        compress_to_file(store.clone(), board_version_path, &board).await?;

        for page_id in &self.page_ids {
            let src_path = self.page_path(page_id);
            let dst_path = self.versioned_page_path(version, page_id);
            let page_proto: proto::Page = from_compressed(store.clone(), src_path).await?;
            compress_to_file(store.clone(), dst_path, &page_proto).await?;
        }

        let new_version = match version_type {
            VersionType::Major => (version.0 + 1, 0, 0),
            VersionType::Minor => (version.0, version.1 + 1, 0),
            VersionType::Patch => (version.0, version.1, version.2 + 1),
        };

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

        while let Some(Ok(meta)) = versions.next().await {
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

    #[instrument(name = "Board::load", skip(app_state), level = "debug")]
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

    /// PAGE FUNCTIONS

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

    /// TEMPLATE FUNCTIONS

    pub async fn save_as_template(
        &self,
        store: Option<Arc<dyn ObjectStore>>,
    ) -> flow_like_types::Result<()> {
        let to = self.board_dir.child(format!("{}.template", self.id));
        let store = self.get_store(store).await?;

        let board = self.to_proto();
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

        let board = self.to_proto();
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
}
