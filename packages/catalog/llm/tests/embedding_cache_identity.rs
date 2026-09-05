use flow_like_catalog_embedding::CachedEmbeddingModelObject as SharedModel;
use flow_like_catalog_llm::embedding::CachedEmbeddingModelObject as CatalogModel;
use flow_like_types::Cacheable;
use std::{any::TypeId, sync::Arc};

#[test]
fn catalog_and_wasm_support_share_the_same_cache_type() {
    assert_eq!(TypeId::of::<CatalogModel>(), TypeId::of::<SharedModel>());
    let cached: Arc<dyn Cacheable> = Arc::new(CatalogModel {
        text_model: None,
        image_model: None,
    });
    assert!(cached.as_any().downcast_ref::<SharedModel>().is_some());
}
