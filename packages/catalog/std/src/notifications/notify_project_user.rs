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

/// Node to notify a specific user in the project by their sub (user ID).
/// Currently sends a local notification - remote user targeting requires API integration.
#[crate::register_node]
#[derive(Default)]
pub struct NotifyProjectUserNode {}

impl NotifyProjectUserNode {
    pub fn new() -> Self {
        NotifyProjectUserNode {}
    }
}

#[async_trait]
impl NodeLogic for NotifyProjectUserNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "notify_project_user",
            "Notify Project User",
            "Send a notification to a specific user in this project",
            "Notifications",
        );
        node.add_icon("/flow/icons/bell-ring.svg");

        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        node.add_input_pin(
            "user_sub",
            "User ID",
            "The user's sub/ID to notify (must be a member of this project)",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));

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

        let user_sub = context.evaluate_pin::<String>("user_sub").await?;
        let title = context.evaluate_pin::<String>("title").await?;
        let description = context.evaluate_pin::<String>("description").await?;
        let icon = context.evaluate_pin::<String>("icon").await?;
        let link = context.evaluate_pin::<String>("link").await?;

        if user_sub.is_empty() {
            context.log_message(
                "User ID is required for Notify Project User node",
                LogLevel::Error,
            );
            context
                .set_pin_value("success", flow_like_types::json::json!(false))
                .await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        // Build notification (no desktop since this is for another user)
        let mut notification = NotificationEvent::new(&title)
            .with_desktop(false)
            .with_target_user_sub(&user_sub)
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

        // For now, send as local notification - remote targeting would require API integration
        // TODO: When remote execution is available, call API to notify specific user
        context.log_message(
            &format!("Sending notification intended for user: {}", user_sub),
            LogLevel::Info,
        );

        // stream_response logs errors internally but always returns Ok
        context
            .stream_response("flow_notification", notification)
            .await?;
        context.log_message(
            &format!("Notification sent (target user: {})", user_sub),
            LogLevel::Info,
        );

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
