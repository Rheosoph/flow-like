use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
#[cfg(feature = "execute")]
use flow_like_storage_contracts::column_metadata::DataSensitivity;
use flow_like_storage_contracts::column_metadata::{ColumnMetadata, DataClassification};
use flow_like_types::{async_trait, json::json};

use super::NodeDBConnection;

#[cfg(any(feature = "execute", test))]
fn parse_classification(value: &str) -> flow_like_types::Result<Option<DataClassification>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    DataClassification::parse(value).map(Some).ok_or_else(|| {
        flow_like_types::anyhow!(
            "Unknown classification '{value}'; use one of {} or leave it empty to clear it",
            DataClassification::names().join(", ")
        )
    })
}

#[cfg(feature = "execute")]
async fn requested_metadata(context: &ExecutionContext) -> flow_like_types::Result<ColumnMetadata> {
    let classification: String = context.evaluate_pin("classification").await?;
    Ok(ColumnMetadata {
        display_name: Some(context.evaluate_pin("display_name").await?),
        description: Some(context.evaluate_pin("description").await?),
        sensitivity: DataSensitivity {
            classification: parse_classification(&classification)?,
            pii: context.evaluate_pin("pii").await?,
        },
        read_attributes: context.evaluate_pin("read_attributes").await?,
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct SetColumnMetadataLocalDatabaseNode {}

impl SetColumnMetadataLocalDatabaseNode {
    pub fn new() -> Self {
        SetColumnMetadataLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for SetColumnMetadataLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "set_column_metadata_local_db",
            "Set Column Metadata",
            "Replaces a column's metadata: display name, description, data classification, personal data flag and the role attributes allowed to read it through the API. Every value is written as given, so empty text, false and an empty list clear what was stored. The table key and other column settings stay. Flow runs always read every column.",
            "Data/Database/Schema",
        );
        node.set_flowscript_name("db", "setColumnMetadata");
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
            "Column whose metadata to replace",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "display_name",
            "Display Name",
            "Human-readable name shown instead of the column name, at most 128 characters. Empty clears it.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "description",
            "Description",
            "What the column holds and how to use it, at most 4096 characters. Empty clears it.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "classification",
            "Classification",
            "How sensitive the column's data is, from least to most: public, internal, confidential or restricted. Empty clears the classification.",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(
                    std::iter::once(String::new())
                        .chain(DataClassification::names())
                        .collect(),
                )
                .build(),
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "pii",
            "PII",
            "Whether the column holds personal data that identifies or relates to a person",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "read_attributes",
            "Read Attributes",
            "Role attributes allowed to read the column through the API, at most 64. Empty lets everyone who can read the table see it; otherwise only admins and roles holding at least one of these attributes see it.",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_output_pin(
            "exec_out",
            "Done",
            "The column's metadata was replaced",
            VariableType::Execution,
        );
        node.add_output_pin(
            "metadata",
            "Metadata",
            "The column's metadata as stored: values trimmed, blank ones dropped and read attributes de-duplicated",
            VariableType::Struct,
        )
        .set_schema::<ColumnMetadata>();
        node.add_output_pin(
            "schema",
            "Schema",
            "Updated database schema",
            VariableType::Struct,
        )
        .set_schema::<crate::data::db::table_schema::TableSchema>();

        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(6)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let column_name: String = context.evaluate_pin("column_name").await?;
        let metadata = requested_metadata(context).await?;

        let cached_db = database.load(context).await?;
        let database = cached_db.flushed_read().await?;

        let metadata = database
            .inner()
            .set_column_metadata(&column_name, metadata)
            .await?;

        let schema = database.schema().await?;
        context.set_pin_value("metadata", json!(metadata)).await?;
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

    #[test]
    fn empty_classification_clears_the_level() {
        assert_eq!(parse_classification("").unwrap(), None);
        assert_eq!(parse_classification("  ").unwrap(), None);
    }

    #[test]
    fn classification_names_parse_case_insensitively() {
        assert_eq!(
            parse_classification(" Confidential ").unwrap(),
            Some(DataClassification::Confidential)
        );
    }

    #[test]
    fn unknown_classification_names_the_value_and_every_level() {
        let error = parse_classification("secret").unwrap_err().to_string();
        assert!(error.contains("'secret'"), "{error}");
        for level in DataClassification::names() {
            assert!(error.contains(&level), "{error}");
        }
    }

    #[test]
    fn classification_pin_offers_empty_and_every_level() {
        let node = SetColumnMetadataLocalDatabaseNode::new().get_node();
        let valid_values = node
            .get_pin_by_name("classification")
            .and_then(|pin| pin.options.as_ref())
            .and_then(|options| options.valid_values.clone())
            .expect("classification valid values");
        assert_eq!(
            valid_values,
            ["", "public", "internal", "confidential", "restricted"]
        );
    }
}
