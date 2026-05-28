use crate::flow::{
    board::{Board, Comment, ExecutionMode, ExecutionStage, Layer, LayerType},
    execution::LogLevel,
    node::Node,
    pin::Pin,
    variable::Variable,
};
use flow_like_storage::Path;
use flow_like_types::{FromProto, Timestamp, ToProto};
use std::{collections::HashMap, time::SystemTime};

impl ExecutionStage {
    fn to_proto(&self) -> i32 {
        match self {
            ExecutionStage::Dev => 0,
            ExecutionStage::Int => 1,
            ExecutionStage::QA => 2,
            ExecutionStage::PreProd => 3,
            ExecutionStage::Prod => 4,
        }
    }

    fn from_proto(value: i32) -> Self {
        match value {
            0 => ExecutionStage::Dev,
            1 => ExecutionStage::Int,
            2 => ExecutionStage::QA,
            3 => ExecutionStage::PreProd,
            4 => ExecutionStage::Prod,
            _ => ExecutionStage::Dev,
        }
    }
}

impl ExecutionMode {
    fn to_proto(&self) -> i32 {
        match self {
            ExecutionMode::Hybrid => 0,
            ExecutionMode::Remote => 1,
            ExecutionMode::Local => 2,
        }
    }

    fn from_proto(value: i32) -> Self {
        match value {
            0 => ExecutionMode::Hybrid,
            1 => ExecutionMode::Remote,
            2 => ExecutionMode::Local,
            _ => ExecutionMode::Hybrid,
        }
    }
}

impl LayerType {
    fn to_proto(&self) -> i32 {
        match self {
            LayerType::Function => 0,
            LayerType::Macro => 1,
            LayerType::Collapsed => 2,
        }
    }

    fn from_proto(value: i32) -> Self {
        match value {
            0 => LayerType::Function,
            1 => LayerType::Macro,
            2 => LayerType::Collapsed,
            _ => LayerType::Function,
        }
    }
}

impl LogLevel {
    fn to_proto(&self) -> i32 {
        match self {
            LogLevel::Debug => 0,
            LogLevel::Info => 1,
            LogLevel::Warn => 2,
            LogLevel::Error => 3,
            LogLevel::Fatal => 4,
        }
    }

    fn from_proto(value: i32) -> Self {
        match value {
            0 => LogLevel::Debug,
            1 => LogLevel::Info,
            2 => LogLevel::Warn,
            3 => LogLevel::Error,
            4 => LogLevel::Fatal,
            _ => LogLevel::Debug, // Default
        }
    }
}

impl ToProto<flow_like_types::proto::Board> for Board {
    fn to_proto(&self) -> flow_like_types::proto::Board {
        flow_like_types::proto::Board {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            nodes: self
                .nodes
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            variables: self
                .variables
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            comments: self
                .comments
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            layers: self
                .layers
                .iter()
                .map(|(layer_id, layer)| (layer_id.clone(), layer.to_proto()))
                .collect(),
            page_ids: self.page_ids.clone(),
            viewport_x: self.viewport.0,
            viewport_y: self.viewport.1,
            viewport_zoom: self.viewport.2,
            version_major: self.version.0,
            version_minor: self.version.1,
            version_patch: self.version.2,
            stage: self.stage.to_proto(),
            log_level: self.log_level.to_proto(),
            execution_mode: self.execution_mode.to_proto(),
            refs: self.refs.clone(),
            hash: self.hash,
            created_at: Some(Timestamp::from(self.created_at)),
            updated_at: Some(Timestamp::from(self.updated_at)),
        }
    }
}

impl FromProto<flow_like_types::proto::Board> for Board {
    fn from_proto(proto: flow_like_types::proto::Board) -> Self {
        Board {
            id: proto.id,
            name: proto.name,
            description: proto.description,
            nodes: proto
                .nodes
                .into_iter()
                .map(|(k, v)| (k, Node::from_proto(v)))
                .collect(),
            variables: proto
                .variables
                .into_iter()
                .map(|(k, v)| (k, Variable::from_proto(v)))
                .collect(),
            comments: proto
                .comments
                .into_iter()
                .map(|(k, v)| (k, Comment::from_proto(v)))
                .collect(),
            viewport: (proto.viewport_x, proto.viewport_y, proto.viewport_zoom),
            version: (
                proto.version_major,
                proto.version_minor,
                proto.version_patch,
            ),
            layers: proto
                .layers
                .into_iter()
                .map(|(layer_id, layer)| (layer_id, Layer::from_proto(layer)))
                .collect(),
            page_ids: proto.page_ids,
            stage: ExecutionStage::from_proto(proto.stage),
            log_level: LogLevel::from_proto(proto.log_level),
            execution_mode: ExecutionMode::from_proto(proto.execution_mode),
            refs: proto.refs,
            hash: proto.hash,
            created_at: proto
                .created_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
            updated_at: proto
                .updated_at
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
            parent: None,
            board_dir: Path::from("/default"),
            logic_nodes: HashMap::new(),
            app_state: None,
        }
    }
}

impl ToProto<flow_like_types::proto::Layer> for Layer {
    fn to_proto(&self) -> flow_like_types::proto::Layer {
        flow_like_types::proto::Layer {
            id: self.id.clone(),
            name: self.name.clone(),
            comments: self
                .comments
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            coord_x: self.coordinates.0,
            coord_y: self.coordinates.1,
            coord_z: self.coordinates.2,
            coord_x_in: self.in_coordinates.map(|c| c.0),
            coord_y_in: self.in_coordinates.map(|c| c.1),
            coord_z_in: self.in_coordinates.map(|c| c.2),
            coord_x_out: self.out_coordinates.map(|c| c.0),
            coord_y_out: self.out_coordinates.map(|c| c.1),
            coord_z_out: self.out_coordinates.map(|c| c.2),
            parent_id: self.parent_id.clone(),
            pins: self
                .pins
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            r#type: self.r#type.to_proto(),
            nodes: self
                .nodes
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            variables: self
                .variables
                .iter()
                .map(|(k, v)| (k.clone(), v.to_proto()))
                .collect(),
            comment: self.comment.clone(),
            error: self.error.clone(),
            color: self.color.clone(),
            hash: self.hash,
        }
    }
}

impl FromProto<flow_like_types::proto::Layer> for Layer {
    fn from_proto(proto: flow_like_types::proto::Layer) -> Self {
        let (in_default, out_default) = if proto.nodes.is_empty() {
            let base = (proto.coord_x, proto.coord_y, proto.coord_z);
            (
                (base.0 - 50.0, base.1, base.2),
                (base.0 + 50.0, base.1, base.2),
            )
        } else {
            let mut min_x = f32::INFINITY;
            let mut min_y = 0.0;
            let mut min_z = 0.0;
            let mut max_x = f32::NEG_INFINITY;
            let mut max_y = 0.0;
            let mut max_z = 0.0;

            for n in proto.nodes.values() {
                let x = n.coord_x;
                if x < min_x {
                    min_x = x;
                    min_y = n.coord_y;
                    min_z = n.coord_z;
                }
                if x > max_x {
                    max_x = x;
                    max_y = n.coord_y;
                    max_z = n.coord_z;
                }
            }

            if proto.nodes.is_empty() {
                ((-50.0, min_y, min_z), (50.0, max_y, max_z))
            } else {
                ((min_x - 50.0, min_y, min_z), (max_x + 50.0, max_y, max_z))
            }
        };

        Layer {
            id: proto.id,
            name: proto.name,
            comments: proto
                .comments
                .into_iter()
                .map(|(k, v)| (k, Comment::from_proto(v)))
                .collect(),
            coordinates: (proto.coord_x, proto.coord_y, proto.coord_z),
            parent_id: proto.parent_id,
            pins: proto
                .pins
                .into_iter()
                .map(|(k, v)| (k, Pin::from_proto(v)))
                .collect(),
            r#type: LayerType::from_proto(proto.r#type),
            nodes: proto
                .nodes
                .into_iter()
                .map(|(k, v)| (k, Node::from_proto(v)))
                .collect(),
            variables: proto
                .variables
                .into_iter()
                .map(|(k, v)| (k, Variable::from_proto(v)))
                .collect(),
            in_coordinates: Some((
                proto.coord_x_in.unwrap_or(in_default.0),
                proto.coord_y_in.unwrap_or(in_default.1),
                proto.coord_z_in.unwrap_or(in_default.2),
            )),
            out_coordinates: Some((
                proto.coord_x_out.unwrap_or(out_default.0),
                proto.coord_y_out.unwrap_or(out_default.1),
                proto.coord_z_out.unwrap_or(out_default.2),
            )),
            comment: proto.comment,
            error: proto.error,
            color: proto.color,
            hash: proto.hash,
        }
    }
}
