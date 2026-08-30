//! Multi-hop traversal, path finding, and structural analytics.
//!
//! lance-graph has no algorithms module, so traversal beyond one hop is done
//! here: breadth-first frontier expansion with batched `IN` scans directly on
//! the mapped Lance tables, and petgraph for path reconstruction and metrics.

#[cfg(test)]
use super::PropertyProjectionMode;
use super::{
    GraphOverlayDef, LanceGraphStore, dedupe_and_limit_subgraph, filter_identifier, load_overlay,
    resolve_object_mapping, resolve_property_names, value_sql_literal, value_to_id_string,
};
use crate::arrow_utils::record_batch_to_value;
use crate::databases::graph::{
    EdgeLabelCount, GraphAnalyticsResult, GraphPath, GraphPathsResult, LabelCount, NodeMetric,
    SubgraphEdge, SubgraphNode, SubgraphNodeStats, SubgraphResult, TraversalDirection,
};
use flow_like_types::{Result, Value, anyhow};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use petgraph::unionfind::UnionFind;
use std::collections::{HashMap, HashSet};

const IN_CHUNK_SIZE: usize = 400;
const ANALYTICS_MAX_EDGES: usize = 100_000;
const TOP_METRIC_NODES: usize = 25;
const MAX_ALTERNATIVE_PATHS: usize = 3;
/// Rows the seedless sampler looks at to work out which objects a relationship
/// table is actually about. Only the two endpoint columns are read, so this is
/// far cheaper than the projected scan that follows it.
const CENSUS_MAX_ROWS: usize = 20_000;
const CENSUS_MIN_ROWS: usize = 2_000;
const CENSUS_BUDGET_FACTOR: usize = 24;
/// How many distinct sources the first view aims to show, and how many of each
/// source's neighbours it carries.
const HUB_TARGET: usize = 24;
const MAX_CHILDREN_PER_HUB: usize = 8;
/// Ceiling on the projected re-read that puts edge properties back onto the
/// sampled pairs.
const SAMPLE_PROPS_MAX_ROWS: usize = 4_000;
/// Rows read per chosen pair: the filter matches a whole source, so the window
/// has to overshoot for the wanted pairs to be inside it.
const SAMPLE_PROPS_OVERSCAN: usize = 8;

/// A node discovered during expansion, keyed by its full `label:id` identity.
#[derive(Clone)]
struct Discovered {
    label: String,
    raw_id: Value,
}

struct ExpansionState {
    nodes: HashMap<String, Discovered>,
    edges: Vec<SubgraphEdge>,
    seen_edge_ids: HashSet<String>,
    warnings: Vec<String>,
    truncated: bool,
    /// Population counts, keyed by full `label:id`. Only the seedless sampler
    /// fills this — every other path leaves it empty.
    node_stats: HashMap<String, SubgraphNodeStats>,
}

impl ExpansionState {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            seen_edge_ids: HashSet::new(),
            warnings: Vec::new(),
            truncated: false,
            node_stats: HashMap::new(),
        }
    }

    fn record_fan_out(&mut self, full_id: &str, label: &str, count: usize, exact: bool) {
        let stats = self
            .node_stats
            .entry(full_id.to_string())
            .or_insert_with(|| SubgraphNodeStats {
                out_by_label: Vec::new(),
                exact: true,
            });
        stats.exact = stats.exact && exact;
        match stats
            .out_by_label
            .iter_mut()
            .find(|entry| entry.label == label)
        {
            Some(entry) => entry.count = entry.count.saturating_add(count),
            None => stats.out_by_label.push(EdgeLabelCount {
                label: label.to_string(),
                count,
            }),
        }
    }
}

/// What the seedless sampler took from one relationship table.
struct SampledEdgeRows {
    rows: Vec<Value>,
    /// Fan-out of each chosen source within the census window, keyed by the
    /// source's raw id string.
    fan_out: Vec<(String, usize)>,
    /// True when the census window covered the whole table, which makes every
    /// fan-out an exact count rather than a lower bound.
    exact: bool,
    /// True when the table holds rows this sample left behind.
    more_rows: bool,
}

/// Containment children that a sibling overlay maps, keyed by
/// `(overlay id, child label, id-column override)` and holding one
/// `(parent key, child id, edge)` entry per discovered child.
type ExternalChildren =
    HashMap<(String, String, Option<String>), Vec<(String, Value, SubgraphEdge)>>;

impl LanceGraphStore {
    async fn open_table_cached(
        &self,
        name: &str,
        cache: &mut HashMap<String, lancedb::Table>,
    ) -> Result<lancedb::Table> {
        if let Some(table) = cache.get(name) {
            return Ok(table.clone());
        }
        let table = self
            .connection
            .open_table(name)
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to open table '{}': {}", name, e))?;
        cache.insert(name.to_string(), table.clone());
        Ok(table)
    }

    async fn edge_rows(
        &self,
        table: &lancedb::Table,
        columns: &[String],
        predicate: Option<String>,
        limit: usize,
    ) -> Result<Vec<Value>> {
        let mut query = table
            .query()
            .select(lancedb::query::Select::Columns(columns.to_vec()))
            .limit(limit);
        if let Some(predicate) = predicate {
            query = query.only_if(predicate);
        }
        let batches = query
            .execute()
            .await
            .map_err(|e| anyhow!("Failed to scan edge table: {}", e))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| anyhow!("Failed to collect edge rows: {}", e))?;
        let mut rows = Vec::new();
        for batch in &batches {
            rows.extend(record_batch_to_value(batch)?);
        }
        Ok(rows)
    }

    async fn edge_rows_for_ids(
        &self,
        table: &lancedb::Table,
        filter_column: &str,
        columns: &[String],
        ids: &[Value],
        limit: usize,
    ) -> Result<Vec<Value>> {
        let literals = ids
            .iter()
            .map(value_sql_literal)
            .collect::<Result<Vec<_>>>()?;
        let predicate = format!(
            "{} IN ({})",
            filter_identifier(filter_column),
            literals.join(", ")
        );
        self.edge_rows(table, columns, Some(predicate), limit).await
    }

    /// Column projection and property names for one edge mapping.
    async fn edge_projection(
        &self,
        edge: &super::EdgeMappingDef,
        schema_cache: &mut HashMap<String, Vec<String>>,
    ) -> Result<(Vec<String>, Vec<String>)> {
        let excluded = HashSet::from([edge.src_column.clone(), edge.dst_column.clone()]);
        let prop_names = resolve_property_names(
            &self.connection,
            &edge.table,
            &edge.property_columns,
            self.overlay.property_projection_mode,
            schema_cache,
            &excluded,
            &[],
        )
        .await?;
        let mut columns = vec![edge.src_column.clone(), edge.dst_column.clone()];
        columns.extend(prop_names.iter().cloned());
        let mut seen_columns = HashSet::new();
        columns.retain(|column| seen_columns.insert(column.clone()));
        Ok((columns, prop_names))
    }

    fn edge_endpoints_mapped(&self, edge: &super::EdgeMappingDef) -> bool {
        self.overlay
            .nodes
            .iter()
            .any(|node| node.label == edge.src_label)
            && self
                .overlay
                .nodes
                .iter()
                .any(|node| node.label == edge.dst_label)
    }

    /// Breadth-first expansion from seed objects across all edge mappings,
    /// honoring depth, direction, and node/edge budgets.
    pub(super) async fn expand_subgraph(
        &self,
        seeds: Vec<(String, Value)>,
        depth: usize,
        direction: TraversalDirection,
        node_limit: usize,
        edge_labels: Option<&[String]>,
    ) -> Result<SubgraphResult> {
        let depth = depth.clamp(1, self.safety.max_depth);
        let node_limit = node_limit.max(1);
        let edge_limit = node_limit.saturating_mul(3);

        let mut state = ExpansionState::new();
        let mut table_cache: HashMap<String, lancedb::Table> = HashMap::new();
        let mut schema_cache: HashMap<String, Vec<String>> = HashMap::new();

        let mut frontier: Vec<(String, Value)> = Vec::new();
        for (label, raw_id) in seeds {
            if !self.overlay.nodes.iter().any(|node| node.label == label) {
                return Err(anyhow!(
                    "Label '{}' not found in overlay node mappings",
                    label
                ));
            }
            let full_id = format!("{}:{}", label, value_to_id_string(Some(&raw_id)));
            if state
                .nodes
                .insert(
                    full_id,
                    Discovered {
                        label: label.clone(),
                        raw_id: raw_id.clone(),
                    },
                )
                .is_none()
            {
                frontier.push((label, raw_id));
            }
        }

        for _hop in 0..depth {
            if frontier.is_empty() || state.truncated {
                break;
            }
            let mut ids_by_label: HashMap<&str, Vec<Value>> = HashMap::new();
            for (label, raw_id) in &frontier {
                ids_by_label
                    .entry(label.as_str())
                    .or_default()
                    .push(raw_id.clone());
            }

            let mut next_frontier: Vec<(String, Value)> = Vec::new();
            for edge in &self.overlay.edges {
                // An empty allowlist is treated as "no restriction" rather than as
                // "nothing", so a caller that sends its filter unset cannot silently
                // receive an empty graph.
                if let Some(allowed) = edge_labels
                    && !allowed.is_empty()
                    && !allowed.iter().any(|label| label == &edge.label)
                {
                    continue;
                }

                let mut sides: Vec<(&str, &str, &str, &str, bool)> = Vec::new();
                // (filter_col, filter_label, neighbor_label, neighbor_col, filter_is_source)
                if matches!(
                    direction,
                    TraversalDirection::Outgoing | TraversalDirection::Both
                ) && ids_by_label.contains_key(edge.src_label.as_str())
                {
                    sides.push((
                        edge.src_column.as_str(),
                        edge.src_label.as_str(),
                        edge.dst_label.as_str(),
                        edge.dst_column.as_str(),
                        true,
                    ));
                }
                if matches!(
                    direction,
                    TraversalDirection::Incoming | TraversalDirection::Both
                ) && ids_by_label.contains_key(edge.dst_label.as_str())
                {
                    sides.push((
                        edge.dst_column.as_str(),
                        edge.dst_label.as_str(),
                        edge.src_label.as_str(),
                        edge.src_column.as_str(),
                        false,
                    ));
                }
                if sides.is_empty() || !self.edge_endpoints_mapped(edge) {
                    continue;
                }

                let table = match self.open_table_cached(&edge.table, &mut table_cache).await {
                    Ok(table) => table,
                    Err(error) => {
                        state
                            .warnings
                            .push(format!("Edge mapping '{}': {}", edge.label, error));
                        continue;
                    }
                };
                let (columns, prop_names) =
                    match self.edge_projection(edge, &mut schema_cache).await {
                        Ok(projection) => projection,
                        Err(error) => {
                            state
                                .warnings
                                .push(format!("Edge mapping '{}': {}", edge.label, error));
                            continue;
                        }
                    };

                for (filter_col, filter_label, neighbor_label, _neighbor_col, filter_is_source) in
                    sides
                {
                    let Some(ids) = ids_by_label.get(filter_label) else {
                        continue;
                    };
                    for chunk in ids.chunks(IN_CHUNK_SIZE) {
                        if state.truncated {
                            break;
                        }
                        let remaining_edges =
                            edge_limit.saturating_sub(state.edges.len()).min(edge_limit);
                        if remaining_edges == 0 {
                            state.truncated = true;
                            break;
                        }
                        let rows = match self
                            .edge_rows_for_ids(&table, filter_col, &columns, chunk, remaining_edges)
                            .await
                        {
                            Ok(rows) => rows,
                            Err(error) => {
                                state
                                    .warnings
                                    .push(format!("Edge mapping '{}': {}", edge.label, error));
                                continue;
                            }
                        };
                        for row in rows {
                            let Value::Object(map) = row else { continue };
                            let src_raw = map.get(&edge.src_column).cloned();
                            let dst_raw = map.get(&edge.dst_column).cloned();
                            let src_key = value_to_id_string(src_raw.as_ref());
                            let dst_key = value_to_id_string(dst_raw.as_ref());
                            if src_key.is_empty() || dst_key.is_empty() {
                                continue;
                            }
                            let src_full = format!("{}:{}", edge.src_label, src_key);
                            let dst_full = format!("{}:{}", edge.dst_label, dst_key);
                            let edge_id = format!("{}-{}->{}", src_full, edge.label, dst_full);
                            if state.seen_edge_ids.insert(edge_id.clone()) {
                                if state.edges.len() >= edge_limit {
                                    state.truncated = true;
                                    break;
                                }
                                let mut props = serde_json::Map::new();
                                for name in &prop_names {
                                    if let Some(value) = map.get(name) {
                                        props.insert(name.clone(), value.clone());
                                    }
                                }
                                state.edges.push(SubgraphEdge {
                                    id: edge_id,
                                    source: src_full.clone(),
                                    target: dst_full.clone(),
                                    label: edge.label.clone(),
                                    props: Value::Object(props),
                                });
                            }
                            let (neighbor_full, neighbor_raw) = if filter_is_source {
                                (dst_full, dst_raw)
                            } else {
                                (src_full, src_raw)
                            };
                            let Some(neighbor_raw) = neighbor_raw else {
                                continue;
                            };
                            if !state.nodes.contains_key(&neighbor_full) {
                                if state.nodes.len() >= node_limit {
                                    state.truncated = true;
                                    continue;
                                }
                                state.nodes.insert(
                                    neighbor_full,
                                    Discovered {
                                        label: neighbor_label.to_string(),
                                        raw_id: neighbor_raw.clone(),
                                    },
                                );
                                next_frontier.push((neighbor_label.to_string(), neighbor_raw));
                            }
                        }
                    }
                }
            }
            frontier = next_frontier;
        }

        self.close_edges_among_discovered(
            &mut state,
            edge_limit,
            &mut table_cache,
            &mut schema_cache,
        )
        .await;

        self.hydrate_expansion(state, &mut table_cache, &mut schema_cache)
            .await
    }

    /// Adds the edges that run between objects already in the result.
    ///
    /// Frontier expansion only records the edges it traversed, so two neighbors
    /// found on the same hop come back looking unconnected even though the data
    /// links them. Without this pass the viewer renders a star per expansion and
    /// the real structure only appears once every node is expanded by hand.
    async fn close_edges_among_discovered(
        &self,
        state: &mut ExpansionState,
        edge_limit: usize,
        table_cache: &mut HashMap<String, lancedb::Table>,
        schema_cache: &mut HashMap<String, Vec<String>>,
    ) {
        if state.nodes.len() < 2 || state.edges.len() >= edge_limit {
            return;
        }

        let mut ids_by_label: HashMap<String, Vec<Value>> = HashMap::new();
        for node in state.nodes.values() {
            ids_by_label
                .entry(node.label.clone())
                .or_default()
                .push(node.raw_id.clone());
        }

        for edge in &self.overlay.edges {
            if state.edges.len() >= edge_limit {
                state.truncated = true;
                break;
            }
            if !self.edge_endpoints_mapped(edge) {
                continue;
            }
            let Some(src_ids) = ids_by_label.get(&edge.src_label) else {
                continue;
            };
            if !ids_by_label.contains_key(&edge.dst_label) {
                continue;
            }

            let table = match self.open_table_cached(&edge.table, table_cache).await {
                Ok(table) => table,
                Err(error) => {
                    state
                        .warnings
                        .push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };
            let (columns, prop_names) = match self.edge_projection(edge, schema_cache).await {
                Ok(projection) => projection,
                Err(error) => {
                    state
                        .warnings
                        .push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };

            for chunk in src_ids.chunks(IN_CHUNK_SIZE) {
                let remaining = edge_limit.saturating_sub(state.edges.len());
                if remaining == 0 {
                    state.truncated = true;
                    break;
                }
                let rows = match self
                    .edge_rows_for_ids(&table, &edge.src_column, &columns, chunk, remaining)
                    .await
                {
                    Ok(rows) => rows,
                    Err(error) => {
                        state
                            .warnings
                            .push(format!("Edge mapping '{}': {}", edge.label, error));
                        continue;
                    }
                };
                // Only edges whose other end is already on screen are kept — this
                // pass connects the current result, it never grows it.
                self.absorb_edge_rows(state, edge, &prop_names, rows, edge_limit, None);
            }
        }
    }

    /// Turns raw edge rows into [`SubgraphEdge`]s.
    ///
    /// With `node_limit` set, the endpoints are added to the result until that
    /// budget is spent; with `None`, rows whose endpoints are not already in the
    /// result are dropped. Either way no edge is kept unless both of its
    /// endpoints are, so the result never carries a dangling edge.
    fn absorb_edge_rows(
        &self,
        state: &mut ExpansionState,
        edge: &super::EdgeMappingDef,
        prop_names: &[String],
        rows: Vec<Value>,
        edge_limit: usize,
        node_limit: Option<usize>,
    ) {
        for row in rows {
            let Value::Object(map) = row else { continue };
            let src_raw = map.get(&edge.src_column).cloned();
            let dst_raw = map.get(&edge.dst_column).cloned();
            let src_key = value_to_id_string(src_raw.as_ref());
            let dst_key = value_to_id_string(dst_raw.as_ref());
            if src_key.is_empty() || dst_key.is_empty() {
                continue;
            }
            let src_full = format!("{}:{}", edge.src_label, src_key);
            let dst_full = format!("{}:{}", edge.dst_label, dst_key);

            match node_limit {
                Some(node_limit) => {
                    let (Some(src_raw), Some(dst_raw)) = (src_raw, dst_raw) else {
                        continue;
                    };
                    let admitted = usize::from(!state.nodes.contains_key(&src_full))
                        + usize::from(src_full != dst_full && !state.nodes.contains_key(&dst_full));
                    if state.nodes.len() + admitted > node_limit {
                        state.truncated = true;
                        break;
                    }
                    for (full_id, label, raw_id) in [
                        (&src_full, &edge.src_label, src_raw),
                        (&dst_full, &edge.dst_label, dst_raw),
                    ] {
                        if state.nodes.contains_key(full_id) {
                            continue;
                        }
                        state.nodes.insert(
                            full_id.clone(),
                            Discovered {
                                label: label.clone(),
                                raw_id,
                            },
                        );
                    }
                }
                None => {
                    if !state.nodes.contains_key(&src_full) || !state.nodes.contains_key(&dst_full)
                    {
                        continue;
                    }
                }
            }

            let edge_id = format!("{}-{}->{}", src_full, edge.label, dst_full);
            if !state.seen_edge_ids.insert(edge_id.clone()) {
                continue;
            }
            if state.edges.len() >= edge_limit {
                state.truncated = true;
                break;
            }
            let mut props = serde_json::Map::new();
            for name in prop_names {
                if let Some(value) = map.get(name) {
                    props.insert(name.clone(), value.clone());
                }
            }
            state.edges.push(SubgraphEdge {
                id: edge_id,
                source: src_full,
                target: dst_full,
                label: edge.label.clone(),
                props: Value::Object(props),
            });
        }
    }

    /// The seedless view of an overlay: read the mapped edge tables first and
    /// materialise the objects they connect.
    ///
    /// Sampling node tables instead would return an arbitrary slice per label,
    /// which almost never contains both ends of the same edge — so the first
    /// render shows a cloud of unlinked nodes and the structure only appears
    /// after the user expands nodes one by one.
    pub(super) async fn scan_subgraph(&self, node_limit: usize) -> Result<SubgraphResult> {
        let node_limit = node_limit.max(1);
        let edge_limit = node_limit.saturating_mul(3);

        let mut state = ExpansionState::new();
        let mut table_cache: HashMap<String, lancedb::Table> = HashMap::new();
        let mut schema_cache: HashMap<String, Vec<String>> = HashMap::new();

        let mapped_edges = self
            .overlay
            .edges
            .iter()
            .filter(|edge| self.edge_endpoints_mapped(edge))
            .collect::<Vec<_>>();
        // Every mapping gets a share of the budget so one large relationship
        // table cannot crowd the others out of the first view.
        let share = edge_limit.div_ceil(mapped_edges.len().max(1)).max(1);

        for edge in mapped_edges {
            if state.nodes.len() >= node_limit || state.edges.len() >= edge_limit {
                state.truncated = true;
                break;
            }

            let table = match self.open_table_cached(&edge.table, &mut table_cache).await {
                Ok(table) => table,
                Err(error) => {
                    state
                        .warnings
                        .push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };
            let (columns, prop_names) = match self.edge_projection(edge, &mut schema_cache).await {
                Ok(projection) => projection,
                Err(error) => {
                    state
                        .warnings
                        .push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };

            let budget = share
                .min(edge_limit.saturating_sub(state.edges.len()))
                // Each row can introduce two objects, so the node budget bounds
                // the scan as well.
                .min(node_limit.saturating_sub(state.nodes.len()))
                .max(1);
            let sampled = match self
                .sample_edge_rows(
                    &table,
                    edge,
                    &columns,
                    &prop_names,
                    budget,
                    &mut state.warnings,
                )
                .await
            {
                Ok(sampled) => sampled,
                Err(error) => {
                    state
                        .warnings
                        .push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };
            self.absorb_edge_rows(
                &mut state,
                edge,
                &prop_names,
                sampled.rows,
                edge_limit,
                Some(node_limit),
            );
            for (src_key, count) in sampled.fan_out {
                let full_id = format!("{}:{}", edge.src_label, src_key);
                if state.nodes.contains_key(&full_id) {
                    state.record_fan_out(&full_id, &edge.label, count, sampled.exact);
                }
            }
            if sampled.more_rows {
                state.truncated = true;
            }
        }

        let covered_labels = state
            .nodes
            .values()
            .map(|node| node.label.clone())
            .collect::<HashSet<_>>();

        let mut result = self
            .hydrate_expansion(state, &mut table_cache, &mut schema_cache)
            .await?;
        let mut truncated = result.truncated;

        // Labels no edge mapping reached would otherwise be invisible. Each one
        // gets a share of what is left, so the first uncovered label cannot
        // spend the whole remainder and hide the rest.
        let uncovered = self
            .overlay
            .nodes
            .iter()
            .filter(|node| !covered_labels.contains(&node.label))
            .collect::<Vec<_>>();
        for (index, node) in uncovered.iter().enumerate() {
            let remaining = node_limit.saturating_sub(result.nodes.len());
            if remaining == 0 {
                truncated = true;
                break;
            }
            let quota = (remaining / (uncovered.len() - index)).max(1);
            // One row past the quota: a label that returned exactly its quota is
            // otherwise indistinguishable from one the quota cut off, and calling
            // a complete view truncated is its own kind of wrong.
            match self.load_nodes_for_label(&node.label, quota + 1).await {
                Ok(mut nodes) => {
                    truncated = truncated || nodes.len() > quota;
                    nodes.truncate(quota);
                    result.nodes.extend(nodes);
                }
                Err(error) => result
                    .warnings
                    .push(format!("Node mapping '{}': {}", node.label, error)),
            }
        }

        // dedupe_and_limit_subgraph truncates in vector order, so the objects
        // that carry population counts must be dropped last.
        result.nodes.sort_by_key(|node| node.stats.is_none());

        let mut limited =
            dedupe_and_limit_subgraph(result.nodes, result.edges, node_limit, result.warnings);
        limited.truncated = limited.truncated || truncated;
        Ok(limited)
    }

    /// Picks the rows one relationship table contributes to the seedless view.
    ///
    /// Reading the head of the table hands the whole budget to whichever object
    /// happens to have been written first, so a corpus of a thousand documents
    /// arrives as one document and its chunks. A census over the two endpoint
    /// columns — cheap enough to run before the projected read — says which
    /// sources the table is about, and the budget is then spread across the
    /// busiest of them.
    async fn sample_edge_rows(
        &self,
        table: &lancedb::Table,
        edge: &super::EdgeMappingDef,
        columns: &[String],
        prop_names: &[String],
        budget: usize,
        warnings: &mut Vec<String>,
    ) -> Result<SampledEdgeRows> {
        let census_window = CENSUS_MAX_ROWS.min(
            budget
                .saturating_mul(CENSUS_BUDGET_FACTOR)
                .max(CENSUS_MIN_ROWS),
        );
        let mut census_columns = vec![edge.src_column.clone()];
        if edge.dst_column != edge.src_column {
            census_columns.push(edge.dst_column.clone());
        }
        let census = self
            .edge_rows(table, &census_columns, None, census_window)
            .await?;
        let census_len = census.len();
        let exact = census_len < census_window;

        if census_len <= budget {
            let rows = if prop_names.is_empty() {
                census
            } else {
                self.edge_rows(table, columns, None, budget).await?
            };
            return Ok(SampledEdgeRows {
                rows,
                fan_out: Vec::new(),
                exact,
                more_rows: !exact,
            });
        }

        let mut order: Vec<String> = Vec::new();
        let mut pairs_by_src: HashMap<String, Vec<(Value, Value)>> = HashMap::new();
        for row in &census {
            let Value::Object(map) = row else { continue };
            let (Some(src_raw), Some(dst_raw)) =
                (map.get(&edge.src_column), map.get(&edge.dst_column))
            else {
                continue;
            };
            let src_key = value_to_id_string(Some(src_raw));
            if src_key.is_empty() || value_to_id_string(Some(dst_raw)).is_empty() {
                continue;
            }
            pairs_by_src
                .entry(src_key.clone())
                .or_insert_with(|| {
                    order.push(src_key.clone());
                    Vec::new()
                })
                .push((src_raw.clone(), dst_raw.clone()));
        }
        if order.is_empty() {
            return Ok(SampledEdgeRows {
                rows: Vec::new(),
                fan_out: Vec::new(),
                exact,
                more_rows: true,
            });
        }

        // Busiest first, ties broken by where the source appeared in the table,
        // so two runs of the same data always pick the same objects.
        let mut ranked = order.iter().enumerate().collect::<Vec<_>>();
        ranked.sort_by(|(left_seen, left), (right_seen, right)| {
            pairs_by_src[*right]
                .len()
                .cmp(&pairs_by_src[*left].len())
                .then(left_seen.cmp(right_seen))
        });

        let per_hub = (budget / HUB_TARGET)
            .saturating_sub(1)
            .clamp(1, MAX_CHILDREN_PER_HUB);
        let hubs = (budget / per_hub.saturating_add(1)).clamp(1, ranked.len());

        // Slot-major: one child for every source before any source gets a
        // second. absorb_edge_rows stops at the first row that would overrun the
        // node budget, so breadth has to come before depth.
        let mut chosen: Vec<(Value, Value)> = Vec::new();
        let mut taken: HashMap<&str, usize> = HashMap::new();

        for slot in 0..per_hub {
            if chosen.len() >= budget {
                break;
            }
            for (_, src_key) in &ranked[..hubs] {
                if chosen.len() >= budget {
                    break;
                }
                if let Some(pair) = pairs_by_src[*src_key].get(slot) {
                    chosen.push(pair.clone());
                    taken.insert(src_key.as_str(), slot + 1);
                }
            }
        }
        // Whatever budget the hubs could not spend goes to the long tail, so the
        // view does not imply that every object is a large one.
        for (_, src_key) in &ranked[hubs..] {
            if chosen.len() >= budget {
                break;
            }
            if let Some(pair) = pairs_by_src[*src_key].first() {
                chosen.push(pair.clone());
                taken.insert(src_key.as_str(), 1);
            }
        }
        // A corpus of two enormous documents cannot fill the view breadth-first,
        // and a near-empty first paint is worse than a deep one. Round-robin the
        // remaining budget so depth is only ever bought after every source has
        // had its turn.
        while chosen.len() < budget {
            let before = chosen.len();
            for (_, src_key) in &ranked {
                if chosen.len() >= budget {
                    break;
                }
                let next = taken.entry(src_key.as_str()).or_insert(0);
                if let Some(pair) = pairs_by_src[*src_key].get(*next) {
                    chosen.push(pair.clone());
                    *next += 1;
                }
            }
            if chosen.len() == before {
                break;
            }
        }

        let mut seen_src = HashSet::new();
        let mut fan_out = Vec::new();
        for (src_raw, _) in &chosen {
            let src_key = value_to_id_string(Some(src_raw));
            if !seen_src.insert(src_key.clone()) {
                continue;
            }
            fan_out.push((src_key.clone(), pairs_by_src[&src_key].len()));
        }

        // The census carries no properties, so the chosen pairs are read back with
        // the full projection. The window is scoped to each chunk of chosen pairs
        // rather than shared across the scan: one global limit would be filled from
        // the head of the busiest source, which is the bias this whole path exists
        // to remove.
        let mut projected: HashMap<(String, String), serde_json::Map<String, Value>> =
            HashMap::new();
        if !prop_names.is_empty() {
            for pairs in chosen.chunks(IN_CHUNK_SIZE) {
                let mut seen_chunk_src = HashSet::new();
                let chunk_src = pairs
                    .iter()
                    .filter(|(src_raw, _)| seen_chunk_src.insert(value_to_id_string(Some(src_raw))))
                    .map(|(src_raw, _)| src_raw.clone())
                    .collect::<Vec<_>>();
                let window = pairs
                    .len()
                    .saturating_mul(SAMPLE_PROPS_OVERSCAN)
                    .min(SAMPLE_PROPS_MAX_ROWS);
                match self
                    .edge_rows_for_ids(table, &edge.src_column, columns, &chunk_src, window)
                    .await
                {
                    Ok(rows) => {
                        for row in rows {
                            let Value::Object(map) = row else { continue };
                            let key = (
                                value_to_id_string(map.get(&edge.src_column)),
                                value_to_id_string(map.get(&edge.dst_column)),
                            );
                            projected.insert(key, map);
                        }
                    }
                    Err(error) => {
                        warnings.push(format!(
                            "Edge mapping '{}': properties were not loaded: {}",
                            edge.label, error
                        ));
                        break;
                    }
                }
            }
        }

        let mut rows = Vec::with_capacity(chosen.len());
        for (src_raw, dst_raw) in chosen {
            let key = (
                value_to_id_string(Some(&src_raw)),
                value_to_id_string(Some(&dst_raw)),
            );
            let map = projected.remove(&key).unwrap_or_else(|| {
                let mut map = serde_json::Map::new();
                map.insert(edge.src_column.clone(), src_raw);
                map.insert(edge.dst_column.clone(), dst_raw);
                map
            });
            rows.push(Value::Object(map));
        }

        Ok(SampledEdgeRows {
            rows,
            fan_out,
            exact,
            more_rows: true,
        })
    }

    async fn hydrate_expansion(
        &self,
        state: ExpansionState,
        table_cache: &mut HashMap<String, lancedb::Table>,
        schema_cache: &mut HashMap<String, Vec<String>>,
    ) -> Result<SubgraphResult> {
        let ExpansionState {
            nodes: discovered,
            edges,
            warnings: mut collected_warnings,
            truncated,
            mut node_stats,
            ..
        } = state;

        let mut by_label: HashMap<String, Vec<(String, Value)>> = HashMap::new();
        for (full_id, node) in discovered {
            by_label
                .entry(node.label)
                .or_default()
                .push((full_id, node.raw_id));
        }

        let mut nodes = Vec::new();
        let mut hydrated_ids = HashSet::new();
        let mut dangling = 0usize;
        for (label, entries) in by_label {
            let Some(mapping) = self.overlay.nodes.iter().find(|node| node.label == label) else {
                continue;
            };
            let id_col = self.find_id_column_for_label(&label)?;
            let display_col = self.find_display_column_for_label(&label);
            let table = match self.open_table_cached(&mapping.table, table_cache).await {
                Ok(table) => table,
                Err(error) => {
                    collected_warnings.push(format!("Node mapping '{}': {}", label, error));
                    continue;
                }
            };
            let excluded = HashSet::from([id_col.clone()]);
            let always_include = display_col
                .clone()
                .filter(|column| *column != id_col)
                .into_iter()
                .collect::<Vec<_>>();
            let prop_names = resolve_property_names(
                &self.connection,
                &mapping.table,
                &mapping.property_columns,
                self.overlay.property_projection_mode,
                schema_cache,
                &excluded,
                &always_include,
            )
            .await?;
            let mut columns = vec![id_col.clone()];
            columns.extend(prop_names.iter().cloned());
            let mut seen_columns = HashSet::new();
            columns.retain(|column| seen_columns.insert(column.clone()));

            let mut raw_to_full: HashMap<String, String> = HashMap::new();
            for (full_id, raw_id) in &entries {
                raw_to_full.insert(value_to_id_string(Some(raw_id)), full_id.clone());
            }
            for chunk in entries.chunks(IN_CHUNK_SIZE) {
                let ids = chunk.iter().map(|(_, raw)| raw.clone()).collect::<Vec<_>>();
                let literals = ids
                    .iter()
                    .map(value_sql_literal)
                    .collect::<Result<Vec<_>>>()?;
                let predicate = format!(
                    "{} IN ({})",
                    filter_identifier(&id_col),
                    literals.join(", ")
                );
                let batches = table
                    .query()
                    .only_if(predicate)
                    .select(lancedb::query::Select::Columns(columns.clone()))
                    .limit(chunk.len())
                    .execute()
                    .await
                    .map_err(|e| anyhow!("Failed to load nodes for '{}': {}", label, e))?
                    .try_collect::<Vec<_>>()
                    .await
                    .map_err(|e| anyhow!("Failed to collect nodes for '{}': {}", label, e))?;
                for batch in &batches {
                    for row in record_batch_to_value(batch)? {
                        let Value::Object(map) = row else { continue };
                        let raw_key = value_to_id_string(map.get(&id_col));
                        let Some(full_id) = raw_to_full.get(&raw_key) else {
                            continue;
                        };
                        if !hydrated_ids.insert(full_id.clone()) {
                            continue;
                        }
                        let caption = display_col
                            .as_ref()
                            .and_then(|column| map.get(column))
                            .and_then(|value| value.as_str())
                            .map(String::from)
                            .or_else(|| Some(raw_key.clone()));
                        nodes.push(SubgraphNode {
                            id: full_id.clone(),
                            label: label.clone(),
                            caption,
                            props: Value::Object(map),
                            stats: node_stats.remove(full_id),
                        });
                    }
                }
            }
            dangling += entries
                .iter()
                .filter(|(full_id, _)| !hydrated_ids.contains(full_id))
                .count();
        }

        let edges = edges
            .into_iter()
            .filter(|edge| {
                hydrated_ids.contains(&edge.source) && hydrated_ids.contains(&edge.target)
            })
            .collect::<Vec<_>>();
        if dangling > 0 {
            collected_warnings.push(format!(
                "{dangling} referenced object(s) were not found in their node tables; connected edges were dropped"
            ));
        }

        Ok(SubgraphResult {
            nodes,
            edges,
            truncated,
            warnings: collected_warnings,
        })
    }

    /// Single containment hop from a parent object to its children.
    ///
    /// Follows only edges flagged `containment` whose `src_label` matches the
    /// parent type. Same-overlay children ride the shared [`Self::hydrate_expansion`]
    /// path; children mapped by a sibling local overlay (`dst_ontology`) are
    /// loaded from that overlay's tables. Remote children (`dst_binding_id`) are
    /// not resolved here — a warning points to the remote children node.
    pub(super) async fn overlay_children_impl(
        &self,
        parent_label: &str,
        parent_id: Value,
        node_limit: usize,
    ) -> Result<SubgraphResult> {
        if !self
            .overlay
            .nodes
            .iter()
            .any(|node| node.label == parent_label)
        {
            return Err(anyhow!(
                "Label '{}' not found in overlay node mappings",
                parent_label
            ));
        }

        let node_limit = node_limit.max(1);
        let edge_limit = node_limit.saturating_mul(3);
        let parent_full = format!("{}:{}", parent_label, value_to_id_string(Some(&parent_id)));

        let mut table_cache: HashMap<String, lancedb::Table> = HashMap::new();
        let mut schema_cache: HashMap<String, Vec<String>> = HashMap::new();

        let mut state = ExpansionState::new();
        // Seed the parent so it hydrates and its child edges survive the
        // dangling-edge prune in hydrate_expansion.
        state.nodes.insert(
            parent_full.clone(),
            Discovered {
                label: parent_label.to_string(),
                raw_id: parent_id.clone(),
            },
        );

        // Children mapped by a sibling overlay, grouped by (overlay, child label,
        // optional id-column override). Hydrated separately from self.overlay.
        let mut external: ExternalChildren = HashMap::new();
        let mut emitted = 0usize;

        for edge in &self.overlay.edges {
            if state.truncated {
                break;
            }
            if !edge.containment || edge.src_label != parent_label {
                continue;
            }
            if edge.dst_binding_id.is_some() {
                state.warnings.push(format!(
                    "Containment edge '{}' targets a remote ontology; expand it with the remote children node",
                    edge.label
                ));
                continue;
            }
            let cross_overlay = edge
                .dst_ontology
                .as_deref()
                .filter(|id| *id != self.overlay.id.as_str());
            // Same-overlay children need a resolvable node mapping.
            if cross_overlay.is_none()
                && !self
                    .overlay
                    .nodes
                    .iter()
                    .any(|node| node.label == edge.dst_label)
            {
                continue;
            }

            let table = match self.open_table_cached(&edge.table, &mut table_cache).await {
                Ok(table) => table,
                Err(error) => {
                    state
                        .warnings
                        .push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };
            let excluded = HashSet::from([edge.src_column.clone(), edge.dst_column.clone()]);
            let prop_names = match resolve_property_names(
                &self.connection,
                &edge.table,
                &edge.property_columns,
                self.overlay.property_projection_mode,
                &mut schema_cache,
                &excluded,
                &[],
            )
            .await
            {
                Ok(names) => names,
                Err(error) => {
                    state
                        .warnings
                        .push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };
            let mut columns = vec![edge.src_column.clone(), edge.dst_column.clone()];
            columns.extend(prop_names.iter().cloned());
            let mut seen_columns = HashSet::new();
            columns.retain(|column| seen_columns.insert(column.clone()));

            let remaining = edge_limit.saturating_sub(emitted);
            if remaining == 0 {
                state.truncated = true;
                break;
            }
            let rows = match self
                .edge_rows_for_ids(
                    &table,
                    &edge.src_column,
                    &columns,
                    std::slice::from_ref(&parent_id),
                    remaining,
                )
                .await
            {
                Ok(rows) => rows,
                Err(error) => {
                    state
                        .warnings
                        .push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };

            for row in rows {
                let Value::Object(map) = row else { continue };
                let dst_raw = map.get(&edge.dst_column).cloned();
                let dst_key = value_to_id_string(dst_raw.as_ref());
                if dst_key.is_empty() {
                    continue;
                }
                let Some(dst_raw) = dst_raw else { continue };
                let dst_full = format!("{}:{}", edge.dst_label, dst_key);
                let edge_id = format!("{}-{}->{}", parent_full, edge.label, dst_full);
                if !state.seen_edge_ids.insert(edge_id.clone()) {
                    continue;
                }
                if emitted >= edge_limit {
                    state.truncated = true;
                    break;
                }
                let mut props = serde_json::Map::new();
                for name in &prop_names {
                    if let Some(value) = map.get(name) {
                        props.insert(name.clone(), value.clone());
                    }
                }
                let subgraph_edge = SubgraphEdge {
                    id: edge_id,
                    source: parent_full.clone(),
                    target: dst_full.clone(),
                    label: edge.label.clone(),
                    props: Value::Object(props),
                };
                match cross_overlay {
                    None => {
                        if !state.nodes.contains_key(&dst_full) && state.nodes.len() >= node_limit {
                            state.truncated = true;
                            continue;
                        }
                        state.nodes.entry(dst_full).or_insert_with(|| Discovered {
                            label: edge.dst_label.clone(),
                            raw_id: dst_raw,
                        });
                        state.edges.push(subgraph_edge);
                        emitted += 1;
                    }
                    Some(overlay_id) => {
                        external
                            .entry((
                                overlay_id.to_string(),
                                edge.dst_label.clone(),
                                edge.dst_node_column.clone(),
                            ))
                            .or_default()
                            .push((dst_full, dst_raw, subgraph_edge));
                        emitted += 1;
                    }
                }
            }
        }

        let mut result = self
            .hydrate_expansion(state, &mut table_cache, &mut schema_cache)
            .await?;

        if !external.is_empty() {
            if result.nodes.iter().any(|node| node.id == parent_full) {
                let (nodes, edges, mut warnings) = self
                    .hydrate_external_children(external, &mut table_cache, &mut schema_cache)
                    .await;
                result.nodes.extend(nodes);
                result.edges.extend(edges);
                result.warnings.append(&mut warnings);
            } else {
                result.warnings.push(
                    "Parent object was not found; linked children were not loaded".to_string(),
                );
            }
        }

        Ok(result)
    }

    /// Loads containment children that a sibling local overlay maps, keyed by
    /// `(overlay id, child label, id-column override)`. Mirrors the per-label
    /// load in [`Self::hydrate_expansion`] but resolves the mapping through the
    /// sibling overlay rather than `self.overlay`.
    async fn hydrate_external_children(
        &self,
        external: ExternalChildren,
        table_cache: &mut HashMap<String, lancedb::Table>,
        schema_cache: &mut HashMap<String, Vec<String>>,
    ) -> (Vec<SubgraphNode>, Vec<SubgraphEdge>, Vec<String>) {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut warnings = Vec::new();
        let mut overlay_cache: HashMap<String, Option<GraphOverlayDef>> = HashMap::new();

        for ((overlay_id, label, id_override), entries) in external {
            let overlay = match overlay_cache.get(&overlay_id) {
                Some(cached) => cached.clone(),
                None => {
                    let loaded = load_overlay(&self.connection, &overlay_id).await.ok();
                    overlay_cache.insert(overlay_id.clone(), loaded.clone());
                    loaded
                }
            };
            let Some(overlay) = overlay else {
                warnings.push(format!(
                    "Linked ontology '{}' could not be loaded",
                    overlay_id
                ));
                continue;
            };
            let Some(mapping) = resolve_object_mapping(&overlay, &label) else {
                warnings.push(format!(
                    "Object type '{}' is not part of linked ontology '{}'",
                    label, overlay_id
                ));
                continue;
            };
            let id_col = id_override.unwrap_or_else(|| mapping.id_column.clone());
            let display_col = mapping.display_column.clone();
            let table = match self.open_table_cached(&mapping.table, table_cache).await {
                Ok(table) => table,
                Err(error) => {
                    warnings.push(format!("Node mapping '{}': {}", label, error));
                    continue;
                }
            };
            let excluded = HashSet::from([id_col.clone()]);
            let always_include = display_col
                .clone()
                .filter(|column| *column != id_col)
                .into_iter()
                .collect::<Vec<_>>();
            let prop_names = match resolve_property_names(
                &self.connection,
                &mapping.table,
                &mapping.property_columns,
                self.overlay.property_projection_mode,
                schema_cache,
                &excluded,
                &always_include,
            )
            .await
            {
                Ok(names) => names,
                Err(error) => {
                    warnings.push(format!("Node mapping '{}': {}", label, error));
                    continue;
                }
            };
            let mut columns = vec![id_col.clone()];
            columns.extend(prop_names.iter().cloned());
            let mut seen_columns = HashSet::new();
            columns.retain(|column| seen_columns.insert(column.clone()));

            let mut edges_by_raw: HashMap<String, Vec<SubgraphEdge>> = HashMap::new();
            let mut full_by_raw: HashMap<String, String> = HashMap::new();
            let mut raw_by_key: HashMap<String, Value> = HashMap::new();
            for (full_id, raw_id, edge) in entries {
                let raw_key = value_to_id_string(Some(&raw_id));
                edges_by_raw.entry(raw_key.clone()).or_default().push(edge);
                full_by_raw.insert(raw_key.clone(), full_id);
                raw_by_key.insert(raw_key, raw_id);
            }
            let raw_ids = raw_by_key.values().cloned().collect::<Vec<_>>();
            let literals = match raw_ids
                .iter()
                .map(value_sql_literal)
                .collect::<Result<Vec<_>>>()
            {
                Ok(literals) => literals,
                Err(error) => {
                    warnings.push(format!("Node mapping '{}': {}", label, error));
                    continue;
                }
            };
            if literals.is_empty() {
                continue;
            }
            let predicate = format!(
                "{} IN ({})",
                filter_identifier(&id_col),
                literals.join(", ")
            );
            let batches = match table
                .query()
                .only_if(predicate)
                .select(lancedb::query::Select::Columns(columns.clone()))
                .limit(raw_ids.len())
                .execute()
                .await
            {
                Ok(stream) => match stream.try_collect::<Vec<_>>().await {
                    Ok(batches) => batches,
                    Err(error) => {
                        warnings.push(format!("Failed to load '{}': {}", label, error));
                        continue;
                    }
                },
                Err(error) => {
                    warnings.push(format!("Failed to load '{}': {}", label, error));
                    continue;
                }
            };

            let mut hydrated = HashSet::new();
            for batch in &batches {
                let rows = match record_batch_to_value(batch) {
                    Ok(rows) => rows,
                    Err(error) => {
                        warnings.push(format!("Failed to decode '{}': {}", label, error));
                        continue;
                    }
                };
                for row in rows {
                    let Value::Object(map) = row else { continue };
                    let raw_key = value_to_id_string(map.get(&id_col));
                    let Some(full_id) = full_by_raw.get(&raw_key) else {
                        continue;
                    };
                    if !hydrated.insert(full_id.clone()) {
                        continue;
                    }
                    let caption = display_col
                        .as_ref()
                        .and_then(|column| map.get(column))
                        .and_then(|value| value.as_str())
                        .map(String::from)
                        .or_else(|| Some(raw_key.clone()));
                    nodes.push(SubgraphNode {
                        id: full_id.clone(),
                        label: label.clone(),
                        caption,
                        props: Value::Object(map),
                        stats: None,
                    });
                    if let Some(child_edges) = edges_by_raw.get(&raw_key) {
                        edges.extend(child_edges.iter().cloned());
                    }
                }
            }
            let missing = full_by_raw.len().saturating_sub(hydrated.len());
            if missing > 0 {
                warnings.push(format!(
                    "{missing} linked '{}' object(s) were not found; connected edges were dropped",
                    label
                ));
            }
        }

        (nodes, edges, warnings)
    }

    pub(super) async fn shortest_paths_impl(
        &self,
        from: (String, Value),
        to: (String, Value),
        max_depth: usize,
        node_limit: usize,
    ) -> Result<GraphPathsResult> {
        let max_depth = max_depth.clamp(1, self.safety.max_depth);
        let from_full = format!("{}:{}", from.0, value_to_id_string(Some(&from.1)));
        let target_full = format!("{}:{}", to.0, value_to_id_string(Some(&to.1)));
        let expansion = self
            .expand_subgraph(
                vec![from, to],
                max_depth,
                TraversalDirection::Both,
                node_limit,
                None,
            )
            .await?;

        let mut graph = petgraph::Graph::<String, usize, petgraph::Undirected>::new_undirected();
        let mut index_of: HashMap<&str, petgraph::graph::NodeIndex> = HashMap::new();
        for node in &expansion.nodes {
            let index = graph.add_node(node.id.clone());
            index_of.insert(node.id.as_str(), index);
        }
        for (position, edge) in expansion.edges.iter().enumerate() {
            if let (Some(&source), Some(&target)) = (
                index_of.get(edge.source.as_str()),
                index_of.get(edge.target.as_str()),
            ) {
                graph.add_edge(source, target, position);
            }
        }

        let mut paths = Vec::new();
        let mut used_edge_positions: HashSet<usize> = HashSet::new();
        let from_idx = index_of.get(from_full.as_str()).copied();
        let to_idx = index_of.get(target_full.as_str()).copied();

        if let (Some(from_idx), Some(to_idx)) = (from_idx, to_idx) {
            for _ in 0..MAX_ALTERNATIVE_PATHS {
                let path = petgraph::algo::astar(
                    &graph,
                    from_idx,
                    |node| node == to_idx,
                    |edge| {
                        if used_edge_positions.contains(edge.weight()) {
                            1_000_000usize
                        } else {
                            1
                        }
                    },
                    |_| 0,
                );
                let Some((cost, node_indices)) = path else {
                    break;
                };
                if cost >= 1_000_000 {
                    break;
                }
                let path_length = node_indices.len().saturating_sub(1);
                // Bidirectional expansion can expose a route up to twice the
                // requested depth. The API contract bounds the final route,
                // not each half of the discovery process.
                if path_length > max_depth {
                    break;
                }
                let node_ids = node_indices
                    .iter()
                    .map(|index| graph[*index].clone())
                    .collect::<Vec<_>>();
                let mut edge_ids = Vec::new();
                for pair in node_indices.windows(2) {
                    if let Some(edge_ref) = graph.find_edge(pair[0], pair[1]) {
                        let position = graph[edge_ref];
                        used_edge_positions.insert(position);
                        edge_ids.push(expansion.edges[position].id.clone());
                    }
                }
                paths.push(GraphPath {
                    length: path_length,
                    node_ids,
                    edge_ids,
                });
            }
        }

        let path_node_ids = paths
            .iter()
            .flat_map(|path| path.node_ids.iter().cloned())
            .collect::<HashSet<_>>();
        let path_edge_ids = paths
            .iter()
            .flat_map(|path| path.edge_ids.iter().cloned())
            .collect::<HashSet<_>>();
        let nodes = expansion
            .nodes
            .iter()
            .filter(|node| path_node_ids.contains(&node.id))
            .cloned()
            .collect::<Vec<_>>();
        let edges = expansion
            .edges
            .iter()
            .filter(|edge| path_edge_ids.contains(&edge.id))
            .cloned()
            .collect::<Vec<_>>();

        Ok(GraphPathsResult {
            found: !paths.is_empty(),
            paths,
            nodes,
            edges,
            truncated: expansion.truncated,
            warnings: expansion.warnings,
        })
    }

    pub(super) async fn analytics_impl(&self, edge_limit: usize) -> Result<GraphAnalyticsResult> {
        let mut table_cache: HashMap<String, lancedb::Table> = HashMap::new();
        let mut warnings = Vec::new();
        let mut truncated = false;

        let mut label_counts = Vec::new();
        for node in &self.overlay.nodes {
            match self.open_table_cached(&node.table, &mut table_cache).await {
                Ok(table) => match table.count_rows(None).await {
                    Ok(count) => label_counts.push(LabelCount {
                        label: node.label.clone(),
                        nodes: count,
                    }),
                    Err(error) => warnings.push(format!(
                        "Could not count rows for '{}': {}",
                        node.label, error
                    )),
                },
                Err(error) => warnings.push(format!("Node mapping '{}': {}", node.label, error)),
            }
        }

        let edge_budget = edge_limit.clamp(1, ANALYTICS_MAX_EDGES);
        let edge_mapping_count = self.overlay.edges.len();
        let base_mapping_cap = edge_budget / edge_mapping_count.max(1);
        let mappings_with_extra_row = edge_budget % edge_mapping_count.max(1);

        let mut graph = petgraph::Graph::<(String, String), (), petgraph::Directed>::new();
        let mut index_of: HashMap<String, petgraph::graph::NodeIndex> = HashMap::new();
        let mut edge_count = 0usize;

        for (mapping_index, edge) in self.overlay.edges.iter().enumerate() {
            // Quotient/remainder allocation keeps the sum of accepted rows at
            // exactly `edge_budget`. Mappings assigned zero rows still query
            // one sentinel so `truncated` reflects omitted data.
            let per_mapping_cap = base_mapping_cap
                .saturating_add(usize::from(mapping_index < mappings_with_extra_row));
            let table = match self.open_table_cached(&edge.table, &mut table_cache).await {
                Ok(table) => table,
                Err(error) => {
                    warnings.push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };
            let columns = vec![edge.src_column.clone(), edge.dst_column.clone()];
            let batches = match table
                .query()
                .select(lancedb::query::Select::Columns(columns))
                .limit(per_mapping_cap.saturating_add(1))
                .execute()
                .await
            {
                Ok(stream) => match stream.try_collect::<Vec<_>>().await {
                    Ok(batches) => batches,
                    Err(error) => {
                        warnings.push(format!("Edge mapping '{}': {}", edge.label, error));
                        continue;
                    }
                },
                Err(error) => {
                    warnings.push(format!("Edge mapping '{}': {}", edge.label, error));
                    continue;
                }
            };
            let mut mapping_rows = 0usize;
            'mapping: for batch in &batches {
                for row in record_batch_to_value(batch)? {
                    mapping_rows += 1;
                    if mapping_rows > per_mapping_cap {
                        truncated = true;
                        break 'mapping;
                    }
                    let Value::Object(map) = row else { continue };
                    let src_key = value_to_id_string(map.get(&edge.src_column));
                    let dst_key = value_to_id_string(map.get(&edge.dst_column));
                    if src_key.is_empty() || dst_key.is_empty() {
                        continue;
                    }
                    let src_full = format!("{}:{}", edge.src_label, src_key);
                    let dst_full = format!("{}:{}", edge.dst_label, dst_key);
                    let src_index = *index_of.entry(src_full.clone()).or_insert_with(|| {
                        graph.add_node((edge.src_label.clone(), src_key.clone()))
                    });
                    let dst_index = *index_of.entry(dst_full.clone()).or_insert_with(|| {
                        graph.add_node((edge.dst_label.clone(), dst_key.clone()))
                    });
                    graph.add_edge(src_index, dst_index, ());
                    edge_count += 1;
                }
            }
        }

        let sampled_node_count = graph.node_count();
        let mut union_find: UnionFind<usize> = UnionFind::new(sampled_node_count);
        for edge_ref in graph.edge_indices() {
            if let Some((source, target)) = graph.edge_endpoints(edge_ref) {
                union_find.union(source.index(), target.index());
            }
        }
        let mut component_of = vec![0usize; sampled_node_count];
        let mut component_sizes: HashMap<usize, usize> = HashMap::new();
        for index in graph.node_indices() {
            let root = union_find.find(index.index());
            component_of[index.index()] = root;
            *component_sizes.entry(root).or_default() += 1;
        }
        let total_declared_nodes: usize = label_counts.iter().map(|entry| entry.nodes).sum();
        // Edge rows are bounded, while node counts are exact table counts. Nodes
        // absent from the sampled edge snapshot are singleton components in that
        // snapshot. When `truncated` is true, component/isolation metrics are
        // explicitly sample-relative rather than claims about the full edge set.
        let snapshot_isolated_node_count = total_declared_nodes.saturating_sub(sampled_node_count);
        let component_count = component_sizes
            .len()
            .saturating_add(snapshot_isolated_node_count);
        let mut largest_components = component_sizes.values().copied().collect::<Vec<_>>();
        largest_components.extend(std::iter::repeat_n(1, snapshot_isolated_node_count.min(10)));
        largest_components.sort_unstable_by(|left, right| right.cmp(left));
        largest_components.truncate(10);

        let pagerank = if sampled_node_count > 0 {
            petgraph::algo::page_rank(&graph, 0.85_f64, 20)
        } else {
            Vec::new()
        };

        let mut metrics = graph
            .node_indices()
            .map(|index| {
                let (label, raw_id) = graph[index].clone();
                NodeMetric {
                    id: format!("{}:{}", label, raw_id),
                    label,
                    caption: None,
                    degree_in: graph
                        .neighbors_directed(index, petgraph::Direction::Incoming)
                        .count(),
                    degree_out: graph
                        .neighbors_directed(index, petgraph::Direction::Outgoing)
                        .count(),
                    pagerank: pagerank.get(index.index()).copied().unwrap_or_default(),
                    component: component_of[index.index()],
                }
            })
            .collect::<Vec<_>>();

        metrics.sort_unstable_by(|left, right| {
            (right.degree_in + right.degree_out).cmp(&(left.degree_in + left.degree_out))
        });
        let mut top_by_degree = metrics
            .iter()
            .take(TOP_METRIC_NODES)
            .cloned()
            .collect::<Vec<_>>();
        metrics.sort_unstable_by(|left, right| {
            right
                .pagerank
                .partial_cmp(&left.pagerank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut top_by_pagerank = metrics
            .iter()
            .take(TOP_METRIC_NODES)
            .cloned()
            .collect::<Vec<_>>();
        drop(metrics);

        self.attach_captions(&mut top_by_degree, &mut table_cache, &mut warnings)
            .await;
        self.attach_captions(&mut top_by_pagerank, &mut table_cache, &mut warnings)
            .await;

        if truncated {
            warnings.push(format!(
                "Analytics sampled at most {edge_budget} edge rows; edge count, components, isolation, degree, and PageRank describe that bounded snapshot"
            ));
        }

        Ok(GraphAnalyticsResult {
            node_count: total_declared_nodes,
            edge_count,
            truncated,
            label_counts,
            component_count,
            largest_components,
            isolated_node_count: snapshot_isolated_node_count,
            top_by_degree,
            top_by_pagerank,
            warnings,
        })
    }

    async fn attach_captions(
        &self,
        metrics: &mut [NodeMetric],
        table_cache: &mut HashMap<String, lancedb::Table>,
        warnings: &mut Vec<String>,
    ) {
        let mut by_label: HashMap<String, Vec<usize>> = HashMap::new();
        for (position, metric) in metrics.iter().enumerate() {
            by_label
                .entry(metric.label.clone())
                .or_default()
                .push(position);
        }
        for (label, positions) in by_label {
            let Some(mapping) = self.overlay.nodes.iter().find(|node| node.label == label) else {
                continue;
            };
            let Some(display_col) = mapping.display_column.clone() else {
                continue;
            };
            let Ok(id_col) = self.find_id_column_for_label(&label) else {
                continue;
            };
            let table = match self.open_table_cached(&mapping.table, table_cache).await {
                Ok(table) => table,
                Err(error) => {
                    warnings.push(format!("Node mapping '{}': {}", label, error));
                    continue;
                }
            };
            let raw_ids = positions
                .iter()
                .filter_map(|position| {
                    metrics[*position]
                        .id
                        .split_once(':')
                        .map(|(_, raw)| Value::String(raw.to_string()))
                })
                .collect::<Vec<_>>();
            let Ok(literals) = raw_ids
                .iter()
                .map(value_sql_literal)
                .collect::<Result<Vec<_>>>()
            else {
                continue;
            };
            if literals.is_empty() {
                continue;
            }
            let predicate = format!(
                "{} IN ({})",
                filter_identifier(&id_col),
                literals.join(", ")
            );
            let result = table
                .query()
                .only_if(predicate)
                .select(lancedb::query::Select::Columns(vec![
                    id_col.clone(),
                    display_col.clone(),
                ]))
                .limit(raw_ids.len())
                .execute()
                .await;
            let batches = match result {
                Ok(stream) => match stream.try_collect::<Vec<_>>().await {
                    Ok(batches) => batches,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };
            let mut captions: HashMap<String, String> = HashMap::new();
            for batch in &batches {
                let Ok(rows) = record_batch_to_value(batch) else {
                    continue;
                };
                for row in rows {
                    let Value::Object(map) = row else { continue };
                    let raw_key = value_to_id_string(map.get(&id_col));
                    if let Some(caption) = map.get(&display_col).and_then(|value| value.as_str()) {
                        captions.insert(raw_key, caption.to_string());
                    }
                }
            }
            for position in positions {
                let raw = metrics[position]
                    .id
                    .split_once(':')
                    .map(|(_, raw)| raw.to_string());
                if let Some(raw) = raw {
                    metrics[position].caption = captions.get(&raw).cloned();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databases::graph::GraphStore;
    use crate::databases::graph::lancegraph::{CypherSafetyConfig, EdgeMappingDef, NodeMappingDef};
    use arrow::array::{RecordBatch, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use lancedb::connect;
    use std::sync::Arc;

    async fn graph_fixture(
        node_count: usize,
        edge_pairs: &[(&str, &str)],
    ) -> Result<(LanceGraphStore, String)> {
        let test_path = format!("./tmp/{}", flow_like_types::create_id());
        std::fs::create_dir_all(&test_path).unwrap();
        let connection = connect(&test_path).execute().await?;

        let node_schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
        let nodes = RecordBatch::try_new(
            node_schema,
            vec![Arc::new(StringArray::from_iter_values(
                (1..=node_count).map(|id| id.to_string()),
            ))],
        )?;
        connection
            .create_table("people", vec![nodes])
            .execute()
            .await?;

        let edges = if edge_pairs.is_empty() {
            Vec::new()
        } else {
            let edge_schema = Arc::new(Schema::new(vec![
                Field::new("source", DataType::Utf8, false),
                Field::new("target", DataType::Utf8, false),
            ]));
            let rows = RecordBatch::try_new(
                edge_schema,
                vec![
                    Arc::new(StringArray::from_iter_values(
                        edge_pairs.iter().map(|(source, _)| *source),
                    )),
                    Arc::new(StringArray::from_iter_values(
                        edge_pairs.iter().map(|(_, target)| *target),
                    )),
                ],
            )?;
            connection
                .create_table("links", vec![rows])
                .execute()
                .await?;
            vec![EdgeMappingDef {
                id: Some("knows".to_string()),
                api_name: Some("knows".to_string()),
                label: "KNOWS".to_string(),
                table: "links".to_string(),
                src_column: "source".to_string(),
                dst_column: "target".to_string(),
                src_label: "Person".to_string(),
                dst_label: "Person".to_string(),
                src_node_column: None,
                dst_node_column: None,
                containment: false,
                dst_ontology: None,
                dst_binding_id: None,
                property_columns: Vec::new(),
                style: Value::Null,
            }]
        };
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
            edges,
            object_views: Vec::new(),
            actions: Vec::new(),
            exposed: false,
            bindings_enabled: false,
            property_projection_mode: PropertyProjectionMode::Dynamic,
            default_limit: 100,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let store =
            LanceGraphStore::new(connection, overlay, Some(CypherSafetyConfig::default())).await?;
        Ok((store, test_path))
    }

    #[tokio::test]
    async fn shortest_path_never_exceeds_requested_depth() -> Result<()> {
        let (store, test_path) =
            graph_fixture(5, &[("1", "2"), ("2", "3"), ("3", "4"), ("4", "5")]).await?;

        let bounded = store
            .shortest_paths(
                ("Person".to_string(), Value::String("1".to_string())),
                ("Person".to_string(), Value::String("5".to_string())),
                2,
                Some(100),
            )
            .await?;
        assert!(!bounded.found);
        assert!(bounded.paths.is_empty());

        let permitted = store
            .shortest_paths(
                ("Person".to_string(), Value::String("1".to_string())),
                ("Person".to_string(), Value::String("5".to_string())),
                4,
                Some(100),
            )
            .await?;
        assert!(permitted.found);
        assert_eq!(permitted.paths[0].length, 4);

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn analytics_counts_all_nodes_when_there_are_no_edges() -> Result<()> {
        let (store, test_path) = graph_fixture(5, &[]).await?;
        let analytics = store.analytics(Some(100)).await?;

        assert_eq!(analytics.node_count, 5);
        assert_eq!(analytics.edge_count, 0);
        assert_eq!(analytics.component_count, 5);
        assert_eq!(analytics.isolated_node_count, 5);
        assert_eq!(analytics.largest_components, vec![1, 1, 1, 1, 1]);
        assert!(!analytics.truncated);

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn analytics_includes_unsampled_nodes_as_snapshot_singletons() -> Result<()> {
        let (store, test_path) = graph_fixture(4, &[("1", "2"), ("2", "3")]).await?;
        let analytics = store.analytics(Some(1)).await?;

        assert_eq!(analytics.node_count, 4);
        assert_eq!(analytics.edge_count, 1);
        assert_eq!(analytics.component_count, 3);
        assert_eq!(analytics.isolated_node_count, 2);
        assert_eq!(analytics.largest_components, vec![2, 1, 1]);
        assert!(analytics.truncated);
        assert!(
            analytics
                .warnings
                .iter()
                .any(|warning| warning.contains("bounded snapshot"))
        );

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn analytics_edge_budget_is_global_across_mappings() -> Result<()> {
        let (mut store, test_path) = graph_fixture(3, &[("1", "2")]).await?;
        let second_edge_schema = Arc::new(Schema::new(vec![
            Field::new("source", DataType::Utf8, false),
            Field::new("target", DataType::Utf8, false),
        ]));
        let second_edge_rows = RecordBatch::try_new(
            second_edge_schema,
            vec![
                Arc::new(StringArray::from(vec!["2"])),
                Arc::new(StringArray::from(vec!["3"])),
            ],
        )?;
        store
            .connection()
            .create_table("second_links", vec![second_edge_rows])
            .execute()
            .await?;
        let mut second_mapping = store.overlay.edges[0].clone();
        second_mapping.id = Some("likes".to_string());
        second_mapping.api_name = Some("likes".to_string());
        second_mapping.label = "LIKES".to_string();
        second_mapping.table = "second_links".to_string();
        store.overlay.edges.push(second_mapping);

        let analytics = store.analytics(Some(1)).await?;
        assert_eq!(
            analytics.edge_count, 1,
            "the budget is global, not per mapping"
        );
        assert!(analytics.truncated);

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn seedless_subgraph_returns_the_edges_between_its_nodes() -> Result<()> {
        let (store, test_path) =
            graph_fixture(5, &[("1", "2"), ("2", "3"), ("3", "4"), ("4", "5")]).await?;

        let result = store.subgraph(Vec::new(), 1, Some(100)).await?;

        assert_eq!(result.nodes.len(), 5);
        assert_eq!(
            result.edges.len(),
            4,
            "the first view must arrive connected, not as unlinked nodes"
        );
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn seedless_subgraph_still_lists_labels_no_edge_reaches() -> Result<()> {
        let (store, test_path) = graph_fixture(4, &[]).await?;

        let result = store.subgraph(Vec::new(), 1, Some(100)).await?;

        assert_eq!(result.nodes.len(), 4);
        assert!(result.edges.is_empty());

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn seedless_subgraph_keeps_edges_within_the_node_budget() -> Result<()> {
        let (store, test_path) =
            graph_fixture(6, &[("1", "2"), ("2", "3"), ("3", "4"), ("5", "6")]).await?;

        let result = store.subgraph(Vec::new(), 1, Some(3)).await?;

        assert!(result.nodes.len() <= 3);
        assert!(result.truncated);
        let node_ids = result
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect::<HashSet<_>>();
        for edge in &result.edges {
            assert!(
                node_ids.contains(edge.source.as_str()) && node_ids.contains(edge.target.as_str()),
                "truncation must not leave dangling edges"
            );
        }

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn seedless_subgraph_spreads_across_parents() -> Result<()> {
        // One parent with fifty children written first, then nine parents with
        // one child each — reading the head of this table returns the first
        // parent and nothing else.
        let ids = (1..=69).map(|id| id.to_string()).collect::<Vec<_>>();
        let mut pairs = (1..=50)
            .map(|child| (ids[0].as_str(), ids[child].as_str()))
            .collect::<Vec<_>>();
        for (offset, parent) in (51..60).enumerate() {
            pairs.push((ids[parent].as_str(), ids[60 + offset].as_str()));
        }
        let (store, test_path) = graph_fixture(69, &pairs).await?;

        let result = store.subgraph(Vec::new(), 1, Some(12)).await?;

        let sources = result
            .edges
            .iter()
            .map(|edge| edge.source.as_str())
            .collect::<HashSet<_>>();
        assert!(
            sources.len() >= 5,
            "the first view must spread across parents, got {sources:?}"
        );

        let busiest = result
            .nodes
            .iter()
            .find(|node| node.id == "Person:1")
            .expect("the busiest parent must survive truncation");
        let stats = busiest
            .stats
            .as_ref()
            .expect("a sampled parent carries its fan-out");
        assert!(stats.exact, "the census window covers the whole fixture");
        assert_eq!(stats.out_by_label.len(), 1);
        assert_eq!(stats.out_by_label[0].label, "KNOWS");
        assert_eq!(stats.out_by_label[0].count, 50);

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn seedless_subgraph_fills_the_budget_from_a_single_parent() -> Result<()> {
        // Spreading across parents is impossible when there is only one, and a
        // near-empty first paint is worse than a deep one.
        let ids = (1..=61).map(|id| id.to_string()).collect::<Vec<_>>();
        let pairs = (1..=60)
            .map(|child| (ids[0].as_str(), ids[child].as_str()))
            .collect::<Vec<_>>();
        let (store, test_path) = graph_fixture(61, &pairs).await?;

        let result = store.subgraph(Vec::new(), 1, Some(30)).await?;

        assert_eq!(
            result.nodes.len(),
            30,
            "one busy parent must still fill the view, got {} nodes",
            result.nodes.len()
        );
        assert!(result.truncated);

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn seedless_subgraph_reports_a_complete_view_as_untruncated() -> Result<()> {
        // Two labels with no mapped edges, both fully inside the limit: the fair
        // share must not make a complete result look like a sample.
        let (store, test_path) = graph_fixture(4, &[]).await?;

        let result = store.subgraph(Vec::new(), 1, Some(50)).await?;

        assert_eq!(result.nodes.len(), 4);
        assert!(
            !result.truncated,
            "the whole population fits, so nothing was cut off"
        );

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn expansion_traverses_only_the_requested_relationships() -> Result<()> {
        let (store, test_path) = graph_fixture(3, &[("1", "2"), ("1", "3")]).await?;

        let named = store
            .neighbors(
                "Person",
                Value::String("1".to_string()),
                1,
                TraversalDirection::Both,
                Some(100),
                Some(&["KNOWS".to_string()]),
            )
            .await?;
        assert_eq!(named.nodes.len(), 3, "the named relationship still expands");

        let other = store
            .neighbors(
                "Person",
                Value::String("1".to_string()),
                1,
                TraversalDirection::Both,
                Some(100),
                Some(&["OWNS".to_string()]),
            )
            .await?;
        assert_eq!(
            other.nodes.len(),
            1,
            "a relationship the overlay does not map reaches nobody"
        );
        assert!(other.edges.is_empty());

        // An unset filter must not read as "allow nothing" — a caller that sends
        // an empty list would otherwise get a blank graph with no error to explain it.
        let empty = store
            .neighbors(
                "Person",
                Value::String("1".to_string()),
                1,
                TraversalDirection::Both,
                Some(100),
                Some(&[]),
            )
            .await?;
        assert_eq!(empty.nodes.len(), 3);

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }

    #[tokio::test]
    async fn expansion_links_neighbors_discovered_on_the_same_hop() -> Result<()> {
        // 2 and 3 are both neighbors of 1, and are linked to each other.
        let (store, test_path) = graph_fixture(3, &[("1", "2"), ("1", "3"), ("2", "3")]).await?;

        let result = store
            .neighbors(
                "Person",
                Value::String("1".to_string()),
                1,
                TraversalDirection::Both,
                Some(100),
                None,
            )
            .await?;

        assert_eq!(result.nodes.len(), 3);
        assert!(
            result
                .edges
                .iter()
                .any(|edge| edge.source == "Person:2" && edge.target == "Person:3"),
            "an expansion must show how its neighbors connect to each other"
        );

        std::fs::remove_dir_all(&test_path).ok();
        Ok(())
    }
}
