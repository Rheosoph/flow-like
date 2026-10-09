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
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
#[cfg(feature = "execute")]
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Mutex, mpsc, watch},
    task::JoinHandle,
};

use super::MqttBrokerConfig;
#[cfg(feature = "execute")]
use super::topic::{topic_matches, valid_filter};

#[cfg(feature = "execute")]
use crate::web::message_handler::{
    IncomingPayload, MessageHandlerContext, create_message_handler_context, trigger_message_handler,
};
#[cfg(feature = "execute")]
use rumqttc::{
    ConnAck, ConnectReturnCode, Packet, PubAck, Publish, QoS, SubAck, SubscribeReasonCode, UnsubAck,
};

#[cfg(feature = "execute")]
type Broker = Arc<Mutex<BrokerState>>;

#[cfg(feature = "execute")]
const CLIENT_QUEUE: usize = 64;
#[cfg(feature = "execute")]
const HANDLER_QUEUE: usize = 32;
#[cfg(feature = "execute")]
const MAX_PACKET: usize = 1024 * 1024;
#[cfg(feature = "execute")]
const MAX_RETAINED_BYTES: usize = 8 * 1024 * 1024;
#[cfg(feature = "execute")]
const MAX_QUEUED_PUBLICATIONS: usize = 1024;

#[cfg(feature = "execute")]
#[derive(Default)]
struct BrokerState {
    clients: HashMap<u64, BrokerClient>,
    retained: HashMap<String, Publish>,
    retained_bytes: usize,
    next_connection: u64,
    stopping: bool,
}

#[cfg(feature = "execute")]
struct BrokerClient {
    client_id: String,
    tx: mpsc::Sender<Packet>,
    close: watch::Sender<bool>,
    filters: HashMap<String, QoS>,
    pending: HashSet<u16>,
    next_pkid: u16,
    publications: VecDeque<Publish>,
    publication_bytes: usize,
}

#[cfg(feature = "execute")]
struct MqttClientTask {
    close: watch::Sender<bool>,
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
            "Binds a clean-session MQTT 3.1.1 broker with QoS 0/1, wildcard subscriptions, retained messages in memory, and Last Will delivery. QoS 2 and persistent sessions are refused. Published messages reach the referenced on-message handler.",
            "Web/MQTT",
        );
        node.set_flowscript_name("mqtt", "broker");
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
        let listener =
            match tokio::net::TcpListener::bind((config.host.as_str(), config.port)).await {
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
        let tls_acceptor = match crate::web::tls::ServiceAcceptor::new(context, &config.tls).await {
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
                        "payload_bytes",
                    ],
                )
                .await,
            )
        } else {
            None
        };
        let broker: Broker = Arc::new(Mutex::new(BrokerState::default()));

        let timeout = config.timeout_seconds;
        let cancellation_token = context.get_cancellation_token();
        let mut cancelled = false;
        let active_connections = Arc::new(AtomicU32::new(0));
        let mut client_tasks = Vec::new();

        loop {
            client_tasks.retain(|task: &MqttClientTask| {
                !task.reader_handle.is_finished() || !task.writer_handle.is_finished()
            });
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

            let accepted = tokio::select! {
                result = tokio::time::timeout(std::time::Duration::from_secs(10), tls_acceptor.accept(stream)) => result,
                _ = super::super::wait_for_cancel(cancellation_token.clone()) => { cancelled = true; break; }
            };
            let mut stream: crate::web::tls::BoxedIo = match accepted {
                Ok(Ok(stream)) => stream,
                Ok(Err(err)) => {
                    context.log_message(
                        &format!("MQTT TLS handshake failed: {err}"),
                        LogLevel::Error,
                    );
                    continue;
                }
                Err(_) => continue,
            };

            let handshake = tokio::select! {
                result = tokio::time::timeout(std::time::Duration::from_secs(10), read_mqtt_packet(&mut stream)) => result,
                _ = super::super::wait_for_cancel(cancellation_token.clone()) => { cancelled = true; break; }
            };
            let connect = match handshake {
                Ok(Ok(Some(Packet::Connect(connect)))) => connect,
                Ok(Ok(Some(packet))) => {
                    context.log_message(
                        &format!("MQTT expected CONNECT, received {:?}", packet),
                        LogLevel::Error,
                    );
                    continue;
                }
                Ok(Ok(None)) => continue,
                Ok(Err(err)) => {
                    context.log_message(
                        &format!("MQTT CONNECT read failed: {}", err),
                        LogLevel::Error,
                    );
                    continue;
                }
                Err(_) => {
                    context.log_message("MQTT CONNECT timed out", LogLevel::Warn);
                    continue;
                }
            };

            if let Some(reason) = reject_connect(&connect) {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    write_mqtt_packet(&mut stream, Packet::ConnAck(ConnAck::new(reason, false))),
                )
                .await;
                continue;
            }

            if !matches!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(1),
                    write_mqtt_packet(
                        &mut stream,
                        Packet::ConnAck(ConnAck::new(ConnectReturnCode::Success, false)),
                    )
                )
                .await,
                Ok(Ok(()))
            ) {
                continue;
            }

            active_connections.fetch_add(1, Ordering::Relaxed);
            let client_id = connect.client_id.clone();
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
            let (tx, rx) = mpsc::channel(CLIENT_QUEUE);
            let (close, _) = watch::channel(false);
            let connection_id = {
                let mut state = broker.lock().await;
                for client in state.clients.values() {
                    if client.client_id == client_id {
                        client.close.send_replace(true);
                    }
                }
                state.next_connection += 1;
                let id = state.next_connection;
                state.clients.insert(
                    id,
                    BrokerClient {
                        client_id: client_id.clone(),
                        tx: tx.clone(),
                        close: close.clone(),
                        filters: HashMap::new(),
                        pending: HashSet::new(),
                        next_pkid: 0,
                        publications: VecDeque::new(),
                        publication_bytes: 0,
                    },
                );
                id
            };
            let writer_handle = tokio::spawn(mqtt_client_writer(
                writer,
                rx,
                close.clone(),
                broker.clone(),
                connection_id,
            ));
            let reader_handle = tokio::spawn(handle_mqtt_client(
                reader,
                tx,
                broker.clone(),
                handler_context.clone(),
                client_id,
                remote_addr,
                close.clone(),
                active_connections.clone(),
                connection_id,
                connect.last_will,
                connect.keep_alive,
            ));
            client_tasks.push(MqttClientTask {
                close,
                reader_handle,
                writer_handle,
            });
        }

        broker.lock().await.stopping = true;
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
        task.close.send_replace(true);
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
fn reject_connect(connect: &rumqttc::Connect) -> Option<ConnectReturnCode> {
    if connect.protocol != rumqttc::Protocol::V4 {
        return Some(ConnectReturnCode::RefusedProtocolVersion);
    }
    if !connect.clean_session {
        return Some(ConnectReturnCode::ServiceUnavailable);
    }
    if connect.client_id.is_empty() || connect.client_id.contains('\0') {
        return Some(ConnectReturnCode::BadClientId);
    }
    if connect
        .last_will
        .as_ref()
        .is_some_and(|will| will.qos == QoS::ExactlyOnce || !valid_topic(&will.topic))
    {
        return Some(ConnectReturnCode::ServiceUnavailable);
    }
    None
}

#[cfg(feature = "execute")]
fn valid_topic(topic: &str) -> bool {
    !topic.is_empty() && topic.len() <= u16::MAX as usize && !topic.contains(['#', '+', '\0'])
}

#[cfg(feature = "execute")]
impl BrokerClient {
    fn send(&self, packet: Packet) -> bool {
        if *self.close.borrow() {
            return false;
        }
        if self.tx.try_send(packet).is_err() {
            self.close.send_replace(true);
            return false;
        }
        true
    }

    fn publish(&mut self, source: &Publish, qos: QoS, retained: bool) {
        if *self.close.borrow() {
            return;
        }
        let mut publish = source.clone();
        publish.qos = if source.qos == QoS::AtMostOnce {
            QoS::AtMostOnce
        } else {
            qos
        };
        publish.retain = retained;
        publish.dup = false;
        publish.pkid = 0;
        let bytes = publish.topic.len() + publish.payload.len();
        if self.publications.len() >= MAX_QUEUED_PUBLICATIONS
            || self.publication_bytes + bytes > MAX_RETAINED_BYTES
        {
            self.close.send_replace(true);
            return;
        }
        self.publication_bytes += bytes;
        self.publications.push_back(publish);
        self.flush_publications();
    }

    fn flush_publications(&mut self) {
        // Leave room for control packets. Resume after socket writes and PUBACKs,
        // without holding the broker lock while waiting for either one.
        while self.tx.capacity() > 1 && !*self.close.borrow() {
            let Some(mut publish) = self.publications.front().cloned() else {
                break;
            };
            if publish.qos == QoS::AtLeastOnce {
                if self.pending.len() >= CLIENT_QUEUE {
                    break;
                }
                loop {
                    self.next_pkid = self.next_pkid.wrapping_add(1).max(1);
                    if !self.pending.contains(&self.next_pkid) {
                        break;
                    }
                }
                publish.pkid = self.next_pkid;
            }
            let pkid = publish.pkid;
            let bytes = publish.topic.len() + publish.payload.len();
            match self.tx.try_send(Packet::Publish(publish)) {
                Ok(()) => {
                    if pkid != 0 {
                        self.pending.insert(pkid);
                    }
                    self.publications.pop_front();
                    self.publication_bytes -= bytes;
                }
                Err(mpsc::error::TrySendError::Full(_)) => break,
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    self.close.send_replace(true);
                    break;
                }
            }
        }
    }
}

#[cfg(feature = "execute")]
impl BrokerState {
    fn publish(&mut self, publish: &Publish) -> bool {
        if !valid_topic(&publish.topic) || publish.qos == QoS::ExactlyOnce {
            return false;
        }
        if publish.retain {
            let previous = self
                .retained
                .get(&publish.topic)
                .map_or(0, |old| old.topic.len() + old.payload.len());
            let bytes = publish.topic.len() + publish.payload.len();
            if !publish.payload.is_empty()
                && (self.retained_bytes - previous + bytes > MAX_RETAINED_BYTES
                    || (!self.retained.contains_key(&publish.topic) && self.retained.len() >= 1024))
            {
                return false;
            }
            self.retained_bytes -= previous;
            self.retained.remove(&publish.topic);
            if !publish.payload.is_empty() {
                self.retained_bytes += bytes;
                self.retained.insert(publish.topic.clone(), publish.clone());
            }
        }
        for client in self.clients.values_mut() {
            let qos = client
                .filters
                .iter()
                .filter(|(filter, _)| topic_matches(filter, &publish.topic))
                .map(|(_, qos)| *qos)
                .max_by_key(|qos| *qos as u8);
            if let Some(qos) = qos {
                client.publish(publish, qos, false);
            }
        }
        true
    }

    fn subscribe(&mut self, connection_id: u64, subscribe: rumqttc::Subscribe) -> bool {
        if subscribe.pkid == 0 || subscribe.filters.is_empty() || subscribe.filters.len() > 128 {
            return false;
        }
        let Some(client) = self.clients.get_mut(&connection_id) else {
            return false;
        };
        let mut codes = Vec::with_capacity(subscribe.filters.len());
        let mut accepted = Vec::new();
        for filter in subscribe.filters {
            if filter.qos == QoS::ExactlyOnce
                || !valid_filter(&filter.path)
                || (!client.filters.contains_key(&filter.path) && client.filters.len() >= 128)
            {
                codes.push(SubscribeReasonCode::Failure);
            } else {
                codes.push(SubscribeReasonCode::Success(filter.qos));
                client.filters.insert(filter.path.clone(), filter.qos);
                accepted.push(filter);
            }
        }
        if !client.send(Packet::SubAck(SubAck::new(subscribe.pkid, codes))) {
            return false;
        }
        for retained in self.retained.values() {
            let qos = accepted
                .iter()
                .filter(|filter| topic_matches(&filter.path, &retained.topic))
                .map(|filter| filter.qos)
                .max_by_key(|qos| *qos as u8);
            if let Some(qos) = qos {
                client.publish(retained, qos, true);
            }
        }
        !*client.close.borrow()
    }
}

#[cfg(feature = "execute")]
async fn dispatch_publish(
    handler: Option<&MessageHandlerContext>,
    publish: &Publish,
    client_id: &str,
    remote_addr: &str,
) {
    if let Some(handler) = handler {
        let payload = match std::str::from_utf8(&publish.payload) {
            Ok(text) => IncomingPayload::Text(text.to_owned()),
            Err(_) => IncomingPayload::Binary(publish.payload.to_vec()),
        };
        let _ = tokio::time::timeout(std::time::Duration::from_secs(30), trigger_message_handler(
            handler, payload,
            &[
                ("topic", json!(publish.topic)),
                ("payload_bytes", json!(publish.payload.to_vec())),
                ("_client", json!({"client_id": client_id, "remote_addr": remote_addr, "topic": publish.topic,
                    "qos": format!("{:?}", publish.qos), "retain": publish.retain, "payload_bytes": publish.payload.to_vec()})),
                ("client_id", json!(client_id)), ("remote_addr", json!(remote_addr)),
                ("qos", json!(format!("{:?}", publish.qos))), ("retain", json!(publish.retain)),
            ], "MQTT broker on_message",
        )).await;
    }
}

#[cfg(feature = "execute")]
#[allow(clippy::too_many_arguments)]
async fn handle_mqtt_client<R>(
    mut reader: R,
    tx: mpsc::Sender<Packet>,
    broker: Broker,
    handler_context: Option<MessageHandlerContext>,
    client_id: String,
    remote_addr: String,
    close: watch::Sender<bool>,
    active_connections: Arc<AtomicU32>,
    connection_id: u64,
    will: Option<rumqttc::LastWill>,
    keep_alive: u16,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut closed = close.subscribe();
    let (workflow_tx, mut workflow_rx) = mpsc::channel::<Publish>(HANDLER_QUEUE);
    // Poll the ordered workflow queue alongside socket reads, including keep-alive packets.
    // The future belongs to this client task, so aborting the client also cancels its handler.
    let workflow = async {
        while let Some(publish) = workflow_rx.recv().await {
            dispatch_publish(handler_context.as_ref(), &publish, &client_id, &remote_addr).await;
        }
    };
    tokio::pin!(workflow);
    let mut graceful = false;
    loop {
        if *closed.borrow() {
            break;
        }
        let packet = tokio::select! {
            biased;
            _ = closed.changed() => break,
            _ = &mut workflow => break,
            result = async {
                if keep_alive == 0 { read_mqtt_packet(&mut reader).await }
                else { tokio::time::timeout(std::time::Duration::from_millis(u64::from(keep_alive) * 1500), read_mqtt_packet(&mut reader))
                    .await.map_err(|_| flow_like_types::anyhow!("MQTT keep-alive expired"))? }
            } => match result {
                Ok(Some(packet)) => packet,
                _ => break,
            }
        };
        match packet {
            Packet::Subscribe(subscribe) => {
                if !broker.lock().await.subscribe(connection_id, subscribe) {
                    break;
                }
            }
            Packet::Unsubscribe(unsubscribe) => {
                if unsubscribe.pkid == 0 || unsubscribe.topics.is_empty() {
                    break;
                }
                let mut state = broker.lock().await;
                let Some(client) = state.clients.get_mut(&connection_id) else {
                    break;
                };
                for topic in unsubscribe.topics {
                    client.filters.remove(&topic);
                }
                if !client.send(Packet::UnsubAck(UnsubAck::new(unsubscribe.pkid))) {
                    break;
                }
            }
            Packet::Publish(publish) => {
                if publish.qos == QoS::ExactlyOnce
                    || (publish.qos == QoS::AtLeastOnce && publish.pkid == 0)
                {
                    break;
                }
                if !broker.lock().await.publish(&publish) {
                    break;
                }
                if handler_context.is_some() && workflow_tx.try_send(publish.clone()).is_err() {
                    tracing::warn!(
                        "MQTT workflow queue full; closing publisher before acknowledging delivery"
                    );
                    break;
                }
                // The PUBACK confirms broker acceptance; workflow handlers are a separate subscriber.
                if publish.qos == QoS::AtLeastOnce
                    && tx
                        .try_send(Packet::PubAck(PubAck::new(publish.pkid)))
                        .is_err()
                {
                    break;
                }
            }
            Packet::PubAck(ack) => {
                if ack.pkid == 0 {
                    break;
                }
                if let Some(client) = broker.lock().await.clients.get_mut(&connection_id) {
                    client.pending.remove(&ack.pkid);
                    client.flush_publications();
                }
            }
            Packet::PingReq => {
                if tx.try_send(Packet::PingResp).is_err() {
                    break;
                }
            }
            Packet::Disconnect => {
                graceful = true;
                break;
            }
            _ => break,
        }
    }
    let send_will = {
        let mut state = broker.lock().await;
        state.clients.remove(&connection_id);
        !graceful && !state.stopping
    };
    close.send_replace(true);
    let pending_will = if send_will && let Some(will) = will {
        let mut publish = Publish::new(will.topic, will.qos, will.message.to_vec());
        publish.retain = will.retain;
        broker.lock().await.publish(&publish).then_some(publish)
    } else {
        None
    };
    drop(workflow_tx);
    // Each accepted delivery keeps its own handler deadline. Broker shutdown can still
    // abort this client task, but a normal disconnect drains every accepted publication.
    workflow.await;
    if let Some(publish) = pending_will {
        dispatch_publish(handler_context.as_ref(), &publish, &client_id, &remote_addr).await;
    }
    active_connections.fetch_sub(1, Ordering::Relaxed);
}

#[cfg(feature = "execute")]
async fn mqtt_client_writer<W>(
    mut writer: W,
    mut rx: mpsc::Receiver<Packet>,
    close: watch::Sender<bool>,
    broker: Broker,
    connection_id: u64,
) where
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut closed = close.subscribe();
    loop {
        if *closed.borrow() {
            break;
        }
        let packet = tokio::select! {
            biased;
            _ = closed.changed() => break,
            packet = rx.recv() => match packet { Some(packet) => packet, None => break },
        };
        let result = tokio::select! {
            biased;
            _ = closed.changed() => break,
            result = tokio::time::timeout(std::time::Duration::from_secs(10), write_mqtt_packet(&mut writer, packet)) => result,
        };
        if !matches!(result, Ok(Ok(()))) {
            break;
        }
        if let Some(client) = broker.lock().await.clients.get_mut(&connection_id) {
            client.flush_publications();
        }
    }
    close.send_replace(true);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), writer.shutdown()).await;
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

    if remaining_len > MAX_PACKET {
        return Err(flow_like_types::anyhow!(
            "MQTT packet exceeds the 1 MiB limit"
        ));
    }
    let mut payload = vec![0_u8; remaining_len];
    reader.read_exact(&mut payload).await?;
    frame.extend_from_slice(&payload);

    let mut bytes = BytesMut::from(&frame[..]);
    let packet = Packet::read(&mut bytes, MAX_PACKET)
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
        .write(&mut bytes, MAX_PACKET)
        .map_err(|err| flow_like_types::anyhow!("Failed to encode MQTT packet: {}", err))?;
    writer.write_all(&bytes).await?;
    Ok(())
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use crate::web::test_support::{
        free_tcp_port, internal_node, internal_node_with_logic, node_with_outputs, output_value,
        test_context,
    };
    use flow_like::flow::{node::NodeLogic, variable::VariableType};
    use flow_like_types::json::json;
    use rumqttc::Event;
    use std::time::Duration;

    async fn wire_client(
        broker: &Broker,
        will: Option<rumqttc::LastWill>,
        keep_alive: u16,
    ) -> (tokio::io::DuplexStream, MqttClientTask) {
        wire_client_with_handler(broker, will, keep_alive, None).await
    }

    async fn wire_client_with_handler(
        broker: &Broker,
        will: Option<rumqttc::LastWill>,
        keep_alive: u16,
        handler: Option<MessageHandlerContext>,
    ) -> (tokio::io::DuplexStream, MqttClientTask) {
        let (peer, server) = tokio::io::duplex(MAX_PACKET * 2);
        let (reader, writer) = tokio::io::split(server);
        let (tx, rx) = mpsc::channel(CLIENT_QUEUE);
        let (close, _) = watch::channel(false);
        let id = {
            let mut state = broker.lock().await;
            state.next_connection += 1;
            let id = state.next_connection;
            state.clients.insert(
                id,
                BrokerClient {
                    client_id: id.to_string(),
                    tx: tx.clone(),
                    close: close.clone(),
                    filters: HashMap::new(),
                    pending: HashSet::new(),
                    next_pkid: 0,
                    publications: VecDeque::new(),
                    publication_bytes: 0,
                },
            );
            id
        };
        let writer_handle = tokio::spawn(mqtt_client_writer(
            writer,
            rx,
            close.clone(),
            broker.clone(),
            id,
        ));
        let reader_handle = tokio::spawn(handle_mqtt_client(
            reader,
            tx,
            broker.clone(),
            handler,
            id.to_string(),
            "loopback".into(),
            close.clone(),
            Arc::new(AtomicU32::new(1)),
            id,
            will,
            keep_alive,
        ));
        (
            peer,
            MqttClientTask {
                close,
                writer_handle,
                reader_handle,
            },
        )
    }

    async fn receive(peer: &mut tokio::io::DuplexStream) -> Packet {
        tokio::time::timeout(Duration::from_secs(3), read_mqtt_packet(peer))
            .await
            .unwrap()
            .unwrap()
            .unwrap()
    }

    async fn subscribe(peer: &mut tokio::io::DuplexStream, topic: &str) {
        let mut subscription = rumqttc::Subscribe::new(topic, QoS::AtLeastOnce);
        subscription.pkid = 1;
        write_mqtt_packet(peer, Packet::Subscribe(subscription))
            .await
            .unwrap();
        assert!(matches!(receive(peer).await, Packet::SubAck(_)));
    }

    struct BlockedHandler {
        entered: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl NodeLogic for BlockedHandler {
        fn get_node(&self) -> Node {
            let mut node = Node::new("blocked", "Blocked", "Blocked handler", "Tests");
            node.add_output_pin("payload", "Payload", "Payload", VariableType::Struct);
            node
        }

        async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            self.entered.notify_one();
            std::future::pending().await
        }
    }

    #[tokio::test]
    async fn blocked_workflow_keeps_ping_responsive_and_bounds_pending_publications() {
        let entered = Arc::new(tokio::sync::Notify::new());
        let logic = Arc::new(BlockedHandler {
            entered: entered.clone(),
        });
        let handler = internal_node_with_logic(logic.get_node(), logic);
        let parent = internal_node(MqttBrokerNode::new().get_node());
        let context = test_context(parent, vec![handler.clone()]).await;
        let handler = create_message_handler_context(&context, handler, &["topic"]).await;
        let broker = Arc::new(Mutex::new(BrokerState::default()));
        let (mut publisher, task) = wire_client_with_handler(&broker, None, 1, Some(handler)).await;
        let mut publish = Publish::new("workflow/slow", QoS::AtLeastOnce, vec![1]);
        publish.pkid = 1;
        write_mqtt_packet(&mut publisher, Packet::Publish(publish.clone()))
            .await
            .unwrap();
        assert!(matches!(receive(&mut publisher).await, Packet::PubAck(_)));
        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .unwrap();
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(400)).await;
            write_mqtt_packet(&mut publisher, Packet::PingReq)
                .await
                .unwrap();
            let packet =
                tokio::time::timeout(Duration::from_millis(300), read_mqtt_packet(&mut publisher))
                    .await
                    .expect("workflow must not block PINGRESP")
                    .unwrap()
                    .unwrap();
            assert!(matches!(packet, Packet::PingResp));
        }
        // One workflow is running and at most HANDLER_QUEUE more may wait.
        for id in 2..=HANDLER_QUEUE + 1 {
            publish.pkid = id as u16;
            write_mqtt_packet(&mut publisher, Packet::Publish(publish.clone()))
                .await
                .unwrap();
            assert!(
                matches!(receive(&mut publisher).await, Packet::PubAck(ack) if ack.pkid == id as u16)
            );
        }
        publish.pkid = (HANDLER_QUEUE + 2) as u16;
        write_mqtt_packet(&mut publisher, Packet::Publish(publish))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), read_mqtt_packet(&mut publisher))
                .await
                .unwrap()
                .unwrap()
                .is_none(),
            "overflow must close without PUBACK"
        );
        broker.lock().await.stopping = true;
        shutdown_mqtt_clients(vec![task]).await;
    }

    struct GatedHandler {
        started: mpsc::UnboundedSender<String>,
        permits: Arc<tokio::sync::Semaphore>,
    }

    #[async_trait]
    impl NodeLogic for GatedHandler {
        fn get_node(&self) -> Node {
            let mut node = Node::new("gated", "Gated", "Gated handler", "Tests");
            node.add_output_pin("topic", "Topic", "Topic", VariableType::String);
            node.add_output_pin("payload", "Payload", "Payload", VariableType::Struct);
            node
        }

        async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            self.started
                .send(context.evaluate_pin("topic").await?)
                .unwrap();
            self.permits.acquire().await.unwrap().forget();
            Ok(())
        }
    }

    #[tokio::test]
    async fn disconnect_drains_accepted_workflows_and_full_queue_preserves_last_will() {
        for graceful in [true, false] {
            let (started, mut observed) = mpsc::unbounded_channel();
            let permits = Arc::new(tokio::sync::Semaphore::new(0));
            let logic = Arc::new(GatedHandler {
                started,
                permits: permits.clone(),
            });
            let handler = internal_node_with_logic(logic.get_node(), logic);
            let parent = internal_node(MqttBrokerNode::new().get_node());
            let context = test_context(parent, vec![handler.clone()]).await;
            let handler = create_message_handler_context(&context, handler, &["topic"]).await;
            let broker = Arc::new(Mutex::new(BrokerState::default()));
            let will = rumqttc::LastWill::new("workflow/will", vec![255], QoS::AtLeastOnce, false);
            let (mut publisher, task) =
                wire_client_with_handler(&broker, Some(will), 0, Some(handler)).await;
            for index in 0..=HANDLER_QUEUE {
                let mut publish =
                    Publish::new(format!("workflow/{index}"), QoS::AtLeastOnce, vec![1]);
                publish.pkid = index as u16 + 1;
                write_mqtt_packet(&mut publisher, Packet::Publish(publish))
                    .await
                    .unwrap();
                assert!(
                    matches!(receive(&mut publisher).await, Packet::PubAck(ack) if ack.pkid == index as u16 + 1)
                );
                if index == 0 {
                    assert_eq!(
                        tokio::time::timeout(Duration::from_secs(1), observed.recv())
                            .await
                            .unwrap()
                            .unwrap(),
                        "workflow/0"
                    );
                }
            }
            if graceful {
                write_mqtt_packet(&mut publisher, Packet::Disconnect)
                    .await
                    .unwrap();
                assert!(
                    tokio::time::timeout(Duration::from_secs(1), read_mqtt_packet(&mut publisher))
                        .await
                        .unwrap()
                        .unwrap()
                        .is_none()
                );
            }
            drop(publisher);
            assert!(
                !task.reader_handle.is_finished(),
                "accepted workflows must survive socket closure"
            );
            for index in 1..=HANDLER_QUEUE {
                permits.add_permits(1);
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(1), observed.recv())
                        .await
                        .unwrap()
                        .unwrap(),
                    format!("workflow/{index}")
                );
            }
            permits.add_permits(1);
            if !graceful {
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(1), observed.recv())
                        .await
                        .unwrap()
                        .unwrap(),
                    "workflow/will"
                );
                permits.add_permits(1);
            }
            tokio::time::timeout(Duration::from_secs(1), task.reader_handle)
                .await
                .unwrap()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(1), task.writer_handle)
                .await
                .unwrap()
                .unwrap();
            assert!(observed.try_recv().is_err());
            assert!(broker.lock().await.clients.is_empty());
        }
    }

    #[tokio::test]
    async fn mqtt_broker_wire_qos1_retention_wildcards_and_unsubscribe() {
        let broker = Arc::new(Mutex::new(BrokerState::default()));
        let (mut publisher, ptask) = wire_client(&broker, None, 0).await;
        let (mut subscriber, stask) = wire_client(&broker, None, 0).await;
        subscribe(&mut subscriber, "plant/+/value").await;
        let mut publish = Publish::new("plant/one/value", QoS::AtLeastOnce, vec![0, 255]);
        publish.pkid = 7;
        publish.retain = true;
        write_mqtt_packet(&mut publisher, Packet::Publish(publish.clone()))
            .await
            .unwrap();
        assert!(matches!(receive(&mut publisher).await, Packet::PubAck(ack) if ack.pkid == 7));
        let Packet::Publish(delivery) = receive(&mut subscriber).await else {
            panic!("expected publication")
        };
        assert_eq!(delivery.payload.as_ref(), &[0, 255]);
        assert_eq!(delivery.qos, QoS::AtLeastOnce);
        assert!(!delivery.retain);
        assert_ne!(delivery.pkid, 0);
        write_mqtt_packet(&mut subscriber, Packet::PubAck(PubAck::new(delivery.pkid)))
            .await
            .unwrap();
        write_mqtt_packet(&mut subscriber, Packet::PingReq)
            .await
            .unwrap();
        assert!(matches!(receive(&mut subscriber).await, Packet::PingResp));
        assert!(
            broker
                .lock()
                .await
                .clients
                .values()
                .all(|client| client.pending.is_empty())
        );
        let (mut late, ltask) = wire_client(&broker, None, 0).await;
        subscribe(&mut late, "plant/#").await;
        assert!(
            matches!(receive(&mut late).await, Packet::Publish(value) if value.retain && value.payload.as_ref() == [0, 255])
        );
        let mut unsubscribe = rumqttc::Unsubscribe::new("plant/+/value");
        unsubscribe.pkid = 12;
        write_mqtt_packet(&mut subscriber, Packet::Unsubscribe(unsubscribe))
            .await
            .unwrap();
        assert!(matches!(receive(&mut subscriber).await, Packet::UnsubAck(ack) if ack.pkid == 12));
        publish.payload = bytes::Bytes::new();
        publish.pkid = 8;
        write_mqtt_packet(&mut publisher, Packet::Publish(publish))
            .await
            .unwrap();
        assert!(matches!(receive(&mut publisher).await, Packet::PubAck(_)));
        assert!(broker.lock().await.retained.is_empty());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), read_mqtt_packet(&mut subscriber))
                .await
                .is_err()
        );
        broker.lock().await.stopping = true;
        shutdown_mqtt_clients(vec![ptask, stask, ltask]).await;
    }

    #[tokio::test]
    async fn retained_replay_larger_than_the_inflight_window_finishes_without_disconnect() {
        for qos in [QoS::AtMostOnce, QoS::AtLeastOnce] {
            let broker = Arc::new(Mutex::new(BrokerState::default()));
            for index in 0..MAX_QUEUED_PUBLICATIONS {
                let mut publish = Publish::new(format!("plant/{index}"), qos, vec![1]);
                publish.retain = true;
                assert!(broker.lock().await.publish(&publish));
            }
            let (mut subscriber, task) = wire_client(&broker, None, 0).await;
            subscribe(&mut subscriber, "plant/#").await;
            let mut topics = HashSet::new();
            for _ in 0..MAX_QUEUED_PUBLICATIONS {
                let Packet::Publish(publish) = receive(&mut subscriber).await else {
                    panic!("expected retained publication");
                };
                assert!(publish.retain);
                assert_eq!(publish.qos, qos);
                assert!(topics.insert(publish.topic));
                if qos == QoS::AtLeastOnce {
                    write_mqtt_packet(&mut subscriber, Packet::PubAck(PubAck::new(publish.pkid)))
                        .await
                        .unwrap();
                }
            }
            write_mqtt_packet(&mut subscriber, Packet::PingReq)
                .await
                .unwrap();
            assert!(matches!(receive(&mut subscriber).await, Packet::PingResp));
            {
                let state = broker.lock().await;
                let client = state.clients.values().next().unwrap();
                assert!(client.publications.is_empty());
                assert!(client.pending.is_empty());
                assert_eq!(client.publication_bytes, 0);
            }
            broker.lock().await.stopping = true;
            shutdown_mqtt_clients(vec![task]).await;
        }
    }

    #[tokio::test]
    async fn live_messages_follow_retained_replay_and_wait_for_acknowledgements() {
        let broker = Arc::new(Mutex::new(BrokerState::default()));
        for index in 0..CLIENT_QUEUE * 2 {
            let mut publish = Publish::new(format!("plant/{index}"), QoS::AtLeastOnce, vec![0]);
            publish.retain = true;
            assert!(broker.lock().await.publish(&publish));
        }
        let (mut subscriber, task) = wire_client(&broker, None, 0).await;
        subscribe(&mut subscriber, "plant/#").await;
        assert!(
            broker
                .lock()
                .await
                .publish(&Publish::new("plant/0", QoS::AtLeastOnce, vec![1]))
        );
        let mut first_window = Vec::new();
        for _ in 0..CLIENT_QUEUE {
            let Packet::Publish(publish) = receive(&mut subscriber).await else {
                panic!("expected publication");
            };
            first_window.push(publish.pkid);
        }
        // The wire stays open while the in-flight window is full.
        write_mqtt_packet(&mut subscriber, Packet::PingReq)
            .await
            .unwrap();
        assert!(matches!(receive(&mut subscriber).await, Packet::PingResp));
        for pkid in first_window {
            write_mqtt_packet(&mut subscriber, Packet::PubAck(PubAck::new(pkid)))
                .await
                .unwrap();
        }
        for index in 0..=CLIENT_QUEUE {
            let Packet::Publish(publish) = receive(&mut subscriber).await else {
                panic!("expected publication");
            };
            if index == CLIENT_QUEUE {
                assert_eq!(publish.topic, "plant/0");
                assert_eq!(publish.payload.as_ref(), &[1]);
                assert!(!publish.retain);
            } else {
                assert!(publish.retain);
                assert_eq!(publish.payload.as_ref(), &[0]);
            }
            write_mqtt_packet(&mut subscriber, Packet::PubAck(PubAck::new(publish.pkid)))
                .await
                .unwrap();
        }
        broker.lock().await.stopping = true;
        shutdown_mqtt_clients(vec![task]).await;
    }

    #[tokio::test]
    async fn mqtt_broker_wire_last_will_and_graceful_disconnect() {
        let broker = Arc::new(Mutex::new(BrokerState::default()));
        let (mut subscriber, stask) = wire_client(&broker, None, 0).await;
        subscribe(&mut subscriber, "will/#").await;
        let will = rumqttc::LastWill::new("will/offline", vec![0, 255], QoS::AtLeastOnce, true);
        let (peer, ptask) = wire_client(&broker, Some(will.clone()), 0).await;
        drop(peer);
        assert!(
            matches!(receive(&mut subscriber).await, Packet::Publish(value) if value.topic == "will/offline" && value.payload.as_ref() == [0,255])
        );
        assert!(broker.lock().await.retained.contains_key("will/offline"));
        let (mut graceful, gtask) = wire_client(&broker, Some(will), 0).await;
        write_mqtt_packet(&mut graceful, Packet::Disconnect)
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                read_mqtt_packet(&mut subscriber)
            )
            .await
            .is_err()
        );
        let timeout_will = rumqttc::LastWill::new("will/timeout", vec![1], QoS::AtMostOnce, false);
        let (_idle, itask) = wire_client(&broker, Some(timeout_will), 1).await;
        assert!(
            matches!(receive(&mut subscriber).await, Packet::Publish(value) if value.topic == "will/timeout")
        );
        broker.lock().await.stopping = true;
        shutdown_mqtt_clients(vec![stask, ptask, gtask, itask]).await;
        assert!(broker.lock().await.clients.is_empty());
    }

    #[tokio::test]
    async fn mqtt_broker_rejects_unsupported_qos_and_oversize_before_allocation() {
        let mut connect = rumqttc::Connect::new("test");
        connect.clean_session = false;
        assert_eq!(
            reject_connect(&connect),
            Some(ConnectReturnCode::ServiceUnavailable)
        );
        connect.clean_session = true;
        connect.last_will = Some(rumqttc::LastWill::new(
            "test",
            vec![],
            QoS::ExactlyOnce,
            false,
        ));
        assert_eq!(
            reject_connect(&connect),
            Some(ConnectReturnCode::ServiceUnavailable)
        );
        let broker = Arc::new(Mutex::new(BrokerState::default()));
        let (mut peer, task) = wire_client(&broker, None, 0).await;
        let mut unsupported = rumqttc::Subscribe::new("no/qos2", QoS::ExactlyOnce);
        unsupported.pkid = 9;
        write_mqtt_packet(&mut peer, Packet::Subscribe(unsupported))
            .await
            .unwrap();
        assert!(
            matches!(receive(&mut peer).await, Packet::SubAck(ack) if ack.return_codes == [SubscribeReasonCode::Failure])
        );
        let mut publish = Publish::new("no/qos2", QoS::ExactlyOnce, vec![0]);
        publish.pkid = 8;
        write_mqtt_packet(&mut peer, Packet::Publish(publish))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), read_mqtt_packet(&mut peer))
                .await
                .unwrap()
                .unwrap()
                .is_none()
        );
        shutdown_mqtt_clients(vec![task]).await;
        let (mut source, mut destination) = tokio::io::duplex(64);
        source.write_all(&[0x30, 0x81, 0x80, 0x40]).await.unwrap();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                read_mqtt_packet(&mut destination)
            )
            .await
            .unwrap()
            .is_err()
        );
    }

    #[tokio::test]
    async fn mqtt_broker_bounds_slow_consumers_and_retained_storage() {
        let (tx, _rx) = mpsc::channel(1);
        let (close, _) = watch::channel(false);
        let mut state = BrokerState::default();
        state.clients.insert(
            1,
            BrokerClient {
                client_id: "slow".into(),
                tx,
                close: close.clone(),
                filters: HashMap::from([("#".into(), QoS::AtMostOnce)]),
                pending: HashSet::new(),
                next_pkid: 0,
                publications: VecDeque::new(),
                publication_bytes: 0,
            },
        );
        let publish = Publish::new("test", QoS::AtMostOnce, vec![1]);
        for _ in 0..=MAX_QUEUED_PUBLICATIONS {
            assert!(state.publish(&publish));
        }
        assert!(*close.borrow());
        for index in 0..8 {
            let mut retained = Publish::new(
                format!("retained/{index}"),
                QoS::AtMostOnce,
                vec![1; MAX_PACKET - 1024],
            );
            retained.retain = true;
            assert!(state.publish(&retained));
        }
        let mut rejected = Publish::new("retained/overflow", QoS::AtMostOnce, vec![1; MAX_PACKET]);
        rejected.retain = true;
        assert!(!state.publish(&rejected));
        assert!(topic_matches("a/#", "a"));
        assert!(topic_matches("a/+", "a/"));
        assert!(!topic_matches("#", "$SYS/broker"));
        assert!(topic_matches("$SYS/#", "$SYS/broker"));
        assert!(!valid_filter("a/#/b"));
        assert!(!valid_filter("a+b"));
    }

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
