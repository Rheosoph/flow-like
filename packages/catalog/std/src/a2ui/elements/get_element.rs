use super::element_utils::{extract_element_id_from_pin, find_element};
use super::schema_utils::set_component_schema_by_type;
use flow_like::a2ui::A2UIElement;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

/// Gets an element's data from the workflow payload.
///
/// The element data should be included in the workflow payload when triggered from the UI.
/// The payload structure expected is:
/// ```json
/// {
///   "_elements": {
///     "element_id": { /* full component data */ }
///   }
/// }
/// ```
#[crate::register_node]
#[derive(Default)]
pub struct GetElement;

impl GetElement {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for GetElement {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_get_element",
            "Get Element",
            "Gets an element's data from the page",
            "UI/Elements",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the page element",
            VariableType::String,
        );

        node.add_output_pin(
            "element",
            "Element",
            "The element data",
            VariableType::Struct,
        )
        .set_schema::<A2UIElement>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the element exists",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id_from_pin(element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        context.log_message(
            &format!("[GetElement] Looking for element_id: {}", element_id),
            LogLevel::Debug,
        );

        let elements = context.get_frontend_elements().await?;
        let element = elements.as_ref().and_then(|e| find_element(e, &element_id));

        if let Some((found_id, element_value)) = element {
            // Create element with __element_id for use with setter nodes
            let mut enriched_element = element_value.clone();
            if let Some(obj) = enriched_element.as_object_mut() {
                obj.insert("__element_id".to_string(), Value::String(found_id.clone()));
            }

            let element_pin = context.get_pin_by_name("element").await?;
            element_pin.set_value(enriched_element).await;

            let exists_pin = context.get_pin_by_name("exists").await?;
            exists_pin.set_value(Value::Bool(true)).await;

            context.log_message(
                &format!("Got element: {} (found as {})", element_id, found_id),
                LogLevel::Debug,
            );
        } else {
            let element_pin = context.get_pin_by_name("element").await?;
            element_pin.set_value(Value::Null).await;

            let exists_pin = context.get_pin_by_name("exists").await?;
            exists_pin.set_value(Value::Bool(false)).await;

            context.log_message(
                &format!("Element not found: {}", element_id),
                LogLevel::Debug,
            );
        }

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        let read_only_node = node.clone();
        let element_ref = match read_only_node.get_pin_by_name("element_ref") {
            Some(pin) => pin,
            None => {
                node.error = Some("Element reference pin not found!".to_string());
                return;
            }
        };

        let element_id = element_ref.default_value.as_ref().and_then(|v| {
            let parsed: Value = flow_like_types::json::from_slice(v).ok()?;
            parsed.as_str().map(String::from)
        });

        if let Some(id) = &element_id {
            let short_name = id.rsplit('/').next().unwrap_or(id);
            let component_type = find_component_type_in_board(board, id).await;

            if let Some(comp_type) = &component_type {
                node.friendly_name = format!("Get {} ({})", short_name, comp_type);

                if let Some(element_pin) = node.get_pin_mut_by_name("element") {
                    set_component_schema_by_type(element_pin, comp_type);
                }
            } else {
                node.friendly_name = format!("Get {}", short_name);
            }
        } else {
            node.friendly_name = "Get Element".to_string();
        }
    }
}

async fn find_component_type_in_board(board: &Board, element_id: &str) -> Option<String> {
    let pages = board.load_all_pages(None).await.ok()?;
    for page in pages {
        for component in &page.components {
            if component.id == element_id
                || element_id.ends_with(&format!("/{}", component.id))
                || element_id == component.id.split('/').next_back().unwrap_or("")
            {
                return component.get_component_type_name().into();
            }
        }
    }
    None
}
