#[cfg(feature = "execute")]
use ahash::AHashSet;
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
use std::sync::Arc;

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
             Holds execution until the connection closes or timeout, then triggers on_close.",
            "Web/MQTT",
        );
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
        use flow_like::flow::pin::PinType;
        use flow_like_types::sync::{DashMap, Mutex};

        context.deactivate_exec_pin("on_subscribed").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;

        let session: MqttSession = context.evaluate_pin("session").await?;
        let topic: String = context.evaluate_pin("topic").await?;
        let qos_str: String = context.evaluate_pin("qos").await?;
        let referenced_fns = context.get_referenced_functions().await?;
        let handler = referenced_fns
            .first()
            .ok_or_else(|| flow_like_types::anyhow!("No on-message handler function referenced"))?
            .clone();
        let timeout: i64 = context.evaluate_pin("timeout_seconds").await?;

        let qos = match qos_str.as_str() {
            "AtLeastOnce" => MqttQoS::AtLeastOnce,
            "ExactlyOnce" => MqttQoS::ExactlyOnce,
            _ => MqttQoS::AtMostOnce,
        };

        let conn = super::get_mqtt_connection(context, &session.ref_id).await?;

        {
            let client = conn.client.lock().await;
            client
                .subscribe(&topic, super::to_rumqttc_qos(&qos))
                .await
                .map_err(|e| {
                    context.log_message(&format!("MQTT subscribe error: {}", e), LogLevel::Error);
                    flow_like_types::anyhow!("MQTT subscribe failed: {}", e)
                })?;
        }

        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("on_subscribed").await?;

        let on_sub_pin = context.get_pin_by_name("on_subscribed").await?;
        let connected_on_sub = on_sub_pin.get_connected_nodes();
        for node in connected_on_sub {
            let mut sub = context.create_sub_context(&node).await;
            sub.delegated = true;
            let mut message = LogMessage::new("MQTT on_subscribed", LogLevel::Debug, None);
            let _ = InternalNode::trigger(&mut sub, &mut None, true).await;
            message.end();
            sub.log(message);
            sub.end_trace();
            context.push_sub_context(&mut sub);
        }

        let reference_function = &handler;

        let ref_node_pins = reference_function.pins.clone();
        let mut has_string_pin = false;
        let mut has_byte_pin = false;
        let mut has_payload_pin = false;
        let mut has_topic_pin = false;
        let mut typed_pin_count: usize = 0;

        for (_, pin) in ref_node_pins.iter() {
            if pin.pin_type != PinType::Output || pin.data_type == VariableType::Execution {
                continue;
            }
            if pin.name == "payload" {
                has_payload_pin = true;
                continue;
            }
            if pin.name == "topic" {
                has_topic_pin = true;
                continue;
            }
            typed_pin_count += 1;
            match pin.data_type {
                VariableType::String => has_string_pin = true,
                VariableType::Byte => has_byte_pin = true,
                _ => {}
            }
        }

        let single_string = typed_pin_count == 1 && has_string_pin;
        let single_byte = typed_pin_count == 1 && has_byte_pin;
        let only_payload = typed_pin_count == 0 && has_payload_pin;

        let connected_nodes: Arc<DashMap<String, Arc<Mutex<ExecutionContext>>>> =
            Arc::new(DashMap::new());

        let on_message_node = reference_function.clone();
        let sub = Arc::new(Mutex::new(
            context.create_sub_context(&on_message_node).await,
        ));
        connected_nodes.insert(on_message_node.node.lock().await.id.clone(), sub);

        let parent_node_id = context.node.node.lock().await.id.clone();
        let close_notify = conn.close_notify.clone();
        let event_loop = conn.event_loop.clone();
        let close_notify_spawn = close_notify.clone();

        let handle = tokio::spawn(async move {
            loop {
                let event = {
                    let mut el = event_loop.lock().await;
                    el.poll().await
                };

                let event = match event {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!("MQTT event loop error: {}", e);
                        break;
                    }
                };

                let publish = match event {
                    rumqttc::Event::Incoming(rumqttc::Packet::Publish(p)) => p,
                    _ => continue,
                };

                let payload_bytes = publish.payload.to_vec();
                let msg_topic = publish.topic.clone();
                let text = String::from_utf8_lossy(&payload_bytes);

                let mut recursion_guard = AHashSet::new();
                recursion_guard.insert(parent_node_id.clone());

                for entry in connected_nodes.iter() {
                    let (_id, ctx) = entry.pair();
                    let mut ctx = ctx.lock().await;

                    if has_topic_pin {
                        let topic_pins: Vec<_> = ctx
                            .node
                            .pins
                            .iter()
                            .filter(|(_, p)| p.pin_type == PinType::Output && p.name == "topic")
                            .map(|(_, p)| p.clone())
                            .collect();
                        for pin in topic_pins {
                            pin.set_value(json!(msg_topic.as_str())).await;
                        }
                    }

                    if single_string {
                        let pins = &ctx.node.pins;
                        for (_, pin) in pins.iter() {
                            if pin.pin_type == PinType::Output
                                && pin.data_type == VariableType::String
                                && pin.name != "payload"
                                && pin.name != "topic"
                            {
                                pin.set_value(json!(text.as_ref())).await;
                                break;
                            }
                        }
                    } else if single_byte {
                        let pins: Vec<_> = ctx
                            .node
                            .pins
                            .iter()
                            .filter(|(_, p)| {
                                p.pin_type == PinType::Output
                                    && p.data_type == VariableType::Byte
                                    && p.name != "payload"
                                    && p.name != "topic"
                            })
                            .map(|(_, p)| p.clone())
                            .collect();
                        for pin in pins {
                            pin.set_value(json!(payload_bytes)).await;
                            break;
                        }
                    } else if only_payload {
                        let parsed =
                            flow_like_types::json::from_str::<flow_like_types::Value>(&text);
                        let value = parsed.unwrap_or_else(|_| json!(text.as_ref()));
                        let payload_pins: Vec<_> = ctx
                            .node
                            .pins
                            .iter()
                            .filter(|(_, p)| p.pin_type == PinType::Output && p.name == "payload")
                            .map(|(_, p)| p.clone())
                            .collect();
                        for pin in payload_pins {
                            pin.set_value(value.clone()).await;
                        }
                    } else if let Ok(parsed) =
                        flow_like_types::json::from_str::<flow_like_types::Value>(&text)
                    {
                        if let Some(obj) = parsed.as_object() {
                            let mut remaining = obj.clone();
                            let pins: Vec<_> = ctx
                                .node
                                .pins
                                .iter()
                                .filter(|(_, p)| {
                                    p.pin_type == PinType::Output
                                        && p.data_type != VariableType::Execution
                                        && p.name != "payload"
                                        && p.name != "topic"
                                })
                                .map(|(_, p)| (p.name.clone(), p.clone()))
                                .collect();

                            for (name, pin) in &pins {
                                if let Some(val) = remaining.remove(name) {
                                    pin.set_value(val).await;
                                } else {
                                    let normalized = name.to_lowercase().replace('_', "");
                                    let key = remaining
                                        .keys()
                                        .find(|k| k.to_lowercase().replace('_', "") == normalized)
                                        .cloned();
                                    if let Some(k) = key
                                        && let Some(val) = remaining.remove(&k)
                                    {
                                        pin.set_value(val).await;
                                    }
                                }
                            }
                            let payload_pins: Vec<_> = ctx
                                .node
                                .pins
                                .iter()
                                .filter(|(_, p)| {
                                    p.pin_type == PinType::Output && p.name == "payload"
                                })
                                .map(|(_, p)| p.clone())
                                .collect();
                            for pin in payload_pins {
                                pin.set_value(json!(remaining)).await;
                            }
                        } else {
                            let payload_pins: Vec<_> = ctx
                                .node
                                .pins
                                .iter()
                                .filter(|(_, p)| {
                                    p.pin_type == PinType::Output && p.name == "payload"
                                })
                                .map(|(_, p)| p.clone())
                                .collect();
                            for pin in payload_pins {
                                pin.set_value(parsed.clone()).await;
                            }
                        }
                    } else {
                        let payload_pins: Vec<_> = ctx
                            .node
                            .pins
                            .iter()
                            .filter(|(_, p)| p.pin_type == PinType::Output && p.name == "payload")
                            .map(|(_, p)| p.clone())
                            .collect();
                        for pin in payload_pins {
                            pin.set_value(json!(text.as_ref())).await;
                        }
                    }

                    let mut log_message = LogMessage::new("MQTT on_message", LogLevel::Debug, None);
                    let run =
                        InternalNode::trigger(&mut ctx, &mut Some(recursion_guard.clone()), true)
                            .await;
                    log_message.end();
                    ctx.log(log_message);
                    ctx.end_trace();
                    if let Err(e) = run {
                        tracing::warn!("MQTT on_message handler error: {:?}", e);
                    }
                }
            }

            close_notify_spawn.notify_waiters();
        });

        let timeout = timeout as u64;
        if timeout > 0 {
            tokio::select! {
                _ = close_notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                    context.log_message("MQTT subscription timed out", LogLevel::Warn);
                    let client = conn.client.lock().await;
                    let _ = client.disconnect().await;
                }
            }
        } else {
            close_notify.notified().await;
        }

        handle.abort();

        {
            let mut cache = context.cache.write().await;
            cache.remove(&session.ref_id);
        }

        context.deactivate_exec_pin("on_subscribed").await?;
        context.activate_exec_pin("on_close").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "MQTT requires the 'execute' feature"
        ))
    }
}
