//! Graph reads on one ontology overlay. The API authorized the caller and loaded the overlay;
//! the service reads it through the source's column reader, within the request's row and time
//! limits. Lane L5 of `todo/1140-query-service` owns this file.

use super::ServiceConfig;
use super::ops::{FrameSink, Opened, failed, send_json, workbench_source};
use crate::protocol::graph::{GraphOperation, ObjectRef};
use crate::protocol::{ErrorCode, Trailer, WireError};
use flow_like_storage::databases::graph::lancegraph::{
    GraphOverlayDef, LanceGraphStore, restrict_overlay, sample_overlay,
};
use flow_like_storage::databases::graph::{
    GraphAnalyticsResult, GraphPathsResult, GraphQueryResult, GraphSchemaResult, GraphStore,
    SubgraphNode, SubgraphResult,
};
use flow_like_storage::databases::workbench::ExecutionBudget;
use flow_like_storage::lancedb::Connection;
use flow_like_storage_contracts::column_metadata::ColumnReader;
use flow_like_types::Value;
use serde::Serialize;
use std::time::Instant;

/// A graph read's result, serialized bare: exactly the JSON its HTTP route returns.
#[derive(Serialize)]
#[serde(untagged)]
enum GraphAnswer {
    Rows(GraphQueryResult),
    Subgraph(SubgraphResult),
    Paths(GraphPathsResult),
    Analytics(GraphAnalyticsResult),
    Nodes(Vec<SubgraphNode>),
    Sample(Vec<Value>),
    Schema(GraphSchemaResult),
}

impl GraphAnswer {
    fn row_count(&self) -> usize {
        match self {
            Self::Rows(result) => result.rows.len(),
            Self::Subgraph(result) => result.nodes.len(),
            Self::Paths(result) => result.paths.len(),
            Self::Nodes(nodes) => nodes.len(),
            Self::Sample(rows) => rows.len(),
            Self::Analytics(_) | Self::Schema(_) => 1,
        }
    }

    fn truncated(&self) -> bool {
        match self {
            Self::Subgraph(result) => result.truncated,
            Self::Paths(result) => result.truncated,
            Self::Analytics(result) => result.truncated,
            Self::Rows(_) | Self::Nodes(_) | Self::Sample(_) | Self::Schema(_) => false,
        }
    }

    /// Results with their own `warnings` also carry the service's, such as a lowered row
    /// limit, so the caller sees them where it already reads warnings.
    fn note(&mut self, warnings: &[String]) {
        let own = match self {
            Self::Subgraph(result) => &mut result.warnings,
            Self::Paths(result) => &mut result.warnings,
            Self::Analytics(result) => &mut result.warnings,
            Self::Rows(_) | Self::Nodes(_) | Self::Sample(_) | Self::Schema(_) => return,
        };
        own.extend_from_slice(warnings);
    }
}

pub(super) async fn execute(
    _config: &ServiceConfig,
    opened: &Opened,
    operation: GraphOperation,
    budget: &ExecutionBudget,
    sink: &FrameSink,
    warnings: Vec<String>,
    started: Instant,
) -> Result<(), WireError> {
    let (connection, reader) = workbench_source(opened)?;
    let mut answer =
        tokio::time::timeout(budget.timeout, read(connection, reader, operation, budget))
            .await
            .map_err(|_| {
                WireError::new(
                    ErrorCode::Timeout,
                    format!(
                        "The graph read timed out after {} ms",
                        budget.timeout.as_millis()
                    ),
                )
            })?
            .map_err(failed)?;
    answer.note(&warnings);
    let trailer = Trailer {
        row_count: answer.row_count(),
        truncated: answer.truncated(),
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
        ..Trailer::default()
    };
    send_json(sink, &answer, trailer).await
}

/// A store whose Cypher and SQL run under the request's memory budget and spill directory.
async fn store(
    connection: &Connection,
    reader: &ColumnReader,
    overlay: Box<GraphOverlayDef>,
    budget: &ExecutionBudget,
) -> flow_like_types::Result<LanceGraphStore> {
    Ok(
        LanceGraphStore::new_for(connection.clone(), *overlay, None, reader.clone())
            .await?
            .with_budget(budget.clone()),
    )
}

/// Runs one read with the same storage calls the API made in-process, every result-size limit
/// lowered to the request's `rows`.
async fn read(
    connection: &Connection,
    reader: &ColumnReader,
    operation: GraphOperation,
    budget: &ExecutionBudget,
) -> flow_like_types::Result<GraphAnswer> {
    let rows = budget.rows;
    let cap = |limit: usize| Some(limit.min(rows));
    Ok(match operation {
        GraphOperation::Cypher {
            overlay,
            query,
            params,
            limit,
        } => GraphAnswer::Rows(
            store(connection, reader, overlay, budget)
                .await?
                .cypher_with_metadata(&query, params, cap(limit))
                .await?,
        ),
        GraphOperation::Sql {
            overlay,
            query,
            params,
            limit,
        } => GraphAnswer::Rows(
            store(connection, reader, overlay, budget)
                .await?
                .sql_with_metadata(&query, params, cap(limit))
                .await?,
        ),
        GraphOperation::Neighbors {
            overlay,
            object,
            depth,
            direction,
            limit,
            edge_labels,
        } => GraphAnswer::Subgraph(
            store(connection, reader, overlay, budget)
                .await?
                .neighbors(
                    &object.label,
                    object.id,
                    depth,
                    direction,
                    cap(limit),
                    edge_labels.as_deref(),
                )
                .await?,
        ),
        GraphOperation::Children {
            overlay,
            object,
            limit,
        } => GraphAnswer::Subgraph(
            store(connection, reader, overlay, budget)
                .await?
                .overlay_children(&object.label, object.id, cap(limit))
                .await?,
        ),
        GraphOperation::Subgraph {
            overlay,
            seeds,
            depth,
            limit,
        } => GraphAnswer::Subgraph(
            store(connection, reader, overlay, budget)
                .await?
                .subgraph(
                    seeds.into_iter().map(ObjectRef::into_pair).collect(),
                    depth,
                    cap(limit),
                )
                .await?,
        ),
        GraphOperation::Paths {
            overlay,
            from,
            to,
            max_depth,
            limit,
        } => GraphAnswer::Paths(
            store(connection, reader, overlay, budget)
                .await?
                .shortest_paths(from.into_pair(), to.into_pair(), max_depth, cap(limit))
                .await?,
        ),
        GraphOperation::Analytics {
            overlay,
            edge_limit,
        } => GraphAnswer::Analytics(
            store(connection, reader, overlay, budget)
                .await?
                .analytics(edge_limit)
                .await?,
        ),
        GraphOperation::Search {
            overlay,
            query,
            limit,
        } => GraphAnswer::Nodes(
            store(connection, reader, overlay, budget)
                .await?
                .search_nodes(&query, cap(limit))
                .await?,
        ),
        GraphOperation::Sample { overlay, label, n } => {
            let overlay = restrict_overlay(connection, *overlay, reader).await?;
            GraphAnswer::Sample(sample_overlay(connection, &overlay, &label, n.min(rows)).await?)
        }
        GraphOperation::Schema { overlay } => GraphAnswer::Schema(
            store(connection, reader, overlay, budget)
                .await?
                .schema()
                .await?,
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::{QueryResponse, ResponseAssembler};
    use crate::protocol::graph::tests::overlay;
    use crate::server::ServiceLimits;
    use crate::server::external::DriverConnector;
    use flow_like_storage::arrow_array::{RecordBatch, StringArray};
    use flow_like_storage::arrow_schema::{DataType, Field, Schema};
    use flow_like_storage::databases::graph::TraversalDirection;
    use flow_like_storage_contracts::column_metadata::READ_ATTRIBUTES_KEY;
    use serde_json::json;
    use std::collections::{BTreeSet, HashMap};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::mpsc;

    const NAMES: [&str; 6] = ["Ada", "Grace", "Linus", "Margaret", "Alan", "Barbara"];

    /// `people(id, name, email)`, where only readers holding `support` see `email`, linked by
    /// `KNOWS` as a chain 1 → 2 → … → `count`.
    struct Chain {
        path: PathBuf,
        connection: Connection,
    }

    impl Chain {
        async fn of(count: usize) -> Self {
            let path = std::env::temp_dir().join(format!(
                "flow-like-query-graph-{}",
                flow_like_types::create_id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            let connection = flow_like_storage::lancedb::connect(&path.to_string_lossy())
                .execute()
                .await
                .unwrap();
            let ids: Vec<String> = (1..=count).map(|id| id.to_string()).collect();
            let email = Field::new("email", DataType::Utf8, true).with_metadata(HashMap::from([(
                READ_ATTRIBUTES_KEY.to_string(),
                "support".to_string(),
            )]));
            let people = RecordBatch::try_new(
                Arc::new(Schema::new(vec![
                    Field::new("id", DataType::Utf8, false),
                    Field::new("name", DataType::Utf8, true),
                    email,
                ])),
                vec![
                    Arc::new(StringArray::from(ids.clone())),
                    Arc::new(StringArray::from(NAMES[..count].to_vec())),
                    Arc::new(StringArray::from(
                        ids.iter()
                            .map(|id| format!("{id}@example.com"))
                            .collect::<Vec<_>>(),
                    )),
                ],
            )
            .unwrap();
            let links = RecordBatch::try_new(
                Arc::new(Schema::new(vec![
                    Field::new("source", DataType::Utf8, false),
                    Field::new("target", DataType::Utf8, false),
                ])),
                vec![
                    Arc::new(StringArray::from(ids[..count - 1].to_vec())),
                    Arc::new(StringArray::from(ids[1..].to_vec())),
                ],
            )
            .unwrap();
            connection
                .create_table("people", vec![people])
                .execute()
                .await
                .unwrap();
            connection
                .create_table("links", vec![links])
                .execute()
                .await
                .unwrap();
            Self { path, connection }
        }

        async fn run(
            &self,
            reader: ColumnReader,
            operation: GraphOperation,
            rows: usize,
            warnings: &[&str],
        ) -> Result<QueryResponse, WireError> {
            let config = ServiceConfig {
                limits: ServiceLimits {
                    max_rows: 1_000,
                    max_memory_bytes: 64 * 1024 * 1024,
                    max_timeout: Duration::from_secs(30),
                    spill: None,
                },
                connector: Arc::new(DriverConnector::new(false)),
            };
            let opened = Opened {
                bare: Some((self.connection.clone(), reader)),
                ..Opened::default()
            };
            let budget = ExecutionBudget {
                rows,
                memory_bytes: None,
                spill: None,
                timeout: Duration::from_secs(30),
            };
            let (sender, mut receiver) = mpsc::channel(8);
            let sink = FrameSink::new(sender);
            execute(
                &config,
                &opened,
                operation,
                &budget,
                &sink,
                warnings.iter().map(|warning| warning.to_string()).collect(),
                Instant::now(),
            )
            .await?;
            drop(sink);
            let mut assembler = ResponseAssembler::default();
            while let Some(frame) = receiver.recv().await {
                assembler.push(&frame).unwrap();
            }
            Ok(assembler.finish().unwrap())
        }
    }

    impl Drop for Chain {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn answer<T: serde::de::DeserializeOwned>(response: &QueryResponse) -> T {
        assert!(response.batches.is_empty(), "graph answers travel as JSON");
        serde_json::from_value(response.json.clone().expect("a JSON answer")).unwrap()
    }

    fn person(id: &str) -> ObjectRef {
        ObjectRef {
            label: "Person".to_string(),
            id: json!(id),
        }
    }

    fn restricted() -> ColumnReader {
        ColumnReader::with_attributes(Vec::<String>::new())
    }

    #[tokio::test]
    async fn sql_and_cypher_answer_rows_through_the_source_reader() {
        let chain = Chain::of(3).await;
        let response = chain
            .run(
                ColumnReader::Unrestricted,
                GraphOperation::Sql {
                    overlay: overlay(),
                    query: "SELECT id, name FROM people WHERE id <> $skip ORDER BY id".to_string(),
                    params: json!({ "skip": "1" }),
                    limit: 100,
                },
                100,
                &[],
            )
            .await
            .unwrap();
        let result: GraphQueryResult = answer(&response);
        assert_eq!(
            result.rows,
            vec![
                json!({ "id": "2", "name": "Grace" }),
                json!({ "id": "3", "name": "Linus" }),
            ]
        );
        assert_eq!(response.trailer.row_count, 2);

        let response = chain
            .run(
                restricted(),
                GraphOperation::Cypher {
                    overlay: overlay(),
                    query: "MATCH (n:Person)-[r:KNOWS]->(m:Person) RETURN n, r, m".to_string(),
                    params: Value::Null,
                    limit: 100,
                },
                100,
                &[],
            )
            .await
            .unwrap();
        let result: GraphQueryResult = answer(&response);
        assert_eq!(result.rows.len(), 2);
        for row in &result.rows {
            assert!(row.get("n.name").is_some(), "{row}");
            let columns = row.as_object().expect("an object row");
            assert!(columns.keys().all(|key| !key.contains("email")), "{row}");
        }
    }

    #[tokio::test]
    async fn traversals_answer_subgraphs_paths_and_matches() {
        let chain = Chain::of(5).await;
        let neighbors: SubgraphResult = answer(
            &chain
                .run(
                    ColumnReader::Unrestricted,
                    GraphOperation::Neighbors {
                        overlay: overlay(),
                        object: person("1"),
                        depth: 1,
                        direction: TraversalDirection::Outgoing,
                        limit: 100,
                        edge_labels: None,
                    },
                    100,
                    &[],
                )
                .await
                .unwrap(),
        );
        let ids: BTreeSet<&str> = neighbors
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect();
        assert_eq!(ids, BTreeSet::from(["Person:1", "Person:2"]));
        assert_eq!(neighbors.edges.len(), 1);

        let paths: GraphPathsResult = answer(
            &chain
                .run(
                    ColumnReader::Unrestricted,
                    GraphOperation::Paths {
                        overlay: overlay(),
                        from: person("1"),
                        to: person("5"),
                        max_depth: 4,
                        limit: 100,
                    },
                    100,
                    &[],
                )
                .await
                .unwrap(),
        );
        assert!(paths.found);
        assert_eq!(paths.paths[0].length, 4);

        let matches: Vec<SubgraphNode> = answer(
            &chain
                .run(
                    ColumnReader::Unrestricted,
                    GraphOperation::Search {
                        overlay: overlay(),
                        query: "grac".to_string(),
                        limit: 10,
                    },
                    10,
                    &[],
                )
                .await
                .unwrap(),
        );
        assert_eq!(matches.len(), 1, "{matches:?}");
        assert_eq!(matches[0].id, "Person:2");
        assert_eq!(matches[0].caption.as_deref(), Some("Grace"));
    }

    #[tokio::test]
    async fn the_row_budget_caps_graph_limits_and_says_so() {
        let chain = Chain::of(6).await;
        let lowered = "The row limit was lowered from 100 to this service's maximum of 3";
        let response = chain
            .run(
                ColumnReader::Unrestricted,
                GraphOperation::Subgraph {
                    overlay: overlay(),
                    seeds: Vec::new(),
                    depth: 1,
                    limit: 100,
                },
                3,
                &[lowered],
            )
            .await
            .unwrap();
        let result: SubgraphResult = answer(&response);
        assert!(result.nodes.len() <= 3, "{} nodes", result.nodes.len());
        assert!(result.truncated);
        assert!(
            result.warnings.iter().any(|warning| warning == lowered),
            "{:?}",
            result.warnings
        );
        assert_eq!(response.trailer.row_count, result.nodes.len());
        assert!(response.trailer.truncated);
        assert_eq!(response.trailer.warnings, vec![lowered.to_string()]);
    }

    #[tokio::test]
    async fn analytics_sample_their_edges_and_keep_the_service_warnings() {
        let chain = Chain::of(5).await;
        let response = chain
            .run(
                ColumnReader::Unrestricted,
                GraphOperation::Analytics {
                    overlay: overlay(),
                    edge_limit: Some(1),
                },
                1,
                &["service note"],
            )
            .await
            .unwrap();
        let analytics: GraphAnalyticsResult = answer(&response);
        assert_eq!(analytics.node_count, 5);
        assert_eq!(analytics.edge_count, 1);
        assert!(analytics.truncated);
        assert!(
            analytics
                .warnings
                .iter()
                .any(|warning| warning == "service note")
        );
        assert_eq!(response.trailer.row_count, 1);
    }

    #[tokio::test]
    async fn column_restrictions_follow_the_source_reader() {
        let chain = Chain::of(3).await;
        let sample = || GraphOperation::Sample {
            overlay: overlay(),
            label: "Person".to_string(),
            n: 10,
        };

        let hidden: Vec<Value> = answer(&chain.run(restricted(), sample(), 10, &[]).await.unwrap());
        assert_eq!(hidden.len(), 3);
        assert!(
            hidden
                .iter()
                .all(|row| row.get("email").is_none() && row.get("name").is_some()),
            "{hidden:?}"
        );

        let support = ColumnReader::with_attributes(["support"]);
        let visible: Vec<Value> = answer(&chain.run(support, sample(), 10, &[]).await.unwrap());
        assert!(
            visible.iter().all(|row| row.get("email").is_some()),
            "{visible:?}"
        );

        let schema: GraphSchemaResult = answer(
            &chain
                .run(
                    restricted(),
                    GraphOperation::Schema { overlay: overlay() },
                    1,
                    &[],
                )
                .await
                .unwrap(),
        );
        let properties: Vec<&str> = schema.node_labels[0]
            .properties
            .iter()
            .map(|property| property.name.as_str())
            .collect();
        assert!(
            properties.contains(&"name") && !properties.contains(&"email"),
            "{properties:?}"
        );

        let refused = chain
            .run(
                restricted(),
                GraphOperation::Sql {
                    overlay: overlay(),
                    query: "SELECT email FROM people".to_string(),
                    params: Value::Null,
                    limit: 10,
                },
                10,
                &[],
            )
            .await
            .unwrap_err();
        assert_eq!(refused.code, ErrorCode::QueryFailed, "{}", refused.message);
    }

    #[tokio::test]
    async fn a_graph_read_needs_the_bare_app_source() {
        let (sender, _receiver) = mpsc::channel(8);
        let error = execute(
            &ServiceConfig::from_env(),
            &Opened::default(),
            GraphOperation::Schema { overlay: overlay() },
            &ExecutionBudget {
                rows: 1,
                memory_bytes: None,
                spill: None,
                timeout: Duration::from_secs(5),
            },
            &FrameSink::new(sender),
            Vec::new(),
            Instant::now(),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }
}
