//! What a public store page may show about an app's tables.
//!
//! The visitor is treated as a reader with no role attributes: every column with read
//! attributes is left out entirely. Columns labelled personal (PII, or classified confidential
//! or higher) stay listed but are never read for samples. Samples only select the columns they
//! show, so personal, vector, binary and geometry values never leave the table.

use crate::databases::column_access::{hidden_columns, is_hidden};
use crate::databases::graph::lancegraph::{GraphOverlayDef, list_overlays};
use crate::databases::table_cascade::normalize_table_name;
use crate::databases::table_summary::{ColumnFamily, IndexSummary, style_string, summarize_field};
use flow_like_storage_contracts::column_metadata::{
    ColumnMetadata, ColumnReader, DataClassification,
};
use futures::stream::{self, StreamExt, TryStreamExt};
use lancedb::Connection;
use lancedb::query::{ExecutableQuery, QueryBase};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

pub const SAMPLE_ROWS: usize = 5;
const TABLE_CONCURRENCY: usize = 4;
const TABLE_BUDGET: Duration = Duration::from_secs(6);
const MAX_SAMPLE_TEXT: usize = 160;

#[derive(Debug, Clone, Copy, Default)]
pub struct ShowcaseTableOptions {
    pub row_counts: bool,
    pub schema: bool,
    pub samples: bool,
    pub object_types: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowcaseColumn {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub data_type: String,
    pub family: ColumnFamily,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vector_size: Option<u32>,
    pub personal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowcaseSample {
    /// One row per entry, values in `ShowcaseTable::columns` order. Columns that are not read
    /// (personal, vector, binary, geometry) hold null.
    pub rows: Vec<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowcaseObjectType {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowcaseTable {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rows: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<ShowcaseColumn>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexes: Option<Vec<IndexSummary>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sample: Option<ShowcaseSample>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_type: Option<ShowcaseObjectType>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShowcaseObjectLink {
    pub from_table: String,
    pub to_table: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ShowcaseData {
    pub tables: Vec<ShowcaseTable>,
    pub links: Vec<ShowcaseObjectLink>,
}

/// The visitor's view of `names`, in the given order. A table that cannot be read within its
/// budget is left out rather than failing the page.
pub async fn showcase_tables(
    connection: &Connection,
    names: Vec<String>,
    options: ShowcaseTableOptions,
) -> ShowcaseData {
    let overlays = if options.object_types {
        list_overlays(connection).await.unwrap_or_default()
    } else {
        Vec::new()
    };

    let tables: Vec<ShowcaseTable> = stream::iter(names)
        .map(|name| async move {
            let read = tokio::time::timeout(
                TABLE_BUDGET,
                showcase_one(connection, name.clone(), options),
            )
            .await;
            match read {
                Ok(Ok(table)) => Some(table),
                Ok(Err(error)) => {
                    tracing::warn!(table = %name, %error, "showcase: table skipped");
                    None
                }
                Err(_) => {
                    tracing::warn!(table = %name, "showcase: table read timed out");
                    None
                }
            }
        })
        .buffered(TABLE_CONCURRENCY)
        .filter_map(|table| async move { table })
        .collect()
        .await;

    if overlays.is_empty() {
        return ShowcaseData {
            tables,
            links: Vec::new(),
        };
    }
    let shown: Vec<&str> = tables.iter().map(|table| table.name.as_str()).collect();
    let links = object_links(&overlays, &shown);
    let tables = tables
        .into_iter()
        .map(|mut table| {
            table.object_type = object_type(&overlays, &table.name);
            table
        })
        .collect();
    ShowcaseData { tables, links }
}

fn is_personal(metadata: &ColumnMetadata) -> bool {
    metadata.sensitivity.pii
        || metadata
            .sensitivity
            .classification
            .is_some_and(|level| level >= DataClassification::Confidential)
}

fn samples_family(family: ColumnFamily) -> bool {
    !matches!(
        family,
        ColumnFamily::Vector | ColumnFamily::Binary | ColumnFamily::Geo
    )
}

async fn showcase_one(
    connection: &Connection,
    name: String,
    options: ShowcaseTableOptions,
) -> flow_like_types::Result<ShowcaseTable> {
    let table = connection.open_table(&name).execute().await?;
    let rows = if options.row_counts || options.samples {
        Some(table.count_rows(None).await? as u64)
    } else {
        None
    };

    let needs_schema = options.schema || options.samples;
    let mut columns = Vec::new();
    let mut indexes = None;
    if needs_schema {
        let schema = table.schema().await?;
        let hidden = hidden_columns(&schema, &ColumnReader::Attributes(Default::default()));
        for field in schema.fields() {
            if is_hidden(&hidden, field.name()) {
                continue;
            }
            let summary = summarize_field(field);
            let metadata = ColumnMetadata::from_field_metadata(field.metadata());
            columns.push(ShowcaseColumn {
                personal: is_personal(&metadata),
                name: summary.name,
                display_name: metadata.display_name,
                description: metadata.description,
                data_type: summary.data_type,
                family: summary.family,
                nullable: summary.nullable,
                vector_size: summary.vector_size,
            });
        }
        if options.schema {
            indexes = Some(
                crate::databases::vector::lancedb::list_table_indices(&table)
                    .await
                    .map(|found| {
                        found
                            .into_iter()
                            .filter(|index| {
                                !index
                                    .columns
                                    .iter()
                                    .any(|column| is_hidden(&hidden, column))
                            })
                            .map(|index| IndexSummary {
                                name: index.name,
                                index_type: index.index_type.to_string(),
                                columns: index.columns,
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            );
        }
    }

    let sample = if options.samples && rows.unwrap_or(0) > 0 {
        let readable: Vec<String> = columns
            .iter()
            .filter(|column| !column.personal && samples_family(column.family))
            .map(|column| column.name.clone())
            .collect();
        if readable.is_empty() {
            None
        } else {
            let batches = table
                .query()
                .limit(SAMPLE_ROWS)
                .select(lancedb::query::Select::Columns(readable.clone()))
                .execute()
                .await?
                .try_collect::<Vec<_>>()
                .await?;
            let records = crate::databases::vector::lancedb::record_batches_to_vec(Some(batches))?;
            Some(ShowcaseSample {
                rows: records
                    .iter()
                    .map(|record| {
                        columns
                            .iter()
                            .map(|column| {
                                if readable.contains(&column.name) {
                                    trim_value(record.get(&column.name).cloned())
                                } else {
                                    Value::Null
                                }
                            })
                            .collect()
                    })
                    .collect(),
            })
        }
    } else {
        None
    };

    Ok(ShowcaseTable {
        name,
        rows: rows.filter(|_| options.row_counts),
        columns: options.schema.then_some(columns),
        indexes,
        sample,
        object_type: None,
    })
}

/// Long text and nested values are shortened so five rows stay a preview.
fn trim_value(value: Option<Value>) -> Value {
    match value.unwrap_or(Value::Null) {
        Value::String(text) => Value::String(truncate(&text)),
        nested @ (Value::Array(_) | Value::Object(_)) => {
            let text = nested.to_string();
            if text.chars().count() > MAX_SAMPLE_TEXT {
                Value::String(truncate(&text))
            } else {
                nested
            }
        }
        other => other,
    }
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_SAMPLE_TEXT {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(MAX_SAMPLE_TEXT).collect();
    cut.push('…');
    cut
}

fn object_type(overlays: &[GraphOverlayDef], table: &str) -> Option<ShowcaseObjectType> {
    let target = normalize_table_name(table);
    overlays.iter().find_map(|overlay| {
        overlay
            .nodes
            .iter()
            .find(|node| normalize_table_name(&node.table) == target)
            .map(|node| ShowcaseObjectType {
                name: node.label.clone(),
                color: style_string(node, "color"),
                icon: style_string(node, "icon"),
            })
    })
}

/// Links between object types whose tables are both shown. An edge whose end lives in another
/// ontology or a remote import is left out.
fn object_links(overlays: &[GraphOverlayDef], shown: &[&str]) -> Vec<ShowcaseObjectLink> {
    let shown_table = |table: &str| {
        let target = normalize_table_name(table);
        shown
            .iter()
            .find(|name| normalize_table_name(name) == target)
            .map(|name| name.to_string())
    };
    let mut links = Vec::new();
    for overlay in overlays {
        let table_of = |label: &str| {
            overlay
                .nodes
                .iter()
                .find(|node| node.label == label)
                .and_then(|node| shown_table(&node.table))
        };
        for edge in &overlay.edges {
            if edge.dst_ontology.is_some() || edge.dst_binding_id.is_some() {
                continue;
            }
            let (Some(from_table), Some(to_table)) =
                (table_of(&edge.src_label), table_of(&edge.dst_label))
            else {
                continue;
            };
            let link = ShowcaseObjectLink {
                from_table,
                to_table,
                label: edge.label.clone(),
            };
            if !links.contains(&link) {
                links.push(link);
            }
        }
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage_contracts::column_metadata::DataSensitivity;

    fn metadata(pii: bool, classification: Option<DataClassification>) -> ColumnMetadata {
        ColumnMetadata {
            sensitivity: DataSensitivity {
                classification,
                pii,
            },
            ..Default::default()
        }
    }

    #[test]
    fn personal_means_pii_or_confidential_and_up() {
        assert!(is_personal(&metadata(true, None)));
        assert!(is_personal(&metadata(
            false,
            Some(DataClassification::Confidential)
        )));
        assert!(is_personal(&metadata(
            false,
            Some(DataClassification::Restricted)
        )));
        assert!(!is_personal(&metadata(
            false,
            Some(DataClassification::Internal)
        )));
        assert!(!is_personal(&metadata(false, None)));
    }

    #[test]
    fn long_values_are_shortened() {
        let long = "x".repeat(400);
        let Value::String(text) = trim_value(Some(Value::String(long))) else {
            panic!("expected text");
        };
        assert_eq!(text.chars().count(), MAX_SAMPLE_TEXT + 1);
        assert_eq!(trim_value(None), Value::Null);
        assert_eq!(
            trim_value(Some(serde_json::json!({"a": 1}))),
            serde_json::json!({"a": 1})
        );
    }
}
