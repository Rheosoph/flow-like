//! Multi-hop traversal, path finding, and structural analytics.
//!
//! lance-graph has no algorithms module, so traversal beyond one hop is done
//! here: breadth-first frontier expansion with batched `IN` scans directly on
//! the mapped Lance tables, and petgraph for path reconstruction and metrics.

use super::{
    LanceGraphStore, quote_identifier, resolve_property_names, value_sql_literal,
    value_to_id_string,
};
use crate::arrow_utils::record_batch_to_value;
use crate::databases::graph::{
    GraphAnalyticsResult, GraphPath, GraphPathsResult, LabelCount, NodeMetric, SubgraphEdge,
    SubgraphNode, SubgraphResult, TraversalDirection,
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
}

impl LanceGraphStore {
    async fn open_table_cached<'a>(
        &self,
        name: &str,
        cache: &'a mut HashMap<String, lancedb::Table>,
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
            quote_identifier(filter_column),
            literals.join(", ")
        );
        let batches = table
            .query()
            .only_if(predicate)
            .select(lancedb::query::Select::Columns(columns.to_vec()))
            .limit(limit)
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

    /// Breadth-first expansion from seed objects across all edge mappings,
    /// honoring depth, direction, and node/edge budgets.
    pub(super) async fn expand_subgraph(
        &self,
        seeds: Vec<(String, Value)>,
        depth: usize,
        direction: TraversalDirection,
        node_limit: usize,
    ) -> Result<SubgraphResult> {
        let depth = depth.clamp(1, self.safety.max_depth);
        let node_limit = node_limit.max(1);
        let edge_limit = node_limit.saturating_mul(3);

        let mut state = ExpansionState {
            nodes: HashMap::new(),
            edges: Vec::new(),
            seen_edge_ids: HashSet::new(),
            warnings: Vec::new(),
            truncated: false,
        };
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
                if sides.is_empty()
                    || !self
                        .overlay
                        .nodes
                        .iter()
                        .any(|node| node.label == edge.src_label)
                    || !self
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

        self.hydrate_expansion(state, &mut table_cache, &mut schema_cache)
            .await
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
                let predicate =
                    format!("{} IN ({})", quote_identifier(&id_col), literals.join(", "));
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

    pub(super) async fn shortest_paths_impl(
        &self,
        from: (String, Value),
        to: (String, Value),
        max_depth: usize,
        node_limit: usize,
    ) -> Result<GraphPathsResult> {
        let from_full = format!("{}:{}", from.0, value_to_id_string(Some(&from.1)));
        let target_full = format!("{}:{}", to.0, value_to_id_string(Some(&to.1)));
        let expansion = self
            .expand_subgraph(
                vec![from, to],
                max_depth.clamp(1, self.safety.max_depth),
                TraversalDirection::Both,
                node_limit,
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
                    length: node_ids.len().saturating_sub(1),
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
        let per_mapping_cap = if self.overlay.edges.is_empty() {
            0
        } else {
            (edge_budget / self.overlay.edges.len()).max(1)
        };

        let mut graph = petgraph::Graph::<(String, String), (), petgraph::Directed>::new();
        let mut index_of: HashMap<String, petgraph::graph::NodeIndex> = HashMap::new();
        let mut edge_count = 0usize;

        for edge in &self.overlay.edges {
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
                .limit(per_mapping_cap)
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
            for batch in &batches {
                for row in record_batch_to_value(batch)? {
                    let Value::Object(map) = row else { continue };
                    let src_key = value_to_id_string(map.get(&edge.src_column));
                    let dst_key = value_to_id_string(map.get(&edge.dst_column));
                    if src_key.is_empty() || dst_key.is_empty() {
                        continue;
                    }
                    mapping_rows += 1;
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
            if mapping_rows >= per_mapping_cap {
                truncated = true;
            }
        }

        let node_count = graph.node_count();
        let mut union_find: UnionFind<usize> = UnionFind::new(node_count);
        for edge_ref in graph.edge_indices() {
            if let Some((source, target)) = graph.edge_endpoints(edge_ref) {
                union_find.union(source.index(), target.index());
            }
        }
        let mut component_of = vec![0usize; node_count];
        let mut component_sizes: HashMap<usize, usize> = HashMap::new();
        for index in graph.node_indices() {
            let root = union_find.find(index.index());
            component_of[index.index()] = root;
            *component_sizes.entry(root).or_default() += 1;
        }
        let component_count = component_sizes.len();
        let mut largest_components = component_sizes.values().copied().collect::<Vec<_>>();
        largest_components.sort_unstable_by(|left, right| right.cmp(left));
        largest_components.truncate(10);

        let total_declared_nodes: usize = label_counts.iter().map(|entry| entry.nodes).sum();
        let isolated_node_count = total_declared_nodes.saturating_sub(node_count);

        let pagerank = if node_count > 0 {
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

        Ok(GraphAnalyticsResult {
            node_count,
            edge_count,
            truncated,
            label_counts,
            component_count,
            largest_components,
            isolated_node_count,
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
            let predicate = format!("{} IN ({})", quote_identifier(&id_col), literals.join(", "));
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
