use crate::layer::{LayerCache, LayerCacheScope};
use flow_like_types_proto::{FromProto, ToProto};

impl LayerCacheScope {
    fn to_proto(self) -> i32 {
        match self {
            LayerCacheScope::App => 0,
            LayerCacheScope::User => 1,
        }
    }

    fn from_proto(value: i32) -> Self {
        match value {
            1 => LayerCacheScope::User,
            _ => LayerCacheScope::App,
        }
    }
}

impl ToProto<flow_like_types_proto::proto::LayerCache> for LayerCache {
    fn to_proto(&self) -> flow_like_types_proto::proto::LayerCache {
        flow_like_types_proto::proto::LayerCache {
            enabled: self.enabled,
            prefix: self.prefix.clone(),
            ttl_seconds: self.ttl_seconds,
            scope: self.scope.to_proto(),
        }
    }
}

impl FromProto<flow_like_types_proto::proto::LayerCache> for LayerCache {
    fn from_proto(proto: flow_like_types_proto::proto::LayerCache) -> Self {
        LayerCache {
            enabled: proto.enabled,
            prefix: proto.prefix,
            ttl_seconds: proto.ttl_seconds,
            scope: LayerCacheScope::from_proto(proto.scope),
        }
    }
}
