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
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use flow_like_types::json::json;

#[cfg(feature = "execute")]
use bytes::BytesMut;
#[cfg(feature = "execute")]
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
#[cfg(feature = "execute")]
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Mutex, Notify, mpsc},
    task::JoinHandle,
};

use super::MqttBrokerConfig;

#[cfg(feature = "execute")]
use crate::web::message_handler::{
    IncomingPayload, MessageHandlerContext, create_message_handler_context, trigger_message_handler,
};
#[cfg(feature = "execute")]
use rumqttc::{
    ConnAck, ConnectReturnCode, Packet, PubAck, Publish, QoS, SubAck, SubscribeReasonCode,
};

#[cfg(feature = "execute")]
type SubscriberMap = Arc<Mutex<HashMap<String, Vec<mpsc::UnboundedSender<Packet>>>>>;

#[cfg(feature = "execute")]
struct MqttClientTask {
    close_notify: Arc<Notify>,
    reader_handle: JoinHandle<()>,
    writer_handle: JoinHandle<()>,
}

#[crate::register_node]
#[derive(Default)]
pub struct MqttBrokerNode {}

impl MqttBrokerNode {
    pub fn new() -> Self {
        MqttBrokerNode {}
    }
}

#[async_trait]
impl NodeLogic for MqttBrokerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "mqtt_broker",
            "MQTT Broker",
            "Binds a lightweight MQTT broker for daemon workflows. Typed lifecycle events are exposed as pins; published messages are delivered to the referenced on-message handler.",
            "Web/MQTT",
        );
        node.add_icon("/flow/icons/web.svg");
        node.set_long_running(true);
        node.set_can_reference_fns(true);
        node.scores = Some(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(6)
                .set_governance(6)
                .set_reliability(6)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Start the MQTT broker",
            VariableType::Execution,
        );
        node.add_input_pin(
            "config",
            "Config",
            "MQTT broker configuration",
            VariableType::Struct,
        )
        .set_schema::<MqttBrokerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "on_listening",
            "On Listening",
            "Fires when the broker is bound and ready",
            VariableType::Execution,
        );
        node.add_output_pin(
            "local_addr",
            "Local Addr",
            "Bound broker socket address",
            VariableType::String,
        );
        node.add_output_pin(
            "on_client_connect",
            "On Client Connect",
            "Fires when an MQTT client connects",
            VariableType::Execution,
        );
        node.add_output_pin(
            "client_id",
            "Client ID",
            "Connected MQTT client id",
            VariableType::String,
        );
        node.add_output_pin(
            "remote_addr",
            "Remote Addr",
            "Remote client socket address",
            VariableType::String,
        );
        node.add_output_pin(
            "on_close",
            "On Close",
            "Fires when the broker stops",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Fires if the broker fails to bind",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_client_connect").await?;
        context.deactivate_exec_pin("on_close").await?;
        context.activate_exec_pin("exec_error").await?;

        let config: MqttBrokerConfig = context.evaluate_pin("config").await?;
        let referenced_fns = context.get_referenced_functions().await?;
        let handler = referenced_fns.first().cloned();
        if referenced_fns.len() > 1 {
            context.log_message(
                "MQTT Broker uses only the first referenced on-message handler",
                LogLevel::Warn,
            );
        }

        let addr = format!("{}:{}", config.host, config.port);
        let listener = match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => listener,
            Err(err) => {
                context.log_message(
                    &format!("MQTT broker bind failed on {}: {}", addr, err),
                    LogLevel::Error,
                );
                return Ok(());
            }
        };

        let local_addr = listener.local_addr()?.to_string();
        let tls_acceptor = match crate::web::tls::server_acceptor(&config.tls) {
            Ok(acceptor) => acceptor,
            Err(err) => {
                context.log_message(
                    &format!("MQTT broker TLS configuration failed: {}", err),
                    LogLevel::Error,
                );
                return Ok(());
            }
        };
        context
            .set_pin_value("local_addr", json!(local_addr.clone()))
            .await?;
        context.deactivate_exec_pin("exec_error").await?;
        context.activate_exec_pin("on_listening").await?;
        trigger_connected_exec(context, "on_listening", "MQTT broker on_listening").await;

        let handler_context = if let Some(handler) = handler {
            Some(
                create_message_handler_context(
                    context,
                    handler,
                    &[
                        "_client",
                        "topic",
                        "client_id",
                        "remote_addr",
                        "qos",
                        "retain",
                    ],
                )
                .await,
            )
        } else {
            None
        };
        let subscribers: SubscriberMap = Arc::new(Mutex::new(HashMap::new()));

        let timeout = config.timeout_seconds;
        let cancellation_token = context.get_cancellation_token();
        let mut cancelled = false;
        let active_connections = Arc::new(AtomicU32::new(0));
        let mut client_tasks = Vec::new();

        loop {
            let accept = if timeout > 0 {
                tokio::select! {
                    result = listener.accept() => Some(result),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(timeout)) => {
                        context.log_message("MQTT broker timed out", LogLevel::Warn);
                        None
                    }
                    _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                        cancelled = true;
                        context.log_message("MQTT broker cancelled", LogLevel::Warn);
                        None
                    }
                }
            } else {
                tokio::select! {
                    result = listener.accept() => Some(result),
                    _ = super::super::wait_for_cancel(cancellation_token.clone()) => {
                        cancelled = true;
                        context.log_message("MQTT broker cancelled", LogLevel::Warn);
                        None
                    }
                }
            };

            let Some(accept) = accept else {
                break;
            };

            let (stream, remote_addr) = match accept {
                Ok(pair) => pair,
                Err(err) => {
                    context.log_message(&format!("MQTT accept error: {}", err), LogLevel::Error);
                    continue;
                }
            };

            if config.max_connections > 0
                && active_connections.load(Ordering::Relaxed) >= config.max_connections
            {
                context.log_message(
                    "MQTT broker rejected connection because max_connections was reached",
                    LogLevel::Warn,
                );
                continue;
            }

            let mut stream: crate::web::tls::BoxedIo = if let Some(acceptor) = &tls_acceptor {
                match acceptor.accept(stream).await {
                    Ok(stream) => Box::new(stream),
                    Err(err) => {
                        context.log_message(
                            &format!("MQTT TLS handshake failed: {}", err),
                            LogLevel::Error,
                        );
                        continue;
                    }
                }
            } else {
                Box::new(stream)
            };

            let connect = match read_mqtt_packet(&mut stream).await {
                Ok(Some(Packet::Connect(connect))) => connect,
                Ok(Some(packet)) => {
                    context.log_message(
                        &format!("MQTT expected CONNECT, received {:?}", packet),
                        LogLevel::Error,
                    );
                    continue;
                }
                Ok(None) => continue,
                Err(err) => {
                    context.log_message(
                        &format!("MQTT CONNECT read failed: {}", err),
                        LogLevel::Error,
                    );
                    continue;
                }
            };

            write_mqtt_packet(
                &mut stream,
                Packet::ConnAck(ConnAck::new(ConnectReturnCode::Success, false)),
            )
            .await?;

            active_connections.fetch_add(1, Ordering::Relaxed);
            let client_id = connect.client_id;
            let remote_addr = remote_addr.to_string();
            context
                .set_pin_value("client_id", json!(client_id.clone()))
                .await?;
            context
                .set_pin_value("remote_addr", json!(remote_addr.clone()))
                .await?;
            context.activate_exec_pin("on_client_connect").await?;
            trigger_connected_exec(
                context,
                "on_client_connect",
                "MQTT broker on_client_connect",
            )
            .await;

            let (reader, writer) = crate::web::tls::boxed_split(stream);
            let (tx, rx) = mpsc::unbounded_channel();
            let close_notify = Arc::new(Notify::new());
            let writer_handle = tokio::spawn(mqtt_client_writer(writer, rx, close_notify.clone()));
            let reader_handle = tokio::spawn(handle_mqtt_client(
                reader,
                tx,
                subscribers.clone(),
                handler_context.clone(),
                client_id,
                remote_addr,
                close_notify.clone(),
                active_connections.clone(),
            ));
            client_tasks.push(MqttClientTask {
                close_notify,
                reader_handle,
                writer_handle,
            });
        }

        shutdown_mqtt_clients(client_tasks).await;
        context.deactivate_exec_pin("on_listening").await?;
        context.deactivate_exec_pin("on_client_connect").await?;
        context.activate_exec_pin("on_close").await?;
        trigger_connected_exec(context, "on_close", "MQTT broker on_close").await;

        if cancelled {
            return Err(flow_like_types::anyhow!("Execution was cancelled"));
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

#[cfg(feature = "execute")]
async fn shutdown_mqtt_clients(client_tasks: Vec<MqttClientTask>) {
    for task in &client_tasks {
        task.close_notify.notify_waiters();
    }

    let handles = client_tasks
        .into_iter()
        .flat_map(|task| [task.reader_handle, task.writer_handle])
        .collect::<Vec<_>>();
    futures::future::join_all(handles.into_iter().map(await_or_abort)).await;
}

#[cfg(feature = "execute")]
async fn await_or_abort(mut handle: JoinHandle<()>) {
    tokio::select! {
        _ = &mut handle => {}
        _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {
            handle.abort();
            let _ = handle.await;
        }
    }
}

#[cfg(feature = "execute")]
async fn trigger_connected_exec(context: &mut ExecutionContext, pin_name: &str, log_name: &str) {
    let Ok(pin) = context.get_pin_by_name(pin_name).await else {
        return;
    };

    for node in pin.get_connected_nodes() {
        let mut sub = context.create_sub_context(&node).await;
        sub.delegated = true;
        let mut message = LogMessage::new(log_name, LogLevel::Debug, None);
        let _ = InternalNode::trigger(&mut sub, &mut None, true).await;
        message.end();
        sub.log(message);
        sub.end_trace();
        context.push_sub_context(&mut sub);
    }
}

#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
async fn handle_mqtt_client<R>(
    mut reader: R,
    tx: mpsc::UnboundedSender<Packet>,
    subscribers: SubscriberMap,
    handler_context: Option<MessageHandlerContext>,
    client_id: String,
    remote_addr: String,
    close_notify: Arc<Notify>,
    active_connections: Arc<AtomicU32>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    loop {
        let packet = tokio::select! {
            result = read_mqtt_packet(&mut reader) => {
                match result {
                    Ok(Some(packet)) => packet,
                    Ok(None) => break,
                    Err(err) => {
                        tracing::warn!("MQTT broker read error: {}", err);
                        break;
                    }
                }
            }
            _ = close_notify.notified() => break,
        };

        match packet {
            Packet::Subscribe(subscribe) => {
                let mut return_codes = Vec::with_capacity(subscribe.filters.len());
                {
                    let mut subscribers = subscribers.lock().await;
                    for filter in subscribe.filters {
                        subscribers.entry(filter.path).or_default().push(tx.clone());
                        return_codes.push(SubscribeReasonCode::Success(filter.qos));
                    }
                }
                let _ = tx.send(Packet::SubAck(SubAck::new(subscribe.pkid, return_codes)));
            }
            Packet::Publish(publish) => {
                if publish.qos != QoS::AtMostOnce {
                    let _ = tx.send(Packet::PubAck(PubAck::new(publish.pkid)));
                }

                if let Some(handler_context) = &handler_context {
                    let payload_bytes = publish.payload.to_vec();
                    let payload = match std::str::from_utf8(&payload_bytes) {
                        Ok(text) => IncomingPayload::Text(text.to_string()),
                        Err(_) => IncomingPayload::Binary(payload_bytes),
                    };
                    trigger_message_handler(
                        handler_context,
                        payload,
                        &[
                            ("topic", json!(publish.topic.clone())),
                            (
                                "_client",
                                json!({
                                    "client_id": client_id.clone(),
                                    "remote_addr": remote_addr.clone(),
                                    "topic": publish.topic.clone(),
                                    "qos": format!("{:?}", publish.qos),
                                    "retain": publish.retain,
                                }),
                            ),
                            ("client_id", json!(client_id.clone())),
                            ("remote_addr", json!(remote_addr.clone())),
                            ("qos", json!(format!("{:?}", publish.qos))),
                            ("retain", json!(publish.retain)),
                        ],
                        "MQTT broker on_message",
                    )
                    .await;
                }

                let forward = Packet::Publish(Publish::new(
                    publish.topic.clone(),
                    QoS::AtMostOnce,
                    publish.payload.to_vec(),
                ));
                let subscribers = subscribers.lock().await;
                if let Some(topic_subscribers) = subscribers.get(&publish.topic) {
                    for subscriber in topic_subscribers {
                        let _ = subscriber.send(forward.clone());
                    }
                }
            }
            Packet::PingReq => {
                let _ = tx.send(Packet::PingResp);
            }
            Packet::Disconnect => break,
            _ => {}
        }
    }

    active_connections.fetch_sub(1, Ordering::Relaxed);
}

#[cfg(feature = "execute")]
async fn mqtt_client_writer<W>(
    mut writer: W,
    mut rx: mpsc::UnboundedReceiver<Packet>,
    close_notify: Arc<Notify>,
) where
    W: AsyncWrite + Unpin + Send + 'static,
{
    loop {
        let packet = tokio::select! {
            packet = rx.recv() => packet,
            _ = close_notify.notified() => break,
        };
        let Some(packet) = packet else {
            break;
        };

        if let Err(err) = write_mqtt_packet(&mut writer, packet).await {
            tracing::warn!("MQTT broker write error: {}", err);
            break;
        }
    }

    let _ = writer.shutdown().await;
}

#[cfg(feature = "execute")]
async fn read_mqtt_packet<R>(reader: &mut R) -> flow_like_types::Result<Option<Packet>>
where
    R: AsyncRead + Unpin,
{
    let mut first = [0_u8; 1];
    match reader.read_exact(&mut first).await {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err.into()),
    }

    let mut frame = vec![first[0]];
    let mut multiplier = 1_usize;
    let mut remaining_len = 0_usize;
    loop {
        let mut byte = [0_u8; 1];
        reader.read_exact(&mut byte).await?;
        frame.push(byte[0]);
        remaining_len += ((byte[0] & 127) as usize) * multiplier;
        if (byte[0] & 128) == 0 {
            break;
        }
        multiplier *= 128;
        if multiplier > 128 * 128 * 128 {
            return Err(flow_like_types::anyhow!("Malformed MQTT remaining length"));
        }
    }

    let mut payload = vec![0_u8; remaining_len];
    reader.read_exact(&mut payload).await?;
    frame.extend_from_slice(&payload);

    let mut bytes = BytesMut::from(&frame[..]);
    let packet = Packet::read(&mut bytes, 1024 * 1024)
        .map_err(|err| flow_like_types::anyhow!("Failed to decode MQTT packet: {}", err))?;
    Ok(Some(packet))
}

#[cfg(feature = "execute")]
async fn write_mqtt_packet<W>(writer: &mut W, packet: Packet) -> flow_like_types::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut bytes = BytesMut::new();
    packet
        .write(&mut bytes, 1024 * 1024)
        .map_err(|err| flow_like_types::anyhow!("Failed to encode MQTT packet: {}", err))?;
    writer.write_all(&bytes).await?;
    Ok(())
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::web::test_support::{
        free_tcp_port, internal_node, node_with_outputs, output_value, test_context,
    };
    use flow_like::flow::{node::NodeLogic, variable::VariableType};
    use flow_like_types::json::json;
    use rumqttc::Event;
    use std::time::Duration;

    #[tokio::test]
    async fn mqtt_broker_e2e_delivers_publish_to_payload_handler() {
        let port = free_tcp_port();
        let handler = node_with_outputs(&[
            ("topic", VariableType::String),
            ("payload", VariableType::Struct),
        ]);
        let mut node = MqttBrokerNode::new().get_node();
        node.fn_refs
            .as_mut()
            .unwrap()
            .fn_refs
            .push(handler.node_id().to_string());
        let parent = internal_node(node);
        let mut context = test_context(parent, vec![handler.clone()]).await;
        context
            .set_pin_value(
                "config",
                json!(MqttBrokerConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    timeout_seconds: 1,
                    max_connections: 8,
                    tls: Default::default(),
                }),
            )
            .await
            .unwrap();

        let server = tokio::spawn(async move { MqttBrokerNode::new().run(&mut context).await });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut options = rumqttc::MqttOptions::new("flow-like-test-client", "127.0.0.1", port);
        options.set_keep_alive(Duration::from_secs(5));
        let (client, mut event_loop) = rumqttc::AsyncClient::new(options, 10);
        let poller = tokio::spawn(async move {
            loop {
                if event_loop.poll().await.is_err() {
                    break;
                }
            }
        });

        client
            .publish("flow/test", rumqttc::QoS::AtMostOnce, false, "hello")
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = client.disconnect().await;
        let _ = poller.await;

        server.await.unwrap().unwrap();
        let handler_context = test_context(handler, Vec::new()).await;
        assert_eq!(
            output_value(&handler_context, "topic").await,
            Some(json!("flow/test"))
        );
        let payload = output_value(&handler_context, "payload").await.unwrap();
        assert_eq!(payload["payload"], json!("hello"));
        assert_eq!(
            payload["_client"]["client_id"],
            json!("flow-like-test-client")
        );
        assert_eq!(payload["_client"]["topic"], json!("flow/test"));
    }

    #[tokio::test]
    async fn mqtt_broker_e2e_closes_open_clients_when_broker_stops() {
        let port = free_tcp_port();
        let parent = internal_node(MqttBrokerNode::new().get_node());
        let mut context = test_context(parent, Vec::new()).await;
        context
            .set_pin_value(
                "config",
                json!(MqttBrokerConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    timeout_seconds: 1,
                    max_connections: 8,
                    tls: Default::default(),
                }),
            )
            .await
            .unwrap();

        let server = tokio::spawn(async move { MqttBrokerNode::new().run(&mut context).await });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let mut options = rumqttc::MqttOptions::new("flow-like-open-client", "127.0.0.1", port);
        options.set_keep_alive(Duration::from_secs(5));
        let (_client, mut event_loop) = rumqttc::AsyncClient::new(options, 10);

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if let Event::Incoming(Packet::ConnAck(_)) = event_loop.poll().await.unwrap() {
                    break;
                }
            }
        })
        .await
        .expect("mqtt client should connect to broker");

        tokio::time::timeout(Duration::from_secs(3), server)
            .await
            .expect("broker should stop on timeout")
            .unwrap()
            .unwrap();

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if event_loop.poll().await.is_err() {
                    break;
                }
            }
        })
        .await
        .expect("mqtt client should observe broker-side shutdown");
    }

    #[tokio::test]
    async fn mqtt_broker_tls_e2e_delivers_publish_to_payload_handler() {
        let ca = crate::web::tls::create_ca_certificate("FlowLike MQTT Test CA").unwrap();
        let leaf = crate::web::tls::create_signed_certificate(
            &ca,
            "localhost",
            vec!["localhost".to_string(), "127.0.0.1".to_string()],
            "Server",
        )
        .unwrap();
        let port = free_tcp_port();
        let handler = node_with_outputs(&[
            ("topic", VariableType::String),
            ("payload", VariableType::Struct),
        ]);
        let mut node = MqttBrokerNode::new().get_node();
        node.fn_refs
            .as_mut()
            .unwrap()
            .fn_refs
            .push(handler.node_id().to_string());
        let parent = internal_node(node);
        let mut context = test_context(parent, vec![handler.clone()]).await;
        context
            .set_pin_value(
                "config",
                json!(MqttBrokerConfig {
                    host: "127.0.0.1".to_string(),
                    port,
                    timeout_seconds: 1,
                    max_connections: 8,
                    tls: crate::web::tls::TlsConfig {
                        secure: true,
                        certificate: Some(leaf),
                        ..Default::default()
                    },
                }),
            )
            .await
            .unwrap();

        let server = tokio::spawn(async move { MqttBrokerNode::new().run(&mut context).await });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let client_tls = crate::web::tls::TlsConfig {
            secure: true,
            ca_certificate_pem: Some(ca.certificate_pem),
            ..Default::default()
        };
        let client_config = crate::web::tls::client_config(&client_tls)
            .unwrap()
            .unwrap();
        let mut options = rumqttc::MqttOptions::new("flow-like-tls-client", "127.0.0.1", port);
        options.set_keep_alive(Duration::from_secs(5));
        options.set_transport(rumqttc::Transport::tls_with_config(client_config.into()));
        let (client, mut event_loop) = rumqttc::AsyncClient::new(options, 10);
        let poller = tokio::spawn(async move {
            loop {
                if event_loop.poll().await.is_err() {
                    break;
                }
            }
        });

        client
            .publish(
                "flow/tls",
                rumqttc::QoS::AtMostOnce,
                false,
                "hello mqtt tls",
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = client.disconnect().await;
        let _ = poller.await;

        server.await.unwrap().unwrap();
        let handler_context = test_context(handler, Vec::new()).await;
        assert_eq!(
            output_value(&handler_context, "topic").await,
            Some(json!("flow/tls"))
        );
        let payload = output_value(&handler_context, "payload").await.unwrap();
        assert_eq!(payload["payload"], json!("hello mqtt tls"));
        assert_eq!(
            payload["_client"]["client_id"],
            json!("flow-like-tls-client")
        );
    }
}
