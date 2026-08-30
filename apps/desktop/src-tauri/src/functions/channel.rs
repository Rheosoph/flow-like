use flow_like_types::channel::{ChannelPush, InProcessChannel, InProcessPushResult};

use crate::functions::TauriFunctionError;

pub fn push_result_label(result: InProcessPushResult) -> &'static str {
    match result {
        InProcessPushResult::Delivered => "delivered",
        InProcessPushResult::UnknownChannel => "unknown_channel",
        InProcessPushResult::UnknownRequest => "unknown_request",
        InProcessPushResult::Expired => "expired",
        InProcessPushResult::Duplicate => "duplicate",
        InProcessPushResult::Full => "full",
    }
}

/// Deliver a client push (reply, unsolicited inbound message, or cancel) to the in-process
/// channel it addresses. Every desktop waiter — flow runs, interaction nodes, widget queries and
/// the FlowPilot frontend tool bridge — registers an `InProcessChannel` under its run id, so this
/// single command replaces the per-mechanism respond commands.
#[tauri::command(async)]
pub async fn channel_push(push: ChannelPush) -> Result<String, TauriFunctionError> {
    Ok(push_result_label(InProcessChannel::deliver(push).await).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::Value;
    use flow_like_types::channel::{Channel, ChannelOutcome, ChannelPushKind};
    use std::time::Duration;

    #[test]
    fn every_push_result_has_a_snake_case_label() {
        let expected = [
            (InProcessPushResult::Delivered, "delivered"),
            (InProcessPushResult::UnknownChannel, "unknown_channel"),
            (InProcessPushResult::UnknownRequest, "unknown_request"),
            (InProcessPushResult::Expired, "expired"),
            (InProcessPushResult::Duplicate, "duplicate"),
            (InProcessPushResult::Full, "full"),
        ];
        for (result, label) in expected {
            assert_eq!(push_result_label(result), label);
        }
    }

    #[tokio::test]
    async fn command_resolves_a_waiting_ticket_and_reports_late_pushes() {
        let channel =
            InProcessChannel::register("desktop-channel-test", Duration::from_secs(60)).await;
        let ticket = channel.open(Duration::from_secs(5)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;

        let push = ChannelPush {
            channel_id: "desktop-channel-test".to_string(),
            request_id: Some(ticket.request_id.clone()),
            kind: ChannelPushKind::Reply,
            value: Value::from("answer"),
        };
        assert_eq!(channel_push(push.clone()).await.unwrap(), "delivered");
        assert_eq!(
            waiter.await.unwrap(),
            ChannelOutcome::Responded(Value::from("answer"))
        );
        assert_eq!(channel_push(push).await.unwrap(), "unknown_request");

        let unknown = ChannelPush {
            channel_id: "no-such-channel".to_string(),
            request_id: None,
            kind: ChannelPushKind::Cancel,
            value: Value::Null,
        };
        assert_eq!(channel_push(unknown).await.unwrap(), "unknown_channel");
        channel.close().await;
    }
}
