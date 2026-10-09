#![cfg(feature = "execute")]

use flow_like_industrial::sparkplug::{
    EdgeAction, EdgeState, ObservedNode, PrepareSession, WireMessage, decode,
};
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Outgoing, Packet, Publish, QoS};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::{sync::mpsc, task::JoinHandle};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn group() -> String {
    format!(
        "flow-like-e2e-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

fn qos(value: u8) -> QoS {
    match value {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        2 => QoS::ExactlyOnce,
        _ => panic!("invalid QoS from Sparkplug encoder"),
    }
}

// Sparkplug codec/state APIs compose with a separate MQTT transport in the product.
// This harness exercises those production APIs against Tahu over a real broker.
struct Transport {
    client: AsyncClient,
    incoming: mpsc::Receiver<Publish>,
    subscribed: mpsc::Receiver<()>,
    task: JoinHandle<()>,
}

impl Transport {
    async fn connect(id: String, will: Option<&WireMessage>) -> Self {
        let host = std::env::var("FLOW_LIKE_SPARKPLUG_MQTT_HOST")
            .expect("Set FLOW_LIKE_SPARKPLUG_MQTT_HOST to the dedicated Mosquitto fixture");
        let port = std::env::var("FLOW_LIKE_SPARKPLUG_MQTT_PORT")
            .expect("Set FLOW_LIKE_SPARKPLUG_MQTT_PORT to the dedicated Mosquitto fixture")
            .parse::<u16>()
            .unwrap();
        let mut options = MqttOptions::new(id, host, port);
        options.set_clean_session(true);
        options.set_keep_alive(Duration::from_secs(5));
        if let Some(will) = will {
            options.set_last_will(LastWill::new(
                &will.topic,
                will.payload.clone(),
                qos(will.qos),
                will.retain,
            ));
        }
        let (client, mut events) = AsyncClient::new(options, 32);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(5), events.poll())
                .await
                .unwrap()
                .unwrap(),
            Event::Incoming(Packet::ConnAck(_))
        ));
        let (messages, incoming) = mpsc::channel(1024);
        let (acks, subscribed) = mpsc::channel(8);
        let task =
            tokio::spawn(async move {
                loop {
                    match events
                        .poll()
                        .await
                        .expect("Sparkplug test MQTT transport failed")
                    {
                        Event::Incoming(Packet::Publish(message)) => {
                            messages.send(message).await.unwrap()
                        }
                        Event::Incoming(Packet::SubAck(ack)) => {
                            assert!(ack.return_codes.iter().all(|code| matches!(
                                code,
                                rumqttc::SubscribeReasonCode::Success(_)
                            )));
                            acks.send(()).await.unwrap();
                        }
                        Event::Outgoing(Outgoing::Disconnect) => break,
                        _ => {}
                    }
                }
            });
        Self {
            client,
            incoming,
            subscribed,
            task,
        }
    }

    async fn subscribe(&mut self, topic: String) {
        self.client
            .subscribe(topic, QoS::AtLeastOnce)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), self.subscribed.recv())
            .await
            .unwrap()
            .unwrap();
    }

    async fn publish(&self, message: &WireMessage) {
        self.client
            .publish(
                &message.topic,
                qos(message.qos),
                message.retain,
                message.payload.clone(),
            )
            .await
            .unwrap();
    }

    async fn receive(&mut self) -> WireMessage {
        let message = tokio::time::timeout(Duration::from_secs(5), self.incoming.recv())
            .await
            .expect("Tahu publication deadline")
            .expect("MQTT transport closed");
        WireMessage {
            topic: message.topic,
            payload: message.payload.to_vec(),
            qos: message.qos as u8,
            retain: message.retain,
        }
    }

    async fn disconnect(&mut self) {
        self.client.disconnect().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), &mut self.task)
            .await
            .unwrap()
            .unwrap();
    }

    async fn abort(&mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct Oracle {
    url: String,
    http: reqwest::Client,
}

impl Oracle {
    async fn new() -> Self {
        let oracle = Self {
            url: std::env::var("FLOW_LIKE_SPARKPLUG_ORACLE_URL")
                .expect("Set FLOW_LIKE_SPARKPLUG_ORACLE_URL to the Eclipse Tahu fixture"),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
        };
        let health: Value = oracle
            .http
            .get(format!("{}/health", oracle.url))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(health["ready"], true);
        oracle
    }

    async fn action(&self, action: &str, group: &str, node: &str) {
        let response = self
            .http
            .post(format!("{}{action}", self.url))
            .json(&json!({"group":group,"node":node}))
            .send()
            .await
            .unwrap();
        let status = response.status();
        let body: Value = response.json().await.unwrap();
        assert!(status.is_success(), "Tahu action {action}: {body}");
    }

    async fn state(&self, group: &str, node: &str) -> Value {
        self.http
            .get(format!("{}/state", self.url))
            .query(&[("group", group), ("node", node)])
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    async fn wait(&self, group: &str, node: &str, condition: impl Fn(&Value) -> bool) -> Value {
        let mut latest = Value::Null;
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                latest = self.state(group, node).await;
                if condition(&latest) {
                    return latest.clone();
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("Tahu observation deadline: {latest}"))
    }
}

fn assert_valid(state: &Value) {
    assert_eq!(
        state["errors"],
        json!([]),
        "independent Tahu host validation: {state}"
    );
}

async fn transition(transport: &Transport, state: EdgeState, action: EdgeAction) -> EdgeState {
    let result = state.transition(action, now()).unwrap();
    for message in &result.messages {
        transport.publish(message).await;
    }
    result.state
}

fn prepare(group: &str, previous_bd_seq: u8) -> flow_like_industrial::sparkplug::PreparedSession {
    EdgeState::prepare(PrepareSession {
        group_id: group.into(), node_id: "rust-edge".into(), previous_bd_seq, timestamp_ms: now(),
        metrics: vec![
            json!({"name":"temperature","alias":"10","datatype":10,"doubleValue":20.5}),
            json!({"name":"label","alias":"11","datatype":12,"stringValue":"温度センサー"}),
            json!({"name":"counter","alias":"12","datatype":8,"longValue":"18446744073709551615"}),
            json!({"name":"raw","alias":"13","datatype":17,"bytesValue":"AAH/gA=="}),
            json!({"name":"table","alias":"14","datatype":16,"datasetValue":{"numOfColumns":"2","columns":["name","value"],"types":[12,10],"rows":[{"elements":[{"stringValue":"pump"},{"doubleValue":12.25}]}]}}),
            json!({"name":"file","alias":"15","datatype":18,"bytesValue":"AQID"}),
        ],
    }).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Mosquitto and the Eclipse Tahu reference peer fixtures"]
async fn sparkplug_tahu_validates_rust_birth_alias_sequence_rebirth_and_death() {
    let oracle = Oracle::new().await;
    let group = group();
    let prepared = prepare(&group, 254);
    let old_death = prepared.last_will.clone();
    let mut edge = Transport::connect(format!("rust-{group}"), Some(&prepared.last_will)).await;
    edge.subscribe(format!("spBv1.0/{group}/NCMD/rust-edge"))
        .await;
    let mut state = transition(&edge, prepared.state, EdgeAction::Birth).await;
    state = transition(
        &edge,
        state,
        EdgeAction::DeviceBirth {
            device_id: "device".into(),
            metrics: vec![json!({"name":"running","alias":"20","datatype":11,"booleanValue":true})],
        },
    )
    .await;
    state = transition(
        &edge,
        state,
        EdgeAction::Data {
            device_id: None,
            metrics: vec![json!({"alias":"10","doubleValue":23.75})],
        },
    )
    .await;
    state = transition(
        &edge,
        state,
        EdgeAction::Data {
            device_id: Some("device".into()),
            metrics: vec![json!({"alias":"20","booleanValue":false})],
        },
    )
    .await;
    for _ in 0..253 {
        state = transition(
            &edge,
            state,
            EdgeAction::Data {
                device_id: None,
                metrics: vec![json!({"alias":"10","doubleValue":42.5})],
            },
        )
        .await;
    }
    let observed = oracle
        .wait(&group, "rust-edge", |v| {
            v["events"].as_array().unwrap().len() >= 257
        })
        .await;
    assert_valid(&observed);
    assert_eq!(observed["bd_seq"], 255);
    assert_eq!(observed["next_seq"], 1);
    assert_eq!(observed["metrics"]["temperature"]["doubleValue"], 42.5);
    assert_eq!(observed["metrics"]["label"]["stringValue"], "温度センサー");
    assert_eq!(
        observed["metrics"]["counter"]["longValue"],
        "18446744073709551615"
    );
    assert_eq!(observed["metrics"]["raw"]["bytesValue"], "AAH/gA==");
    assert_eq!(observed["metrics"]["file"]["bytesValue"], "AQID");
    assert_eq!(
        observed["metrics"]["table"]["datasetValue"]["rows"][0]["elements"][1]["doubleValue"],
        12.25
    );
    assert_eq!(
        observed["devices"]["device"]["metrics"]["running"]["booleanValue"],
        false
    );

    oracle.action("/rebirth-command", &group, "rust-edge").await;
    let command = decode(&edge.receive().await).unwrap();
    assert_eq!(command.topic, format!("spBv1.0/{group}/NCMD/rust-edge"));
    assert_eq!(
        command.payload["metrics"][0]["name"],
        "Node Control/Rebirth"
    );
    assert_eq!(command.payload["metrics"][0]["booleanValue"], true);
    state = transition(&edge, state, EdgeAction::Birth).await;
    let observed = oracle
        .wait(&group, "rust-edge", |v| {
            v["births"] == 2 && v["next_seq"] == 2
        })
        .await;
    assert_valid(&observed);
    assert_eq!(observed["metrics"]["temperature"]["doubleValue"], 42.5);
    assert_eq!(
        observed["devices"]["device"]["metrics"]["running"]["booleanValue"],
        false
    );
    state = transition(
        &edge,
        state,
        EdgeAction::DeviceDeath {
            device_id: "device".into(),
        },
    )
    .await;
    let observed = oracle
        .wait(&group, "rust-edge", |v| {
            v["devices"]["device"]["online"] == false
        })
        .await;
    assert_valid(&observed);
    assert!(!state.devices.contains_key("device"));
    edge.abort().await;
    let observed = oracle.wait(&group, "rust-edge", |v| v["deaths"] == 1).await;
    assert_valid(&observed);
    assert_eq!(observed["online"], false);

    let prepared = prepare(&group, 255);
    let mut reconnected = Transport::connect(
        format!("rust-reconnected-{group}"),
        Some(&prepared.last_will),
    )
    .await;
    let state = transition(&reconnected, prepared.state, EdgeAction::Birth).await;
    let observed = oracle.wait(&group, "rust-edge", |v| v["births"] == 3).await;
    assert_valid(&observed);
    assert_eq!(observed["bd_seq"], 0);
    assert_eq!(observed["online"], true);
    reconnected.publish(&old_death).await;
    let observed = oracle
        .wait(&group, "rust-edge", |v| v["stale_deaths"] == 1)
        .await;
    assert_valid(&observed);
    assert_eq!(observed["online"], true);
    let _ = transition(&reconnected, state, EdgeAction::Death).await;
    let observed = oracle.wait(&group, "rust-edge", |v| v["deaths"] == 3).await;
    assert_valid(&observed);
    assert_eq!(observed["online"], false);
    reconnected.disconnect().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Mosquitto and the Eclipse Tahu reference peer fixtures"]
async fn sparkplug_decodes_tahu_and_detects_sequence_gap_rebirth_and_broker_will() {
    let oracle = Oracle::new().await;
    let group = group();
    let node = "tahu-edge";
    let mut observer = Transport::connect(format!("observer-{group}"), None).await;
    observer.subscribe(format!("spBv1.0/{group}/#")).await;
    oracle.action("/reference/start", &group, node).await;
    let mut state = ObservedNode::default();
    for kind in ["NBIRTH", "DBIRTH", "NDATA", "DDATA"] {
        let wire = observer.receive().await;
        assert!(wire.topic.contains(&format!("/{kind}/")), "{}", wire.topic);
        assert_eq!(wire.qos, 0);
        assert!(!wire.retain);
        let message = decode(&wire).unwrap();
        if kind == "NBIRTH" {
            let metrics = message.payload["metrics"].as_array().unwrap();
            assert_eq!(
                metrics.iter().find(|m| m["name"] == "label").unwrap()["stringValue"],
                "温度センサー"
            );
            assert_eq!(
                metrics.iter().find(|m| m["name"] == "raw").unwrap()["bytesValue"],
                "AP+A"
            );
        } else if kind == "NDATA" {
            assert_eq!(message.payload["metrics"][0]["alias"], "10");
            assert!(message.payload["metrics"][0].get("name").is_none());
            assert_eq!(message.payload["metrics"][0]["doubleValue"], 22.75);
        }
        state = state.apply(&message).unwrap();
        assert!(state.online);
        assert!(!state.needs_rebirth);
    }
    assert_eq!(state.bd_seq, Some(41));
    assert_eq!(state.next_sequence, Some(4));
    oracle.action("/reference/gap", &group, node).await;
    state = state
        .apply(&decode(&observer.receive().await).unwrap())
        .unwrap();
    assert!(state.needs_rebirth);
    oracle.action("/reference/rebirth", &group, node).await;
    for _ in 0..2 {
        state = state
            .apply(&decode(&observer.receive().await).unwrap())
            .unwrap();
        assert!(!state.needs_rebirth);
    }
    assert_eq!(state.next_sequence, Some(2));
    oracle.action("/reference/stale-death", &group, node).await;
    state = state
        .apply(&decode(&observer.receive().await).unwrap())
        .unwrap();
    assert!(state.online);
    oracle.action("/reference/abort", &group, node).await;
    let death = observer.receive().await;
    assert_eq!(death.qos, 1);
    assert!(death.topic.contains("/NDEATH/"));
    state = state.apply(&decode(&death).unwrap()).unwrap();
    assert!(!state.online);
    assert_eq!(state.next_sequence, None);
    observer.disconnect().await;
}
