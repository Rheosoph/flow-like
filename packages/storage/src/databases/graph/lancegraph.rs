use super::{
    GraphLabelInfo, GraphPropertyInfo, GraphSchemaResult, GraphStore, SubgraphEdge, SubgraphNode,
    SubgraphResult, TraversalDirection,
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
use std::sync::Arc;

const MAX_QUERY_DEPTH: usize = 5;
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const MAX_CONCURRENT_QUERIES: usize = 4;
const MAX_QUERY_LIMIT: usize = 1_000_000;

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
    pub default_limit: usize,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PropertyColumnDef {
    pub name: String,
    pub data_type: String,
    #[serde(default)]
    pub nullable: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NodeMappingDef {
    pub label: String,
    pub table: String,
    pub id_column: String,
    pub display_column: Option<String>,
    pub property_columns: Vec<PropertyColumnDef>,
    pub style: serde_json::Value,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EdgeMappingDef {
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
        let semaphore = Arc::new(tokio::sync::Semaphore::new(safety.max_concurrent));

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

        let limited_query = append_limit_clause(query, limit);
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

        let ctx = self.build_query_context(true).await?;

        let df = ctx.sql(query).await?;
        let batches = df.collect().await?;

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
        _depth: usize,
        direction: TraversalDirection,
        limit: Option<usize>,
    ) -> Result<SubgraphResult> {
        let limit = self.enforce_limit(limit);
        let id_col = self.find_id_column_for_label(label)?;

        let mut all_nodes = Vec::new();
        let mut all_edges = Vec::new();

        // (query, n_label, m_label, n_is_source)
        // n is always the seed variable, m is the neighbor
        for edge in &self.overlay.edges {
            let mut query_infos: Vec<(String, &str, &str, bool)> = Vec::new();

            let is_src = edge.src_label == label;
            let is_dst = edge.dst_label == label;

            if matches!(
                direction,
                TraversalDirection::Outgoing | TraversalDirection::Both
            ) && is_src
            {
                query_infos.push((
                    format!(
                        "MATCH (n:{src})-[r:{rel}]->(m:{dst}) WHERE n.{id_col} = $seed_id RETURN n, m",
                        src = edge.src_label, rel = edge.label, dst = edge.dst_label,
                    ),
                    &edge.src_label,
                    &edge.dst_label,
                    true,
                ));
            }
            if matches!(
                direction,
                TraversalDirection::Incoming | TraversalDirection::Both
            ) && is_dst
            {
                query_infos.push((
                    format!(
                        "MATCH (m:{src})-[r:{rel}]->(n:{dst}) WHERE n.{id_col} = $seed_id RETURN n, m",
                        src = edge.src_label, rel = edge.label, dst = edge.dst_label,
                    ),
                    &edge.dst_label,
                    &edge.src_label,
                    false,
                ));
            }

            for (query, n_label, m_label, n_is_source) in query_infos {
                let mut params = HashMap::new();
                params.insert("seed_id".to_string(), id.clone());

                match self.execute_cypher_with_safety(&query, params, limit).await {
                    Ok(batch) => {
                        let rows = record_batch_to_value(&batch)?;
                        let sub = self.parse_flat_rows(
                            &rows,
                            n_label,
                            m_label,
                            &edge.label,
                            n_is_source,
                            limit,
                        )?;
                        all_nodes.extend(sub.nodes);
                        all_edges.extend(sub.edges);
                    }
                    Err(e) => {
                        eprintln!("neighbors query failed for edge '{}': {}", edge.label, e);
                    }
                }
            }
        }

        Ok(dedupe_and_limit_subgraph(all_nodes, all_edges, limit))
    }

    async fn subgraph(
        &self,
        seeds: Vec<(String, Value)>,
        depth: usize,
        limit: Option<usize>,
    ) -> Result<SubgraphResult> {
        let depth = depth.min(self.safety.max_depth);
        let limit = self.enforce_limit(limit);

        if seeds.is_empty() {
            return self.full_subgraph(limit).await;
        }

        let mut all_nodes = Vec::new();
        let mut all_edges = Vec::new();

        for (label, id) in seeds {
            let result = self
                .neighbors(&label, id, depth, TraversalDirection::Both, Some(limit))
                .await?;
            all_nodes.extend(result.nodes);
            all_edges.extend(result.edges);
        }

        Ok(dedupe_and_limit_subgraph(all_nodes, all_edges, limit))
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

        let mut matches = Vec::new();
        let mut seen_node_ids = HashSet::new();

        for node in &self.overlay.nodes {
            let searchable_columns = self.searchable_columns_for_label(&node.label)?;
            if searchable_columns.is_empty() {
                continue;
            }

            let where_clause = searchable_columns
                .iter()
                .map(|column| {
                    let quoted = quote_identifier(column);
                    format!("CAST({quoted} AS VARCHAR) ILIKE {pattern}")
                })
                .collect::<Vec<_>>()
                .join(" OR ");

            let sql = format!(
                "SELECT * FROM {} WHERE {} LIMIT {}",
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
        let table_name = if let Some(nd) = self.overlay.nodes.iter().find(|nd| nd.label == label) {
            &nd.table
        } else if let Some(edge_def) = self.overlay.edges.iter().find(|ed| ed.label == label) {
            &edge_def.table
        } else {
            return Err(anyhow!("Label '{}' not found in overlay", label));
        };

        let table = self
            .connection
            .open_table(table_name)
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to open table '{}': {}", table_name, e))?;

        let result = table
            .query()
            .limit(n)
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to query table '{}': {}", table_name, e))?;

        let batches = result
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| anyhow!("Failed to collect from '{}': {}", table_name, e))?;

        let mut results = Vec::new();
        for batch in &batches {
            let vals = record_batch_to_value(batch)?;
            results.extend(vals);
        }
        results.truncate(n);
        Ok(results)
    }
}

impl LanceGraphStore {
    fn find_id_column_for_label(&self, label: &str) -> Result<String> {
        for node in &self.overlay.nodes {
            if node.label == label {
                // Respect edge-level overrides (src_node_column / dst_node_column)
                // to stay consistent with what build_graph_config passes to lance-graph.
                for edge in &self.overlay.edges {
                    if edge.src_label == label
                        && let Some(ref col) = edge.src_node_column
                    {
                        return Ok(col.clone());
                    }
                    if edge.dst_label == label
                        && let Some(ref col) = edge.dst_node_column
                    {
                        return Ok(col.clone());
                    }
                }
                return Ok(node.id_column.clone());
            }
        }
        Err(anyhow!(
            "Label '{}' not found in overlay node mappings",
            label
        ))
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

        let table = self
            .connection
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
        let adapter = Arc::new(adapter);
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

        for edge in &self.overlay.edges {
            let query = format!(
                "MATCH (n:{src})-[r:{rel}]->(m:{dst}) RETURN n, m",
                src = edge.src_label,
                rel = edge.label,
                dst = edge.dst_label,
            );
            let batch = self
                .execute_cypher_with_safety(&query, HashMap::new(), limit)
                .await?;
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

        Ok(dedupe_and_limit_subgraph(all_nodes, all_edges, limit))
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
    if has_limit_clause(trimmed) {
        return trimmed.to_string();
    }

    let trimmed = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    format!("{trimmed} LIMIT {limit}")
}

fn has_limit_clause(query: &str) -> bool {
    query
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
        .any(|token| token.eq_ignore_ascii_case("limit"))
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
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
) -> SubgraphResult {
    let mut seen_node_ids = HashSet::new();
    all_nodes.retain(|node| seen_node_ids.insert(node.id.clone()));

    let mut seen_edge_ids = HashSet::new();
    all_edges.retain(|edge| seen_edge_ids.insert(edge.id.clone()));

    let truncated = all_nodes.len() >= limit || all_edges.len() >= limit.saturating_mul(3);
    all_nodes.truncate(limit);
    all_edges.truncate(limit.saturating_mul(3));

    SubgraphResult {
        nodes: all_nodes,
        edges: all_edges,
        truncated,
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
    schema_cache: &mut HashMap<String, Vec<String>>,
    excluded: &HashSet<String>,
    always_include: &[String],
) -> Result<Vec<String>> {
    let mut prop_names = if !configured.is_empty() {
        configured
            .iter()
            .map(|p| p.name.clone())
            .collect::<Vec<_>>()
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

    // Collect per-label id_field overrides from edges.
    // If an edge specifies src_node_column/dst_node_column, that overrides the
    // node's default id_column for the join.
    let mut label_id_overrides: HashMap<String, String> = HashMap::new();
    for edge in &overlay.edges {
        if let Some(ref col) = edge.src_node_column {
            label_id_overrides
                .entry(edge.src_label.clone())
                .or_insert_with(|| col.clone());
        }
        if let Some(ref col) = edge.dst_node_column {
            label_id_overrides
                .entry(edge.dst_label.clone())
                .or_insert_with(|| col.clone());
        }
    }

    for node in &overlay.nodes {
        let id_col = label_id_overrides
            .get(&node.label)
            .unwrap_or(&node.id_column);
        let excluded = HashSet::from([id_col.clone()]);
        let always_include = node
            .display_column
            .as_ref()
            .filter(|column| *column != id_col)
            .cloned()
            .into_iter()
            .collect::<Vec<_>>();
        let prop_names = resolve_property_names(
            connection,
            &node.table,
            &node.property_columns,
            &mut schema_cache,
            &excluded,
            &always_include,
        )
        .await?;
        let mapping =
            lance_graph::NodeMapping::new(&node.label, id_col).with_properties(prop_names);
        builder = builder.with_node_mapping(mapping);
    }

    for edge in &overlay.edges {
        let excluded = HashSet::from([edge.src_column.clone(), edge.dst_column.clone()]);
        let prop_names = resolve_property_names(
            connection,
            &edge.table,
            &edge.property_columns,
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
                let overlay: GraphOverlayDef = serde_json::from_str(def_json)
                    .map_err(|e| anyhow!("Failed to parse overlay definition: {}", e))?;
                overlays.push(overlay);
            }
        }
    }

    Ok(overlays)
}

pub async fn load_overlay(connection: &Connection, overlay_id: &str) -> Result<GraphOverlayDef> {
    let overlays = list_overlays(connection).await?;
    overlays
        .into_iter()
        .find(|o| o.id == overlay_id)
        .ok_or_else(|| anyhow!("Overlay '{}' not found", overlay_id))
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

        table
            .delete(&format!("id = '{}'", overlay.id.replace('\'', "''")))
            .await
            .map_err(|e| anyhow!("Failed to delete old overlay row: {}", e))?;

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
