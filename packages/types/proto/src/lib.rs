//! Generated protobuf wire types and their conversion traits.

// prost emits one oneof variant per component / page-content kind; their sizes differ by
// design and the file is regenerated on every build, so the allow has to live at the include.
#[allow(clippy::large_enum_variant)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/flow_like_types.rs"));
}

pub trait ToProto<T> {
    fn to_proto(&self) -> T;
}

pub trait FromProto<T> {
    fn from_proto(proto: T) -> Self;
}

pub type Timestamp = prost_types::Timestamp;

pub use prost::Message;

#[cfg(test)]
mod tests {
    use super::{Message, proto};

    fn board(keys: &[&str]) -> proto::Board {
        proto::Board {
            id: "board".into(),
            refs: keys
                .iter()
                .map(|key| (key.to_string(), key.to_string()))
                .collect(),
            nodes: keys
                .iter()
                .map(|key| {
                    let node = proto::Node {
                        id: key.to_string(),
                        ..Default::default()
                    };
                    (key.to_string(), node)
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn map_insertion_order_does_not_change_encoded_bytes() {
        let keys = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let mut reversed = keys;
        reversed.reverse();
        assert_eq!(
            board(&keys).encode_to_vec(),
            board(&reversed).encode_to_vec()
        );
    }
}
