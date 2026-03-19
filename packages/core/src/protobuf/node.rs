use flow_like_types::{FromProto, ToProto};

use crate::flow::{
    node::{FnRefs, Node, NodePermission, NodeScores, NodeWasm},
    pin::Pin,
};

impl ToProto<flow_like_types::proto::NodeScores> for NodeScores {
    fn to_proto(&self) -> flow_like_types::proto::NodeScores {
        flow_like_types::proto::NodeScores {
            privacy: self.privacy as u32,
            security: self.security as u32,
            performance: self.performance as u32,
            governance: self.governance as u32,
            reliability: self.reliability as u32,
            cost: self.cost as u32,
        }
    }
}

impl FromProto<flow_like_types::proto::NodeScores> for NodeScores {
    fn from_proto(proto: flow_like_types::proto::NodeScores) -> Self {
        NodeScores {
            privacy: proto.privacy as u8,
            security: proto.security as u8,
            performance: proto.performance as u8,
            governance: proto.governance as u8,
            reliability: proto.reliability as u8,
            cost: proto.cost as u8,
        }
    }
}

impl ToProto<flow_like_types::proto::FnRefs> for FnRefs {
    fn to_proto(&self) -> flow_like_types::proto::FnRefs {
        flow_like_types::proto::FnRefs {
            fn_refs: self.fn_refs.clone(),
            can_reference_fns: self.can_reference_fns,
            can_be_referenced_by_fns: self.can_be_referenced_by_fns,
        }
    }
}

impl FromProto<flow_like_types::proto::FnRefs> for FnRefs {
    fn from_proto(proto: flow_like_types::proto::FnRefs) -> Self {
        FnRefs {
            fn_refs: proto.fn_refs,
            can_reference_fns: proto.can_reference_fns,
            can_be_referenced_by_fns: proto.can_be_referenced_by_fns,
        }
    }
}

impl ToProto<flow_like_types::proto::NodeWasm> for NodeWasm {
    fn to_proto(&self) -> flow_like_types::proto::NodeWasm {
        flow_like_types::proto::NodeWasm {
            package_id: self.package_id.clone(),
            permissions: self
                .permissions
                .iter()
                .filter_map(|p| serde_json::to_value(p).ok()?.as_str().map(String::from))
                .collect(),
        }
    }
}

impl FromProto<flow_like_types::proto::NodeWasm> for NodeWasm {
    fn from_proto(proto: flow_like_types::proto::NodeWasm) -> Self {
        NodeWasm {
            package_id: proto.package_id,
            permissions: proto
                .permissions
                .iter()
                .filter_map(|s| serde_json::from_value(serde_json::Value::String(s.clone())).ok())
                .collect(),
        }
    }
}

impl ToProto<flow_like_types::proto::Node> for Node {
    fn to_proto(&self) -> flow_like_types::proto::Node {
        let (coord_x, coord_y, coord_z) = self.coordinates.unwrap_or((0.0, 0.0, 0.0));
        flow_like_types::proto::Node {
            id: self.id.clone(),
            name: self.name.clone(),
            friendly_name: self.friendly_name.clone(),
            description: self.description.clone(),
            coord_x,
            coord_y,
            coord_z,
            category: self.category.clone(),
            scores: self.scores.as_ref().map(|s| s.to_proto()),
            pins: self
                .pins
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            start: self.start.unwrap_or(false),
            icon: self.icon.clone().unwrap_or_default(),
            comment: self.comment.clone(),
            long_running: self.long_running.unwrap_or(false),
            error: self.error.clone(),
            docs: self.docs.clone(),
            layer: self.layer.clone(),
            event_callback: self.event_callback.unwrap_or(false),
            hash: self.hash,
            fn_refs: self.fn_refs.as_ref().map(|f| f.to_proto()),
            oauth_providers: self.oauth_providers.clone().unwrap_or_default(),
            required_oauth_scopes: self
                .required_oauth_scopes
                .as_ref()
                .map(|scopes| {
                    scopes
                        .iter()
                        .map(|(k, v)| {
                            (
                                k.clone(),
                                flow_like_types::proto::StringList { values: v.clone() },
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
            only_offline: self.only_offline,
            version: self.version,
            wasm: self.wasm.as_ref().map(|w| w.to_proto()),
        }
    }
}

impl FromProto<flow_like_types::proto::Node> for Node {
    fn from_proto(proto: flow_like_types::proto::Node) -> Self {
        Node {
            id: proto.id,
            name: proto.name,
            friendly_name: proto.friendly_name,
            description: proto.description,
            coordinates: Some((proto.coord_x, proto.coord_y, proto.coord_z)),
            category: proto.category,
            scores: proto.scores.map(NodeScores::from_proto),
            pins: proto
                .pins
                .into_iter()
                .map(|(k, v)| (k, Pin::from_proto(v)))
                .collect(),
            start: if proto.start { Some(true) } else { None },
            icon: if proto.icon.is_empty() {
                None
            } else {
                Some(proto.icon)
            },
            comment: proto.comment,
            long_running: if proto.long_running { Some(true) } else { None },
            error: proto.error,
            docs: proto.docs,
            event_callback: if proto.event_callback {
                Some(true)
            } else {
                None
            },
            layer: proto.layer,
            hash: proto.hash,
            fn_refs: proto.fn_refs.map(FnRefs::from_proto),
            oauth_providers: if proto.oauth_providers.is_empty() {
                None
            } else {
                Some(proto.oauth_providers)
            },
            required_oauth_scopes: if proto.required_oauth_scopes.is_empty() {
                None
            } else {
                Some(
                    proto
                        .required_oauth_scopes
                        .into_iter()
                        .map(|(k, v)| (k, v.values))
                        .collect(),
                )
            },
            only_offline: proto.only_offline,
            version: proto.version,
            wasm: proto.wasm.map(NodeWasm::from_proto),
        }
    }
}
