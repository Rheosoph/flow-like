//! One metadata-only pass over the tables in an app database.
//!
//! Everything here reads manifests, never data pages: the schema, the index
//! list and LanceDB's own table statistics. The semantic half — which object
//! type a table backs, and what reads it — comes from the overlay and saved
//! query manifests that already live in the same connection, so the whole
//! summary costs one connection and no scans.
//!
//! Nothing is allowed to fail the listing. A table that cannot be opened comes
//! back with its `error` set and every other table still resolves.

use crate::databases::graph::lancegraph::{GraphOverlayDef, NodeMappingDef, list_overlays};
use crate::databases::table_cascade::{normalize_table_name, sql_references_table};
use crate::databases::workbench::saved_query::list_saved_queries;
use arrow_schema::DataType;
use futures::stream::{self, StreamExt};
use lancedb::Connection;
use serde::{Deserialize, Serialize};

/// Tables are summarized concurrently, but a project with a hundred tables must
/// not open a hundred object-store connections at once.
const SUMMARY_CONCURRENCY: usize = 8;

/// Coarse bucket a column falls into, used by the client to colour a schema
/// strip without teaching the frontend the whole Arrow type system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColumnFamily {
    Text,
    Number,
    Time,
    Bool,
    Vector,
    Struct,
    Binary,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnSummary {
    pub name: String,
    /// Arrow type rendered for display, e.g. `Utf8` or `Timestamp(Millisecond, "UTC")`.
    pub data_type: String,
    pub family: ColumnFamily,
    pub nullable: bool,
    /// Set only for fixed-size float lists, i.e. embedding columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexSummary {
    pub name: String,
    pub index_type: String,
    pub columns: Vec<String>,
}

/// LanceDB's fragment-level accounting. Absent when `stats()` fails, which must
/// not cost the caller the rest of the summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSummary {
    pub total_bytes: u64,
    pub num_fragments: usize,
    /// Fragments below LanceDB's compaction threshold. A high count is the
    /// signal that `optimize` is worth running.
    pub num_small_fragments: usize,
}

/// What the semantic layer and the workbench do with this table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConsumerSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ontology: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ontology_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    /// The colour the ontology already assigns this object type, so a table
    /// looks the same here as it does on the Model canvas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_icon: Option<String>,
    /// Edges with this table as an endpoint or as the edge's own storage.
    pub relations: usize,
    pub actions: usize,
    pub views: usize,
    pub queries: usize,
    /// The owning ontology is published to connected projects.
    pub exposed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableSummary {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<usize>,
    pub columns: Vec<ColumnSummary>,
    pub indexes: Vec<IndexSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageSummary>,
    pub consumers: ConsumerSummary,
    /// Set when this one table could not be read. Every other field is then
    /// whatever was resolvable before the failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl TableSummary {
    fn failed(name: String, consumers: ConsumerSummary, error: String) -> Self {
        Self {
            name,
            rows: None,
            columns: Vec::new(),
            indexes: Vec::new(),
            storage: None,
            consumers,
            error: Some(error),
        }
    }
}

/// Summarize `names` against `connection`. The overlay and saved query
/// manifests are read once and indexed, then each table contributes its own
/// metadata reads. Never returns Err: a broken manifest degrades to empty
/// consumer counts, a broken table degrades to `TableSummary::error`.
pub async fn summarize_tables(connection: &Connection, names: Vec<String>) -> Vec<TableSummary> {
    let overlays = list_overlays(connection).await.unwrap_or_default();
    let queries = list_saved_queries(connection).await.unwrap_or_default();

    stream::iter(names)
        .map(|name| {
            let consumers = collect_consumers(&overlays, &queries, &name);
            async move { summarize_one(connection, name, consumers).await }
        })
        .buffer_unordered(SUMMARY_CONCURRENCY)
        .collect()
        .await
}

async fn summarize_one(
    connection: &Connection,
    name: String,
    consumers: ConsumerSummary,
) -> TableSummary {
    let table = match connection.open_table(&name).execute().await {
        Ok(table) => table,
        Err(error) => {
            return TableSummary::failed(name, consumers, format!("Failed to open table: {error}"));
        }
    };

    let columns = match table.schema().await {
        Ok(schema) => schema.fields().iter().map(summarize_field).collect(),
        Err(error) => {
            return TableSummary::failed(
                name,
                consumers,
                format!("Failed to read schema: {error}"),
            );
        }
    };

    let indexes = table
        .list_indices()
        .await
        .map(|indices| {
            indices
                .into_iter()
                .map(|index| IndexSummary {
                    name: index.name,
                    index_type: index.index_type.to_string(),
                    columns: index.columns,
                })
                .collect()
        })
        .unwrap_or_default();

    // `stats()` walks every fragment, so it is the one call here that can be
    // slow or fail on a cold object store. Row count falls back to the cheap
    // metadata read when it does.
    let (rows, storage) = match table.stats().await {
        Ok(stats) => (
            Some(stats.num_rows),
            Some(StorageSummary {
                total_bytes: stats.total_bytes as u64,
                num_fragments: stats.fragment_stats.num_fragments,
                num_small_fragments: stats.fragment_stats.num_small_fragments,
            }),
        ),
        Err(_) => (table.count_rows(None).await.ok(), None),
    };

    TableSummary {
        name,
        rows,
        columns,
        indexes,
        storage,
        consumers,
        error: None,
    }
}

fn summarize_field(field: &std::sync::Arc<arrow_schema::Field>) -> ColumnSummary {
    let (family, vector_size) = classify(field.data_type());
    ColumnSummary {
        name: field.name().clone(),
        data_type: format!("{}", field.data_type()),
        family,
        nullable: field.is_nullable(),
        vector_size,
    }
}

fn classify(data_type: &DataType) -> (ColumnFamily, Option<u32>) {
    match data_type {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => (ColumnFamily::Text, None),
        DataType::Boolean => (ColumnFamily::Bool, None),
        DataType::Int8
        | DataType::Int16
        | DataType::Int32
        | DataType::Int64
        | DataType::UInt8
        | DataType::UInt16
        | DataType::UInt32
        | DataType::UInt64
        | DataType::Float16
        | DataType::Float32
        | DataType::Float64
        | DataType::Decimal128(_, _)
        | DataType::Decimal256(_, _) => (ColumnFamily::Number, None),
        DataType::Timestamp(_, _)
        | DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Duration(_)
        | DataType::Interval(_) => (ColumnFamily::Time, None),
        DataType::Binary
        | DataType::LargeBinary
        | DataType::BinaryView
        | DataType::FixedSizeBinary(_) => (ColumnFamily::Binary, None),
        // An embedding column is a fixed-size list of floats; a fixed-size list
        // of anything else is just a list.
        DataType::FixedSizeList(field, size) => {
            if matches!(
                field.data_type(),
                DataType::Float16 | DataType::Float32 | DataType::Float64
            ) {
                (ColumnFamily::Vector, u32::try_from(*size).ok())
            } else {
                (ColumnFamily::Struct, None)
            }
        }
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::ListView(_)
        | DataType::LargeListView(_)
        | DataType::Struct(_)
        | DataType::Map(_, _)
        | DataType::Union(_, _)
        | DataType::Dictionary(_, _) => (ColumnFamily::Struct, None),
        _ => (ColumnFamily::Other, None),
    }
}

fn collect_consumers(
    overlays: &[GraphOverlayDef],
    queries: &[crate::databases::workbench::saved_query::SavedQueryDef],
    table_name: &str,
) -> ConsumerSummary {
    let target = normalize_table_name(table_name);
    let mut summary = ConsumerSummary {
        queries: queries
            .iter()
            .filter(|query| sql_references_table(&query.sql, table_name))
            .count(),
        ..Default::default()
    };

    for overlay in overlays {
        let node = overlay
            .nodes
            .iter()
            .find(|node| normalize_table_name(&node.table) == target);

        let relations = overlay
            .edges
            .iter()
            .filter(|edge| {
                normalize_table_name(&edge.table) == target
                    || node.is_some_and(|node| {
                        edge.src_label == node.label || edge.dst_label == node.label
                    })
            })
            .count();

        let Some(node) = node else {
            // An edge can be stored in a table that no node maps. That table is
            // still part of the ontology, so it keeps the relationship count.
            if relations > 0 {
                summary.relations += relations;
                summary.ontology.get_or_insert_with(|| overlay.name.clone());
                summary
                    .ontology_id
                    .get_or_insert_with(|| overlay.id.clone());
                summary.exposed |= overlay.exposed;
            }
            continue;
        };

        summary.relations += relations;
        summary.actions += overlay
            .actions
            .iter()
            .filter(|action| object_type_matches(node, &action.object_type))
            .count();
        summary.views += overlay
            .object_views
            .iter()
            .filter(|view| object_type_matches(node, &view.object_type))
            .count();
        summary.exposed |= overlay.exposed;

        // A table is normally mapped by exactly one overlay; if several claim
        // it, the first one wins the identity and the rest still add counts.
        summary.ontology.get_or_insert_with(|| overlay.name.clone());
        summary
            .ontology_id
            .get_or_insert_with(|| overlay.id.clone());
        summary
            .object_type
            .get_or_insert_with(|| node.label.clone());
        if summary.object_color.is_none() {
            summary.object_color = style_string(node, "color");
        }
        if summary.object_icon.is_none() {
            summary.object_icon = style_string(node, "icon");
        }
    }

    summary
}

fn style_string(node: &NodeMappingDef, key: &str) -> Option<String> {
    node.style
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn object_type_matches(node: &NodeMappingDef, object_type: &str) -> bool {
    node.id.as_deref() == Some(object_type)
        || node.api_name.as_deref() == Some(object_type)
        || node.label == object_type
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databases::graph::lancegraph::{
        EdgeMappingDef, ObjectViewDef, OntologyActionDef, PropertyProjectionMode,
    };
    use crate::databases::workbench::saved_query::{
        SavedQueryDef, SavedQueryKind, SavedQuerySurface,
    };
    use arrow_schema::{Field, TimeUnit};
    use std::sync::Arc;

    fn node(label: &str, table: &str, color: &str) -> NodeMappingDef {
        NodeMappingDef {
            id: None,
            api_name: None,
            label: label.to_string(),
            table: table.to_string(),
            id_column: "id".to_string(),
            display_column: None,
            property_columns: Vec::new(),
            style: serde_json::json!({ "color": color, "icon": "database" }),
        }
    }

    fn edge(label: &str, table: &str, src: &str, dst: &str) -> EdgeMappingDef {
        EdgeMappingDef {
            id: None,
            api_name: None,
            label: label.to_string(),
            table: table.to_string(),
            src_column: "src".to_string(),
            dst_column: "dst".to_string(),
            src_label: src.to_string(),
            dst_label: dst.to_string(),
            src_node_column: None,
            dst_node_column: None,
            containment: false,
            dst_ontology: None,
            dst_binding_id: None,
            property_columns: Vec::new(),
            style: serde_json::Value::Null,
        }
    }

    fn overlay(nodes: Vec<NodeMappingDef>, edges: Vec<EdgeMappingDef>) -> GraphOverlayDef {
        GraphOverlayDef {
            id: "ont-1".to_string(),
            name: "Feedback".to_string(),
            description: None,
            nodes,
            edges,
            object_views: Vec::new(),
            actions: Vec::new(),
            exposed: true,
            bindings_enabled: true,
            property_projection_mode: PropertyProjectionMode::Dynamic,
            default_limit: 100,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    fn query(name: &str, sql: &str) -> SavedQueryDef {
        SavedQueryDef {
            id: name.to_string(),
            app_id: "app".to_string(),
            name: name.to_string(),
            description: None,
            kind: SavedQueryKind::Query,
            surface: SavedQuerySurface::Native,
            overlay_id: None,
            sql: sql.to_string(),
            param_schema: None,
            viz_config: None,
            default_limit: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn maps_a_table_to_its_object_type_and_counts_what_reads_it() {
        let mut overlay = overlay(
            vec![
                node("Submission", "feedback_submission", "#6ea8fe"),
                node("Reporter", "feedback_reporter", "#a78bfa"),
            ],
            vec![edge(
                "reported_by",
                "feedback_submission",
                "Submission",
                "Reporter",
            )],
        );
        overlay.actions.push(OntologyActionDef {
            id: "a1".to_string(),
            name: "Triage".to_string(),
            description: None,
            object_type: "Submission".to_string(),
            board_id: "b1".to_string(),
            board_version: None,
            start_node_id: None,
            event_id: None,
            enabled: true,
            allow_bulk: false,
            parameter_schema: None,
            exposed: true,
        });
        overlay.object_views.push(ObjectViewDef {
            object_type: "Submission".to_string(),
            title_property: Some("title".to_string()),
            prominent_properties: vec!["status".to_string()],
        });

        let queries = vec![
            query("Weekly volume", "SELECT count(*) FROM feedback_submission"),
            query("Unrelated", "SELECT * FROM feedback_submission_archive"),
        ];

        let consumers = collect_consumers(&[overlay], &queries, "feedback_submission");

        assert_eq!(consumers.object_type.as_deref(), Some("Submission"));
        assert_eq!(consumers.ontology.as_deref(), Some("Feedback"));
        assert_eq!(consumers.object_color.as_deref(), Some("#6ea8fe"));
        assert_eq!(consumers.actions, 1);
        assert_eq!(consumers.views, 1);
        assert_eq!(consumers.relations, 1);
        // `feedback_submission_archive` must not count as a reference.
        assert_eq!(consumers.queries, 1);
        assert!(consumers.exposed);
    }

    #[test]
    fn a_table_no_overlay_names_reports_no_object_type() {
        let overlay = overlay(
            vec![node("Submission", "feedback_submission", "#6ea8fe")],
            vec![],
        );
        let consumers = collect_consumers(&[overlay], &[], "feedback_search");

        assert!(consumers.object_type.is_none());
        assert!(consumers.ontology.is_none());
        assert_eq!(consumers.relations, 0);
        assert!(!consumers.exposed);
    }

    #[test]
    fn overlay_mappings_match_across_the_lance_suffix() {
        let overlay = overlay(
            vec![node("Submission", "feedback_submission.lance", "#6ea8fe")],
            vec![],
        );
        let consumers = collect_consumers(&[overlay], &[], "feedback_submission");
        assert_eq!(consumers.object_type.as_deref(), Some("Submission"));
    }

    #[test]
    fn classifies_embedding_columns_apart_from_plain_lists() {
        let embedding =
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 1024);
        assert_eq!(classify(&embedding), (ColumnFamily::Vector, Some(1024)));

        let coordinates =
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Int32, true)), 2);
        assert_eq!(classify(&coordinates), (ColumnFamily::Struct, None));

        assert_eq!(
            classify(&DataType::Timestamp(TimeUnit::Millisecond, None)).0,
            ColumnFamily::Time
        );
        assert_eq!(classify(&DataType::Utf8).0, ColumnFamily::Text);
        assert_eq!(classify(&DataType::Boolean).0, ColumnFamily::Bool);
        assert_eq!(classify(&DataType::Int64).0, ColumnFamily::Number);
    }
}
