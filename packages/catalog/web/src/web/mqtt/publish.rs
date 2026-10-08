#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;

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
        node.set_flowscript_name("mqtt", "publish");
        node.set_receiver("session");
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
            "payload_bytes",
            "Payload Bytes",
            "Optional raw message bytes. When connected, these take precedence over Payload.",
            VariableType::Byte,
        )
        .set_options(PinOptions::new().set_optional(true).build())
        .set_default_value(Some(json!(null)));
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
            "Fires after the message is written at QoS 0 or acknowledged by the broker at QoS 1 or 2",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: MqttSession = context.evaluate_pin("session").await?;
        let topic: String = context.evaluate_pin("topic").await?;
        let payload_bytes: Option<Vec<u8>> =
            if context.get_pin_by_name("payload_bytes").await.is_ok() {
                context.evaluate_pin("payload_bytes").await?
            } else {
                None
            };
        let payload = match payload_bytes {
            Some(bytes) => bytes,
            None => context
                .evaluate_pin::<String>("payload")
                .await?
                .into_bytes(),
        };
        let qos_str: String = context.evaluate_pin("qos").await?;
        let retain: bool = context.evaluate_pin("retain").await?;

        let qos = match qos_str.as_str() {
            "AtLeastOnce" => MqttQoS::AtLeastOnce,
            "ExactlyOnce" => MqttQoS::ExactlyOnce,
            _ => MqttQoS::AtMostOnce,
        };

        let conn = super::get_mqtt_connection(context, &session.ref_id).await?;
        conn.publish(topic, payload, super::to_rumqttc_qos(&qos), retain)
            .await?;

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

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::web::{
        mqtt::{CachedMqttConnection, MqttConfig, MqttConnection},
        test_support::{internal_node, test_context},
    };
    use bytes::BytesMut;
    use flow_like::flow::execution::ExecutionEnvironment;
    use rumqttc::{Packet, QoS};
    use std::{sync::Arc, time::Duration};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[tokio::test]
    async fn binary_input_and_existing_text_graphs_publish_the_exact_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config: MqttConfig = flow_like_types::json::from_value(json!({"host":"127.0.0.1","port":listener.local_addr().unwrap().port(),"client_id":"node-test"})).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = BytesMut::new();
            let mut expected = [
                vec![0, 255, 42],
                Vec::new(),
                b"text".to_vec(),
                b"legacy".to_vec(),
            ]
            .into_iter();
            loop {
                match Packet::read(&mut bytes, 2 * 1024 * 1024) {
                    Ok(Packet::Connect(_)) => {
                        stream.write_all(&[0x20, 2, 0, 0]).await.unwrap();
                    }
                    Ok(Packet::Publish(message)) => {
                        assert_eq!(message.qos, QoS::AtMostOnce);
                        assert_eq!(
                            message.payload.to_vec(),
                            expected.next().expect("Unexpected publish")
                        );
                    }
                    Ok(Packet::Disconnect) => {
                        assert!(expected.next().is_none());
                        break;
                    }
                    Ok(packet) => panic!("Unexpected MQTT packet: {packet:?}"),
                    Err(rumqttc::Error::InsufficientBytes(_)) => {
                        assert_ne!(stream.read_buf(&mut bytes).await.unwrap(), 0);
                    }
                    Err(error) => panic!("Invalid MQTT packet: {error}"),
                }
            }
        });
        let connection = MqttConnection::connect(&config, ExecutionEnvironment::Local, None)
            .await
            .unwrap();
        for (payload_bytes, text, legacy) in [
            (Some(vec![0_u8, 255, 42]), "ignored", false),
            (Some(Vec::new()), "ignored", false),
            (None, "text", false),
            (None, "legacy", true),
        ] {
            let logic = MqttPublishNode::new();
            let mut node = logic.get_node();
            if legacy {
                node.pins.retain(|_, pin| pin.name != "payload_bytes");
            }
            let mut context = test_context(internal_node(node), vec![]).await;
            context
                .set_cache(
                    "mqtt-test",
                    Arc::new(CachedMqttConnection {
                        connection: connection.clone(),
                    }),
                )
                .await;
            context
                .set_pin_value(
                    "session",
                    json!({"ref_id":"mqtt-test","client_id":"node-test"}),
                )
                .await
                .unwrap();
            context
                .set_pin_value("topic", json!("spBv1.0/group/NDATA/node"))
                .await
                .unwrap();
            context.set_pin_value("payload", json!(text)).await.unwrap();
            if let Some(bytes) = payload_bytes {
                context
                    .set_pin_value("payload_bytes", json!(bytes))
                    .await
                    .unwrap();
            }
            logic.run(&mut context).await.unwrap();
        }
        connection.disconnect().await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }
}
