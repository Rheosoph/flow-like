mod expansion;
mod validation;

pub use validation::{MappingValidation, ValidationReport, validate_overlay_definition};

use super::{
    GraphAnalyticsResult, GraphLabelInfo, GraphPathsResult, GraphPropertyInfo, GraphSchemaResult,
    GraphStore, SubgraphEdge, SubgraphNode, SubgraphResult, TraversalDirection,
};
use crate::arrow_utils::record_batch_to_value;
use datafusion::prelude::SessionContext;
use flow_like_types::{Result, Value, anyhow, async_trait};
use futures::TryStreamExt;
use lance_graph::{CypherQuery, GraphConfig};
use lancedb::Connection;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::table::datafusion::BaseTableAdapter;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

const MAX_QUERY_DEPTH: usize = 5;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_CONCURRENT_QUERIES: usize = 4;
/// Hard ceiling on rows serialized per query. High enough for the largest
/// explorer view (10k), low enough that a single request cannot materialize
/// millions of rows into JSON.
const MAX_QUERY_LIMIT: usize = 50_000;
const DEFAULT_ANALYTICS_EDGE_LIMIT: usize = 50_000;

/// Query permits are process-wide: stores are constructed per request, so a
/// per-instance semaphore would never observe concurrent load. First
/// initialization wins; later stores share the same pool.
static GLOBAL_QUERY_SEMAPHORE: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();

pub(crate) fn global_query_semaphore(max_concurrent: usize) -> Arc<tokio::sync::Semaphore> {
    GLOBAL_QUERY_SEMAPHORE
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(max_concurrent.max(1))))
        .clone()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OverlayRow {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub definition_json: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraphOverlayDef {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub nodes: Vec<NodeMappingDef>,
    pub edges: Vec<EdgeMappingDef>,
    #[serde(default)]
    pub object_views: Vec<ObjectViewDef>,
    #[serde(default)]
    pub actions: Vec<OntologyActionDef>,
    #[serde(default)]
    pub exposed: bool,
    #[serde(default)]
    pub bindings_enabled: bool,
    /// Controls the meaning of an empty `property_columns` list. Local
    /// overlays stay dynamic (empty = all scalar columns); installed remote
    /// contracts are frozen (empty = deliberately no additional properties).
    #[serde(default)]
    pub property_projection_mode: PropertyProjectionMode,
    pub default_limit: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropertyProjectionMode {
    #[default]
    Dynamic,
    Frozen,
}

/// A sanitized ontology contract pinned into a consuming project.
///
/// Imports live separately from graph overlays because their table mappings
/// point at a connected project's database, not the consuming project's local
/// database.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RemoteOntologyImportDef {
    pub id: String,
    pub target_app_id: String,
    pub remote_ontology_id: String,
    pub contract: GraphOverlayDef,
    pub source_updated_at: String,
    #[serde(default = "default_true")]
    pub bindings_enabled: bool,
    pub installed_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ObjectViewDef {
    pub object_type: String,
    #[serde(default)]
    pub title_property: Option<String>,
    #[serde(default)]
    pub prominent_properties: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OntologyActionDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub object_type: String,
    pub board_id: String,
    #[serde(default)]
    pub board_version: Option<[u32; 3]>,
    #[serde(default)]
    pub start_node_id: Option<String>,
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub allow_bulk: bool,
    #[serde(default)]
    pub parameter_schema: Option<serde_json::Value>,
    /// Per-action exposure to connected projects. Absent (legacy) or `true`
    /// keeps the current whole-ontology behavior; `false` hides the action from
    /// remote contracts and rejects connected-app invocation while still
    /// allowing local execution.
    #[serde(default = "default_true")]
    pub exposed: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PropertyColumnDef {
    pub name: String,
    pub data_type: String,
    #[serde(default)]
    pub nullable: bool,
}

/// The exact object surface a managed ontology action was authorized to read
/// when its protected event binding was materialized.
///
/// Unlike an ordinary local overlay, this projection never treats an empty
/// property list as a schema wildcard. The concrete identity and columns are
/// stored in the event config and covered by its contract hash.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GovernedObjectProjection {
    pub table: String,
    pub identity_column: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeMappingDef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub api_name: Option<String>,
    pub label: String,
    pub table: String,
    pub id_column: String,
    pub display_column: Option<String>,
    pub property_columns: Vec<PropertyColumnDef>,
    pub style: serde_json::Value,
}

fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(left, _)| *left);
            serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_json(value)))
                    .collect(),
            )
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonical_json).collect())
        }
        _ => value.clone(),
    }
}

/// Hashes every saved field that can change what a governed action may read or
/// execute. The hash is stored in the protected managed event and checked at
/// invocation, so direct edits to ontology metadata cannot widen the contract.
pub fn ontology_action_contract_hash(
    ontology_id: &str,
    exposed: bool,
    action: &OntologyActionDef,
    object: &NodeMappingDef,
    projection: &GovernedObjectProjection,
) -> Result<String> {
    let contract = serde_json::json!({
        "ontology_id": ontology_id,
        "exposed": exposed,
        "action": {
            "id": action.id,
            "name": action.name,
            "description": action.description,
            "object_type": action.object_type,
            "board_id": action.board_id,
            "board_version": action.board_version,
            "start_node_id": action.start_node_id,
            "enabled": action.enabled,
            "allow_bulk": action.allow_bulk,
            "parameter_schema": action.parameter_schema,
            "exposed": action.exposed,
        },
        "object": {
            "id": object.id,
            "api_name": object.api_name,
            "label": object.label,
            "table": object.table,
            "id_column": object.id_column,
            "display_column": object.display_column,
            "property_columns": object.property_columns,
        },
        "resolved_object_projection": projection,
    });
    let encoded = serde_json::to_vec(&canonical_json(&contract))?;
    Ok(blake3::hash(&encoded).to_hex().to_string())
}

pub fn ontology_action_contract_hash_for_overlay(
    overlay: &GraphOverlayDef,
    action: &OntologyActionDef,
) -> Result<String> {
    let object = overlay
        .nodes
        .iter()
        .find(|object| {
            object.id.as_deref() == Some(action.object_type.as_str())
                || object.api_name.as_deref() == Some(action.object_type.as_str())
                || object.label == action.object_type
        })
        .ok_or_else(|| {
            anyhow!(
                "Ontology action '{}' references unknown object type '{}'",
                action.id,
                action.object_type
            )
        })?;
    let projection = declared_governed_object_projection(overlay, action)?;
    ontology_action_contract_hash(&overlay.id, overlay.exposed, action, object, &projection)
}

pub fn ontology_action_contracts_equal(
    left: &GraphOverlayDef,
    right: &GraphOverlayDef,
) -> Result<bool> {
    if left.actions.len() != right.actions.len() {
        return Ok(false);
    }
    for left_action in &left.actions {
        let Some(right_action) = right
            .actions
            .iter()
            .find(|action| action.id == left_action.id)
        else {
            return Ok(false);
        };
        if ontology_action_contract_hash_for_overlay(left, left_action)?
            != ontology_action_contract_hash_for_overlay(right, right_action)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EdgeMappingDef {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub api_name: Option<String>,
    pub label: String,
    pub table: String,
    pub src_column: String,
    pub dst_column: String,
    pub src_label: String,
    pub dst_label: String,
    #[serde(default)]
    pub src_node_column: Option<String>,
    #[serde(default)]
    pub dst_node_column: Option<String>,
    /// Marks a hierarchy/drill-down spine edge (src_label = parent, dst_label = child).
    #[serde(default)]
    pub containment: bool,
    /// Child objects live in another local overlay (its id).
    #[serde(default)]
    pub dst_ontology: Option<String>,
    /// Child objects live in an installed remote ontology (its import id).
    #[serde(default)]
    pub dst_binding_id: Option<String>,
    pub property_columns: Vec<PropertyColumnDef>,
    pub style: serde_json::Value,
}

pub struct CypherSafetyConfig {
    pub max_depth: usize,
    pub max_limit: usize,
    pub timeout_ms: u64,
    pub max_concurrent: usize,
}

impl Default for CypherSafetyConfig {
    fn default() -> Self {
        CypherSafetyConfig {
            max_depth: MAX_QUERY_DEPTH,
            max_limit: MAX_QUERY_LIMIT,
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_concurrent: MAX_CONCURRENT_QUERIES,
        }
    }
}

pub struct LanceGraphStore {
    connection: Connection,
    overlay: GraphOverlayDef,
    graph_config: GraphConfig,
    safety: CypherSafetyConfig,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl LanceGraphStore {
    pub async fn new(
        connection: Connection,
        overlay: GraphOverlayDef,
        safety: Option<CypherSafetyConfig>,
    ) -> Result<Self> {
        let safety = safety.unwrap_or_default();
        let semaphore = global_query_semaphore(safety.max_concurrent);

        let graph_config = build_graph_config(&connection, &overlay).await?;

        Ok(LanceGraphStore {
            connection,
            overlay,
            graph_config,
            safety,
            semaphore,
        })
    }

    pub fn overlay(&self) -> &GraphOverlayDef {
        &self.overlay
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Upsert node rows into the underlying table of a node label, merging on the label's id
    /// column. Rows must carry the id column value; existing ids are updated, new ids inserted.
    /// Returns the number of rows written. Shared by the write API and the graph write nodes.
    pub async fn upsert_nodes(&self, label: &str, rows: Vec<Value>) -> Result<usize> {
        let node_def = self
            .overlay
            .nodes
            .iter()
            .find(|node| node.label == label)
            .ok_or_else(|| anyhow!("Node label '{}' not found in overlay", label))?;
        let table_name = node_def.table.clone();
        let id_column = node_def.id_column.clone();
        self.merge_upsert(&table_name, &[id_column.as_str()], rows)
            .await
    }

    /// Upsert edge rows into the underlying table of an edge label, merging on the label's
    /// (source, target) columns so re-adding an existing pair updates it instead of duplicating.
    /// Returns the number of rows written.
    pub async fn upsert_edges(&self, label: &str, rows: Vec<Value>) -> Result<usize> {
        let edge_def = self
            .overlay
            .edges
            .iter()
            .find(|edge| edge.label == label)
            .ok_or_else(|| anyhow!("Edge label '{}' not found in overlay", label))?;
        let table_name = edge_def.table.clone();
        let src_column = edge_def.src_column.clone();
        let dst_column = edge_def.dst_column.clone();
        self.merge_upsert(
            &table_name,
            &[src_column.as_str(), dst_column.as_str()],
            rows,
        )
        .await
    }

    async fn merge_upsert(
        &self,
        table_name: &str,
        keys: &[&str],
        rows: Vec<Value>,
    ) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let count = rows.len();
        let table = self
            .connection
            .open_table(table_name)
            .execute()
            .await
            .map_err(|error| anyhow!("Failed to open table '{}': {}", table_name, error))?;

        let batch = crate::arrow_utils::value_to_record_batch(rows)?;
        let schema = batch.schema();
        let reader: Box<dyn crate::arrow::record_batch::RecordBatchReader + Send> = Box::new(
            crate::arrow::record_batch::RecordBatchIterator::new(vec![Ok(batch)], schema),
        );

        let mut merger = table.merge_insert(keys);
        merger
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        merger
            .execute(reader)
            .await
            .map_err(|error| anyhow!("Failed to upsert into '{}': {}", table_name, error))?;
        Ok(count)
    }

    fn enforce_limit(&self, limit: Option<usize>) -> usize {
        let user_limit = limit.unwrap_or(self.overlay.default_limit);
        user_limit.min(self.safety.max_limit)
    }

    async fn execute_cypher_with_safety(
        &self,
        query: &str,
        params: HashMap<String, serde_json::Value>,
        limit: usize,
    ) -> Result<arrow::array::RecordBatch> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;

        let parsed =
            CypherQuery::new(query).map_err(|e| anyhow!("Failed to parse Cypher query: {}", e))?;
        preflight_cypher(parsed.ast(), &self.safety)?;
        let limited_query = if parsed.ast().limit.is_some() {
            query.trim().to_string()
        } else {
            append_limit_clause(query, limit)
        };
        let cypher = CypherQuery::new(&limited_query)
            .map_err(|e| anyhow!("Failed to parse Cypher query: {}", e))?
            .with_config(self.graph_config.clone())
            .with_parameters(params);

        let ctx = self.build_cypher_context().await?;

        let result = tokio::time::timeout(
            std::time::Duration::from_millis(self.safety.timeout_ms),
            cypher.execute_with_context(ctx),
        )
        .await
        .map_err(|_| anyhow!("Query timed out after {}ms", self.safety.timeout_ms))?
        .map_err(|e| anyhow!("Cypher execution failed: {}", e))?;

        let result = if result.num_rows() > limit {
            result.slice(0, limit)
        } else {
            result
        };

        Ok(result)
    }
}

#[async_trait]
impl GraphStore for LanceGraphStore {
    async fn cypher(&self, query: &str, params: Value, limit: Option<usize>) -> Result<Vec<Value>> {
        let limit = self.enforce_limit(limit);
        let params_map: HashMap<String, serde_json::Value> = match params {
            Value::Object(map) => map.into_iter().collect(),
            Value::Null => HashMap::new(),
            _ => return Err(anyhow!("params must be an object or null")),
        };

        let batch = self
            .execute_cypher_with_safety(query, params_map, limit)
            .await?;

        record_batch_to_value(&batch)
    }

    async fn sql(&self, query: &str, limit: Option<usize>) -> Result<Vec<Value>> {
        let limit = self.enforce_limit(limit);
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;

        validate_readonly_sql(query)?;
        let timeout = std::time::Duration::from_millis(self.safety.timeout_ms);
        let ctx = self.build_query_context(true).await?;

        let df = tokio::time::timeout(timeout, ctx.sql(query))
            .await
            .map_err(|_| anyhow!("SQL planning timed out after {}ms", self.safety.timeout_ms))??
            .limit(0, Some(limit))?;
        let batches = tokio::time::timeout(timeout, df.collect())
            .await
            .map_err(|_| anyhow!("SQL query timed out after {}ms", self.safety.timeout_ms))??;

        let mut results = Vec::new();
        for batch in &batches {
            let vals = record_batch_to_value(batch)?;
            results.extend(vals);
            if results.len() >= limit {
                results.truncate(limit);
                break;
            }
        }

        Ok(results)
    }

    async fn neighbors(
        &self,
        label: &str,
        id: Value,
        depth: usize,
        direction: TraversalDirection,
        limit: Option<usize>,
    ) -> Result<SubgraphResult> {
        let limit = self.enforce_limit(limit);
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;
        self.expand_subgraph(
            vec![(label.to_string(), id)],
            depth.max(1),
            direction,
            limit,
        )
        .await
    }

    async fn subgraph(
        &self,
        seeds: Vec<(String, Value)>,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<SubgraphResult> {
        let limit = self.enforce_limit(limit);

        if seeds.is_empty() {
            return self.full_subgraph(limit).await;
        }

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;
        self.expand_subgraph(seeds, depth.max(1), TraversalDirection::Both, limit)
            .await
    }

    async fn overlay_children(
        &self,
        parent_label: &str,
        parent_id: Value,
        limit: Option<usize>,
    ) -> Result<SubgraphResult> {
        let limit = self.enforce_limit(limit);
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;
        self.overlay_children_impl(parent_label, parent_id, limit)
            .await
    }

    async fn search_nodes(&self, query: &str, limit: Option<usize>) -> Result<Vec<SubgraphNode>> {
        let limit = self.enforce_limit(limit);
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Ok(Vec::new());
        }

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;

        let ctx = self.build_query_context(false).await?;
        let pattern = sql_string_literal(&format!("%{}%", trimmed_query));
        let normalized_query = trimmed_query.to_lowercase();
        let per_label_limit = limit.min(25);
        let mut schema_cache: HashMap<String, Vec<String>> = HashMap::new();

        let mut matches = Vec::new();
        let mut seen_node_ids = HashSet::new();

        for node in &self.overlay.nodes {
            let searchable_columns = self.searchable_columns_for_label(&node.label)?;
            if searchable_columns.is_empty() {
                continue;
            }

            let id_col = self.find_id_column_for_label(&node.label)?;
            let excluded = HashSet::from([id_col.clone()]);
            let always_include = node
                .display_column
                .clone()
                .filter(|column| *column != id_col)
                .into_iter()
                .collect::<Vec<_>>();
            let prop_names = resolve_property_names(
                &self.connection,
                &node.table,
                &node.property_columns,
                self.overlay.property_projection_mode,
                &mut schema_cache,
                &excluded,
                &always_include,
            )
            .await?;
            let mut projected = vec![id_col.clone()];
            projected.extend(prop_names);
            projected.extend(searchable_columns.iter().cloned());
            let mut seen_columns = HashSet::new();
            projected.retain(|column| seen_columns.insert(column.clone()));
            let projection = projected
                .iter()
                .map(|column| quote_identifier(column))
                .collect::<Vec<_>>()
                .join(", ");

            let where_clause = searchable_columns
                .iter()
                .map(|column| {
                    let quoted = quote_identifier(column);
                    format!("CAST({quoted} AS VARCHAR) ILIKE {pattern}")
                })
                .collect::<Vec<_>>()
                .join(" OR ");

            let sql = format!(
                "SELECT {} FROM {} WHERE {} LIMIT {}",
                projection,
                quote_identifier(&node.table),
                where_clause,
                per_label_limit,
            );

            let df = ctx.sql(&sql).await?;
            let batches = df.collect().await?;

            for batch in &batches {
                let rows = record_batch_to_value(batch)?;
                for result_node in self.rows_to_nodes(&node.label, rows)? {
                    if seen_node_ids.insert(result_node.id.clone()) {
                        matches.push(result_node);
                    }
                }
            }
        }

        matches.sort_by_key(|node| search_match_rank(node, &normalized_query));
        matches.truncate(limit);
        Ok(matches)
    }

    async fn schema(&self) -> Result<GraphSchemaResult> {
        let mut node_labels = Vec::new();
        let mut edge_labels = Vec::new();

        for node_def in &self.overlay.nodes {
            let table = self
                .connection
                .open_table(&node_def.table)
                .execute()
                .await
                .map_err(|e| anyhow!("Failed to open table '{}': {}", node_def.table, e))?;

            let schema = table
                .schema()
                .await
                .map_err(|e| anyhow!("Failed to get schema for '{}': {}", node_def.table, e))?;

            let properties = schema
                .fields()
                .iter()
                .map(|f| GraphPropertyInfo {
                    name: f.name().clone(),
                    data_type: format!("{:?}", f.data_type()),
                    nullable: f.is_nullable(),
                })
                .collect();

            node_labels.push(GraphLabelInfo {
                label: node_def.label.clone(),
                table: node_def.table.clone(),
                properties,
            });
        }

        for edge_def in &self.overlay.edges {
            let table = self
                .connection
                .open_table(&edge_def.table)
                .execute()
                .await
                .map_err(|e| anyhow!("Failed to open table '{}': {}", edge_def.table, e))?;

            let schema = table
                .schema()
                .await
                .map_err(|e| anyhow!("Failed to get schema for '{}': {}", edge_def.table, e))?;

            let properties = schema
                .fields()
                .iter()
                .map(|f| GraphPropertyInfo {
                    name: f.name().clone(),
                    data_type: format!("{:?}", f.data_type()),
                    nullable: f.is_nullable(),
                })
                .collect();

            edge_labels.push(GraphLabelInfo {
                label: edge_def.label.clone(),
                table: edge_def.table.clone(),
                properties,
            });
        }

        Ok(GraphSchemaResult {
            node_labels,
            edge_labels,
        })
    }

    async fn sample(&self, label: &str, n: usize) -> Result<Vec<Value>> {
        sample_overlay(&self.connection, &self.overlay, label, n).await
    }

    async fn shortest_paths(
        &self,
        from: (String, Value),
        to: (String, Value),
        max_depth: usize,
        limit: Option<usize>,
    ) -> Result<GraphPathsResult> {
        let limit = self.enforce_limit(limit);
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;
        self.shortest_paths_impl(from, to, max_depth, limit).await
    }

    async fn analytics(&self, limit: Option<usize>) -> Result<GraphAnalyticsResult> {
        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|e| anyhow!("Semaphore acquire failed: {}", e))?;
        self.analytics_impl(limit.unwrap_or(DEFAULT_ANALYTICS_EDGE_LIMIT))
            .await
    }
}

impl LanceGraphStore {
    fn find_id_column_for_label(&self, label: &str) -> Result<String> {
        effective_node_id_column(&self.overlay, label)
            .ok_or_else(|| anyhow!("Label '{}' not found in overlay node mappings", label))
    }

    fn find_display_column_for_label(&self, label: &str) -> Option<String> {
        self.overlay
            .nodes
            .iter()
            .find(|n| n.label == label)
            .and_then(|n| n.display_column.clone())
    }

    fn searchable_columns_for_label(&self, label: &str) -> Result<Vec<String>> {
        let node = self
            .overlay
            .nodes
            .iter()
            .find(|node| node.label == label)
            .ok_or_else(|| anyhow!("Label '{}' not found in overlay node mappings", label))?;

        let mut columns = Vec::new();
        let mut seen_columns = HashSet::new();

        if seen_columns.insert(node.id_column.clone()) {
            columns.push(node.id_column.clone());
        }

        if let Some(display_column) = node.display_column.clone()
            && seen_columns.insert(display_column.clone())
        {
            columns.push(display_column);
        }

        for property in &node.property_columns {
            let is_text =
                property.data_type.contains("Utf8") || property.data_type.contains("String");
            if is_text && seen_columns.insert(property.name.clone()) {
                columns.push(property.name.clone());
            }
        }

        Ok(columns)
    }

    async fn table_adapter(
        &self,
        table_name: &str,
        cache: &mut HashMap<String, Arc<BaseTableAdapter>>,
    ) -> Result<Arc<BaseTableAdapter>> {
        if let Some(adapter) = cache.get(table_name) {
            return Ok(adapter.clone());
        }

        let adapter = open_table_adapter(&self.connection, table_name).await?;
        cache.insert(table_name.to_string(), adapter.clone());
        Ok(adapter)
    }

    async fn build_cypher_context(&self) -> Result<SessionContext> {
        let ctx = SessionContext::new();
        let mut adapters: HashMap<String, Arc<BaseTableAdapter>> = HashMap::new();
        let mut registered_labels = HashSet::new();

        for node in &self.overlay.nodes {
            let label = node.label.to_lowercase();
            if !registered_labels.insert(label.clone()) {
                continue;
            }

            let adapter = self.table_adapter(&node.table, &mut adapters).await?;
            ctx.register_table(&label, adapter)
                .map_err(|e| anyhow!("Failed to register node label '{}': {}", node.label, e))?;
        }

        for edge in &self.overlay.edges {
            let label = edge.label.to_lowercase();
            if !registered_labels.insert(label.clone()) {
                continue;
            }

            let adapter = self.table_adapter(&edge.table, &mut adapters).await?;
            ctx.register_table(&label, adapter).map_err(|e| {
                anyhow!(
                    "Failed to register relationship label '{}': {}",
                    edge.label,
                    e
                )
            })?;
        }

        Ok(ctx)
    }

    async fn build_query_context(&self, include_edges: bool) -> Result<SessionContext> {
        let ctx = SessionContext::new();
        let mut adapters: HashMap<String, Arc<BaseTableAdapter>> = HashMap::new();
        let mut registered_tables = HashSet::new();

        for table_name in self.overlay.nodes.iter().map(|node| &node.table) {
            if !registered_tables.insert(table_name.clone()) {
                continue;
            }

            let adapter = self.table_adapter(table_name, &mut adapters).await?;
            ctx.register_table(table_name, adapter)?;
        }

        if include_edges {
            for table_name in self.overlay.edges.iter().map(|edge| &edge.table) {
                if !registered_tables.insert(table_name.clone()) {
                    continue;
                }

                let adapter = self.table_adapter(table_name, &mut adapters).await?;
                ctx.register_table(table_name, adapter)?;
            }
        }

        Ok(ctx)
    }

    fn rows_to_nodes(&self, label: &str, rows: Vec<Value>) -> Result<Vec<SubgraphNode>> {
        let id_col = self.find_id_column_for_label(label)?;
        let display_col = self.find_display_column_for_label(label);
        let mut nodes = Vec::new();
        let mut seen_node_ids = HashSet::new();

        for row in rows {
            let map = match row {
                Value::Object(map) => map,
                _ => continue,
            };

            let raw_id = value_to_id_string(map.get(&id_col));
            if raw_id.is_empty() {
                continue;
            }

            let full_id = format!("{label}:{raw_id}");
            if !seen_node_ids.insert(full_id.clone()) {
                continue;
            }

            let caption = display_col
                .as_ref()
                .and_then(|column| map.get(column))
                .and_then(|value| value.as_str())
                .map(String::from)
                .or_else(|| Some(raw_id.clone()));

            nodes.push(SubgraphNode {
                id: full_id,
                label: label.to_string(),
                caption,
                props: Value::Object(map),
            });
        }

        Ok(nodes)
    }

    async fn load_nodes_for_label(&self, label: &str, limit: usize) -> Result<Vec<SubgraphNode>> {
        let rows = self.sample(label, limit).await?;
        self.rows_to_nodes(label, rows)
    }

    async fn full_subgraph(&self, limit: usize) -> Result<SubgraphResult> {
        let mut all_nodes = Vec::new();
        let mut all_edges = Vec::new();
        let mut connected_labels = HashSet::new();
        let mut warnings = Vec::new();

        for edge in &self.overlay.edges {
            let query = format!(
                "MATCH (n:{src})-[r:{rel}]->(m:{dst}) RETURN n, r, m",
                src = edge.src_label,
                rel = edge.label,
                dst = edge.dst_label,
            );
            let batch = match self
                .execute_cypher_with_safety(&query, HashMap::new(), limit)
                .await
            {
                Ok(batch) => batch,
                Err(error) => {
                    tracing::warn!(%error, edge = %edge.label, "Full subgraph edge query failed");
                    warnings.push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };
            let rows = record_batch_to_value(&batch)?;
            let sub = self.parse_flat_rows(
                &rows,
                &edge.src_label,
                &edge.dst_label,
                &edge.label,
                true,
                limit,
            )?;
            all_nodes.extend(sub.nodes);
            all_edges.extend(sub.edges);
            connected_labels.insert(edge.src_label.clone());
            connected_labels.insert(edge.dst_label.clone());
        }

        for node in &self.overlay.nodes {
            if connected_labels.contains(&node.label) {
                continue;
            }

            let remaining = limit.saturating_sub(all_nodes.len());
            if remaining == 0 {
                break;
            }

            all_nodes.extend(self.load_nodes_for_label(&node.label, remaining).await?);
        }

        Ok(dedupe_and_limit_subgraph(
            all_nodes, all_edges, limit, warnings,
        ))
    }

    /// Parse flat rows from lance-graph cypher results.
    ///
    /// After RETURN projection, lance-graph aliases columns as `{var}.{column}` (e.g. `n.name`, `m.map`).
    /// `n` is the seed node variable, `m` is the neighbor variable, `r` is the relationship variable.
    fn parse_flat_rows(
        &self,
        rows: &[Value],
        n_label: &str,
        m_label: &str,
        edge_label: &str,
        n_is_source: bool,
        limit: usize,
    ) -> Result<SubgraphResult> {
        let n_id_col = self.find_id_column_for_label(n_label)?;
        let m_id_col = self.find_id_column_for_label(m_label)?;
        let n_display = self.find_display_column_for_label(n_label);
        let m_display = self.find_display_column_for_label(m_label);

        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut seen_node_ids = std::collections::HashSet::new();
        let mut seen_edge_ids = std::collections::HashSet::new();

        for row in rows {
            let map = match row {
                Value::Object(m) => m,
                _ => continue,
            };

            let n_id = value_to_id_string(map.get(&format!("n.{n_id_col}")));
            let m_id = value_to_id_string(map.get(&format!("m.{m_id_col}")));

            let n_full_id = format!("{n_label}:{n_id}");
            if !n_id.is_empty() && seen_node_ids.insert(n_full_id.clone()) {
                let caption = n_display
                    .as_ref()
                    .and_then(|dc| map.get(&format!("n.{dc}")))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| Some(n_id.clone()));
                nodes.push(SubgraphNode {
                    id: n_full_id.clone(),
                    label: n_label.to_string(),
                    caption,
                    props: Value::Object(extract_prefixed_props(map, "n.")),
                });
            }

            let m_full_id = format!("{m_label}:{m_id}");
            if !m_id.is_empty() && seen_node_ids.insert(m_full_id.clone()) {
                let caption = m_display
                    .as_ref()
                    .and_then(|dc| map.get(&format!("m.{dc}")))
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(|| Some(m_id.clone()));
                nodes.push(SubgraphNode {
                    id: m_full_id.clone(),
                    label: m_label.to_string(),
                    caption,
                    props: Value::Object(extract_prefixed_props(map, "m.")),
                });
            }

            if !n_id.is_empty() && !m_id.is_empty() {
                let (source, target) = if n_is_source {
                    (n_full_id.clone(), m_full_id.clone())
                } else {
                    (m_full_id.clone(), n_full_id.clone())
                };
                let edge_id = format!("{source}-{edge_label}->{target}");
                if seen_edge_ids.insert(edge_id.clone()) {
                    edges.push(SubgraphEdge {
                        id: edge_id,
                        source,
                        target,
                        label: edge_label.to_string(),
                        props: Value::Object(extract_prefixed_props(map, "r.")),
                    });
                }
            }
        }

        let truncated = nodes.len() >= limit;
        Ok(SubgraphResult {
            nodes,
            edges,
            truncated,
            warnings: Vec::new(),
        })
    }
}

fn value_to_id_string(val: Option<&Value>) -> String {
    match val {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(v) if !v.is_null() => v.to_string().trim_matches('"').to_string(),
        _ => String::new(),
    }
}

fn append_limit_clause(query: &str, limit: usize) -> String {
    let trimmed = query.trim();
    let trimmed = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    format!("{trimmed} LIMIT {limit}")
}

/// Rejects queries the engine cannot bound: variable-length path segments with
/// no upper bound or an upper bound beyond the configured depth cap. The
/// wallclock timeout is advisory only (DataFusion compute is not cancel-safe),
/// so unbounded traversals must never reach execution.
fn preflight_cypher(
    ast: &lance_graph::ast::CypherQuery,
    safety: &CypherSafetyConfig,
) -> Result<()> {
    use lance_graph::ast::{GraphPattern, ReadingClause};

    let clauses = ast
        .reading_clauses
        .iter()
        .chain(ast.post_with_reading_clauses.iter());
    for clause in clauses {
        let ReadingClause::Match(match_clause) = clause else {
            continue;
        };
        for pattern in &match_clause.patterns {
            let GraphPattern::Path(path) = pattern else {
                continue;
            };
            for segment in &path.segments {
                let Some(length) = &segment.relationship.length else {
                    continue;
                };
                match length.max {
                    None => {
                        return Err(anyhow!(
                            "Unbounded variable-length paths ([*]) are not allowed; specify an upper bound of at most {}",
                            safety.max_depth
                        ));
                    }
                    Some(max) if max as usize > safety.max_depth => {
                        return Err(anyhow!(
                            "Variable-length path bound {} exceeds the maximum allowed depth of {}",
                            max,
                            safety.max_depth
                        ));
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

/// Allows exactly one read-only statement through the SQL surface. DataFusion
/// would otherwise happily execute DDL (CREATE EXTERNAL TABLE) or COPY, which
/// reach the server filesystem.
/// Opens a LanceDB table and wraps it in a DataFusion table provider. Shared by
/// the graph query contexts and the Data Studio query workbench so there is one
/// adapter-construction path.
pub(crate) async fn open_table_adapter(
    connection: &Connection,
    table_name: &str,
) -> Result<Arc<BaseTableAdapter>> {
    let table = connection
        .open_table(table_name)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open table '{}': {}", table_name, e))?;

    let adapter = BaseTableAdapter::try_new(table.base_table().clone())
        .await
        .map_err(|e| {
            anyhow!(
                "Failed to create DataFusion adapter for '{}': {}",
                table_name,
                e
            )
        })?;
    Ok(Arc::new(adapter))
}

pub(crate) fn validate_readonly_sql(query: &str) -> Result<()> {
    use datafusion::sql::parser::{DFParser, Statement as DFStatement};

    let statements =
        DFParser::parse_sql(query).map_err(|e| anyhow!("Failed to parse SQL query: {}", e))?;
    if statements.len() != 1 {
        return Err(anyhow!("Exactly one SQL statement is allowed per query"));
    }
    match statements.front() {
        Some(DFStatement::Statement(inner))
            if matches!(
                inner.as_ref(),
                datafusion::sql::sqlparser::ast::Statement::Query(_)
            ) =>
        {
            Ok(())
        }
        _ => Err(anyhow!(
            "Only read-only SELECT queries are allowed on the graph SQL surface"
        )),
    }
}

/// Quotes an identifier for a DataFusion `ctx.sql()` string, where `"col"` is a
/// delimited identifier.
fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// Quotes an identifier for a LanceDB `only_if` filter. LanceDB's filter parser
/// reads a double-quoted `"col"` as a string LITERAL (so `quote_identifier`
/// there silently matches nothing); backticks delimit the column and stay safe
/// for names with spaces or reserved words.
pub(crate) fn filter_identifier(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

fn sql_string_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn search_match_rank(node: &SubgraphNode, query: &str) -> (u8, usize, String, String) {
    let caption = node.caption.clone().unwrap_or_default();
    let caption_lower = caption.to_lowercase();
    let label_lower = node.label.to_lowercase();
    let id_lower = node.id.to_lowercase();

    let rank = if caption_lower == query || id_lower == query {
        0
    } else if caption_lower.starts_with(query) || id_lower.starts_with(query) {
        1
    } else if caption_lower.contains(query) {
        2
    } else if label_lower.contains(query) {
        3
    } else {
        4
    };

    (rank, caption.len(), caption_lower, id_lower)
}

fn extract_prefixed_props(
    map: &serde_json::Map<String, Value>,
    prefix: &str,
) -> serde_json::Map<String, Value> {
    let mut props = serde_json::Map::new();
    for (k, v) in map {
        if let Some(stripped) = k.strip_prefix(prefix) {
            props.insert(stripped.to_string(), v.clone());
        }
    }
    props
}

fn dedupe_and_limit_subgraph(
    mut all_nodes: Vec<SubgraphNode>,
    mut all_edges: Vec<SubgraphEdge>,
    limit: usize,
    warnings: Vec<String>,
) -> SubgraphResult {
    let mut seen_node_ids = HashSet::new();
    all_nodes.retain(|node| seen_node_ids.insert(node.id.clone()));

    let mut seen_edge_ids = HashSet::new();
    all_edges.retain(|edge| seen_edge_ids.insert(edge.id.clone()));

    let truncated = all_nodes.len() > limit || all_edges.len() > limit.saturating_mul(3);
    all_nodes.truncate(limit);
    all_edges.truncate(limit.saturating_mul(3));

    let kept_node_ids = all_nodes
        .iter()
        .map(|node| node.id.as_str())
        .collect::<HashSet<_>>();
    all_edges.retain(|edge| {
        kept_node_ids.contains(edge.source.as_str()) && kept_node_ids.contains(edge.target.as_str())
    });

    SubgraphResult {
        nodes: all_nodes,
        edges: all_edges,
        truncated,
        warnings,
    }
}

fn include_default_property(data_type: &arrow::datatypes::DataType) -> bool {
    !matches!(
        data_type,
        arrow::datatypes::DataType::FixedSizeList(_, _)
            | arrow::datatypes::DataType::List(_)
            | arrow::datatypes::DataType::LargeList(_)
            | arrow::datatypes::DataType::Binary
            | arrow::datatypes::DataType::LargeBinary
            | arrow::datatypes::DataType::FixedSizeBinary(_)
    )
}

async fn resolve_property_names(
    connection: &Connection,
    table_name: &str,
    configured: &[PropertyColumnDef],
    projection_mode: PropertyProjectionMode,
    schema_cache: &mut HashMap<String, Vec<String>>,
    excluded: &HashSet<String>,
    always_include: &[String],
) -> Result<Vec<String>> {
    let mut prop_names = if !configured.is_empty() {
        configured
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
    } else if projection_mode == PropertyProjectionMode::Frozen {
        Vec::new()
    } else if let Some(columns) = schema_cache.get(table_name) {
        columns.clone()
    } else {
        let table = connection
            .open_table(table_name)
            .execute()
            .await
            .map_err(|e| {
                anyhow!(
                    "Failed to open table '{}' while resolving graph properties: {}",
                    table_name,
                    e
                )
            })?;

        let schema = table.schema().await.map_err(|e| {
            anyhow!(
                "Failed to read schema for '{}' while resolving graph properties: {}",
                table_name,
                e
            )
        })?;

        let columns = schema
            .fields()
            .iter()
            .filter(|field| include_default_property(field.data_type()))
            .map(|field| field.name().clone())
            .collect::<Vec<_>>();

        schema_cache.insert(table_name.to_string(), columns.clone());
        columns
    };

    let mut seen = HashSet::new();
    prop_names.retain(|name| !excluded.contains(name) && seen.insert(name.clone()));

    for column in always_include {
        if !excluded.contains(column) && seen.insert(column.clone()) {
            prop_names.push(column.clone());
        }
    }

    Ok(prop_names)
}

async fn build_graph_config(
    connection: &Connection,
    overlay: &GraphOverlayDef,
) -> Result<GraphConfig> {
    let mut builder = GraphConfig::builder();
    let mut schema_cache: HashMap<String, Vec<String>> = HashMap::new();

    for node in &overlay.nodes {
        let id_col = effective_node_id_column_checked(overlay, &node.label)?
            .ok_or_else(|| anyhow!("Object type '{}' has no identity column", node.label))?;
        let excluded = HashSet::from([id_col.clone()]);
        let always_include = node
            .display_column
            .as_ref()
            .filter(|column| column.as_str() != id_col.as_str())
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        let prop_names = resolve_property_names(
            connection,
            &node.table,
            &node.property_columns,
            overlay.property_projection_mode,
            &mut schema_cache,
            &excluded,
            &always_include,
        )
        .await?;
        let mapping =
            lance_graph::NodeMapping::new(&node.label, &id_col).with_properties(prop_names);
        builder = builder.with_node_mapping(mapping);
    }

    for edge in &overlay.edges {
        let excluded = HashSet::from([edge.src_column.clone(), edge.dst_column.clone()]);
        let prop_names = resolve_property_names(
            connection,
            &edge.table,
            &edge.property_columns,
            overlay.property_projection_mode,
            &mut schema_cache,
            &excluded,
            &[],
        )
        .await?;
        let mapping =
            lance_graph::RelationshipMapping::new(&edge.label, &edge.src_column, &edge.dst_column)
                .with_properties(prop_names);
        builder = builder.with_relationship_mapping(mapping);
    }

    builder
        .build()
        .map_err(|e| anyhow!("Failed to build GraphConfig: {}", e))
}

pub const GRAPH_OVERLAYS_TABLE: &str = "__graph_overlays__";
pub const ONTOLOGY_IMPORTS_TABLE: &str = "__ontology_imports__";

pub async fn list_overlays(connection: &Connection) -> Result<Vec<GraphOverlayDef>> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    if !table_names.iter().any(|n| n == GRAPH_OVERLAYS_TABLE) {
        return Ok(Vec::new());
    }

    let table = connection
        .open_table(GRAPH_OVERLAYS_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open overlays table: {}", e))?;

    let result = table
        .query()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to query overlays table: {}", e))?;

    let batches = result
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| anyhow!("Failed to collect overlays: {}", e))?;

    let mut overlays = Vec::new();
    for batch in &batches {
        let rows = record_batch_to_value(batch)?;
        for row in rows {
            if let Some(def_json) = row.get("definition_json").and_then(|v| v.as_str()) {
                match serde_json::from_str::<GraphOverlayDef>(def_json) {
                    Ok(overlay) => overlays.push(overlay),
                    Err(error) => {
                        let overlay_id = row
                            .get("id")
                            .and_then(|value| value.as_str())
                            .unwrap_or("<unknown>");
                        tracing::warn!(
                            %error,
                            overlay_id,
                            "Skipping graph overlay with unparseable definition"
                        );
                    }
                }
            }
        }
    }

    Ok(overlays)
}

pub async fn load_overlay(connection: &Connection, overlay_id: &str) -> Result<GraphOverlayDef> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    if !table_names.iter().any(|name| name == GRAPH_OVERLAYS_TABLE) {
        return Err(anyhow!("Overlay '{}' not found", overlay_id));
    }

    let table = connection
        .open_table(GRAPH_OVERLAYS_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open overlays table: {}", e))?;
    let filter = format!("id = '{}'", overlay_id.replace('\'', "''"));
    let result = table
        .query()
        .only_if(filter)
        .limit(1)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to query overlay '{}': {}", overlay_id, e))?;
    let batches = result
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| anyhow!("Failed to collect overlay '{}': {}", overlay_id, e))?;

    for batch in &batches {
        for row in record_batch_to_value(batch)? {
            if let Some(definition) = row.get("definition_json").and_then(|value| value.as_str()) {
                return serde_json::from_str(definition)
                    .map_err(|e| anyhow!("Failed to parse overlay definition: {}", e));
            }
        }
    }

    Err(anyhow!("Overlay '{}' not found", overlay_id))
}

/// Resolves the column that identifies a node label across the graph.
///
/// Edge-level `src_node_column`/`dst_node_column` overrides win over the node's
/// own `id_column`, mirroring `build_graph_config` and `rows_to_nodes`. Identity
/// lookups must resolve the column the same way, or an id a client derived from a
/// rendered node will not match the column the query filters on.
pub fn effective_node_id_column(overlay: &GraphOverlayDef, label: &str) -> Option<String> {
    effective_node_id_column_checked(overlay, label)
        .ok()
        .flatten()
}

/// Checked identity resolution used at every governed/remote boundary. Edge
/// overrides may replace a node's base id column, but all overrides for one
/// label must agree; accepting the first one would make identity depend on
/// mapping order.
pub fn effective_node_id_column_checked(
    overlay: &GraphOverlayDef,
    label: &str,
) -> Result<Option<String>> {
    effective_node_id_column_for_mappings(&overlay.nodes, &overlay.edges, label)
}

pub fn effective_node_id_column_for_mappings(
    nodes: &[NodeMappingDef],
    edges: &[EdgeMappingDef],
    label: &str,
) -> Result<Option<String>> {
    let Some(node) = nodes.iter().find(|node| node.label == label) else {
        return Ok(None);
    };
    let mut override_column: Option<String> = None;
    for edge in edges {
        for (matches_label, column) in [
            (edge.src_label == label, edge.src_node_column.as_ref()),
            (edge.dst_label == label, edge.dst_node_column.as_ref()),
        ] {
            let Some(column) = column.filter(|_| matches_label) else {
                continue;
            };
            if let Some(existing) = override_column.as_deref()
                && existing != column
            {
                return Err(anyhow!(
                    "Object type '{}' has conflicting node identity overrides '{}' and '{}'",
                    label,
                    existing,
                    column
                ));
            }
            override_column = Some(column.clone());
        }
    }
    Ok(Some(
        override_column.unwrap_or_else(|| node.id_column.clone()),
    ))
}

/// Resolves an object type by any of its identities — stable id, API name, or
/// display label — matching how actions and remote queries resolve types.
pub fn resolve_object_mapping<'a>(
    overlay: &'a GraphOverlayDef,
    object_type: &str,
) -> Option<&'a NodeMappingDef> {
    resolve_object_mapping_from_nodes(&overlay.nodes, object_type)
}

fn resolve_object_mapping_from_nodes<'a>(
    nodes: &'a [NodeMappingDef],
    object_type: &str,
) -> Option<&'a NodeMappingDef> {
    nodes.iter().find(|node| {
        node.id.as_deref() == Some(object_type)
            || node.api_name.as_deref() == Some(object_type)
            || node.label == object_type
    })
}

fn required_object_columns(mapping: &NodeMappingDef, identity_column: &str) -> Vec<String> {
    let mut columns = vec![identity_column.to_string(), mapping.id_column.clone()];
    if let Some(display_column) = &mapping.display_column {
        columns.push(display_column.clone());
    }
    let mut seen = HashSet::new();
    columns.retain(|column| seen.insert(column.clone()));
    columns
}

/// Returns the explicit portion of a governed projection without consulting a
/// live schema. This is used when comparing ontology definitions, where an
/// effective identity override must count as a governed contract change even
/// if the node mapping itself is byte-for-byte unchanged.
fn declared_governed_object_projection(
    overlay: &GraphOverlayDef,
    action: &OntologyActionDef,
) -> Result<GovernedObjectProjection> {
    let mapping = resolve_object_mapping(overlay, &action.object_type).ok_or_else(|| {
        anyhow!(
            "Ontology action '{}' references unknown object type '{}'",
            action.id,
            action.object_type
        )
    })?;
    let identity_column = effective_node_id_column_checked(overlay, &mapping.label)?
        .ok_or_else(|| anyhow!("Object type '{}' has no identity column", mapping.label))?;
    let mut columns = required_object_columns(mapping, &identity_column);
    columns.extend(
        mapping
            .property_columns
            .iter()
            .map(|property| property.name.clone()),
    );
    let mut seen = HashSet::new();
    columns.retain(|column| seen.insert(column.clone()));
    Ok(GovernedObjectProjection {
        table: mapping.table.clone(),
        identity_column,
        columns,
    })
}

/// Resolves the live local wildcard once, producing the concrete projection
/// that will be protected by a managed action event.
pub async fn resolve_governed_object_projection(
    connection: &Connection,
    overlay: &GraphOverlayDef,
    action: &OntologyActionDef,
) -> Result<GovernedObjectProjection> {
    resolve_governed_object_projection_for_mappings(
        connection,
        &overlay.nodes,
        &overlay.edges,
        overlay.property_projection_mode,
        action,
    )
    .await
}

pub async fn resolve_governed_object_projection_for_mappings(
    connection: &Connection,
    nodes: &[NodeMappingDef],
    edges: &[EdgeMappingDef],
    projection_mode: PropertyProjectionMode,
    action: &OntologyActionDef,
) -> Result<GovernedObjectProjection> {
    let mapping =
        resolve_object_mapping_from_nodes(nodes, &action.object_type).ok_or_else(|| {
            anyhow!(
                "Ontology action '{}' references unknown object type '{}'",
                action.id,
                action.object_type
            )
        })?;
    let identity_column = effective_node_id_column_for_mappings(nodes, edges, &mapping.label)?
        .ok_or_else(|| anyhow!("Object type '{}' has no identity column", mapping.label))?;
    let columns = resolve_object_projection(
        connection,
        &mapping.table,
        &mapping.property_columns,
        projection_mode,
        required_object_columns(mapping, &identity_column),
    )
    .await?;
    Ok(GovernedObjectProjection {
        table: mapping.table.clone(),
        identity_column,
        columns,
    })
}

/// Confirms that a stored action projection still belongs to the current
/// ontology contract. Dynamic local mappings may keep the concrete columns
/// captured at materialization; explicit and frozen mappings must match their
/// declared surface exactly.
pub fn validate_governed_object_projection<'a>(
    overlay: &'a GraphOverlayDef,
    action: &OntologyActionDef,
    projection: &GovernedObjectProjection,
) -> Result<&'a NodeMappingDef> {
    validate_governed_object_projection_for_mappings(
        &overlay.nodes,
        &overlay.edges,
        overlay.property_projection_mode,
        action,
        projection,
    )
}

pub fn validate_governed_object_projection_for_mappings<'a>(
    nodes: &'a [NodeMappingDef],
    edges: &[EdgeMappingDef],
    projection_mode: PropertyProjectionMode,
    action: &OntologyActionDef,
    projection: &GovernedObjectProjection,
) -> Result<&'a NodeMappingDef> {
    let mapping =
        resolve_object_mapping_from_nodes(nodes, &action.object_type).ok_or_else(|| {
            anyhow!(
                "Ontology action '{}' references unknown object type '{}'",
                action.id,
                action.object_type
            )
        })?;
    let identity_column = effective_node_id_column_for_mappings(nodes, edges, &mapping.label)?
        .ok_or_else(|| anyhow!("Object type '{}' has no identity column", mapping.label))?;
    if projection.table != mapping.table || projection.identity_column != identity_column {
        return Err(anyhow!(
            "The stored action object identity no longer matches object type '{}'",
            action.object_type
        ));
    }
    if projection.columns.is_empty() || projection.columns.iter().any(|column| column.is_empty()) {
        return Err(anyhow!(
            "The stored action object projection is empty or invalid"
        ));
    }
    let projected = projection
        .columns
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if projected.len() != projection.columns.len() {
        return Err(anyhow!(
            "The stored action object projection contains duplicate columns"
        ));
    }
    let mut expected = required_object_columns(mapping, &identity_column);
    for required in &expected {
        if !projected.contains(required.as_str()) {
            return Err(anyhow!(
                "The stored action object projection is missing required column '{}'",
                required
            ));
        }
    }
    if projection_mode == PropertyProjectionMode::Frozen || !mapping.property_columns.is_empty() {
        expected.extend(
            mapping
                .property_columns
                .iter()
                .map(|property| property.name.clone()),
        );
        let mut seen = HashSet::new();
        expected.retain(|column| seen.insert(column.clone()));
        let expected = expected.iter().map(String::as_str).collect::<HashSet<_>>();
        if projected != expected {
            return Err(anyhow!(
                "The stored action object projection no longer matches object type '{}'",
                action.object_type
            ));
        }
    }
    Ok(mapping)
}

/// Reads a managed action's protected projection from its event config.
pub fn governed_object_projection_from_event_config(
    config: &[u8],
) -> Result<GovernedObjectProjection> {
    let config = serde_json::from_slice::<serde_json::Value>(config)
        .map_err(|error| anyhow!("Invalid managed action event config: {}", error))?;
    let projection = config
        .get("object_projection")
        .cloned()
        .ok_or_else(|| anyhow!("The managed action event has no protected object projection"))?;
    serde_json::from_value(projection)
        .map_err(|error| anyhow!("Invalid managed action object projection: {}", error))
}

async fn freeze_property_columns(
    connection: &Connection,
    table_name: &str,
    configured: &[PropertyColumnDef],
    excluded: &HashSet<String>,
) -> Result<Vec<PropertyColumnDef>> {
    let table = connection
        .open_table(table_name)
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to open table '{}': {}", table_name, error))?;
    let schema = table
        .schema()
        .await
        .map_err(|error| anyhow!("Failed to read schema for '{}': {}", table_name, error))?;
    let schema_names = schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect::<HashSet<_>>();
    let mut missing_required = excluded
        .iter()
        .filter(|column| !schema_names.contains(column.as_str()))
        .map(String::as_str)
        .collect::<Vec<_>>();
    missing_required.sort_unstable();
    if !missing_required.is_empty() {
        return Err(anyhow!(
            "Required columns [{}] do not exist in table '{}'",
            missing_required.join(", "),
            table_name
        ));
    }
    let configured_names = configured
        .iter()
        .map(|property| property.name.as_str())
        .filter(|name| !excluded.contains(*name))
        .collect::<HashSet<_>>();
    let explicit = !configured.is_empty();
    let mut properties = Vec::new();
    for field in schema.fields() {
        if excluded.contains(field.name())
            || (explicit && !configured_names.contains(field.name().as_str()))
            || (!explicit && !include_default_property(field.data_type()))
        {
            continue;
        }
        properties.push(PropertyColumnDef {
            name: field.name().clone(),
            data_type: format!("{:?}", field.data_type()),
            nullable: field.is_nullable(),
        });
    }
    if explicit && properties.len() != configured_names.len() {
        let resolved = properties
            .iter()
            .map(|property| property.name.as_str())
            .collect::<HashSet<_>>();
        let missing = configured_names
            .difference(&resolved)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        return Err(anyhow!(
            "Configured properties [{}] do not exist in table '{}'",
            missing,
            table_name
        ));
    }
    Ok(properties)
}

/// Resolve every dynamic property wildcard against the producer's live schema
/// and mark the installed contract frozen. A resolved empty list remains empty
/// forever instead of gaining columns added to the physical table later.
pub async fn freeze_remote_contract_projection(
    connection: &Connection,
    overlay: &mut GraphOverlayDef,
) -> Result<()> {
    if overlay.property_projection_mode == PropertyProjectionMode::Frozen {
        return Ok(());
    }
    let identities = overlay
        .nodes
        .iter()
        .map(|node| {
            Ok((
                node.label.clone(),
                effective_node_id_column_checked(overlay, &node.label)?.ok_or_else(|| {
                    anyhow!("Object type '{}' has no identity column", node.label)
                })?,
            ))
        })
        .collect::<Result<HashMap<_, _>>>()?;

    for node in &mut overlay.nodes {
        let identity = identities
            .get(&node.label)
            .expect("identity was resolved for every node");
        let mut excluded = HashSet::from([identity.clone(), node.id_column.clone()]);
        if let Some(display) = &node.display_column {
            excluded.insert(display.clone());
        }
        node.property_columns =
            freeze_property_columns(connection, &node.table, &node.property_columns, &excluded)
                .await?;
    }
    for edge in &mut overlay.edges {
        let excluded = HashSet::from([edge.src_column.clone(), edge.dst_column.clone()]);
        edge.property_columns =
            freeze_property_columns(connection, &edge.table, &edge.property_columns, &excluded)
                .await?;
    }
    overlay.property_projection_mode = PropertyProjectionMode::Frozen;
    Ok(())
}

/// Contract-approved columns grouped by physical table. Used to expose a
/// projected remote SQL surface without ever registering the underlying full
/// Lance tables in the DataFusion catalog.
pub fn frozen_remote_table_projections(
    overlay: &GraphOverlayDef,
) -> Result<HashMap<String, Vec<String>>> {
    if overlay.property_projection_mode != PropertyProjectionMode::Frozen {
        return Err(anyhow!(
            "Remote ontology contract does not contain a frozen property projection"
        ));
    }
    let mut projections: HashMap<String, Vec<String>> = HashMap::new();
    for node in &overlay.nodes {
        let columns = projections.entry(node.table.clone()).or_default();
        columns.push(
            effective_node_id_column_checked(overlay, &node.label)?
                .ok_or_else(|| anyhow!("Object type '{}' has no identity column", node.label))?,
        );
        columns.push(node.id_column.clone());
        if let Some(display) = &node.display_column {
            columns.push(display.clone());
        }
        columns.extend(
            node.property_columns
                .iter()
                .map(|property| property.name.clone()),
        );
    }
    for edge in &overlay.edges {
        let columns = projections.entry(edge.table.clone()).or_default();
        columns.push(edge.src_column.clone());
        columns.push(edge.dst_column.clone());
        columns.extend(
            edge.property_columns
                .iter()
                .map(|property| property.name.clone()),
        );
    }
    for columns in projections.values_mut() {
        let mut seen = HashSet::new();
        columns.retain(|column| seen.insert(column.clone()));
    }
    Ok(projections)
}

/// Builds a projection for an ontology mapping. An empty property list means
/// "all scalar/non-vector properties", matching graph traversal and Cypher
/// hydration. Required identity/display columns are always retained.
async fn resolve_object_projection(
    connection: &Connection,
    table_name: &str,
    configured: &[PropertyColumnDef],
    projection_mode: PropertyProjectionMode,
    required: Vec<String>,
) -> Result<Vec<String>> {
    let excluded = required.iter().cloned().collect::<HashSet<_>>();
    let mut schema_cache = HashMap::new();
    let properties = resolve_property_names(
        connection,
        table_name,
        configured,
        projection_mode,
        &mut schema_cache,
        &excluded,
        &[],
    )
    .await?;

    let mut columns = required;
    columns.extend(properties);
    let mut seen = HashSet::new();
    columns.retain(|column| seen.insert(column.clone()));
    Ok(columns)
}

pub async fn sample_overlay(
    connection: &Connection,
    overlay: &GraphOverlayDef,
    label: &str,
    limit: usize,
) -> Result<Vec<Value>> {
    let (table_name, configured, required) =
        if let Some(node) = resolve_object_mapping(overlay, label) {
            let mut required = vec![
                effective_node_id_column_checked(overlay, &node.label)?
                    .unwrap_or_else(|| node.id_column.clone()),
                node.id_column.clone(),
            ];
            if let Some(display_column) = &node.display_column {
                required.push(display_column.clone());
            }
            (
                node.table.as_str(),
                node.property_columns.as_slice(),
                required,
            )
        } else if let Some(edge) = overlay.edges.iter().find(|edge| {
            edge.id.as_deref() == Some(label)
                || edge.api_name.as_deref() == Some(label)
                || edge.label == label
        }) {
            // `src_node_column`/`dst_node_column` live on the corresponding
            // node tables; only the edge endpoint columns belong here.
            let required = vec![edge.src_column.clone(), edge.dst_column.clone()];
            (
                edge.table.as_str(),
                edge.property_columns.as_slice(),
                required,
            )
        } else {
            return Err(anyhow!("Label '{}' not found in overlay", label));
        };
    let columns = resolve_object_projection(
        connection,
        table_name,
        configured,
        overlay.property_projection_mode,
        required,
    )
    .await?;

    let table = connection
        .open_table(table_name)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open table '{}': {}", table_name, e))?;
    let mut query = table.query().limit(limit);
    if !columns.is_empty() {
        query = query.select(lancedb::query::Select::Columns(columns));
    }
    let result = query
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to query table '{}': {}", table_name, e))?;
    let batches = result
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| anyhow!("Failed to collect from '{}': {}", table_name, e))?;
    let mut rows = Vec::new();
    for batch in &batches {
        rows.extend(record_batch_to_value(batch)?);
    }
    rows.truncate(limit);
    Ok(rows)
}

/// Samples the exact already-resolved object mapping. This avoids a second
/// lookup by display label, which is not a stable or necessarily unique key.
pub async fn sample_overlay_object(
    connection: &Connection,
    object: &NodeMappingDef,
    identity_column: &str,
    projection_mode: PropertyProjectionMode,
    limit: usize,
) -> Result<Vec<Value>> {
    let mut required = vec![identity_column.to_string(), object.id_column.clone()];
    if let Some(display_column) = &object.display_column {
        required.push(display_column.clone());
    }
    let columns = resolve_object_projection(
        connection,
        &object.table,
        &object.property_columns,
        projection_mode,
        required,
    )
    .await?;

    let table = connection
        .open_table(&object.table)
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to open table '{}': {}", object.table, error))?;
    let mut query = table.query().limit(limit);
    if !columns.is_empty() {
        query = query.select(lancedb::query::Select::Columns(columns));
    }
    let batches = query
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to query table '{}': {}", object.table, error))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|error| anyhow!("Failed to collect from '{}': {}", object.table, error))?;
    let mut rows = Vec::new();
    for batch in &batches {
        rows.extend(record_batch_to_value(batch)?);
    }
    rows.truncate(limit);
    Ok(rows)
}

/// Loads ontology objects by their governed object identity.
///
/// The saved ontology supplies the table and identity column. Callers supply
/// only identity values, so an action invocation never has to trust a
/// client-provided table, column, predicate, or full object payload.
pub async fn load_overlay_objects(
    connection: &Connection,
    overlay: &GraphOverlayDef,
    object_type: &str,
    ids: &[Value],
) -> Result<Vec<Value>> {
    let mapping = resolve_object_mapping(overlay, object_type).ok_or_else(|| {
        anyhow!(
            "Object type '{}' was not found in the ontology",
            object_type
        )
    })?;
    // The graph identifies nodes by the effective id column (which honors
    // edge-level `src_node_column`/`dst_node_column` overrides), so the id the
    // client sends is a value of that column, not necessarily `mapping.id_column`.
    let id_column = effective_node_id_column_checked(overlay, &mapping.label)?
        .ok_or_else(|| anyhow!("Object type '{}' has no identity column", mapping.label))?;
    let mut required = vec![id_column.clone(), mapping.id_column.clone()];
    if let Some(display_column) = &mapping.display_column {
        required.push(display_column.clone());
    }
    let columns = resolve_object_projection(
        connection,
        &mapping.table,
        &mapping.property_columns,
        overlay.property_projection_mode,
        required,
    )
    .await?;

    load_projected_overlay_objects(connection, &mapping.table, &id_column, &columns, ids).await
}

/// Loads governed action objects through the concrete projection stored in
/// the managed event, never by re-resolving a local wildcard at invocation.
pub async fn load_overlay_objects_with_projection(
    connection: &Connection,
    overlay: &GraphOverlayDef,
    action: &OntologyActionDef,
    projection: &GovernedObjectProjection,
    ids: &[Value],
) -> Result<Vec<Value>> {
    validate_governed_object_projection(overlay, action, projection)?;
    load_projected_overlay_objects(
        connection,
        &projection.table,
        &projection.identity_column,
        &projection.columns,
        ids,
    )
    .await
}

async fn load_projected_overlay_objects(
    connection: &Connection,
    table_name: &str,
    id_column: &str,
    columns: &[String],
    ids: &[Value],
) -> Result<Vec<Value>> {
    if ids.is_empty() {
        return Err(anyhow!("At least one object identity is required"));
    }
    if ids.len() > 100 {
        return Err(anyhow!(
            "At most 100 object identities may be loaded at once"
        ));
    }
    let mut seen = HashSet::with_capacity(ids.len());
    let mut literals = Vec::with_capacity(ids.len());
    for id in ids {
        let key = value_to_id_string(Some(id));
        if key.is_empty() {
            return Err(anyhow!(
                "Object identities must be strings, numbers, or booleans"
            ));
        }
        if !seen.insert(key) {
            return Err(anyhow!("Duplicate object identities are not allowed"));
        }
        literals.push(value_sql_literal(id)?);
    }

    let table = connection
        .open_table(table_name)
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to open table '{}': {}", table_name, error))?;
    let predicate = format!(
        "{} IN ({})",
        filter_identifier(id_column),
        literals.join(", ")
    );

    // Do not clamp to ids.len(): if the identity column has duplicate rows, a
    // tight limit can return only some ids' duplicates and report the rest as
    // missing. The IN predicate already bounds the scan to the requested ids;
    // a generous cap guards against a pathological table without truncating a
    // healthy one.
    let batches = table
        .query()
        .only_if(&predicate)
        .select(lancedb::query::Select::Columns(columns.to_vec()))
        .limit(ids.len().saturating_mul(64).max(1_000))
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to load ontology objects: {}", error))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|error| anyhow!("Failed to collect ontology objects: {}", error))?;

    let allowed_columns = columns.iter().map(String::as_str).collect::<HashSet<_>>();

    let mut rows_by_id = HashMap::with_capacity(ids.len());
    for batch in &batches {
        for mut row in record_batch_to_value(batch)? {
            let Some(row_object) = row.as_object_mut() else {
                continue;
            };
            let key = value_to_id_string(row_object.get(id_column));
            if !key.is_empty() {
                row_object.retain(|column, _| allowed_columns.contains(column.as_str()));
                rows_by_id.insert(key, row);
            }
        }
    }

    let mut ordered = Vec::with_capacity(ids.len());
    for id in ids {
        let key = value_to_id_string(Some(id));
        let row = rows_by_id
            .remove(&key)
            .ok_or_else(|| anyhow!("Object '{}' was not found", key))?;
        ordered.push(row);
    }
    Ok(ordered)
}

fn value_sql_literal(value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(format!("'{}'", value.replace('\'', "''"))),
        Value::Number(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(if *value { "TRUE" } else { "FALSE" }.to_string()),
        _ => Err(anyhow!(
            "Object identities must be strings, numbers, or booleans"
        )),
    }
}

pub async fn save_overlay(connection: &Connection, overlay: &GraphOverlayDef) -> Result<()> {
    use arrow::array::{RecordBatch, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("description", DataType::Utf8, true),
        Field::new("definition_json", DataType::Utf8, false),
        Field::new("updated_at", DataType::Utf8, false),
    ]));

    let def_json = serde_json::to_string(overlay)
        .map_err(|e| anyhow!("Failed to serialize overlay: {}", e))?;

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec![overlay.id.as_str()])),
            Arc::new(StringArray::from(vec![overlay.name.as_str()])),
            Arc::new(StringArray::from(vec![
                overlay.description.as_deref().unwrap_or(""),
            ])),
            Arc::new(StringArray::from(vec![def_json.as_str()])),
            Arc::new(StringArray::from(vec![overlay.updated_at.as_str()])),
        ],
    )?;

    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    if table_names.iter().any(|n| n == GRAPH_OVERLAYS_TABLE) {
        let table = connection
            .open_table(GRAPH_OVERLAYS_TABLE)
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to open overlays table: {}", e))?;

        let mut merger = table.merge_insert(&["id"]);
        merger
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        let reader: Box<dyn arrow::record_batch::RecordBatchReader + Send> = Box::new(
            arrow::record_batch::RecordBatchIterator::new(vec![Ok(batch.clone())], schema.clone()),
        );
        merger
            .execute(reader)
            .await
            .map_err(|e| anyhow!("Failed to upsert overlay: {}", e))?;
    } else {
        connection
            .create_table(GRAPH_OVERLAYS_TABLE, vec![batch.clone()])
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to create overlays table: {}", e))?;
    }

    Ok(())
}

/// Atomically updates an existing overlay only when its persisted revision
/// still matches the one the caller loaded. This prevents concurrent action
/// edits from committing an overlay that no longer matches its managed event.
pub async fn save_overlay_if_unchanged(
    connection: &Connection,
    overlay: &GraphOverlayDef,
    expected_updated_at: &str,
) -> Result<bool> {
    use arrow::array::{RecordBatch, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("description", DataType::Utf8, true),
        Field::new("definition_json", DataType::Utf8, false),
        Field::new("updated_at", DataType::Utf8, false),
    ]));
    let def_json = serde_json::to_string(overlay)
        .map_err(|error| anyhow!("Failed to serialize overlay: {}", error))?;
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec![overlay.id.as_str()])),
            Arc::new(StringArray::from(vec![overlay.name.as_str()])),
            Arc::new(StringArray::from(vec![
                overlay.description.as_deref().unwrap_or(""),
            ])),
            Arc::new(StringArray::from(vec![def_json.as_str()])),
            Arc::new(StringArray::from(vec![overlay.updated_at.as_str()])),
        ],
    )?;

    let table = connection
        .open_table(GRAPH_OVERLAYS_TABLE)
        .execute()
        .await
        .map_err(|error| anyhow!("Failed to open overlays table: {}", error))?;
    let expected = expected_updated_at.replace('\'', "''");
    let mut merger = table.merge_insert(&["id"]);
    merger.when_matched_update_all(Some(format!("target.updated_at = '{expected}'")));
    let reader: Box<dyn arrow::record_batch::RecordBatchReader + Send> = Box::new(
        arrow::record_batch::RecordBatchIterator::new(vec![Ok(batch)], schema),
    );
    let result = merger
        .execute(reader)
        .await
        .map_err(|error| anyhow!("Failed to conditionally update overlay: {}", error))?;
    Ok(result.num_updated_rows == 1)
}

pub async fn delete_overlay(connection: &Connection, overlay_id: &str) -> Result<()> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    if !table_names.iter().any(|n| n == GRAPH_OVERLAYS_TABLE) {
        return Err(anyhow!("No overlays table found"));
    }

    let table = connection
        .open_table(GRAPH_OVERLAYS_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open overlays table: {}", e))?;

    table
        .delete(&format!("id = '{}'", overlay_id.replace('\'', "''")))
        .await
        .map_err(|e| anyhow!("Failed to delete overlay: {}", e))?;

    Ok(())
}

pub async fn list_ontology_imports(
    connection: &Connection,
) -> Result<Vec<RemoteOntologyImportDef>> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    if !table_names
        .iter()
        .any(|name| name == ONTOLOGY_IMPORTS_TABLE)
    {
        return Ok(Vec::new());
    }

    let table = connection
        .open_table(ONTOLOGY_IMPORTS_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open ontology imports table: {}", e))?;
    let result = table
        .query()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to query ontology imports: {}", e))?;
    let batches = result
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| anyhow!("Failed to collect ontology imports: {}", e))?;

    let mut imports: Vec<RemoteOntologyImportDef> = Vec::new();
    for batch in &batches {
        for row in record_batch_to_value(batch)? {
            if let Some(definition) = row.get("definition_json").and_then(|value| value.as_str()) {
                match serde_json::from_str(definition) {
                    Ok(import) => imports.push(import),
                    Err(error) => {
                        let import_id = row
                            .get("id")
                            .and_then(|value| value.as_str())
                            .unwrap_or("<unknown>");
                        tracing::warn!(
                            %error,
                            import_id,
                            "Skipping ontology import with unparseable definition"
                        );
                    }
                }
            }
        }
    }
    imports.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(imports)
}

pub async fn find_ontology_import(
    connection: &Connection,
    import_id: &str,
) -> Result<Option<RemoteOntologyImportDef>> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;

    if !table_names
        .iter()
        .any(|name| name == ONTOLOGY_IMPORTS_TABLE)
    {
        return Ok(None);
    }

    let table = connection
        .open_table(ONTOLOGY_IMPORTS_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open ontology imports table: {}", e))?;
    let filter = format!("id = '{}'", import_id.replace('\'', "''"));
    let result = table
        .query()
        .only_if(filter)
        .limit(1)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to query ontology import '{}': {}", import_id, e))?;
    let batches = result
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| anyhow!("Failed to collect ontology import '{}': {}", import_id, e))?;

    for batch in &batches {
        for row in record_batch_to_value(batch)? {
            if let Some(definition) = row.get("definition_json").and_then(|value| value.as_str()) {
                return serde_json::from_str(definition)
                    .map(Some)
                    .map_err(|e| anyhow!("Failed to parse ontology import: {}", e));
            }
        }
    }
    Ok(None)
}

pub async fn load_ontology_import(
    connection: &Connection,
    import_id: &str,
) -> Result<RemoteOntologyImportDef> {
    find_ontology_import(connection, import_id)
        .await?
        .ok_or_else(|| anyhow!("Ontology import '{}' not found", import_id))
}

pub async fn save_ontology_import(
    connection: &Connection,
    ontology_import: &RemoteOntologyImportDef,
) -> Result<()> {
    use arrow::array::{RecordBatch, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("target_app_id", DataType::Utf8, false),
        Field::new("remote_ontology_id", DataType::Utf8, false),
        Field::new("definition_json", DataType::Utf8, false),
        Field::new("updated_at", DataType::Utf8, false),
    ]));
    let definition_json = serde_json::to_string(ontology_import)
        .map_err(|e| anyhow!("Failed to serialize ontology import: {}", e))?;
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec![ontology_import.id.as_str()])),
            Arc::new(StringArray::from(vec![
                ontology_import.target_app_id.as_str(),
            ])),
            Arc::new(StringArray::from(vec![
                ontology_import.remote_ontology_id.as_str(),
            ])),
            Arc::new(StringArray::from(vec![definition_json.as_str()])),
            Arc::new(StringArray::from(vec![ontology_import.updated_at.as_str()])),
        ],
    )?;

    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;
    if table_names
        .iter()
        .any(|name| name == ONTOLOGY_IMPORTS_TABLE)
    {
        let table = connection
            .open_table(ONTOLOGY_IMPORTS_TABLE)
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to open ontology imports table: {}", e))?;
        let mut merger = table.merge_insert(&["id"]);
        merger
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        let reader: Box<dyn arrow::record_batch::RecordBatchReader + Send> = Box::new(
            arrow::record_batch::RecordBatchIterator::new(vec![Ok(batch)], schema),
        );
        merger
            .execute(reader)
            .await
            .map_err(|e| anyhow!("Failed to upsert ontology import: {}", e))?;
    } else {
        connection
            .create_table(ONTOLOGY_IMPORTS_TABLE, vec![batch])
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to create ontology imports table: {}", e))?;
    }
    Ok(())
}

pub async fn delete_ontology_import(connection: &Connection, import_id: &str) -> Result<()> {
    let table_names = connection
        .table_names()
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to list tables: {}", e))?;
    if !table_names
        .iter()
        .any(|name| name == ONTOLOGY_IMPORTS_TABLE)
    {
        return Ok(());
    }

    let table = connection
        .open_table(ONTOLOGY_IMPORTS_TABLE)
        .execute()
        .await
        .map_err(|e| anyhow!("Failed to open ontology imports table: {}", e))?;
    table
        .delete(&format!("id = '{}'", import_id.replace('\'', "''")))
        .await
        .map_err(|e| anyhow!("Failed to delete ontology import: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn safety() -> CypherSafetyConfig {
        CypherSafetyConfig::default()
    }

    fn parse(query: &str) -> CypherQuery {
        CypherQuery::new(query).expect("query should parse")
    }

    #[test]
    fn preflight_rejects_unbounded_variable_length_paths() {
        let query = parse("MATCH (a:Person)-[r:KNOWS*]->(b:Person) RETURN a, b");
        assert!(preflight_cypher(query.ast(), &safety()).is_err());
    }

    #[test]
    fn preflight_rejects_overdeep_bounds() {
        let query = parse("MATCH (a:Person)-[r:KNOWS*1..99]->(b:Person) RETURN a, b");
        assert!(preflight_cypher(query.ast(), &safety()).is_err());
    }

    #[test]
    fn preflight_accepts_bounded_paths_and_plain_matches() {
        let bounded = parse("MATCH (a:Person)-[r:KNOWS*1..3]->(b:Person) RETURN a, b");
        assert!(preflight_cypher(bounded.ast(), &safety()).is_ok());
        let plain = parse("MATCH (a:Person)-[r:KNOWS]->(b:Person) RETURN a, b LIMIT 10");
        assert!(preflight_cypher(plain.ast(), &safety()).is_ok());
    }

    #[test]
    fn readonly_sql_accepts_selects_only() {
        assert!(validate_readonly_sql("SELECT * FROM people LIMIT 5").is_ok());
        assert!(validate_readonly_sql("WITH x AS (SELECT 1) SELECT * FROM x").is_ok());
        assert!(validate_readonly_sql("DROP TABLE people").is_err());
        assert!(validate_readonly_sql("COPY people TO '/tmp/out.csv'").is_err());
        assert!(
            validate_readonly_sql("CREATE EXTERNAL TABLE t STORED AS CSV LOCATION '/etc/passwd'")
                .is_err()
        );
        assert!(validate_readonly_sql("SELECT 1; SELECT 2").is_err());
        assert!(validate_readonly_sql("INSERT INTO people VALUES (1)").is_err());
    }

    #[test]
    fn contract_hash_binds_per_action_exposure() {
        let object = NodeMappingDef {
            id: Some("shipment".to_string()),
            api_name: Some("shipment".to_string()),
            label: "Shipment".to_string(),
            table: "shipments".to_string(),
            id_column: "id".to_string(),
            display_column: None,
            property_columns: Vec::new(),
            style: Value::Null,
        };
        let mut action = OntologyActionDef {
            id: "approve".to_string(),
            name: "Approve".to_string(),
            description: None,
            object_type: "shipment".to_string(),
            board_id: "board".to_string(),
            board_version: Some([1, 0, 0]),
            start_node_id: Some("start".to_string()),
            event_id: None,
            enabled: true,
            allow_bulk: false,
            parameter_schema: None,
            exposed: true,
        };
        let projection = GovernedObjectProjection {
            table: object.table.clone(),
            identity_column: object.id_column.clone(),
            columns: vec![object.id_column.clone()],
        };
        let exposed_hash =
            ontology_action_contract_hash("ont", true, &action, &object, &projection).unwrap();
        action.exposed = false;
        let hidden_hash =
            ontology_action_contract_hash("ont", true, &action, &object, &projection).unwrap();
        assert_ne!(exposed_hash, hidden_hash);
    }

    #[test]
    fn dedupe_drops_edges_with_truncated_endpoints() {
        let nodes = (0..3)
            .map(|index| SubgraphNode {
                id: format!("Person:{index}"),
                label: "Person".to_string(),
                caption: None,
                props: Value::Null,
            })
            .collect::<Vec<_>>();
        let edges = vec![
            SubgraphEdge {
                id: "Person:0-KNOWS->Person:1".to_string(),
                source: "Person:0".to_string(),
                target: "Person:1".to_string(),
                label: "KNOWS".to_string(),
                props: Value::Null,
            },
            SubgraphEdge {
                id: "Person:0-KNOWS->Person:2".to_string(),
                source: "Person:0".to_string(),
                target: "Person:2".to_string(),
                label: "KNOWS".to_string(),
                props: Value::Null,
            },
        ];
        let result = dedupe_and_limit_subgraph(nodes, edges, 2, Vec::new());
        assert_eq!(result.nodes.len(), 2);
        assert!(result.truncated);
        assert_eq!(
            result.edges.len(),
            1,
            "edge to the truncated node must be dropped"
        );
        assert_eq!(result.edges[0].target, "Person:1");
    }

    #[test]
    fn preflight_checks_post_with_clauses() {
        let query = parse("MATCH (a:Person) WITH a MATCH (a)-[r:KNOWS*]->(b:Person) RETURN a, b");
        assert!(preflight_cypher(query.ast(), &safety()).is_err());
    }

    fn node_mapping(label: &str, table: &str, id_column: &str) -> NodeMappingDef {
        NodeMappingDef {
            id: Some(label.to_lowercase()),
            api_name: Some(label.to_lowercase()),
            label: label.to_string(),
            table: table.to_string(),
            id_column: id_column.to_string(),
            display_column: None,
            property_columns: Vec::new(),
            style: Value::Null,
        }
    }

    fn overlay_with(nodes: Vec<NodeMappingDef>, edges: Vec<EdgeMappingDef>) -> GraphOverlayDef {
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
    fn effective_id_column_falls_back_to_node_id_column() {
        let overlay = overlay_with(vec![node_mapping("Name", "names", "name")], Vec::new());
        assert_eq!(
            effective_node_id_column(&overlay, "Name").as_deref(),
            Some("name")
        );
        assert_eq!(effective_node_id_column(&overlay, "Missing"), None);
    }

    // Guards the ontology-action "Object not found" regression: the graph
    // identifies a node by the edge-level `dst_node_column` override, so a
    // client sends that column's value. Object lookups must resolve the same
    // column instead of the node's own `id_column`, or the id never matches.
    #[test]
    fn effective_id_column_honors_edge_override() {
        let edge = EdgeMappingDef {
            id: None,
            api_name: None,
            label: "REFERS".to_string(),
            table: "refs".to_string(),
            src_column: "src".to_string(),
            dst_column: "dst".to_string(),
            src_label: "Source".to_string(),
            dst_label: "Name".to_string(),
            src_node_column: None,
            dst_node_column: Some("id".to_string()),
            containment: false,
            dst_ontology: None,
            dst_binding_id: None,
            property_columns: Vec::new(),
            style: Value::Null,
        };
        let overlay = overlay_with(
            vec![
                node_mapping("Source", "sources", "id"),
                node_mapping("Name", "names", "name"),
            ],
            vec![edge],
        );
        assert_eq!(
            effective_node_id_column(&overlay, "Name").as_deref(),
            Some("id"),
            "edge dst_node_column override must win over the node id_column"
        );
    }

    #[test]
    fn action_contract_comparison_detects_effective_identity_changes() {
        let edge = EdgeMappingDef {
            id: Some("knows".to_string()),
            api_name: Some("knows".to_string()),
            label: "KNOWS".to_string(),
            table: "links".to_string(),
            src_column: "source".to_string(),
            dst_column: "target".to_string(),
            src_label: "Person".to_string(),
            dst_label: "Person".to_string(),
            src_node_column: Some("external_id".to_string()),
            dst_node_column: Some("external_id".to_string()),
            containment: false,
            dst_ontology: None,
            dst_binding_id: None,
            property_columns: Vec::new(),
            style: Value::Null,
        };
        let mut left = overlay_with(vec![node_mapping("Person", "people", "id")], vec![edge]);
        left.actions.push(OntologyActionDef {
            id: "approve".to_string(),
            name: "Approve".to_string(),
            description: None,
            object_type: "person".to_string(),
            board_id: "board".to_string(),
            board_version: Some([1, 0, 0]),
            start_node_id: Some("start".to_string()),
            event_id: None,
            enabled: true,
            allow_bulk: false,
            parameter_schema: None,
            exposed: false,
        });
        let mut right = left.clone();
        right.edges[0].src_node_column = Some("alternate_id".to_string());
        right.edges[0].dst_node_column = Some("alternate_id".to_string());

        assert!(!ontology_action_contracts_equal(&left, &right).unwrap());
    }

    #[tokio::test]
    async fn freezing_remote_contract_rejects_missing_required_columns() -> Result<()> {
        use arrow::array::{RecordBatch, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use lancedb::connect;

        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let connection = connect(&test_path).execute().await?;
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(StringArray::from(vec!["1"]))])?;
        connection
            .create_table("people", vec![batch])
            .execute()
            .await?;

        let mut overlay = overlay_with(
            vec![NodeMappingDef {
                id: Some("person".to_string()),
                api_name: Some("person".to_string()),
                label: "Person".to_string(),
                table: "people".to_string(),
                id_column: "id".to_string(),
                display_column: Some("missing_display".to_string()),
                property_columns: Vec::new(),
                style: Value::Null,
            }],
            Vec::new(),
        );
        let error = freeze_remote_contract_projection(&connection, &mut overlay)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("missing_display"));

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    // Faithful reproduction of the reported "Object '<id>' was not found" bug:
    // a plain node table with a string `id` column, looked up by that id.
    #[tokio::test]
    async fn load_overlay_objects_finds_row_by_string_id() -> Result<()> {
        use arrow::array::{Int64Array, RecordBatch, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use lancedb::connect;

        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let connection = connect(&test_path).execute().await?;

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, true),
            Field::new("int", DataType::Int64, true),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(StringArray::from(vec![
                    "nppvjlghzrlto3iefrk9bx36",
                    "someotherobjectidentifier",
                ])),
                Arc::new(StringArray::from(vec![Some("Omari Yost"), Some("Ada Lin")])),
                Arc::new(Int64Array::from(vec![0, 1])),
            ],
        )?;
        connection
            .create_table("names", vec![batch])
            .execute()
            .await?;

        let overlay = overlay_with(
            vec![NodeMappingDef {
                id: Some("Name".to_string()),
                api_name: Some("Name".to_string()),
                label: "Name".to_string(),
                table: "names".to_string(),
                id_column: "id".to_string(),
                display_column: Some("name".to_string()),
                property_columns: vec![PropertyColumnDef {
                    name: "int".to_string(),
                    data_type: "Int64".to_string(),
                    nullable: true,
                }],
                style: Value::Null,
            }],
            Vec::new(),
        );

        let result = load_overlay_objects(
            &connection,
            &overlay,
            "Name",
            &[Value::String("nppvjlghzrlto3iefrk9bx36".to_string())],
        )
        .await;

        std::fs::remove_dir_all(&test_path).ok();

        let objects = result?;
        assert_eq!(objects.len(), 1, "expected exactly one object");
        assert_eq!(
            objects[0].get("id").and_then(Value::as_str),
            Some("nppvjlghzrlto3iefrk9bx36")
        );
        Ok(())
    }

    #[tokio::test]
    async fn empty_property_mapping_projects_all_scalar_columns() -> Result<()> {
        use arrow::array::{BinaryArray, RecordBatch, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use lancedb::connect;

        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let connection = connect(&test_path).execute().await?;
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("status", DataType::Utf8, false),
            Field::new("embedding", DataType::Binary, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["1"])),
                Arc::new(StringArray::from(vec!["Ada"])),
                Arc::new(StringArray::from(vec!["active"])),
                Arc::new(BinaryArray::from(vec![Some(&b"vector"[..])])),
            ],
        )?;
        connection
            .create_table("people", vec![batch])
            .execute()
            .await?;

        let mapping = NodeMappingDef {
            id: Some("person".to_string()),
            api_name: Some("person".to_string()),
            label: "Person".to_string(),
            table: "people".to_string(),
            id_column: "id".to_string(),
            display_column: Some("name".to_string()),
            property_columns: Vec::new(),
            style: Value::Null,
        };
        let overlay = overlay_with(vec![mapping.clone()], Vec::new());
        let action = OntologyActionDef {
            id: "inspect".to_string(),
            name: "Inspect".to_string(),
            description: None,
            object_type: "person".to_string(),
            board_id: "board".to_string(),
            board_version: Some([1, 0, 0]),
            start_node_id: Some("start".to_string()),
            event_id: None,
            enabled: true,
            allow_bulk: false,
            parameter_schema: None,
            exposed: false,
        };
        let governed_projection =
            resolve_governed_object_projection(&connection, &overlay, &action).await?;

        let sampled = sample_overlay(&connection, &overlay, "person", 1).await?;
        let directly_sampled = sample_overlay_object(
            &connection,
            &mapping,
            &mapping.id_column,
            PropertyProjectionMode::Dynamic,
            1,
        )
        .await?;
        let loaded = load_overlay_objects(
            &connection,
            &overlay,
            "person",
            &[Value::String("1".to_string())],
        )
        .await?;

        for rows in [&sampled, &directly_sampled, &loaded] {
            assert_eq!(
                rows[0].get("status").and_then(Value::as_str),
                Some("active")
            );
            assert!(
                rows[0].get("embedding").is_none(),
                "vector/binary columns must stay out of the default projection"
            );
        }

        let mut frozen_overlay = overlay.clone();
        frozen_overlay.property_projection_mode = PropertyProjectionMode::Frozen;
        let frozen_sample = sample_overlay(&connection, &frozen_overlay, "person", 1).await?;
        let frozen_direct = sample_overlay_object(
            &connection,
            &mapping,
            &mapping.id_column,
            PropertyProjectionMode::Frozen,
            1,
        )
        .await?;
        let frozen_loaded = load_overlay_objects(
            &connection,
            &frozen_overlay,
            "person",
            &[Value::String("1".to_string())],
        )
        .await?;
        for rows in [&frozen_sample, &frozen_direct, &frozen_loaded] {
            assert!(
                rows[0].get("status").is_none(),
                "a frozen empty property set must not re-expand against the live schema"
            );
            assert_eq!(rows[0].get("id").and_then(Value::as_str), Some("1"));
        }

        use lancedb::table::NewColumnTransform;
        connection
            .open_table("people")
            .execute()
            .await?
            .add_columns(
                NewColumnTransform::SqlExpressions(vec![(
                    "secret".to_string(),
                    "'added later'".to_string(),
                )]),
                None,
            )
            .await?;
        let explored = load_overlay_objects(
            &connection,
            &overlay,
            "person",
            &[Value::String("1".to_string())],
        )
        .await?;
        assert_eq!(
            explored[0].get("secret").and_then(Value::as_str),
            Some("added later"),
            "ordinary local exploration keeps its dynamic wildcard"
        );
        let governed = load_overlay_objects_with_projection(
            &connection,
            &overlay,
            &action,
            &governed_projection,
            &[Value::String("1".to_string())],
        )
        .await?;
        assert!(
            governed[0].get("secret").is_none(),
            "a governed action must use the stored concrete projection"
        );
        assert_eq!(
            governed[0].get("status").and_then(Value::as_str),
            Some("active")
        );

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn object_loading_uses_effective_edge_join_identity() -> Result<()> {
        use arrow::array::{RecordBatch, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use lancedb::connect;

        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let connection = connect(&test_path).execute().await?;
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("external_id", DataType::Utf8, false),
            Field::new("name", DataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["internal-1"])),
                Arc::new(StringArray::from(vec!["public-1"])),
                Arc::new(StringArray::from(vec!["Ada"])),
            ],
        )?;
        connection
            .create_table("people", vec![batch])
            .execute()
            .await?;
        let edge_schema = Arc::new(Schema::new(vec![
            Field::new("source", DataType::Utf8, false),
            Field::new("target", DataType::Utf8, false),
        ]));
        let edge_batch = RecordBatch::try_new(
            edge_schema,
            vec![
                Arc::new(StringArray::from(vec!["public-1"])),
                Arc::new(StringArray::from(vec!["public-1"])),
            ],
        )?;
        connection
            .create_table("links", vec![edge_batch])
            .execute()
            .await?;

        let edge = EdgeMappingDef {
            id: Some("knows".to_string()),
            api_name: Some("knows".to_string()),
            label: "KNOWS".to_string(),
            table: "links".to_string(),
            src_column: "source".to_string(),
            dst_column: "target".to_string(),
            src_label: "Person".to_string(),
            dst_label: "Person".to_string(),
            src_node_column: Some("external_id".to_string()),
            dst_node_column: Some("external_id".to_string()),
            containment: false,
            dst_ontology: None,
            dst_binding_id: None,
            property_columns: Vec::new(),
            style: Value::Null,
        };
        let overlay = overlay_with(
            vec![NodeMappingDef {
                id: Some("person".to_string()),
                api_name: Some("person".to_string()),
                label: "Person".to_string(),
                table: "people".to_string(),
                id_column: "id".to_string(),
                display_column: Some("name".to_string()),
                property_columns: vec![PropertyColumnDef {
                    name: "name".to_string(),
                    data_type: "Utf8".to_string(),
                    nullable: false,
                }],
                style: Value::Null,
            }],
            vec![edge],
        );

        let sampled = sample_overlay(&connection, &overlay, "person", 1).await?;
        assert_eq!(
            sampled[0].get("external_id").and_then(Value::as_str),
            Some("public-1")
        );
        let frozen_remote_sample = sample_overlay_object(
            &connection,
            &overlay.nodes[0],
            "external_id",
            PropertyProjectionMode::Frozen,
            1,
        )
        .await?;
        assert_eq!(
            frozen_remote_sample[0]
                .get("external_id")
                .and_then(Value::as_str),
            Some("public-1"),
            "a frozen remote sample must retain the effective edge identity override"
        );
        let sampled_edges = sample_overlay(&connection, &overlay, "knows", 1).await?;
        assert_eq!(
            sampled_edges[0].get("source").and_then(Value::as_str),
            Some("public-1")
        );
        let loaded = load_overlay_objects(
            &connection,
            &overlay,
            "person",
            &[Value::String("public-1".to_string())],
        )
        .await?;
        assert_eq!(
            loaded[0].get("id").and_then(Value::as_str),
            Some("internal-1")
        );
        assert_eq!(
            loaded[0].get("external_id").and_then(Value::as_str),
            Some("public-1")
        );

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    // LanceDB's `only_if` filter parser reads a double-quoted `"col"` as a
    // string LITERAL, so `quote_identifier` there silently matches nothing;
    // backticks delimit the column. This pins that dialect distinction.
    #[tokio::test]
    async fn only_if_identifier_quoting_matches_lance_dialect() -> Result<()> {
        use arrow::array::{RecordBatch, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use lancedb::connect;

        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let connection = connect(&test_path).execute().await?;
        let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
        let batch = RecordBatch::try_new(
            schema,
            vec![Arc::new(StringArray::from(vec![
                "nppvjlghzrlto3iefrk9bx36",
            ]))],
        )?;
        connection
            .create_table("names", vec![batch])
            .execute()
            .await?;
        let table = connection.open_table("names").execute().await?;

        async fn count(table: &lancedb::table::Table, predicate: &str) -> usize {
            table
                .query()
                .only_if(predicate)
                .execute()
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .iter()
                .map(arrow::record_batch::RecordBatch::num_rows)
                .sum()
        }

        let id = "nppvjlghzrlto3iefrk9bx36";
        assert_eq!(
            count(&table, &format!("\"id\" IN ('{id}')")).await,
            0,
            "double-quoted identifier is a string literal to LanceDB — matches nothing"
        );
        assert_eq!(
            count(&table, &format!("{} IN ('{id}')", filter_identifier("id"))).await,
            1,
            "filter_identifier must produce a real column reference"
        );

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }
}
