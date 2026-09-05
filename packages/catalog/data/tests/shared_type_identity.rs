//! Shared cache and wire handles must retain one Rust type through every public path.

use flow_like_catalog_data as catalog;
use flow_like_catalog_data_support as support;
use std::any::TypeId;

macro_rules! assert_same_type {
    ($facade:ty, $support:ty) => {
        assert_eq!(
            TypeId::of::<$facade>(),
            TypeId::of::<$support>(),
            "{} must re-export {} to preserve cache downcasts and typed values",
            stringify!($facade),
            stringify!($support),
        );
    };
}

#[test]
fn facade_and_support_share_cache_query_path_and_chat_types() {
    assert_same_type!(
        catalog::data::cache::FlowCache,
        support::data::cache::FlowCache
    );
    assert_same_type!(
        catalog::data::cache::CacheScope,
        support::data::cache::CacheScope
    );
    assert_same_type!(
        catalog::data::cache::CacheHit,
        support::data::cache::CacheHit
    );
    assert_same_type!(
        catalog::data::datafusion::session::DataFusionSession,
        support::data::datafusion::session::DataFusionSession
    );
    assert_same_type!(
        catalog::data::datafusion::query::QueryRow,
        support::data::datafusion::query::QueryRow
    );
    assert_same_type!(
        catalog::data::excel::CSVTable,
        support::data::excel::CSVTable
    );
    assert_same_type!(catalog::data::excel::Cell, support::data::excel::Cell);
    assert_same_type!(catalog::data::path::FlowPath, support::data::path::FlowPath);
    assert_same_type!(
        catalog::data::path::FlowPathRuntime,
        support::data::path::FlowPathRuntime
    );
    assert_same_type!(
        catalog::events::chat_event::Attachment,
        support::events::chat_event::Attachment
    );
    assert_same_type!(
        catalog::events::chat_event::ComplexAttachment,
        support::events::chat_event::ComplexAttachment
    );
}

#[cfg(feature = "execute")]
#[test]
fn facade_and_support_share_runtime_cache_types() {
    assert_same_type!(
        catalog::data::datafusion::session::CachedDataFusionSession,
        support::data::datafusion::session::CachedDataFusionSession
    );
    assert_same_type!(
        dyn catalog::data::datafusion::session::DeferredMount,
        dyn support::data::datafusion::session::DeferredMount
    );
    assert_same_type!(
        catalog::data::db::graph::CachedGraphStore,
        support::data::db::graph::CachedGraphStore
    );
}
