use crate::{Error, Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sparkplug_rs::protobuf::{Message, MessageFull};
use std::collections::{BTreeMap, BTreeSet};

const MAX_PAYLOAD: usize = 1024 * 1024;
const METRIC_VALUES: &[&str] = &[
    "intValue",
    "longValue",
    "floatValue",
    "doubleValue",
    "booleanValue",
    "stringValue",
    "bytesValue",
    "datasetValue",
    "templateValue",
    "extensionValue",
];

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct WireMessage {
    pub topic: String,
    pub payload: Vec<u8>,
    pub qos: u8,
    pub retain: bool,
}

/// Metric fields use the Sparkplug protobuf JSON mapping, preserving all supported datatypes.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Payload {
    pub topic: String,
    pub payload: Value,
}

pub fn validate_topic(topic: &str) -> Result<()> {
    require(
        topic.len() <= 65535 && !topic.contains(['\0', '#', '+']),
        "Invalid Sparkplug topic",
    )?;
    let parts: Vec<_> = topic.split('/').collect();
    require(
        parts.iter().all(|part| !part.is_empty()),
        "Sparkplug topic identifiers cannot be empty",
    )?;
    let valid = match parts.as_slice() {
        ["spBv1.0", "STATE", _] => true,
        ["spBv1.0", _, kind, _] => matches!(*kind, "NBIRTH" | "NDEATH" | "NDATA" | "NCMD"),
        ["spBv1.0", _, kind, _, _] => matches!(*kind, "DBIRTH" | "DDEATH" | "DDATA" | "DCMD"),
        _ => false,
    };
    require(valid, "Expected a Sparkplug B node, device, or STATE topic")
}

pub fn encode(input: &Payload) -> Result<WireMessage> {
    validate_topic(&input.topic)?;
    let state = input.topic.starts_with("spBv1.0/STATE/");
    let text = serde_json::to_string(&input.payload).map_err(|e| Error::Other(e.into()))?;
    require(text.len() <= MAX_PAYLOAD, "Sparkplug payload exceeds 1 MiB")?;
    let bytes = if state {
        require(
            input
                .payload
                .get("online")
                .and_then(Value::as_bool)
                .is_some()
                && input
                    .payload
                    .get("timestamp")
                    .and_then(Value::as_u64)
                    .is_some(),
            "STATE requires online and timestamp",
        )?;
        text.into_bytes()
    } else {
        protobuf_json_mapping::parse_from_str::<sparkplug_rs::Payload>(&text)
            .map_err(|e| Error::Invalid(format!("Invalid Sparkplug payload: {e}")))?
            .write_to_bytes()
            .map_err(|e| Error::Invalid(format!("Sparkplug encoding failed: {e}")))?
    };
    require(
        bytes.len() <= MAX_PAYLOAD,
        "Sparkplug payload exceeds 1 MiB",
    )?;
    Ok(WireMessage {
        topic: input.topic.clone(),
        payload: bytes,
        qos: if state || input.topic.contains("/NDEATH/") {
            1
        } else {
            0
        },
        retain: state,
    })
}

pub fn decode(message: &WireMessage) -> Result<Payload> {
    validate_topic(&message.topic)?;
    require(
        message.payload.len() <= MAX_PAYLOAD,
        "Sparkplug payload exceeds 1 MiB",
    )?;
    let payload = if message.topic.starts_with("spBv1.0/STATE/") {
        let value: Value =
            serde_json::from_slice(&message.payload).map_err(|e| Error::Other(e.into()))?;
        require(
            value.get("online").and_then(Value::as_bool).is_some()
                && value.get("timestamp").and_then(Value::as_u64).is_some(),
            "Invalid STATE payload",
        )?;
        value
    } else {
        let value = sparkplug_rs::Payload::parse_from_bytes(&message.payload)
            .map_err(|e| Error::Invalid(format!("Invalid Sparkplug bytes: {e}")))?;
        let text = protobuf_json_mapping::print_to_string(&value)
            .map_err(|e| Error::Invalid(format!("Sparkplug JSON conversion failed: {e}")))?;
        serde_json::from_str(&text).map_err(|e| Error::Other(e.into()))?
    };
    Ok(Payload {
        topic: message.topic.clone(),
        payload,
    })
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct EdgeState {
    pub group_id: String,
    pub node_id: String,
    pub bd_seq: u8,
    pub next_sequence: u8,
    pub born: bool,
    pub node_metrics: Vec<Value>,
    pub devices: BTreeMap<String, Vec<Value>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PrepareSession {
    pub group_id: String,
    pub node_id: String,
    /// Persist the returned bd_seq and supply it here on the next MQTT connection.
    pub previous_bd_seq: u8,
    pub timestamp_ms: u64,
    pub metrics: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PreparedSession {
    pub state: EdgeState,
    /// Install this MQTT Last Will before connecting with clean_session=true.
    pub last_will: WireMessage,
}

fn identifier(value: &str) -> Result<()> {
    require(
        !value.is_empty() && value.len() <= 1024 && !value.contains(['/', '#', '+', '\0']),
        "Invalid Sparkplug identifier",
    )
}

fn normalize_metrics(metrics: &mut [Value]) -> Result<()> {
    // Protobuf accepts both spellings. Canonicalize before validation and state merging.
    let descriptor = sparkplug_rs::payload::Metric::descriptor();
    for metric in metrics {
        let object = metric
            .as_object_mut()
            .ok_or_else(|| Error::Invalid("Expected metric object".into()))?;
        for field in descriptor.fields() {
            if field.name() != field.json_name()
                && let Some(value) = object.remove(field.name())
            {
                require(
                    !object.contains_key(field.json_name()),
                    "Metric contains both protobuf and JSON spellings of a field",
                )?;
                object.insert(field.json_name().to_owned(), value);
            }
        }
    }
    Ok(())
}

fn metric_definitions(metrics: &mut [Value]) -> Result<()> {
    require(
        metrics.len() <= 4096,
        "A birth can contain at most 4096 metrics",
    )?;
    normalize_metrics(metrics)?;
    let mut names = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    for metric in metrics {
        metric_value(metric, true)?;
        let name = metric
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Invalid("Birth metrics require names".into()))?;
        require(
            !name.is_empty() && names.insert(name),
            "Metric names must be nonempty and unique",
        )?;
        let datatype = number(metric.get("datatype"))
            .ok_or_else(|| Error::Invalid("Birth metrics require datatypes".into()))?;
        metric_datatype(metric, datatype)?;
        if let Some(alias) = metric.get("alias") {
            let alias =
                number(Some(alias)).ok_or_else(|| Error::Invalid("Invalid metric alias".into()))?;
            require(
                aliases.insert(alias),
                "Metric aliases must be unique within a birth",
            )?;
        }
    }
    Ok(())
}

fn metric_value(metric: &Value, required: bool) -> Result<()> {
    let count = METRIC_VALUES
        .iter()
        .filter(|key| metric.get(**key).is_some())
        .count();
    let null = metric.get("isNull") == Some(&Value::Bool(true));
    require(
        count <= 1 && !(null && count != 0),
        "A metric requires one value or isNull=true, never both",
    )?;
    require(
        !required || null || count == 1,
        "Birth metrics require a current value or isNull=true",
    )
}

fn metric_datatype(metric: &Value, datatype: u64) -> Result<()> {
    // Sparkplug 3.0 section 6.4.17 maps metric datatypes to protobuf value fields.
    // PropertySet (20) and PropertySetList (21) are only PropertyValue types.
    let expected = match datatype {
        1..=3 | 5..=7 => "intValue",
        4 | 8 | 13 => "longValue",
        9 => "floatValue",
        10 => "doubleValue",
        11 => "booleanValue",
        12 | 14 | 15 => "stringValue",
        16 => "datasetValue",
        17 | 18 | 22..=34 => "bytesValue",
        19 => "templateValue",
        _ => {
            return Err(Error::Invalid(
                "Unsupported Sparkplug metric datatype".into(),
            ));
        }
    };
    require(
        METRIC_VALUES
            .iter()
            .all(|key| *key == expected || metric.get(*key).is_none()),
        "Metric value field does not match its birth datatype",
    )
}

fn timestamp_metrics(metrics: &mut [Value], timestamp: u64) {
    for metric in metrics {
        if let Some(object) = metric.as_object_mut() {
            object
                .entry("timestamp")
                .or_insert_with(|| json!(timestamp.to_string()));
        }
    }
}

fn unique_node_aliases(state: &EdgeState) -> Result<()> {
    let mut aliases = BTreeSet::new();
    for metric in state
        .node_metrics
        .iter()
        .chain(state.devices.values().flatten())
    {
        if let Some(alias) = number(metric.get("alias")) {
            require(
                aliases.insert(alias),
                "Aliases must be unique across the edge node and all of its devices",
            )?;
        }
    }
    Ok(())
}

fn number(value: Option<&Value>) -> Option<u64> {
    value.and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}

impl EdgeState {
    pub fn prepare(mut input: PrepareSession) -> Result<PreparedSession> {
        identifier(&input.group_id)?;
        identifier(&input.node_id)?;
        metric_definitions(&mut input.metrics)?;
        timestamp_metrics(&mut input.metrics, input.timestamp_ms);
        require(
            input.metrics.iter().all(|m| {
                !matches!(
                    m.get("name").and_then(Value::as_str),
                    Some("bdSeq" | "Node Control/Rebirth")
                )
            }),
            "bdSeq and Node Control/Rebirth are managed by the state layer",
        )?;
        let state = Self {
            group_id: input.group_id,
            node_id: input.node_id,
            bd_seq: input.previous_bd_seq.wrapping_add(1),
            next_sequence: 0,
            born: false,
            node_metrics: input.metrics,
            devices: BTreeMap::new(),
        };
        let last_will = state.death_message(input.timestamp_ms)?;
        Ok(PreparedSession { state, last_will })
    }

    fn topic(&self, kind: &str, device: Option<&str>) -> String {
        let topic = format!("spBv1.0/{}/{kind}/{}", self.group_id, self.node_id);
        device.map_or(topic.clone(), |device| format!("{topic}/{device}"))
    }

    fn death_message(&self, timestamp: u64) -> Result<WireMessage> {
        encode(&Payload {
            topic: self.topic("NDEATH", None),
            payload: json!({"timestamp": timestamp.to_string(), "metrics": [{"name":"bdSeq", "datatype":4, "longValue": self.bd_seq.to_string()}]}),
        })
    }

    fn emit(
        &mut self,
        kind: &str,
        device: Option<&str>,
        mut metrics: Vec<Value>,
        timestamp: u64,
    ) -> Result<WireMessage> {
        timestamp_metrics(&mut metrics, timestamp);
        let message = encode(&Payload {
            topic: self.topic(kind, device),
            payload: json!({"timestamp":timestamp.to_string(), "seq":self.next_sequence.to_string(), "metrics":metrics}),
        })?;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        Ok(message)
    }

    pub fn transition(mut self, action: EdgeAction, timestamp: u64) -> Result<Transition> {
        identifier(&self.group_id)?;
        identifier(&self.node_id)?;
        metric_definitions(&mut self.node_metrics)?;
        require(
            self.node_metrics.iter().all(|m| {
                !matches!(
                    m.get("name").and_then(Value::as_str),
                    Some("bdSeq" | "Node Control/Rebirth")
                )
            }),
            "bdSeq and Node Control/Rebirth are managed by the state layer",
        )?;
        require(self.devices.len() <= 1024, "Too many Sparkplug devices")?;
        for (device, metrics) in &mut self.devices {
            identifier(device)?;
            metric_definitions(metrics)?;
        }
        unique_node_aliases(&self)?;
        let mut messages = Vec::new();
        match action {
            EdgeAction::Birth => {
                self.next_sequence = 0;
                let mut metrics = self.node_metrics.clone();
                metrics
                    .push(json!({"name":"bdSeq","datatype":4,"longValue":self.bd_seq.to_string()}));
                metrics.push(
                    json!({"name":"Node Control/Rebirth","datatype":11,"booleanValue":false}),
                );
                messages.push(self.emit("NBIRTH", None, metrics, timestamp)?);
                for (device, metrics) in self.devices.clone() {
                    messages.push(self.emit("DBIRTH", Some(&device), metrics, timestamp)?);
                }
                self.born = true;
            }
            EdgeAction::DeviceBirth {
                device_id,
                mut metrics,
            } => {
                require(self.born, "Publish NBIRTH before DBIRTH")?;
                identifier(&device_id)?;
                metric_definitions(&mut metrics)?;
                require(
                    self.devices.len() < 1024 || self.devices.contains_key(&device_id),
                    "Too many Sparkplug devices",
                )?;
                timestamp_metrics(&mut metrics, timestamp);
                self.devices.insert(device_id.clone(), metrics.clone());
                unique_node_aliases(&self)?;
                messages.push(self.emit("DBIRTH", Some(&device_id), metrics, timestamp)?);
            }
            EdgeAction::Data {
                device_id,
                mut metrics,
            } => {
                require(self.born, "Publish birth before data")?;
                timestamp_metrics(&mut metrics, timestamp);
                let definitions = if let Some(device) = &device_id {
                    self.devices
                        .get_mut(device)
                        .ok_or_else(|| Error::Invalid("Publish DBIRTH before DDATA".into()))?
                } else {
                    &mut self.node_metrics
                };
                require(metrics.len() <= 4096, "Too many metric updates")?;
                normalize_metrics(&mut metrics)?;
                for metric in &metrics {
                    metric_value(metric, false)?;
                    let definition = definitions
                        .iter_mut()
                        .find(|definition| {
                            metric
                                .get("name")
                                .is_some_and(|name| Some(name) == definition.get("name"))
                                || number(metric.get("alias")).is_some_and(|alias| {
                                    Some(alias) == number(definition.get("alias"))
                                })
                        })
                        .ok_or_else(|| {
                            Error::Invalid(
                                "Data references a metric absent from birth; publish a new birth"
                                    .into(),
                            )
                        })?;
                    if let Some(name) = metric.get("name") {
                        require(
                            Some(name) == definition.get("name"),
                            "Metric name and alias refer to different birth metrics",
                        )?;
                    }
                    if let Some(alias) = metric.get("alias") {
                        require(
                            number(Some(alias)).is_some()
                                && number(Some(alias)) == number(definition.get("alias")),
                            "Metric name and alias refer to different birth metrics",
                        )?;
                    }
                    if let Some(datatype) = metric.get("datatype") {
                        require(
                            number(Some(datatype)) == number(definition.get("datatype")),
                            "Metric datatype changed; publish a new birth",
                        )?;
                    }
                    metric_datatype(
                        metric,
                        number(definition.get("datatype"))
                            .ok_or_else(|| Error::Invalid("Birth metric has no datatype".into()))?,
                    )?;
                    // Historical samples are published without replacing the current rebirth value.
                    if metric.get("isHistorical") == Some(&Value::Bool(true)) {
                        continue;
                    }
                    let object = metric
                        .as_object()
                        .ok_or_else(|| Error::Invalid("Expected metric object".into()))?;
                    let target = definition
                        .as_object_mut()
                        .ok_or_else(|| Error::Invalid("Invalid birth metric".into()))?;
                    if METRIC_VALUES.iter().any(|key| object.contains_key(*key))
                        || object.get("isNull") == Some(&Value::Bool(true))
                    {
                        for key in METRIC_VALUES {
                            target.remove(*key);
                        }
                        target.remove("isNull");
                    }
                    for (key, value) in object {
                        target.insert(key.clone(), value.clone());
                    }
                    metric_value(definition, true)?;
                }
                messages.push(self.emit(
                    if device_id.is_some() {
                        "DDATA"
                    } else {
                        "NDATA"
                    },
                    device_id.as_deref(),
                    metrics,
                    timestamp,
                )?);
            }
            EdgeAction::DeviceDeath { device_id } => {
                require(
                    self.born && self.devices.remove(&device_id).is_some(),
                    "Device has no active birth",
                )?;
                messages.push(self.emit("DDEATH", Some(&device_id), Vec::new(), timestamp)?);
            }
            EdgeAction::Death => {
                require(self.born, "Node has no active birth")?;
                messages.push(self.death_message(timestamp)?);
                self.born = false;
            }
        }
        Ok(Transition {
            state: self,
            messages,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EdgeAction {
    Birth,
    DeviceBirth {
        device_id: String,
        metrics: Vec<Value>,
    },
    Data {
        device_id: Option<String>,
        metrics: Vec<Value>,
    },
    DeviceDeath {
        device_id: String,
    },
    Death,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct Transition {
    pub state: EdgeState,
    pub messages: Vec<WireMessage>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ObservedNode {
    pub node_topic: Option<String>,
    pub online: bool,
    pub bd_seq: Option<u8>,
    pub next_sequence: Option<u8>,
    pub needs_rebirth: bool,
}

impl ObservedNode {
    /// Track one edge node. Callers keep separate state for each group/node pair.
    pub fn apply(mut self, message: &Payload) -> Result<Self> {
        validate_topic(&message.topic)?;
        let parts: Vec<_> = message.topic.split('/').collect();
        require(parts.len() >= 4, "Observe a node or device topic")?;
        let node_topic = format!("{}/{}", parts[1], parts[3]);
        if let Some(expected) = &self.node_topic {
            require(
                expected == &node_topic,
                "Keep separate observer state for each Sparkplug edge node",
            )?;
        }
        self.node_topic = Some(node_topic.clone());
        let kind = parts[2];
        let bd_seq = message
            .payload
            .get("metrics")
            .and_then(Value::as_array)
            .and_then(|metrics| {
                metrics
                    .iter()
                    .find(|m| m.get("name").and_then(Value::as_str) == Some("bdSeq"))
            })
            .and_then(|m| number(m.get("longValue").or_else(|| m.get("long_value"))))
            .and_then(|n| u8::try_from(n).ok());
        match kind {
            "NBIRTH" => {
                let sequence =
                    number(message.payload.get("seq")).and_then(|n| u8::try_from(n).ok());
                require(
                    sequence.is_some() && bd_seq.is_some(),
                    "NBIRTH requires seq in 0..255 and bdSeq",
                )?;
                self = Self {
                    node_topic: Some(node_topic),
                    online: true,
                    bd_seq,
                    next_sequence: sequence.map(|n| n.wrapping_add(1)),
                    needs_rebirth: false,
                };
            }
            "NDEATH" => {
                require(bd_seq.is_some(), "NDEATH requires bdSeq")?;
                if self.bd_seq == bd_seq {
                    self.online = false;
                    self.next_sequence = None;
                }
            }
            "NDATA" | "DBIRTH" | "DDATA" | "DDEATH" => {
                let sequence =
                    number(message.payload.get("seq")).and_then(|n| u8::try_from(n).ok());
                if !self.online || sequence.is_none() || sequence != self.next_sequence {
                    self.needs_rebirth = true;
                } else {
                    self.next_sequence = sequence.map(|n| n.wrapping_add(1));
                }
            }
            "NCMD" | "DCMD" => {}
            _ => return Err(Error::Invalid("Unexpected Sparkplug message type".into())),
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared() -> PreparedSession {
        EdgeState::prepare(PrepareSession {
            group_id: "factory".into(),
            node_id: "line1".into(),
            previous_bd_seq: 255,
            timestamp_ms: 1,
            metrics: vec![
                json!({"name":"temperature","alias":"10","datatype":10,"doubleValue":20.0}),
            ],
        })
        .unwrap()
    }
    #[test]
    fn protobuf_roundtrip_and_shared_sequence_wrap() {
        let prepared = prepared();
        assert_eq!(prepared.state.bd_seq, 0);
        assert_eq!(prepared.last_will.qos, 1);
        let birth = prepared.state.transition(EdgeAction::Birth, 2).unwrap();
        let decoded = decode(&birth.messages[0]).unwrap();
        assert_eq!(number(decoded.payload.get("seq")), Some(0));
        let mut state = birth.state;
        for _ in 0..255 {
            state = state
                .transition(
                    EdgeAction::Data {
                        device_id: None,
                        metrics: vec![json!({"alias":"10","doubleValue":21.0})],
                    },
                    3,
                )
                .unwrap()
                .state;
        }
        assert_eq!(state.next_sequence, 0);
        let rebirth = state.transition(EdgeAction::Birth, 4).unwrap();
        assert_eq!(
            decode(&rebirth.messages[0]).unwrap().payload["metrics"][0]["doubleValue"],
            21.0
        );
    }
    #[test]
    fn rejects_data_before_birth_and_unknown_alias() {
        let request = EdgeAction::Data {
            device_id: None,
            metrics: vec![json!({"alias":"99","doubleValue":21.0})],
        };
        assert!(prepared().state.transition(request.clone(), 1).is_err());
        let birth = prepared().state.transition(EdgeAction::Birth, 1).unwrap();
        assert!(birth.state.transition(request, 2).is_err());
    }
    #[test]
    fn observer_rejects_stale_death_and_detects_lost_data() {
        let birth = prepared().state.transition(EdgeAction::Birth, 1).unwrap();
        let observed = ObservedNode::default()
            .apply(&decode(&birth.messages[0]).unwrap())
            .unwrap();
        let stale = Payload {
            topic: "spBv1.0/factory/NDEATH/line1".into(),
            payload: json!({"metrics":[{"name":"bdSeq","longValue":"255"}]}),
        };
        assert!(observed.clone().apply(&stale).unwrap().online);
        let gap = Payload {
            topic: "spBv1.0/factory/NDATA/line1".into(),
            payload: json!({"seq":"4"}),
        };
        assert!(observed.apply(&gap).unwrap().needs_rebirth);
    }
    #[test]
    fn state_uses_json_retention_and_current_namespace() {
        let message = encode(&Payload {
            topic: "spBv1.0/STATE/host".into(),
            payload: json!({"online":true,"timestamp":1}),
        })
        .unwrap();
        assert!(message.retain);
        assert_eq!(decode(&message).unwrap().payload["online"], true);
        assert!(validate_topic("spBv1.0/a/NDATA/b/extra").is_err());
    }

    #[test]
    fn rejects_conflicting_identity_and_keeps_rebirth_null_values() {
        let state = prepared()
            .state
            .transition(EdgeAction::Birth, 1)
            .unwrap()
            .state;
        assert!(
            state
                .clone()
                .transition(
                    EdgeAction::Data {
                        device_id: None,
                        metrics: vec![json!({"name":"another","alias":"10","doubleValue":21.0})]
                    },
                    2
                )
                .is_err()
        );
        let state = state
            .transition(
                EdgeAction::Data {
                    device_id: None,
                    metrics: vec![json!({"alias":"10","isNull":true})],
                },
                2,
            )
            .unwrap()
            .state;
        let birth = state.transition(EdgeAction::Birth, 3).unwrap();
        let payload = decode(&birth.messages[0]).unwrap().payload;
        assert_eq!(payload["metrics"][0]["isNull"], true);
        assert!(payload["metrics"][0].get("doubleValue").is_none());
        let observed = ObservedNode::default()
            .apply(&decode(&birth.messages[0]).unwrap())
            .unwrap();
        let other = Payload {
            topic: "spBv1.0/factory/NDATA/line2".into(),
            payload: json!({"seq":"1"}),
        };
        assert!(observed.apply(&other).is_err());
    }

    #[test]
    fn managed_bdseq_is_int64_and_birth_data_metrics_have_timestamps() {
        let prepared = prepared();
        let will = decode(&prepared.last_will).unwrap();
        assert_eq!(will.payload["metrics"][0]["datatype"], 4);
        let birth = prepared.state.transition(EdgeAction::Birth, 20).unwrap();
        let payload = decode(&birth.messages[0]).unwrap().payload;
        let metrics = payload["metrics"].as_array().unwrap();
        assert_eq!(
            metrics.iter().find(|m| m["name"] == "bdSeq").unwrap()["datatype"],
            4
        );
        assert!(
            metrics
                .iter()
                .all(|metric| number(metric.get("timestamp")).is_some())
        );
        let data = birth
            .state
            .transition(
                EdgeAction::Data {
                    device_id: None,
                    metrics: vec![json!({"alias":"10","doubleValue":22.0})],
                },
                30,
            )
            .unwrap();
        assert_eq!(
            number(decode(&data.messages[0]).unwrap().payload["metrics"][0].get("timestamp")),
            Some(30)
        );
        let rebirth = data.state.transition(EdgeAction::Birth, 40).unwrap();
        assert_eq!(
            number(decode(&rebirth.messages[0]).unwrap().payload["metrics"][0].get("timestamp")),
            Some(30)
        );
    }

    #[test]
    fn birth_values_null_flags_and_node_wide_aliases_are_validated() {
        for metric in [
            json!({"name":"missing","datatype":10}),
            json!({"name":"null","datatype":10,"isNull":true,"doubleValue":1.0}),
            json!({"name":"unknown","datatype":35,"longValue":"1"}),
        ] {
            assert!(metric_definitions(&mut [metric]).is_err());
        }
        assert!(
            metric_definitions(&mut [json!({"name":"counter","datatype":8,"longValue":"42"})])
                .is_ok()
        );
        let state = prepared()
            .state
            .transition(EdgeAction::Birth, 1)
            .unwrap()
            .state;
        assert!(state.clone().transition(EdgeAction::DeviceBirth {device_id:"device".into(),metrics:vec![json!({"name":"duplicate","alias":"10","datatype":11,"booleanValue":true})]},2).is_err());
        let state = state
            .transition(
                EdgeAction::Data {
                    device_id: None,
                    metrics: vec![json!({"alias":"10","isNull":true})],
                },
                2,
            )
            .unwrap()
            .state;
        assert!(
            state
                .transition(
                    EdgeAction::Data {
                        device_id: None,
                        metrics: vec![json!({"alias":"10","isNull":false})]
                    },
                    3
                )
                .is_err()
        );
    }

    #[test]
    fn birth_and_data_reject_value_fields_that_disagree_with_the_datatype() {
        for metric in [
            json!({"name":"temperature","datatype":10,"stringValue":"broken"}),
            json!({"name":"temperature","datatype":10,"string_value":"broken"}),
            json!({"name":"temperature","datatype":10,"doubleValue":1.0,"string_value":"broken"}),
            json!({"name":"properties","datatype":20,"extensionValue":{}}),
            json!({"name":"properties","datatype":21,"isNull":true}),
        ] {
            assert!(
                EdgeState::prepare(PrepareSession {
                    group_id: "factory".into(),
                    node_id: "line1".into(),
                    previous_bd_seq: 0,
                    timestamp_ms: 1,
                    metrics: vec![metric],
                })
                .is_err()
            );
        }
        let state = prepared()
            .state
            .transition(EdgeAction::Birth, 1)
            .unwrap()
            .state;
        for metric in [
            json!({"name":"temperature","stringValue":"broken"}),
            json!({"alias":"10","string_value":"broken"}),
            json!({"alias":"10","datatype":10,"intValue":42}),
            json!({"alias":"10","isHistorical":true,"stringValue":"broken"}),
            json!({"alias":"10","is_historical":true,"string_value":"broken"}),
            json!({"alias":"10","doubleValue":1.0,"double_value":2.0}),
            json!({"alias":"10","isNull":false,"is_null":true}),
        ] {
            assert!(
                state
                    .clone()
                    .transition(
                        EdgeAction::Data {
                            device_id: None,
                            metrics: vec![metric],
                        },
                        2
                    )
                    .is_err()
            );
        }
        let updated = state
            .transition(
                EdgeAction::Data {
                    device_id: None,
                    metrics: vec![json!({"alias":"10","doubleValue":42.5})],
                },
                2,
            )
            .unwrap();
        let rebirth = updated.state.transition(EdgeAction::Birth, 3).unwrap();
        assert_eq!(
            decode(&rebirth.messages[0]).unwrap().payload["metrics"][0]["doubleValue"],
            42.5
        );
    }

    #[test]
    fn protobuf_metric_spellings_preserve_null_and_historical_semantics() {
        let prepared = EdgeState::prepare(PrepareSession {
            group_id: "factory".into(),
            node_id: "line1".into(),
            previous_bd_seq: 0,
            timestamp_ms: 1,
            metrics: vec![
                json!({"name":"temperature","alias":"10","datatype":10,"double_value":20.0}),
            ],
        })
        .unwrap();
        assert_eq!(prepared.state.node_metrics[0]["doubleValue"], 20.0);
        assert!(prepared.state.node_metrics[0].get("double_value").is_none());
        let birth = prepared.state.transition(EdgeAction::Birth, 2).unwrap();
        let device = birth
            .state
            .transition(
                EdgeAction::DeviceBirth {
                    device_id: "sensor".into(),
                    metrics: vec![json!({"name":"reading","datatype":10,"is_null":true})],
                },
                3,
            )
            .unwrap();
        assert_eq!(device.state.devices["sensor"][0]["isNull"], true);
        let historical = device
            .state
            .transition(
                EdgeAction::Data {
                    device_id: None,
                    metrics: vec![json!({"alias":"10","double_value":-5.0,"is_historical":true})],
                },
                4,
            )
            .unwrap();
        assert_eq!(historical.state.node_metrics[0]["doubleValue"], 20.0);
        let metric = &decode(&historical.messages[0]).unwrap().payload["metrics"][0];
        assert_eq!(metric["doubleValue"], -5.0);
        assert_eq!(metric["isHistorical"], true);
        let null = historical
            .state
            .transition(
                EdgeAction::Data {
                    device_id: None,
                    metrics: vec![json!({"alias":"10","is_null":true})],
                },
                5,
            )
            .unwrap();
        let rebirth = null.state.transition(EdgeAction::Birth, 6).unwrap();
        let metric = &decode(&rebirth.messages[0]).unwrap().payload["metrics"][0];
        assert_eq!(metric["isNull"], true);
        assert!(metric.get("doubleValue").is_none());
    }

    #[test]
    fn restored_state_cannot_bypass_datatype_validation_with_protobuf_spelling() {
        for device in [false, true] {
            let mut state = prepared().state;
            let metrics = if device {
                state.devices.entry("sensor".into()).or_default()
            } else {
                &mut state.node_metrics
            };
            *metrics = vec![json!({"name":"temperature","datatype":10,"string_value":"broken"})];
            assert!(state.transition(EdgeAction::Birth, 1).is_err());
        }
    }

    #[test]
    fn metric_value_fields_cover_scalar_composite_and_array_types() {
        for (datatype, key, value) in [
            (1, "intValue", json!(u32::MAX)),
            (2, "intValue", json!(u32::MAX)),
            (3, "intValue", json!(u32::MAX)),
            (4, "longValue", json!(u64::MAX.to_string())),
            (5, "intValue", json!(255)),
            (6, "intValue", json!(65535)),
            (7, "intValue", json!(u32::MAX)),
            (8, "longValue", json!(u64::MAX.to_string())),
            (9, "floatValue", json!(1.25)),
            (10, "doubleValue", json!(2.5)),
            (11, "booleanValue", json!(false)),
            (12, "stringValue", json!("value")),
            (13, "longValue", json!("123")),
            (14, "stringValue", json!("text")),
            (15, "stringValue", json!("uuid")),
            (16, "datasetValue", json!({})),
            (17, "bytesValue", json!("AQ==")),
            (18, "bytesValue", json!("AQ==")),
            (19, "templateValue", json!({})),
        ]
        .into_iter()
        .chain((22..=34).map(|datatype| (datatype, "bytesValue", json!(""))))
        {
            let mut metric = json!({"name":"value","datatype":datatype});
            metric[key] = value;
            metric_definitions(&mut [metric.clone()]).unwrap();
            let wire = encode(&Payload {
                topic: "spBv1.0/g/NBIRTH/n".into(),
                payload: json!({"metrics":[metric]}),
            })
            .unwrap();
            assert!(
                decode(&wire).unwrap().payload["metrics"][0]
                    .get(key)
                    .is_some()
            );
            metric_definitions(&mut [json!({"name":"value","datatype":datatype,"isNull":true})])
                .unwrap();
        }
        for metric in [
            json!({"name":"dataset","datatype":16,"bytesValue":"AQ=="}),
            json!({"name":"file","datatype":18,"datasetValue":{}}),
        ] {
            assert!(metric_definitions(&mut [metric]).is_err());
        }
    }

    #[test]
    fn observer_accepts_nonzero_initial_sequence_and_wraps() {
        let birth = prepared().state.transition(EdgeAction::Birth, 1).unwrap();
        let mut payload = decode(&birth.messages[0]).unwrap();
        payload.payload["seq"] = json!("255");
        let observed = ObservedNode::default().apply(&payload).unwrap();
        assert_eq!(observed.next_sequence, Some(0));
        let data = Payload {
            topic: "spBv1.0/factory/NDATA/line1".into(),
            payload: json!({"seq":"0"}),
        };
        assert!(!observed.apply(&data).unwrap().needs_rebirth);
    }
    #[test]
    fn historical_samples_do_not_replace_current_birth_value() {
        let birth = prepared().state.transition(EdgeAction::Birth, 1).unwrap();
        let historical = birth.state.transition(EdgeAction::Data {
            device_id:None,
            metrics:vec![json!({"alias":"10","doubleValue":-5.0,"timestamp":"0","isHistorical":true})],
        },2).unwrap();
        assert_eq!(
            decode(&historical.messages[0]).unwrap().payload["metrics"][0]["doubleValue"],
            -5.0
        );
        let rebirth = historical.state.transition(EdgeAction::Birth, 3).unwrap();
        assert_eq!(
            decode(&rebirth.messages[0]).unwrap().payload["metrics"][0]["doubleValue"],
            20.0
        );
    }
}
