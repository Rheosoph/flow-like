use super::{FrontendToolApproval, NativePlatformSession, native_tool_bridge};
use flow_like_types::{
    channel::{
        ChannelClientDescriptor, ChannelHandle, ChannelPush, ChannelPushKind, InProcessChannel,
        InProcessPushResult,
    },
    interaction::{InteractionRequest, InteractionStatus},
};
use serde_json::{Value, json};
use std::time::Duration;

fn validate_request<'a>(
    app: &str,
    request: &'a InteractionRequest,
    now: i64,
) -> Result<&'a ChannelHandle, String> {
    let handle = request
        .channel
        .as_ref()
        .ok_or("The app interaction has no reply channel.")?;
    if request.app_id.as_deref() != Some(app)
        || request.run_id.as_deref() != Some(handle.channel_id.as_str())
        || handle.request_id.as_deref() != Some(request.id.as_str())
        || request.id.is_empty()
    {
        return Err(
            "The app interaction's reply channel does not match its app, run, and request.".into(),
        );
    }
    if !matches!(request.status, InteractionStatus::Pending)
        || handle.expires_at <= now
        || request.expires_at <= now.max(0) as u64
    {
        return Err("The app interaction is no longer pending.".into());
    }
    Ok(handle)
}

fn reply_url(hub: &str, channel: &str, supplied: &str) -> Result<reqwest::Url, String> {
    let origin = reqwest::Url::parse(hub).map_err(|_| "The app's hub is invalid.")?;
    let url =
        reqwest::Url::parse(supplied).map_err(|_| "The app interaction reply URL is invalid.")?;
    let expected = format!("/api/v1/channels/{}/push", super::segment(channel));
    if url.origin() != origin.origin()
        || url.path() != expected
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "The app interaction reply URL does not belong to this run's hub and channel.".into(),
        );
    }
    Ok(url)
}

fn unavailable_push(handle: &ChannelHandle) -> ChannelPush {
    ChannelPush {
        channel_id: handle.channel_id.clone(),
        request_id: None,
        kind: ChannelPushKind::Cancel,
        value: native_tool_bridge::unavailable_result("app_chat_interaction"),
    }
}

/// Stop only the app workflow awaiting input. The owning FlowPilot turn can continue.
pub(crate) async fn cancel_unavailable(
    session: &NativePlatformSession,
    app: &str,
    value: &Value,
) -> Result<(), String> {
    if value.to_string().len() > 128 * 1024 {
        return Err("The app interaction exceeds the native request size limit.".into());
    }
    let request: InteractionRequest = serde_json::from_value(value.clone())
        .map_err(|_| "The app returned an invalid interaction request.")?;
    let handle = validate_request(app, &request, chrono::Utc::now().timestamp())?;
    session.assert_current_profile().await?;
    deliver(session, handle, unavailable_push(handle)).await
}

pub(super) async fn answer(
    session: &NativePlatformSession,
    app: &str,
    value: &Value,
) -> Result<bool, String> {
    if value.to_string().len() > 128 * 1024 {
        return Err("The app interaction exceeds the native request size limit.".into());
    }
    let request: InteractionRequest = serde_json::from_value(value.clone())
        .map_err(|_| "The app returned an invalid interaction request.")?;
    let handle = validate_request(app, &request, chrono::Utc::now().timestamp())?;
    session.assert_current_profile().await?;
    let mut visible = value.clone();
    if let Some(fields) = visible.as_object_mut() {
        fields.remove("channel");
    }
    let lifetime = (handle.expires_at - chrono::Utc::now().timestamp()).max(0) as u64;
    let response = tokio::time::timeout(
        Duration::from_secs(lifetime.min(600)),
        native_tool_bridge::request_interaction(
            &session.app_handle,
            session.context.as_ref(),
            "app_chat_interaction",
            json!({"app_id":app,"interaction":visible}),
            FrontendToolApproval {
                kind: "interaction".into(),
                title: request.name.clone(),
                description: request.description.clone(),
                session_key: request.id.clone(),
            },
        ),
    )
    .await
    .map_err(|_| "The app interaction expired while awaiting an answer.")??;
    if !response.approved || response.error.is_some() {
        return Ok(false);
    }
    let response = response
        .result
        .and_then(|result| result.get("value").cloned())
        .ok_or("The app interaction response must contain its answer in 'value'.")?;
    if response.to_string().len() > 1024 * 1024 {
        return Err("The app interaction answer exceeds the native response size limit.".into());
    }
    validate_request(app, &request, chrono::Utc::now().timestamp())?;
    session.assert_current_profile().await?;
    let push = ChannelPush {
        channel_id: handle.channel_id.clone(),
        request_id: Some(request.id.clone()),
        kind: ChannelPushKind::Reply,
        value: response,
    };
    deliver(session, handle, push).await?;
    Ok(true)
}

async fn deliver(
    session: &NativePlatformSession,
    handle: &ChannelHandle,
    push: ChannelPush,
) -> Result<(), String> {
    if matches!(handle.transport, ChannelClientDescriptor::InProcess {}) {
        return match InProcessChannel::deliver(push).await {
            InProcessPushResult::Delivered => Ok(()),
            _ => Err("The app interaction no longer accepts an answer.".into()),
        };
    }
    // Cloud transports publish an authenticated HTTP fallback for clients without that SDK.
    let descriptor = if matches!(handle.transport, ChannelClientDescriptor::Http { .. }) {
        Some(&handle.transport)
    } else {
        handle.fallback.as_ref()
    };
    let Some(ChannelClientDescriptor::Http { push_url, token }) = descriptor else {
        return Err(
            "The app interaction has no supported reply transport or HTTP fallback.".into(),
        );
    };
    let url = reply_url(&session.hub, &handle.channel_id, push_url)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "The app interaction transport could not be initialized.")?;
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&push)
        .send()
        .await
        .map_err(|_| "The app interaction reply could not be delivered.")?;
    if !response.status().is_success() {
        return Err(format!(
            "The app interaction reply was refused (HTTP {}).",
            response.status().as_u16()
        ));
    }
    session.assert_current_profile().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::channel::Channel;

    #[tokio::test]
    async fn unavailable_cancels_the_app_channel_without_answering_or_cancelling_its_parent() {
        let app = InProcessChannel::register("unavailable-app-run", Duration::from_secs(60)).await;
        let parent =
            InProcessChannel::register("unavailable-flowpilot-run", Duration::from_secs(60)).await;
        let ticket = app.open(Duration::from_secs(60)).await.unwrap();
        let push = unavailable_push(&ticket.handle);
        assert!(matches!(push.kind, ChannelPushKind::Cancel));
        assert!(push.request_id.is_none());
        assert_eq!(push.value["status"], "unavailable");
        assert_eq!(push.value["approved"], false);
        assert!(push.value.get("native_approved").is_none());
        assert_eq!(
            InProcessChannel::deliver(push).await,
            InProcessPushResult::Delivered
        );
        assert!(app.is_cancelled().await);
        assert!(!parent.is_cancelled().await);
        assert!(matches!(
            app.wait(&ticket, None).await.unwrap(),
            flow_like_types::channel::ChannelOutcome::Cancelled
        ));
    }

    #[test]
    fn interaction_reply_cannot_send_a_capability_to_another_endpoint() {
        let hub = "https://hub.example";
        assert!(reply_url(hub, "run", "https://hub.example/api/v1/channels/run/push").is_ok());
        for url in [
            "https://other.example/api/v1/channels/run/push",
            "https://hub.example/api/v1/channels/other/push",
            "https://user@hub.example/api/v1/channels/run/push",
            "https://hub.example/api/v1/channels/run/push?redirect=other",
            "http://hub.example/api/v1/channels/run/push",
        ] {
            assert!(reply_url(hub, "run", url).is_err());
        }
    }
}
