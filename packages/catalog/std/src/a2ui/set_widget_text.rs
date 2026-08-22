use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Set Widget Text - Sets the text of an element inside a widget instance.
///
/// Operates on the `element_ref` produced by Instantiate Widget: it rewrites the
/// embedded inline widget definition, so the change is baked into the instance
/// before it reaches the frontend (Push Widget / Push To Container). The updated
/// component is also re-registered on the surface channel so already-rendered
/// instances in the app view pick it up.
#[crate::register_node]
#[derive(Default)]
pub struct SetWidgetText;

impl SetWidgetText {
    pub fn new() -> Self {
        Self
    }
}

const TEXT_KEYS: [&str; 3] = ["content", "text", "label"];

fn set_text_on_component(component: &mut Value, text: &str) {
    let Some(obj) = component.as_object_mut() else {
        return;
    };
    let bound = json!({ "literalString": text });
    let existing: Vec<&str> = TEXT_KEYS
        .iter()
        .copied()
        .filter(|k| obj.contains_key(*k))
        .collect();
    if existing.is_empty() {
        obj.insert("content".to_string(), bound);
        return;
    }
    for key in existing {
        obj.insert(key.to_string(), bound.clone());
    }
}

#[async_trait]
impl NodeLogic for SetWidgetText {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_widget_set_text",
            "Set Widget Text",
            "Sets the text of an element inside a widget instance (from Instantiate Widget) before it is pushed to the frontend",
            "UI/Container",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Widget",
            "Widget instance reference (from Instantiate Widget)",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::ElementRef>();

        node.add_input_pin(
            "element_id",
            "Element ID",
            "ID of the element inside the widget (e.g. 'title-text')",
            VariableType::String,
        );

        node.add_input_pin("text", "Text", "The text to set", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "element_ref_out",
            "Widget",
            "The updated widget instance reference (connect to Push Widget / Push To Container)",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::ElementRef>();

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut element_ref: Value = context.evaluate_pin("element_ref").await?;
        let element_id: String = context.evaluate_pin("element_id").await?;
        let text: String = context.evaluate_pin("text").await?;

        let instance_id = element_ref
            .get("instanceId")
            .or_else(|| element_ref.get("id"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                flow_like_types::anyhow!("Invalid widget reference: missing 'instanceId'")
            })?
            .to_string();

        let component = element_ref.get_mut("component").ok_or_else(|| {
            flow_like_types::anyhow!(
                "Widget reference is missing 'component'. Re-add the Instantiate Widget node (requires version 4 or newer)."
            )
        })?;

        let components = component
            .get_mut("inlineWidgetDef")
            .and_then(|def| def.get_mut("components"))
            .and_then(|c| c.as_array_mut())
            .ok_or_else(|| {
                flow_like_types::anyhow!("Widget reference has no inline widget definition")
            })?;

        // Exact id first — a suffix hit must never shadow an exact-id child.
        let suffix = format!("-{element_id}");
        let index = components
            .iter()
            .position(|c| c.get("id").and_then(|id| id.as_str()) == Some(element_id.as_str()))
            .or_else(|| {
                components.iter().position(|c| {
                    c.get("id")
                        .and_then(|id| id.as_str())
                        .is_some_and(|id| id.ends_with(&suffix))
                })
            })
            .ok_or_else(|| {
                let available: Vec<String> = components
                    .iter()
                    .filter_map(|c| c.get("id").and_then(|id| id.as_str()))
                    .map(str::to_string)
                    .collect();
                flow_like_types::anyhow!(
                    "Element '{}' not found in widget. Available elements: [{}]",
                    element_id,
                    available.join(", ")
                )
            })?;

        if let Some(inner) = components[index].get_mut("component") {
            set_text_on_component(inner, &text);
        }

        // Re-register the updated widgetInstance so instances already rendered
        // on a surface (app view via Push To Container) pick up the change too.
        context
            .upsert_element(
                &instance_id,
                json!({
                    "type": "createComponent",
                    "component": element_ref.get("component").cloned().unwrap_or(Value::Null)
                }),
            )
            .await?;

        context
            .get_pin_by_name("element_ref_out")
            .await?
            .set_value(element_ref)
            .await;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
