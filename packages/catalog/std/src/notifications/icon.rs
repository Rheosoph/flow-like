use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::Node,
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use std::time::Duration;

pub const DEFAULT_NOTIFICATION_ICON: &str = "/app-logo.webp";

const ICON_PIN_NAME: &str = "icon";
const ICON_PIN_FRIENDLY_NAME: &str = "Icon";
const ICON_PIN_DESCRIPTION: &str = "FlowPath to a notification icon image (optional)";
const ICON_SIGN_EXPIRATION_SECS: u64 = 7 * 24 * 60 * 60;

pub fn add_notification_icon_pin(node: &mut Node) {
    node.add_input_pin(
        ICON_PIN_NAME,
        ICON_PIN_FRIENDLY_NAME,
        ICON_PIN_DESCRIPTION,
        VariableType::Struct,
    )
    .set_schema::<FlowPath>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

pub fn migrate_notification_icon_pin(node: &mut Node) {
    if node.get_pin_by_name(ICON_PIN_NAME).is_none() {
        add_notification_icon_pin(node);
        return;
    }

    if let Some(pin) = node.get_pin_mut_by_name(ICON_PIN_NAME) {
        pin.friendly_name = ICON_PIN_FRIENDLY_NAME.to_string();
        pin.description = ICON_PIN_DESCRIPTION.to_string();
        pin.data_type = VariableType::Struct;
        pin.default_value = None;
        pin.set_schema::<FlowPath>();
        pin.set_options(PinOptions::new().set_enforce_schema(true).build());
    }
}

pub async fn resolve_notification_icon(context: &mut ExecutionContext) -> String {
    if let Ok(icon_path) = context.evaluate_pin::<FlowPath>(ICON_PIN_NAME).await
        && let Some(icon) = signed_icon_url(context, icon_path).await {
            return icon;
        }

    if let Ok(icon) = context.evaluate_pin::<String>(ICON_PIN_NAME).await {
        let icon = icon.trim();
        if !icon.is_empty() {
            return icon.to_string();
        }
    }

    DEFAULT_NOTIFICATION_ICON.to_string()
}

async fn signed_icon_url(context: &mut ExecutionContext, icon_path: FlowPath) -> Option<String> {
    if icon_path.path.trim().is_empty() {
        return None;
    }

    let runtime = match icon_path.to_runtime(context).await {
        Ok(runtime) => runtime,
        Err(error) => {
            context.log_message(
                &format!("Failed to resolve notification icon FlowPath: {error}"),
                LogLevel::Warn,
            );
            return None;
        }
    };

    if let Err(error) = runtime.store.as_generic().head(&runtime.path).await {
        context.log_message(
            &format!(
                "Notification icon file '{}' is unavailable, using the default icon: {error}",
                icon_path.path
            ),
            LogLevel::Warn,
        );
        return None;
    }

    match runtime
        .store
        .sign(
            "GET",
            &runtime.path,
            Duration::from_secs(ICON_SIGN_EXPIRATION_SECS),
        )
        .await
    {
        Ok(url) => Some(url.to_string()),
        Err(error) => {
            context.log_message(
                &format!(
                    "Failed to sign notification icon '{}', using the default icon: {error}",
                    icon_path.path
                ),
                LogLevel::Warn,
            );
            None
        }
    }
}
