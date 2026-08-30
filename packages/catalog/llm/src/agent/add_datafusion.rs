use crate::generative::agent::Agent;
#[cfg(feature = "execute")]
use crate::generative::agent::DataFusionContext;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_catalog_data::data::datafusion::session::DataFusionSession;
use flow_like_types::{async_trait, json};
use std::collections::HashMap;

/// Node to add a DataFusion session context to an Agent
/// Enables the agent to query SQL data using DataFusion
#[crate::register_node]
#[derive(Default)]
pub struct AddDataFusionNode;

impl AddDataFusionNode {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for AddDataFusionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "add_datafusion_to_agent",
            "Add DataFusion",
            "Add a DataFusion SQL session to an agent for data analysis capabilities",
            "AI/Agents/Builder",
        );
        node.set_flowscript_name("agent", "addDatafusion");
        node.set_receiver("agent");
        node.set_version(2);
        node.add_icon("/flow/icons/database.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Add",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "agent",
            "Agent",
            "Agent to add DataFusion context to",
            VariableType::Struct,
        )
        .set_schema::<Agent>();

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session from CreateDataFusionSession node",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "description",
            "Description",
            "User-friendly description of this data source",
            VariableType::String,
        );

        node.add_input_pin(
            "table_descriptions",
            "Table Descriptions",
            "Map of table names to descriptions (JSON object)",
            VariableType::Struct,
        )
        .set_schema::<HashMap<String, String>>();

        node.add_input_pin(
            "example_queries",
            "Example Queries",
            "Example SQL queries that work with this data",
            VariableType::Generic,
        );

        node.add_input_pin(
            "discover_schemas",
            "Discover Schemas",
            "Automatically discover table schemas at runtime",
            VariableType::Boolean,
        )
        .set_default_value(Some(json::json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution completed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Agent with DataFusion context added",
            VariableType::Struct,
        )
        .set_schema::<Agent>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut agent: Agent = context.evaluate_pin("agent").await?;
        let session: DataFusionSession = context.evaluate_pin("session").await?;

        let description: Option<String> = context.evaluate_pin("description").await.ok();
        let table_descriptions: Option<HashMap<String, String>> =
            context.evaluate_pin("table_descriptions").await.ok();
        let example_queries: Option<Vec<String>> =
            context.evaluate_pin("example_queries").await.ok();
        let discover_schemas: bool = context
            .evaluate_pin("discover_schemas")
            .await
            .unwrap_or(true);

        let mut df_context = DataFusionContext::new(session.cache_key.clone());
        df_context.description = description;

        if let Some(table_descs) = table_descriptions {
            df_context.table_descriptions = table_descs;
        }

        if let Some(examples) = example_queries {
            df_context.example_queries = examples;
        }

        // Auto-discover table schemas if requested
        if discover_schemas {
            let cached_session = session.load(context).await?;
            let catalog_names = cached_session.ctx.catalog_names();
            for catalog in catalog_names {
                if let Some(catalog_provider) = cached_session.ctx.catalog(&catalog) {
                    for schema_name in catalog_provider.schema_names() {
                        if let Some(schema_provider) = catalog_provider.schema(&schema_name) {
                            for table_name in schema_provider.table_names() {
                                if let Ok(Some(table)) = schema_provider.table(&table_name).await {
                                    let schema = table.schema();
                                    let fields: Vec<String> = schema
                                        .fields()
                                        .iter()
                                        .map(|f| format!("{}: {}", f.name(), f.data_type()))
                                        .collect();
                                    df_context
                                        .table_schemas
                                        .insert(table_name.to_string(), fields.join(", "));
                                }
                            }
                        }
                    }
                }
            }
        }

        agent.add_datafusion_context(df_context);

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "DataFusion agent integration requires the 'execute' feature"
        ))
    }
}
