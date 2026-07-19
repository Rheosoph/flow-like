//! Structural validation of overlay definitions against the live database.
//!
//! Shared by the API and desktop write/validate paths so the setup wizard can
//! surface per-mapping problems before an overlay is saved.

use super::GraphOverlayDef;
#[cfg(test)]
use super::PropertyProjectionMode;
use flow_like_types::Result;
use lancedb::Connection;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MappingValidation {
    pub kind: String,
    pub label: String,
    pub ok: bool,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationReport {
    pub ok: bool,
    pub issues: Vec<String>,
    pub mappings: Vec<MappingValidation>,
}

fn is_valid_graph_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Reserved infrastructure tables (`__…__`) hold overlay metadata; an overlay
/// must never map an object type or link type onto one, which would let a board
/// read or write graph internals through a mapping.
fn is_reserved_table(name: &str) -> bool {
    name.len() >= 4 && name.starts_with("__") && name.ends_with("__")
}

async fn table_columns(
    connection: &Connection,
    table: &str,
    cache: &mut HashMap<String, Option<HashSet<String>>>,
) -> Option<HashSet<String>> {
    if let Some(cached) = cache.get(table) {
        return cached.clone();
    }
    let columns = match connection.open_table(table).execute().await {
        Ok(opened) => match opened.schema().await {
            Ok(schema) => Some(
                schema
                    .fields()
                    .iter()
                    .map(|field| field.name().clone())
                    .collect::<HashSet<_>>(),
            ),
            Err(_) => None,
        },
        Err(_) => None,
    };
    cache.insert(table.to_string(), columns.clone());
    columns
}

/// Checks that every mapping references existing tables and columns, that
/// labels are unique and usable in Cypher, and that edges point at declared
/// node labels. Purely structural — no data is scanned.
pub async fn validate_overlay_definition(
    connection: &Connection,
    overlay: &GraphOverlayDef,
) -> Result<ValidationReport> {
    let mut report = ValidationReport {
        ok: true,
        issues: Vec::new(),
        mappings: Vec::new(),
    };
    let mut schema_cache: HashMap<String, Option<HashSet<String>>> = HashMap::new();

    if overlay.name.trim().is_empty() {
        report.issues.push("The overlay needs a name".to_string());
    }
    if overlay.nodes.is_empty() {
        report
            .issues
            .push("The overlay defines no object types".to_string());
    }

    let mut seen_labels: HashSet<String> = HashSet::new();
    let mut seen_api_names: HashSet<String> = HashSet::new();
    for node in &overlay.nodes {
        let mut issues = Vec::new();
        if node.label.trim().is_empty() {
            issues.push("Object type label must not be empty".to_string());
        } else if !is_valid_graph_identifier(&node.label) {
            issues.push(format!(
                "Label '{}' cannot be used in queries; use letters, digits, and underscores, starting with a letter",
                node.label
            ));
        }
        if !seen_labels.insert(node.label.to_lowercase()) {
            issues.push(format!(
                "Label '{}' is defined more than once (labels are case-insensitive in queries)",
                node.label
            ));
        }
        if let Some(api_name) = node.api_name.as_deref()
            && !api_name.trim().is_empty()
            && !seen_api_names.insert(api_name.to_lowercase())
        {
            issues.push(format!("API name '{}' is used more than once", api_name));
        }
        if is_reserved_table(&node.table) {
            issues.push(format!(
                "Table '{}' is reserved for graph internals and cannot be mapped",
                node.table
            ));
        }
        match table_columns(connection, &node.table, &mut schema_cache).await {
            None => issues.push(format!(
                "Table '{}' does not exist or cannot be read",
                node.table
            )),
            Some(columns) => {
                if !columns.contains(&node.id_column) {
                    issues.push(format!(
                        "ID column '{}' does not exist in table '{}'",
                        node.id_column, node.table
                    ));
                }
                if let Some(display) = node.display_column.as_deref()
                    && !columns.contains(display)
                {
                    issues.push(format!(
                        "Display column '{}' does not exist in table '{}'",
                        display, node.table
                    ));
                }
                for property in &node.property_columns {
                    if !columns.contains(&property.name) {
                        issues.push(format!(
                            "Property column '{}' does not exist in table '{}'",
                            property.name, node.table
                        ));
                    }
                }
            }
        }
        report.mappings.push(MappingValidation {
            kind: "node".to_string(),
            label: node.label.clone(),
            ok: issues.is_empty(),
            issues,
        });
    }

    let node_labels = overlay
        .nodes
        .iter()
        .map(|node| node.label.as_str())
        .collect::<HashSet<_>>();
    let mut identity_overrides: HashMap<String, String> = HashMap::new();
    for edge in &overlay.edges {
        let mut issues = Vec::new();
        if edge.label.trim().is_empty() {
            issues.push("Link type label must not be empty".to_string());
        } else if !is_valid_graph_identifier(&edge.label) {
            issues.push(format!(
                "Label '{}' cannot be used in queries; use letters, digits, and underscores, starting with a letter",
                edge.label
            ));
        }
        if !seen_labels.insert(edge.label.to_lowercase()) {
            issues.push(format!(
                "Label '{}' collides with another label (labels are case-insensitive in queries)",
                edge.label
            ));
        }
        if !node_labels.contains(edge.src_label.as_str()) {
            issues.push(format!(
                "Source object type '{}' is not defined in this overlay",
                edge.src_label
            ));
        }
        if !node_labels.contains(edge.dst_label.as_str()) {
            issues.push(format!(
                "Target object type '{}' is not defined in this overlay",
                edge.dst_label
            ));
        }
        if is_reserved_table(&edge.table) {
            issues.push(format!(
                "Table '{}' is reserved for graph internals and cannot be mapped",
                edge.table
            ));
        }
        match table_columns(connection, &edge.table, &mut schema_cache).await {
            None => issues.push(format!(
                "Table '{}' does not exist or cannot be read",
                edge.table
            )),
            Some(columns) => {
                for (role, column) in [("Source", &edge.src_column), ("Target", &edge.dst_column)] {
                    if !columns.contains(column) {
                        issues.push(format!(
                            "{} column '{}' does not exist in table '{}'",
                            role, column, edge.table
                        ));
                    }
                }
                for property in &edge.property_columns {
                    if !columns.contains(&property.name) {
                        issues.push(format!(
                            "Property column '{}' does not exist in table '{}'",
                            property.name, edge.table
                        ));
                    }
                }
            }
        }
        for (role, label, override_column) in [
            (
                "Source",
                edge.src_label.as_str(),
                edge.src_node_column.as_deref(),
            ),
            (
                "Target",
                edge.dst_label.as_str(),
                edge.dst_node_column.as_deref(),
            ),
        ] {
            let Some(override_column) = override_column else {
                continue;
            };
            if let Some(existing) = identity_overrides.get(label) {
                if existing != override_column {
                    issues.push(format!(
                        "{} object type '{}' conflicts with node identity override '{}' already declared as '{}'",
                        role, label, override_column, existing
                    ));
                }
            } else {
                identity_overrides.insert(label.to_string(), override_column.to_string());
            }
            let Some(node) = overlay.nodes.iter().find(|node| node.label == label) else {
                continue;
            };
            if let Some(columns) = table_columns(connection, &node.table, &mut schema_cache).await
                && !columns.contains(override_column)
            {
                issues.push(format!(
                    "{} node join column '{}' does not exist in table '{}' for object type '{}'",
                    role, override_column, node.table, label
                ));
            }
        }
        report.mappings.push(MappingValidation {
            kind: "edge".to_string(),
            label: edge.label.clone(),
            ok: issues.is_empty(),
            issues,
        });
    }

    report.ok = report.issues.is_empty() && report.mappings.iter().all(|mapping| mapping.ok);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databases::graph::lancegraph::{EdgeMappingDef, NodeMappingDef};
    use arrow::array::{RecordBatch, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use flow_like_types::Value;
    use lancedb::connect;
    use std::sync::Arc;

    #[tokio::test]
    async fn rejects_missing_and_conflicting_node_side_join_overrides() -> Result<()> {
        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let connection = connect(&test_path).execute().await?;

        let node_schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
        connection
            .create_table(
                "people",
                vec![RecordBatch::try_new(
                    node_schema,
                    vec![Arc::new(StringArray::from(vec!["1"]))],
                )?],
            )
            .execute()
            .await?;
        let edge_schema = Arc::new(Schema::new(vec![
            Field::new("source", DataType::Utf8, false),
            Field::new("target", DataType::Utf8, false),
        ]));
        connection
            .create_table(
                "links",
                vec![RecordBatch::try_new(
                    edge_schema,
                    vec![
                        Arc::new(StringArray::from(vec!["1"])),
                        Arc::new(StringArray::from(vec!["1"])),
                    ],
                )?],
            )
            .execute()
            .await?;

        let overlay = GraphOverlayDef {
            id: "ontology".to_string(),
            name: "Ontology".to_string(),
            description: None,
            nodes: vec![NodeMappingDef {
                id: Some("person".to_string()),
                api_name: Some("person".to_string()),
                label: "Person".to_string(),
                table: "people".to_string(),
                id_column: "id".to_string(),
                display_column: None,
                property_columns: Vec::new(),
                style: Value::Null,
            }],
            edges: vec![EdgeMappingDef {
                id: Some("knows".to_string()),
                api_name: Some("knows".to_string()),
                label: "KNOWS".to_string(),
                table: "links".to_string(),
                src_column: "source".to_string(),
                dst_column: "target".to_string(),
                src_label: "Person".to_string(),
                dst_label: "Person".to_string(),
                src_node_column: Some("external_id".to_string()),
                dst_node_column: Some("alternate_id".to_string()),
                containment: false,
                dst_ontology: None,
                dst_binding_id: None,
                property_columns: Vec::new(),
                style: Value::Null,
            }],
            object_views: Vec::new(),
            actions: Vec::new(),
            exposed: false,
            bindings_enabled: false,
            property_projection_mode: PropertyProjectionMode::Dynamic,
            default_limit: 100,
            created_at: String::new(),
            updated_at: String::new(),
        };

        let report = validate_overlay_definition(&connection, &overlay).await?;
        let edge = report
            .mappings
            .iter()
            .find(|mapping| mapping.kind == "edge")
            .unwrap();
        assert!(!report.ok);
        assert!(
            edge.issues
                .iter()
                .any(|issue| issue.contains("external_id") && issue.contains("people"))
        );
        assert!(
            edge.issues
                .iter()
                .any(|issue| issue.contains("conflicts with node identity override"))
        );

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }
}
