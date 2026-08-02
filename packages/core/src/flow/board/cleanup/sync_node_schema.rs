//! Node schema synchronization cleanup step.
//!
//! This module handles automatic migration of placed nodes when their catalog
//! definition changes. It reconciles pins by name, preserving connections where
//! compatible and adding/removing pins as needed.

use std::collections::{HashMap, HashSet};

use crate::flow::{
    board::Board,
    node::{Node, NodeLogic},
    pin::Pin,
};
use std::sync::Arc;

/// Determines if a placed node needs schema synchronization based on version comparison.
///
/// Returns true if:
/// - Catalog has version, placed doesn't (upgrade from unversioned)
/// - Catalog version > placed version (newer schema available)
fn needs_sync(catalog_version: Option<u32>, placed_version: Option<u32>) -> bool {
    match (catalog_version, placed_version) {
        (None, None) => false,       // Both unversioned = no sync needed
        (Some(_), None) => true,     // Catalog versioned, placed isn't = sync (upgrade)
        (None, Some(_)) => false,    // Catalog unversioned, placed versioned = skip (unusual)
        (Some(c), Some(p)) => c > p, // Catalog newer = sync
    }
}

/// Resolves compact board refs until a concrete schema (or an unresolved terminal value) is
/// reached. Cycles terminate at one of their keys and are treated as unresolved by the caller.
fn resolve_schema_ref<'a>(schema: &'a str, refs: &'a HashMap<String, String>) -> &'a str {
    let mut current = schema;
    let mut seen = HashSet::new();

    while seen.insert(current) {
        let Some(resolved) = refs.get(current) else {
            break;
        };
        current = resolved;
    }

    current
}

fn is_json_schema(schema: &str) -> bool {
    matches!(
        serde_json::from_str::<serde_json::Value>(schema),
        Ok(serde_json::Value::Object(_) | serde_json::Value::Bool(_))
    )
}

fn schemas_equal(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }

    match (
        serde_json::from_str::<serde_json::Value>(left),
        serde_json::from_str::<serde_json::Value>(right),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn can_repair_from_catalog(catalog_version: Option<u32>, placed_version: Option<u32>) -> bool {
    catalog_version == placed_version
}

/// Pins whose catalog schema is only a fallback and is intentionally replaced by `on_update`.
/// Keep this list narrow: every other catalog schema is authoritative, including when a node
/// author changes it without incrementing the node version.
fn is_runtime_owned_schema(node_name: &str, pin_name: &str) -> bool {
    matches!(
        (node_name, pin_name),
        ("events_widget_action", "action_context")
            | ("a2ui_update_calendar", "events")
            | ("a2ui_update_gantt", "tasks")
            | ("a2ui_get_element", "element")
            | ("a2ui_query_elements_by_type", "elements")
    )
}

fn expand_schema_ref(schema: &mut Option<String>, refs: &HashMap<String, String>) {
    let Some(schema_ref) = schema.as_deref() else {
        return;
    };
    let resolved = resolve_schema_ref(schema_ref, refs);
    if resolved != schema_ref && is_json_schema(resolved) {
        *schema = Some(resolved.to_string());
    }
}

fn expand_board_schema_refs(board: &mut Board, refs: &HashMap<String, String>) {
    for variable in board.variables.values_mut() {
        expand_schema_ref(&mut variable.schema, refs);
    }
    for node in board.nodes.values_mut() {
        for pin in node.pins.values_mut() {
            expand_schema_ref(&mut pin.schema, refs);
        }
    }
    for layer in board.layers.values_mut() {
        for variable in layer.variables.values_mut() {
            expand_schema_ref(&mut variable.schema, refs);
        }
        for pin in layer.pins.values_mut() {
            expand_schema_ref(&mut pin.schema, refs);
        }
        for node in layer.nodes.values_mut() {
            for pin in node.pins.values_mut() {
                expand_schema_ref(&mut pin.schema, refs);
            }
        }
    }
}

/// Repairs missing, dangling, or stale static pin schemas from the matching catalog definition.
///
/// Runtime-owned schemas are preserved even when they differ from their catalog fallback. All
/// other catalog schemas are authoritative at the same node version. Before this runs, compact refs
/// are expanded so dynamic `on_update` implementations never receive a ref-to-ref chain.
fn repair_catalog_pin_schemas(
    placed_node: &mut Node,
    catalog_node: &Node,
    refs: &HashMap<String, String>,
) {
    let node_name = placed_node.name.clone();
    for placed_pin in placed_node.pins.values_mut() {
        let Some(catalog_pin) = catalog_node.pins.values().find(|catalog_pin| {
            catalog_pin.name == placed_pin.name && catalog_pin.pin_type == placed_pin.pin_type
        }) else {
            continue;
        };
        let Some(catalog_schema) = catalog_pin.schema.as_deref() else {
            continue;
        };

        let should_repair = match placed_pin.schema.as_deref() {
            None => true,
            Some(placed_schema) => {
                let resolved = resolve_schema_ref(placed_schema, refs);
                !is_json_schema(resolved)
                    || (!is_runtime_owned_schema(&node_name, &placed_pin.name)
                        && !schemas_equal(resolved, catalog_schema))
            }
        };

        if should_repair {
            placed_pin.schema = Some(catalog_schema.to_string());
        }
    }
}

/// Synchronizes a placed node's pins with the canonical catalog definition.
///
/// This function:
/// 1. Matches pins by name (not ID, since IDs are generated)
/// 2. Preserves existing pin IDs and connections for pins that still exist
/// 3. Adds new pins from the catalog definition
/// 4. Removes pins that no longer exist in the catalog
/// 5. Updates the placed node's version to match the catalog
///
/// Dynamic nodes (those that modify pins in `on_update`) will have their
/// dynamic pins re-added after this sync since `on_update` runs afterwards.
pub fn sync_node_with_catalog(placed_node: &mut Node, catalog_node: &Node) {
    // Build lookup of catalog pins by name (owned strings to avoid borrow issues)
    let catalog_pins_by_name: std::collections::HashMap<String, Pin> = catalog_node
        .pins
        .values()
        .map(|p| (p.name.clone(), p.clone()))
        .collect();

    // Track which pin names from catalog we've processed (owned strings)
    let mut processed_names: HashSet<String> = HashSet::new();

    // Collect existing placed pins info for iteration
    let existing_pins_info: Vec<(String, String)> = placed_node
        .pins
        .values()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();

    // Phase 1: Update existing pins that match by name, remove those that don't exist
    let mut pins_to_remove: Vec<String> = Vec::new();

    for (pin_id, pin_name) in &existing_pins_info {
        if let Some(catalog_pin) = catalog_pins_by_name.get(pin_name) {
            // Pin exists in both - update metadata, preserve ID and connections
            processed_names.insert(pin_name.clone());

            if let Some(placed_pin) = placed_node.pins.get_mut(pin_id) {
                // Update non-connection fields
                placed_pin.friendly_name = catalog_pin.friendly_name.clone();
                placed_pin.description = catalog_pin.description.clone();
                placed_pin.pin_type = catalog_pin.pin_type.clone();
                placed_pin.options = catalog_pin.options.clone();

                // Check if type changed - if so, clear connections as they may be invalid
                if placed_pin.data_type != catalog_pin.data_type
                    || placed_pin.value_type != catalog_pin.value_type
                {
                    placed_pin.data_type = catalog_pin.data_type.clone();
                    placed_pin.value_type = catalog_pin.value_type.clone();
                    // Clear connections since type changed
                    placed_pin.connected_to.clear();
                    placed_pin.depends_on.clear();
                    // Reset to catalog default value
                    placed_pin.default_value = catalog_pin.default_value.clone();
                }

                // Update schema reference
                placed_pin.schema = catalog_pin.schema.clone();
            }
        } else {
            // Pin no longer exists in catalog - mark for removal
            pins_to_remove.push(pin_id.clone());
        }
    }

    // Remove pins that no longer exist
    for pin_id in pins_to_remove {
        placed_node.pins.remove(&pin_id);
    }

    // Phase 2: Add new pins from catalog that don't exist in placed node
    for (name, catalog_pin) in &catalog_pins_by_name {
        if !processed_names.contains(name) {
            // This is a new pin - add it with a new ID
            let mut new_pin = catalog_pin.clone();
            new_pin.id = flow_like_types::create_id();
            // Clear any connections since this is a fresh pin
            new_pin.connected_to.clear();
            new_pin.depends_on.clear();
            placed_node.pins.insert(new_pin.id.clone(), new_pin);
        }
    }

    // Update placed node version to match catalog
    placed_node.version = catalog_node.version;

    // Copy other metadata that should stay in sync
    placed_node.friendly_name = catalog_node.friendly_name.clone();
    placed_node.description = catalog_node.description.clone();
    placed_node.category = catalog_node.category.clone();
    placed_node.icon = catalog_node.icon.clone();
    placed_node.docs = catalog_node.docs.clone();
    placed_node.scores = catalog_node.scores.clone();
    placed_node.long_running = catalog_node.long_running;
    placed_node.only_offline = catalog_node.only_offline;
    placed_node.oauth_providers = catalog_node.oauth_providers.clone();
    placed_node.required_oauth_scopes = catalog_node.required_oauth_scopes.clone();

    // Sync WASM permissions from catalog so placed nodes reflect current declarations
    sync_wasm_permissions(placed_node, catalog_node);

    // Clear any previous error since we've updated the node
    placed_node.error = None;
}

/// Synchronizes versioned node shapes and repairs unresolved schemas across the board.
///
/// This should be called BEFORE `on_update()` so that:
/// 1. Static pins are reconciled first
/// 2. Dynamic nodes can then add their dynamic pins via `on_update()`
pub async fn sync_board_node_schemas(
    board: &mut Board,
    registry: &crate::state::FlowNodeRegistryInner,
) {
    let refs = board.refs.clone();
    // Dynamic nodes commonly resolve schema refs only once. Expand every valid chain before
    // invoking `on_update`; the cleanup pass afterwards compacts them back to one-hop refs.
    expand_board_schema_refs(board, &refs);
    let sync_node = |node: &mut Node, registry: &crate::state::FlowNodeRegistryInner| {
        let catalog_node = match registry.get_node(&node.name) {
            Ok(n) => n,
            Err(_) => return,
        };

        let catalog_version = catalog_node.version;
        let placed_version = node.version;
        if needs_sync(catalog_version, placed_version) {
            sync_node_with_catalog(node, &catalog_node);
        } else {
            // Only use the catalog as a repair source when both sides describe the same schema
            // generation. In particular, never overwrite a newer board with an older registry.
            if can_repair_from_catalog(catalog_version, placed_version) {
                repair_catalog_pin_schemas(node, &catalog_node, &refs);
            }
            sync_oauth_metadata(node, &catalog_node);
            sync_wasm_permissions(node, &catalog_node);
        }
    };

    for node in board.nodes.values_mut() {
        sync_node(node, registry);
    }

    for layer in board.layers.values_mut() {
        for node in layer.nodes.values_mut() {
            sync_node(node, registry);
        }
    }
}

/// Copies OAuth-related metadata from the catalog node to the placed node.
/// Called independently of version-based sync because OAuth provider references
/// must always reflect the current catalog definition.
fn sync_oauth_metadata(placed_node: &mut Node, catalog_node: &Node) {
    placed_node.oauth_providers = catalog_node.oauth_providers.clone();
    placed_node.required_oauth_scopes = catalog_node.required_oauth_scopes.clone();
}

/// Syncs WASM permission declarations from the catalog node to the placed node.
/// Called regardless of version sync because WASM modules may update their
/// declared permissions independently of node schema version bumps.
fn sync_wasm_permissions(placed_node: &mut Node, catalog_node: &Node) {
    if let Some(catalog_wasm) = &catalog_node.wasm
        && let Some(placed_wasm) = &mut placed_node.wasm
    {
        placed_wasm.permissions = catalog_wasm.permissions.clone();
    }
}

/// Helper to create a sync function that can be used with NodeLogic
pub fn should_sync_node(logic: &Arc<dyn NodeLogic>, placed_node: &Node) -> bool {
    let catalog_node = logic.get_node();
    needs_sync(catalog_node.version, placed_node.version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{node::Node, variable::VariableType};

    #[test]
    fn test_needs_sync() {
        // Both unversioned = no sync
        assert!(!needs_sync(None, None));

        // Catalog versioned, placed not = sync
        assert!(needs_sync(Some(1), None));

        // Catalog not versioned, placed is = no sync
        assert!(!needs_sync(None, Some(1)));

        // Catalog newer = sync
        assert!(needs_sync(Some(2), Some(1)));

        // Same version = no sync
        assert!(!needs_sync(Some(1), Some(1)));

        // Catalog older = no sync (shouldn't happen but handle gracefully)
        assert!(!needs_sync(Some(1), Some(2)));
    }

    #[test]
    fn test_sync_adds_new_pins() {
        let mut placed = Node::new("test", "Test", "desc", "Cat");
        placed.add_input_pin("existing", "Existing", "desc", VariableType::String);
        placed.version = Some(1);

        let mut catalog = Node::new("test", "Test", "desc", "Cat");
        catalog.add_input_pin("existing", "Existing", "desc", VariableType::String);
        catalog.add_input_pin("new_pin", "New Pin", "new desc", VariableType::Integer);
        catalog.version = Some(2);

        sync_node_with_catalog(&mut placed, &catalog);

        assert_eq!(placed.pins.len(), 2);
        assert!(placed.get_pin_by_name("new_pin").is_some());
        assert_eq!(placed.version, Some(2));
    }

    #[test]
    fn test_sync_removes_old_pins() {
        let mut placed = Node::new("test", "Test", "desc", "Cat");
        placed.add_input_pin("keep", "Keep", "desc", VariableType::String);
        placed.add_input_pin("remove", "Remove", "desc", VariableType::String);
        placed.version = Some(1);

        let mut catalog = Node::new("test", "Test", "desc", "Cat");
        catalog.add_input_pin("keep", "Keep", "desc", VariableType::String);
        catalog.version = Some(2);

        sync_node_with_catalog(&mut placed, &catalog);

        assert_eq!(placed.pins.len(), 1);
        assert!(placed.get_pin_by_name("keep").is_some());
        assert!(placed.get_pin_by_name("remove").is_none());
    }

    #[test]
    fn test_sync_preserves_connections() {
        let mut placed = Node::new("test", "Test", "desc", "Cat");
        let pin = placed.add_input_pin("data", "Data", "desc", VariableType::String);
        let pin_id = pin.id.clone();
        pin.connected_to.insert("some_other_pin".to_string());
        placed.version = Some(1);

        let mut catalog = Node::new("test", "Test", "desc", "Cat");
        catalog.add_input_pin("data", "Data Updated", "new desc", VariableType::String);
        catalog.version = Some(2);

        sync_node_with_catalog(&mut placed, &catalog);

        let synced_pin = placed.get_pin_by_name("data").unwrap();
        // ID should be preserved
        assert_eq!(synced_pin.id, pin_id);
        // Connection should be preserved (same type)
        assert!(synced_pin.connected_to.contains("some_other_pin"));
        // Friendly name should be updated
        assert_eq!(synced_pin.friendly_name, "Data Updated");
    }

    #[test]
    fn test_sync_clears_connections_on_type_change() {
        let mut placed = Node::new("test", "Test", "desc", "Cat");
        let pin = placed.add_input_pin("data", "Data", "desc", VariableType::String);
        pin.connected_to.insert("some_other_pin".to_string());
        placed.version = Some(1);

        let mut catalog = Node::new("test", "Test", "desc", "Cat");
        // Type changed from String to Integer
        catalog.add_input_pin("data", "Data", "desc", VariableType::Integer);
        catalog.version = Some(2);

        sync_node_with_catalog(&mut placed, &catalog);

        let synced_pin = placed.get_pin_by_name("data").unwrap();
        // Connection should be cleared due to type change
        assert!(synced_pin.connected_to.is_empty());
        assert_eq!(synced_pin.data_type, VariableType::Integer);
    }

    #[test]
    fn same_version_node_repairs_dangling_schema_ref() {
        let current_schema = r#"{"type":"object","properties":{"current":{"type":"string"}}}"#;
        let mut placed = Node::new("test", "Test", "desc", "Cat");
        placed
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some("old-schema-ref".to_string());
        placed.version = Some(1);

        let mut catalog = Node::new("test", "Test", "desc", "Cat");
        catalog
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some(current_schema.to_string());
        catalog.version = Some(1);

        repair_catalog_pin_schemas(&mut placed, &catalog, &HashMap::new());

        assert_eq!(
            placed.get_pin_by_name("data").unwrap().schema.as_deref(),
            Some(current_schema)
        );
    }

    #[test]
    fn same_version_node_refreshes_stale_static_schema() {
        let old_schema = r#"{"type":"object","properties":{"old":{"type":"string"}}}"#;
        let current_schema = r#"{"type":"object","properties":{"current":{"type":"string"}}}"#;
        let mut placed = Node::new("test", "Test", "desc", "Cat");
        placed
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some(old_schema.to_string());

        let mut catalog = Node::new("test", "Test", "desc", "Cat");
        catalog
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some(current_schema.to_string());

        repair_catalog_pin_schemas(&mut placed, &catalog, &HashMap::new());

        assert_eq!(
            placed.get_pin_by_name("data").unwrap().schema.as_deref(),
            Some(current_schema)
        );
    }

    #[test]
    fn current_direct_schema_ref_is_left_compact() {
        let schema = r#"{"type":"object","properties":{"value":{"type":"string"}}}"#;
        let mut placed = Node::new("test", "Test", "desc", "Cat");
        placed
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some("schema-ref".to_string());

        let mut catalog = Node::new("test", "Test", "desc", "Cat");
        catalog
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some(schema.to_string());

        repair_catalog_pin_schemas(
            &mut placed,
            &catalog,
            &HashMap::from([("schema-ref".to_string(), schema.to_string())]),
        );

        assert_eq!(
            placed.get_pin_by_name("data").unwrap().schema.as_deref(),
            Some("schema-ref")
        );
    }

    #[test]
    fn ref_to_ref_schema_chain_is_preserved_for_cleanup() {
        let schema = r#"{"type":"object","properties":{"value":{"type":"string"}}}"#;
        let mut placed = Node::new("test", "Test", "desc", "Cat");
        placed
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some("outer-ref".to_string());

        let mut catalog = Node::new("test", "Test", "desc", "Cat");
        catalog
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some(schema.to_string());

        repair_catalog_pin_schemas(
            &mut placed,
            &catalog,
            &HashMap::from([
                ("outer-ref".to_string(), "inner-ref".to_string()),
                ("inner-ref".to_string(), schema.to_string()),
            ]),
        );

        assert_eq!(
            placed.get_pin_by_name("data").unwrap().schema.as_deref(),
            Some("outer-ref")
        );
    }

    #[test]
    fn valid_dynamic_schemas_are_preserved_over_catalog_fallbacks() {
        let dynamic_schema = r#"{"type":"object","properties":{"runtime":{"type":"boolean"}}}"#;
        let fallback_schema = r#"{"type":"object","additionalProperties":true}"#;
        for (node_name, pin_name) in [
            ("events_widget_action", "action_context"),
            ("a2ui_update_calendar", "events"),
            ("a2ui_update_gantt", "tasks"),
            ("a2ui_get_element", "element"),
            ("a2ui_query_elements_by_type", "elements"),
        ] {
            let mut placed = Node::new(node_name, "Test", "desc", "Cat");
            placed
                .add_output_pin(pin_name, "Dynamic", "desc", VariableType::Struct)
                .schema = Some(dynamic_schema.to_string());

            let mut catalog = Node::new(node_name, "Test", "desc", "Cat");
            catalog
                .add_output_pin(pin_name, "Dynamic", "desc", VariableType::Struct)
                .schema = Some(fallback_schema.to_string());

            repair_catalog_pin_schemas(&mut placed, &catalog, &HashMap::new());

            assert_eq!(
                placed.get_pin_by_name(pin_name).unwrap().schema.as_deref(),
                Some(dynamic_schema),
                "runtime schema for {node_name}.{pin_name} must survive catalog repair"
            );
        }
    }

    #[test]
    fn expands_ref_chains_before_dynamic_updates() {
        let schema = r#"{"type":"object","properties":{"value":{"type":"string"}}}"#;
        let refs = HashMap::from([
            ("outer-ref".to_string(), "inner-ref".to_string()),
            ("inner-ref".to_string(), schema.to_string()),
        ]);
        let mut compact = Some("outer-ref".to_string());

        expand_schema_ref(&mut compact, &refs);

        assert_eq!(compact.as_deref(), Some(schema));
    }

    #[test]
    fn newer_placed_schema_is_not_repaired_from_older_catalog() {
        let mut placed = Node::new("test", "Test", "desc", "Cat");
        placed
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some("newer-schema-ref".to_string());
        placed.version = Some(2);

        let mut catalog = Node::new("test", "Test", "desc", "Cat");
        catalog
            .add_output_pin("data", "Data", "desc", VariableType::Struct)
            .schema = Some(r#"{"type":"object","properties":{"old":{}}}"#.to_string());
        catalog.version = Some(1);

        assert!(!can_repair_from_catalog(catalog.version, placed.version));

        assert_eq!(
            placed.get_pin_by_name("data").unwrap().schema.as_deref(),
            Some("newer-schema-ref")
        );
    }
}
