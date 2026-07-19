use flow_like::flow::{node::Node, variable::VariableType};
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

/// Maps a scalar JSON-schema property to a pin type. Nullable unions
/// (`["string", "null"]`) use the non-null variant. Non-scalar properties
/// (nested objects/arrays) return `None`, keeping the single struct pin.
fn scalar_variable_type(property: &Value) -> Option<VariableType> {
    let type_name = match property.get("type") {
        Some(Value::String(name)) => name.as_str(),
        Some(Value::Array(variants)) => variants
            .iter()
            .filter_map(Value::as_str)
            .find(|variant| *variant != "null")?,
        _ => return None,
    };
    match type_name {
        "string" => Some(VariableType::String),
        "integer" => Some(VariableType::Integer),
        "number" => Some(VariableType::Float),
        "boolean" => Some(VariableType::Boolean),
        _ => None,
    }
}

/// A flat object schema of scalar properties, as `(name, pin type, subschema)`.
/// Returns `None` for non-object, empty, or nested/complex schemas so those
/// keep the single typed `parameters` struct pin.
fn flat_scalar_properties(schema: &Value) -> Option<Vec<(String, VariableType, Value)>> {
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return None;
    }
    let properties = schema.get("properties")?.as_object()?;
    if properties.is_empty() {
        return None;
    }
    let mut specs = Vec::with_capacity(properties.len());
    for (name, property) in properties {
        specs.push((
            name.clone(),
            scalar_variable_type(property)?,
            property.clone(),
        ));
    }
    Some(specs)
}

/// Expands a flat scalar parameter schema into one typed input pin per property
/// on a generated action binding, replacing the single `parameters` struct pin.
/// The runtime reassembles the parameters object from these `param_*` pins.
/// Returns `false` (leaving the struct pin in place) for complex schemas.
fn apply_parameter_pins(binding: &mut Node, schema: &Value) -> bool {
    let Some(specs) = flat_scalar_properties(schema) else {
        return false;
    };
    binding.pins.retain(|_, pin| pin.name != "parameters");
    for (name, variable_type, property) in specs {
        let friendly = property
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| name.clone());
        let description = property
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let pin = binding.add_input_pin(
            &format!("param_{name}"),
            &friendly,
            &description,
            variable_type,
        );
        if let Some(default) = property.get("default") {
            pin.set_default_value(Some(default.clone()));
        }
    }
    true
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
                    && !apply_parameter_pins(&mut binding, schema)
                    && let Some(parameters) = binding
                        .pins
                        .values_mut()
                        .find(|pin| pin.name == "parameters")
                {
                    parameters.schema = Some(schema.to_string());
                }
                if let Some(object) = ontology.nodes.iter().find(|object| {
                    object.id.as_deref() == Some(action.object_type.as_str())
                        || object.api_name.as_deref() == Some(action.object_type.as_str())
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
    let query_prototype = catalog
        .iter()
        .find(|node| node.name == "ontology_query_remote_objects");
    let action_prototype = catalog
        .iter()
        .find(|node| node.name == "ontology_action_request_remote");
    let children_prototype = catalog
        .iter()
        .find(|node| node.name == "ontology_query_remote_children");
    if query_prototype.is_none() && action_prototype.is_none() && children_prototype.is_none() {
        return Vec::new();
    }
    let mut bindings = Vec::new();
    let mut binding_ids = std::collections::HashSet::new();

    let mut imports = imports
        .iter()
        .filter(|import| import.bindings_enabled)
        .collect::<Vec<_>>();
    imports.sort_by(|left, right| left.id.cmp(&right.id));
    for import in imports {
        if let Some(query_prototype) = query_prototype {
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

        if let Some(children_prototype) = children_prototype {
            let mut parents = import
                .contract
                .nodes
                .iter()
                .filter(|object| {
                    import
                        .contract
                        .edges
                        .iter()
                        .any(|edge| edge.containment && edge.src_label == object.label)
                })
                .collect::<Vec<_>>();
            parents.sort_by(|left, right| object_identifier(left).cmp(object_identifier(right)));
            for object in parents {
                let object_id = object_identifier(object);
                let mut binding = children_prototype.clone();
                let base_id = format!(
                    "remote_ontology_binding_{}_children_{}",
                    resource_key(&import.id),
                    resource_key(object_id)
                );
                binding.id = reserve_binding_id(
                    base_id,
                    &format!("remote-children\0{}\0{object_id}", import.id),
                    &mut binding_ids,
                );
                binding.friendly_name = format!("Expand {} Children", object.label);
                binding.description = format!(
                    "Loads containment children of a {} through the installed {} ontology from a connected project",
                    object.label, import.contract.name
                );
                binding.category = format!("Data Studio/Remote/{}/Objects", import.contract.name);
                set_default(&mut binding, "binding_id", json!(import.id));
                set_default(&mut binding, "object_type", json!(object_id));
                bindings.push(binding);
            }
        }

        if let Some(action_prototype) = action_prototype {
            let mut actions = import
                .contract
                .actions
                .iter()
                .filter(|action| action.enabled)
                .collect::<Vec<_>>();
            actions.sort_by(|left, right| left.id.cmp(&right.id));
            for action in actions {
                let mut binding = action_prototype.clone();
                let base_id = format!(
                    "remote_ontology_binding_{}_action_{}",
                    resource_key(&import.id),
                    resource_key(&action.id)
                );
                binding.id = reserve_binding_id(
                    base_id,
                    &format!("remote-action\0{}\0{}", import.id, action.id),
                    &mut binding_ids,
                );
                binding.friendly_name = action.name.clone();
                binding.description = action.description.clone().unwrap_or_else(|| {
                    format!(
                        "Invokes the {} action in the connected {} project",
                        action.name, import.contract.name
                    )
                });
                binding.category = format!("Data Studio/Remote/{}/Actions", import.contract.name);
                set_default(&mut binding, "binding_id", json!(import.id));
                set_default(&mut binding, "action_id", json!(action.id));
                if let Some(schema) = &action.parameter_schema
                    && !apply_parameter_pins(&mut binding, schema)
                    && let Some(parameters) = binding
                        .pins
                        .values_mut()
                        .find(|pin| pin.name == "parameters")
                {
                    parameters.schema = Some(schema.to_string());
                }
                if let Some(object) = import.contract.nodes.iter().find(|object| {
                    object.id.as_deref() == Some(action.object_type.as_str())
                        || object.api_name.as_deref() == Some(action.object_type.as_str())
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
        let object = NodeLabelMapping {
            id: Some("shipment".to_string()),
            api_name: Some("shipment_api".to_string()),
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
        };
        let expected_objects_schema = object_schema(&object);
        let ontology = GraphOverlay {
            id: "operations".to_string(),
            name: "Operations".to_string(),
            bindings_enabled: true,
            nodes: vec![object],
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
                exposed: true,
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
        // A flat scalar schema is expanded into one typed pin per property; the
        // single struct pin is dropped.
        assert!(
            bindings[0]
                .pins
                .values()
                .all(|pin| pin.name != "parameters")
        );
        let param_pin = bindings[0]
            .pins
            .values()
            .find(|pin| pin.name == "param_reason")
            .unwrap();
        assert_eq!(param_pin.data_type, VariableType::String);
        assert_eq!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "objects")
                .unwrap()
                .schema
                .as_deref(),
            Some(expected_objects_schema.as_str())
        );
    }

    #[test]
    fn keeps_struct_pin_for_nested_action_schema() {
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
                // A nested property keeps the single struct pin.
                parameter_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "address": { "type": "object", "properties": { "city": { "type": "string" } } }
                    }
                })),
                exposed: true,
            }],
            ..Default::default()
        };

        let bindings = ontology_binding_nodes(&[ontology], &[prototype]);

        assert_eq!(bindings.len(), 1);
        assert!(
            bindings[0]
                .pins
                .values()
                .all(|pin| !pin.name.starts_with("param_"))
        );
        assert!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "parameters")
                .unwrap()
                .schema
                .as_deref()
                .is_some_and(|schema| schema.contains("address"))
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

    #[test]
    fn builds_remote_action_binding_with_contract_schema() {
        let mut prototype = Node::new(
            "ontology_action_request_remote",
            "Invoke Remote Ontology Action",
            "",
            "Data Studio/Remote Actions",
        );
        prototype.add_input_pin("binding_id", "Installed Ontology", "", VariableType::String);
        prototype.add_input_pin("action_id", "Action", "", VariableType::String);
        prototype.add_input_pin("objects", "Objects", "", VariableType::Struct);
        prototype.add_input_pin("parameters", "Parameters", "", VariableType::Struct);
        let object = NodeLabelMapping {
            id: Some("shipment_record".to_string()),
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
        };
        let expected_objects_schema = object_schema(&object);
        let import = RemoteOntologyImport {
            id: "warehouse-app::operations".to_string(),
            target_app_id: "warehouse-app".to_string(),
            remote_ontology_id: "operations".to_string(),
            contract: GraphOverlay {
                id: "operations".to_string(),
                name: "Operations".to_string(),
                nodes: vec![object],
                actions: vec![OntologyActionDefinition {
                    id: "approve_shipment".to_string(),
                    name: "Approve shipment".to_string(),
                    description: None,
                    object_type: "shipment".to_string(),
                    // Sanitized contracts never carry producer coordinates.
                    board_id: String::new(),
                    board_version: None,
                    start_node_id: None,
                    event_id: None,
                    enabled: true,
                    allow_bulk: false,
                    parameter_schema: Some(json!({
                        "type": "object",
                        "properties": { "reason": { "type": "string" } }
                    })),
                    exposed: true,
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
        assert_eq!(bindings[0].name, "ontology_action_request_remote");
        assert_eq!(bindings[0].friendly_name, "Approve shipment");
        assert_eq!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "binding_id")
                .unwrap()
                .default_value,
            Some(flow_like_types::json::to_vec(&json!("warehouse-app::operations")).unwrap())
        );
        assert_eq!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "action_id")
                .unwrap()
                .default_value,
            Some(flow_like_types::json::to_vec(&json!("approve_shipment")).unwrap())
        );
        // The installed contract's flat schema expands into typed per-property
        // pins, matching the local binding behavior.
        assert!(
            bindings[0]
                .pins
                .values()
                .all(|pin| pin.name != "parameters")
        );
        assert!(
            bindings[0]
                .pins
                .values()
                .any(|pin| pin.name == "param_reason")
        );
        assert_eq!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "objects")
                .unwrap()
                .schema
                .as_deref(),
            Some(expected_objects_schema.as_str())
        );
    }

    #[test]
    fn builds_remote_children_binding_for_containment_parent() {
        let mut prototype = Node::new(
            "ontology_query_remote_children",
            "Query Remote Ontology Children",
            "",
            "Data Studio/Remote Objects",
        );
        prototype.add_input_pin("binding_id", "Installed Ontology", "", VariableType::String);
        prototype.add_input_pin("object_type", "Parent Object", "", VariableType::String);
        prototype.add_input_pin("node_id", "Parent ID", "", VariableType::Generic);
        prototype.add_output_pin("objects", "Children", "", VariableType::Struct);

        let object = |id: &str, label: &str, table: &str| NodeLabelMapping {
            id: Some(id.to_string()),
            api_name: Some(id.to_string()),
            label: label.to_string(),
            table: table.to_string(),
            id_column: "id".to_string(),
            display_column: None,
            property_columns: vec![PropertyColumn {
                name: "id".to_string(),
                data_type: "Utf8".to_string(),
                nullable: false,
            }],
            style: Default::default(),
        };
        let edge = crate::EdgeLabelMapping {
            id: Some("edge".to_string()),
            api_name: Some("dept_people".to_string()),
            label: "has_member".to_string(),
            table: "memberships".to_string(),
            src_column: "department_id".to_string(),
            dst_column: "person_id".to_string(),
            src_label: "Department".to_string(),
            dst_label: "Person".to_string(),
            src_node_column: None,
            dst_node_column: None,
            containment: true,
            dst_ontology: None,
            dst_binding_id: None,
            property_columns: Vec::new(),
            style: Default::default(),
        };
        let import = RemoteOntologyImport {
            id: "hr-app::org".to_string(),
            target_app_id: "hr-app".to_string(),
            remote_ontology_id: "org".to_string(),
            contract: GraphOverlay {
                id: "org".to_string(),
                name: "Org".to_string(),
                nodes: vec![
                    object("department", "Department", "departments"),
                    object("person", "Person", "people"),
                ],
                edges: vec![edge],
                ..Default::default()
            },
            source_updated_at: "t".to_string(),
            bindings_enabled: true,
            installed_at: "t".to_string(),
            updated_at: "t".to_string(),
        };

        let bindings = remote_ontology_binding_nodes(&[import], &[prototype]);

        // Only the parent (source of a containment edge) yields a children binding.
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].name, "ontology_query_remote_children");
        assert_eq!(bindings[0].friendly_name, "Expand Department Children");
        assert_eq!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "binding_id")
                .unwrap()
                .default_value,
            Some(flow_like_types::json::to_vec(&json!("hr-app::org")).unwrap())
        );
        assert_eq!(
            bindings[0]
                .pins
                .values()
                .find(|pin| pin.name == "object_type")
                .unwrap()
                .default_value,
            Some(flow_like_types::json::to_vec(&json!("department")).unwrap())
        );
    }
}
