use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::{async_trait, json::json};

use super::NodeDBConnection;

#[cfg(any(feature = "execute", test))]
#[derive(Debug, PartialEq)]
enum ColumnDefinition {
    SqlExpression(String),
    Vector(u32),
}

#[cfg(any(feature = "execute", test))]
async fn read_column_definition(
    context: &ExecutionContext,
) -> flow_like_types::Result<ColumnDefinition> {
    // Boards saved before version 3 only have the SQL expression input.
    let mode = if context.get_pin_by_name("mode").await.is_ok() {
        context.evaluate_pin::<String>("mode").await?
    } else {
        "SQL Expression".to_string()
    };
    match mode.as_str() {
        "SQL Expression" => Ok(ColumnDefinition::SqlExpression(
            context.evaluate_pin("sql_expression").await?,
        )),
        "Vector" => {
            let dimensions: i64 = context.evaluate_pin("vector_size").await?;
            if !(1..=i64::from(i32::MAX)).contains(&dimensions) {
                return Err(flow_like_types::anyhow!(
                    "Vector dimensions must be between 1 and {} and match the embedding model's output length",
                    i32::MAX
                ));
            }
            Ok(ColumnDefinition::Vector(dimensions as u32))
        }
        _ => Err(flow_like_types::anyhow!(
            "Unsupported column mode '{mode}'; choose SQL Expression or Vector"
        )),
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AddColumnLocalDatabaseNode {}

impl AddColumnLocalDatabaseNode {
    pub fn new() -> Self {
        AddColumnLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for AddColumnLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "add_column_local_db",
            "Add Column",
            "Adds a column from a typed SQL expression or creates an empty vector column with a fixed dimension. Vector dimensions must match the embedding model's output length. Existing rows get NULL vectors until embeddings are written.",
            "Data/Database/Schema",
        );
        node.set_flowscript_name("db", "addColumn");
        node.set_receiver("database");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "database",
            "Database",
            "Database Connection Reference",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "column_name",
            "Column Name",
            "Name of the column to add",
            VariableType::String,
        )
        .set_default_value(Some(json!("new_column")));

        node.add_input_pin(
            "sql_expression",
            "SQL Expression",
            "Typed SQL expression used to populate existing rows. Examples: 0, '', CAST(NULL AS STRING). Flow-Like requires an explicit type for NULL. Supported types: int, bigint, float, double, string, binary, boolean, date, timestamp.",
            VariableType::String,
        )
        .set_default_value(Some(json!("CAST(NULL AS STRING)")));

        node.add_input_pin(
            "mode",
            "Mode",
            "SQL Expression computes values from SQL Expression. Vector creates a nullable Float32 vector column and ignores SQL Expression.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["SQL Expression".into(), "Vector".into()])
                .build(),
        )
        .set_default_value(Some(json!("SQL Expression")));

        node.add_input_pin(
            "vector_size",
            "Vector Dimensions",
            "Required in Vector mode: the positive number of values in each embedding. Set this to the embedding model's output length. Ignored in SQL Expression mode.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done altering schema",
            VariableType::Execution,
        );
        node.add_output_pin(
            "schema",
            "Schema",
            "Updated database schema",
            VariableType::Struct,
        )
        .set_schema::<crate::data::db::table_schema::TableSchema>();

        node.set_version(3);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let column_name: String = context.evaluate_pin("column_name").await?;
        let definition = read_column_definition(context).await?;

        let cached_db = database.load(context).await?;
        let database = cached_db.flushed_read().await?;

        match definition {
            ColumnDefinition::SqlExpression(expression) => {
                database
                    .inner()
                    .add_column(&column_name, &expression)
                    .await?;
            }
            ColumnDefinition::Vector(dimensions) => {
                database
                    .inner()
                    .add_typed_column(&column_name, "vector", Some(dimensions))
                    .await?;
            }
        }

        let schema = database.schema().await?;
        context.set_pin_value("schema", json!(schema)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ahash::AHashMap;
    use flow_like::{
        flow::{
            board::ExecutionStage,
            execution::{LogLevel, Run, internal_node::InternalNode, internal_pin::InternalPin},
        },
        profile::Profile,
        state::{FlowLikeConfig, FlowLikeState},
        utils::http::HTTPClient,
    };
    use flow_like_types::sync::{Mutex, RwLock};
    use std::sync::{Arc, Weak};

    async fn context(node: Node) -> ExecutionContext {
        let mut pins = AHashMap::new();
        let mut names: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();
        for pin in node.pins.values() {
            let internal = Arc::new(InternalPin::new(pin, false));
            names
                .entry(pin.name.clone())
                .or_default()
                .push(internal.clone());
            pins.insert(pin.id.clone(), internal);
        }
        let node = Arc::new(InternalNode::new(
            node,
            pins,
            Arc::new(AddColumnLocalDatabaseNode::new()),
            names,
        ));
        for pin in node.pins.iter() {
            pin.init_node(Arc::downgrade(&node));
        }
        let mut nodes = AHashMap::new();
        nodes.insert(node.node_id().to_string(), node.clone());
        let state = Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ));
        let run: Weak<Mutex<Run>> = Weak::new();
        ExecutionContext::new(
            Arc::new(nodes),
            &run,
            &state,
            &node,
            &Arc::new(Mutex::new(AHashMap::new())),
            &Arc::new(RwLock::new(AHashMap::new())),
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

    fn set_default(node: &mut Node, name: &str, value: flow_like_types::Value) {
        node.get_pin_mut_by_name(name)
            .unwrap()
            .set_default_value(Some(value));
    }

    #[tokio::test]
    async fn old_boards_keep_the_sql_expression_path() {
        let mut node = AddColumnLocalDatabaseNode::new().get_node();
        node.set_version(2);
        node.pins
            .retain(|_, pin| pin.name != "mode" && pin.name != "vector_size");
        set_default(&mut node, "sql_expression", json!("CAST(7 AS INT)"));
        assert_eq!(
            read_column_definition(&context(node).await).await.unwrap(),
            ColumnDefinition::SqlExpression("CAST(7 AS INT)".into())
        );
    }

    #[tokio::test]
    async fn sql_mode_is_default_and_does_not_read_vector_dimensions() {
        let mut node = AddColumnLocalDatabaseNode::new().get_node();
        set_default(&mut node, "vector_size", json!("unused"));
        assert_eq!(
            read_column_definition(&context(node).await).await.unwrap(),
            ColumnDefinition::SqlExpression("CAST(NULL AS STRING)".into())
        );
    }

    #[tokio::test]
    async fn vector_mode_does_not_read_the_sql_expression() {
        let mut node = AddColumnLocalDatabaseNode::new().get_node();
        set_default(&mut node, "mode", json!("Vector"));
        set_default(&mut node, "vector_size", json!(768));
        node.pins.retain(|_, pin| pin.name != "sql_expression");
        assert_eq!(
            read_column_definition(&context(node).await).await.unwrap(),
            ColumnDefinition::Vector(768)
        );
    }

    #[tokio::test]
    async fn vector_dimensions_reject_invalid_and_overflowing_sizes() {
        for dimensions in [
            json!(-1),
            json!(0),
            json!(i64::from(i32::MAX) + 1),
            json!(2.5),
        ] {
            let mut node = AddColumnLocalDatabaseNode::new().get_node();
            set_default(&mut node, "mode", json!("Vector"));
            set_default(&mut node, "vector_size", dimensions.clone());
            assert!(
                read_column_definition(&context(node).await).await.is_err(),
                "invalid vector dimensions were accepted: {dimensions}"
            );
        }
    }

    #[tokio::test]
    async fn invalid_modes_do_not_fall_back_to_sql() {
        for mode in [json!("Unknown"), json!(true)] {
            let mut node = AddColumnLocalDatabaseNode::new().get_node();
            set_default(&mut node, "mode", mode);
            assert!(read_column_definition(&context(node).await).await.is_err());
        }
    }
}
