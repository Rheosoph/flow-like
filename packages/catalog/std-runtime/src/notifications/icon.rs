use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::Node,
    pin::PinOptions,
    variable::VariableType,
};
use flow_like::flow_like_storage::object_store::ObjectStoreExt;
use flow_like_catalog_core::FlowPath;
use flow_like_types::{
    base64::{Engine, engine::general_purpose::STANDARD},
    images::{MAX_NOTIFICATION_IMAGE_BYTES, notification_thumbnail},
};
use futures::TryStreamExt;

const ICON_PIN_NAME: &str = "icon";
const ICON_PIN_FRIENDLY_NAME: &str = "Icon";
const ICON_PIN_DESCRIPTION: &str = "FlowPath to a notification icon image (optional)";

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
        && let Some(icon) = portable_icon(context, icon_path).await
    {
        return icon;
    }

    if let Ok(icon) = context.evaluate_pin::<String>(ICON_PIN_NAME).await {
        let icon = icon.trim();
        let named_icon = !icon.is_empty()
            && icon.len() <= 128
            && icon
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if named_icon
            || icon.starts_with("https://")
            || (icon.starts_with('/') && !icon.starts_with("//"))
        {
            return icon.to_string();
        }
        if !icon.is_empty() {
            match portable_legacy_icon(icon).await {
                Ok(icon) => return icon,
                Err(error) => context.log_message(
                    &format!(
                        "Failed to prepare notification image, using the default icon: {error}"
                    ),
                    LogLevel::Warn,
                ),
            }
        }
    }

    // Preserve absence so each notification surface can use the source app artwork.
    String::new()
}

async fn portable_legacy_icon(icon: &str) -> flow_like_types::Result<String> {
    if !icon.starts_with("data:image/") {
        flow_like_types::bail!(
            "Notification images must use a FlowPath, HTTPS URL or image data URL"
        );
    }
    if icon.len() > MAX_NOTIFICATION_IMAGE_BYTES * 4 / 3 + 128 {
        flow_like_types::bail!("Notification image exceeds 5 MiB");
    }
    let (_, data) = icon
        .split_once(',')
        .filter(|(header, _)| header.ends_with(";base64"))
        .ok_or_else(|| flow_like_types::anyhow!("Invalid notification image data URL"))?;
    let bytes = STANDARD.decode(data)?;
    let thumbnail = tokio::task::spawn_blocking(move || notification_thumbnail(&bytes)).await??;
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(thumbnail)
    ))
}

async fn portable_icon(context: &mut ExecutionContext, icon_path: FlowPath) -> Option<String> {
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

    let result: flow_like_types::Result<String> = async {
        let object = runtime.store.as_generic().get(&runtime.path).await?;
        if object.meta.size > MAX_NOTIFICATION_IMAGE_BYTES as u64 {
            return Err(flow_like_types::anyhow!("Notification image exceeds 5 MiB"));
        }
        let mut stream = object.into_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.try_next().await? {
            if bytes.len() + chunk.len() > MAX_NOTIFICATION_IMAGE_BYTES {
                return Err(flow_like_types::anyhow!("Notification image exceeds 5 MiB"));
            }
            bytes.extend_from_slice(&chunk);
        }
        let thumbnail =
            tokio::task::spawn_blocking(move || notification_thumbnail(&bytes)).await??;
        // The API stores this bounded image before building the push payload.
        // Local notification consumers can use the same bytes without cloud access.
        Ok(format!(
            "data:image/png;base64,{}",
            STANDARD.encode(thumbnail)
        ))
    }
    .await;

    match result {
        Ok(icon) => Some(icon),
        Err(error) => {
            context.log_message(
                &format!(
                    "Failed to prepare notification icon '{}', using the default icon: {error}",
                    icon_path.path
                ),
                LogLevel::Warn,
            );
            None
        }
    }
}
