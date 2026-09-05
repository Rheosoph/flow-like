//! Keep the web facade's public paths and existing registration order stable.

use flow_like_catalog_web::{self as catalog, NodeLogic};

macro_rules! assert_legacy_registry {
    ($($node:path),* $(,)?) => {
        #[test]
        #[allow(clippy::default_constructed_unit_structs)]
        fn facade_preserves_public_nodes_and_registration_order() {
            let expected = vec![$(<$node>::default().get_node().name),*];
            for (entry_point, nodes) in [
                ("collect_nodes", catalog::collect_nodes()),
                ("get_catalog", catalog::get_catalog()),
            ] {
                let actual: Vec<_> = nodes.iter().map(|node| node.get_node().name).collect();
                assert_eq!(actual, expected, "{entry_point} changed the web catalog");
            }
        }
    };
}

macro_rules! assert_public_bot_paths {
    ($($node:path),* $(,)?) => {
        #[test]
        fn facade_preserves_bot_node_types() {
            fn assert_node<T: NodeLogic + Default>() {}
            $(assert_node::<$node>();)*
        }
    };
}

include!("fixtures/legacy_node_paths.rs");
include!("fixtures/bot_node_paths.rs");
