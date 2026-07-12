use flow_like::flow::node::Node;
use flow_like_types::{Value, json::json};

use super::{
    graph_overlay::{GraphOverlay, NodeLabelMapping},
    remote_ontology::RemoteOntologyImport,
};

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
            let data_type = json_type(&property.data_type);
            let schema = if property.nullable {
                json!({ "type": [data_type, "null"] })
            } else {
                json!({ "type": data_type })
            };
            (property.name.clone(), schema)
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

fn object_identifier(object: &NodeLabelMapping) -> &str {
    object
        .id
        .as_deref()
        .or(object.api_name.as_deref())
        .unwrap_or(&object.label)
}

fn resource_key(value: &str) -> String {
    let readable = value
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
        .to_string();
    if readable.is_empty() {
        "resource".to_string()
    } else {
        readable
    }
}

fn stable_resource_hash(value: &str) -> u64 {
    // `DefaultHasher` is not stable across Rust releases. FNV-1a keeps a
    // deterministic suffix for the rare IDs whose readable slugs collide.
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn reserve_binding_id(
    base: String,
    identity: &str,
    used: &mut std::collections::HashSet<String>,
) -> String {
    // Preserve the original readable IDs for all non-colliding bindings so
    // boards created by the first Data Studio slice remain compatible.
    if used.insert(base.clone()) {
        return base;
    }
    let hash = stable_resource_hash(identity);
    let candidate = format!("{base}_{hash:016x}");
    if used.insert(candidate.clone()) {
        return candidate;
    }
    for suffix in 2_u32.. {
        let candidate = format!("{base}_{hash:016x}_{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!()
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
    let mut binding_ids = std::collections::HashSet::new();

    let mut ontologies = ontologies
        .iter()
        .filter(|ontology| ontology.bindings_enabled)
        .collect::<Vec<_>>();
    ontologies.sort_by(|left, right| left.id.cmp(&right.id));
    for ontology in ontologies {
        if let Some(prototype) = query_prototype {
            let mut objects = ontology.nodes.iter().collect::<Vec<_>>();
            objects.sort_by(|left, right| object_identifier(left).cmp(object_identifier(right)));
            for object in objects {
                let object_id = object_identifier(object);
                let mut binding = prototype.clone();
                let base_id = format!(
                    "ontology_binding_{}_object_{}",
                    resource_key(&ontology.id),
                    resource_key(object_id)
                );
                binding.id = reserve_binding_id(
                    base_id,
                    &format!("object\0{}\0{object_id}", ontology.id),
                    &mut binding_ids,
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
            let mut actions = ontology
                .actions
                .iter()
                .filter(|action| action.enabled)
                .collect::<Vec<_>>();
            actions.sort_by(|left, right| left.id.cmp(&right.id));
            for action in actions {
                let mut binding = prototype.clone();
                let base_id = format!(
                    "ontology_binding_{}_action_{}",
                    resource_key(&ontology.id),
                    resource_key(&action.id)
                );
                binding.id = reserve_binding_id(
                    base_id,
                    &format!("action\0{}\0{}", ontology.id, action.id),
                    &mut binding_ids,
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

/// Builds project-level catalog presets for ontology contracts installed from
/// connected projects.
///
/// Remote presets contain only the local import identifier and stable object
/// type identifier. The runtime resolves target project coordinates from the
/// trusted import record, so editable node defaults cannot redirect reads to a
/// different connected project or ontology.
pub fn remote_ontology_binding_nodes(
    imports: &[RemoteOntologyImport],
    catalog: &[Node],
) -> Vec<Node> {
    let Some(query_prototype) = catalog
        .iter()
        .find(|node| node.name == "ontology_query_remote_objects")
    else {
        return Vec::new();
    };
    let mut bindings = Vec::new();
    let mut binding_ids = std::collections::HashSet::new();

    let mut imports = imports
        .iter()
        .filter(|import| import.bindings_enabled)
        .collect::<Vec<_>>();
    imports.sort_by(|left, right| left.id.cmp(&right.id));
    for import in imports {
        let mut objects = import.contract.nodes.iter().collect::<Vec<_>>();
        objects.sort_by(|left, right| object_identifier(left).cmp(object_identifier(right)));
        for object in objects {
            let object_id = object_identifier(object);
            let mut binding = query_prototype.clone();
            let base_id = format!(
                "remote_ontology_binding_{}_object_{}",
                resource_key(&import.id),
                resource_key(object_id)
            );
            binding.id = reserve_binding_id(
                base_id,
                &format!("remote-object\0{}\0{object_id}", import.id),
                &mut binding_ids,
            );
            binding.friendly_name = format!("List {}", object.label);
            binding.description = format!(
                "Reads {} objects through the installed {} ontology from a connected project",
                object.label, import.contract.name
            );
            binding.category = format!("Data Studio/Remote/{}/Objects", import.contract.name);
            set_default(&mut binding, "binding_id", json!(import.id));
            set_default(&mut binding, "object_type", json!(object_id));
            if let Some(output) = binding.pins.values_mut().find(|pin| pin.name == "objects") {
                output.schema = Some(object_schema(object));
            }
            bindings.push(binding);
        }
    }

    bindings
}

#[cfg(test)]
mod tests {
    use flow_like::flow::{node::Node, variable::VariableType};
    use flow_like_types::json::{from_str, json};

    use crate::{
        GraphOverlay, NodeLabelMapping, OntologyActionDefinition, PropertyColumn,
        RemoteOntologyImport,
    };

    use super::{
        object_schema, ontology_binding_nodes, remote_ontology_binding_nodes, reserve_binding_id,
        resource_key,
    };

    #[test]
    fn binding_ids_preserve_legacy_slugs_and_disambiguate_collisions() {
        let base = format!("binding_{}", resource_key("warehouse-app"));
        let mut used = std::collections::HashSet::new();
        let first = reserve_binding_id(base.clone(), "warehouse-app", &mut used);
        let second = reserve_binding_id(base.clone(), "warehouse_app", &mut used);

        assert_eq!(first, "binding_warehouse_app");
        assert!(second.starts_with("binding_warehouse_app_"));
        assert_ne!(first, second);

        let mut repeated = std::collections::HashSet::new();
        assert_eq!(second, {
            reserve_binding_id(base.clone(), "warehouse-app", &mut repeated);
            reserve_binding_id(base, "warehouse_app", &mut repeated)
        });
    }

    #[test]
    fn object_schema_allows_null_only_for_nullable_properties() {
        let schema = object_schema(&NodeLabelMapping {
            id: Some("shipment".to_string()),
            api_name: Some("shipment".to_string()),
            label: "Shipment".to_string(),
            table: "shipments".to_string(),
            id_column: "id".to_string(),
            display_column: None,
            property_columns: vec![
                PropertyColumn {
                    name: "id".to_string(),
                    data_type: "Utf8".to_string(),
                    nullable: false,
                },
                PropertyColumn {
                    name: "notes".to_string(),
                    data_type: "Utf8".to_string(),
                    nullable: true,
                },
            ],
            style: Default::default(),
        });
        let schema: flow_like_types::Value = from_str(&schema).unwrap();

        assert_eq!(schema["properties"]["id"]["type"], json!("string"));
        assert_eq!(
            schema["properties"]["notes"]["type"],
            json!(["string", "null"])
        );
    }

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

    #[test]
    fn builds_remote_binding_from_local_import_id_only() {
        let mut prototype = Node::new(
            "ontology_query_remote_objects",
            "Query Remote Ontology Objects",
            "",
            "Data Studio/Remote Objects",
        );
        prototype.add_input_pin("binding_id", "Installed Ontology", "", VariableType::String);
        prototype.add_input_pin("object_type", "Object", "", VariableType::String);
        prototype.add_output_pin("objects", "Objects", "", VariableType::Struct);
        let import = RemoteOntologyImport {
            id: "warehouse-app::operations".to_string(),
            target_app_id: "warehouse-app".to_string(),
            remote_ontology_id: "operations".to_string(),
            contract: GraphOverlay {
                id: "operations".to_string(),
                name: "Operations".to_string(),
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
            },
            source_updated_at: "2026-07-12T12:00:00Z".to_string(),
            bindings_enabled: true,
            installed_at: "2026-07-12T12:00:00Z".to_string(),
            updated_at: "2026-07-12T12:00:00Z".to_string(),
        };

        let bindings = remote_ontology_binding_nodes(&[import], &[prototype]);

        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].name, "ontology_query_remote_objects");
        assert_eq!(bindings[0].friendly_name, "List Shipment");
        assert_eq!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "binding_id")
                .unwrap()
                .default_value,
            Some(flow_like_types::json::to_vec(&json!("warehouse-app::operations")).unwrap())
        );
        assert!(
            bindings[0]
                .pins
                .values()
                .all(|pin| pin.name != "target_app_id" && pin.name != "remote_ontology_id")
        );
    }
}
