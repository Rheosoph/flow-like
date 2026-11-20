//! Graph reads on one ontology overlay: Cypher, graph SQL, neighbours, children, subgraphs,
//! paths, search, sampling, analytics and the overlay's schema. Every variant carries the overlay
//! the API loaded and authorized; the service restricts it to the source's column reader and
//! answers one JSON value, shaped like the HTTP response of the route it replaces. Lane L5 of
//! `todo/1140-query-service` owns this file.

use flow_like_storage::databases::graph::TraversalDirection;
use flow_like_storage::databases::graph::lancegraph::GraphOverlayDef;
use flow_like_types::Value;
use serde::{Deserialize, Serialize};

/// One object of the overlay: its object type label and its identity value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectRef {
    pub label: String,
    pub id: Value,
}

impl ObjectRef {
    pub fn into_pair(self) -> (String, Value) {
        (self.label, self.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GraphOperation {
    /// Rows and projected property metadata of a Cypher query (`GraphQueryResult`).
    Cypher {
        overlay: Box<GraphOverlayDef>,
        query: String,
        #[serde(default)]
        params: Value,
        limit: usize,
    },
    /// Rows and projected property metadata of a read-only SQL statement over the overlay's
    /// tables (`GraphQueryResult`).
    Sql {
        overlay: Box<GraphOverlayDef>,
        query: String,
        #[serde(default)]
        params: Value,
        limit: usize,
    },
    /// Objects within `depth` hops of one object (`SubgraphResult`).
    Neighbors {
        overlay: Box<GraphOverlayDef>,
        object: ObjectRef,
        depth: usize,
        direction: TraversalDirection,
        limit: usize,
        /// Relationship labels to follow; `None` or empty follows all of them.
        #[serde(default)]
        edge_labels: Option<Vec<String>>,
    },
    /// Containment children of one object, one hop (`SubgraphResult`).
    Children {
        overlay: Box<GraphOverlayDef>,
        object: ObjectRef,
        limit: usize,
    },
    /// The neighbourhood of the seeds, or a scan of the overlay without seeds (`SubgraphResult`).
    Subgraph {
        overlay: Box<GraphOverlayDef>,
        #[serde(default)]
        seeds: Vec<ObjectRef>,
        depth: usize,
        limit: usize,
    },
    /// Shortest connections between two objects; `limit` bounds the objects explored
    /// (`GraphPathsResult`).
    Paths {
        overlay: Box<GraphOverlayDef>,
        from: ObjectRef,
        to: ObjectRef,
        max_depth: usize,
        limit: usize,
    },
    /// Object counts, components and the most connected objects over at most `edge_limit`
    /// sampled relationships, the store's default when `None` (`GraphAnalyticsResult`).
    Analytics {
        overlay: Box<GraphOverlayDef>,
        #[serde(default)]
        edge_limit: Option<usize>,
    },
    /// Objects whose caption or identifier contains `query` (`[SubgraphNode]`).
    Search {
        overlay: Box<GraphOverlayDef>,
        query: String,
        limit: usize,
    },
    /// Up to `n` rows of one object type or relationship (`[row]`).
    Sample {
        overlay: Box<GraphOverlayDef>,
        label: String,
        n: usize,
    },
    /// Labels with the properties the reader may see (`GraphSchemaResult`).
    Schema { overlay: Box<GraphOverlayDef> },
}

impl GraphOperation {
    /// The most records the answer may hold, which the request's row limit carries. Analytics
    /// and schema answer one summary each.
    pub fn row_limit(&self) -> usize {
        match self {
            Self::Cypher { limit, .. }
            | Self::Sql { limit, .. }
            | Self::Neighbors { limit, .. }
            | Self::Children { limit, .. }
            | Self::Subgraph { limit, .. }
            | Self::Paths { limit, .. }
            | Self::Search { limit, .. } => *limit,
            Self::Sample { n, .. } => *n,
            Self::Analytics { .. } | Self::Schema { .. } => 1,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::protocol::Operation;
    use serde_json::json;
    use std::collections::BTreeSet;

    /// `Person` objects in `people(id, name, …)`, linked by `KNOWS` rows `links(source, target)`.
    fn overlay_json() -> Value {
        json!({
            "id": "people",
            "name": "People",
            "description": null,
            "nodes": [{
                "label": "Person",
                "table": "people",
                "id_column": "id",
                "display_column": "name",
                "property_columns": [],
                "style": null
            }],
            "edges": [{
                "label": "KNOWS",
                "table": "links",
                "src_column": "source",
                "dst_column": "target",
                "src_label": "Person",
                "dst_label": "Person",
                "property_columns": [],
                "style": null
            }],
            "default_limit": 100,
            "created_at": "",
            "updated_at": ""
        })
    }

    pub(crate) fn overlay() -> Box<GraphOverlayDef> {
        Box::new(serde_json::from_value(overlay_json()).unwrap())
    }

    fn person(id: Value) -> ObjectRef {
        ObjectRef {
            label: "Person".to_string(),
            id,
        }
    }

    fn kind(operation: &GraphOperation) -> &'static str {
        match operation {
            GraphOperation::Cypher { .. } => "cypher",
            GraphOperation::Sql { .. } => "sql",
            GraphOperation::Neighbors { .. } => "neighbors",
            GraphOperation::Children { .. } => "children",
            GraphOperation::Subgraph { .. } => "subgraph",
            GraphOperation::Paths { .. } => "paths",
            GraphOperation::Analytics { .. } => "analytics",
            GraphOperation::Search { .. } => "search",
            GraphOperation::Sample { .. } => "sample",
            GraphOperation::Schema { .. } => "schema",
        }
    }

    /// Every variant with the row limit it reports.
    fn every_operation() -> Vec<(GraphOperation, usize)> {
        vec![
            (
                GraphOperation::Cypher {
                    overlay: overlay(),
                    query: "MATCH (p:Person) WHERE p.name = $name RETURN p".to_string(),
                    params: json!({ "name": "Ada" }),
                    limit: 100,
                },
                100,
            ),
            (
                GraphOperation::Sql {
                    overlay: overlay(),
                    query: "SELECT * FROM people WHERE id = $id".to_string(),
                    params: json!({ "id": 1 }),
                    limit: 10,
                },
                10,
            ),
            (
                GraphOperation::Neighbors {
                    overlay: overlay(),
                    object: person(json!("1")),
                    depth: 2,
                    direction: TraversalDirection::Incoming,
                    limit: 50,
                    edge_labels: Some(vec!["KNOWS".to_string()]),
                },
                50,
            ),
            (
                GraphOperation::Children {
                    overlay: overlay(),
                    object: person(json!(7)),
                    limit: 25,
                },
                25,
            ),
            (
                GraphOperation::Subgraph {
                    overlay: overlay(),
                    seeds: vec![person(json!("1")), person(json!(2))],
                    depth: 1,
                    limit: 0,
                },
                0,
            ),
            (
                GraphOperation::Paths {
                    overlay: overlay(),
                    from: person(json!("1")),
                    to: person(json!("5")),
                    max_depth: 4,
                    limit: 200,
                },
                200,
            ),
            (
                GraphOperation::Analytics {
                    overlay: overlay(),
                    edge_limit: Some(5_000),
                },
                1,
            ),
            (
                GraphOperation::Search {
                    overlay: overlay(),
                    query: "ada".to_string(),
                    limit: 20,
                },
                20,
            ),
            (
                GraphOperation::Sample {
                    overlay: overlay(),
                    label: "KNOWS".to_string(),
                    n: 10,
                },
                10,
            ),
            (GraphOperation::Schema { overlay: overlay() }, 1),
        ]
    }

    #[test]
    fn every_operation_round_trips_inside_a_query_operation() {
        let operations = every_operation();
        let kinds: BTreeSet<_> = operations.iter().map(|(op, _)| kind(op)).collect();
        assert_eq!(kinds.len(), 10, "every variant is covered once");

        for (graph, rows) in operations {
            let name = kind(&graph);
            assert_eq!(graph.row_limit(), rows, "{name}");
            let wire = serde_json::to_value(Operation::Graph { graph }).unwrap();
            assert_eq!(wire["op"], "graph", "{name}");
            assert_eq!(wire["graph"]["kind"], name);
            assert_eq!(wire["graph"]["overlay"]["nodes"][0]["table"], "people");

            let back: Operation = serde_json::from_value(wire.clone()).unwrap();
            let Operation::Graph { graph } = &back else {
                panic!("{name} came back as another operation");
            };
            assert_eq!(kind(graph), name);
            assert_eq!(serde_json::to_value(&back).unwrap(), wire, "{name}");
        }
    }

    #[test]
    fn object_references_and_directions_keep_their_wire_shape() {
        let wire = serde_json::to_value(GraphOperation::Neighbors {
            overlay: overlay(),
            object: person(json!(42)),
            depth: 1,
            direction: TraversalDirection::Outgoing,
            limit: 5,
            edge_labels: None,
        })
        .unwrap();
        assert_eq!(wire["object"], json!({ "label": "Person", "id": 42 }));
        assert_eq!(wire["direction"], "Outgoing");
        assert_eq!(
            person(json!("7")).into_pair(),
            ("Person".to_string(), json!("7"))
        );
    }

    #[test]
    fn optional_fields_default_when_absent() {
        let parse = |mut body: Value| -> GraphOperation {
            body["overlay"] = overlay_json();
            serde_json::from_value(body).unwrap()
        };

        let cypher = parse(json!({ "kind": "cypher", "query": "MATCH (p) RETURN p", "limit": 5 }));
        assert!(matches!(
            cypher,
            GraphOperation::Cypher {
                params: Value::Null,
                limit: 5,
                ..
            }
        ));

        let neighbors = parse(json!({
            "kind": "neighbors",
            "object": { "label": "Person", "id": "1" },
            "depth": 1,
            "direction": "Both",
            "limit": 10
        }));
        assert!(matches!(
            neighbors,
            GraphOperation::Neighbors {
                direction: TraversalDirection::Both,
                edge_labels: None,
                ..
            }
        ));

        let subgraph = parse(json!({ "kind": "subgraph", "depth": 1, "limit": 10 }));
        assert!(matches!(subgraph, GraphOperation::Subgraph { ref seeds, .. } if seeds.is_empty()));

        let analytics = parse(json!({ "kind": "analytics" }));
        assert!(matches!(
            analytics,
            GraphOperation::Analytics {
                edge_limit: None,
                ..
            }
        ));

        let unknown = serde_json::from_value::<GraphOperation>(json!({
            "kind": "drop_table",
            "overlay": overlay_json()
        }));
        assert!(unknown.is_err(), "writes have no graph read variant");
    }
}
