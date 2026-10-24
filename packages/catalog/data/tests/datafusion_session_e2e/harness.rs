use ahash::AHashMap;
use flow_like_catalog_data::data::datafusion::query::SqlQueryNode;
use flow_like_catalog_data::data::datafusion::session::{
    CachedDataFusionSession, CreateDataFusionSessionNode, DataFusionSession,
};
use flow_like_catalog_data::data::datafusion::tools::ListTablesNode;
use flow_like_runtime::flow::board::ExecutionStage;
use flow_like_runtime::flow::execution::{
    ExecutionEnvironment, LogLevel, context::ExecutionContext, internal_node::InternalNode,
    internal_pin::InternalPin,
};
use flow_like_runtime::flow::node::NodeLogic;
use flow_like_runtime::profile::Profile;
use flow_like_runtime::state::{FlowLikeConfig, FlowLikeState};
use flow_like_runtime::utils::http::HTTPClient;
use flow_like_storage::datafusion::prelude::SessionContext;
use flow_like_types::json::{Map, json};
use flow_like_types::sync::{Mutex, RwLock};
use flow_like_types::{Cacheable, Value};
use std::sync::{Arc, Weak};

pub type Row = Map<String, Value>;

/// One flow execution: every node run through it shares the execution cache, which is
/// where DataFusion sessions and database connections live between nodes.
pub struct Flow {
    state: Arc<FlowLikeState>,
    cache: Arc<RwLock<AHashMap<String, Arc<dyn Cacheable>>>>,
}

pub struct NodeRun {
    pub result: flow_like_types::Result<()>,
    context: ExecutionContext,
}

impl NodeRun {
    pub async fn output(&self, name: &str) -> Value {
        self.context
            .get_pin_by_name(name)
            .await
            .unwrap_or_else(|_| panic!("node has no output pin '{name}'"))
            .get_raw_value()
            .await
            .unwrap_or(Value::Null)
    }

    pub async fn output_if_present(&self, name: &str) -> Option<Value> {
        let pin = self.context.get_pin_by_name(name).await.ok()?;
        pin.get_raw_value().await
    }

    pub async fn fired(&self) -> bool {
        self.output("exec_out").await == json!(true)
    }

    pub async fn expect_fired(self, what: &str) -> Self {
        if let Err(error) = &self.result {
            panic!("{what} failed: {error:#}");
        }
        assert!(
            self.fired().await,
            "{what} succeeded without firing exec_out"
        );
        self
    }

    pub async fn expect_failed(self, what: &str) -> String {
        let error = match &self.result {
            Ok(()) => panic!("{what} was expected to fail but succeeded"),
            Err(error) => format!("{error:#}"),
        };
        assert!(
            !self.fired().await,
            "{what} failed ({error}) but still fired exec_out, so the flow would continue"
        );
        error
    }
}

impl Flow {
    pub fn local() -> Self {
        Self::in_environment(ExecutionEnvironment::Local)
    }

    pub fn server() -> Self {
        Self::in_environment(ExecutionEnvironment::Server)
    }

    fn in_environment(environment: ExecutionEnvironment) -> Self {
        let mut state =
            FlowLikeState::new(FlowLikeConfig::new(), HTTPClient::new_without_refetch());
        state.execution_environment = environment;
        Self {
            state: Arc::new(state),
            cache: Arc::new(RwLock::new(AHashMap::new())),
        }
    }

    pub async fn cache_insert(&self, key: &str, value: Arc<dyn Cacheable>) {
        self.cache.write().await.insert(key.to_string(), value);
    }

    /// Runs one node with `inputs` written onto its (unconnected) input pins.
    pub async fn run(&self, logic: Arc<dyn NodeLogic>, inputs: Value) -> NodeRun {
        let mut context = self.context_for(logic.clone()).await;
        let Value::Object(inputs) = inputs else {
            panic!("node inputs must be a JSON object, got {inputs}");
        };
        for (name, value) in inputs {
            context
                .get_pin_by_name(&name)
                .await
                .unwrap_or_else(|_| panic!("node has no input pin '{name}'"))
                .set_value(value)
                .await;
        }
        let result = logic.run(&mut context).await;
        NodeRun { result, context }
    }

    async fn context_for(&self, logic: Arc<dyn NodeLogic>) -> ExecutionContext {
        let node = logic.get_node();
        let mut pins = AHashMap::new();
        let mut names = AHashMap::<String, Vec<Arc<InternalPin>>>::new();
        for pin in node.pins.values() {
            let internal = Arc::new(InternalPin::new(pin, false));
            names
                .entry(pin.name.clone())
                .or_default()
                .push(internal.clone());
            pins.insert(pin.id.clone(), internal);
        }
        let current = Arc::new(InternalNode::new(node, pins, logic, names));
        for pin in current.pins.iter() {
            pin.init_node(Arc::downgrade(&current));
            pin.init_connected_to(Vec::new());
            pin.init_depends_on(Vec::new());
        }
        ExecutionContext::new(
            Arc::new(AHashMap::from_iter([(
                current.node_id().to_owned(),
                current.clone(),
            )])),
            &Weak::new(),
            &self.state,
            &current,
            &Arc::new(Mutex::new(AHashMap::new())),
            &self.cache,
            LogLevel::Debug,
            ExecutionStage::Dev,
            Arc::new(Profile::default()),
            None,
            Arc::new(RwLock::new(Vec::new())),
            None,
            None,
            Arc::new(AHashMap::new()),
            None,
        )
        .await
    }

    pub async fn create_session(&self, name: &str) -> DataFusionSession {
        let run = self
            .run(
                Arc::new(CreateDataFusionSessionNode::new()),
                json!({ "session_name": name }),
            )
            .await
            .expect_fired("Create DataFusion Session")
            .await;
        flow_like_types::json::from_value(run.output("session").await)
            .expect("session output is a DataFusionSession")
    }

    pub async fn try_query(
        &self,
        session: &DataFusionSession,
        sql: &str,
        params: Value,
    ) -> NodeRun {
        self.run(
            Arc::new(SqlQueryNode::new()),
            json!({ "session": session, "query": sql, "params": params }),
        )
        .await
    }

    pub async fn query(&self, session: &DataFusionSession, sql: &str) -> Vec<Row> {
        self.query_with(session, sql, json!({})).await
    }

    pub async fn query_with(
        &self,
        session: &DataFusionSession,
        sql: &str,
        params: Value,
    ) -> Vec<Row> {
        let run = self
            .try_query(session, sql, params)
            .await
            .expect_fired(&format!("SQL Query `{sql}`"))
            .await;
        match run.output("rows").await {
            Value::Array(rows) => rows
                .into_iter()
                .map(|row| match row {
                    Value::Object(row) => row,
                    other => panic!("SQL Query row is not an object: {other}"),
                })
                .collect(),
            other => panic!("SQL Query rows output is not an array: {other}"),
        }
    }

    pub async fn table_names(&self, session: &DataFusionSession) -> Vec<String> {
        let run = self
            .run(
                Arc::new(ListTablesNode::new()),
                json!({ "session": session }),
            )
            .await
            .expect_fired("List Tables")
            .await;
        flow_like_types::json::from_value(run.output("table_names").await)
            .expect("table_names output is a list of strings")
    }

    pub async fn session_context(&self, session: &DataFusionSession) -> Arc<SessionContext> {
        let cache = self.cache.read().await;
        let cached = cache
            .get(&session.cache_key)
            .unwrap_or_else(|| panic!("session '{}' is not cached", session.cache_key));
        cached
            .as_any()
            .downcast_ref::<CachedDataFusionSession>()
            .expect("session cache entry is a CachedDataFusionSession")
            .ctx
            .clone()
    }
}

pub fn unique(prefix: &str) -> String {
    format!(
        "{prefix}_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

pub fn column<'a>(rows: &'a [Row], name: &str) -> Vec<&'a Value> {
    rows.iter()
        .map(|row| {
            row.get(name)
                .unwrap_or_else(|| panic!("row {row:?} has no column '{name}'"))
        })
        .collect()
}

pub fn ints(rows: &[Row], name: &str) -> Vec<i64> {
    column(rows, name)
        .into_iter()
        .map(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                .unwrap_or_else(|| panic!("column '{name}' value {value} is not an integer"))
        })
        .collect()
}

pub fn floats(rows: &[Row], name: &str) -> Vec<f64> {
    column(rows, name)
        .into_iter()
        .map(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
                .unwrap_or_else(|| panic!("column '{name}' value {value} is not a number"))
        })
        .collect()
}

pub fn strings(rows: &[Row], name: &str) -> Vec<String> {
    column(rows, name)
        .into_iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("column '{name}' value {value} is not a string"))
                .to_string()
        })
        .collect()
}
