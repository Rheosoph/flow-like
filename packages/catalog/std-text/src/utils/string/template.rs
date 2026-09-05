use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, dynamic_pin_source_literal, remove_unwired_pins},
    pin::PinType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json, minijinja};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
#[crate::register_node]
#[doc = "Render jinja templates based on a template string and dynamic placeholder inputs."]
pub struct TemplateStringNode {}

impl TemplateStringNode {
    pub fn new() -> Self {
        TemplateStringNode {}
    }
}

#[async_trait]
impl NodeLogic for TemplateStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "string_render_template",
            "Render Template",
            "Template Engine based on Jinja Templates",
            "Utils/String",
        );
        node.set_flowscript_name("string", "renderTemplate");
        node.set_receiver("template");
        node.add_icon("/flow/icons/string.svg");

        // inputs
        node.add_input_pin(
            "template",
            "Template",
            "Jinja Template String",
            VariableType::String,
        );

        // outputs
        node.add_output_pin(
            "rendered",
            "Rendered",
            "Rendered String",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // load inputs & templates
        let template_string: String = context.evaluate_pin("template").await?;
        let mut jinja_env = minijinja::Environment::new();

        // collect placeholders & values
        jinja_env
            .add_template("template", &template_string)
            .unwrap();
        let template = jinja_env.get_template("template")?;
        let placeholders = template.undeclared_variables(false);
        context.log_message(
            &format!("extracted placeholders: {:?}", placeholders),
            LogLevel::Debug,
        );

        let mut template_context = HashMap::new();
        for placeholder in placeholders {
            let value: flow_like_types::Value = context.evaluate_pin(&placeholder).await?;
            template_context.insert(placeholder, value);
        }

        // render template
        let rendered = template.render(template_context)?;

        // set outputs
        context.set_pin_value("rendered", json!(rendered)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        // A wired, absent or non-string template says nothing about which variables the node will
        // actually be rendered with. Reading it as "no variables" would delete every derived pin
        // along with the wires feeding them.
        let Some(template_string) = dynamic_pin_source_literal(node, "template") else {
            return;
        };

        let pins: Vec<_> = node
            .pins
            .values()
            .filter(|p| p.name != "template" && p.pin_type == PinType::Input)
            .collect();

        let mut current_placeholders = pins
            .iter()
            .map(|p| (p.name.clone(), *p))
            .collect::<HashMap<_, _>>();

        let mut jinja_env = minijinja::Environment::new();
        let err = jinja_env.add_template("template", &template_string);

        if let Err(e) = err {
            println!(
                "Failed to parse template: {}. Error: {}",
                template_string, e
            );
            return;
        }

        let Ok(template) = jinja_env.get_template("template") else {
            println!("Failed to parse template: {}", template_string);
            return;
        };
        let template_placeholders = template.undeclared_variables(false);
        let mut all_placeholders = HashSet::new();
        let mut missing_placeholders = HashSet::new();

        for placeholder in template_placeholders {
            // `{{ template }}` would otherwise mint a second pin named `template` and then
            // `match_type` it, rewriting the type of the node's own template input.
            if placeholder == "template" {
                node.error = Some(
                    "`template` is the name of this node's own input and cannot be a template variable. Rename it."
                        .to_string(),
                );
                continue;
            }
            all_placeholders.insert(placeholder.clone());
            if current_placeholders.remove(&placeholder).is_none() {
                missing_placeholders.insert(placeholder.clone());
            }
        }

        let ids_to_remove = current_placeholders
            .values()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>();
        remove_unwired_pins(node, &ids_to_remove);

        for placeholder in missing_placeholders {
            node.add_input_pin(&placeholder, &placeholder, "", VariableType::Generic);
        }

        all_placeholders.iter().for_each(|placeholder| {
            let _ = node.match_type(placeholder, board, None, None);
        })
    }
}
