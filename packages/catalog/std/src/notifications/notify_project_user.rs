use super::{
    icon::{add_notification_icon_pin, migrate_notification_icon_pin, resolve_notification_icon},
    persist::{PersistNotificationParams, build_notification_link, persist_notification},
};
use flow_like::{
    flow::{
        board::Board,
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic, remove_pin_by_name},
        variable::VariableType,
    },
    state::NotificationEvent,
};
use flow_like_types::async_trait;

const USER_SUB_PIN_NAME: &str = "_flow_user_sub";
const USER_SUB_PIN_FRIENDLY_NAME: &str = "User";
const USER_SUB_PIN_DESCRIPTION: &str = "Project user to notify";

/// Node to notify a specific user in the project by their sub (user ID).
/// Persists the notification via the backend API for push delivery.
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
            USER_SUB_PIN_NAME,
            USER_SUB_PIN_FRIENDLY_NAME,
            USER_SUB_PIN_DESCRIPTION,
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

        add_notification_icon_pin(&mut node);

        node.add_input_pin(
            "link",
            "Link",
            "Relative path for the notification link (e.g. /dashboard or /store?item=abc)",
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

        let user_sub = context.evaluate_pin::<String>(USER_SUB_PIN_NAME).await?;
        let title = context.evaluate_pin::<String>("title").await?;
        let description = context.evaluate_pin::<String>("description").await?;
        let icon = resolve_notification_icon(context).await;
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

        // Build the resolved link with app_id + route query params
        let app_id = context
            .execution_cache
            .as_ref()
            .map(|c| c.app_id.as_str())
            .unwrap_or("");
        let resolved_link =
            build_notification_link(app_id, if link.is_empty() { None } else { Some(&link) });

        let mut notification = NotificationEvent::new(&title)
            .with_desktop(false)
            .with_target_user_sub(&user_sub)
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
                target_user_sub: Some(user_sub.clone()),
            },
        )
        .await
        {
            Ok(true) => context.log_message(
                &format!("Notification persisted via API (target user: {user_sub})"),
                LogLevel::Debug,
            ),
            Ok(false) => context.log_message(
                &format!(
                    "Notification sent locally for user: {user_sub} (no hub/token or offline)"
                ),
                LogLevel::Debug,
            ),
            Err(e) => context.log_message(
                &format!("Failed to persist notification for user {user_sub} via API: {e}"),
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
        if node.get_pin_by_name(USER_SUB_PIN_NAME).is_some() {
            remove_pin_by_name(node, "user_sub");
        } else if let Some(pin) = node.get_pin_mut_by_name("user_sub") {
            pin.name = USER_SUB_PIN_NAME.to_string();
        }

        if let Some(pin) = node.get_pin_mut_by_name(USER_SUB_PIN_NAME) {
            pin.friendly_name = USER_SUB_PIN_FRIENDLY_NAME.to_string();
            pin.description = USER_SUB_PIN_DESCRIPTION.to_string();
        }

        migrate_notification_icon_pin(node);
    }
}
