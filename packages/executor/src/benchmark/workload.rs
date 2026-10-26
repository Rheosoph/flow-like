//! Synthetic rental history using the same database and UI nodes as hosted actions.

use flow_like::a2ui::{
    widget::{ExposedProp, ExposedPropType, Widget},
    SurfaceComponent,
};
use flow_like::flow::{
    board::Board,
    compiled::{compile_board_with_catalog, encode_artifact, CompiledRunTemplate},
    execution::{InternalRun, LogLevel, RunPayload, RunStatus},
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like::profile::Profile;
use flow_like::state::{FlowLikeState, FlowNodeRegistryInner};
use flow_like_catalog::CachedDB;
use flow_like_catalog_data::data::db::vector::filter::FilterLocalDatabaseNode;
use flow_like_catalog_std_ui::a2ui::{
    instantiate_widget::InstantiateWidget, push_to_container::PushToContainer,
};
use flow_like_storage::{
    databases::vector::{buffered::BufferedVectorStore, lancedb::LanceDBVectorStore, VectorStore},
    Path,
};
use flow_like_types::{
    anyhow, intercom::InterComCallback, json::json, sync::RwLock, Result, Value,
};
use std::{collections::HashMap, path::Path as FilePath, sync::Arc};

pub(crate) const APP_ID: &str = "widget-action-bench";
pub(crate) const BOARD_ID: &str = "rental-history";
pub(crate) const START_ID: &str = "filter-history";
pub(crate) const PANEL_ID: &str = "rental-history-panel";
pub(crate) const INSTANCE_ID: &str = "rental-history-instance";
const DATABASE_KEY: &str = "rental-history-db";

pub(crate) struct Workload {
    pub widget: Widget,
    pub registry: Arc<FlowNodeRegistryInner>,
    pub artifact: Vec<u8>,
    pub persons: usize,
    pub rows_per_person: usize,
    pub board_nodes: usize,
    database: LanceDBVectorStore,
}

impl Workload {
    pub async fn prepare(
        root: &FilePath,
        persons: usize,
        rows_per_person: usize,
        dormant_nodes: usize,
    ) -> Result<Self> {
        if persons == 0 || rows_per_person == 0 {
            return Err(anyhow!("persons and rows_per_person must be positive"));
        }
        let total = persons
            .checked_mul(rows_per_person)
            .filter(|total| *total <= 1_000_000)
            .ok_or_else(|| anyhow!("synthetic fixture is limited to 1,000,000 rows"))?;
        if dormant_nodes > 10_000 {
            return Err(anyhow!("dormant_nodes is limited to 10,000"));
        }

        let database_root = root.join(format!("rental-history-{}", flow_like_types::create_id()));
        std::fs::create_dir_all(&database_root)?;
        let mut database = LanceDBVectorStore::new(database_root, "rental_history".into()).await?;
        let mut rows = Vec::with_capacity(total);
        for person in 1..=persons {
            for rental in 1..=rows_per_person {
                rows.push(json!({
                    "id": format!("person-{person}-rental-{rental}"),
                    "person_id": person as i64,
                    "property": format!("Person {person} rental {rental}"),
                    "start_date": format!("2024-{:02}-01", 1 + (rental - 1) % 12),
                    "monthly_rent": 700 + ((person * 17 + rental * 31) % 1_000) as i64,
                }));
            }
        }
        database.insert(rows).await?;
        let widget = history_widget();
        let filter: Arc<dyn NodeLogic> = Arc::new(FilterLocalDatabaseNode::new());
        let instantiate: Arc<dyn NodeLogic> = Arc::new(InstantiateWidget::new());
        let push: Arc<dyn NodeLogic> = Arc::new(PushToContainer::new());
        let registry = Arc::new(FlowNodeRegistryInner::prepare(&Arc::new(vec![
            filter.clone(),
            instantiate.clone(),
            push.clone(),
        ])));
        let mut board = Board::new_detached(Some(BOARD_ID.into()), Path::from("apps").join(APP_ID));
        board.log_level = LogLevel::Debug;

        let mut filter = filter.get_node();
        filter.id = START_ID.into();
        set_default(&mut filter, "database", json!({"cache_key": DATABASE_KEY}))?;
        set_default(&mut filter, "filter", json!("person_id = 1"))?;
        set_default(&mut filter, "limit", json!(rows_per_person))?;
        let mut instantiate = instantiate.get_node();
        instantiate.id = "instantiate-history".into();
        set_default(&mut instantiate, "widget_selector", json!(widget.name))?;
        set_default(&mut instantiate, "instance_id", json!(INSTANCE_ID))?;
        instantiate
            .add_input_pin(
                "dyn_prop_rows",
                "Rows",
                "Rental history rows",
                VariableType::Generic,
            )
            .set_value_type(ValueType::Array);
        let mut push = push.get_node();
        push.id = "show-history".into();
        set_default(&mut push, "container_ref", json!({"id": PANEL_ID}))?;
        connect(&mut filter, "exec_out", &mut instantiate, "exec_in")?;
        connect(&mut filter, "values", &mut instantiate, "dyn_prop_rows")?;
        connect(&mut instantiate, "exec_out", &mut push, "exec_in")?;
        connect(&mut instantiate, "element_ref", &mut push, "element_ref")?;
        board.nodes.insert(filter.id.clone(), filter);
        board.nodes.insert(instantiate.id.clone(), instantiate);
        board.nodes.insert(push.id.clone(), push);

        // These measure template decoding and per-run allocation on a larger
        // board. They do not pretend to reproduce the live app's executed loop.
        for index in 0..dormant_nodes {
            let mut node = InstantiateWidget::new().get_node();
            node.id = format!("dormant-{index}");
            set_default(&mut node, "widget_selector", json!(widget.name))?;
            set_default(&mut node, "instance_id", json!(format!("dormant-{index}")))?;
            board.nodes.insert(node.id.clone(), node);
        }
        let board_nodes = board.nodes.len();
        let compiled = compile_board_with_catalog(&board, &registry)?;
        let artifact = encode_artifact(&compiled, &registry.fingerprint())?;
        Ok(Self {
            widget,
            registry,
            artifact,
            persons,
            rows_per_person,
            board_nodes,
            database,
        })
    }

    pub async fn new_run(
        &self,
        state: &Arc<FlowLikeState>,
        template: Arc<CompiledRunTemplate>,
        callback: InterComCallback,
        person_id: usize,
    ) -> Result<InternalRun> {
        if !(1..=self.persons).contains(&person_id) {
            return Err(anyhow!("person_id must be in 1..={}", self.persons));
        }
        let run = InternalRun::from_template(
            APP_ID,
            template,
            None,
            state,
            &Profile::default(),
            &RunPayload {
                id: START_ID.into(),
                payload: None,
                runtime_variables: None,
                filter_secrets: Some(true),
            },
            false,
            callback,
            None,
            None,
            HashMap::new(),
            None,
            None,
        )
        .await?;
        // Each execution owns its catalog cache wrapper, as hosted runs do.
        // Sharing that wrapper would serialize readers on its refresh write lock.
        let database = CachedDB {
            db: Arc::new(RwLock::new(BufferedVectorStore::new(
                self.database.clone(),
                100,
            ))),
        };
        run.cache
            .write()
            .await
            .insert(DATABASE_KEY.into(), Arc::new(database));
        // Input pins belong to this run. Changing the clicked person leaves the
        // shared template and other concurrent actions untouched.
        run.nodes
            .get(START_ID)
            .ok_or_else(|| anyhow!("filter node missing"))?
            .get_pin_by_name("filter")
            .await?
            .set_value(json!(format!("person_id = {person_id}")))
            .await;
        Ok(run)
    }

    pub async fn validate(&self, run: &InternalRun, person_id: usize) -> Result<()> {
        if !matches!(run.get_status().await, RunStatus::Success) {
            return Err(anyhow!(
                "rental-history graph did not complete successfully"
            ));
        }
        let rows: Vec<Value> = run
            .nodes
            .get(START_ID)
            .ok_or_else(|| anyhow!("filter node missing"))?
            .get_pin_by_name("values")
            .await?
            .get_value()
            .await
            .ok_or_else(|| anyhow!("filter node did not return rows"))?;
        if rows.len() != self.rows_per_person
            || rows
                .iter()
                .any(|row| row["person_id"].as_u64() != Some(person_id as u64))
        {
            return Err(anyhow!(
                "rental-history rows do not match the clicked person"
            ));
        }
        let element: Value = run
            .nodes
            .get("instantiate-history")
            .ok_or_else(|| anyhow!("widget node missing"))?
            .get_pin_by_name("element_ref")
            .await?
            .get_value()
            .await
            .ok_or_else(|| anyhow!("widget node did not return an element"))?;
        let emitted_rows = element
            .pointer("/component/exposedPropValues/rows")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("rendered widget does not contain rental rows"))?;
        if emitted_rows != &rows || element["component"]["widgetId"] != self.widget.id {
            return Err(anyhow!("rendered widget differs from the database result"));
        }
        Ok(())
    }
}

fn set_default(node: &mut Node, pin: &str, value: Value) -> Result<()> {
    let node_id = node.id.clone();
    node.get_pin_mut_by_name(pin)
        .ok_or_else(|| anyhow!("node {node_id} has no {pin} pin"))?
        .set_default_value(Some(value));
    Ok(())
}

fn connect(source: &mut Node, output: &str, target: &mut Node, input: &str) -> Result<()> {
    let output_id = source
        .get_pin_by_name(output)
        .ok_or_else(|| anyhow!("missing output pin {output}"))?
        .id
        .clone();
    let input_id = target
        .get_pin_by_name(input)
        .ok_or_else(|| anyhow!("missing input pin {input}"))?
        .id
        .clone();
    source
        .get_pin_mut_by_name(output)
        .unwrap()
        .connected_to
        .insert(input_id);
    target
        .get_pin_mut_by_name(input)
        .unwrap()
        .depends_on
        .insert(output_id);
    Ok(())
}

fn history_widget() -> Widget {
    let mut widget = Widget::new("rental-history-widget", "Rental history", "history-table");
    widget.components.push(SurfaceComponent::new("history-table", json!({
        "type": "table",
        "columns": {"literalJson": serde_json::to_string(&json!([
            {"id":"property","header":{"literalString":"Property"},"accessor":{"literalString":"property"}},
            {"id":"start_date","header":{"literalString":"Started"},"accessor":{"literalString":"start_date"}},
            {"id":"monthly_rent","header":{"literalString":"Monthly rent"},"accessor":{"literalString":"monthly_rent"}},
        ])).expect("static columns serialize")},
        "data": {"literalJson":"[]"},
        "compact": {"literalBool":true},
        "paginated": {"literalBool":false}
    })));
    widget.exposed_props.push(
        ExposedProp::new(
            "rows",
            "Rows",
            "history-table",
            "data",
            ExposedPropType::Json,
        )
        .with_default(Vec::<Value>::new()),
    );
    widget
}
