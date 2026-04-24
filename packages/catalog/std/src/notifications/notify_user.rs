use super::{
    icon::{add_notification_icon_pin, migrate_notification_icon_pin, resolve_notification_icon},
    persist::{PersistNotificationParams, build_notification_link, persist_notification},
};
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

/// Node to notify the user who executed the workflow.
/// Sends an InterCom notification event that can be displayed locally
/// and persists it via the backend API for push delivery.
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

        add_notification_icon_pin(&mut node);

        node.add_input_pin(
            "link",
            "Link",
            "Relative path for the notification link (e.g. /dashboard or /store?item=abc)",
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
        let icon = resolve_notification_icon(context).await;
        let link = context.evaluate_pin::<String>("link").await?;
        let show_desktop = context.evaluate_pin::<bool>("show_desktop").await?;

        // Build the resolved link with app_id + route query params
        let app_id = context
            .execution_cache
            .as_ref()
            .map(|c| c.app_id.as_str())
            .unwrap_or("");
        let resolved_link =
            build_notification_link(app_id, if link.is_empty() { None } else { Some(&link) });

        let mut notification = NotificationEvent::new(&title)
            .with_desktop(show_desktop)
            .with_source_run_id(context.run_id())
            .with_source_node_id(&context.id)
            .with_link(&resolved_link);

        if let Some(event_id) = context.event_id().await {
            notification = notification.with_event_id(&event_id);
        }
        if !description.is_empty() {
            notification = notification.with_description(&description);
        }
        notification = notification.with_icon(&icon);

        // Send notification via InterCom stream (local display / SSE forwarding)
        context
            .stream_response("flow_notification", notification)
            .await?;

        // Persist notification via backend API for push delivery
        match persist_notification(
            context,
            PersistNotificationParams {
                title,
                description: (!description.is_empty()).then_some(description),
                icon: Some(icon),
                link: Some(resolved_link),
                target_user_sub: None,
            },
        )
        .await
        {
            Ok(true) => context.log_message("Notification persisted via API", LogLevel::Debug),
            Ok(false) => {
                context.log_message("Notification sent locally (no hub/token)", LogLevel::Debug)
            }
            Err(e) => context.log_message(
                &format!("Failed to persist notification via API: {e}"),
                LogLevel::Warn,
            ),
        }

        context
            .set_pin_value("success", flow_like_types::json::json!(true))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        migrate_notification_icon_pin(node);
    }
}
