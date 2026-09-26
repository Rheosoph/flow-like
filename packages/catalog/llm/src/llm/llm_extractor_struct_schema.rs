use async_trait::async_trait;
use flow_like::{
    bit::Bit,
    flow::{
        board::Board,
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::{PinOptions, ValueType, is_open_object_schema, resolve_schema},
        variable::VariableType,
    },
};
use flow_like_types::{Value, json};

#[cfg(feature = "execute")]
use super::llm_extractor::{prepare_reference_schema, run_text_extraction};

#[crate::register_node]
#[derive(Default)]
pub struct LLMExtractWithStructSchemaNode {}

impl LLMExtractWithStructSchemaNode {
    pub fn new() -> Self {
        Self {}
    }
}

fn set_schema(node: &mut Node, schema: Option<String>) {
    for pin_name in ["struct_shape", "response"] {
        let Some(pin) = node.get_pin_mut_by_name(pin_name) else {
            continue;
        };

        match &schema {
            Some(schema) => pin.schema = Some(schema.clone()),
            None => {
                pin.set_open_schema();
            }
        }
        pin.data_type = VariableType::Struct;
        pin.value_type = ValueType::Normal;
    }

    if let Some(reference) = node.get_pin_mut_by_name("struct_shape") {
        reference.set_options(PinOptions::new().set_enforce_schema(false).build());
    }
}

fn forget_schema(node: &mut Node, error: Option<String>) {
    set_schema(node, None);
    node.error = error;
}

fn schema_is_struct(schema: &str) -> Result<(), String> {
    let parsed: Value = json::from_str(schema)
        .map_err(|error| format!("Reference struct schema is not valid JSON: {error}"))?;
    let object = parsed
        .as_object()
        .ok_or_else(|| "Reference struct schema must be a JSON object".to_string())?;

    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err("Reference struct schema must describe an object at its root".to_string());
    }

    #[cfg(feature = "execute")]
    if !jsonschema::meta::try_is_valid(&parsed).unwrap_or(false) {
        return Err("Reference struct does not carry a valid JSON Schema".to_string());
    }

    #[cfg(feature = "execute")]
    super::llm_extractor::compile_validator(&parsed)
        .map_err(|error| format!("Reference struct schema cannot be compiled: {error}"))?;

    Ok(())
}

#[cfg(feature = "execute")]
async fn stamped_response_schema(context: &ExecutionContext) -> flow_like_types::Result<String> {
    let declared = {
        let node = context.node.node.lock().await;
        node.get_pin_by_name("response")
            .and_then(|pin| pin.schema.clone())
    }
    .ok_or_else(|| flow_like_types::anyhow!("Reference struct does not carry a schema"))?;

    let board = context.get_board().await?;
    let resolved = resolve_schema(&declared, &board.refs)?.to_string();
    if is_open_object_schema(&resolved) {
        return Err(flow_like_types::anyhow!(
            "Reference struct does not carry a concrete schema"
        ));
    }
    Ok(resolved)
}

#[async_trait]
impl NodeLogic for LLMExtractWithStructSchemaNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "llm_extractor_struct_schema",
            "AI Extractor with Struct Schema",
            "Uses an LLM and a reference struct's schema to extract structured data from free-form text",
            "AI/Generative",
        );
        node.set_flowscript_name("ai", "extractWithStructSchema");
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(2);

        node.set_scores(
            NodeScores::new()
                .set_privacy(4)
                .set_security(4)
                .set_performance(6)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger to start the extraction",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Bit pointing to the LLM that will perform the extraction",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "struct_shape",
            "Schema Reference",
            "A reference struct whose schema defines the extracted data. Its value is never evaluated",
            VariableType::Struct,
        )
        .set_open_schema()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "text",
            "Text",
            "Raw text that should be structured via the reference schema",
            VariableType::String,
        );

        node.add_input_pin(
            "hint",
            "Extraction Hint",
            "Optional hint to guide the extraction, such as selecting line items instead of totals",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("")));

        node.add_input_pin(
            "max_tokens",
            "Max Tokens",
            "Output token budget for the model. 0 leaves it to the provider. Raise this if large extractions come back empty because the model ran out of room before calling the tool",
            VariableType::Integer,
        )
        .set_default_value(Some(json::json!(0)));

        node.add_output_pin(
            "exec_out",
            "Execution Output",
            "Executes after extraction succeeds",
            VariableType::Execution,
        );

        node.add_output_pin(
            "response",
            "Json",
            "Structured JSON value that matches the reference schema",
            VariableType::Struct,
        )
        .set_open_schema();

        node.add_output_pin(
            "stats",
            "Stats",
            "Token usage, cost, and model statistics",
            VariableType::Struct,
        )
        .set_schema::<flow_like_model_provider::response::LLMUsageStats>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let schema = stamped_response_schema(context).await?;
        let prepared_schema = prepare_reference_schema(&schema)?;
        run_text_extraction(context, prepared_schema).await
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        let Some(reference) = node.get_pin_by_name("struct_shape") else {
            return;
        };

        let Some(donor_id) = reference.depends_on.iter().next().cloned() else {
            forget_schema(node, None);
            return;
        };

        // The board can briefly omit a node during an update sweep. Preserve the last known
        // shape until a complete pass can decide whether the connection still exists.
        let Some(donor) = board.get_pin_by_id(&donor_id) else {
            return;
        };

        let Some(schema_ref) = donor.schema.as_deref() else {
            forget_schema(
                node,
                Some("Connected reference struct has no schema".to_string()),
            );
            return;
        };

        let schema = match resolve_schema(schema_ref, &board.refs) {
            Ok(schema) => schema.to_string(),
            Err(error) => {
                forget_schema(
                    node,
                    Some(format!(
                        "Could not resolve reference struct schema: {error}"
                    )),
                );
                return;
            }
        };

        if is_open_object_schema(&schema) {
            forget_schema(
                node,
                Some("Reference struct does not carry a concrete schema".to_string()),
            );
            return;
        }

        if let Err(error) = schema_is_struct(&schema) {
            forget_schema(node, Some(error));
            return;
        }

        set_schema(node, Some(schema));
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use flow_like::flow::{node::NodeLogic, pin::is_open_object_schema};
    use flow_like_storage::object_store::path::Path;

    use super::*;

    const PERSON_SCHEMA: &str = r#"{"type":"object","properties":{"name":{"type":"string"},"age":{"type":"integer"}},"required":["name","age"]}"#;
    const ORDER_SCHEMA: &str =
        r#"{"type":"object","properties":{"orderId":{"type":"string"}},"required":["orderId"]}"#;

    fn board() -> Board {
        Board::new_detached(
            Some("struct-schema-extractor-test".to_string()),
            Path::default(),
        )
    }

    fn donor(schema: Option<&str>) -> Node {
        let mut donor = Node::new("schema_donor", "Schema Donor", "", "Tests");
        let pin = donor.add_output_pin("value", "Value", "", VariableType::Struct);
        pin.schema = schema.map(str::to_string);
        donor
    }

    fn connect(extractor: &mut Node, donor: &Node) {
        let donor_id = donor.get_pin_by_name("value").unwrap().id.clone();
        extractor
            .get_pin_mut_by_name("struct_shape")
            .unwrap()
            .depends_on = BTreeSet::from([donor_id]);
    }

    #[tokio::test]
    async fn donor_schema_shapes_the_reference_and_response() {
        let logic = LLMExtractWithStructSchemaNode::new();
        let mut extractor = logic.get_node();
        let donor = donor(Some(PERSON_SCHEMA));
        connect(&mut extractor, &donor);

        let mut board = board();
        board.nodes.insert(donor.id.clone(), donor);
        logic.on_update(&mut extractor, &board).await;

        assert_eq!(extractor.error, None);
        for pin_name in ["struct_shape", "response"] {
            assert_eq!(
                extractor
                    .get_pin_by_name(pin_name)
                    .unwrap()
                    .schema
                    .as_deref(),
                Some(PERSON_SCHEMA)
            );
        }
    }

    #[tokio::test]
    async fn a_new_donor_replaces_the_previous_shape() {
        let logic = LLMExtractWithStructSchemaNode::new();
        let mut extractor = logic.get_node();
        let person = donor(Some(PERSON_SCHEMA));
        let order = donor(Some(ORDER_SCHEMA));
        connect(&mut extractor, &person);

        let mut board = board();
        board.nodes.insert(person.id.clone(), person);
        board.nodes.insert(order.id.clone(), order.clone());
        logic.on_update(&mut extractor, &board).await;

        connect(&mut extractor, &order);
        logic.on_update(&mut extractor, &board).await;

        for pin_name in ["struct_shape", "response"] {
            assert_eq!(
                extractor
                    .get_pin_by_name(pin_name)
                    .unwrap()
                    .schema
                    .as_deref(),
                Some(ORDER_SCHEMA)
            );
        }
    }

    #[tokio::test]
    async fn compact_schema_references_are_resolved() {
        let logic = LLMExtractWithStructSchemaNode::new();
        let mut extractor = logic.get_node();
        let donor = donor(Some("outer-ref"));
        connect(&mut extractor, &donor);

        let mut board = board();
        board.refs.insert("outer-ref".into(), "inner-ref".into());
        board.refs.insert("inner-ref".into(), PERSON_SCHEMA.into());
        board.nodes.insert(donor.id.clone(), donor);
        logic.on_update(&mut extractor, &board).await;

        assert_eq!(
            extractor
                .get_pin_by_name("response")
                .unwrap()
                .schema
                .as_deref(),
            Some(PERSON_SCHEMA)
        );
    }

    #[tokio::test]
    async fn a_schema_reference_cycle_is_reported() {
        let logic = LLMExtractWithStructSchemaNode::new();
        let mut extractor = logic.get_node();
        let donor = donor(Some("ref-a"));
        connect(&mut extractor, &donor);

        let mut board = board();
        board.refs.insert("ref-a".into(), "ref-b".into());
        board.refs.insert("ref-b".into(), "ref-a".into());
        board.nodes.insert(donor.id.clone(), donor);
        logic.on_update(&mut extractor, &board).await;

        assert!(
            extractor
                .error
                .as_deref()
                .is_some_and(|error| error.contains("Cyclic schema reference"))
        );
        assert!(
            extractor
                .get_pin_by_name("response")
                .unwrap()
                .schema
                .as_deref()
                .is_some_and(is_open_object_schema)
        );
    }

    #[tokio::test]
    async fn disconnecting_clears_the_stale_shape() {
        let logic = LLMExtractWithStructSchemaNode::new();
        let mut extractor = logic.get_node();
        let donor = donor(Some(PERSON_SCHEMA));
        connect(&mut extractor, &donor);

        let mut board = board();
        board.nodes.insert(donor.id.clone(), donor);
        logic.on_update(&mut extractor, &board).await;

        extractor
            .get_pin_mut_by_name("struct_shape")
            .unwrap()
            .depends_on
            .clear();
        logic.on_update(&mut extractor, &board).await;

        for pin_name in ["struct_shape", "response"] {
            let schema = extractor
                .get_pin_by_name(pin_name)
                .unwrap()
                .schema
                .as_deref()
                .unwrap();
            assert!(is_open_object_schema(schema), "{pin_name}: {schema}");
        }
        assert_eq!(extractor.error, None);
    }

    #[tokio::test]
    async fn an_open_donor_is_reported() {
        let logic = LLMExtractWithStructSchemaNode::new();
        let mut extractor = logic.get_node();
        let donor = donor(Some(flow_like::flow::pin::OPEN_OBJECT_SCHEMA));
        connect(&mut extractor, &donor);

        let mut board = board();
        board.nodes.insert(donor.id.clone(), donor);
        logic.on_update(&mut extractor, &board).await;

        assert!(
            extractor
                .error
                .as_deref()
                .is_some_and(|error| error.contains("concrete schema"))
        );
        assert!(
            extractor
                .get_pin_by_name("response")
                .unwrap()
                .schema
                .as_deref()
                .is_some_and(is_open_object_schema)
        );
    }

    #[tokio::test]
    async fn a_schema_less_donor_is_reported() {
        let logic = LLMExtractWithStructSchemaNode::new();
        let mut extractor = logic.get_node();
        let donor = donor(None);
        connect(&mut extractor, &donor);

        let mut board = board();
        board.nodes.insert(donor.id.clone(), donor);
        logic.on_update(&mut extractor, &board).await;

        assert!(
            extractor
                .error
                .as_deref()
                .is_some_and(|error| error.contains("has no schema"))
        );
        assert!(
            extractor
                .get_pin_by_name("response")
                .unwrap()
                .schema
                .as_deref()
                .is_some_and(is_open_object_schema)
        );
    }

    #[tokio::test]
    async fn a_non_object_schema_is_reported() {
        let logic = LLMExtractWithStructSchemaNode::new();
        let mut extractor = logic.get_node();
        let donor = donor(Some(r#"{"anyOf":[{"type":"string"},{"type":"integer"}]}"#));
        connect(&mut extractor, &donor);

        let mut board = board();
        board.nodes.insert(donor.id.clone(), donor);
        logic.on_update(&mut extractor, &board).await;

        assert!(
            extractor
                .error
                .as_deref()
                .is_some_and(|error| error.contains("object at its root"))
        );
        assert!(
            extractor
                .get_pin_by_name("response")
                .unwrap()
                .schema
                .as_deref()
                .is_some_and(is_open_object_schema)
        );
    }

    #[cfg(feature = "execute")]
    #[test]
    fn reference_schema_preparation_rejects_example_values() {
        assert!(prepare_reference_schema(PERSON_SCHEMA).is_ok());
        assert!(prepare_reference_schema(r#"{"name":"Ada"}"#).is_err());
        assert!(
            prepare_reference_schema(r#"{"anyOf":[{"type":"string"},{"type":"integer"}]}"#)
                .is_err()
        );
    }
}
