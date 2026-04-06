use flow_like::{
    flow::{
        board::Board,
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic},
        variable::VariableType,
    },
    state::NotificationEvent,
};
use flow_like_types::async_trait;
use std::sync::Arc;

/// Node to notify the user who executed the workflow.
/// Sends an InterCom notification event that can be displayed locally.
#[crate::register_node]
#[derive(Default)]
pub struct NotifyUserNode {}

impl NotifyUserNode {
    pub fn new() -> Self {
        NotifyUserNode {}
    }
}

#[async_trait]
impl NodeLogic for NotifyUserNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "notify_user",
            "Notify User",
            "Send a notification to the user who executed this workflow",
            "Notifications",
        );
        node.add_icon("/flow/icons/bell.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin("title", "Title", "Notification title", VariableType::String)
            .set_default_value(Some(flow_like_types::json::json!("Notification")));

        node.add_input_pin(
            "description",
            "Description",
            "Notification description (optional)",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));

        node.add_input_pin(
            "icon",
            "Icon",
            "Icon URL or path (optional)",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));

        node.add_input_pin(
            "link",
            "Link",
            "Link to navigate to when clicked (optional, e.g., /use?project=xyz)",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));

        node.add_input_pin(
            "show_desktop",
            "Desktop Notification",
            "Show desktop notification if available",
            VariableType::Boolean,
        )
        .set_default_value(Some(flow_like_types::json::json!(true)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Continue execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the notification was sent successfully",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let title = context.evaluate_pin::<String>("title").await?;
        let description = context.evaluate_pin::<String>("description").await?;
        let icon = context.evaluate_pin::<String>("icon").await?;
        let link = context.evaluate_pin::<String>("link").await?;
        let show_desktop = context.evaluate_pin::<bool>("show_desktop").await?;

        // Build notification event
        let mut notification = NotificationEvent::new(&title)
            .with_desktop(show_desktop)
            .with_source_run_id(context.run_id())
            .with_source_node_id(&context.id);

        if let Some(event_id) = context.event_id().await {
            notification = notification.with_event_id(&event_id);
        }

        if !description.is_empty() {
            notification = notification.with_description(&description);
        }
        if !icon.is_empty() {
            notification = notification.with_icon(&icon);
        }
        if !link.is_empty() {
            notification = notification.with_link(&link);
        }

        // Send notification via InterCom stream
        context
            .stream_response("flow_notification", notification)
            .await?;
        context.log_message("Notification sent", LogLevel::Info);

        context
            .set_pin_value("success", flow_like_types::json::json!(true))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {
        // No type matching needed
    }
}
