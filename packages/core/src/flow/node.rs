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
    pin::{Pin, PinType, ValueType, is_open_object_schema},
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
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
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

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
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

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq)]
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
    /// FlowScript namespace path (`string`, `http`, `jira`, `ui`; dotted when nested).
    /// Presentation only — never part of node identity (`name` stays the id). `None` derives
    /// it from `category` (`flow_like_ast::naming::derive_namespace`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// FlowScript member name inside `namespace` (`trim`, `fetch`, `createIssue`).
    /// Presentation only — never part of node identity. `None` derives it from `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Name of the data input pin that receives the value in FlowScript method form
    /// (`s` in `s.trim()`). Presentation only. `None` applies the default rule
    /// (`flowscript_receiver`); `Some("")` opts out: the node is callable statically only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
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
            namespace: None,
            alias: None,
            receiver: None,
        }
    }

    /// Set the explicit FlowScript spelling of this node (`namespace::alias`).
    pub fn set_flowscript_name(&mut self, namespace: &str, alias: &str) -> &mut Self {
        self.namespace = Some(namespace.to_string());
        self.alias = Some(alias.to_string());
        self
    }

    /// Name the data input pin that receives the value in method form; `""` makes the node
    /// static-only.
    pub fn set_receiver(&mut self, pin_name: &str) -> &mut Self {
        self.receiver = Some(pin_name.to_string());
        self
    }

    fn name_fields(&self) -> flow_like_ast::NameFields<'_> {
        flow_like_ast::NameFields {
            namespace: self.namespace.as_deref(),
            alias: self.alias.as_deref(),
            receiver: self.receiver.as_deref(),
        }
    }

    /// Effective FlowScript namespace: the explicit field (every first-party node sets one),
    /// else the `NAME_OVERRIDES` residue table, else derived from `category`
    /// (`flow_like_ast::effective_spelling`).
    pub fn flowscript_namespace(&self) -> String {
        flow_like_ast::effective_spelling(&self.name, &self.category, self.name_fields()).0
    }

    /// Effective FlowScript alias: the explicit field, else the override table, else derived
    /// from `name` and `category`.
    pub fn flowscript_alias(&self) -> String {
        flow_like_ast::effective_spelling(&self.name, &self.category, self.name_fields()).1
    }

    /// The data input pin a method-form call binds the receiver to, or `None` when the node is
    /// static-only.
    ///
    /// Without an explicit `receiver` (or an override-table entry), the first data input (by
    /// `index`) is the receiver iff its type is the effective namespace's own value type
    /// (`string` ↔ `String`, `array` ↔ any `Array`, …; `flow_like_ast::VALUE_TYPE_NAMESPACES`).
    /// Nodes outside value-type namespaces are static unless they opt in with
    /// [`Self::set_receiver`]. The rule itself lives in `flow_like_ast::effective_names` so
    /// catalog metadata derives the same answer.
    pub fn flowscript_receiver(&self) -> Option<String> {
        let inputs = self
            .data_inputs_in_order()
            .map(|pin| {
                (
                    pin.name.clone(),
                    format!("{:?}", pin.data_type),
                    format!("{:?}", pin.value_type),
                )
            })
            .collect::<Vec<_>>();
        flow_like_ast::effective_names(
            &self.name,
            &self.category,
            self.name_fields(),
            inputs.iter().map(|(name, data_type, value_type)| {
                (name.as_str(), data_type.as_str(), value_type.as_str())
            }),
        )
        .receiver
    }

    /// The FlowScript method class of the effective receiver pin (`string`, `array`, …; the
    /// schema title for a struct receiver with one), or `None` for static-only nodes and
    /// receivers without a value-type class.
    pub fn flowscript_receiver_class(&self) -> Option<String> {
        let receiver = self.flowscript_receiver()?;
        let pin = self
            .pins
            .values()
            .find(|pin| pin.pin_type == PinType::Input && pin.name == receiver)?;
        flow_like_ast::receiver_class_of(
            &format!("{:?}", pin.data_type),
            &format!("{:?}", pin.value_type),
            pin.schema.as_deref(),
        )
    }

    fn data_inputs_in_order(&self) -> impl Iterator<Item = &Pin> {
        let mut inputs = self
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Input && pin.data_type != VariableType::Execution
            })
            .collect::<Vec<_>>();
        inputs.sort_by(|a, b| (a.index, a.name.as_str()).cmp(&(b.index, b.name.as_str())));
        inputs.into_iter()
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

    /// The schema the listed pins should agree on, scanned in the caller's pin order.
    ///
    /// Ranked, best first, because a pin's schema is only as trustworthy as where it came from:
    ///
    /// 1. A concrete schema on an input pin that has a producer — the shape actually flowing in.
    /// 2. Any other concrete schema. An output pin can only have inherited one *backwards* from
    ///    whatever it feeds, which goes stale the moment the producer upstream is swapped out.
    /// 3. The open-object marker. `set_open_schema` says "this pin accepts any shape", never "its
    ///    peers have no shape", so a permissive consumer must not erase a real schema.
    ///
    /// See [`crate::flow::pin::is_open_object_schema`].
    fn donor_schema(&self, pins: &[&str]) -> Option<String> {
        fn rank(pin: &Pin) -> u8 {
            match pin.schema.as_deref() {
                None => u8::MAX,
                Some(schema) if is_open_object_schema(schema) => 2,
                Some(_) if pin.pin_type == PinType::Input && !pin.depends_on.is_empty() => 0,
                Some(_) => 1,
            }
        }

        let mut best: Option<(u8, &Pin)> = None;
        for name in pins {
            for pin in self.pins.values().filter(|pin| pin.name == *name) {
                let pin_rank = rank(pin);
                if pin_rank == u8::MAX {
                    continue;
                }
                if best.is_none_or(|(best_rank, _)| pin_rank < best_rank) {
                    best = Some((pin_rank, pin));
                }
            }
        }

        best.and_then(|(_, pin)| pin.schema.clone())
    }

    pub fn harmonize_schema(&mut self, pins: Vec<&str>) -> Option<String> {
        let schema = self.donor_schema(&pins)?;

        for pin in self.pins.values_mut() {
            if pins.contains(&pin.name.as_str()) {
                pin.schema = Some(schema.clone());
            }
        }

        Some(schema)
    }

    pub fn harmonize_type(&mut self, pins: Vec<&str>, schema: bool) -> Option<VariableType> {
        // Scan in the caller's pin order, not map iteration order, so the
        // donor pin is deterministic. The type comes from the first
        // non-generic pin; the schema from `donor_schema` — a schema-less or
        // open-shaped sibling must never wipe another pin's concrete schema.
        let variable_type = pins.iter().find_map(|name| {
            self.pins
                .values()
                .find(|pin| pin.name == *name && pin.data_type != VariableType::Generic)
                .map(|pin| pin.data_type.clone())
        })?;

        let found_schema = if schema {
            self.donor_schema(&pins)
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
                    // The open marker declares "any shape", so there is nothing to inherit from it.
                    // Adopting it — from a consumer like Break Struct's `struct_in`, which this pin
                    // reaches through `connected_to` — would erase the shape this pin's own
                    // producer gave it, and `harmonize_type` would then spread the blank downstream.
                    mutable_pin.schema = pin
                        .schema
                        .clone()
                        .filter(|schema| !is_open_object_schema(schema));
                    found_type = pin.data_type.clone();

                    if value_type.is_none() {
                        mutable_pin.value_type = pin.value_type.clone();
                    }
                }
                None => {
                    // The source pin is not on the board handed to us, which is not evidence that
                    // the edge is stale: `node_updates` lifts the node being updated out of the
                    // board before calling `on_update`, and on load this runs before `cleanup` has
                    // repaired anything. Dropping `depends_on` here deletes a wire on an incomplete
                    // view, and `fix_pin_connections` then removes the producer's surviving half —
                    // the whole connection disappears with no error. That cleanup sees the entire
                    // board and is the single authority on pruning.
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

    /// Hash the user-facing definition of a node without including board placement or
    /// graph wiring.
    ///
    /// This is intended for caches of semantic artifacts such as embeddings. Moving a
    /// node, connecting it elsewhere, or changing its generated id does not change the
    /// text or pin shape represented by those artifacts and therefore must not invalidate
    /// them.
    pub fn semantic_hash(&self) -> u64 {
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

        let mut pins: Vec<_> = self.pins.values().collect();
        pins.sort_by(|left, right| {
            let pin_type_order = |pin_type: &PinType| match pin_type {
                PinType::Input => 0_u8,
                PinType::Output => 1_u8,
            };
            pin_type_order(&left.pin_type)
                .cmp(&pin_type_order(&right.pin_type))
                .then_with(|| left.index.cmp(&right.index))
                .then_with(|| left.name.cmp(&right.name))
        });
        for pin in pins {
            hasher.append(pin.name.as_bytes());
            hasher.append(pin.friendly_name.as_bytes());
            hasher.append(pin.description.as_bytes());
            hasher.append(&(pin.pin_type.clone() as u8).to_le_bytes());
            hasher.append(&(pin.data_type.clone() as u8).to_le_bytes());
            hasher.append(&(pin.value_type.clone() as u8).to_le_bytes());
            hasher.append(&pin.index.to_le_bytes());
            if let Some(schema) = &pin.schema {
                hasher.append(schema.as_bytes());
            }
        }

        hasher.finalize64()
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

/// Node types whose `on_update` mints input pins that are not in their catalog definition.
///
/// Three separate mechanisms have to agree on this membership, which is why it lives in one
/// place:
/// - schema sync must not delete these pins (they carry the user's wires, and re-minting
///   assigns fresh ids)
/// - the executability lint must not read them as unfilled required inputs
/// - the FlowScript reconciler predicts them instead of reporting an unknown pin
///
/// Every entry must have an `on_update` that reconciles its dynamic pins **by name**, since
/// that is what lets a preserved pin be adopted rather than duplicated.
pub fn mints_pins_on_update(node_type: &str) -> bool {
    matches!(
        node_type,
        // Placeholder-driven: one pin per token in a literal.
        "string_format"
            | "string_render_template"
            // Mode-driven: pins swap with a dropdown.
            | "a2ui_push_csv_to_chart"
            // Case-driven: one exec pin per case, from a literal or a wired enum.
            | "control_switch"
            // Mirror-driven: pins copied from a target function layer.
            | "control_call_function"
            | "control_call_reference"
            // Schema-driven: one pin per field of the struct schema a wired peer declares.
            | "struct_break"
            | "struct_make_from_schema"
            // Widget-driven: pins derived from a persisted widget's bindings/contract.
            | "a2ui_instantiate_widget"
            | "a2ui_widget_update_inputs"
            | "a2ui_widget_query"
            // SQL-driven: one pin per `$placeholder` in a query literal.
            | "df_sql_query"
            | "df_sql_query_cached"
            | "df_execute_sql"
            | "df_write_delta"
            | "graph_sql_query"
            // Backend-driven: the `source` dropdown decides whether the node reads a database
            // table or a raw vector, and mints the matching inputs. Every one of these reads
            // only its own `source` pin, so the literal-driven prediction path applies.
            | "fit_adaboost"
            | "fit_dbscan"
            | "fit_decision_tree"
            | "fit_elastic_net"
            | "fit_feature_scaler"
            | "fit_gaussian_mixture"
            | "fit_glm"
            | "fit_kmeans"
            | "fit_knn_classifier"
            | "fit_knn_regressor"
            | "fit_linear_regression"
            | "fit_logistic_regression"
            | "fit_multinomial_naive_bayes"
            | "fit_naive_bayes"
            | "fit_one_class_svm"
            | "fit_ordinal_adjacent_category"
            | "fit_ordinal_continuation_ratio"
            | "fit_ordinal_frank_hall"
            | "fit_ordinal_logistic"
            | "fit_ordinal_neural"
            | "fit_ordinal_ridge"
            | "fit_pca"
            | "fit_random_forest"
            | "fit_svm_multi_class"
            | "fit_svm_regression"
            | "fit_tfidf_vectorizer"
            | "fit_tsne"
            | "ml_apply_transform"
            | "ml_predict"
            | "ai_ml_tuning_auto_classifier"
            | "ai_ml_tuning_auto_ordinal"
            | "ai_ml_tuning_grid_search"
            | "ai_ml_tuning_ordinal_grid_search"
    )
}

/// The literal a node derives its dynamic pins from, or `None` when it cannot be read.
///
/// `None` means "unknown", never "declares nothing": the config pin is wired (what runs is decided
/// at runtime), holds no value yet, or holds a non-string. An `on_update` that treats those cases
/// as an empty template reconciles every dynamic pin away — with its wires — so callers must leave
/// their existing pins untouched instead.
pub fn dynamic_pin_source_literal(node: &Node, config_pin: &str) -> Option<String> {
    let pin = node.get_pin_by_name(config_pin)?;
    if !pin.depends_on.is_empty() {
        return None;
    }
    let bytes = pin.default_value.as_ref()?;
    flow_like_types::json::from_slice::<flow_like_types::Value>(bytes)
        .ok()?
        .as_str()
        .map(ToOwned::to_owned)
}

/// Whether `pin` carries a graph edge in either direction.
pub fn pin_is_wired(pin: &Pin) -> bool {
    !pin.depends_on.is_empty() || !pin.connected_to.is_empty()
}

/// Drop the stale dynamic pins named by `stale_ids`, but keep any that still carry a wire and
/// record them on `node.error`.
///
/// An `on_update` that removes a wired pin also removes its half of the edge; `Board::cleanup`'s
/// `fix_pin_connections` then prunes the surviving half on the producer, so the connection
/// disappears from both ends with no error anywhere and the producer is left dead on the canvas.
/// Reconciling a pin away is only ever safe while nothing is attached to it — when something is,
/// the wire is the user's intent and the stale declaration is the thing to report.
pub fn remove_unwired_pins(node: &mut Node, stale_ids: &[String]) {
    let mut kept: Vec<String> = Vec::new();
    for id in stale_ids {
        match node.pins.get(id) {
            Some(pin) if pin_is_wired(pin) => kept.push(pin.name.clone()),
            Some(_) => {
                node.pins.remove(id);
            }
            None => {}
        }
    }

    if kept.is_empty() {
        return;
    }
    kept.sort();
    kept.dedup();
    let message = format!(
        "Still connected but no longer declared: {}. Disconnect them, or restore what declared them.",
        kept.join(", ")
    );
    node.error = Some(match node.error.take() {
        Some(existing) if !existing.is_empty() => format!("{existing} {message}"),
        _ => message,
    });
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

    use crate::flow::pin::OPEN_OBJECT_SCHEMA;

    const CONCRETE_SCHEMA: &str =
        r#"{"title":"A2UIFileInputFile","type":"object","properties":{"name":{"type":"string"}}}"#;
    const STALE_SCHEMA: &str =
        r#"{"title":"Bit","type":"object","properties":{"id":{"type":"string"}}}"#;

    /// `Get Element` harmonizes its element pin with its array pin. The element pin reaches its
    /// *consumer* (Break Struct's open `struct_in`) through `connected_to`, so without a
    /// preference the open marker would be stamped over the array's real schema.
    #[test]
    fn a_concrete_schema_outranks_the_open_marker_when_harmonizing() {
        let mut node = super::Node::new("array_get", "Get Element", "", "Utils/Array");
        node.add_input_pin("array_in", "Array", "", super::VariableType::Struct)
            .schema = Some(CONCRETE_SCHEMA.to_string());
        node.add_output_pin("element", "Element", "", super::VariableType::Struct)
            .schema = Some(OPEN_OBJECT_SCHEMA.to_string());

        node.harmonize_type(vec!["element", "array_in"], true);

        for name in ["element", "array_in"] {
            assert_eq!(
                node.get_pin_by_name(name).unwrap().schema.as_deref(),
                Some(CONCRETE_SCHEMA),
                "`{name}` must keep the concrete schema"
            );
        }

        node.harmonize_schema(vec!["element", "array_in"]);
        assert_eq!(
            node.get_pin_by_name("element").unwrap().schema.as_deref(),
            Some(CONCRETE_SCHEMA)
        );
    }

    /// A passthrough follows its producer, not the consumer it feeds. The consumer's copy of the
    /// previous shape would otherwise be stamped back over the array that was just rewired.
    #[test]
    fn an_upstream_schema_outranks_one_inherited_from_a_consumer() {
        let mut node = super::Node::new("array_get", "Get Element", "", "Utils/Array");
        let array_in = node.add_input_pin("array_in", "Array", "", super::VariableType::Struct);
        array_in.schema = Some(CONCRETE_SCHEMA.to_string());
        array_in.depends_on.insert("producer-pin".to_string());
        node.add_output_pin("element", "Element", "", super::VariableType::Struct)
            .schema = Some(STALE_SCHEMA.to_string());

        node.harmonize_type(vec!["element", "array_in"], true);

        for name in ["element", "array_in"] {
            assert_eq!(
                node.get_pin_by_name(name).unwrap().schema.as_deref(),
                Some(CONCRETE_SCHEMA),
                "`{name}` must follow the wired producer"
            );
        }
    }

    /// With nothing concrete to spread, the marker is still the best answer available: the pins
    /// agree that the shape is open rather than losing the declaration entirely.
    #[test]
    fn the_open_marker_still_spreads_when_no_pin_has_a_real_schema() {
        let mut node = super::Node::new("array_get", "Get Element", "", "Utils/Array");
        node.add_input_pin("array_in", "Array", "", super::VariableType::Struct);
        node.add_output_pin("element", "Element", "", super::VariableType::Struct)
            .schema = Some(OPEN_OBJECT_SCHEMA.to_string());

        node.harmonize_type(vec!["element", "array_in"], true);

        assert_eq!(
            node.get_pin_by_name("array_in").unwrap().schema.as_deref(),
            Some(OPEN_OBJECT_SCHEMA)
        );
    }

    /// `match_type` on an output pin reads `connected_to`, so it inherits from the consumer.
    #[tokio::test]
    async fn match_type_does_not_inherit_an_open_marker_from_a_consumer() {
        use flow_like_storage::object_store::path::Path;

        let mut board = super::Board::new_detached(Some("match-type".to_string()), Path::default());

        let mut consumer = super::Node::new("struct_break", "Break Struct", "", "Structs");
        consumer.id = "consumer".to_string();
        consumer
            .add_input_pin("struct_in", "Struct", "", super::VariableType::Struct)
            .set_open_schema();
        let consumer_pin = consumer.get_pin_by_name("struct_in").unwrap().id.clone();
        board.nodes.insert(consumer.id.clone(), consumer);

        let mut passthrough = super::Node::new("array_get", "Get Element", "", "Utils/Array");
        passthrough.id = "passthrough".to_string();
        let element =
            passthrough.add_output_pin("element", "Element", "", super::VariableType::Generic);
        element.connected_to.insert(consumer_pin);

        passthrough
            .match_type("element", &board, Some(super::ValueType::Normal), None)
            .unwrap();

        assert_eq!(
            passthrough.get_pin_by_name("element").unwrap().data_type,
            super::VariableType::Struct,
            "the data type is still inherited from the consumer"
        );
        assert_eq!(
            passthrough.get_pin_by_name("element").unwrap().schema,
            None,
            "the open marker declares nothing, so there is no schema to inherit"
        );
    }

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
    fn flowscript_names_derive_unless_explicit() {
        let mut node = super::Node::new("string_trim", "Trim", "", "Utils/String");
        node.add_input_pin("string", "String", "", super::VariableType::String);
        assert_eq!(node.flowscript_namespace(), "string");
        assert_eq!(node.flowscript_alias(), "trim");
        assert_eq!(node.flowscript_receiver().as_deref(), Some("string"));
        assert_eq!(node.flowscript_receiver_class().as_deref(), Some("string"));

        node.set_flowscript_name("text", "strip").set_receiver("");
        assert_eq!(node.flowscript_namespace(), "text");
        assert_eq!(node.flowscript_alias(), "strip");
        assert_eq!(node.flowscript_receiver(), None);
        assert_eq!(node.flowscript_receiver_class(), None);
    }

    #[test]
    fn default_receiver_requires_the_namespace_value_type() {
        let mut from_int = super::Node::new("string_from_int", "From Int", "", "Utils/String");
        from_int.add_input_pin("exec_in", "In", "", super::VariableType::Execution);
        from_int.add_input_pin("value", "Value", "", super::VariableType::Integer);
        assert_eq!(from_int.flowscript_receiver(), None);

        let mut push = super::Node::new("array_push", "Push", "", "Utils/Array");
        push.add_input_pin("array", "Array", "", super::VariableType::Generic)
            .set_value_type(super::ValueType::Array);
        push.add_input_pin("item", "Item", "", super::VariableType::Generic);
        assert_eq!(push.flowscript_receiver().as_deref(), Some("array"));
        assert_eq!(push.flowscript_receiver_class().as_deref(), Some("array"));

        let mut probe = super::Node::new("http_probe", "Probe", "", "Web/API");
        probe.add_input_pin("url", "URL", "", super::VariableType::String);
        assert_eq!(probe.flowscript_receiver(), None);
        probe.set_receiver("url");
        assert_eq!(probe.flowscript_receiver().as_deref(), Some("url"));
        assert_eq!(probe.flowscript_receiver_class().as_deref(), Some("string"));
    }

    #[test]
    fn explicit_fields_win_over_derivation() {
        let mut md5 = super::Node::new("utils_hash_md5", "MD5", "", "Utils/Hash");
        md5.add_input_pin("input", "Input", "", super::VariableType::String);
        // The override residue is empty after the bake-in: an un-annotated node derives.
        assert_eq!(md5.flowscript_namespace(), "hash");
        assert_eq!(md5.flowscript_alias(), "md5");
        assert_eq!(md5.flowscript_receiver(), None);

        md5.set_flowscript_name("hash", "md5").set_receiver("input");
        assert_eq!(md5.flowscript_receiver().as_deref(), Some("input"));
        assert_eq!(md5.flowscript_receiver_class().as_deref(), Some("string"));

        md5.set_flowscript_name("digest", "md5").set_receiver("");
        assert_eq!(md5.flowscript_namespace(), "digest");
        assert_eq!(md5.flowscript_receiver(), None);
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

    #[test]
    fn semantic_hash_ignores_canvas_position_and_graph_wiring() {
        let mut node = super::Node::new("test_node", "Test", "desc", "Cat");
        let pin_id = node
            .add_input_pin("input", "Input", "An input", super::VariableType::String)
            .id
            .clone();
        let initial = node.semantic_hash();

        node.coordinates = Some((10.0, 20.0, 1.0));
        node.pins
            .get_mut(&pin_id)
            .unwrap()
            .depends_on
            .insert("upstream-pin".to_string());

        assert_eq!(initial, node.semantic_hash());
    }

    #[test]
    fn semantic_hash_tracks_display_metadata_and_pin_shape() {
        let mut node = super::Node::new("test_node", "Test", "desc", "Cat");
        node.add_input_pin("input", "Input", "An input", super::VariableType::String);
        let initial = node.semantic_hash();

        node.description = "updated description".to_string();
        assert_ne!(initial, node.semantic_hash());

        let after_description = node.semantic_hash();
        node.add_output_pin("output", "Output", "An output", super::VariableType::String);
        assert_ne!(after_description, node.semantic_hash());
    }
}
