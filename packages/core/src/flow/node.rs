use flow_like_types::{async_trait, create_id};
use highway::{HighwayHash, HighwayHasher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use super::{
    board::Board,
    execution::context::ExecutionContext,
    pin::{Pin, PinType, ValueType},
    variable::VariableType,
};

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
pub enum NodeState {
    Idle,
    Running,
    Success,
    Error,
}

/// Represents quality metrics for a node, with scores ranging from 0 to 10 (low - high).
/// Higher values indicate higher risk/impact in the given category. Use 0 for "none/low"
/// and 10 for "very high".
///
/// # Score Categories
/// * `privacy` - Measures data protection and confidentiality (0 low - 10 high).
/// * `security` - Assesses resistance against potential attacks and exposure (0 low - 10 high).
/// * `performance` - Evaluates computational efficiency and speed. Higher means worse performance.
/// * `governance` - Indicates compliance and auditability with policies and regulations.
/// * `reliability` - Measures stability, error rates and recoverability.
/// * `cost` - Represents resource/cost impact for running this node.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct NodeScores {
    pub privacy: u8,
    pub security: u8,
    pub performance: u8,
    pub governance: u8,
    pub reliability: u8,
    pub cost: u8,
}

impl Default for NodeScores {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeScores {
    pub fn new() -> Self {
        NodeScores {
            privacy: 0,
            security: 0,
            performance: 0,
            governance: 0,
            reliability: 0,
            cost: 0,
        }
    }

    pub fn set_privacy(&mut self, score: u8) -> &mut Self {
        self.privacy = score;
        self
    }
    pub fn set_security(&mut self, score: u8) -> &mut Self {
        self.security = score;
        self
    }
    pub fn set_performance(&mut self, score: u8) -> &mut Self {
        self.performance = score;
        self
    }
    pub fn set_governance(&mut self, score: u8) -> &mut Self {
        self.governance = score;
        self
    }
    pub fn set_reliability(&mut self, score: u8) -> &mut Self {
        self.reliability = score;
        self
    }
    pub fn set_cost(&mut self, score: u8) -> &mut Self {
        self.cost = score;
        self
    }
    pub fn build(&self) -> Self {
        self.clone()
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct FnRefs {
    pub fn_refs: Vec<String>,
    pub can_reference_fns: bool,
    pub can_be_referenced_by_fns: bool,
}

/// Permissions a WASM node can request.
/// Each node declares exactly which capabilities it needs so the sandbox
/// and UI can enforce/display them precisely.
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum NodePermission {
    /// Outbound HTTP requests
    #[serde(rename = "network:http")]
    NetworkHttp,
    /// WebSocket connections
    #[serde(rename = "network:websocket")]
    NetworkWebsocket,
    /// TCP socket access
    #[serde(rename = "network:tcp")]
    NetworkTcp,
    /// UDP socket access
    #[serde(rename = "network:udp")]
    NetworkUdp,
    /// DNS lookups
    #[serde(rename = "network:dns")]
    NetworkDns,
    /// Read from node/user storage
    #[serde(rename = "storage:read")]
    StorageRead,
    /// Write to node/user storage
    #[serde(rename = "storage:write")]
    StorageWrite,
    /// Access flow variables
    #[serde(rename = "variables")]
    Variables,
    /// Access execution cache
    #[serde(rename = "cache")]
    Cache,
    /// Stream responses to the client
    #[serde(rename = "streaming")]
    Streaming,
    /// Access LLM / model providers
    #[serde(rename = "models")]
    Models,
    /// Dynamic UI (Agent-to-UI)
    #[serde(rename = "a2ui")]
    A2ui,
    /// OAuth authentication
    #[serde(rename = "oauth")]
    OAuth,
    /// Call other functions/sub-flows
    #[serde(rename = "functions")]
    Functions,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct NodeWasm {
    pub package_id: String,
    #[serde(default)]
    pub permissions: Vec<NodePermission>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Node {
    pub id: String,
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub coordinates: Option<(f32, f32, f32)>,
    pub category: String,
    pub scores: Option<NodeScores>,
    pub pins: HashMap<String, Pin>,
    pub start: Option<bool>,
    pub icon: Option<String>,
    pub comment: Option<String>,
    pub long_running: Option<bool>,
    pub error: Option<String>,
    pub docs: Option<String>,
    pub event_callback: Option<bool>,
    pub layer: Option<String>,
    pub hash: Option<u64>,
    pub fn_refs: Option<FnRefs>,
    /// OAuth provider IDs this node requires (references Hub's oauth_providers config)
    pub oauth_providers: Option<Vec<String>>,
    /// OAuth scopes required by this node (provider_id -> scopes)
    pub required_oauth_scopes: Option<HashMap<String, Vec<String>>>,
    /// If true, this node can only run locally (compute-intensive, RPA, browser automation)
    #[serde(default)]
    pub only_offline: bool,
    /// Schema version for node migration. When catalog version > placed version, pins are synced.
    /// None means unversioned (legacy). Bump this when changing pins in get_node().
    pub version: Option<u32>,
    /// WASM metadata for external nodes. None for built-in catalog nodes.
    /// Populated automatically when placing or pasting nodes; never trust frontend-supplied values.
    pub wasm: Option<NodeWasm>,
}

impl Node {
    pub fn new(name: &str, friendly_name: &str, description: &str, category: &str) -> Self {
        Node {
            id: create_id(),
            name: name.to_string(),
            friendly_name: friendly_name.to_string(),
            description: description.to_string(),
            coordinates: None,
            category: category.to_string(),
            pins: HashMap::new(),
            scores: None,
            start: None,
            icon: None,
            comment: None,
            long_running: None,
            error: None,
            docs: None,
            event_callback: None,
            layer: None,
            hash: None,
            fn_refs: None,
            oauth_providers: None,
            required_oauth_scopes: None,
            only_offline: false,
            version: None,
            wasm: None,
        }
    }

    pub fn add_comment(&mut self, comment: &str) {
        self.comment = Some(comment.to_string());
    }

    pub fn set_version(&mut self, version: u32) {
        self.version = Some(version);
    }

    pub fn add_icon(&mut self, icon: &str) {
        self.icon = Some(icon.to_string());
    }

    pub fn set_start(&mut self, start: bool) {
        self.start = Some(start);
    }

    pub fn set_event_callback(&mut self, callback: bool) {
        self.event_callback = Some(callback);
    }

    pub fn set_can_be_referenced_by_fns(&mut self, can_be_referenced: bool) {
        if let Some(fn_refs) = &mut self.fn_refs {
            fn_refs.can_be_referenced_by_fns = can_be_referenced;
        } else {
            self.fn_refs = Some(FnRefs {
                fn_refs: Vec::new(),
                can_reference_fns: false,
                can_be_referenced_by_fns: can_be_referenced,
            });
        }
    }

    pub fn set_can_reference_fns(&mut self, can_reference: bool) {
        if let Some(fn_refs) = &mut self.fn_refs {
            fn_refs.can_reference_fns = can_reference;
        } else {
            self.fn_refs = Some(FnRefs {
                fn_refs: Vec::new(),
                can_reference_fns: can_reference,
                can_be_referenced_by_fns: false,
            });
        }
    }

    /// Add an OAuth provider ID requirement to this node
    pub fn add_oauth_provider(&mut self, provider_id: &str) {
        if let Some(providers) = &mut self.oauth_providers {
            if !providers.contains(&provider_id.to_string()) {
                providers.push(provider_id.to_string());
            }
        } else {
            self.oauth_providers = Some(vec![provider_id.to_string()]);
        }
    }

    /// Get all OAuth provider IDs required by this node
    pub fn get_oauth_provider_ids(&self) -> Vec<String> {
        self.oauth_providers.clone().unwrap_or_default()
    }

    /// Add required OAuth scopes for a specific provider.
    /// These scopes will be merged with the provider's base scopes when OAuth is initiated.
    pub fn add_required_oauth_scopes(&mut self, provider_id: &str, scopes: Vec<&str>) {
        let scopes: Vec<String> = scopes.into_iter().map(|s| s.to_string()).collect();
        if let Some(ref mut required_scopes) = self.required_oauth_scopes {
            if let Some(existing) = required_scopes.get_mut(provider_id) {
                for scope in scopes {
                    if !existing.contains(&scope) {
                        existing.push(scope);
                    }
                }
            } else {
                required_scopes.insert(provider_id.to_string(), scopes);
            }
        } else {
            let mut map = HashMap::new();
            map.insert(provider_id.to_string(), scopes);
            self.required_oauth_scopes = Some(map);
        }
    }

    /// Get required OAuth scopes for a specific provider
    pub fn get_required_oauth_scopes(&self, provider_id: &str) -> Vec<String> {
        self.required_oauth_scopes
            .as_ref()
            .and_then(|scopes| scopes.get(provider_id))
            .cloned()
            .unwrap_or_default()
    }

    /// Set whether this node can only run locally (offline)
    pub fn set_only_offline(&mut self, only_offline: bool) {
        self.only_offline = only_offline;
    }

    pub fn add_input_pin(
        &mut self,
        name: &str,
        friendly_name: &str,
        description: &str,
        data_type: VariableType,
    ) -> &mut Pin {
        let pin_id = create_id();
        let num_outputs = self
            .pins
            .iter()
            .filter(|(_, v)| v.pin_type == PinType::Input)
            .count();
        self.pins.insert(
            pin_id.clone(),
            Pin {
                id: pin_id.clone(),
                name: name.to_string(),
                friendly_name: friendly_name.to_string(),
                description: description.to_string(),
                schema: None,
                pin_type: PinType::Input,
                data_type,
                value_type: super::pin::ValueType::Normal,
                depends_on: BTreeSet::new(),
                connected_to: BTreeSet::new(),
                default_value: None,
                options: None,
                value: None,
                index: num_outputs as u16 + 1,
            },
        );
        self.pins.get_mut(&pin_id).unwrap()
    }

    pub fn add_output_pin(
        &mut self,
        name: &str,
        friendly_name: &str,
        description: &str,
        data_type: VariableType,
    ) -> &mut Pin {
        let pin_id = create_id();
        let num_outputs = self
            .pins
            .iter()
            .filter(|(_, v)| v.pin_type == PinType::Output)
            .count();
        self.pins.insert(
            pin_id.clone(),
            Pin {
                id: pin_id.clone(),
                name: name.to_string(),
                friendly_name: friendly_name.to_string(),
                description: description.to_string(),
                schema: None,
                options: None,
                pin_type: PinType::Output,
                data_type,
                value_type: super::pin::ValueType::Normal,
                depends_on: BTreeSet::new(),
                connected_to: BTreeSet::new(),
                default_value: None,
                value: None,
                index: num_outputs as u16 + 1,
            },
        );
        self.pins.get_mut(&pin_id).unwrap()
    }

    pub fn is_pure(&self) -> bool {
        for pin in self.pins.values() {
            if pin.data_type == VariableType::Execution {
                return false;
            }
        }

        true
    }

    pub fn get_pin_by_name(&self, name: &str) -> Option<&Pin> {
        self.pins.values().find(|&pin| pin.name == name)
    }

    pub fn get_pin_mut_by_name(&mut self, name: &str) -> Option<&mut Pin> {
        self.pins.values_mut().find(|pin| pin.name == name)
    }

    pub fn set_long_running(&mut self, long_running: bool) {
        self.long_running = Some(long_running);
    }

    pub fn mut_scores(&mut self) -> &mut NodeScores {
        self.scores.as_mut().unwrap()
    }

    pub fn set_scores(&mut self, scores: NodeScores) {
        self.scores = Some(scores);
    }

    pub fn harmonize_schema(&mut self, pins: Vec<&str>) -> Option<String> {
        let schema = match self
            .pins
            .iter()
            .find(|(_, pin)| pins.contains(&pin.name.as_str()) && pin.schema.is_some())
        {
            Some((_, pin)) => pin.schema.clone(),
            None => return None,
        };

        for pin in self.pins.values_mut() {
            if pins.contains(&pin.name.as_str()) {
                pin.schema = schema.clone();
            }
        }

        schema
    }

    pub fn harmonize_type(&mut self, pins: Vec<&str>, schema: bool) -> Option<VariableType> {
        // Scan in the caller's pin order, not map iteration order, so the
        // donor pin is deterministic. The type comes from the first
        // non-generic pin; the schema from the first pin that actually has
        // one — a schema-less sibling must never wipe another pin's schema.
        let variable_type = pins.iter().find_map(|name| {
            self.pins
                .values()
                .find(|pin| pin.name == *name && pin.data_type != VariableType::Generic)
                .map(|pin| pin.data_type.clone())
        })?;

        let found_schema = if schema {
            pins.iter().find_map(|name| {
                self.pins
                    .values()
                    .filter(|pin| pin.name == *name)
                    .find_map(|pin| pin.schema.clone())
            })
        } else {
            None
        };

        for pin in self.pins.values_mut() {
            if pins.contains(&pin.name.as_str()) {
                pin.data_type = variable_type.clone();
                if schema {
                    pin.schema = found_schema.clone();
                }
            }
        }

        Some(variable_type)
    }

    pub fn harmonize_value_type(&mut self, pins: Vec<&str>) -> Option<ValueType> {
        let value_type = match self.pins.iter().find(|(_, pin)| {
            pins.contains(&pin.name.as_str()) && pin.value_type != ValueType::Normal
        }) {
            Some((_, pin)) => pin.value_type.clone(),
            None => return None,
        };

        for pin in self.pins.values_mut() {
            if pins.contains(&pin.name.as_str()) {
                pin.value_type = value_type.clone();
            }
        }

        Some(value_type)
    }

    pub fn match_type(
        &mut self,
        pin_name: &str,
        board: &Board,
        value_type: Option<ValueType>,
        default_type: Option<ValueType>,
    ) -> flow_like_types::Result<VariableType> {
        let mut found_type = VariableType::Generic;
        let pin = self
            .get_pin_by_name(pin_name)
            .ok_or(flow_like_types::anyhow!("Pin not found"))?;
        let mut nodes = pin.connected_to.clone();
        if pin.pin_type == PinType::Input {
            nodes = pin.depends_on.clone();
        }

        let default_type = default_type.unwrap_or(ValueType::Normal);

        self.get_pin_mut_by_name(pin_name).unwrap().data_type = VariableType::Generic;
        self.get_pin_mut_by_name(pin_name).unwrap().value_type = default_type;
        self.get_pin_mut_by_name(pin_name).unwrap().schema = None;
        if let Some(value_type) = &value_type {
            self.get_pin_mut_by_name(pin_name).unwrap().value_type = value_type.clone();
        }

        if let Some(first_node) = nodes.iter().next() {
            let pin = board.get_pin_by_id(first_node);
            let mutable_pin = self.get_pin_mut_by_name(pin_name).unwrap();

            match pin {
                Some(pin) => {
                    mutable_pin.data_type = pin.data_type.clone();
                    mutable_pin.schema = pin.schema.clone();
                    found_type = pin.data_type.clone();

                    if value_type.is_none() {
                        mutable_pin.value_type = pin.value_type.clone();
                    }
                }
                None => {
                    mutable_pin.depends_on.remove(first_node);
                }
            }
        }

        Ok(found_type)
    }

    pub fn hash(&mut self) {
        let mut hasher = HighwayHasher::new(highway::Key([
            0x0123456789abcdef,
            0xfedcba9876543210,
            0x0011223344556677,
            0x8899aabbccddeeff,
        ]));

        hasher.append(self.name.as_bytes());
        hasher.append(self.friendly_name.as_bytes());
        hasher.append(self.description.as_bytes());
        hasher.append(self.category.as_bytes());

        if let Some(coords) = &self.coordinates {
            hasher.append(&coords.0.to_le_bytes());
            hasher.append(&coords.1.to_le_bytes());
            hasher.append(&coords.2.to_le_bytes());
        }

        if let Some(scores) = &self.scores {
            hasher.append(&[
                scores.privacy,
                scores.security,
                scores.performance,
                scores.governance,
                scores.reliability,
                scores.cost,
            ]);
        }

        let mut pin_keys: Vec<_> = self.pins.keys().collect();
        pin_keys.sort();
        for key in pin_keys {
            let pin = &self.pins[key];
            hasher.append(pin.name.as_bytes());
            hasher.append(pin.friendly_name.as_bytes());
            hasher.append(pin.description.as_bytes());
            hasher.append(&(pin.pin_type.clone() as u8).to_le_bytes());
            hasher.append(&(pin.data_type.clone() as u8).to_le_bytes());
            hasher.append(&pin.index.to_le_bytes());
            hasher.append(&(pin.value_type.clone() as u8).to_le_bytes());
            if let Some(schema) = &pin.schema {
                hasher.append(schema.as_bytes());
            }
            if let Some(default_value) = &pin.default_value {
                hasher.append(default_value);
            }
            if let Some(options) = &pin.options {
                if let Some(valid_values) = &options.valid_values {
                    for value in valid_values {
                        hasher.append(value.as_bytes());
                    }
                }

                if let Some(range) = &options.range {
                    hasher.append(&range.0.to_le_bytes());
                    hasher.append(&range.1.to_le_bytes());
                }

                if let Some(step) = &options.step {
                    hasher.append(&step.to_le_bytes());
                }

                if let Some(enforce_schema) = &options.enforce_schema {
                    hasher.append(&[*enforce_schema as u8]);
                }

                if let Some(enforce_generic_value_type) = &options.enforce_generic_value_type {
                    hasher.append(&[*enforce_generic_value_type as u8]);
                }
            }

            for dep in pin.depends_on.iter() {
                hasher.append(dep.as_bytes());
            }

            for conn in pin.connected_to.iter() {
                hasher.append(conn.as_bytes());
            }
        }

        if let Some(start) = &self.start {
            hasher.append(&[*start as u8]);
        }

        if let Some(icon) = &self.icon {
            hasher.append(icon.as_bytes());
        }

        if let Some(comment) = &self.comment {
            hasher.append(comment.as_bytes());
        }

        if let Some(long_running) = &self.long_running {
            hasher.append(&[*long_running as u8]);
        }

        if let Some(event_callback) = &self.event_callback {
            hasher.append(&[*event_callback as u8]);
        }

        if let Some(layer) = &self.layer {
            hasher.append(layer.as_bytes());
        }

        if let Some(wasm) = &self.wasm {
            hasher.append(wasm.package_id.as_bytes());
        }

        self.hash = Some(hasher.finalize64());
    }
}

#[async_trait]
pub trait NodeLogic: Send + Sync {
    /// Returns the node definition. This is a sync function that constructs
    /// the node's metadata, pins, and configuration.
    /// For dynamic updates based on board state, use `on_update()` instead.
    fn get_node(&self) -> Node;

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()>;
    async fn on_drop(&self) {}

    async fn get_progress(&self, context: &mut ExecutionContext) -> i32 {
        let state = context.get_state();

        match state {
            NodeState::Running => return 50,
            NodeState::Success => return 100,
            NodeState::Error => return 0,
            _ => return 0,
        }
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
    async fn on_delete(&self, _node: &mut Node, _board: Arc<Board>) {}
}

/// Utility for .on_update()
pub fn remove_pin(node: &mut Node, pin: Option<Pin>) {
    if let Some(pin) = pin {
        node.pins.remove(&pin.id);
    }
}

/// Utility for .on_update()
pub fn remove_pin_by_name(node: &mut Node, name: &str) {
    if let Some(pin) = node.get_pin_by_name(name) {
        node.pins.remove(&pin.id.clone());
    }
}

#[cfg(test)]
mod tests {

    use flow_like_types::{FromProto, ToProto};
    use flow_like_types::{Message, tokio};

    #[tokio::test]
    async fn serialize_node() {
        let node = super::Node::new("Hi", "Test Node", "What a wonderful day", "IDK");

        let mut buf = Vec::new();
        node.to_proto().encode(&mut buf).unwrap();
        let deser_node =
            super::Node::from_proto(flow_like_types::proto::Node::decode(&buf[..]).unwrap());

        assert_eq!(node.id, deser_node.id);
    }

    #[test]
    fn node_hash_changes_with_scores() {
        use super::NodeScores;

        let mut node = super::Node::new("test_node", "Test", "desc", "Cat");
        node.scores = Some(NodeScores {
            privacy: 0,
            security: 0,
            performance: 0,
            governance: 0,
            reliability: 0,
            cost: 0,
        });
        node.hash();
        let first = node.hash.unwrap();

        // change reliability and cost only
        if let Some(scores) = &mut node.scores {
            scores.reliability = 9;
            scores.cost = 3;
        }
        node.hash();
        let second = node.hash.unwrap();

        assert_ne!(first, second, "Node hash should change when scores change");
    }
}
