use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Serialize;

use super::context::GraphContext;
use super::provider::CatalogProvider;
use super::tools::EmitCommandsArgs;
use super::types::{BoardCommand, PinMetadata, PlaceholderPinDef};
use crate::flow::ast::MAX_NODES_PER_LAYER;

#[derive(Debug, Clone, Serialize)]
pub struct ValidationIssue {
    pub severity: &'static str,
    pub code: &'static str,
    pub command_index: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EmitValidationOutcome {
    pub status: &'static str,
    pub validated_command_count: usize,
    pub errors: Vec<ValidationIssue>,
    pub warnings: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PinDirection {
    Input,
    Output,
}

#[derive(Debug, Clone)]
struct KnownPin {
    name: String,
    data_type: String,
    direction: PinDirection,
    has_default_value: bool,
}

#[derive(Debug, Clone)]
struct KnownEntity {
    key: String,
    display_name: String,
    is_layer: bool,
    pins: Vec<KnownPin>,
}

const MAX_EMIT_COMMANDS: usize = 20;

const EXECUTABLE_COMMAND_REQUIRES_FLOWSCRIPT: &str = "executable-command-requires-flowscript";
const VISUAL_LAYER_MEMBERSHIP_UNSAFE: &str = "visual-layer-membership-unsafe";

pub fn emit_validation_requires_flowscript(outcome: &EmitValidationOutcome) -> bool {
    outcome.errors.iter().any(|issue| {
        matches!(
            issue.code,
            EXECUTABLE_COMMAND_REQUIRES_FLOWSCRIPT | VISUAL_LAYER_MEMBERSHIP_UNSAFE
        )
    })
}

/// Validate the deliberately narrow command surface exposed to workflow-authoring models.
///
/// `BoardCommand` remains the host's complete internal transaction language. Models only get the
/// visual subset which FlowScript cannot represent; executable graph structure must pass through
/// the retained write/patch/check/commit source lifecycle instead.
pub fn validate_model_facing_emit_commands_scope(args: &EmitCommandsArgs) -> EmitValidationOutcome {
    let mut errors = Vec::new();
    if args.commands.is_empty() {
        errors.push(issue(
            "error",
            "empty-command-batch",
            None,
            "emit_commands requires at least one visual command".to_string(),
        ));
    }
    if args.commands.len() > MAX_EMIT_COMMANDS {
        errors.push(issue(
            "error",
            "too-many-commands",
            None,
            format!("emit_commands is limited to {MAX_EMIT_COMMANDS} visual commands per call"),
        ));
    }

    errors.extend(
        args.commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let command_name = match command {
                    BoardCommand::MoveNode {
                        target_layer: None,
                        ..
                    }
                    | BoardCommand::AddComment { .. }
                    | BoardCommand::RemoveComment { .. } => return None,
                    BoardCommand::CreateLayer { .. } | BoardCommand::RemoveLayer { .. } => {
                        return Some(issue(
                            "error",
                            VISUAL_LAYER_MEMBERSHIP_UNSAFE,
                            Some(index),
                            "Layer creation/removal is not accepted by model-facing emit_commands because it can reassign executable node.layer membership and the compact graph context cannot prove a purely visual change. Author Function membership in FlowScript."
                                .to_string(),
                        ));
                    }
                    BoardCommand::AddNode { .. } => "AddNode",
                    BoardCommand::AddPlaceholder { .. } => "AddPlaceholder",
                    BoardCommand::RemoveNode { .. } => "RemoveNode",
                    BoardCommand::ConnectPins { .. } => "ConnectPins",
                    BoardCommand::DisconnectPins { .. } => "DisconnectPins",
                    BoardCommand::UpdateNodePin { .. } => "UpdateNodePin",
                    BoardCommand::RenameNode { .. } => "RenameNode",
                    BoardCommand::SetNodeFunctionRefs { .. } => "SetNodeFunctionRefs",
                    BoardCommand::MoveNode { .. } => "MoveNode(target_layer)",
                    BoardCommand::CreateVariable { .. } => "CreateVariable",
                    BoardCommand::UpdateVariable { .. } => "UpdateVariable",
                    BoardCommand::RemoveVariable { .. } => "DeleteVariable",
                    BoardCommand::UpdateLayerCache { .. } => "UpdateLayerCache",
                };

                Some(issue(
                    "error",
                    EXECUTABLE_COMMAND_REQUIRES_FLOWSCRIPT,
                    Some(index),
                    format!(
                        "{command_name} changes executable workflow behavior and is not accepted by model-facing emit_commands. Author the behavior with write_flowscript, repair it with patch_flowscript, validate it with check_flowscript, then queue it with commit_flowscript."
                    ),
                ))
            }),
    );

    EmitValidationOutcome {
        status: if errors.is_empty() {
            "valid"
        } else {
            "invalid"
        },
        validated_command_count: args.commands.len(),
        errors,
        warnings: Vec::new(),
    }
}

/// Run model-facing scope validation before the complete graph-aware legacy validator. This keeps
/// the reusable host validator intact while guaranteeing that models cannot select its internal
/// executable command variants as an alternative authoring representation.
pub async fn validate_model_facing_emit_commands(
    args: &EmitCommandsArgs,
    graph_context: &GraphContext,
    provider: &dyn CatalogProvider,
) -> EmitValidationOutcome {
    let scope = validate_model_facing_emit_commands_scope(args);
    if !scope.errors.is_empty() {
        return scope;
    }

    validate_emit_commands(args, graph_context, provider).await
}

pub async fn validate_emit_commands(
    args: &EmitCommandsArgs,
    graph_context: &GraphContext,
    provider: &dyn CatalogProvider,
) -> EmitValidationOutcome {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if args.commands.is_empty() {
        errors.push(issue(
            "error",
            "empty-command-batch",
            None,
            "emit_commands requires at least one command".to_string(),
        ));
    }

    if args.commands.len() > MAX_EMIT_COMMANDS {
        errors.push(issue(
            "error",
            "too-many-commands",
            None,
            format!(
                "emit_commands is limited to {MAX_EMIT_COMMANDS} commands per call — split the batch into several emit_commands calls (nodes+connections first, then follow-up batches)"
            ),
        ));
    }

    let mut entities = build_known_entities(graph_context);
    // Layers are addressable by id AND by name (apply resolves both).
    let mut known_layer_refs: HashSet<String> = graph_context
        .layers
        .iter()
        .flat_map(|layer| [layer.id.clone(), layer.name.clone()])
        .collect();
    let mut function_layer_refs: HashSet<String> = graph_context
        .layers
        .iter()
        .filter(|layer| layer.layer_type.eq_ignore_ascii_case("function"))
        .flat_map(|layer| [layer.id.clone(), layer.name.clone()])
        .collect();
    let known_variables: HashSet<String> = graph_context
        .variables
        .iter()
        .map(|variable| variable.id.clone())
        .collect();
    let existing_connections: HashSet<(String, String, String, String)> = graph_context
        .edges
        .iter()
        .map(|edge| {
            (
                edge.from_node_id.clone(),
                edge.from_pin_name.clone(),
                edge.to_node_id.clone(),
                edge.to_pin_name.clone(),
            )
        })
        .collect();
    let mut proposed_connections = existing_connections.clone();
    let mut incoming_inputs: HashSet<(String, String)> = graph_context
        .edges
        .iter()
        .map(|edge| (edge.to_node_id.clone(), edge.to_pin_name.clone()))
        .collect();
    let mut explicit_values: HashSet<(String, String)> = HashSet::new();
    let mut entities_to_check: BTreeSet<String> = BTreeSet::new();

    // An execution OUTPUT drives exactly one target (the board replaces the previous edge on a
    // second connect). Track occupancy so a batch cannot silently rewire an existing chain.
    let mut exec_outgoing: HashMap<(String, String), (String, String)> = graph_context
        .edges
        .iter()
        .filter(|edge| {
            entities
                .get(&edge.from_node_id)
                .and_then(|entity| find_pin(entity, &edge.from_pin_name))
                .is_some_and(|pin| pin.data_type == "Execution")
        })
        .map(|edge| {
            (
                (edge.from_node_id.clone(), edge.from_pin_name.clone()),
                (edge.to_node_id.clone(), edge.to_pin_name.clone()),
            )
        })
        .collect();

    // Per-layer node population (None = root) for the MAX_NODES_PER_LAYER gate.
    let mut node_layer: HashMap<String, Option<String>> = HashMap::new();
    let mut layer_counts: HashMap<Option<String>, i64> = HashMap::new();
    let mut layer_display: HashMap<String, String> = HashMap::new();
    for layer in &graph_context.layers {
        layer_display.insert(layer.id.clone(), layer.name.clone());
        *layer_counts.entry(Some(layer.id.clone())).or_default() += layer.node_ids.len() as i64;
        for node_id in &layer.node_ids {
            node_layer.insert(node_id.clone(), Some(layer.id.clone()));
        }
    }
    for node in &graph_context.nodes {
        if !node_layer.contains_key(&node.id) {
            node_layer.insert(node.id.clone(), None);
            *layer_counts.entry(None).or_default() += 1;
        }
    }

    for (index, command) in args.commands.iter().enumerate() {
        match command {
            BoardCommand::AddNode {
                node_type,
                ref_id,
                position,
                additional_pins,
                target_layer,
                summary,
                ..
            } => {
                if summary.as_deref().unwrap_or_default().trim().is_empty() {
                    warnings.push(issue(
                        "warning",
                        "missing-summary",
                        Some(index),
                        format!("AddNode '{}' is missing a summary field", node_type),
                    ));
                }

                if ref_id.as_deref().unwrap_or_default().trim().is_empty() {
                    errors.push(issue(
                        "error",
                        "missing-ref-id",
                        Some(index),
                        format!("AddNode '{}' requires a ref_id like '$0'", node_type),
                    ));
                }

                if position.is_none() {
                    errors.push(issue(
                        "error",
                        "missing-position",
                        Some(index),
                        format!("AddNode '{}' requires a position", node_type),
                    ));
                }

                validate_target_layer(index, target_layer, &known_layer_refs, &mut errors);

                let key = ref_id
                    .clone()
                    .unwrap_or_else(|| format!("__new_node_{}", index));

                if entities.contains_key(&key) {
                    errors.push(issue(
                        "error",
                        "duplicate-ref",
                        Some(index),
                        format!("Reference '{}' is already in use", key),
                    ));
                    continue;
                }

                *layer_counts.entry(target_layer.clone()).or_default() += 1;
                node_layer.insert(key.clone(), target_layer.clone());

                let Some(metadata) = provider.get_node_metadata(node_type).await else {
                    errors.push(issue(
                        "error",
                        "unknown-node-type",
                        Some(index),
                        format!("Node type '{}' was not found in the catalog", node_type),
                    ));
                    continue;
                };

                validate_additional_node_pins(
                    index,
                    node_type,
                    additional_pins.as_deref(),
                    &metadata,
                    &mut errors,
                );
                let mut entity = entity_from_node_metadata(&key, &metadata);
                if let Some(pins) = additional_pins {
                    entity.pins.extend(pins.iter().map(known_pin_from_def));
                }
                entities.insert(key.clone(), entity);
                entities_to_check.insert(key);
            }
            BoardCommand::AddPlaceholder {
                name,
                ref_id,
                position,
                pins,
                target_layer,
                summary,
                ..
            } => {
                if summary.as_deref().unwrap_or_default().trim().is_empty() {
                    warnings.push(issue(
                        "warning",
                        "missing-summary",
                        Some(index),
                        format!("AddPlaceholder '{}' is missing a summary field", name),
                    ));
                }

                if ref_id.as_deref().unwrap_or_default().trim().is_empty() {
                    errors.push(issue(
                        "error",
                        "missing-ref-id",
                        Some(index),
                        format!("AddPlaceholder '{}' requires a ref_id like '$0'", name),
                    ));
                }

                if position.is_none() {
                    errors.push(issue(
                        "error",
                        "missing-position",
                        Some(index),
                        format!("AddPlaceholder '{}' requires a position", name),
                    ));
                }

                validate_target_layer(index, target_layer, &known_layer_refs, &mut errors);
                validate_placeholder_pins(index, pins.as_deref(), &mut errors);

                let key = ref_id
                    .clone()
                    .unwrap_or_else(|| format!("__new_placeholder_{}", index));

                if entities.contains_key(&key) {
                    errors.push(issue(
                        "error",
                        "duplicate-ref",
                        Some(index),
                        format!("Reference '{}' is already in use", key),
                    ));
                    continue;
                }

                entities.insert(key.clone(), entity_from_placeholder(&key, name, pins));
                known_layer_refs.insert(key.clone());
                entities_to_check.insert(key);
            }
            BoardCommand::RemoveNode { node_id, .. } => {
                if !entities.contains_key(node_id) {
                    errors.push(issue(
                        "error",
                        "unknown-node",
                        Some(index),
                        format!("Cannot remove unknown node '{}'", node_id),
                    ));
                } else {
                    if let Some(layer) = node_layer.get(node_id).cloned() {
                        *layer_counts.entry(layer).or_default() -= 1;
                    }
                    // Removal frees the node's connections: later ConnectPins re-using its exec
                    // sources must not false-positive as occupied, and later commands that still
                    // reference the removed node must error.
                    entities.remove(node_id);
                    exec_outgoing.retain(|(from_node, _), (to_node, _)| {
                        from_node != node_id && to_node != node_id
                    });
                    proposed_connections.retain(|(from_node, _, to_node, _)| {
                        from_node != node_id && to_node != node_id
                    });
                    incoming_inputs.retain(|(target_node, _)| target_node != node_id);
                }
            }
            BoardCommand::ConnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => {
                let Some(from_entity) = entities.get(from_node) else {
                    errors.push(issue(
                        "error",
                        "unknown-from-node",
                        Some(index),
                        unknown_entity_message(&entities, from_node, "Source"),
                    ));
                    continue;
                };

                let Some(to_entity) = entities.get(to_node) else {
                    errors.push(issue(
                        "error",
                        "unknown-to-node",
                        Some(index),
                        unknown_entity_message(&entities, to_node, "Target"),
                    ));
                    continue;
                };

                if from_entity.key == to_entity.key {
                    errors.push(issue(
                        "error",
                        "self-connection",
                        Some(index),
                        format!(
                            "Cannot connect '{}' to itself via {} -> {}",
                            from_entity.display_name, from_pin, to_pin
                        ),
                    ));
                    continue;
                }

                let Some(source_pin) = find_pin(from_entity, from_pin) else {
                    errors.push(issue(
                        "error",
                        "unknown-from-pin",
                        Some(index),
                        pin_not_found_message(from_entity, from_pin, Some(PinDirection::Output)),
                    ));
                    continue;
                };

                let Some(target_pin) = find_pin(to_entity, to_pin) else {
                    errors.push(issue(
                        "error",
                        "unknown-to-pin",
                        Some(index),
                        pin_not_found_message(to_entity, to_pin, Some(PinDirection::Input)),
                    ));
                    continue;
                };

                if source_pin.direction != PinDirection::Output && !from_entity.is_layer {
                    errors.push(issue(
                        "error",
                        "invalid-source-direction",
                        Some(index),
                        format!(
                            "Pin '{}.{}' is not an output pin",
                            from_entity.display_name, source_pin.name
                        ),
                    ));
                }

                if target_pin.direction != PinDirection::Input && !to_entity.is_layer {
                    errors.push(issue(
                        "error",
                        "invalid-target-direction",
                        Some(index),
                        format!(
                            "Pin '{}.{}' is not an input pin",
                            to_entity.display_name, target_pin.name
                        ),
                    ));
                }

                if !pin_types_compatible(source_pin, target_pin) {
                    errors.push(issue(
                        "error",
                        "incompatible-types",
                        Some(index),
                        format!(
                            "Cannot connect {} ({}) to {} ({})",
                            source_pin.name,
                            source_pin.data_type,
                            target_pin.name,
                            target_pin.data_type,
                        ),
                    ));
                    continue;
                }

                let connection_key = (
                    from_entity.key.clone(),
                    canonical_pin_ref(from_pin, source_pin),
                    to_entity.key.clone(),
                    canonical_pin_ref(to_pin, target_pin),
                );

                if proposed_connections.contains(&connection_key) {
                    errors.push(issue(
                        "error",
                        "duplicate-connection",
                        Some(index),
                        format!(
                            "Connection {}.{} -> {}.{} already exists",
                            from_entity.display_name,
                            source_pin.name,
                            to_entity.display_name,
                            target_pin.name,
                        ),
                    ));
                    continue;
                }

                if source_pin.data_type == "Execution" {
                    let exec_key = (
                        from_entity.key.clone(),
                        canonical_pin_ref(from_pin, source_pin),
                    );
                    if let Some((occupied_node, occupied_pin)) = exec_outgoing.get(&exec_key) {
                        errors.push(issue(
                            "error",
                            "exec-output-already-connected",
                            Some(index),
                            format!(
                                "Execution output '{}.{}' already drives {}.{} — an exec output has exactly ONE target, so this connect would silently rewire that chain. DisconnectPins the existing edge first if the rewire is intended, or continue from a different execution output",
                                from_entity.display_name,
                                source_pin.name,
                                occupied_node,
                                occupied_pin,
                            ),
                        ));
                        continue;
                    }
                    exec_outgoing.insert(
                        exec_key,
                        (to_entity.key.clone(), canonical_pin_ref(to_pin, target_pin)),
                    );
                }

                proposed_connections.insert(connection_key);
                incoming_inputs
                    .insert((to_entity.key.clone(), canonical_pin_ref(to_pin, target_pin)));
                entities_to_check.insert(from_entity.key.clone());
                entities_to_check.insert(to_entity.key.clone());
            }
            BoardCommand::DisconnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => {
                let from_entity = entities.get(from_node.as_str());
                let to_entity = entities.get(to_node.as_str());

                let canonical_from_pin = from_entity
                    .and_then(|e| find_pin(e, from_pin))
                    .map(|p| canonical_pin_ref(from_pin, p))
                    .unwrap_or_else(|| from_pin.clone());

                let canonical_to_pin = to_entity
                    .and_then(|e| find_pin(e, to_pin))
                    .map(|p| canonical_pin_ref(to_pin, p))
                    .unwrap_or_else(|| to_pin.clone());

                let key = (
                    from_node.clone(),
                    canonical_from_pin,
                    to_node.clone(),
                    canonical_to_pin,
                );
                if !proposed_connections.contains(&key) {
                    warnings.push(issue(
                        "warning",
                        "disconnect-missing-connection",
                        Some(index),
                        format!(
                            "DisconnectPins references a connection that is not present: {}.{} -> {}.{}",
                            from_node, from_pin, to_node, to_pin
                        ),
                    ));
                }
                exec_outgoing.remove(&(key.0.clone(), key.1.clone()));
                proposed_connections.remove(&key);
            }
            BoardCommand::UpdateNodePin {
                node_id,
                pin_id,
                value,
                ..
            } => {
                let Some(entity) = entities.get(node_id) else {
                    errors.push(issue(
                        "error",
                        "unknown-node",
                        Some(index),
                        format!("Cannot update pin on unknown node '{}'", node_id),
                    ));
                    continue;
                };

                let Some(pin) = find_pin(entity, pin_id) else {
                    errors.push(issue(
                        "error",
                        "unknown-pin",
                        Some(index),
                        pin_not_found_message(entity, pin_id, Some(PinDirection::Input)),
                    ));
                    continue;
                };

                if pin.direction != PinDirection::Input {
                    warnings.push(issue(
                        "warning",
                        "updating-output-pin",
                        Some(index),
                        format!(
                            "Pin '{}.{}' is not an input pin; verify this update is intentional",
                            entity.display_name, pin.name
                        ),
                    ));
                }

                if pin.name == "function_layer_id"
                    && let Some(target) = value.as_str()
                    && !known_layer_refs.contains(target)
                {
                    errors.push(issue(
                        "error",
                        "unknown-function-layer",
                        Some(index),
                        format!(
                            "'{}.function_layer_id' targets '{}', which is not a known function layer (use a layer id/name or a CreateLayer ref from this batch)",
                            entity.display_name, target
                        ),
                    ));
                }

                explicit_values.insert((entity.key.clone(), canonical_pin_ref(pin_id, pin)));
                entities_to_check.insert(entity.key.clone());
            }
            BoardCommand::RenameNode {
                node_id,
                friendly_name,
                ..
            } => {
                if !entities.contains_key(node_id) {
                    errors.push(issue(
                        "error",
                        "unknown-node",
                        Some(index),
                        format!("Cannot rename unknown node '{}'", node_id),
                    ));
                } else if friendly_name.trim().is_empty() {
                    errors.push(issue(
                        "error",
                        "empty-friendly-name",
                        Some(index),
                        format!(
                            "RenameNode '{}' requires a non-empty friendly_name",
                            node_id
                        ),
                    ));
                }
            }
            BoardCommand::MoveNode {
                node_id,
                target_layer,
                ..
            } => {
                if !entities.contains_key(node_id) {
                    errors.push(issue(
                        "error",
                        "unknown-node",
                        Some(index),
                        format!("Cannot move unknown node '{}'", node_id),
                    ));
                } else if let Some(new_layer) = target_layer {
                    if let Some(old_layer) = node_layer.get(node_id).cloned() {
                        *layer_counts.entry(old_layer).or_default() -= 1;
                    }
                    *layer_counts.entry(Some(new_layer.clone())).or_default() += 1;
                    node_layer.insert(node_id.clone(), Some(new_layer.clone()));
                }
                validate_target_layer(index, target_layer, &known_layer_refs, &mut errors);
            }
            BoardCommand::CreateLayer {
                name,
                ref_id,
                layer_type,
                node_ids,
                pins,
                position,
                target_layer,
                cache,
                summary,
                ..
            } => {
                if summary.as_deref().unwrap_or_default().trim().is_empty() {
                    warnings.push(issue(
                        "warning",
                        "missing-summary",
                        Some(index),
                        format!("CreateLayer '{}' is missing a summary field", name),
                    ));
                }
                if node_ids.is_empty() && position.is_none() {
                    errors.push(issue(
                        "error",
                        "empty-layer-without-position",
                        Some(index),
                        format!(
                            "CreateLayer '{}' needs either node_ids to group or a position for an empty layer",
                            name
                        ),
                    ));
                }
                for node_id in node_ids {
                    if !entities.contains_key(node_id) {
                        errors.push(issue(
                            "error",
                            "unknown-layer-node",
                            Some(index),
                            format!(
                                "CreateLayer '{}' references unknown node '{}'",
                                name, node_id
                            ),
                        ));
                    }
                }
                validate_target_layer(index, target_layer, &known_layer_refs, &mut errors);
                validate_placeholder_pins(index, pins.as_deref(), &mut errors);
                if cache.is_some()
                    && !matches!(layer_type.as_deref(), Some("Function") | Some("function"))
                {
                    errors.push(issue(
                        "error",
                        "cache-requires-function-layer",
                        Some(index),
                        format!(
                            "CreateLayer '{}' can only configure cache when layer_type is Function",
                            name
                        ),
                    ));
                }

                let key = ref_id
                    .clone()
                    .unwrap_or_else(|| format!("__new_layer_{}", index));
                if entities.contains_key(&key) {
                    errors.push(issue(
                        "error",
                        "duplicate-ref",
                        Some(index),
                        format!("Reference '{}' is already in use", key),
                    ));
                } else {
                    entities.insert(key.clone(), entity_from_layer(&key, name, pins));
                    known_layer_refs.insert(key.clone());
                    known_layer_refs.insert(name.clone());
                    layer_display.insert(key.clone(), name.clone());
                    if matches!(layer_type.as_deref(), Some(kind) if kind.eq_ignore_ascii_case("function"))
                    {
                        function_layer_refs.insert(key.clone());
                        function_layer_refs.insert(name.clone());
                        entities_to_check.insert(key);
                    }
                }
            }
            BoardCommand::RemoveLayer { layer_id, .. } => {
                if !known_layer_refs.contains(layer_id) {
                    errors.push(issue(
                        "error",
                        "unknown-layer",
                        Some(index),
                        format!("Cannot remove unknown layer '{}'", layer_id),
                    ));
                }
            }
            BoardCommand::UpdateLayerCache {
                layer_id, summary, ..
            } => {
                if !known_layer_refs.contains(layer_id) {
                    errors.push(issue(
                        "error",
                        "unknown-layer",
                        Some(index),
                        format!("Cannot update cache on unknown layer '{}'", layer_id),
                    ));
                } else if !function_layer_refs.contains(layer_id) {
                    errors.push(issue(
                        "error",
                        "cache-requires-function-layer",
                        Some(index),
                        format!("Cannot update cache on non-Function layer '{}'", layer_id),
                    ));
                }
                if summary.as_deref().unwrap_or_default().trim().is_empty() {
                    warnings.push(issue(
                        "warning",
                        "missing-summary",
                        Some(index),
                        format!("UpdateLayerCache '{}' is missing a summary field", layer_id),
                    ));
                }
            }
            BoardCommand::CreateVariable {
                name,
                data_type,
                value_type,
                summary,
                ..
            } => {
                if name.trim().is_empty() {
                    errors.push(issue(
                        "error",
                        "missing-variable-name",
                        Some(index),
                        "CreateVariable requires a non-empty name".to_string(),
                    ));
                }
                if data_type.trim().is_empty() || value_type.trim().is_empty() {
                    errors.push(issue(
                        "error",
                        "missing-variable-type",
                        Some(index),
                        format!(
                            "CreateVariable '{}' requires data_type and value_type",
                            name
                        ),
                    ));
                }
                if summary.as_deref().unwrap_or_default().trim().is_empty() {
                    warnings.push(issue(
                        "warning",
                        "missing-summary",
                        Some(index),
                        format!("CreateVariable '{}' is missing a summary field", name),
                    ));
                }
            }
            BoardCommand::UpdateVariable {
                variable_id,
                name,
                data_type,
                value_type,
                summary,
                ..
            } => {
                if !known_variables.contains(variable_id) {
                    errors.push(issue(
                        "error",
                        "unknown-variable",
                        Some(index),
                        format!("Cannot update unknown variable '{}'", variable_id),
                    ));
                }
                if name.as_deref().is_some_and(|name| name.trim().is_empty()) {
                    errors.push(issue(
                        "error",
                        "missing-variable-name",
                        Some(index),
                        format!("UpdateVariable '{}' cannot set an empty name", variable_id),
                    ));
                }
                if data_type
                    .as_deref()
                    .is_some_and(|data_type| data_type.trim().is_empty())
                    || value_type
                        .as_deref()
                        .is_some_and(|value_type| value_type.trim().is_empty())
                {
                    errors.push(issue(
                        "error",
                        "missing-variable-type",
                        Some(index),
                        format!(
                            "UpdateVariable '{}' cannot set an empty data_type or value_type",
                            variable_id
                        ),
                    ));
                }
                if summary.as_deref().unwrap_or_default().trim().is_empty() {
                    warnings.push(issue(
                        "warning",
                        "missing-summary",
                        Some(index),
                        format!(
                            "UpdateVariable '{}' is missing a summary field",
                            variable_id
                        ),
                    ));
                }
            }
            BoardCommand::RemoveVariable { variable_id, .. } => {
                if !known_variables.contains(variable_id) {
                    errors.push(issue(
                        "error",
                        "unknown-variable",
                        Some(index),
                        format!("Cannot remove unknown variable '{}'", variable_id),
                    ));
                }
            }
            BoardCommand::AddComment {
                content,
                target_layer,
                summary,
                ..
            } => {
                if content.trim().is_empty() {
                    errors.push(issue(
                        "error",
                        "empty-comment",
                        Some(index),
                        "CreateComment requires non-empty content".to_string(),
                    ));
                }
                validate_target_layer(index, target_layer, &known_layer_refs, &mut errors);
                if summary.as_deref().unwrap_or_default().trim().is_empty() {
                    warnings.push(issue(
                        "warning",
                        "missing-summary",
                        Some(index),
                        "CreateComment is missing a summary field".to_string(),
                    ));
                }
            }
            BoardCommand::RemoveComment { .. } => {}
            // Function references are additive metadata resolved at apply time (the targets may be
            // `$N` refs for nodes created in the same batch), so nothing to validate structurally.
            BoardCommand::SetNodeFunctionRefs { .. } => {}
        }
    }

    for entity_key in entities_to_check {
        let Some(entity) = entities.get(&entity_key) else {
            continue;
        };

        if entity.is_layer {
            continue;
        }

        let missing_inputs: Vec<_> = entity
            .pins
            .iter()
            .enumerate()
            .filter(|(_, pin)| pin.direction == PinDirection::Input && pin.data_type != "Execution")
            .filter(|(_, pin)| !pin.has_default_value)
            .filter(|(index, pin)| {
                let pin_ref = known_pin_ref_at(entity, *index);
                !incoming_inputs.contains(&(entity.key.clone(), pin_ref.clone()))
                    && !explicit_values.contains(&(entity.key.clone(), pin_ref))
                    // Existing graph edges predate occurrence refs and only carry a pin name.
                    && !incoming_inputs.contains(&(entity.key.clone(), pin.name.clone()))
                    && !explicit_values.contains(&(entity.key.clone(), pin.name.clone()))
            })
            .map(|(index, _)| known_pin_ref_at(entity, index))
            .collect();

        if !missing_inputs.is_empty() {
            errors.push(issue(
                "error",
                "missing-required-inputs",
                None,
                format!(
                    "'{}' is still missing required inputs: {}",
                    entity.display_name,
                    missing_inputs.join(", ")
                ),
            ));
        }
    }

    for (layer, count) in &layer_counts {
        if *count > MAX_NODES_PER_LAYER as i64 {
            let scope = match layer {
                Some(id) => format!(
                    "function/layer '{}'",
                    layer_display.get(id).cloned().unwrap_or_else(|| id.clone())
                ),
                None => "the root layer".to_string(),
            };
            errors.push(issue(
                "error",
                "layer-node-limit",
                None,
                format!(
                    "this batch would leave {scope} with {count} nodes (max {MAX_NODES_PER_LAYER}). Split the logic into function layers — each has its own {MAX_NODES_PER_LAYER}-node budget — and place nodes there via target_layer"
                ),
            ));
        }
    }

    EmitValidationOutcome {
        status: if errors.is_empty() {
            "valid"
        } else {
            "invalid"
        },
        validated_command_count: args.commands.len(),
        errors,
        warnings,
    }
}

fn validate_target_layer(
    command_index: usize,
    target_layer: &Option<String>,
    known_layers: &HashSet<String>,
    errors: &mut Vec<ValidationIssue>,
) {
    if let Some(layer_id) = target_layer
        && !known_layers.contains(layer_id)
    {
        errors.push(issue(
            "error",
            "unknown-target-layer",
            Some(command_index),
            format!(
                "target_layer '{}' does not exist in the current graph",
                layer_id
            ),
        ));
    }
}

fn validate_placeholder_pins(
    command_index: usize,
    pins: Option<&[PlaceholderPinDef]>,
    errors: &mut Vec<ValidationIssue>,
) {
    let Some(pins) = pins else {
        return;
    };

    let mut seen = HashSet::new();
    for pin in pins {
        if pin.name.trim().is_empty() {
            errors.push(issue(
                "error",
                "empty-placeholder-pin-name",
                Some(command_index),
                "Placeholder pin names cannot be empty".to_string(),
            ));
            continue;
        }
        if !seen.insert(pin.name.clone()) {
            errors.push(issue(
                "error",
                "duplicate-placeholder-pin",
                Some(command_index),
                format!("Placeholder pin '{}' is defined more than once", pin.name),
            ));
        }
        if !matches!(pin.pin_type.as_str(), "Input" | "Output") {
            errors.push(issue(
                "error",
                "invalid-placeholder-pin-direction",
                Some(command_index),
                format!(
                    "Placeholder pin '{}' has invalid pin_type '{}'; use Input or Output",
                    pin.name, pin.pin_type
                ),
            ));
        }
        if !matches!(
            pin.data_type.as_str(),
            "String"
                | "Integer"
                | "Float"
                | "Boolean"
                | "Struct"
                | "Generic"
                | "Execution"
                | "Date"
                | "PathBuf"
                | "Byte"
        ) {
            errors.push(issue(
                "error",
                "invalid-placeholder-pin-data-type",
                Some(command_index),
                format!(
                    "Placeholder pin '{}' has unsupported data_type '{}'",
                    pin.name, pin.data_type
                ),
            ));
        }
    }
}

fn validate_additional_node_pins(
    command_index: usize,
    node_type: &str,
    pins: Option<&[PlaceholderPinDef]>,
    metadata: &super::types::NodeMetadata,
    errors: &mut Vec<ValidationIssue>,
) {
    validate_placeholder_pins(command_index, pins, errors);
    let Some(pins) = pins else {
        return;
    };

    if !pins.is_empty() && node_type != "events_generic" {
        errors.push(issue(
            "error",
            "additional-pins-unsupported-node",
            Some(command_index),
            "Additional catalog-node pins are only supported on events_generic".to_string(),
        ));
    }

    for pin in pins {
        if pin.pin_type != "Output" || pin.data_type == "Execution" {
            errors.push(issue(
                "error",
                "invalid-generic-event-pin",
                Some(command_index),
                format!(
                    "Additional events_generic pin '{}' must be a non-execution Output",
                    pin.name
                ),
            ));
        }
        if metadata
            .outputs
            .iter()
            .any(|existing| existing.name == pin.name)
        {
            errors.push(issue(
                "error",
                "duplicate-catalog-pin",
                Some(command_index),
                format!(
                    "Node type '{}' already defines an output pin named '{}'",
                    node_type, pin.name
                ),
            ));
        }
    }
}

pub fn render_emit_commands_result(
    args: &EmitCommandsArgs,
    outcome: &EmitValidationOutcome,
) -> String {
    let requires_flowscript = emit_validation_requires_flowscript(outcome);
    let validation_json = if requires_flowscript {
        serde_json::to_string_pretty(&serde_json::json!({
            "status": "representation_rejected",
            "next_action": "write_patch_check_commit_flowscript",
            "retry_emit_commands": false,
            "validated_command_count": outcome.validated_command_count,
            "errors": &outcome.errors,
            "warnings": &outcome.warnings,
        }))
        .unwrap_or_default()
    } else {
        serde_json::to_string_pretty(outcome).unwrap_or_default()
    };

    if !outcome.errors.is_empty() {
        let mut lines = if requires_flowscript {
            vec![
                "Representation rejected; nothing was queued. Do not retry executable or layer commands through emit_commands. Author the behavior with write_flowscript, repair with patch_flowscript, validate with check_flowscript, then queue with commit_flowscript:"
                    .to_string(),
            ]
        } else {
            vec!["Validation failed. Fix these issues and call emit_commands again:".to_string()]
        };

        for issue in &outcome.errors {
            lines.push(format!("- {}", issue.message));
        }

        for issue in &outcome.warnings {
            lines.push(format!("- Warning: {}", issue.message));
        }

        return format!(
            "<validation>{}</validation>\n\n{}",
            validation_json,
            lines.join("\n")
        );
    }

    let commands_json = serde_json::to_string(&args.commands).unwrap_or_default();
    let mut lines = vec![format!("✓ Queued {} commands:", args.commands.len())];

    for issue in &outcome.warnings {
        lines.push(format!("- Warning: {}", issue.message));
    }

    lines.push(format!("\nExplanation: {}", args.explanation));
    lines.push(
        "\n⚠️ These commands are now queued. Do NOT emit the same commands again.".to_string(),
    );

    format!(
        "<commands>{}</commands>\n<validation>{}</validation>\n\n{}",
        commands_json,
        validation_json,
        lines.join("\n")
    )
}

fn build_known_entities(graph_context: &GraphContext) -> HashMap<String, KnownEntity> {
    let mut entities = HashMap::new();

    for node in &graph_context.nodes {
        entities.insert(
            node.id.clone(),
            KnownEntity {
                key: node.id.clone(),
                display_name: node.friendly_name.clone(),
                is_layer: false,
                pins: node
                    .inputs
                    .iter()
                    .map(|pin| KnownPin {
                        name: pin.name.clone(),
                        data_type: pin.type_name.clone(),
                        direction: PinDirection::Input,
                        has_default_value: pin.default_value.is_some(),
                    })
                    .chain(node.outputs.iter().map(|pin| KnownPin {
                        name: pin.name.clone(),
                        data_type: pin.type_name.clone(),
                        direction: PinDirection::Output,
                        has_default_value: false,
                    }))
                    .collect(),
            },
        );
    }

    for layer in &graph_context.layers {
        entities.insert(
            layer.id.clone(),
            KnownEntity {
                key: layer.id.clone(),
                display_name: layer.name.clone(),
                is_layer: true,
                pins: layer
                    .inputs
                    .iter()
                    .map(|pin| KnownPin {
                        name: pin.name.clone(),
                        data_type: pin.type_name.clone(),
                        direction: PinDirection::Input,
                        has_default_value: pin.default_value.is_some(),
                    })
                    .chain(layer.outputs.iter().map(|pin| KnownPin {
                        name: pin.name.clone(),
                        data_type: pin.type_name.clone(),
                        direction: PinDirection::Output,
                        has_default_value: false,
                    }))
                    .collect(),
            },
        );
    }

    entities
}

fn entity_from_node_metadata(key: &str, metadata: &super::types::NodeMetadata) -> KnownEntity {
    KnownEntity {
        key: key.to_string(),
        display_name: metadata.friendly_name.clone(),
        is_layer: false,
        pins: metadata
            .inputs
            .iter()
            .map(pin_from_metadata_input)
            .chain(metadata.outputs.iter().map(pin_from_metadata_output))
            .collect(),
    }
}

fn entity_from_placeholder(
    key: &str,
    name: &str,
    pins: &Option<Vec<PlaceholderPinDef>>,
) -> KnownEntity {
    let mut known_pins = vec![
        KnownPin {
            name: "exec_in".to_string(),
            data_type: "Execution".to_string(),
            direction: PinDirection::Input,
            has_default_value: false,
        },
        KnownPin {
            name: "exec_out".to_string(),
            data_type: "Execution".to_string(),
            direction: PinDirection::Output,
            has_default_value: false,
        },
    ];

    if let Some(custom_pins) = pins {
        for pin in custom_pins {
            known_pins.push(KnownPin {
                name: pin.name.clone(),
                data_type: pin.data_type.clone(),
                direction: if pin.pin_type == "Input" {
                    PinDirection::Input
                } else {
                    PinDirection::Output
                },
                has_default_value: false,
            });
        }
    }

    KnownEntity {
        key: key.to_string(),
        display_name: name.to_string(),
        is_layer: true,
        pins: known_pins,
    }
}

fn entity_from_layer(key: &str, name: &str, pins: &Option<Vec<PlaceholderPinDef>>) -> KnownEntity {
    KnownEntity {
        key: key.to_string(),
        display_name: name.to_string(),
        is_layer: true,
        pins: pins
            .as_ref()
            .map(|pins| {
                pins.iter()
                    .map(|pin| KnownPin {
                        name: pin.name.clone(),
                        data_type: pin.data_type.clone(),
                        direction: if pin.pin_type == "Input" {
                            PinDirection::Input
                        } else {
                            PinDirection::Output
                        },
                        has_default_value: false,
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn pin_from_metadata_input(pin: &PinMetadata) -> KnownPin {
    KnownPin {
        name: pin.name.clone(),
        data_type: pin.data_type.clone(),
        direction: PinDirection::Input,
        has_default_value: pin.default_value.is_some(),
    }
}

fn pin_from_metadata_output(pin: &PinMetadata) -> KnownPin {
    KnownPin {
        name: pin.name.clone(),
        data_type: pin.data_type.clone(),
        direction: PinDirection::Output,
        has_default_value: false,
    }
}

fn known_pin_from_def(pin: &PlaceholderPinDef) -> KnownPin {
    KnownPin {
        name: pin.name.clone(),
        data_type: pin.data_type.clone(),
        direction: if pin.pin_type == "Input" {
            PinDirection::Input
        } else {
            PinDirection::Output
        },
        has_default_value: false,
    }
}

fn find_pin<'a>(entity: &'a KnownEntity, pin_name: &str) -> Option<&'a KnownPin> {
    if let Some((name, occurrence)) = crate::flow::ast::parse_pin_occurrence_ref(pin_name) {
        return entity
            .pins
            .iter()
            .filter(|pin| pin.name == name)
            .nth(occurrence);
    }
    entity.pins.iter().find(|pin| pin.name == pin_name)
}

fn canonical_pin_ref(requested: &str, resolved: &KnownPin) -> String {
    if crate::flow::ast::parse_pin_occurrence_ref(requested).is_some() {
        requested.to_string()
    } else {
        resolved.name.clone()
    }
}

fn known_pin_ref_at(entity: &KnownEntity, index: usize) -> String {
    let pin = &entity.pins[index];
    let matching = entity
        .pins
        .iter()
        .filter(|candidate| candidate.direction == pin.direction && candidate.name == pin.name)
        .count();
    if matching <= 1 {
        return pin.name.clone();
    }
    let occurrence = entity.pins[..index]
        .iter()
        .filter(|candidate| candidate.direction == pin.direction && candidate.name == pin.name)
        .count();
    crate::flow::ast::pin_occurrence_ref(&pin.name, occurrence)
}

fn find_pin_case_insensitive<'a>(entity: &'a KnownEntity, pin_name: &str) -> Option<&'a KnownPin> {
    let normalized = pin_name.to_lowercase();
    entity
        .pins
        .iter()
        .find(|pin| pin.name.to_lowercase() == normalized)
}

fn pin_not_found_message(
    entity: &KnownEntity,
    requested_pin: &str,
    expected_direction: Option<PinDirection>,
) -> String {
    if let Some(pin) = find_pin_case_insensitive(entity, requested_pin) {
        return format!(
            "Pin '{}.{}' was not found. Pin names are case-sensitive; use exact pin name '{}'",
            entity.display_name, requested_pin, pin.name
        );
    }

    let expected_pins: Vec<_> = entity
        .pins
        .iter()
        .filter(|pin| expected_direction.is_none_or(|direction| pin.direction == direction))
        .map(|pin| pin.name.as_str())
        .collect();

    if expected_pins.is_empty() {
        format!(
            "Pin '{}.{}' was not found",
            entity.display_name, requested_pin
        )
    } else {
        format!(
            "Pin '{}.{}' was not found. Available matching pins: {}",
            entity.display_name,
            requested_pin,
            expected_pins.join(", ")
        )
    }
}

fn unknown_entity_message(
    entities: &HashMap<String, KnownEntity>,
    requested_id: &str,
    role: &str,
) -> String {
    if let Some(entity) = entities
        .values()
        .find(|entity| entity.display_name.eq_ignore_ascii_case(requested_id))
    {
        return format!(
            "{} node '{}' does not exist in the current plan. Use exact node id/ref_id '{}' for '{}'",
            role, requested_id, entity.key, entity.display_name
        );
    }

    format!(
        "{} node '{}' does not exist in the current plan",
        role, requested_id
    )
}

fn pin_types_compatible(source: &KnownPin, target: &KnownPin) -> bool {
    if source.data_type == "Execution" || target.data_type == "Execution" {
        return source.data_type == target.data_type;
    }

    if source.data_type == target.data_type {
        return true;
    }

    if source.data_type == "Generic" || target.data_type == "Generic" {
        return true;
    }

    source.data_type == "Struct" && target.data_type == "Struct"
}

fn issue(
    severity: &'static str,
    code: &'static str,
    command_index: Option<usize>,
    message: String,
) -> ValidationIssue {
    ValidationIssue {
        severity,
        code,
        command_index,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyCatalogProvider;

    #[flow_like_types::async_trait]
    impl CatalogProvider for EmptyCatalogProvider {
        async fn search(&self, _query: &str) -> Vec<super::super::types::NodeMetadata> {
            Vec::new()
        }

        async fn search_by_pin_type(
            &self,
            _pin_type: &str,
            _is_input: bool,
        ) -> Vec<super::super::types::NodeMetadata> {
            Vec::new()
        }

        async fn filter_by_category(
            &self,
            _category_prefix: &str,
        ) -> Vec<super::super::types::NodeMetadata> {
            Vec::new()
        }

        async fn get_node_metadata(
            &self,
            _node_type: &str,
        ) -> Option<super::super::types::NodeMetadata> {
            None
        }

        async fn get_all_nodes(&self) -> Vec<String> {
            Vec::new()
        }
    }

    fn layer_context(
        id: &str,
        name: &str,
        layer_type: &str,
    ) -> super::super::context::LayerContext {
        super::super::context::LayerContext {
            id: id.to_string(),
            name: name.to_string(),
            layer_type: layer_type.to_string(),
            parent_id: None,
            node_ids: Vec::new(),
            position: (0, 0),
            inputs: Vec::new(),
            outputs: Vec::new(),
            cache: None,
        }
    }

    #[tokio::test]
    async fn cache_update_preflight_requires_a_function_layer() {
        let context = GraphContext {
            nodes: Vec::new(),
            edges: Vec::new(),
            layers: vec![
                layer_context("function-layer", "Lookup", "Function"),
                layer_context("group-layer", "Group", "Collapsed"),
            ],
            variables: Vec::new(),
            selected_nodes: Vec::new(),
        };
        let commands = |layer_id: &str| EmitCommandsArgs {
            commands: vec![BoardCommand::UpdateLayerCache {
                layer_id: layer_id.to_string(),
                cache: None,
                summary: Some("Disable cache".to_string()),
            }],
            explanation: "Update function caching".to_string(),
        };

        let valid =
            validate_emit_commands(&commands("function-layer"), &context, &EmptyCatalogProvider)
                .await;
        assert!(valid.errors.is_empty(), "{:?}", valid.errors);

        let invalid =
            validate_emit_commands(&commands("group-layer"), &context, &EmptyCatalogProvider).await;
        assert!(invalid.errors.iter().any(|issue| {
            issue.code == "cache-requires-function-layer"
                && issue.message.contains("non-Function layer")
        }));
    }

    #[test]
    fn model_facing_emit_scope_requires_flowscript_for_executable_commands() {
        let args = EmitCommandsArgs {
            commands: vec![
                BoardCommand::AddNode {
                    node_type: "log_info".to_string(),
                    ref_id: Some("$0".to_string()),
                    position: Some(super::super::types::NodePosition { x: 0.0, y: 0.0 }),
                    friendly_name: None,
                    additional_pins: None,
                    target_layer: None,
                    summary: Some("Add log".to_string()),
                },
                BoardCommand::ConnectPins {
                    from_node: "start".to_string(),
                    from_pin: "exec_out".to_string(),
                    to_node: "$0".to_string(),
                    to_pin: "exec_in".to_string(),
                    summary: Some("Connect log".to_string()),
                },
            ],
            explanation: "Build executable behavior".to_string(),
        };

        let outcome = validate_model_facing_emit_commands_scope(&args);

        assert_eq!(outcome.status, "invalid");
        assert_eq!(outcome.errors.len(), 2);
        assert!(outcome.errors.iter().all(|issue| {
            issue.code == EXECUTABLE_COMMAND_REQUIRES_FLOWSCRIPT
                && issue.message.contains("write_flowscript")
                && issue.message.contains("patch_flowscript")
                && issue.message.contains("check_flowscript")
                && issue.message.contains("commit_flowscript")
        }));
        assert_eq!(outcome.errors[0].command_index, Some(0));
        assert_eq!(outcome.errors[1].command_index, Some(1));

        let rendered = render_emit_commands_result(&args, &outcome);
        assert!(rendered.contains("\"status\": \"representation_rejected\""));
        assert!(rendered.contains("\"next_action\": \"write_patch_check_commit_flowscript\""));
        assert!(rendered.contains("Do not retry executable or layer commands"));
        assert!(!rendered.contains("call emit_commands again"));
    }

    #[test]
    fn model_facing_emit_scope_accepts_move_node() {
        let args = EmitCommandsArgs {
            commands: vec![BoardCommand::MoveNode {
                node_id: "node-1".to_string(),
                position: super::super::types::NodePosition { x: 120.0, y: 80.0 },
                target_layer: None,
                summary: Some("Align node".to_string()),
            }],
            explanation: "Align the workflow".to_string(),
        };

        let outcome = validate_model_facing_emit_commands_scope(&args);

        assert_eq!(outcome.status, "valid");
        assert!(outcome.errors.is_empty());
    }

    #[test]
    fn model_facing_emit_scope_rejects_empty_and_layer_batches() {
        let empty = EmitCommandsArgs {
            commands: Vec::new(),
            explanation: "Nothing".to_string(),
        };
        assert_eq!(
            validate_model_facing_emit_commands_scope(&empty).errors[0].code,
            "empty-command-batch"
        );

        let function_layer = EmitCommandsArgs {
            commands: vec![BoardCommand::CreateLayer {
                name: "Executable helper".to_string(),
                ref_id: Some("$0".to_string()),
                layer_type: Some("Function".to_string()),
                node_ids: vec!["node-1".to_string()],
                pins: Some(Vec::new()),
                position: None,
                color: None,
                target_layer: None,
                cache: None,
                summary: Some("Create helper".to_string()),
            }],
            explanation: "Create function structure".to_string(),
        };
        let outcome = validate_model_facing_emit_commands_scope(&function_layer);
        assert_eq!(outcome.errors.len(), 1);
        assert_eq!(outcome.errors[0].code, "visual-layer-membership-unsafe");
    }

    #[test]
    fn model_facing_emit_scope_cannot_change_existing_layer_membership() {
        let args = EmitCommandsArgs {
            commands: vec![
                BoardCommand::MoveNode {
                    node_id: "node-1".to_string(),
                    position: super::super::types::NodePosition { x: 10.0, y: 20.0 },
                    target_layer: Some("unknown-kind-layer".to_string()),
                    summary: Some("Move into layer".to_string()),
                },
                BoardCommand::RemoveLayer {
                    layer_id: "unknown-kind-layer".to_string(),
                    summary: Some("Remove layer".to_string()),
                },
            ],
            explanation: "Change layer membership".to_string(),
        };

        let outcome = validate_model_facing_emit_commands_scope(&args);

        assert_eq!(outcome.errors.len(), 2);
        assert_eq!(
            outcome.errors[0].code,
            EXECUTABLE_COMMAND_REQUIRES_FLOWSCRIPT
        );
        assert_eq!(outcome.errors[1].code, "visual-layer-membership-unsafe");
    }

    #[test]
    fn duplicate_pin_occurrence_refs_validate_independently() {
        let entity = KnownEntity {
            key: "$0".to_string(),
            display_name: "Equal String".to_string(),
            is_layer: false,
            pins: vec![
                KnownPin {
                    name: "string".to_string(),
                    data_type: "String".to_string(),
                    direction: PinDirection::Input,
                    has_default_value: false,
                },
                KnownPin {
                    name: "string".to_string(),
                    data_type: "String".to_string(),
                    direction: PinDirection::Input,
                    has_default_value: false,
                },
            ],
        };

        assert!(std::ptr::eq(
            find_pin(&entity, "string[#1]").expect("first occurrence"),
            &entity.pins[0]
        ));
        assert!(std::ptr::eq(
            find_pin(&entity, "string[#2]").expect("second occurrence"),
            &entity.pins[1]
        ));
        assert_eq!(known_pin_ref_at(&entity, 0), "string[#1]");
        assert_eq!(known_pin_ref_at(&entity, 1), "string[#2]");
        assert!(find_pin(&entity, "string[#3]").is_none());
    }
}
