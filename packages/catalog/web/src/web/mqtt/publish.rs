#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::{LogLevel, context::ExecutionContext};

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
pub struct MqttPublishNode {}

impl MqttPublishNode {
    pub fn new() -> Self {
        MqttPublishNode {}
    }
}

#[async_trait]
impl NodeLogic for MqttPublishNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mqtt_publish",
            "MQTT Publish",
            "Publishes a message to an MQTT topic",
            "Web/MQTT",
        );
        node.add_icon("/flow/icons/web.svg");
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(7)
                .set_security(6)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Trigger the publish",
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
            "The MQTT topic to publish to",
            VariableType::String,
        );
        node.add_input_pin(
            "payload",
            "Payload",
            "The message content to publish",
            VariableType::String,
        );
        node.add_input_pin(
            "qos",
            "QoS",
            "Quality of Service level",
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
            "retain",
            "Retain",
            "Whether the broker should retain this message",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the message is published",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: MqttSession = context.evaluate_pin("session").await?;
        let topic: String = context.evaluate_pin("topic").await?;
        let payload: String = context.evaluate_pin("payload").await?;
        let qos_str: String = context.evaluate_pin("qos").await?;
        let retain: bool = context.evaluate_pin("retain").await?;

        let qos = match qos_str.as_str() {
            "AtLeastOnce" => MqttQoS::AtLeastOnce,
            "ExactlyOnce" => MqttQoS::ExactlyOnce,
            _ => MqttQoS::AtMostOnce,
        };

        let conn = super::get_mqtt_connection(context, &session.ref_id).await?;
        let client = conn.client.lock().await;

        client
            .publish(
                &topic,
                super::to_rumqttc_qos(&qos),
                retain,
                payload.as_bytes(),
            )
            .await
            .map_err(|e| {
                context.log_message(&format!("MQTT publish error: {}", e), LogLevel::Error);
                flow_like_types::anyhow!("MQTT publish failed: {}", e)
            })?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "MQTT requires the 'execute' feature"
        ))
    }
}
