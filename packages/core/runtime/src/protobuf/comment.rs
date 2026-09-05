use crate::flow::board::{Comment, CommentType};
use flow_like_types::{FromProto, Timestamp, ToProto};
use std::time::SystemTime;

impl CommentType {
    fn to_proto(&self) -> i32 {
        match self {
            CommentType::Text => 0,
            CommentType::Image => 1,
            CommentType::Video => 2,
        }
    }

    fn from_proto(value: i32) -> Self {
        match value {
            0 => CommentType::Text,
            1 => CommentType::Image,
            2 => CommentType::Video,
            _ => CommentType::Text, // Default
        }
    }
}

impl ToProto<flow_like_types::proto::Comment> for Comment {
    fn to_proto(&self) -> flow_like_types::proto::Comment {
        flow_like_types::proto::Comment {
            id: self.id.clone(),
            author: self.author.clone().unwrap_or_default(),
            content: self.content.clone(),
            comment_type: self.comment_type.to_proto(),
            timestamp: Some(Timestamp::from(self.timestamp)),
            coord_x: self.coordinates.0,
            coord_y: self.coordinates.1,
            coord_z: self.coordinates.2,
            layer: self.layer.clone(),
            width: self.width,
            height: self.height,
            color: self.color.clone(),
            z_index: self.z_index,
            hash: self.hash,
            is_locked: self.is_locked,
            node_id: self.node_id.clone(),
        }
    }
}

impl FromProto<flow_like_types::proto::Comment> for Comment {
    fn from_proto(proto: flow_like_types::proto::Comment) -> Self {
        Comment {
            id: proto.id,
            author: if proto.author.is_empty() {
                None
            } else {
                Some(proto.author)
            },
            content: proto.content,
            comment_type: CommentType::from_proto(proto.comment_type),
            timestamp: proto
                .timestamp
                .map(|t| SystemTime::try_from(t).unwrap_or(SystemTime::UNIX_EPOCH))
                .unwrap_or(SystemTime::UNIX_EPOCH),
            coordinates: (proto.coord_x, proto.coord_y, proto.coord_z),
            layer: proto.layer,
            width: proto.width,
            height: proto.height,
            color: proto.color,
            z_index: proto.z_index,
            hash: proto.hash,
            is_locked: proto.is_locked,
            node_id: proto.node_id,
        }
    }
}
