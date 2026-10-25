use crate::harness::{Flow, NodeRun, floats, ints, strings, unique};
use flow_like_catalog_data::data::datafusion::session::DataFusionSession;
use flow_like_runtime::flow::node::NodeLogic;
use flow_like_storage::datafusion::arrow::array::{Int64Array, StringArray};
use flow_like_storage::datafusion::arrow::datatypes::{DataType, Field, Schema};
use flow_like_storage::datafusion::arrow::record_batch::RecordBatch;
use flow_like_storage::datafusion::arrow::util::pretty::pretty_format_batches;
use flow_like_types::{Value, json::json};
use flow_like_types::{async_trait, json::from_value};
use std::sync::Arc;

/// How a backend surfaces the fixture's BOOLEAN and TIMESTAMP columns in DataFusion.
/// SQLite has neither storage class, so its provider reports integers and text.
#[derive(Clone, Copy)]
pub struct Surface {
    pub boolean_as_integer: bool,
    pub timestamp_as_text: bool,
}

impl Surface {
    pub const NATIVE: Self = Self {
        boolean_as_integer: false,
        timestamp_as_text: false,
    };

    fn active(self, column: &str) -> String {
        if self.boolean_as_integer {
            format!("{column} = 1")
        } else {
            column.to_string()
        }
    }

    fn timestamp(self, value: &str) -> String {
        if self.timestamp_as_text {
            format!("'{value}'")
        } else {
            format!("TIMESTAMP '{value}'")
        }
    }
}

/// A database a registration node can mount into a DataFusion session.
#[async_trait]
pub trait Backend: Send + Sync {
    fn register_node(&self) -> Arc<dyn NodeLogic>;

    fn surface(&self) -> Surface {
        Surface::NATIVE
    }

    /// Register-node inputs, besides `session`, that mount `source` as `table_name`.
    fn register_inputs(&self, source: &str, table_name: &str, readonly: bool) -> Value;

    /// Inputs aimed at the same server with credentials it rejects.
    fn rejected_inputs(&self, source: &str, table_name: &str) -> Value;

    /// A source name that does not exist on the server.
    fn missing_source(&self) -> String {
        unique("missing")
    }

    /// Whether a read-write registration accepts `INSERT INTO` through DataFusion.
    fn writable(&self) -> bool {
        false
    }

    /// Whether predicates on the mounted table are rendered into the database's own SQL.
    fn pushes_down(&self) -> bool {
        true
    }

    /// Creates a uniquely named source holding `PEOPLE` and returns its name.
    async fn create_source(&self) -> String;

    async fn count_rows(&self, source: &str) -> i64;

    async fn drop_source(&self, source: &str);
}

pub async fn register(
    flow: &Flow,
    backend: &dyn Backend,
    session: &DataFusionSession,
    inputs: Value,
) -> NodeRun {
    let Value::Object(mut inputs) = inputs else {
        panic!("register inputs must be a JSON object");
    };
    inputs.insert("session".into(), json!(session));
    flow.run(backend.register_node(), Value::Object(inputs))
        .await
}

pub async fn mount(
    flow: &Flow,
    backend: &dyn Backend,
    session: &DataFusionSession,
    source: &str,
    table_name: &str,
    readonly: bool,
) -> NodeRun {
    register(
        flow,
        backend,
        session,
        backend.register_inputs(source, table_name, readonly),
    )
    .await
    .expect_fired(&format!("registering '{source}' as '{table_name}'"))
    .await
}

pub async fn count(flow: &Flow, session: &DataFusionSession, table: &str) -> i64 {
    let rows = flow
        .query(session, &format!("SELECT COUNT(*) AS n FROM {table}"))
        .await;
    ints(&rows, "n")[0]
}

pub async fn registers_and_queries(backend: &dyn Backend) {
    let flow = Flow::local();
    let source = backend.create_source().await;
    let session = flow.create_session(&unique("e2e")).await;
    let inputs = backend.register_inputs(&source, "people", true);
    let run = register(&flow, backend, &session, inputs.clone())
        .await
        .expect_fired("registration")
        .await;

    let session_out: DataFusionSession =
        from_value(run.output("session_out").await).expect("session_out is a DataFusionSession");
    assert_eq!(session_out.cache_key, session.cache_key);
    if let Some(password) = inputs["password"].as_str().filter(|p| !p.is_empty()) {
        for pin in ["connection_url", "endpoint"] {
            if let Some(Value::String(text)) = run.output_if_present(pin).await {
                assert!(
                    !text.contains(&format!(":{password}@")),
                    "{pin} leaks the password: {text}"
                );
            }
        }
    }
    assert!(flow.table_names(&session).await.contains(&"people".into()));
    assert_fixture_queries(&flow, &session, "people", backend.surface()).await;
    backend.drop_source(&source).await;
}

/// Filters, aggregates over a timestamp predicate, a bound `$param`, non-ASCII text and
/// top-k against a table holding `PEOPLE`.
pub async fn assert_fixture_queries(
    flow: &Flow,
    session: &DataFusionSession,
    table: &str,
    surface: Surface,
) {
    let rows = flow
        .query(
            session,
            &format!(
                "SELECT id, name FROM {table} WHERE score > 80 AND {} ORDER BY id",
                surface.active("active")
            ),
        )
        .await;
    assert_eq!(ints(&rows, "id"), vec![1, 3]);
    assert_eq!(strings(&rows, "name"), vec!["ada", "linus"]);

    let rows = flow
        .query(
            session,
            &format!(
                "SELECT COUNT(*) AS n, SUM(score) AS total FROM {table} \
                 WHERE created_at >= {}",
                surface.timestamp("2026-01-03 00:00:00")
            ),
        )
        .await;
    assert_eq!(ints(&rows, "n"), vec![3]);
    assert_eq!(floats(&rows, "total"), vec![246.25]);

    let rows = flow
        .query_with(
            session,
            &format!("SELECT name FROM {table} WHERE id = $id"),
            json!({ "id": 5 }),
        )
        .await;
    assert_eq!(strings(&rows, "name"), vec!["zoë"]);

    let rows = flow
        .query(
            session,
            &format!("SELECT id FROM {table} ORDER BY score DESC LIMIT 2"),
        )
        .await;
    assert_eq!(ints(&rows, "id"), vec![5, 1]);
}

/// Pushdown is what makes a database mount cheap: the predicate has to run inside the
/// database instead of over a full table fetch.
pub async fn filters_run_inside_the_database(backend: &dyn Backend) {
    if !backend.pushes_down() {
        return;
    }
    let flow = Flow::local();
    let source = backend.create_source().await;
    let session = flow.create_session(&unique("e2e")).await;
    mount(&flow, backend, &session, &source, "people", true).await;

    let explain = flow
        .session_context(&session)
        .await
        .sql("EXPLAIN SELECT id FROM people WHERE score > 80.5")
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let plan = pretty_format_batches(&explain).unwrap().to_string();
    let remote_sql = plan
        .lines()
        .filter(|line| line.contains(&source))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        remote_sql.to_lowercase().contains("where") && remote_sql.contains("80.5"),
        "the filter never reached the database's SQL:\n{plan}"
    );
    backend.drop_source(&source).await;
}

/// A registration node inside a loop, or a re-run of a board that reuses the session,
/// registers the same table name again. That must rebind the table, not fail.
pub async fn rerun_rebinds_table(backend: &dyn Backend) {
    let flow = Flow::local();
    let source = backend.create_source().await;
    let session = flow.create_session(&unique("e2e")).await;
    for _ in 0..2 {
        mount(&flow, backend, &session, &source, "people", true).await;
        assert_eq!(count(&flow, &session, "people").await, 5);
    }
    backend.drop_source(&source).await;
}

pub async fn sessions_are_isolated_and_reused_by_name(backend: &dyn Backend) {
    let flow = Flow::local();
    let source = backend.create_source().await;
    let name = unique("e2e");
    let session = flow.create_session(&name).await;
    mount(&flow, backend, &session, &source, "people", true).await;

    let other = flow.create_session(&unique("e2e")).await;
    assert_ne!(other.cache_key, session.cache_key);
    assert!(!flow.table_names(&other).await.contains(&"people".into()));
    flow.try_query(&other, "SELECT COUNT(*) FROM people", json!({}))
        .await
        .expect_failed("querying a table registered in a different session")
        .await;

    let again = flow.create_session(&name).await;
    assert_eq!(again.cache_key, session.cache_key);
    assert_eq!(count(&flow, &again, "people").await, 5);
    backend.drop_source(&source).await;
}

pub async fn failed_registration_stops_the_flow(backend: &dyn Backend) {
    let flow = Flow::local();
    let source = backend.create_source().await;
    let session = flow.create_session(&unique("e2e")).await;

    register(
        &flow,
        backend,
        &session,
        backend.rejected_inputs(&source, "people"),
    )
    .await
    .expect_failed("registering with rejected credentials")
    .await;
    register(
        &flow,
        backend,
        &session,
        backend.register_inputs(&backend.missing_source(), "people", true),
    )
    .await
    .expect_failed("registering a source that does not exist")
    .await;
    assert!(!flow.table_names(&session).await.contains(&"people".into()));
    backend.drop_source(&source).await;
}

pub async fn joins_within_the_session(backend: &dyn Backend) {
    let flow = Flow::local();
    let source = backend.create_source().await;
    let session = flow.create_session(&unique("e2e")).await;
    mount(&flow, backend, &session, &source, "people", true).await;
    mount(&flow, backend, &session, &source, "people_again", true).await;

    let rows = flow
        .query(
            &session,
            "SELECT a.id FROM people a JOIN people_again b ON a.id = b.id \
             WHERE b.score > 90 ORDER BY a.id",
        )
        .await;
    assert_eq!(
        ints(&rows, "id"),
        vec![1, 5],
        "self-join across two registrations"
    );

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("badge", DataType::Utf8, false),
    ]));
    let badges = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(vec![1, 3, 5])),
            Arc::new(StringArray::from(vec!["gold", "silver", "bronze"])),
        ],
    )
    .unwrap();
    flow.session_context(&session)
        .await
        .register_batch("badges", badges)
        .unwrap();
    let rows = flow
        .query(
            &session,
            &format!(
                "SELECT p.name, b.badge FROM people p JOIN badges b ON p.id = b.id \
                 WHERE {} ORDER BY p.id",
                backend.surface().active("p.active")
            ),
        )
        .await;
    assert_eq!(strings(&rows, "name"), vec!["ada", "linus"]);
    assert_eq!(strings(&rows, "badge"), vec!["gold", "silver"]);
    backend.drop_source(&source).await;
}

pub async fn writes_follow_the_readonly_pin(backend: &dyn Backend) {
    let flow = Flow::local();
    let source = backend.create_source().await;
    let session = flow.create_session(&unique("e2e")).await;
    let insert = |table: &str| {
        format!(
            "INSERT INTO {table} (id, name, score, active, created_at) \
             VALUES (6, 'barbara', 70.5, true, TIMESTAMP '2026-01-06 07:00:00')"
        )
    };

    mount(&flow, backend, &session, &source, "people_ro", true).await;
    flow.try_query(&session, &insert("people_ro"), json!({}))
        .await
        .expect_failed("INSERT into a read-only registration")
        .await;
    assert_eq!(backend.count_rows(&source).await, 5);

    let writable = register(
        &flow,
        backend,
        &session,
        backend.register_inputs(&source, "people_rw", false),
    )
    .await;
    if backend.writable() {
        writable.expect_fired("read-write registration").await;
        flow.query(&session, &insert("people_rw")).await;
        assert_eq!(backend.count_rows(&source).await, 6);
        assert_eq!(count(&flow, &session, "people_rw").await, 6);
        let rows = flow
            .query(&session, "SELECT name FROM people_rw WHERE id = 6")
            .await;
        assert_eq!(strings(&rows, "name"), vec!["barbara"]);
    } else {
        if writable.result.is_ok() {
            flow.try_query(&session, &insert("people_rw"), json!({}))
                .await
                .expect_failed("INSERT into a provider without write support")
                .await;
        }
        assert_eq!(backend.count_rows(&source).await, 5);
    }
    backend.drop_source(&source).await;
}

/// Expands to one `#[tokio::test]` per scenario for a backend constructor.
#[macro_export]
macro_rules! backend_suite {
    ($backend:expr $(, $attr:meta)*) => {
        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn registers_and_queries() {
            $crate::scenarios::registers_and_queries(&$backend).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn filters_run_inside_the_database() {
            $crate::scenarios::filters_run_inside_the_database(&$backend).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn rerun_rebinds_table() {
            $crate::scenarios::rerun_rebinds_table(&$backend).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn sessions_are_isolated_and_reused_by_name() {
            $crate::scenarios::sessions_are_isolated_and_reused_by_name(&$backend).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn failed_registration_stops_the_flow() {
            $crate::scenarios::failed_registration_stops_the_flow(&$backend).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn joins_within_the_session() {
            $crate::scenarios::joins_within_the_session(&$backend).await;
        }

        #[tokio::test(flavor = "multi_thread")]
        $(#[$attr])*
        async fn writes_follow_the_readonly_pin() {
            $crate::scenarios::writes_follow_the_readonly_pin(&$backend).await;
        }
    };
}
