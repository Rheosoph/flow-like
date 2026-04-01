//! Node schema synchronization cleanup step.
//!
//! This module handles automatic migration of placed nodes when their catalog
//! definition changes. It reconciles pins by name, preserving connections where
//! compatible and adding/removing pins as needed.

use std::collections::HashSet;

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

/// Synchronizes all nodes in the board that have version mismatches with their catalog definitions.
///
/// This should be called BEFORE `on_update()` so that:
/// 1. Static pins are reconciled first
/// 2. Dynamic nodes can then add their dynamic pins via `on_update()`
pub async fn sync_board_node_schemas(
    board: &mut Board,
    registry: &crate::state::FlowNodeRegistryInner,
) {
    let sync_node = |node: &mut Node, registry: &crate::state::FlowNodeRegistryInner| {
        let catalog_node = match registry.get_node(&node.name) {
            Ok(n) => n,
            Err(_) => return,
        };

        if needs_sync(catalog_node.version, node.version) {
            sync_node_with_catalog(node, &catalog_node);
        } else {
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
    use crate::flow::{
        node::Node,
        pin::{PinType, ValueType},
        variable::VariableType,
    };

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
}
