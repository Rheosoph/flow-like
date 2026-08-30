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
