//! Structural validation of overlay definitions against the live database.
//!
//! Shared by the API and desktop write/validate paths so the setup wizard can
//! surface per-mapping problems before an overlay is saved.

use super::GraphOverlayDef;
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
