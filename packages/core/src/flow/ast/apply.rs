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
        node::{FnRefs, Node, NodeLogic},
        pin::{Pin, PinOptions, PinType, ValueType},
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
    /// Non-blocking deterministic source repairs (for example a stale anchor rebound to the one
    /// compatible live entry). Clients should reload canonical FlowScript when this is non-empty,
    /// even if the repaired document required no board mutation.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub corrections: Vec<String>,
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

fn reconcile_is_safe_to_apply(commands: &[BoardCommand], diagnostics: &[String]) -> bool {
    !commands.is_empty() && diagnostics.is_empty()
}

pub async fn apply_flowscript_to_board(
    board: &mut Board,
    flowscript: &str,
    catalog_nodes: &[Node],
    state: Arc<FlowLikeState>,
    current_layer: Option<String>,
    allow_deletions: bool,
) -> flow_like_types::Result<ApplyFlowScriptResult> {
    apply_flowscript_to_board_scoped(
        board,
        flowscript,
        catalog_nodes,
        state,
        current_layer,
        allow_deletions,
        None,
    )
    .await
}

/// Like [`apply_flowscript_to_board`] with an editing scope: when `scope_anchors` is `Some`, the
/// document is a selection-scoped render (see `board_to_flowscript_scoped`) and board
/// events/functions whose anchor is not listed are invisible to the deletion diff — an omitted
/// out-of-scope section is never treated as a removal. In-scope deletions still work and stay
/// behind the `allow_deletions` gate.
#[allow(clippy::too_many_arguments)]
pub async fn apply_flowscript_to_board_scoped(
    board: &mut Board,
    flowscript: &str,
    catalog_nodes: &[Node],
    state: Arc<FlowLikeState>,
    current_layer: Option<String>,
    allow_deletions: bool,
    scope_anchors: Option<&[String]>,
) -> flow_like_types::Result<ApplyFlowScriptResult> {
    let catalog_metadata = catalog_nodes
        .iter()
        .map(node_to_metadata)
        .collect::<Vec<_>>();

    // Resolve dynamic (on_update-generated) pins during reconcile by running each node's on_update on
    // an in-memory scratch node seeded with the call's literal args — no board mutation. Limited to
    // audited pure nodes whose on_update only reads their own pins (no network / cross-node reads).
    //
    // The SQL nodes qualify: their `on_update` reads only their own `query` pin and derives
    // pins from it. The widget nodes deliberately do NOT — their `on_update` awaits app
    // storage to resolve the widget, which cannot be driven from a synchronous `block_on`
    // here; reconcile predicts their `dyn*` pins permissively instead.
    const ENRICH_ALLOWLIST: &[&str] = &[
        "string_format",
        "string_render_template",
        "a2ui_push_csv_to_chart",
        "df_sql_query",
        "df_sql_query_cached",
        "df_execute_sql",
        "df_write_delta",
        "graph_sql_query",
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
                            // Reuse the reconciler's exact matcher (name OR friendly_name, each
                            // snake-or-camel; `to_camel_case` normalizes spaces in friendly names)
                            // so seeding can never drift from `metadata_pin_name_matches`.
                            pin.pin_type == PinType::Input
                                && (super::reconcile::pin_name_matches(&pin.name, arg_name)
                                    || super::reconcile::pin_name_matches(
                                        &pin.friendly_name,
                                        arg_name,
                                    ))
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
        Some(enricher) => super::reconcile_text_with_catalog_enriched_scoped(
            board,
            flowscript,
            &catalog_metadata,
            enricher,
            scope_anchors,
        ),
        None => super::reconcile_text_with_catalog_scoped(
            board,
            flowscript,
            &catalog_metadata,
            scope_anchors,
        ),
    };

    let corrections = std::mem::take(&mut reconcile.corrections);

    // FlowScript is a program, not a bag of best-effort mutations. Every reconcile diagnostic means
    // some requested call, pin, connection, execution edge or boundary could not be represented.
    // Applying the remaining setup commands is how empty function layers and disconnected nodes were
    // created, so the server apply boundary is atomic even if a caller bypasses the agent tool gate.
    if !reconcile_is_safe_to_apply(&reconcile.commands, &reconcile.diagnostics) {
        return Ok(ApplyFlowScriptResult {
            commands: Vec::new(),
            board_commands: reconcile.commands,
            corrections,
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
                corrections,
                diagnostics,
            });
        }
    }

    let mut applied = apply_board_commands_to_board(
        board,
        reconcile.commands,
        catalog_nodes,
        state,
        current_layer,
    )
    .await?;
    applied.corrections = corrections;
    Ok(applied)
}

/// Apply an exact, already-validated [`BoardCommand`] batch without reconciling FlowScript again.
///
/// This is the execution half of the typed-IR commit boundary. Callers must obtain the batch from
/// the retained pending claim while holding the live board lock; arbitrary client commands must
/// continue through reconciliation and deletion approval in [`apply_flowscript_to_board`]. The
/// planner still performs its normal two-phase execution, rollback, and Function-layer
/// postcondition validation.
pub async fn apply_board_commands_to_board(
    board: &mut Board,
    board_commands: Vec<BoardCommand>,
    catalog_nodes: &[Node],
    state: Arc<FlowLikeState>,
    current_layer: Option<String>,
) -> flow_like_types::Result<ApplyFlowScriptResult> {
    let mut planner = FlowScriptApplyPlanner::new(board, catalog_nodes, current_layer);
    let setup_commands = planner.build_setup_commands(board, &board_commands)?;
    let mut applied_commands = Vec::new();

    // `execute_commands` appends the node state `on_update` derived during this phase, so the
    // flattened batch stays replayable on a machine that has never run it.
    if !setup_commands.is_empty() {
        match board.execute_commands(setup_commands, state.clone()).await {
            Ok(mut executed) => applied_commands.append(&mut executed),
            Err(error) => return Err(error),
        }
    }

    let remaining_commands = match planner.build_remaining_commands(board, &board_commands) {
        Ok(commands) => commands,
        Err(error) => {
            rollback_applied(board, &applied_commands, state.clone(), error).await?;
            return Ok(ApplyFlowScriptResult {
                commands: Vec::new(),
                board_commands,
                corrections: Vec::new(),
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

    if let Err(error) = planner.validate_new_function_layers(board, &board_commands) {
        rollback_applied(board, &applied_commands, state.clone(), error).await?;
    }

    Ok(ApplyFlowScriptResult {
        commands: applied_commands,
        board_commands,
        corrections: Vec::new(),
        diagnostics: Vec::new(),
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
    current_layer: Option<String>,
    base_x: f32,
    base_y: f32,
    next_position: usize,
    next_node_index: usize,
    /// `(resolved_node_id, pin_ref, value)` pin writes whose target pin does not exist yet in the
    /// setup phase because a node's `on_update` mints it (e.g. `string_format` placeholders). They
    /// are applied in the remaining phase, after `execute_commands` has run `on_update`.
    deferred_pin_updates: Vec<(String, String, flow_like_types::Value)>,
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
            current_layer,
            base_x: 100.0,
            base_y: 100.0,
            next_position: 0,
            next_node_index: 0,
            deferred_pin_updates: Vec::new(),
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
            let friendly_name =
                (!node.friendly_name.trim().is_empty()).then_some(node.friendly_name.as_str());
            planner.register_node_aliases(&[Some(node.id.as_str()), friendly_name], &node.id);
        }
        for layer in board.layers.values() {
            planner.register_node_aliases(
                &[Some(layer.id.as_str()), Some(layer.name.as_str())],
                &layer.id,
            );
        }
        for (name, node_id) in &board.refs {
            if crate::flow::board::is_internal_board_ref(name) {
                continue;
            }
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
        // Reconciliation assigns stable explicit refs before it reorders setup commands (for
        // example, moving the Event entry to the end). Reserve every explicit ref up front so an
        // order-derived positional alias can never shadow it, even when that explicit command is
        // encountered later in this loop.
        let explicit_refs = commands
            .iter()
            .filter_map(|command| match command {
                BoardCommand::AddNode {
                    ref_id: Some(ref_id),
                    ..
                }
                | BoardCommand::AddPlaceholder {
                    ref_id: Some(ref_id),
                    ..
                }
                | BoardCommand::CreateLayer {
                    ref_id: Some(ref_id),
                    ..
                } => Some(ref_id.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>();

        for command in commands {
            match command {
                BoardCommand::AddNode {
                    node_type,
                    ref_id,
                    position,
                    friendly_name,
                    additional_pins,
                    target_layer,
                    ..
                } => {
                    let Some(catalog_node) = self.catalog_nodes.get(node_type) else {
                        return Err(flow_like_types::anyhow!(
                            "Node type `{node_type}` is not available in the catalog"
                        ));
                    };

                    let mut add_command = AddNodeCommand::new(catalog_node.clone());
                    append_additional_node_pins(&mut add_command.node, additional_pins.as_deref())?;
                    let coordinates = self.position_or_next(position.as_ref());
                    add_command.node.coordinates = Some(coordinates);
                    if let Some(friendly_name) = friendly_name {
                        add_command.node.friendly_name = friendly_name.clone();
                    }
                    add_command.current_layer =
                        self.target_layer_or_current(board, target_layer.as_deref())?;
                    // Setup-phase pin updates resolve this staged clone before AddNode executes.
                    // Mirror AddNodeCommand::execute's layer assignment now; otherwise the first
                    // UpdateNode built from `staged_nodes` replaces the live node with layer=None
                    // and silently ejects every configured function-body node back to root.
                    add_command.node.layer = add_command.current_layer.clone();

                    let node_id = add_command.node.id.clone();
                    self.staged_nodes
                        .insert(node_id.clone(), add_command.node.clone());
                    let index_alias = format!("${}", self.next_node_index);
                    self.next_node_index += 1;
                    let positional_alias = (ref_id.is_none()
                        && !explicit_refs.contains(index_alias.as_str()))
                    .then_some(index_alias.as_str());
                    self.register_node_aliases(
                        &[
                            ref_id.as_deref(),
                            positional_alias,
                            friendly_name.as_deref(),
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
                    let positional_alias = (ref_id.is_none()
                        && !explicit_refs.contains(index_alias.as_str()))
                    .then_some(index_alias.as_str());
                    self.register_node_aliases(
                        &[
                            ref_id.as_deref(),
                            positional_alias,
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
                    cache,
                    ..
                } if node_ids.is_empty() || ref_id.is_some() || pins.is_some() => {
                    let layer_id = create_id();
                    let resolved_layer_type = layer_type_from_str(layer_type);
                    if cache.is_some() && !matches!(&resolved_layer_type, LayerType::Function) {
                        return Err(flow_like_types::anyhow!(
                            "Layer `{name}` is not a Function layer and cannot be cached"
                        ));
                    }
                    let mut layer = Layer::new(layer_id.clone(), name.clone(), resolved_layer_type);
                    layer.coordinates = self.position_or_base(position.as_ref());
                    layer.color = color.clone();
                    layer.pins = layer_pins(pins.as_deref());
                    layer.cache = cache.clone();

                    let mut command = UpsertLayerCommand::new(layer.clone());
                    command.current_layer =
                        self.target_layer_or_current(board, target_layer.as_deref())?;

                    // Keep the staged view identical to what UpsertLayer will persist so later
                    // setup commands can safely target this layer in the same batch.
                    layer.parent_id = command.current_layer.clone();
                    self.staged_layers.insert(layer_id.clone(), layer);
                    let index_alias = format!("${}", self.next_node_index);
                    self.next_node_index += 1;
                    let positional_alias = (ref_id.is_none()
                        && !explicit_refs.contains(index_alias.as_str()))
                    .then_some(index_alias.as_str());
                    self.register_node_aliases(
                        &[
                            ref_id.as_deref(),
                            positional_alias,
                            Some(name.as_str()),
                            Some(layer_id.as_str()),
                        ],
                        &layer_id,
                    );

                    generic_commands.push(GenericCommand::UpsertLayer(command));
                }
                BoardCommand::UpdateLayerCache {
                    layer_id, cache, ..
                } => {
                    let layer_id = self.resolve_node_id(board, layer_id)?;
                    let Some(existing_layer) = self
                        .staged_layers
                        .get(&layer_id)
                        .or_else(|| board.layers.get(&layer_id))
                    else {
                        return Err(flow_like_types::anyhow!("Layer `{layer_id}` not found"));
                    };
                    if !matches!(&existing_layer.r#type, LayerType::Function) {
                        return Err(flow_like_types::anyhow!(
                            "Layer `{layer_id}` is not a Function layer and cannot be cached"
                        ));
                    }
                    let mut layer = existing_layer.clone();
                    layer.cache = cache.clone();

                    let mut command = UpsertLayerCommand::new(layer.clone());
                    // UpsertLayer assigns parent_id from current_layer during execute. Preserve the
                    // existing hierarchy while changing only cache metadata.
                    command.current_layer = layer.parent_id.clone();
                    self.staged_layers.insert(layer_id, layer);
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
                BoardCommand::UpdateNodePin {
                    node_id,
                    pin_id,
                    value,
                    ..
                } => {
                    let node_id = self.resolve_node_id(board, node_id)?;
                    let mut node = self.resolve_node(board, &node_id)?.clone();
                    // The pin may not exist yet: a node's `on_update` mints dynamic pins (e.g. a
                    // `string_format` placeholder) only after the config pin is applied and the
                    // batch runs. Defer such writes to the remaining phase instead of failing.
                    let Ok(pin_id) = resolve_pin_id_in_node(&node, pin_id, Some(PinType::Input))
                    else {
                        self.deferred_pin_updates
                            .push((node_id, pin_id.clone(), value.clone()));
                        continue;
                    };
                    let value =
                        self.resolve_layer_reference_pin_value(board, &node, &pin_id, value);
                    let Some(pin) = node.pins.get_mut(&pin_id) else {
                        return Err(flow_like_types::anyhow!(
                            "Pin `{pin_id}` not found on node `{node_id}`"
                        ));
                    };
                    pin.default_value = Some(flow_like_types::json::to_vec(&value)?);
                    self.staged_nodes.insert(node_id.clone(), node.clone());
                    generic_commands.push(GenericCommand::UpdateNode(UpdateNodeCommand::new(node)));
                }
                BoardCommand::RenameNode {
                    node_id,
                    friendly_name,
                    ..
                } => {
                    let node_id = self.resolve_node_id(board, node_id)?;
                    let mut node = self.resolve_node(board, &node_id)?.clone();
                    node.friendly_name = friendly_name.clone();
                    self.staged_nodes.insert(node_id.clone(), node.clone());
                    generic_commands.push(GenericCommand::UpdateNode(UpdateNodeCommand::new(node)));
                }
                _ => {}
            }
        }

        Ok(generic_commands)
    }

    /// Apply pin writes deferred from setup, now that `on_update` has minted their target pins.
    /// Multiple pins on one node are folded into a single `UpdateNode` (each command is a whole-node
    /// replace, so per-pin commands would overwrite each other). Node order is first-seen.
    ///
    /// A pin still missing at this point is a hard error: reconcile accepted the argument on the
    /// promise that configuring the node would create it, and it did not. See
    /// [`deferred_pin_error`] for why the generic wording is not enough here.
    fn build_deferred_pin_updates(
        &mut self,
        board: &Board,
    ) -> flow_like_types::Result<Vec<GenericCommand>> {
        let deferred = std::mem::take(&mut self.deferred_pin_updates);
        let mut order: Vec<String> = Vec::new();
        let mut nodes: HashMap<String, Node> = HashMap::new();

        for (node_id, pin_ref, value) in deferred {
            let node = match nodes.entry(node_id.clone()) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => {
                    order.push(node_id.clone());
                    entry.insert(self.resolve_node(board, &node_id)?.clone())
                }
            };
            let pin_id = resolve_pin_id_in_node(node, &pin_ref, Some(PinType::Input))
                .map_err(|error| deferred_pin_error(node, &pin_ref, error))?;
            let value = {
                let node_ref = &*node;
                self.resolve_layer_reference_pin_value(board, node_ref, &pin_id, &value)
            };
            let Some(pin) = node.pins.get_mut(&pin_id) else {
                return Err(flow_like_types::anyhow!(
                    "Pin `{pin_ref}` not found on node `{node_id}` after node update"
                ));
            };
            pin.default_value = Some(flow_like_types::json::to_vec(&value)?);
        }

        Ok(order
            .into_iter()
            .filter_map(|node_id| nodes.remove(&node_id))
            .map(|node| GenericCommand::UpdateNode(UpdateNodeCommand::new(node)))
            .collect())
    }

    fn build_remaining_commands(
        &mut self,
        board: &Board,
        commands: &[BoardCommand],
    ) -> flow_like_types::Result<Vec<GenericCommand>> {
        self.staged_nodes.clear();
        // Compose every whole-node mutation per node before creating UpdateNode commands. Both a
        // deferred dynamic-pin write and SetNodeFunctionRefs replace the entire persisted node; if
        // they are built independently from `board`, the later replacement silently erases the
        // earlier one. Emit the composed replacements before moves/removals and connections so a
        // MoveNode remains authoritative and no freshly-created pin edge can be clobbered.
        let mut node_update_order = Vec::new();
        let mut node_updates = HashMap::<String, Node>::new();
        for command in self.build_deferred_pin_updates(board)? {
            let GenericCommand::UpdateNode(command) = command else {
                unreachable!("deferred pin updates only produce UpdateNode commands");
            };
            node_update_order.push(command.node.id.clone());
            node_updates.insert(command.node.id.clone(), command.node);
        }
        let mut fn_refs_initialized = HashSet::new();
        let mut generic_commands = Vec::new();
        // Every Connect/Disconnect must execute after all whole-node UpdateNode commands and all
        // moves/removals. Connections are the final persisted graph mutations in this batch.
        let mut connection_commands = Vec::new();

        for command in commands {
            match command {
                BoardCommand::AddNode { .. }
                | BoardCommand::AddPlaceholder { .. }
                | BoardCommand::CreateVariable { .. }
                | BoardCommand::UpdateVariable { .. }
                | BoardCommand::UpdateNodePin { .. }
                | BoardCommand::RenameNode { .. }
                | BoardCommand::UpdateLayerCache { .. } => {}
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
                    connection_commands.push(GenericCommand::ConnectPin(ConnectPinsCommand::new(
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
                    connection_commands.push(GenericCommand::DisconnectPin(
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
                        node_id: None,
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
                    cache,
                    ..
                } => {
                    if node_ids.is_empty() || ref_id.is_some() || pins.is_some() {
                        continue;
                    }
                    let resolved_layer_type = layer_type_from_str(layer_type);
                    if cache.is_some() && !matches!(&resolved_layer_type, LayerType::Function) {
                        return Err(flow_like_types::anyhow!(
                            "Layer `{name}` is not a Function layer and cannot be cached"
                        ));
                    }
                    let mut layer = Layer::new(create_id(), name.clone(), resolved_layer_type);
                    layer.coordinates = self.position_or_base(position.as_ref());
                    layer.color = color.clone();
                    layer.pins = layer_pins(pins.as_deref());
                    layer.cache = cache.clone();
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
                BoardCommand::SetNodeFunctionRefs {
                    node_id, fn_refs, ..
                } => {
                    let node_id = self.resolve_node_id(board, node_id)?;
                    let mut resolved: Vec<String> = Vec::new();
                    for reference in fn_refs {
                        // A requested tool is part of the executable contract. Silently dropping
                        // an unresolved name leaves a visually plausible agent with fewer tools
                        // than authored, so fail the atomic FlowScript apply instead.
                        let target_id = self.resolve_node_id(board, reference).map_err(|error| {
                            flow_like_types::anyhow!(
                                "Could not resolve requested function reference `{reference}`: {error}"
                            )
                        })?;
                        // Functions are authored as layers; reference the layer's referenceable
                        // entry node so runtime function-reference resolution finds a concrete node.
                        let entry_id = if board.layers.contains_key(&target_id)
                            || self.staged_layers.contains_key(&target_id)
                        {
                            self.referenceable_entry_in_layer(board, &target_id)?
                                .ok_or_else(|| {
                                    flow_like_types::anyhow!(
                                        "Function layer `{reference}` has no referenceable event/handler entry"
                                    )
                                })?
                        } else {
                            target_id
                        };
                        if !resolved.contains(&entry_id) {
                            resolved.push(entry_id);
                        }
                    }
                    if resolved.is_empty() {
                        continue;
                    }
                    let node = match node_updates.entry(node_id.clone()) {
                        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            node_update_order.push(node_id.clone());
                            entry.insert(self.resolve_node(board, &node_id)?.clone())
                        }
                    };
                    let can_be_referenced_by_fns = node
                        .fn_refs
                        .as_ref()
                        .map(|refs| refs.can_be_referenced_by_fns)
                        .unwrap_or(false);
                    if fn_refs_initialized.insert(node_id) {
                        // The first SetNodeFunctionRefs retains the existing replacement semantics;
                        // subsequent commands for the same node add their independently-authored
                        // targets instead of replacing the previous command's targets.
                        node.fn_refs = Some(FnRefs {
                            fn_refs: resolved,
                            can_reference_fns: true,
                            can_be_referenced_by_fns,
                        });
                        continue;
                    }
                    let existing_refs = node.fn_refs.get_or_insert_with(|| FnRefs {
                        fn_refs: Vec::new(),
                        can_reference_fns: true,
                        can_be_referenced_by_fns,
                    });
                    for entry_id in resolved {
                        if !existing_refs.fn_refs.contains(&entry_id) {
                            existing_refs.fn_refs.push(entry_id);
                        }
                    }
                    existing_refs.can_reference_fns = true;
                    existing_refs.can_be_referenced_by_fns = can_be_referenced_by_fns;
                }
            }
        }

        let mut composed_commands = node_update_order
            .into_iter()
            .filter_map(|node_id| node_updates.remove(&node_id))
            .map(|node| GenericCommand::UpdateNode(UpdateNodeCommand::new(node)))
            .collect::<Vec<_>>();
        composed_commands.append(&mut generic_commands);
        let mut generic_commands = composed_commands;
        generic_commands.extend(connection_commands);

        Ok(generic_commands)
    }

    /// `function_layer_id` pins (on `control_call_function` nodes) authored from FlowScript carry
    /// a `$n` ref or function name for layers created in the same batch; resolve it to the real
    /// layer id. Other pins pass through untouched.
    fn resolve_layer_reference_pin_value(
        &self,
        board: &Board,
        node: &Node,
        pin_id: &str,
        value: &flow_like_types::Value,
    ) -> flow_like_types::Value {
        let is_layer_ref_pin = node
            .pins
            .get(pin_id)
            .map(|pin| pin.name == "function_layer_id")
            .unwrap_or(pin_id == "function_layer_id");
        if is_layer_ref_pin
            && let Some(reference) = value.as_str()
            && let Ok(resolved) = self.resolve_node_id(board, reference)
        {
            return flow_like_types::Value::String(resolved);
        }
        value.clone()
    }

    /// If `id` refers to a layer (e.g. an authored FlowScript function), return the id of its
    /// referenceable entry node — an event-type node flagged `can_be_referenced_by_fns`. Returns
    /// `None` when `id` is not a layer or the layer has no referenceable entry. FlowScript uses
    /// the canonical flat board representation (`board.nodes[*].layer`); `layer.nodes` is retained
    /// only for legacy boards, so inspect and de-duplicate both stores.
    fn referenceable_entry_in_layer(
        &self,
        board: &Board,
        id: &str,
    ) -> flow_like_types::Result<Option<String>> {
        let layer = board.layers.get(id).or_else(|| self.staged_layers.get(id));
        let Some(layer) = layer else {
            return Ok(None);
        };

        let is_referenceable = |node: &Node| {
            node.fn_refs
                .as_ref()
                .map(|refs| refs.can_be_referenced_by_fns)
                .unwrap_or(false)
        };
        let mut entries = layer
            .nodes
            .values()
            .filter(|node| is_referenceable(node))
            .map(|node| node.id.clone())
            .chain(
                board
                    .nodes
                    .values()
                    .filter(|node| node.layer.as_deref() == Some(id) && is_referenceable(node))
                    .map(|node| node.id.clone()),
            )
            .collect::<Vec<_>>();
        entries.sort();
        entries.dedup();

        match entries.as_slice() {
            [] => Ok(None),
            [entry] => Ok(Some(entry.clone())),
            _ => Err(flow_like_types::anyhow!(
                "Function layer `{}` has multiple referenceable event/handler entries: {}. Reference the intended handler by name instead of the enclosing layer",
                layer.name,
                entries.join(", ")
            )),
        }
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
            // Function boundary directions are intentionally inverted from an inner-body edge:
            // layer Inputs provide parameter values to body nodes, while layer Outputs receive
            // body return values. Preserve that distinction so a same-named parameter/return can
            // never resolve through HashMap iteration order.
            let boundary_direction = expected.map(invert_boundary_pin_direction);
            return resolve_pin_id_in_pins(&layer.name, &layer.pins, pin_ref, boundary_direction);
        }

        if let Some(layer) = self.staged_layers.get(entity_id) {
            let boundary_direction = expected.map(invert_boundary_pin_direction);
            return resolve_pin_id_in_pins(&layer.name, &layer.pins, pin_ref, boundary_direction);
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

    /// Verify the persisted canonical graph for every Function layer created by this edit. The
    /// reconciler validates its command plan, but whole-node updates can still accidentally erase
    /// `node.layer` while applying. Treat that as an atomic apply failure so an apparently valid
    /// Function can never be committed with an empty runtime body or severed boundaries.
    fn validate_new_function_layers(
        &self,
        board: &Board,
        commands: &[BoardCommand],
    ) -> flow_like_types::Result<()> {
        for command in commands {
            let BoardCommand::CreateLayer {
                name,
                ref_id: Some(ref_id),
                layer_type,
                ..
            } = command
            else {
                continue;
            };
            if !matches!(layer_type.as_deref(), Some("Function") | Some("function")) {
                continue;
            }

            let layer_id = self.resolve_node_id(board, ref_id)?;
            let layer = board.layers.get(&layer_id).ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Applied Function `{name}` resolved to missing layer `{layer_id}`"
                )
            })?;
            let body_nodes = board
                .nodes
                .values()
                .filter(|node| node.layer.as_deref() == Some(layer_id.as_str()))
                .collect::<Vec<_>>();
            if body_nodes.is_empty() {
                return Err(flow_like_types::anyhow!(
                    "Applied Function `{name}` has no canonical body nodes assigned to layer `{layer_id}`"
                ));
            }

            let exec_in = layer
                .pins
                .values()
                .find(|pin| pin.name == "exec_in" && pin.data_type == VariableType::Execution);
            let exec_out = layer
                .pins
                .values()
                .find(|pin| pin.name == "exec_out" && pin.data_type == VariableType::Execution);
            if exec_in.is_none() && exec_out.is_none() {
                continue;
            }
            let (Some(exec_in), Some(exec_out)) = (exec_in, exec_out) else {
                return Err(flow_like_types::anyhow!(
                    "Applied Function `{name}` has an incomplete execution boundary"
                ));
            };
            let body_pin_ids = body_nodes
                .iter()
                .flat_map(|node| node.pins.values().map(|pin| pin.id.as_str()))
                .collect::<HashSet<_>>();
            if !exec_in
                .connected_to
                .iter()
                .any(|pin_id| body_pin_ids.contains(pin_id.as_str()))
            {
                return Err(flow_like_types::anyhow!(
                    "Applied Function `{name}` exec_in is not connected to a body node"
                ));
            }
            if !exec_out
                .depends_on
                .iter()
                .any(|pin_id| body_pin_ids.contains(pin_id.as_str()))
            {
                return Err(flow_like_types::anyhow!(
                    "Applied Function `{name}` exec_out does not depend on a body node"
                ));
            }
        }

        Ok(())
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
        None,
        false,
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
        None,
        false,
        1,
    );

    let Some(defs) = defs else {
        return pins;
    };
    insert_layer_pins(&mut pins, defs, 2);
    pins
}

fn append_additional_node_pins(
    node: &mut Node,
    defs: Option<&[PlaceholderPinDef]>,
) -> flow_like_types::Result<()> {
    let Some(defs) = defs else {
        return Ok(());
    };
    if !defs.is_empty() && node.name != "events_generic" {
        return Err(flow_like_types::anyhow!(
            "Additional catalog-node pins are only supported on events_generic"
        ));
    }

    for def in defs {
        if def.pin_type != "Output" || def.data_type == "Execution" {
            return Err(flow_like_types::anyhow!(
                "Additional events_generic pin `{}` must be a non-execution Output",
                def.name
            ));
        }
        if node
            .pins
            .values()
            .any(|pin| pin.pin_type == PinType::Output && pin.name == def.name)
        {
            return Err(flow_like_types::anyhow!(
                "events_generic already has an output pin named `{}`",
                def.name
            ));
        }

        let pin = node.add_output_pin(
            &def.name,
            &def.friendly_name,
            def.description.as_deref().unwrap_or(""),
            variable_type_from_str(&def.data_type),
        );
        pin.set_value_type(
            def.value_type
                .as_deref()
                .map(value_type_from_str)
                .unwrap_or(ValueType::Normal),
        );
        pin.schema = def.schema.clone();
        if def.enforce_schema {
            pin.set_options(PinOptions {
                enforce_schema: Some(true),
                ..PinOptions::default()
            });
        }
    }

    Ok(())
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
            def.schema.clone(),
            def.enforce_schema,
            (offset + start_index) as u16,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn insert_placeholder_pin(
    pins: &mut HashMap<String, Pin>,
    name: &str,
    friendly_name: &str,
    description: &str,
    pin_type: PinType,
    data_type: VariableType,
    value_type: ValueType,
    schema: Option<String>,
    enforce_schema: bool,
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
            schema,
            value_type,
            depends_on: Default::default(),
            connected_to: Default::default(),
            default_value: None,
            index,
            options: enforce_schema.then(|| PinOptions {
                enforce_schema: Some(true),
                ..PinOptions::default()
            }),
            value: None,
        },
    );
}

fn invert_boundary_pin_direction(direction: PinType) -> PinType {
    match direction {
        PinType::Input => PinType::Output,
        PinType::Output => PinType::Input,
    }
}

/// Explains a deferred pin write whose target still does not exist after `on_update` ran.
///
/// Reconcile deliberately accepts arguments naming pins it cannot see, because the pin is
/// supposed to be created by applying the node's own configuration first. When that promise
/// is not kept, "unknown pin" describes the symptom and hides the cause — which is always
/// either a configuration that did not produce the pin, or a name that does not match one it
/// produces. Both are fixable, and neither is guessable from the generic wording.
fn deferred_pin_error(
    node: &Node,
    pin_ref: &str,
    error: flow_like_types::Error,
) -> flow_like_types::Error {
    if super::reconcile::widget_dynamic_pin_node(&node.name)
        && super::reconcile::is_widget_dynamic_binding_arg(pin_ref)
    {
        return flow_like_types::anyhow!(
            "`{}` still has no input pin `{pin_ref}` after being configured. Widget binding pins come from the persisted widget, so one of these is true: the widget selector does not name an existing widget, the widget has not finished being written yet, or `{pin_ref}` is not a binding this widget exposes. `ui_inspect` with operation `widget` lists the exact pin names for a widget.",
            node.friendly_name
        );
    }
    if super::reconcile::sql_param_node(&node.name) && pin_ref.starts_with("param") {
        return flow_like_types::anyhow!(
            "`{}` still has no input pin `{pin_ref}` after being configured. Parameter pins are derived from the query literal, so the query must contain the matching $placeholder (`paramCustomerId` needs `$customer_id`) and must be set as a literal on the same call that supplies the parameter.",
            node.friendly_name
        );
    }
    error
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
    if let Some(pin) = pins.get(pin_ref)
        && pin_matches_direction(pin, expected.as_ref())
    {
        return Ok(pin_ref.to_string());
    }

    if let Some((name, occurrence)) = super::reconcile::parse_pin_occurrence_ref(pin_ref) {
        let requested = pin_lookup_keys(name);
        let mut matching = pins
            .values()
            .filter(|pin| pin_matches_direction(pin, expected.as_ref()))
            .filter_map(|pin| pin_ref_match_rank(pin, &requested).map(|rank| (rank, pin)))
            .collect::<Vec<_>>();
        // Pin ids are regenerated when a node is added, but the catalog pin indices survive.
        // Sorting by match rank, then index (then id for malformed duplicate indices) makes the
        // selector stable across setup-time default writes and later connections.
        matching.sort_by_key(|(rank, pin)| (*rank, pin.index, pin.id.clone()));
        if let Some((_, pin)) = matching.get(occurrence) {
            return Ok(pin.id.clone());
        }
        return Err(flow_like_types::anyhow!(
            "Pin occurrence `{pin_ref}` not found on `{entity_name}`"
        ));
    }

    let requested = pin_lookup_keys(pin_ref);
    let mut matching = pins
        .values()
        .filter(|pin| pin_matches_direction(pin, expected.as_ref()))
        .filter_map(|pin| pin_ref_match_rank(pin, &requested).map(|rank| (rank, pin)))
        .collect::<Vec<_>>();
    // `pins` is a HashMap, so without an explicit order the winner among several matches was
    // whichever the iterator happened to yield first.
    matching.sort_by_key(|(rank, pin)| (*rank, pin.index, pin.id.clone()));
    if let Some((_, pin)) = matching.first() {
        return Ok(pin.id.clone());
    }

    if expected.as_ref() != Some(&PinType::Input)
        && DEFAULT_OUTPUT_PIN_ALIASES
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(pin_ref))
        && let Some(default_pin) = default_data_output_pin(pins)
    {
        return Ok(default_pin.id.clone());
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

/// How closely `pin` answers to the already-normalized `requested` lookup keys: `Some(0)` when its
/// own name matches, `Some(1)` when only its friendly name does, `None` when neither. Mirrors
/// `reconcile::pin_name_match_rank` — a pin's own name must outrank another pin's friendly name, or
/// `string_format`'s config pin (named `format_string`, presented as "Input") swallows the value
/// meant for an `{input}` placeholder and the template is overwritten.
fn pin_ref_match_rank(pin: &Pin, requested: &HashSet<String>) -> Option<u8> {
    if pin_lookup_keys(&pin.name)
        .iter()
        .any(|key| requested.contains(key))
    {
        return Some(0);
    }
    pin_lookup_keys(&pin.friendly_name)
        .iter()
        .any(|key| requested.contains(key))
        .then_some(1)
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
        "Module" | "module" => LayerType::Module,
        _ => LayerType::Collapsed,
    }
}

fn pin_type_from_str(value: &str) -> PinType {
    match value {
        "Output" => PinType::Output,
        _ => PinType::Input,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::{ExecutionMode, ExecutionStage, LayerCache, LayerCacheScope};
    use crate::flow::execution::{LogLevel, context::ExecutionContext};
    use crate::flow::variable::VariableType;
    use crate::state::FlowLikeConfig;
    use crate::utils::http::HTTPClient;
    use flow_like_storage::Path;
    use flow_like_types::json::json;
    use flow_like_types::tokio;
    use std::time::SystemTime;

    fn empty_board() -> Board {
        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
            pin_index: None,
        }
    }

    /// `string_format`-style catalog node: a `format_string` input + `value` output, with NO
    /// placeholder pins (those are minted by `on_update` at apply time).
    fn dynamic_format_catalog_node() -> Node {
        let mut node = Node::new("dynamic_format", "Dynamic Format", "", "test");
        node.add_input_pin("format_string", "Input", "", VariableType::String);
        node.add_output_pin("value", "Formatted", "", VariableType::String);
        node
    }

    fn generic_event_catalog_node() -> Node {
        let mut node = Node::new("events_generic", "Generic Event", "", "events");
        node.set_can_be_referenced_by_fns(true);
        node.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        node.add_output_pin("payload", "Payload", "", VariableType::Struct);
        node
    }

    fn function_ref_consumer_catalog_node() -> Node {
        let mut node = Node::new("function_ref_consumer", "Function Ref Consumer", "", "test");
        node.set_can_reference_fns(true);
        node
    }

    fn decode_default(pin: &Pin) -> flow_like_types::Value {
        let bytes = pin
            .default_value
            .as_deref()
            .expect("pin has a default value");
        flow_like_types::json::from_slice(bytes).expect("default value decodes")
    }

    struct TestVariableGetLogic;

    #[flow_like_types::async_trait]
    impl NodeLogic for TestVariableGetLogic {
        fn get_node(&self) -> Node {
            let mut node = Node::new("variable_get", "Get Variable", "", "test");
            node.add_input_pin("var_ref", "Variable Reference", "", VariableType::String);
            node.add_output_pin("value_ref", "Value", "", VariableType::Generic);
            node
        }

        async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }

        async fn on_update(&self, node: &mut Node, board: &Board) {
            let Some(variable) = test_referenced_variable(node, board) else {
                return;
            };
            let Some(current) = node.get_pin_by_name("value_ref").cloned() else {
                return;
            };
            if current.data_type == variable.data_type
                && current.value_type == variable.value_type
                && current.schema == variable.schema
            {
                return;
            }

            let mut connected_to = current.connected_to;
            connected_to.retain(|pin_id| {
                board.get_pin_by_id(pin_id).is_some_and(|pin| {
                    pin.data_type == variable.data_type
                        && pin.value_type == variable.value_type
                        && (variable.schema.is_none()
                            || pin.schema.is_none()
                            || pin.schema == variable.schema)
                })
            });
            let output = node
                .get_pin_mut_by_name("value_ref")
                .expect("test variable getter output");
            output.data_type = variable.data_type;
            output.value_type = variable.value_type;
            output.schema = variable.schema;
            output.connected_to = connected_to;
        }
    }

    struct TestVariableSetLogic;

    #[flow_like_types::async_trait]
    impl NodeLogic for TestVariableSetLogic {
        fn get_node(&self) -> Node {
            let mut node = Node::new("variable_set", "Set Variable", "", "test");
            node.add_input_pin("var_ref", "Variable Reference", "", VariableType::String);
            node.add_input_pin("value_in", "Value", "", VariableType::Generic);
            node
        }

        async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }

        async fn on_update(&self, node: &mut Node, board: &Board) {
            let Some(variable) = test_referenced_variable(node, board) else {
                return;
            };
            let Some(current) = node.get_pin_by_name("value_in").cloned() else {
                return;
            };
            if current.data_type == variable.data_type
                && current.value_type == variable.value_type
                && current.schema == variable.schema
            {
                return;
            }

            let mut depends_on = current.depends_on;
            depends_on.retain(|pin_id| {
                board.get_pin_by_id(pin_id).is_some_and(|pin| {
                    pin.data_type == variable.data_type
                        && pin.value_type == variable.value_type
                        && (variable.schema.is_none()
                            || pin.schema.is_none()
                            || pin.schema == variable.schema)
                })
            });
            let input = node
                .get_pin_mut_by_name("value_in")
                .expect("test variable setter input");
            input.data_type = variable.data_type;
            input.value_type = variable.value_type;
            input.schema = variable.schema;
            input.depends_on = depends_on;
        }
    }

    fn test_referenced_variable(node: &Node, board: &Board) -> Option<Variable> {
        let variable_id = node
            .get_pin_by_name("var_ref")?
            .default_value
            .as_deref()
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(bytes).ok())?;
        board.get_any_variable(&variable_id)
    }

    fn set_test_variable_reference(node: &mut Node, variable_id: &str) {
        let encoded =
            flow_like_types::json::to_vec(variable_id).expect("variable reference serializes");
        node.get_pin_mut_by_name("var_ref")
            .expect("test variable reference input")
            .default_value = Some(encoded);
    }

    fn update_variable_data_type(variable_id: &str, data_type: &str) -> BoardCommand {
        BoardCommand::UpdateVariable {
            variable_id: variable_id.to_string(),
            name: None,
            data_type: Some(data_type.to_string()),
            value_type: None,
            default_value: None,
            clear_default_value: false,
            description: None,
            clear_description: false,
            category: None,
            clear_category: false,
            schema: None,
            clear_schema: false,
            exposed: None,
            secret: None,
            editable: None,
            runtime_configured: None,
            value: None,
            summary: None,
        }
    }

    #[test]
    fn any_reconcile_diagnostic_prevents_application() {
        let commands = vec![BoardCommand::AddNode {
            node_type: "log".to_string(),
            ref_id: Some("$0".to_string()),
            position: None,
            friendly_name: None,
            additional_pins: None,
            target_layer: None,
            summary: None,
        }];

        assert!(reconcile_is_safe_to_apply(&commands, &[]));
        assert!(!reconcile_is_safe_to_apply(
            &commands,
            &["even an unfamiliar diagnostic is atomic".to_string()]
        ));
        assert!(!reconcile_is_safe_to_apply(&[], &[]));
    }

    #[tokio::test]
    async fn exact_board_command_batch_uses_atomic_apply_planner_without_reconcile() {
        let mut board = empty_board();
        let mut log = Node::new("log", "Log", "", "debug");
        log.add_input_pin("message", "Message", "", VariableType::String);
        let catalog = vec![log];
        let board_commands = vec![
            BoardCommand::AddNode {
                node_type: "log".to_string(),
                ref_id: Some("$exact".to_string()),
                position: None,
                friendly_name: Some("Exact retained node".to_string()),
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::UpdateNodePin {
                node_id: "$exact".to_string(),
                pin_id: "message".to_string(),
                value: json!("retained value"),
                summary: None,
            },
        ];
        let expected = serde_json::to_value(&board_commands).unwrap();
        let state = Arc::new(crate::state::FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));

        let result =
            apply_board_commands_to_board(&mut board, board_commands, &catalog, state, None)
                .await
                .expect("exact retained command batch applies");

        assert!(result.diagnostics.is_empty());
        assert!(!result.commands.is_empty());
        assert_eq!(
            serde_json::to_value(&result.board_commands).unwrap(),
            expected
        );
        let node = board.nodes.values().next().expect("applied node");
        assert_eq!(node.friendly_name, "Exact retained node");
        let message = node
            .pins
            .values()
            .find(|pin| pin.name == "message")
            .expect("message pin");
        assert_eq!(decode_default(message), json!("retained value"));
    }

    #[tokio::test]
    async fn variable_update_refreshes_dynamic_pin_contracts_before_reconnecting_edges() {
        let mut board = empty_board();
        let mut variable = Variable::new("ticket", VariableType::String, ValueType::Normal);
        variable.id = "ticket-variable".to_string();
        board.variables.insert(variable.id.clone(), variable);

        let mut getter = TestVariableGetLogic.get_node();
        getter.id = "getter".to_string();
        set_test_variable_reference(&mut getter, "ticket-variable");
        getter
            .get_pin_mut_by_name("value_ref")
            .expect("getter output")
            .data_type = VariableType::String;
        let getter_pin_id = getter
            .get_pin_by_name("value_ref")
            .expect("getter output")
            .id
            .clone();

        let mut setter = TestVariableSetLogic.get_node();
        setter.id = "setter".to_string();
        set_test_variable_reference(&mut setter, "ticket-variable");
        setter
            .get_pin_mut_by_name("value_in")
            .expect("setter input")
            .data_type = VariableType::String;
        let setter_pin_id = setter
            .get_pin_by_name("value_in")
            .expect("setter input")
            .id
            .clone();
        getter
            .pins
            .get_mut(&getter_pin_id)
            .expect("getter output")
            .connected_to
            .insert(setter_pin_id.clone());
        setter
            .pins
            .get_mut(&setter_pin_id)
            .expect("setter input")
            .depends_on
            .insert(getter_pin_id.clone());
        board.nodes.insert(getter.id.clone(), getter);
        board.nodes.insert(setter.id.clone(), setter);

        let commands = vec![
            update_variable_data_type("ticket-variable", "Date"),
            BoardCommand::ConnectPins {
                from_node: "getter".to_string(),
                from_pin: "value_ref".to_string(),
                to_node: "setter".to_string(),
                to_pin: "value_in".to_string(),
                summary: None,
            },
        ];

        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("variable update is planned in setup");
        assert!(matches!(
            setup.as_slice(),
            [GenericCommand::UpsertVariable(_)]
        ));
        let remaining = planner
            .build_remaining_commands(&board, &commands)
            .expect("only the edge remains after setup planning");
        assert!(matches!(
            remaining.as_slice(),
            [GenericCommand::ConnectPin(_)]
        ));

        let state = Arc::new(crate::state::FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        {
            let registry = state.node_registry();
            let mut registry = registry.write().await;
            registry.push_node(Arc::new(TestVariableGetLogic));
            registry.push_node(Arc::new(TestVariableSetLogic));
        }

        let result = apply_board_commands_to_board(&mut board, commands, &[], state, None)
            .await
            .expect("variable update and reconnect apply");
        assert!(result.diagnostics.is_empty());
        assert_eq!(
            board.variables["ticket-variable"].data_type,
            VariableType::Date
        );
        let getter_pin = &board.nodes["getter"].pins[&getter_pin_id];
        let setter_pin = &board.nodes["setter"].pins[&setter_pin_id];
        assert_eq!(getter_pin.data_type, VariableType::Date);
        assert_eq!(setter_pin.data_type, VariableType::Date);
        assert!(getter_pin.connected_to.contains(&setter_pin_id));
        assert!(setter_pin.depends_on.contains(&getter_pin_id));
    }

    #[test]
    fn positional_pin_refs_resolve_duplicate_names_by_stable_index() {
        let mut node = Node::new("equal_string", "Equal String", "", "logic");
        let first_id = node
            .add_input_pin("string", "String", "", VariableType::String)
            .id
            .clone();
        let second_id = node
            .add_input_pin("string", "String", "", VariableType::String)
            .id
            .clone();

        assert_eq!(
            resolve_pin_id_in_node(&node, "string[#1]", Some(PinType::Input)).unwrap(),
            first_id
        );
        assert_eq!(
            resolve_pin_id_in_node(&node, "string[#2]", Some(PinType::Input)).unwrap(),
            second_id
        );

        // Setup may configure one input before a later connection is resolved. Population state
        // must not reorder the occurrence selector.
        node.pins
            .get_mut(&second_id)
            .expect("second input")
            .default_value = Some(b"\"configured\"".to_vec());
        assert_eq!(
            resolve_pin_id_in_node(&node, "string[#1]", Some(PinType::Input)).unwrap(),
            first_id
        );
        assert_eq!(
            resolve_pin_id_in_node(&node, "string[#2]", Some(PinType::Input)).unwrap(),
            second_id
        );
    }

    #[test]
    fn event_last_explicit_refs_do_not_collide_with_positional_aliases() {
        let board = empty_board();
        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        let mut log = Node::new("log", "Log", "", "debug");
        log.add_input_pin("exec_in", "Exec In", "", VariableType::Execution);
        let catalog = vec![event, log];
        let mut planner = FlowScriptApplyPlanner::new(&board, &catalog, None);
        let commands = vec![
            // Event-last setup order is intentionally the opposite of ref-allocation order.
            BoardCommand::AddNode {
                node_type: "log".to_string(),
                ref_id: Some("$1".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::AddNode {
                node_type: "events_simple".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::ConnectPins {
                from_node: "$0".to_string(),
                from_pin: "exec_out".to_string(),
                to_node: "$1".to_string(),
                to_pin: "exec_in".to_string(),
                summary: None,
            },
        ];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("out-of-order explicit refs must remain unambiguous");
        assert!(!planner.ambiguous_node_refs.contains("$0"));
        assert!(!planner.ambiguous_node_refs.contains("$1"));

        let mut staged_board = board;
        for command in setup {
            if let GenericCommand::AddNode(command) = command {
                staged_board
                    .nodes
                    .insert(command.node.id.clone(), command.node);
            }
        }
        let remaining = planner
            .build_remaining_commands(&staged_board, &commands)
            .expect("event-last refs must resolve when connections are built");
        assert_eq!(
            remaining
                .iter()
                .filter(|command| matches!(command, GenericCommand::ConnectPin(_)))
                .count(),
            1
        );
    }

    #[test]
    fn layered_add_followed_by_pin_update_retains_function_layer() {
        let board = empty_board();
        let mut log = Node::new("log", "Log", "", "debug");
        log.add_input_pin("message", "Message", "", VariableType::String);
        let catalog = vec![log];
        let mut planner = FlowScriptApplyPlanner::new(&board, &catalog, None);
        let commands = vec![
            BoardCommand::CreateLayer {
                name: "configuredHelper".to_string(),
                ref_id: Some("$0".to_string()),
                layer_type: Some("Function".to_string()),
                node_ids: Vec::new(),
                pins: None,
                position: None,
                color: None,
                target_layer: None,
                cache: None,
                summary: None,
            },
            BoardCommand::AddNode {
                node_type: "log".to_string(),
                ref_id: Some("$1".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: Some("$0".to_string()),
                summary: None,
            },
            BoardCommand::UpdateNodePin {
                node_id: "$1".to_string(),
                pin_id: "message".to_string(),
                value: flow_like_types::Value::String("configured".to_string()),
                summary: None,
            },
        ];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("layered configured node should plan");
        let layer_id = planner
            .node_refs
            .get("$0")
            .cloned()
            .expect("function layer ref");

        let added = setup.iter().find_map(|command| match command {
            GenericCommand::AddNode(command) => Some(&command.node),
            _ => None,
        });
        let updated = setup.iter().find_map(|command| match command {
            GenericCommand::UpdateNode(command) => Some(&command.node),
            _ => None,
        });
        assert_eq!(
            added.and_then(|node| node.layer.as_deref()),
            Some(layer_id.as_str())
        );
        assert_eq!(
            updated.and_then(|node| node.layer.as_deref()),
            Some(layer_id.as_str()),
            "pin-update staging must not clear function-layer membership"
        );
    }

    #[test]
    fn setup_applies_cache_to_new_function_layer() {
        let board = empty_board();
        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let cache = LayerCache {
            enabled: true,
            prefix: "pricing".to_string(),
            ttl_seconds: Some(300),
            scope: LayerCacheScope::User,
        };
        let commands = vec![BoardCommand::CreateLayer {
            name: "cachedLookup".to_string(),
            ref_id: Some("$0".to_string()),
            layer_type: Some("Function".to_string()),
            node_ids: Vec::new(),
            pins: Some(Vec::new()),
            position: None,
            color: None,
            target_layer: None,
            cache: Some(cache.clone()),
            summary: None,
        }];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("cached layer should plan");
        let [GenericCommand::UpsertLayer(command)] = setup.as_slice() else {
            panic!("expected one layer upsert, got {} commands", setup.len());
        };
        assert_eq!(command.layer.cache.as_ref(), Some(&cache));
    }

    #[test]
    fn setup_can_update_a_function_layer_created_earlier_in_the_batch() {
        let mut board = empty_board();
        let parent = Layer::new(
            "parent-layer".to_string(),
            "Functions".to_string(),
            LayerType::Collapsed,
        );
        board.layers.insert(parent.id.clone(), parent);
        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let cache = LayerCache {
            enabled: true,
            prefix: "pricing".to_string(),
            ttl_seconds: Some(300),
            scope: LayerCacheScope::User,
        };
        let commands = vec![
            BoardCommand::CreateLayer {
                name: "cachedLookup".to_string(),
                ref_id: Some("$0".to_string()),
                layer_type: Some("Function".to_string()),
                node_ids: Vec::new(),
                pins: Some(Vec::new()),
                position: None,
                color: None,
                target_layer: Some("parent-layer".to_string()),
                cache: None,
                summary: None,
            },
            BoardCommand::UpdateLayerCache {
                layer_id: "$0".to_string(),
                cache: Some(cache.clone()),
                summary: None,
            },
        ];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("a newly created layer should accept a same-batch cache update");
        let [
            GenericCommand::UpsertLayer(created),
            GenericCommand::UpsertLayer(updated),
        ] = setup.as_slice()
        else {
            panic!("expected two layer upserts, got {} commands", setup.len());
        };
        assert_eq!(updated.layer.id, created.layer.id);
        assert_eq!(updated.layer.cache.as_ref(), Some(&cache));
        assert_eq!(updated.current_layer.as_deref(), Some("parent-layer"));
    }

    #[test]
    fn setup_updates_existing_layer_cache_without_changing_parent() {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "cached-function".to_string(),
            "Cached Lookup".to_string(),
            LayerType::Function,
        );
        layer.parent_id = Some("parent-layer".to_string());
        layer.cache = Some(LayerCache {
            enabled: true,
            prefix: "old".to_string(),
            ttl_seconds: Some(60),
            scope: LayerCacheScope::App,
        });
        board.layers.insert(layer.id.clone(), layer);
        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let cache = LayerCache {
            enabled: true,
            prefix: "pricing".to_string(),
            ttl_seconds: Some(300),
            scope: LayerCacheScope::User,
        };
        let commands = vec![BoardCommand::UpdateLayerCache {
            layer_id: "cached-function".to_string(),
            cache: Some(cache.clone()),
            summary: None,
        }];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("cache update should plan");
        let [GenericCommand::UpsertLayer(command)] = setup.as_slice() else {
            panic!("expected one layer upsert, got {} commands", setup.len());
        };
        assert_eq!(command.layer.cache.as_ref(), Some(&cache));
        assert_eq!(command.current_layer.as_deref(), Some("parent-layer"));
    }

    #[test]
    fn setup_removes_existing_layer_cache() {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "cached-function".to_string(),
            "Cached Lookup".to_string(),
            LayerType::Function,
        );
        layer.cache = Some(LayerCache {
            enabled: true,
            prefix: "pricing".to_string(),
            ttl_seconds: Some(300),
            scope: LayerCacheScope::User,
        });
        board.layers.insert(layer.id.clone(), layer);
        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let commands = vec![BoardCommand::UpdateLayerCache {
            layer_id: "cached-function".to_string(),
            cache: None,
            summary: None,
        }];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("cache removal should plan");
        let [GenericCommand::UpsertLayer(command)] = setup.as_slice() else {
            panic!("expected one layer upsert, got {} commands", setup.len());
        };
        assert!(command.layer.cache.is_none());
    }

    #[test]
    fn setup_rejects_cache_updates_for_non_function_layers() {
        let mut board = empty_board();
        let layer = Layer::new(
            "group-layer".to_string(),
            "Group".to_string(),
            LayerType::Collapsed,
        );
        board.layers.insert(layer.id.clone(), layer);
        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let commands = vec![BoardCommand::UpdateLayerCache {
            layer_id: "group-layer".to_string(),
            cache: Some(LayerCache::default()),
            summary: None,
        }];

        let error = match planner.build_setup_commands(&board, &commands) {
            Ok(_) => panic!("only Function layers may carry result-cache settings"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("not a Function layer"));
    }

    #[test]
    fn setup_rejects_cached_non_function_layer_creation() {
        let board = empty_board();
        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let commands = vec![BoardCommand::CreateLayer {
            name: "Group".to_string(),
            ref_id: Some("$0".to_string()),
            layer_type: Some("Collapsed".to_string()),
            node_ids: Vec::new(),
            pins: Some(Vec::new()),
            position: None,
            color: None,
            target_layer: None,
            cache: Some(LayerCache::default()),
            summary: None,
        }];

        let error = match planner.build_setup_commands(&board, &commands) {
            Ok(_) => panic!("only Function layers may be created with cache settings"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("not a Function layer"));
    }

    #[test]
    fn referenceable_entry_resolves_canonical_flat_function_member() {
        let mut board = empty_board();
        let layer = Layer::new(
            "function-layer".to_string(),
            "fetchPage".to_string(),
            LayerType::Function,
        );
        board.layers.insert(layer.id.clone(), layer);

        let mut entry = generic_event_catalog_node();
        entry.id = "flat-entry".to_string();
        entry.layer = Some("function-layer".to_string());
        board.nodes.insert(entry.id.clone(), entry);

        let planner = FlowScriptApplyPlanner::new(&board, &[], None);
        assert_eq!(
            planner
                .referenceable_entry_in_layer(&board, "function-layer")
                .expect("flat entry lookup succeeds"),
            Some("flat-entry".to_string())
        );
    }

    #[test]
    fn referenceable_entry_rejects_ambiguous_function_layer() {
        let mut board = empty_board();
        let layer = Layer::new(
            "function-layer".to_string(),
            "toolScope".to_string(),
            LayerType::Function,
        );
        board.layers.insert(layer.id.clone(), layer);

        for id in ["first-entry", "second-entry"] {
            let mut entry = generic_event_catalog_node();
            entry.id = id.to_string();
            entry.layer = Some("function-layer".to_string());
            board.nodes.insert(entry.id.clone(), entry);
        }

        let planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let error = planner
            .referenceable_entry_in_layer(&board, "function-layer")
            .expect_err("ambiguous layer must not choose a random HashMap entry");
        let message = error.to_string();
        assert!(message.contains("multiple referenceable"));
        assert!(message.contains("first-entry, second-entry"));
    }

    #[test]
    fn named_flat_handler_resolves_to_concrete_function_reference() {
        let board = empty_board();
        let catalog = vec![
            generic_event_catalog_node(),
            function_ref_consumer_catalog_node(),
        ];
        let mut planner = FlowScriptApplyPlanner::new(&board, &catalog, None);
        let commands = vec![
            BoardCommand::CreateLayer {
                name: "toolScope".to_string(),
                ref_id: Some("$0".to_string()),
                layer_type: Some("Function".to_string()),
                node_ids: Vec::new(),
                pins: None,
                position: None,
                color: None,
                target_layer: None,
                cache: None,
                summary: None,
            },
            BoardCommand::AddNode {
                node_type: "events_generic".to_string(),
                ref_id: Some("$1".to_string()),
                position: None,
                friendly_name: Some("fetchPage".to_string()),
                additional_pins: None,
                target_layer: Some("$0".to_string()),
                summary: None,
            },
            BoardCommand::AddNode {
                node_type: "function_ref_consumer".to_string(),
                ref_id: Some("$2".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::SetNodeFunctionRefs {
                node_id: "$2".to_string(),
                // Both names resolve to the same concrete entry and must de-duplicate.
                fn_refs: vec!["fetchPage".to_string(), "$0".to_string()],
                summary: None,
            },
        ];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("function and named handler setup plans");
        let handler_id = planner
            .node_refs
            .get("fetchPage")
            .cloned()
            .expect("friendly handler name is a same-batch alias");

        let mut staged_board = board;
        for command in setup {
            match command {
                GenericCommand::UpsertLayer(command) => {
                    staged_board
                        .layers
                        .insert(command.layer.id.clone(), command.layer);
                }
                GenericCommand::AddNode(command) => {
                    staged_board
                        .nodes
                        .insert(command.node.id.clone(), command.node);
                }
                _ => {}
            }
        }

        let remaining = planner
            .build_remaining_commands(&staged_board, &commands)
            .expect("flat handler function reference resolves");
        let updated = remaining
            .iter()
            .find_map(|command| match command {
                GenericCommand::UpdateNode(command)
                    if command.node.name == "function_ref_consumer" =>
                {
                    Some(&command.node)
                }
                _ => None,
            })
            .expect("function-reference update exists");
        let refs = updated.fn_refs.as_ref().expect("consumer has fn refs");
        assert_eq!(refs.fn_refs, vec![handler_id]);

        let mut validated = refs.clone();
        assert!(
            !crate::flow::board::commands::nodes::validate_and_deduplicate_fn_refs(
                &mut validated,
                &staged_board,
            ),
            "the concrete flat entry survives command-time fn-ref validation"
        );
    }

    #[test]
    fn unresolved_function_reference_rejects_remaining_plan() {
        let board = empty_board();
        let catalog = vec![function_ref_consumer_catalog_node()];
        let mut planner = FlowScriptApplyPlanner::new(&board, &catalog, None);
        let commands = vec![
            BoardCommand::AddNode {
                node_type: "function_ref_consumer".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::SetNodeFunctionRefs {
                node_id: "$0".to_string(),
                fn_refs: vec!["missingTool".to_string()],
                summary: None,
            },
        ];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("consumer setup plans");
        let mut staged_board = board;
        for command in setup {
            if let GenericCommand::AddNode(command) = command {
                staged_board
                    .nodes
                    .insert(command.node.id.clone(), command.node);
            }
        }

        let error = match planner.build_remaining_commands(&staged_board, &commands) {
            Ok(_) => panic!("an authored but missing tool must reject the atomic edit"),
            Err(error) => error,
        };
        assert!(
            error
                .to_string()
                .contains("Could not resolve requested function reference `missingTool`")
        );
    }

    #[test]
    fn existing_named_flat_handler_is_a_function_reference_alias() {
        let mut board = empty_board();
        let mut handler = generic_event_catalog_node();
        handler.id = "existing-handler".to_string();
        handler.friendly_name = "fetchPage".to_string();
        board.nodes.insert(handler.id.clone(), handler);

        let catalog = vec![function_ref_consumer_catalog_node()];
        let mut planner = FlowScriptApplyPlanner::new(&board, &catalog, None);
        let commands = vec![
            BoardCommand::AddNode {
                node_type: "function_ref_consumer".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::SetNodeFunctionRefs {
                node_id: "$0".to_string(),
                fn_refs: vec!["fetchPage".to_string()],
                summary: None,
            },
        ];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("consumer setup plans");
        let mut staged_board = board;
        for command in setup {
            if let GenericCommand::AddNode(command) = command {
                staged_board
                    .nodes
                    .insert(command.node.id.clone(), command.node);
            }
        }
        let remaining = planner
            .build_remaining_commands(&staged_board, &commands)
            .expect("existing friendly handler name resolves");
        let refs = remaining
            .iter()
            .find_map(|command| match command {
                GenericCommand::UpdateNode(command)
                    if command.node.name == "function_ref_consumer" =>
                {
                    command.node.fn_refs.as_ref()
                }
                _ => None,
            })
            .expect("function refs are applied");
        assert_eq!(refs.fn_refs, vec!["existing-handler".to_string()]);
    }

    #[test]
    fn function_postcondition_rejects_nodes_whose_layer_was_cleared() {
        let board = empty_board();
        let log = Node::new("log", "Log", "", "debug");
        let catalog = vec![log];
        let mut planner = FlowScriptApplyPlanner::new(&board, &catalog, None);
        let commands = vec![
            BoardCommand::CreateLayer {
                name: "brokenHelper".to_string(),
                ref_id: Some("$0".to_string()),
                layer_type: Some("Function".to_string()),
                node_ids: Vec::new(),
                pins: None,
                position: None,
                color: None,
                target_layer: None,
                cache: None,
                summary: None,
            },
            BoardCommand::AddNode {
                node_type: "log".to_string(),
                ref_id: Some("$1".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: Some("$0".to_string()),
                summary: None,
            },
        ];
        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("setup commands");
        let mut applied_board = board;
        for command in setup {
            match command {
                GenericCommand::UpsertLayer(command) => {
                    applied_board
                        .layers
                        .insert(command.layer.id.clone(), command.layer);
                }
                GenericCommand::AddNode(command) => {
                    let mut node = command.node;
                    node.layer = None; // Reproduce the former staged UpdateNode corruption.
                    applied_board.nodes.insert(node.id.clone(), node);
                }
                _ => {}
            }
        }

        let error = planner
            .validate_new_function_layers(&applied_board, &commands)
            .expect_err("runtime-empty applied Function must be rejected");
        assert!(error.to_string().contains("no canonical body nodes"));
    }

    #[tokio::test]
    async fn applying_function_keeps_body_nodes_and_exec_boundary_connections() {
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "Simple Event", "", "events");
        event.set_start(true);
        event.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);

        let mut call = Node::new("control_call_function", "Call Function", "", "control");
        call.add_input_pin("exec_in", "Exec In", "", VariableType::Execution);
        call.add_input_pin(
            "function_layer_id",
            "Function Layer",
            "",
            VariableType::String,
        );
        call.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);

        let mut log = Node::new("log", "Log", "", "debug");
        log.add_input_pin("exec_in", "Exec In", "", VariableType::Execution);
        log.add_input_pin("message", "Message", "", VariableType::String);
        log.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        let catalog = vec![event, call, log];

        let state = Arc::new(crate::state::FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let result = apply_flowscript_to_board(
            &mut board,
            r#"function configuredHelper() {
    log({ message: "configured" })
}

eventsSimple() {
    configuredHelper()
}
"#,
            &catalog,
            state,
            None,
            false,
        )
        .await
        .expect("function FlowScript should apply");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(!result.commands.is_empty());
        let layer = board
            .layers
            .values()
            .find(|layer| layer.name == "configuredHelper")
            .expect("configuredHelper Function layer");
        let body_nodes = board
            .nodes
            .values()
            .filter(|node| node.layer.as_deref() == Some(layer.id.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(body_nodes.len(), 1, "function body nodes: {body_nodes:?}");
        let body = body_nodes[0];
        assert_eq!(body.name, "log");

        let layer_exec_in = layer
            .pins
            .values()
            .find(|pin| pin.name == "exec_in")
            .expect("Function exec_in boundary");
        let layer_exec_out = layer
            .pins
            .values()
            .find(|pin| pin.name == "exec_out")
            .expect("Function exec_out boundary");
        let body_exec_in = body
            .pins
            .values()
            .find(|pin| pin.name == "exec_in")
            .expect("body exec_in");
        let body_exec_out = body
            .pins
            .values()
            .find(|pin| pin.name == "exec_out")
            .expect("body exec_out");

        assert!(layer_exec_in.connected_to.contains(&body_exec_in.id));
        assert!(body_exec_in.depends_on.contains(&layer_exec_in.id));
        assert!(body_exec_out.connected_to.contains(&layer_exec_out.id));
        assert!(layer_exec_out.depends_on.contains(&body_exec_out.id));
    }

    #[tokio::test]
    async fn applying_multiple_events_persists_every_entry_and_its_wiring() {
        let mut board = empty_board();

        let mut simple = Node::new("events_simple", "Simple Event", "", "events");
        simple.set_start(true);
        simple.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);

        let mut generic = Node::new("events_generic", "Generic Event", "", "events");
        generic.set_start(true);
        generic.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        generic.add_output_pin("payload", "Payload", "", VariableType::Struct);

        let mut chat = Node::new("events_chat", "Chat Event", "", "events");
        chat.set_start(true);
        chat.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        chat.add_output_pin("history", "History", "", VariableType::Struct);

        let mut log = Node::new("log", "Log", "", "debug");
        log.add_input_pin("exec_in", "Exec In", "", VariableType::Execution);
        log.add_input_pin("message", "Message", "", VariableType::String);
        log.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);

        let catalog = vec![simple, generic, chat, log];
        let state = Arc::new(crate::state::FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let result = apply_flowscript_to_board(
            &mut board,
            r#"eventsSimple() {
    log({ message: "simple one" })
}

eventsSimple() {
    log({ message: "simple two" })
}

eventsGeneric(payload: Struct, ticketId: string) {
    log({ message: ticketId })
}

eventsChat() {
    log({ message: "chat" })
}
"#,
            &catalog,
            state,
            None,
            false,
        )
        .await
        .expect("multiple Event FlowScript should apply atomically");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(!result.commands.is_empty());
        assert_eq!(board.nodes.len(), 8, "four entries plus four body nodes");

        let event_nodes = board
            .nodes
            .values()
            .filter(|node| {
                matches!(
                    node.name.as_str(),
                    "events_simple" | "events_generic" | "events_chat"
                )
            })
            .collect::<Vec<_>>();
        let event_positions = event_nodes
            .iter()
            .map(|node| {
                let (x, y, _) = node.coordinates.expect("Event placement coordinates");
                (x.round() as i32, y.round() as i32)
            })
            .collect::<HashSet<_>>();
        assert_eq!(
            event_positions.len(),
            4,
            "multiple Event entries must not overlap"
        );

        let mut event_counts: HashMap<&str, usize> = HashMap::new();
        for event in event_nodes {
            *event_counts.entry(event.name.as_str()).or_default() += 1;
            assert!(event.layer.is_none(), "top-level Event must remain at root");

            let exec_out = event
                .pins
                .values()
                .find(|pin| pin.name == "exec_out")
                .expect("Event execution output");
            assert_eq!(
                exec_out.connected_to.len(),
                1,
                "every Event must retain its own executable body connection"
            );
            let target_pin_id = exec_out.connected_to.iter().next().unwrap();
            let target = board
                .nodes
                .values()
                .find(|node| node.pins.contains_key(target_pin_id))
                .expect("Event body target persists on the board");
            assert_eq!(target.name, "log");
        }
        assert_eq!(event_counts.get("events_simple"), Some(&2));
        assert_eq!(event_counts.get("events_generic"), Some(&1));
        assert_eq!(event_counts.get("events_chat"), Some(&1));

        let generic_event = board
            .nodes
            .values()
            .find(|node| node.name == "events_generic")
            .expect("Generic Event persists");
        let ticket_id = generic_event
            .pins
            .values()
            .find(|pin| pin.name == "ticketId")
            .expect("Generic Event custom output persists");
        assert_eq!(ticket_id.connected_to.len(), 1);
        let message_pin = board
            .nodes
            .values()
            .flat_map(|node| node.pins.values())
            .find(|pin| ticket_id.connected_to.contains(&pin.id))
            .expect("Generic Event payload field target persists");
        assert_eq!(message_pin.name, "message");
        assert!(message_pin.depends_on.contains(&ticket_id.id));

        let lowered = super::super::lower_to_ast(&board);
        assert_eq!(lowered.events.len(), 4, "all persisted Events lower back");
        let mut lowered_counts: HashMap<&str, usize> = HashMap::new();
        for event in &lowered.events {
            *lowered_counts.entry(event.node_type.as_str()).or_default() += 1;
            assert!(
                !event.body.stmts.is_empty(),
                "no Event may lower as an empty handler"
            );
        }
        assert_eq!(lowered_counts.get("events_simple"), Some(&2));
        assert_eq!(lowered_counts.get("events_generic"), Some(&1));
        assert_eq!(lowered_counts.get("events_chat"), Some(&1));
    }

    #[tokio::test]
    async fn function_ref_update_executes_before_new_pin_connections() {
        let mut board = empty_board();
        let mut source = Node::new("source", "Source", "", "test");
        source.id = "source".to_string();
        let source_out = source
            .add_output_pin("exec_out", "Exec Out", "", VariableType::Execution)
            .id
            .clone();
        let mut sink = Node::new("sink", "Sink", "", "test");
        sink.id = "sink".to_string();
        let sink_in = sink
            .add_input_pin("exec_in", "Exec In", "", VariableType::Execution)
            .id
            .clone();
        let mut function_target = Node::new("target", "Target", "", "test");
        function_target.id = "function_target".to_string();
        function_target.fn_refs = Some(FnRefs {
            fn_refs: Vec::new(),
            can_reference_fns: false,
            can_be_referenced_by_fns: true,
        });
        board.nodes.insert(source.id.clone(), source);
        board.nodes.insert(sink.id.clone(), sink);
        board
            .nodes
            .insert(function_target.id.clone(), function_target);

        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let commands = vec![
            BoardCommand::ConnectPins {
                from_node: "source".to_string(),
                from_pin: "exec_out".to_string(),
                to_node: "sink".to_string(),
                to_pin: "exec_in".to_string(),
                summary: None,
            },
            BoardCommand::SetNodeFunctionRefs {
                node_id: "source".to_string(),
                fn_refs: vec!["function_target".to_string()],
                summary: None,
            },
        ];
        let remaining = planner
            .build_remaining_commands(&board, &commands)
            .expect("remaining commands");
        let update_index = remaining
            .iter()
            .position(|command| matches!(command, GenericCommand::UpdateNode(_)))
            .expect("function-ref UpdateNode");
        let connect_index = remaining
            .iter()
            .position(|command| matches!(command, GenericCommand::ConnectPin(_)))
            .expect("ConnectPin");
        assert!(update_index < connect_index);

        let state = Arc::new(crate::state::FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        board
            .execute_commands(remaining, state)
            .await
            .expect("remaining commands execute");
        let source_pin = board.nodes["source"]
            .pins
            .get(&source_out)
            .expect("source pin");
        let sink_pin = board.nodes["sink"].pins.get(&sink_in).expect("sink pin");
        assert!(source_pin.connected_to.contains(&sink_in));
        assert!(sink_pin.depends_on.contains(&source_out));
    }

    #[test]
    fn deferred_pin_write_and_function_refs_compose_into_one_node_update() {
        let mut board = empty_board();
        let mut target = Node::new("target", "Target", "", "test");
        target.id = "function_target".to_string();
        target.set_can_be_referenced_by_fns(true);
        board.nodes.insert(target.id.clone(), target);

        let mut consumer = Node::new("dynamic_consumer", "Dynamic Consumer", "", "test");
        consumer.set_can_reference_fns(true);
        let catalog = vec![consumer];
        let mut planner = FlowScriptApplyPlanner::new(&board, &catalog, None);
        let commands = vec![
            BoardCommand::AddNode {
                node_type: "dynamic_consumer".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            // `late_value` represents a pin minted by the node's setup-time on_update.
            BoardCommand::UpdateNodePin {
                node_id: "$0".to_string(),
                pin_id: "late_value".to_string(),
                value: json!("preserved"),
                summary: None,
            },
            BoardCommand::SetNodeFunctionRefs {
                node_id: "$0".to_string(),
                fn_refs: vec!["function_target".to_string()],
                summary: None,
            },
        ];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("setup plans and defers the dynamic pin write");
        assert_eq!(planner.deferred_pin_updates.len(), 1);
        let mut staged_board = board;
        for command in setup {
            if let GenericCommand::AddNode(command) = command {
                staged_board
                    .nodes
                    .insert(command.node.id.clone(), command.node);
            }
        }
        let consumer_id = planner.node_refs["$0"].clone();
        staged_board
            .nodes
            .get_mut(&consumer_id)
            .expect("new consumer")
            .add_input_pin("late_value", "Late Value", "", VariableType::String);

        let remaining = planner
            .build_remaining_commands(&staged_board, &commands)
            .expect("deferred pin and function refs compose");
        let updates = remaining
            .iter()
            .filter_map(|command| match command {
                GenericCommand::UpdateNode(command) if command.node.id == consumer_id => {
                    Some(&command.node)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 1, "same-node replacements must be composed");
        let updated = updates[0];
        let late_pin = updated
            .pins
            .values()
            .find(|pin| pin.name == "late_value")
            .expect("dynamic pin survives function-ref update");
        assert_eq!(decode_default(late_pin), json!("preserved"));
        assert_eq!(
            updated.fn_refs.as_ref().expect("function refs").fn_refs,
            vec!["function_target".to_string()]
        );
    }

    #[test]
    fn multiple_function_ref_commands_merge_into_one_node_update() {
        let mut board = empty_board();
        let mut consumer = function_ref_consumer_catalog_node();
        consumer.id = "consumer".to_string();
        board.nodes.insert(consumer.id.clone(), consumer);
        for id in ["target_a", "target_b"] {
            let mut target = Node::new(id, id, "", "test");
            target.id = id.to_string();
            target.set_can_be_referenced_by_fns(true);
            board.nodes.insert(target.id.clone(), target);
        }

        let commands = vec![
            BoardCommand::SetNodeFunctionRefs {
                node_id: "consumer".to_string(),
                fn_refs: vec!["target_a".to_string()],
                summary: None,
            },
            BoardCommand::SetNodeFunctionRefs {
                node_id: "consumer".to_string(),
                fn_refs: vec!["target_b".to_string(), "target_a".to_string()],
                summary: None,
            },
        ];
        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let remaining = planner
            .build_remaining_commands(&board, &commands)
            .expect("function references merge");
        let updates = remaining
            .iter()
            .filter_map(|command| match command {
                GenericCommand::UpdateNode(command) => Some(&command.node),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(updates.len(), 1);
        assert_eq!(
            updates[0].fn_refs.as_ref().expect("function refs").fn_refs,
            vec!["target_a".to_string(), "target_b".to_string()]
        );
    }

    #[tokio::test]
    async fn composed_node_updates_execute_before_moves_and_removals() {
        let mut board = empty_board();
        let mut target = Node::new("target", "Target", "", "test");
        target.id = "function_target".to_string();
        target.set_can_be_referenced_by_fns(true);
        board.nodes.insert(target.id.clone(), target);
        for id in ["moved", "removed"] {
            let mut consumer = function_ref_consumer_catalog_node();
            consumer.id = id.to_string();
            consumer.coordinates = Some((0.0, 0.0, 0.0));
            board.nodes.insert(consumer.id.clone(), consumer);
        }

        let commands = vec![
            BoardCommand::MoveNode {
                node_id: "moved".to_string(),
                position: NodePosition { x: 420.0, y: 240.0 },
                target_layer: None,
                summary: None,
            },
            BoardCommand::SetNodeFunctionRefs {
                node_id: "moved".to_string(),
                fn_refs: vec!["function_target".to_string()],
                summary: None,
            },
            BoardCommand::RemoveNode {
                node_id: "removed".to_string(),
                summary: None,
            },
            BoardCommand::SetNodeFunctionRefs {
                node_id: "removed".to_string(),
                fn_refs: vec!["function_target".to_string()],
                summary: None,
            },
        ];
        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);
        let remaining = planner
            .build_remaining_commands(&board, &commands)
            .expect("remaining commands");
        let last_update = remaining
            .iter()
            .rposition(|command| matches!(command, GenericCommand::UpdateNode(_)))
            .expect("composed updates");
        let first_move_or_remove = remaining
            .iter()
            .position(|command| {
                matches!(
                    command,
                    GenericCommand::MoveNode(_) | GenericCommand::RemoveNode(_)
                )
            })
            .expect("move/remove command");
        assert!(last_update < first_move_or_remove);

        let state = Arc::new(crate::state::FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        board
            .execute_commands(remaining, state)
            .await
            .expect("composed batch executes");
        assert_eq!(board.nodes["moved"].coordinates, Some((420.0, 240.0, 0.0)));
        assert_eq!(
            board.nodes["moved"]
                .fn_refs
                .as_ref()
                .expect("moved node refs")
                .fn_refs,
            vec!["function_target".to_string()]
        );
        assert!(!board.nodes.contains_key("removed"));
    }

    #[test]
    fn add_node_materializes_additional_generic_event_outputs() {
        let board = empty_board();
        let catalog = vec![generic_event_catalog_node()];
        let mut planner = FlowScriptApplyPlanner::new(&board, &catalog, None);
        let commands = vec![BoardCommand::AddNode {
            node_type: "events_generic".to_string(),
            ref_id: Some("$0".to_string()),
            position: None,
            friendly_name: None,
            additional_pins: Some(vec![
                PlaceholderPinDef {
                    name: "ticketIds".to_string(),
                    friendly_name: "ticketIds".to_string(),
                    description: None,
                    pin_type: "Output".to_string(),
                    data_type: "String".to_string(),
                    value_type: Some("Array".to_string()),
                    schema: None,
                    enforce_schema: false,
                },
                PlaceholderPinDef {
                    name: "ticket".to_string(),
                    friendly_name: "ticket".to_string(),
                    description: None,
                    pin_type: "Output".to_string(),
                    data_type: "Struct".to_string(),
                    value_type: Some("Normal".to_string()),
                    schema: Some(
                        r#"{"type":"object","properties":{"id":{"type":"string"}}}"#.to_string(),
                    ),
                    enforce_schema: true,
                },
            ]),
            target_layer: None,
            summary: None,
        }];

        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("additional event output is valid");
        let GenericCommand::AddNode(command) = &setup[0] else {
            panic!("expected AddNode");
        };
        let pin = command
            .node
            .pins
            .values()
            .find(|pin| pin.name == "ticketIds")
            .expect("custom output exists before node creation");
        assert_eq!(pin.pin_type, PinType::Output);
        assert_eq!(pin.data_type, VariableType::String);
        assert_eq!(pin.value_type, ValueType::Array);
        let typed_pin = command
            .node
            .pins
            .values()
            .find(|pin| pin.name == "ticket")
            .expect("schema-bearing custom output exists before node creation");
        assert!(typed_pin.schema.is_some());
        assert_eq!(
            typed_pin
                .options
                .as_ref()
                .and_then(|options| options.enforce_schema),
            Some(true)
        );
    }

    #[test]
    fn function_layer_pin_resolution_inverts_boundary_direction() {
        let mut board = empty_board();
        let mut layer = Layer::new(
            "function".to_string(),
            "Function".to_string(),
            LayerType::Function,
        );
        let mut template = Node::new("boundary", "Boundary", "", "test");
        let parameter = template
            .add_input_pin("value", "Value", "", VariableType::String)
            .clone();
        let returned = template
            .add_output_pin("value", "Value", "", VariableType::String)
            .clone();
        let parameter_id = parameter.id.clone();
        let return_id = returned.id.clone();
        layer.pins.insert(parameter.id.clone(), parameter);
        layer.pins.insert(returned.id.clone(), returned);
        board.layers.insert(layer.id.clone(), layer);

        let planner = FlowScriptApplyPlanner::new(&board, &[], None);
        assert_eq!(
            planner
                .resolve_pin_id(&board, "function", "value", Some(PinType::Output))
                .unwrap(),
            parameter_id,
            "a layer used as the edge source exposes its boundary Input parameter"
        );
        assert_eq!(
            planner
                .resolve_pin_id(&board, "function", "value", Some(PinType::Input))
                .unwrap(),
            return_id,
            "a layer used as the edge target exposes its boundary Output return"
        );
    }

    /// The full Part B flow without a node registry: setup defers a write to a not-yet-minted
    /// dynamic pin, `on_update` is simulated by adding the pin to the board, then the deferred write
    /// is applied in the remaining phase.
    #[test]
    fn literal_on_dynamic_pin_defers_then_applies_after_on_update() {
        let board = empty_board();
        let catalog = vec![dynamic_format_catalog_node()];
        let mut planner = FlowScriptApplyPlanner::new(&board, &catalog, None);

        let commands = vec![
            BoardCommand::AddNode {
                node_type: "dynamic_format".to_string(),
                ref_id: Some("$0".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::UpdateNodePin {
                node_id: "$0".to_string(),
                pin_id: "format_string".to_string(),
                value: json!("Hi {idx}"),
                summary: None,
            },
            BoardCommand::UpdateNodePin {
                node_id: "$0".to_string(),
                pin_id: "idx".to_string(),
                value: json!("5"),
                summary: None,
            },
        ];

        // Setup phase: the `idx` write cannot resolve yet (the pin does not exist), so it must be
        // deferred rather than aborting the apply.
        let setup = planner
            .build_setup_commands(&board, &commands)
            .expect("setup must not fail on a not-yet-minted dynamic pin");

        assert_eq!(planner.deferred_pin_updates.len(), 1);
        let (deferred_node_id, deferred_pin, deferred_value) =
            planner.deferred_pin_updates[0].clone();
        assert_eq!(deferred_pin, "idx");
        assert_eq!(
            deferred_value,
            flow_like_types::Value::String("5".to_string())
        );
        assert!(
            !setup.iter().any(|command| matches!(
                command,
                GenericCommand::UpdateNode(cmd) if cmd.node.pins.values().any(|pin| pin.name == "idx")
            )),
            "the `idx` write must NOT be emitted in the setup phase"
        );

        // Simulate `on_update`: the board now carries the `idx` placeholder pin the node minted.
        let mut board = board;
        let mut node = dynamic_format_catalog_node();
        node.id = deferred_node_id.clone();
        node.add_input_pin("idx", "idx", "", VariableType::Generic);
        board.nodes.insert(deferred_node_id.clone(), node);

        // Remaining phase: the deferred write now resolves against the live board and is applied
        // (before any connects, which this document has none of).
        let deferred = planner
            .build_remaining_commands(&board, &commands)
            .expect("deferred write resolves once the pin exists");

        assert_eq!(deferred.len(), 1, "one node → one batched UpdateNode");
        let GenericCommand::UpdateNode(cmd) = &deferred[0] else {
            panic!("expected an UpdateNode command");
        };
        assert_eq!(cmd.node.id, deferred_node_id);
        let idx_pin = cmd
            .node
            .pins
            .values()
            .find(|pin| pin.name == "idx")
            .expect("idx pin present on the updated node");
        assert_eq!(
            decode_default(idx_pin),
            flow_like_types::Value::String("5".to_string())
        );
    }

    /// Several placeholder literals on one node fold into a single whole-node `UpdateNode` (per-pin
    /// commands would each replace the node and clobber the previous write).
    #[test]
    fn multiple_deferred_pins_on_one_node_batch_into_one_update() {
        let board = empty_board();
        let mut planner = FlowScriptApplyPlanner::new(&board, &[], None);

        let mut node = dynamic_format_catalog_node();
        node.id = "fmt".to_string();
        node.add_input_pin("idx", "idx", "", VariableType::Generic);
        node.add_input_pin("total", "total", "", VariableType::Generic);
        let mut board = board;
        board.nodes.insert("fmt".to_string(), node);

        planner.deferred_pin_updates = vec![
            ("fmt".to_string(), "idx".to_string(), json!("1")),
            ("fmt".to_string(), "total".to_string(), json!("9")),
        ];

        let deferred = planner
            .build_deferred_pin_updates(&board)
            .expect("resolves");

        assert_eq!(deferred.len(), 1, "both pins collapse into one UpdateNode");
        let GenericCommand::UpdateNode(cmd) = &deferred[0] else {
            panic!("expected an UpdateNode");
        };
        let idx = cmd.node.pins.values().find(|p| p.name == "idx").unwrap();
        let total = cmd.node.pins.values().find(|p| p.name == "total").unwrap();
        assert_eq!(
            decode_default(idx),
            flow_like_types::Value::String("1".to_string())
        );
        assert_eq!(
            decode_default(total),
            flow_like_types::Value::String("9".to_string())
        );
    }

    struct TestDynamicFormatLogic;

    #[flow_like_types::async_trait]
    impl NodeLogic for TestDynamicFormatLogic {
        fn get_node(&self) -> Node {
            dynamic_format_catalog_node()
        }

        async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }

        /// Mirrors `string_format` and `control_call_function`: placeholder pins are minted with
        /// fresh ids and reconciled by NAME, so a replay that re-derives them allocates a second,
        /// different set of ids.
        async fn on_update(&self, node: &mut Node, _board: &Board) {
            let format_string = node
                .get_pin_by_name("format_string")
                .and_then(|pin| pin.default_value.clone())
                .and_then(|bytes| {
                    flow_like_types::json::from_slice::<flow_like_types::Value>(&bytes).ok()
                })
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap_or_default();

            let placeholders: Vec<String> = format_string
                .split('{')
                .skip(1)
                .filter_map(|part| part.split('}').next())
                .map(ToOwned::to_owned)
                .collect();

            let stale: Vec<String> = node
                .pins
                .values()
                .filter(|pin| {
                    pin.pin_type == PinType::Input
                        && pin.name != "format_string"
                        && !placeholders.contains(&pin.name)
                })
                .map(|pin| pin.id.clone())
                .collect();
            for id in stale {
                node.pins.remove(&id);
            }

            for placeholder in placeholders {
                if node
                    .pins
                    .values()
                    .any(|pin| pin.pin_type == PinType::Input && pin.name == placeholder)
                {
                    continue;
                }
                node.add_input_pin(&placeholder, &placeholder, "", VariableType::Generic);
            }
        }
    }

    /// The desktop ships the applied batch to the Hub, which replays it as ONE command list against
    /// a board that has never run this node's `on_update`. Before the batch carried the derived node
    /// state, the trailing `ConnectPin` referenced a placeholder pin id that existed on no other
    /// machine, so every remote apply failed with "To Pin (...) not found in container" and the
    /// desktop outbox wedged permanently.
    #[tokio::test]
    async fn applied_batch_replays_dynamic_pins_on_a_fresh_board() {
        let state = Arc::new(crate::state::FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        {
            let registry = state.node_registry();
            let mut registry = registry.write().await;
            registry.push_node(Arc::new(TestDynamicFormatLogic));
        }

        let catalog = vec![dynamic_format_catalog_node()];
        let commands = vec![
            BoardCommand::AddNode {
                node_type: "dynamic_format".to_string(),
                ref_id: Some("$source".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::AddNode {
                node_type: "dynamic_format".to_string(),
                ref_id: Some("$target".to_string()),
                position: None,
                friendly_name: None,
                additional_pins: None,
                target_layer: None,
                summary: None,
            },
            BoardCommand::UpdateNodePin {
                node_id: "$target".to_string(),
                pin_id: "format_string".to_string(),
                value: json!("Hi {idx}"),
                summary: None,
            },
            BoardCommand::ConnectPins {
                from_node: "$source".to_string(),
                from_pin: "value".to_string(),
                to_node: "$target".to_string(),
                to_pin: "idx".to_string(),
                summary: None,
            },
        ];

        let mut board = empty_board();
        let applied =
            apply_board_commands_to_board(&mut board, commands, &catalog, state.clone(), None)
                .await
                .expect("the local apply mints the placeholder pin and connects it");
        assert!(
            !applied.commands.is_empty(),
            "the apply must produce a command batch"
        );
        assert!(
            connected_placeholder(&board).is_some(),
            "the local board must carry the connected placeholder pin"
        );

        let mut replay_board = empty_board();
        replay_board
            .execute_commands(applied.commands.clone(), state)
            .await
            .expect("the applied batch must replay verbatim on a board that never ran on_update");

        assert!(
            connected_placeholder(&replay_board).is_some(),
            "the replayed board must carry the same connected placeholder pin"
        );
    }

    fn connected_placeholder(board: &Board) -> Option<&Pin> {
        board.nodes.values().find_map(|node| {
            node.pins
                .values()
                .find(|pin| pin.name == "idx" && !pin.depends_on.is_empty())
        })
    }
}
