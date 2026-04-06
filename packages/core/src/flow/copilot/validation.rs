use std::collections::{BTreeSet, HashMap, HashSet};

use serde::Serialize;

use super::context::GraphContext;
use super::provider::CatalogProvider;
use super::tools::EmitCommandsArgs;
use super::types::{BoardCommand, PinMetadata, PlaceholderPinDef};

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

pub async fn validate_emit_commands(
    args: &EmitCommandsArgs,
    graph_context: &GraphContext,
    provider: &dyn CatalogProvider,
) -> EmitValidationOutcome {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    let mut entities = build_known_entities(graph_context);
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

    for (index, command) in args.commands.iter().enumerate() {
        match command {
            BoardCommand::AddNode {
                node_type,
                ref_id,
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

                let Some(metadata) = provider.get_node_metadata(node_type).await else {
                    errors.push(issue(
                        "error",
                        "unknown-node-type",
                        Some(index),
                        format!("Node type '{}' was not found in the catalog", node_type),
                    ));
                    continue;
                };

                entities.insert(key.clone(), entity_from_node_metadata(&key, &metadata));
                entities_to_check.insert(key);
            }
            BoardCommand::AddPlaceholder {
                name,
                ref_id,
                pins,
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
                        format!("Source node '{}' does not exist in the current plan", from_node),
                    ));
                    continue;
                };

                let Some(to_entity) = entities.get(to_node) else {
                    errors.push(issue(
                        "error",
                        "unknown-to-node",
                        Some(index),
                        format!("Target node '{}' does not exist in the current plan", to_node),
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
                        format!(
                            "Pin '{}.{}' was not found",
                            from_entity.display_name, from_pin
                        ),
                    ));
                    continue;
                };

                let Some(target_pin) = find_pin(to_entity, to_pin) else {
                    errors.push(issue(
                        "error",
                        "unknown-to-pin",
                        Some(index),
                        format!("Pin '{}.{}' was not found", to_entity.display_name, to_pin),
                    ));
                    continue;
                };

                if source_pin.direction != PinDirection::Output {
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

                if target_pin.direction != PinDirection::Input {
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
                    source_pin.name.clone(),
                    to_entity.key.clone(),
                    target_pin.name.clone(),
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

                proposed_connections.insert(connection_key);
                incoming_inputs.insert((to_entity.key.clone(), target_pin.name.clone()));
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
                let key = (
                    from_node.clone(),
                    from_pin.clone(),
                    to_node.clone(),
                    to_pin.clone(),
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
                proposed_connections.remove(&key);
            }
            BoardCommand::UpdateNodePin {
                node_id, pin_id, ..
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
                        format!("Pin '{}.{}' was not found", entity.display_name, pin_id),
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

                explicit_values.insert((entity.key.clone(), pin.name.clone()));
                entities_to_check.insert(entity.key.clone());
            }
            BoardCommand::MoveNode { node_id, .. } => {
                if !entities.contains_key(node_id) {
                    errors.push(issue(
                        "error",
                        "unknown-node",
                        Some(index),
                        format!("Cannot move unknown node '{}'", node_id),
                    ));
                }
            }
            BoardCommand::CreateLayer { .. }
            | BoardCommand::RemoveLayer { .. }
            | BoardCommand::CreateVariable { .. }
            | BoardCommand::RemoveVariable { .. }
            | BoardCommand::AddComment { .. }
            | BoardCommand::RemoveComment { .. } => {}
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
            .filter(|pin| pin.direction == PinDirection::Input && pin.data_type != "Execution")
            .filter(|pin| !pin.has_default_value)
            .filter(|pin| {
                !incoming_inputs.contains(&(entity.key.clone(), pin.name.clone()))
                    && !explicit_values.contains(&(entity.key.clone(), pin.name.clone()))
            })
            .map(|pin| pin.name.clone())
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

    EmitValidationOutcome {
        status: if errors.is_empty() { "valid" } else { "invalid" },
        validated_command_count: args.commands.len(),
        errors,
        warnings,
    }
}

pub fn render_emit_commands_result(
    args: &EmitCommandsArgs,
    outcome: &EmitValidationOutcome,
) -> String {
    let validation_json = serde_json::to_string_pretty(outcome).unwrap_or_default();

    if !outcome.errors.is_empty() {
        let mut lines = vec![
            "Validation failed. Fix these issues and call emit_commands again:"
                .to_string(),
        ];

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

fn find_pin<'a>(entity: &'a KnownEntity, pin_name: &str) -> Option<&'a KnownPin> {
    let normalized = pin_name.to_lowercase();
    entity
        .pins
        .iter()
        .find(|pin| pin.name == pin_name || pin.name.to_lowercase() == normalized)
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