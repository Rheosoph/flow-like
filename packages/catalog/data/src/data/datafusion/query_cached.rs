use crate::data::cache::{CacheScope, FlowCache, cache_get, cache_set};
use crate::data::query_params as params;
use crate::data::datafusion::query::{QueryRow, batches_to_csv_table};
use crate::data::datafusion::session::DataFusionSession;
use crate::data::excel::CSVTable;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

const SCOPE_APP: &str = "App";
const SCOPE_USER: &str = "User";

/// Default lifetime for cached results. Long enough to absorb bursts of identical
/// queries, short enough that slowly changing data does not go visibly stale.
const DEFAULT_TTL_SECONDS: i64 = 300;

#[crate::register_node]
#[derive(Default)]
pub struct CachedSqlQueryNode {}

impl CachedSqlQueryNode {
    pub fn new() -> Self {
        CachedSqlQueryNode {}
    }
}

/// Key under which a query's result lands in the flow cache: one hash over this node's
/// board identity, the session identity, the SQL text and the bound parameter values.
///
/// The node id is the load-bearing part: session names ("default") and queries repeat
/// across boards, but a node id is minted once when the node is placed — so a result can
/// only ever be replayed at the exact node that cached it, never leak into another board
/// that happens to mount different data under the same names. Hashing keeps arbitrarily
/// long SQL inside the backend's key length limit.
///
/// The parameters belong in the key for the same reason the SQL text does: with them
/// omitted, one placeholder value's result would be served for every other value, which is
/// the whole point of the parameter. They arrive ordered by first appearance in the
/// statement, so the key does not depend on how the values were assembled.
fn result_cache_key(
    node_id: &str,
    session: &DataFusionSession,
    query: &str,
    query_params: &[(String, Value)],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(node_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(session.cache_key.as_bytes());
    hasher.update(b"\n");
    hasher.update(query.as_bytes());
    for (name, value) in query_params {
        hasher.update(b"\n");
        hasher.update(name.as_bytes());
        hasher.update(b"=");
        hasher.update(
            flow_like_types::json::to_string(value)
                .unwrap_or_default()
                .as_bytes(),
        );
    }
    format!("dfq_{}", hasher.finalize().to_hex())
}

/// Row objects rebuilt from the columnar table, so the `rows` output is identical
/// whether the result came from the engine or from the cache.
fn table_to_rows(table: &CSVTable) -> Vec<QueryRow> {
    let headers = table.headers();
    table
        .rows_as_values()
        .into_iter()
        .map(|row| headers.iter().cloned().zip(row).collect::<QueryRow>())
        .collect()
}

#[async_trait]
impl NodeLogic for CachedSqlQueryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_sql_query_cached",
            "Cached SQL Query",
            "Execute a SQL query against a DataFusion session, remembering the result in the app's cache. While a live cached result exists for this node's session, query and parameter values, the query — and any deferred table mounting — is skipped entirely and the cached rows are returned. Cached results do not notice changes to the underlying data; pick a lifetime that matches how fresh the data must be. Write any value that comes from outside the flow as a $placeholder and wire it into the pin that appears — never build the SQL string around it.",
            "Data/DataFusion",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(2);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session with registered tables",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "query",
            "Query",
            "SQL query to execute (e.g., SELECT * FROM mytable WHERE column > 10). Use $placeholders for values that come from the flow (SELECT * FROM users WHERE id = $user_id) — each one adds an input pin to wire the value into, and each distinct value is cached separately. Placeholders stand for values only; table and column names cannot be parameterized.",
            VariableType::String,
        )
        .set_default_value(Some(json!("SELECT * FROM data LIMIT 100")));

        params::add_params_pin(&mut node, params::SqlFlavor::Query);

        node.add_input_pin(
            "scope",
            "Scope",
            "App shares cached results with everyone who can run this app. User keeps them private to whoever triggered the run.",
            VariableType::String,
        )
        .set_default_value(Some(json!(SCOPE_APP)))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![SCOPE_APP.to_string(), SCOPE_USER.to_string()])
                .build(),
        );

        node.add_input_pin(
            "namespace",
            "Namespace",
            "Group name for the cached results. Invalidating this namespace (Invalidate Cache Namespace node) clears them in one call; it also keeps results from unrelated flows apart.",
            VariableType::String,
        )
        .set_default_value(Some(json!("datafusion")));

        node.add_input_pin(
            "ttl_seconds",
            "Lifetime (s)",
            "Seconds until a cached result expires and the query runs again. 0 keeps it until it is deleted.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(DEFAULT_TTL_SECONDS)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Query executed or served from cache",
            VariableType::Execution,
        );

        node.add_output_pin(
            "table",
            "Table",
            "Query results as a CSVTable (columnar format, good for analytics)",
            VariableType::Struct,
        )
        .set_schema::<CSVTable>();

        node.add_output_pin(
            "rows",
            "Rows",
            "Query results as array of row structs with Flow-Like-compatible values. Rows derive from the Table representation so cached and fresh runs are identical: date-like strings are normalized to ISO form and unsigned values beyond the signed 64-bit range become strings.",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "row_count",
            "Row Count",
            "Number of rows in the result",
            VariableType::Integer,
        );

        node.add_output_pin(
            "from_cache",
            "From Cache",
            "True when the result was served from the cache and the query never ran",
            VariableType::Boolean,
        );

        node.scores = Some(NodeScores {
            privacy: 8,
            security: 8,
            performance: 9,
            governance: 8,
            reliability: 8,
            cost: 9,
        });

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "query", board, params::SqlFlavor::Query);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let query: String = context.evaluate_pin("query").await?;
        let query_params = params::resolve_params(context, &query, params::SqlFlavor::Query).await?;
        let scope: String = context.evaluate_pin("scope").await?;
        let namespace: String = context.evaluate_pin("namespace").await?;
        let ttl_seconds: i64 = context.evaluate_pin("ttl_seconds").await?;

        if ttl_seconds < 0 {
            return Err(flow_like_types::anyhow!(
                "Cache lifetime must not be negative; use 0 to keep the result indefinitely"
            ));
        }

        let cache = FlowCache {
            scope: CacheScope::from_label(&scope),
            namespace: namespace.trim().to_string(),
        };
        let node_id = context.node.meta.id.clone();
        let key = result_cache_key(&node_id, &session, &query, &query_params);

        // The session is deliberately not loaded before this lookup: on a hit the
        // engine — and whatever mounting produced it — is bypassed entirely.
        // A failed or malformed read degrades to a miss instead of failing the flow;
        // the query still runs and its result overwrites the entry.
        let cached_table = match cache_get(context, &cache, &key).await {
            Ok(Some(hit)) => match flow_like_types::json::from_value::<CSVTable>(hit.value) {
                Ok(table) => Some(table),
                Err(err) => {
                    context.log_message(
                        &format!("Ignoring malformed cached query result: {err}"),
                        LogLevel::Warn,
                    );
                    None
                }
            },
            Ok(None) => None,
            Err(err) => {
                context.log_message(
                    &format!("Cache lookup failed, running the query instead: {err}"),
                    LogLevel::Warn,
                );
                None
            }
        };

        let (csv_table, from_cache) = match cached_table {
            Some(table) => {
                context.log_message(
                    &format!("Serving SQL result from cache (key {key})"),
                    LogLevel::Debug,
                );
                (table, true)
            }
            None => {
                let cached_session = session.load(context).await?;

                context.log_message(&format!("Executing SQL: {}", query), LogLevel::Debug);
                let df = cached_session.ctx.sql(&query).await?;
                let df = params::bind(df, &query_params)?;
                let batches = df.collect().await?;
                let table = batches_to_csv_table(&batches)?;

                // An oversized result fails loudly here (the cache caps value sizes)
                // rather than silently running uncached every time — the fix is to
                // shrink the result (LIMIT, projection) or persist it to app storage.
                cache_set(
                    context,
                    &cache,
                    &key,
                    json!(table),
                    Some(ttl_seconds as u64),
                )
                .await?;

                (table, false)
            }
        };

        let rows = table_to_rows(&csv_table);
        let row_count = csv_table.row_count() as i64;

        context.set_pin_value("table", json!(csv_table)).await?;
        context.set_pin_value("rows", json!(rows)).await?;
        context.set_pin_value("row_count", json!(row_count)).await?;
        context
            .set_pin_value("from_cache", json!(from_cache))
            .await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::PinType;
    use flow_like_types::Value;

    #[test]
    fn cache_keys_separate_nodes_sessions_and_queries() {
        let session_a = DataFusionSession {
            cache_key: "df_session_a".to_string(),
        };
        let session_b = DataFusionSession {
            cache_key: "df_session_b".to_string(),
        };

        let same_query = "SELECT * FROM data";
        let none: &[(String, Value)] = &[];
        assert_ne!(
            result_cache_key("node1", &session_a, same_query, none),
            result_cache_key("node1", &session_b, same_query, none),
            "the same SQL against differently mounted sessions must not share a result"
        );
        assert_ne!(
            result_cache_key("node1", &session_a, same_query, none),
            result_cache_key("node2", &session_a, same_query, none),
            "two node placements with identical defaults must not share a result"
        );
        assert_ne!(
            result_cache_key("node1", &session_a, "SELECT 1", none),
            result_cache_key("node1", &session_a, "SELECT 2", none)
        );
        assert_eq!(
            result_cache_key("node1", &session_a, same_query, none),
            result_cache_key("node1", &session_a, same_query, none)
        );
    }

    #[test]
    fn cache_keys_separate_parameter_values() {
        let session = DataFusionSession {
            cache_key: "df_session_a".to_string(),
        };
        let query = "SELECT * FROM users WHERE id = $id";
        let key =
            |value: Value| result_cache_key("node1", &session, query, &[("id".to_string(), value)]);

        assert_ne!(
            key(json!(1)),
            key(json!(2)),
            "a cached result for one parameter value must never be served for another"
        );
        assert_eq!(key(json!(1)), key(json!(1)));
        // A string "1" and the number 1 select different rows, so they cannot share a key.
        assert_ne!(key(json!(1)), key(json!("1")));
        assert_ne!(
            key(json!(1)),
            result_cache_key("node1", &session, query, &[]),
            "an unbound run must not collide with a bound one"
        );
        // Same values, different placeholder names: still distinct statements.
        assert_ne!(
            result_cache_key("node1", &session, query, &[("a".to_string(), json!(1))]),
            result_cache_key("node1", &session, query, &[("b".to_string(), json!(1))])
        );
    }

    #[test]
    fn cached_table_roundtrips_into_identical_rows() {
        let table = CSVTable::new(
            vec!["id".to_string(), "name".to_string(), "score".to_string()],
            vec![
                vec![json!(1), json!("alice"), json!(10.5)],
                vec![json!(2), json!("bob"), Value::Null],
            ],
            None,
        );

        let direct_rows = table_to_rows(&table);

        let cached: Value = json!(table);
        let restored: CSVTable = flow_like_types::json::from_value(cached).unwrap();
        let restored_rows = table_to_rows(&restored);

        assert_eq!(restored.row_count(), 2);
        assert_eq!(restored.headers(), table.headers());
        assert_eq!(direct_rows, restored_rows);

        assert_eq!(direct_rows[0].get("id"), Some(&json!(1)));
        assert_eq!(direct_rows[0].get("name"), Some(&json!("alice")));
        assert_eq!(direct_rows[0].get("score"), Some(&json!(10.5)));
        assert_eq!(direct_rows[1].get("score"), Some(&Value::Null));
    }

    #[test]
    fn cached_sql_query_node_structure() {
        let node = CachedSqlQueryNode::new().get_node();

        assert_eq!(node.name, "df_sql_query_cached");
        assert_eq!(node.version, Some(2));

        let inputs: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .collect();
        let outputs: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .collect();

        for name in [
            "exec_in",
            "session",
            "query",
            "scope",
            "namespace",
            "ttl_seconds",
        ] {
            assert!(
                inputs.iter().any(|p| p.name == name),
                "missing input {name}"
            );
        }
        for name in ["exec_out", "table", "rows", "row_count", "from_cache"] {
            assert!(
                outputs.iter().any(|p| p.name == name),
                "missing output {name}"
            );
        }

        let rows_pin = outputs.iter().find(|p| p.name == "rows").unwrap();
        assert_eq!(rows_pin.value_type, ValueType::Array);
    }
}
