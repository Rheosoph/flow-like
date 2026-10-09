use super::{
    MqttConfig, MqttQoS,
    topic::{topic_matches, valid_filter},
};
use flow_like::flow::execution::{ExecutionEnvironment, context::ExecutionContext, egress};
use flow_like_types::{Cacheable, Result, anyhow, tokio_util::sync::CancellationToken};
use rumqttc::{AsyncClient, Event, EventLoop, MqttOptions, Outgoing, Packet, Publish, QoS};
use std::{
    any::Any,
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{broadcast, mpsc, oneshot, watch};

const CAPACITY: usize = 32;
const MAX_PAYLOAD: usize = 1024 * 1024;
type Reply = oneshot::Sender<Result<()>>;
enum Operation {
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: QoS,
        retain: bool,
    },
    Subscribe(String, QoS, u64),
    Unsubscribe(String, u64),
    Disconnect,
}
struct Command {
    operation: Operation,
    reply: Reply,
}
#[derive(Clone)]
enum CloseReason {
    Normal,
    Failed(String),
}
impl CloseReason {
    fn message(&self) -> String {
        match self {
            Self::Normal => "MQTT connection closed".into(),
            Self::Failed(error) => error.clone(),
        }
    }
}
#[derive(Clone)]
pub struct CachedMqttConnection {
    pub connection: Arc<MqttConnection>,
}
impl Cacheable for CachedMqttConnection {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
pub struct MqttSubscription {
    receiver: broadcast::Receiver<Publish>,
    replay: VecDeque<Publish>,
    connection: std::sync::Weak<MqttConnection>,
    filter: Option<(String, u64)>,
}
impl MqttSubscription {
    pub async fn recv(&mut self) -> std::result::Result<Publish, broadcast::error::RecvError> {
        if let Some(message) = self.replay.pop_front() {
            return Ok(message);
        }
        self.receiver.recv().await
    }
    pub async fn close(mut self) -> Result<()> {
        if let Some(connection) = self.connection.upgrade()
            && let Some((filter, id)) = self.filter.as_ref()
        {
            connection.unsubscribe(filter.clone(), *id).await?;
        }
        self.unregister();
        self.filter = None;
        Ok(())
    }
}
impl MqttSubscription {
    fn unregister(&self) {
        if let Some(connection) = self.connection.upgrade()
            && let Some((_, id)) = self.filter.as_ref()
            && let Ok(mut messages) = connection.messages.lock()
        {
            messages.listeners.remove(id);
        }
    }
}
impl Drop for MqttSubscription {
    fn drop(&mut self) {
        self.unregister();
        let Some((filter, id)) = self.filter.take() else {
            return;
        };
        let Some(connection) = self.connection.upgrade() else {
            return;
        };
        if connection.stop.is_cancelled() {
            return;
        }
        // Cleanup is idempotent per listener and must survive a dropped reply receiver.
        let (reply, _) = oneshot::channel();
        if connection
            .commands
            .try_send(Command {
                operation: Operation::Unsubscribe(filter, id),
                reply,
            })
            .is_err()
        {
            tracing::warn!("MQTT cleanup queue full or closed; closing the session");
            connection.stop.cancel();
        }
    }
}
// Persistent sessions can replay several filters before workflow listeners attach.
// MQTT 3.1.1 exposes neither the restored filter set nor a replay-complete marker.
// Keep unmatched deliveries until the first matching listener claims them. An overlapping
// listener attached afterward starts with live messages; this queue is not a history log.
struct MessageHub {
    replay: VecDeque<Publish>,
    listeners: HashMap<u64, (String, broadcast::Sender<Publish>)>,
    buffer_unmatched: bool,
}
impl MessageHub {
    fn new(buffer_unmatched: bool) -> Self {
        Self {
            replay: VecDeque::new(),
            listeners: HashMap::new(),
            buffer_unmatched,
        }
    }
    fn attach(
        &mut self,
        filter: &str,
        id: u64,
    ) -> (broadcast::Receiver<Publish>, VecDeque<Publish>) {
        let (sender, receiver) = broadcast::channel(CAPACITY);
        let mut replay = VecDeque::new();
        self.replay.retain(|message| {
            if topic_matches(filter, &message.topic) {
                replay.push_back(message.clone());
                false
            } else {
                true
            }
        });
        self.listeners.insert(id, (filter.to_owned(), sender));
        (receiver, replay)
    }
    fn deliver(&mut self, message: Publish) -> Result<()> {
        let mut matched = false;
        for (filter, sender) in self.listeners.values() {
            if topic_matches(filter, &message.topic) {
                matched = true;
                let _ = sender.send(message.clone());
            }
        }
        if !matched && self.buffer_unmatched {
            if self.replay.len() == CAPACITY {
                return Err(anyhow!(
                    "MQTT pending listener queue overflowed; attach all persistent-session listeners before replay exceeds {CAPACITY} messages"
                ));
            }
            self.replay.push_back(message);
        }
        Ok(())
    }
}
pub struct MqttConnection {
    commands: mpsc::Sender<Command>,
    next_subscription: std::sync::atomic::AtomicU64,
    messages: Arc<std::sync::Mutex<MessageHub>>,
    status: watch::Receiver<Option<CloseReason>>,
    stop: CancellationToken,
}
impl Drop for MqttConnection {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
impl MqttConnection {
    pub async fn connect(
        config: &MqttConfig,
        environment: ExecutionEnvironment,
        cancellation: Option<CancellationToken>,
    ) -> Result<Arc<Self>> {
        if config.client_id.is_empty()
            || config.client_id.len() > u16::MAX as usize
            || config.client_id.contains('\0')
        {
            return Err(anyhow!(
                "MQTT client ID must contain 1 to 65535 bytes without NUL"
            ));
        }
        if config.keep_alive_seconds > u16::MAX as u64
            || (config.keep_alive_seconds > 0 && config.keep_alive_seconds < 5)
        {
            return Err(anyhow!(
                "MQTT keep alive must be zero or between 5 and 65535 seconds"
            ));
        }
        if !(1..=300).contains(&config.connect_timeout_seconds) {
            return Err(anyhow!(
                "MQTT connection timeout must be between 1 and 300 seconds"
            ));
        }
        if config.username.is_none() && config.password.is_some() {
            return Err(anyhow!("MQTT password requires a username"));
        }
        for value in [&config.username, &config.password].into_iter().flatten() {
            if value.len() > u16::MAX as usize || value.contains('\0') {
                return Err(anyhow!(
                    "MQTT credentials must contain at most 65535 bytes without NUL"
                ));
            }
        }
        if secure_name_override(config) {
            return Err(anyhow!("MQTT TLS server_name must match the broker host"));
        }
        let addresses = tokio::select! {
            _ = crate::web::wait_for_cancel(cancellation.clone()) => return Err(anyhow!("MQTT connection cancelled")),
            result = tokio::time::timeout(Duration::from_secs(config.connect_timeout_seconds), egress::resolve_socket_addrs(environment, &config.host, config.port)) => result??,
        };
        let secure = config.use_tls || config.tls.secure;
        if secure && config.host.parse::<std::net::Ipv6Addr>().is_ok() {
            return Err(anyhow!(
                "MQTT TLS with an IPv6 literal is unsupported by the transport; use a DNS hostname on a local executor or an IPv4 broker address"
            ));
        }
        if environment == ExecutionEnvironment::Server
            && secure
            && config.host.parse::<std::net::IpAddr>().is_err()
        {
            return Err(anyhow!(
                "TLS MQTT on shared executors requires an explicit broker IP and a certificate valid for that IP; use a local executor for hostname TLS"
            ));
        }
        let host = if environment == ExecutionEnvironment::Server {
            addresses
                .first()
                .ok_or_else(|| anyhow!("MQTT host has no addresses"))?
                .ip()
                .to_string()
        } else {
            config.host.clone()
        };
        let host = if host.contains(':') {
            format!("[{}]", host.trim_matches(['[', ']']))
        } else {
            host
        };
        let mut options = MqttOptions::new(&config.client_id, host, config.port);
        options.set_keep_alive(Duration::from_secs(config.keep_alive_seconds));
        options.set_clean_session(config.clean_session);
        options.set_max_packet_size(MAX_PAYLOAD + 65540, MAX_PAYLOAD + 65540);
        if let Some(user) = &config.username {
            options.set_credentials(user, config.password.as_deref().unwrap_or(""));
        }
        if let Some(will) = &config.last_will {
            validate_topic(&will.topic)?;
            if will.payload.len() > u16::MAX as usize {
                return Err(anyhow!("MQTT 3.1.1 Last Will payload exceeds 65535 bytes"));
            }
            options.set_last_will(rumqttc::LastWill::new(
                &will.topic,
                will.payload.clone(),
                to_rumqttc_qos(&will.qos),
                will.retain,
            ));
        }
        if config.tls.secure {
            let tls = crate::web::tls::client_config(&config.tls)?
                .ok_or_else(|| anyhow!("MQTT TLS configuration is unavailable"))?;
            options.set_transport(rumqttc::Transport::tls_with_config(tls.into()));
        } else if config.use_tls {
            options.set_transport(rumqttc::Transport::tls_with_default_config());
        }
        let (client, mut events) = AsyncClient::new(options, CAPACITY);
        let mut network = rumqttc::NetworkOptions::new();
        network.set_connection_timeout(config.connect_timeout_seconds);
        events.set_network_options(network);
        let session_present = tokio::select! {
            _ = crate::web::wait_for_cancel(cancellation.clone()) => return Err(anyhow!("MQTT connection cancelled")),
            result = tokio::time::timeout(Duration::from_secs(config.connect_timeout_seconds), events.poll()) => {
                match result?? { Event::Incoming(Packet::ConnAck(ack)) => ack.session_present, other => return Err(anyhow!("Expected MQTT CONNACK, received {other:?}")) }
            }
        };
        let stop = cancellation
            .map(|token| token.child_token())
            .unwrap_or_default();
        let (commands, receiver) = mpsc::channel(CAPACITY);
        let messages = Arc::new(std::sync::Mutex::new(MessageHub::new(session_present)));
        let (state, status) = watch::channel(None);
        let connection = Arc::new(Self {
            commands,
            next_subscription: std::sync::atomic::AtomicU64::new(1),
            messages: messages.clone(),
            status,
            stop: stop.clone(),
        });
        tokio::spawn(async move {
            let result = actor(client, events, receiver, messages, stop.clone()).await;
            state.send_replace(Some(match result {
                Ok(()) => CloseReason::Normal,
                Err(error) => CloseReason::Failed(error.to_string()),
            }));
            stop.cancel();
        });
        Ok(connection)
    }
    #[cfg(test)]
    pub(super) async fn wait_for_pending_replay(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.messages.lock().unwrap().replay.len() < count {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("persistent replay must arrive before attaching workflow listeners");
    }
    async fn command(&self, operation: Operation) -> Result<()> {
        if let Some(error) = self.status.borrow().as_ref() {
            return Err(anyhow!(error.message()));
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .try_send(Command { operation, reply })
            .map_err(|_| anyhow!("MQTT command queue is full or closed"))?;
        tokio::select! {
            biased;
            result = tokio::time::timeout(Duration::from_secs(15), response) => match result {
                Ok(result) => result.map_err(|_| anyhow!("MQTT connection closed before confirming the operation"))?,
                Err(_) => { self.stop.cancel(); Err(anyhow!("MQTT confirmation timed out; delivery is uncertain and the session was closed")) }
            },
            _ = self.stop.cancelled() => Err(anyhow!(self.status.borrow().as_ref().map(CloseReason::message).unwrap_or_else(|| "MQTT connection closed".into()))),
        }
    }
    pub async fn publish(
        &self,
        topic: String,
        payload: Vec<u8>,
        qos: QoS,
        retain: bool,
    ) -> Result<()> {
        validate_topic(&topic)?;
        if payload.len() > MAX_PAYLOAD {
            return Err(anyhow!("MQTT message exceeds 1 MiB"));
        }
        self.command(Operation::Publish {
            topic,
            payload,
            qos,
            retain,
        })
        .await
    }
    pub async fn subscribe(self: &Arc<Self>, filter: String, qos: QoS) -> Result<MqttSubscription> {
        if !valid_filter(&filter) {
            return Err(anyhow!("Invalid MQTT topic filter"));
        }
        let id = self
            .next_subscription
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Attach before SUBSCRIBE so deliveries preceding SUBACK are also captured.
        let (receiver, replay) = self
            .messages
            .lock()
            .map_err(|_| anyhow!("MQTT subscriber state poisoned"))?
            .attach(&filter, id);
        let subscription = MqttSubscription {
            receiver,
            replay,
            connection: Arc::downgrade(self),
            filter: Some((filter.clone(), id)),
        };
        self.command(Operation::Subscribe(filter, qos, id)).await?;
        Ok(subscription)
    }
    async fn unsubscribe(&self, filter: String, id: u64) -> Result<()> {
        if self.stop.is_cancelled() {
            return Ok(());
        }
        self.command(Operation::Unsubscribe(filter, id)).await
    }
    pub async fn disconnect(&self) -> Result<()> {
        if self.stop.is_cancelled() {
            return Ok(());
        }
        self.command(Operation::Disconnect).await
    }
    pub async fn closed(&self) -> Result<()> {
        let mut state = self.status.clone();
        loop {
            if let Some(reason) = state.borrow().as_ref() {
                return match reason {
                    CloseReason::Normal => Ok(()),
                    CloseReason::Failed(error) => Err(anyhow!(error.clone())),
                };
            }
            if state.changed().await.is_err() {
                return Err(anyhow!("MQTT connection state unavailable"));
            }
        }
    }
}
fn secure_name_override(config: &MqttConfig) -> bool {
    (config.use_tls || config.tls.secure)
        && config
            .tls
            .server_name
            .as_ref()
            .is_some_and(|name| name != &config.host)
}
fn validate_topic(topic: &str) -> Result<()> {
    if topic.is_empty() || topic.len() > u16::MAX as usize || topic.contains(['#', '+', '\0']) {
        return Err(anyhow!("Invalid MQTT publish topic"));
    }
    Ok(())
}
fn deliver(event: &Event, messages: &std::sync::Mutex<MessageHub>) -> Result<()> {
    if let Event::Incoming(Packet::Publish(message)) = event {
        validate_topic(&message.topic)?;
        if message.payload.len() > MAX_PAYLOAD {
            return Err(anyhow!("MQTT incoming message exceeds 1 MiB"));
        }
        messages
            .lock()
            .map_err(|_| anyhow!("MQTT subscriber state poisoned"))?
            .deliver(message.clone())?;
    }
    Ok(())
}
async fn actor(
    client: AsyncClient,
    mut events: EventLoop,
    mut commands: mpsc::Receiver<Command>,
    messages: Arc<std::sync::Mutex<MessageHub>>,
    stop: CancellationToken,
) -> Result<()> {
    let mut subscriptions = HashMap::<String, (HashSet<u64>, QoS)>::new();
    loop {
        tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            result = events.poll() => deliver(&result?, &messages)?,
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                if command.reply.is_closed() && !matches!(&command.operation, Operation::Unsubscribe(_, _)) { continue; }
                let disconnect = matches!(&command.operation, Operation::Disconnect);
                let result = tokio::select! {
                    _ = stop.cancelled() => Err(anyhow!("MQTT connection cancelled")),
                    result = tokio::time::timeout(Duration::from_secs(10), execute(&client, &mut events, &messages, &mut subscriptions, command.operation)) => result.unwrap_or_else(|_| Err(anyhow!("MQTT acknowledgement timed out; delivery may have occurred"))),
                };
                let failed = result.is_err();
                let error = result.as_ref().err().map(ToString::to_string);
                let _ = command.reply.send(result);
                if failed { return Err(anyhow!(error.unwrap_or_default())); }
                if disconnect { return Ok(()); }
            }
        }
    }
}
async fn execute(
    client: &AsyncClient,
    events: &mut EventLoop,
    messages: &std::sync::Mutex<MessageHub>,
    subscriptions: &mut HashMap<String, (HashSet<u64>, QoS)>,
    operation: Operation,
) -> Result<()> {
    let mut packet_id = None;
    match &operation {
        Operation::Publish {
            topic,
            payload,
            qos,
            retain,
        } => client.try_publish(topic, *qos, *retain, payload.clone())?,
        Operation::Subscribe(filter, qos, _) => {
            let qos = subscriptions
                .get(filter)
                .map(|(_, existing)| {
                    if (*existing as u8) > (*qos as u8) {
                        *existing
                    } else {
                        *qos
                    }
                })
                .unwrap_or(*qos);
            // Resubscribing lets the broker replay retained messages to a newly attached handler.
            client.try_subscribe(filter, qos)?;
        }
        Operation::Unsubscribe(filter, id) => {
            let Some((ids, _)) = subscriptions.get_mut(filter) else {
                return Ok(());
            };
            if !ids.remove(id) || !ids.is_empty() {
                return Ok(());
            }
            client.try_unsubscribe(filter)?;
        }
        Operation::Disconnect => client.try_disconnect()?,
    }
    loop {
        let event = events.poll().await?;
        deliver(&event, messages)?;
        match (&operation, event) {
            (
                Operation::Publish {
                    qos: QoS::AtMostOnce,
                    ..
                },
                Event::Outgoing(Outgoing::Publish(_)),
            ) => return Ok(()),
            (Operation::Publish { .. }, Event::Outgoing(Outgoing::Publish(id))) => {
                packet_id = Some(id)
            }
            (
                Operation::Publish {
                    qos: QoS::AtLeastOnce,
                    ..
                },
                Event::Incoming(Packet::PubAck(ack)),
            ) if Some(ack.pkid) == packet_id => return Ok(()),
            (
                Operation::Publish {
                    qos: QoS::ExactlyOnce,
                    ..
                },
                Event::Incoming(Packet::PubComp(ack)),
            ) if Some(ack.pkid) == packet_id => return Ok(()),
            (Operation::Subscribe(_, _, _), Event::Outgoing(Outgoing::Subscribe(id))) => {
                packet_id = Some(id)
            }
            (Operation::Subscribe(filter, qos, id), Event::Incoming(Packet::SubAck(ack)))
                if Some(ack.pkid) == packet_id =>
            {
                if ack.return_codes.len() != 1 {
                    return Err(anyhow!("MQTT SUBACK must contain exactly one result"));
                }
                if ack
                    .return_codes
                    .iter()
                    .any(|code| matches!(code, rumqttc::SubscribeReasonCode::Failure))
                {
                    return Err(anyhow!("MQTT broker rejected subscription"));
                }
                let entry = subscriptions
                    .entry(filter.clone())
                    .or_insert_with(|| (HashSet::new(), *qos));
                entry.0.insert(*id);
                entry.1 = match ack.return_codes[0] {
                    rumqttc::SubscribeReasonCode::Success(granted) => granted,
                    rumqttc::SubscribeReasonCode::Failure => unreachable!(),
                };
                return Ok(());
            }
            (Operation::Unsubscribe(_, _), Event::Outgoing(Outgoing::Unsubscribe(id))) => {
                packet_id = Some(id)
            }
            (Operation::Unsubscribe(filter, _), Event::Incoming(Packet::UnsubAck(ack)))
                if Some(ack.pkid) == packet_id =>
            {
                subscriptions.remove(filter);
                return Ok(());
            }
            (Operation::Disconnect, Event::Outgoing(Outgoing::Disconnect)) => return Ok(()),
            _ => {}
        }
    }
}
pub async fn get_mqtt_connection(
    context: &ExecutionContext,
    ref_id: &str,
) -> Result<Arc<MqttConnection>> {
    let cached = context
        .get_cache(ref_id)
        .await
        .ok_or_else(|| anyhow!("MQTT connection is closed or unavailable"))?;
    Ok(cached
        .as_any()
        .downcast_ref::<CachedMqttConnection>()
        .ok_or_else(|| anyhow!("Invalid MQTT session"))?
        .connection
        .clone())
}
pub fn to_rumqttc_qos(qos: &MqttQoS) -> QoS {
    match qos {
        MqttQoS::AtMostOnce => QoS::AtMostOnce,
        MqttQoS::AtLeastOnce => QoS::AtLeastOnce,
        MqttQoS::ExactlyOnce => QoS::ExactlyOnce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;
    use flow_like_types::json::json;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
    };

    async fn read(stream: &mut TcpStream, bytes: &mut BytesMut) -> Packet {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                match Packet::read(bytes, MAX_PAYLOAD + 65540) {
                    Ok(packet) => return packet,
                    Err(rumqttc::Error::InsufficientBytes(_)) => {}
                    Err(error) => panic!("Malformed MQTT packet: {error}"),
                }
                assert_ne!(stream.read_buf(bytes).await.unwrap(), 0, "MQTT peer closed");
            }
        })
        .await
        .expect("MQTT packet deadline")
    }
    async fn write(stream: &mut TcpStream, packet: Packet) {
        let mut bytes = BytesMut::new();
        packet.write(&mut bytes, MAX_PAYLOAD + 65540).unwrap();
        stream.write_all(&bytes).await.unwrap();
    }
    fn config(port: u16) -> MqttConfig {
        flow_like_types::json::from_value(json!({"host":"127.0.0.1","port":port,"client_id":"flow-test","connect_timeout_seconds":1})).unwrap()
    }
    async fn accept(listener: TcpListener) -> (TcpStream, BytesMut, rumqttc::Connect) {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = BytesMut::new();
        let Packet::Connect(connect) = read(&mut stream, &mut bytes).await else {
            panic!("Expected CONNECT")
        };
        (stream, bytes, connect)
    }
    async fn accepted(stream: &mut TcpStream) {
        write(
            stream,
            Packet::ConnAck(rumqttc::ConnAck::new(
                rumqttc::ConnectReturnCode::Success,
                false,
            )),
        )
        .await;
    }

    #[tokio::test]
    async fn publishes_without_subscriber_confirms_qos_and_serializes_binary_will() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = config(listener.local_addr().unwrap().port());
        config.clean_session = false;
        config.last_will = Some(super::super::MqttLastWill {
            topic: "spBv1.0/group/NDEATH/node".into(),
            payload: vec![0, 255, 128, 42],
            qos: super::super::MqttQoS::AtLeastOnce,
            retain: false,
        });
        let server = tokio::spawn(async move {
            let (mut stream, mut bytes, connect) = accept(listener).await;
            assert!(!connect.clean_session);
            let will = connect.last_will.unwrap();
            assert_eq!(will.topic, "spBv1.0/group/NDEATH/node");
            assert_eq!(will.message.as_ref(), &[0, 255, 128, 42]);
            assert_eq!(will.qos, QoS::AtLeastOnce);
            assert!(!will.retain);
            accepted(&mut stream).await;
            for qos in [QoS::AtMostOnce, QoS::AtLeastOnce, QoS::ExactlyOnce] {
                let Packet::Publish(publish) = read(&mut stream, &mut bytes).await else {
                    panic!("Expected PUBLISH")
                };
                assert_eq!(publish.topic, "spBv1.0/group/NDATA/node");
                assert_eq!(publish.payload.as_ref(), &[0, 255, 128, 42]);
                assert_eq!(publish.qos, qos);
                match qos {
                    QoS::AtMostOnce => {}
                    QoS::AtLeastOnce => {
                        write(
                            &mut stream,
                            Packet::PubAck(rumqttc::PubAck::new(publish.pkid)),
                        )
                        .await
                    }
                    QoS::ExactlyOnce => {
                        write(
                            &mut stream,
                            Packet::PubRec(rumqttc::PubRec::new(publish.pkid)),
                        )
                        .await;
                        let Packet::PubRel(release) = read(&mut stream, &mut bytes).await else {
                            panic!("Expected PUBREL")
                        };
                        assert_eq!(release.pkid, publish.pkid);
                        write(
                            &mut stream,
                            Packet::PubComp(rumqttc::PubComp::new(publish.pkid)),
                        )
                        .await;
                    }
                }
            }
            assert!(matches!(
                read(&mut stream, &mut bytes).await,
                Packet::Disconnect
            ));
        });
        let connection = MqttConnection::connect(&config, ExecutionEnvironment::Local, None)
            .await
            .unwrap();
        let (reply, response) = oneshot::channel();
        drop(response);
        assert!(
            connection
                .commands
                .send(Command {
                    operation: Operation::Publish {
                        topic: "cancelled".into(),
                        payload: vec![1],
                        qos: QoS::AtMostOnce,
                        retain: false
                    },
                    reply
                })
                .await
                .is_ok()
        );
        for qos in [QoS::AtMostOnce, QoS::AtLeastOnce, QoS::ExactlyOnce] {
            connection
                .publish(
                    "spBv1.0/group/NDATA/node".into(),
                    vec![0, 255, 128, 42],
                    qos,
                    false,
                )
                .await
                .unwrap();
        }
        connection.disconnect().await.unwrap();
        server.await.unwrap();
        assert!(
            connection
                .publish("test".into(), vec![], QoS::AtMostOnce, false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn subscription_receives_binary_and_remains_alive_after_unsubscribe() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = config(listener.local_addr().unwrap().port());
        let server = tokio::spawn(async move {
            let (mut stream, mut bytes, _) = accept(listener).await;
            accepted(&mut stream).await;
            let Packet::Subscribe(subscribe) = read(&mut stream, &mut bytes).await else {
                panic!("Expected SUBSCRIBE")
            };
            assert_eq!(subscribe.filters[0].path, "spBv1.0/#");
            write(
                &mut stream,
                Packet::SubAck(rumqttc::SubAck::new(
                    subscribe.pkid,
                    vec![rumqttc::SubscribeReasonCode::Success(QoS::AtLeastOnce)],
                )),
            )
            .await;
            let mut publish = Publish::new(
                "spBv1.0/group/NDATA/node",
                QoS::AtLeastOnce,
                vec![0, 255, 128, 42],
            );
            publish.pkid = 42;
            write(&mut stream, Packet::Publish(publish)).await;
            let Packet::PubAck(ack) = read(&mut stream, &mut bytes).await else {
                panic!("Expected PUBACK")
            };
            assert_eq!(ack.pkid, 42);
            let Packet::Subscribe(second) = read(&mut stream, &mut bytes).await else {
                panic!("Expected second SUBSCRIBE")
            };
            assert_eq!(second.filters[0].qos, QoS::AtLeastOnce);
            write(
                &mut stream,
                Packet::SubAck(rumqttc::SubAck::new(
                    second.pkid,
                    vec![rumqttc::SubscribeReasonCode::Success(QoS::AtLeastOnce)],
                )),
            )
            .await;
            let Packet::Unsubscribe(unsubscribe) = read(&mut stream, &mut bytes).await else {
                panic!("Expected UNSUBSCRIBE")
            };
            write(
                &mut stream,
                Packet::UnsubAck(rumqttc::UnsubAck::new(unsubscribe.pkid)),
            )
            .await;
            assert!(matches!(
                read(&mut stream, &mut bytes).await,
                Packet::Publish(_)
            ));
            assert!(matches!(
                read(&mut stream, &mut bytes).await,
                Packet::Disconnect
            ));
        });
        let connection = MqttConnection::connect(&config, ExecutionEnvironment::Local, None)
            .await
            .unwrap();
        let mut messages = connection
            .subscribe("spBv1.0/#".into(), QoS::AtLeastOnce)
            .await
            .unwrap();
        let message = tokio::time::timeout(Duration::from_secs(3), messages.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.payload.as_ref(), &[0, 255, 128, 42]);
        assert_eq!(message.topic, "spBv1.0/group/NDATA/node");
        let second = connection
            .subscribe("spBv1.0/#".into(), QoS::AtMostOnce)
            .await
            .unwrap();
        drop(second);
        messages.close().await.unwrap();
        connection
            .publish(
                "test".into(),
                b"still connected".to_vec(),
                QoS::AtMostOnce,
                false,
            )
            .await
            .unwrap();
        connection.disconnect().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn refused_or_missing_connack_fails_connect() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = config(listener.local_addr().unwrap().port());
        let server = tokio::spawn(async move {
            let (mut stream, _, _) = accept(listener).await;
            write(
                &mut stream,
                Packet::ConnAck(rumqttc::ConnAck::new(
                    rumqttc::ConnectReturnCode::NotAuthorized,
                    false,
                )),
            )
            .await;
        });
        assert!(
            MqttConnection::connect(&config, ExecutionEnvironment::Local, None)
                .await
                .is_err()
        );
        server.await.unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = self::config(listener.local_addr().unwrap().port());
        let server = tokio::spawn(async move {
            let (mut stream, _, _) = accept(listener).await;
            let mut byte = [0];
            assert_eq!(stream.read(&mut byte).await.unwrap(), 0);
        });
        assert!(
            MqttConnection::connect(&config, ExecutionEnvironment::Local, None)
                .await
                .is_err()
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn pending_replay_waits_for_each_matching_listener_and_keeps_live_order() {
        let mut hub = MessageHub::new(true);
        hub.deliver(Publish::new("温度/one", QoS::AtLeastOnce, vec![1]))
            .unwrap();
        hub.deliver(Publish::new("pressure/one", QoS::AtLeastOnce, vec![2]))
            .unwrap();
        let (mut temperatures, replay) = hub.attach("温度/+", 1);
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].payload.as_ref(), &[1]);
        assert_eq!(hub.replay.len(), 1);
        hub.deliver(Publish::new("pressure/two", QoS::AtLeastOnce, vec![3]))
            .unwrap();
        let (mut pressures, replay) = hub.attach("pressure/#", 2);
        assert_eq!(
            replay
                .iter()
                .map(|value| value.payload[0])
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert!(hub.replay.is_empty());
        hub.deliver(Publish::new("温度/two", QoS::AtLeastOnce, vec![4]))
            .unwrap();
        assert_eq!(temperatures.recv().await.unwrap().payload.as_ref(), &[4]);
        assert!(matches!(
            pressures.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn overlapping_late_listener_starts_live_after_pending_delivery_was_claimed() {
        let mut hub = MessageHub::new(true);
        hub.deliver(Publish::new("plant/temperature", QoS::AtLeastOnce, vec![1]))
            .unwrap();
        let (mut broad, replay) = hub.attach("plant/#", 1);
        assert_eq!(replay.len(), 1);
        assert_eq!(replay[0].payload.as_ref(), &[1]);
        let (mut specific, replay) = hub.attach("plant/temperature", 2);
        assert!(
            replay.is_empty(),
            "a consumed pending delivery is not retained as history"
        );
        hub.deliver(Publish::new("plant/temperature", QoS::AtLeastOnce, vec![2]))
            .unwrap();
        assert_eq!(broad.recv().await.unwrap().payload.as_ref(), &[2]);
        assert_eq!(specific.recv().await.unwrap().payload.as_ref(), &[2]);
    }

    #[tokio::test]
    async fn busy_topics_do_not_overflow_unrelated_listener_queues() {
        let mut hub = MessageHub::new(false);
        let (mut quiet, _) = hub.attach("quiet/+", 1);
        let (mut busy, _) = hub.attach("busy/+", 2);
        for _ in 0..=CAPACITY {
            hub.deliver(Publish::new("busy/value", QoS::AtLeastOnce, vec![1]))
                .unwrap();
        }
        hub.deliver(Publish::new("quiet/温度", QoS::AtLeastOnce, vec![2]))
            .unwrap();
        assert_eq!(quiet.recv().await.unwrap().payload.as_ref(), &[2]);
        assert!(matches!(
            busy.recv().await,
            Err(broadcast::error::RecvError::Lagged(1))
        ));
    }

    #[test]
    fn pending_replay_reports_overflow_instead_of_silently_dropping_messages() {
        let mut hub = MessageHub::new(true);
        for _ in 0..CAPACITY {
            hub.deliver(Publish::new("offline/topic", QoS::AtLeastOnce, vec![1]))
                .unwrap();
        }
        assert!(
            hub.deliver(Publish::new("offline/topic", QoS::AtLeastOnce, vec![2]))
                .unwrap_err()
                .to_string()
                .contains("pending listener queue overflowed")
        );
        assert_eq!(hub.replay.len(), CAPACITY);
    }

    #[tokio::test]
    async fn cancellation_stops_socket_and_bounded_queues_report_overflow() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = config(listener.local_addr().unwrap().port());
        let server = tokio::spawn(async move {
            let (mut stream, _, _) = accept(listener).await;
            accepted(&mut stream).await;
            let mut byte = [0];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        });
        let token = CancellationToken::new();
        let connection =
            MqttConnection::connect(&config, ExecutionEnvironment::Local, Some(token.clone()))
                .await
                .unwrap();
        token.cancel();
        tokio::time::timeout(Duration::from_secs(2), connection.closed())
            .await
            .unwrap()
            .unwrap();
        server.await.unwrap();
        let (commands, _receiver) = mpsc::channel(CAPACITY);
        let messages = Arc::new(std::sync::Mutex::new(MessageHub::new(false)));
        let (mut subscriber, _) = messages.lock().unwrap().attach("test", 1);
        let (_status, status) = watch::channel(None);
        let connection = MqttConnection {
            commands,
            next_subscription: std::sync::atomic::AtomicU64::new(1),
            messages,
            status,
            stop: CancellationToken::new(),
        };
        for _ in 0..CAPACITY {
            let (reply, _response) = oneshot::channel();
            assert!(
                connection
                    .commands
                    .try_send(Command {
                        operation: Operation::Disconnect,
                        reply
                    })
                    .is_ok()
            );
        }
        assert!(
            connection
                .publish("test".into(), vec![], QoS::AtMostOnce, false)
                .await
                .unwrap_err()
                .to_string()
                .contains("queue is full")
        );
        for _ in 0..=CAPACITY {
            connection
                .messages
                .lock()
                .unwrap()
                .deliver(Publish::new("test", QoS::AtMostOnce, vec![1]))
                .unwrap();
        }
        assert!(matches!(
            subscriber.recv().await,
            Err(broadcast::error::RecvError::Lagged(1))
        ));
    }
}
