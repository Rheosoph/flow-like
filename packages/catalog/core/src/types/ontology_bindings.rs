use flow_like::flow::node::Node;
use flow_like_types::{Value, json::json};

use super::graph_overlay::{GraphOverlay, NodeLabelMapping};

fn set_default(node: &mut Node, pin_name: &str, value: Value) {
    if let Some(pin) = node.pins.values_mut().find(|pin| pin.name == pin_name) {
        pin.set_default_value(Some(value));
    }
}

fn json_type(data_type: &str) -> &'static str {
    let normalized = data_type.to_ascii_lowercase();
    if normalized.contains("bool") {
        "boolean"
    } else if normalized.contains("int") || normalized.contains("uint") {
        "integer"
    } else if normalized.contains("float")
        || normalized.contains("double")
        || normalized.contains("decimal")
    {
        "number"
    } else {
        "string"
    }
}

fn object_schema(object: &NodeLabelMapping) -> String {
    let properties = object
        .property_columns
        .iter()
        .map(|property| {
            (
                property.name.clone(),
                json!({ "type": json_type(&property.data_type) }),
            )
        })
        .collect::<flow_like_types::json::Map<String, Value>>();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": object.label,
        "type": "object",
        "properties": properties,
    })
    .to_string()
}

fn resource_key(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Builds project-level catalog presets from published ontology definitions.
///
/// The presets retain the trusted generic runtime `Node.name`; only defaults,
/// display metadata, and typed pin schemas are specialized. This keeps execution
/// compatible with the built-in registry while making each binding directly
/// discoverable in the board palette.
pub fn ontology_binding_nodes(ontologies: &[GraphOverlay], catalog: &[Node]) -> Vec<Node> {
    let query_prototype = catalog
        .iter()
        .find(|node| node.name == "ontology_query_objects");
    let action_prototype = catalog
        .iter()
        .find(|node| node.name == "ontology_action_request");
    let mut bindings = Vec::new();

    for ontology in ontologies
        .iter()
        .filter(|ontology| ontology.bindings_enabled)
    {
        if let Some(prototype) = query_prototype {
            for object in &ontology.nodes {
                let object_id = object
                    .id
                    .as_deref()
                    .or(object.api_name.as_deref())
                    .unwrap_or(&object.label);
                let mut binding = prototype.clone();
                binding.id = format!(
                    "ontology_binding_{}_object_{}",
                    resource_key(&ontology.id),
                    resource_key(object_id)
                );
                binding.friendly_name = format!("List {}", object.label);
                binding.description = format!(
                    "Reads {} objects through the {} ontology",
                    object.label, ontology.name
                );
                binding.category = format!("Data Studio/{}/Objects", ontology.name);
                set_default(&mut binding, "ontology_id", json!(ontology.id));
                set_default(&mut binding, "object_type", json!(object.label));
                if let Some(output) = binding.pins.values_mut().find(|pin| pin.name == "objects") {
                    output.schema = Some(object_schema(object));
                }
                bindings.push(binding);
            }
        }

        if let Some(prototype) = action_prototype {
            for action in ontology.actions.iter().filter(|action| action.enabled) {
                let mut binding = prototype.clone();
                binding.id = format!(
                    "ontology_binding_{}_action_{}",
                    resource_key(&ontology.id),
                    resource_key(&action.id)
                );
                binding.friendly_name = action.name.clone();
                binding.description = action.description.clone().unwrap_or_else(|| {
                    format!("Builds a validated request for the {} action", action.name)
                });
                binding.category = format!("Data Studio/{}/Actions", ontology.name);
                set_default(&mut binding, "ontology_id", json!(ontology.id));
                set_default(&mut binding, "action_id", json!(action.id));
                if let Some(schema) = &action.parameter_schema
                    && let Some(parameters) = binding
                        .pins
                        .values_mut()
                        .find(|pin| pin.name == "parameters")
                {
                    parameters.schema = Some(schema.to_string());
                }
                if let Some(object) = ontology.nodes.iter().find(|object| {
                    object.id.as_deref() == Some(&action.object_type)
                        || object.api_name.as_deref() == Some(&action.object_type)
                        || object.label == action.object_type
                }) && let Some(objects) =
                    binding.pins.values_mut().find(|pin| pin.name == "objects")
                {
                    objects.schema = Some(object_schema(object));
                }
                bindings.push(binding);
            }
        }
    }

    bindings
}

#[cfg(test)]
mod tests {
    use flow_like::flow::{node::Node, variable::VariableType};
    use flow_like_types::json::json;

    use crate::{GraphOverlay, NodeLabelMapping, OntologyActionDefinition, PropertyColumn};

    use super::ontology_binding_nodes;

    #[test]
    fn builds_stable_typed_object_binding() {
        let mut prototype = Node::new(
            "ontology_query_objects",
            "Query Ontology Objects",
            "",
            "Data Studio/Objects",
        );
        prototype.add_input_pin("ontology_id", "Ontology", "", VariableType::String);
        prototype.add_input_pin("object_type", "Object", "", VariableType::String);
        prototype.add_output_pin("objects", "Objects", "", VariableType::Struct);
        let ontology = GraphOverlay {
            id: "operations".to_string(),
            name: "Operations".to_string(),
            bindings_enabled: true,
            nodes: vec![NodeLabelMapping {
                id: Some("shipment".to_string()),
                api_name: Some("shipment".to_string()),
                label: "Shipment".to_string(),
                table: "shipments".to_string(),
                id_column: "id".to_string(),
                display_column: Some("tracking_number".to_string()),
                property_columns: vec![PropertyColumn {
                    name: "id".to_string(),
                    data_type: "Utf8".to_string(),
                    nullable: false,
                }],
                style: Default::default(),
            }],
            ..Default::default()
        };

        let bindings = ontology_binding_nodes(&[ontology], &[prototype]);

        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].name, "ontology_query_objects");
        assert_eq!(bindings[0].friendly_name, "List Shipment");
        let ontology_pin = bindings[0]
            .pins
            .values()
            .find(|pin| pin.name == "ontology_id")
            .unwrap();
        assert_eq!(
            ontology_pin.default_value,
            Some(flow_like_types::json::to_vec(&json!("operations")).unwrap())
        );
    }

    #[test]
    fn builds_action_binding_with_pinned_defaults() {
        let mut prototype = Node::new(
            "ontology_action_request",
            "Prepare Ontology Action",
            "",
            "Data Studio/Actions",
        );
        prototype.add_input_pin("ontology_id", "Ontology", "", VariableType::String);
        prototype.add_input_pin("action_id", "Action", "", VariableType::String);
        prototype.add_input_pin("objects", "Objects", "", VariableType::Struct);
        prototype.add_input_pin("parameters", "Parameters", "", VariableType::Struct);
        let ontology = GraphOverlay {
            id: "operations".to_string(),
            name: "Operations".to_string(),
            bindings_enabled: true,
            actions: vec![OntologyActionDefinition {
                id: "approve_shipment".to_string(),
                name: "Approve shipment".to_string(),
                description: None,
                object_type: "shipment".to_string(),
                board_id: "shipment_workflow".to_string(),
                board_version: Some([1, 2, 0]),
                start_node_id: Some("approve_start".to_string()),
                event_id: None,
                enabled: true,
                allow_bulk: false,
                parameter_schema: Some(json!({
                    "type": "object",
                    "properties": { "reason": { "type": "string" } }
                })),
            }],
            ..Default::default()
        };

        let bindings = ontology_binding_nodes(&[ontology], &[prototype]);

        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].friendly_name, "Approve shipment");
        assert_eq!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "action_id")
                .unwrap()
                .default_value,
            Some(flow_like_types::json::to_vec(&json!("approve_shipment")).unwrap())
        );
        assert!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "parameters")
                .unwrap()
                .schema
                .as_deref()
                .is_some_and(|schema| schema.contains("reason"))
        );
    }
}
