use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_storage_contracts::column_metadata::ColumnMetadata;
use flow_like_types::{async_trait, json::json};

use super::NodeDBConnection;

#[cfg(any(feature = "execute", test))]
fn output_values(metadata: &ColumnMetadata) -> [(&str, flow_like_types::Value); 6] {
    [
        (
            "display_name",
            json!(metadata.display_name.as_deref().unwrap_or_default()),
        ),
        (
            "description",
            json!(metadata.description.as_deref().unwrap_or_default()),
        ),
        (
            "classification",
            json!(
                metadata
                    .sensitivity
                    .classification
                    .map_or("", |level| level.as_str())
            ),
        ),
        ("pii", json!(metadata.sensitivity.pii)),
        ("read_attributes", json!(metadata.read_attributes)),
        ("metadata", json!(metadata)),
    ]
}

#[crate::register_node]
#[derive(Default)]
pub struct GetColumnMetadataLocalDatabaseNode {}

impl GetColumnMetadataLocalDatabaseNode {
    pub fn new() -> Self {
        GetColumnMetadataLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for GetColumnMetadataLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "get_column_metadata_local_db",
            "Get Column Metadata",
            "Reads a column's metadata: display name, description, data classification, personal data flag and the role attributes allowed to read it through the API. Values nobody set come back empty.",
            "Data/Database/Meta",
        );
        node.set_flowscript_name("db", "columnMetadata");
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
            "Column whose metadata to read",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done reading the column's metadata",
            VariableType::Execution,
        );
        node.add_output_pin(
            "display_name",
            "Display Name",
            "Human-readable name shown instead of the column name; empty when unset",
            VariableType::String,
        );
        node.add_output_pin(
            "description",
            "Description",
            "What the column holds and how to use it; empty when unset",
            VariableType::String,
        );
        node.add_output_pin(
            "classification",
            "Classification",
            "public, internal, confidential or restricted; empty when the column is unclassified",
            VariableType::String,
        );
        node.add_output_pin(
            "pii",
            "PII",
            "Whether the column is marked as holding personal data",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "read_attributes",
            "Read Attributes",
            "Role attributes allowed to read the column through the API; empty when everyone who can read the table may read it",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "metadata",
            "Metadata",
            "All of the column's metadata as one struct",
            VariableType::Struct,
        )
        .set_schema::<ColumnMetadata>();

        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(9)
                .set_performance(9)
                .set_governance(9)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let column_name: String = context.evaluate_pin("column_name").await?;

        let cached_db = database.load(context).await?;
        let database = cached_db.flushed_read().await?;

        let metadata = database.inner().column_metadata(&column_name).await?;

        for (name, value) in output_values(&metadata) {
            context.set_pin_value(name, value).await?;
        }
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
    use flow_like::flow::pin::PinType;
    use flow_like_storage_contracts::column_metadata::{DataClassification, DataSensitivity};

    #[test]
    fn unset_metadata_reads_as_empty_values() {
        assert_eq!(
            output_values(&ColumnMetadata::default()),
            [
                ("display_name", json!("")),
                ("description", json!("")),
                ("classification", json!("")),
                ("pii", json!(false)),
                ("read_attributes", json!([])),
                ("metadata", json!({ "pii": false, "read_attributes": [] })),
            ]
        );
    }

    #[test]
    fn stored_metadata_fills_every_output() {
        let metadata = ColumnMetadata {
            display_name: Some("Annual salary".into()),
            description: Some("Gross, EUR".into()),
            sensitivity: DataSensitivity {
                classification: Some(DataClassification::Confidential),
                pii: true,
            },
            read_attributes: vec!["hr".into()],
        };
        let node = GetColumnMetadataLocalDatabaseNode::new().get_node();
        let outputs = output_values(&metadata);

        for (name, _) in &outputs {
            assert!(
                node.get_pin_by_name(name)
                    .is_some_and(|pin| pin.pin_type == PinType::Output),
                "{name} is not an output pin"
            );
        }
        assert_eq!(outputs[2], ("classification", json!("confidential")));
        assert_eq!(outputs[4], ("read_attributes", json!(["hr"])));
        assert_eq!(
            outputs[5].1,
            json!({
                "display_name": "Annual salary",
                "description": "Gross, EUR",
                "classification": "confidential",
                "pii": true,
                "read_attributes": ["hr"]
            })
        );
    }
}
