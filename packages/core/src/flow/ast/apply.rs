use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::SystemTime,
};

use flow_like_ast::to_camel_case;
use flow_like_types::create_id;
use serde::Serialize;

use crate::{
    flow::{
        board::{
            Board, Comment, CommentType, Layer, LayerType,
            commands::{
                GenericCommand,
                comments::{
                    remove_comment::RemoveCommentCommand, upsert_comment::UpsertCommentCommand,
                },
                layer::{remove_layer::RemoveLayerCommand, upsert_layer::UpsertLayerCommand},
                nodes::{
                    add_node::AddNodeCommand, move_node::MoveNodeCommand,
                    remove_node::RemoveNodeCommand, update_node::UpdateNodeCommand,
                },
                pins::{connect_pins::ConnectPinsCommand, disconnect_pins::DisconnectPinsCommand},
                variables::{
                    remove_variable::RemoveVariableCommand, upsert_variable::UpsertVariableCommand,
                },
            },
        },
        copilot::{BoardCommand, NodeMetadata, NodePosition, PlaceholderPinDef, node_to_metadata},
        node::{Node, NodeLogic},
        pin::{Pin, PinType, ValueType},
        variable::{Variable, VariableType},
    },
    state::FlowLikeState,
};

const DEFAULT_OUTPUT_PIN_ALIASES: &[&str] = &["result", "value", "output", "out"];

/// Result returned by the server-side FlowScript apply path.
///
/// `commands` is the executed generic command batch and is the value callers should record for
/// undo/redo. `board_commands` is the reconciled higher-level command plan for review/debug UI.
#[derive(Clone, Serialize)]
pub struct ApplyFlowScriptResult {
    pub commands: Vec<GenericCommand>,
    pub board_commands: Vec<BoardCommand>,
    pub diagnostics: Vec<String>,
}

pub fn destructive_flowscript_command_summaries(commands: &[BoardCommand]) -> Vec<String> {
    commands
        .iter()
        .filter_map(|command| match command {
            BoardCommand::RemoveNode { node_id, .. } => Some(format!("node `{node_id}`")),
            BoardCommand::RemoveVariable { variable_id, .. } => {
                Some(format!("variable `{variable_id}`"))
            }
            BoardCommand::RemoveLayer { layer_id, .. } => Some(format!("layer `{layer_id}`")),
            BoardCommand::RemoveComment { comment_id, .. } => {
                Some(format!("comment `{comment_id}`"))
            }
            _ => None,
        })
        .collect()
}

pub fn blocked_destructive_flowscript_message(summaries: &[String]) -> String {
    let preview = summaries
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let more = summaries
        .len()
        .checked_sub(8)
        .filter(|count| *count > 0)
        .map(|count| format!(" and {count} more"))
        .unwrap_or_default();

    format!(
        "FlowScript edit would delete {} existing board item(s): {preview}{more}. Deletions are blocked by default so incomplete model edits cannot remove existing work. Re-submit the full current FlowScript with every kept `//@n:<id>` anchor preserved, or set `allow_deletions` only for an explicit delete request.",
        summaries.len()
    )
}

pub async fn apply_flowscript_to_board(
    board: &mut Board,
    flowscript: &str,
    catalog_nodes: &[Node],
    state: Arc<FlowLikeState>,
    current_layer: Option<String>,
    allow_deletions: bool,
) -> flow_like_types::Result<ApplyFlowScriptResult> {
    let catalog_metadata = catalog_nodes
        .iter()
        .map(node_to_metadata)
        .collect::<Vec<_>>();

    // Resolve dynamic (on_update-generated) pins during reconcile by running each node's on_update on
    // an in-memory scratch node seeded with the call's literal args — no board mutation. Limited to
    // audited pure nodes whose on_update only reads their own pins (no network / cross-node reads).
    const ENRICH_ALLOWLIST: &[&str] = &[
        "string_format",
        "string_render_template",
        "a2ui_push_csv_to_chart",
    ];
    let logic_by_type: HashMap<String, Arc<dyn NodeLogic>> = {
        let registry = state.node_registry.read().await.node_registry.clone();
        catalog_nodes
            .iter()
            .filter(|node| ENRICH_ALLOWLIST.contains(&node.name.as_str()))
            .filter_map(|node| {
                registry
                    .instantiate(node)
                    .ok()
                    .map(|logic| (node.name.clone(), logic))
            })
            .collect()
    };
    let enricher: Option<super::MetadataEnricher> = if logic_by_type.is_empty() {
        None
    } else {
        Some(Box::new(
            move |meta: &NodeMetadata,
                  args: &[(String, flow_like_types::Value)],
                  board: &Board|
                  -> Option<NodeMetadata> {
                let logic = logic_by_type.get(&meta.name)?;
                let mut scratch = logic.get_node();
                let mut seeded = false;
                for (arg_name, value) in args {
                    let pin_id = scratch
                        .pins
                        .iter()
                        .find(|(_, pin)| {
                            pin.pin_type == PinType::Input
                                && (pin.name == *arg_name || to_camel_case(&pin.name) == *arg_name)
                        })
                        .map(|(id, _)| id.clone());
                    if let Some(pin_id) = pin_id
                        && let Some(pin) = scratch.pins.get_mut(&pin_id)
                        && let Ok(bytes) = flow_like_types::json::to_vec(value)
                    {
                        pin.default_value = Some(bytes);
                        seeded = true;
                    }
                }
                if !seeded {
                    return None;
                }
                futures::executor::block_on(logic.on_update(&mut scratch, board));
                Some(node_to_metadata(&scratch))
            },
        ))
    };

    let mut reconcile = match &enricher {
        Some(enricher) => super::reconcile_text_with_catalog_enriched(
            board,
            flowscript,
            &catalog_metadata,
            enricher,
        ),
        None => super::reconcile_text_with_catalog(board, flowscript, &catalog_metadata),
    };

    // Block only when nothing is derivable (parse errors / fully-unresolvable input yield no
    // commands). Non-fatal diagnostics — a skipped argument, a dangling execution warning — must not
    // discard the whole apply; the derivable commands are applied and the diagnostics are surfaced as
    // warnings in the result so partial progress is not lost.
    if reconcile.commands.is_empty() {
        return Ok(ApplyFlowScriptResult {
            commands: Vec::new(),
            board_commands: reconcile.commands,
            diagnostics: reconcile.diagnostics,
        });
    }

    if !allow_deletions {
        let destructive = destructive_flowscript_command_summaries(&reconcile.commands);
        if !destructive.is_empty() {
            let mut diagnostics = std::mem::take(&mut reconcile.diagnostics);
            diagnostics.insert(0, blocked_destructive_flowscript_message(&destructive));
            return Ok(ApplyFlowScriptResult {
                commands: Vec::new(),
                board_commands: reconcile.commands,
                diagnostics,
            });
        }
    }

    let mut planner = FlowScriptApplyPlanner::new(board, catalog_nodes, current_layer);
    let setup_commands = planner.build_setup_commands(board, &reconcile.commands)?;
    let mut applied_commands = Vec::new();

    if !setup_commands.is_empty() {
        match board.execute_commands(setup_commands, state.clone()).await {
            Ok(mut executed) => applied_commands.append(&mut executed),
            Err(error) => return Err(error),
        }
    }

    let remaining_commands = match planner.build_remaining_commands(board, &reconcile.commands) {
        Ok(commands) => commands,
        Err(error) => {
            rollback_applied(board, &applied_commands, state.clone(), error).await?;
            return Ok(ApplyFlowScriptResult {
                commands: Vec::new(),
                board_commands: reconcile.commands,
                diagnostics: vec!["FlowScript apply failed and was rolled back".to_string()],
            });
        }
    };

    if !remaining_commands.is_empty() {
        match board
            .execute_commands(remaining_commands, state.clone())
            .await
        {
            Ok(mut executed) => applied_commands.append(&mut executed),
            Err(error) => {
                rollback_applied(board, &applied_commands, state.clone(), error).await?;
            }
        }
    }

    Ok(ApplyFlowScriptResult {
        commands: applied_commands,
        board_commands: reconcile.commands,
        diagnostics: reconcile.diagnostics,
    })
}

async fn rollback_applied(
    board: &mut Board,
    applied_commands: &[GenericCommand],
    state: Arc<FlowLikeState>,
    error: flow_like_types::Error,
) -> flow_like_types::Result<()> {
    if applied_commands.is_empty() {
        return Err(error);
    }

    let primary_error = error.to_string();
    if let Err(rollback_error) = board.undo(applied_commands.to_vec(), state).await {
        return Err(flow_like_types::anyhow!(
            "FlowScript apply failed: {primary_error}; rollback failed: {rollback_error}"
        ));
    }

    Err(flow_like_types::anyhow!(
        "FlowScript apply failed and was rolled back: {primary_error}"
    ))
}

struct FlowScriptApplyPlanner {
    catalog_nodes: HashMap<String, Node>,
    node_refs: HashMap<String, String>,
    ambiguous_node_refs: HashSet<String>,
    staged_nodes: HashMap<String, Node>,
    staged_layers: HashMap<String, Layer>,
    /// (resolved node id, original pin id) for `UpdateNodePin` literals whose target pin did not
    /// exist during setup because it is created dynamically by the node's `on_update`. These are
    /// applied in the remaining phase, which runs after `on_update`.
    deferred_pin_updates: HashSet<(String, String)>,
    current_layer: Option<String>,
    base_x: f32,
    base_y: f32,
    next_position: usize,
    next_node_index: usize,
}

impl FlowScriptApplyPlanner {
    fn new(board: &Board, catalog_nodes: &[Node], current_layer: Option<String>) -> Self {
        let mut planner = Self {
            catalog_nodes: catalog_nodes
                .iter()
                .map(|node| (node.name.clone(), node.clone()))
                .collect(),
            node_refs: HashMap::new(),
            ambiguous_node_refs: HashSet::new(),
            staged_nodes: HashMap::new(),
            staged_layers: HashMap::new(),
            deferred_pin_updates: HashSet::new(),
            current_layer,
            base_x: 100.0,
            base_y: 100.0,
            next_position: 0,
            next_node_index: 0,
        };

        if let Some(rightmost) = board.nodes.values().max_by(|left, right| {
            let left_x = left.coordinates.map(|c| c.0).unwrap_or(0.0);
            let right_x = right.coordinates.map(|c| c.0).unwrap_or(0.0);
            left_x.total_cmp(&right_x)
        }) {
            planner.base_x = rightmost.coordinates.map(|c| c.0).unwrap_or(0.0) + 300.0;
            planner.base_y = rightmost.coordinates.map(|c| c.1).unwrap_or(100.0);
        }

        for node in board.nodes.values() {
            planner.register_node_aliases(&[Some(node.id.as_str())], &node.id);
        }
        for layer in board.layers.values() {
            planner.register_node_aliases(
                &[Some(layer.id.as_str()), Some(layer.name.as_str())],
                &layer.id,
            );
        }
        for (name, node_id) in &board.refs {
            planner.register_node_aliases(&[Some(name.as_str())], node_id);
        }

        planner
    }

    fn build_setup_commands(
        &mut self,
        board: &Board,
        commands: &[BoardCommand],
    ) -> flow_like_types::Result<Vec<GenericCommand>> {
        let mut generic_commands = Vec::new();

        for command in commands {
            match command {
                BoardCommand::AddNode {
                    node_type,
                    ref_id,
                    position,
                    friendly_name,
                    target_layer,
                    ..
                } => {
                    let Some(catalog_node) = self.catalog_nodes.get(node_type) else {
                        return Err(flow_like_types::anyhow!(
                            "Node type `{node_type}` is not available in the catalog"
                        ));
                    };

                    let mut add_command = AddNodeCommand::new(catalog_node.clone());
                    let coordinates = self.position_or_next(position.as_ref());
                    add_command.node.coordinates = Some(coordinates);
                    if let Some(friendly_name) = friendly_name {
                        add_command.node.friendly_name = friendly_name.clone();
                    }
                    add_command.current_layer =
                        self.target_layer_or_current(board, target_layer.as_deref())?;

                    let node_id = add_command.node.id.clone();
                    self.staged_nodes
                        .insert(node_id.clone(), add_command.node.clone());
                    let index_alias = format!("${}", self.next_node_index);
                    self.next_node_index += 1;
                    self.register_node_aliases(
                        &[
                            ref_id.as_deref(),
                            Some(index_alias.as_str()),
                            Some(node_type.as_str()),
                            Some(node_id.as_str()),
                        ],
                        &node_id,
                    );

                    generic_commands.push(GenericCommand::AddNode(add_command));
                }
                BoardCommand::AddPlaceholder {
                    name,
                    ref_id,
                    position,
                    pins,
                    target_layer,
                    ..
                } => {
                    let layer_id = create_id();
                    let mut layer =
                        Layer::new(layer_id.clone(), name.clone(), LayerType::Collapsed);
                    layer.coordinates = self.position_or_next(position.as_ref());
                    layer.pins = placeholder_pins(pins.as_deref());

                    let mut command = UpsertLayerCommand::new(layer);
                    command.current_layer =
                        self.target_layer_or_current(board, target_layer.as_deref())?;

                    let index_alias = format!("${}", self.next_node_index);
                    self.next_node_index += 1;
                    self.register_node_aliases(
                        &[
                            ref_id.as_deref(),
                            Some(index_alias.as_str()),
                            Some(name.as_str()),
                            Some(layer_id.as_str()),
                        ],
                        &layer_id,
                    );

                    generic_commands.push(GenericCommand::UpsertLayer(command));
                }
                BoardCommand::CreateLayer {
                    name,
                    ref_id,
                    layer_type,
                    node_ids,
                    pins,
                    position,
                    color,
                    target_layer,
                    ..
                } if node_ids.is_empty() || ref_id.is_some() || pins.is_some() => {
                    let layer_id = create_id();
                    let mut layer = Layer::new(
                        layer_id.clone(),
                        name.clone(),
                        layer_type_from_str(layer_type),
                    );
                    layer.coordinates = self.position_or_base(position.as_ref());
                    layer.color = color.clone();
                    layer.pins = layer_pins(pins.as_deref());

                    let mut command = UpsertLayerCommand::new(layer.clone());
                    command.current_layer =
                        self.target_layer_or_current(board, target_layer.as_deref())?;

                    self.staged_layers.insert(layer_id.clone(), layer);
                    let index_alias = format!("${}", self.next_node_index);
                    self.next_node_index += 1;
                    self.register_node_aliases(
                        &[
                            ref_id.as_deref(),
                            Some(index_alias.as_str()),
                            Some(name.as_str()),
                            Some(layer_id.as_str()),
                        ],
                        &layer_id,
                    );

                    generic_commands.push(GenericCommand::UpsertLayer(command));
                }
                BoardCommand::CreateVariable {
                    variable_id,
                    name,
                    data_type,
                    value_type,
                    default_value,
                    description,
                    category,
                    schema,
                    exposed,
                    secret,
                    editable,
                    runtime_configured,
                    target_layer,
                    ..
                } => {
                    let mut variable = Variable::new(
                        name,
                        variable_type_from_str(data_type),
                        value_type_from_str(value_type),
                    );
                    variable.id = variable_id.clone().unwrap_or_else(create_id);
                    variable.description = description.clone();
                    variable.category = category.clone();
                    variable.schema = schema.clone();
                    variable.exposed = exposed.unwrap_or(false);
                    variable.secret = secret.unwrap_or(false);
                    variable.editable = editable.unwrap_or(true);
                    variable.runtime_configured = runtime_configured.unwrap_or(false);
                    if let Some(default_value) = default_value {
                        variable.default_value =
                            Some(flow_like_types::json::to_vec(default_value)?);
                    }

                    let mut command = UpsertVariableCommand::new(variable);
                    command.layer_id =
                        self.resolve_optional_layer(board, target_layer.as_deref())?;
                    generic_commands.push(GenericCommand::UpsertVariable(command));
                }
                BoardCommand::UpdateNodePin {
                    node_id,
                    pin_id,
                    value,
                    ..
                } => {
                    let node_id = self.resolve_node_id(board, node_id)?;
                    let mut node = self.resolve_node(board, &node_id)?.clone();
                    // The pin may not exist yet if it is created dynamically by the node's on_update
                    // (which runs after this setup batch executes). Defer such literals to the
                    // remaining phase instead of failing the whole apply.
                    let Some(resolved_pin) = resolve_pin_id_in_node(&node, pin_id, Some(PinType::Input))
                        .ok()
                        .filter(|resolved| node.pins.contains_key(resolved))
                    else {
                        self.deferred_pin_updates
                            .insert((node_id.clone(), pin_id.clone()));
                        continue;
                    };
                    let Some(pin) = node.pins.get_mut(&resolved_pin) else {
                        self.deferred_pin_updates
                            .insert((node_id.clone(), pin_id.clone()));
                        continue;
                    };
                    pin.default_value = Some(flow_like_types::json::to_vec(value)?);
                    self.staged_nodes.insert(node_id.clone(), node.clone());
                    generic_commands.push(GenericCommand::UpdateNode(UpdateNodeCommand::new(node)));
                }
                _ => {}
            }
        }

        Ok(generic_commands)
    }

    fn build_remaining_commands(
        &mut self,
        board: &Board,
        commands: &[BoardCommand],
    ) -> flow_like_types::Result<Vec<GenericCommand>> {
        self.staged_nodes.clear();
        let mut generic_commands = Vec::new();

        for command in commands {
            match command {
                BoardCommand::AddNode { .. }
                | BoardCommand::AddPlaceholder { .. }
                | BoardCommand::CreateVariable { .. } => {}
                BoardCommand::UpdateNodePin {
                    node_id,
                    pin_id,
                    value,
                    ..
                } => {
                    // Only dynamic-pin literals deferred from setup are applied here (after
                    // on_update created the pin); static pins were already set in setup.
                    let node_id = self.resolve_node_id(board, node_id)?;
                    if !self
                        .deferred_pin_updates
                        .contains(&(node_id.clone(), pin_id.clone()))
                    {
                        continue;
                    }
                    let mut node = self.resolve_node(board, &node_id)?.clone();
                    let Some(resolved_pin) = resolve_pin_id_in_node(&node, pin_id, Some(PinType::Input))
                        .ok()
                        .filter(|resolved| node.pins.contains_key(resolved))
                    else {
                        // on_update still did not create the pin (e.g. a mistyped field) — skip it
                        // gracefully rather than failing the apply.
                        continue;
                    };
                    if let Some(pin) = node.pins.get_mut(&resolved_pin) {
                        pin.default_value = Some(flow_like_types::json::to_vec(value)?);
                        self.staged_nodes.insert(node_id.clone(), node.clone());
                        generic_commands
                            .push(GenericCommand::UpdateNode(UpdateNodeCommand::new(node)));
                    }
                }
                BoardCommand::RemoveNode { node_id, .. } => {
                    let node_id = self.resolve_node_id(board, node_id)?;
                    let node = self.resolve_node(board, &node_id)?.clone();
                    generic_commands.push(GenericCommand::RemoveNode(RemoveNodeCommand::new(node)));
                }
                BoardCommand::ConnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } => {
                    let from_node_id = self.resolve_node_id(board, from_node)?;
                    let to_node_id = self.resolve_node_id(board, to_node)?;
                    let from_pin_id =
                        self.resolve_pin_id(board, &from_node_id, from_pin, Some(PinType::Output))?;
                    let to_pin_id =
                        self.resolve_pin_id(board, &to_node_id, to_pin, Some(PinType::Input))?;
                    generic_commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
                        from_node_id,
                        to_node_id,
                        from_pin_id,
                        to_pin_id,
                    )));
                }
                BoardCommand::DisconnectPins {
                    from_node,
                    from_pin,
                    to_node,
                    to_pin,
                    ..
                } => {
                    let from_node_id = self.resolve_node_id(board, from_node)?;
                    let to_node_id = self.resolve_node_id(board, to_node)?;
                    let from_pin_id =
                        self.resolve_pin_id(board, &from_node_id, from_pin, Some(PinType::Output))?;
                    let to_pin_id =
                        self.resolve_pin_id(board, &to_node_id, to_pin, Some(PinType::Input))?;
                    generic_commands.push(GenericCommand::DisconnectPin(
                        DisconnectPinsCommand::new(
                            from_node_id,
                            to_node_id,
                            from_pin_id,
                            to_pin_id,
                        ),
                    ));
                }
                BoardCommand::MoveNode {
                    node_id,
                    position,
                    target_layer,
                    ..
                } => {
                    let node_id = self.resolve_node_id(board, node_id)?;
                    let current_layer =
                        self.target_layer_or_current(board, target_layer.as_deref())?;
                    generic_commands.push(GenericCommand::MoveNode(MoveNodeCommand::new(
                        node_id,
                        (position.x as f32, position.y as f32, 0.0),
                        current_layer,
                    )));
                }
                BoardCommand::UpdateVariable {
                    variable_id,
                    name,
                    data_type,
                    value_type,
                    default_value,
                    clear_default_value,
                    description,
                    clear_description,
                    category,
                    clear_category,
                    schema,
                    clear_schema,
                    exposed,
                    secret,
                    editable,
                    runtime_configured,
                    value,
                    ..
                } => {
                    let Some(existing_variable) = board.variables.get(variable_id) else {
                        return Err(flow_like_types::anyhow!(
                            "Variable `{variable_id}` not found"
                        ));
                    };
                    let mut variable = existing_variable.clone();
                    if let Some(name) = name {
                        variable.name = name.clone();
                    }
                    if let Some(data_type) = data_type {
                        variable.data_type = variable_type_from_str(data_type);
                    }
                    if let Some(value_type) = value_type {
                        variable.value_type = value_type_from_str(value_type);
                    }
                    if *clear_default_value {
                        variable.default_value = None;
                    } else if let Some(default_value) = default_value.as_ref().or(value.as_ref()) {
                        variable.default_value =
                            Some(flow_like_types::json::to_vec(default_value)?);
                    }
                    if *clear_description {
                        variable.description = None;
                    } else if let Some(description) = description {
                        variable.description = Some(description.clone());
                    }
                    if *clear_category {
                        variable.category = None;
                    } else if let Some(category) = category {
                        variable.category = Some(category.clone());
                    }
                    if *clear_schema {
                        variable.schema = None;
                    } else if let Some(schema) = schema {
                        variable.schema = Some(schema.clone());
                    }
                    if let Some(exposed) = exposed {
                        variable.exposed = *exposed;
                    }
                    if let Some(secret) = secret {
                        variable.secret = *secret;
                    }
                    if let Some(editable) = editable {
                        variable.editable = *editable;
                    }
                    if let Some(runtime_configured) = runtime_configured {
                        variable.runtime_configured = *runtime_configured;
                    }
                    generic_commands.push(GenericCommand::UpsertVariable(
                        UpsertVariableCommand::new(variable),
                    ));
                }
                BoardCommand::RemoveVariable { variable_id, .. } => {
                    let Some(variable) = board.variables.get(variable_id) else {
                        return Err(flow_like_types::anyhow!(
                            "Variable `{variable_id}` not found"
                        ));
                    };
                    generic_commands.push(GenericCommand::RemoveVariable(
                        RemoveVariableCommand::new(variable.clone()),
                    ));
                }
                BoardCommand::AddComment {
                    content,
                    position,
                    width,
                    height,
                    color,
                    target_layer,
                    ..
                } => {
                    let coordinates = self.position_or_base(Some(position));
                    let comment = Comment {
                        id: create_id(),
                        author: Some("copilot".to_string()),
                        content: content.clone(),
                        comment_type: CommentType::Text,
                        timestamp: SystemTime::now(),
                        coordinates,
                        width: width.map(|value| value as f32).or(Some(200.0)),
                        height: height.map(|value| value as f32).or(Some(100.0)),
                        layer: None,
                        color: color.clone(),
                        z_index: None,
                        hash: None,
                        is_locked: None,
                    };
                    let mut command = UpsertCommentCommand::new(comment);
                    command.current_layer =
                        self.target_layer_or_current(board, target_layer.as_deref())?;
                    generic_commands.push(GenericCommand::UpsertComment(command));
                }
                BoardCommand::RemoveComment { comment_id, .. } => {
                    let Some(comment) = board.comments.get(comment_id) else {
                        return Err(flow_like_types::anyhow!("Comment `{comment_id}` not found"));
                    };
                    generic_commands.push(GenericCommand::RemoveComment(
                        RemoveCommentCommand::new(comment.clone()),
                    ));
                }
                BoardCommand::CreateLayer {
                    name,
                    ref_id,
                    layer_type,
                    node_ids,
                    pins,
                    position,
                    color,
                    target_layer,
                    ..
                } => {
                    if node_ids.is_empty() || ref_id.is_some() || pins.is_some() {
                        continue;
                    }
                    let mut layer =
                        Layer::new(create_id(), name.clone(), layer_type_from_str(layer_type));
                    layer.coordinates = self.position_or_base(position.as_ref());
                    layer.color = color.clone();
                    layer.pins = layer_pins(pins.as_deref());
                    let node_ids = self.resolve_node_ids(board, node_ids)?;
                    let mut command = UpsertLayerCommand::new(layer);
                    command.node_ids = node_ids;
                    command.current_layer =
                        self.target_layer_or_current(board, target_layer.as_deref())?;
                    generic_commands.push(GenericCommand::UpsertLayer(command));
                }
                BoardCommand::RemoveLayer { layer_id, .. } => {
                    let layer_id = self.resolve_node_id(board, layer_id)?;
                    let Some(layer) = board.layers.get(&layer_id) else {
                        return Err(flow_like_types::anyhow!("Layer `{layer_id}` not found"));
                    };
                    generic_commands.push(GenericCommand::RemoveLayer(RemoveLayerCommand::new(
                        layer.clone(),
                        Vec::new(),
                        true,
                    )));
                }
            }
        }

        Ok(generic_commands)
    }

    fn register_node_aliases(&mut self, aliases: &[Option<&str>], node_id: &str) {
        for alias in aliases.iter().flatten() {
            if alias.trim().is_empty() || self.ambiguous_node_refs.contains(*alias) {
                continue;
            }
            match self.node_refs.get(*alias) {
                Some(existing) if existing == node_id => {}
                Some(_) => {
                    self.node_refs.remove(*alias);
                    self.ambiguous_node_refs.insert((*alias).to_string());
                }
                None => {
                    self.node_refs
                        .insert((*alias).to_string(), node_id.to_string());
                }
            }
        }
    }

    fn resolve_node_id(&self, board: &Board, node_ref: &str) -> flow_like_types::Result<String> {
        if board.nodes.contains_key(node_ref)
            || board.layers.contains_key(node_ref)
            || self.staged_nodes.contains_key(node_ref)
            || self.staged_layers.contains_key(node_ref)
        {
            return Ok(node_ref.to_string());
        }
        if self.ambiguous_node_refs.contains(node_ref) {
            return Err(flow_like_types::anyhow!(
                "Node reference `{node_ref}` is ambiguous"
            ));
        }
        self.node_refs
            .get(node_ref)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("Node reference `{node_ref}` not found"))
    }

    fn resolve_node<'a>(
        &'a self,
        board: &'a Board,
        node_id: &str,
    ) -> flow_like_types::Result<&'a Node> {
        self.staged_nodes
            .get(node_id)
            .or_else(|| board.nodes.get(node_id))
            .or_else(|| {
                board
                    .layers
                    .values()
                    .find_map(|layer| layer.nodes.get(node_id))
            })
            .ok_or_else(|| flow_like_types::anyhow!("Node `{node_id}` not found"))
    }

    fn resolve_pin_id(
        &self,
        board: &Board,
        entity_id: &str,
        pin_ref: &str,
        expected: Option<PinType>,
    ) -> flow_like_types::Result<String> {
        if let Some(node) = self
            .staged_nodes
            .get(entity_id)
            .or_else(|| board.nodes.get(entity_id))
        {
            return resolve_pin_id_in_node(node, pin_ref, expected);
        }

        if let Some(layer) = board.layers.get(entity_id) {
            return resolve_pin_id_in_pins(&layer.name, &layer.pins, pin_ref, None);
        }

        if let Some(layer) = self.staged_layers.get(entity_id) {
            return resolve_pin_id_in_pins(&layer.name, &layer.pins, pin_ref, None);
        }

        Err(flow_like_types::anyhow!("Entity `{entity_id}` not found"))
    }

    fn resolve_optional_layer(
        &self,
        board: &Board,
        layer_ref: Option<&str>,
    ) -> flow_like_types::Result<Option<String>> {
        let Some(layer_ref) = layer_ref.filter(|value| !value.trim().is_empty()) else {
            return Ok(None);
        };
        let layer_id = self.resolve_node_id(board, layer_ref)?;
        if !board.layers.contains_key(&layer_id) && !self.staged_layers.contains_key(&layer_id) {
            return Err(flow_like_types::anyhow!("Layer `{layer_ref}` not found"));
        }
        Ok(Some(layer_id))
    }

    fn target_layer_or_current(
        &self,
        board: &Board,
        target_layer: Option<&str>,
    ) -> flow_like_types::Result<Option<String>> {
        self.resolve_optional_layer(board, target_layer)
            .map(|layer| layer.or_else(|| self.current_layer.clone()))
    }

    fn resolve_node_ids(
        &self,
        board: &Board,
        refs: &[String],
    ) -> flow_like_types::Result<Vec<String>> {
        refs.iter()
            .map(|node_ref| self.resolve_node_id(board, node_ref))
            .collect()
    }

    fn position_or_next(&mut self, position: Option<&NodePosition>) -> (f32, f32, f32) {
        if let Some(position) = position {
            return (position.x as f32, position.y as f32, 0.0);
        }

        let index = self.next_position;
        self.next_position += 1;
        (
            self.base_x + ((index % 3) as f32 * 300.0),
            self.base_y + ((index / 3) as f32 * 200.0),
            0.0,
        )
    }

    fn position_or_base(&self, position: Option<&NodePosition>) -> (f32, f32, f32) {
        position
            .map(|position| (position.x as f32, position.y as f32, 0.0))
            .unwrap_or((self.base_x, self.base_y, 0.0))
    }
}

fn placeholder_pins(defs: Option<&[PlaceholderPinDef]>) -> HashMap<String, Pin> {
    let mut pins = HashMap::new();
    insert_placeholder_pin(
        &mut pins,
        "exec_in",
        "Exec In",
        "",
        PinType::Input,
        VariableType::Execution,
        ValueType::Normal,
        0,
    );
    insert_placeholder_pin(
        &mut pins,
        "exec_out",
        "Exec Out",
        "",
        PinType::Output,
        VariableType::Execution,
        ValueType::Normal,
        1,
    );

    let Some(defs) = defs else {
        return pins;
    };
    insert_layer_pins(&mut pins, defs, 2);
    pins
}

fn layer_pins(defs: Option<&[PlaceholderPinDef]>) -> HashMap<String, Pin> {
    let mut pins = HashMap::new();
    if let Some(defs) = defs {
        insert_layer_pins(&mut pins, defs, 0);
    }
    pins
}

fn insert_layer_pins(
    pins: &mut HashMap<String, Pin>,
    defs: &[PlaceholderPinDef],
    start_index: usize,
) {
    for (offset, def) in defs.iter().enumerate() {
        insert_placeholder_pin(
            pins,
            &def.name,
            &def.friendly_name,
            def.description.as_deref().unwrap_or(""),
            pin_type_from_str(&def.pin_type),
            variable_type_from_str(&def.data_type),
            def.value_type
                .as_deref()
                .map(value_type_from_str)
                .unwrap_or(ValueType::Normal),
            (offset + start_index) as u16,
        );
    }
}

fn insert_placeholder_pin(
    pins: &mut HashMap<String, Pin>,
    name: &str,
    friendly_name: &str,
    description: &str,
    pin_type: PinType,
    data_type: VariableType,
    value_type: ValueType,
    index: u16,
) {
    let id = create_id();
    pins.insert(
        id.clone(),
        Pin {
            id,
            name: name.to_string(),
            friendly_name: friendly_name.to_string(),
            description: description.to_string(),
            pin_type,
            data_type,
            schema: None,
            value_type,
            depends_on: Default::default(),
            connected_to: Default::default(),
            default_value: None,
            index,
            options: None,
            value: None,
        },
    );
}

fn resolve_pin_id_in_node(
    node: &Node,
    pin_ref: &str,
    expected: Option<PinType>,
) -> flow_like_types::Result<String> {
    resolve_pin_id_in_pins(
        &format!("{} ({})", node.friendly_name, node.name),
        &node.pins,
        pin_ref,
        expected,
    )
}

fn resolve_pin_id_in_pins(
    entity_name: &str,
    pins: &HashMap<String, Pin>,
    pin_ref: &str,
    expected: Option<PinType>,
) -> flow_like_types::Result<String> {
    if let Some(pin) = pins.get(pin_ref) {
        if pin_matches_direction(pin, expected.as_ref()) {
            return Ok(pin_ref.to_string());
        }
    }

    let requested = pin_lookup_keys(pin_ref);
    for pin in pins.values() {
        if !pin_matches_direction(pin, expected.as_ref()) {
            continue;
        }
        if pin_lookup_keys(&pin.name)
            .iter()
            .chain(pin_lookup_keys(&pin.friendly_name).iter())
            .any(|key| requested.contains(key))
        {
            return Ok(pin.id.clone());
        }
    }

    if expected.as_ref() != Some(&PinType::Input)
        && DEFAULT_OUTPUT_PIN_ALIASES
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(pin_ref))
    {
        if let Some(default_pin) = default_data_output_pin(pins) {
            return Ok(default_pin.id.clone());
        }
    }

    Err(flow_like_types::anyhow!(
        "Pin `{pin_ref}` not found on `{entity_name}`"
    ))
}

fn default_data_output_pin(pins: &HashMap<String, Pin>) -> Option<&Pin> {
    let mut outputs = pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution)
        .collect::<Vec<_>>();
    outputs.sort_by_key(|pin| pin.index);
    match outputs.as_slice() {
        [single] => Some(*single),
        many => many.iter().copied().find(|pin| {
            DEFAULT_OUTPUT_PIN_ALIASES
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(&pin.name))
        }),
    }
}

fn pin_matches_direction(pin: &Pin, expected: Option<&PinType>) -> bool {
    expected.is_none_or(|expected| &pin.pin_type == expected)
}

fn pin_lookup_keys(value: &str) -> HashSet<String> {
    let camel = to_camel_case(value);
    [
        value.to_string(),
        value.to_lowercase(),
        camel.clone(),
        camel.to_lowercase(),
    ]
    .into_iter()
    .collect()
}

fn variable_type_from_str(value: &str) -> VariableType {
    match value {
        "Execution" | "exec" => VariableType::Execution,
        "Integer" | "int" => VariableType::Integer,
        "Float" | "float" => VariableType::Float,
        "Boolean" | "bool" => VariableType::Boolean,
        "Date" => VariableType::Date,
        "PathBuf" | "Path" => VariableType::PathBuf,
        "Generic" | "any" => VariableType::Generic,
        "Struct" => VariableType::Struct,
        "Byte" | "bytes" => VariableType::Byte,
        _ => VariableType::String,
    }
}

fn value_type_from_str(value: &str) -> ValueType {
    match value {
        "Array" => ValueType::Array,
        "HashMap" | "Map" => ValueType::HashMap,
        "HashSet" | "Set" => ValueType::HashSet,
        _ => ValueType::Normal,
    }
}

fn layer_type_from_str(value: &Option<String>) -> LayerType {
    match value.as_deref().unwrap_or("Collapsed") {
        "Function" | "function" => LayerType::Function,
        "Macro" | "macro" => LayerType::Macro,
        _ => LayerType::Collapsed,
    }
}

fn pin_type_from_str(value: &str) -> PinType {
    match value {
        "Output" => PinType::Output,
        _ => PinType::Input,
    }
}
