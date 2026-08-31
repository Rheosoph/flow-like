//! Best-effort pruning of references to a table that is about to be dropped.
//!
//! Ontology overlays are rewritten (or deleted when nothing is left of them),
//! saved queries are only reported: removing a user's SQL is never implied by
//! dropping a table. Nothing in here is allowed to fail the drop, so every
//! fallible step degrades into a warning.

use crate::databases::graph::lancegraph::{
    EdgeMappingDef, GraphOverlayDef, NodeMappingDef, delete_overlay, list_overlays, save_overlay,
};
use crate::databases::workbench::saved_query::list_saved_queries;
use lancedb::Connection;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TableCascadeReport {
    /// Names of ontology overlays that referenced the table and were pruned.
    pub ontologies: Vec<String>,
    /// Names of saved queries whose SQL references the table. REPORTED ONLY, never deleted.
    pub saved_queries: Vec<String>,
    /// Best-effort failures. Never fatal — the table drop always proceeds.
    pub warnings: Vec<String>,
}

/// Best-effort. NEVER returns Err and never panics: every failure is pushed into
/// `warnings` so a cascade problem can never block the drop.
pub async fn prune_table_references(
    connection: &Connection,
    table_name: &str,
) -> TableCascadeReport {
    let mut report = TableCascadeReport::default();
    prune_overlays(connection, table_name, &mut report).await;
    collect_saved_query_references(connection, table_name, &mut report).await;
    report
}

async fn prune_overlays(
    connection: &Connection,
    table_name: &str,
    report: &mut TableCascadeReport,
) {
    let overlays = match list_overlays(connection).await {
        Ok(overlays) => overlays,
        Err(error) => {
            report
                .warnings
                .push(format!("Failed to list ontology overlays: {error}"));
            return;
        }
    };

    for mut overlay in overlays {
        let outcome = prune_overlay(&mut overlay, table_name);
        report.warnings.extend(outcome.warnings);
        if !outcome.changed {
            continue;
        }

        if overlay.nodes.is_empty() {
            match delete_overlay(connection, &overlay.id).await {
                Ok(()) => report.ontologies.push(overlay.name),
                Err(error) => report.warnings.push(format!(
                    "Failed to delete ontology '{}' left empty by dropping table '{}': {}",
                    overlay.name, table_name, error
                )),
            }
            continue;
        }

        overlay.updated_at = chrono::Utc::now().to_rfc3339();
        match save_overlay(connection, &overlay).await {
            Ok(()) => report.ontologies.push(overlay.name),
            Err(error) => report.warnings.push(format!(
                "Failed to save pruned ontology '{}' after dropping table '{}': {}",
                overlay.name, table_name, error
            )),
        }
    }
}

async fn collect_saved_query_references(
    connection: &Connection,
    table_name: &str,
    report: &mut TableCascadeReport,
) {
    let queries = match list_saved_queries(connection).await {
        Ok(queries) => queries,
        Err(error) => {
            report
                .warnings
                .push(format!("Failed to list saved queries: {error}"));
            return;
        }
    };

    for query in queries {
        if sql_references_table(&query.sql, table_name) {
            report.saved_queries.push(query.name);
        }
    }
}

#[derive(Debug, Default)]
struct OverlayPruneOutcome {
    changed: bool,
    warnings: Vec<String>,
}

/// Removes everything in `overlay` that depends on `table_name`: node mappings
/// on the table, edge mappings on the table or hanging off a removed local
/// endpoint, and the object views and actions that lose their object type.
fn prune_overlay(overlay: &mut GraphOverlayDef, table_name: &str) -> OverlayPruneOutcome {
    let mut outcome = OverlayPruneOutcome::default();
    let target = normalize_table_name(table_name);

    let removed_nodes: Vec<NodeMappingDef> = overlay
        .nodes
        .iter()
        .filter(|node| normalize_table_name(&node.table) == target)
        .cloned()
        .collect();
    if removed_nodes.is_empty()
        && !overlay
            .edges
            .iter()
            .any(|edge| edge_uses_table(edge, target))
    {
        return outcome;
    }

    overlay
        .nodes
        .retain(|node| normalize_table_name(&node.table) != target);
    outcome.changed = true;

    let surviving_labels: Vec<String> = overlay
        .nodes
        .iter()
        .map(|node| node.label.clone())
        .collect();
    let endpoint_removed = |label: &str| {
        removed_nodes.iter().any(|node| node.label == label)
            && !surviving_labels.iter().any(|survivor| survivor == label)
    };

    overlay.edges.retain(|edge| {
        !(edge_uses_table(edge, target)
            || endpoint_removed(&edge.src_label)
            || (edge_dst_is_local(edge) && endpoint_removed(&edge.dst_label)))
    });

    overlay
        .object_views
        .retain(|view| !object_type_is_dangling(&view.object_type, &removed_nodes, &overlay.nodes));

    let mut kept_actions = Vec::with_capacity(overlay.actions.len());
    for action in std::mem::take(&mut overlay.actions) {
        if object_type_is_dangling(&action.object_type, &removed_nodes, &overlay.nodes) {
            continue;
        }
        if !overlay
            .nodes
            .iter()
            .any(|node| object_type_matches(node, &action.object_type))
        {
            outcome.warnings.push(format!(
                "Ontology '{}' action '{}' binds to unresolvable object type '{}'; left in place after dropping table '{}'",
                overlay.name, action.name, action.object_type, table_name
            ));
        }
        kept_actions.push(action);
    }
    overlay.actions = kept_actions;

    outcome
}

fn edge_uses_table(edge: &EdgeMappingDef, target: &str) -> bool {
    normalize_table_name(&edge.table) == target
}

/// Cross-ontology edges resolve their destination label in another overlay, so
/// a same-named local label disappearing must not take them down.
fn edge_dst_is_local(edge: &EdgeMappingDef) -> bool {
    edge.dst_ontology.is_none() && edge.dst_binding_id.is_none()
}

fn object_type_matches(node: &NodeMappingDef, object_type: &str) -> bool {
    node.id.as_deref() == Some(object_type)
        || node.api_name.as_deref() == Some(object_type)
        || node.label == object_type
}

/// Dangling means the object type resolved to a node mapping that was just
/// removed. An object type that resolves to nothing at all is left untouched:
/// it was already broken before the drop, and guessing would delete user data.
fn object_type_is_dangling(
    object_type: &str,
    removed_nodes: &[NodeMappingDef],
    surviving_nodes: &[NodeMappingDef],
) -> bool {
    if surviving_nodes
        .iter()
        .any(|node| object_type_matches(node, object_type))
    {
        return false;
    }
    removed_nodes
        .iter()
        .any(|node| object_type_matches(node, object_type))
}

/// Overlay mappings store the bare LanceDB table name (they are passed straight
/// to `Connection::open_table`), but callers occasionally carry the on-disk
/// directory name. Stripping the suffix on both sides makes the two spellings
/// compare equal without ever widening the match.
pub(crate) fn normalize_table_name(name: &str) -> &str {
    name.strip_suffix(".lance").unwrap_or(name)
}

fn is_identifier_char(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_' || character == '$'
}

/// Identifier-boundary scan over SQL text: string literals and comments are
/// skipped, quoted and backticked identifiers are unwrapped, and a bare
/// identifier only matches when the whole token equals the table name, so
/// `orders` never matches `orders_archive`.
pub(crate) fn sql_references_table(sql: &str, table_name: &str) -> bool {
    let target = normalize_table_name(table_name);
    if target.is_empty() {
        return false;
    }

    let characters: Vec<char> = sql.chars().collect();
    let mut index = 0usize;
    while index < characters.len() {
        let character = characters[index];
        match character {
            '\'' => {
                index += 1;
                while index < characters.len() {
                    if characters[index] == '\'' {
                        if characters.get(index + 1) == Some(&'\'') {
                            index += 2;
                            continue;
                        }
                        break;
                    }
                    index += 1;
                }
                index += 1;
            }
            '"' | '`' => {
                let quote = character;
                index += 1;
                let mut identifier = String::new();
                while index < characters.len() {
                    if characters[index] == quote {
                        if characters.get(index + 1) == Some(&quote) {
                            identifier.push(quote);
                            index += 2;
                            continue;
                        }
                        break;
                    }
                    identifier.push(characters[index]);
                    index += 1;
                }
                index += 1;
                if normalize_table_name(&identifier).eq_ignore_ascii_case(target) {
                    return true;
                }
            }
            '-' if characters.get(index + 1) == Some(&'-') => {
                while index < characters.len() && characters[index] != '\n' {
                    index += 1;
                }
            }
            '/' if characters.get(index + 1) == Some(&'*') => {
                index += 2;
                while index < characters.len() {
                    if characters[index] == '*' && characters.get(index + 1) == Some(&'/') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            _ if is_identifier_char(character) => {
                let start = index;
                while index < characters.len() && is_identifier_char(characters[index]) {
                    index += 1;
                }
                let identifier: String = characters[start..index].iter().collect();
                if identifier.eq_ignore_ascii_case(target) {
                    return true;
                }
            }
            _ => index += 1,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databases::graph::lancegraph::{
        ObjectViewDef, OntologyActionDef, PropertyProjectionMode,
    };
    use flow_like_types::Value;

    fn node(label: &str, table: &str) -> NodeMappingDef {
        NodeMappingDef {
            id: Some(format!("node-{label}")),
            api_name: Some(label.to_lowercase()),
            label: label.to_string(),
            table: table.to_string(),
            id_column: "id".to_string(),
            display_column: None,
            property_columns: Vec::new(),
            style: Value::Null,
        }
    }

    fn edge(label: &str, table: &str, src_label: &str, dst_label: &str) -> EdgeMappingDef {
        EdgeMappingDef {
            id: Some(format!("edge-{label}")),
            api_name: None,
            label: label.to_string(),
            table: table.to_string(),
            src_column: "src".to_string(),
            dst_column: "dst".to_string(),
            src_label: src_label.to_string(),
            dst_label: dst_label.to_string(),
            src_node_column: None,
            dst_node_column: None,
            containment: false,
            dst_ontology: None,
            dst_binding_id: None,
            property_columns: Vec::new(),
            style: Value::Null,
        }
    }

    fn action(id: &str, object_type: &str) -> OntologyActionDef {
        OntologyActionDef {
            id: id.to_string(),
            name: format!("Action {id}"),
            description: None,
            object_type: object_type.to_string(),
            board_id: "board".to_string(),
            board_version: None,
            start_node_id: None,
            event_id: None,
            enabled: true,
            allow_bulk: false,
            parameter_schema: None,
            exposed: true,
        }
    }

    fn overlay(nodes: Vec<NodeMappingDef>, edges: Vec<EdgeMappingDef>) -> GraphOverlayDef {
        GraphOverlayDef {
            id: "ont".to_string(),
            name: "Ontology".to_string(),
            description: None,
            nodes,
            edges,
            object_views: Vec::new(),
            actions: Vec::new(),
            exposed: false,
            bindings_enabled: false,
            property_projection_mode: PropertyProjectionMode::Dynamic,
            default_limit: 200,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn prunes_node_on_target_table() {
        let mut definition = overlay(
            vec![node("Order", "orders"), node("Customer", "customers")],
            Vec::new(),
        );
        let outcome = prune_overlay(&mut definition, "orders");
        assert!(outcome.changed);
        assert_eq!(definition.nodes.len(), 1);
        assert_eq!(definition.nodes[0].label, "Customer");
    }

    #[test]
    fn leaves_unrelated_overlay_untouched() {
        let mut definition = overlay(
            vec![node("Customer", "customers")],
            vec![edge("BOUGHT", "purchases", "Customer", "Customer")],
        );
        let outcome = prune_overlay(&mut definition, "orders");
        assert!(!outcome.changed);
        assert!(outcome.warnings.is_empty());
        assert_eq!(definition.nodes.len(), 1);
        assert_eq!(definition.edges.len(), 1);
    }

    #[test]
    fn overlay_reduced_to_zero_nodes() {
        let mut definition = overlay(vec![node("Order", "orders")], Vec::new());
        let outcome = prune_overlay(&mut definition, "orders");
        assert!(outcome.changed);
        assert!(definition.nodes.is_empty());
    }

    #[test]
    fn table_prefix_does_not_match() {
        let mut definition = overlay(vec![node("Archive", "orders_archive")], Vec::new());
        let outcome = prune_overlay(&mut definition, "orders");
        assert!(!outcome.changed);
        assert_eq!(definition.nodes.len(), 1);
        assert!(!sql_references_table(
            "SELECT * FROM orders_archive",
            "orders"
        ));
        assert!(sql_references_table("SELECT * FROM orders", "orders"));
    }

    #[test]
    fn lance_suffix_is_tolerated_on_both_sides() {
        let mut definition = overlay(vec![node("Order", "orders.lance")], Vec::new());
        assert!(prune_overlay(&mut definition, "orders").changed);

        let mut definition = overlay(vec![node("Order", "orders")], Vec::new());
        assert!(prune_overlay(&mut definition, "orders.lance").changed);
    }

    #[test]
    fn drops_edges_on_target_table_and_removed_endpoints() {
        let mut definition = overlay(
            vec![node("Order", "orders"), node("Customer", "customers")],
            vec![
                edge("PLACED", "placements", "Customer", "Order"),
                edge("ON_TABLE", "orders", "Customer", "Customer"),
                edge("KNOWS", "knows", "Customer", "Customer"),
            ],
        );
        let outcome = prune_overlay(&mut definition, "orders");
        assert!(outcome.changed);
        assert_eq!(definition.edges.len(), 1);
        assert_eq!(definition.edges[0].label, "KNOWS");
    }

    #[test]
    fn keeps_cross_ontology_edge_with_remote_destination() {
        let mut remote = edge("LINKS", "links", "Customer", "Order");
        remote.dst_ontology = Some("other-ontology".to_string());
        let mut definition = overlay(
            vec![node("Order", "orders"), node("Customer", "customers")],
            vec![remote],
        );
        let outcome = prune_overlay(&mut definition, "orders");
        assert!(outcome.changed);
        assert_eq!(definition.edges.len(), 1);
    }

    #[test]
    fn drops_dangling_views_and_actions_but_warns_on_unresolvable_binding() {
        let mut definition = overlay(
            vec![node("Order", "orders"), node("Customer", "customers")],
            Vec::new(),
        );
        definition.object_views = vec![
            ObjectViewDef {
                object_type: "Order".to_string(),
                title_property: None,
                prominent_properties: Vec::new(),
            },
            ObjectViewDef {
                object_type: "Customer".to_string(),
                title_property: None,
                prominent_properties: Vec::new(),
            },
        ];
        definition.actions = vec![
            action("a1", "Order"),
            action("a2", "Customer"),
            action("a3", "Ghost"),
        ];

        let outcome = prune_overlay(&mut definition, "orders");
        assert!(outcome.changed);
        assert_eq!(definition.object_views.len(), 1);
        assert_eq!(definition.object_views[0].object_type, "Customer");
        let action_ids: Vec<&str> = definition
            .actions
            .iter()
            .map(|action| action.id.as_str())
            .collect();
        assert_eq!(action_ids, vec!["a2", "a3"]);
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("Ghost"));
    }

    #[test]
    fn sql_scan_handles_quoting_literals_and_comments() {
        assert!(sql_references_table("SELECT * FROM `orders`", "orders"));
        assert!(sql_references_table("SELECT * FROM \"orders\"", "orders"));
        assert!(sql_references_table(
            "SELECT * FROM db.orders o JOIN x ON x.id = o.id",
            "orders"
        ));
        assert!(!sql_references_table("SELECT 'orders' AS label", "orders"));
        assert!(!sql_references_table("-- orders\nSELECT 1", "orders"));
        assert!(!sql_references_table("/* orders */ SELECT 1", "orders"));
        assert!(!sql_references_table(
            "SELECT * FROM `orders_archive`",
            "orders"
        ));
        assert!(sql_references_table("SELECT * FROM ORDERS", "orders"));
    }
}
