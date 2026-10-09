#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::{
    LogLevel, context::ExecutionContext, internal_node::InternalNode, log::LogMessage,
};

use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use super::MqttQoS;
use super::MqttSession;

#[crate::register_node]
#[derive(Default)]
pub struct MqttSubscribeNode {}

impl MqttSubscribeNode {
    pub fn new() -> Self {
        MqttSubscribeNode {}
    }
}

#[async_trait]
impl NodeLogic for MqttSubscribeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mqtt_subscribe",
            "MQTT Subscribe",
            "Subscribes to an MQTT topic and invokes a handler for each incoming message. \
             Receives text or raw bytes through payload_bytes on the handler. Holds execution until the connection closes or timeout, then triggers on_close.",
            "Web/MQTT",
        );
        node.set_flowscript_name("mqtt", "subscribe");
        node.set_receiver("session");
        node.add_icon("/flow/icons/web.svg");
        node.set_long_running(true);
        node.set_can_reference_fns(true);
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(7)
                .set_security(6)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Start subscribing",
            VariableType::Execution,
        );
        node.add_input_pin(
            "session",
            "Session",
            "MQTT session reference",
            VariableType::Struct,
        )
        .set_schema::<MqttSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "topic",
            "Topic",
            "The MQTT topic filter to subscribe to",
            VariableType::String,
        );
        node.add_input_pin(
            "qos",
            "QoS",
            "Quality of Service level for the subscription",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "AtMostOnce".to_string(),
                    "AtLeastOnce".to_string(),
                    "ExactlyOnce".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("AtMostOnce")));
        node.add_input_pin(
            "timeout_seconds",
            "Timeout (s)",
            "How long to listen before auto-closing (0 = indefinite)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "on_subscribed",
            "On Subscribed",
            "Fires after the subscription is established",
            VariableType::Execution,
        );
        node.add_output_pin(
            "on_close",
            "On Close",
            "Fires when the subscription ends (timeout, disconnect, or error)",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Fires if the subscription fails",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use crate::web::message_handler::{
            IncomingPayload, create_message_handler_context, trigger_message_handler_checked,
        };
        context.deactivate_exec_pin("on_subscribed").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;
        let session: MqttSession = context.evaluate_pin("session").await?;
        let topic: String = context.evaluate_pin("topic").await?;
        let qos_str: String = context.evaluate_pin("qos").await?;
        let timeout: i64 = context.evaluate_pin("timeout_seconds").await?;
        if timeout < 0 {
            return Err(flow_like_types::anyhow!(
                "MQTT subscription timeout cannot be negative"
            ));
        }
        let referenced_fns = context.get_referenced_functions().await?;
        if referenced_fns.len() != 1 {
            return Err(flow_like_types::anyhow!(
                "Reference exactly one MQTT on-message handler"
            ));
        }
        let handler = create_message_handler_context(
            context,
            referenced_fns[0].clone(),
            &["topic", "payload_bytes", "qos", "retain"],
        )
        .await;
        let qos = match qos_str.as_str() {
            "AtLeastOnce" => MqttQoS::AtLeastOnce,
            "ExactlyOnce" => MqttQoS::ExactlyOnce,
            _ => MqttQoS::AtMostOnce,
        };
        let conn = super::get_mqtt_connection(context, &session.ref_id).await?;
        let mut messages = conn
            .subscribe(topic.clone(), super::to_rumqttc_qos(&qos))
            .await?;
        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("on_subscribed").await?;
        let on_sub_pin = context.get_pin_by_name("on_subscribed").await?;
        for node in on_sub_pin.get_connected_nodes() {
            let mut sub = context.create_sub_context(&node).await;
            sub.delegated = true;
            let mut message = LogMessage::new("MQTT on_subscribed", LogLevel::Debug, None);
            let result = InternalNode::trigger(&mut sub, &mut None, true).await;
            message.end();
            sub.log(message);
            sub.end_trace();
            context.push_sub_context(&mut sub);
            if let Err(error) = result {
                let _ = messages.close().await;
                return Err(flow_like_types::anyhow!(
                    "MQTT on_subscribed failed: {error:?}"
                ));
            }
        }
        context.deactivate_exec_pin("on_subscribed").await?;
        let deadline = async {
            if timeout == 0 {
                std::future::pending::<()>().await;
            } else {
                tokio::time::sleep(std::time::Duration::from_secs(timeout as u64)).await;
            }
        };
        tokio::pin!(deadline);
        let cancellation = context.get_cancellation_token();
        let result = loop {
            let message = tokio::select! {
                _ = crate::web::wait_for_cancel(cancellation.clone()) => break Ok(()),
                _ = &mut deadline => break Ok(()),
                closed = conn.closed() => break closed,
                message = messages.recv() => match message {
                    Ok(message) => message,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => break Err(flow_like_types::anyhow!("MQTT handler queue overflowed; {count} messages were lost")),
                    Err(_) => break Err(flow_like_types::anyhow!("MQTT message stream closed")),
                },
            };
            let bytes = message.payload.to_vec();
            let payload = match String::from_utf8(bytes.clone()) {
                Ok(text) => IncomingPayload::Text(text),
                Err(_) => IncomingPayload::Binary(bytes.clone()),
            };
            let metadata = [
                ("topic", json!(message.topic)),
                ("payload_bytes", json!(bytes)),
                ("qos", json!(message.qos as u8)),
                ("retain", json!(message.retain)),
            ];
            tokio::select! {
                _ = crate::web::wait_for_cancel(cancellation.clone()) => break Ok(()),
                _ = &mut deadline => break Ok(()),
                handled = trigger_message_handler_checked(&handler, payload, &metadata, "MQTT on_message") => { if let Err(error) = handled { break Err(error); } },
            }
        };
        let cleanup = messages.close().await;
        context.activate_exec_pin("on_close").await?;
        if result.is_err() {
            context.activate_exec_pin("exec_error").await?;
        }
        result?;
        if !cancellation.is_some_and(|token| token.is_cancelled()) {
            cleanup?;
        }
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "MQTT requires the 'execute' feature"
        ))
    }
}
