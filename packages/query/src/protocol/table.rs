//! Table browsing reads: rows with filters and sorting, counts, schemas, indices, comparisons
//! and table listings. Every variant answers one JSON value, shaped like the HTTP response of
//! the route it replaces. Lane L6 of `todo/1140-query-service` owns this file.

use flow_like_storage_contracts::database::DatabaseSelector;
use flow_like_types::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TableOperation {
    /// Rows by SQL, vector, full-text, hybrid or filter search; the answer is an array of rows.
    Query {
        table: String,
        #[serde(default)]
        selector: DatabaseSelector,
        #[serde(default)]
        sql: Option<String>,
        #[serde(default)]
        sql_params: Value,
        #[serde(default)]
        vector: Option<Vec<f64>>,
        #[serde(default)]
        filter: Option<String>,
        #[serde(default)]
        fts_term: Option<String>,
        #[serde(default)]
        rerank: Option<bool>,
        #[serde(default)]
        select: Option<Vec<String>>,
        limit: usize,
        offset: usize,
    },
    /// One page of rows; the answer is an array of rows.
    List {
        table: String,
        #[serde(default)]
        selector: DatabaseSelector,
        limit: usize,
        offset: usize,
    },
    /// Row count; the answer is a number.
    Count {
        table: String,
        #[serde(default)]
        selector: DatabaseSelector,
    },
    /// Arrow schema as JSON.
    Schema {
        table: String,
        #[serde(default)]
        selector: DatabaseSelector,
    },
    /// `[{ name, index_type, columns }]`.
    Indices {
        table: String,
        #[serde(default)]
        selector: DatabaseSelector,
    },
    /// `{ schema, count, items }` over one connection.
    View {
        table: String,
        #[serde(default)]
        selector: DatabaseSelector,
        limit: usize,
        offset: usize,
    },
    /// Versions, branches and tags of a table.
    History {
        table: String,
        #[serde(default)]
        selector: DatabaseSelector,
    },
    /// Differences between two references of a table by an application key.
    Compare {
        table: String,
        #[serde(default)]
        selector: DatabaseSelector,
        other: DatabaseSelector,
        key: String,
        limit: usize,
    },
    /// Table names, or summaries when `summary` is set.
    ListTables {
        #[serde(default)]
        summary: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip(operation: &TableOperation) -> Value {
        let wire = serde_json::to_value(operation).unwrap();
        let back: TableOperation = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), wire);
        wire
    }

    #[test]
    fn every_variant_round_trips() {
        let selector = DatabaseSelector {
            branch: "dev".into(),
            version: Some(3),
            tag: None,
            read_only: true,
        };
        let operations = [
            TableOperation::Query {
                table: "orders".into(),
                selector: selector.clone(),
                sql: Some("SELECT * FROM orders WHERE id = $id".into()),
                sql_params: json!({ "id": 1 }),
                vector: Some(vec![0.5, 1.5]),
                filter: Some("id > 1".into()),
                fts_term: Some("shoes".into()),
                rerank: Some(false),
                select: Some(vec!["id".into()]),
                limit: 25,
                offset: 50,
            },
            TableOperation::List {
                table: "orders".into(),
                selector: selector.clone(),
                limit: 25,
                offset: 0,
            },
            TableOperation::Count {
                table: "orders".into(),
                selector: selector.clone(),
            },
            TableOperation::Schema {
                table: "orders".into(),
                selector: selector.clone(),
            },
            TableOperation::Indices {
                table: "orders".into(),
                selector: selector.clone(),
            },
            TableOperation::View {
                table: "orders".into(),
                selector: selector.clone(),
                limit: 10,
                offset: 5,
            },
            TableOperation::History {
                table: "orders".into(),
                selector: selector.clone(),
            },
            TableOperation::Compare {
                table: "orders".into(),
                selector: selector.clone(),
                other: DatabaseSelector::default(),
                key: "id".into(),
                limit: 100,
            },
            TableOperation::ListTables { summary: true },
        ];
        let kinds: Vec<String> = operations
            .iter()
            .map(|operation| round_trip(operation)["kind"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            kinds,
            [
                "query",
                "list",
                "count",
                "schema",
                "indices",
                "view",
                "history",
                "compare",
                "list_tables"
            ]
        );
    }

    #[test]
    fn optional_fields_default() {
        let operation: TableOperation = serde_json::from_value(
            json!({ "kind": "query", "table": "t", "filter": "a = 1", "limit": 5, "offset": 0 }),
        )
        .unwrap();
        let TableOperation::Query {
            selector,
            sql,
            sql_params,
            vector,
            ..
        } = operation
        else {
            panic!("expected a query");
        };
        assert_eq!(selector, DatabaseSelector::default());
        assert!(sql.is_none() && vector.is_none());
        assert!(sql_params.is_null());
        let listing: TableOperation =
            serde_json::from_value(json!({ "kind": "list_tables" })).unwrap();
        assert!(matches!(
            listing,
            TableOperation::ListTables { summary: false }
        ));
    }
}
